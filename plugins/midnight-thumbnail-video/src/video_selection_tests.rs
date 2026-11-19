use super::{eligible_video, ffmpeg, native, open_video, playback, video_index, with_decoder};
use ffmpeg::{codec::Id, format::stream::Disposition, media::Type};
use std::{io, io::Cursor};

#[path = "../tests/fixtures/tracks_avi.rs"]
mod fixture;

#[test]
fn preferred_eligible_video_is_preserved_instead_of_the_first_eligible_track() {
    let tracks = [(0, true), (1, false), (2, true)];
    assert_eq!(video_index(Some(2), tracks.into_iter()), Some(2));
}

#[test]
fn an_unsupported_preferred_track_falls_back_past_cover_art_and_audio() {
    let tracks = [
        (
            0,
            eligible_video(Type::Video, Id::MJPEG, Disposition::DEFAULT),
        ),
        (
            1,
            eligible_video(Type::Video, Id::RAWVIDEO, Disposition::ATTACHED_PIC),
        ),
        (
            2,
            eligible_video(Type::Audio, Id::RAWVIDEO, Disposition::empty()),
        ),
        (
            3,
            eligible_video(Type::Video, Id::RAWVIDEO, Disposition::empty()),
        ),
    ];
    assert_eq!(video_index(Some(0), tracks.into_iter()), Some(3));
}

#[test]
fn cover_art_and_audio_alone_never_become_a_video_track() {
    let tracks = [
        (
            0,
            eligible_video(Type::Video, Id::VP8, Disposition::ATTACHED_PIC),
        ),
        (
            1,
            eligible_video(Type::Audio, Id::VP9, Disposition::DEFAULT),
        ),
    ];
    assert_eq!(video_index(Some(0), tracks.into_iter()), None);
}

#[test]
fn an_absent_preference_uses_the_first_eligible_video_without_inventing_an_index() {
    assert_eq!(
        video_index(None, [(0, false), (1, true)].into_iter()),
        Some(1)
    );
    assert_eq!(video_index(Some(99), [(0, false)].into_iter()), None);
}

fn native_selection(bytes: &[u8]) -> (Option<usize>, io::Result<usize>) {
    with_decoder(|| {
        native::with_input(&mut Cursor::new(bytes), bytes.len() as u64, |input| {
            let preferred = input
                .streams()
                .best(Type::Video)
                .map(|stream| stream.index());
            Ok((preferred, open_video(input).map(|video| video.index)))
        })
    })
    .unwrap()
}

fn check_second_track(bytes: &[u8], seek: bool) {
    playback::with_cursor(
        &mut Cursor::new(bytes),
        bytes.len() as u64,
        playback::OutputBounds {
            width: 16,
            height: 16,
        },
        |cursor| {
            let first = cursor.next_frame()?.unwrap();
            assert_eq!(first.presentation_micros, 0);
            assert_eq!((first.width, first.height), (2, 2));
            assert!(
                first
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| *pixel == [70, 60, 50, 255])
            );
            if seek {
                cursor.seek(200_000)?;
                assert_eq!(cursor.next_frame()?.unwrap().presentation_micros, 333_333);
            }
            Ok(())
        },
    )
    .unwrap();
}

#[test]
fn real_avi_keeps_a_supported_preferred_track_after_an_earlier_eligible_track() {
    let bytes = fixture::two_tracks(1, false);
    let (preferred, selected) = native_selection(&bytes);
    assert_eq!(
        preferred,
        Some(1),
        "fixture must exercise FFmpeg's non-first preference"
    );
    assert_eq!(selected.unwrap(), 1);
    check_second_track(&bytes, false);
}

#[test]
fn real_avi_can_decode_and_seek_an_alternate_to_an_unsupported_preferred_track() {
    let bytes = fixture::two_tracks(0, true);
    let (preferred, selected) = native_selection(&bytes);
    assert_eq!(
        preferred,
        Some(0),
        "fixture must exercise an unsupported preferred video"
    );
    assert_eq!(selected.unwrap(), 1);
    check_second_track(&bytes, true);
}

#[test]
fn real_avi_alternate_seek_does_not_walk_the_unsupported_default_track() {
    let bytes = fixture::long_unsupported_prefix();
    let (preferred, selected) = native_selection(&bytes);
    assert_eq!(
        preferred,
        Some(0),
        "fixture must retain its unsupported native preference"
    );
    assert_eq!(selected.unwrap(), 1);
    playback::with_cursor(
        &mut Cursor::new(&bytes),
        bytes.len() as u64,
        playback::OutputBounds {
            width: 16,
            height: 16,
        },
        |cursor| {
            cursor.seek(200_000)?;
            let frame = cursor.next_frame()?.unwrap();
            assert_eq!(frame.presentation_micros, 333_333);
            assert!(
                frame
                    .rgba
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .all(|pixel| *pixel == [70, 60, 50, 255])
            );
            Ok(())
        },
    )
    .unwrap();
}
