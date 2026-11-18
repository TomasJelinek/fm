//! C++ raw delimiters have at most 16 basic source characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Raw {
    delimiter: [u8; 16],
    length: u8,
}

pub(super) fn start(bytes: &[u8]) -> Option<(Raw, usize)> {
    let prefix = [b"R\"".as_slice(), b"u8R\"", b"uR\"", b"UR\"", b"LR\""]
        .into_iter()
        .find(|prefix| bytes.starts_with(prefix))?;
    let mut raw = Raw {
        delimiter: [0; 16],
        length: 0,
    };
    let mut at = prefix.len();
    loop {
        let byte = *bytes.get(at)?;
        if byte == b'(' {
            return Some((raw, at + 1));
        }
        if raw.length == 16
            || !byte.is_ascii()
            || byte.is_ascii_whitespace()
            || byte.is_ascii_control()
            || matches!(byte, b')' | b'\\')
        {
            return None;
        }
        raw.delimiter[raw.length as usize] = byte;
        raw.length += 1;
        at += 1;
    }
}

impl Raw {
    pub(super) fn consume<F: Fn() -> bool>(
        &self,
        text: &str,
        mut at: usize,
        final_chunk: bool,
        poll: &mut super::Poll<F>,
    ) -> Option<(usize, bool)> {
        let bytes = text.as_bytes();
        let len = self.length as usize;
        while at < bytes.len() {
            if poll.cancelled(at) {
                return None;
            }
            if !final_chunk && bytes[at] == b')' && bytes.len() - at < len + 2 {
                let matched = (bytes.len() - at - 1).min(len);
                if bytes[at + 1..at + 1 + matched] == self.delimiter[..matched] {
                    return Some((at, false));
                }
            }
            if bytes[at] == b')'
                && bytes.get(at + 1..at + 1 + len) == Some(&self.delimiter[..len])
                && bytes.get(at + 1 + len) == Some(&b'"')
            {
                return Some((at + len + 2, true));
            }
            at += text[at..].chars().next()?.len_utf8();
        }
        Some((at, false))
    }
}

pub(super) fn incomplete_start(bytes: &[u8]) -> bool {
    for prefix in [b"R\"".as_slice(), b"u8R\"", b"uR\"", b"UR\"", b"LR\""] {
        if bytes.len() < prefix.len() && prefix.starts_with(bytes) {
            return true;
        }
        if let Some(tail) = bytes.strip_prefix(prefix) {
            return tail.len() <= 16
                && tail.iter().all(|b| {
                    b.is_ascii()
                        && !b.is_ascii_whitespace()
                        && !b.is_ascii_control()
                        && !matches!(b, b'(' | b')' | b'\\')
                });
        }
    }
    false
}
