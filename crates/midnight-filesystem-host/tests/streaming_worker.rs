//! Real restricted-worker qualification against the user's explicitly named read-only ZIP.
use midnight_core::{ArchiveMount, Cancellation};
use midnight_filesystem_host::{Catalog, Mount};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};

#[derive(Debug)]
struct Measured {
    source: CapturedFile,
    bytes: AtomicU64,
    largest: AtomicU64,
    changed: AtomicBool,
    cancel_after: AtomicU64,
    cancel: Cancellation,
}
impl ContentSource for Measured {
    fn length(&self) -> u64 {
        self.source.length()
    }
    fn display_name(&self) -> String {
        "measured archive source".into()
    }
    fn version(&self) -> Vec<u8> {
        self.source.stable_version()
    }
    fn validate(&self) -> Result<(), Error> {
        if self.changed.load(Ordering::Acquire) {
            return Err(Error("fixture source changed".into()));
        }
        self.source.revalidate()
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.largest.fetch_max(u64::from(length), Ordering::Relaxed);
        let bytes = self.source.read_at(offset, length)?;
        let total =
            self.bytes.fetch_add(bytes.len() as u64, Ordering::Relaxed) + bytes.len() as u64;
        let threshold = self.cancel_after.load(Ordering::Relaxed);
        if threshold != 0 && total >= threshold {
            self.cancel.cancel();
        }
        Ok(bytes)
    }
}
fn setup() -> (Catalog, PathBuf, Arc<Measured>, CapturedFile) {
    let installation = PathBuf::from(
        std::env::var_os("ARCHIVE_CAPTURE_INSTALLATION").expect("protected archive installation"),
    );
    let path = PathBuf::from(
        std::env::var_os("MIDNIGHT_ARCHIVE_TEST_ZIP").expect("explicit read-only ZIP fixture"),
    );
    let catalog = Catalog::discover(&installation).unwrap();
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    let source = Arc::new(Measured {
        source: CapturedFile::capture(&path).unwrap(),
        bytes: AtomicU64::new(0),
        largest: AtomicU64::new(0),
        changed: AtomicBool::new(false),
        cancel_after: AtomicU64::new(0),
        cancel: Cancellation::default(),
    });
    let captured = CapturedFile::from_source(source.clone()).unwrap();
    (catalog, path, source, captured)
}

#[test]
#[ignore = "requires protected current archive worker and MIDNIGHT_ARCHIVE_TEST_ZIP"]
fn large_zip_browses_by_index_and_member_reads_use_verified_private_disk_cache() {
    let (catalog, path, measured, captured) = setup();
    assert!(
        captured.length() > 64 * 1024 * 1024,
        "qualification must exceed old source ceiling"
    );
    let (package, format) = catalog.choose(&path).unwrap();
    assert_eq!(package.protocol, midnight_filesystem_api::Protocol::V2);
    let cancel = Cancellation::default();
    let mount = Mount::open_captured(package, format, &path, captured, &cancel).unwrap();
    let nodes = mount.list(1, &cancel).unwrap();
    let index_bytes = measured.bytes.load(Ordering::Relaxed);
    assert!(
        index_bytes < 2 * 1024 * 1024,
        "index consumed {index_bytes} bytes"
    );
    let file = nodes
        .iter()
        .find(|node| node.readable && node.display_name.ends_with(".exe"))
        .unwrap();
    assert!(
        file.modified.is_some(),
        "ZIP timestamp must reach core metadata"
    );
    let bytes = mount.read(file.id.entry, 0, 2, &cancel).unwrap();
    assert_eq!(bytes, b"MZ");
    let extracted_bytes = measured.bytes.load(Ordering::Relaxed);
    assert!(extracted_bytes > index_bytes);
    for offset in [0, 1, file.size.unwrap() - 64] {
        assert_eq!(
            mount
                .read(file.id.entry, offset, 64, &cancel)
                .unwrap()
                .len(),
            64
        );
    }
    assert_eq!(
        measured.bytes.load(Ordering::Relaxed),
        extracted_bytes,
        "cache hits must not reread compressed source"
    );
    assert!(measured.largest.load(Ordering::Relaxed) <= 256 * 1024);
    eprintln!(
        "source={} index_reads={index_bytes} first_member_source_reads={} member_size={}",
        measured.length(),
        extracted_bytes - index_bytes,
        file.size.unwrap()
    );
    measured.changed.store(true, Ordering::Release);
    assert!(
        mount.read(file.id.entry, 0, 1, &cancel).is_err(),
        "cached bytes must not bypass source identity"
    );
}

#[test]
#[ignore = "requires protected current archive worker and MIDNIGHT_ARCHIVE_TEST_ZIP"]
fn cancellation_during_member_extraction_returns_no_partial_content() {
    let (catalog, path, measured, captured) = setup();
    let (package, format) = catalog.choose(&path).unwrap();
    let cancel = measured.cancel.clone();
    let mount = Mount::open_captured(package, format, &path, captured, &cancel).unwrap();
    let file = mount
        .list(1, &cancel)
        .unwrap()
        .into_iter()
        .find(|node| node.readable)
        .unwrap();
    measured.cancel_after.store(
        measured.bytes.load(Ordering::Relaxed) + 1,
        Ordering::Release,
    );
    assert!(mount.read(file.id.entry, 0, 1, &cancel).is_err());
    assert!(cancel.is_cancelled());
    assert!(
        mount
            .read(file.id.entry, 0, 1, &Cancellation::default())
            .is_err()
    );
}

#[test]
#[ignore = "requires protected archive worker and generated MIDNIGHT_ARCHIVE_FIXTURES"]
fn all_supported_formats_use_the_same_verified_stage_protocol() {
    let installation = PathBuf::from(std::env::var_os("ARCHIVE_CAPTURE_INSTALLATION").unwrap());
    let root = PathBuf::from(std::env::var_os("MIDNIGHT_ARCHIVE_FIXTURES").unwrap());
    let catalog = Catalog::discover(&installation).unwrap();
    for name in [
        "owned.zip",
        "owned.tar",
        "owned.gz",
        "owned.tar.gz",
        "owned.7z",
        "owned.xz",
        "owned.tar.xz",
        "owned.bz2",
        "owned.tar.bz2",
        "owned.zst",
        "owned.tar.zst",
        "corrupt.zip",
        "corrupt.7z",
    ] {
        let path = root.join(name);
        let (package, format) = catalog.choose(&path).unwrap();
        let cancel = Cancellation::default();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mount = loop {
            match Mount::open(package, format, &path, &cancel) {
                Ok(mount) => break mount,
                Err(error)
                    if error.to_string().contains("Two archive mounts")
                        && std::time::Instant::now() < deadline =>
                {
                    std::thread::sleep(std::time::Duration::from_millis(10))
                }
                Err(error) => panic!("{name}: {error}"),
            }
        };
        let member = mount
            .list(1, &cancel)
            .unwrap()
            .into_iter()
            .find(|node| node.readable)
            .unwrap();
        let result = mount.read(member.id.entry, 0, 256, &cancel);
        if name.starts_with("corrupt") {
            assert!(result.is_err(), "{name} published corrupt bytes");
        } else {
            assert_eq!(result.unwrap(), b"shared verified content", "{name}");
            assert_eq!(
                mount.read(member.id.entry, 7, 8, &cancel).unwrap(),
                b"verified",
                "{name}"
            );
        }
    }
}
