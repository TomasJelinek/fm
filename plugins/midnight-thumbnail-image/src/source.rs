//! The brokered source with its first bytes already fetched.
//!
//! Choosing between the image and PDF decoders needs the first bytes. Every broker
//! read is a host round trip, so the prefix is fetched with the size of the image
//! decoder's own first read and then serves that read from memory: sniffing costs
//! no additional round trip.
use std::io::{self, Read, Seek, SeekFrom};

/// The image decoder's first window, which is also the largest brokered read.
const PREFIX_BYTES: usize = 256 * 1024;

pub(crate) struct Prefixed<'a, R: ?Sized> {
    inner: &'a mut R,
    prefix: Vec<u8>,
    length: u64,
    position: u64,
    /// The inner reader's position, when it is known to equal `position`'s last read end.
    inner_position: Option<u64>,
}

impl<'a, R: Read + Seek + ?Sized> Prefixed<'a, R> {
    /// Fetch the prefix. One read normally returns all of it; after a short reply,
    /// reading continues only until `needed` bytes (or the whole source) are present.
    pub(crate) fn new(inner: &'a mut R, needed: usize) -> io::Result<Self> {
        let length = inner.seek(SeekFrom::End(0))?;
        inner.seek(SeekFrom::Start(0))?;
        let wanted = usize::try_from(length)
            .unwrap_or(usize::MAX)
            .min(PREFIX_BYTES);
        let mut prefix = vec![0; wanted];
        let mut filled = 0;
        while filled < wanted.min(needed.max(1)) {
            let count = inner.read(&mut prefix[filled..])?;
            if count == 0 {
                break;
            }
            filled += count;
        }
        prefix.truncate(filled);
        Ok(Self {
            inner,
            prefix,
            length,
            position: 0,
            inner_position: Some(filled as u64),
        })
    }

    pub(crate) fn prefix(&self) -> &[u8] {
        &self.prefix
    }
}

impl<R: Read + Seek + ?Sized> Read for Prefixed<'_, R> {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        if output.is_empty() || self.position >= self.length {
            return Ok(0);
        }
        if let Some(offset) = usize::try_from(self.position)
            .ok()
            .filter(|offset| *offset < self.prefix.len())
        {
            let count = (self.prefix.len() - offset).min(output.len());
            output[..count].copy_from_slice(&self.prefix[offset..offset + count]);
            self.position += count as u64;
            return Ok(count);
        }
        // Until a seek and read succeed, the inner position is unknown.
        if self.inner_position.take() != Some(self.position) {
            self.inner.seek(SeekFrom::Start(self.position))?;
        }
        let count = self.inner.read(output)?;
        self.position = self
            .position
            .checked_add(count as u64)
            .filter(|position| *position <= self.length)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Read beyond the source"))?;
        self.inner_position = Some(self.position);
        Ok(count)
    }
}

/// Positions are checked like the broker's own: never beyond the captured length.
impl<R: ?Sized> Seek for Prefixed<'_, R> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.position = match from {
            SeekFrom::Start(position) => Some(position),
            SeekFrom::Current(delta) => self.position.checked_add_signed(delta),
            SeekFrom::End(delta) => self.length.checked_add_signed(delta),
        }
        .filter(|position| *position <= self.length)
        .ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Seek outside captured source")
        })?;
        Ok(self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    /// Counts reads and serves at most `chunk` bytes per call, like a short broker reply.
    struct Counted {
        inner: Cursor<Vec<u8>>,
        reads: usize,
        chunk: usize,
    }

    impl Read for Counted {
        fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
            self.reads += 1;
            let limit = output.len().min(self.chunk);
            self.inner.read(&mut output[..limit])
        }
    }

    impl Seek for Counted {
        fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
            self.inner.seek(from)
        }
    }

    fn source(length: usize, chunk: usize) -> Counted {
        Counted {
            inner: Cursor::new((0..length).map(|index| (index % 251) as u8).collect()),
            reads: 0,
            chunk,
        }
    }

    #[test]
    fn the_prefix_serves_reads_and_the_rest_continues_from_the_source() {
        let mut inner = source(PREFIX_BYTES + 1000, usize::MAX);
        let mut prefixed = Prefixed::new(&mut inner, 1024).unwrap();
        assert_eq!(prefixed.prefix().len(), PREFIX_BYTES);
        let mut all = Vec::new();
        prefixed.read_to_end(&mut all).unwrap();
        let expected: Vec<u8> = (0..PREFIX_BYTES + 1000)
            .map(|index| (index % 251) as u8)
            .collect();
        assert_eq!(all, expected);
        // Seeking back into the prefix and past it again reads the same bytes.
        prefixed.seek(SeekFrom::Start(10)).unwrap();
        let mut byte = [0];
        prefixed.read_exact(&mut byte).unwrap();
        assert_eq!(byte[0], 10);
        prefixed
            .seek(SeekFrom::End(-1))
            .expect("seek within the source");
        prefixed.read_exact(&mut byte).unwrap();
        assert_eq!(byte[0], expected[expected.len() - 1]);
        assert!(prefixed.seek(SeekFrom::End(1)).is_err());
        assert!(prefixed.seek(SeekFrom::Current(-100_000_000)).is_err());
        // One read for the prefix, then reads for the remainder only.
        assert!(inner.reads <= 4, "{} reads", inner.reads);
    }

    #[test]
    fn short_replies_continue_only_until_the_needed_bytes_are_present() {
        let mut inner = source(4096, 100);
        let prefixed = Prefixed::new(&mut inner, 1024).unwrap();
        assert_eq!(prefixed.prefix().len(), 1100);
        assert_eq!(inner.reads, 11);
        let mut empty = source(0, usize::MAX);
        let mut prefixed = Prefixed::new(&mut empty, 1024).unwrap();
        assert!(prefixed.prefix().is_empty());
        assert_eq!(prefixed.read(&mut [0; 8]).unwrap(), 0);
        assert_eq!(empty.reads, 0);
    }
}
