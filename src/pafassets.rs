//! Bounded PNG import and PAF icon variants. Independent MIT implementation.
//! Format reference: https://portableapps.com/development/portableapps.com_format
//! DIB reference: https://learn.microsoft.com/windows/win32/api/wingdi/ns-wingdi-bitmapinfoheader
use crate::png::{self, Rgba};
use anyhow::{anyhow, Result};
use std::io::Read;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct IconAssets {
    pub source_hash: [u8; 32],
    pub original_size: u32,
    pub preview: Rgba,
    pub files: Vec<(String, Vec<u8>)>,
}

/// Wizard limits: square non-animated PNG, 256..4096 px, encoded size <=16 MiB.
/// Every chunk is bounded and duplicate headers are rejected before decoding.
pub fn import(path: &Path, cancel: &dyn Fn() -> bool) -> Result<IconAssets> {
    if cancel() {
        return Err(anyhow!("Icon import cancelled"));
    }
    if !std::fs::metadata(path)?.is_file() {
        return Err(anyhow!("Choose a regular PNG file"));
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path)?
        .take(16 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    from_png(&bytes, cancel)
}
pub fn from_png(bytes: &[u8], cancel: &dyn Fn() -> bool) -> Result<IconAssets> {
    if cancel() {
        return Err(anyhow!("Icon import cancelled"));
    }
    if bytes.len() > 16 * 1024 * 1024
        || bytes.len() < 33
        || &bytes[..8] != b"\x89PNG\r\n\x1a\n"
        || &bytes[12..16] != b"IHDR"
        || bytes[8..12] != 13u32.to_be_bytes()
    {
        return Err(anyhow!("Choose a PNG image no larger than 16 MiB; ICO, JPEG and SVG import are not supported yet"));
    }
    let w = u32::from_be_bytes(bytes[16..20].try_into()?);
    let h = u32::from_be_bytes(bytes[20..24].try_into()?);
    if w != h || !(256..=4096).contains(&w) {
        return Err(anyhow!("Use a square PNG from 256×256 to 4096×4096 pixels. Crop or resize a copy in your image editor first."));
    }
    let mut at = 8usize;
    let mut header = false;
    let mut ended = false;
    while at < bytes.len() {
        let len_bytes = bytes
            .get(at..at + 4)
            .ok_or_else(|| anyhow!("Truncated PNG chunk"))?;
        let n = u32::from_be_bytes(len_bytes.try_into()?) as usize;
        let end = at
            .checked_add(n)
            .and_then(|x| x.checked_add(12))
            .filter(|&x| x <= bytes.len())
            .ok_or_else(|| anyhow!("Truncated PNG chunk"))?;
        let kind = &bytes[at + 4..at + 8];
        if !kind.iter().all(u8::is_ascii_alphabetic) || kind[2] & 32 != 0 {
            return Err(anyhow!("Invalid PNG chunk type"));
        }
        if kind == b"IHDR" {
            if header {
                return Err(anyhow!("Duplicate PNG header"));
            }
            header = true;
        }
        if kind == b"acTL" {
            return Err(anyhow!(
                "Animated PNG icons are not supported; export a static PNG"
            ));
        }
        if kind[0] & 32 == 0 && ![b"IHDR", b"PLTE", b"IDAT", b"IEND"].contains(&kind.try_into()?) {
            return Err(anyhow!("Unsupported critical PNG chunk"));
        }
        if kind == b"IEND" {
            if n != 0 || end != bytes.len() {
                return Err(anyhow!("Invalid PNG end chunk"));
            }
            ended = true;
        }
        at = end;
    }
    if !ended {
        return Err(anyhow!("PNG end chunk is missing"));
    }
    let original = png::decode(bytes)?;
    let mut files = Vec::new();
    let mut frames = Vec::new();
    let mut preview = None;
    for size in [16u32, 32, 48, 75, 128, 256] {
        let image = resize(&original, size, cancel)?;
        let png = png::encode_rgba(size, size, &image.pixels)
            .ok_or_else(|| anyhow!("Could not encode icon"))?;
        if size != 48 {
            files.push((format!("appicon_{size}.png"), png.clone()));
        }
        if size <= 48 {
            for depth in [32u16, 8] {
                frames.push((size, depth, dib(&image, depth)));
            }
        }
        if size == 128 {
            preview = Some(image);
        }
        if size == 256 {
            frames.push((256, 32, png));
        }
    }
    let mut ico = vec![0, 0, 1, 0];
    ico.extend_from_slice(&(frames.len() as u16).to_le_bytes());
    let mut offset = 6 + frames.len() * 16;
    for (size, depth, data) in &frames {
        let size = if *size == 256 { 0 } else { *size as u8 };
        ico.extend_from_slice(&[size, size, 0, 0]);
        ico.extend_from_slice(&1u16.to_le_bytes());
        ico.extend_from_slice(&depth.to_le_bytes());
        ico.extend_from_slice(&(data.len() as u32).to_le_bytes());
        ico.extend_from_slice(&(offset as u32).to_le_bytes());
        offset += data.len();
    }
    for (_, _, data) in frames {
        ico.extend(data);
    }
    files.push(("appicon.ico".into(), ico));
    Ok(IconAssets {
        source_hash: crate::util::sha256(bytes),
        original_size: w,
        preview: preview.unwrap(),
        files,
    })
}

/// Area-weighted downsampling with premultiplied alpha avoids dark edge halos.
fn resize(input: &Rgba, size: u32, cancel: &dyn Fn() -> bool) -> Result<Rgba> {
    let scale = input.width as f64 / size as f64;
    let mut pixels = Vec::with_capacity(size as usize * size as usize * 4);
    for y in 0..size {
        if cancel() {
            return Err(anyhow!("Icon conversion cancelled"));
        }
        let (top, bottom) = (y as f64 * scale, (y + 1) as f64 * scale);
        for x in 0..size {
            let (left, right) = (x as f64 * scale, (x + 1) as f64 * scale);
            let mut sum = [0f64; 4];
            let mut area = 0f64;
            for sy in top.floor() as u32..bottom.ceil().min(input.height as f64) as u32 {
                for sx in left.floor() as u32..right.ceil().min(input.width as f64) as u32 {
                    let weight = (right.min((sx + 1) as f64) - left.max(sx as f64))
                        * (bottom.min((sy + 1) as f64) - top.max(sy as f64));
                    let i = ((sy * input.width + sx) * 4) as usize;
                    let alpha = input.pixels[i + 3] as f64;
                    for (c, value) in sum[..3].iter_mut().enumerate() {
                        *value += input.pixels[i + c] as f64 * alpha * weight;
                    }
                    sum[3] += alpha * weight;
                    area += weight;
                }
            }
            for value in &sum[..3] {
                pixels.push(if sum[3] > 0.0 {
                    (value / sum[3]).round() as u8
                } else {
                    0
                });
            }
            pixels.push((sum[3] / area).round() as u8);
        }
    }
    Ok(Rgba {
        width: size,
        height: size,
        pixels,
    })
}

/// Bottom-up uncompressed DIB with XOR pixels followed by the one-bit AND mask.
/// Legacy 8-bit frames use an RGB332 palette and alpha<128 transparency; 32-bit
/// frames preserve alpha. Palette reduction affects only the compatibility frames.
fn dib(image: &Rgba, depth: u16) -> Vec<u8> {
    let n = image.width as usize;
    let stride = (n * depth as usize).div_ceil(32) * 4;
    let mask_stride = n.div_ceil(32) * 4;
    let mut out = Vec::new();
    out.extend_from_slice(&40u32.to_le_bytes());
    out.extend_from_slice(&(n as i32).to_le_bytes());
    out.extend_from_slice(&((n * 2) as i32).to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&depth.to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    out.extend_from_slice(&((stride + mask_stride) * n).to_le_bytes()[..4]);
    out.extend_from_slice(&[0; 8]);
    out.extend_from_slice(&(if depth == 8 { 256u32 } else { 0 }).to_le_bytes());
    out.extend_from_slice(&0u32.to_le_bytes());
    if depth == 8 {
        for i in 0..256u32 {
            out.extend_from_slice(&[
                ((i & 3) * 255 / 3) as u8,
                (((i >> 2) & 7) * 255 / 7) as u8,
                (((i >> 5) & 7) * 255 / 7) as u8,
                0,
            ]);
        }
    }
    for y in (0..n).rev() {
        let start = out.len();
        for x in 0..n {
            let p = &image.pixels[(y * n + x) * 4..][..4];
            if depth == 8 {
                out.push((p[0] & 0xe0) | ((p[1] >> 3) & 0x1c) | (p[2] >> 6));
            } else {
                out.extend_from_slice(&[p[2], p[1], p[0], p[3]]);
            }
        }
        out.resize(start + stride, 0);
    }
    for y in (0..n).rev() {
        let start = out.len();
        out.resize(start + mask_stride, 0);
        for x in 0..n {
            let alpha = image.pixels[(y * n + x) * 4 + 3];
            if (depth == 8 && alpha < 128) || (depth == 32 && alpha == 0) {
                out[start + x / 8] |= 0x80 >> (x % 8);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    fn png() -> Vec<u8> {
        let mut pixels = vec![0u8; 256 * 256 * 4];
        for p in pixels.as_chunks_mut::<4>().0 {
            p.copy_from_slice(&[230, 30, 60, 128]);
        }
        png::encode_rgba(256, 256, &pixels).unwrap()
    }
    #[test]
    fn variants_have_all_sizes_depths_and_valid_pngs() {
        let assets = from_png(&png(), &|| false).unwrap();
        assert_eq!(assets.files.len(), 6);
        for size in [16, 32, 75, 128, 256] {
            let bytes = &assets
                .files
                .iter()
                .find(|(n, _)| n == &format!("appicon_{size}.png"))
                .unwrap()
                .1;
            let image = png::decode(bytes).unwrap();
            assert_eq!((image.width, image.height), (size, size));
            assert_eq!(&image.pixels[..4], &[230, 30, 60, 128]);
        }
        let ico = &assets.files.last().unwrap().1;
        assert_eq!(&ico[..6], &[0, 0, 1, 0, 7, 0]);
        let expected = [
            (16, 32),
            (16, 8),
            (32, 32),
            (32, 8),
            (48, 32),
            (48, 8),
            (0, 32),
        ];
        for (i, (size, depth)) in expected.into_iter().enumerate() {
            let entry = &ico[6 + i * 16..6 + (i + 1) * 16];
            assert_eq!(entry[0], size);
            assert_eq!(u16::from_le_bytes(entry[6..8].try_into().unwrap()), depth);
            let len = u32::from_le_bytes(entry[8..12].try_into().unwrap()) as usize;
            let at = u32::from_le_bytes(entry[12..16].try_into().unwrap()) as usize;
            assert!(at + len <= ico.len());
        }
        if let Ok(dir) = std::env::var("PAF_ICON_TEST_OUTPUT") {
            std::fs::create_dir_all(&dir).unwrap();
            for (name, bytes) in &assets.files {
                std::fs::write(Path::new(&dir).join(name), bytes).unwrap();
            }
        }
    }
    #[cfg(windows)]
    #[test]
    fn windows_loads_all_icon_sizes_with_alpha() {
        let assets = from_png(&png(), &|| false).unwrap();
        let temp = crate::util::tempdir().unwrap();
        let path = temp.path().join("appicon.ico");
        std::fs::write(&path, &assets.files.last().unwrap().1).unwrap();
        for size in [16, 32, 48, 256] {
            let (w, h, pixels) = crate::platform::icon_rgba(&path.to_string_lossy(), 0, size)
                .expect("Windows must load the generated icon");
            assert_eq!((w, h), (size, size));
            assert_eq!(pixels[3], 128, "32-bit icon alpha for size {size}");
            assert!(pixels[0] > pixels[1]);
        }
    }
    #[test]
    fn cancellation_and_malformed_images_fail() {
        assert!(from_png(&png(), &|| true).is_err());
        assert!(from_png(b"not PNG", &|| false).is_err());
        let bytes = png();
        assert!(from_png(&bytes[..bytes.len() - 12], &|| false).is_err());
        let mut duplicate = bytes[..33].to_vec();
        duplicate.extend_from_slice(&bytes[8..]);
        assert!(from_png(&duplicate, &|| false).is_err());
        let mut oversized = bytes.clone();
        oversized[16..20].copy_from_slice(&8192u32.to_be_bytes());
        assert!(from_png(&oversized, &|| false).is_err());
    }
    #[test]
    fn invalid_zlib_checksum_is_rejected_even_with_valid_chunk_crc() {
        let mut bytes = png();
        let mut at = 8;
        while at < bytes.len() {
            let n = u32::from_be_bytes(bytes[at..at + 4].try_into().unwrap()) as usize;
            if &bytes[at + 4..at + 8] == b"IDAT" {
                bytes[at + 8 + n - 1] ^= 1;
                let crc = crate::deflate::crc32(&bytes[at + 4..at + 8 + n]);
                bytes[at + 8 + n..at + 12 + n].copy_from_slice(&crc.to_be_bytes());
                break;
            }
            at += n + 12;
        }
        assert!(from_png(&bytes, &|| false)
            .unwrap_err()
            .to_string()
            .contains("Adler-32"));
    }
    #[test]
    fn alpha_resampling_does_not_add_dark_halos() {
        let image = Rgba {
            width: 2,
            height: 2,
            pixels: vec![255, 0, 0, 255, 0, 0, 0, 0, 255, 0, 0, 255, 0, 0, 0, 0],
        };
        assert_eq!(
            resize(&image, 1, &|| false).unwrap().pixels,
            vec![255, 0, 0, 128]
        );
    }
}
