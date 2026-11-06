//! LiveValidated revisions detect changes; they do not promise an immutable snapshot.
use crate::{Error, api::viewer, native};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

static REVISIONS: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Debug)]
pub struct CapturedFile {
    path: PathBuf,
    snapshot: native::Snapshot,
    revision: u64,
}
pub(crate) struct Reader {
    file: File,
    snapshot: native::Snapshot,
}

impl CapturedFile {
    /// Backend only. Opens briefly to obtain stable native identity, then closes.
    pub fn capture(path: &Path) -> Result<Self, Error> {
        if !path.is_absolute() {
            return Err(Error("Captured path must be absolute".into()));
        }
        let file = native::open_read(path)?;
        let snapshot = native::snapshot(&file)?;
        let revision = REVISIONS
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |id| id.checked_add(1))
            .map_err(|_| Error("Revision IDs exhausted".into()))?;
        Ok(Self {
            path: path.into(),
            snapshot,
            revision,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn length(&self) -> u64 {
        self.snapshot.length
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn display_name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or(self.path.as_os_str())
            .to_string_lossy()
            .into_owned()
    }
    /// Backend only; reads at most 4096 bytes through the same revision validation.
    pub fn preferred_basic_mode(&self) -> Result<u16, Error> {
        let bytes = self.open()?.read_at(0, 4096)?;
        if bytes.starts_with(&[0xff, 0xfe]) || bytes.starts_with(&[0xfe, 0xff]) {
            return Ok(viewer::MODE_TEXT);
        }
        let controls = bytes
            .iter()
            .filter(|&&b| b < 32 && !matches!(b, b'\n' | b'\r' | b'\t' | 12))
            .count();
        Ok(if bytes.contains(&0) || controls > bytes.len() / 20 {
            viewer::MODE_BINARY
        } else {
            viewer::MODE_TEXT
        })
    }
    pub(crate) fn open(&self) -> Result<Reader, Error> {
        let file = native::open_read(&self.path)?;
        if native::snapshot(&file)? != self.snapshot {
            return Err(changed());
        }
        Ok(Reader {
            file,
            snapshot: self.snapshot.clone(),
        })
    }
}
impl Reader {
    pub(crate) fn read_at(&mut self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        if length == 0
            || length as usize > viewer::MAX_PAGE_BYTES
            || offset.checked_add(u64::from(length)).is_none()
        {
            return Err(Error("Invalid read bounds".into()));
        }
        if native::snapshot(&self.file)? != self.snapshot {
            return Err(changed());
        }
        let length = self
            .snapshot
            .length
            .saturating_sub(offset)
            .min(u64::from(length)) as usize;
        let mut bytes = vec![0; length];
        self.file.seek(SeekFrom::Start(offset))?;
        self.file.read_exact(&mut bytes)?;
        if native::snapshot(&self.file)? != self.snapshot {
            return Err(changed());
        }
        Ok(bytes)
    }
}
fn changed() -> Error {
    Error("Source changed since capture (LiveValidated revision)".into())
}
