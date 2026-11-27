//! Rows the authorization helper read (`Visible::authorized`).
use super::*;

/// The backend's dispatch of a listing of `path` as `navigation`.
fn dispatch(controller: &Controller, navigation: u64, path: &str, authorized: bool) {
    let navigation = NavigationId(navigation);
    if authorized {
        controller.authorized_listing_started(PaneId::Left, navigation, Path::new(path));
    } else {
        controller.listing_started(PaneId::Left, navigation, Path::new(path));
    }
    controller.handle().clear_scope(PaneId::Left, navigation);
}

/// A check of a folder that this process may not read fails, and a failed check
/// counts as a change: its hint re-read the folder with ordinary rights, which was
/// denied, and the pane recovered to the parent folder. Rows the authorization
/// helper read, and a re-sort of them, are neither subscribed nor checked, and the
/// target of the rows shown before is released. Rows listed with ordinary rights
/// later are watched again.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn rows_the_authorization_helper_read_are_neither_subscribed_nor_checked() {
    let files = Arc::new(Files::default());
    files.put("left", 7);
    files.put("denied", 3);
    let native = Arc::new(Native::default());
    let (_owner, controller, mut tasks, mut events) = controller(
        files.clone(),
        Some(native.clone()),
        Duration::from_millis(20),
    );
    controller.set_visible(files.visible(PaneId::Left, 1, "left"));
    native.registrations(1).await;
    files.wait_probes(1).await;

    dispatch(&controller, 2, "denied", true);
    controller.set_visible(files.visible(PaneId::Left, 2, "denied"));
    native.released(1).await;
    // From now on, a check of either folder is denied.
    files.listing_failed.store(true, Ordering::SeqCst);
    let probes = files.probes.load(Ordering::SeqCst);
    // A re-sort shows the same rows.
    controller.resort_started(PaneId::Left, NavigationId(3), Path::new("denied"));
    controller.rows_resorted(PaneId::Left, NavigationId(2), NavigationId(3));
    controller
        .handle()
        .clear_scope(PaneId::Left, NavigationId(3));
    controller.set_visible(files.visible(PaneId::Left, 3, "denied"));
    // Ten check periods: no check, no hint, no status and no registration.
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        files.probes.load(Ordering::SeqCst),
        probes,
        "a folder the authorization helper listed was checked with ordinary rights"
    );
    if let Ok(event) = events.try_recv() {
        panic!("rows the authorization helper read were hinted: {event:?}");
    }
    assert_eq!(
        native.history(),
        [
            Watched::Registered("left".into()),
            Watched::Dropped("left".into()),
        ]
    );

    // Access was granted meanwhile: an ordinary listing's rows are watched again.
    files.listing_failed.store(false, Ordering::SeqCst);
    dispatch(&controller, 4, "denied", false);
    controller.set_visible(files.visible(PaneId::Left, 4, "denied"));
    native.registrations(2).await;
    files.wait_probes(probes + 1).await;
    assert_eq!(
        native.history()[2..],
        [Watched::Registered("denied".into())]
    );
    stop(&controller, &mut tasks).await;
}
