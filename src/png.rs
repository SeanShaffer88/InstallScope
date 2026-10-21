//! PNG reading and writing (RFC 2083 / ISO 15948) on the own DEFLATE code.
//!
//! Reading covers every non-interlaced PNG: grey, RGB, palette, grey+alpha
//! and RGBA at any legal bit depth, with tRNS transparency; output is always
//! 8-bit RGBA. Writing produces 8-bit RGBA with a per-row filter choice.

use crate::deflate::{crc32, Crc32, Deflate, Inflate};
use std::io::{self, Read, Write};

const SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];
/// Refuse images that would need more memory than this to decode.
const MAX_PIXELS: u64 = 64 * 1024 * 1024;

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1u32, 0u32);
    for chunk in data.chunks(5552) {
        for &x in chunk {
            a += x as u32;
            b += a;
        }
        a %= 65521;
        b %= 65521;
    }
    (b << 16) | a
}

/// A decoded image: 8-bit RGBA, not premultiplied, rows top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    pub width: u32,
    pub height: u32,
    pub pixels: Vec<u8>,
}

fn paeth(a: u8, b: u8, c: u8) -> u8 {
    let p = a as i16 + b as i16 - c as i16;
    let (pa, pb, pc) = (
        (p - a as i16).abs(),
        (p - b as i16).abs(),
        (p - c as i16).abs(),
    );
    if pa <= pb && pa <= pc {
        a
    } else if pb <= pc {
        b
    } else {
        c
    }
}

/// Decode a PNG file.
pub fn decode(bytes: &[u8]) -> io::Result<Rgba> {
    if bytes.len() < 8 || bytes[..8] != SIGNATURE {
        return Err(bad("not a PNG image"));
    }
    let mut at = 8;
    let (mut w, mut h, mut depth, mut color) = (0u32, 0u32, 0u8, 0u8);
    let mut palette: Vec<[u8; 4]> = Vec::new();
    let mut trns: Vec<u8> = Vec::new();
    let mut idat = Vec::new();
    let mut seen_header = false;
    loop {
        if at + 12 > bytes.len() {
            return Err(bad("the PNG image is cut off"));
        }
        let len = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
        let kind = &bytes[at + 4..at + 8];
        let end = at
            .checked_add(12 + len)
            .filter(|&e| e <= bytes.len())
            .ok_or_else(|| bad("the PNG image is cut off"))?;
        let data = &bytes[at + 8..at + 8 + len];
        let crc = u32::from_be_bytes(bytes[end - 4..end].try_into().unwrap());
        if crc32(&bytes[at + 4..at + 8 + len]) != crc {
            return Err(bad("the PNG image is damaged (checksum mismatch)"));
        }
        match kind {
            b"IHDR" => {
                if len != 13 {
                    return Err(bad("bad PNG header"));
                }
                w = u32::from_be_bytes(data[0..4].try_into().unwrap());
                h = u32::from_be_bytes(data[4..8].try_into().unwrap());
                depth = data[8];
                color = data[9];
                if data[10] != 0 || data[11] != 0 {
                    return Err(bad("unknown PNG compression or filter method"));
                }
                if data[12] != 0 {
                    return Err(bad("interlaced PNG images aren't supported"));
                }
                let ok = match color {
                    0 => matches!(depth, 1 | 2 | 4 | 8 | 16),
                    3 => matches!(depth, 1 | 2 | 4 | 8),
                    2 | 4 | 6 => matches!(depth, 8 | 16),
                    _ => false,
                };
                if !ok || w == 0 || h == 0 || w as u64 * h as u64 > MAX_PIXELS {
                    return Err(bad("unsupported PNG image"));
                }
                seen_header = true;
            }
            b"PLTE" => {
                palette = data
                    .chunks_exact(3)
                    .map(|c| [c[0], c[1], c[2], 255])
                    .collect();
            }
            b"tRNS" => trns = data.to_vec(),
            b"IDAT" => idat.extend_from_slice(data),
            b"IEND" => break,
            _ => {
                // Unknown critical chunks (upper-case first letter) can't be skipped.
                if kind[0].is_ascii_uppercase() {
                    return Err(bad("unsupported PNG chunk"));
                }
            }
        }
        at = end;
    }
    if !seen_header || idat.len() < 2 {
        return Err(bad("the PNG image has no data"));
    }
    if color == 3 {
        if palette.is_empty() {
            return Err(bad("the PNG palette is missing"));
        }
        for (p, &a) in palette.iter_mut().zip(&trns) {
            p[3] = a;
        }
    }
    let channels = match color {
        0 | 3 => 1,
        2 => 3,
        4 => 2,
        _ => 4,
    };
    let bits = channels * depth as usize;
    let stride = (w as usize * bits).div_ceil(8);
    let bpp = bits.div_ceil(8).max(1);
    let expected = (stride + 1) * h as usize;
    // zlib wrapper: 2-byte header, DEFLATE data, Adler-32.
    if idat[0] & 0x0F != 8
        || !(idat[0] as u16 * 256 + idat[1] as u16).is_multiple_of(31)
        || idat[1] & 0x20 != 0
    {
        return Err(bad("bad PNG data header"));
    }
    let mut raw = Vec::with_capacity(expected);
    Inflate::new(&idat[2..])
        .take(expected as u64 + 1)
        .read_to_end(&mut raw)?;
    if raw.len() != expected {
        return Err(bad("the PNG image data has the wrong size"));
    }
    // Undo the filters in place.
    let mut prev_start: Option<usize> = None;
    for y in 0..h as usize {
        let row = y * (stride + 1);
        let filter = raw[row];
        let cur = row + 1;
        for x in 0..stride {
            let a = if x >= bpp { raw[cur + x - bpp] } else { 0 };
            let b = prev_start.map_or(0, |p| raw[p + x]);
            let c = if x >= bpp {
                prev_start.map_or(0, |p| raw[p + x - bpp])
            } else {
                0
            };
            let v = raw[cur + x];
            raw[cur + x] = match filter {
                0 => v,
                1 => v.wrapping_add(a),
                2 => v.wrapping_add(b),
                3 => v.wrapping_add(((a as u16 + b as u16) / 2) as u8),
                4 => v.wrapping_add(paeth(a, b, c)),
                _ => return Err(bad("bad PNG filter")),
            };
        }
        prev_start = Some(cur);
    }
    // Expand to RGBA.
    let key16 = |i: usize| -> Option<u16> {
        trns.get(i * 2..i * 2 + 2)
            .map(|s| u16::from_be_bytes([s[0], s[1]]))
    };
    let mut pixels = Vec::with_capacity(w as usize * h as usize * 4);
    for y in 0..h as usize {
        let row = &raw[y * (stride + 1) + 1..(y + 1) * (stride + 1)];
        for x in 0..w as usize {
            // Sample `i` of this pixel at full precision, plus as 8 bits.
            let sample = |i: usize| -> (u16, u8) {
                match depth {
                    16 => {
                        let o = (x * channels + i) * 2;
                        let v = u16::from_be_bytes([row[o], row[o + 1]]);
                        (v, (v >> 8) as u8)
                    }
                    8 => {
                        let v = row[x * channels + i];
                        (v as u16, v)
                    }
                    d => {
                        let bit = x * d as usize;
                        let v = (row[bit / 8] >> (8 - d as usize - bit % 8)) & ((1 << d) - 1);
                        // Scale grey up to 8 bits (palette indexes stay as they are).
                        let scaled = if color == 0 {
                            v * (255 / ((1 << d) - 1))
                        } else {
                            v
                        };
                        (v as u16, scaled)
                    }
                }
            };
            let px: [u8; 4] = match color {
                0 => {
                    let (v, g) = sample(0);
                    let a = if key16(0) == Some(v) { 0 } else { 255 };
                    [g, g, g, a]
                }
                2 => {
                    let (r, r8) = sample(0);
                    let (g, g8) = sample(1);
                    let (b, b8) = sample(2);
                    let a = if key16(0) == Some(r) && key16(1) == Some(g) && key16(2) == Some(b) {
                        0
                    } else {
                        255
                    };
                    [r8, g8, b8, a]
                }
                3 => *palette
                    .get(sample(0).1 as usize)
                    .ok_or_else(|| bad("PNG palette index out of range"))?,
                4 => {
                    let g = sample(0).1;
                    [g, g, g, sample(1).1]
                }
                _ => [sample(0).1, sample(1).1, sample(2).1, sample(3).1],
            };
            pixels.extend_from_slice(&px);
        }
    }
    Ok(Rgba {
        width: w,
        height: h,
        pixels,
    })
}

fn chunk(out: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    out.extend_from_slice(&(data.len() as u32).to_be_bytes());
    out.extend_from_slice(kind);
    out.extend_from_slice(data);
    let mut c = Crc32::default();
    c.update(kind);
    c.update(data);
    out.extend_from_slice(&c.value().to_be_bytes());
}

/// Encode 8-bit RGBA pixels as a PNG; None when the sizes don't agree.
pub fn encode_rgba(width: u32, height: u32, pixels: &[u8]) -> Option<Vec<u8>> {
    let stride = width as usize * 4;
    if width == 0 || height == 0 || pixels.len() != stride * height as usize {
        return None;
    }
    // Filter each row with whichever filter gives the smallest sum of
    // absolute values (the usual heuristic).
    let mut filtered = Vec::with_capacity((stride + 1) * height as usize);
    let mut trial = vec![0u8; stride];
    let mut best = vec![0u8; stride];
    let zero = vec![0u8; stride];
    for y in 0..height as usize {
        let cur = &pixels[y * stride..(y + 1) * stride];
        let up = if y > 0 {
            &pixels[(y - 1) * stride..y * stride]
        } else {
            &zero[..]
        };
        let mut best_kind = 0u8;
        let mut best_cost = u64::MAX;
        for kind in 0..5u8 {
            let mut cost = 0u64;
            for x in 0..stride {
                let a = if x >= 4 { cur[x - 4] } else { 0 };
                let b = up[x];
                let c = if x >= 4 { up[x - 4] } else { 0 };
                let v = cur[x].wrapping_sub(match kind {
                    0 => 0,
                    1 => a,
                    2 => b,
                    3 => ((a as u16 + b as u16) / 2) as u8,
                    _ => paeth(a, b, c),
                });
                trial[x] = v;
                cost += (v as i8).unsigned_abs() as u64;
            }
            if cost < best_cost {
                best_cost = cost;
                best_kind = kind;
                std::mem::swap(&mut best, &mut trial);
            }
        }
        filtered.push(best_kind);
        filtered.extend_from_slice(&best);
    }
    let mut z = vec![0x78, 0x9C];
    let mut d = Deflate::new(z);
    d.write_all(&filtered).ok()?;
    z = d.finish().ok()?;
    z.extend_from_slice(&adler32(&filtered).to_be_bytes());

    let mut out = Vec::with_capacity(z.len() + 64);
    out.extend_from_slice(&SIGNATURE);
    let mut ihdr = [0u8; 13];
    ihdr[0..4].copy_from_slice(&width.to_be_bytes());
    ihdr[4..8].copy_from_slice(&height.to_be_bytes());
    ihdr[8] = 8; // bit depth
    ihdr[9] = 6; // RGBA
    chunk(&mut out, b"IHDR", &ihdr);
    chunk(&mut out, b"IDAT", &z);
    chunk(&mut out, b"IEND", &[]);
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(w: u32, h: u32) -> Vec<u8> {
        let mut p = Vec::new();
        for y in 0..h {
            for x in 0..w {
                p.extend_from_slice(&[
                    (x * 255 / w) as u8,
                    (y * 255 / h) as u8,
                    ((x ^ y) & 0xFF) as u8,
                    (255 - x % 256) as u8,
                ]);
            }
        }
        p
    }

    #[test]
    fn round_trip() {
        for (w, h) in [(1, 1), (16, 16), (37, 5), (256, 256)] {
            let p = gradient(w, h);
            let png = encode_rgba(w, h, &p).unwrap();
            let back = decode(&png).unwrap();
            assert_eq!((back.width, back.height), (w, h));
            assert_eq!(back.pixels, p);
        }
        assert!(encode_rgba(2, 2, &[0; 15]).is_none());
    }

    #[test]
    fn decodes_the_bundled_icons() {
        for bytes in [
            &include_bytes!("../assets/icon-taskbar.png")[..],
            &include_bytes!("../assets/icon-256.png")[..],
        ] {
            let img = decode(bytes).unwrap();
            assert_eq!(img.pixels.len(), (img.width * img.height * 4) as usize);
            // Re-encoding gives the same pixels.
            let again = decode(&encode_rgba(img.width, img.height, &img.pixels).unwrap()).unwrap();
            assert_eq!(again, img);
        }
    }

    #[test]
    fn decodes_every_color_type_and_depth() {
        // Made with Python (zlib + struct): each file's pixels as RGBA next
        // to it, covering grey 1/2/4/8/16, RGB 8/16 with tRNS, palette
        // 1/2/4/8 with tRNS, grey+alpha 8/16 and RGBA 8/16.
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/testdata/png");
        let mut n = 0;
        for e in std::fs::read_dir(&dir).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "png") {
                let img = decode(&std::fs::read(&p).unwrap()).unwrap();
                let want = std::fs::read(p.with_extension("rgba")).unwrap();
                assert_eq!(img.pixels, want, "{}", p.display());
                n += 1;
            }
        }
        assert!(n >= 15, "{n}");
    }

    #[test]
    fn our_files_open_elsewhere() {
        let Some(dir) = std::env::var_os("SETUPSWEEP_PNG_DUMP") else {
            return;
        };
        let d = std::path::Path::new(&dir);
        for name in ["icon-taskbar", "icon-256"] {
            let src =
                std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("assets/{name}.png"));
            std::fs::write(
                d.join(format!("{name}.rgba")),
                decode(&std::fs::read(src).unwrap()).unwrap().pixels,
            )
            .unwrap();
        }
        let p = gradient(64, 48);
        std::fs::write(
            std::path::Path::new(&dir).join("ours.png"),
            encode_rgba(64, 48, &p).unwrap(),
        )
        .unwrap();
        std::fs::write(std::path::Path::new(&dir).join("ours.rgba"), p).unwrap();
    }

    #[test]
    fn damaged_files_are_refused() {
        let png = encode_rgba(8, 8, &gradient(8, 8)).unwrap();
        assert!(decode(&png[..png.len() - 20]).is_err());
        let mut bad = png.clone();
        bad[40] ^= 1;
        assert!(decode(&bad).is_err());
        assert!(decode(b"GIF89a").is_err());
        assert_eq!(adler32(b"Wikipedia"), 0x11E6_0398);
    }
}
