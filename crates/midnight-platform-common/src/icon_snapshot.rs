//! Content-addressed command artwork; native adapters own directory and file authority.
use image::ImageEncoder;
use midnight_core::Cancellation;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
};

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
    mut create_exclusive: impl FnMut(&Path) -> io::Result<File>,
    mut open_existing: impl FnMut(&Path) -> io::Result<Option<File>>,
) -> io::Result<PathBuf> {
    active(cancel)?;
    let destination = directory.join(&snapshot.name);
    if same_contents(open_existing(&destination)?, &snapshot.bytes)? {
        return Ok(destination);
    }
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
    fs2::FileExt::try_lock_exclusive(&lock)?;
    active(cancel)?;
    if same_contents(open_existing(&destination)?, &snapshot.bytes)? {
        return Ok(destination);
    }
    let mut count = 0;
    for entry in fs::read_dir(directory)? {
        active(cancel)?;
        if entry?.file_name() == ".snapshot.lock" {
            continue;
        }
        count += 1;
        if count >= MAX_SNAPSHOTS {
            return Err(invalid("The managed command icon directory is full."));
        }
    }
    crate::settings::publish_atomic(&destination, &snapshot.bytes, create_exclusive, |path| {
        active(cancel)?;
        same_contents(open_existing(path)?, &snapshot.bytes).map(|_| ())
    })?;
    active(cancel)?;
    Ok(destination)
}

fn same_contents(file: Option<File>, expected: &[u8]) -> io::Result<bool> {
    let Some(file) = file else {
        return Ok(false);
    };
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
    Ok(true)
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
        let root = tempfile::tempdir().unwrap();
        for index in 0..MAX_SNAPSHOTS {
            fs::write(root.path().join(format!("existing-{index}.png")), []).unwrap();
        }
        let cancel = Cancellation::default();
        let snapshot = encode(&pixels(), &cancel).unwrap();
        assert!(publish(root.path(), &snapshot, &cancel, create, open).is_err());
        assert!(!root.path().join(snapshot.name).exists());
    }
}
