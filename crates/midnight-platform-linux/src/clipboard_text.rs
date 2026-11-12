//! Text MIME selection uses the existing bounded native clipboard transports.
use crate::file_clipboard::{Files, wayland_read};
use midnight_core::FsError;
use std::{io, path::Path};

pub(crate) fn decode(bytes: Vec<u8>) -> Result<String, FsError> {
    if bytes.len() > midnight_core::MAX_FILE_CLIPBOARD_BYTES {
        return Err(error(
            "Clipboard text exceeds its 4 MiB operation resource budget.",
        ));
    }
    String::from_utf8(bytes).map_err(|_| {
        error("The clipboard text is not valid UTF-8; no replacement path was created.")
    })
}
pub(crate) fn error(message: &str) -> FsError {
    FsError::from_io(
        "Linux clipboard text",
        Path::new(""),
        io::Error::new(io::ErrorKind::InvalidData, message),
    )
}
pub(crate) fn read(files: &mut Files) -> Result<String, FsError> {
    match files {
        Files::X11(clipboard) => clipboard.read_text(),
        Files::Wayland => {
            use wl_clipboard_rs::paste::{self, ClipboardType, Seat};
            let formats = paste::get_mime_types(ClipboardType::Regular, Seat::Unspecified)
                .map_err(|e| error(&e.to_string()))?;
            for format in [
                "text/plain;charset=utf-8",
                "text/plain;charset=UTF-8",
                "text/plain",
            ] {
                if formats.contains(format) {
                    return decode(wayland_read(format)?);
                }
            }
            Err(error("The clipboard contains no supported text format."))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn text_is_strict_unicode_without_trimming_or_uri_interpretation() {
        for text in [
            "/资料/e\u{301} ",
            "file:///literal%20name",
            "\"quoted\"",
            "",
        ] {
            assert_eq!(decode(text.as_bytes().to_vec()).unwrap(), text);
        }
        assert!(decode(vec![0xff]).is_err());
    }
}
