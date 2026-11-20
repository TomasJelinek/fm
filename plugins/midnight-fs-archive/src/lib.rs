//! Read-only archive parsing. This code runs only inside the isolated worker.
pub mod broker;
mod formats;
#[cfg(test)]
mod reader;
mod sevenzip;
pub mod storage;
mod streams;
mod tar;
mod timestamps;
mod tree;
mod window;
use midnight_filesystem_api::StagePurpose;
use midnight_filesystem_api::{ErrorCode, Format, Kind, MAX_BATCH, MAX_FRAME, MAX_READ, Node};
use std::rc::Rc;
use std::{
    collections::HashMap,
    io::{self, Read},
};
use storage::{Content, Reader, Stage, Storage};

/// Process memory the host grants this worker: its memory policy for archive workers,
/// the largest the public worker SDK accepts. The worker cannot query its own limit
/// inside the sandbox, so the fixed memory budgets of the formats derive from this.
pub(crate) const WORKER_MEMORY: u64 = 1024 * 1024 * 1024;

/// Declared-size budgets. Production deliberately keeps them unlimited: content
/// length is bounded by the host's spool volume, not by an artificial ceiling. The
/// fields let embedders and tests bound output. Neither the number of members nor
/// the bytes of their names are budgeted: the index of an archive with millions of
/// members is bounded only by the worker's process memory, which the host sizes.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_entry: u64,
    pub max_total: u64,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            max_entry: u64::MAX,
            max_total: u64::MAX,
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
/// Decoders report what they read as I/O errors; one that carries no `Failure` is
/// about the archive's bytes (a truncated or malformed structure).
impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        match Failure::of(&error) {
            Some(failure) => failure.clone().into(),
            None => Self::new(ErrorCode::Corrupt, error.to_string()),
        }
    }
}

/// A failure that says nothing about the archive's bytes: the host's failure reply to
/// a worker request, or a failed read of the captured source (a dropped share, a
/// removed drive). It travels inside `io::Error`s through the decoders and is reported
/// with its own code, never as a corrupt archive.
#[derive(Clone, Debug)]
pub(crate) struct Failure {
    pub(crate) code: ErrorCode,
    pub(crate) message: String,
}
impl Failure {
    /// The failure that `error` carries, if any.
    pub(crate) fn of(error: &io::Error) -> Option<&Self> {
        error.get_ref()?.downcast_ref()
    }
}
impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.message.fmt(f)
    }
}
impl std::error::Error for Failure {}
impl From<Failure> for Error {
    fn from(failure: Failure) -> Self {
        Self::new(failure.code, failure.message)
    }
}

/// A corrupt-source failure of opening `format`, led by a plain sentence.
fn not_valid(format: Format, error: Error) -> Error {
    const LEAD: &str = "This file is not a valid ";
    if error.code != ErrorCode::Corrupt || error.message.starts_with(LEAD) {
        return error;
    }
    let kind = match format {
        Format::Zip => "ZIP archive",
        Format::Tar => "TAR archive",
        Format::SevenZip => "7z archive",
        Format::Gzip => "gzip file",
        Format::TarGzip => "gzip-compressed TAR archive",
        Format::Xz => "XZ file",
        Format::TarXz => "XZ-compressed TAR archive",
        Format::Bzip2 => "bzip2 file",
        Format::TarBzip2 => "bzip2-compressed TAR archive",
        Format::Zstd => "Zstandard file",
        Format::TarZstd => "Zstandard-compressed TAR archive",
    };
    Error::new(
        ErrorCode::Corrupt,
        format!("{LEAD}{kind}.\n{}", error.message),
    )
}

/// One bounded, immutable mount. ZIP members and 7z blocks are verified on first read.
/// Decoded bytes are staged through host capabilities; no native path is opened here.
pub struct Archive {
    members: Vec<tree::Member>,
    /// Raw member names, each followed by its label when that differs.
    names: Vec<u8>,
    contents: HashMap<u64, Content>,
    storage: Rc<dyn Storage>,
    /// Folders by parent and raw name, while members are indexed.
    directories: HashMap<(u64, Vec<u8>), u64>,
    /// Member positions of each folder, in increasing ID order.
    children: HashMap<u64, Vec<usize>>,
    zip: Option<zip::ZipArchive<formats::Shared>>,
    /// The ZIP entry index of each readable member, in increasing ID order.
    zip_members: Vec<(u64, usize)>,
    tar_content: Option<tar::Content>,
    sevenzip_content: Option<sevenzip::Content>,
    limits: Limits,
    total: u64,
    /// The first failed read of the source during the current open or member read.
    failure: window::Failures,
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

    fn empty(storage: Rc<dyn Storage>, limits: Limits, failure: window::Failures) -> Self {
        Self {
            failure,
            members: Vec::new(),
            names: Vec::new(),
            contents: HashMap::new(),
            storage,
            directories: HashMap::new(),
            children: HashMap::new(),
            zip: None,
            zip_members: Vec::new(),
            tar_content: None,
            sevenzip_content: None,
            limits,
            total: 0,
        }
    }

    /// Production entry point: source and decoded storage are host-brokered streams.
    /// A source its decoder rejects as corrupt says first that it is no valid archive
    /// of `format` (an archive member named `.zip` may be any file); the decoder's own
    /// reason follows on the next line. A failed read of the source is reported as
    /// itself, whatever a decoder made of it.
    pub fn open_stream(
        format: Format,
        source: Box<dyn storage::ReadSeek>,
        source_length: u64,
        storage: Rc<dyn Storage>,
        limits: Limits,
    ) -> Result<Self, Error> {
        let source = Reader::new(source, source_length);
        let mut result = Self::empty(storage, limits, source.failures());
        if let Err(error) = result.open_format(format, source, source_length, limits) {
            return Err(match result.failure.take() {
                Some(failure) => failure.into(),
                None => not_valid(format, error),
            });
        }
        result.finish_index();
        Ok(result)
    }

    fn open_format(
        &mut self,
        format: Format,
        source: Reader,
        source_length: u64,
        limits: Limits,
    ) -> Result<(), Error> {
        let result = self;
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
                // No artificial decoded-size ceiling: the free space of the host's
                // spool volume bounds the scratch stream.
                let mut stage = Stage::new(
                    result.storage.clone(),
                    StagePurpose::Scratch,
                    None,
                    limits.max_total,
                )?;
                if matches!(
                    format,
                    Format::TarGzip | Format::TarXz | Format::TarBzip2 | Format::TarZstd
                ) {
                    result.tar_stream(streams::reader(format, source)?, stage)?;
                } else {
                    streams::decode(format, source, &mut stage)?;
                    let content = stage.finish()?;
                    result.add(b"content", "content", Kind::File, Some(content), true)?;
                }
            }
        }
        Ok(())
    }

    pub fn list(&self, directory: u64, after: u64) -> Result<Vec<Node>, Error> {
        Ok(self
            .children_after(directory, after)?
            .iter()
            .take(MAX_BATCH)
            .map(|position| self.node(*position))
            .collect())
    }

    /// One listing reply: at most `MAX_BATCH` children after `after` that fit the
    /// frame budget, and whether more children follow. Nodes are built once.
    pub fn page(&self, directory: u64, after: u64) -> Result<(Vec<Node>, bool), Error> {
        let children = self.children_after(directory, after)?;
        let mut bytes = 32usize;
        let mut page = Vec::new();
        for &position in children.iter().take(MAX_BATCH) {
            bytes = bytes.saturating_add(self.name_bytes(position) + 64);
            if bytes > MAX_FRAME {
                break;
            }
            page.push(self.node(position));
        }
        // A node's two bounded names always fit one frame, so a page is never empty
        // while children remain.
        let more = !page.is_empty() && page.len() < children.len();
        Ok((page, more))
    }

    /// Children of `directory` with IDs above `after`, in increasing ID order.
    fn children_after(&self, directory: u64, after: u64) -> Result<&[usize], Error> {
        if directory != 1
            && self
                .member(directory)
                .is_none_or(|member| member.kind != Kind::Directory)
        {
            return Err(Error::new(ErrorCode::NotFound, "Unknown archive directory"));
        }
        // IDs are increasing and remain stable after sorting, filtering and pagination.
        let Some(children) = self.children.get(&directory) else {
            return Ok(&[]);
        };
        let start = children.partition_point(|position| *position as u64 + 2 <= after);
        Ok(&children[start..])
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

    /// Complete format integrity checks before publishing a sealed content range. A
    /// failed read of the source is reported as itself, whatever a decoder made of it.
    pub fn prepare(&mut self, entry: u64) -> Result<Content, Error> {
        self.failure.take();
        let result = self.prepare_member(entry);
        match (result, self.failure.take()) {
            (Err(_), Some(failure)) => Err(failure.into()),
            (result, _) => result,
        }
    }

    fn prepare_member(&mut self, entry: u64) -> Result<Content, Error> {
        let member = self
            .member(entry)
            .ok_or_else(|| Error::new(ErrorCode::NotFound, "Unknown archive member"))?;
        if !member.readable {
            return Err(Error::new(
                ErrorCode::Unsupported,
                "Unsafe, encrypted, linked or unsupported archive member",
            ));
        }
        let declared_size = member.size();
        if let Some(content) = self.contents.get(&entry) {
            return Ok(*content);
        }
        if let Some(tar) = &mut self.tar_content {
            return tar.prepare(entry, self.storage.clone());
        }
        if let Some(sevenzip) = &mut self.sevenzip_content {
            return sevenzip.decode(entry, self.storage.clone());
        }
        if let Some(index) = tree::find(&self.zip_members, entry).copied() {
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
}
