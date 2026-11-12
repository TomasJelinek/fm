use super::Catalog;
use std::{
    cell::Cell,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

const DEADLOCK_GUARD: Duration = Duration::from_secs(5);

struct Release(Option<mpsc::Sender<()>>);
impl Release {
    fn finish(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        self.finish();
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Activity {
    Waiting,
    Discovering,
}

fn names(value: &str) -> Arc<[String]> {
    vec![value.to_owned()].into()
}

fn waiting_request(
    catalog: &Catalog,
    cancelled: &AtomicBool,
    activity: &mpsc::Sender<Activity>,
    discoveries: &AtomicUsize,
) -> Option<Arc<[String]>> {
    let polls = Cell::new(0);
    catalog.get(
        || {
            polls.set(polls.get() + 1);
            if polls.get() == 2 {
                activity.send(Activity::Waiting).unwrap();
            }
            cancelled.load(Ordering::Acquire)
        },
        || {
            discoveries.fetch_add(1, Ordering::Relaxed);
            activity.send(Activity::Discovering).unwrap();
            Some(names("Consolas"))
        },
    )
}

#[test]
fn concurrent_misses_share_one_complete_discovery() {
    let catalog = Catalog::new();
    let discoveries = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let (release, held) = mpsc::channel();
    let (started, begun) = mpsc::channel();
    let (activity, observed) = mpsc::channel();
    thread::scope(|threads| {
        let mut release = Release(Some(release));
        let catalog = &catalog;
        let discoveries = &discoveries;
        let owner = threads.spawn(move || {
            catalog.get(
                || false,
                || {
                    discoveries.fetch_add(1, Ordering::Relaxed);
                    started.send(()).unwrap();
                    held.recv().unwrap();
                    Some(names("Cascadia Mono"))
                },
            )
        });
        begun.recv_timeout(DEADLOCK_GUARD).unwrap();
        let cancelled = &cancelled;
        let activity = &activity;
        let contender =
            threads.spawn(move || waiting_request(catalog, cancelled, activity, discoveries));
        assert_eq!(
            observed.recv_timeout(DEADLOCK_GUARD).unwrap(),
            Activity::Waiting,
            "a concurrent miss must wait instead of starting a second font scan"
        );
        release.finish();
        let first = owner.join().unwrap().unwrap();
        let second = contender.join().unwrap().unwrap();
        assert_eq!(&*first, &["Cascadia Mono"]);
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(discoveries.load(Ordering::Relaxed), 1);
    });
}

#[test]
fn waiting_request_can_cancel_before_the_active_discovery_finishes() {
    let catalog = Catalog::new();
    let discoveries = AtomicUsize::new(0);
    let cancelled = AtomicBool::new(false);
    let (release, held) = mpsc::channel();
    let (started, begun) = mpsc::channel();
    let (activity, observed) = mpsc::channel();
    let (completed, result) = mpsc::channel();
    thread::scope(|threads| {
        let mut release = Release(Some(release));
        let catalog = &catalog;
        let owner = threads.spawn(move || {
            catalog.get(
                || false,
                || {
                    started.send(()).unwrap();
                    held.recv().unwrap();
                    Some(names("Cascadia Mono"))
                },
            )
        });
        begun.recv_timeout(DEADLOCK_GUARD).unwrap();
        let cancelled = &cancelled;
        let activity = &activity;
        let discoveries = &discoveries;
        let contender = threads.spawn(move || {
            completed
                .send(waiting_request(catalog, cancelled, activity, discoveries))
                .unwrap();
        });
        assert_eq!(
            observed.recv_timeout(DEADLOCK_GUARD).unwrap(),
            Activity::Waiting
        );
        cancelled.store(true, Ordering::Release);
        assert!(result.recv_timeout(DEADLOCK_GUARD).unwrap().is_none());
        assert!(
            !owner.is_finished(),
            "cancellation must not wait for the scan"
        );
        contender.join().unwrap();
        assert_eq!(discoveries.load(Ordering::Relaxed), 0);
        release.finish();
        assert!(owner.join().unwrap().is_some());
    });
}

#[test]
fn cancelled_initializer_does_not_publish_its_catalog_and_a_waiter_can_retry() {
    let catalog = Catalog::new();
    let owner_cancelled = AtomicBool::new(false);
    let waiter_cancelled = AtomicBool::new(false);
    let discoveries = AtomicUsize::new(0);
    let (release, held) = mpsc::channel();
    let (started, begun) = mpsc::channel();
    let (activity, observed) = mpsc::channel();
    thread::scope(|threads| {
        let mut release = Release(Some(release));
        let catalog = &catalog;
        let owner_cancelled = &owner_cancelled;
        let owner = threads.spawn(move || {
            catalog.get(
                || owner_cancelled.load(Ordering::Acquire),
                || {
                    started.send(()).unwrap();
                    held.recv().unwrap();
                    Some(names("Discard this cancelled discovery"))
                },
            )
        });
        begun.recv_timeout(DEADLOCK_GUARD).unwrap();
        let waiter_cancelled = &waiter_cancelled;
        let activity = &activity;
        let discoveries = &discoveries;
        let contender = threads
            .spawn(move || waiting_request(catalog, waiter_cancelled, activity, discoveries));
        assert_eq!(
            observed.recv_timeout(DEADLOCK_GUARD).unwrap(),
            Activity::Waiting
        );
        owner_cancelled.store(true, Ordering::Release);
        release.finish();
        assert!(owner.join().unwrap().is_none());
        let recovered = contender.join().unwrap().unwrap();
        assert_eq!(&*recovered, &["Consolas"]);
        assert_eq!(discoveries.load(Ordering::Relaxed), 1);
        assert!(Arc::ptr_eq(
            &recovered,
            &catalog.get(|| false, || panic!("already cached")).unwrap()
        ));
    });
}

#[test]
fn incomplete_and_panicking_initializers_leave_the_catalog_retryable() {
    let catalog = Catalog::new();
    assert!(catalog.get(|| false, || None).is_none());
    assert!(
        std::panic::catch_unwind(|| {
            catalog.get(|| false, || panic!("interrupted font discovery"))
        })
        .is_err()
    );
    let recovered = catalog.get(|| false, || Some(names("Consolas"))).unwrap();
    assert_eq!(&*recovered, &["Consolas"]);
    assert!(Arc::ptr_eq(
        &recovered,
        &catalog.get(|| false, || panic!("already cached")).unwrap()
    ));
}

#[test]
fn cancellation_is_checked_even_for_a_cached_catalog() {
    let catalog = Catalog::new();
    catalog.get(|| false, || Some(names("Consolas"))).unwrap();
    assert!(
        catalog
            .get(|| true, || panic!("cancelled discovery must never start"))
            .is_none()
    );
}
