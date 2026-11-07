//! Read-only archive parsing. This code runs only inside the isolated worker.
mod formats;
mod tree;
use midnight_filesystem_api::{ErrorCode, Format, Kind, MAX_BATCH, MAX_READ, Node};
use std::{
    collections::HashMap,
    io::{self, Read},
};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_source: u64,
    pub max_entry: u64,
    pub max_total: u64,
    pub max_entries: usize,
    pub max_names: usize,
    pub max_ratio: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_source: 64 * 1024 * 1024,
            max_entry: 64 * 1024 * 1024,
            max_total: 128 * 1024 * 1024,
            max_entries: 100_000,
            max_names: 8 * 1024 * 1024,
            max_ratio: 200,
        }
    }
}

#[derive(Debug)]
pub struct Error {
    pub code: ErrorCode,
    pub message: String,
}
impl Error {
    pub(crate) fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
    pub(crate) fn limit() -> Self {
        Self::new(
            ErrorCode::Limit,
            "Archive exceeds the read-only worker budget",
        )
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}
impl std::error::Error for Error {}
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::new(ErrorCode::Corrupt, error.to_string())
    }
}

/// One bounded, immutable mount. Member bytes are verified before publication.
/// No extraction, path opening or host association is implemented here.
pub struct Archive {
    nodes: Vec<Node>,
    contents: HashMap<u64, Vec<u8>>,
    directories: HashMap<(u64, Vec<u8>), u64>,
    limits: Limits,
    names: usize,
    total: u64,
}
impl Archive {
    pub fn open(format: Format, source: Vec<u8>, limits: Limits) -> Result<Self, Error> {
        if source.len() as u64 > limits.max_source {
            return Err(Error::limit());
        }
        let mut result = Self {
            nodes: vec![],
            contents: HashMap::new(),
            directories: HashMap::new(),
            limits,
            names: 0,
            total: 0,
        };
        match format {
            Format::Zip => result.zip(&source)?,
            Format::Tar => result.tar(&source)?,
            Format::Gzip | Format::TarGzip => {
                let budget = limits
                    .max_total
                    .min((source.len() as u64).saturating_mul(limits.max_ratio));
                let bytes =
                    bounded_read(flate2::read::MultiGzDecoder::new(source.as_slice()), budget)?;
                if format == Format::TarGzip {
                    result.tar(&bytes)?;
                } else {
                    result.add(b"content", "content", Kind::File, Some(bytes), true)?;
                }
            }
            Format::SevenZip => result.sevenzip(&source)?,
        }
        if result.total > (source.len() as u64).saturating_mul(limits.max_ratio) {
            return Err(Error::limit());
        }
        Ok(result)
    }

    pub fn list(&self, directory: u64, after: u64) -> Result<Vec<Node>, Error> {
        if directory != 1
            && !self
                .nodes
                .iter()
                .any(|n| n.id == directory && n.kind == Kind::Directory)
        {
            return Err(Error::new(ErrorCode::NotFound, "Unknown archive directory"));
        }
        // IDs are increasing and remain stable after sorting, filtering and pagination.
        Ok(self
            .nodes
            .iter()
            .filter(|n| n.parent == directory && n.id > after)
            .take(MAX_BATCH)
            .cloned()
            .collect())
    }

    pub fn read(&mut self, entry: u64, offset: u64, length: u32) -> Result<(Vec<u8>, bool), Error> {
        if length == 0 || length > MAX_READ {
            return Err(Error::limit());
        }
        let node = self
            .nodes
            .iter()
            .find(|n| n.id == entry)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Unknown archive member"))?;
        if !node.readable {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Unsafe, encrypted, linked or unsupported archive member",
            ));
        }
        let data = self
            .contents
            .get(&entry)
            .ok_or_else(|| Error::new(ErrorCode::Unsupported, "Directory has no byte content"))?;
        let start = usize::try_from(offset.min(data.len() as u64)).map_err(|_| Error::limit())?;
        let end = start.saturating_add(length as usize).min(data.len());
        Ok((data[start..end].to_vec(), end == data.len()))
    }
}

pub(crate) fn bounded_read(mut reader: impl Read, limit: u64) -> Result<Vec<u8>, Error> {
    let mut result = Vec::new();
    let mut buffer = [0; 32 * 1024];
    loop {
        let length = reader.read(&mut buffer)?;
        if length == 0 {
            return Ok(result);
        }
        if result.len() as u64 + length as u64 > limit {
            return Err(Error::limit());
        }
        result.extend_from_slice(&buffer[..length]);
    }
}
