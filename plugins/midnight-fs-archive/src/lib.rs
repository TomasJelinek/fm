//! Read-only archive parsing. This code runs only inside the isolated worker.
pub mod broker;
mod formats;
mod sevenzip;
pub mod storage;
mod streams;
mod tar;
mod timestamps;
mod tree;
use midnight_filesystem_api::StagePurpose;
use midnight_filesystem_api::{ErrorCode, Format, Kind, MAX_BATCH, MAX_READ, Node};
use std::rc::Rc;
use std::{
    collections::HashMap,
    io::{self, Read},
};
use storage::{Content, Reader, Stage, Storage};

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
            max_source: u64::MAX,
            max_entry: u64::MAX,
            max_total: u64::MAX,
            max_entries: 100_000,
            max_names: 8 * 1024 * 1024,
            max_ratio: u64::MAX,
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
    pub(crate) fn storage(error: io::Error) -> Self {
        let code = if error.kind() == io::ErrorKind::Interrupted {
            ErrorCode::Cancelled
        } else {
            ErrorCode::Io
        };
        Self::new(code, error.to_string())
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
/// Decoded bytes are staged through host capabilities; no native path is opened here.
pub struct Archive {
    nodes: Vec<Node>,
    contents: HashMap<u64, Content>,
    storage: Rc<dyn Storage>,
    directories: HashMap<(u64, Vec<u8>), u64>,
    children: HashMap<u64, Vec<usize>>,
    zip: Option<zip::ZipArchive<Reader>>,
    zip_members: HashMap<u64, usize>,
    tar_content: Option<tar::Content>,
    sevenzip_content: Option<sevenzip::Content>,
    limits: Limits,
    names: usize,
    total: u64,
}
impl Archive {
    /// Owned in-memory fixtures. The executable uses `open_stream` with the host broker.
    pub fn open(format: Format, source: Vec<u8>, limits: Limits) -> Result<Self, Error> {
        let length = source.len() as u64;
        Self::open_stream(
            format,
            Box::new(std::io::Cursor::new(source)),
            length,
            Rc::new(storage::MemoryStorage::default()),
            limits,
        )
    }

    /// Production entry point: source and decoded storage are host-brokered streams.
    pub fn open_stream(
        format: Format,
        source: Box<dyn storage::ReadSeek>,
        source_length: u64,
        storage: Rc<dyn Storage>,
        limits: Limits,
    ) -> Result<Self, Error> {
        if source_length > limits.max_source {
            return Err(Error::limit());
        }
        let source = std::io::BufReader::with_capacity(MAX_READ as usize, source);
        let mut result = Self {
            nodes: vec![],
            contents: HashMap::new(),
            storage,
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
            Format::Tar => result.tar(source, source_length, None)?,
            Format::SevenZip => result.sevenzip(source, source_length)?,
            Format::Gzip
            | Format::TarGzip
            | Format::Xz
            | Format::TarXz
            | Format::Bzip2
            | Format::TarBzip2
            | Format::Zstd
            | Format::TarZstd => {
                let budget = limits
                    .max_total
                    .min(source_length.saturating_mul(limits.max_ratio));
                let mut stage =
                    Stage::new(result.storage.clone(), StagePurpose::Scratch, None, budget)?;
                streams::decode(format, source, &mut stage)?;
                let content = stage.finish()?;
                if matches!(
                    format,
                    Format::TarGzip | Format::TarXz | Format::TarBzip2 | Format::TarZstd
                ) {
                    let reader: Box<dyn storage::ReadSeek> =
                        Box::new(storage::StoredReader::new(result.storage.clone(), content));
                    result.tar(
                        std::io::BufReader::with_capacity(MAX_READ as usize, reader),
                        content.length,
                        Some(content),
                    )?;
                } else {
                    result.add(b"content", "content", Kind::File, Some(content), true)?;
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
        let content = if let Some(content) = self.contents.get(&entry) {
            *content
        } else {
            let content = self.prepare(entry)?;
            self.contents.insert(entry, content);
            content
        };
        let offset = offset.min(content.length);
        let length = (content.length - offset).min(u64::from(length)) as u32;
        let bytes = if length == 0 {
            Vec::new()
        } else {
            self.storage
                .read(content.stage, content.offset + offset, length)
                .map_err(Error::storage)?
        };
        Ok((bytes, offset + u64::from(length) == content.length))
    }

    /// Complete format integrity checks before publishing a sealed content range.
    pub fn prepare(&mut self, entry: u64) -> Result<Content, Error> {
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
        if let Some(content) = self.contents.get(&entry) {
            return Ok(*content);
        }
        if let Some(tar) = &mut self.tar_content {
            return tar.prepare(entry, self.storage.clone());
        }
        if let Some(sevenzip) = &mut self.sevenzip_content {
            return sevenzip.decode(entry, self.storage.clone());
        }
        if let Some(index) = self.zip_members.get(&entry).copied() {
            let zip = self
                .zip
                .as_mut()
                .ok_or_else(|| Error::new(ErrorCode::Corrupt, "Missing ZIP source"))?;
            let mut stage = Stage::new(
                self.storage.clone(),
                StagePurpose::Member { entry },
                declared_size,
                declared_size.unwrap_or(u64::MAX),
            )?;
            stage.copy(zip.by_index(index).map_err(formats::zip_error)?)?;
            return stage.finish();
        }
        Err(Error::new(
            ErrorCode::Unsupported,
            "Directory has no byte content",
        ))
    }

    fn node(&self, id: u64) -> Option<&Node> {
        self.nodes.get(usize::try_from(id.checked_sub(2)?).ok()?)
    }
}
