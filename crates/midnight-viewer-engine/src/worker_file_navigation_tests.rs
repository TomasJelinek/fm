use super::*;

const ORDER: [&str; 3] = ["third.rs", "first.md", "second.txt"];

#[test]
fn failed_replacement_capture_retires_video_before_any_new_layout() {
    let (mut worker, output) = worker();
    captured_order(&mut worker, &["first.mp4", "second.txt"]);
    worker.mode = VIEWER_MODE_VIDEO;
    worker.video = Some(crate::video_actor::VideoView::finished_for_test(
        MediaSessionId(7),
    ));
    worker.input(ViewerInput::NextFile, &output);
    assert_eq!(worker.current, 1);
    assert!(worker.video.is_none());
    let events = output.take();
    assert!(events.iter().any(|event| matches!(
        event, ViewerOutput::MediaRetired { session } if *session == MediaSessionId(7)
    )));
    assert!(
        events
            .iter()
            .all(|event| !matches!(event, ViewerOutput::LayoutSelected { .. }))
    );
    assert_eq!(
        serve_capture(&mut worker, &output, Err("Access denied".into())),
        1
    );
    let events = output.take();
    assert!(events.iter().all(|event| !matches!(
        event,
        ViewerOutput::LayoutSelected { .. } | ViewerOutput::MediaRetired { .. }
    )));
    assert!(events.iter().any(|event| matches!(
        event, ViewerOutput::Status(message) if message == "Access denied"
    )));
}

#[test]
fn navigation_without_an_admitted_neighbor_does_not_retire_video() {
    let (mut worker, output) = worker();
    worker.mode = VIEWER_MODE_VIDEO;
    worker.video = Some(crate::video_actor::VideoView::finished_for_test(
        MediaSessionId(7),
    ));
    assert!(worker.files.is_empty());
    for input in [ViewerInput::NextFile, ViewerInput::PreviousFile] {
        worker.input(input, &output);
        assert_eq!(
            worker.video.as_ref().unwrap().session_id(),
            MediaSessionId(7)
        );
        assert!(output.take().is_empty());
    }
}

#[test]
fn previous_and_next_file_wrap_in_captured_order_and_reset_mode() {
    let (mut worker, output) = worker();
    captured_order(&mut worker, &ORDER);
    for (input, expected, name) in [
        (ViewerInput::PreviousFile, 2, "second.txt"),
        (ViewerInput::PreviousFile, 1, "first.md"),
        (ViewerInput::NextFile, 2, "second.txt"),
        (ViewerInput::NextFile, 0, "third.rs"),
    ] {
        worker.selected_mode = Some(viewer::MODE_BINARY);
        worker.input(input, &output);
        assert_eq!(worker.current, expected);
        assert_eq!(worker.selected_mode, None);
        assert_eq!(
            serve_capture(
                &mut worker,
                &output,
                Err(format!("captured identity: {name}"))
            ),
            expected,
            "identities are requested by order position, never from a displayed caption"
        );
        assert!(worker.caption.starts_with(name));
        assert_eq!(worker.status, format!("captured identity: {name}"));
    }
    worker.files = Arc::from(&worker.files[..1]);
    worker.input(ViewerInput::PreviousFile, &output);
    assert_eq!(worker.current, 0, "one captured file wraps to itself");
    assert!(
        !worker.pending_open,
        "the held identity reopens without another capture"
    );
}

#[test]
fn previous_file_waits_for_capture_without_repainting_old_content() {
    let (mut worker, output) = worker();
    captured_order(&mut worker, &ORDER);
    worker.page = Some(text_page("old visible content".into()));
    worker.input(ViewerInput::PreviousFile, &output);
    assert_eq!(worker.current, 2);
    assert!(worker.pending_open);
    assert!(worker.page.is_none());
    assert_eq!(worker.status, "Preparing file identity…");
    let requested = worker.capture_request;
    assert!(requested.is_some());
    worker.open(&output);
    assert!(worker.pending_open);
    assert_eq!(
        worker.capture_request, requested,
        "a repeated wait keeps the in-flight request instead of superseding it"
    );
    serve_capture(&mut worker, &output, Err("retained capture failure".into()));
    assert!(!worker.pending_open);
    assert_eq!(worker.status, "retained capture failure");
    assert!(worker.caption.starts_with("second.txt"));
}

#[test]
fn navigation_supersedes_an_unserved_identity_request() {
    let (mut worker, output) = worker();
    captured_order(&mut worker, &ORDER);
    worker.input(ViewerInput::NextFile, &output);
    worker.input(ViewerInput::NextFile, &output);
    assert_eq!(worker.current, 2);
    assert_eq!(
        serve_capture(&mut worker, &output, Err("latest".into())),
        2,
        "only the latest target is captured"
    );
    assert_eq!(worker.status, "latest");
}

#[test]
fn previous_file_leaves_loading_and_ready_image_or_pdf() {
    for document in [false, true] {
        for ready in [false, true] {
            let (mut worker, output) = worker();
            captured_order(&mut worker, &ORDER);
            worker.current = 1;
            worker.image = Some(crate::image_view::ImageView::new(
                crate::image_worker::ImageWorker::opened_for_test(
                    vec![midnight_image_viewer_host::Metadata {
                        width: 400,
                        height: 600,
                    }],
                    document,
                ),
                worker.image_ids.clone(),
            ));
            if ready {
                worker.poll_image(&output);
            }
            worker.input(ViewerInput::PreviousFile, &output);
            assert_eq!(worker.current, 0, "document={document}, ready={ready}");
            assert!(worker.image.is_none());
            serve_capture(
                &mut worker,
                &output,
                Err("captured identity: third.rs".into()),
            );
            assert_eq!(worker.status, "captured identity: third.rs");
        }
    }
}

#[test]
fn identity_capture_overlaps_the_teardown_of_the_previous_image() {
    for capture_first in [true, false] {
        let (mut worker, output) = worker();
        captured_order(&mut worker, &ORDER);
        // Declared after the worker, so a failed assertion releases the thread before the
        // worker joins it.
        let (release, teardown) = std::sync::mpsc::channel::<()>();
        let (_, events) = std::sync::mpsc::sync_channel(1);
        let teardown: Box<dyn FnOnce() + Send> = Box::new(move || {
            let _ = teardown.recv();
        });
        worker.image = Some(crate::image_view::ImageView::new(
            crate::image_worker::ImageWorker::for_test(events, Some(teardown)),
            worker.image_ids.clone(),
        ));
        worker.input(ViewerInput::NextFile, &output);
        assert_eq!(worker.current, 1);
        assert!(worker.image.is_none() && worker.pending_open);
        assert_eq!(worker.status, "Closing previous image…");
        assert_eq!(
            worker.prefetch.as_ref().unwrap().wanted(),
            Some(1),
            "the identity is requested while the previous image closes"
        );
        let serve = |worker: &mut Worker| {
            let prefetch = worker.prefetch.as_ref().unwrap();
            prefetch.serve(Err("captured identity: first.md".into()));
            worker.resume_open(&output);
        };
        let finish_teardown = |worker: &mut Worker| {
            release.send(()).unwrap();
            let deadline = std::time::Instant::now() + Duration::from_secs(10);
            while !worker.closings_finished() {
                assert!(std::time::Instant::now() < deadline, "teardown finishes");
                std::thread::yield_now();
            }
            worker.resume_open(&output);
        };
        if capture_first {
            serve(&mut worker);
            assert!(
                worker.capture_ready() && worker.pending_open,
                "waits for teardown"
            );
            assert_eq!(worker.status, "Closing previous image…");
            finish_teardown(&mut worker);
        } else {
            finish_teardown(&mut worker);
            assert!(worker.pending_open, "waits for the identity");
            assert_eq!(worker.status, "Preparing file identity…");
            serve(&mut worker);
        }
        assert!(!worker.pending_open, "capture_first={capture_first}");
        assert_eq!(worker.status, "captured identity: first.md");
        assert!(worker.caption.starts_with("first.md"));
    }
}
