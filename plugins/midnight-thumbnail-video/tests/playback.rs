use midnight_thumbnail_video::playback::{self, Frame, Metadata, OutputBounds};
use std::{
    cell::Cell,
    io::{self, Cursor, Read, Seek, SeekFrom},
    rc::Rc,
};

#[path = "fixtures/avi.rs"]
mod fixture;

const SMALL: OutputBounds = OutputBounds {
    width: 32,
    height: 32,
};

fn check(frame: &Frame, timestamp: i64, color: [u8; 4]) {
    assert_eq!(frame.presentation_micros, timestamp);
    assert_eq!((frame.width, frame.height), (2, 2));
    assert_eq!(frame.rgba.len(), 16);
    assert!(
        frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .all(|pixel| *pixel == color)
    );
}

#[test]
fn consecutive_frames_keep_real_timestamps_and_do_not_sample_the_duration() {
    // Reopening/sampling instead of retaining the decoder repeats or skips these known pixels.
    let bytes = fixture::sized_video(2, 2, 6, 60);
    playback::with_cursor(
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        SMALL,
        |cursor| {
            assert_eq!(cursor.metadata().duration_micros, Some(10_000_000));
            for (timestamp, color) in [
                (0, [30, 255, 0, 255]),
                (166_666, [30, 215, 40, 255]),
                (333_333, [30, 175, 80, 255]),
                (500_000, [30, 135, 120, 255]),
                (666_666, [30, 95, 160, 255]),
                (833_333, [30, 55, 200, 255]),
            ] {
                check(
                    &cursor.next_frame()?.expect("fixture frame"),
                    timestamp,
                    color,
                );
            }
            // Declared duration is ten seconds; actual EOF must not repeat the last frame.
            assert!(cursor.next_frame()?.is_none());
            assert!(cursor.next_frame()?.is_none());
            Ok(())
        },
    )
    .unwrap();
}

#[test]
fn metadata_reports_real_container_duration_and_original_unscaled_dimensions() {
    let bytes = fixture::sized_video(640, 480, 3, 3);
    playback::with_cursor(
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        SMALL,
        |cursor| {
            let expected = Metadata {
                duration_micros: Some(500_000),
                width: 640,
                height: 480,
            };
            assert_eq!(cursor.metadata(), expected);
            let frame = cursor.next_frame()?.unwrap();
            assert_eq!((frame.width, frame.height), (32, 24));
            cursor.seek(200_000)?;
            let frame = cursor.next_frame()?.unwrap();
            assert_eq!(frame.presentation_micros, 333_333);
            assert_eq!(cursor.metadata(), expected);
            Ok(())
        },
    )
    .unwrap();
}

#[test]
fn actual_seek_flushes_old_frames_and_skips_every_frame_before_the_target() {
    let bytes = fixture::video();
    playback::with_cursor(
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        SMALL,
        |cursor| {
            check(&cursor.next_frame()?.unwrap(), 0, [30, 255, 0, 255]);
            cursor.seek(500_000)?;
            check(&cursor.next_frame()?.unwrap(), 500_000, [30, 135, 120, 255]);
            // A non-frame-aligned backwards seek must skip 0 and 1/6, not return a keyframe.
            cursor.seek(200_000)?;
            check(&cursor.next_frame()?.unwrap(), 333_333, [30, 175, 80, 255]);
            check(&cursor.next_frame()?.unwrap(), 500_000, [30, 135, 120, 255]);
            Ok(())
        },
    )
    .unwrap();
}

#[test]
fn seeking_after_eof_restarts_the_same_cursor_and_clears_decoder_drain_state() {
    let bytes = fixture::video();
    playback::with_cursor(
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        SMALL,
        |cursor| {
            for _ in 0..6 {
                assert!(cursor.next_frame()?.is_some());
            }
            assert!(cursor.next_frame()?.is_none());
            cursor.seek(0)?;
            check(&cursor.next_frame()?.unwrap(), 0, [30, 255, 0, 255]);
            check(&cursor.next_frame()?.unwrap(), 166_666, [30, 215, 40, 255]);
            Ok(())
        },
    )
    .unwrap();
}

#[test]
fn invalid_seek_range_does_not_discard_the_next_consecutive_frame() {
    let bytes = fixture::video();
    playback::with_cursor(
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        SMALL,
        |cursor| {
            check(&cursor.next_frame()?.unwrap(), 0, [30, 255, 0, 255]);
            assert_eq!(
                cursor.seek(u64::MAX).unwrap_err().kind(),
                io::ErrorKind::InvalidInput
            );
            check(&cursor.next_frame()?.unwrap(), 166_666, [30, 215, 40, 255]);
            Ok(())
        },
    )
    .unwrap();
}

struct Untouched;
impl Read for Untouched {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("invalid playback request read its source")
    }
}
impl Seek for Untouched {
    fn seek(&mut self, _: SeekFrom) -> io::Result<u64> {
        panic!("invalid playback request sought its source")
    }
}

#[test]
fn invalid_output_allocations_and_source_length_are_rejected_before_open() {
    for bounds in [
        OutputBounds {
            width: 0,
            height: 1,
        },
        OutputBounds {
            width: 4097,
            height: 1,
        },
        OutputBounds {
            width: 4096,
            height: 4096,
        },
    ] {
        let result = playback::with_cursor(&mut Untouched, 1, bounds, |_| Ok(()));
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }
    let result = playback::with_cursor(&mut Untouched, u64::MAX, SMALL, |_| Ok(()));
    assert_eq!(result.unwrap_err().kind(), io::ErrorKind::InvalidInput);
}

struct Source {
    bytes: Cursor<Vec<u8>>,
    largest_read: usize,
    fail: Rc<Cell<bool>>,
    reads: Rc<Cell<usize>>,
}

impl Read for Source {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.reads.set(self.reads.get() + 1);
        self.largest_read = self.largest_read.max(bytes.len());
        if self.fail.get() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "owned playback source failed",
            ));
        }
        self.bytes.read(bytes)
    }
}
impl Seek for Source {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.bytes.seek(from)
    }
}

fn source(bytes: Vec<u8>) -> Source {
    Source {
        bytes: Cursor::new(bytes),
        largest_read: 0,
        fail: Rc::new(Cell::new(false)),
        reads: Rc::new(Cell::new(0)),
    }
}

#[test]
fn large_frames_use_bounded_broker_reads_and_aspect_preserving_output() {
    let mut source = source(fixture::sized_video(640, 480, 3, 3));
    let length = source.bytes.get_ref().len() as u64;
    let bounds = OutputBounds {
        width: 320,
        height: 200,
    };
    playback::with_cursor(&mut source, length, bounds, |cursor| {
        let frame = cursor.next_frame()?.unwrap();
        assert_eq!(frame.presentation_micros, 0);
        assert_eq!((frame.width, frame.height), (267, 200));
        assert_eq!(frame.rgba.len(), 267 * 200 * 4);
        assert!(
            frame
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == [30, 255, 0, 255])
        );
        Ok(())
    })
    .unwrap();
    assert!(source.largest_read > 0 && source.largest_read <= 256 * 1024);
}

#[test]
fn an_open_read_failure_cannot_reach_the_metadata_consumer() {
    let mut source = source(fixture::video());
    let length = source.bytes.get_ref().len() as u64;
    source.fail.set(true);
    let result: io::Result<()> = playback::with_cursor(&mut source, length, SMALL, |_| {
        panic!("failed source reached the metadata consumer")
    });
    let error = result.unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    assert_eq!(error.to_string(), "owned playback source failed");
}

#[test]
fn a_source_failure_is_terminal_and_cannot_reopen_or_resume_the_cursor() {
    let mut source = source(fixture::sized_video(640, 480, 48, 48));
    let length = source.bytes.get_ref().len() as u64;
    let fail = source.fail.clone();
    let reads = source.reads.clone();
    playback::with_cursor(&mut source, length, SMALL, |cursor| {
        fail.set(true);
        // Stream probing can retain some complete packets, but cannot retain this whole movie.
        let mut failed = false;
        for _ in 0..49 {
            match cursor.next_frame() {
                Ok(Some(_)) => {}
                Ok(None) => panic!("source failure became a successful EOF"),
                Err(error) => {
                    assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
                    assert_eq!(error.to_string(), "owned playback source failed");
                    failed = true;
                    break;
                }
            }
        }
        assert!(failed, "playback did not report the source read failure");
        fail.set(false);
        let before = reads.get();
        assert!(cursor.next_frame().is_err());
        assert!(cursor.seek(0).is_err());
        assert_eq!(
            reads.get(),
            before,
            "failed cursor touched the source again"
        );
        Ok(())
    })
    .unwrap();
}

#[test]
fn malformed_and_playlist_sources_are_rejected_without_a_playback_cursor() {
    for bytes in [
        b"not a video".as_slice(),
        b"#EXTM3U\nhttp://127.0.0.1/private\n".as_slice(),
    ] {
        let result: io::Result<()> =
            playback::with_cursor(&mut Cursor::new(bytes), bytes.len() as u64, SMALL, |_| {
                panic!("invalid source reached playback")
            });
        assert!(result.is_err());
    }
}
