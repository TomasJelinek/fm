//! Read-only archive parsing. This code runs only inside the isolated worker.
mod formats;
mod sevenzip;
mod streams;
mod tar;
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

/// One bounded, immutable mount. ZIP members and 7z blocks are verified on first read.
/// No extraction, path opening or host association is implemented here.
pub struct Archive {
    nodes: Vec<Node>,
    contents: HashMap<u64, Vec<u8>>,
    directories: HashMap<(u64, Vec<u8>), u64>,
    children: HashMap<u64, Vec<usize>>,
    zip: Option<zip::ZipArchive<std::io::Cursor<Vec<u8>>>>,
    zip_members: HashMap<u64, usize>,
    tar_content: Option<tar::Content>,
    sevenzip_content: Option<sevenzip::Content>,
    limits: Limits,
    names: usize,
    total: u64,
}
impl Archive {
    pub fn open(format: Format, source: Vec<u8>, limits: Limits) -> Result<Self, Error> {
        if source.len() as u64 > limits.max_source {
            return Err(Error::limit());
        }
        let source_length = source.len() as u64;
        let mut result = Self {
            nodes: vec![],
            contents: HashMap::new(),
            directories: HashMap::new(),
            children: HashMap::new(),
            zip: None,
            zip_members: HashMap::new(),
            tar_content: None,
            sevenzip_content: None,
            limits,
            names: 0,
            total: 0,
        };
        match format {
            Format::Zip => result.zip(source)?,
            Format::Tar => result.tar(source)?,
            Format::Gzip | Format::TarGzip => {
                let budget = limits
                    .max_total
                    .min((source.len() as u64).saturating_mul(limits.max_ratio));
                let bytes =
                    bounded_read(flate2::read::MultiGzDecoder::new(source.as_slice()), budget)?;
                if format == Format::TarGzip {
                    result.tar(bytes)?;
                } else {
                    result.add(b"content", "content", Kind::File, Some(bytes), true)?;
                }
            }
            Format::SevenZip => result.sevenzip(source)?,
            Format::Xz
            | Format::TarXz
            | Format::Bzip2
            | Format::TarBzip2
            | Format::Zstd
            | Format::TarZstd => {
                let budget = limits
                    .max_total
                    .min(source_length.saturating_mul(limits.max_ratio));
                let bytes = streams::decode(format, &source, budget, limits.max_entry)?;
                if matches!(format, Format::TarXz | Format::TarBzip2 | Format::TarZstd) {
                    result.tar(bytes)?;
                } else {
                    result.add(b"content", "content", Kind::File, Some(bytes), true)?;
                }
            }
        }
        if result.total > source_length.saturating_mul(limits.max_ratio) {
            return Err(Error::limit());
        }
        Ok(result)
    }

    pub fn list(&self, directory: u64, after: u64) -> Result<Vec<Node>, Error> {
        if directory != 1
            && self
                .node(directory)
                .is_none_or(|n| n.kind != Kind::Directory)
        {
            return Err(Error::new(ErrorCode::NotFound, "Unknown archive directory"));
        }
        // IDs are increasing and remain stable after sorting, filtering and pagination.
        let Some(children) = self.children.get(&directory) else {
            return Ok(Vec::new());
        };
        let start = children.partition_point(|index| self.nodes[*index].id <= after);
        Ok(children[start..]
            .iter()
            .take(MAX_BATCH)
            .map(|index| self.nodes[*index].clone())
            .collect())
    }

    pub fn read(&mut self, entry: u64, offset: u64, length: u32) -> Result<(Vec<u8>, bool), Error> {
        if length == 0 || length > MAX_READ {
            return Err(Error::limit());
        }
        let node = self
            .node(entry)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Unknown archive member"))?;
        if !node.readable {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Unsafe, encrypted, linked or unsupported archive member",
            ));
        }
        let declared_size = node.size;
        if let Some(tar) = &self.tar_content {
            return tar.read(entry, offset, length);
        }
        if !self.contents.contains_key(&entry)
            && let Some(sevenzip) = &mut self.sevenzip_content
        {
            // The whole requested block is staged and verified before any cache publication.
            self.contents.extend(sevenzip.decode(entry, self.limits)?);
        }
        if !self.contents.contains_key(&entry)
            && let Some(index) = self.zip_members.get(&entry).copied()
        {
            let zip = self
                .zip
                .as_mut()
                .ok_or_else(|| Error::new(ErrorCode::Corrupt, "Missing ZIP source"))?;
            let data = bounded_read(
                zip.by_index(index).map_err(formats::zip_error)?,
                self.limits.max_entry,
            )?;
            if Some(data.len() as u64) != declared_size {
                return Err(Error::new(
                    ErrorCode::Corrupt,
                    "ZIP member length differs from its index",
                ));
            }
            self.contents.insert(entry, data);
        }
        let data = self
            .contents
            .get(&entry)
            .ok_or_else(|| Error::new(ErrorCode::Unsupported, "Directory has no byte content"))?;
        read_slice(data, offset, length)
    }

    fn node(&self, id: u64) -> Option<&Node> {
        self.nodes.get(usize::try_from(id.checked_sub(2)?).ok()?)
    }
}

fn read_slice(data: &[u8], offset: u64, length: u32) -> Result<(Vec<u8>, bool), Error> {
    let start = usize::try_from(offset.min(data.len() as u64)).map_err(|_| Error::limit())?;
    let end = start.saturating_add(length as usize).min(data.len());
    Ok((data[start..end].to_vec(), end == data.len()))
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
