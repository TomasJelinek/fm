use super::*;
use crate::captured_native::tests::{Gate, Native, Probe, Release, Source};
use std::sync::atomic::Ordering;

async fn result(events: &mut mpsc::Receiver<Event>) -> Result<DesktopActionResult, FsError> {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Event::DesktopActionFinished {
                request: DesktopRequestId(1),
                result,
            } = events.recv().await.unwrap()
            {
                return result;
            }
        }
    })
    .await
    .unwrap()
}
fn fixture(native: Arc<Native>) -> (Desktop, JoinSet<()>, mpsc::Receiver<Event>) {
    let (sender, events) = mpsc::channel(32);
    let mut tasks = JoinSet::new();
    let desktop = Desktop::testing(
        Some(native),
        None,
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    (desktop, tasks, events)
}
fn paths() -> Vec<PathBuf> {
    let root = std::env::temp_dir();
    vec![
        root.join("root.txt"),
        root.join("nested/literal $() 文档.txt"),
    ]
}

#[tokio::test(flavor = "current_thread")]
async fn captured_properties_keep_exact_paths_on_worker_through_native_or_metadata_fallback() {
    for properties_available in [true, false] {
        let probe = Arc::new(Probe::default());
        let native = Arc::new(Native {
            properties_available,
            ..Native::new(probe.clone())
        });
        let (desktop, mut tasks, mut events) = fixture(native.clone());
        desktop
            .properties_captured(
                native,
                DesktopRequestId(1),
                Arc::new(Source::new(paths(), probe.clone())),
                Cancellation::default(),
                &mut tasks,
            )
            .await;
        assert!(matches!(
            result(&mut events).await.unwrap(),
            DesktopActionResult::PropertiesOpened | DesktopActionResult::PropertiesInfo { .. }
        ));
        probe.wait_for_drop().await;
        let calls = probe.calls.lock().unwrap();
        assert_eq!(calls[0].0, "resolve");
        assert_eq!(calls[1].0, "properties");
        assert_eq!(calls[1].1, paths());
        assert_eq!(calls.len(), if properties_available { 2 } else { 4 });
        assert!(
            calls
                .iter()
                .all(|call| call.2 != std::thread::current().id())
        );
        tasks.abort_all();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn captured_open_uses_exact_occurrence_and_rejects_foreign_or_ambiguous_identity() {
    for scenario in 0..5 {
        let probe = Arc::new(Probe::default());
        let native = Arc::new(Native::new(probe.clone()));
        let (desktop, mut tasks, mut events) = fixture(native);
        let mut source = Source::new(paths(), probe.clone());
        let (pane, navigation, row) = match scenario {
            1 => (PaneId::Right, NavigationId(5), TreeRowId(2)),
            2 => (PaneId::Left, NavigationId(6), TreeRowId(2)),
            3 => (PaneId::Left, NavigationId(5), TreeRowId(99)),
            _ => (PaneId::Left, NavigationId(5), TreeRowId(2)),
        };
        if scenario == 4 {
            source.occurrences[0] = source.occurrences[1];
        }
        desktop
            .open_captured_entry(
                DesktopRequestId(1),
                pane,
                navigation,
                Arc::new(source),
                Cancellation::default(),
                row,
                &mut tasks,
            )
            .await;
        assert_eq!(result(&mut events).await.is_ok(), scenario == 0);
        probe.wait_for_drop().await;
        let calls = probe.calls.lock().unwrap();
        assert_eq!(calls.len(), if scenario == 0 { 2 } else { 1 });
        if scenario == 0 {
            assert_eq!(calls[1].1, vec![paths()[1].clone()]);
        }
        assert!(
            calls
                .iter()
                .all(|call| call.2 != std::thread::current().id())
        );
        tasks.abort_all();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn cancelled_revoked_or_changed_captures_never_invoke_desktop_services() {
    for properties in [false, true] {
        for scenario in 0..4 {
            let probe = Arc::new(Probe::default());
            let native = Arc::new(Native::new(probe.clone()));
            let (desktop, mut tasks, mut events) = fixture(native.clone());
            let mut source = Source::new(paths(), probe.clone());
            let cancel = Cancellation::default();
            match scenario {
                0 => cancel.cancel(),
                1 => source.cancel_during_resolve = true,
                2 => source.rejected = true,
                _ => source.intent = FileClipboardIntent::Move,
            }
            if properties {
                desktop
                    .properties_captured(
                        native,
                        DesktopRequestId(1),
                        Arc::new(source),
                        cancel,
                        &mut tasks,
                    )
                    .await;
            } else {
                desktop
                    .open_captured_entry(
                        DesktopRequestId(1),
                        PaneId::Left,
                        NavigationId(5),
                        Arc::new(source),
                        cancel,
                        TreeRowId(1),
                        &mut tasks,
                    )
                    .await;
            }
            assert!(result(&mut events).await.is_err());
            probe.wait_for_drop().await;
            assert_eq!(
                probe.calls.lock().unwrap().len(),
                usize::from(scenario != 0)
            );
            tasks.abort_all();
        }
    }
}

#[tokio::test(flavor = "current_thread")]
async fn shared_desktop_admission_rejects_capture_without_resolving_and_retires_it() {
    for properties in [false, true] {
        let probe = Arc::new(Probe::default());
        let native = Arc::new(Native::new(probe.clone()));
        let (desktop, mut tasks, mut events) = fixture(native.clone());
        let budget = if properties {
            &desktop.property_budget
        } else {
            &desktop.open_budget
        };
        let _held = budget
            .clone()
            .acquire_many_owned(budget.available_permits() as u32)
            .await
            .unwrap();
        let source = Arc::new(Source::new(paths(), probe.clone()));
        if properties {
            desktop
                .properties_captured(
                    native,
                    DesktopRequestId(1),
                    source,
                    Cancellation::default(),
                    &mut tasks,
                )
                .await;
        } else {
            desktop
                .open_captured_entry(
                    DesktopRequestId(1),
                    PaneId::Left,
                    NavigationId(5),
                    source,
                    Cancellation::default(),
                    TreeRowId(1),
                    &mut tasks,
                )
                .await;
        }
        assert!(result(&mut events).await.is_err());
        probe.wait_for_drop().await;
        assert!(probe.calls.lock().unwrap().is_empty());
        tasks.abort_all();
    }
}

#[tokio::test(flavor = "current_thread")]
async fn aborting_desktop_reply_retains_source_and_admission_through_native_return() {
    for properties in [false, true] {
        let probe = Arc::new(Probe::default());
        let gate = Arc::new(Gate::default());
        let release = Release(gate.clone());
        let native = Arc::new(Native {
            gate: Some(gate.clone()),
            ..Native::new(probe.clone())
        });
        let (desktop, mut tasks, _events) = fixture(native.clone());
        let source = Arc::new(Source::new(paths(), probe.clone()));
        if properties {
            desktop
                .properties_captured(
                    native,
                    DesktopRequestId(1),
                    source,
                    Cancellation::default(),
                    &mut tasks,
                )
                .await;
        } else {
            desktop
                .open_captured_entry(
                    DesktopRequestId(1),
                    PaneId::Left,
                    NavigationId(5),
                    source,
                    Cancellation::default(),
                    TreeRowId(1),
                    &mut tasks,
                )
                .await;
        }
        gate.wait().await;
        tasks.abort_all();
        while tasks.join_next().await.is_some() {}
        assert!(!probe.dropped.load(Ordering::Acquire));
        let budget = if properties {
            &desktop.property_budget
        } else {
            &desktop.open_budget
        };
        assert_eq!(budget.available_permits(), if properties { 0 } else { 3 });
        drop(release);
        probe.wait_for_drop().await;
    }
}
