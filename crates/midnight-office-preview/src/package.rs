//! Package members through zip's eager index over the preflighted central directory.
use crate::{
    Cancellation, OfficeError, OfficeSource,
    index::IndexReader,
    model::CONTAINER_LIMIT,
    preflight::preflight,
    source::{CapturedReader, Decoded, MAX_WORK, Work},
};
use std::{io::Read, sync::Arc};

pub(crate) struct Package {
    archive: zip::ZipArchive<IndexReader>,
    pub work: Arc<Work>,
}
impl Package {
    pub fn open(source: Arc<dyn OfficeSource>, cancel: &Cancellation) -> Result<Self, OfficeError> {
        let length = source.len();
        if length > MAX_WORK {
            return Err(OfficeError::LimitExceeded(CONTAINER_LIMIT));
        }
        let work = Work::new(cancel);
        let mut reader = CapturedReader {
            source,
            length,
            position: 0,
            work: work.clone(),
        };
        // zip indexes only this canonical directory: no extra fields or comments.
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
