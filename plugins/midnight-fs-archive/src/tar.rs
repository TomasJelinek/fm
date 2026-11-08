//! Seekable TAR metadata and checked ranges into one immutable backing buffer.
use crate::*;
use std::io::{Cursor, Seek, SeekFrom};
use std::ops::Range;

pub(crate) struct Content {
    source: Vec<u8>,
    ranges: HashMap<u64, Range<usize>>,
}

impl Content {
    pub(crate) fn read(
        &self,
        entry: u64,
        offset: u64,
        length: u32,
    ) -> Result<(Vec<u8>, bool), Error> {
        let range = self.ranges.get(&entry).ok_or_else(|| {
            Error::new(
                ErrorCode::Unsupported,
                "Archive member has no regular byte content",
            )
        })?;
        read_slice(&self.source[range.clone()], offset, length)
    }
}

// Cursor normally permits seeking past EOF. TAR's seek iterator treats that as a
// clean end, so reject out-of-source skips, including padding of the final member.
struct BoundedCursor<'a>(Cursor<&'a [u8]>);
impl Read for BoundedCursor<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.0.read(buffer)
    }
}
impl Seek for BoundedCursor<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let position = self.0.seek(position)?;
        if position > self.0.get_ref().len() as u64 {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        Ok(position)
    }
}

impl Archive {
    pub(crate) fn tar(&mut self, source: Vec<u8>) -> Result<(), Error> {
        let mut archive = ::tar::Archive::new(BoundedCursor(Cursor::new(&source)));
        let mut ranges = HashMap::new();
        for entry in archive.entries_with_seek()? {
            let entry = entry?;
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
            self.add(&raw, &display, kind, None, readable)?;
            if readable {
                // Only regular, contiguous files use ranges. Sparse/special/link entries
                // remain unreadable; GNU/PAX extension processing stays in tar-rs.
                let start = entry.raw_file_position();
                let end = start
                    .checked_add(size)
                    .filter(|end| *end <= source.len() as u64)
                    .ok_or_else(|| Error::new(ErrorCode::Corrupt, "Truncated TAR member"))?;
                let id = self.indexed_size(size)?;
                ranges.insert(
                    id,
                    usize::try_from(start).map_err(|_| Error::limit())?
                        ..usize::try_from(end).map_err(|_| Error::limit())?,
                );
            }
        }
        self.tar_content = Some(Content { source, ranges });
        Ok(())
    }
}
