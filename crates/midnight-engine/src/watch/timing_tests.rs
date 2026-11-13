use super::*;

async fn production_timing() -> Timing {
    let files = Arc::new(Files::default());
    let (sender, _events) = mpsc::channel(8);
    let mut tasks = JoinSet::new();
    let controller = Controller::new(
        Retirement::testing(),
        TaskLifetime::testing(files.clone()),
        files,
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let timing = controller.panes[0].timing;
    // A RED assertion must not leave real blocking watch owners behind. The
    // controller has no visible target, so obtaining its policy performs no I/O.
    stop(&controller, &mut tasks).await;
    timing
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_native_burst_coalesces_for_150ms_then_allows_later_refreshes() {
    let timing = production_timing().await;
    // The workers use std::Instant/thread::park, not Tokio's virtual clock.
    // Explicit instants exercise the real debounce policy without sleeping.
    let start = Instant::now();
    let mut debounce = Debouncer::default();
    for offset in [0, 40, 80] {
        debounce.record(start + Duration::from_millis(offset));
    }
    assert!(!debounce.take_due(
        start + Duration::from_millis(229),
        timing.quiet,
        timing.maximum,
    ));
    assert!(
        debounce.take_due(
            start + Duration::from_millis(230),
            timing.quiet,
            timing.maximum,
        ),
        "the coalesced burst must refresh after 150ms of quiet",
    );
    assert!(!debounce.take_due(
        start + Duration::from_millis(500),
        timing.quiet,
        timing.maximum,
    ));
    debounce.record(start + Duration::from_millis(800));
    assert!(!debounce.take_due(
        start + Duration::from_millis(949),
        timing.quiet,
        timing.maximum,
    ));
    assert!(debounce.take_due(
        start + Duration::from_millis(950),
        timing.quiet,
        timing.maximum,
    ));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn production_continuous_native_events_cannot_defer_refresh_beyond_500ms() {
    let timing = production_timing().await;
    let start = Instant::now();
    let mut debounce = Debouncer::default();
    for offset in [0, 90, 180, 270, 360, 450] {
        debounce.record(start + Duration::from_millis(offset));
    }
    assert!(!debounce.take_due(
        start + Duration::from_millis(499),
        timing.quiet,
        timing.maximum,
    ));
    assert!(
        debounce.take_due(
            start + Duration::from_millis(500),
            timing.quiet,
            timing.maximum,
        ),
        "continuous notifications must refresh by the 500ms burst cap",
    );
    assert!(!debounce.take_due(
        start + Duration::from_millis(501),
        timing.quiet,
        timing.maximum,
    ));
}
