use midnight_thumbnail_api::Request;
use midnight_thumbnail_host::decode::{Source, decode};
use std::{io, path::PathBuf, sync::atomic::AtomicBool, time::Duration};
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
#[test]
#[ignore = "Run with MIDNIGHT_VIDEO_TEST_HELPER pointing to the separately built video provider and adjacent native runtime"]
fn actual_sandboxed_video_provider_returns_distinct_sampled_frames() {
    let helper = PathBuf::from(
        std::env::var_os("MIDNIGHT_VIDEO_TEST_HELPER").expect("Explicit video helper required"),
    );
    let source = Bytes(fixture::video());
    let mut frames = Vec::new();
    for frame in [0, 2, 4] {
        let result = decode(
            &helper,
            &source,
            Request {
                id: 9,
                source_length: source.length(),
                width: 32,
                height: 32,
                frame,
            },
            &AtomicBool::new(false),
            Duration::from_secs(10),
        )
        .unwrap();
        assert_eq!((result.width, result.height, result.frame), (2, 2, frame));
        frames.push(result.rgba);
    }
    assert_ne!(frames[0], frames[1]);
    assert_ne!(frames[1], frames[2]);
}

/// The Windows platform helper (Media Foundation) in the real sandbox: AppContainer, job and its
/// own signed-images-only policy, reading through the broker. The fixtures come from the video
/// plugin's `write_fixtures_for_the_sandboxed_helper_test`.
#[test]
#[ignore = "Run with MIDNIGHT_PLATFORM_VIDEO_TEST_HELPER (midnight-thumbnail-video-system) and MIDNIGHT_PLATFORM_VIDEO_TEST_FIXTURES"]
fn actual_sandboxed_platform_video_provider_decodes_what_windows_decodes() {
    let helper = PathBuf::from(
        std::env::var_os("MIDNIGHT_PLATFORM_VIDEO_TEST_HELPER")
            .expect("Explicit platform helper required"),
    );
    let fixtures = PathBuf::from(
        std::env::var_os("MIDNIGHT_PLATFORM_VIDEO_TEST_FIXTURES").expect("Fixture folder required"),
    );
    let mut decoded = 0;
    for name in ["h264.mp4", "hevc.mp4"] {
        let path = fixtures.join(name);
        if name != "h264.mp4" && !path.exists() {
            println!("{name}: not written on this system");
            continue;
        }
        let source = Bytes(std::fs::read(&path).unwrap());
        for frame in [0, 5] {
            let result = decode(
                &helper,
                &source,
                Request {
                    id: 11,
                    source_length: source.length(),
                    width: 64,
                    height: 64,
                    frame,
                },
                &AtomicBool::new(false),
                Duration::from_secs(10),
            )
            .unwrap_or_else(|error| panic!("{name} sample {frame}: {error}"));
            assert_eq!(
                (result.width, result.height, result.frame),
                (64, 36, frame),
                "{name}"
            );
            // The left half is magenta until halfway through the movie, then green.
            let pixel = &result.rgba[(18 * 64 + 16) * 4..][..4];
            let magenta = pixel[0] > 140 && pixel[1] < 110 && pixel[2] > 140;
            let green = pixel[0] < 110 && pixel[1] > 140 && pixel[2] < 110;
            assert!(
                if frame == 0 { magenta } else { green },
                "{name} sample {frame}: {pixel:?}"
            );
        }
        println!("{name}: decoded in the sandbox");
        decoded += 1;
    }
    assert!(decoded >= 1);
}
