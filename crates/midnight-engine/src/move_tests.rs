//! The first Move increment accepts bounded ordinary file roots only.
use super::*;
use std::ffi::OsStr;
#[cfg(target_os = "linux")]
#[path = "move_linux_tests.rs"]
mod linux;
#[path = "copy_metadata_tests.rs"]
mod metadata;

fn protected_fixture() -> (tempfile::TempDir, PathBuf) {
    use std::io::Write;
    let temp = fixture();
    let provider = midnight_platform::LocalFileSystem::new(None);
    let cancel = Cancellation::default();
    fs::write(temp.path().join("seed"), b"seed").unwrap();
    let destination = provider
        .prepare_transfer_destination(temp.path(), &cancel)
        .unwrap();
    let target = destination
        .prepare_target(OsStr::new("seed"), &cancel)
        .unwrap()
        .unwrap();
    let mut stage = destination
        .stage_file_for_target(
            OsStr::new("seed"),
            target,
            TransferReplacementPolicy::RecoverableCurrentEntry,
            &cancel,
        )
        .unwrap();
    stage.write_all(b"fixture replacement").unwrap();
    let TransferPublication::Replaced { recovery, .. } =
        stage.publish_replace(OsStr::new("seed"), &cancel).unwrap()
    else {
        panic!("fixture recovery creation failed");
    };
    let parent = recovery.path.parent().unwrap().to_owned();
    drop(stage);
    (temp, parent)
}

#[cfg(windows)]
#[path = "move_windows_tests.rs"]
mod windows;
