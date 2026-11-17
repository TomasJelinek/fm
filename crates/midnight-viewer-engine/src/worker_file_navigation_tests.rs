use super::*;

const ORDER: [&str; 3] = ["third.rs", "first.md", "second.txt"];

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
    worker.open(&output);
    assert!(
        worker.pending_open,
        "a repeated wait does not request again"
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
