use midnight_thumbnail_host::scheduler::{Item, Scheduler};

fn item(token: u64, selected: bool, hovered: bool) -> Item {
    Item {
        token,
        edge: 96,
        video: true,
        selected,
        hovered,
    }
}

#[test]
fn larger_visible_request_upgrades_only_insufficient_frames_and_smaller_reuses_completion() {
    let mut scheduler = Scheduler::default();
    scheduler.set_view(1, vec![item(1, false, false), item(2, false, false)]);
    while let Some(work) = scheduler.next_work() {
        assert!(scheduler.complete(work, true));
    }
    let mut larger = item(1, false, false);
    larger.edge = 256;
    scheduler.set_view(1, vec![larger, item(2, false, false)]);
    let upgrade = scheduler
        .next_work()
        .expect("the larger target needs an individual upgrade");
    assert_eq!((upgrade.token, upgrade.edge, upgrade.frame), (1, 256, 0));
    assert!(
        scheduler.next_work().is_none(),
        "the other completed thumbnail remains sufficient"
    );
    assert!(scheduler.complete(upgrade, true));
    scheduler.set_view(1, vec![item(1, false, false), item(2, false, false)]);
    assert!(
        scheduler.next_work().is_none(),
        "smaller display must reuse prior coverage"
    );
}

#[test]
fn stale_small_completion_never_satisfies_an_upgrade_or_releases_capacity_early() {
    let mut scheduler = Scheduler::default();
    scheduler.set_view(1, vec![item(1, false, false)]);
    let small = scheduler.next_work().unwrap();
    let mut larger = item(1, false, false);
    larger.edge = 512;
    scheduler.set_view(1, vec![larger]);
    assert!(
        scheduler.next_work().is_none(),
        "the old worker still owns its slot"
    );
    assert!(!scheduler.complete(small, true));
    let upgrade = scheduler.next_work().unwrap();
    assert_eq!(upgrade.edge, 512);
    assert!(scheduler.complete(upgrade, true));
    assert!(scheduler.next_work().is_none());
}

#[test]
fn invalid_resolution_does_not_replace_the_current_view() {
    let mut scheduler = Scheduler::default();
    assert!(scheduler.set_view(1, vec![item(1, false, false)]));
    for edge in [0, 513] {
        let mut invalid = item(2, false, false);
        invalid.edge = edge;
        assert!(!scheduler.set_view(1, vec![invalid]));
    }
    assert_eq!(scheduler.next_work().unwrap().token, 1);
}

#[test]
fn failed_large_upgrade_does_not_discard_or_block_a_completed_smaller_thumbnail() {
    let mut scheduler = Scheduler::default();
    scheduler.set_view(1, vec![item(1, false, false)]);
    let small = scheduler.next_work().unwrap();
    assert!(scheduler.complete(small, true));
    let mut larger = item(1, false, false);
    larger.edge = 256;
    scheduler.set_view(1, vec![larger]);
    let upgrade = scheduler.next_work().unwrap();
    scheduler.complete(upgrade, false);
    assert!(scheduler.next_work().is_none());
    scheduler.set_view(1, vec![item(1, false, false)]);
    assert!(scheduler.next_work().is_none());
}

#[test]
fn rejected_upload_retries_only_when_requested_and_preserves_retained_coverage() {
    let mut scheduler = Scheduler::default();
    let mut larger = item(1, false, false);
    larger.edge = 256;
    scheduler.set_view(1, vec![larger, item(2, false, false)]);
    let rejected = scheduler.next_work().unwrap();
    assert_eq!(rejected.token, 1);
    assert!(scheduler.complete(rejected, true));
    let other = scheduler.next_work().unwrap();
    assert!(scheduler.complete(other, true));
    assert!(
        scheduler.next_work().is_none(),
        "no repeated upload while the view is full"
    );

    scheduler.set_view(1, vec![larger]);
    assert!(scheduler.retry_delivery(rejected, 96));
    let retry = scheduler
        .next_work()
        .expect("space can be reclaimed after another row leaves");
    assert_eq!(retry, rejected);
    assert!(
        scheduler.next_work().is_none(),
        "retry still consumes its worker slot"
    );
    assert!(scheduler.complete(retry, true));
    scheduler.set_view(1, vec![item(1, false, false)]);
    assert!(scheduler.retry_delivery(retry, 96));
    assert!(
        scheduler.next_work().is_none(),
        "a retained fallback covers the smaller view"
    );
}

#[test]
fn stale_rejected_upload_cannot_discard_newer_coverage_or_generation() {
    let mut scheduler = Scheduler::default();
    scheduler.set_view(1, vec![item(1, false, false)]);
    let small = scheduler.next_work().unwrap();
    scheduler.complete(small, true);
    let mut larger = item(1, false, false);
    larger.edge = 512;
    scheduler.set_view(1, vec![larger]);
    let large = scheduler.next_work().unwrap();
    scheduler.complete(large, true);
    assert!(!scheduler.retry_delivery(small, 0));
    assert!(scheduler.next_work().is_none());
    scheduler.set_view(2, vec![larger]);
    let current = scheduler.next_work().unwrap();
    scheduler.complete(current, true);
    assert!(!scheduler.retry_delivery(large, 0));
    assert!(!scheduler.retry_delivery(
        midnight_thumbnail_host::scheduler::Work {
            frame: 6,
            ..current
        },
        0
    ));
    assert!(!scheduler.retry_delivery(current, 512));
    assert!(scheduler.next_work().is_none());
}

#[test]
fn ordinary_video_has_one_static_frame() {
    let mut scheduler = Scheduler::default();
    scheduler.set_view(1, vec![item(1, false, false)]);
    let work = scheduler.next_work().unwrap();
    assert_eq!(work.frame, 0);
    assert!(scheduler.complete(work, true));
    assert!(scheduler.next_work().is_none());
}

#[test]
fn deselecting_cancels_animation_without_poisoning_a_later_selection() {
    let mut scheduler = Scheduler::default();
    scheduler.set_view(1, vec![item(1, true, false)]);
    let still = scheduler.next_work().unwrap();
    scheduler.complete(still, true);
    let animation = scheduler.next_work().unwrap();
    assert_eq!(animation.frame, 1);
    scheduler.set_view(1, vec![item(1, false, false)]);
    scheduler.abandon(animation);
    assert!(scheduler.next_work().is_none());
    scheduler.set_view(1, vec![item(1, true, false)]);
    assert_eq!(scheduler.next_work().unwrap().frame, 1);
}

#[test]
fn hover_has_priority_and_all_selected_videos_make_fair_progress() {
    let mut scheduler = Scheduler::default();
    scheduler.set_view(
        1,
        vec![
            item(1, true, false),
            item(2, true, false),
            item(3, false, true),
        ],
    );
    let first = scheduler.next_work().unwrap();
    assert_eq!((first.token, first.frame), (3, 0));
    scheduler.complete(first, true);
    let mut frames = std::collections::HashMap::<u64, Vec<u8>>::new();
    frames.entry(3).or_default().push(0);
    while let Some(work) = scheduler.next_work() {
        frames.entry(work.token).or_default().push(work.frame);
        scheduler.complete(work, true);
        assert!(frames.values().map(Vec::len).sum::<usize>() <= 18);
    }
    for token in [1, 2, 3] {
        assert_eq!(frames[&token], [0, 1, 2, 3, 4, 5]);
    }
}

#[test]
fn offscreen_generation_and_worker_bounds_are_enforced() {
    let mut scheduler = Scheduler::default();
    scheduler.set_view(
        1,
        vec![
            item(1, true, false),
            item(2, true, false),
            item(3, true, false),
        ],
    );
    let stale = scheduler.next_work().unwrap();
    let second = scheduler.next_work().unwrap();
    assert!(
        scheduler.next_work().is_none(),
        "Only two CPU decoders may be admitted"
    );
    scheduler.set_view(2, vec![item(8, false, false)]);
    assert!(!scheduler.complete(stale, true));
    assert!(!scheduler.complete(second, true));
    let current = scheduler.next_work().unwrap();
    assert_eq!(current.token, 8);
    scheduler.complete(current, true);
    assert!(scheduler.next_work().is_none());
}
