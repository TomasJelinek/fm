//! Real junctions below `_build` that deny Everyone listing, as Windows' compatibility
//! junctions (`My Music`, `Application Data`) do; the ACL edits apply only to these
//! fixtures and are restored before they are removed.
use super::*;
use crate::providers::NativeProvider;
use midnight_core::{DirectoryService, EntryKind};
use midnight_platform::LocalFileSystem;
use midnight_platform_windows::{SavedDacl, deny_read_data, new_junction, restore_dacl};
use std::{
    ffi::OsString,
    fs,
    os::windows::ffi::{OsStrExt, OsStringExt},
    path::{Component, PathBuf, Prefix},
    sync::Arc,
};

/// Denies Everyone listing on one fixture entry itself (a junction, never its target)
/// and restores the original DACL on drop.
struct DeniedListing(PathBuf, SavedDacl);
impl DeniedListing {
    fn new(path: &Path) -> Self {
        Self(path.to_owned(), deny_read_data(path).unwrap())
    }
}
impl Drop for DeniedListing {
    fn drop(&mut self) {
        if let Err(error) = restore_dacl(&self.0, &self.1)
            && !std::thread::panicking()
        {
            panic!(
                "cannot restore fixture ACL at {}: {error}",
                self.0.display()
            );
        }
    }
}

/// Two denied junctions: `alias` to a readable `target` (one subfolder `Album` with a
/// `leaf`, and two files: 5 and 7 bytes), and `private_alias` to a `private` folder
/// whose own listing is denied too.
pub(crate) struct DeniedJunctions {
    // Declared first, so dropped first: the ACLs are restored before removal.
    _denied: Vec<DeniedListing>,
    pub(crate) alias: PathBuf,
    pub(crate) target: PathBuf,
    pub(crate) private_alias: PathBuf,
    pub(crate) private: PathBuf,
    _directory: tempfile::TempDir,
}

/// The ordinary (non-verbatim) spelling of the canonical `_build/tmp`, as panels use.
fn fixture_root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tmp/browse-listing");
    fs::create_dir_all(&root).unwrap();
    let root = fs::canonicalize(root).unwrap();
    match root.components().next() {
        Some(Component::Prefix(prefix)) if matches!(prefix.kind(), Prefix::VerbatimDisk(_)) => {
            let units: Vec<_> = root.as_os_str().encode_wide().collect();
            PathBuf::from(OsString::from_wide(&units[4..]))
        }
        _ => root,
    }
}

pub(crate) fn denied_junctions() -> DeniedJunctions {
    let directory = tempfile::Builder::new()
        .prefix("denied-junctions-")
        .tempdir_in(fixture_root())
        .unwrap();
    let target = directory.path().join("Music 文档 & co");
    fs::create_dir_all(target.join("Album").join("leaf")).unwrap();
    fs::write(target.join("song.txt"), [1; 5]).unwrap();
    fs::write(target.join("Album").join("track.bin"), [2; 7]).unwrap();
    let private = directory.path().join("Private");
    fs::create_dir_all(private.join("inner")).unwrap();
    let alias = directory.path().join("My Music");
    let private_alias = directory.path().join("My Private");
    new_junction(&alias, &target).unwrap();
    new_junction(&private_alias, &private).unwrap();
    let denied = vec![
        DeniedListing::new(&alias),
        DeniedListing::new(&private_alias),
        DeniedListing::new(&private),
    ];
    let filesystem = LocalFileSystem::new(None);
    for path in [&alias, &private_alias] {
        let error = filesystem
            .read_directory(path, &Cancellation::default(), &mut |_| true)
            .unwrap_err();
        assert_eq!(error.kind, io::ErrorKind::PermissionDenied, "{path:?}");
    }
    assert!(
        fs::read_dir(&target).is_ok(),
        "the alias's denial is its own"
    );
    DeniedJunctions {
        _denied: denied,
        alias,
        target,
        private_alias,
        private,
        _directory: directory,
    }
}

fn listed(path: &Path, link: bool) -> Result<Vec<(PathBuf, EntryKind)>, FsError> {
    let mut rows = Vec::new();
    read_browse_directory(
        &LocalFileSystem::new(None),
        path,
        link,
        &Cancellation::default(),
        &mut |entry| {
            rows.push((entry.path, entry.kind));
            true
        },
    )?;
    rows.sort_by(|left, right| left.0.cmp(&right.0));
    Ok(rows)
}

#[test]
fn denied_junction_lists_its_target_below_the_link_only_as_a_link() {
    let fixture = denied_junctions();
    let alias = &fixture.alias;
    assert_eq!(
        listed(alias, true).unwrap(),
        [
            (alias.join("Album"), EntryKind::Directory),
            (alias.join("song.txt"), EntryKind::File),
        ]
    );
    // An entry not known as a link keeps the ordinary listing and its denial.
    assert_eq!(
        listed(alias, false).unwrap_err().kind,
        io::ErrorKind::PermissionDenied
    );
    // A link whose target denies listing still fails, naming the target.
    let error = listed(&fixture.private_alias, true).unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::PermissionDenied);
    assert!(
        error.path.ends_with(fixture.private.file_name().unwrap()),
        "{error}"
    );
    // A readable link is listed through its own path once, without resolution.
    assert_eq!(
        listed(&fixture.target, true).unwrap(),
        [
            (fixture.target.join("Album"), EntryKind::Directory),
            (fixture.target.join("song.txt"), EntryKind::File),
        ]
    );
}

#[test]
fn denied_junction_presence_probe_reports_folders_and_a_denied_target_stays_unavailable() {
    let fixture = denied_junctions();
    let native = NativeProvider::new(Arc::new(LocalFileSystem::new(None)));
    let cancel = Cancellation::default();
    assert!(
        native
            .has_subfolders(&fixture.alias, true, &cancel)
            .unwrap()
    );
    assert!(
        native
            .has_subfolders(&fixture.alias.join("Album"), false, &cancel)
            .unwrap(),
        "a child is reached through the link's path"
    );
    let error = native
        .has_subfolders(&fixture.private_alias, true, &cancel)
        .unwrap_err()
        .to_string();
    assert!(error.contains("PermissionDenied"), "{error}");
}

#[test]
fn denied_junction_watch_probe_observes_its_target() {
    let fixture = denied_junctions();
    let filesystem = LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    let probe = |path: &Path, link| {
        crate::watch::signature::probe_shown(&filesystem, path, true, false, link, &cancel)
    };
    let through_link = probe(&fixture.alias, true).expect("observed through its target");
    assert!(probe(&fixture.alias, false).is_err());
    assert!(probe(&fixture.private_alias, true).is_err());
    fs::create_dir(fixture.target.join("added")).unwrap();
    let changed = probe(&fixture.alias, true).unwrap();
    assert_ne!(
        through_link.full, changed.full,
        "target changes are observed"
    );
    assert_ne!(through_link.display, changed.display);
}
