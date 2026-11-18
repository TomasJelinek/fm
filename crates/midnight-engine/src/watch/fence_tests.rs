//! Listing-start fences, relisting and folders subscribed while they are listed,
//! in the backend's order: it records a dispatched listing or resort, then
//! clears the pane's watch scope; the listing reports when it read entries, and
//! later the UI accepts the new view or the listing is reported failed.
//!
//! Unless a test sets its own interval, every watcher here polls only after
//! 30 s, so each probe a test counts is an immediate check after an acceptance.
//! Such a probe runs before the worker can hint a notification that arrives
//! after the acceptance subscribed again, so that hint is the barrier before a
//! test reads the probe count.
use super::*;

fn watcher(
    files: &Arc<Files>,
) -> (
    midnight_work_budget::resident::Owner,
    Controller,
    JoinSet<()>,
    mpsc::Receiver<Event>,
    Arc<Native>,
) {
    let native = Arc::new(Native::default());
    let (owner, controller, tasks, events) =
        controller(files.clone(), Some(native.clone()), Duration::from_secs(30));
    (owner, controller, tasks, events, native)
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
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
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
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
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
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
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
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
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
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
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
async fn a_check_that_finds_the_rows_current_spares_a_later_resort() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    // The refresh lists the directory after its last write, whose notification
    // is counted only after the listing started.
    list(&controller, 2, "left");
    files.put("left", 8);
    native.fire(0);
    // Another notification is counted while the accepted rows are subscribed
    // again; it is hinted after the check of the accepted rows.
    native.fire_on_watch.store(true, Ordering::SeqCst);
    controller.set_visible(files.visible(PaneId::Left, 2, "left"));
    hint_for(&mut events, 2).await;
    assert_eq!(probes(&files), 1, "the accepted rows are checked once");
    // That check found the rows current, so a resort of them is not checked.
    resort(&controller, 3, "left");
    controller.set_visible(files.visible(PaneId::Left, 3, "left"));
    native.registrations(3).await;
    native.fire(2);
    hint_for(&mut events, 3).await;
    assert_eq!(probes(&files), 1);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_refresh_keeps_hinting_for_the_rows_still_shown() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    // The refresh fails, and the pane keeps showing navigation 1.
    list(&controller, 2, "left");
    controller
        .reporter()
        .listing_failed(PaneId::Left, NavigationId(2));
    native.fire(0);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    assert_eq!(
        native.callbacks.lock().unwrap().len(),
        1,
        "the directory stays subscribed"
    );
    assert_eq!(
        probes(&files),
        0,
        "a failed refresh is not checked at once, so it is not retried in a loop"
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_refresh_keeps_checking_the_rows_still_shown() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let native = Arc::new(Native::default());
    let (_owner, controller, mut tasks, mut events) = controller(
        files.clone(),
        Some(native.clone()),
        Duration::from_millis(40),
    );
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    list(&controller, 2, "left");
    // A change without a notification, while that refresh fails.
    files.put("left", 8);
    controller
        .reporter()
        .listing_failed(PaneId::Left, NavigationId(2));
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_superseded_listing_failure_does_not_rearm_the_watch() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    list(&controller, 2, "left");
    list(&controller, 3, "left");
    let stale = files.visible(PaneId::Left, 3, "left");
    // The superseded listing reports after the newer dispatch; the directory
    // stays counting for the newer listing, without hints.
    controller
        .reporter()
        .listing_failed(PaneId::Left, NavigationId(2));
    files.put("left", 8);
    native.fire(0);
    assert!(
        tokio::time::timeout(Duration::from_millis(100), events.recv())
            .await
            .is_err(),
        "no hint for the rows of navigation 1 while navigation 3 loads"
    );
    // The newer listing lacks the change and is checked once accepted.
    controller.set_visible(stale);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(3)));
    assert_eq!(probes(&files), 1);
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn listing_another_directory_keeps_the_shown_rows_watched_until_it_is_accepted() {
    let files = Arc::new(Files::default());
    let (_owner, controller, mut tasks, _events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    list(&controller, 2, "right");
    native.mark("accepted");
    controller.set_visible(files.visible(PaneId::Left, 2, "right"));
    native.released(1).await;
    // The shown rows are released only after the accepted folder is watched.
    assert_eq!(
        native.history(),
        [
            Watched::Registered("left".into()),
            Watched::Mark("accepted"),
            Watched::Registered("right".into()),
            Watched::Dropped("left".into()),
        ]
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_navigation_to_another_directory_keeps_hinting_for_the_rows_still_shown() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    // Navigating elsewhere fails, and the pane keeps showing navigation 1.
    list(&controller, 2, "right");
    controller
        .reporter()
        .listing_failed(PaneId::Left, NavigationId(2));
    native.fire(0);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    assert_eq!(probes(&files), 0);
    assert_eq!(
        native.drops.load(Ordering::SeqCst),
        0,
        "the shown directory stays subscribed"
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rows_accepted_while_the_worker_is_busy_are_watched_when_the_next_navigation_fails() {
    let files = Arc::new(Files {
        blocked_path: Some("blocked".into()),
        ignore_cancel: true,
        ..Files::default()
    });
    let release = ReleaseOnDrop(files.clone());
    files.put("left", 7);
    let native = Arc::new(Native::default());
    let (_owner, controller, mut tasks, mut events) = controller(
        files.clone(),
        Some(native.clone()),
        Duration::from_millis(30),
    );
    controller.set_visible(files.visible(PaneId::Left, 1, "blocked"));
    tokio::time::timeout(Duration::from_secs(10), files.blocked.notified())
        .await
        .expect("the first periodic check must run");
    // While the worker is blocked, the UI accepts "left" and navigates on, so
    // the worker sees that acceptance only after the next dispatch.
    controller.set_visible(files.visible(PaneId::Left, 2, "left"));
    list(&controller, 3, "right");
    drop(release);
    controller
        .reporter()
        .listing_failed(PaneId::Left, NavigationId(3));
    // The rows the pane shows are watched, not those of navigation 1.
    native.registrations(2).await;
    assert_eq!(native.callbacks.lock().unwrap()[1].0, Path::new("left"));
    native.fire(1);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(2)));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restart_keeps_hinting_for_the_rows_a_failed_navigation_shows() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let native = Arc::new(Native::default());
    let (_owner, controller, mut tasks, mut events) = controller(
        files.clone(),
        Some(native.clone()),
        Duration::from_millis(40),
    );
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    list(&controller, 2, "right");
    // The next check of the rows still shown panics, and the worker restarts.
    files.panic_probes.store(1, Ordering::SeqCst);
    controller
        .reporter()
        .listing_failed(PaneId::Left, NavigationId(2));
    native.registrations(2).await;
    assert_eq!(native.callbacks.lock().unwrap()[1].0, Path::new("left"));
    native.fire(1);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_navigated_directory_is_subscribed_while_it_is_listed() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    files.put("right", 7);
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    list(&controller, 2, "right");
    let listed = files.visible(PaneId::Left, 2, "right");
    controller
        .reporter()
        .listing_read(PaneId::Left, NavigationId(2));
    native.registrations(2).await;
    assert_eq!(native.callbacks.lock().unwrap()[1].0, Path::new("right"));
    native.mark("accepted");
    controller.set_visible(listed);
    native.released(1).await;
    // Adopted without registering it again; the shown rows were released
    // after it.
    assert_eq!(
        native.history(),
        [
            Watched::Registered("left".into()),
            Watched::Registered("right".into()),
            Watched::Mark("accepted"),
            Watched::Dropped("left".into()),
        ]
    );
    native.fire(1);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(2)));
    assert_eq!(probes(&files), 0, "a quiet listing is not checked again");
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_change_notified_while_a_navigated_directory_is_listed_is_checked_once_shown() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    files.put("right", 7);
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    list(&controller, 2, "right");
    let listed = files.visible(PaneId::Left, 2, "right");
    controller
        .reporter()
        .listing_read(PaneId::Left, NavigationId(2));
    native.registrations(2).await;
    // A change after the listing read the folder is notified while it loads.
    files.put("right", 8);
    native.fire(1);
    native.mark("accepted");
    controller.set_visible(listed);
    hint_for(&mut events, 2).await;
    assert_eq!(probes(&files), 1, "the accepted rows are checked once");
    // The folder may have been replaced while it was listed, so it is
    // subscribed again, keeping its counter, before the prepared subscription
    // and then the shown rows are released.
    native.released(2).await;
    assert_eq!(
        native.history(),
        [
            Watched::Registered("left".into()),
            Watched::Registered("right".into()),
            Watched::Mark("accepted"),
            Watched::Registered("right".into()),
            Watched::Dropped("right".into()),
            Watched::Dropped("left".into()),
        ]
    );
    native.fire(2);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(2)));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dispatch_whose_listing_reads_nothing_registers_only_when_accepted() {
    let files = Arc::new(Files::default());
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    // The backend records a dispatch before it clears the pane's scope. An
    // empty, unreadable or blocked folder never reports entries read.
    controller.listing_started(PaneId::Left, NavigationId(2), Path::new("right"));
    // The still current rows hint a notification. The worker looked at that
    // dispatch before it sent the hint, so nothing was subscribed for it.
    native.fire(0);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    assert_eq!(native.callbacks.lock().unwrap().len(), 1);
    controller
        .handle()
        .clear_scope(PaneId::Left, NavigationId(2));
    native.mark("accepted");
    controller.set_visible(files.visible(PaneId::Left, 2, "right"));
    native.registrations(2).await;
    native.fire(1);
    hint_for(&mut events, 2).await;
    assert_eq!(
        native.history(),
        [
            Watched::Registered("left".into()),
            Watched::Mark("accepted"),
            Watched::Registered("right".into()),
            Watched::Dropped("left".into()),
        ]
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_failed_navigation_releases_the_directory_subscribed_while_it_was_listed() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    list(&controller, 2, "right");
    controller
        .reporter()
        .listing_read(PaneId::Left, NavigationId(2));
    native.registrations(2).await;
    controller
        .reporter()
        .listing_failed(PaneId::Left, NavigationId(2));
    native.released(1).await;
    assert_eq!(
        native.history().last(),
        Some(&Watched::Dropped("right".into()))
    );
    native.fire(0);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(1)));
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_dispatch_back_to_the_shown_directory_releases_the_subscribed_one() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    let (_owner, controller, mut tasks, mut events, native) = watcher(&files);
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    list(&controller, 2, "right");
    controller
        .reporter()
        .listing_read(PaneId::Left, NavigationId(2));
    native.registrations(2).await;
    list(&controller, 3, "left");
    native.released(1).await;
    assert_eq!(
        native.history().last(),
        Some(&Watched::Dropped("right".into()))
    );
    // The shown rows were counted for the listing's fence and are retargeted.
    controller.set_visible(files.visible(PaneId::Left, 3, "left"));
    native.registrations(3).await;
    assert_eq!(native.callbacks.lock().unwrap()[2].0, Path::new("left"));
    native.fire(2);
    assert_eq!(changed(&mut events).await, (PaneId::Left, NavigationId(3)));
    assert_eq!(probes(&files), 0);
    stop(&controller, &mut tasks).await;
}

/// The "left" folder, whose reads block until released.
fn blocked_files() -> (Arc<Files>, ReleaseOnDrop) {
    let files = Arc::new(Files {
        blocked_path: Some("left".into()),
        ..Files::default()
    });
    files.put("left", 7);
    (files.clone(), ReleaseOnDrop(files))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_check_interrupted_by_a_resort_runs_when_the_resort_is_shown() {
    let (files, _release) = blocked_files();
    let native = Arc::new(Native::default());
    let (_owner, controller, mut tasks, mut events) = controller(
        files.clone(),
        Some(native.clone()),
        Duration::from_millis(200),
    );
    let shown = files.visible(PaneId::Left, 1, "left");
    controller.set_visible(shown.clone());
    native.registrations(1).await;
    tokio::time::timeout(Duration::from_secs(10), files.blocked.notified())
        .await
        .expect("the first periodic check must run");
    files.put("left", 8);
    // The resort supersedes the check; its probe returns without a result.
    resort(&controller, 2, "left");
    files.release.store(true, Ordering::SeqCst);
    controller.set_visible(VisibleDirectory {
        navigation: NavigationId(2),
        ..shown
    });
    native.registrations(2).await;
    native.fire(1);
    hint_for(&mut events, 2).await;
    // A slow test thread can also see the next periodic check (200 ms later);
    // without the check at once only the interrupted one precedes the hint.
    let probes = probes(&files);
    assert!(
        probes >= 2,
        "the resorted rows are no newer than the interrupted check, which runs at once: {probes} probes"
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_refresh_that_interrupts_a_check_is_not_listed_again() {
    let (files, _release) = blocked_files();
    let native = Arc::new(Native::default());
    let (_owner, controller, mut tasks, mut events) = controller(
        files.clone(),
        Some(native.clone()),
        Duration::from_millis(500),
    );
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    tokio::time::timeout(Duration::from_secs(10), files.blocked.notified())
        .await
        .expect("the first periodic check must run");
    files.put("left", 8);
    list(&controller, 2, "left");
    files.release.store(true, Ordering::SeqCst);
    // The refresh lists the folder after the change.
    controller.set_visible(files.visible(PaneId::Left, 2, "left"));
    native.registrations(2).await;
    native.fire(1);
    hint_for(&mut events, 2).await;
    assert_eq!(
        probes(&files),
        1,
        "the refresh includes whatever the interrupted check could have found"
    );
    stop(&controller, &mut tasks).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_view_without_a_flat_listing_releases_the_watch_at_dispatch() {
    let files = Arc::new(Files::default());
    let (_owner, controller, mut tasks, _events, native) = watcher(&files);
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
