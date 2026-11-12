//! Strict bounded CF_UNICODETEXT decoding. Clipboard ownership stays in file_clipboard.
use midnight_core::{FsError, MAX_FILE_CLIPBOARD_BYTES};
use std::{io, path::Path};

pub(crate) fn decode(bytes: &[u8]) -> Result<String, FsError> {
    let invalid = |message| {
        FsError::from_io(
            "Windows clipboard text",
            Path::new(""),
            io::Error::new(io::ErrorKind::InvalidData, message),
        )
    };
    if bytes.is_empty() || bytes.len() > MAX_FILE_CLIPBOARD_BYTES || !bytes.len().is_multiple_of(2)
    {
        return Err(invalid(
            "The Unicode clipboard allocation has an invalid size.",
        ));
    }
    let mut units = Vec::new();
    for bytes in bytes.as_chunks::<2>().0 {
        let unit = u16::from_le_bytes(*bytes);
        if unit == 0 {
            return String::from_utf16(&units).map_err(|_| invalid("The clipboard text contains undecodable UTF-16; no replacement path was created."));
        }
        units.push(unit);
    }
    Err(invalid("The Unicode clipboard text is not NUL terminated."))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn strict_text_preserves_unicode_and_rejects_ambiguous_native_input() {
        let text = "C:\\资料\\e\u{301} 🦀 ";
        let bytes: Vec<_> = text
            .encode_utf16()
            .chain([0])
            .flat_map(u16::to_le_bytes)
            .collect();
        assert_eq!(decode(&bytes).unwrap(), text);
        for bytes in [vec![0, 0xd8, 0, 0], vec![b'x', 0], vec![0], vec![]] {
            assert!(decode(&bytes).is_err());
        }
        assert!(decode(&vec![0; MAX_FILE_CLIPBOARD_BYTES + 2]).is_err());
        assert_eq!(decode(&[0, 0]).unwrap(), "");
        assert_eq!(decode(&[0xfd, 0xff, 0, 0]).unwrap(), "\u{fffd}");
    }
}
