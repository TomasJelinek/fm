//! Listing-start fences and same-directory relisting, in the backend's order:
//! it records a dispatched listing or resort, then clears the pane's watch
//! scope, and later accepts the new view.
//!
//! Every watcher here polls only after 30 s, so each probe a test counts is an
//! immediate check after an acceptance. Such a probe runs before the worker can
//! hint a notification that arrives after the acceptance subscribed again, so
//! that hint is the barrier before a test reads the probe count.
use super::*;

fn watcher(files: &Arc<Files>) -> (Controller, JoinSet<()>, mpsc::Receiver<Event>, Arc<Native>) {
    let native = Arc::new(Native::default());
    let (controller, tasks, events) =
        controller(files.clone(), Some(native.clone()), Duration::from_secs(30));
    (controller, tasks, events, native)
}

fn probes(files: &Files) -> usize {
    files.probes.load(Ordering::SeqCst)
}

/// The backend's dispatch of a Navigate/Refresh listing of `path`.
fn list(controller: &Controller, navigation: u64, path: &str) {
    let navigation = NavigationId(navigation);
    controller.listing_started(PaneId::Left, navigation, Path::new(path));
    controller.handle().clear_scope(PaneId::Left, navigation);
}

/// The backend's dispatch of a Resort of `path`.
fn resort(controller: &Controller, navigation: u64, path: &str) {
    let navigation = NavigationId(navigation);
    controller.resort_started(PaneId::Left, navigation, Path::new(path));
    controller.handle().clear_scope(PaneId::Left, navigation);
}

/// Waits for a hint for `navigation`. Hints for the navigation it replaced are
/// skipped: the UI ignores them while it loads the new view.
async fn hint_for(events: &mut mpsc::Receiver<Event>, navigation: u64) {
    loop {
        let (pane, hinted) = changed(events).await;
        assert_eq!(pane, PaneId::Left);
        if hinted == NavigationId(navigation) {
            return;
        }
        assert!(hinted < NavigationId(navigation), "hint for {hinted:?}");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_event_driven_refresh_without_later_changes_is_not_probed_again() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let (controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    native.fire(0);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    // The UI refreshes for the hint.
    list(&controller, 2, "left");
    controller.set_visible(files.visible(PaneId::Left, 2, "left"));
    native.registrations(2).await;
    native.fire(1);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(2)));
    assert_eq!(
        probes(&files),
        0,
        "the refresh listing already includes the notified change"
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_during_a_same_path_refresh_are_checked_against_the_accepted_rows() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let (controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    native.fire(0);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    // The refresh lists the directory, then a final write lands and notifies
    // before the UI accepts that already stale listing. The directory stays
    // subscribed meanwhile.
    list(&controller, 2, "left");
    let stale = files.visible(PaneId::Left, 2, "left");
    files.put("left", 8);
    native.fire(0);
    controller.set_visible(stale);
    hint_for(&mut events, 2).await;
    assert_eq!(probes(&files), 1);
    // The directory is subscribed again (it may have been replaced) before the
    // previous subscription is released, and the new one reports the new
    // navigation.
    let callbacks = native.callbacks.lock().unwrap().clone();
    assert_eq!(callbacks.len(), 2);
    assert!(callbacks.iter().all(|(path, _)| path == Path::new("left")));
    assert_eq!(native.drops.load(Ordering::SeqCst), 1);
    native.fire(1);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(2)));
    stop(&controller, &mut tasks).await;
    assert_eq!(native.drops.load(Ordering::SeqCst), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refresh_started_before_a_hinted_change_is_checked_once_accepted() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let (controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    // The user refreshes before anything changed; the listing reads the old
    // contents.
    controller.listing_started(PaneId::Left, NavigationId(2), Path::new("left"));
    let stale = files.visible(PaneId::Left, 2, "left");
    // A change is hinted before the backend clears the pane's watch scope. The
    // UI, already loading, ignores that hint.
    files.put("left", 8);
    native.fire(0);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    controller
        .handle()
        .clear_scope(PaneId::Left, NavigationId(2));
    // A fence taken when the hint was sent would cover the change; the
    // listing-start fence does not.
    controller.set_visible(stale);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(2)));
    assert_eq!(probes(&files), 1);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_found_by_a_probe_during_a_refresh_listing_is_checked_once_accepted() {
    let files = Arc::new(Files {
        blocked_path: Some("left".into()),
        ..Files::default()
    });
    let _release = ReleaseOnDrop(files.clone());
    files.put("left", 7);
    let (controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    native.fire(0);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    // A resort is accepted while that notification is unchecked, so its rows
    // are probed at once; the probe blocks.
    resort(&controller, 2, "left");
    controller.set_visible(files.visible(PaneId::Left, 2, "left"));
    tokio::time::timeout(Duration::from_secs(3), files.blocked.notified())
        .await
        .expect("the resorted rows must be probed at once");
    // Meanwhile a refresh starts listing the old contents, and then the
    // directory changes without a notification.
    controller.listing_started(PaneId::Left, NavigationId(3), Path::new("left"));
    let stale = files.visible(PaneId::Left, 3, "left");
    files.put("left", 8);
    // The probe finds the change before the backend clears the scope; the UI
    // ignores its hint while it loads.
    files.release.store(true, Ordering::SeqCst);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(2)));
    controller
        .handle()
        .clear_scope(PaneId::Left, NavigationId(3));
    // The found change is counted after the refresh's fence.
    controller.set_visible(stale);
    hint_for(&mut events, 3).await;
    assert_eq!(probes(&files), 2);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resort_keeps_what_its_rows_include() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let (controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    native.fire(0);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    list(&controller, 2, "left");
    controller.set_visible(files.visible(PaneId::Left, 2, "left"));
    native.registrations(2).await;
    // The resort shows the refreshed rows again, which include the change.
    resort(&controller, 3, "left");
    controller.set_visible(files.visible(PaneId::Left, 3, "left"));
    native.registrations(3).await;
    native.fire(2);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(3)));
    assert_eq!(probes(&files), 0);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listing_another_directory_releases_the_watch_at_dispatch() {
    let files = Arc::new(Files::default());
    let (controller, mut tasks, _events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    list(&controller, 2, "right");
    native.released(1).await;
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_view_without_a_flat_listing_releases_the_watch_at_dispatch() {
    let files = Arc::new(Files::default());
    let (controller, mut tasks, _events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    list(&controller, 2, "left");
    controller.set_visible(files.visible(PaneId::Left, 2, "left"));
    native.registrations(2).await;
    native.released(1).await;
    // An archive or network view records no flat listing, and the record of
    // the earlier listing does not keep the watch.
    controller
        .handle()
        .clear_scope(PaneId::Left, NavigationId(3));
    native.released(2).await;
    stop(&controller, &mut tasks).await;
}
