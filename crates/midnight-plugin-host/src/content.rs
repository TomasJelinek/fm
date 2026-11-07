//! LiveValidated revisions detect changes; they do not promise an immutable snapshot.
use crate::{Error, api::viewer, native};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

static REVISIONS: AtomicU64 = AtomicU64::new(1);

/// Host-owned content provider. Plugins receive the ordinary bounded reader ABI,
/// never this trait, a native path, or the provider implementation.
pub trait ContentSource: Send + Sync + std::fmt::Debug {
    fn length(&self) -> u64;
    fn display_name(&self) -> String;
    fn version(&self) -> Vec<u8>;
    fn validate(&self) -> Result<(), Error>;
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error>;
}

#[derive(Clone, Debug)]
pub struct CapturedFile {
    path: PathBuf,
    snapshot: Option<native::Snapshot>,
    source: Option<Arc<dyn ContentSource>>,
    source_version: Vec<u8>,
    source_length: u64,
    revision: u64,
}
pub(crate) enum Reader {
    Native {
        file: File,
        snapshot: native::Snapshot,
    },
    Provider {
        source: Arc<dyn ContentSource>,
        version: Vec<u8>,
        length: u64,
    },
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
            snapshot: Some(snapshot),
            source: None,
            source_version: Vec::new(),
            source_length: 0,
            revision,
        })
    }
    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn length(&self) -> u64 {
        self.source.as_ref().map_or_else(
            || self.snapshot.as_ref().map_or(0, |s| s.length),
            |_| self.source_length,
        )
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    /// Stable native identity and revision bytes for host-owned cache keys.
    /// Versioned separately for each platform; not a portable filesystem address.
    pub fn stable_version(&self) -> Vec<u8> {
        self.source.as_ref().map_or_else(
            || {
                self.snapshot
                    .as_ref()
                    .map_or_else(Vec::new, |s| s.version_bytes())
            },
            |_| self.source_version.clone(),
        )
    }
    /// Validate the captured native identity against the current path.
    pub fn revalidate(&self) -> Result<(), Error> {
        self.open().map(|_| ())
    }
    /// Bounded broker read. Reopening validates both pathname replacement and
    /// before/after handle metadata against the original captured revision.
    pub fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.open()?.read_at(offset, length)
    }
    pub fn display_name(&self) -> String {
        if let Some(source) = &self.source {
            return source.display_name();
        }
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
        if let Some(source) = &self.source {
            validate_source(source.as_ref(), self.source_length, &self.source_version)?;
            return Ok(Reader::Provider {
                source: source.clone(),
                version: self.source_version.clone(),
                length: self.source_length,
            });
        }
        let snapshot = self
            .snapshot
            .as_ref()
            .ok_or_else(|| Error("Missing content identity".into()))?;
        let file = native::open_read(&self.path)?;
        if native::snapshot(&file)? != *snapshot {
            return Err(changed());
        }
        Ok(Reader::Native {
            file,
            snapshot: snapshot.clone(),
        })
    }
    /// Capture a non-native content lease without fabricating a host filesystem path.
    pub fn from_source(source: Arc<dyn ContentSource>) -> Result<Self, Error> {
        let source_version = source.version();
        let source_length = source.length();
        if source_version.is_empty() || source_version.len() > 4096 {
            return Err(Error("Invalid provider revision".into()));
        }
        validate_source(source.as_ref(), source_length, &source_version)?;
        let revision = REVISIONS
            .try_update(Ordering::Relaxed, Ordering::Relaxed, |v| v.checked_add(1))
            .map_err(|_| Error("Revision IDs exhausted".into()))?;
        Ok(Self {
            path: PathBuf::new(),
            snapshot: None,
            source: Some(source),
            source_version,
            source_length,
            revision,
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
        if let Self::Provider {
            source,
            version,
            length: source_length,
        } = self
        {
            validate_source(source.as_ref(), *source_length, version)?;
            let length = source_length.saturating_sub(offset).min(u64::from(length)) as u32;
            let bytes = if length == 0 {
                Vec::new()
            } else {
                source.read_at(offset, length)?
            };
            if bytes.len() != length as usize {
                return Err(Error(
                    "Provider returned an inconsistent read length".into(),
                ));
            }
            validate_source(source.as_ref(), *source_length, version)?;
            return Ok(bytes);
        }
        let Self::Native { file, snapshot } = self else {
            return Err(Error("Invalid content reader".into()));
        };
        if native::snapshot(file)? != *snapshot {
            return Err(changed());
        }
        let length = snapshot
            .length
            .saturating_sub(offset)
            .min(u64::from(length)) as usize;
        let mut bytes = vec![0; length];
        file.seek(SeekFrom::Start(offset))?;
        file.read_exact(&mut bytes)?;
        if native::snapshot(file)? != *snapshot {
            return Err(changed());
        }
        Ok(bytes)
    }
}
fn changed() -> Error {
    Error("Source changed since capture (LiveValidated revision)".into())
}
fn validate_source(source: &dyn ContentSource, length: u64, version: &[u8]) -> Result<(), Error> {
    source.validate()?;
    if source.length() != length || source.version() != version {
        return Err(changed());
    }
    Ok(())
}

#[cfg(test)]
mod provider_tests {
    use super::*;
    #[derive(Debug, Default)]
    struct Source {
        version: AtomicU64,
        oversized: std::sync::atomic::AtomicBool,
    }
    impl ContentSource for Source {
        fn length(&self) -> u64 {
            5
        }
        fn display_name(&self) -> String {
            "same.txt".into()
        }
        fn version(&self) -> Vec<u8> {
            self.version.load(Ordering::Acquire).to_le_bytes().to_vec()
        }
        fn validate(&self) -> Result<(), Error> {
            Ok(())
        }
        fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
            if self.oversized.load(Ordering::Acquire) {
                return Ok(vec![0; length as usize + 1]);
            }
            Ok(b"hello"[offset as usize..offset as usize + length as usize].to_vec())
        }
    }
    #[test]
    fn provider_reader_bounds_and_revision_are_captured_without_native_path() {
        let source = Arc::new(Source::default());
        let captured = CapturedFile::from_source(source.clone()).unwrap();
        assert!(captured.path().as_os_str().is_empty());
        assert_eq!(captured.display_name(), "same.txt");
        assert_eq!(captured.read_at(1, 3).unwrap(), b"ell");
        assert_eq!(captured.read_at(3, 99).unwrap(), b"lo");
        assert!(captured.read_at(99, 1).unwrap().is_empty());
        assert!(captured.read_at(0, 0).is_err());
        assert!(captured.read_at(0, 256 * 1024 + 1).is_err());
        assert!(captured.read_at(u64::MAX, 1).is_err());
        let revision = captured.stable_version();
        source.version.store(1, Ordering::Release);
        assert_eq!(captured.stable_version(), revision);
        assert!(captured.read_at(0, 1).is_err());
        assert!(captured.revalidate().is_err());
    }
    #[test]
    fn provider_cannot_return_more_than_the_broker_requested() {
        let source = Arc::new(Source::default());
        source.oversized.store(true, Ordering::Release);
        let captured = CapturedFile::from_source(source).unwrap();
        assert!(captured.read_at(0, 2).is_err());
    }
}
