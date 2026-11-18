//! Eager ZIP indexing is confined to preflighted metadata, including error fallback.
use crate::{
    OfficeError,
    source::{CHUNK, CapturedReader},
};
use std::{
    io::{self, Cursor, Read, Seek, SeekFrom},
    sync::{Arc, Mutex},
};

type Snapshot = Arc<Mutex<Option<Cursor<Vec<u8>>>>>;

pub(crate) struct IndexReader {
    source: CapturedReader,
    snapshot: Snapshot,
}

impl IndexReader {
    pub fn new(source: CapturedReader, metadata: Vec<u8>) -> (Self, Snapshot) {
        let snapshot = Arc::new(Mutex::new(Some(Cursor::new(metadata))));
        (
            Self {
                source,
                snapshot: snapshot.clone(),
            },
            snapshot,
        )
    }

    /// Only called after the synchronous eager index returned successfully.
    /// Local header offsets are unchanged; only metadata offsets were normalized.
    pub fn finish(snapshot: &Snapshot) -> Result<(), OfficeError> {
        let bytes = snapshot.lock().map_err(|_| OfficeError::Cancelled)?.take();
        drop(bytes);
        Ok(())
    }
}

impl Read for IndexReader {
    fn read(&mut self, output: &mut [u8]) -> io::Result<usize> {
        let mut state = self
            .snapshot
            .lock()
            .map_err(|_| io::Error::other(OfficeError::Cancelled))?;
        if let Some(snapshot) = state.as_mut() {
            self.source.work.check().map_err(io::Error::other)?;
            let size = self
                .source
                .work
                .reserve(output.len().min(CHUNK))
                .map_err(io::Error::other)?;
            let count = snapshot.read(&mut output[..size])?;
            self.source.work.refund(size - count);
            return Ok(count);
        }
        drop(state);
        self.source.read(output)
    }
}

impl Seek for IndexReader {
    fn seek(&mut self, offset: SeekFrom) -> io::Result<u64> {
        let mut state = self
            .snapshot
            .lock()
            .map_err(|_| io::Error::other(OfficeError::Cancelled))?;
        if let Some(snapshot) = state.as_mut() {
            self.source.work.check().map_err(io::Error::other)?;
            return snapshot.seek(offset);
        }
        drop(state);
        self.source.seek(offset)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Cancellation, OfficeSource, source::Work};

    #[test]
    fn eager_index_error_fallback_cannot_search_original_package_bytes() {
        struct ForbiddenSource;
        impl OfficeSource for ForbiddenSource {
            fn len(&self) -> u64 {
                192 * 1024 * 1024
            }
            fn read_at(&self, _: u64, _: &mut [u8], _: &Cancellation) -> io::Result<usize> {
                panic!("eager index fallback escaped metadata snapshot");
            }
        }
        // Intentional semantic error after finding CDFH: the advertised name
        // cannot fit in this snapshot. ZIP then retries earlier EOCD discovery.
        let mut metadata = vec![0u8; 46 + 22];
        metadata[..4].copy_from_slice(b"PK\x01\x02");
        metadata[28..30].copy_from_slice(&100u16.to_le_bytes());
        metadata[46..50].copy_from_slice(b"PK\x05\x06");
        metadata[54..56].copy_from_slice(&1u16.to_le_bytes());
        metadata[56..58].copy_from_slice(&1u16.to_le_bytes());
        metadata[58..62].copy_from_slice(&46u32.to_le_bytes());
        let source = CapturedReader {
            source: Arc::new(ForbiddenSource),
            length: 192 * 1024 * 1024,
            position: 0,
            work: Work::new(&Cancellation::default()),
        };
        let (reader, _) = IndexReader::new(source, metadata);
        assert!(
            zip::ZipArchive::with_config(
                zip::read::Config {
                    archive_offset: zip::read::ArchiveOffset::Known(0),
                },
                reader
            )
            .is_err()
        );
    }
}
