use super::*;
use crate::captured_native::tests::{Gate, Native, Probe, Release, Source};
use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};
use tokio::sync::mpsc;

fn input(source: Arc<dyn CapturedClipboardFiles>, cancel: Cancellation) -> CapturedInput {
    CapturedInput {
        request: DesktopRequestId(1),
        source,
        cancel,
        owner: 0,
        position: Default::default(),
        modifiers: Default::default(),
    }
}
async fn result(events: &mut mpsc::Receiver<Event>) -> Result<ShellMenuOutcome, FsError> {
    match tokio::time::timeout(Duration::from_secs(5), events.recv())
        .await
        .unwrap()
        .unwrap()
    {
        Event::ShellMenuFinished { result, .. } => result,
        other => panic!("unexpected event: {other:?}"),
    }
}
fn fixture(native: Arc<Native>) -> (Controller, JoinSet<()>, mpsc::Receiver<Event>) {
    let (sender, events) = mpsc::channel(8);
    (
        Controller::testing(
            Some(native),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
        ),
        JoinSet::new(),
        events,
    )
}
fn paths(cross_parent: bool) -> Vec<PathBuf> {
    let root = std::env::temp_dir();
    vec![
        root.join("literal $() 文档.txt"),
        root.join(if cross_parent {
            "nested/second"
        } else {
            "second"
        }),
    ]
}

#[tokio::test(flavor = "current_thread")]
async fn captured_menu_preserves_same_parent_paths_and_falls_back_for_valid_cross_parent_selection()
{
    for cross_parent in [false, true] {
        let probe = Arc::new(Probe::default());
        let (controller, mut tasks, mut events) = fixture(Arc::new(Native::new(probe.clone())));
        controller
            .show_captured(
                input(
                    Arc::new(Source::new(paths(cross_parent), probe.clone())),
                    Cancellation::default(),
                ),
                &mut tasks,
            )
            .await;
        assert_eq!(
            result(&mut events).await.unwrap(),
            if cross_parent {
                ShellMenuOutcome::Unavailable
            } else {
                ShellMenuOutcome::Dismissed
            }
        );
        probe.wait_for_drop().await;
        let calls = probe.calls.lock().unwrap();
        assert_eq!(calls.len(), if cross_parent { 1 } else { 2 });
        assert_eq!(calls[0].0, "resolve");
        if !cross_parent {
            assert_eq!(calls[1].0, "menu");
            assert_eq!(calls[1].1, paths(false));
        }
        assert!(
            calls
                .iter()
                .all(|call| call.2 != std::thread::current().id())
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn invalid_cancelled_or_revoked_menu_capture_never_becomes_fallback_or_native_invocation() {
    for scenario in 0..7 {
        let probe = Arc::new(Probe::default());
        let (controller, mut tasks, mut events) = fixture(Arc::new(Native::new(probe.clone())));
        let mut source = Source::new(paths(true), probe.clone());
        let cancel = Cancellation::default();
        match scenario {
            0 => cancel.cancel(),
            1 => source.cancel_during_resolve = true,
            2 => source.rejected = true,
            3 => source.intent = FileClipboardIntent::Move,
            4 => source.paths[1] = "relative".into(),
            5 => {
                source.paths = vec![std::env::temp_dir().join("same"); MAX_FILE_CLIPBOARD_PATHS + 1]
            }
            _ => controller.shutdown(),
        }
        controller
            .show_captured(input(Arc::new(source), cancel), &mut tasks)
            .await;
        assert!(result(&mut events).await.is_err());
        probe.wait_for_drop().await;
        assert_eq!(
            probe.calls.lock().unwrap().len(),
            usize::from(scenario != 0 && scenario != 6)
        );
    }
}

#[tokio::test(flavor = "current_thread")]
async fn captured_and_ordinary_menus_share_admission_and_abort_keeps_native_source_alive() {
    let probe = Arc::new(Probe::default());
    let gate = Arc::new(Gate::default());
    let release = Release(gate.clone());
    let (controller, mut tasks, mut events) = fixture(Arc::new(Native {
        gate: Some(gate.clone()),
        ..Native::new(probe.clone())
    }));
    controller
        .show_captured(
            input(
                Arc::new(Source::new(paths(false), probe.clone())),
                Cancellation::default(),
            ),
            &mut tasks,
        )
        .await;
    gate.wait().await;
    controller
        .show(
            Input {
                request: DesktopRequestId(2),
                files: SelectedFiles {
                    selection: Arc::new(SelectionSnapshot::default()),
                    view: Arc::new(DirectoryIndex::default()),
                    fallback: None,
                },
                owner: 0,
                position: Default::default(),
                modifiers: Default::default(),
                target: ShellMenuTarget::Items,
            },
            &mut tasks,
        )
        .await;
    assert_eq!(
        result(&mut events).await.unwrap_err().kind,
        io::ErrorKind::WouldBlock
    );
    let rejected = Arc::new(Probe::default());
    controller
        .show_captured(
            input(
                Arc::new(Source::new(paths(false), rejected.clone())),
                Cancellation::default(),
            ),
            &mut tasks,
        )
        .await;
    assert_eq!(
        result(&mut events).await.unwrap_err().kind,
        io::ErrorKind::WouldBlock
    );
    rejected.wait_for_drop().await;
    assert!(rejected.calls.lock().unwrap().is_empty());
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    assert_eq!(controller.budget.available_permits(), 0);
    assert!(!probe.dropped.load(Ordering::Acquire));
    drop(release);
    probe.wait_for_drop().await;
    tokio::time::timeout(Duration::from_secs(5), async {
        while controller.budget.available_permits() == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
