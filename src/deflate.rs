//! DEFLATE (RFC 1951), gzip (RFC 1952) and CRC-32 without a library.
//!
//! [`Inflate`] decodes any valid stream (stored, fixed and dynamic blocks)
//! as a `Read`, holding only a 32 KB window. [`Deflate`] encodes as a
//! `Write`: LZ77 with hash chains over a 32 KB window carried between
//! blocks, then dynamic Huffman codes limited to 15 bits (or a stored block
//! when that is smaller).

use std::io::{self, Read, Write};

// ---------------------------------------------------------------------------
// CRC-32 (IEEE, as zip and gzip use)
// ---------------------------------------------------------------------------

const fn crc_table() -> [u32; 256] {
    let mut t = [0u32; 256];
    let mut i = 0;
    while i < 256 {
        let mut c = i as u32;
        let mut k = 0;
        while k < 8 {
            c = if c & 1 != 0 {
                0xEDB8_8320 ^ (c >> 1)
            } else {
                c >> 1
            };
            k += 1;
        }
        t[i] = c;
        i += 1;
    }
    t
}
static CRC_TABLE: [u32; 256] = crc_table();

/// A running CRC-32.
#[derive(Clone, Copy, Debug)]
pub struct Crc32(u32);

impl Default for Crc32 {
    fn default() -> Self {
        Crc32(0xFFFF_FFFF)
    }
}

impl Crc32 {
    pub fn update(&mut self, data: &[u8]) {
        let mut c = self.0;
        for &b in data {
            c = CRC_TABLE[((c ^ b as u32) & 0xFF) as usize] ^ (c >> 8);
        }
        self.0 = c;
    }
    pub fn value(&self) -> u32 {
        !self.0
    }
}

pub fn crc32(data: &[u8]) -> u32 {
    let mut c = Crc32::default();
    c.update(data);
    c.value()
}

// ---------------------------------------------------------------------------
// Shared tables
// ---------------------------------------------------------------------------

const LEN_BASE: [u16; 29] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51, 59, 67, 83, 99, 115, 131,
    163, 195, 227, 258,
];
const LEN_EXTRA: [u8; 29] = [
    0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4, 5, 5, 5, 5, 0,
];
const DIST_BASE: [u16; 30] = [
    1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385, 513, 769, 1025, 1537,
    2049, 3073, 4097, 6145, 8193, 12289, 16385, 24577,
];
const DIST_EXTRA: [u8; 30] = [
    0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9, 10, 10, 11, 11, 12, 12, 13,
    13,
];
/// Order code-length code lengths are sent in.
const CL_ORDER: [usize; 19] = [
    16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
];

const WINDOW: usize = 32 * 1024;

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// Bits of lookahead resolved by one table lookup.
const FAST: u32 = 10;

/// A canonical Huffman code for decoding.
struct Decoder {
    /// Indexed by the next FAST bits: (symbol, length), length 0 = longer code.
    fast: Vec<(u16, u8)>,
    /// For codes longer than FAST: counts per length and symbols in order.
    count: [u16; 16],
    symbols: Vec<u16>,
}

impl Decoder {
    fn new(lengths: &[u8]) -> io::Result<Decoder> {
        let mut count = [0u16; 16];
        for &l in lengths {
            count[l as usize] += 1;
        }
        count[0] = 0;
        // Over-subscribed codes are invalid; incomplete ones are allowed
        // (a single distance code, for instance).
        let mut left: i32 = 1;
        for &c in &count[1..] {
            left = (left << 1) - c as i32;
            if left < 0 {
                return Err(invalid("bad Huffman code"));
            }
        }
        let mut offs = [0u16; 16];
        for l in 1..15 {
            offs[l + 1] = offs[l] + count[l];
        }
        let mut symbols = vec![0u16; lengths.len()];
        for (s, &l) in lengths.iter().enumerate() {
            if l != 0 {
                symbols[offs[l as usize] as usize] = s as u16;
                offs[l as usize] += 1;
            }
        }
        // Fast table: walk codes in canonical order.
        let mut fast = vec![(0u16, 0u8); 1 << FAST];
        let mut code: u32 = 0;
        let mut idx = 0usize;
        for len in 1..=15u32 {
            for _ in 0..count[len as usize] {
                if len <= FAST {
                    // Codes are sent MSB-first: reverse for LSB-first lookup.
                    let rev = code.reverse_bits() >> (32 - len);
                    let step = 1 << len;
                    let mut i = rev as usize;
                    while i < fast.len() {
                        fast[i] = (symbols[idx], len as u8);
                        i += step;
                    }
                }
                code += 1;
                idx += 1;
            }
            code <<= 1;
        }
        Ok(Decoder {
            fast,
            count,
            symbols,
        })
    }
}

enum State {
    Header,
    Stored(usize),
    Codes(Box<(Decoder, Decoder)>),
    Fixed,
    Done,
}

/// A DEFLATE decoder reading compressed data from `R`.
pub struct Inflate<R: Read> {
    src: R,
    inbuf: Box<[u8; 8192]>,
    inpos: usize,
    inlen: usize,
    bits: u64,
    nbits: u32,
    window: Box<[u8; WINDOW]>,
    wpos: usize,
    state: State,
    last: bool,
    /// A back-reference still being copied: (length left, distance).
    copy: (usize, usize),
    fixed: Option<Box<(Decoder, Decoder)>>,
}

impl<R: Read> Inflate<R> {
    pub fn new(src: R) -> Self {
        Inflate {
            src,
            inbuf: Box::new([0; 8192]),
            inpos: 0,
            inlen: 0,
            bits: 0,
            nbits: 0,
            window: Box::new([0; WINDOW]),
            wpos: 0,
            state: State::Header,
            last: false,
            copy: (0, 0),
            fixed: None,
        }
    }

    /// The bytes read from the source but not used (after the stream ends).
    pub fn into_parts(self) -> (R, Vec<u8>) {
        let mut rest = Vec::new();
        // Whole bytes still in the bit buffer, then the input buffer.
        // The low bits finish a byte already partly used; whole bytes follow.
        let mut bits = self.bits >> (self.nbits % 8);
        let mut n = self.nbits - self.nbits % 8;
        while n > 0 {
            rest.push(bits as u8);
            bits >>= 8;
            n -= 8;
        }
        rest.extend_from_slice(&self.inbuf[self.inpos..self.inlen]);
        (self.src, rest)
    }

    fn byte(&mut self) -> io::Result<Option<u8>> {
        if self.inpos == self.inlen {
            self.inlen = self.src.read(&mut self.inbuf[..])?;
            self.inpos = 0;
            if self.inlen == 0 {
                return Ok(None);
            }
        }
        let b = self.inbuf[self.inpos];
        self.inpos += 1;
        Ok(Some(b))
    }

    /// Make at least `n` bits available (fewer only at the end of input).
    fn need(&mut self, n: u32) -> io::Result<bool> {
        while self.nbits < n {
            match self.byte()? {
                Some(b) => {
                    self.bits |= (b as u64) << self.nbits;
                    self.nbits += 8;
                }
                None => return Ok(false),
            }
        }
        Ok(true)
    }

    fn take(&mut self, n: u32) -> io::Result<u32> {
        if n == 0 {
            return Ok(0);
        }
        if !self.need(n)? {
            return Err(invalid("compressed data ends early"));
        }
        let v = (self.bits & ((1u64 << n) - 1)) as u32;
        self.bits >>= n;
        self.nbits -= n;
        Ok(v)
    }

    fn decode(&mut self, d: &Decoder) -> io::Result<u16> {
        self.need(FAST)?;
        let (sym, len) = d.fast[(self.bits & ((1 << FAST) - 1)) as usize];
        if len != 0 && (len as u32) <= self.nbits {
            self.bits >>= len;
            self.nbits -= len as u32;
            return Ok(sym);
        }
        // Long (or near the end of input): canonical decode bit by bit.
        let (mut code, mut first, mut index) = (0i32, 0i32, 0i32);
        for len in 1..16 {
            code |= self.take(1)? as i32;
            let count = d.count[len] as i32;
            if code - count < first {
                return Ok(d.symbols[(index + (code - first)) as usize]);
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        Err(invalid("bad Huffman code"))
    }

    fn fixed_codes() -> io::Result<(Decoder, Decoder)> {
        let mut l = [0u8; 288];
        l[..144].fill(8);
        l[144..256].fill(9);
        l[256..280].fill(7);
        l[280..].fill(8);
        Ok((Decoder::new(&l)?, Decoder::new(&[5u8; 30])?))
    }

    fn dynamic_codes(&mut self) -> io::Result<(Decoder, Decoder)> {
        let nlen = self.take(5)? as usize + 257;
        let ndist = self.take(5)? as usize + 1;
        let ncode = self.take(4)? as usize + 4;
        if nlen > 286 || ndist > 30 {
            return Err(invalid("bad block header"));
        }
        let mut cl = [0u8; 19];
        for &i in CL_ORDER.iter().take(ncode) {
            cl[i] = self.take(3)? as u8;
        }
        let cld = Decoder::new(&cl)?;
        let mut lengths = [0u8; 316];
        let mut i = 0;
        while i < nlen + ndist {
            let sym = self.decode(&cld)?;
            let (val, rep) = match sym {
                0..=15 => (sym as u8, 1),
                16 => {
                    if i == 0 {
                        return Err(invalid("repeat with nothing before it"));
                    }
                    (lengths[i - 1], 3 + self.take(2)? as usize)
                }
                17 => (0, 3 + self.take(3)? as usize),
                _ => (0, 11 + self.take(7)? as usize),
            };
            if i + rep > nlen + ndist {
                return Err(invalid("too many code lengths"));
            }
            lengths[i..i + rep].fill(val);
            i += rep;
        }
        if lengths[256] == 0 {
            return Err(invalid("no end-of-block code"));
        }
        Ok((
            Decoder::new(&lengths[..nlen])?,
            Decoder::new(&lengths[nlen..nlen + ndist])?,
        ))
    }

    fn emit(&mut self, out: &mut [u8], n: &mut usize, b: u8) {
        out[*n] = b;
        *n += 1;
        self.window[self.wpos] = b;
        self.wpos = (self.wpos + 1) % WINDOW;
    }
}

impl<R: Read> Read for Inflate<R> {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        let mut n = 0;
        while n < out.len() {
            // Finish a back-reference first.
            while self.copy.0 > 0 && n < out.len() {
                let b = self.window[(self.wpos + WINDOW - self.copy.1) % WINDOW];
                self.emit(out, &mut n, b);
                self.copy.0 -= 1;
            }
            if n == out.len() {
                break;
            }
            match std::mem::replace(&mut self.state, State::Done) {
                State::Done => {
                    self.state = State::Done;
                    break;
                }
                State::Header => {
                    if self.last {
                        self.state = State::Done;
                        continue;
                    }
                    self.last = self.take(1)? == 1;
                    self.state = match self.take(2)? {
                        0 => {
                            // Stored: skip to a byte boundary.
                            let drop = self.nbits % 8;
                            self.bits >>= drop;
                            self.nbits -= drop;
                            let len = self.take(16)?;
                            let nlen = self.take(16)?;
                            if len != !nlen & 0xFFFF {
                                return Err(invalid("bad stored block length"));
                            }
                            State::Stored(len as usize)
                        }
                        1 => State::Fixed,
                        2 => State::Codes(Box::new(self.dynamic_codes()?)),
                        _ => return Err(invalid("bad block type")),
                    };
                }
                State::Stored(left) => {
                    if left == 0 {
                        self.state = State::Header;
                        continue;
                    }
                    let b = self.take(8)? as u8;
                    self.emit(out, &mut n, b);
                    self.state = State::Stored(left - 1);
                }
                State::Fixed => {
                    let codes = match self.fixed.take() {
                        Some(c) => c,
                        None => Box::new(Self::fixed_codes()?),
                    };
                    let end = self.codes(out, &mut n, &codes)?;
                    self.fixed = Some(codes);
                    self.state = if end { State::Header } else { State::Fixed };
                }
                State::Codes(codes) => {
                    let end = self.codes(out, &mut n, &codes)?;
                    self.state = if end {
                        State::Header
                    } else {
                        State::Codes(codes)
                    };
                }
            }
        }
        Ok(n)
    }
}

impl<R: Read> Inflate<R> {
    /// Decode symbols until `out` is full or the block ends (true).
    fn codes(&mut self, out: &mut [u8], n: &mut usize, c: &(Decoder, Decoder)) -> io::Result<bool> {
        while *n < out.len() {
            let sym = self.decode(&c.0)?;
            match sym {
                0..=255 => self.emit(out, n, sym as u8),
                256 => return Ok(true),
                257..=285 => {
                    let i = (sym - 257) as usize;
                    let len = LEN_BASE[i] as usize + self.take(LEN_EXTRA[i] as u32)? as usize;
                    let d = self.decode(&c.1)? as usize;
                    if d >= 30 {
                        return Err(invalid("bad distance code"));
                    }
                    let dist = DIST_BASE[d] as usize + self.take(DIST_EXTRA[d] as u32)? as usize;
                    self.copy = (len, dist);
                    while self.copy.0 > 0 && *n < out.len() {
                        let b = self.window[(self.wpos + WINDOW - dist) % WINDOW];
                        self.emit(out, n, b);
                        self.copy.0 -= 1;
                    }
                }
                _ => return Err(invalid("bad literal/length code")),
            }
        }
        Ok(false)
    }
}

/// Decompress a whole raw DEFLATE stream.
pub fn inflate(data: &[u8]) -> io::Result<Vec<u8>> {
    let mut out = Vec::with_capacity(data.len() * 3);
    Inflate::new(data).read_to_end(&mut out)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Encoding
// ---------------------------------------------------------------------------

struct BitWriter<W: Write> {
    out: W,
    bits: u64,
    nbits: u32,
    buf: Vec<u8>,
}

impl<W: Write> BitWriter<W> {
    fn put(&mut self, v: u32, n: u32) {
        self.bits |= (v as u64) << self.nbits;
        self.nbits += n;
        while self.nbits >= 8 {
            self.buf.push(self.bits as u8);
            self.bits >>= 8;
            self.nbits -= 8;
        }
    }
    /// A Huffman code (MSB-first) of `len` bits.
    fn code(&mut self, code: u16, len: u8) {
        let rev = (code as u32).reverse_bits() >> (32 - len as u32);
        self.put(rev, len as u32);
    }
    fn align(&mut self) {
        if self.nbits > 0 {
            self.put(0, 8 - self.nbits);
        }
    }
    fn flush_bytes(&mut self) -> io::Result<()> {
        if !self.buf.is_empty() {
            self.out.write_all(&self.buf)?;
            self.buf.clear();
        }
        Ok(())
    }
}

/// Huffman code lengths for `freq`, at most `limit` bits; zero-frequency
/// symbols get no code.
fn code_lengths(freq: &[u32], limit: u8) -> Vec<u8> {
    let mut lens = vec![0u8; freq.len()];
    let mut used: Vec<usize> = (0..freq.len()).filter(|&i| freq[i] > 0).collect();
    match used.len() {
        0 => return lens,
        1 => {
            lens[used[0]] = 1;
            return lens;
        }
        _ => {}
    }
    // Build the tree with two sorted queues (leaves, then merged nodes).
    used.sort_by_key(|&i| (freq[i], i));
    let n = used.len();
    let mut weight: Vec<u64> = used.iter().map(|&i| freq[i] as u64).collect();
    let mut parent = vec![usize::MAX; 2 * n - 1];
    let (mut leaf, mut node) = (0usize, n);
    let pick = |weight: &Vec<u64>, leaf: &mut usize, node: &mut usize, made: usize| -> usize {
        if *leaf < n && (*node >= made || weight[*leaf] <= weight[*node]) {
            *leaf += 1;
            *leaf - 1
        } else {
            *node += 1;
            *node - 1
        }
    };
    for made in n..2 * n - 1 {
        let a = pick(&weight, &mut leaf, &mut node, made);
        let b = pick(&weight, &mut leaf, &mut node, made);
        weight.push(weight[a] + weight[b]);
        parent[a] = made;
        parent[b] = made;
    }
    // Depths.
    let mut depth = vec![0u32; 2 * n - 1];
    for i in (0..2 * n - 2).rev() {
        depth[i] = depth[parent[i]] + 1;
    }
    let mut count = vec![0u32; 64];
    for &d in &depth[..n] {
        count[d as usize] += 1;
    }
    // Limit lengths (zlib's overflow fix-up), keeping the code complete.
    let limit = limit as usize;
    let mut overflow: u32 = count[limit + 1..].iter().sum();
    for l in limit + 1..count.len() {
        count[limit] += count[l];
        count[l] = 0;
    }
    while overflow > 0 {
        let mut bits = limit - 1;
        while count[bits] == 0 {
            bits -= 1;
        }
        count[bits] -= 1;
        count[bits + 1] += 2;
        count[limit] -= 1;
        overflow = overflow.saturating_sub(2);
    }
    // Fix-up can leave the Kraft sum short by one leaf; it's still valid.
    // Longest codes to the rarest symbols (`used` is sorted ascending).
    let mut k = 0;
    for len in (1..=limit).rev() {
        for _ in 0..count[len] {
            if k < n {
                lens[used[k]] = len as u8;
                k += 1;
            }
        }
    }
    lens
}

/// Canonical codes for lengths.
fn canonical(lens: &[u8]) -> Vec<u16> {
    let mut count = [0u16; 16];
    for &l in lens {
        count[l as usize] += 1;
    }
    count[0] = 0;
    let mut next = [0u16; 16];
    let mut code = 0u16;
    for l in 1..16 {
        code = (code + count[l - 1]) << 1;
        next[l] = code;
    }
    lens.iter()
        .map(|&l| {
            if l == 0 {
                0
            } else {
                let c = next[l as usize];
                next[l as usize] += 1;
                c
            }
        })
        .collect()
}

fn len_symbol(len: usize) -> (usize, u32, u32) {
    let i = match LEN_BASE.binary_search(&(len as u16)) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    let i = if len == 258 { 28 } else { i.min(27) };
    (
        257 + i,
        LEN_EXTRA[i] as u32,
        (len - LEN_BASE[i] as usize) as u32,
    )
}

fn dist_symbol(dist: usize) -> (usize, u32, u32) {
    let i = match DIST_BASE.binary_search(&(dist as u16)) {
        Ok(i) => i,
        Err(i) => i - 1,
    };
    (
        i,
        DIST_EXTRA[i] as u32,
        (dist - DIST_BASE[i] as usize) as u32,
    )
}

#[derive(Clone, Copy)]
enum Token {
    Lit(u8),
    Match(u16, u16),
}

/// Input gathered before a block is compressed.
const BLOCK: usize = 256 * 1024;
const HASH_BITS: u32 = 15;

/// A DEFLATE encoder writing compressed data to `W`.
pub struct Deflate<W: Write> {
    bw: BitWriter<W>,
    /// History (up to WINDOW bytes) followed by the pending block.
    data: Vec<u8>,
    hist: usize,
    head: Vec<u32>,
    prev: Vec<u32>,
    /// Absolute position of data[0].
    base: u64,
    tokens: Vec<Token>,
    max_chain: usize,
    max_insert: usize,
}

/// Length of the common prefix of `a` and `b` (same length), 8 bytes at a time.
fn match_len(a: &[u8], b: &[u8]) -> usize {
    let n = a.len().min(b.len());
    let mut l = 0;
    while l + 8 <= n {
        let x = u64::from_le_bytes(a[l..l + 8].try_into().unwrap())
            ^ u64::from_le_bytes(b[l..l + 8].try_into().unwrap());
        if x != 0 {
            return l + (x.trailing_zeros() / 8) as usize;
        }
        l += 8;
    }
    while l < n && a[l] == b[l] {
        l += 1;
    }
    l
}

impl<W: Write> Deflate<W> {
    /// Good compression (about zlib level 6).
    pub fn new(out: W) -> Self {
        Self::with_effort(out, 48, usize::MAX)
    }

    /// Faster, a little larger (about zlib level 1-2).
    pub fn fast(out: W) -> Self {
        Self::with_effort(out, 4, 8)
    }

    fn with_effort(out: W, max_chain: usize, max_insert: usize) -> Self {
        Deflate {
            max_chain,
            max_insert,
            bw: BitWriter {
                out,
                bits: 0,
                nbits: 0,
                buf: Vec::with_capacity(BLOCK),
            },
            data: Vec::with_capacity(WINDOW + BLOCK),
            hist: 0,
            head: vec![u32::MAX; 1 << HASH_BITS],
            prev: vec![u32::MAX; WINDOW],
            base: 0,
            tokens: Vec::with_capacity(BLOCK),
        }
    }

    fn hash(b: &[u8]) -> usize {
        let v = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        (v.wrapping_mul(0x9E37_79B1) >> (32 - HASH_BITS)) as usize
    }

    /// Compress the pending data into one block.
    fn block(&mut self, last: bool) -> io::Result<()> {
        let start = self.hist;
        let end = self.data.len();
        self.tokens.clear();
        let mut i = start;
        while i < end {
            let mut best = (0usize, 0usize);
            if i + 3 <= end {
                let h = Self::hash(&self.data[i..]);
                let here = (self.base + i as u64) as u32;
                let mut cand = self.head[h];
                let mut chain = 0;
                while cand != u32::MAX && chain < self.max_chain {
                    // Positions wrap at 4 GB; a stale entry only costs a
                    // failed comparison, since every match is checked.
                    let dist = here.wrapping_sub(cand) as usize;
                    if dist == 0 || dist > WINDOW || dist > i {
                        break;
                    }
                    let c = i - dist;
                    let max = (end - i).min(258);
                    // Can't beat the best so far unless the byte just past
                    // it matches too.
                    if best.0 > 0
                        && (best.0 >= max || self.data[c + best.0] != self.data[i + best.0])
                    {
                        cand = self.prev[cand as usize % WINDOW];
                        chain += 1;
                        continue;
                    }
                    let l = match_len(&self.data[c..c + max], &self.data[i..i + max]);
                    if l > best.0 {
                        best = (l, dist);
                        if l == max {
                            break;
                        }
                    }
                    cand = self.prev[cand as usize % WINDOW];
                    chain += 1;
                }
            }
            let step = if best.0 >= 3 {
                self.tokens.push(Token::Match(best.0 as u16, best.1 as u16));
                best.0
            } else {
                self.tokens.push(Token::Lit(self.data[i]));
                1
            };
            // Long matches only index their start (speed over a little size).
            let indexed = if step > self.max_insert { 1 } else { step };
            for p in i..i + indexed {
                if p + 3 <= end {
                    let h = Self::hash(&self.data[p..]);
                    let abs = (self.base + p as u64) as u32;
                    self.prev[abs as usize % WINDOW] = self.head[h];
                    self.head[h] = abs;
                }
            }
            i += step;
        }
        let data = std::mem::take(&mut self.data);
        let wrote = self.write_tokens(&data[start..end], last);
        self.data = data;
        wrote?;
        // Keep the last WINDOW bytes as history for the next block.
        if end > WINDOW {
            let drop = end - WINDOW;
            self.data.drain(..drop);
            self.base += drop as u64;
        }
        self.hist = self.data.len();
        self.bw.flush_bytes()
    }

    fn write_tokens(&mut self, raw: &[u8], last: bool) -> io::Result<()> {
        let mut lf = [0u32; 286];
        let mut df = [0u32; 30];
        for t in &self.tokens {
            match *t {
                Token::Lit(b) => lf[b as usize] += 1,
                Token::Match(l, d) => {
                    lf[len_symbol(l as usize).0] += 1;
                    df[dist_symbol(d as usize).0] += 1;
                }
            }
        }
        lf[256] = 1;
        // At least two codes each, so every decoder accepts them.
        if df.iter().filter(|&&f| f > 0).count() < 2 {
            df[0] = df[0].max(1);
            df[1] = df[1].max(1);
        }
        let ll = code_lengths(&lf, 15);
        let dl = code_lengths(&df, 15);
        let nlen = (257..=286)
            .rev()
            .find(|&n| ll[n - 1] != 0)
            .unwrap_or(257)
            .max(257);
        let ndist = (1..=30).rev().find(|&n| dl[n - 1] != 0).unwrap_or(1);
        // Run-length encode the code lengths.
        let mut all: Vec<u8> = ll[..nlen].to_vec();
        all.extend_from_slice(&dl[..ndist]);
        let mut rle: Vec<(u8, u8)> = Vec::new(); // (symbol, extra value)
        let mut i = 0;
        while i < all.len() {
            let v = all[i];
            let mut run = 1;
            while i + run < all.len() && all[i + run] == v {
                run += 1;
            }
            if v == 0 && run >= 3 {
                let mut r = run;
                while r >= 3 {
                    if r >= 11 {
                        let k = r.min(138);
                        rle.push((18, (k - 11) as u8));
                        r -= k;
                    } else {
                        let k = r.min(10);
                        rle.push((17, (k - 3) as u8));
                        r -= k;
                    }
                }
                for _ in 0..r {
                    rle.push((0, 0));
                }
            } else if v != 0 && run >= 4 {
                rle.push((v, 0));
                let mut r = run - 1;
                while r >= 3 {
                    let k = r.min(6);
                    rle.push((16, (k - 3) as u8));
                    r -= k;
                }
                for _ in 0..r {
                    rle.push((v, 0));
                }
            } else {
                for _ in 0..run {
                    rle.push((v, 0));
                }
            }
            i += run;
        }
        let mut cf = [0u32; 19];
        for &(s, _) in &rle {
            cf[s as usize] += 1;
        }
        let cl = code_lengths(&cf, 7);
        let ncode = (4..=19)
            .rev()
            .find(|&n| cl[CL_ORDER[n - 1]] != 0)
            .unwrap_or(4);

        // Size in bits of the dynamic block vs a stored one.
        let mut bits: u64 = 3 + 14 + 3 * ncode as u64;
        for &(s, _) in &rle {
            bits += cl[s as usize] as u64
                + [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 3, 7][s as usize];
        }
        for t in &self.tokens {
            bits += match *t {
                Token::Lit(b) => ll[b as usize] as u64,
                Token::Match(l, d) => {
                    let (ls, le, _) = len_symbol(l as usize);
                    let (ds, de, _) = dist_symbol(d as usize);
                    (ll[ls] as u32 + le + dl[ds] as u32 + de) as u64
                }
            };
        }
        bits += ll[256] as u64;
        let stored_bits = (raw.len() as u64 + 5 * (raw.len() as u64 / 65535 + 1)) * 8 + 8;
        if stored_bits < bits {
            let chunks: Vec<&[u8]> = if raw.is_empty() {
                vec![raw]
            } else {
                raw.chunks(65535).collect()
            };
            let n = chunks.len();
            for (k, c) in chunks.into_iter().enumerate() {
                self.bw.put((last && k + 1 == n) as u32, 1);
                self.bw.put(0, 2);
                self.bw.align();
                self.bw.put(c.len() as u32, 16);
                self.bw.put(!(c.len() as u32) & 0xFFFF, 16);
                self.bw.buf.extend_from_slice(c);
            }
            return Ok(());
        }
        let lc = canonical(&ll);
        let dc = canonical(&dl);
        let cc = canonical(&cl);
        self.bw.put(last as u32, 1);
        self.bw.put(2, 2);
        self.bw.put((nlen - 257) as u32, 5);
        self.bw.put((ndist - 1) as u32, 5);
        self.bw.put((ncode - 4) as u32, 4);
        for &i in CL_ORDER.iter().take(ncode) {
            self.bw.put(cl[i] as u32, 3);
        }
        for &(s, x) in &rle {
            self.bw.code(cc[s as usize], cl[s as usize]);
            match s {
                16 => self.bw.put(x as u32, 2),
                17 => self.bw.put(x as u32, 3),
                18 => self.bw.put(x as u32, 7),
                _ => {}
            }
        }
        for t in &self.tokens {
            match *t {
                Token::Lit(b) => self.bw.code(lc[b as usize], ll[b as usize]),
                Token::Match(l, d) => {
                    let (ls, le, lx) = len_symbol(l as usize);
                    self.bw.code(lc[ls], ll[ls]);
                    self.bw.put(lx, le);
                    let (ds, de, dx) = dist_symbol(d as usize);
                    self.bw.code(dc[ds], dl[ds]);
                    self.bw.put(dx, de);
                }
            }
            if self.bw.buf.len() > BLOCK {
                self.bw.flush_bytes()?;
            }
        }
        self.bw.code(lc[256], ll[256]);
        Ok(())
    }

    /// Write the final block and return the output.
    pub fn finish(mut self) -> io::Result<W> {
        self.end_stream()?;
        Ok(self.into_inner())
    }

    /// End this stream (final block, byte-aligned, flushed) and get ready
    /// for a new one on the same output, keeping the buffers.
    pub fn end_stream(&mut self) -> io::Result<()> {
        self.block(true)?;
        self.bw.align();
        self.bw.flush_bytes()?;
        self.bw.out.flush()?;
        self.data.clear();
        self.hist = 0;
        self.base = 0;
        self.head.fill(u32::MAX);
        self.prev.fill(u32::MAX);
        Ok(())
    }

    /// The output, after [`Deflate::end_stream`] (nothing pending).
    pub fn into_inner(self) -> W {
        let Deflate { bw, .. } = self;
        bw.out
    }

    pub fn get_ref(&self) -> &W {
        &self.bw.out
    }

    pub fn get_mut(&mut self) -> &mut W {
        &mut self.bw.out
    }
}

impl<W: Write> Write for Deflate<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let room = (self.hist + BLOCK) - self.data.len();
        let n = room.min(buf.len());
        self.data.extend_from_slice(&buf[..n]);
        if self.data.len() == self.hist + BLOCK {
            self.block(false)?;
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.bw.flush_bytes()?;
        self.bw.out.flush()
    }
}

/// Compress a whole buffer.
pub fn deflate(data: &[u8]) -> Vec<u8> {
    let mut d = Deflate::new(Vec::new());
    d.write_all(data).unwrap();
    d.finish().unwrap()
}

// ---------------------------------------------------------------------------
// gzip
// ---------------------------------------------------------------------------

/// Writes a gzip file: header, DEFLATE data, CRC-32 and length.
pub struct GzWriter<W: Write> {
    inner: Option<Deflate<W>>,
    crc: Crc32,
    len: u64,
}

impl<W: Write> GzWriter<W> {
    pub fn new(out: W) -> io::Result<Self> {
        Self::with(out, false)
    }

    /// Favor speed over size.
    pub fn fast(out: W) -> io::Result<Self> {
        Self::with(out, true)
    }

    fn with(mut out: W, fast: bool) -> io::Result<Self> {
        // No name, no timestamp (so equal input gives equal files).
        out.write_all(&[0x1f, 0x8b, 8, 0, 0, 0, 0, 0, 0, 0xff])?;
        Ok(GzWriter {
            inner: Some(if fast {
                Deflate::fast(out)
            } else {
                Deflate::new(out)
            }),
            crc: Crc32::default(),
            len: 0,
        })
    }

    pub fn finish(mut self) -> io::Result<W> {
        let d = self.inner.take().expect("finished twice");
        let mut out = d.finish()?;
        out.write_all(&self.crc.value().to_le_bytes())?;
        out.write_all(&(self.len as u32).to_le_bytes())?;
        out.flush()?;
        Ok(out)
    }
}

impl<W: Write> Write for GzWriter<W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self
            .inner
            .as_mut()
            .expect("write after finish")
            .write(buf)?;
        self.crc.update(&buf[..n]);
        self.len += n as u64;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.inner.as_mut().map_or(Ok(()), |d| d.flush())
    }
}

/// Reads a gzip file, checking its CRC-32 and length at the end.
pub struct GzReader<R: Read> {
    inner: Option<Inflate<R>>,
    crc: Crc32,
    len: u64,
    done: bool,
}

impl<R: Read> GzReader<R> {
    pub fn new(mut src: R) -> io::Result<Self> {
        let mut h = [0u8; 10];
        src.read_exact(&mut h)?;
        if h[0] != 0x1f || h[1] != 0x8b || h[2] != 8 {
            return Err(invalid("not a gzip file"));
        }
        let flags = h[3];
        if flags & 4 != 0 {
            let mut n = [0u8; 2];
            src.read_exact(&mut n)?;
            io::copy(
                &mut (&mut src).take(u16::from_le_bytes(n) as u64),
                &mut io::sink(),
            )?;
        }
        for bit in [8u8, 16] {
            if flags & bit != 0 {
                let mut b = [0u8; 1];
                loop {
                    src.read_exact(&mut b)?;
                    if b[0] == 0 {
                        break;
                    }
                }
            }
        }
        if flags & 2 != 0 {
            let mut c = [0u8; 2];
            src.read_exact(&mut c)?;
        }
        Ok(GzReader {
            inner: Some(Inflate::new(src)),
            crc: Crc32::default(),
            len: 0,
            done: false,
        })
    }
}

impl<R: Read> Read for GzReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.done || buf.is_empty() {
            return Ok(0);
        }
        let n = self.inner.as_mut().unwrap().read(buf)?;
        self.crc.update(&buf[..n]);
        self.len += n as u64;
        if n == 0 {
            self.done = true;
            let (mut src, rest) = self.inner.take().unwrap().into_parts();
            let mut trailer = [0u8; 8];
            let have = rest.len().min(8);
            trailer[..have].copy_from_slice(&rest[..have]);
            if have < 8 {
                src.read_exact(&mut trailer[have..])?;
            }
            let crc = u32::from_le_bytes(trailer[..4].try_into().unwrap());
            let len = u32::from_le_bytes(trailer[4..].try_into().unwrap());
            if crc != self.crc.value() || len != self.len as u32 {
                return Err(invalid("the file is damaged (checksum mismatch)"));
            }
        }
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn samples() -> Vec<Vec<u8>> {
        let mut text = Vec::new();
        for i in 0..20_000 {
            text.extend_from_slice(
                format!("HKLM\\SOFTWARE\\Vendor\\Key{} = value {}\n", i % 97, i).as_bytes(),
            );
        }
        let mut noise = vec![0u8; 300_000];
        let mut x: u32 = 1;
        for b in &mut noise {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            *b = x as u8;
        }
        vec![
            vec![],
            b"a".to_vec(),
            b"abcabcabcabcabcabc".to_vec(),
            vec![0u8; 100_000],
            text,
            noise,
            (0..=255u8).cycle().take(700_000).collect(),
            fibonacci_skew(),
        ]
    }

    /// Symbol counts in a Fibonacci series force Huffman depths past 15,
    /// so the length limit has to kick in.
    fn fibonacci_skew() -> Vec<u8> {
        let (mut a, mut b) = (1usize, 1usize);
        let mut out = Vec::new();
        for sym in 0..26u8 {
            out.extend(std::iter::repeat_n(b'A' + sym, a));
            (a, b) = (b, a + b);
        }
        // Shuffle so LZ77 finds few long matches.
        let mut x: u32 = 7;
        for i in (1..out.len()).rev() {
            x ^= x << 13;
            x ^= x >> 17;
            x ^= x << 5;
            out.swap(i, x as usize % (i + 1));
        }
        out
    }

    /// With INSTALLSCOPE_DEFLATE_DUMP set, write each sample's compressed form
    /// there so another decoder can check it.
    #[test]
    fn dump_for_other_decoders() {
        let Some(dir) = std::env::var_os("INSTALLSCOPE_DEFLATE_DUMP") else {
            return;
        };
        for (i, s) in samples().iter().enumerate() {
            let d = std::path::Path::new(&dir);
            std::fs::write(d.join(format!("{i}.raw")), s).unwrap();
            std::fs::write(d.join(format!("{i}.def")), deflate(s)).unwrap();
            let mut w = GzWriter::new(Vec::new()).unwrap();
            w.write_all(s).unwrap();
            std::fs::write(d.join(format!("{i}.gz")), w.finish().unwrap()).unwrap();
        }
    }

    /// cargo test --release --lib deflate::tests::speed -- --ignored --nocapture
    #[test]
    #[ignore]
    fn speed() {
        let s = samples();
        let data: Vec<u8> = [&s[4][..], &s[7][..], &s[5][..]].concat().repeat(20);
        for (name, fast) in [("normal", false), ("fast", true)] {
            let t = std::time::Instant::now();
            let mut d = if fast {
                Deflate::fast(Vec::new())
            } else {
                Deflate::new(Vec::new())
            };
            d.write_all(&data).unwrap();
            let c = d.finish().unwrap();
            let enc = t.elapsed();
            let t = std::time::Instant::now();
            let back = inflate(&c).unwrap();
            let dec = t.elapsed();
            assert_eq!(back, data);
            let mb = data.len() as f64 / 1e6;
            println!(
                "{name}: {} -> {} | enc {:.0} MB/s dec {:.0} MB/s",
                data.len(),
                c.len(),
                mb / enc.as_secs_f64(),
                mb / dec.as_secs_f64()
            );
        }
    }

    #[test]
    fn crc32_known_values() {
        assert_eq!(crc32(b""), 0);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        assert_eq!(
            crc32(b"The quick brown fox jumps over the lazy dog"),
            0x414F_A339
        );
    }

    #[test]
    fn round_trips_every_kind_of_data() {
        for s in samples() {
            let c = deflate(&s);
            assert_eq!(inflate(&c).unwrap(), s, "len {}", s.len());
        }
    }

    #[test]
    fn compresses_repetitive_data_well_and_random_data_barely_grows() {
        let s = samples();
        assert!(
            deflate(&s[3]).len() < 400,
            "zeros: {}",
            deflate(&s[3]).len()
        );
        assert!(deflate(&s[4]).len() * 5 < s[4].len(), "text");
        assert!(
            deflate(&s[5]).len() <= s[5].len() + s[5].len() / 1000 + 64,
            "random"
        );
    }

    #[test]
    fn decodes_streams_from_other_encoders() {
        // From zlib (Python's zlib.compress, wbits=-15): fixed and dynamic
        // Huffman and stored blocks.
        let fixed = include_bytes!("testdata/deflate/fixed.bin");
        assert_eq!(
            inflate(fixed).unwrap(),
            include_bytes!("testdata/deflate/fixed.txt")
        );
        let dynamic = include_bytes!("testdata/deflate/dynamic.bin");
        assert_eq!(
            inflate(dynamic).unwrap(),
            include_bytes!("testdata/deflate/dynamic.txt")
        );
        let stored = include_bytes!("testdata/deflate/stored.bin");
        assert_eq!(
            inflate(stored).unwrap(),
            include_bytes!("testdata/deflate/dynamic.txt")
        );
    }

    #[test]
    fn rejects_damaged_streams() {
        let c = deflate(&samples()[4]);
        assert!(inflate(&c[..c.len() / 2]).is_err());
        assert!(inflate(&[0xff, 0xff, 0xff]).is_err());
        let mut bad = c.clone();
        bad[10] ^= 0x55;
        // Either an error or different data, never the original silently.
        assert!(inflate(&bad).map(|d| d != samples()[4]).unwrap_or(true));
    }

    #[test]
    fn gzip_round_trip_and_checksum() {
        for s in samples() {
            let mut w = GzWriter::new(Vec::new()).unwrap();
            w.write_all(&s).unwrap();
            let gz = w.finish().unwrap();
            let mut out = Vec::new();
            GzReader::new(&gz[..])
                .unwrap()
                .read_to_end(&mut out)
                .unwrap();
            assert_eq!(out, s);
            if gz.len() > 30 {
                let mut bad = gz.clone();
                let n = bad.len();
                bad[n - 6] ^= 1; // CRC
                let mut out = Vec::new();
                assert!(GzReader::new(&bad[..])
                    .unwrap()
                    .read_to_end(&mut out)
                    .is_err());
            }
        }
        // A gzip file from another program (Python's gzip, with a name).
        let mut out = Vec::new();
        GzReader::new(&include_bytes!("testdata/deflate/named.gz")[..])
            .unwrap()
            .read_to_end(&mut out)
            .unwrap();
        assert_eq!(out, include_bytes!("testdata/deflate/dynamic.txt"));
    }
}
