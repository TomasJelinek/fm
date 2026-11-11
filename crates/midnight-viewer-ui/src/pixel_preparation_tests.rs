use super::*;
use midnight_core::{ViewerRasterGutter, ViewerRasterInfo, ViewerSurfaceId};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    mpsc,
};

fn frame(revision: u64) -> ViewerFrame {
    ViewerFrame {
        generation: 1,
        position_sequence: [0; 2],
        revision,
        mode: 1,
        modes: vec![1],
        wrap_enabled: false,
        wrap_available: true,
        image: None,
        raster: None,
        raster_rebase: None,
        width: 4,
        height: 3,
        pixels: Arc::from([31; 48]),
        tiles: None,
        accessible_text: Arc::from("text"),
        position: 0.0,
        vertical_extent: 1.0,
        horizontal_position: 0.0,
        horizontal_extent: 1.0,
        caption: String::new(),
        status: String::new(),
    }
}

fn retained(revision: u64) -> ViewerFrame {
    let mut frame = frame(revision);
    frame.raster = Some(ViewerRasterInfo {
        surface: ViewerSurfaceId(1),
        pixel_id: 1,
        origin: [0.0; 2],
        raster_origin: [0.0; 2],
        viewport: [5, 3],
        coverage_min: [0.0; 2],
        coverage_max: [0.0; 2],
        limit_min: [Some(0.0); 2],
        limit_max: [Some(0.0); 2],
        pan_sequence: 0,
        scroll_per_pixel: [0.0; 2],
        settled: true,
        gutter: Some(ViewerRasterGutter {
            width: 1,
            height: 3,
            pixels: Arc::from([7; 12]),
        }),
    });
    frame
}

#[test]
fn copies_legacy_and_retained_pixels_but_reuses_retained_metadata_without_source_ownership() {
    let mut cache = Cache::default();
    let mut frame = retained(1);
    let source = Arc::downgrade(&frame.pixels);
    let first = cache.prepare(&frame).unwrap().unwrap();
    assert_eq!(first.body.as_bytes(), frame.pixels.as_ref());
    assert_eq!(first.gutter.as_ref().unwrap().as_bytes(), &[7; 12]);
    frame.revision = 2;
    frame.raster.as_mut().unwrap().pan_sequence = 19;
    let second = cache.prepare(&frame).unwrap().unwrap();
    assert_eq!(second.frame_revision, 2);
    assert_eq!(
        first.body.as_bytes().as_ptr(),
        second.body.as_bytes().as_ptr()
    );
    assert_eq!(
        first.gutter.as_ref().unwrap().as_bytes().as_ptr(),
        second.gutter.as_ref().unwrap().as_bytes().as_ptr()
    );
    frame.generation += 1;
    let replaced = cache.prepare(&frame).unwrap().unwrap();
    assert_ne!(
        second.body.as_bytes().as_ptr(),
        replaced.body.as_bytes().as_ptr()
    );
    drop(frame);
    assert_eq!(
        source.strong_count(),
        0,
        "prepared cache must not hold raster-budget source allocations"
    );
    let plain = self::frame(3);
    let prepared = cache.prepare(&plain).unwrap().unwrap();
    assert_eq!(prepared.body.as_bytes(), plain.pixels.as_ref());
    assert!(prepared.gutter.is_none());
}

#[test]
fn rejects_all_invalid_buffer_bounds_before_allocating_and_skips_tiles() {
    let mut invalid = frame(1);
    for (width, height) in [(0, 3), (4, 0), (u32::MAX, u32::MAX), (4097, 4097)] {
        invalid.width = width;
        invalid.height = height;
        assert!(prepare(&invalid).is_err());
    }
    let mut invalid = retained(1);
    invalid
        .raster
        .as_mut()
        .unwrap()
        .gutter
        .as_mut()
        .unwrap()
        .height = 4;
    assert!(prepare(&invalid).is_err());
    invalid
        .raster
        .as_mut()
        .unwrap()
        .gutter
        .as_mut()
        .unwrap()
        .height = 3;
    invalid
        .raster
        .as_mut()
        .unwrap()
        .gutter
        .as_mut()
        .unwrap()
        .width = 5;
    assert!(prepare(&invalid).is_err());
    invalid.raster.as_mut().unwrap().gutter = None;
    invalid.width = 4096;
    invalid.height = 2048;
    assert!(prepare(&invalid).is_err());
    invalid.tiles = Some(Arc::from([]));
    assert!(prepare(&invalid).unwrap().is_none());
}

fn next(preparation: &Preparation, ready: &mpsc::Receiver<()>) -> PreparedBatch {
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    preparation.take().expect("notified prepared batch")
}

#[test]
fn worker_keeps_one_batch_and_preserves_controls_without_redraining_the_source() {
    let pending = Arc::new(Mutex::new(vec![
        ViewerOutput::Status("before".into()),
        ViewerOutput::Frame(Box::new(frame(1))),
        ViewerOutput::RasterRebase {
            surface: ViewerSurfaceId(1),
            transition: 7,
        },
        ViewerOutput::CopyText("copy".into()),
    ]));
    let drains = Arc::new(AtomicUsize::new(0));
    let source = pending.clone();
    let counted = drains.clone();
    let (wake, ready) = mpsc::channel();
    let mut cache = Cache::default();
    let preparation = Preparation::start(
        move || {
            counted.fetch_add(1, Ordering::SeqCst);
            std::mem::take(&mut *lock(&source))
        },
        move |outputs| batch(outputs, &mut cache),
        Arc::new(move || wake.send(()).is_ok()),
    )
    .unwrap();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    *lock(&pending) = vec![ViewerOutput::Closed];
    preparation.shared.signal();
    assert_eq!(
        drains.load(Ordering::SeqCst),
        1,
        "unconsumed batch applies backpressure"
    );
    let first = preparation.take().unwrap();
    assert!(matches!(
        &first.outputs[..],
        [
            ViewerOutput::Status(_),
            ViewerOutput::Frame(_),
            ViewerOutput::RasterRebase { transition: 7, .. },
            ViewerOutput::CopyText(_)
        ]
    ));
    assert_eq!(first.pixels.unwrap().unwrap().body.as_bytes(), &[31; 48]);
    let last = next(&preparation, &ready);
    assert!(matches!(&last.outputs[..], [ViewerOutput::Closed]));
    assert_eq!(drains.load(Ordering::SeqCst), 2);
}

#[test]
fn failed_event_loop_wake_retries_without_waiting_for_another_source_frame() {
    let attempts = Arc::new(AtomicUsize::new(0));
    let counted = attempts.clone();
    let (wake, ready) = mpsc::channel();
    let mut cache = Cache::default();
    let preparation = Preparation::start(
        || vec![ViewerOutput::Closed],
        move |outputs| batch(outputs, &mut cache),
        Arc::new(move || {
            if counted.fetch_add(1, Ordering::SeqCst) == 0 {
                false
            } else {
                wake.send(()).is_ok()
            }
        }),
    )
    .unwrap();
    assert!(matches!(
        &next(&preparation, &ready).outputs[..],
        [ViewerOutput::Closed]
    ));
    assert_eq!(attempts.load(Ordering::SeqCst), 2);
}

#[test]
fn stopping_during_preparation_never_waits_and_reclaims_pending_work_on_the_worker() {
    let caller = std::thread::current().id();
    let (entered, preparing) = mpsc::channel();
    let (release, resume) = mpsc::channel();
    let (finished, retired) = mpsc::channel();
    let wakes = Arc::new(AtomicUsize::new(0));
    let counted = wakes.clone();
    struct Finished(mpsc::Sender<std::thread::ThreadId>);
    impl Drop for Finished {
        fn drop(&mut self) {
            let _ = self.0.send(std::thread::current().id());
        }
    }
    let finished = Finished(finished);
    let mut cache = Cache::default();
    let preparation = Preparation::start(
        || vec![ViewerOutput::Frame(Box::new(frame(1)))],
        move |outputs| {
            let _ = &finished;
            entered.send(()).unwrap();
            resume.recv_timeout(Duration::from_secs(5)).unwrap();
            batch(outputs, &mut cache)
        },
        Arc::new(move || {
            counted.fetch_add(1, Ordering::SeqCst);
            true
        }),
    )
    .unwrap();
    preparing.recv_timeout(Duration::from_secs(5)).unwrap();
    let shared = preparation.shared.clone();
    drop(preparation);
    assert!(lock(&shared.state).stopped);
    release.send(()).unwrap();
    assert_ne!(
        retired.recv_timeout(Duration::from_secs(5)).unwrap(),
        caller
    );
    assert!(lock(&shared.state).batch.is_none());
    assert_eq!(wakes.load(Ordering::SeqCst), 0);
}

#[test]
fn stopping_with_an_unconsumed_batch_reclaims_source_pixels_without_ui_delivery() {
    let frame = frame(1);
    let pixels = Arc::downgrade(&frame.pixels);
    let mut outputs = Some(vec![ViewerOutput::Frame(Box::new(frame))]);
    let (wake, ready) = mpsc::channel();
    let (finished, retired) = mpsc::channel();
    struct Finished(mpsc::Sender<()>);
    impl Drop for Finished {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }
    let finished = Finished(finished);
    let mut cache = Cache::default();
    let preparation = Preparation::start(
        move || outputs.take().unwrap_or_default(),
        move |outputs| {
            let _ = &finished;
            batch(outputs, &mut cache)
        },
        Arc::new(move || wake.send(()).is_ok()),
    )
    .unwrap();
    ready.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(pixels.strong_count(), 1);
    preparation.stop();
    retired.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!(pixels.strong_count(), 0);
    assert!(preparation.take().is_none());
    assert!(!preparation.shared.signal());
}

#[test]
fn invalid_pixels_leave_control_and_terminal_outputs_available_in_the_original_order() {
    let mut invalid = frame(1);
    invalid.pixels = Arc::from([]);
    let prepared = batch(
        vec![
            ViewerOutput::Status("before".into()),
            ViewerOutput::Frame(Box::new(invalid)),
            ViewerOutput::RasterUnavailable {
                surface: ViewerSurfaceId(1),
                message: "unavailable".into(),
            },
            ViewerOutput::Closed,
        ],
        &mut Cache::default(),
    );
    assert!(prepared.pixels.is_err());
    assert!(matches!(
        &prepared.outputs[..],
        [
            ViewerOutput::Status(_),
            ViewerOutput::Frame(_),
            ViewerOutput::RasterUnavailable { .. },
            ViewerOutput::Closed
        ]
    ));
}
