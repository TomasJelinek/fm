//! Incremental strict UTF-8 and BOM-marked UTF-16 decoding with reusable storage.

#[derive(Clone, Copy)]
enum Encoding {
    Utf8,
    Utf16 { little: bool },
}

#[derive(Default)]
pub(super) struct Decoder {
    encoding: Option<Encoding>,
    prefix: [u8; 3],
    prefix_len: usize,
    tail: [u8; 3],
    tail_len: usize,
    odd: Option<u8>,
    high: Option<u16>,
    utf8: Vec<u8>,
    text: String,
}

impl Decoder {
    pub(super) fn reset(&mut self) {
        self.encoding = None;
        self.prefix_len = 0;
        self.tail_len = 0;
        self.odd = None;
        self.high = None;
        self.utf8.clear();
        self.text.clear();
    }

    pub(super) fn push(&mut self, mut bytes: &[u8], end: bool) -> Result<&str, ()> {
        self.text.clear();
        if self.encoding.is_none() {
            let take = bytes.len().min(3 - self.prefix_len);
            self.prefix[self.prefix_len..self.prefix_len + take].copy_from_slice(&bytes[..take]);
            self.prefix_len += take;
            bytes = &bytes[take..];
            if self.prefix_len < 3 && !end {
                return Ok("");
            }
            let prefix = self.prefix;
            let prefix = &prefix[..self.prefix_len];
            let (encoding, skip) = if prefix.starts_with(&[0xff, 0xfe]) {
                (Encoding::Utf16 { little: true }, 2)
            } else if prefix.starts_with(&[0xfe, 0xff]) {
                (Encoding::Utf16 { little: false }, 2)
            } else {
                (
                    Encoding::Utf8,
                    if prefix.starts_with(&[0xef, 0xbb, 0xbf]) {
                        3
                    } else {
                        0
                    },
                )
            };
            self.encoding = Some(encoding);
            self.decode(&prefix[skip..], encoding)?;
        }
        self.decode(bytes, self.encoding.ok_or(())?)?;
        if end && (self.tail_len != 0 || self.odd.is_some() || self.high.is_some()) {
            return Err(());
        }
        Ok(&self.text)
    }

    fn decode(&mut self, bytes: &[u8], encoding: Encoding) -> Result<(), ()> {
        match encoding {
            Encoding::Utf8 => {
                self.utf8.clear();
                self.utf8.extend_from_slice(&self.tail[..self.tail_len]);
                self.utf8.extend_from_slice(bytes);
                let (valid, remaining) = match std::str::from_utf8(&self.utf8) {
                    Ok(text) => (text, &[][..]),
                    Err(error) if error.error_len().is_none() => (
                        std::str::from_utf8(&self.utf8[..error.valid_up_to()]).map_err(|_| ())?,
                        &self.utf8[error.valid_up_to()..],
                    ),
                    Err(_) => return Err(()),
                };
                self.text.push_str(valid);
                if remaining.len() > self.tail.len() {
                    return Err(());
                }
                self.tail_len = remaining.len();
                self.tail[..self.tail_len].copy_from_slice(remaining);
            }
            Encoding::Utf16 { little } => {
                for &byte in bytes {
                    let Some(first) = self.odd.take() else {
                        self.odd = Some(byte);
                        continue;
                    };
                    let word = if little {
                        u16::from_le_bytes([first, byte])
                    } else {
                        u16::from_be_bytes([first, byte])
                    };
                    if let Some(high) = self.high.take() {
                        if !(0xdc00..=0xdfff).contains(&word) {
                            return Err(());
                        }
                        let scalar =
                            0x10000 + (((high as u32 - 0xd800) << 10) | (word as u32 - 0xdc00));
                        self.text.push(char::from_u32(scalar).ok_or(())?);
                    } else if (0xd800..=0xdbff).contains(&word) {
                        self.high = Some(word);
                    } else {
                        self.text.push(char::from_u32(word as u32).ok_or(())?);
                    }
                }
            }
        }
        Ok(())
    }
}
