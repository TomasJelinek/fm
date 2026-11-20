//! Show hidden files off: a flat listing leaves hidden entries out, so the
//! checks of its accepted rows leave them out too, by the same rule
//! (`hidden_entries`). Every watcher here polls without notifications.
use super::*;

/// Hidden on every platform: a dot-name (Unix) with the hidden attribute
/// (Windows).
fn hidden(length: u64) -> Entry {
    file(".設定-hidden", length, Some(0x2))
}

/// Shown on every platform.
fn shown(length: u64) -> Entry {
    file("shown-📄.txt", length, None)
}

fn file(name: &str, length: u64, attributes: Option<u32>) -> Entry {
    let mut entry = Entry::new(EntryId(1), Path::new("left").join(name), EntryKind::File);
    entry.metadata = EntryMetadata {
        size: Some(length),
        modified: Some(SystemTime::UNIX_EPOCH),
        native_attributes: attributes,
        ..EntryMetadata::default()
    };
    entry
}

fn put_left(files: &Files, entries: Vec<Entry>) {
    files
        .directories
        .lock()
        .unwrap()
        .insert("left".into(), entries);
}

/// The rows a listing of `left` shows, without hidden entries with `hide`.
fn listed(files: &Files, navigation: u64, hide: bool) -> VisibleDirectory {
    let mut visible = files.visible(PaneId::Left, navigation, "left");
    let mut entries = visible.entries.to_vec();
    hidden_entries::retain_visible(&mut entries, hide);
    visible.entries = entries.into();
    visible
}

/// The backend's dispatch of a Navigate/Refresh listing of `left`.
fn list(controller: &Controller, navigation: u64) {
    let navigation = NavigationId(navigation);
    controller.listing_started(PaneId::Left, navigation, Path::new("left"));
    controller.handle().clear_scope(PaneId::Left, navigation);
}

/// The backend's dispatch of a Resort of the UI's rows of `left`.
fn resort(controller: &Controller, navigation: u64) {
    let navigation = NavigationId(navigation);
    controller.resort_started(PaneId::Left, navigation, Path::new("left"));
    controller.handle().clear_scope(PaneId::Left, navigation);
}

/// No hint while three more checks run. A change found by a check is hinted
/// after the 20 ms quiet period, before the next check 40 ms later.
async fn quiet(files: &Files, events: &mut mpsc::Receiver<Event>) {
    let probes = files.probes.load(Ordering::SeqCst);
    files.wait_probes(probes + 3).await;
    if let Ok(event) = events.try_recv() {
        panic!("an unchanged view was hinted: {event:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hidden_entries_left_out_of_the_rows_are_left_out_of_checks() {
    let files = Arc::new(Files::default());
    put_left(&files, vec![shown(7), hidden(1)]);
    let (_owner, controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(40));
    controller.set_hide_hidden(true);
    list(&controller, 1);
    controller.set_visible(listed(&files, 1, true));
    // The folder holds an entry the rows lack, which is no change.
    quiet(&files, &mut events).await;
    // Neither is a change of a hidden entry alone, nor a new one.
    put_left(
        &files,
        vec![shown(7), hidden(2), file(".new", 1, Some(0x2))],
    );
    quiet(&files, &mut events).await;
    // A change of a shown entry is hinted.
    put_left(&files, vec![shown(8), hidden(2)]);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rows_that_show_hidden_entries_are_checked_with_them() {
    let files = Arc::new(Files::default());
    put_left(&files, vec![shown(7), hidden(1)]);
    let (_owner, controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(40));
    list(&controller, 1);
    controller.set_visible(listed(&files, 1, false));
    quiet(&files, &mut events).await;
    put_left(&files, vec![shown(7), hidden(2)]);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rows_are_checked_with_the_choice_they_were_listed_with() {
    let files = Arc::new(Files::default());
    put_left(&files, vec![shown(7), hidden(1)]);
    let (_owner, controller, mut tasks, mut events) =
        controller(files.clone(), None, Duration::from_millis(40));
    controller.set_hide_hidden(true);
    list(&controller, 1);
    controller.set_visible(listed(&files, 1, true));
    quiet(&files, &mut events).await;
    // Hidden files are shown again: the rows listed without them are still
    // checked without them until the UI accepts the pane's re-read.
    controller.set_hide_hidden(false);
    quiet(&files, &mut events).await;
    list(&controller, 2);
    controller.set_visible(listed(&files, 2, false));
    quiet(&files, &mut events).await;
    // A resort of those rows keeps their choice, even after it changed again.
    controller.set_hide_hidden(true);
    resort(&controller, 3);
    controller.set_visible(listed(&files, 3, false));
    quiet(&files, &mut events).await;
    put_left(&files, vec![shown(7), hidden(2)]);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(3)));
    // The re-read with the new choice leaves the hidden entry out again.
    list(&controller, 4);
    controller.set_visible(listed(&files, 4, true));
    let probes = files.probes.load(Ordering::SeqCst);
    files.wait_probes(probes + 3).await;
    while let Ok(event) = events.try_recv() {
        // Hints for the replaced rows may still be queued.
        assert!(
            matches!(
                event,
                Event::DirectoryChanged {
                    navigation: NavigationId(3),
                    ..
                }
            ),
            "{event:?}"
        );
    }
    quiet(&files, &mut events).await;
    put_left(&files, vec![shown(7), hidden(3)]);
    quiet(&files, &mut events).await;
    stop(&controller, &mut tasks).await;
}
