use super::*;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::cell::Cell;

struct TestPlatform;

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

type SentControls = Rc<RefCell<Vec<MediaControl>>>;

fn window() -> (ViewerWindow, MediaControls, SentControls, Rc<Cell<bool>>) {
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    let window = ViewerWindow::new().unwrap();
    let sent = SentControls::default();
    let accepts = Rc::new(Cell::new(true));
    let binding = MediaControls::install(
        &window,
        Rc::new({
            let sent = sent.clone();
            let accepts = accepts.clone();
            move |control| {
                sent.borrow_mut().push(control);
                accepts.get()
            }
        }),
    );
    (window, binding, sent, accepts)
}

fn snapshot(
    session: MediaSessionId,
    control: u64,
    revision: u64,
    seconds: u64,
    playing: bool,
) -> MediaSnapshot {
    MediaSnapshot {
        session,
        control: MediaControlId(control),
        revision,
        position: Duration::from_secs(seconds),
        duration: Some(Duration::from_secs(120)),
        seekable: true,
        playing,
        phase: if playing {
            MediaPhase::Playing
        } else {
            MediaPhase::Paused
        },
    }
}

#[test]
fn frame_fence_tracks_accepted_seeks_without_invalidating_play_pause_previews() {
    let (window, media, _, accepts) = window();
    let session = MediaSessionId(7);
    assert!(media.begin(session));
    assert!(media.apply(snapshot(session, 0, 1, 30, false)));
    assert!(media.accepts_frame(session, Some(MediaControlId(0))));
    assert!(!media.accepts_frame(session, None));
    assert!(!media.accepts_frame(MediaSessionId(6), Some(MediaControlId(0))));
    assert!(!media.accepts_frame(session, Some(MediaControlId(1))));

    window.invoke_media_toggle("7".into()); // Play 1.
    window.invoke_media_toggle("7".into()); // Pause 2.
    assert!(media.accepts_frame(session, Some(MediaControlId(0))));
    window.invoke_media_seek("7".into(), 0.75); // Seek 3.
    window.invoke_media_seek("7".into(), 0.5); // Seek 4 supersedes 3.
    assert!(!media.accepts_frame(session, Some(MediaControlId(3))));
    assert!(media.accepts_frame(session, Some(MediaControlId(4))));
    window.invoke_media_toggle("7".into()); // Play 5.
    window.invoke_media_toggle("7".into()); // Pause 6.
    for control in 4..=6 {
        assert!(media.accepts_frame(session, Some(MediaControlId(control))));
    }
    accepts.set(false);
    window.invoke_media_seek("7".into(), 0.9); // Refused 7 cannot move the floor.
    assert!(media.accepts_frame(session, Some(MediaControlId(4))));
    assert!(!media.accepts_frame(session, Some(MediaControlId(7))));
    assert!(media.retire(session));
    assert!(!media.accepts_frame(session, Some(MediaControlId(4))));
    assert!(media.begin(MediaSessionId(8)));
    assert!(!media.accepts_frame(session, Some(MediaControlId(4))));
    assert!(media.accepts_frame(MediaSessionId(8), Some(MediaControlId(0))));
}

#[test]
fn replay_fences_old_pixels_but_an_ended_snapshot_cannot_supersede_a_pending_seek() {
    let (window, media, _, _) = window();
    let session = MediaSessionId(7);
    assert!(media.begin(session));
    let mut ended = snapshot(session, 0, 1, 120, false);
    ended.phase = MediaPhase::Ended;
    assert!(media.apply(ended.clone()));
    window.invoke_media_toggle("7".into()); // Replay 1 starts at zero.
    assert!(!media.accepts_frame(session, Some(MediaControlId(0))));
    assert!(media.accepts_frame(session, Some(MediaControlId(1))));
    window.invoke_media_toggle("7".into()); // Pause 2 retains the replay preview.
    assert!(media.accepts_frame(session, Some(MediaControlId(1))));

    ended.control = MediaControlId(2);
    ended.revision = 2;
    assert!(media.apply(ended));
    window.invoke_media_seek("7".into(), 0.25); // Seek 3 owns the new preview.
    window.invoke_media_toggle("7".into()); // Play 4 must not infer another replay.
    window.invoke_media_toggle("7".into()); // Pause 5; no further frame need arrive.
    assert!(media.accepts_frame(session, Some(MediaControlId(3))));
    assert!(!media.accepts_frame(session, Some(MediaControlId(2))));
    assert!(!media.accepts_frame(session, Some(MediaControlId(6))));
}

#[test]
fn video_controls_dispatch_play_pause_and_replace_seek_through_the_window() {
    let (window, media, sent, _) = window();
    let session = MediaSessionId(7);
    assert!(media.begin(session));
    let opening = window.get_media_controls();
    assert_eq!(opening.session.as_str(), "7");
    assert_eq!(opening.status.as_str(), "Opening video…");
    assert!(!opening.can_play_pause && !opening.can_seek);
    window.invoke_media_toggle("7".into());
    assert!(sent.borrow().is_empty());

    assert!(media.apply(snapshot(session, 0, 1, 30, false)));
    let ready = window.get_media_controls();
    assert_eq!(ready.time.as_str(), "0:30 / 2:00");
    assert_eq!(ready.position, 0.25);
    assert!(ready.can_play_pause && ready.can_seek && !ready.playing);
    window.invoke_media_seek("7".into(), f32::NAN);
    window.invoke_media_seek("7".into(), 1.01);
    assert!(sent.borrow().is_empty());
    window.invoke_media_toggle("7".into());
    assert_eq!(
        sent.borrow()[0],
        MediaControl {
            session,
            id: MediaControlId(1),
            action: MediaAction::SetPlaying(true)
        },
    );
    assert!(media.apply(snapshot(session, 1, 2, 31, true)));
    assert!(window.get_media_controls().playing);

    window.invoke_media_seek("7".into(), 0.75);
    window.invoke_media_seek("7".into(), 0.5);
    assert_eq!(
        sent.borrow()[1].action,
        MediaAction::Seek(Duration::from_secs(90))
    );
    assert_eq!(
        sent.borrow()[2].action,
        MediaAction::Seek(Duration::from_secs(60))
    );
    window.invoke_media_toggle("7".into());
    assert_eq!(
        sent.borrow()[3],
        MediaControl {
            session,
            id: MediaControlId(4),
            action: MediaAction::SetPlaying(false)
        },
    );
    let seeking = window.get_media_controls();
    assert_eq!(seeking.status.as_str(), "Seeking…");
    assert_eq!(seeking.position, 0.5);
    assert!(!seeking.playing);
    assert!(!media.apply(snapshot(session, 2, 3, 90, true)));
    assert!(!media.apply(snapshot(session, 3, 4, 60, true)));
    assert!(media.apply(snapshot(session, 4, 5, 60, false)));
    assert_eq!(window.get_media_controls().time.as_str(), "1:00 / 2:00");
    assert!(!media.apply(snapshot(session, 4, 4, 88, true)));
    assert_eq!(window.get_media_controls().position, 0.5);
    assert!(!window.get_media_controls().playing);
}

#[test]
fn refused_input_and_obsolete_sessions_cannot_replace_current_video_state() {
    let (window, media, sent, accepts) = window();
    let first = MediaSessionId(20);
    let next = MediaSessionId(21);
    assert!(media.begin(first));
    assert!(media.apply(snapshot(first, 0, 1, 10, false)));
    accepts.set(false);
    window.invoke_media_seek("20".into(), 0.5);
    assert_eq!(sent.borrow()[0].id, MediaControlId(1));
    let refusal = window.get_media_controls();
    assert!(!refusal.error.is_empty());
    assert_eq!(refusal.time.as_str(), "0:10 / 2:00");
    assert!(media.apply(snapshot(first, 0, 2, 11, false)));
    assert!(!window.get_media_controls().error.is_empty());

    accepts.set(true);
    window.invoke_media_seek("20".into(), 0.25);
    assert_eq!(sent.borrow()[1].id, MediaControlId(1));
    assert!(media.begin(next));
    let mut late = snapshot(first, 1, 3, 30, false);
    late.phase = MediaPhase::Failed("old device error".into());
    assert!(!media.apply(late));
    assert!(!media.retire(first));
    assert!(!media.begin(first));
    assert_eq!(window.get_media_controls().session.as_str(), "21");
    assert!(window.get_media_controls().error.is_empty());

    let mut live = snapshot(next, 0, 1, 20, false);
    live.duration = None;
    assert!(media.apply(live));
    assert!(!window.get_media_controls().can_seek);
    window.invoke_media_seek("20".into(), 0.5);
    window.invoke_media_seek("21".into(), f32::NAN);
    assert_eq!(sent.borrow().len(), 2);
    let mut failed = snapshot(next, 0, 2, 20, false);
    failed.phase = MediaPhase::Failed("Audio device unavailable".into());
    assert!(media.apply(failed));
    assert_eq!(
        window.get_media_controls().error.as_str(),
        "Audio device unavailable"
    );
    assert!(!window.get_media_controls().can_play_pause);
    assert!(media.retire(next));
    assert!(window.get_media_controls().session.is_empty());
}

#[test]
fn active_video_preserves_viewer_file_commands_and_leaves_canvas_scroll_local() {
    use midnight_core::ViewerInput;

    let (_runtime, _windows, open, _service) = crate::image_presentation_tests::open();
    open.timer.stop();
    let inputs = Rc::new(RefCell::new(Vec::new()));
    let capture = inputs.clone();
    *open.input.borrow_mut() = Rc::new(move |input| {
        capture.borrow_mut().push(input);
        true
    });
    let media = MediaControls::install(&open.window, Rc::new(|_| false));
    assert!(media.begin(MediaSessionId(1)));
    assert!(!open.window.invoke_scroll(1., 12.));
    assert!(!crate::dispatch_key(&open, "viewer.find"));
    assert!(crate::dispatch_key(&open, "viewer.next-file"));
    assert!(crate::dispatch_key(&open, "viewer.previous-file"));
    assert!(matches!(
        inputs.borrow().as_slice(),
        [ViewerInput::NextFile, ViewerInput::PreviousFile],
    ));
}
