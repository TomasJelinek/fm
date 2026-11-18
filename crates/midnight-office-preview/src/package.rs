//! Preflight actual central records before zip allocates its eager index.
use crate::{
    Cancellation, OfficeError, OfficeSource,
    index::IndexReader,
    source::{CapturedReader, Decoded, MAX_WORK, Work},
};
use std::{
    collections::BTreeSet,
    io::{Read, Seek, SeekFrom},
    sync::Arc,
};

pub(crate) struct Package {
    archive: zip::ZipArchive<IndexReader>,
    pub work: Arc<Work>,
}
impl Package {
    pub fn open(source: Arc<dyn OfficeSource>, cancel: &Cancellation) -> Result<Self, OfficeError> {
        let length = source.len();
        if length > MAX_WORK {
            return Err(OfficeError::LimitExceeded("encoded container"));
        }
        let work = Work::new(cancel);
        let mut reader = CapturedReader {
            source,
            length,
            position: 0,
            work: work.clone(),
        };
        let metadata = preflight(&mut reader)?;
        let (reader, indexing) = IndexReader::new(reader, metadata);
        let archive = zip::ZipArchive::with_config(
            zip::read::Config {
                archive_offset: zip::read::ArchiveOffset::Known(0),
            },
            reader,
        )?;
        IndexReader::finish(&indexing)?;
        Ok(Self { archive, work })
    }
    pub fn begin(&self, cancel: &Cancellation) -> Result<(), OfficeError> {
        self.work.begin(cancel)
    }
    pub fn check(&self) -> Result<(), OfficeError> {
        self.work.check()
    }
    pub fn contains(&self, name: &str) -> bool {
        self.archive.index_for_name(name).is_some()
    }
    /// Bounded scalar package metadata under the current operation's shared work budget.
    pub fn read_small(&mut self, name: &str, maximum: usize) -> Result<Vec<u8>, OfficeError> {
        if maximum > 64 * 1024 {
            return Err(OfficeError::LimitExceeded("small package member bound"));
        }
        self.work.check()?;
        let file = self.archive.by_name(name)?;
        let mut reader = Decoded {
            inner: file,
            work: self.work.clone(),
        };
        let mut bytes = vec![0; maximum + 1];
        let mut filled = 0;
        while filled < bytes.len() {
            self.work.check()?;
            let count = reader.read(&mut bytes[filled..])?;
            if count == 0 {
                break;
            }
            filled += count;
        }
        if filled > maximum {
            return Err(OfficeError::LimitExceeded("small package member"));
        }
        self.work.check()?;
        bytes.truncate(filled);
        Ok(bytes)
    }
    pub fn xml(&mut self, name: &str) -> Result<crate::xml::Xml<impl Read + '_>, OfficeError> {
        let file = self.archive.by_name(name)?;
        // Indexed ZipFile borrows metadata: its Drop does NOT drain the tail.
        Ok(crate::xml::Xml::new(Decoded {
            inner: file,
            work: self.work.clone(),
        }))
    }
}

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
fn read_at<const N: usize>(
    reader: &mut CapturedReader,
    offset: u64,
) -> Result<[u8; N], OfficeError> {
    reader.seek(SeekFrom::Start(offset))?;
    let mut bytes = [0; N];
    reader.read_exact(&mut bytes)?;
    Ok(bytes)
}

fn preflight(reader: &mut CapturedReader) -> Result<Vec<u8>, OfficeError> {
    let length = reader.length;
    if length < 22 {
        return Err(OfficeError::Malformed("not a ZIP container"));
    }
    let magic = read_at::<8>(reader, 0)?;
    if magic == [0xd0, 0xcf, 0x11, 0xe0, 0xa1, 0xb1, 0x1a, 0xe1] {
        return Err(OfficeError::Unsupported(
            "legacy binary or encrypted Office container",
        ));
    }
    // EOCD plus bounded ZIP comment. Do not trust the first matching byte pattern.
    let tail_len = length.min(65_557) as usize;
    let mut tail = vec![0; tail_len];
    reader.seek(SeekFrom::Start(length - tail_len as u64))?;
    reader.read_exact(&mut tail)?;
    let end = (0..=tail_len - 22)
        .rev()
        .find(|&i| {
            tail[i..i + 4] == *b"PK\x05\x06"
                && i + 22 + usize::from(u16_at(&tail, i + 20)) == tail_len
        })
        .ok_or(OfficeError::Malformed("missing ZIP end record"))?;
    let record = &tail[end..];
    if u16_at(record, 20) != 0 {
        return Err(OfficeError::Unsupported(
            "ZIP archive comments in first slice",
        ));
    }
    if u16_at(record, 4) != 0 || u16_at(record, 6) != 0 || u16_at(record, 8) != u16_at(record, 10) {
        return Err(OfficeError::Unsupported("multi-disk ZIP"));
    }
    let count = usize::from(u16_at(record, 10));
    let central_size = u64::from(u32_at(record, 12));
    let central_start = u64::from(u32_at(record, 16));
    if count > 10_000 {
        return Err(OfficeError::LimitExceeded("package entries"));
    }
    if central_size > 8 * 1024 * 1024 {
        return Err(OfficeError::LimitExceeded("central metadata"));
    }
    let end_position = length - tail_len as u64 + end as u64;
    if central_start.checked_add(central_size) != Some(end_position) {
        return Err(OfficeError::Malformed("central directory bounds or ZIP64"));
    }
    let mut end_record = [0; 22];
    end_record.copy_from_slice(&record[..22]);
    drop(tail);
    // This is the sole metadata interpretation supplied to the eager index.
    // Validate the immutable bytes below; never let ZIP search payload or an
    // alternate EOCD after a central metadata error. The 8 MiB bound precedes
    // allocation; actual count/name limits constrain accepted snapshots further.
    let mut metadata = vec![0; central_size as usize + 22];
    reader.seek(SeekFrom::Start(central_start))?;
    reader.read_exact(&mut metadata)?;
    if metadata[central_size as usize..] != end_record {
        return Err(OfficeError::Malformed(
            "ZIP end record changed during preflight",
        ));
    }
    if metadata[..central_size as usize]
        .windows(4)
        .any(|magic| magic == b"PK\x05\x06" || magic == b"PK\x06\x06" || magic == b"PK\x06\x07")
    {
        return Err(OfficeError::Unsupported(
            "ambiguous ZIP end signatures in central metadata",
        ));
    }
    let mut position = central_start;
    let mut names = BTreeSet::new();
    let mut name_bytes = 0usize;
    let mut ranges = Vec::new();
    while position < end_position {
        if names.len() >= 10_000 || names.len() >= count {
            return Err(OfficeError::Malformed("actual central record count"));
        }
        let relative = (position - central_start) as usize;
        let header_bytes = metadata
            .get(relative..relative + 46)
            .filter(|_| position + 46 <= end_position)
            .ok_or(OfficeError::Malformed("truncated central record"))?;
        let mut header = [0; 46];
        header.copy_from_slice(header_bytes);
        if &header[..4] != b"PK\x01\x02" {
            return Err(OfficeError::Malformed("central record"));
        }
        let flags = u16_at(&header, 8);
        if flags & (1 | 0x40 | 0x2000) != 0 {
            return Err(OfficeError::Encrypted);
        }
        if flags & !(0x800 | 8 | 6) != 0 {
            return Err(OfficeError::Unsupported("ZIP flags"));
        }
        let method = u16_at(&header, 10);
        if method != 0 && method != 8 {
            return Err(OfficeError::Unsupported("ZIP compression"));
        }
        let name_len = usize::from(u16_at(&header, 28));
        let extra_len = u16_at(&header, 30);
        let comment_len = u16_at(&header, 32);
        if extra_len != 0 || comment_len != 0 {
            // Avoid unaccounted third-party extra-field decoding/Unicode aliases.
            return Err(OfficeError::Unsupported(
                "ZIP entry extras/comments in first slice",
            ));
        }
        if u16_at(&header, 34) != 0 {
            return Err(OfficeError::Unsupported("multi-disk entry"));
        }
        if name_len == 0 || name_len > 4096 {
            return Err(OfficeError::LimitExceeded("entry name"));
        }
        name_bytes += name_len;
        if name_bytes > 1024 * 1024 {
            return Err(OfficeError::LimitExceeded("retained package names"));
        }
        let next = position + 46 + name_len as u64;
        if next > end_position {
            return Err(OfficeError::Malformed("central record bounds"));
        }
        let name = &metadata[relative + 46..relative + 46 + name_len];
        let name =
            std::str::from_utf8(name).map_err(|_| OfficeError::Unsupported("non-UTF8 ZIP name"))?;
        if !name.is_ascii() && flags & 0x800 == 0 {
            return Err(OfficeError::Unsupported("ambiguous ZIP name encoding"));
        }
        validate_name(name)?;
        if !names.insert(name.to_owned()) {
            return Err(OfficeError::Malformed("duplicate ZIP member"));
        }
        let mode = u32_at(&header, 38) >> 16;
        if mode & 0xf000 == 0xa000 {
            return Err(OfficeError::Malformed("ZIP symbolic link"));
        }
        let local = u64::from(u32_at(&header, 42));
        let local_header = read_at::<30>(reader, local)?;
        if &local_header[..4] != b"PK\x03\x04"
            || u16_at(&local_header, 6) != flags
            || u16_at(&local_header, 8) != method
            || usize::from(u16_at(&local_header, 26)) != name_len
        {
            return Err(OfficeError::Malformed("local/central header mismatch"));
        }
        if u16_at(&local_header, 28) != 0 {
            return Err(OfficeError::Unsupported("local ZIP extras in first slice"));
        }
        let mut local_name = vec![0; name_len];
        reader.read_exact(&mut local_name)?;
        if local_name != name.as_bytes() {
            return Err(OfficeError::Malformed("local/central name mismatch"));
        }
        let compressed = u64::from(u32_at(&header, 20));
        if compressed == u64::from(u32::MAX) || u32_at(&header, 24) == u32::MAX {
            return Err(OfficeError::Unsupported("ZIP64"));
        }
        if flags & 8 == 0 && (local_header[14..26] != header[16..28]) {
            return Err(OfficeError::Malformed(
                "local/central size or checksum mismatch",
            ));
        }
        let mut data_end = local + 30 + name_len as u64 + compressed;
        if data_end > central_start {
            return Err(OfficeError::Malformed("member outside data region"));
        }
        if flags & 8 != 0 {
            let descriptor = read_at::<12>(reader, data_end)?;
            let values = if &descriptor[..4] == b"PK\x07\x08" {
                data_end += 4;
                read_at::<12>(reader, data_end)?
            } else {
                descriptor
            };
            if values != header[16..28] {
                return Err(OfficeError::Malformed("ZIP data descriptor mismatch"));
            }
            data_end += 12;
            if data_end > central_start {
                return Err(OfficeError::Malformed("ZIP data descriptor bounds"));
            }
        }
        ranges.push((local, data_end));
        position = next;
    }
    if names.len() != count {
        return Err(OfficeError::Malformed("central count mismatch"));
    }
    ranges.sort_unstable();
    if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
        return Err(OfficeError::Malformed("overlapping ZIP entries"));
    }
    // Conservative reservation for pinned ZIP index capacities, duplicate strings,
    // map buckets and per-entry structs. Extras/comments are excluded above.
    if count * 1024 + name_bytes * 3 > 16 * 1024 * 1024 {
        return Err(OfficeError::LimitExceeded("package index retained bytes"));
    }
    // Present the validated central directory at virtual offset zero. ZIP's
    // local-header offsets remain the original captured offsets because the
    // explicit archive offset is zero. No central offsets are used for reads
    // after indexing; by_name uses the retained original local-header offset.
    metadata[central_size as usize + 16..central_size as usize + 20].fill(0);
    Ok(metadata)
}

pub(crate) fn validate_name(name: &str) -> Result<(), OfficeError> {
    if name.starts_with('/')
        || name.contains(['\\', ':', '%', '?', '#'])
        || name.chars().any(char::is_control)
        || name
            .trim_end_matches('/')
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(OfficeError::Malformed("unsafe package name"));
    }
    Ok(())
}

pub(crate) fn relationship_target(base: &str, target: &str) -> Result<String, OfficeError> {
    if target.len() > 4096
        || target.starts_with("//")
        || target.contains(['\\', ':', '%', '?', '#'])
        || target.chars().any(char::is_control)
    {
        return Err(OfficeError::Malformed("unsafe relationship target"));
    }
    let mut parts: Vec<&str> = if target.starts_with('/') {
        Vec::new()
    } else {
        base.rsplit_once('/')
            .map_or(Vec::new(), |(parent, _)| parent.split('/').collect())
    };
    for part in target.strip_prefix('/').unwrap_or(target).split('/') {
        match part {
            "" => return Err(OfficeError::Malformed("empty relationship component")),
            "." => (),
            ".." => {
                if parts.pop().is_none() {
                    return Err(OfficeError::Malformed("relationship escapes package"));
                }
            }
            part => parts.push(part),
        }
    }
    let result = parts.join("/");
    validate_name(&result)?;
    Ok(result)
}
