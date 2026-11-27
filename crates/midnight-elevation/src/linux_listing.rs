//! The helper's folder access on Linux. A requested folder is opened as a directory,
//! following symbolic links as a navigation would; its location is the path and mount that
//! the kernel reports for that open descriptor; and it is listed through the same
//! descriptor, so nothing can exchange the folder between the containment check and the
//! listing.
use crate::{
    helper::Entries,
    protocol::{self, WireEntry},
};
use midnight_core::Cancellation;
use std::{
    ffi::CString,
    fs::{self, DirEntry},
    io::{self, Read},
    os::{
        fd::{AsRawFd, FromRawFd, OwnedFd},
        unix::{ffi::OsStrExt, fs::MetadataExt},
    },
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

/// Longest `/proc` text read for a descriptor.
const MAX_PROC_TEXT: u64 = 64 * 1024;

/// A folder opened for listing.
pub(crate) struct Directory(OwnedFd);

/// Where an opened folder is: the mount it is on and its path there.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Location {
    mount: u64,
    path: PathBuf,
}

impl Location {
    /// Whether `other` is this folder or below it on the same mount: a filesystem mounted
    /// on a folder inside it is not.
    pub(crate) fn contains(&self, other: &Self) -> bool {
        self.mount == other.mount && other.path.starts_with(&self.path)
    }
}

/// Open the folder at `path` for listing; anything but a directory is refused without being
/// opened (a FIFO or device is never opened).
pub(crate) fn open_directory(path: &Path) -> io::Result<Directory> {
    let name = CString::new(path.as_os_str().as_bytes())
        .map_err(|_| protocol::invalid("NUL in native path"))?;
    // SAFETY: a terminated path; ownership is taken only after success.
    let raw = unsafe {
        libc::open(
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOCTTY,
        )
    };
    if raw < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: open returned a new owned descriptor.
    Ok(Directory(unsafe { OwnedFd::from_raw_fd(raw) }))
}

fn proc_text(path: &str) -> io::Result<Vec<u8>> {
    let mut text = Vec::new();
    fs::File::open(path)?
        .take(MAX_PROC_TEXT + 1)
        .read_to_end(&mut text)?;
    if text.len() as u64 > MAX_PROC_TEXT {
        return Err(protocol::invalid("Oversized descriptor information"));
    }
    Ok(text)
}

/// The mount identifier in a descriptor's `/proc/self/fdinfo` text.
fn parse_mount(text: &[u8]) -> io::Result<u64> {
    text.split(|byte| *byte == b'\n')
        .find_map(|line| line.strip_prefix(b"mnt_id:"))
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.trim().parse().ok())
        .ok_or_else(|| protocol::invalid("Missing folder mount"))
}

impl Directory {
    /// The descriptor's own path, through which the folder is listed.
    fn proc_path(&self) -> String {
        format!("/proc/self/fd/{}", self.0.as_raw_fd())
    }

    /// The folder's location now, wherever links led the open.
    pub(crate) fn location(&self) -> io::Result<Location> {
        let path = fs::read_link(self.proc_path())?;
        if !path.is_absolute() {
            return Err(protocol::invalid("Unexpected folder location"));
        }
        let mount = parse_mount(&proc_text(&format!(
            "/proc/self/fdinfo/{}",
            self.0.as_raw_fd()
        ))?)?;
        Ok(Location { mount, path })
    }
}

/// Unix links record no target type. stat follows the link but never opens the target.
fn is_directory_link(entry: &DirEntry) -> bool {
    fs::metadata(entry.path()).is_ok_and(|value| value.is_dir())
}

/// List the opened folder. Its descriptor's `/proc` path names exactly the opened folder;
/// entries come from enumeration and a no-follow stat relative to it, so no entry is opened.
pub(crate) fn list_directory(
    directory: &Directory,
    cancel: &Cancellation,
) -> io::Result<Vec<WireEntry>> {
    let mut entries = Entries::default();
    for entry in fs::read_dir(directory.proc_path())? {
        if cancel.is_cancelled() {
            return Err(crate::cancelled());
        }
        let entry = entry?;
        let file_type = entry.file_type()?;
        let metadata = entry.metadata().ok();
        let kind = if file_type.is_symlink() {
            if is_directory_link(&entry) { 1 } else { 3 }
        } else if file_type.is_dir() {
            0
        } else if file_type.is_file() {
            2
        } else {
            4
        };
        let modified = metadata
            .as_ref()
            .and_then(|value| value.modified().ok())
            .and_then(|time| match time.duration_since(UNIX_EPOCH) {
                Ok(value) => i64::try_from(value.as_secs())
                    .ok()
                    .map(|seconds| (seconds, value.subsec_nanos())),
                Err(error) => {
                    let value = error.duration();
                    let seconds = i64::try_from(value.as_secs()).ok()?;
                    if value.subsec_nanos() == 0 {
                        Some((-seconds, 0))
                    } else {
                        seconds
                            .checked_add(1)
                            .map(|seconds| (-seconds, 1_000_000_000 - value.subsec_nanos()))
                    }
                }
            });
        entries.push(WireEntry {
            name: entry.file_name().as_bytes().to_vec(),
            kind,
            size: metadata
                .as_ref()
                .filter(|_| kind == 2)
                .map(fs::Metadata::len),
            modified,
            native_attributes: None,
            unix_mode: metadata.as_ref().map(MetadataExt::mode),
        })?;
    }
    Ok(entries.into_vec())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locations_contain_their_own_subtree_on_their_own_mount_only() {
        let location = |mount, path: &str| Location {
            mount,
            path: PathBuf::from(path),
        };
        let root = location(7, "/home/other");
        assert!(root.contains(&root));
        assert!(root.contains(&location(7, "/home/other/x/y")));
        for outside in [
            location(7, "/home"),
            location(7, "/home/other2"),
            location(7, "/home/othe"),
            location(8, "/home/other/mounted"),
        ] {
            assert!(!root.contains(&outside), "{outside:?}");
        }
    }

    #[test]
    fn mount_identifiers_are_read_from_descriptor_information() {
        assert_eq!(
            parse_mount(b"pos:\t0\nflags:\t02100000\nmnt_id:\t31\nino:\t5\n").unwrap(),
            31
        );
        assert!(parse_mount(b"pos:\t0\nflags:\t0\n").is_err());
        assert!(parse_mount(b"mnt_id:\tx\n").is_err());
    }
}
