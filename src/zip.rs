//! ZIP archives (PKWARE APPNOTE 6.3) without a library: stored and
//! deflated entries, ZIP64 for big files and many entries.
//!
//! [`ZipArchive`] reads from any `Read + Seek`; every entry's size and
//! CRC-32 are checked as it is read, and it can never produce more than its
//! declared size. [`ZipWriter`] writes deflated entries to any
//! `Write + Seek`, filling in sizes after the data.

use crate::deflate::{Crc32, Deflate, Inflate};
use std::collections::HashMap;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Component, Path, PathBuf};

const LOCAL: u32 = 0x0403_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const EOCD: u32 = 0x0605_4b50;
const EOCD64: u32 = 0x0606_4b50;
const EOCD64_LOC: u32 = 0x0706_4b50;
const ZIP64_EXTRA: u16 = 0x0001;
/// 1980-01-01 00:00 in DOS form; entries carry no meaningful time.
const DOS_DATE: u16 = (1 << 5) | 1;

fn bad(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}
fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(b[at..at + 4].try_into().unwrap())
}
fn u64_at(b: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(b[at..at + 8].try_into().unwrap())
}

/// Code page 437 for bytes 0x80-0xFF, used by names without the UTF-8 flag.
const CP437: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}";

fn decode_name(raw: &[u8], utf8: bool) -> String {
    if utf8 || raw.is_ascii() {
        return String::from_utf8_lossy(raw).into_owned();
    }
    raw.iter()
        .map(|&b| {
            if b < 0x80 {
                b as char
            } else {
                CP437.chars().nth(b as usize - 0x80).unwrap_or('?')
            }
        })
        .collect()
}

/// One entry from the central directory.
#[derive(Clone, Debug)]
pub struct Entry {
    pub name: String,
    pub method: u16,
    pub crc: u32,
    pub compressed: u64,
    pub size: u64,
    offset: u64,
    encrypted: bool,
}

impl Entry {
    pub fn is_dir(&self) -> bool {
        self.name.ends_with('/') || self.name.ends_with('\\')
    }

    /// The name as a relative path, or None when it could escape the
    /// folder it is extracted into (absolute, drive letter, `..`).
    pub fn enclosed_name(&self) -> Option<PathBuf> {
        if self.name.contains('\0') {
            return None;
        }
        let name = self.name.replace('\\', "/");
        let p = Path::new(&name);
        let mut out = PathBuf::new();
        for c in p.components() {
            match c {
                Component::Normal(s) => {
                    let s = s.to_string_lossy();
                    // "C:" inside a name would be a drive on Windows.
                    if s.contains(':') {
                        return None;
                    }
                    out.push(&*s);
                }
                Component::CurDir => {}
                _ => return None,
            }
        }
        Some(out)
    }
}

/// A ZIP file opened for reading.
pub struct ZipArchive<R: Read + Seek> {
    src: R,
    entries: Vec<Entry>,
    index: HashMap<String, usize>,
}

impl<R: Read + Seek> ZipArchive<R> {
    pub fn new(mut src: R) -> io::Result<Self> {
        let len = src.seek(SeekFrom::End(0))?;
        // No end record can fit in a shorter input. Guard before scanning:
        // saturating_sub alone would still inspect offset zero in an empty slice.
        if len < 22 {
            return Err(bad("not a ZIP file: end record is missing"));
        }
        // The end record is 22 bytes plus a comment of up to 64 KB.
        let tail_len = len.min(22 + 65_535) as usize;
        let mut tail = vec![0u8; tail_len];
        src.seek(SeekFrom::Start(len - tail_len as u64))?;
        src.read_exact(&mut tail)?;
        let at = (0..=tail_len.saturating_sub(22))
            .rev()
            .find(|&i| {
                u32_at(&tail, i) == EOCD && i + 22 + u16_at(&tail, i + 20) as usize <= tail_len
            })
            .ok_or_else(|| bad("not a ZIP file"))?;
        let eocd_pos = len - (tail_len - at) as u64;
        let mut count = u16_at(&tail, at + 10) as u64;
        let mut cd_size = u32_at(&tail, at + 12) as u64;
        let mut cd_off = u32_at(&tail, at + 16) as u64;
        // ZIP64 end record, found through its locator just before.
        if eocd_pos >= 20 {
            let mut loc = [0u8; 20];
            src.seek(SeekFrom::Start(eocd_pos - 20))?;
            src.read_exact(&mut loc)?;
            if u32_at(&loc, 0) == EOCD64_LOC {
                let rec = u64_at(&loc, 8);
                let mut e = [0u8; 56];
                src.seek(SeekFrom::Start(rec))?;
                src.read_exact(&mut e)?;
                if u32_at(&e, 0) != EOCD64 {
                    return Err(bad("damaged ZIP64 end record"));
                }
                count = u64_at(&e, 32);
                cd_size = u64_at(&e, 40);
                cd_off = u64_at(&e, 48);
            }
        }
        if cd_off.checked_add(cd_size).is_none_or(|e| e > len) {
            return Err(bad("the ZIP directory is outside the file"));
        }
        // Each entry takes at least 46 bytes, which caps `count`.
        if count > cd_size / 46 + 1 {
            return Err(bad("damaged ZIP directory"));
        }
        let mut cd = vec![0u8; cd_size as usize];
        src.seek(SeekFrom::Start(cd_off))?;
        src.read_exact(&mut cd)?;
        let mut entries = Vec::with_capacity(count as usize);
        let mut p = 0usize;
        for _ in 0..count {
            if p + 46 > cd.len() || u32_at(&cd, p) != CENTRAL {
                return Err(bad("damaged ZIP directory"));
            }
            let flags = u16_at(&cd, p + 8);
            let method = u16_at(&cd, p + 10);
            let crc = u32_at(&cd, p + 16);
            let mut compressed = u32_at(&cd, p + 20) as u64;
            let mut size = u32_at(&cd, p + 24) as u64;
            let nlen = u16_at(&cd, p + 28) as usize;
            let xlen = u16_at(&cd, p + 30) as usize;
            let clen = u16_at(&cd, p + 32) as usize;
            let mut offset = u32_at(&cd, p + 42) as u64;
            let end = p + 46 + nlen + xlen + clen;
            if end > cd.len() {
                return Err(bad("damaged ZIP directory"));
            }
            let name = decode_name(&cd[p + 46..p + 46 + nlen], flags & 0x800 != 0);
            // ZIP64 extra: only the fields whose 32-bit value is all ones.
            let mut x = p + 46 + nlen;
            let xend = x + xlen;
            while x + 4 <= xend {
                let id = u16_at(&cd, x);
                let n = u16_at(&cd, x + 2) as usize;
                let body = x + 4;
                if body + n > xend {
                    break;
                }
                if id == ZIP64_EXTRA {
                    let mut f = body;
                    for v in [&mut size, &mut compressed, &mut offset] {
                        if *v == 0xFFFF_FFFF && f + 8 <= body + n {
                            *v = u64_at(&cd, f);
                            f += 8;
                        }
                    }
                }
                x = body + n;
            }
            entries.push(Entry {
                name,
                method,
                crc,
                compressed,
                size,
                offset,
                encrypted: flags & 1 != 0,
            });
            p = end;
        }
        let mut index = HashMap::with_capacity(entries.len());
        for (i, e) in entries.iter().enumerate() {
            index.entry(e.name.clone()).or_insert(i);
        }
        Ok(ZipArchive {
            src,
            entries,
            index,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entry(&self, i: usize) -> &Entry {
        &self.entries[i]
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn by_name(&mut self, name: &str) -> io::Result<ZipFile<'_, R>> {
        let i = *self.index.get(name).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                format!("{name} isn't in the archive"),
            )
        })?;
        self.by_index(i)
    }

    /// Open entry `i` for reading.
    pub fn by_index(&mut self, i: usize) -> io::Result<ZipFile<'_, R>> {
        let e = self.entries[i].clone();
        if e.encrypted {
            return Err(bad("encrypted entries aren't supported"));
        }
        let mut h = [0u8; 30];
        self.src.seek(SeekFrom::Start(e.offset))?;
        self.src.read_exact(&mut h)?;
        if u32_at(&h, 0) != LOCAL {
            return Err(bad("damaged ZIP entry"));
        }
        let skip = u16_at(&h, 26) as i64 + u16_at(&h, 28) as i64;
        self.src.seek(SeekFrom::Current(skip))?;
        let raw = (&mut self.src).take(e.compressed);
        let body = match e.method {
            0 => {
                if e.compressed != e.size {
                    return Err(bad("damaged ZIP entry"));
                }
                Body::Stored(raw)
            }
            8 => Body::Deflated(Box::new(Inflate::new(raw))),
            m => return Err(bad(&format!("compression method {m} isn't supported"))),
        };
        Ok(ZipFile {
            entry: e,
            body,
            crc: Crc32::default(),
            read: 0,
        })
    }
}

enum Body<'a, R: Read> {
    Stored(io::Take<&'a mut R>),
    Deflated(Box<Inflate<io::Take<&'a mut R>>>),
}

/// An entry being read; checks size and CRC-32 at the end.
pub struct ZipFile<'a, R: Read> {
    entry: Entry,
    body: Body<'a, R>,
    crc: Crc32,
    read: u64,
}

impl<R: Read> ZipFile<'_, R> {
    pub fn entry(&self) -> &Entry {
        &self.entry
    }
    pub fn is_dir(&self) -> bool {
        self.entry.is_dir()
    }
    pub fn enclosed_name(&self) -> Option<PathBuf> {
        self.entry.enclosed_name()
    }
    pub fn name(&self) -> &str {
        &self.entry.name
    }
}

impl<R: Read> Read for ZipFile<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        // Never more than the declared size (+1 to notice an overrun).
        let left = (self.entry.size - self.read).saturating_add(1);
        let cap = buf.len().min(left.min(usize::MAX as u64) as usize);
        let n = match &mut self.body {
            Body::Stored(r) => r.read(&mut buf[..cap])?,
            Body::Deflated(r) => r.read(&mut buf[..cap])?,
        };
        self.read += n as u64;
        if self.read > self.entry.size {
            return Err(bad("a ZIP entry is larger than it says"));
        }
        self.crc.update(&buf[..n]);
        if n == 0 && (self.read != self.entry.size || self.crc.value() != self.entry.crc) {
            return Err(bad("a ZIP entry is damaged (checksum mismatch)"));
        }
        Ok(n)
    }
}

// ---------------------------------------------------------------------------
// Writing
// ---------------------------------------------------------------------------

/// Counts bytes written through it.
struct Counter<W: Write> {
    w: W,
    n: u64,
}

impl<W: Write> Write for Counter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.w.write(buf)?;
        self.n += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.w.flush()
    }
}

struct Written {
    name: String,
    crc: u32,
    compressed: u64,
    size: u64,
    offset: u64,
}

struct Open {
    name: String,
    offset: u64,
    data_start: u64,
    crc: Crc32,
    size: u64,
}

/// Writes a ZIP file. Every entry is deflated (the encoder falls back to
/// stored blocks for data that doesn't shrink) and carries a ZIP64 field,
/// so no size limits apply.
pub struct ZipWriter<W: Write + Seek> {
    out: Deflate<Counter<W>>,
    done: Vec<Written>,
    open: Option<Open>,
    header: Vec<u8>,
}

impl<W: Write + Seek> ZipWriter<W> {
    pub fn new(mut w: W) -> io::Result<Self> {
        let n = w.stream_position()?;
        Ok(ZipWriter {
            out: Deflate::new(Counter { w, n }),
            done: Vec::new(),
            open: None,
            header: Vec::with_capacity(256),
        })
    }

    /// Begin a new entry; later writes go into it.
    pub fn start_file(&mut self, name: &str) -> io::Result<()> {
        self.end_file()?;
        let offset = self.out.get_ref().n;
        let h = &mut self.header;
        h.clear();
        h.extend_from_slice(&LOCAL.to_le_bytes());
        h.extend_from_slice(&45u16.to_le_bytes()); // version needed: ZIP64
        h.extend_from_slice(&0x0800u16.to_le_bytes()); // UTF-8 names
        h.extend_from_slice(&8u16.to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        h.extend_from_slice(&DOS_DATE.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes()); // CRC, filled in later
        h.extend_from_slice(&u32::MAX.to_le_bytes());
        h.extend_from_slice(&u32::MAX.to_le_bytes());
        h.extend_from_slice(&(name.len() as u16).to_le_bytes());
        h.extend_from_slice(&20u16.to_le_bytes());
        h.extend_from_slice(name.as_bytes());
        h.extend_from_slice(&ZIP64_EXTRA.to_le_bytes());
        h.extend_from_slice(&16u16.to_le_bytes());
        h.extend_from_slice(&[0u8; 16]); // sizes, filled in later
        if name.len() > u16::MAX as usize {
            return Err(io::Error::new(io::ErrorKind::InvalidInput, "name too long"));
        }
        self.out.get_mut().write_all(&self.header)?;
        self.open = Some(Open {
            name: name.to_string(),
            offset,
            data_start: self.out.get_ref().n,
            crc: Crc32::default(),
            size: 0,
        });
        Ok(())
    }

    fn end_file(&mut self) -> io::Result<()> {
        let Some(o) = self.open.take() else {
            return Ok(());
        };
        self.out.end_stream()?;
        let end = self.out.get_ref().n;
        let compressed = end - o.data_start;
        let crc = o.crc.value();
        // Fill in the CRC and the ZIP64 sizes in the local header.
        let w = &mut self.out.get_mut().w;
        w.seek(SeekFrom::Start(o.offset + 14))?;
        w.write_all(&crc.to_le_bytes())?;
        w.seek(SeekFrom::Start(o.offset + 30 + o.name.len() as u64 + 4))?;
        w.write_all(&o.size.to_le_bytes())?;
        w.write_all(&compressed.to_le_bytes())?;
        w.seek(SeekFrom::Start(end))?;
        self.done.push(Written {
            name: o.name,
            crc,
            compressed,
            size: o.size,
            offset: o.offset,
        });
        Ok(())
    }

    /// Write the directory and return the output.
    pub fn finish(mut self) -> io::Result<W> {
        self.end_file()?;
        let cd_start = self.out.get_ref().n;
        let count = self.done.len() as u64;
        let mut h = std::mem::take(&mut self.header);
        for e in &self.done {
            h.clear();
            h.extend_from_slice(&CENTRAL.to_le_bytes());
            h.extend_from_slice(&45u16.to_le_bytes()); // made by: MS-DOS, 4.5
            h.extend_from_slice(&45u16.to_le_bytes());
            h.extend_from_slice(&0x0800u16.to_le_bytes());
            h.extend_from_slice(&8u16.to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            h.extend_from_slice(&DOS_DATE.to_le_bytes());
            h.extend_from_slice(&e.crc.to_le_bytes());
            h.extend_from_slice(&u32::MAX.to_le_bytes());
            h.extend_from_slice(&u32::MAX.to_le_bytes());
            h.extend_from_slice(&(e.name.len() as u16).to_le_bytes());
            h.extend_from_slice(&28u16.to_le_bytes());
            h.extend_from_slice(&[0u8; 6]); // comment, disk, internal attrs
            h.extend_from_slice(&0u32.to_le_bytes()); // external attrs
            h.extend_from_slice(&u32::MAX.to_le_bytes());
            h.extend_from_slice(e.name.as_bytes());
            h.extend_from_slice(&ZIP64_EXTRA.to_le_bytes());
            h.extend_from_slice(&24u16.to_le_bytes());
            h.extend_from_slice(&e.size.to_le_bytes());
            h.extend_from_slice(&e.compressed.to_le_bytes());
            h.extend_from_slice(&e.offset.to_le_bytes());
            self.out.get_mut().write_all(&h)?;
        }
        let cd_end = self.out.get_ref().n;
        let cd_size = cd_end - cd_start;
        h.clear();
        // ZIP64 end record and locator, then the classic end record.
        h.extend_from_slice(&EOCD64.to_le_bytes());
        h.extend_from_slice(&44u64.to_le_bytes());
        h.extend_from_slice(&45u16.to_le_bytes());
        h.extend_from_slice(&45u16.to_le_bytes());
        h.extend_from_slice(&[0u8; 8]); // disk numbers
        h.extend_from_slice(&count.to_le_bytes());
        h.extend_from_slice(&count.to_le_bytes());
        h.extend_from_slice(&cd_size.to_le_bytes());
        h.extend_from_slice(&cd_start.to_le_bytes());
        h.extend_from_slice(&EOCD64_LOC.to_le_bytes());
        h.extend_from_slice(&0u32.to_le_bytes());
        h.extend_from_slice(&cd_end.to_le_bytes());
        h.extend_from_slice(&1u32.to_le_bytes());
        h.extend_from_slice(&EOCD.to_le_bytes());
        h.extend_from_slice(&[0u8; 4]);
        let c16 = count.min(0xFFFF) as u16;
        h.extend_from_slice(&c16.to_le_bytes());
        h.extend_from_slice(&c16.to_le_bytes());
        h.extend_from_slice(&(cd_size.min(0xFFFF_FFFF) as u32).to_le_bytes());
        h.extend_from_slice(&(cd_start.min(0xFFFF_FFFF) as u32).to_le_bytes());
        h.extend_from_slice(&0u16.to_le_bytes());
        self.out.get_mut().write_all(&h)?;
        let mut w = self.out.into_inner().w;
        w.flush()?;
        Ok(w)
    }
}

impl<W: Write + Seek> Write for ZipWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let o = self
            .open
            .as_mut()
            .ok_or_else(|| io::Error::other("no entry started"))?;
        let n = self.out.write(buf)?;
        o.crc.update(&buf[..n]);
        o.size += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn build(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut z = ZipWriter::new(Cursor::new(Vec::new())).unwrap();
        for (name, data) in entries {
            z.start_file(name).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap().into_inner()
    }

    #[test]
    fn rejects_short_inputs_without_panicking() {
        for len in 0..22 {
            for byte in [0, b'P', 0xff] {
                assert!(ZipArchive::new(Cursor::new(vec![byte; len])).is_err());
            }
        }
    }

    #[test]
    fn rejects_every_truncated_prefix() {
        let zip = build(&[("example.txt", b"payload")]);
        for end in 0..zip.len() {
            assert!(
                ZipArchive::new(Cursor::new(&zip[..end])).is_err(),
                "prefix {end}"
            );
        }
        assert!(ZipArchive::new(Cursor::new(zip)).is_ok());
    }

    #[test]
    fn empty_archive_and_comment_signatures() {
        let empty = build(&[]);
        assert!(ZipArchive::new(Cursor::new(empty.clone()))
            .unwrap()
            .is_empty());
        // An end-record signature in the comment must not hide the real record.
        let mut commented = empty;
        commented[20..22].copy_from_slice(&4u16.to_le_bytes());
        commented.extend_from_slice(&EOCD.to_le_bytes());
        assert!(ZipArchive::new(Cursor::new(commented)).unwrap().is_empty());
    }

    #[test]
    fn writes_and_reads_back() {
        let big: Vec<u8> = (0..400_000u32).map(|i| (i * 7 / 3) as u8).collect();
        let zip = build(&[
            ("a.txt", b"hello"),
            ("dir/b.bin", &big),
            ("empty", b""),
            ("ü/ñ.txt", b"x"),
        ]);
        let mut a = ZipArchive::new(Cursor::new(zip)).unwrap();
        assert_eq!(a.len(), 4);
        let mut s = String::new();
        a.by_name("a.txt").unwrap().read_to_string(&mut s).unwrap();
        assert_eq!(s, "hello");
        let mut v = Vec::new();
        a.by_name("dir/b.bin").unwrap().read_to_end(&mut v).unwrap();
        assert_eq!(v, big);
        v.clear();
        a.by_index(2).unwrap().read_to_end(&mut v).unwrap();
        assert!(v.is_empty());
        assert_eq!(a.entry(3).name, "ü/ñ.txt");
        assert!(a.by_name("missing").is_err());
    }

    #[test]
    fn reads_archives_from_other_programs() {
        // Python's zipfile: stored and deflated entries, a folder entry,
        // and a CP437 name; then the same with ZIP64 records forced.
        for raw in [
            &include_bytes!("testdata/zip/python.zip")[..],
            &include_bytes!("testdata/zip/python64.zip")[..],
        ] {
            let mut a = ZipArchive::new(Cursor::new(raw)).unwrap();
            let names: Vec<_> = a.entries().iter().map(|e| e.name.clone()).collect();
            assert_eq!(
                names,
                ["top/", "top/stored.txt", "top/deflated.txt", "top/café.txt"]
            );
            assert!(a.entry(0).is_dir());
            for (i, name) in names.iter().enumerate().skip(1) {
                let mut v = Vec::new();
                a.by_index(i).unwrap().read_to_end(&mut v).unwrap();
                assert_eq!(v, include_bytes!("testdata/deflate/dynamic.txt"), "{name}");
            }
        }
    }

    #[test]
    fn our_archives_open_elsewhere() {
        // Checked against Python's zipfile through INSTALLSCOPE_ZIP_DUMP.
        let Some(dir) = std::env::var_os("INSTALLSCOPE_ZIP_DUMP") else {
            return;
        };
        let zip = build(&[
            ("files/0", include_bytes!("testdata/deflate/dynamic.txt")),
            ("files/1", b""),
            ("manifest.json", b"{}"),
        ]);
        std::fs::write(Path::new(&dir).join("ours.zip"), zip).unwrap();
    }

    #[test]
    fn damage_is_noticed() {
        let data = include_bytes!("testdata/deflate/dynamic.txt");
        let zip = build(&[("f", data)]);
        // A flipped byte inside the compressed data.
        let mut bad = zip.clone();
        bad[100] ^= 0x10;
        let mut a = ZipArchive::new(Cursor::new(bad)).unwrap();
        let mut v = Vec::new();
        assert!(a.by_index(0).unwrap().read_to_end(&mut v).is_err());
        // A size that is smaller than the data.
        let mut a = ZipArchive::new(Cursor::new(zip.clone())).unwrap();
        a.entries[0].size = 10;
        let mut v = Vec::new();
        assert!(a.by_index(0).unwrap().read_to_end(&mut v).is_err());
        assert!(v.len() <= 11);
        // Not a ZIP at all, and a cut-off one.
        assert!(ZipArchive::new(Cursor::new(b"hello".to_vec())).is_err());
        assert!(ZipArchive::new(Cursor::new(zip[..zip.len() / 2].to_vec())).is_err());
    }

    #[test]
    fn names_that_escape_are_refused() {
        let e = |n: &str| Entry {
            name: n.into(),
            method: 0,
            crc: 0,
            compressed: 0,
            size: 0,
            offset: 0,
            encrypted: false,
        };
        assert_eq!(
            e("a/b/c.txt").enclosed_name(),
            Some(PathBuf::from("a/b/c.txt"))
        );
        assert_eq!(e("./a").enclosed_name(), Some(PathBuf::from("a")));
        for n in [
            "../x",
            "a/../../x",
            "/etc/passwd",
            "C:/Windows/x",
            "a\\..\\..\\x",
            "c:x",
            "a\0b",
        ] {
            assert_eq!(e(n).enclosed_name(), None, "{n}");
        }
    }
}
