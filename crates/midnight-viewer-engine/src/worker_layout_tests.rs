use super::*;

fn prepare_html(worker: &mut Worker) -> (tempfile::TempDir, super::super::layout::PreparedOpening) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("owned.html");
    std::fs::write(&path, b"<!doctype html><title>Owned</title>").unwrap();
    let file = midnight_plugin_host::CapturedFile::capture(&path).unwrap();
    worker.layout = crate::layout_handshake::Handshake::new(true);
    worker.positions.reset().unwrap();
    worker.mode = VIEWER_MODE_HTML;
    worker.modes = vec![VIEWER_MODE_HTML];
    (
        directory,
        super::super::layout::PreparedOpening {
            file,
            route: crate::routing::Route::Process { index: 0 },
            image: None,
            video: None,
            video_first: midnight_media_host::video_installation::VideoWorker::Bundled,
        },
    )
}

fn selection(output: &Mailbox) -> (u64, u16, u64) {
    output
        .take()
        .into_iter()
        .find_map(|output| match output {
            ViewerOutput::LayoutSelected {
                token,
                mode,
                content_generation,
            } => Some((token, mode, content_generation)),
            _ => None,
        })
        .expect("backend-selected layout must be published")
}

#[test]
fn html_selection_retains_source_without_opening_any_transport() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, opening) = prepare_html(&mut worker);
    let expected_path = opening.file.path().to_owned();
    worker.select_opening(opening, &output).unwrap();
    let (token, mode, generation) = selection(&output);
    assert_ne!(token, 0);
    assert_eq!(mode, VIEWER_MODE_HTML);
    assert_eq!(generation, worker.positions.generation);
    assert!(worker.web.is_none());
    assert!(worker.image.is_none());
    assert!(worker.session.is_none());
    assert_eq!(
        worker.pending_layout.as_ref().unwrap().file.path(),
        expected_path.as_path()
    );
    assert!(output.take_open_result().is_none());
}

#[test]
fn resize_while_waiting_keeps_the_token_and_content_generation() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, opening) = prepare_html(&mut worker);
    worker.select_opening(opening, &output).unwrap();
    let (token, _, generation) = selection(&output);
    for width in [1000, 2952, 1600] {
        worker.input(
            ViewerInput::Resize(ViewerGeometry {
                width,
                height: 1611,
                scale: 2.25,
            }),
            &output,
        );
        assert_eq!(worker.layout.selection().unwrap().token, token);
        assert_eq!(worker.positions.generation, generation);
        assert!(worker.web.is_none());
    }
    assert!(output.take().is_empty());
}

#[test]
fn stale_acknowledgement_cannot_launch_the_pending_browser() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, opening) = prepare_html(&mut worker);
    worker.select_opening(opening, &output).unwrap();
    let (token, _, _) = selection(&output);
    worker.input(
        ViewerInput::LayoutReady {
            token: token + 1,
            geometry: worker.geometry,
        },
        &output,
    );
    assert!(worker.web.is_none());
    assert!(worker.pending_layout.is_some());
    assert_eq!(worker.layout.selection().unwrap().token, token);
}

#[test]
fn matching_acknowledgement_supplies_the_first_bounded_and_raw_viewports() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, opening) = prepare_html(&mut worker);
    worker.select_opening(opening, &output).unwrap();
    let (token, _, generation) = selection(&output);
    let geometry = ViewerGeometry {
        width: 2952,
        height: 1611,
        scale: 2.25,
    };
    let ready = worker.take_layout_ready(token, geometry).unwrap();
    assert!(matches!(
        ready.route,
        crate::routing::Route::Process { index: 0 }
    ));
    assert_eq!(
        worker.image_size,
        [2952, 1611],
        "Fit retains the raw viewport"
    );
    assert_eq!(
        [worker.geometry.width, worker.geometry.height],
        [2772, 1512]
    );
    assert!((worker.geometry.scale - 2.1130342).abs() < 0.000001);
    assert_eq!(worker.positions.generation, generation);
    assert!(!worker.layout.waiting());
    assert!(worker.pending_layout.is_none());
    assert!(worker.take_layout_ready(token, geometry).is_none());
}

#[test]
fn unchanged_geometry_is_still_a_valid_initial_acknowledgement() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, opening) = prepare_html(&mut worker);
    worker.select_opening(opening, &output).unwrap();
    let (token, _, _) = selection(&output);
    assert!(worker.take_layout_ready(token, worker.geometry).is_some());
}

#[test]
fn invalid_geometry_does_not_consume_the_opening_token() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, opening) = prepare_html(&mut worker);
    worker.select_opening(opening, &output).unwrap();
    let (token, _, _) = selection(&output);
    assert!(
        worker
            .take_layout_ready(
                token,
                ViewerGeometry {
                    width: 0,
                    height: 200,
                    scale: f32::NAN,
                },
            )
            .is_none()
    );
    assert!(worker.layout.waiting());
    assert!(worker.pending_layout.is_some());
}

#[test]
fn close_invalidates_the_opening_before_any_later_acknowledgement() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, opening) = prepare_html(&mut worker);
    worker.select_opening(opening, &output).unwrap();
    let (token, _, _) = selection(&output);
    worker.input(ViewerInput::Close, &output);
    worker.input(
        ViewerInput::LayoutReady {
            token,
            geometry: worker.geometry,
        },
        &output,
    );
    assert!(worker.layout.closed());
    assert!(worker.pending_layout.is_none());
    assert!(worker.web.is_none());
}

#[test]
fn file_cycle_invalidates_pending_html_and_preserves_the_next_startup_error() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, opening) = prepare_html(&mut worker);
    worker.select_opening(opening, &output).unwrap();
    let (token, _, _) = selection(&output);
    captured_order(&mut worker, &["first.html", "second.txt"]);
    worker.input(ViewerInput::NextFile, &output);
    assert_eq!(worker.current, 1);
    assert!(worker.pending_layout.is_none());
    assert!(output.take_open_result().is_none());
    serve_capture(&mut worker, &output, Err("retained failure".into()));
    assert_eq!(
        output.take_open_result(),
        Some(Err("retained failure".into()))
    );
    assert!(worker.take_layout_ready(token, worker.geometry).is_none());
}

#[test]
fn capture_wait_retries_do_not_publish_or_replace_layout_tokens() {
    let (mut worker, _) = worker();
    worker.layout = crate::layout_handshake::Handshake::new(true);
    captured_order(&mut worker, &["pending.html"]);
    let output = Mailbox::with_layout();
    for _ in 0..3 {
        worker.open(&output);
        assert!(worker.pending_open);
        assert!(!worker.layout.waiting());
        assert!(worker.pending_layout.is_none());
        assert!(
            output
                .take()
                .iter()
                .all(|item| !matches!(item, ViewerOutput::LayoutSelected { .. }))
        );
    }
    assert!(output.take_open_result().is_none());
}

#[test]
fn matching_acknowledgement_reaches_startup_errors_without_completing_early() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, opening) = prepare_html(&mut worker);
    worker.select_opening(opening, &output).unwrap();
    let (token, _, _) = selection(&output);
    assert!(output.take_open_result().is_none());
    worker.input(
        ViewerInput::LayoutReady {
            token,
            geometry: worker.geometry,
        },
        &output,
    );
    assert_eq!(
        output.take_open_result(),
        Some(Err("Selected HTML viewer is no longer available.".into()))
    );
    assert!(worker.pending_layout.is_none());
    assert!(!worker.layout.waiting());
    assert!(worker.web.is_none());
}

#[test]
fn supported_native_mode_change_invalidates_an_unacknowledged_opening() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, mut opening) = prepare_html(&mut worker);
    worker.mode = viewer::MODE_TEXT;
    worker.modes = vec![viewer::MODE_TEXT, viewer::MODE_BINARY];
    opening.route = crate::routing::Route::Native {
        index: 0,
        mode: viewer::MODE_TEXT,
        detect_binary: false,
    };
    worker.files = Arc::from([opening.file.path().to_owned()]);
    worker.captured = Some((0, Err("retained failure".into())));
    worker.select_opening(opening, &output).unwrap();
    let (token, _, _) = selection(&output);
    worker.input(ViewerInput::Mode(viewer::MODE_BINARY), &output);
    assert_eq!(worker.selected_mode, Some(viewer::MODE_BINARY));
    assert!(worker.pending_layout.is_none());
    assert!(worker.take_layout_ready(token, worker.geometry).is_none());
    assert_eq!(
        output.take_open_result(),
        Some(Err("retained failure".into()))
    );
}

#[test]
fn video_layout_waits_for_matching_canvas_before_admitting_the_capture() {
    let (mut worker, _) = worker();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("owned.mp4");
    std::fs::write(&path, b"captured layout fixture").unwrap();
    let file = midnight_plugin_host::CapturedFile::capture(&path).unwrap();
    worker.layout = crate::layout_handshake::Handshake::new(true);
    worker.positions.reset().unwrap();
    worker.mode = VIEWER_MODE_VIDEO;
    worker.modes = vec![VIEWER_MODE_VIDEO];
    let output = Mailbox::with_layout();
    worker
        .select_opening(
            super::super::layout::PreparedOpening {
                file,
                route: crate::routing::Route::Process { index: 0 },
                image: None,
                video: None,
                video_first: midnight_media_host::video_installation::VideoWorker::Bundled,
            },
            &output,
        )
        .unwrap();
    let (token, mode, generation) = selection(&output);
    assert_eq!(mode, VIEWER_MODE_VIDEO);
    assert_eq!(generation, worker.positions.generation);
    assert!(worker.video.is_none());
    assert!(
        worker
            .take_layout_ready(token + 1, worker.geometry)
            .is_none()
    );
    assert!(output.take_open_result().is_none());

    let opening = worker.take_layout_ready(token, worker.geometry).unwrap();
    assert_eq!(opening.file.path(), path);
    assert!(worker.video.is_none());
}

#[test]
fn video_controls_and_frames_share_capture_generation() {
    let (mut worker, output) = worker();
    worker.positions.reset().unwrap();
    worker.mode = VIEWER_MODE_VIDEO;
    let generation = worker.positions.generation;
    let mut snapshot = crate::video_actor::opening(MediaSessionId(generation));
    snapshot.playing = true;
    snapshot.control = MediaControlId(73);
    snapshot.position = Duration::from_micros(500);
    snapshot.duration = Some(Duration::from_micros(1000));
    snapshot.phase = MediaPhase::Playing;
    worker
        .publish_video(
            crate::video_actor::VideoUpdate {
                frame: Some(midnight_media_host::video_transport::VideoFrame {
                    presentation: Duration::from_micros(500),
                    width: 1,
                    height: 1,
                    rgba: Box::new([255, 0, 128, 128]),
                }),
                snapshot,
                notice: None,
            },
            &output,
        )
        .unwrap();
    let events = output.take();
    assert!(events.iter().any(|event| matches!(
        event,
        ViewerOutput::MediaSnapshot(snapshot)
            if snapshot.session.0 == generation
                && snapshot.playing
                && snapshot.position == Duration::from_micros(500)
    )));
    let frame = events
        .iter()
        .find_map(|event| match event {
            ViewerOutput::Frame(frame) => Some(frame),
            _ => None,
        })
        .unwrap();
    assert_eq!(frame.mode, VIEWER_MODE_VIDEO);
    assert_eq!(frame.media_control, Some(MediaControlId(73)));
    assert_eq!(frame.generation, generation);
    assert_eq!(&*frame.pixels, &[128, 0, 64, 128]);
    assert_eq!(output.take_open_result(), Some(Ok(())));
}

fn prepare_image(
    worker: &mut Worker,
) -> (tempfile::TempDir, super::super::layout::PreparedOpening) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("owned.png");
    std::fs::write(&path, b"captured layout fixture").unwrap();
    let file = midnight_plugin_host::CapturedFile::capture(&path).unwrap();
    worker.images = Arc::from(
        [midnight_image_viewer_host::Provider::unavailable_for_tests(
            "test-image",
            &["png"],
        )],
    );
    worker.layout = crate::layout_handshake::Handshake::new(true);
    worker.positions.reset().unwrap();
    worker.mode = VIEWER_MODE_IMAGE;
    worker.modes = vec![VIEWER_MODE_IMAGE];
    (
        directory,
        super::super::layout::PreparedOpening {
            file,
            route: crate::routing::Route::Process { index: 0 },
            image: None,
            video: None,
            video_first: midnight_media_host::video_installation::VideoWorker::Bundled,
        },
    )
}

#[test]
fn image_provider_starts_while_the_canvas_is_negotiated_and_is_adopted_on_acknowledgement() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, opening) = prepare_image(&mut worker);
    worker.select_opening(opening, &output).unwrap();
    let (token, mode, _) = selection(&output);
    assert_eq!(mode, VIEWER_MODE_IMAGE);
    assert!(
        worker
            .pending_layout
            .as_ref()
            .is_some_and(|opening| opening.image.is_some()),
        "decoding must not wait for the frontend canvas"
    );
    assert!(
        worker.image.is_none(),
        "no viewport before the acknowledgement"
    );

    let opening = worker.take_layout_ready(token, worker.geometry).unwrap();
    assert!(opening.image.is_some());
    worker.start_opening(opening, &output).unwrap();
    assert!(
        worker.image.is_some(),
        "the started provider is the displayed one"
    );
    assert!(worker.closing_image.is_none());
}

#[test]
fn an_abandoned_negotiation_retires_its_started_image_provider_without_joining_it() {
    let (mut worker, _) = worker();
    let output = Mailbox::with_layout();
    let (_directory, opening) = prepare_image(&mut worker);
    worker.select_opening(opening, &output).unwrap();
    selection(&output);
    worker.input(ViewerInput::Close, &output);
    assert!(worker.pending_layout.is_none());
    assert!(worker.image.is_none());
    assert!(
        worker.closing_image.is_some(),
        "the next opening must wait for the provider like any closing image"
    );
}
