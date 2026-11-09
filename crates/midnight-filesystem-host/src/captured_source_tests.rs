use crate::{Catalog, Mount, Package};
use midnight_core::{ArchiveMount, Cancellation};
use midnight_filesystem_api::Format;
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    io::ErrorKind,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};

#[derive(Debug)]
struct Source {
    bytes: &'static [u8],
    length: u64,
    changed: AtomicBool,
    reads: AtomicUsize,
}
impl ContentSource for Source {
    fn length(&self) -> u64 {
        self.length
    }
    fn display_name(&self) -> String {
        "display only.gz".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1, 2, 3]
    }
    fn validate(&self) -> Result<(), Error> {
        if self.changed.load(Ordering::Acquire) {
            Err(Error("source changed".into()))
        } else {
            Ok(())
        }
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.validate()?;
        self.reads.fetch_add(1, Ordering::Relaxed);
        let start = usize::try_from(offset).map_err(|_| Error("offset overflow".into()))?;
        let end = start.saturating_add(length as usize).min(self.bytes.len());
        self.bytes
            .get(start..end)
            .map(|bytes| bytes.to_vec())
            .ok_or_else(|| Error("invalid offset".into()))
    }
}
fn source(length: Option<u64>) -> (Arc<Source>, CapturedFile) {
    let bytes = include_bytes!("../tests/fixtures/hello.gz");
    let source = Arc::new(Source {
        bytes,
        length: length.unwrap_or(bytes.len() as u64),
        changed: AtomicBool::new(false),
        reads: AtomicUsize::new(0),
    });
    let captured = CapturedFile::from_source(source.clone()).unwrap();
    assert!(captured.path().as_os_str().is_empty());
    (source, captured)
}
fn package(root: &Path) -> Package {
    Package {
        id: "midnight.fs.archive".into(),
        executable: root.join("must-not-spawn.exe"),
        formats: vec![Format::Gzip],
    }
}

#[test]
fn captured_open_checks_limits_locator_changes_and_cancel_before_starting_worker() {
    let root = tempfile::tempdir().unwrap();
    let locator = root.path().join("not on disk 文档.gz");
    let package = package(root.path());
    let cancel = Cancellation::default();
    let (source, captured) = source(Some(64 * 1024 * 1024 + 1));
    let error =
        Mount::open_captured(&package, Format::Gzip, &locator, captured, &cancel).unwrap_err();
    assert!(error.to_string().contains("64 MiB"), "{error}");
    assert_eq!(source.reads.load(Ordering::Relaxed), 0);
    let (_, captured) = self::source(None);
    assert_eq!(
        Mount::open_captured(
            &package,
            Format::Gzip,
            Path::new("relative.gz"),
            captured,
            &cancel
        )
        .unwrap_err()
        .kind,
        ErrorKind::InvalidInput
    );
    let (source, captured) = self::source(None);
    source.changed.store(true, Ordering::Release);
    let error =
        Mount::open_captured(&package, Format::Gzip, &locator, captured, &cancel).unwrap_err();
    assert!(error.to_string().contains("source changed"), "{error}");
    let (_, captured) = self::source(None);
    cancel.cancel();
    assert_eq!(
        Mount::open_captured(&package, Format::Gzip, &locator, captured, &cancel)
            .unwrap_err()
            .kind,
        ErrorKind::Interrupted
    );
}

#[test]
#[ignore = "requires protected ARCHIVE_CAPTURE_INSTALLATION containing archive worker"]
fn real_worker_uses_pathless_content_and_retains_exact_native_locator() {
    let install = std::env::var_os("ARCHIVE_CAPTURE_INSTALLATION").unwrap();
    let catalog = Catalog::discover(Path::new(&install)).unwrap();
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    let root = tempfile::tempdir().unwrap();
    let locator = root.path().join("not on disk 文档.gz");
    let (package, format) = catalog.choose(&locator).unwrap();
    let (source, captured) = source(None);
    let cancel = Cancellation::default();
    let mount = Mount::open_captured(package, format, &locator, captured, &cancel).unwrap();
    assert_eq!(mount.address().container, locator);
    assert_eq!(mount.address().source_version, vec![1, 2, 3]);
    let entries = mount.list(1, &cancel).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(
        mount.read(entries[0].id.entry, 1, 3, &cancel).unwrap(),
        b"ell"
    );
    assert!(source.reads.load(Ordering::Relaxed) > 0);
    assert!(
        !locator.exists(),
        "Content capture fabricated a native file"
    );
    source.changed.store(true, Ordering::Release);
    assert!(mount.list(1, &cancel).is_err());
    assert!(mount.read(entries[0].id.entry, 0, 1, &cancel).is_err());
}
