//! Metadata-only TAR index and checked u64 ranges into a captured source or sealed scratch.
use crate::*;
use std::{
    io::{Seek, SeekFrom},
    ops::Range,
};
pub(crate) struct Content {
    source: Reader,
    ranges: HashMap<u64, Range<u64>>,
    scratch: Option<storage::Content>,
}
impl Content {
    pub(crate) fn prepare(
        &mut self,
        entry: u64,
        storage: Rc<dyn Storage>,
    ) -> Result<storage::Content, Error> {
        let range = self.ranges.get(&entry).ok_or_else(|| {
            Error::new(
                ErrorCode::Unsupported,
                "Archive member has no regular byte content",
            )
        })?;
        let length = range.end - range.start;
        if let Some(scratch) = self.scratch {
            return Ok(storage::Content {
                stage: scratch.stage,
                offset: scratch.offset + range.start,
                length,
            });
        }
        self.source.seek(SeekFrom::Start(range.start))?;
        let mut stage = Stage::new(
            storage,
            StagePurpose::Member { entry },
            Some(length),
            length,
        )?;
        stage.copy((&mut self.source).take(length))?;
        stage.finish()
    }
}
struct BoundedReader {
    reader: Reader,
    length: u64,
}
impl Read for BoundedReader {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.reader.read(buffer)
    }
}
impl Seek for BoundedReader {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = self.reader.seek(from)?;
        if position > self.length {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(position)
    }
}
impl Archive {
    pub(crate) fn tar(
        &mut self,
        source: Reader,
        length: u64,
        scratch: Option<storage::Content>,
    ) -> Result<(), Error> {
        let mut archive = ::tar::Archive::new(BoundedReader {
            reader: source,
            length,
        });
        let mut ranges = HashMap::new();
        for entry in archive.entries_with_seek()? {
            let mut entry = entry?;
            let raw = entry.path_bytes().into_owned();
            let display = String::from_utf8_lossy(&raw).into_owned();
            let typ = entry.header().entry_type();
            let kind = if typ.is_dir() {
                Kind::Directory
            } else if typ.is_file() {
                Kind::File
            } else if typ.is_symlink() || typ.is_hard_link() {
                Kind::Link
            } else {
                Kind::Other
            };
            let readable = kind == Kind::File && tree::safe_name(&raw);
            let size = entry.size();
            if size > self.limits.max_entry {
                return Err(Error::limit());
            }
            let modified = timestamps::tar_modified(&mut entry);
            let id = self.add(&raw, &display, kind, None, readable)?;
            if let Some(node) = self.nodes.get_mut((id - 2) as usize) {
                node.modified = modified;
            }
            if readable {
                let start = entry.raw_file_position();
                let end = start
                    .checked_add(size)
                    .filter(|end| *end <= length)
                    .ok_or_else(|| Error::new(ErrorCode::Corrupt, "Truncated TAR member"))?;
                let id = self.indexed_size(size)?;
                ranges.insert(id, start..end);
            }
        }
        self.tar_content = Some(Content {
            source: archive.into_inner().reader,
            ranges,
            scratch,
        });
        Ok(())
    }
}
