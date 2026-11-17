//! Reconcile previews after the window closes, within the existing exit grace.
use midnight_core::{Command, Event};
use midnight_engine::{CommandSendError, CommandSender};
use midnight_ui::{DeferredDrop, Presenter};
use std::{cell::RefCell, future::Future};
use tokio::{sync::mpsc, time::Instant};

/// Drain event backpressure while reserving room for the newest final geometry
/// save. The same exit deadline bounds both this flush and the engine shutdown.
#[expect(
    clippy::result_large_err,
    reason = "Keep refused commands intact in inline results for worker retirement."
)]
pub(crate) async fn flush_final(
    presenter: &RefCell<Presenter>,
    events: &mut mpsc::Receiver<Event>,
    commands: &CommandSender,
    deadline: Instant,
    retired: &mut Vec<DeferredDrop>,
) {
    flush_final_reserved(
        presenter,
        events,
        commands.reserve(),
        |permit, command| permit.try_send(command),
        deadline,
        retired,
    )
    .await;
}

async fn flush_final_reserved<P, E>(
    presenter: &RefCell<Presenter>,
    events: &mut mpsc::Receiver<Event>,
    reserve: impl Future<Output = Result<P, E>>,
    publish: impl Fn(P, Command) -> Result<(), CommandSendError>,
    deadline: Instant,
    retired: &mut Vec<DeferredDrop>,
) {
    let Some(command) = presenter.borrow().pending_preferences_save() else {
        return;
    };
    tokio::pin!(reserve);
    let mut events_open = true;
    loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => break,
            result = &mut reserve => {
                match result {
                    Ok(permit) => {
                        retain_send_result(publish(permit, command), retired);
                    }
                    Err(_) => retired.push(Box::new(command)),
                }
                return;
            }
            event = events.recv(), if events_open => match event {
                Some(event) => retired.push(Box::new(event)),
                None => events_open = false,
            },
        }
    }
    retired.push(Box::new(command));
}

#[expect(
    clippy::result_large_err,
    reason = "Keep refused commands intact in inline results for worker retirement."
)]
pub(crate) async fn reconcile(
    presenter: &RefCell<Presenter>,
    events: &mut mpsc::Receiver<Event>,
    commands: &CommandSender,
    deadline: Instant,
    retired: &mut Vec<DeferredDrop>,
) {
    reconcile_reserved(
        presenter,
        events,
        commands.reserve(),
        |permit, command| permit.try_send(command),
        deadline,
        retired,
    )
    .await;
}

async fn reconcile_reserved<P, E>(
    presenter: &RefCell<Presenter>,
    events: &mut mpsc::Receiver<Event>,
    reserve: impl Future<Output = Result<P, E>>,
    publish: impl Fn(P, Command) -> Result<(), CommandSendError>,
    deadline: Instant,
    retired: &mut Vec<DeferredDrop>,
) {
    if !presenter.borrow().needs_initial_preferences() {
        return;
    }
    // Reserve before applying the read so a saturated command queue cannot
    // discard the merged save. Keep draining meanwhile: the engine may itself
    // be waiting for outbox room before it can consume another command.
    tokio::pin!(reserve);
    let mut initial = None;
    let mut events_open = true;
    let permit = loop {
        tokio::select! {
            biased;
            _ = tokio::time::sleep_until(deadline) => {
                if let Some(event) = initial.take() { retired.push(Box::new(event)); }
                return;
            }
            result = &mut reserve => match result {
                Ok(permit) => break permit,
                Err(_) => {
                    if let Some(event) = initial.take() { retired.push(Box::new(event)); }
                    return;
                }
            },
            event = events.recv(), if events_open => match event {
                Some(event) if matches!(&event, Event::PreferencesLoaded(_)) && initial.is_none() => {
                    initial = Some(event);
                }
                Some(event) => retired.push(Box::new(event)),
                None => {
                    events_open = false;
                    if initial.is_none() { return; }
                }
            },
        }
    };
    let permit = RefCell::new(Some(permit));
    while presenter.borrow().needs_initial_preferences() {
        let event = if let Some(event) = initial.take() {
            event
        } else {
            let Ok(Some(event)) = tokio::time::timeout_at(deadline, events.recv()).await else {
                return;
            };
            event
        };
        if matches!(&event, Event::PreferencesLoaded(_)) {
            {
                let refused = RefCell::new(&mut *retired);
                presenter.borrow_mut().apply_with_sender(event, &|command| {
                    if !matches!(&command, Command::SavePreferences { .. }) {
                        refused.borrow_mut().push(Box::new(command));
                        return false;
                    }
                    let Some(permit) = permit.borrow_mut().take() else {
                        refused.borrow_mut().push(Box::new(command));
                        return false;
                    };
                    retain_send_result(publish(permit, command), &mut refused.borrow_mut())
                });
            }
            retired.extend(presenter.borrow_mut().take_retired());
        } else {
            // Ownership remains in the caller even if a later await times out.
            retired.push(Box::new(event));
        }
    }
}

fn retain_send_result(
    result: Result<(), CommandSendError>,
    retired: &mut Vec<DeferredDrop>,
) -> bool {
    match result {
        Ok(()) => true,
        Err(error) => {
            retired.push(Box::new(error.into_inner()));
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{Bookmark, BookmarkId, FsError, PaneId, UserPreferences};
    use std::{io, path::PathBuf, sync::Arc, time::Duration};

    // Raw channels model only reservation/event backpressure in the shared logic.
    // Real CommandSender admission and typed permit coverage remain Engine fixtures.
    #[expect(
        clippy::result_large_err,
        reason = "Match the inline publication result contract in reservation/backpressure tests."
    )]
    async fn flush_final_fake(
        presenter: &RefCell<Presenter>,
        events: &mut mpsc::Receiver<Event>,
        commands: &mpsc::Sender<Command>,
        deadline: Instant,
        retired: &mut Vec<DeferredDrop>,
    ) {
        flush_final_reserved(
            presenter,
            events,
            commands.reserve(),
            |permit, command| {
                permit.send(command);
                Ok(())
            },
            deadline,
            retired,
        )
        .await;
    }

    #[expect(
        clippy::result_large_err,
        reason = "Match the inline publication result contract in reservation/backpressure tests."
    )]
    async fn reconcile_fake(
        presenter: &RefCell<Presenter>,
        events: &mut mpsc::Receiver<Event>,
        commands: &mpsc::Sender<Command>,
        deadline: Instant,
        retired: &mut Vec<DeferredDrop>,
    ) {
        reconcile_reserved(
            presenter,
            events,
            commands.reserve(),
            |permit, command| {
                permit.send(command);
                Ok(())
            },
            deadline,
            retired,
        )
        .await;
    }

    fn editing() -> RefCell<Presenter> {
        let root = std::env::current_dir().unwrap();
        let mut presenter = Presenter::new(root.clone(), root);
        assert!(presenter.assign_bookmark_slot(PaneId::Left, 1, &|_| {
            panic!("preview cannot save before the initial read")
        }));
        RefCell::new(presenter)
    }

    #[tokio::test]
    async fn final_geometry_save_drains_event_backpressure_and_precedes_shutdown() {
        let mut state = Presenter::new("left".into(), "right".into());
        state.apply_with_sender(Event::PreferencesLoaded(Ok(Box::default())), &|_| true);
        let geometry = midnight_core::WindowGeometry {
            normal: midnight_core::WindowRect {
                x: -1800,
                y: 121,
                width: 973,
                height: 641,
            },
            monitor: None,
            maximized: true,
            fullscreen: false,
        };
        state.save_window_geometry(geometry.clone(), &|_| false);
        let presenter = RefCell::new(state);
        let (sender, mut events) = mpsc::channel(1);
        let (commands, mut received) = mpsc::channel(1);
        commands.send(Command::ListVolumes).await.unwrap();
        sender
            .send(Event::PreferencesSaved {
                revision: midnight_core::PreferencesRevision(0),
                result: Ok(()),
            })
            .await
            .unwrap();
        let worker = tokio::spawn(async move {
            sender
                .send(Event::PreferencesLoaded(Ok(Box::default())))
                .await
                .unwrap();
            assert!(matches!(received.recv().await, Some(Command::ListVolumes)));
            received.recv().await.unwrap()
        });
        let mut retired = Vec::new();
        flush_final_fake(
            &presenter,
            &mut events,
            &commands,
            Instant::now() + Duration::from_secs(1),
            &mut retired,
        )
        .await;
        let Command::SavePreferences { preferences, .. } = worker.await.unwrap() else {
            panic!()
        };
        assert_eq!(preferences.window_geometry, Some(geometry));
        tokio::task::spawn_blocking(move || drop(retired))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delayed_initial_read_enqueues_merged_save_before_shutdown() {
        let presenter = editing();
        let local = presenter.borrow().bookmarks()[0].clone();
        let (sender, mut events) = mpsc::channel(2);
        let (commands, mut received) = mpsc::channel(1);
        let saved = UserPreferences {
            font_family: "Stored 文档 🧪".into(),
            bookmarks: vec![Bookmark {
                network: None,
                archive: None,
                id: BookmarkId(1),
                name: "Previously saved".into(),
                path: std::env::current_dir().unwrap().join("saved"),
                slot: Some(1),
            }],
            ..Default::default()
        };
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            sender
                .send(Event::PreferencesLoaded(Ok(saved.into())))
                .await
                .unwrap();
        });
        let mut retired = Vec::new();
        reconcile_fake(
            &presenter,
            &mut events,
            &commands,
            Instant::now() + Duration::from_secs(1),
            &mut retired,
        )
        .await;
        let Command::SavePreferences { preferences, .. } = received.try_recv().unwrap() else {
            panic!("merged settings save must precede shutdown")
        };
        preferences.validate().unwrap();
        assert_eq!(preferences.font_family, "Stored 文档 🧪");
        assert_eq!(preferences.bookmarks.len(), 2);
        assert!(preferences.bookmarks.contains(&local));
        assert!(!presenter.borrow().needs_initial_preferences());
        commands.send(Command::Shutdown).await.unwrap();
        assert!(matches!(received.try_recv(), Ok(Command::Shutdown)));
        tokio::task::spawn_blocking(move || drop(retired))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn read_error_during_exit_preserves_unknown_existing_settings() {
        let presenter = editing();
        let (sender, mut events) = mpsc::channel(1);
        let (commands, mut received) = mpsc::channel(1);
        sender
            .send(Event::PreferencesLoaded(Err(FsError::from_io(
                "load preferences",
                &PathBuf::new(),
                io::Error::from(io::ErrorKind::PermissionDenied),
            ))))
            .await
            .unwrap();
        let mut retired = Vec::new();
        reconcile_fake(
            &presenter,
            &mut events,
            &commands,
            Instant::now() + Duration::from_secs(1),
            &mut retired,
        )
        .await;
        assert!(
            received.try_recv().is_err(),
            "read failures cannot emit a save"
        );
        assert!(!presenter.borrow().needs_initial_preferences());
        tokio::task::spawn_blocking(move || drop(retired))
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn timeout_keeps_unrelated_large_payloads_owned_until_backend_retirement() {
        let presenter = editing();
        let (sender, mut events) = mpsc::channel(1);
        let (commands, mut received) = mpsc::channel(1);
        let entries: Arc<[midnight_core::Entry]> = vec![midnight_core::Entry::new(
            midnight_core::EntryId(7),
            "unrelated".into(),
            midnight_core::EntryKind::Directory,
        )]
        .into();
        let weak = Arc::downgrade(&entries);
        sender
            .send(Event::ListingFinished {
                pane: PaneId::Right,
                navigation: midnight_core::NavigationId(4),
                index: Arc::new(midnight_core::DirectoryIndex::from_entries(&entries)),
                entries,
            })
            .await
            .unwrap();
        let mut retired = Vec::new();
        reconcile_fake(
            &presenter,
            &mut events,
            &commands,
            Instant::now() + Duration::from_millis(20),
            &mut retired,
        )
        .await;
        assert!(
            received.try_recv().is_err(),
            "timeout cannot publish unread defaults"
        );
        assert!(presenter.borrow().needs_initial_preferences());
        assert!(
            weak.upgrade().is_some(),
            "timeout must not destroy queued listings"
        );
        tokio::task::spawn_blocking(move || drop(retired))
            .await
            .unwrap();
        assert!(weak.upgrade().is_none());
    }

    #[tokio::test]
    async fn full_command_and_event_queues_cannot_deadlock_the_final_save() {
        let presenter = editing();
        let (sender, mut events) = mpsc::channel(1);
        let (commands, mut received) = mpsc::channel(1);
        commands.send(Command::ListVolumes).await.unwrap();
        sender
            .send(Event::ListingStarted {
                pane: PaneId::Right,
                navigation: midnight_core::NavigationId(4),
                path: "unrelated".into(),
            })
            .await
            .unwrap();
        let worker = tokio::spawn(async move {
            // Match an engine blocked delivering an outbox event before its
            // dispatcher can drain the already-full command queue.
            sender
                .send(Event::PreferencesLoaded(Ok(Box::default())))
                .await
                .unwrap();
            assert!(matches!(received.recv().await, Some(Command::ListVolumes)));
            received.recv().await.unwrap()
        });
        let mut retired = Vec::new();
        reconcile_fake(
            &presenter,
            &mut events,
            &commands,
            Instant::now() + Duration::from_secs(1),
            &mut retired,
        )
        .await;
        let command = tokio::time::timeout(Duration::from_secs(1), worker)
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(command, Command::SavePreferences { .. }));
        assert!(!presenter.borrow().needs_initial_preferences());
        tokio::task::spawn_blocking(move || drop(retired))
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn refused_permit_payloads_stay_owned_under_presenter_borrow_until_worker_reclaims_them()
    {
        use midnight_core::{
            Cancellation, CapturedTransferRequest, CapturedTransferSources,
            PreparedCapturedTransfer, TransferIntent, TransferMetadataPolicy, TransferService,
            TransferStartPolicy, TransferVerification,
        };
        use std::{sync::Mutex, thread::ThreadId};
        use tokio::sync::mpsc::error::TrySendError;

        #[derive(Debug)]
        struct NativeTail(Arc<Mutex<Option<ThreadId>>>);
        impl Drop for NativeTail {
            fn drop(&mut self) {
                *self.0.lock().unwrap() = Some(std::thread::current().id());
            }
        }
        impl CapturedTransferSources for NativeTail {
            fn intent(&self) -> TransferIntent {
                panic!("reclamation must not inspect providers")
            }
            fn root_count(&self) -> usize {
                panic!("reclamation must not inspect providers")
            }
            fn metadata_available(&self) -> bool {
                panic!("reclamation must not inspect providers")
            }
            fn prepare(
                &self,
                _: &dyn TransferService,
                _: &Cancellation,
            ) -> Result<PreparedCapturedTransfer, FsError> {
                panic!("reclamation must not prepare filesystem work")
            }
        }

        let caller = std::thread::current().id();
        for closed in [false, true] {
            let dropped = Arc::new(Mutex::new(None));
            let source: Arc<dyn CapturedTransferSources> = Arc::new(NativeTail(dropped.clone()));
            let weak = Arc::downgrade(&source);
            let command = Command::EnqueueCapturedTransfer(CapturedTransferRequest {
                id: midnight_core::OperationId(7),
                verification: TransferVerification::FullSha256,
                start_policy: TransferStartPolicy::WaitForEarlier,
                sources: source,
                destination: PathBuf::from("destination/文档"),
                metadata: TransferMetadataPolicy::ContentOnly,
            });
            let refused = if closed {
                TrySendError::Closed(command)
            } else {
                TrySendError::Full(command)
            };
            let presenter = editing();
            let retired = RefCell::new(Vec::new());
            {
                let _active = presenter.borrow_mut();
                assert!(!retain_send_result(Err(refused), &mut retired.borrow_mut()));
                assert!(weak.upgrade().is_some());
                assert!(dropped.lock().unwrap().is_none());
            }
            let queue = midnight_engine::reclaim::start(
                &tokio::runtime::Handle::current(),
                Arc::new(|| {}),
            );
            assert!(queue.send(retired.into_inner()).await.is_ok());
            midnight_engine::reclaim::barrier(&queue).await.unwrap();
            assert!(weak.upgrade().is_none());
            assert_ne!(dropped.lock().unwrap().as_ref().unwrap(), &caller);
        }
    }

    #[test]
    fn accepted_permit_result_needs_no_caller_retirement() {
        let mut retired = Vec::new();
        assert!(retain_send_result(Ok(()), &mut retired));
        assert!(retired.is_empty());
    }
}
