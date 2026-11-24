//! A release ZIP carries no NTFS permissions: extracted, the video helper's FFmpeg libraries
//! inherit only their folder's, which grant the helpers' AppContainer nothing. The helper
//! starts only because the host adds that grant in a private installation.
#![cfg(windows)]
use midnight_thumbnail_api::Request;
use midnight_thumbnail_host::decode::{Source, decode};
use std::{
    io,
    path::{Path, PathBuf},
    sync::atomic::AtomicBool,
    time::Duration,
};
#[path = "../../../plugins/midnight-thumbnail-video/tests/fixtures/avi.rs"]
mod fixture;

struct Bytes(Vec<u8>);
impl Source for Bytes {
    fn length(&self) -> u64 {
        self.0.len() as u64
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> io::Result<()> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        self.0
            .get(offset as usize..offset as usize + length as usize)
            .map(<[u8]>::to_vec)
            .ok_or_else(|| io::ErrorKind::UnexpectedEof.into())
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
fn an_extracted_video_helper_loads_its_libraries_in_a_private_installation() {
    let package = PathBuf::from(
        std::env::var_os("MIDNIGHT_THUMBNAILERS_TEST_PACKAGE").expect("A built package"),
    );
    let base = std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("A private test root");
    let root = tempfile::tempdir_in(base).unwrap();
    let installation = root.path().join("Midnight Explorer");
    std::fs::create_dir(&installation).unwrap();
    let thumbnailers = installation.join("thumbnailers");
    extract(&package, &thumbnailers);
    let thumbnailers = thumbnailers.canonicalize().unwrap();
    let avcodec = thumbnailers.join("avcodec-63.dll");
    assert!(
        midnight_worker_platform::unloadable_library(std::slice::from_ref(&avcodec)).is_some(),
        "the extracted library already grants the helpers' sandbox"
    );
    let source = Bytes(fixture::video());
    let frame = decode(
        &thumbnailers.join("midnight-thumbnail-video.exe"),
        &source,
        Request {
            id: 1,
            source_length: source.length(),
            width: 32,
            height: 32,
            frame: 0,
        },
        &AtomicBool::new(false),
        Duration::from_secs(10),
    )
    .unwrap_or_else(|error| panic!("{error}"));
    assert_eq!((frame.width, frame.height), (2, 2));
    assert_eq!(
        midnight_worker_platform::unloadable_library(&midnight_worker_platform::worker_libraries(
            &thumbnailers.join("midnight-thumbnail-video.exe")
        )),
        None
    );
}
