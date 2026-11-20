//! Minimal, dependency-free writers for the fixture formats and the digests of the
//! checksum manifests. They are written from the format specifications, not from
//! application code, so fixtures and expected hashes stay independent of what the
//! suite tests. Each writer produces small, valid, deterministic files; none aims for
//! compression.

/// CRC-32 (IEEE 802.3, reflected), as used by PNG, gzip, ZIP and SFV.
pub fn crc32(data: &[u8]) -> u32 {
    crc32_update(0, data)
}

fn crc32_update(crc: u32, data: &[u8]) -> u32 {
    let mut crc = !crc;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 != 0 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn adler32(data: &[u8]) -> u32 {
    let (mut a, mut b) = (1_u32, 0_u32);
    for &byte in data {
        a = (a + u32::from(byte)) % 65_521;
        b = (b + a) % 65_521;
    }
    (b << 16) | a
}

/// Deflate "stored" blocks (RFC 1951 BTYPE=00): valid, uncompressed.
fn deflate_stored(data: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(data.len() + data.len() / 65_535 * 5 + 5);
    let mut chunks = data.chunks(65_535).peekable();
    if chunks.peek().is_none() {
        output.extend_from_slice(&[1, 0, 0, 0xFF, 0xFF]);
    }
    while let Some(chunk) = chunks.next() {
        output.push(u8::from(chunks.peek().is_none()));
        let length = chunk.len() as u16;
        output.extend_from_slice(&length.to_le_bytes());
        output.extend_from_slice(&(!length).to_le_bytes());
        output.extend_from_slice(chunk);
    }
    output
}

fn zlib_stored(data: &[u8]) -> Vec<u8> {
    let mut output = vec![0x78, 0x01];
    output.extend(deflate_stored(data));
    output.extend_from_slice(&adler32(data).to_be_bytes());
    output
}

/// A gzip member (RFC 1952) with stored deflate blocks and a zero timestamp.
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut output = vec![0x1F, 0x8B, 8, 0, 0, 0, 0, 0, 0, 0xFF];
    output.extend(deflate_stored(data));
    output.extend_from_slice(&crc32(data).to_le_bytes());
    output.extend_from_slice(&(data.len() as u32).to_le_bytes());
    output
}

fn png_chunk(output: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    output.extend_from_slice(&(data.len() as u32).to_be_bytes());
    let start = output.len();
    output.extend_from_slice(kind);
    output.extend_from_slice(data);
    let crc = crc32(&output[start..]);
    output.extend_from_slice(&crc.to_be_bytes());
}

/// An 8-bit RGB PNG; `pixel(x, y)` gives each colour.
pub fn png(width: u32, height: u32, pixel: impl Fn(u32, u32) -> [u8; 3]) -> Vec<u8> {
    let mut raw = Vec::with_capacity((width as usize * 3 + 1) * height as usize);
    for y in 0..height {
        raw.push(0);
        for x in 0..width {
            raw.extend_from_slice(&pixel(x, y));
        }
    }
    let mut output = b"\x89PNG\r\n\x1a\n".to_vec();
    let mut header = Vec::new();
    header.extend_from_slice(&width.to_be_bytes());
    header.extend_from_slice(&height.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    png_chunk(&mut output, b"IHDR", &header);
    png_chunk(&mut output, b"IDAT", &zlib_stored(&raw));
    png_chunk(&mut output, b"IEND", &[]);
    output
}

/// Least-significant-bit-first bit packing, as GIF LZW and WebP VP8L require.
#[derive(Default)]
struct LsbBits {
    bytes: Vec<u8>,
    bit: u32,
}

impl LsbBits {
    fn put(&mut self, value: u32, count: u32) {
        for index in 0..count {
            if self.bit == 0 {
                self.bytes.push(0);
            }
            if value >> index & 1 != 0 {
                *self.bytes.last_mut().unwrap() |= 1 << self.bit;
            }
            self.bit = (self.bit + 1) % 8;
        }
    }
}

/// A GIF89a with a 256-entry palette. LZW emits every index as a literal code and
/// restarts the table before codes widen, which every decoder accepts.
pub fn gif(
    width: u16,
    height: u16,
    palette: &[[u8; 3]; 256],
    index: impl Fn(u16, u16) -> u8,
) -> Vec<u8> {
    let mut output = b"GIF89a".to_vec();
    output.extend_from_slice(&width.to_le_bytes());
    output.extend_from_slice(&height.to_le_bytes());
    output.extend_from_slice(&[0xF7, 0, 0]);
    for colour in palette {
        output.extend_from_slice(colour);
    }
    output.push(0x2C);
    output.extend_from_slice(&[0, 0, 0, 0]);
    output.extend_from_slice(&width.to_le_bytes());
    output.extend_from_slice(&height.to_le_bytes());
    output.push(0);
    output.push(8);
    let (clear, end) = (256, 257);
    let mut bits = LsbBits::default();
    let mut since_clear = 0;
    bits.put(clear, 9);
    for y in 0..height {
        for x in 0..width {
            if since_clear == 250 {
                bits.put(clear, 9);
                since_clear = 0;
            }
            bits.put(u32::from(index(x, y)), 9);
            since_clear += 1;
        }
    }
    bits.put(end, 9);
    for block in bits.bytes.chunks(255) {
        output.push(block.len() as u8);
        output.extend_from_slice(block);
    }
    output.extend_from_slice(&[0, 0x3B]);
    output
}

/// A lossless WebP (VP8L) of two colours. Each channel uses a "simple" prefix code
/// of one or two symbols, so a pixel costs at most one bit per channel. Every channel
/// of `low` must not exceed the same channel of `high`.
pub fn webp(
    width: u16,
    height: u16,
    low: [u8; 3],
    high: [u8; 3],
    is_high: impl Fn(u16, u16) -> bool,
) -> Vec<u8> {
    let mut bits = LsbBits::default();
    bits.put(0x2F, 8);
    bits.put(u32::from(width) - 1, 14);
    bits.put(u32::from(height) - 1, 14);
    bits.put(0, 1); // alpha_is_used
    bits.put(0, 3); // version
    bits.put(0, 1); // no transform
    bits.put(0, 1); // no colour cache
    bits.put(0, 1); // one prefix-code group
    let simple = |bits: &mut LsbBits, symbols: &[u8]| {
        bits.put(1, 1);
        bits.put(symbols.len() as u32 - 1, 1);
        bits.put(1, 1); // the first symbol has eight bits
        for symbol in symbols {
            bits.put(u32::from(*symbol), 8);
        }
    };
    let channels = |channel: usize| -> Vec<u8> {
        if low[channel] == high[channel] {
            vec![low[channel]]
        } else {
            vec![low[channel], high[channel]]
        }
    };
    // Green, red, blue, alpha, distance.
    let (green, red, blue) = (channels(1), channels(0), channels(2));
    simple(&mut bits, &green);
    simple(&mut bits, &red);
    simple(&mut bits, &blue);
    simple(&mut bits, &[255]);
    simple(&mut bits, &[0]);
    for y in 0..height {
        for x in 0..width {
            let high = u32::from(is_high(x, y));
            for symbols in [&green, &red, &blue] {
                if symbols.len() == 2 {
                    bits.put(high, 1);
                }
            }
        }
    }
    let mut data = bits.bytes;
    let length = data.len() as u32;
    if data.len() % 2 == 1 {
        data.push(0);
    }
    let mut output = b"RIFF".to_vec();
    output.extend_from_slice(&(4 + 8 + data.len() as u32).to_le_bytes());
    output.extend_from_slice(b"WEBPVP8L");
    output.extend_from_slice(&length.to_le_bytes());
    output.extend(data);
    output
}

/// Most-significant-bit-first bits with JPEG byte stuffing.
#[derive(Default)]
struct JpegBits {
    bytes: Vec<u8>,
    current: u32,
    count: u32,
}

impl JpegBits {
    fn put(&mut self, value: u32, count: u32) {
        for index in (0..count).rev() {
            self.current = self.current << 1 | (value >> index & 1);
            self.count += 1;
            if self.count == 8 {
                self.flush_byte();
            }
        }
    }

    fn flush_byte(&mut self) {
        let byte = self.current as u8;
        self.bytes.push(byte);
        if byte == 0xFF {
            self.bytes.push(0);
        }
        self.current = 0;
        self.count = 0;
    }

    fn finish(mut self) -> Vec<u8> {
        while self.count != 0 {
            self.put(1, 1);
        }
        self.bytes
    }
}

/// A baseline greyscale JPEG whose 8x8 blocks are flat; `level(bx, by)` is the grey
/// value of each block. Only DC coefficients are coded (every block ends with EOB).
pub fn jpeg(blocks_x: u16, blocks_y: u16, level: impl Fn(u16, u16) -> u8) -> Vec<u8> {
    const QUANT: u8 = 16;
    let mut output = vec![0xFF, 0xD8];
    output.extend_from_slice(&[0xFF, 0xE0, 0, 16]);
    output.extend_from_slice(b"JFIF\0\x01\x01\0\0\x01\0\x01\0\0");
    output.extend_from_slice(&[0xFF, 0xDB, 0, 67, 0]);
    output.extend_from_slice(&[QUANT; 64]);
    output.extend_from_slice(&[0xFF, 0xC0, 0, 11, 8]);
    output.extend_from_slice(&(blocks_y * 8).to_be_bytes());
    output.extend_from_slice(&(blocks_x * 8).to_be_bytes());
    output.extend_from_slice(&[1, 1, 0x11, 0]);
    // DC table 0: the twelve size categories, all with four-bit codes 0000..1011.
    let mut dc = vec![0xFF, 0xC4, 0, 31, 0x00];
    let mut counts = [0_u8; 16];
    counts[3] = 12;
    dc.extend_from_slice(&counts);
    dc.extend(0..12_u8);
    output.extend(dc);
    // AC table 0: only end-of-block, as the one-bit code 0.
    let mut ac = vec![0xFF, 0xC4, 0, 20, 0x10];
    let mut counts = [0_u8; 16];
    counts[0] = 1;
    ac.extend_from_slice(&counts);
    ac.push(0x00);
    output.extend(ac);
    output.extend_from_slice(&[0xFF, 0xDA, 0, 8, 1, 1, 0x00, 0, 63, 0]);
    let mut bits = JpegBits::default();
    let mut previous = 0_i32;
    for by in 0..blocks_y {
        for bx in 0..blocks_x {
            // A flat block's DC coefficient is eight times its level-shifted value.
            let coefficient = (i32::from(level(bx, by)) - 128) * 8 / i32::from(QUANT);
            let difference = coefficient - previous;
            previous = coefficient;
            let category = 32 - difference.unsigned_abs().leading_zeros();
            bits.put(category, 4);
            if category > 0 {
                let value = if difference < 0 {
                    difference + (1 << category) - 1
                } else {
                    difference
                };
                bits.put(value as u32, category);
            }
            bits.put(0, 1);
        }
    }
    output.extend(bits.finish());
    output.extend_from_slice(&[0xFF, 0xD9]);
    output
}

/// A one-page PDF 1.4 with a Helvetica text line and an exact cross-reference table.
pub fn pdf(lines: &[&str]) -> Vec<u8> {
    let mut content = String::from("BT /F1 14 Tf 24 160 Td 18 TL\n");
    for line in lines {
        let escaped = line
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        content.push_str(&format!("({escaped}) Tj T*\n"));
    }
    content.push_str("ET\n");
    let objects = [
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_owned(),
        "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R \
         /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_owned(),
        format!(
            "<< /Length {} >>\nstream\n{content}endstream",
            content.len()
        ),
        "<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_owned(),
    ];
    let mut output = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = output.len();
    output.extend_from_slice(
        format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
    );
    for offset in offsets {
        output.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    output.extend_from_slice(
        format!(
            "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
            objects.len() + 1
        )
        .as_bytes(),
    );
    output
}

/// One archive member: a directory when `data` is `None`. Paths use `/`.
pub struct Member {
    pub path: String,
    pub data: Option<Vec<u8>>,
}

impl Member {
    pub fn file(path: &str, data: impl Into<Vec<u8>>) -> Self {
        Self {
            path: path.to_owned(),
            data: Some(data.into()),
        }
    }

    pub fn directory(path: &str) -> Self {
        Self {
            path: format!("{}/", path.trim_end_matches('/')),
            data: None,
        }
    }
}

/// A ZIP with stored (uncompressed) members, UTF-8 names and a fixed 2026-01-02 date.
pub fn zip(members: &[Member]) -> Vec<u8> {
    const TIME: u16 = 12 << 11;
    const DATE: u16 = ((2026 - 1980) << 9) | (1 << 5) | 2;
    let mut output = Vec::new();
    let mut central = Vec::new();
    for member in members {
        let data = member.data.as_deref().unwrap_or_default();
        let crc = crc32(data);
        let name = member.path.as_bytes();
        let offset = output.len() as u32;
        let header = |signature: u32, central: bool| {
            let mut header = Vec::new();
            header.extend_from_slice(&signature.to_le_bytes());
            if central {
                header.extend_from_slice(&(0x0300_u16 | 20).to_le_bytes());
            }
            header.extend_from_slice(&20_u16.to_le_bytes());
            header.extend_from_slice(&0x0800_u16.to_le_bytes());
            header.extend_from_slice(&0_u16.to_le_bytes());
            header.extend_from_slice(&TIME.to_le_bytes());
            header.extend_from_slice(&DATE.to_le_bytes());
            header.extend_from_slice(&crc.to_le_bytes());
            header.extend_from_slice(&(data.len() as u32).to_le_bytes());
            header.extend_from_slice(&(data.len() as u32).to_le_bytes());
            header.extend_from_slice(&(name.len() as u16).to_le_bytes());
            header.extend_from_slice(&0_u16.to_le_bytes());
            header
        };
        output.extend(header(0x0403_4B50, false));
        output.extend_from_slice(name);
        output.extend_from_slice(data);
        let mut entry = header(0x0201_4B50, true);
        entry.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        let mode: u32 = if member.data.is_some() {
            0o100_644 << 16
        } else {
            (0o040_755 << 16) | 0x10
        };
        entry.extend_from_slice(&mode.to_le_bytes());
        entry.extend_from_slice(&offset.to_le_bytes());
        entry.extend_from_slice(name);
        central.extend(entry);
    }
    let central_offset = output.len() as u32;
    let central_size = central.len() as u32;
    output.extend(central);
    output.extend_from_slice(&0x0605_4B50_u32.to_le_bytes());
    output.extend_from_slice(&[0, 0, 0, 0]);
    let count = members.len() as u16;
    output.extend_from_slice(&count.to_le_bytes());
    output.extend_from_slice(&count.to_le_bytes());
    output.extend_from_slice(&central_size.to_le_bytes());
    output.extend_from_slice(&central_offset.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output
}

/// A POSIX ustar archive with a fixed modification time; names must fit 100 bytes.
pub fn tar(members: &[Member]) -> Vec<u8> {
    fn octal(field: &mut [u8], value: u64) {
        let text = format!("{value:0width$o}", width = field.len() - 1);
        field[..text.len()].copy_from_slice(text.as_bytes());
    }
    let mut output = Vec::new();
    for member in members {
        let data = member.data.as_deref().unwrap_or_default();
        let mut header = [0_u8; 512];
        let name = member.path.as_bytes();
        assert!(
            name.len() <= 100,
            "tar fixture name too long: {}",
            member.path
        );
        header[..name.len()].copy_from_slice(name);
        octal(
            &mut header[100..108],
            if member.data.is_some() { 0o644 } else { 0o755 },
        );
        octal(&mut header[108..116], 0);
        octal(&mut header[116..124], 0);
        octal(&mut header[124..136], data.len() as u64);
        octal(&mut header[136..148], 1_767_312_000);
        header[148..156].fill(b' ');
        header[156] = if member.data.is_some() { b'0' } else { b'5' };
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        header[265..273].copy_from_slice(b"fixture\0");
        header[297..305].copy_from_slice(b"fixture\0");
        let sum: u32 = header.iter().map(|&byte| u32::from(byte)).sum();
        let text = format!("{sum:06o}\0 ");
        header[148..156].copy_from_slice(text.as_bytes());
        output.extend_from_slice(&header);
        output.extend_from_slice(data);
        output.resize(output.len().next_multiple_of(512), 0);
    }
    output.resize(output.len() + 1024, 0);
    output
}

/// SHA-256 (FIPS 180-4).
pub fn sha256(data: &[u8]) -> [u8; 32] {
    const K: [u32; 64] = [
        0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4,
        0xab1c5ed5, 0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe,
        0x9bdc06a7, 0xc19bf174, 0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f,
        0x4a7484aa, 0x5cb0a9dc, 0x76f988da, 0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7,
        0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967, 0x27b70a85, 0x2e1b2138, 0x4d2c6dfc,
        0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85, 0xa2bfe8a1, 0xa81a664b,
        0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070, 0x19a4c116,
        0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
        0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7,
        0xc67178f2,
    ];
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(data.len() as u64 * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0_u32; 64];
        for (index, word) in block.chunks(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..64 {
            let s0 = w[index - 15].rotate_right(7)
                ^ w[index - 15].rotate_right(18)
                ^ (w[index - 15] >> 3);
            let s1 = w[index - 2].rotate_right(17)
                ^ w[index - 2].rotate_right(19)
                ^ (w[index - 2] >> 10);
            w[index] = w[index - 16]
                .wrapping_add(s0)
                .wrapping_add(w[index - 7])
                .wrapping_add(s1);
        }
        let mut v = state;
        for index in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let choice = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(choice)
                .wrapping_add(K[index])
                .wrapping_add(w[index]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let majority = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(majority);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (word, value) in state.iter_mut().zip(v) {
            *word = word.wrapping_add(value);
        }
    }
    let mut digest = [0_u8; 32];
    for (bytes, word) in digest.chunks_mut(4).zip(state) {
        bytes.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// MD5 (RFC 1321).
pub fn md5(data: &[u8]) -> [u8; 16] {
    const SHIFTS: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5,
        9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10,
        15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    const CONSTANTS: [u32; 64] = [
        0xd76aa478, 0xe8c7b756, 0x242070db, 0xc1bdceee, 0xf57c0faf, 0x4787c62a, 0xa8304613,
        0xfd469501, 0x698098d8, 0x8b44f7af, 0xffff5bb1, 0x895cd7be, 0x6b901122, 0xfd987193,
        0xa679438e, 0x49b40821, 0xf61e2562, 0xc040b340, 0x265e5a51, 0xe9b6c7aa, 0xd62f105d,
        0x02441453, 0xd8a1e681, 0xe7d3fbc8, 0x21e1cde6, 0xc33707d6, 0xf4d50d87, 0x455a14ed,
        0xa9e3e905, 0xfcefa3f8, 0x676f02d9, 0x8d2a4c8a, 0xfffa3942, 0x8771f681, 0x6d9d6122,
        0xfde5380c, 0xa4beea44, 0x4bdecfa9, 0xf6bb4b60, 0xbebfbc70, 0x289b7ec6, 0xeaa127fa,
        0xd4ef3085, 0x04881d05, 0xd9d4d039, 0xe6db99e5, 0x1fa27cf8, 0xc4ac5665, 0xf4292244,
        0x432aff97, 0xab9423a7, 0xfc93a039, 0x655b59c3, 0x8f0ccc92, 0xffeff47d, 0x85845dd1,
        0x6fa87e4f, 0xfe2ce6e0, 0xa3014314, 0x4e0811a1, 0xf7537e82, 0xbd3af235, 0x2ad7d2bb,
        0xeb86d391,
    ];
    let mut state: [u32; 4] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(data.len() as u64 * 8).to_le_bytes());
    for block in message.chunks(64) {
        let words: Vec<u32> = block
            .chunks(4)
            .map(|word| u32::from_le_bytes([word[0], word[1], word[2], word[3]]))
            .collect();
        let [mut a, mut b, mut c, mut d] = state;
        for index in 0..64 {
            let (mix, word) = match index / 16 {
                0 => ((b & c) | (!b & d), index),
                1 => ((d & b) | (!d & c), (5 * index + 1) % 16),
                2 => (b ^ c ^ d, (3 * index + 5) % 16),
                _ => (c ^ (b | !d), (7 * index) % 16),
            };
            let rotated = a
                .wrapping_add(mix)
                .wrapping_add(CONSTANTS[index])
                .wrapping_add(words[word])
                .rotate_left(SHIFTS[index]);
            (a, b, c, d) = (d, b.wrapping_add(rotated), b, c);
        }
        for (word, value) in state.iter_mut().zip([a, b, c, d]) {
            *word = word.wrapping_add(value);
        }
    }
    let mut digest = [0_u8; 16];
    for (bytes, word) in digest.chunks_mut(4).zip(state) {
        bytes.copy_from_slice(&word.to_le_bytes());
    }
    digest
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Known-answer checks of the digests and CRC; run before fixtures are written.
pub fn self_test() -> Result<(), String> {
    let checks = [
        (
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e"),
        (
            hex(&md5(b"The quick brown fox jumps over the lazy dog")),
            "9e107d9d372bb6826bd81d3542a419d6",
        ),
        (format!("{:08x}", crc32(b"123456789")), "cbf43926"),
    ];
    for (actual, expected) in checks {
        if actual != expected {
            return Err(format!("digest self-test failed: {actual} != {expected}"));
        }
    }
    Ok(())
}
