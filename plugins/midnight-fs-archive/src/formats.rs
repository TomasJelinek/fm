use crate::*;
use std::{
    cell::RefCell,
    io::{Seek, SeekFrom},
};
use zip::read::HasZipMetadata;

const CENTRAL_HEADER: [u8; 4] = *b"PK\x01\x02";
/// Fixed bytes of a central-directory record before its name.
const CENTRAL_RECORD: usize = 46;
const ZIP64: u16 = 0x0001;
const UNICODE_PATH: u16 = 0x7075;
/// Extra-field tags zip 8.6 parses (`zip::extra_fields::UsedExtraField`). zip rejects
/// a record that ends inside one of their headers; any other short tail is padding.
const PARSED_FIELDS: [u16; 7] = [0x0001, 0x000a, 0x5455, 0x6375, 0x7075, 0x9901, 0xa11e];

impl Archive {
    pub(crate) fn zip(&mut self, source: Reader) -> Result<(), Error> {
        let source = Shared(Rc::new(RefCell::new(source)));
        let mut zip = zip::ZipArchive::new(source.clone()).map_err(not_zip)?;
        // zip keeps the last record of each name, so the newest record it parsed
        // always survives: its offset ends the record walk.
        let mut last = None;
        for index in 0..zip.len() {
            let raw = zip.by_index_raw(index).map_err(zip_error)?;
            last = last.max(Some(raw.central_header_start()));
        }
        let records = match last {
            Some(last) => indexed_records(source, zip.central_directory_start(), last)?,
            None => HashMap::new(),
        };
        if records.len() != zip.len() {
            return Err(ambiguous());
        }
        for index in 0..zip.len() {
            let raw = zip.by_index_raw(index).map_err(zip_error)?;
            // Each entry must be the last record the walk indexed under its name.
            let merged = match records.get(raw.name_raw()) {
                Some(record) if record.last == raw.central_header_start() => record.count > 1,
                _ => return Err(ambiguous()),
            };
            let kind = if raw.is_symlink() {
                Kind::Link
            } else if raw.is_dir() {
                Kind::Directory
            } else {
                Kind::File
            };
            let metadata = raw.get_metadata();
            let aes = metadata.aes_mode.map(|(mode, ..)| mode);
            let encryption = encryption(raw.encrypted(), metadata.flags, aes);
            // The real method: the AES extra field replaces method 99 with it.
            let method = compression_name(raw.compression());
            let reason = if kind != Kind::File {
                Reason::None
            } else if merged {
                Reason::AmbiguousName
            } else if !decryptable(encryption) {
                Reason::Encryption
            } else if let Some(method) = method {
                self.compression(method.as_ref())
            } else {
                Reason::None
            };
            if raw.size() > self.limits.max_entry {
                return Err(Error::limit());
            }
            let size = raw.size();
            let modified = timestamps::zip_modified(raw.extra_data_fields(), raw.last_modified());
            let access = Access { encryption, reason };
            // The index copies both names from the entry; `raw` borrows only `zip`.
            let id = self.add(raw.name_raw(), raw.name(), kind, None, access)?;
            drop(raw);
            let Some(member) = self.member_mut(id) else {
                continue;
            };
            member.modified = modified;
            let (readable, kind) = (member.readable, member.kind);
            if readable {
                let id = self.indexed_size(size)?;
                self.zip_members.push((id, index));
            } else if kind == Kind::File && !merged {
                // Merged records leave one entry standing for several sizes.
                self.declared_size(size);
            }
        }
        self.zip = Some(zip);
        Ok(())
    }
}

/// A member's encryption: the AES extra field names WinZip AES and its key length,
/// general-purpose bit 6 PKWARE strong encryption, and bit 0 alone ZipCrypto.
fn encryption(encrypted: bool, flags: u16, aes: Option<zip::AesMode>) -> Encryption {
    const STRONG_ENCRYPTION: u16 = 1 << 6;
    match aes {
        Some(zip::AesMode::Aes128) => Encryption::Aes128,
        Some(zip::AesMode::Aes192) => Encryption::Aes192,
        Some(zip::AesMode::Aes256) => Encryption::Aes256,
        None if !encrypted => Encryption::None,
        None if flags & STRONG_ENCRYPTION != 0 => Encryption::Unknown,
        None => Encryption::ZipCrypto,
    }
}

/// Whether this worker decrypts `encryption`: ZipCrypto, and WinZip AES through the
/// `zip` crate's `aes-crypto` feature. PKWARE strong encryption is not supported.
fn decryptable(encryption: Encryption) -> bool {
    encryption != Encryption::Unknown
}

/// The name of a compression method that this worker cannot decode, or None when it
/// decodes it (APPNOTE 4.4.5 numbers the methods).
fn compression_name(method: zip::CompressionMethod) -> Option<std::borrow::Cow<'static, str>> {
    #[allow(deprecated)]
    let number = match method {
        zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated => return None,
        zip::CompressionMethod::Unsupported(number) => number,
        // Variants of methods whose `zip` features this worker does not enable.
        other => return Some(other.to_string().into()),
    };
    Some(match number {
        1 => "Shrink".into(),
        2..=5 => "Reduce".into(),
        6 => "Implode".into(),
        9 => "Deflate64".into(),
        10 => "PKWARE DCL Implode".into(),
        12 => "BZIP2".into(),
        14 => "LZMA".into(),
        16 => "IBM z/OS CMPSC".into(),
        18 => "IBM TERSE".into(),
        19 => "IBM LZ77".into(),
        20 | 93 => "Zstandard".into(),
        94 => "MP3".into(),
        95 => "XZ".into(),
        96 => "JPEG".into(),
        97 => "WavPack".into(),
        98 => "PPMd".into(),
        99 => "AES".into(),
        number => format!("method {number}").into(),
    })
}

/// Central-directory records zip indexed under one name.
struct Record {
    count: usize,
    /// Offset of the last of them, the only one zip keeps.
    last: u64,
}

/// zip 8.6 indexes central-directory records by name after applying their Info-ZIP
/// Unicode Path fields, keeping only the last record of a name. The earlier records
/// become unreachable and the surviving entry silently stands for several members.
/// Walk exactly the records zip parsed, from the directory start through its newest
/// surviving record at `last`, and index them the same way, so such a name can be
/// listed but unreadable. The caller checks the result against zip's own entries,
/// and any disagreement refuses the mount instead of guessing. Every step advances
/// by at least one fixed record header without passing `last`, so the walk ends
/// within the directory however many records it holds.
fn indexed_records(
    mut source: Shared,
    start: u64,
    last: u64,
) -> Result<HashMap<Box<[u8]>, Record>, Error> {
    let mut records: HashMap<Box<[u8]>, Record> = HashMap::new();
    let mut header = [0; CENTRAL_RECORD];
    let (mut name, mut extra) = (Vec::new(), Vec::new());
    let mut offset = start;
    loop {
        source.seek(SeekFrom::Start(offset))?;
        source.read_exact(&mut header)?;
        if header[..4] != CENTRAL_HEADER {
            return Err(ambiguous());
        }
        let field = |at: usize| usize::from(u16::from_le_bytes([header[at], header[at + 1]]));
        let length = CENTRAL_RECORD + field(28) + field(30) + field(32);
        name.resize(field(28), 0);
        source.read_exact(&mut name)?;
        extra.resize(field(30), 0);
        source.read_exact(&mut extra)?;
        let key = indexed_name(&name, &extra).ok_or_else(ambiguous)?;
        if let Some(record) = records.get_mut(key) {
            record.count += 1;
            record.last = offset;
        } else {
            records.insert(
                key.into(),
                Record {
                    count: 1,
                    last: offset,
                },
            );
        }
        if offset == last {
            return Ok(records);
        }
        offset = offset
            .checked_add(length as u64)
            .filter(|next| *next <= last)
            .ok_or_else(ambiguous)?;
    }
}

/// The name zip 8.6 indexes a central record by (`zip::read::parse_extra_field`): its
/// raw name, replaced in field order by every Unicode Path field whose CRC-32 covers
/// the name it replaces. zip consumes each field's declared length (ZIP64 at most to
/// the end of the block); a tail too short for a field header ends the block. zip
/// rejects a record with a truncated field or a short, mismatched or non-UTF-8
/// Unicode Path, so in an archive zip opened `None` means the walk disagrees with it.
fn indexed_name<'a>(name: &'a [u8], mut extra: &'a [u8]) -> Option<&'a [u8]> {
    let mut key = name;
    while let Some((tag, rest)) = extra.split_first_chunk::<2>() {
        let tag = u16::from_le_bytes(*tag);
        let Some((length, body)) = rest.split_first_chunk::<2>() else {
            return (!PARSED_FIELDS.contains(&tag)).then_some(key);
        };
        let length = usize::from(u16::from_le_bytes(*length));
        if tag == ZIP64 {
            extra = body.get(length..).unwrap_or_default();
            continue;
        }
        let (field, rest) = body.split_at_checked(length)?;
        if tag == UNICODE_PATH {
            // Version byte, CRC-32 of the name it replaces, then the UTF-8 name.
            let (crc, path) = field.get(1..)?.split_first_chunk::<4>()?;
            if u32::from_le_bytes(*crc) != crc32(key) || std::str::from_utf8(path).is_err() {
                return None;
            }
            key = path;
        }
        extra = rest;
    }
    Some(key)
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = flate2::Crc::new();
    crc.update(bytes);
    crc.sum()
}

fn ambiguous() -> Error {
    Error::new(
        ErrorCode::Unsupported,
        "Ambiguous ZIP central directory: its records do not match the indexed members",
    )
}

/// The ZIP reader and the record walk share one read window. Both seek before
/// reading, and no borrow outlives a single read or seek.
#[derive(Clone)]
pub(crate) struct Shared(Rc<RefCell<Reader>>);
impl Shared {
    fn reader(&self) -> io::Result<std::cell::RefMut<'_, Reader>> {
        self.0
            .try_borrow_mut()
            .map_err(|_| io::Error::other("ZIP source is already in use"))
    }
}
impl Read for Shared {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        self.reader()?.read(output)
    }
}
impl Seek for Shared {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.reader()?.seek(from)
    }
    fn stream_position(&mut self) -> io::Result<u64> {
        self.reader()?.stream_position()
    }
}

/// Opening found no ZIP structure. The host shows the first line as the reason and the
/// reader's own wording below it as details.
fn not_zip(error: zip::result::ZipError) -> Error {
    match error {
        zip::result::ZipError::InvalidArchive(_) => Error::new(
            ErrorCode::Corrupt,
            format!("This file is not a valid ZIP archive.\n{error}"),
        ),
        error => zip_error(error),
    }
}

/// Opening an encrypted member: a password that its verifier refuses keeps the session.
pub(crate) fn decrypt_error(error: zip::result::ZipError) -> Error {
    match error {
        zip::result::ZipError::InvalidPassword => Error::new(
            ErrorCode::WrongPassword,
            "The archive's password does not open this member; it may use another password",
        ),
        error => zip_error(error),
    }
}

pub(crate) fn zip_error(error: zip::result::ZipError) -> Error {
    if let zip::result::ZipError::Io(io) = &error
        && let Some(failure) = Failure::of(io)
    {
        return failure.clone().into();
    }
    let code = match error {
        zip::result::ZipError::UnsupportedArchive(_) => ErrorCode::Unsupported,
        _ => ErrorCode::Corrupt,
    };
    Error::new(code, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(tag: u16, body: &[u8]) -> Vec<u8> {
        let mut field = tag.to_le_bytes().to_vec();
        field.extend((body.len() as u16).to_le_bytes());
        field.extend(body);
        field
    }

    fn unicode_path(replaces: &[u8], path: &str) -> Vec<u8> {
        let mut body = vec![1];
        body.extend(crc32(replaces).to_le_bytes());
        body.extend(path.as_bytes());
        field(UNICODE_PATH, &body)
    }

    #[test]
    fn unicode_paths_replace_the_name_in_field_order_like_zip() {
        assert_eq!(indexed_name(b"raw", &[]), Some(b"raw".as_slice()));
        // A timestamp before the path and an unparsed tag after it are skipped by length.
        let mut extra = field(0x5455, &[1, 0, 0, 0, 0]);
        extra.extend(unicode_path(b"raw", "first"));
        extra.extend(field(0xcafe, b"ignored"));
        assert_eq!(indexed_name(b"raw", &extra), Some(b"first".as_slice()));
        // A second path field must cover the name the first one produced.
        extra.extend(unicode_path(b"first", "second"));
        assert_eq!(indexed_name(b"raw", &extra), Some(b"second".as_slice()));
        // zip reads 24 ZIP64 bytes and then skips at most to the end of the block, so a
        // path inside an overlong ZIP64 field is never parsed.
        let mut overlong = field(ZIP64, &[0; 24]);
        overlong[2] = 255;
        overlong.extend(unicode_path(b"raw", "hidden"));
        assert_eq!(indexed_name(b"raw", &overlong), Some(b"raw".as_slice()));
        // Fragments too short for a header are padding, except a parsed tag's header.
        for tail in [&[0x34][..], &[0x34, 0x12], &[0x34, 0x12, 9]] {
            let mut padded = unicode_path(b"raw", "path");
            padded.extend(tail);
            assert_eq!(indexed_name(b"raw", &padded), Some(b"path".as_slice()));
        }
        assert_eq!(indexed_name(b"raw", &[0x75, 0x70, 9]), None);
    }

    #[test]
    fn records_zip_would_reject_are_reported_as_a_disagreement() {
        assert_eq!(indexed_name(b"raw", &unicode_path(b"other", "path")), None);
        let mut invalid = unicode_path(b"raw", "path");
        let at = invalid.len() - 1;
        invalid[at] = 0xff;
        assert_eq!(indexed_name(b"raw", &invalid), None, "not UTF-8");
        assert_eq!(indexed_name(b"raw", &field(UNICODE_PATH, &[1, 0, 0])), None);
        let mut truncated = field(0x5455, &[1, 0, 0, 0, 0]);
        truncated.pop();
        assert_eq!(indexed_name(b"raw", &truncated), None);
    }

    #[test]
    fn a_walk_that_cannot_reproduce_the_reader_refuses_the_mount() {
        let mut writer = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
        for name in ["a", "b"] {
            writer
                .start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
        }
        let bytes = writer.finish().unwrap().into_inner();
        let start = zip::ZipArchive::new(io::Cursor::new(bytes.clone()))
            .unwrap()
            .central_directory_start();
        let second = start
            + 4
            + bytes[start as usize + 4..]
                .windows(4)
                .position(|window| window == CENTRAL_HEADER)
                .unwrap() as u64;
        let walk = |last: u64| {
            let length = bytes.len() as u64;
            let reader = Reader::new(Box::new(io::Cursor::new(bytes.clone())), length);
            indexed_records(Shared(Rc::new(RefCell::new(reader))), start, last)
        };
        let records = walk(second).unwrap();
        assert_eq!(records[b"b".as_slice()].last, second);
        assert_eq!(records[b"a".as_slice()].count, 1);
        // Zip's newest record must end the walk exactly on a record boundary.
        assert_eq!(walk(second - 1).err().unwrap().code, ErrorCode::Unsupported);
        // Past its records the walk meets the end-of-directory record and fails too.
        assert!(walk(bytes.len() as u64).is_err());
    }
}
