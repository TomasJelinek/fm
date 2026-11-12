use super::*;
use crate::native_output_test_fixture as fixture;
use std::{thread, time::Duration};

#[test]
fn queued_size_root_retires_off_caller_after_worker_unwind() {
    let (directory, owner, dropped) = fixture::owner();
    let listing = fixture::archive_listing(owner, directory.path());
    let mut state = crate::SnapshotState::default();
    state.remember_archive_tree_root(PaneId::Left, midnight_core::NavigationId(1), &listing);
    let ready = state.tree_root.take().unwrap();
    drop(listing);
    drop(state);
    let lane = Arc::new(Lane::default());
    let worker = lane.clone();
    let result = thread::spawn(move || {
        worker.pending.lock().unwrap().queue.push_back(Request {
            ready,
            scan: FolderSizeScanId(1),
            selected: None,
            single: false,
        });
        panic!("injected metadata worker unwind before lane drain");
    })
    .join();
    assert!(result.is_err());
    lane.close();
    let caller = thread::current().id();
    drop(lane);
    assert_ne!(
        dropped.recv_timeout(Duration::from_secs(5)).unwrap(),
        caller,
        "undrained size root's native file owner must retire off caller"
    );
}

#[test]
fn pending_lane_keeps_its_cleanup_slot_until_native_destruction_finishes() {
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let lane = Arc::new(Lane::new(domain.try_reserve().unwrap()));
    let (directory, mut owner, dropped) = fixture::owner();
    let (release, gate) = std::sync::mpsc::channel();
    Arc::get_mut(&mut owner).unwrap().hold_drop(gate);
    let listing = fixture::archive_listing(owner, directory.path());
    let mut state = crate::SnapshotState::default();
    state.remember_archive_tree_root(PaneId::Left, midnight_core::NavigationId(1), &listing);
    lane.pending.lock().unwrap().queue.push_back(Request {
        ready: state.tree_root.take().unwrap(),
        scan: FolderSizeScanId(1),
        selected: None,
        single: false,
    });
    drop(state);
    drop(listing);
    lane.close();
    drop(lane);
    let cleanup_thread = dropped.recv_timeout(Duration::from_secs(5)).unwrap();
    let occupied = domain.try_reserve().is_err();
    release.send(()).unwrap();
    assert_ne!(cleanup_thread, thread::current().id());
    assert!(occupied, "native cleanup still owns the lane's only slot");
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    loop {
        if let Ok(lease) = domain.try_reserve() {
            drop(lease);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "cleanup must release its slot"
        );
        thread::yield_now();
    }
}
