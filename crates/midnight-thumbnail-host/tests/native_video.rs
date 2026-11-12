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
