use midnight_thumbnail_sdk::api::Request;
use std::io::Cursor;

#[path = "fixtures/avi.rs"]
mod fixture;
use fixture::{color, sized_video, video};

fn sample(bytes: &[u8], frame: u8) -> std::io::Result<midnight_thumbnail_sdk::api::Frame> {
    let request = Request {
        id: 1,
        source_length: bytes.len() as u64,
        width: 32,
        height: 32,
        frame,
    };
    midnight_thumbnail_video::decode(&request, &mut Cursor::new(bytes))
}

/// The opaque RGBA pixel of uncompressed fixture frame `index`, stored as BGR.
fn rgba(index: u8) -> [u8; 4] {
    let [blue, green, red] = color(index);
    [red, green, blue, 255]
}

#[test]
fn frames_larger_than_one_brokered_read_are_decoded() {
    // 640x480 BGR24 frames are 921,600 bytes. Beyond what its 64 KiB buffer holds,
    // AVIO reads the rest of each chunk directly, asking for far more than the
    // 256 KiB a single brokered read returns.
    let bytes = sized_video(640, 480, 3, 3);
    for frame in [0, 2] {
        let result = sample(&bytes, frame).unwrap();
        assert_eq!((result.width, result.height), (32, 24));
        let first = result.rgba.as_chunks::<4>().0[0];
        assert!((0..3).any(|index| rgba(index) == first), "{first:?}");
        assert!(
            result
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == first)
        );
    }
}

#[test]
fn overstated_duration_samples_the_last_decoded_frame() {
    // The headers declare 60 frames (10 s) but only 6 frames (1 s) are present.
    let bytes = sized_video(2, 2, 6, 60);
    for frame in [3, 5] {
        let result = sample(&bytes, frame).unwrap();
        assert_eq!(result.rgba.as_chunks::<4>().0[0], rgba(5), "sample {frame}");
    }
}
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
