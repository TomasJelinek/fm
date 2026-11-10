#![cfg(windows)]
//! Real native staging, including long paths and cancelled independent readback.
use midnight_core::{Cancellation, TransferPublication, TransferService};
use midnight_platform_common::transfer::{CopyPhase, VerifiedFileCopy};
use midnight_platform_windows::WindowsFileSystem;
use std::{
    ffi::OsStr,
    fs, io,
    os::windows::ffi::OsStrExt,
    path::{Path, PathBuf},
};

fn fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("compact-stage-")
        .tempdir_in(root)
        .unwrap()
}

fn names(path: &Path) -> Vec<PathBuf> {
    let mut paths: Vec<_> = fs::read_dir(path)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    paths.sort();
    paths
}

#[test]
fn compact_stages_publish_short_names_under_long_unicode_parents() {
    let temp = fixture();
    let source = temp.path().join("source");
    fs::write(&source, b"long-path payload").unwrap();
    let mut target = temp.path().join("destination");
    while target.as_os_str().encode_wide().count() <= 280 {
        target.push("文档-component-0123456789");
    }
    fs::create_dir_all(&target).unwrap();
    let cancel = Cancellation::default();
    let provider = WindowsFileSystem::new(None);
    let directory = provider
        .prepare_transfer_destination(&target, &cancel)
        .unwrap();
    let stage = directory.stage_file(&cancel).unwrap();
    let paths = names(&target);
    assert_eq!(paths.len(), 1);
    let name = paths[0].file_name().unwrap().to_str().unwrap();
    assert_eq!(name.len(), 20, "staging basename must be compact: {name}");
    assert!(name.starts_with(".me-"));
    assert!(name[4..].bytes().all(|byte| byte.is_ascii_hexdigit()));
    let source_pin = provider.prepare_transfer_source(&source, &cancel).unwrap();
    let mut copy = VerifiedFileCopy::new(source_pin, stage).unwrap();
    while copy.phase() != CopyPhase::ReadyToPublish {
        copy.step(&cancel).unwrap();
    }
    assert!(matches!(
        copy.publish(OsStr::new("x"), &cancel).unwrap(),
        TransferPublication::Published(_)
            | TransferPublication::PublishedDurabilityUnconfirmed { .. }
    ));
    drop(copy);
    assert_eq!(names(&target), vec![target.join("x")]);
    assert_eq!(fs::read(target.join("x")).unwrap(), b"long-path payload");
    assert_eq!(fs::read(source).unwrap(), b"long-path payload");
}

#[test]
fn cancellation_removes_partial_write_and_readback_stages_without_touching_existing_files() {
    for readback in [false, true] {
        let temp = fixture();
        let source = temp.path().join("source");
        let target = temp.path().join("destination");
        fs::create_dir(&target).unwrap();
        let existing = target.join("existing");
        fs::write(&existing, b"keep existing target").unwrap();
        let bytes = vec![0x59; 900_123];
        fs::write(&source, &bytes).unwrap();
        let provider = WindowsFileSystem::new(None);
        let cancel = Cancellation::default();
        let directory = provider
            .prepare_transfer_destination(&target, &cancel)
            .unwrap();
        let mut copy = VerifiedFileCopy::new(
            provider.prepare_transfer_source(&source, &cancel).unwrap(),
            directory.stage_file(&cancel).unwrap(),
        )
        .unwrap();
        copy.step(&cancel).unwrap();
        assert!(copy.copied_bytes() > 0 && copy.copied_bytes() < bytes.len() as u64);
        if readback {
            while copy.phase() != CopyPhase::Verifying {
                copy.step(&cancel).unwrap();
            }
            copy.step(&cancel).unwrap();
            assert!(copy.verified_bytes() > 0 && copy.verified_bytes() < bytes.len() as u64);
        }
        assert_eq!(
            names(&target).len(),
            2,
            "a real unpublished stage exists before cancellation"
        );
        cancel.cancel();
        assert_eq!(
            copy.step(&cancel).unwrap_err().kind,
            io::ErrorKind::Interrupted
        );
        copy.cleanup()
            .expect("explicit worker cleanup must report any OS refusal");
        drop(copy);
        assert_eq!(names(&target), vec![existing.clone()]);
        assert_eq!(fs::read(existing).unwrap(), b"keep existing target");
        assert_eq!(fs::read(source).unwrap(), bytes);
    }
}
