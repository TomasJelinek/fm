//! Reconcile previews after the window closes, within the existing exit grace.
use fm_core::{Command, Event};
use fm_ui::{DeferredDrop, Presenter};
use std::cell::RefCell;
use tokio::{sync::mpsc, time::Instant};

pub(crate) async fn reconcile(
    presenter: &RefCell<Presenter>,
    events: &mut mpsc::Receiver<Event>,
    commands: &mpsc::Sender<Command>,
    deadline: Instant,
    retired: &mut Vec<DeferredDrop>,
) {
    if !presenter.borrow().needs_initial_preferences() {
        return;
    }
    // Reserve before applying the read so a saturated command queue cannot
    // discard the merged save. Keep draining meanwhile: the engine may itself
    // be waiting for outbox room before it can consume another command.
    let reserve = commands.reserve();
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
            presenter.borrow_mut().apply_with_sender(event, &|command| {
                if !matches!(&command, Command::SavePreferences { .. }) {
                    return false;
                }
                let Some(permit) = permit.borrow_mut().take() else {
                    return false;
                };
                permit.send(command);
                true
            });
            retired.extend(presenter.borrow_mut().take_retired());
        } else {
            // Ownership remains in the caller even if a later await times out.
            retired.push(Box::new(event));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fm_core::{Bookmark, BookmarkId, FsError, PaneId, UserPreferences};
    use std::{io, path::PathBuf, sync::Arc, time::Duration};

    fn editing() -> RefCell<Presenter> {
        let root = std::env::current_dir().unwrap();
        let mut presenter = Presenter::new(root.clone(), root);
        assert!(presenter.assign_bookmark_slot(PaneId::Left, 1, &|_| {
            panic!("preview cannot save before the initial read")
        }));
        RefCell::new(presenter)
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
                .send(Event::PreferencesLoaded(Ok(saved)))
                .await
                .unwrap();
        });
        let mut retired = Vec::new();
        reconcile(
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
        reconcile(
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
        let entries: Arc<[fm_core::Entry]> = vec![fm_core::Entry::new(
            fm_core::EntryId(7),
            "unrelated".into(),
            fm_core::EntryKind::Directory,
        )]
        .into();
        let weak = Arc::downgrade(&entries);
        sender
            .send(Event::ListingFinished {
                pane: PaneId::Right,
                navigation: fm_core::NavigationId(4),
                index: Arc::new(fm_core::DirectoryIndex::from_entries(&entries)),
                entries,
            })
            .await
            .unwrap();
        let mut retired = Vec::new();
        reconcile(
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
                navigation: fm_core::NavigationId(4),
                path: "unrelated".into(),
            })
            .await
            .unwrap();
        let worker = tokio::spawn(async move {
            // Match an engine blocked delivering an outbox event before its
            // dispatcher can drain the already-full command queue.
            sender
                .send(Event::PreferencesLoaded(Ok(UserPreferences::default())))
                .await
                .unwrap();
            assert!(matches!(received.recv().await, Some(Command::ListVolumes)));
            received.recv().await.unwrap()
        });
        let mut retired = Vec::new();
        reconcile(
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
}
