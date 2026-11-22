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
    let mut output = gif_screen(width, height, palette);
    gif_image(&mut output, width, height, &index);
    output.push(0x3B);
    output
}

/// An animated GIF89a: `frames` full images of `delay` hundredths of a second each,
/// each after a Graphic Control Extension, looping forever (the NETSCAPE2.0
/// application extension). Viewers that show one picture show the first frame.
pub fn gif_animated(
    width: u16,
    height: u16,
    palette: &[[u8; 3]; 256],
    delay: u16,
    frames: &[&dyn Fn(u16, u16) -> u8],
) -> Vec<u8> {
    let mut output = gif_screen(width, height, palette);
    output.extend_from_slice(b"\x21\xFF\x0BNETSCAPE2.0\x03\x01\x00\x00\x00");
    for frame in frames {
        // Graphic Control Extension: disposal 1 (keep), no transparency, the delay.
        output.extend_from_slice(&[0x21, 0xF9, 4, 0x04]);
        output.extend_from_slice(&delay.to_le_bytes());
        output.extend_from_slice(&[0, 0]);
        gif_image(&mut output, width, height, frame);
    }
    output.push(0x3B);
    output
}

/// The header, logical screen descriptor and 256-colour global palette.
fn gif_screen(width: u16, height: u16, palette: &[[u8; 3]; 256]) -> Vec<u8> {
    let mut output = b"GIF89a".to_vec();
    output.extend_from_slice(&width.to_le_bytes());
    output.extend_from_slice(&height.to_le_bytes());
    output.extend_from_slice(&[0xF7, 0, 0]);
    for colour in palette {
        output.extend_from_slice(colour);
    }
    output
}

/// One full-screen image: its descriptor and literal-only LZW data blocks.
fn gif_image(output: &mut Vec<u8>, width: u16, height: u16, index: &dyn Fn(u16, u16) -> u8) {
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
    output.push(0);
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
    let chunk = vp8l_chunk(width, height, low, high, &is_high);
    let mut output = b"RIFF".to_vec();
    output.extend_from_slice(&(4 + chunk.len() as u32).to_le_bytes());
    output.extend_from_slice(b"WEBP");
    output.extend(chunk);
    output
}

/// One frame of [`webp_animated`]: its two colours (as [`webp`] takes them), which
/// pixels take the high one, and how long it shows.
pub struct WebpFrame<'a> {
    pub low: [u8; 3],
    pub high: [u8; 3],
    pub is_high: &'a dyn Fn(u16, u16) -> bool,
    pub milliseconds: u32,
}

/// An animated WebP (the extended format: VP8X with the animation flag, ANIM, then one
/// ANMF per frame holding a lossless VP8L image of the whole canvas), looping forever.
/// Viewers that show one picture show the first frame.
pub fn webp_animated(width: u16, height: u16, frames: &[WebpFrame<'_>]) -> Vec<u8> {
    let u24 = |value: u32| value.to_le_bytes()[..3].to_vec();
    let mut chunks = Vec::new();
    // VP8X: the animation flag (bit 1), three reserved bytes, the canvas size minus one.
    chunks.extend_from_slice(b"VP8X");
    chunks.extend_from_slice(&10_u32.to_le_bytes());
    chunks.extend_from_slice(&[0x02, 0, 0, 0]);
    chunks.extend(u24(u32::from(width) - 1));
    chunks.extend(u24(u32::from(height) - 1));
    // ANIM: a white background (BGRA) and an endless loop.
    chunks.extend_from_slice(b"ANIM");
    chunks.extend_from_slice(&6_u32.to_le_bytes());
    chunks.extend_from_slice(&[0xFF, 0xFF, 0xFF, 0xFF, 0, 0]);
    for frame in frames {
        let image = vp8l_chunk(width, height, frame.low, frame.high, frame.is_high);
        let mut body = Vec::new();
        body.extend(u24(0)); // x / 2
        body.extend(u24(0)); // y / 2
        body.extend(u24(u32::from(width) - 1));
        body.extend(u24(u32::from(height) - 1));
        body.extend(u24(frame.milliseconds));
        // Not blended with the previous frame, not disposed.
        body.push(0x02);
        body.extend(image);
        chunks.extend_from_slice(b"ANMF");
        chunks.extend_from_slice(&(body.len() as u32).to_le_bytes());
        chunks.extend(body);
    }
    let mut output = b"RIFF".to_vec();
    output.extend_from_slice(&(4 + chunks.len() as u32).to_le_bytes());
    output.extend_from_slice(b"WEBP");
    output.extend(chunks);
    output
}

/// The "VP8L" chunk of a two-colour lossless image (see [`webp`]), padded to an even
/// size as RIFF requires.
fn vp8l_chunk(
    width: u16,
    height: u16,
    low: [u8; 3],
    high: [u8; 3],
    is_high: &dyn Fn(u16, u16) -> bool,
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
    let mut chunk = b"VP8L".to_vec();
    chunk.extend_from_slice(&length.to_le_bytes());
    chunk.extend(data);
    chunk
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

/// RC4 (the PDF standard security handler's cipher): `data` with the keystream of `key`.
pub fn rc4(key: &[u8], data: &[u8]) -> Vec<u8> {
    let mut state: Vec<u8> = (0..=255).collect();
    let mut j = 0_u8;
    for i in 0..256 {
        j = j.wrapping_add(state[i]).wrapping_add(key[i % key.len()]);
        state.swap(i, usize::from(j));
    }
    let (mut i, mut j) = (0_u8, 0_u8);
    data.iter()
        .map(|byte| {
            i = i.wrapping_add(1);
            j = j.wrapping_add(state[usize::from(i)]);
            state.swap(usize::from(i), usize::from(j));
            let index = state[usize::from(i)].wrapping_add(state[usize::from(j)]);
            byte ^ state[usize::from(index)]
        })
        .collect()
}

/// [`pdf`] protected by a user password with the standard security handler, revision 2
/// (40-bit RC4; ISO 32000-1 7.6.3, algorithms 1 to 4): a reader needs the password to
/// show it. The owner password is the user password.
pub fn pdf_encrypted(lines: &[&str], password: &str) -> Vec<u8> {
    const PADDING: [u8; 32] = [
        0x28, 0xBF, 0x4E, 0x5E, 0x4E, 0x75, 0x8A, 0x41, 0x64, 0x00, 0x4E, 0x56, 0xFF, 0xFA, 0x01,
        0x08, 0x2E, 0x2E, 0x00, 0xB6, 0xD0, 0x68, 0x3E, 0x80, 0x2F, 0x0C, 0xA9, 0xFE, 0x64, 0x53,
        0x69, 0x7A,
    ];
    const PERMISSIONS: i32 = -4;
    let padded = |password: &str| {
        let mut bytes: Vec<u8> = password.bytes().take(32).collect();
        bytes.extend_from_slice(&PADDING[..32 - bytes.len()]);
        bytes
    };
    let id = md5(format!("{lines:?}{password}").as_bytes());
    // Algorithm 3: the owner entry.
    let owner_key = md5(&padded(password));
    let owner = rc4(&owner_key[..5], &padded(password));
    // Algorithm 2: the file's key.
    let mut material = padded(password);
    material.extend_from_slice(&owner);
    material.extend_from_slice(&PERMISSIONS.to_le_bytes());
    material.extend_from_slice(&id);
    let key = md5(&material)[..5].to_vec();
    // Algorithm 4: the user entry.
    let user = rc4(&key, &PADDING);
    // Algorithm 1: the content stream (object 4, generation 0) is the one string data.
    let mut object_key = key.clone();
    object_key.extend_from_slice(&[4, 0, 0, 0, 0]);
    let object_key = md5(&object_key)[..10].to_vec();
    let mut content = String::from("BT /F1 14 Tf 24 160 Td 18 TL\n");
    for line in lines {
        let escaped = line
            .replace('\\', "\\\\")
            .replace('(', "\\(")
            .replace(')', "\\)");
        content.push_str(&format!("({escaped}) Tj T*\n"));
    }
    content.push_str("ET\n");
    let stream = rc4(&object_key, content.as_bytes());
    let mut objects: Vec<Vec<u8>> = vec![
        b"<< /Type /Catalog /Pages 2 0 R >>".to_vec(),
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>".to_vec(),
        b"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 300 200] /Contents 4 0 R \
          /Resources << /Font << /F1 5 0 R >> >> >>"
            .to_vec(),
    ];
    let mut contents = format!("<< /Length {} >>\nstream\n", stream.len()).into_bytes();
    contents.extend_from_slice(&stream);
    contents.extend_from_slice(b"\nendstream");
    objects.push(contents);
    objects.push(b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>".to_vec());
    objects.push(
        format!(
            "<< /Filter /Standard /V 1 /R 2 /Length 40 /O <{}> /U <{}> /P {PERMISSIONS} >>",
            hex(&owner),
            hex(&user)
        )
        .into_bytes(),
    );
    let mut output = b"%PDF-1.4\n%\xE2\xE3\xCF\xD3\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.iter().enumerate() {
        offsets.push(output.len());
        output.extend_from_slice(format!("{} 0 obj\n", index + 1).as_bytes());
        output.extend_from_slice(object);
        output.extend_from_slice(b"\nendobj\n");
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
            "trailer\n<< /Size {} /Root 1 0 R /Encrypt 6 0 R /ID [<{id}> <{id}>] >>\n\
             startxref\n{xref}\n%%EOF\n",
            objects.len() + 1,
            id = hex(&id)
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
    let entries: Vec<Entry> = members.iter().map(Entry::from).collect();
    zip_entries(&entries)
}

// ---------------------------------------------------------------------------------------
// ZIP members with storage options: Deflate, ZipCrypto, WinZip AES and extra fields
// (PKWARE APPNOTE 6.3.10; WinZip's "AES Encryption Information" for AE-1 and AE-2)

/// How a ZIP member's bytes are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// Method 0.
    Stored,
    /// Method 8, written as stored Deflate blocks: valid Deflate, no compression.
    Deflated,
}

impl Method {
    fn number(self) -> u16 {
        match self {
            Self::Stored => 0,
            Self::Deflated => 8,
        }
    }

    fn compress(self, data: &[u8]) -> Vec<u8> {
        match self {
            Self::Stored => data.to_vec(),
            Self::Deflated => deflate_stored(data),
        }
    }
}

/// A WinZip AES key strength (its "AES encryption strength" codes 1, 2 and 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AesStrength {
    Aes128,
    Aes192,
    Aes256,
}

impl AesStrength {
    fn key_len(self) -> usize {
        match self {
            Self::Aes128 => 16,
            Self::Aes192 => 24,
            Self::Aes256 => 32,
        }
    }

    /// The salt is half as long as the key.
    fn salt_len(self) -> usize {
        self.key_len() / 2
    }

    fn code(self) -> u8 {
        match self {
            Self::Aes128 => 1,
            Self::Aes192 => 2,
            Self::Aes256 => 3,
        }
    }
}

/// A ZIP member's encryption. Passwords are UTF-8, as current writers encode them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Lock {
    None,
    /// Traditional PKWARE encryption, "ZipCrypto" (APPNOTE 6.1).
    ZipCrypto(String),
    /// WinZip AES: AE-2 stores no CRC (its authentication code checks the data); AE-1
    /// (`ae1`) stores the CRC as well.
    Aes {
        strength: AesStrength,
        password: String,
        ae1: bool,
    },
}

/// One member of [`zip_entries`] with its storage options; `Entry::from(&Member)` is a
/// member as [`zip`] stores it.
#[derive(Clone, Debug)]
pub struct Entry {
    /// The member's name, `/`-separated; a folder ends with `/`.
    pub path: String,
    /// The member's bytes (a symbolic link's: its target); `None` for a folder.
    pub data: Option<Vec<u8>>,
    pub method: Method,
    pub lock: Lock,
    /// Extra-field records ([`extra_record`], [`growth_hint`]) of the local header and
    /// of the central directory header.
    pub local_extra: Vec<u8>,
    pub central_extra: Vec<u8>,
    /// Flag bits besides UTF-8 names (bit 11) and encryption (bit 0): for example 6,
    /// the Deflate options Microsoft Office sets, or 0x40, strong encryption.
    pub flags: u16,
    /// A method number written instead of `method`'s, over `method`'s bytes: for
    /// example 6 (Implode), which no current reader decompresses.
    pub method_number: Option<u16>,
    /// The Unix type and mode in the external attributes instead of a regular file's
    /// 0o100644 or a folder's 0o040755: 0o120777 makes a symbolic link.
    pub mode: Option<u32>,
}

impl From<&Member> for Entry {
    fn from(member: &Member) -> Self {
        Self {
            path: member.path.clone(),
            data: member.data.clone(),
            method: Method::Stored,
            lock: Lock::None,
            local_extra: Vec::new(),
            central_extra: Vec::new(),
            flags: 0,
            method_number: None,
            mode: None,
        }
    }
}

impl Entry {
    /// A stored, unencrypted file.
    pub fn file(path: &str, data: impl Into<Vec<u8>>) -> Self {
        Self::from(&Member::file(path, data))
    }

    /// A folder.
    pub fn directory(path: &str) -> Self {
        Self::from(&Member::directory(path))
    }

    pub fn method(mut self, method: Method) -> Self {
        self.method = method;
        self
    }

    pub fn lock(mut self, lock: Lock) -> Self {
        self.lock = lock;
        self
    }

    /// ZipCrypto with `password`.
    pub fn zip_crypto(self, password: &str) -> Self {
        self.lock(Lock::ZipCrypto(password.to_owned()))
    }

    /// WinZip AES (AE-2) of `strength` with `password`.
    pub fn aes(self, strength: AesStrength, password: &str) -> Self {
        self.lock(Lock::Aes {
            strength,
            password: password.to_owned(),
            ae1: false,
        })
    }
}

/// One extra-field record: header ID, data size, data (APPNOTE 4.5).
pub fn extra_record(id: u16, data: &[u8]) -> Vec<u8> {
    let mut record = id.to_le_bytes().to_vec();
    record.extend_from_slice(&(data.len() as u16).to_le_bytes());
    record.extend_from_slice(data);
    record
}

/// Microsoft's growth hint, record 0xA220, as Office writes it before the first parts of
/// a package so they can grow in place: signature 0xA028, the padding's length, then
/// `padding` zero bytes.
pub fn growth_hint(padding: u16) -> Vec<u8> {
    let mut data = 0xA028_u16.to_le_bytes().to_vec();
    data.extend_from_slice(&padding.to_le_bytes());
    data.resize(4 + usize::from(padding), 0);
    extra_record(0xA220, &data)
}

/// Deterministic stand-ins for the random bytes a ZIP writer draws (ZipCrypto's
/// encryption header, a WinZip AES salt): the same member always gets the same bytes,
/// so fixtures stay byte-identical from run to run.
fn seeded_bytes(seed: &[u8], length: usize) -> Vec<u8> {
    let mut state = seed.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, &byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    });
    (0..length)
        .map(|_| {
            // xorshift64*
            state ^= state >> 12;
            state ^= state << 25;
            state ^= state >> 27;
            (state.wrapping_mul(0x2545_f491_4f6c_dd1d) >> 56) as u8
        })
        .collect()
}

/// Traditional PKWARE encryption's three keys (APPNOTE 6.1.5).
struct ZipCryptoKeys([u32; 3]);

/// One step of the CRC-32 register without its initial and final inversion, as
/// APPNOTE 6.1 defines `crc32(pval, char)`.
fn crc32_step(crc: u32, byte: u8) -> u32 {
    let mut crc = crc ^ u32::from(byte);
    for _ in 0..8 {
        crc = if crc & 1 != 0 {
            (crc >> 1) ^ 0xEDB8_8320
        } else {
            crc >> 1
        };
    }
    crc
}

impl ZipCryptoKeys {
    fn new(password: &[u8]) -> Self {
        let mut keys = Self([0x1234_5678, 0x2345_6789, 0x3456_7890]);
        for &byte in password {
            keys.update(byte);
        }
        keys
    }

    fn update(&mut self, byte: u8) {
        let [zero, one, two] = &mut self.0;
        *zero = crc32_step(*zero, byte);
        *one = one
            .wrapping_add(*zero & 0xFF)
            .wrapping_mul(134_775_813)
            .wrapping_add(1);
        *two = crc32_step(*two, (*one >> 24) as u8);
    }

    /// The keystream byte: `temp = key2 | 2` as a 16-bit value, `(temp * (temp ^ 1)) >> 8`.
    fn stream(&self) -> u8 {
        let temp = (self.0[2] | 2) & 0xFFFF;
        ((temp * (temp ^ 1)) >> 8) as u8
    }

    fn encrypt(&mut self, plain: u8) -> u8 {
        let cipher = plain ^ self.stream();
        self.update(plain);
        cipher
    }
}

/// ZipCrypto (APPNOTE 6.1.6): a 12-byte encryption header whose last byte is the high
/// byte of the CRC (no data descriptor follows), then `stored` (the member's bytes as
/// stored), all encrypted. `header` gives the header's first 11 bytes.
pub fn zip_crypto_encrypt(password: &[u8], crc: u32, header: &[u8; 11], stored: &[u8]) -> Vec<u8> {
    let mut keys = ZipCryptoKeys::new(password);
    let mut plain = header.to_vec();
    plain.push((crc >> 24) as u8);
    plain.extend_from_slice(stored);
    plain.into_iter().map(|byte| keys.encrypt(byte)).collect()
}

/// A ZIP of `entries` with a fixed 2026-01-02 date: local headers, the central
/// directory, the end record. Names are UTF-8 (flag bit 11).
pub fn zip_entries(entries: &[Entry]) -> Vec<u8> {
    const TIME: u16 = 12 << 11;
    const DATE: u16 = ((2026 - 1980) << 9) | (1 << 5) | 2;
    let mut output = Vec::new();
    let mut central = Vec::new();
    for entry in entries {
        let data = entry.data.as_deref().unwrap_or_default();
        let crc = crc32(data);
        let name = entry.path.as_bytes();
        let stored = entry.method.compress(data);
        let mut flags = 0x0800 | entry.flags;
        let mut version = 20_u16;
        let mut method = entry.method_number.unwrap_or(entry.method.number());
        let mut stored_crc = crc;
        let mut local_extra = entry.local_extra.clone();
        let mut central_extra = entry.central_extra.clone();
        let stored = match &entry.lock {
            Lock::None => stored,
            Lock::ZipCrypto(password) => {
                flags |= 1;
                let header: [u8; 11] = seeded_bytes(name, 11)
                    .try_into()
                    .expect("eleven header bytes");
                zip_crypto_encrypt(password.as_bytes(), crc, &header, &stored)
            }
            Lock::Aes {
                strength,
                password,
                ae1,
            } => {
                flags |= 1;
                version = 51;
                // The real method moves into the AES record; the headers say 99.
                let record = aes_record(*strength, *ae1, method);
                local_extra.extend_from_slice(&record);
                central_extra.extend_from_slice(&record);
                method = 99;
                if !ae1 {
                    stored_crc = 0;
                }
                let salt = seeded_bytes(name, strength.salt_len());
                winzip_aes_encrypt(password.as_bytes(), *strength, &salt, &stored)
            }
        };
        let offset = output.len() as u32;
        let header = |signature: u32, central: bool, extra: &[u8]| {
            let mut header = Vec::new();
            header.extend_from_slice(&signature.to_le_bytes());
            if central {
                header.extend_from_slice(&(0x0300_u16 | version).to_le_bytes());
            }
            header.extend_from_slice(&version.to_le_bytes());
            header.extend_from_slice(&flags.to_le_bytes());
            header.extend_from_slice(&method.to_le_bytes());
            header.extend_from_slice(&TIME.to_le_bytes());
            header.extend_from_slice(&DATE.to_le_bytes());
            header.extend_from_slice(&stored_crc.to_le_bytes());
            header.extend_from_slice(&(stored.len() as u32).to_le_bytes());
            header.extend_from_slice(&(data.len() as u32).to_le_bytes());
            header.extend_from_slice(&(name.len() as u16).to_le_bytes());
            header.extend_from_slice(&(extra.len() as u16).to_le_bytes());
            header
        };
        output.extend(header(0x0403_4B50, false, &local_extra));
        output.extend_from_slice(name);
        output.extend_from_slice(&local_extra);
        output.extend_from_slice(&stored);
        let mut record = header(0x0201_4B50, true, &central_extra);
        // Comment length, disk number, internal attributes.
        record.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
        let mode: u32 = match (entry.mode, &entry.data) {
            (Some(mode), Some(_)) => mode << 16,
            (Some(mode), None) => (mode << 16) | 0x10,
            (None, Some(_)) => 0o100_644 << 16,
            (None, None) => (0o040_755 << 16) | 0x10,
        };
        record.extend_from_slice(&mode.to_le_bytes());
        record.extend_from_slice(&offset.to_le_bytes());
        record.extend_from_slice(name);
        record.extend_from_slice(&central_extra);
        central.extend(record);
    }
    let central_offset = output.len() as u32;
    let central_size = central.len() as u32;
    output.extend(central);
    output.extend_from_slice(&0x0605_4B50_u32.to_le_bytes());
    output.extend_from_slice(&[0, 0, 0, 0]);
    let count = entries.len() as u16;
    output.extend_from_slice(&count.to_le_bytes());
    output.extend_from_slice(&count.to_le_bytes());
    output.extend_from_slice(&central_size.to_le_bytes());
    output.extend_from_slice(&central_offset.to_le_bytes());
    output.extend_from_slice(&0_u16.to_le_bytes());
    output
}

/// An Office Open XML document kind (ECMA-376).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Office {
    /// A Word document: one paragraph.
    Docx,
    /// A PowerPoint presentation: one slide with one text box.
    Pptx,
    /// An Excel workbook: one sheet with one inline-string cell.
    Xlsx,
}

/// How [`ooxml`] stores a package's parts.
#[derive(Clone, Debug)]
pub struct OfficeStorage {
    /// Microsoft's growth hints (0xA220) on the first two parts' local headers, 512
    /// and 256 bytes, and an extended timestamp record (0x5455) on every header, as
    /// Office and common ZIP tools write them.
    pub padded: bool,
    /// Every part's encryption: a ZipCrypto or AES package is password-protected.
    pub lock: Lock,
}

fn xml_text(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// A minimal Open Packaging Conventions package (ECMA-376 part 2: content types and
/// relationships, part 1: the main part) whose one paragraph, text box or cell holds
/// `text`: parts deflated (stored Deflate blocks) with the speed option bits Office sets.
pub fn ooxml(kind: Office, text: &str, storage: &OfficeStorage) -> Vec<u8> {
    const PACKAGE: &str = "http://schemas.openxmlformats.org/package/2006/relationships";
    const DOCUMENT: &str = "http://schemas.openxmlformats.org/officeDocument/2006/relationships";
    const DRAWING: &str = "http://schemas.openxmlformats.org/drawingml/2006/main";
    const OFFICE_TYPE: &str = "application/vnd.openxmlformats-officedocument";
    let text = xml_text(text);
    let declaration = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>"#;
    let (main, main_type, extra_parts): (&str, &str, Vec<(String, String, String)>) = match kind {
        Office::Docx => (
            "word/document.xml",
            "wordprocessingml.document.main+xml",
            vec![(
                "word/document.xml".into(),
                format!(
                    r#"{declaration}<w:document xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"><w:body><w:p><w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p></w:body></w:document>"#
                ),
                String::new(),
            )],
        ),
        Office::Pptx => (
            "ppt/presentation.xml",
            "presentationml.presentation.main+xml",
            vec![
                (
                    "ppt/presentation.xml".into(),
                    format!(
                        r#"{declaration}<p:presentation xmlns:a="{DRAWING}" xmlns:r="{DOCUMENT}" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="9144000" cy="6858000"/><p:notesSz cx="6858000" cy="9144000"/></p:presentation>"#
                    ),
                    String::new(),
                ),
                (
                    "ppt/_rels/presentation.xml.rels".into(),
                    format!(
                        r#"{declaration}<Relationships xmlns="{PACKAGE}"><Relationship Id="rId2" Type="{DOCUMENT}/slide" Target="slides/slide1.xml"/></Relationships>"#
                    ),
                    String::new(),
                ),
                (
                    "ppt/slides/slide1.xml".into(),
                    format!(
                        r#"{declaration}<p:sld xmlns:a="{DRAWING}" xmlns:r="{DOCUMENT}" xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main"><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Text 1"/><p:cNvSpPr txBox="1"/><p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="7315200" cy="1371600"/></a:xfrm></p:spPr><p:txBody><a:bodyPr/><a:p><a:r><a:t>{text}</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
                    ),
                    format!("{OFFICE_TYPE}.presentationml.slide+xml"),
                ),
            ],
        ),
        Office::Xlsx => (
            "xl/workbook.xml",
            "spreadsheetml.sheet.main+xml",
            vec![
                (
                    "xl/workbook.xml".into(),
                    format!(
                        r#"{declaration}<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="{DOCUMENT}"><sheets><sheet name="Sheet1" sheetId="1" r:id="rId1"/></sheets></workbook>"#
                    ),
                    String::new(),
                ),
                (
                    "xl/_rels/workbook.xml.rels".into(),
                    format!(
                        r#"{declaration}<Relationships xmlns="{PACKAGE}"><Relationship Id="rId1" Type="{DOCUMENT}/worksheet" Target="worksheets/sheet1.xml"/></Relationships>"#
                    ),
                    String::new(),
                ),
                (
                    "xl/worksheets/sheet1.xml".into(),
                    format!(
                        r#"{declaration}<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>{text}</t></is></c></row></sheetData></worksheet>"#
                    ),
                    format!("{OFFICE_TYPE}.spreadsheetml.worksheet+xml"),
                ),
            ],
        ),
    };
    let overrides: String = extra_parts
        .iter()
        .filter(|(_, _, content_type)| !content_type.is_empty())
        .map(|(name, _, content_type)| {
            format!(r#"<Override PartName="/{name}" ContentType="{content_type}"/>"#)
        })
        .collect();
    let mut parts = vec![
        (
            "[Content_Types].xml".to_owned(),
            format!(
                r#"{declaration}<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/{main}" ContentType="{OFFICE_TYPE}.{main_type}"/>{overrides}<Override PartName="/docProps/core.xml" ContentType="application/vnd.openxmlformats-package.core-properties+xml"/></Types>"#
            ),
        ),
        (
            "_rels/.rels".to_owned(),
            format!(
                r#"{declaration}<Relationships xmlns="{PACKAGE}"><Relationship Id="rId1" Type="{DOCUMENT}/officeDocument" Target="{main}"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/package/2006/relationships/metadata/core-properties" Target="docProps/core.xml"/></Relationships>"#
            ),
        ),
    ];
    parts.extend(extra_parts.into_iter().map(|(name, xml, _)| (name, xml)));
    parts.push((
        "docProps/core.xml".to_owned(),
        format!(
            r#"{declaration}<cp:coreProperties xmlns:cp="http://schemas.openxmlformats.org/package/2006/metadata/core-properties" xmlns:dc="http://purl.org/dc/elements/1.1/"><dc:title>Acceptance fixture</dc:title></cp:coreProperties>"#
        ),
    ));
    let entries: Vec<Entry> = parts
        .into_iter()
        .enumerate()
        .map(|(index, (name, xml))| {
            let mut entry = Entry::file(&name, xml)
                .method(Method::Deflated)
                .lock(storage.lock.clone());
            entry.flags = 6;
            if storage.padded {
                // Extended timestamp: modification time only (2026-01-02 12:00 UTC).
                let mut stamp = vec![1];
                stamp.extend_from_slice(&1_767_355_200_u32.to_le_bytes());
                let stamp = extra_record(0x5455, &stamp);
                entry.local_extra = match index {
                    0 => growth_hint(512),
                    1 => growth_hint(256),
                    _ => Vec::new(),
                };
                entry.local_extra.extend_from_slice(&stamp);
                entry.central_extra = stamp;
            }
            entry
        })
        .collect();
    zip_entries(&entries)
}

/// The start of an OLE compound file (MS-CFB): what a password-protected Office document
/// (an encrypted package) or a legacy .doc, .xls or .ppt begins with.
pub fn compound_file_start() -> Vec<u8> {
    let mut bytes = vec![0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
    // Minor and major version 3, little-endian byte order, 512-byte sectors.
    bytes.resize(24, 0);
    bytes.extend_from_slice(&[0x3E, 0x00, 0x03, 0x00, 0xFE, 0xFF, 0x09, 0x00]);
    bytes.resize(512, 0);
    bytes
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

/// SHA-1 (FIPS 180-4), for HMAC-SHA1 and PBKDF2 of WinZip AES.
pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut state: [u32; 5] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];
    let mut message = data.to_vec();
    message.push(0x80);
    while message.len() % 64 != 56 {
        message.push(0);
    }
    message.extend_from_slice(&(data.len() as u64 * 8).to_be_bytes());
    for block in message.chunks(64) {
        let mut w = [0_u32; 80];
        for (index, word) in block.chunks(4).enumerate() {
            w[index] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for index in 16..80 {
            w[index] = (w[index - 3] ^ w[index - 8] ^ w[index - 14] ^ w[index - 16]).rotate_left(1);
        }
        let [mut a, mut b, mut c, mut d, mut e] = state;
        for (index, word) in w.iter().enumerate() {
            let (mix, constant) = match index / 20 {
                0 => ((b & c) | (!b & d), 0x5a827999),
                1 => (b ^ c ^ d, 0x6ed9eba1),
                2 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let next = a
                .rotate_left(5)
                .wrapping_add(mix)
                .wrapping_add(e)
                .wrapping_add(constant)
                .wrapping_add(*word);
            (a, b, c, d, e) = (next, a, b.rotate_left(30), c, d);
        }
        for (word, value) in state.iter_mut().zip([a, b, c, d, e]) {
            *word = word.wrapping_add(value);
        }
    }
    let mut digest = [0_u8; 20];
    for (bytes, word) in digest.chunks_mut(4).zip(state) {
        bytes.copy_from_slice(&word.to_be_bytes());
    }
    digest
}

/// HMAC-SHA1 (RFC 2104).
pub fn hmac_sha1(key: &[u8], message: &[u8]) -> [u8; 20] {
    let mut block = [0_u8; 64];
    if key.len() > block.len() {
        block[..20].copy_from_slice(&sha1(key));
    } else {
        block[..key.len()].copy_from_slice(key);
    }
    let mut inner: Vec<u8> = block.iter().map(|byte| byte ^ 0x36).collect();
    inner.extend_from_slice(message);
    let mut outer: Vec<u8> = block.iter().map(|byte| byte ^ 0x5c).collect();
    outer.extend_from_slice(&sha1(&inner));
    sha1(&outer)
}

/// PBKDF2 with HMAC-SHA1 (RFC 8018), `length` bytes.
pub fn pbkdf2_hmac_sha1(password: &[u8], salt: &[u8], rounds: u32, length: usize) -> Vec<u8> {
    let mut output = Vec::with_capacity(length + 20);
    let mut index = 1_u32;
    while output.len() < length {
        let mut first = salt.to_vec();
        first.extend_from_slice(&index.to_be_bytes());
        let mut round = hmac_sha1(password, &first);
        let mut block = round;
        for _ in 1..rounds {
            round = hmac_sha1(password, &round);
            for (sum, byte) in block.iter_mut().zip(round) {
                *sum ^= byte;
            }
        }
        output.extend_from_slice(&block);
        index += 1;
    }
    output.truncate(length);
    output
}

/// AES (FIPS-197) block encryption with a 128, 192 or 256-bit key: all that CTR mode,
/// which WinZip AES uses, needs.
pub struct Aes {
    sbox: [u8; 256],
    round_keys: Vec<[u8; 16]>,
}

/// Multiplication by x in GF(2^8) modulo x^8 + x^4 + x^3 + x + 1.
fn xtime(byte: u8) -> u8 {
    (byte << 1) ^ if byte & 0x80 != 0 { 0x1b } else { 0 }
}

impl Aes {
    pub fn new(key: &[u8]) -> Self {
        assert!(
            matches!(key.len(), 16 | 24 | 32),
            "AES keys are 16, 24 or 32 bytes"
        );
        // The S-box: the multiplicative inverse in GF(2^8), then the affine map. p walks
        // the field's elements as powers of 3, q the matching powers of its inverse.
        let mut sbox = [0_u8; 256];
        let (mut p, mut q) = (1_u8, 1_u8);
        loop {
            p ^= xtime(p);
            q ^= q << 1;
            q ^= q << 2;
            q ^= q << 4;
            if q & 0x80 != 0 {
                q ^= 0x09;
            }
            sbox[usize::from(p)] = q
                ^ q.rotate_left(1)
                ^ q.rotate_left(2)
                ^ q.rotate_left(3)
                ^ q.rotate_left(4)
                ^ 0x63;
            if p == 1 {
                break;
            }
        }
        sbox[0] = 0x63;
        // Key expansion: Nk key words, Nr = Nk + 6 rounds, 4 (Nr + 1) words.
        let words = key.len() / 4;
        let rounds = words + 6;
        let mut schedule: Vec<[u8; 4]> = key
            .chunks(4)
            .map(|word| [word[0], word[1], word[2], word[3]])
            .collect();
        let mut constant = 1_u8;
        for index in words..4 * (rounds + 1) {
            let mut word = schedule[index - 1];
            if index % words == 0 {
                word = [
                    sbox[usize::from(word[1])] ^ constant,
                    sbox[usize::from(word[2])],
                    sbox[usize::from(word[3])],
                    sbox[usize::from(word[0])],
                ];
                constant = xtime(constant);
            } else if words > 6 && index % words == 4 {
                word = word.map(|byte| sbox[usize::from(byte)]);
            }
            let earlier = schedule[index - words];
            schedule.push([
                earlier[0] ^ word[0],
                earlier[1] ^ word[1],
                earlier[2] ^ word[2],
                earlier[3] ^ word[3],
            ]);
        }
        let round_keys = schedule
            .chunks(4)
            .map(|words| {
                let mut key = [0_u8; 16];
                for (target, word) in key.chunks_mut(4).zip(words) {
                    target.copy_from_slice(word);
                }
                key
            })
            .collect();
        Self { sbox, round_keys }
    }

    pub fn encrypt_block(&self, block: &[u8; 16]) -> [u8; 16] {
        // The state in input order: row r of column c is byte r + 4c.
        let add = |state: &mut [u8; 16], key: &[u8; 16]| {
            for (byte, key) in state.iter_mut().zip(key) {
                *byte ^= key;
            }
        };
        let substitute_and_shift = |state: &[u8; 16]| {
            let mut next = [0_u8; 16];
            for column in 0..4 {
                for row in 0..4 {
                    next[row + 4 * column] =
                        self.sbox[usize::from(state[row + 4 * ((column + row) % 4)])];
                }
            }
            next
        };
        let mut state = *block;
        add(&mut state, &self.round_keys[0]);
        let last = self.round_keys.len() - 1;
        for key in &self.round_keys[1..last] {
            state = substitute_and_shift(&state);
            for column in state.chunks_mut(4) {
                let [a0, a1, a2, a3] = [column[0], column[1], column[2], column[3]];
                column[0] = xtime(a0) ^ xtime(a1) ^ a1 ^ a2 ^ a3;
                column[1] = a0 ^ xtime(a1) ^ xtime(a2) ^ a2 ^ a3;
                column[2] = a0 ^ a1 ^ xtime(a2) ^ xtime(a3) ^ a3;
                column[3] = xtime(a0) ^ a0 ^ a1 ^ a2 ^ xtime(a3);
            }
            add(&mut state, key);
        }
        state = substitute_and_shift(&state);
        add(&mut state, &self.round_keys[last]);
        state
    }
}

/// WinZip's AES extra-field record 0x9901: the vendor version (1 for AE-1, 2 for AE-2),
/// the vendor ID "AE", the key strength and the member's real compression method.
fn aes_record(strength: AesStrength, ae1: bool, method: u16) -> Vec<u8> {
    let mut data = (if ae1 { 1_u16 } else { 2 }).to_le_bytes().to_vec();
    data.extend_from_slice(b"AE");
    data.push(strength.code());
    data.extend_from_slice(&method.to_le_bytes());
    extra_record(0x9901, &data)
}

/// WinZip AES: PBKDF2-HMAC-SHA1 with 1,000 rounds derives the AES key, the HMAC key and
/// a two-byte password verifier from the password and `salt` (half the key's length);
/// `stored` is encrypted in AES-CTR with a little-endian counter starting at 1, and the
/// first 10 bytes of HMAC-SHA1 over the encrypted bytes follow them. Returns the salt,
/// the verifier, the encrypted bytes and that authentication code.
pub fn winzip_aes_encrypt(
    password: &[u8],
    strength: AesStrength,
    salt: &[u8],
    stored: &[u8],
) -> Vec<u8> {
    let length = strength.key_len();
    assert_eq!(salt.len(), strength.salt_len(), "a WinZip AES salt");
    let derived = pbkdf2_hmac_sha1(password, salt, 1000, 2 * length + 2);
    let (key, rest) = derived.split_at(length);
    let (authentication, verifier) = rest.split_at(length);
    let cipher = Aes::new(key);
    let mut counter = [0_u8; 16];
    let mut encrypted = Vec::with_capacity(stored.len());
    for block in stored.chunks(16) {
        for byte in &mut counter {
            *byte = byte.wrapping_add(1);
            if *byte != 0 {
                break;
            }
        }
        let stream = cipher.encrypt_block(&counter);
        encrypted.extend(block.iter().zip(stream).map(|(byte, key)| byte ^ key));
    }
    let code = hmac_sha1(authentication, &encrypted);
    let mut output = salt.to_vec();
    output.extend_from_slice(verifier);
    output.extend(encrypted);
    output.extend_from_slice(&code[..10]);
    output
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&text[index..index + 2], 16).expect("hex digits"))
        .collect()
}

/// Known-answer checks of the digests, the CRC, the ciphers and the ZIP writer; run
/// before fixtures are written. The digest, HMAC, PBKDF2 and AES vectors are the
/// standards' own (FIPS 180-4 examples, RFC 1321, RFC 2202, RFC 6070, FIPS-197
/// appendix C); the ZIP vectors were checked against independent readers (see each).
pub fn self_test() -> Result<(), String> {
    let block = |key: &str, plain: &str| {
        let plain: [u8; 16] = unhex(plain).try_into().expect("one block");
        hex(&Aes::new(&unhex(key)).encrypt_block(&plain))
    };
    let checks = [
        (
            "SHA-256 abc",
            hex(&sha256(b"abc")),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad",
        ),
        (
            "SHA-256 empty",
            hex(&sha256(b"")),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855",
        ),
        (
            "MD5 empty",
            hex(&md5(b"")),
            "d41d8cd98f00b204e9800998ecf8427e",
        ),
        (
            "MD5 fox",
            hex(&md5(b"The quick brown fox jumps over the lazy dog")),
            "9e107d9d372bb6826bd81d3542a419d6",
        ),
        ("CRC-32", format!("{:08x}", crc32(b"123456789")), "cbf43926"),
        (
            "SHA-1 abc",
            hex(&sha1(b"abc")),
            "a9993e364706816aba3e25717850c26c9cd0d89d",
        ),
        (
            "SHA-1 empty",
            hex(&sha1(b"")),
            "da39a3ee5e6b4b0d3255bfef95601890afd80709",
        ),
        (
            "SHA-1 two blocks",
            hex(&sha1(
                b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq",
            )),
            "84983e441c3bd26ebaae4aa1f95129e5e54670f1",
        ),
        (
            "HMAC-SHA1 RFC 2202 case 1",
            hex(&hmac_sha1(&[0x0b; 20], b"Hi There")),
            "b617318655057264e28bc0b6fb378c8ef146be00",
        ),
        (
            "HMAC-SHA1 RFC 2202 case 2",
            hex(&hmac_sha1(b"Jefe", b"what do ya want for nothing?")),
            "effcdf6ae5eb2fa2d27416d5f184df9c259a7c79",
        ),
        (
            "HMAC-SHA1 RFC 2202 case 6 (a key longer than a block)",
            hex(&hmac_sha1(
                &[0xaa; 80],
                b"Test Using Larger Than Block-Size Key - Hash Key First",
            )),
            "aa4ae5e15272d00e95705637ce8a3b55ed402112",
        ),
        (
            "PBKDF2-HMAC-SHA1 RFC 6070, 1 round",
            hex(&pbkdf2_hmac_sha1(b"password", b"salt", 1, 20)),
            "0c60c80f961f0e71f3a9b524af6012062fe037a6",
        ),
        (
            "PBKDF2-HMAC-SHA1 RFC 6070, 2 rounds",
            hex(&pbkdf2_hmac_sha1(b"password", b"salt", 2, 20)),
            "ea6c014dc72d6f8ccd1ed92ace1d41f0d8de8957",
        ),
        (
            "PBKDF2-HMAC-SHA1 RFC 6070, 4,096 rounds",
            hex(&pbkdf2_hmac_sha1(b"password", b"salt", 4096, 20)),
            "4b007901b765489abead49d926f721d065a429c1",
        ),
        (
            "PBKDF2-HMAC-SHA1 RFC 6070, 25 bytes",
            hex(&pbkdf2_hmac_sha1(
                b"passwordPASSWORDpassword",
                b"saltSALTsaltSALTsaltSALTsaltSALTsalt",
                4096,
                25,
            )),
            "3d2eec4fe41c849b80c8d83662c0e44a8b291a964cf2f07038",
        ),
        (
            "RC4 Key",
            hex(&rc4(b"Key", b"Plaintext")),
            "bbf316e8d940af0ad3",
        ),
        ("RC4 Wiki", hex(&rc4(b"Wiki", b"pedia")), "1021bf0420"),
        (
            "RC4 Secret",
            hex(&rc4(b"Secret", b"Attack at dawn")),
            "45a01f645fc35b383552544b9bf5",
        ),
        (
            "AES-128 FIPS-197 C.1",
            block(
                "000102030405060708090a0b0c0d0e0f",
                "00112233445566778899aabbccddeeff",
            ),
            "69c4e0d86a7b0430d8cdb78070b4c55a",
        ),
        (
            "AES-192 FIPS-197 C.2",
            block(
                "000102030405060708090a0b0c0d0e0f1011121314151617",
                "00112233445566778899aabbccddeeff",
            ),
            "dda97ca4864cdfe06eaf70a0ec0d7191",
        ),
        (
            "AES-256 FIPS-197 C.3",
            block(
                "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
                "00112233445566778899aabbccddeeff",
            ),
            "8ea2b7ca516745bfeafc49904b496089",
        ),
        // The plain writer's output is unchanged by its options (pinned before they came).
        (
            "ZIP of stored members",
            hex(&sha256(&zip(&[
                Member::directory("bundle"),
                Member::file("bundle/readme.txt", "Inside the bundle.\n"),
                Member::file("ünïcödé 🦀.txt", "Unicode member.\n"),
                Member::directory("bundle/empty"),
            ]))),
            "6354021388ff04e20be8eea5e2c9fe76f3f1cbf54061c44b388a12f8fb15f184",
        ),
        (
            "empty ZIP",
            hex(&sha256(&zip(&[]))),
            "8739c76e681f900923b900c9df0ef75cf421d39cabb54650c4b9ad19b6a76d85",
        ),
        // The still GIF and WebP writers are unchanged by the animated ones.
        (
            "GIF",
            hex(&sha256(&gif(48, 48, &test_palette(), |x, y| {
                ((x / 6 + y / 6) * 32 % 256) as u8
            }))),
            "f34f740541f2d8556012948c089dd9de9b82376c5e2fcb12d26b568bf76cb95f",
        ),
        (
            "WebP",
            hex(&sha256(&webp(
                64,
                64,
                [40, 60, 90],
                [230, 200, 120],
                |x, y| (x / 8 + y / 8) % 2 == 0,
            ))),
            "b950cdd683f00d9bff323d29dd0c6e34a3a72466b537177dd546531c231a4ff1",
        ),
        // Two frames, red then blue: Windows' own GIF and WebP decoders (WIC) read both
        // frames with these colours.
        (
            "animated GIF",
            hex(&sha256(&gif_animated(
                4,
                4,
                &two_colours(),
                50,
                &[&|_, _| 1, &|_, _| 2],
            ))),
            "2dcc9584ad652a31961e7fa33613b23c563bccfae2dfc0cf4d327f1249b23cdc",
        ),
        (
            "animated WebP",
            hex(&sha256(&webp_animated(
                4,
                4,
                &[
                    WebpFrame {
                        low: [200, 40, 40],
                        high: [200, 40, 40],
                        is_high: &|_, _| true,
                        milliseconds: 500,
                    },
                    WebpFrame {
                        low: [40, 40, 200],
                        high: [40, 40, 200],
                        is_high: &|_, _| true,
                        milliseconds: 500,
                    },
                ],
            ))),
            "c03a7a65897318d9ff4efdb00bcc86ef4424ce0eec2ff18bd0e5dbaa1d169f0e",
        ),
        // A padded presentation: Python's zipfile checks every part's CRC and its XML
        // parser reads every part; the first local header carries 529 bytes of extra
        // field (a 512-byte growth hint and a timestamp).
        (
            "OOXML package",
            hex(&sha256(&ooxml(
                Office::Pptx,
                "Padded",
                &OfficeStorage {
                    padded: true,
                    lock: Lock::None,
                },
            ))),
            "2b52021ceddef015f59691ed7cc6364116f2c9728bd17e57c5a8d4c7c5b7fcdb",
        ),
        // Windows.Data.Pdf (an independent reader) refuses this document without its
        // password or with a wrong one and opens its one page with it.
        (
            "password-protected PDF",
            hex(&sha256(&pdf_encrypted(
                &["Locked PDF fixture", "Second line"],
                "pdf-heslo",
            ))),
            "777c18f5ee245cf9cf60099ff41453c67bc1c6f9680042cdb39fb04f0bda0de5",
        ),
        // ZipCrypto: Python's zipfile (an independent reader) decrypts this member with
        // its CRC verified; the header bytes are APPNOTE's 11 + the CRC's high byte.
        (
            "ZipCrypto",
            hex(&zip_crypto_encrypt(
                "Kyrandia-heslo-🔑".as_bytes(),
                crc32(b"Inside the bundle.\n"),
                b"\x01\x02\x03\x04\x05\x06\x07\x08\x09\x0a\x0b",
                b"Inside the bundle.\n",
            )),
            ZIP_CRYPTO_VECTOR,
        ),
        // WinZip AES-256: the `zip` crate's reader (an independent implementation)
        // decrypts and authenticates this member.
        (
            "WinZip AES-256 (AE-2)",
            hex(&winzip_aes_encrypt(
                "Kyrandia-heslo-🔑".as_bytes(),
                AesStrength::Aes256,
                b"0123456789abcdef",
                b"Inside the bundle.\n",
            )),
            WINZIP_AES_VECTOR,
        ),
    ];
    for (what, actual, expected) in checks {
        if actual != expected {
            return Err(format!("{what} self-test failed: {actual} != {expected}"));
        }
    }
    Ok(())
}

/// A 256-colour palette for [`self_test`]'s GIF.
fn test_palette() -> [[u8; 3]; 256] {
    let mut palette = [[0_u8; 3]; 256];
    for (index, colour) in palette.iter_mut().enumerate() {
        *colour = [index as u8, (255 - index) as u8, (index * 7 % 256) as u8];
    }
    palette
}

/// A palette whose index 1 is red and index 2 blue, for [`self_test`]'s animated GIF.
fn two_colours() -> [[u8; 3]; 256] {
    let mut palette = [[0_u8; 3]; 256];
    palette[1] = [200, 40, 40];
    palette[2] = [40, 40, 200];
    palette
}

/// The ZipCrypto known answer of [`self_test`].
const ZIP_CRYPTO_VECTOR: &str = "321354e8f2ccfbb41cb1077240104ed0c6270b31e1b5da5bcaf7eb81289c99";
/// The WinZip AES known answer of [`self_test`].
const WINZIP_AES_VECTOR: &str = concat!(
    "30313233343536373839616263646566",
    "db344c187b062feee0f8c9839dca23c96a1e58c988f2892de8b3533390aee6"
);
