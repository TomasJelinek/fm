//! Exclusive creation and explicit native cleanup failures, without fake filesystem calls.
use super::*;
use std::{cell::Cell, fs, mem::size_of};

fn directory(path: &Path) -> Directory {
    Directory::from_file(native::open_parent(path).unwrap(), path.to_owned()).unwrap()
}

#[test]
fn fat_copy_stages_verifies_and_publishes_without_a_persistent_file_id() {
    let temp = crate::test_fixtures::build_temp("fixture-");
    let cancel = Cancellation::default();
    let directory = Directory::from_copy_probe(
        native::open_parent(temp.path()).unwrap(),
        temp.path().to_owned(),
        Err(io::Error::from_raw_os_error(87)),
        |_| Ok(true),
    )
    .unwrap();
    let name = OsStr::new("the published file has a much longer name.txt");
    assert!(
        directory
            .native_child_namespace_scope(name, EntryKind::File, &cancel)
            .unwrap()
            .is_none()
    );
    let child = directory
        .create_directory(OsStr::new("copies"), &cancel)
        .unwrap();
    let target = directory
        .prepare_target(OsStr::new("copies"), &cancel)
        .unwrap()
        .unwrap();
    let moved = temp.path().join("swapped folder");
    assert!(fs::rename(child.path(), &moved).is_err());
    target.revalidate(&cancel).unwrap();
    let merged = directory
        .open_merge_directory(OsStr::new("copies"), target.entry(), &cancel)
        .unwrap();
    assert_eq!(merged.identity(), child.identity());
    drop(merged);
    drop(target);
    let mut stage = child.stage_job_file(&cancel).unwrap();
    stage.write_all(b"copy from NTFS").unwrap();
    stage.sync(&cancel).unwrap();
    let guard = stage.rewind_for_validation(&cancel).unwrap();
    let mut contents = Vec::new();
    stage.read_to_end(&mut contents).unwrap();
    guard.revalidate(&cancel).unwrap();
    drop(guard);
    assert_eq!(contents, b"copy from NTFS");
    stage.publish_no_replace(name, &cancel).unwrap();
    let published = stage.revalidate_publication(&cancel).unwrap();
    assert_eq!(
        child.inspect(name, &cancel).unwrap().unwrap().identity,
        published
    );
    drop(stage);
    let mut collision = child.stage_job_file(&cancel).unwrap();
    collision.write_all(b"do not overwrite").unwrap();
    assert_eq!(
        collision
            .publish_no_replace(name, &cancel)
            .unwrap_err()
            .kind,
        io::ErrorKind::AlreadyExists
    );
    collision.cleanup().unwrap();
    drop(collision);
    assert_eq!(fs::read(child.path().join(name)).unwrap(), contents);
    assert_eq!(fs::read_dir(child.path()).unwrap().count(), 1);
}

#[test]
fn fat_copy_fallback_requires_an_unsupported_id_class_and_a_known_local_fat_volume() {
    let temp = crate::test_fixtures::build_temp("fixture-");
    let path = temp.path();
    let denied = Directory::from_copy_probe(
        native::open_parent(path).unwrap(),
        path.to_owned(),
        Err(io::ErrorKind::PermissionDenied.into()),
        |_| panic!("permission denial is final"),
    )
    .err()
    .unwrap();
    assert_eq!(denied.kind, io::ErrorKind::PermissionDenied);
    let unknown = Directory::from_copy_probe(
        native::open_parent(path).unwrap(),
        path.to_owned(),
        Err(io::ErrorKind::Unsupported.into()),
        |_| Ok(false),
    )
    .err()
    .unwrap();
    assert_eq!(unknown.kind, io::ErrorKind::Unsupported);
}

#[test]
fn fat_copy_requires_an_unknown_namespace_reservation_before_creating_a_stage() {
    use midnight_core::{TransferNamespacePin, TransferNamespaceProbe};
    use midnight_platform_common::transfer_namespace::{
        TransferNamespaceChange, TransferNamespaceChanges, with_transfer_namespace_changes,
    };
    struct Refuse;
    impl TransferNamespaceChanges for Refuse {
        fn begin(&self, _: &Cancellation) -> Result<Box<dyn TransferNamespaceChange>, FsError> {
            Ok(Box::new(Refuse))
        }
    }
    impl TransferNamespaceChange for Refuse {
        fn reserve_private_child(
            &mut self,
            pin: Option<Arc<dyn TransferNamespacePin>>,
            cancel: &Cancellation,
        ) -> Result<bool, FsError> {
            assert!(matches!(
                pin.unwrap().refresh(cancel)?,
                TransferNamespaceProbe::Unknown
            ));
            Ok(false)
        }
        fn finish(self: Box<Self>) -> Result<(), FsError> {
            Ok(())
        }
    }
    let temp = crate::test_fixtures::build_temp("fixture-");
    let directory = Directory::from_copy_probe(
        native::open_parent(temp.path()).unwrap(),
        temp.path().to_owned(),
        Err(io::ErrorKind::Unsupported.into()),
        |_| Ok(true),
    )
    .unwrap();
    let result = with_transfer_namespace_changes(Arc::new(Refuse), || {
        directory.stage_job_file(&Cancellation::default())
    });
    assert_eq!(result.err().unwrap().kind, io::ErrorKind::WouldBlock);
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn colliding_compact_names_retry_exclusively_and_preserve_the_existing_winner() {
    let temp = crate::test_fixtures::build_temp("fixture-");
    let directory = directory(temp.path());
    let occupied = OsString::from(".me-0000000000000000");
    let fresh = OsString::from(".me-0000000000000001");
    fs::write(temp.path().join(&occupied), b"existing winner").unwrap();
    let attempts = Cell::new(0);
    let mut stage = directory
        .create_stage_named(false, None, &Cancellation::default(), || {
            let attempt = attempts.get();
            attempts.set(attempt + 1);
            Ok(if attempt == 0 {
                occupied.clone()
            } else {
                fresh.clone()
            })
        })
        .unwrap();
    assert_eq!(attempts.get(), 2);
    stage.write_all(b"new stage").unwrap();
    assert_eq!(
        fs::read(temp.path().join(&occupied)).unwrap(),
        b"existing winner"
    );
    stage.cleanup().unwrap();
    drop(stage);
    assert!(!temp.path().join(fresh).exists());
    assert_eq!(
        fs::read(temp.path().join(&occupied)).unwrap(),
        b"existing winner"
    );

    attempts.set(0);
    let failure = directory
        .create_stage_named(false, None, &Cancellation::default(), || {
            attempts.set(attempts.get() + 1);
            Ok(occupied.clone())
        })
        .err()
        .expect("bounded collisions must fail without replacing a winner");
    assert_eq!(failure.kind, io::ErrorKind::AlreadyExists);
    assert_eq!(attempts.get(), 16);
    assert_eq!(
        fs::read(temp.path().join(occupied)).unwrap(),
        b"existing winner"
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 1);
}

fn attributes(stage: &Stage, attributes: u32) {
    let information = FILE_BASIC_INFO {
        FileAttributes: attributes,
        ..Default::default()
    };
    // SAFETY: this fixture stage requested WRITE_ATTRIBUTES. Only its retained handle is
    // changed; the owned test restores NORMAL before retrying cleanup and leaving scope.
    assert_ne!(
        unsafe {
            SetFileInformationByHandle(
                stage.file.as_raw_handle(),
                FileBasicInfo,
                (&information as *const FILE_BASIC_INFO).cast(),
                size_of::<FILE_BASIC_INFO>() as u32,
            )
        },
        0
    );
}

#[test]
fn cleanup_refusal_is_reported_and_retry_removes_only_the_owned_stage() {
    let temp = crate::test_fixtures::build_temp("fixture-");
    let directory = directory(temp.path());
    let mut stage = directory
        .create_stage(true, None, &Cancellation::default())
        .unwrap();
    stage.write_all(b"partially written bytes").unwrap();
    let stage_path = stage.path.clone();
    attributes(&stage, FILE_ATTRIBUTE_READONLY);
    let failure = stage.cleanup();
    // Always restore the fixture attribute before inspecting the expected refusal.
    attributes(&stage, FILE_ATTRIBUTE_NORMAL);
    let failure = failure.expect_err("an OS refusal cannot silently claim cleanup success");
    assert_eq!(failure.path, stage_path);
    assert!(!stage.cleaned);
    assert!(stage_path.exists());
    stage.cleanup().unwrap();
    drop(stage);
    assert!(!stage_path.exists());
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
}

#[test]
fn cleanup_unlinks_stage_while_a_delete_sharing_reader_retains_its_bytes() {
    use std::os::windows::fs::OpenOptionsExt;
    let temp = crate::test_fixtures::build_temp("fixture-");
    let directory = directory(temp.path());
    let mut stage = directory
        .create_stage(false, None, &Cancellation::default())
        .unwrap();
    stage.write_all(b"retained reader bytes").unwrap();
    let stage_path = stage.path.clone();
    let mut reader = fs::OpenOptions::new()
        .access_mode(FILE_READ_DATA)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(&stage_path)
        .unwrap();
    stage.cleanup().unwrap();
    drop(stage);
    assert!(
        !stage_path.exists(),
        "successful cleanup must remove the stage namespace entry"
    );
    assert_eq!(fs::read_dir(temp.path()).unwrap().count(), 0);
    let mut retained = Vec::new();
    reader.read_to_end(&mut retained).unwrap();
    assert_eq!(retained, b"retained reader bytes");
}

struct FailingTarget {
    entry: TransferEntry,
    directory: PathBuf,
    residual: std::sync::Mutex<Option<(PathBuf, fs::Permissions)>>,
    failure_kind: io::ErrorKind,
}
impl TransferTarget for FailingTarget {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, cancel: &Cancellation) -> Result<(), FsError> {
        let stage = fs::read_dir(&self.directory)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path != &self.entry.item.path);
        let Some(stage) = stage else {
            return Ok(());
        };
        let mut permissions = fs::metadata(&stage).unwrap().permissions();
        let original_permissions = permissions.clone();
        permissions.set_readonly(true);
        fs::set_permissions(&stage, permissions).unwrap();
        *self.residual.lock().unwrap() = Some((stage, original_permissions));
        if self.failure_kind == io::ErrorKind::Interrupted {
            cancel.cancel();
        }
        Err(error(
            &self.entry.item.path,
            io::Error::new(self.failure_kind, "fixture target validation failed"),
        ))
    }
}

#[test]
fn stage_factory_failure_reports_original_error_and_exact_cleanup_residual() {
    factory_failure(io::ErrorKind::InvalidInput);
}

#[test]
fn stage_factory_cancel_does_not_hide_native_cleanup_refusal_as_interrupted() {
    factory_failure(io::ErrorKind::Interrupted);
}

fn factory_failure(failure_kind: io::ErrorKind) {
    for preserving in [false, true] {
        let temp = crate::test_fixtures::build_temp("fixture-");
        let target_path = temp.path().join("target");
        fs::write(&target_path, b"original target").unwrap();
        let directory = directory(temp.path());
        let cancel = Cancellation::default();
        let expected = Arc::new(FailingTarget {
            entry: directory
                .inspect(OsStr::new("target"), &cancel)
                .unwrap()
                .unwrap(),
            directory: temp.path().to_owned(),
            residual: Default::default(),
            failure_kind,
        });
        let result = if preserving {
            directory.stage_file_preserving_target(
                OsStr::new("target"),
                expected.clone(),
                TransferReplacementPolicy::RecoverableCurrentEntry,
                &cancel,
            )
        } else {
            directory.stage_file_for_target(
                OsStr::new("target"),
                expected.clone(),
                TransferReplacementPolicy::RecoverableCurrentEntry,
                &cancel,
            )
        };
        let failure = result.err().expect("target validation must fail");
        let (residual, original_permissions) = expected
            .residual
            .lock()
            .unwrap()
            .clone()
            .expect("fixture touched only newly owned stage");
        // Restore only this owned fixture, even when the diagnostic assertion fails.
        fs::set_permissions(&residual, original_permissions).unwrap();
        fs::remove_file(&residual).unwrap();
        assert_eq!(fs::read(&target_path).unwrap(), b"original target");
        assert_eq!(
            cancel.is_cancelled(),
            failure_kind == io::ErrorKind::Interrupted
        );
        assert_eq!(
            failure.kind,
            io::ErrorKind::PermissionDenied,
            "real native cleanup refusal must survive cancellation filtering"
        );
        assert_eq!(failure.path, residual);
        assert!(failure.message.contains("fixture target validation failed"));
        assert!(
            failure.message.contains(target_path.to_str().unwrap()),
            "factory cleanup failure lost original operation context: {failure}"
        );
    }
}
