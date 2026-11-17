//! Event-driven loop, input batching and window-level inputs.
use super::*;

fn complete_page(worker: &mut Worker, output: &Mailbox, rows: usize) {
    let source: String = (0..rows)
        .map(|row| format!("line {row}: {}\n", "word ".repeat(row % 7 + 1)))
        .collect();
    let mut decoded = decoded_page(&source);
    decoded.source.offset = 0;
    decoded.has_before = false;
    decoded.has_after = false;
    decoded.anchors = source
        .char_indices()
        .map(|(at, _)| viewer::Anchor {
            text_offset: at as u32,
            source_offset: at as u64,
        })
        .chain(std::iter::once(viewer::Anchor {
            text_offset: source.len() as u32,
            source_offset: source.len() as u64,
        }))
        .collect();
    worker.length = source.len() as u64;
    assert!(worker.set_page(Page::text(decoded, &worker.appearance), output));
}

fn frame(worker: &mut Worker, output: &Mailbox) -> ViewerFrame {
    worker.paint(output).unwrap();
    output
        .take()
        .into_iter()
        .find_map(|event| match event {
            ViewerOutput::Frame(frame) => Some(*frame),
            _ => None,
        })
        .expect("frame")
}

#[test]
fn idle_worker_blocks_and_retries_only_work_without_a_wake_signal() {
    let (mut worker, _) = worker();
    assert_eq!(worker.retry_delay(), None, "an idle viewer does not poll");
    worker.pending_open = true;
    assert_eq!(
        worker.retry_delay(),
        None,
        "file identities, sessions and settings wake the worker"
    );
    worker.dirty = true;
    assert_eq!(
        worker.retry_delay(),
        Some(RETRY),
        "a deferred paint is retried"
    );
    worker.dirty = false;
    worker.web_backlog = true;
    assert_eq!(worker.retry_delay(), Some(Duration::ZERO));
    worker.web_backlog = false;
    worker.orphan_failed("fatal plugin output".into());
    assert_eq!(
        worker.retry_delay(),
        None,
        "without a live session nothing is watched"
    );
}

#[test]
fn consecutive_resizes_in_one_batch_lay_out_once_with_the_final_geometry() {
    let (mut worker, output) = worker();
    prefer_wrap(&mut worker, &output, true);
    assert!(worker.set_page(text_page("wrapped words ".repeat(200)), &output));
    let (sender, receiver) = std::sync::mpsc::sync_channel(4);
    let wakes = crate::input_queue::Wakes::new(sender);
    assert!(wakes.wake());
    let resize = |width| {
        Message::Input(ViewerInput::Resize(ViewerGeometry {
            width,
            height: 100,
            scale: 1.0,
        }))
    };
    let generation = worker.positions.generation;
    let closed = AtomicBool::new(false);
    worker.input_batch(
        vec![resize(200), Message::Ready, resize(180), resize(160)],
        &closed,
        &wakes,
        &output,
    );
    assert_eq!(
        worker.positions.generation,
        generation + 1,
        "one layout context for the whole run of resizes"
    );
    assert_eq!(worker.geometry.width, 160);
    assert!(worker.shaped.as_ref().unwrap().width() <= 161.0);
    assert!(wakes.wake(), "Ready in the batch re-arms backend wakes");
    assert_eq!(receiver.try_iter().count(), 2);
    worker.input_batch(
        vec![
            resize(150),
            Message::Input(ViewerInput::Pan(0.0)),
            resize(140),
        ],
        &closed,
        &wakes,
        &output,
    );
    assert_eq!(
        worker.positions.generation,
        generation + 4,
        "another input between resizes keeps its own layout"
    );
    assert_eq!(worker.geometry.width, 140);
}

#[test]
fn scrollbar_seek_within_the_page_ends_a_cancelled_search_status() {
    let (mut worker, output) = worker();
    complete_page(&mut worker, &output, 80);
    frame(&mut worker, &output);
    worker.search_request = Some(7);
    worker.status = SEARCHING.into();
    worker.input(
        ViewerInput::Position {
            generation: worker.positions.generation,
            sequence: 1,
            horizontal: false,
            position: 0.5,
        },
        &output,
    );
    assert!(worker.search_request.is_none());
    assert!(
        worker.viewport_request.is_none(),
        "served from the loaded page"
    );
    let shown = frame(&mut worker, &output);
    assert_eq!(shown.position_sequence[0], 1);
    assert!(!shown.status.contains(SEARCHING), "{}", shown.status);
}

#[test]
fn retained_raster_choice_survives_an_image_for_the_next_text_file() {
    let (mut worker, output) = worker();
    worker.image = Some(crate::image_view::ImageView::new(
        crate::image_worker::ImageWorker::opened_for_test(
            vec![midnight_image_viewer_host::Metadata {
                width: 400,
                height: 600,
            }],
            false,
        ),
        worker.image_ids.clone(),
    ));
    worker.poll_image(&output);
    worker.input(ViewerInput::RetainedRaster(true), &output);
    assert!(worker.raster.enabled);
    worker.input(ViewerInput::RetainedRaster(false), &output);
    assert!(!worker.raster.enabled);
}

#[test]
fn failure_of_a_cancelled_request_is_reported_when_its_session_ends() {
    let (mut worker, output) = worker();
    worker.event(
        9,
        EventKind::Failed(messages::Failure {
            code: midnight_plugin_host::api::abi::INVALID,
            message: "Malformed plugin output".into(),
        }),
        &output,
    );
    assert!(
        output.take_open_result().is_none() && worker.status.is_empty(),
        "an unrelated late failure is not shown while its session continues"
    );
    worker.report_session_end(false, &output);
    assert!(worker.status.is_empty());
    worker.report_session_end(true, &output);
    assert_eq!(worker.status, "Malformed plugin output");
    assert_eq!(
        output.take_open_result(),
        Some(Err("Malformed plugin output".into()))
    );
    worker.event(
        10,
        EventKind::Failed(messages::Failure {
            code: midnight_plugin_host::api::abi::UNSUPPORTED,
            message: "optional request rejected".into(),
        }),
        &output,
    );
    assert!(
        worker.orphan_failure.is_none(),
        "plugin-host reports fatal failures as INVALID only"
    );
}

#[test]
fn source_gutter_shapes_only_numbers_near_the_painted_rows() {
    let (mut worker, output) = worker();
    worker.mode = viewer::MODE_SOURCE;
    complete_page(&mut worker, &output, 400);
    frame(&mut worker, &output);
    let shaped = |worker: &Worker| {
        let lines = worker.text.gutter.as_ref().unwrap();
        (
            lines.len(),
            lines.iter().filter(|line| line.label.is_some()).count(),
        )
    };
    let (lines, labels) = shaped(&worker);
    assert!(lines >= 400);
    assert!(labels > 0 && labels < 20, "{labels} numbers shaped");
    let bottom = worker.shaped.as_ref().unwrap().height() - worker.geometry.height as f32;
    worker.input(ViewerInput::Scroll { x: 0.0, y: bottom }, &output);
    frame(&mut worker, &output);
    let last = worker.text.gutter.as_ref().unwrap().last().unwrap();
    assert!(last.label.is_some(), "rows scrolled into view are numbered");
    assert!(shaped(&worker).1 < 40);
}

#[test]
fn select_page_selects_the_visible_rows_without_retained_pixels() {
    let (mut worker, output) = worker();
    complete_page(&mut worker, &output, 80);
    worker.origin[1] = 200.0;
    let revision = frame(&mut worker, &output).revision;
    worker.input(ViewerInput::SelectPage { frame: revision }, &output);
    let (first, last) = worker.selection.unwrap();
    let page = worker.shaped.as_ref().unwrap();
    assert_eq!(
        (first, last),
        visible_rows(page, worker.origin[1], worker.geometry)
    );
    assert!(first > 0 && last < page.text().len(), "{first}..{last}");
}
