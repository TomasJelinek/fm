//! A completed change stays completed when its private staging cannot be removed.
use super::simulation::{with_interference_after_failed_removal, with_interference_before_cleanup};
use crate::LinuxFileSystem;
use midnight_core::{
    Cancellation, DeleteMode, MetadataPatch, MutationResidue, MutationService, PreparedMutation,
    TimeEdit,
};
use std::{
    ffi::OsStr,
    fs,
    io::ErrorKind,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    slice,
    time::{Duration, UNIX_EPOCH},
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

/// The text of a residue whose staging binding was not confirmed claims nothing more.
fn assert_unconfirmed(residue: &MutationResidue) {
    assert!(residue.cleanup_unconfirmed, "{residue:?}");
    let text = residue.to_string();
    assert!(
        text.contains("could not be confirmed after cleanup"),
        "{text}"
    );
    assert!(
        text.contains("another program may have moved, removed or replaced it"),
        "{text}"
    );
    for claim in ["remains at", "no longer needed"] {
        assert!(!text.contains(claim), "{text}");
    }
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
        assert_unconfirmed(&residue);
        assert!(moved.is_dir());
        assert_eq!(residue.path.is_dir(), replaced, "{residue:?}");
        if !replaced {
            assert_eq!(residue.error.kind, ErrorKind::NotFound, "{residue:?}");
        }
    }
}

#[test]
fn a_staging_directory_moved_or_replaced_after_its_failed_removal_is_never_claimed() {
    for replaced in [false, true] {
        let fixture = tempfile::tempdir().unwrap();
        let original = fixture.path().join("source");
        fs::write(&original, b"payload").unwrap();
        let moved = fixture.path().join("moved by another program");
        let cancel = Cancellation::default();
        let prepared = LinuxFileSystem::new(None)
            .prepare(&original, &cancel)
            .unwrap();
        // rmdir fails on the foreign entry; only then does the binding change.
        let interfere = {
            let moved = moved.clone();
            move |stage: &Path| {
                fs::rename(stage, &moved).unwrap();
                if replaced {
                    fs::create_dir(stage).unwrap();
                }
            }
        };
        let completion = with_interference_before_cleanup(foreign_entry, || {
            with_interference_after_failed_removal(interfere, || {
                prepared.rename(OsStr::new("renamed"), &cancel)
            })
        })
        .unwrap();
        assert_eq!(fs::read(&completion.destination).unwrap(), b"payload");
        let residue = completion.residue.unwrap();
        assert_unconfirmed(&residue);
        let changed = if replaced {
            ErrorKind::Other
        } else {
            ErrorKind::NotFound
        };
        assert_eq!(residue.error.kind, changed, "{residue:?}");
        // The moved stage keeps the foreign entry; a replacement is left untouched.
        assert_eq!(fs::read(moved.join("foreign")).unwrap(), b"another program");
        assert_eq!(residue.path.is_dir(), replaced, "{residue:?}");
        if replaced {
            assert_eq!(fs::read_dir(&residue.path).unwrap().count(), 0);
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
fn a_failed_capture_request_reports_where_the_item_may_be_never_an_unneeded_stage() {
    let fixture = tempfile::tempdir().unwrap();
    let source = fixture.path().join("source");
    fs::write(&source, b"selected").unwrap();
    let cancel = Cancellation::default();
    let prepared = super::Prepared::new(&source, None, &cancel).unwrap();
    // A capture request that returned an error may still have moved the item (a lost
    // reply on a network or FUSE filesystem). An entry already at the staged name
    // stands in for it: the request fails and the stage cannot be removed.
    let staged = super::Staged::create(&prepared).unwrap();
    let occupied = staged.path.join(super::STAGED_NAME);
    fs::write(&occupied, b"occupant").unwrap();
    let failure = staged.capture(&prepared, &cancel).unwrap_err();
    // Never a plain name conflict, which would drop the location.
    assert_eq!(failure.kind, ErrorKind::Other, "{failure:?}");
    let location = format!(
        "; the item may have been moved into {}: ",
        occupied.display()
    );
    assert!(failure.message.contains(&location), "{failure:?}");
    for claim in ["unused", "no longer needed", "not part of this operation"] {
        assert!(!failure.message.contains(claim), "{failure:?}");
    }
    assert_eq!(fs::read(&source).unwrap(), b"selected");
    assert_eq!(fs::read(&occupied).unwrap(), b"occupant");
    // Without a capture request the stage cannot hold the item: it is a residue.
    let staged = super::Staged::create(&prepared).unwrap();
    fs::write(staged.path.join("other"), b"other").unwrap();
    fs::rename(&source, fixture.path().join("moved")).unwrap();
    let failure = staged.capture(&prepared, &cancel).unwrap_err();
    assert_eq!(failure.kind, ErrorKind::Other, "{failure:?}");
    let residue = format!(
        "; its temporary staging folder remains at {} because",
        staged.path.display()
    );
    assert!(failure.message.contains(&residue), "{failure:?}");
    assert!(!failure.message.contains("may have been moved into"));
}

/// A name, permissions and a modification time in one Properties edit.
fn rename_mode_and_time() -> (MetadataPatch, std::time::SystemTime) {
    let modified = UNIX_EPOCH + Duration::from_secs(1_600_000_000);
    let patch = MetadataPatch {
        unix_mask: 0o777,
        unix_value: 0o600,
        modified: TimeEdit::Set(modified),
        ..MetadataPatch::default()
    };
    (patch, modified)
}

#[test]
fn a_metadata_rename_with_a_staging_residue_still_applies_the_remaining_fields() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture.path().join("source");
    fs::write(&original, b"payload").unwrap();
    fs::set_permissions(&original, fs::Permissions::from_mode(0o644)).unwrap();
    let (patch, modified) = rename_mode_and_time();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare(&original, &cancel)
        .unwrap();
    let outcome = with_interference_before_cleanup(foreign_entry, || {
        prepared.edit_metadata(&patch, Some(OsStr::new("renamed")), &cancel)
    });
    let changes = outcome.changes;
    assert!(
        changes.renamed && changes.permissions && changes.modified,
        "{outcome:?}"
    );
    assert_eq!(outcome.current_path, fixture.path().join("renamed"));
    assert_eq!(fs::read(&outcome.current_path).unwrap(), b"payload");
    let applied = fs::symlink_metadata(&outcome.current_path).unwrap();
    assert_eq!(applied.permissions().mode() & 0o7777, 0o600);
    assert_eq!(applied.modified().unwrap(), modified);
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

#[test]
fn a_metadata_step_failing_after_a_residue_names_both_and_is_never_a_cancellation() {
    let fixture = tempfile::tempdir().unwrap();
    let original = fixture.path().join("source");
    fs::write(&original, b"payload").unwrap();
    fs::set_permissions(&original, fs::Permissions::from_mode(0o644)).unwrap();
    let (patch, _) = rename_mode_and_time();
    let cancel = Cancellation::default();
    let prepared = LinuxFileSystem::new(None)
        .prepare(&original, &cancel)
        .unwrap();
    // Cancellation after publication stops the permission step that follows.
    let interfere = {
        let cancel = cancel.clone();
        move |stage: &Path| {
            foreign_entry(stage);
            cancel.cancel();
        }
    };
    let outcome = with_interference_before_cleanup(interfere, || {
        prepared.edit_metadata(&patch, Some(OsStr::new("renamed")), &cancel)
    });
    assert!(outcome.changes.renamed, "{outcome:?}");
    assert!(!outcome.changes.permissions && !outcome.changes.modified);
    let renamed = fixture.path().join("renamed");
    assert_eq!(outcome.current_path, renamed);
    let unchanged = fs::symlink_metadata(&renamed).unwrap();
    assert_eq!(unchanged.permissions().mode() & 0o7777, 0o644);
    let error = outcome.error.unwrap();
    assert_eq!(error.kind, ErrorKind::Other, "{error:?}");
    let stage = stages(fixture.path());
    assert_eq!(stage.len(), 1);
    let residue = format!(
        "; the entry was renamed, but its temporary staging folder remains at {}",
        stage[0].display()
    );
    assert!(
        error.message.starts_with("operation cancelled"),
        "{error:?}"
    );
    assert!(error.message.contains(&residue), "{error:?}");
}
