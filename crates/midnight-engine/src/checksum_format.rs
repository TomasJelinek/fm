//! Bounded checksum manifest decoding, independent of filesystem access.
use super::worker::failure;
use midnight_core::*;
use std::{
    borrow::Cow,
    collections::HashSet,
    io,
    path::{Component, Path, PathBuf},
};

pub(super) const MAX_MANIFEST_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug)]
pub(super) struct ParsedManifest {
    pub algorithm: ChecksumAlgorithm,
    pub entries: Vec<ManifestEntry>,
}
#[derive(Debug)]
pub(super) struct ManifestEntry {
    pub path: PathBuf,
    pub digest: Vec<u8>,
}

pub(super) fn parse_manifest(
    bytes: &[u8],
    path: &Path,
    limits: ChecksumLimits,
    cancel: &Cancellation,
) -> Result<ParsedManifest, FsError> {
    check_cancel(path, cancel)?;
    limits
        .validate()
        .map_err(|text| failure(path, io::ErrorKind::InvalidInput, text))?;
    if !path.is_absolute()
        || path.as_os_str().as_encoded_bytes().contains(&0)
        || path
            .components()
            .any(|part| matches!(part, Component::CurDir | Component::ParentDir))
    {
        return Err(invalid(
            path,
            "Checksum manifest needs an absolute native path without traversal.",
        ));
    }
    let parent = path
        .parent()
        .ok_or_else(|| invalid(path, "Checksum manifest needs a parent directory."))?;
    if bytes.len() > MAX_MANIFEST_BYTES || bytes.len() > limits.max_retained_bytes {
        return Err(budget(path));
    }
    let text = decode_text(bytes, path, limits.max_retained_bytes, cancel)?;
    let mut retained = text.len();
    let mut algorithm = None;
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut sidecar = false;
    for (index, line) in text.split('\n').enumerate() {
        check_cancel(path, cancel)?;
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() || line.trim_start().starts_with([';', '#']) {
            continue;
        }
        // Bound temporary decoded names, native paths and alias keys before
        // allocating them, including when the record will later prove malformed.
        let _temporary = line
            .len()
            .checked_add(parent.as_os_str().as_encoded_bytes().len())
            .and_then(|length| length.checked_mul(4))
            .and_then(|length| length.checked_add(1024))
            .and_then(|length| retained.checked_add(length))
            .filter(|total| *total <= limits.max_retained_bytes)
            .ok_or_else(|| budget(path))?;
        let decoded = decode_record(line, path).map_err(|mut error| {
            error.message = format!("Line {}: {}", index + 1, error.message);
            error
        })?;
        if algorithm.is_some_and(|previous| previous != decoded.algorithm) {
            return Err(invalid(
                path,
                "Checksum manifest mixes different algorithms.",
            ));
        }
        if sidecar || (decoded.name.is_none() && !entries.is_empty()) {
            return Err(invalid(
                path,
                "A hash-only sidecar must contain exactly one checksum.",
            ));
        }
        algorithm = Some(decoded.algorithm);
        let relative = if let Some(name) = decoded.name {
            relative_name(&name, decoded.escaped, path, limits.max_depth)?
        } else {
            sidecar = true;
            sidecar_name(path, decoded.algorithm)?
        };
        let absolute = parent.join(&relative);
        // Names are never case-folded for opening. Conservative alias detection rejects
        // an entire manifest instead of letting another platform silently reuse a row.
        let key = unicase::UniCase::new(
            relative
                .to_str()
                .ok_or_else(|| {
                    invalid(
                        path,
                        "Checksum filename cannot be represented without loss.",
                    )
                })?
                .to_owned(),
        );
        let cost = absolute
            .as_os_str()
            .as_encoded_bytes()
            .len()
            .checked_mul(4)
            .and_then(|length| length.checked_add(1024))
            .and_then(|length| retained.checked_add(length))
            .filter(|total| *total <= limits.max_retained_bytes)
            .ok_or_else(|| budget(path))?;
        if entries.len() >= limits.max_rows || entries.len() as u64 >= limits.max_nodes {
            return Err(budget(path));
        }
        seen.try_reserve(1).map_err(|_| budget(path))?;
        if !seen.insert(key) {
            return Err(invalid(path, "Duplicate or ambiguous checksum filename."));
        }
        entries.try_reserve(1).map_err(|_| budget(path))?;
        retained = cost;
        entries.push(ManifestEntry {
            path: absolute,
            digest: decoded.digest,
        });
    }
    let algorithm =
        algorithm.ok_or_else(|| invalid(path, "Checksum manifest contains no entries."))?;
    Ok(ParsedManifest { algorithm, entries })
}

struct Record<'a> {
    algorithm: ChecksumAlgorithm,
    digest: Vec<u8>,
    name: Option<Cow<'a, str>>,
    escaped: bool,
}

fn decode_record<'a>(line: &'a str, path: &Path) -> Result<Record<'a>, FsError> {
    if line.contains(['\0', '\r']) {
        return Err(invalid(
            path,
            "Invalid control character in checksum record.",
        ));
    }
    let sfv = if line.starts_with('\\') {
        None
    } else {
        let trimmed_end = line.trim_end_matches([' ', '\t']);
        trimmed_end.rfind([' ', '\t']).and_then(|separator| {
            let hex = &trimmed_end[separator + 1..];
            (hex_algorithm(hex) == Some(ChecksumAlgorithm::Crc32))
                .then(|| (trimmed_end[..separator].trim_end_matches([' ', '\t']), hex))
        })
    };
    // BSD names may themselves contain parentheses; the final ')' closes the name.
    if let Some(open) = line.find('(')
        && let Some(algorithm) = label_algorithm(line[..open].trim())
        && let Some(close) = line.rfind(')').filter(|close| *close > open)
        && let Some(hex) = line[close + 1..].trim_start().strip_prefix('=')
    {
        if let Some((name, sfv_hex)) = sfv {
            if hex_algorithm(hex.trim()) == Some(algorithm) {
                return Err(invalid(
                    path,
                    "Ambiguous BSD/SFV record identifies different filenames.",
                ));
            }
            if extension_algorithm(path) == Some(ChecksumAlgorithm::Crc32) {
                return sfv_record(name, sfv_hex, path);
            }
        }
        return Ok(Record {
            algorithm,
            digest: decode_hex(hex.trim(), algorithm, path)?,
            name: Some(Cow::Borrowed(&line[open + 1..close])),
            escaped: false,
        });
    }
    let trimmed = line.trim();
    if let Some(algorithm) = hex_algorithm(trimmed) {
        return Ok(Record {
            algorithm,
            digest: decode_hex(trimmed, algorithm, path)?,
            name: None,
            escaped: false,
        });
    }
    let (body, escaped) = line
        .strip_prefix('\\')
        .map_or((line, false), |body| (body, true));
    if let Some(separator) = body.find([' ', '\t'])
        && let Some(algorithm) = hex_algorithm(&body[..separator])
        && algorithm != ChecksumAlgorithm::Crc32
    {
        if let Some((name, hex)) = sfv {
            match extension_algorithm(path) {
                Some(ChecksumAlgorithm::Crc32) => return sfv_record(name, hex, path),
                Some(hint) if hint == algorithm => {}
                _ => {
                    return Err(invalid(
                        path,
                        "Ambiguous GNU/SFV record needs a matching checksum filename extension.",
                    ));
                }
            }
        }
        let rest = &body[separator + 1..];
        // GNU's first separator and text/binary marker are syntax. Further spaces
        // belong to the exact filename; do not trim them.
        let name = rest.strip_prefix([' ', '*']).unwrap_or(rest);
        let name = if escaped {
            Cow::Owned(unescape(name, path)?)
        } else {
            Cow::Borrowed(name)
        };
        return Ok(Record {
            algorithm,
            digest: decode_hex(&body[..separator], algorithm, path)?,
            name: Some(name),
            escaped,
        });
    }
    if escaped {
        return Err(invalid(path, "Invalid GNU escaped checksum record."));
    }
    let (name, hex) = sfv.ok_or_else(|| invalid(path, "Unrecognized checksum record."))?;
    sfv_record(name, hex, path)
}

fn sfv_record<'a>(name: &'a str, hex: &str, path: &Path) -> Result<Record<'a>, FsError> {
    Ok(Record {
        algorithm: ChecksumAlgorithm::Crc32,
        digest: decode_hex(hex, ChecksumAlgorithm::Crc32, path)?,
        name: Some(Cow::Borrowed(name)),
        escaped: false,
    })
}

fn extension_algorithm(path: &Path) -> Option<ChecksumAlgorithm> {
    let extension = path.extension()?.to_str()?;
    ChecksumAlgorithm::ALL
        .into_iter()
        .find(|algorithm| extension.eq_ignore_ascii_case(algorithm.extension()))
}

fn label_algorithm(label: &str) -> Option<ChecksumAlgorithm> {
    if label.eq_ignore_ascii_case("CRC32") {
        Some(ChecksumAlgorithm::Crc32)
    } else if label.eq_ignore_ascii_case("MD5") {
        Some(ChecksumAlgorithm::Md5)
    } else if label.eq_ignore_ascii_case("SHA1") || label.eq_ignore_ascii_case("SHA-1") {
        Some(ChecksumAlgorithm::Sha1)
    } else if label.eq_ignore_ascii_case("SHA256") || label.eq_ignore_ascii_case("SHA-256") {
        Some(ChecksumAlgorithm::Sha256)
    } else if label.eq_ignore_ascii_case("SHA512") || label.eq_ignore_ascii_case("SHA-512") {
        Some(ChecksumAlgorithm::Sha512)
    } else {
        None
    }
}

fn hex_algorithm(hex: &str) -> Option<ChecksumAlgorithm> {
    ChecksumAlgorithm::ALL.into_iter().find(|algorithm| {
        hex.len() == 2 * algorithm.bytes() && hex.bytes().all(|byte| byte.is_ascii_hexdigit())
    })
}

fn decode_hex(hex: &str, algorithm: ChecksumAlgorithm, path: &Path) -> Result<Vec<u8>, FsError> {
    if hex.len() != algorithm.bytes() * 2 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid(
            path,
            "Malformed checksum length or hexadecimal digits.",
        ));
    }
    let nibble = |byte: u8| {
        if byte.is_ascii_digit() {
            byte - b'0'
        } else {
            byte.to_ascii_lowercase() - b'a' + 10
        }
    };
    Ok(hex
        .as_bytes()
        .as_chunks::<2>()
        .0
        .iter()
        .map(|pair| nibble(pair[0]) * 16 + nibble(pair[1]))
        .collect())
}

fn unescape(name: &str, path: &Path) -> Result<String, FsError> {
    let mut decoded = String::new();
    decoded.try_reserve(name.len()).map_err(|_| budget(path))?;
    let mut chars = name.chars();
    while let Some(character) = chars.next() {
        decoded.push(if character == '\\' {
            match chars.next() {
                Some('\\') => '\\',
                Some('n') => '\n',
                Some('r') => '\r',
                _ => return Err(invalid(path, "Invalid GNU filename escape.")),
            }
        } else {
            character
        });
    }
    Ok(decoded)
}

fn relative_name(
    name: &str,
    escaped: bool,
    path: &Path,
    max_depth: usize,
) -> Result<PathBuf, FsError> {
    if name.is_empty() || name.contains(['\0', ':']) {
        return Err(invalid(path, "Empty or unsafe checksum filename."));
    }
    let normalized = if escaped {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(name.replace('\\', "/"))
    };
    let mut relative = PathBuf::new();
    for (index, part) in normalized.split('/').enumerate() {
        if index >= max_depth {
            return Err(budget(path));
        }
        // Checking the native component shape also prevents a GNU escaped literal
        // backslash from becoming a separator when read on Windows.
        let mut components = Path::new(part).components();
        if part.is_empty()
            || part == "."
            || part == ".."
            || part.ends_with('.')
            || !matches!(components.next(), Some(Component::Normal(value)) if value == part)
            || components.next().is_some()
            || device_name(part)
        {
            return Err(invalid(
                path,
                "Checksum filenames must be unambiguous relative paths without traversal, roots or device names.",
            ));
        }
        relative.push(part);
    }
    Ok(relative)
}

fn device_name(part: &str) -> bool {
    let stem = part.split('.').next().unwrap_or(part).trim_end_matches(' ');
    ["CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$"]
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
        || ((stem.get(..3).is_some_and(|prefix| {
            prefix.eq_ignore_ascii_case("COM") || prefix.eq_ignore_ascii_case("LPT")
        })) && matches!(
            stem.get(3..),
            Some("1" | "2" | "3" | "4" | "5" | "6" | "7" | "8" | "9" | "¹" | "²" | "³")
        ))
}

fn sidecar_name(path: &Path, algorithm: ChecksumAlgorithm) -> Result<PathBuf, FsError> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| extension.eq_ignore_ascii_case(algorithm.extension()))
    {
        return Err(invalid(
            path,
            "A hash-only file needs the matching checksum extension.",
        ));
    }
    let name = path
        .file_stem()
        .and_then(|name| name.to_str())
        .ok_or_else(|| {
            invalid(
                path,
                "Hash-only sidecar filename cannot be decoded without loss.",
            )
        })?;
    relative_name(name, true, path, 1)
}

fn decode_text<'a>(
    bytes: &'a [u8],
    path: &Path,
    limit: usize,
    cancel: &Cancellation,
) -> Result<Cow<'a, str>, FsError> {
    let utf16 = bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]);
    if !utf16 {
        let bytes = bytes.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(bytes);
        return std::str::from_utf8(bytes).map(Cow::Borrowed).map_err(|_| {
            invalid(
                path,
                "Checksum files require UTF-8 or BOM-marked UTF-16 text.",
            )
        });
    }
    if !bytes.len().is_multiple_of(2) {
        return Err(invalid(path, "Truncated UTF-16 checksum text."));
    }
    let little = bytes[0] == 0xff;
    let units = bytes[2..].as_chunks::<2>().0.iter().map(|pair| {
        if little {
            u16::from_le_bytes([pair[0], pair[1]])
        } else {
            u16::from_be_bytes([pair[0], pair[1]])
        }
    });
    let mut decoded = String::new();
    for (index, result) in char::decode_utf16(units).enumerate() {
        if index.is_multiple_of(4096) {
            check_cancel(path, cancel)?;
        }
        let character = result.map_err(|_| invalid(path, "Invalid UTF-16 checksum text."))?;
        let size = decoded
            .len()
            .checked_add(character.len_utf8())
            .filter(|size| *size <= limit)
            .ok_or_else(|| budget(path))?;
        decoded
            .try_reserve(size - decoded.len())
            .map_err(|_| budget(path))?;
        decoded.push(character);
    }
    Ok(Cow::Owned(decoded))
}

fn invalid(path: &Path, message: &str) -> FsError {
    failure(path, io::ErrorKind::InvalidData, message)
}
fn budget(path: &Path) -> FsError {
    failure(
        path,
        io::ErrorKind::OutOfMemory,
        "Checksum manifest exceeds its bounded parsing budget.",
    )
}
fn check_cancel(path: &Path, cancel: &Cancellation) -> Result<(), FsError> {
    if cancel.is_cancelled() {
        Err(failure(
            path,
            io::ErrorKind::Interrupted,
            "Checksum parsing cancelled.",
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
#[path = "checksum_format_tests.rs"]
mod tests;
