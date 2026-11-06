//! Raw-byte local file URIs; desktop clipboard ownership stays off the application UI.

use midnight_core::{
    FileClipboard, FileClipboardIntent, FsError, MAX_FILE_CLIPBOARD_BYTES, MAX_FILE_CLIPBOARD_PATHS,
};
use std::{
    ffi::OsString,
    io::{self, Read},
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

#[cfg(test)]
#[path = "file_clipboard_tests.rs"]
mod tests;
#[path = "file_clipboard_x11.rs"]
mod x11;

const URI: &str = "text/uri-list";
const GNOME: &str = "x-special/gnome-copied-files";
const KDE: &str = "application/x-kde-cutselection";

fn error(message: impl std::fmt::Display) -> FsError {
    FsError::from_io(
        "Linux file clipboard",
        Path::new(""),
        io::Error::other(message.to_string()),
    )
}

pub(crate) fn encode_uris(files: &FileClipboard) -> Result<Vec<u8>, FsError> {
    files.validate()?;
    const HEX: &[u8] = b"0123456789ABCDEF";
    let mut bytes = Vec::new();
    for path in &files.paths {
        bytes.extend_from_slice(b"file://");
        for &byte in path.as_os_str().as_bytes() {
            if byte.is_ascii_alphanumeric() || b"/-._~".contains(&byte) {
                bytes.push(byte);
            } else {
                bytes.extend_from_slice(&[
                    b'%',
                    HEX[(byte >> 4) as usize],
                    HEX[(byte & 15) as usize],
                ]);
            }
            if bytes.len() > MAX_FILE_CLIPBOARD_BYTES - 8 {
                return Err(error(
                    "Encoded file URIs exceed their 4 MiB resource budget.",
                ));
            }
        }
        bytes.extend_from_slice(b"\r\n");
    }
    Ok(bytes)
}

pub(crate) fn decode_uris(bytes: &[u8]) -> Result<Vec<PathBuf>, FsError> {
    if bytes.len() > MAX_FILE_CLIPBOARD_BYTES {
        return Err(error("File URIs exceed their resource budget."));
    }
    let mut paths = Vec::new();
    for line in bytes.split(|byte| *byte == b'\n') {
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if line.is_empty() || line.starts_with(b"#") {
            continue;
        }
        let rest = line
            .strip_prefix(b"file://")
            .ok_or_else(|| error("The clipboard contains a non-file URI."))?;
        let slash = rest
            .iter()
            .position(|byte| *byte == b'/')
            .ok_or_else(|| error("A file URI has no absolute path."))?;
        let authority = &rest[..slash];
        if !authority.is_empty() && !authority.eq_ignore_ascii_case(b"localhost") {
            return Err(error("Remote file-URI authorities are unsupported."));
        }
        let mut raw = Vec::new();
        let mut remaining = &rest[slash..];
        while let Some((&byte, tail)) = remaining.split_first() {
            if byte == b'%' {
                if tail.len() < 2 {
                    return Err(error("A file URI contains a truncated escape."));
                }
                let high = (tail[0] as char)
                    .to_digit(16)
                    .ok_or_else(|| error("A file URI has an invalid escape."))?;
                let low = (tail[1] as char)
                    .to_digit(16)
                    .ok_or_else(|| error("A file URI has an invalid escape."))?;
                raw.push((high * 16 + low) as u8);
                remaining = &tail[2..];
            } else {
                if !byte.is_ascii() || byte.is_ascii_control() || matches!(byte, b'?' | b'#' | b' ')
                {
                    return Err(error("A file URI contains an unescaped or ambiguous byte."));
                }
                raw.push(byte);
                remaining = tail;
            }
        }
        paths.push(PathBuf::from(OsString::from_vec(raw)));
        if paths.len() > MAX_FILE_CLIPBOARD_PATHS {
            return Err(error("The file clipboard exceeds its item budget."));
        }
    }
    let files = FileClipboard {
        paths,
        intent: FileClipboardIntent::Copy,
    };
    files.validate()?;
    Ok(files.paths)
}

fn decode_gnome(bytes: &[u8]) -> Result<FileClipboard, FsError> {
    if bytes.len() > MAX_FILE_CLIPBOARD_BYTES {
        return Err(error(
            "The GNOME file clipboard exceeds its resource budget.",
        ));
    }
    let newline = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| error("The GNOME file clipboard has no intent header."))?;
    let header = bytes[..newline]
        .strip_suffix(b"\r")
        .unwrap_or(&bytes[..newline]);
    let intent = match header {
        b"copy" => FileClipboardIntent::Copy,
        b"cut" => FileClipboardIntent::Move,
        _ => return Err(error("The GNOME clipboard has an unknown transfer intent.")),
    };
    Ok(FileClipboard {
        paths: decode_uris(&bytes[newline + 1..])?,
        intent,
    })
}

fn decode_kde(bytes: Option<&[u8]>) -> Result<FileClipboardIntent, FsError> {
    match bytes {
        None | Some(b"0") => Ok(FileClipboardIntent::Copy),
        Some(b"1") => Ok(FileClipboardIntent::Move),
        _ => Err(error("The KDE clipboard has an unknown transfer intent.")),
    }
}

fn payloads(files: &FileClipboard) -> Result<Vec<(&'static str, Vec<u8>)>, FsError> {
    let uris = encode_uris(files)?;
    let mut gnome = match files.intent {
        FileClipboardIntent::Copy => b"copy\n".to_vec(),
        FileClipboardIntent::Move => b"cut\n".to_vec(),
    };
    gnome.extend(uris.iter().copied().filter(|byte| *byte != b'\r'));
    if gnome.len() > MAX_FILE_CLIPBOARD_BYTES {
        return Err(error(
            "The GNOME file clipboard exceeds its resource budget.",
        ));
    }
    let kde = vec![if files.intent == FileClipboardIntent::Move {
        b'1'
    } else {
        b'0'
    }];
    Ok(vec![(URI, uris), (GNOME, gnome), (KDE, kde)])
}

pub(super) enum Files {
    Wayland,
    X11(Box<x11::Clipboard>),
}

#[derive(Debug)]
enum ClipboardBackend<T> {
    X11(T),
    WaylandReadOnly,
}

fn unsupported(message: &str) -> FsError {
    FsError::from_io(
        "Linux file clipboard",
        Path::new(""),
        io::Error::new(io::ErrorKind::Unsupported, message),
    )
}

fn select_backend<T>(
    has_display: bool,
    has_wayland: bool,
    connect_x11: impl FnOnce() -> Result<T, FsError>,
) -> Result<ClipboardBackend<T>, FsError> {
    let mut failure = None;
    if has_display {
        match connect_x11() {
            Ok(clipboard) => return Ok(ClipboardBackend::X11(clipboard)),
            Err(problem) => failure = Some(problem),
        }
    }
    if has_wayland {
        Ok(ClipboardBackend::WaylandReadOnly)
    } else {
        Err(failure.unwrap_or_else(|| unsupported("No native clipboard display is available.")))
    }
}

impl Files {
    pub(super) fn new() -> Result<Self, FsError> {
        // Prefer the bounded owner, including the compositor's XWayland bridge.
        // Pure Wayland stays read-only until its owner has bounded cancellation.
        let has_display = std::env::var_os("DISPLAY").is_some_and(|value| !value.is_empty());
        let has_wayland =
            std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty());
        match select_backend(has_display, has_wayland, x11::Clipboard::new)? {
            ClipboardBackend::X11(clipboard) => Ok(Self::X11(Box::new(clipboard))),
            ClipboardBackend::WaylandReadOnly => Ok(Self::Wayland),
        }
    }

    pub(super) fn copy(&mut self, files: &FileClipboard) -> Result<(), FsError> {
        match self {
            Self::X11(clipboard) => clipboard.copy(files),
            Self::Wayland => Err(unsupported(
                "Native file copying requires an X11 or XWayland clipboard bridge; this Wayland session supports reading file lists only.",
            )),
        }
    }

    pub(super) fn read(&mut self) -> Result<FileClipboard, FsError> {
        match self {
            Self::X11(clipboard) => clipboard.read(),
            Self::Wayland => {
                use wl_clipboard_rs::paste::{self, ClipboardType, Seat};
                let formats = paste::get_mime_types(ClipboardType::Regular, Seat::Unspecified)
                    .map_err(error)?;
                if formats.contains(GNOME) {
                    return decode_gnome(&wayland_read(GNOME)?);
                }
                if !formats.contains(URI) {
                    return Err(error("The clipboard contains no supported file list."));
                }
                let paths = decode_uris(&wayland_read(URI)?)?;
                let effect = if formats.contains(KDE) {
                    Some(wayland_read(KDE)?)
                } else {
                    None
                };
                Ok(FileClipboard {
                    paths,
                    intent: decode_kde(effect.as_deref())?,
                })
            }
        }
    }

    pub(super) fn service_pending(&mut self) -> Result<(), FsError> {
        match self {
            Self::X11(clipboard) => clipboard.service_pending(),
            Self::Wayland => Ok(()),
        }
    }
}

fn wayland_read(format: &str) -> Result<Vec<u8>, FsError> {
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use wl_clipboard_rs::paste::{self, ClipboardType, MimeType, Seat};
    let (mut pipe, _) = paste::get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        MimeType::Specific(format),
    )
    .map_err(error)?;
    let started = Instant::now();
    let mut result = Vec::new();
    let mut buffer = [0u8; 16 * 1024];
    loop {
        if started.elapsed() > Duration::from_secs(5) {
            return Err(error(
                "The Wayland clipboard owner did not finish its response.",
            ));
        }
        let mut descriptors = [PollFd::new(&pipe, PollFlags::IN)];
        if poll(
            &mut descriptors,
            Some(&Timespec {
                tv_sec: 0,
                tv_nsec: 100_000_000,
            }),
        )
        .map_err(error)?
            == 0
        {
            continue;
        }
        let read = pipe.read(&mut buffer).map_err(error)?;
        if read == 0 {
            return Ok(result);
        }
        if result.len() + read > MAX_FILE_CLIPBOARD_BYTES {
            return Err(error(
                "The Wayland file clipboard exceeds its resource budget.",
            ));
        }
        result.extend_from_slice(&buffer[..read]);
    }
}
