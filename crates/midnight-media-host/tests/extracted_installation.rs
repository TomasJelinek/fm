//! A release ZIP carries no NTFS permissions: extracted, the playback helpers and the FFmpeg
//! libraries inherit only their folder's, which grant the helpers' AppContainer nothing. The
//! helpers start only because the host adds that grant in a private installation.
#![cfg(windows)]
use midnight_media_host::{
    CpuBudget, ResourcePolicy,
    video_installation::{VideoInstallation, VideoWorker},
    video_process::VideoProcess,
};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};

/// A new folder below the private test root, removed afterwards.
struct Folder(PathBuf);
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Write each file of the built `package` into a new `destination` as an extraction does:
/// new files with the folder's inherited permissions only.
fn extract(package: &Path, destination: &Path) {
    std::fs::create_dir(destination).unwrap();
    for entry in std::fs::read_dir(package).unwrap() {
        let entry = entry.unwrap();
        if entry.file_type().unwrap().is_file() {
            let bytes = std::fs::read(entry.path()).unwrap();
            std::fs::write(destination.join(entry.file_name()), bytes).unwrap();
        }
    }
}

#[test]
#[ignore = "Run with a private HOST_TEST_INSTALL_ROOT and MIDNIGHT_THUMBNAILERS_TEST_PACKAGE, \
            a built thumbnailers folder"]
fn extracted_playback_helpers_start_in_a_private_installation() {
    let package = PathBuf::from(
        std::env::var_os("MIDNIGHT_THUMBNAILERS_TEST_PACKAGE").expect("A built package"),
    );
    let base = PathBuf::from(std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("A private root"));
    let root = Folder(base.join(format!("extracted-playback-{}", std::process::id())));
    std::fs::create_dir(&root.0).unwrap();
    let installation = root.0.canonicalize().unwrap();
    extract(&package, &installation.join("thumbnailers"));
    let executable = installation.join("midnight-explorer.exe");
    std::fs::write(&executable, b"application").unwrap();
    let avcodec = installation.join("thumbnailers").join("avcodec-63.dll");
    assert!(
        midnight_worker_platform::unloadable_library(std::slice::from_ref(&avcodec)).is_some(),
        "the extracted library already grants the helpers' sandbox"
    );
    let video = VideoInstallation::discover(&executable).unwrap();
    assert!(
        video.platform_worker().is_some(),
        "both helpers are installed"
    );
    let mut failures = Vec::new();
    for which in [VideoWorker::Bundled, VideoWorker::Platform] {
        let launched = VideoProcess::launch(
            &video,
            which,
            ResourcePolicy {
                memory_bytes: 1024 * 1024 * 1024,
                cpu: CpuBudget::NoAdditionalLimit,
            },
            Arc::new(AtomicBool::new(false)),
            Instant::now() + Duration::from_secs(20),
            (),
        );
        match launched {
            Ok(process) => {
                let retirement = process.retirement();
                drop(process);
                retirement
                    .wait(Instant::now() + Duration::from_secs(10))
                    .unwrap();
            }
            Err(error) => failures.push(format!("{which:?}: {error}")),
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
