//! Checksum manifest decoding, independent of filesystem access. A manifest is read as a
//! stream of lines: its memory follows its entries (a short name and a fixed-length
//! digest each), never its raw size, and nothing limits how large it may be.
use super::worker::failure;
use midnight_core::*;
use std::{
    borrow::Cow,
    collections::{HashMap, hash_map::Entry},
    hash::{BuildHasher, RandomState},
    io,
    path::{Component, Path, PathBuf},
};

/// A manifest's entries, in file order.
#[derive(Debug)]
pub(super) struct ParsedManifest {
    pub algorithm: ChecksumAlgorithm,
    /// Each entry's name below the manifest's folder.
    pub names: Vec<Box<Path>>,
    /// Each entry's expected digest, `algorithm.bytes()` long, in the same order.
    digests: Vec<u8>,
}
impl ParsedManifest {
    /// The expected digest of entry `index`.
    pub fn digest(&self, index: usize) -> Option<&[u8]> {
        let length = self.algorithm.bytes();
        self.digests
            .get(index.checked_mul(length)?..index.checked_add(1)?.checked_mul(length)?)
    }
    /// Each entry's name and expected digest.
    pub fn entries(&self) -> impl Iterator<Item = (&Path, &[u8])> {
        self.names
            .iter()
            .map(AsRef::as_ref)
            .zip(self.digests.chunks_exact(self.algorithm.bytes()))
    }
}

/// The whole text at once: what a save is about to write, or a test's input.
pub(super) fn parse_manifest(
    bytes: &[u8],
    path: &Path,
    cancel: &Cancellation,
) -> Result<ParsedManifest, FsError> {
    let mut parser = ManifestParser::new(path, cancel)?;
    for chunk in bytes.chunks(CHECKSUM_BUFFER_BYTES) {
        parser.feed(chunk)?;
    }
    parser.finish()
}

#[derive(Clone, Copy)]
enum Encoding {
    Utf8,
    Utf16 { little: bool },
}

/// Decodes a manifest fed in chunks of any size. It holds the entries so far, the text of
/// the line being read and at most one chunk: never the whole text.
pub(super) struct ManifestParser<'a> {
    path: &'a Path,
    cancel: &'a Cancellation,
    /// Unknown until the first bytes (and any byte-order mark) are in.
    encoding: Option<Encoding>,
    /// The first bytes, until they show the encoding.
    head: Vec<u8>,
    /// UTF-8 text not yet split into lines: at most a line and a chunk.
    text: Vec<u8>,
    /// How much of `text` is known to hold no line end: a line of any length that
    /// arrives in many chunks is searched once, not again with every chunk.
    searched: usize,
    /// A UTF-16 byte waiting for its pair, and a high surrogate waiting for its low one.
    odd: Option<u8>,
    high: Option<u16>,
    lines: usize,
    algorithm: Option<ChecksumAlgorithm>,
    sidecar: bool,
    names: Vec<Box<Path>>,
    digests: Vec<u8>,
    aliases: Aliases,
}

impl<'a> ManifestParser<'a> {
    pub(super) fn new(path: &'a Path, cancel: &'a Cancellation) -> Result<Self, FsError> {
        check_cancel(path, cancel)?;
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
        // Names are below the manifest's folder.
        path.parent()
            .ok_or_else(|| invalid(path, "Checksum manifest needs a parent directory."))?;
        Ok(Self {
            path,
            cancel,
            encoding: None,
            head: Vec::new(),
            text: Vec::new(),
            searched: 0,
            odd: None,
            high: None,
            lines: 0,
            algorithm: None,
            sidecar: false,
            names: Vec::new(),
            digests: Vec::new(),
            aliases: Aliases::default(),
        })
    }

    /// The next bytes of the manifest.
    pub(super) fn feed(&mut self, mut bytes: &[u8]) -> Result<(), FsError> {
        check_cancel(self.path, self.cancel)?;
        if self.encoding.is_none() {
            // Three bytes tell a UTF-8 byte-order mark; two a UTF-16 one.
            let wanted = 3usize.saturating_sub(self.head.len()).min(bytes.len());
            self.head.extend_from_slice(&bytes[..wanted]);
            bytes = &bytes[wanted..];
            if self.head.len() < 3 {
                return Ok(());
            }
            self.start()?;
        }
        self.decode(bytes)?;
        self.split_lines(false)
    }

    /// The end of the manifest: its entries.
    pub(super) fn finish(mut self) -> Result<ParsedManifest, FsError> {
        if self.encoding.is_none() {
            self.start()?;
        }
        if self.odd.is_some() {
            return Err(invalid(self.path, "Truncated UTF-16 checksum text."));
        }
        if self.high.is_some() {
            return Err(invalid(self.path, "Invalid UTF-16 checksum text."));
        }
        self.split_lines(true)?;
        let algorithm = self
            .algorithm
            .ok_or_else(|| invalid(self.path, "Checksum manifest contains no entries."))?;
        Ok(ParsedManifest {
            algorithm,
            names: self.names,
            digests: self.digests,
        })
    }

    /// Decide the encoding from the first bytes and decode them.
    fn start(&mut self) -> Result<(), FsError> {
        let head = std::mem::take(&mut self.head);
        let (encoding, rest) = match head.as_slice() {
            [0xff, 0xfe, rest @ ..] => (Encoding::Utf16 { little: true }, rest),
            [0xfe, 0xff, rest @ ..] => (Encoding::Utf16 { little: false }, rest),
            [0xef, 0xbb, 0xbf, rest @ ..] => (Encoding::Utf8, rest),
            rest => (Encoding::Utf8, rest),
        };
        self.encoding = Some(encoding);
        self.decode(rest)
    }

    /// Append `bytes` to the text as UTF-8.
    fn decode(&mut self, bytes: &[u8]) -> Result<(), FsError> {
        let Some(Encoding::Utf16 { little }) = self.encoding else {
            self.text.extend_from_slice(bytes);
            return Ok(());
        };
        let mut bytes = bytes.iter().copied();
        let mut next_unit = |odd: &mut Option<u8>| {
            let first = odd.take().or_else(|| bytes.next())?;
            match bytes.next() {
                Some(second) => Some(if little {
                    u16::from_le_bytes([first, second])
                } else {
                    u16::from_be_bytes([first, second])
                }),
                None => {
                    *odd = Some(first);
                    None
                }
            }
        };
        while let Some(unit) = next_unit(&mut self.odd) {
            let character = match (self.high.take(), unit) {
                (None, 0xd800..=0xdbff) => {
                    self.high = Some(unit);
                    continue;
                }
                (Some(high), 0xdc00..=0xdfff) => {
                    char::decode_utf16([high, unit]).next().and_then(Result::ok)
                }
                (None, unit) => char::from_u32(u32::from(unit)),
                (Some(_), _) => None,
            }
            .ok_or_else(|| invalid(self.path, "Invalid UTF-16 checksum text."))?;
            let mut buffer = [0; 4];
            self.text
                .extend_from_slice(character.encode_utf8(&mut buffer).as_bytes());
        }
        Ok(())
    }

    /// Parse every complete line of the text; at the end also the last one.
    fn split_lines(&mut self, end: bool) -> Result<(), FsError> {
        let text = std::mem::take(&mut self.text);
        let mut start = 0;
        let mut from = self.searched.min(text.len());
        while let Some(length) = text[from..].iter().position(|byte| *byte == b'\n') {
            self.line(&text[start..from + length])?;
            start = from + length + 1;
            from = start;
        }
        if end {
            if start < text.len() {
                self.line(&text[start..])?;
            }
            return Ok(());
        }
        self.text = text;
        self.text.drain(..start);
        self.searched = self.text.len();
        Ok(())
    }

    fn line(&mut self, bytes: &[u8]) -> Result<(), FsError> {
        self.lines += 1;
        check_cancel(self.path, self.cancel)?;
        let line = std::str::from_utf8(bytes).map_err(|_| {
            invalid(
                self.path,
                "Checksum files require UTF-8 or BOM-marked UTF-16 text.",
            )
        })?;
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() || line.trim_start().starts_with([';', '#']) {
            return Ok(());
        }
        let number = self.lines;
        let decoded = decode_record(line, self.path).map_err(|mut error| {
            error.message = format!("Line {number}: {}", error.message);
            error
        })?;
        if self
            .algorithm
            .is_some_and(|previous| previous != decoded.algorithm)
        {
            return Err(invalid(
                self.path,
                "Checksum manifest mixes different algorithms.",
            ));
        }
        if self.sidecar || (decoded.name.is_none() && !self.names.is_empty()) {
            return Err(invalid(
                self.path,
                "A hash-only sidecar must contain exactly one checksum.",
            ));
        }
        self.algorithm = Some(decoded.algorithm);
        let relative = if let Some(name) = decoded.name {
            relative_name(&name, decoded.escaped, self.path)?
        } else {
            self.sidecar = true;
            sidecar_name(self.path, decoded.algorithm)?
        };
        // Names are never case-folded for opening. Conservative alias detection rejects
        // an entire manifest instead of letting another platform silently reuse a row.
        let key = relative.to_str().ok_or_else(|| {
            invalid(
                self.path,
                "Checksum filename cannot be represented without loss.",
            )
        })?;
        if self.aliases.repeated(&self.names, key) {
            return Err(invalid(
                self.path,
                "Duplicate or ambiguous checksum filename.",
            ));
        }
        self.names.push(relative.into_boxed_path());
        self.digests.extend_from_slice(&decoded.digest);
        Ok(())
    }
}

/// Names seen so far by the hash of their case-folded text: a lookup per entry, and the
/// names themselves (kept by the parser) settle a shared hash.
#[derive(Default)]
struct Aliases {
    hasher: RandomState,
    first: HashMap<u64, usize>,
    /// Further names whose hash an earlier name already had.
    more: Vec<(u64, usize)>,
}
impl Aliases {
    /// Whether `name` folds to the name of an entry in `names`; if not, it is the next
    /// entry's.
    fn repeated(&mut self, names: &[Box<Path>], name: &str) -> bool {
        let hash = self.hasher.hash_one(unicase::UniCase::new(name));
        let same = |index: usize| {
            names[index]
                .to_str()
                .is_some_and(|known| unicase::UniCase::new(known) == unicase::UniCase::new(name))
        };
        match self.first.entry(hash) {
            Entry::Vacant(entry) => {
                entry.insert(names.len());
                false
            }
            Entry::Occupied(entry) => {
                if same(*entry.get())
                    || self
                        .more
                        .iter()
                        .any(|(known, index)| *known == hash && same(*index))
                {
                    return true;
                }
                self.more.push((hash, names.len()));
                false
            }
        }
    }
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
    let mut decoded = String::with_capacity(name.len());
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

/// A manifest name as a relative native path, at any depth.
fn relative_name(name: &str, escaped: bool, path: &Path) -> Result<PathBuf, FsError> {
    if name.is_empty() || name.contains(['\0', ':']) {
        return Err(invalid(path, "Empty or unsafe checksum filename."));
    }
    let normalized = if escaped {
        Cow::Borrowed(name)
    } else {
        Cow::Owned(name.replace('\\', "/"))
    };
    let mut relative = PathBuf::new();
    for part in normalized.split('/') {
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

/// The file a hash-only `name.ext.<algorithm>` sidecar checks: `name.ext`, beside it.
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
    relative_name(name, true, path)
}

fn invalid(path: &Path, message: &str) -> FsError {
    failure(path, io::ErrorKind::InvalidData, message)
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
