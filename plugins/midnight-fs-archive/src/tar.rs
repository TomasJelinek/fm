//! Metadata-only TAR index and checked u64 ranges into a captured source or sealed scratch.
use crate::*;
use std::{
    io::{Seek, SeekFrom},
    ops::Range,
};
/// Byte ranges of readable members, by member ID in increasing order.
type Ranges = Vec<(u64, Range<u64>)>;

pub(crate) struct Content {
    /// The captured source of an uncompressed TAR; compressed TARs read their scratch.
    source: Option<Reader>,
    ranges: Ranges,
    scratch: Option<storage::Content>,
}
impl Content {
    pub(crate) fn prepare(
        &mut self,
        entry: u64,
        storage: Rc<dyn Storage>,
    ) -> Result<storage::Content, Error> {
        let range = tree::find(&self.ranges, entry).ok_or_else(|| {
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
        let source = self
            .source
            .as_mut()
            .ok_or_else(|| Error::new(ErrorCode::Corrupt, "TAR content has no source"))?;
        source.seek(SeekFrom::Start(range.start))?;
        let mut stage = Stage::new(
            storage,
            StagePurpose::Member { entry },
            Some(length),
            length,
        )?;
        stage.copy(source.take(length))?;
        stage.finish()
    }
}
/// Bytes the tar crate may read between two seeks while indexing. Member data is
/// always skipped by seeking, so this bounds one header plus one GNU long-name/link
/// or PAX record, which the crate buffers completely in memory.
const MAX_METADATA_RUN: u64 = 16 * 1024 * 1024;

struct BoundedReader<R> {
    reader: R,
    length: u64,
    unseeked: u64,
}
impl<R: Read> Read for BoundedReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let count = self.reader.read(buffer)?;
        self.unseeked += count as u64;
        if self.unseeked > MAX_METADATA_RUN {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "TAR metadata record exceeds 16 MiB",
            ));
        }
        Ok(count)
    }
}
impl<R: Seek> Seek for BoundedReader<R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let position = self.reader.seek(from)?;
        if position > self.length {
            return Err(io::ErrorKind::UnexpectedEof.into());
        }
        self.unseeked = 0;
        Ok(position)
    }
}
/// A decoded stream that the TAR index reads while it is staged: each decoded block
/// is appended to the stage once, and member data is skipped without being read back.
struct StagedStream<'a, D> {
    decoded: D,
    stage: &'a mut Stage,
    block: Vec<u8>,
    /// Stream offset of `block[0]`, and how much of `block` was consumed.
    block_start: u64,
    consumed: usize,
    finished: bool,
    /// A staging failure keeps its own code when the TAR reader reports it as I/O.
    failure: Option<Error>,
}
impl<D: Read> StagedStream<'_, D> {
    /// Decode and stage the next block; false at the end of the stream.
    fn refill(&mut self) -> io::Result<bool> {
        if self.finished {
            return Ok(false);
        }
        self.block_start += self.block.len() as u64;
        self.consumed = 0;
        self.block.resize(MAX_READ as usize, 0);
        let mut filled = 0;
        while filled < self.block.len() {
            match self.decoded.read(&mut self.block[filled..]) {
                Ok(0) => break,
                Ok(count) => filled += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => {
                    self.block.clear();
                    return Err(error);
                }
            }
        }
        self.block.truncate(filled);
        if filled == 0 {
            self.finished = true;
            return Ok(false);
        }
        if let Err(error) = self.stage.append(&self.block) {
            let message = error.message.clone();
            self.failure = Some(error);
            self.block.clear();
            return Err(io::Error::other(message));
        }
        Ok(true)
    }
    fn position(&self) -> u64 {
        self.block_start + self.consumed as u64
    }
    /// Stage the rest of the stream, which also completes the codec's integrity checks.
    fn drain(&mut self) -> io::Result<()> {
        while self.refill()? {}
        Ok(())
    }
}
impl<D: Read> Read for StagedStream<'_, D> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if self.consumed == self.block.len() && !self.refill()? {
            return Ok(0);
        }
        let count = output.len().min(self.block.len() - self.consumed);
        output[..count].copy_from_slice(&self.block[self.consumed..self.consumed + count]);
        self.consumed += count;
        Ok(count)
    }
}
impl<D: Read> Seek for StagedStream<'_, D> {
    /// Forward only: the TAR reader skips member data, which is staged but not copied.
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        let target = match from {
            SeekFrom::Start(target) => Some(target),
            SeekFrom::Current(delta) => self.position().checked_add_signed(delta),
            SeekFrom::End(_) => None,
        }
        .filter(|target| *target >= self.position())
        .ok_or_else(|| io::Error::from(io::ErrorKind::Unsupported))?;
        while self.position() < target {
            if self.consumed == self.block.len() && !self.refill()? {
                return Err(io::ErrorKind::UnexpectedEof.into());
            }
            let step = (target - self.position()).min((self.block.len() - self.consumed) as u64);
            self.consumed += step as usize;
        }
        Ok(target)
    }
}
impl Archive {
    pub(crate) fn tar(
        &mut self,
        source: Reader,
        length: u64,
        scratch: Option<storage::Content>,
    ) -> Result<(), Error> {
        let (ranges, source) = self.index_tar(source, length)?;
        self.tar_content = Some(Content {
            source: Some(source),
            ranges,
            scratch,
        });
        Ok(())
    }

    /// Index a compressed TAR in the pass that stages its decoded stream. Reading the
    /// staged stream back for the index once cost as much host traffic as staging it.
    pub(crate) fn tar_stream(&mut self, decoded: impl Read, mut stage: Stage) -> Result<(), Error> {
        let mut stream = StagedStream {
            decoded,
            stage: &mut stage,
            block: Vec::new(),
            block_start: 0,
            consumed: 0,
            finished: false,
            failure: None,
        };
        let indexed = self
            .index_tar(&mut stream, u64::MAX)
            .map(|(ranges, _)| ranges)
            .and_then(|ranges| stream.drain().map(|()| ranges).map_err(Error::from));
        let failure = stream.failure.take();
        drop(stream);
        let ranges = match (indexed, failure) {
            (_, Some(failure)) => return Err(failure),
            (result, None) => result?,
        };
        let content = stage.finish()?;
        if ranges.iter().any(|(_, range)| range.end > content.length) {
            return Err(Error::new(ErrorCode::Corrupt, "Truncated TAR member"));
        }
        self.tar_content = Some(Content {
            source: None,
            ranges,
            scratch: Some(content),
        });
        Ok(())
    }

    fn index_tar<R: Read + Seek>(&mut self, reader: R, length: u64) -> Result<(Ranges, R), Error> {
        let mut archive = ::tar::Archive::new(BoundedReader {
            reader,
            length,
            unseeked: 0,
        });
        let mut ranges = Ranges::new();
        for entry in archive.entries_with_seek()? {
            let mut entry = entry?;
            let typ = entry.header().entry_type();
            // A PAX global header (`git archive` writes one) is archive metadata, not a
            // member; GNU tar and bsdtar do not list it either.
            if typ.is_pax_global_extensions() {
                continue;
            }
            let raw = entry.path_bytes().into_owned();
            let display = String::from_utf8_lossy(&raw).into_owned();
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
            if let Some(member) = self.member_mut(id) {
                member.modified = modified;
            }
            if readable {
                let start = entry.raw_file_position();
                let end = start
                    .checked_add(size)
                    .filter(|end| *end <= length)
                    .ok_or_else(|| Error::new(ErrorCode::Corrupt, "Truncated TAR member"))?;
                let id = self.indexed_size(size)?;
                ranges.push((id, start..end));
            }
        }
        ranges.shrink_to_fit();
        Ok((ranges, archive.into_inner().reader))
    }
}
