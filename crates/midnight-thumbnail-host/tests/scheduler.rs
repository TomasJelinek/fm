use midnight_thumbnail_host::scheduler::{Item, Scheduler};

fn item(token: u64, selected: bool, hovered: bool) -> Item {
    Item {
        token,
        video: true,
        selected,
        hovered,
    }
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
