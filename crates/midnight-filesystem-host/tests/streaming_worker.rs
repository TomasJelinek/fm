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
    setup_path(installation, path)
}
fn setup_path(
    installation: PathBuf,
    path: PathBuf,
) -> (Catalog, PathBuf, Arc<Measured>, CapturedFile) {
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
    assert!(package.protocol.supports_staging());
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
    // Protocol 3 cancels the member cooperatively; earlier protocols cannot interrupt
    // staging, so the host finishes that exchange and discards it. Either way the
    // shared mount stays usable.
    let next = mount.read(file.id.entry, 0, 1, &Cancellation::default());
    assert_eq!(next.unwrap().len(), 1);
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
        let source = CapturedFile::capture(&path).unwrap();
        let mount = Mount::open_captured(package, format, &path, source, &cancel)
            .unwrap_or_else(|error| panic!("{name}: {error}"));
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
        // Deterministic confirmed teardown frees the mount permit for the next format.
        mount
            .release_on_worker()
            .unwrap_or_else(|error| panic!("{name}: {error}"));
    }
}

#[test]
#[ignore = "requires protected protocol 4 worker and exported MIDNIGHT_ARCHIVE_FIXTURES"]
fn cooperative_installed_zip_and_solid_sevenzip_keep_mount_and_cached_member() {
    let installation = PathBuf::from(std::env::var_os("ARCHIVE_CAPTURE_INSTALLATION").unwrap());
    let fixtures = PathBuf::from(std::env::var_os("MIDNIGHT_ARCHIVE_FIXTURES").unwrap());
    for name in ["cancel.zip", "cancel.7z"] {
        let (catalog, path, measured, source) =
            setup_path(installation.clone(), fixtures.join(name));
        let (package, format) = catalog.choose(&path).unwrap();
        assert_eq!(package.protocol, midnight_filesystem_api::Protocol::V4);
        let mount =
            Mount::open_captured(package, format, &path, source, &Cancellation::default()).unwrap();
        let nodes = mount.list(1, &Cancellation::default()).unwrap();
        let cached = nodes
            .iter()
            .find(|node| node.display_name == "cached.bin")
            .unwrap();
        let target = nodes
            .iter()
            .find(|node| node.display_name == "large.bin")
            .unwrap();
        assert_eq!(
            mount
                .read(cached.id.entry, 0, 4, &Cancellation::default())
                .unwrap(),
            b"keep"
        );
        let before = measured.bytes.load(Ordering::Relaxed);
        measured
            .cancel_after
            .store(before + 2 * 256 * 1024, Ordering::Release);
        let result =
            mount.read_cooperative(target.id.entry, 0, 4, &|| measured.cancel.is_cancelled());
        assert_eq!(
            result.unwrap_err().kind,
            std::io::ErrorKind::Interrupted,
            "{name}"
        );
        assert!(measured.cancel.is_cancelled());
        let after = measured.bytes.load(Ordering::Relaxed);
        assert!(
            after - before < 2 * 1024 * 1024,
            "{name} drained the complete 2MiB payload after cancellation"
        );
        measured.cancel_after.store(0, Ordering::Release);
        assert_eq!(
            mount
                .read(cached.id.entry, 0, 4, &Cancellation::default())
                .unwrap(),
            b"keep"
        );
        assert_eq!(
            measured.bytes.load(Ordering::Relaxed),
            after,
            "cancellation evicted an unrelated cached member"
        );
        let listed = mount.list(1, &Cancellation::default()).unwrap();
        assert_eq!(
            listed.iter().map(|node| node.id).collect::<Vec<_>>(),
            nodes.iter().map(|node| node.id).collect::<Vec<_>>()
        );
        assert_eq!(
            mount
                .read_cooperative(target.id.entry, 0, 4, &|| false)
                .unwrap(),
            [42; 4]
        );
        assert_eq!(
            mount
                .read(
                    target.id.entry,
                    target.size.unwrap() - 4,
                    4,
                    &Cancellation::default()
                )
                .unwrap(),
            [42; 4]
        );
        assert!(mount.validate().is_ok());
    }
}

/// The password of the archive plugin's exported `password.zip` and `password.7z`.
const FIXTURE_PASSWORD: &str = "Kyrandia-heslo-\u{1F511}";
/// Their members' content.
const FIXTURE_PAYLOAD: &[u8] = b"shared verified content";

fn named(nodes: &[midnight_core::ArchiveNode], name: &str) -> midnight_core::ArchiveNode {
    nodes
        .iter()
        .find(|node| node.display_name == name)
        .unwrap_or_else(|| panic!("{name} listed: {nodes:?}"))
        .clone()
}

/// One password per mount through the installed worker: encrypted ZIP members list
/// with their sizes and encryption, reading one before the password fails without
/// ending the mount, a wrong password is refused, and the right one reads WinZip AES
/// and ZipCrypto members. A 7z whose names are encrypted lists only after it.
#[test]
#[ignore = "requires protected protocol 4 worker and exported MIDNIGHT_ARCHIVE_FIXTURES"]
fn installed_worker_asks_for_checks_and_uses_one_archive_password() {
    use midnight_core::{
        ArchiveEncryption, ArchivePassword, ArchiveUnlock, is_archive_password_required,
    };
    let installation = PathBuf::from(std::env::var_os("ARCHIVE_CAPTURE_INSTALLATION").unwrap());
    let fixtures = PathBuf::from(std::env::var_os("MIDNIGHT_ARCHIVE_FIXTURES").unwrap());
    let cancel = Cancellation::default();
    let whole = FIXTURE_PAYLOAD.len() as u32;
    let wrong = ArchivePassword::new("Kyrandia".into());
    let right = ArchivePassword::new(FIXTURE_PASSWORD.into());
    let catalog = Catalog::discover(&installation).unwrap();
    assert!(catalog.diagnostics.is_empty(), "{:?}", catalog.diagnostics);
    // Directly captured sources, whose teardown `release_on_worker` confirms.
    let open = |name: &str| {
        let path = fixtures.join(name);
        let (package, format) = catalog.choose(&path).unwrap();
        assert_eq!(package.protocol, midnight_filesystem_api::Protocol::V4);
        let source = CapturedFile::capture(&path).unwrap();
        Mount::open_captured(package, format, &path, source, &cancel).unwrap()
    };

    let mount = open("password.zip");
    let root = mount.list(1, &cancel).unwrap();
    let folder = named(&root, "Kyrandia");
    assert!(folder.access.contains_encrypted);
    let legacy = named(&root, "legacy.txt");
    assert_eq!(legacy.access.encryption, ArchiveEncryption::ZipCrypto);
    let plain = named(&root, "plain.txt");
    assert_eq!(plain.access.encryption, ArchiveEncryption::None);
    let balcony = named(
        &mount.list(folder.id.entry, &cancel).unwrap(),
        "balcony.emc",
    );
    assert_eq!(balcony.access.encryption, ArchiveEncryption::Aes256);
    assert!(balcony.readable && balcony.access.unreadable.is_none());
    assert_eq!(balcony.size, Some(FIXTURE_PAYLOAD.len() as u64));
    let error = mount.read(balcony.id.entry, 0, whole, &cancel).unwrap_err();
    assert!(is_archive_password_required(&error), "{error:?}");
    // The worker keeps the mount: its plain member still reads.
    assert_eq!(
        mount.read(plain.id.entry, 0, whole, &cancel).unwrap(),
        FIXTURE_PAYLOAD
    );
    assert_eq!(
        mount.unlock(&wrong, &Cancellation::default()).unwrap(),
        ArchiveUnlock::WrongPassword
    );
    assert!(!mount.unlocked());
    assert_eq!(
        mount.unlock(&right, &Cancellation::default()).unwrap(),
        ArchiveUnlock::Unlocked
    );
    assert!(mount.unlocked());
    for member in [&balcony, &legacy] {
        assert_eq!(
            mount.read(member.id.entry, 0, whole, &cancel).unwrap(),
            FIXTURE_PAYLOAD,
            "{}",
            member.display_name
        );
    }
    mount.release_on_worker().unwrap();

    let mount = open("password.7z");
    let error = mount.list(1, &cancel).unwrap_err();
    assert!(is_archive_password_required(&error), "{error:?}");
    assert!(mount.listing_locked());
    assert_eq!(
        mount.unlock(&wrong, &Cancellation::default()).unwrap(),
        ArchiveUnlock::WrongPassword
    );
    assert!(mount.listing_locked());
    assert_eq!(
        mount.unlock(&right, &Cancellation::default()).unwrap(),
        ArchiveUnlock::Unlocked
    );
    assert!(!mount.listing_locked());
    let secret = named(&mount.list(1, &cancel).unwrap(), "secret.txt");
    assert_eq!(secret.access.encryption, ArchiveEncryption::SevenZipAes256);
    assert_eq!(
        mount.read(secret.id.entry, 0, whole, &cancel).unwrap(),
        FIXTURE_PAYLOAD
    );
    mount.release_on_worker().unwrap();
}
