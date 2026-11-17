//! A completed change stays completed when its private staging cannot be removed.
use super::simulation::with_interference_before_cleanup;
use crate::LinuxFileSystem;
use midnight_core::{
    Cancellation, DeleteMode, MetadataPatch, MutationResidue, MutationService, PreparedMutation,
};
use std::{
    ffi::OsStr,
    fs,
    io::ErrorKind,
    path::{Path, PathBuf},
    slice,
};

/// Every staging directory left in `directory`.
fn stages(directory: &Path) -> Vec<PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name.as_encoded_bytes().starts_with(b".midnight-mutation-"))
        })
        .collect()
}

/// Another program of the same user creates an entry in the staging directory.
fn foreign_entry(stage: &Path) {
    fs::write(stage.join("foreign"), b"another program").unwrap();
}

/// The staging directory remains, is named exactly and keeps the foreign entry.
fn assert_confirmed(residue: &MutationResidue, directory: &Path) {
    assert!(!residue.cleanup_unconfirmed, "{residue:?}");
    assert_eq!(stages(directory), slice::from_ref(&residue.path));
    // POSIX rmdir reports a non-empty directory as ENOTEMPTY or EEXIST.
    assert!(
        matches!(
            residue.error.kind,
            ErrorKind::DirectoryNotEmpty | ErrorKind::AlreadyExists
        ),
        "{residue:?}"
    );
    assert_eq!(
        fs::read(residue.path.join("foreign")).unwrap(),
        b"another program"
    );
    assert!(residue.to_string().contains("remains at"), "{residue}");
}

#[test]
fn a_completed_rename_reports_a_confirmed_staging_residue_instead_of_failing() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture.path().join("photo.jpg");
    fs::write(&original, b"pixels").unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare(&original, &cancel)
        .unwrap();
    let completion = with_interference_before_cleanup(foreign_entry, || {
        prepared.rename(OsStr::new("renamed.jpg"), &cancel)
    })
    .unwrap();
    assert_eq!(completion.destination, fixture.path().join("renamed.jpg"));
    assert_eq!(fs::read(&completion.destination).unwrap(), b"pixels");
    assert!(!original.exists());
    assert_confirmed(&completion.residue.unwrap(), fixture.path());
}

#[test]
fn a_moved_or_replaced_staging_directory_is_never_claimed_or_touched() {
    for replaced in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let original = fixture.path().join("source");
        fs::write(&original, b"payload").unwrap();
        let moved = fixture.path().join("moved by another program");
        let cancel = Cancellation::default();
        let prepared = LinuxFileSystem::new(None)
            .prepare(&original, &cancel)
            .unwrap();
        let interfere = {
            let moved = moved.clone();
            move |stage: &Path| {
                fs::rename(stage, &moved).unwrap();
                if replaced {
                    // An empty replacement must not be removed in place of ours.
                    fs::create_dir(stage).unwrap();
                }
            }
        };
        let completion = with_interference_before_cleanup(interfere, || {
            prepared.rename(OsStr::new("renamed"), &cancel)
        })
        .unwrap();
        assert_eq!(fs::read(&completion.destination).unwrap(), b"payload");
        let residue = completion.residue.unwrap();
        assert!(residue.cleanup_unconfirmed, "{residue:?}");
        let text = residue.to_string();
        assert!(text.contains("moved, removed or replaced"), "{text}");
        for claim in ["remains at", "no longer needed"] {
            assert!(!text.contains(claim), "{text}");
        }
        assert!(moved.is_dir());
        assert_eq!(residue.path.is_dir(), replaced, "{residue:?}");
        if !replaced {
            assert_eq!(residue.error.kind, ErrorKind::NotFound, "{residue:?}");
        }
    }
}

#[test]
fn a_completed_permanent_delete_reports_a_staging_residue_for_files_and_trees() {
    for tree in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let target = fixture.path().join("target");
        if tree {
            fs::create_dir_all(target.join("nested")).unwrap();
            fs::write(target.join("nested/file"), b"delete").unwrap();
        } else {
            fs::write(&target, b"delete").unwrap();
        }
        let cancel = Cancellation::default();
        let prepared = LinuxFileSystem::new(None)
            .prepare(&target, &cancel)
            .unwrap();
        let mut removed = 0;
        let completion = with_interference_before_cleanup(foreign_entry, || {
            prepared.delete(DeleteMode::Permanent, &cancel, &mut |_| removed += 1)
        })
        .unwrap();
        assert!(fs::symlink_metadata(&target).is_err());
        assert_eq!(removed, if tree { 3 } else { 1 });
        assert_confirmed(&completion.residue.unwrap(), fixture.path());
    }
}

#[test]
fn a_completed_trash_move_reports_a_staging_residue() {
    let fixture = tempfile::tempdir().unwrap();
    let trash = fixture.path().join("data/Trash");
    let source = fixture.path().join("source");
    fs::write(&source, b"recoverable").unwrap();
    let cancel = Cancellation::default();
    let prepared = super::Prepared::new(&source, Some(trash.clone()), &cancel).unwrap();
    let mut progress = Vec::new();
    let completion = with_interference_before_cleanup(foreign_entry, || {
        prepared.delete(DeleteMode::Trash, &cancel, &mut |path| {
            progress.push(path.to_owned())
        })
    })
    .unwrap();
    assert!(!source.exists());
    assert_eq!(progress, slice::from_ref(&source));
    let trashed: Vec<_> = fs::read_dir(trash.join("files"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(trashed.len(), 1);
    assert_eq!(fs::read(&trashed[0]).unwrap(), b"recoverable");
    assert_eq!(fs::read_dir(trash.join("info")).unwrap().count(), 1);
    assert_confirmed(&completion.residue.unwrap(), fixture.path());
}

#[test]
fn a_refused_rename_that_leaves_its_staging_names_it_and_is_never_a_plain_conflict() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    let occupied = fixture.path().join("occupied");
    fs::write(&source, b"selected").unwrap();
    fs::write(&occupied, b"existing").unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare(&source, &cancel)
        .unwrap();
    // The restored entry's now empty stage receives a foreign entry.
    let failure = with_interference_before_cleanup(foreign_entry, || {
        prepared.rename(OsStr::new("occupied"), &cancel)
    })
    .unwrap_err();
    assert_eq!(failure.kind, ErrorKind::Other, "{failure:?}");
    let stage = stages(fixture.path());
    assert_eq!(stage.len(), 1);
    assert!(
        failure.message.contains(stage[0].to_str().unwrap()),
        "{failure:?}"
    );
    assert!(failure.message.contains("remains at"), "{failure:?}");
    assert_eq!(fs::read(&source).unwrap(), b"selected");
    assert_eq!(fs::read(&occupied).unwrap(), b"existing");
    // Without interference the same refusal stays an ordinary name conflict.
    assert_eq!(
        prepared
            .rename(OsStr::new("occupied"), &cancel)
            .unwrap_err()
            .kind,
        ErrorKind::AlreadyExists
    );
}

#[test]
fn a_completed_metadata_rename_reports_its_staging_residue_as_kind_other() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture.path().join("source");
    fs::write(&original, b"payload").unwrap();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare(&original, &cancel)
        .unwrap();
    let outcome = with_interference_before_cleanup(foreign_entry, || {
        prepared.edit_metadata(
            &MetadataPatch::default(),
            Some(OsStr::new("renamed")),
            &cancel,
        )
    });
    assert!(outcome.changes.renamed);
    assert_eq!(outcome.current_path, fixture.path().join("renamed"));
    assert_eq!(fs::read(&outcome.current_path).unwrap(), b"payload");
    let error = outcome.error.unwrap();
    // Never a name conflict (which would prompt) or a cancellation of the completed rename.
    assert_eq!(error.kind, ErrorKind::Other, "{error:?}");
    assert!(
        error
            .message
            .starts_with("The entry was renamed, but its temporary staging folder remains at"),
        "{error:?}"
    );
    assert_eq!(stages(fixture.path()), slice::from_ref(&error.path));
}
