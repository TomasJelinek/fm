//! Content-addressed command artwork; native adapters own directory and file authority.
use image::ImageEncoder;
use midnight_core::Cancellation;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    time::SystemTime,
};

#[path = "icon_snapshot_collection.rs"]
mod collection;
pub use collection::{COLLECTION_GRACE, collect, referenced_names};

pub const ICON_EDGE: u32 = 128;
const MAX_PNG_BYTES: usize = 128 * 128 * 4 + 4096;
const MAX_SNAPSHOTS: usize = 1024;

/// Artwork pixels have a separate bound from ordinary 64px desktop icons.
#[derive(Debug)]
pub struct SnapshotPixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}
impl SnapshotPixels {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> io::Result<Self> {
        validate_pixels(width, height, rgba.len())?;
        Ok(Self {
            width,
            height,
            rgba,
        })
    }
    pub fn from_rgba(width: u32, height: u32, rgba: &[u8]) -> io::Result<Self> {
        validate_pixels(width, height, rgba.len())?;
        Self::new(width, height, rgba.to_vec())
    }
}
fn validate_pixels(width: u32, height: u32, bytes: usize) -> io::Result<()> {
    if !(1..=ICON_EDGE).contains(&width)
        || !(1..=ICON_EDGE).contains(&height)
        || bytes != width as usize * height as usize * 4
    {
        return Err(invalid(
            "Command artwork requires at most 128 × 128 RGBA pixels.",
        ));
    }
    Ok(())
}
pub struct Snapshot {
    name: String,
    bytes: Vec<u8>,
}

pub fn encode(pixels: &SnapshotPixels, cancel: &Cancellation) -> io::Result<Snapshot> {
    active(cancel)?;
    validate_pixels(pixels.width, pixels.height, pixels.rgba.len())?;
    let source = image::RgbaImage::from_raw(pixels.width, pixels.height, pixels.rgba.clone())
        .ok_or_else(|| invalid("Invalid command artwork buffer."))?;
    let longest = pixels.width.max(pixels.height);
    let width = ((pixels.width * ICON_EDGE + longest / 2) / longest).max(1);
    let height = ((pixels.height * ICON_EDGE + longest / 2) / longest).max(1);
    let fitted = if (width, height) == (pixels.width, pixels.height) {
        source
    } else {
        image::imageops::resize(
            &source,
            width,
            height,
            image::imageops::FilterType::Triangle,
        )
    };
    let mut canvas = image::RgbaImage::new(ICON_EDGE, ICON_EDGE);
    image::imageops::overlay(
        &mut canvas,
        &fitted,
        i64::from((ICON_EDGE - width) / 2),
        i64::from((ICON_EDGE - height) / 2),
    );
    let mut bytes = Vec::new();
    image::codecs::png::PngEncoder::new(&mut bytes)
        .write_image(
            &canvas,
            ICON_EDGE,
            ICON_EDGE,
            image::ExtendedColorType::Rgba8,
        )
        .map_err(io::Error::other)?;
    active(cancel)?;
    if bytes.len() > MAX_PNG_BYTES {
        return Err(invalid(
            "The encoded command icon exceeds its resource budget.",
        ));
    }
    let name = format!("{:x}.png", Sha256::digest(&bytes));
    Ok(Snapshot { name, bytes })
}

/// The native caller supplies a pinned private directory and non-following regular-file
/// open/create operations. Existing different content is never replaced.
pub fn publish(
    directory: &Path,
    snapshot: &Snapshot,
    cancel: &Cancellation,
    create_exclusive: impl FnMut(&Path) -> io::Result<File>,
    open_existing: impl FnMut(&Path) -> io::Result<Option<File>>,
) -> io::Result<PathBuf> {
    publish_with_limit(
        directory,
        snapshot,
        cancel,
        create_exclusive,
        open_existing,
        MAX_SNAPSHOTS,
    )
}

/// Managed snapshots are named by their lowercase SHA-256 and `.png`; other entries, such as
/// a temporary file left by an interrupted publication, do not consume the snapshot budget.
fn is_snapshot_name(name: &std::ffi::OsStr) -> bool {
    name.to_str().is_some_and(|name| {
        name.len() == 68
            && name.ends_with(".png")
            && name.as_bytes()[..64]
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
    })
}

fn publish_with_limit(
    directory: &Path,
    snapshot: &Snapshot,
    cancel: &Cancellation,
    mut create_exclusive: impl FnMut(&Path) -> io::Result<File>,
    mut open_existing: impl FnMut(&Path) -> io::Result<Option<File>>,
    limit: usize,
) -> io::Result<PathBuf> {
    active(cancel)?;
    let destination = directory.join(&snapshot.name);
    // A recently written or refreshed snapshot is reused without the lock: collection
    // removes only snapshots older than its grace period, far beyond this window.
    if let Some(existing) = open_existing(&destination)? {
        verify_contents(&existing, &snapshot.bytes)?;
        if collection::recently_modified(&existing, SystemTime::now()) {
            collection::issued(&snapshot.name);
            return Ok(destination);
        }
    }
    let lock = acquire_lock(directory, &mut create_exclusive, &mut open_existing)?;
    let _unlock = Unlock(&lock);
    active(cancel)?;
    if let Some(existing) = open_existing(&destination)? {
        verify_contents(&existing, &snapshot.bytes)?;
        // Reusing an older snapshot restarts its grace period, so that another running
        // instance whose saved settings do not name it yet does not collect it.
        let _ = existing.set_modified(SystemTime::now());
        collection::issued(&snapshot.name);
        return Ok(destination);
    }
    let mut count = 0;
    for entry in fs::read_dir(directory)? {
        active(cancel)?;
        if !is_snapshot_name(&entry?.file_name()) {
            continue;
        }
        count += 1;
        if count >= limit {
            return Err(invalid("The managed command icon directory is full."));
        }
    }
    crate::settings::publish_atomic(&destination, &snapshot.bytes, create_exclusive, |path| {
        active(cancel)?;
        same_contents(open_existing(path)?, &snapshot.bytes).map(|_| ())
    })?;
    collection::issued(&snapshot.name);
    active(cancel)?;
    Ok(destination)
}

/// Takes the folder's `.snapshot.lock` without waiting: publication and collection
/// never interleave, also across processes. A busy lock is `WouldBlock`.
fn acquire_lock(
    directory: &Path,
    create_exclusive: &mut impl FnMut(&Path) -> io::Result<File>,
    open_existing: &mut impl FnMut(&Path) -> io::Result<Option<File>>,
) -> io::Result<File> {
    let lock_path = directory.join(".snapshot.lock");
    let lock = match create_exclusive(&lock_path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => open_existing(&lock_path)?
            .ok_or_else(|| invalid("The command icon lock disappeared."))?,
        Err(error) => return Err(error),
    };
    if !lock.metadata()?.is_file() || lock.metadata()?.len() != 0 {
        return Err(invalid(
            "The command icon lock is not an empty regular file.",
        ));
    }
    lock.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => io::Error::new(
            io::ErrorKind::WouldBlock,
            "Another command icon is being saved.",
        ),
        std::fs::TryLockError::Error(error) => error,
    })?;
    Ok(lock)
}

/// Releases the publication lock when a publication ends, explicitly rather than by
/// closing the lock file: a child process that another thread is starting at that moment
/// shares the process's open files until it executes its program, and the lock belongs to
/// the open file, so closing this handle alone would leave it held until then.
struct Unlock<'a>(&'a File);

impl Drop for Unlock<'_> {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

fn same_contents(file: Option<File>, expected: &[u8]) -> io::Result<bool> {
    let Some(file) = file else {
        return Ok(false);
    };
    verify_contents(&file, expected)?;
    Ok(true)
}

/// An existing snapshot must be a bounded regular file with exactly these bytes.
fn verify_contents(file: &File, expected: &[u8]) -> io::Result<()> {
    if !file.metadata()?.is_file() || file.metadata()?.len() > MAX_PNG_BYTES as u64 {
        return Err(invalid(
            "A managed command icon is not a bounded regular file.",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PNG_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes != expected {
        return Err(invalid(
            "A managed command icon has changed; it was not replaced.",
        ));
    }
    Ok(())
}
fn active(cancel: &Cancellation) -> io::Result<()> {
    if cancel.is_cancelled() {
        Err(io::ErrorKind::Interrupted.into())
    } else {
        Ok(())
    }
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{self, OpenOptions};
    fn pixels() -> SnapshotPixels {
        SnapshotPixels::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 128]).unwrap()
    }
    fn create(path: &Path) -> io::Result<File> {
        OpenOptions::new().write(true).create_new(true).open(path)
    }
    fn open(path: &Path) -> io::Result<Option<File>> {
        match File::open(path) {
            Ok(file) => Ok(Some(file)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error),
        }
    }
    #[test]
    fn exact_pixels_survive_png_and_identical_snapshots_share_one_managed_file() {
        let cancel = Cancellation::default();
        let snapshot = encode(&pixels(), &cancel).unwrap();
        let decoded = image::load_from_memory_with_format(&snapshot.bytes, image::ImageFormat::Png)
            .unwrap()
            .into_rgba8();
        assert_eq!(decoded.dimensions(), (128, 128));
        assert_eq!(decoded.get_pixel(0, 64).0, [255, 0, 0, 255]);
        assert_eq!(decoded.get_pixel(127, 64).0, [0, 0, 255, 128]);
        assert_eq!(decoded.get_pixel(64, 0).0, [0, 0, 0, 0]);
        assert_eq!(decoded.get_pixel(64, 127).0, [0, 0, 0, 0]);
        let folder = tempfile::tempdir().unwrap();
        let first = publish(folder.path(), &snapshot, &cancel, create, open).unwrap();
        let second = publish(
            folder.path(),
            &encode(&pixels(), &cancel).unwrap(),
            &cancel,
            create,
            open,
        )
        .unwrap();
        assert_eq!(first, second);
        assert_eq!(fs::read(&first).unwrap(), snapshot.bytes);
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 2);
        fs::write(&first, b"changed").unwrap();
        assert!(publish(folder.path(), &snapshot, &cancel, create, open).is_err());
        assert_eq!(fs::read(first).unwrap(), b"changed");
    }
    #[test]
    fn cancelled_or_invalid_artwork_never_publishes() {
        let cancel = Cancellation::default();
        let snapshot = encode(&pixels(), &cancel).unwrap();
        let folder = tempfile::tempdir().unwrap();
        cancel.cancel();
        assert!(encode(&pixels(), &cancel).is_err());
        assert!(publish(folder.path(), &snapshot, &cancel, create, open).is_err());
        assert_eq!(fs::read_dir(folder.path()).unwrap().count(), 0);
        let oversized = SnapshotPixels {
            width: 129,
            height: 1,
            rgba: vec![0; 516],
        };
        assert!(encode(&oversized, &Cancellation::default()).is_err());
    }
    #[test]
    fn full_managed_directory_refuses_an_additional_snapshot() {
        const LIMIT: usize = 4;
        let root = tempfile::tempdir().unwrap();
        for index in 0..LIMIT {
            fs::write(root.path().join(format!("{index:064x}.png")), []).unwrap();
        }
        let cancel = Cancellation::default();
        let snapshot = encode(&pixels(), &cancel).unwrap();
        let error =
            publish_with_limit(root.path(), &snapshot, &cancel, create, open, LIMIT).unwrap_err();
        assert!(error.to_string().contains("is full"), "{error}");
        assert!(!root.path().join(&snapshot.name).exists());
        // Removing one snapshot frees its slot again.
        fs::remove_file(root.path().join(format!("{:064x}.png", 0))).unwrap();
        publish_with_limit(root.path(), &snapshot, &cancel, create, open, LIMIT).unwrap();
        assert!(root.path().join(&snapshot.name).exists());
    }

    #[test]
    fn interrupted_temporaries_and_unrelated_entries_do_not_consume_the_budget() {
        const LIMIT: usize = 2;
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join(format!("{:064x}.png", 1)), []).unwrap();
        for stray in [
            ".midnight-preferences-7-1.tmp",
            ".midnight-preferences-7-2.tmp",
            "notes.txt",
            "ABCDEF.png",
        ] {
            fs::write(root.path().join(stray), []).unwrap();
        }
        let cancel = Cancellation::default();
        let snapshot = encode(&pixels(), &cancel).unwrap();
        publish_with_limit(root.path(), &snapshot, &cancel, create, open, LIMIT).unwrap();
        assert!(is_snapshot_name(std::ffi::OsStr::new(&snapshot.name)));
        assert!(!is_snapshot_name(std::ffi::OsStr::new(".snapshot.lock")));
    }

    #[test]
    fn a_concurrent_snapshot_publication_is_refused_rather_than_interleaved() {
        let root = tempfile::tempdir().unwrap();
        let lock = create(&root.path().join(".snapshot.lock")).unwrap();
        lock.lock().unwrap();
        let cancel = Cancellation::default();
        let snapshot = encode(&pixels(), &cancel).unwrap();
        let error = publish(root.path(), &snapshot, &cancel, create, open).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::WouldBlock);
        assert!(!root.path().join(&snapshot.name).exists());
        // Unlocked explicitly: a child process that another test starts at this moment
        // may share the open lock file until it executes its program.
        lock.unlock().unwrap();
        drop(lock);
        publish(root.path(), &snapshot, &cancel, create, open).unwrap();
    }

    #[test]
    fn a_finished_publication_releases_its_lock_while_a_copy_of_the_lock_file_stays_open() {
        // A copy of the open lock file stands in for a child process that another thread
        // starts during the publication: it shares the lock file until it executes.
        let root = tempfile::tempdir().unwrap();
        let cancel = Cancellation::default();
        let mut copies = Vec::new();
        let first = encode(&pixels(), &cancel).unwrap();
        let sharing = |path: &Path| {
            let file = create(path)?;
            if path.ends_with(".snapshot.lock") {
                copies.push(file.try_clone()?);
            }
            Ok(file)
        };
        publish(root.path(), &first, &cancel, sharing, open).unwrap();
        assert_eq!(copies.len(), 1, "the publication created the lock file");
        let other = SnapshotPixels::new(1, 1, vec![0, 255, 0, 255]).unwrap();
        let second = encode(&other, &cancel).unwrap();
        assert_ne!(first.name, second.name);
        publish(root.path(), &second, &cancel, create, open).unwrap();
        assert!(root.path().join(&second.name).exists());
    }
}
