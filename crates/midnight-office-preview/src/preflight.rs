//! Strict ZIP preflight over captured bytes, before zip allocates its eager index.
//!
//! Every central record and local header is validated here, extra fields
//! included. zip then indexes a canonical copy of the central directory without
//! extra fields or comments, so zip never interprets an extra field itself.
use crate::{OfficeError, model::PART_COUNT_LIMIT, package::validate_name, source::CapturedReader};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Read, Seek, SeekFrom},
};

/// Accepted entries and central-directory bytes; both precede any index allocation.
pub(crate) const MAX_ENTRIES: usize = 10_000;
pub(crate) const MAX_CENTRAL: u64 = 8 * 1024 * 1024;
/// End-of-central-directory record plus the largest possible archive comment.
const TAIL: u64 = 22 + 65_535;
const LOCAL: &[u8; 4] = b"PK\x03\x04";
const CENTRAL: &[u8; 4] = b"PK\x01\x02";
const END: &[u8; 4] = b"PK\x05\x06";
const ZIP64_END: &[u8; 4] = b"PK\x06\x06";
const ZIP64_LOCATOR: &[u8; 4] = b"PK\x06\x07";
const DESCRIPTOR: &[u8; 4] = b"PK\x07\x08";
const COMPOUND_FILE: [u8; 8] = [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1];
/// The extra-field records this preflight interprets; it skips every other one.
const ZIP64: u16 = 0x0001;
const UNICODE_PATH: u16 = 0x7075;
const SATURATED: u32 = u32::MAX;
const UTF8_NAMES: u16 = 0x800;
const DATA_DESCRIPTOR: u16 = 8;
/// Deflate speed options (bits 1 and 2), which Microsoft Office sets on its parts.
const DEFLATE_OPTIONS: u16 = 6;
/// Traditional, strong and central-directory encryption.
const ENCRYPTION: u16 = 1 | 0x40 | 0x2000;

pub(crate) const LEGACY_OR_PROTECTED: &str = "is password-protected or saved in the older \
     binary Office format (.doc, .xls or .ppt), which the preview cannot open";
pub(crate) const SPLIT_ARCHIVE: &str =
    "is part of a ZIP archive split into several files, which the preview cannot open";
pub(crate) const ZIP64_ARCHIVE: &str =
    "is stored in the ZIP64 format for very large archives, which the preview cannot open";
pub(crate) const HIDDEN_DIRECTORY: &str =
    "hides a second archive directory in its ZIP comment, so the preview does not open it";
pub(crate) const STORAGE_OPTIONS: &str =
    "uses ZIP storage options that the preview does not support";
pub(crate) const COMPRESSION: &str =
    "uses a compression method other than Deflate, which the preview cannot read";
pub(crate) const NAME_ENCODING: &str = "names its parts in a legacy character encoding \
     instead of UTF-8, which the preview does not support";

fn u16_at(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}
fn u32_at(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}
fn u64_at(bytes: &[u8], offset: usize) -> u64 {
    let mut value = [0; 8];
    value.copy_from_slice(&bytes[offset..offset + 8]);
    u64::from_le_bytes(value)
}
fn read_at<const N: usize>(
    reader: &mut CapturedReader,
    offset: u64,
) -> Result<[u8; N], OfficeError> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

/// Visits each record of an extra-field block, which must be an exact sequence of
/// (ID, size, data) records: no cut header, overlong record or trailing byte.
pub(crate) fn extra_records<'a>(
    block: &'a [u8],
    mut visit: impl FnMut(u16, &'a [u8]) -> Result<(), OfficeError>,
) -> Result<(), OfficeError> {
    let mut rest = block;
    while !rest.is_empty() {
        if rest.len() < 4 {
            return Err(OfficeError::Malformed(
                "ZIP extra field ends inside a record header",
            ));
        }
        let size = usize::from(u16_at(rest, 2));
        let data = rest.get(4..4 + size).ok_or(OfficeError::Malformed(
            "ZIP extra record overruns its field",
        ))?;
        visit(u16_at(rest, 0), data)?;
        rest = &rest[4 + size..];
    }
    Ok(())
}

/// The ZIP64 record of one header, if any, after checking the whole block.
fn zip64_record<'a>(block: &'a [u8], name: &[u8]) -> Result<Option<&'a [u8]>, OfficeError> {
    let mut zip64 = None;
    extra_records(block, |id, data| {
        match id {
            ZIP64 => {
                if zip64.replace(data).is_some() {
                    return Err(OfficeError::Malformed("repeated ZIP64 extra record"));
                }
            }
            UNICODE_PATH => unicode_path(data, name)?,
            // Growth-hint padding (0xA220), timestamps (0x5455, 0x000A), Unix IDs
            // (0x7875), JAR (0xCAFE) and alignment (0xD935, 0xA11E) records, and
            // every unknown record, describe nothing the preview reads.
            _ => (),
        }
        Ok(())
    })?;
    Ok(zip64)
}

/// Info-ZIP Unicode path: current only when its CRC matches the raw name. A current
/// record that names another part would give the member two names; refuse it.
fn unicode_path(data: &[u8], name: &[u8]) -> Result<(), OfficeError> {
    if data.len() < 5 || data[0] != 1 {
        return Ok(());
    }
    let mut crc = flate2::Crc::new();
    crc.update(name);
    if crc.sum() == u32_at(data, 1) && &data[5..] != name {
        return Err(OfficeError::Malformed(
            "ZIP Unicode path record names a different part",
        ));
    }
    Ok(())
}

/// CRC and sizes after ZIP64 resolution.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Sizes {
    crc: u32,
    compressed: u64,
    uncompressed: u64,
}

/// One validated central record with its resolved values.
struct Record<'a> {
    header: &'a [u8],
    name: &'a [u8],
    flags: u16,
    method: u16,
    sizes: Sizes,
    local: u64,
}

/// Parses the central record at `position`; returns it and the next position.
fn central_record(metadata: &[u8], position: usize) -> Result<(Record<'_>, usize), OfficeError> {
    let header = metadata
        .get(position..position + 46)
        .ok_or(OfficeError::Malformed("truncated central record"))?;
    if &header[..4] != CENTRAL {
        return Err(OfficeError::Malformed("central record signature"));
    }
    let flags = u16_at(header, 8);
    if flags & ENCRYPTION != 0 {
        return Err(OfficeError::Encrypted);
    }
    if flags & !(UTF8_NAMES | DATA_DESCRIPTOR | DEFLATE_OPTIONS) != 0 {
        return Err(OfficeError::Unsupported(STORAGE_OPTIONS));
    }
    let method = u16_at(header, 10);
    if method != 0 && method != 8 {
        return Err(OfficeError::Unsupported(COMPRESSION));
    }
    let name_len = usize::from(u16_at(header, 28));
    let extra_len = usize::from(u16_at(header, 30));
    let comment_len = usize::from(u16_at(header, 32));
    if name_len == 0 || name_len > 4096 {
        return Err(OfficeError::LimitExceeded("entry name"));
    }
    let next = position + 46 + name_len + extra_len + comment_len;
    if next > metadata.len() {
        return Err(OfficeError::Malformed("central record bounds"));
    }
    let name = &metadata[position + 46..][..name_len];
    // The entry comment is skipped; zip never sees it.
    let extra = &metadata[position + 46 + name_len..][..extra_len];
    // Each field at 0xFFFF/0xFFFFFFFF takes its value from the ZIP64 record, which
    // holds exactly those fields, in this order (APPNOTE 4.5.3).
    let mut zip64 = zip64_record(extra, name)?.unwrap_or_default();
    let mut wide = |value: u32, missing: &'static str| -> Result<u64, OfficeError> {
        if value != SATURATED {
            return Ok(u64::from(value));
        }
        let field = zip64.get(..8).ok_or(OfficeError::Malformed(missing))?;
        zip64 = &zip64[8..];
        Ok(u64_at(field, 0))
    };
    let uncompressed = wide(u32_at(header, 24), "ZIP64 record lacks the original size")?;
    let compressed = wide(u32_at(header, 20), "ZIP64 record lacks the compressed size")?;
    let local = wide(
        u32_at(header, 42),
        "ZIP64 record lacks the local header offset",
    )?;
    let disk = match u16_at(header, 34) {
        u16::MAX => zip64
            .get(..4)
            .map(|disk| u32_at(disk, 0))
            .ok_or(OfficeError::Malformed("ZIP64 record lacks the disk number"))?,
        disk => u32::from(disk),
    };
    if disk != 0 {
        return Err(OfficeError::Unsupported(SPLIT_ARCHIVE));
    }
    let record = Record {
        header,
        name,
        flags,
        method,
        sizes: Sizes {
            crc: u32_at(header, 16),
            compressed,
            uncompressed,
        },
        local,
    };
    Ok((record, next))
}

/// Accepted members as disjoint [local header, end of data) ranges.
#[derive(Default)]
struct Ranges(BTreeMap<u64, u64>);
impl Ranges {
    fn check(&self, start: u64, end: u64, limit: u64) -> Result<(), OfficeError> {
        if end > limit {
            return Err(OfficeError::Malformed("member outside data region"));
        }
        let before = self.0.range(..=start).next_back();
        let after = self.0.range(start..).next();
        if before.is_some_and(|(_, &previous)| previous > start)
            || after.is_some_and(|(&next, _)| next < end)
        {
            return Err(OfficeError::Malformed("overlapping ZIP entries"));
        }
        Ok(())
    }
}

/// Checks a member's local header and data descriptor against its central record.
/// The range is checked before each read, so accepted reads are disjoint and lie in
/// the data region: extra fields enlarge headers, never the bytes read in total.
fn local_member(
    reader: &mut CapturedReader,
    record: &Record<'_>,
    data_region: u64,
    ranges: &mut Ranges,
    variable: &mut Vec<u8>,
) -> Result<(), OfficeError> {
    let start = record.local;
    let minimal = start
        .checked_add(30 + record.name.len() as u64)
        .and_then(|end| end.checked_add(record.sizes.compressed))
        .ok_or(OfficeError::Malformed("member outside data region"))?;
    ranges.check(start, minimal, data_region)?;
    let header = read_at::<30>(reader, start)?;
    if &header[..4] != LOCAL
        || u16_at(&header, 6) != record.flags
        || u16_at(&header, 8) != record.method
        || usize::from(u16_at(&header, 26)) != record.name.len()
    {
        return Err(OfficeError::Malformed("local/central header mismatch"));
    }
    let extra_len = usize::from(u16_at(&header, 28));
    let mut end = minimal + extra_len as u64;
    ranges.check(start, end, data_region)?;
    // One reused buffer holds the local name and extra field (at most 69,631 bytes).
    variable.clear();
    variable.resize(record.name.len() + extra_len, 0);
    reader.read_exact(variable)?;
    let (name, extra) = variable.split_at(record.name.len());
    if name != record.name {
        return Err(OfficeError::Malformed("local/central name mismatch"));
    }
    let zip64 = zip64_record(extra, name)?;
    if record.flags & DATA_DESCRIPTOR == 0 {
        // A local ZIP64 record holds both sizes, original size first (APPNOTE 4.5.3).
        let wide = |offset: usize, field: usize| match (u32_at(&header, offset), zip64) {
            (SATURATED, Some(data)) if data.len() >= 16 => Ok(u64_at(data, field * 8)),
            (SATURATED, _) => Err(OfficeError::Malformed(
                "local ZIP64 record lacks the member sizes",
            )),
            (value, _) => Ok(u64::from(value)),
        };
        let local = Sizes {
            crc: u32_at(&header, 14),
            compressed: wide(18, 1)?,
            uncompressed: wide(22, 0)?,
        };
        if local != record.sizes {
            return Err(OfficeError::Malformed(
                "local/central size or checksum mismatch",
            ));
        }
    } else {
        if read_at::<4>(reader, end)? == *DESCRIPTOR {
            end += 4;
        }
        // With a local ZIP64 record the descriptor sizes are eight bytes each.
        let values = read_at::<20>(reader, end)?;
        let (descriptor, length) = match zip64 {
            Some(_) => (
                Sizes {
                    crc: u32_at(&values, 0),
                    compressed: u64_at(&values, 4),
                    uncompressed: u64_at(&values, 12),
                },
                20,
            ),
            None => (
                Sizes {
                    crc: u32_at(&values, 0),
                    compressed: u64::from(u32_at(&values, 4)),
                    uncompressed: u64::from(u32_at(&values, 8)),
                },
                12,
            ),
        };
        if descriptor != record.sizes {
            return Err(OfficeError::Malformed("ZIP data descriptor mismatch"));
        }
        end += length;
        ranges.check(start, end, data_region)?;
    }
    ranges.0.insert(start, end);
    Ok(())
}

/// Validates the container. Returns the canonical central directory and its end
/// record, which zip indexes at virtual offset zero.
pub(crate) fn preflight(reader: &mut CapturedReader) -> Result<Vec<u8>, OfficeError> {
    let length = reader.length;
    if length < 22 {
        return Err(OfficeError::Malformed("not a ZIP container"));
    }
    if read_at::<8>(reader, 0)? == COMPOUND_FILE {
        return Err(OfficeError::Unsupported(LEGACY_OR_PROTECTED));
    }
    // The end record is the last signature whose comment reaches the end exactly.
    let tail_len = length.min(TAIL) as usize;
    let mut tail = vec![0; tail_len];
    reader.seek(SeekFrom::Start(length - tail_len as u64))?;
    reader.read_exact(&mut tail)?;
    let end = (0..=tail_len - 22)
        .rev()
        .find(|&i| {
            &tail[i..i + 4] == END && i + 22 + usize::from(u16_at(&tail, i + 20)) == tail_len
        })
        .ok_or(OfficeError::Malformed("missing ZIP end record"))?;
    let record = &tail[end..];
    // An archive comment is skipped, unless it could pass for archive records.
    if record[22..].windows(4).any(|magic| {
        [LOCAL, CENTRAL, END, ZIP64_END, ZIP64_LOCATOR]
            .iter()
            .any(|signature| magic == signature.as_slice())
    }) {
        return Err(OfficeError::Unsupported(HIDDEN_DIRECTORY));
    }
    if u16_at(record, 4) != 0 || u16_at(record, 6) != 0 || u16_at(record, 8) != u16_at(record, 10) {
        return Err(OfficeError::Unsupported(SPLIT_ARCHIVE));
    }
    let count = usize::from(u16_at(record, 10));
    let central_size = u64::from(u32_at(record, 12));
    let central_start = u64::from(u32_at(record, 16));
    let end_position = length - tail_len as u64 + end as u64;
    let bounded = central_start.checked_add(central_size) == Some(end_position);
    // ZIP64 end records sit between the central directory and this record, which
    // then holds 0xFFFF/0xFFFFFFFF markers or no longer bounds the directory.
    let marked = u16_at(record, 10) == u16::MAX
        || u32_at(record, 12) == SATURATED
        || u32_at(record, 16) == SATURATED;
    if (marked || !bounded) && end_position >= 20 {
        let locator = if end >= 20 {
            tail[end - 20..end - 16] == *ZIP64_LOCATOR
        } else {
            read_at::<4>(reader, end_position - 20)? == *ZIP64_LOCATOR
        };
        if locator {
            return Err(OfficeError::Unsupported(ZIP64_ARCHIVE));
        }
    }
    if count > MAX_ENTRIES {
        return Err(OfficeError::LimitExceeded(PART_COUNT_LIMIT));
    }
    if central_size > MAX_CENTRAL {
        return Err(OfficeError::LimitExceeded("central metadata"));
    }
    if !bounded {
        return Err(OfficeError::Malformed("central directory bounds"));
    }
    let mut end_record = [0; 22];
    end_record.copy_from_slice(&record[..22]);
    drop(tail);
    // The 8 MiB bound precedes this allocation; the canonical copy is never larger.
    let central_size = central_size as usize;
    let mut metadata = vec![0; central_size + 22];
    reader.seek(SeekFrom::Start(central_start))?;
    reader.read_exact(&mut metadata)?;
    if metadata[central_size..] != end_record {
        return Err(OfficeError::Malformed(
            "ZIP end record changed during preflight",
        ));
    }
    let directory = &metadata[..central_size];
    let mut canonical = Vec::with_capacity(central_size + 22);
    let mut position = 0;
    let mut names = BTreeSet::new();
    let mut name_bytes = 0usize;
    let mut ranges = Ranges::default();
    let mut variable = Vec::new();
    while position < central_size {
        if names.len() >= count {
            return Err(OfficeError::Malformed("actual central record count"));
        }
        let (record, next) = central_record(directory, position)?;
        name_bytes += record.name.len();
        if name_bytes > 1024 * 1024 {
            return Err(OfficeError::LimitExceeded("retained package names"));
        }
        let name = std::str::from_utf8(record.name)
            .map_err(|_| OfficeError::Unsupported(NAME_ENCODING))?;
        if !name.is_ascii() && record.flags & UTF8_NAMES == 0 {
            return Err(OfficeError::Unsupported(NAME_ENCODING));
        }
        validate_name(name)?;
        if !names.insert(name.to_owned()) {
            return Err(OfficeError::Malformed("duplicate ZIP member"));
        }
        if (u32_at(record.header, 38) >> 16) & 0xf000 == 0xa000 {
            return Err(OfficeError::Malformed("ZIP symbolic link"));
        }
        local_member(reader, &record, central_start, &mut ranges, &mut variable)?;
        // zip indexes the validated name and resolved 32-bit values only.
        let narrow = |value: u64| {
            u32::try_from(value).map_err(|_| OfficeError::LimitExceeded("member size"))
        };
        let start = canonical.len();
        canonical.extend_from_slice(record.header);
        canonical[start + 20..start + 24]
            .copy_from_slice(&narrow(record.sizes.compressed)?.to_le_bytes());
        canonical[start + 24..start + 28]
            .copy_from_slice(&narrow(record.sizes.uncompressed)?.to_le_bytes());
        canonical[start + 30..start + 36].fill(0);
        canonical[start + 42..start + 46].copy_from_slice(&narrow(record.local)?.to_le_bytes());
        canonical.extend_from_slice(record.name);
        position = next;
    }
    if names.len() != count {
        return Err(OfficeError::Malformed("central count mismatch"));
    }
    // Conservative reservation for pinned ZIP index capacities, duplicate strings,
    // map buckets and per-entry structs. zip retains no extra field or comment.
    if count * 1024 + name_bytes * 3 > 16 * 1024 * 1024 {
        return Err(OfficeError::LimitExceeded("package index retained bytes"));
    }
    // The sole metadata interpretation supplied to the eager index: never let zip
    // find an alternate end record after a central metadata error.
    if canonical
        .windows(4)
        .any(|magic| magic == END || magic == ZIP64_END || magic == ZIP64_LOCATOR)
    {
        return Err(OfficeError::Malformed(
            "ambiguous ZIP end signatures in central metadata",
        ));
    }
    let canonical_size = u32::try_from(canonical.len())
        .map_err(|_| OfficeError::LimitExceeded("central metadata"))?;
    end_record[12..16].copy_from_slice(&canonical_size.to_le_bytes());
    end_record[16..22].fill(0);
    canonical.extend_from_slice(&end_record);
    Ok(canonical)
}
