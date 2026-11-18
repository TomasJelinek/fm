//! Authored Office fixture support only. Native CEF and Office reader are never substituted.
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use midnight_web_viewer_host::api::Frame;
use std::sync::{
    Arc,
    atomic::{AtomicU64, AtomicUsize, Ordering},
};
use std::{
    io::{self, Write},
    path::Path,
};

pub const WORKBOOK_V1: &[u8] = include_bytes!("two-sheet-windows.xlsx");
pub const WORKBOOK_V2: &[u8] = include_bytes!("two-sheet-windows-v2.xlsx");

#[derive(Debug, Default)]
pub struct SourceState {
    pub revision: AtomicU64,
    pub reads: AtomicUsize,
    pub validations: AtomicUsize,
    pub releases: AtomicUsize,
    pub drops: AtomicUsize,
}

impl SourceState {
    /// Invalidate captured revision bytes; a rollback must not revive the adapter's revoked parent.
    pub fn revoke(&self) {
        self.revision.fetch_add(1, Ordering::AcqRel);
    }
}

#[derive(Debug)]
struct FixtureSource {
    bytes: &'static [u8],
    state: Arc<SourceState>,
}

impl ContentSource for FixtureSource {
    fn length(&self) -> u64 {
        self.bytes.len() as u64
    }

    fn display_name(&self) -> String {
        "native-two-sheet-windows.xlsx".into()
    }

    fn version(&self) -> Vec<u8> {
        self.state
            .revision
            .load(Ordering::Acquire)
            .to_le_bytes()
            .to_vec()
    }

    fn validate(&self) -> Result<(), Error> {
        self.state.validations.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.state.reads.fetch_add(1, Ordering::AcqRel);
        let first = usize::try_from(offset).map_err(|_| Error("fixture offset overflow".into()))?;
        let available = self
            .bytes
            .get(first..)
            .ok_or_else(|| Error("fixture offset".into()))?;
        Ok(available[..available.len().min(length as usize)].to_vec())
    }

    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        match Arc::try_unwrap(self) {
            Ok(source) => {
                source.state.releases.fetch_add(1, Ordering::AcqRel);
                Ok(())
            }
            Err(shared) => {
                drop(shared);
                Ok(())
            }
        }
    }
}

impl Drop for FixtureSource {
    fn drop(&mut self) {
        self.state.drops.fetch_add(1, Ordering::AcqRel);
    }
}

/// The caller retains observations, not a source owner. Checked final release can reach count one.
pub fn captured(bytes: &'static [u8]) -> (CapturedFile, Arc<SourceState>) {
    let state = Arc::new(SourceState::default());
    let source = Arc::new(FixtureSource {
        bytes,
        state: state.clone(),
    });
    let file = CapturedFile::from_source(source).expect("authored fixture source metadata");
    (file, state)
}

/// Save the actual native output as a top-down 32-bit BMP, following the first-party host probe.
pub fn save_bmp(path: &Path, frame: &Frame) -> io::Result<()> {
    let invalid = || io::Error::new(io::ErrorKind::InvalidInput, "invalid fixture proof frame");
    let bytes = frame
        .width
        .checked_mul(frame.height)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(invalid)?;
    if frame.width == 0 || frame.height == 0 || frame.rgba.len() != bytes as usize {
        return Err(invalid());
    }
    let file_bytes = bytes.checked_add(54).ok_or_else(invalid)?;
    let width = i32::try_from(frame.width).map_err(|_| invalid())?;
    let height = i32::try_from(frame.height).map_err(|_| invalid())?;
    let mut file = io::BufWriter::new(std::fs::File::create(path)?);
    file.write_all(b"BM")?;
    file.write_all(&file_bytes.to_le_bytes())?;
    file.write_all(&[0; 4])?;
    file.write_all(&54u32.to_le_bytes())?;
    file.write_all(&40u32.to_le_bytes())?;
    file.write_all(&width.to_le_bytes())?;
    file.write_all(&(-height).to_le_bytes())?;
    file.write_all(&1u16.to_le_bytes())?;
    file.write_all(&32u16.to_le_bytes())?;
    file.write_all(&[0; 24])?;
    for pixel in frame.rgba.as_chunks::<4>().0 {
        file.write_all(&[pixel[2], pixel[1], pixel[0], pixel[3]])?;
    }
    file.flush()
}
