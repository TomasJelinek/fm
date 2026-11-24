//! A refresh that changes nothing must not republish presentation state:
//! dirty properties repaint every bound control, and a full refresh walks all
//! models. Software test platforms have no native icons, so every action icon
//! is absent here, the case where Slint image equality never holds.
use crate::{HotPathsPresentation, PanelFilterPresentation, test_harness::Harness};
use i_slint_core::properties::PropertyTracker;
use midnight_core::{Command, Event, PaneId, UserPreferences};
use slint::{ComponentHandle, platform::Key};
use std::{cell::Cell, pin::Pin, rc::Rc, sync::Arc, time::Duration};

/// Track the properties `read` reads; `is_dirty` then reports any later set
/// that Slint considered a change.
fn track(read: impl FnOnce()) -> Pin<Box<PropertyTracker>> {
    let tracker = Box::pin(PropertyTracker::default());
    tracker.as_ref().evaluate(read);
    tracker
}

#[test]
fn unchanged_refreshes_leave_open_tool_dialog_icons_clean() {
    let root = std::env::temp_dir().join("refresh-churn");
    let mut preferences = UserPreferences::default();
    for tabs in &mut preferences.tabs {
        tabs.paths = vec![root.clone()];
    }
    let mut h = Harness::with_location(preferences, root);
    let main = h.window.clone_strong();
    assert_eq!(
        main.get_action_icons().close.size().width,
        0,
        "absent native icons are the case these guards cover"
    );

    main.invoke_open_hot_paths_window(0);
    h.settle();
    let hot_paths = main.global::<HotPathsPresentation>();
    assert!(!hot_paths.get_token().is_empty(), "the manager is open");
    let palette = track(|| {
        hot_paths.get_action_icons();
    });
    h.refresh();
    h.refresh();
    assert!(
        !palette.is_dirty(),
        "an unchanged palette is not republished to the open Hot Paths manager"
    );
    hot_paths.invoke_dismiss(hot_paths.get_token());
    h.settle();

    h.presenter.borrow_mut().open_panel_filter(PaneId::Left);
    h.refresh();
    let filter = main.global::<PanelFilterPresentation>();
    assert!(!filter.get_token().is_empty(), "the filter is open");
    let close = track(|| {
        filter.get_close_icon();
    });
    h.refresh();
    h.refresh();
    assert!(
        !close.is_dirty(),
        "an unchanged close icon is not republished to the open Panel Filter"
    );
}

#[test]
fn a_path_paste_waiting_for_the_clipboard_refreshes_only_when_it_inserts() {
    let mut h = Harness::new(UserPreferences::default());
    let refreshes = Rc::new(Cell::new(0));
    let count = refreshes.clone();
    // Only the application host installs this hook; every full refresh calls it.
    h.window
        .on_refresh_warning_viewer(move || count.set(count.get() + 1));
    h.window.invoke_focus_files();
    h.chord(&[Key::Control], "g");
    assert_eq!(h.focused_label().as_deref(), Some("Folder path"));
    h.freeze_time();
    h.sent.borrow_mut().clear();
    h.chord(&[Key::Control], "v");
    let request = h
        .sent
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::ReadPathClipboardText { request } => Some(*request),
            _ => None,
        })
        .expect("Paste asks the backend for clipboard text");
    let waiting = refreshes.get();
    for _ in 0..8 {
        h.advance(Duration::from_millis(25));
    }
    assert_eq!(
        refreshes.get(),
        waiting,
        "ticks that are still waiting for the reply change nothing"
    );
    h.presenter
        .borrow_mut()
        .apply(Event::PathClipboardTextRead {
            request,
            result: Ok(Arc::from("pasted")),
        });
    h.advance(Duration::from_millis(25));
    assert_eq!(
        refreshes.get(),
        waiting + 1,
        "the tick that inserts the reply refreshes once"
    );
    assert!(
        h.window.get_left_path_draft().contains("pasted"),
        "{}",
        h.window.get_left_path_draft()
    );
}

/// A progress report changes its job's row in place: the Jobs model stays the same
/// model, so its rows are not rebuilt, and only the rows that differ are told. A job
/// that ends, or a new one, still shows; one that is cleared leaves.
#[test]
fn job_progress_updates_its_row_in_place_and_keeps_the_jobs_model() {
    use midnight_core::{
        CopyEvent, CopyOperationState, CopyProgress, DragDropEvent, DropToken, OperationId,
        TransferIntent,
    };
    use slint::Model;
    let mut h = Harness::new(UserPreferences::default());
    let job = |id: u64| {
        let destination = std::env::temp_dir().join("refresh-churn-jobs");
        h.presenter.borrow_mut().apply_with_sender(
            Event::DragDrop(DragDropEvent::DropAccepted {
                drag: None,
                token: DropToken(id),
                pane: PaneId::Right,
                item_count: 1,
                intent: TransferIntent::Copy,
                verification: None,
                destination: Arc::new(destination),
            }),
            &|_| true,
        );
    };
    job(1);
    job(2);
    h.refresh();
    let identities = |h: &Harness| {
        h.window
            .get_operations()
            .iter()
            .map(|row| row.identity.to_string())
            .collect::<Vec<_>>()
    };
    let shown = identities(&h);
    assert_eq!(shown.len(), 2, "{shown:?}");
    let model = h.window.get_operations();
    let id = |identity: &str| {
        identity
            .strip_prefix("copy-job:")
            .and_then(|id| id.parse().ok())
            .map(OperationId)
            .unwrap()
    };
    let progress = |h: &mut Harness, identity: &str, state, bytes| {
        let mut progress =
            CopyProgress::queued(id(identity), "destination".into(), 1, TransferIntent::Copy);
        progress.state = state;
        progress.completed_bytes = bytes;
        h.presenter.borrow_mut().apply_with_sender(
            Event::Copy(CopyEvent::Progress(Arc::new(progress))),
            &|_| true,
        );
        h.refresh();
    };
    for bytes in [1_000, 2_000, 3_000] {
        progress(&mut h, &shown[1], CopyOperationState::Running, bytes);
        assert!(
            h.window.get_operations() == model,
            "a report keeps the model"
        );
        assert_eq!(identities(&h), shown);
    }
    progress(&mut h, &shown[1], CopyOperationState::Completed, 3_000);
    assert!(
        h.window.get_operations() == model,
        "an ended job keeps the model"
    );
    let ended = model.row_data(1).unwrap();
    assert!(
        !ended.active,
        "the row shows that it ended: {}",
        ended.state
    );
    // Clearing the finished job removes its row: a new model of the remaining job.
    h.window.invoke_clear_finished_jobs(false);
    h.refresh();
    assert_eq!(identities(&h), [shown[0].clone()]);
    assert!(h.window.get_operations() != model);
}
