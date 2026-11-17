//! Private per-user spool roots for generated file lists.
use midnight_worker_platform::spool::{PrivateSpoolDirectory, Spool};
use std::{io, path::PathBuf};

#[cfg(windows)]
#[path = "file_list_spool_windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "file_list_spool_linux.rs"]
mod native;

/// Create an anonymous spool in this user's first usable private root. A root that
/// is refused (foreign owner, shared permissions, links) or lacks anonymous-file
/// support yields to the next candidate; the last failure is reported otherwise.
pub(super) fn create() -> io::Result<Spool> {
    create_in(native::candidates())
}

fn create_in(candidates: Vec<PathBuf>) -> io::Result<Spool> {
    let mut failure = io::Error::new(
        io::ErrorKind::NotFound,
        "No private per-user spool directory is available.",
    );
    for root in candidates {
        match PrivateSpoolDirectory::open(&root).and_then(|directory| directory.create()) {
            Ok(spool) => return Ok(spool),
            Err(error) => {
                failure = io::Error::new(error.kind(), format!("{}: {error}", root.display()));
            }
        }
    }
    Err(failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unusable_spool_root_yields_to_the_next_private_candidate() {
        let temp = tempfile::tempdir().unwrap();
        // Another account (or anything else) occupying a candidate name cannot
        // deny the spool while a later private candidate is usable.
        let occupied = temp.path().join("occupied");
        std::fs::write(&occupied, b"not a directory").unwrap();
        let private = temp.path().join("private");
        let mut spool = create_in(vec![occupied.clone(), private.clone()]).unwrap();
        assert_eq!(spool.append(0, b"listed").unwrap(), 6);
        spool.finish().unwrap();
        assert!(private.is_dir());
        let Err(error) = create_in(vec![occupied.clone()]) else {
            panic!("an occupied name is not a private spool root");
        };
        assert!(error.to_string().contains("occupied"), "{error}");
        assert!(create_in(Vec::new()).is_err());
    }
}
