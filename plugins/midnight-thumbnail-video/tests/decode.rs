use midnight_thumbnail_sdk::api::Request;
use std::io::Cursor;

#[path = "fixtures/avi.rs"]
mod fixture;
use fixture::video;
#[test]
fn real_video_samples_different_bounded_frames() {
    let bytes = video();
    let mut results = Vec::new();
    for frame in [0, 2, 4] {
        let request = Request {
            id: 1,
            source_length: bytes.len() as u64,
            width: 32,
            height: 32,
            frame,
        };
        let result = midnight_thumbnail_video::decode(&request, &mut Cursor::new(&bytes)).unwrap();
        assert_eq!((result.width, result.height), (2, 2));
        assert_eq!(result.rgba.len(), 16);
        assert!(
            result
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[3] == 255)
        );
        results.push(result.rgba);
    }
    assert_ne!(results[0], results[1]);
    assert_ne!(results[1], results[2]);
}
#[test]
fn malformed_and_playlist_sources_are_rejected() {
    for bytes in [
        b"not a video".as_slice(),
        b"#EXTM3U\nhttp://127.0.0.1/private\n".as_slice(),
    ] {
        let request = Request {
            id: 1,
            source_length: bytes.len() as u64,
            width: 32,
            height: 32,
            frame: 0,
        };
        assert!(midnight_thumbnail_video::decode(&request, &mut Cursor::new(bytes)).is_err());
    }
}
