use super::*;

#[test]
fn replaced_snapshot_and_index_stay_alive_until_background_reclamation() {
    let mut p = Presenter::new("a".into(), "right".into());
    let id = begin(&mut p, PaneId::Left, "a");
    let entries: Arc<[Entry]> = vec![entry(1, "a/child")].into();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    let weak_entries = Arc::downgrade(&entries);
    let weak_index = Arc::downgrade(&index);
    p.apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: id,
        entries,
        index,
    });
    let id = begin(&mut p, PaneId::Left, "b");
    finish(&mut p, PaneId::Left, id, vec![entry(2, "b/child")]);
    assert!(weak_entries.upgrade().is_some());
    assert!(weak_index.upgrade().is_some());
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(weak_entries.upgrade().is_none());
    assert!(weak_index.upgrade().is_none());
}
#[test]
fn shutdown_retires_live_and_pending_ownership_before_presenter_is_dropped() {
    let mut p = Presenter::new("a".into(), "right".into());
    let id = begin(&mut p, PaneId::Left, "a");
    let entries: Arc<[Entry]> = vec![entry(1, "a/child")].into();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    let weak_entries = Arc::downgrade(&entries);
    let weak_index = Arc::downgrade(&index);
    p.apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: id,
        entries,
        index,
    });
    let id = begin(&mut p, PaneId::Left, "b");
    p.apply(Event::EntriesBatch {
        pane: PaneId::Left,
        navigation: id,
        entries: vec![entry(2, "b/child")],
    });
    p.retire_all();
    assert!(p.panes[0].rows().is_empty());
    assert!(!p.is_loading());
    let retired = p.take_retired();
    drop(p);
    assert!(weak_entries.upgrade().is_some());
    assert!(weak_index.upgrade().is_some());
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(weak_entries.upgrade().is_none());
    assert!(weak_index.upgrade().is_none());
}
#[test]
fn stale_and_duplicate_final_payloads_are_retired_without_ui_destruction() {
    let mut p = Presenter::new("a".into(), "right".into());
    let old = begin(&mut p, PaneId::Left, "a");
    let current = begin(&mut p, PaneId::Left, "b");
    finish(&mut p, PaneId::Left, current, vec![]);
    for generation in [old, current] {
        let entries: Arc<[Entry]> = vec![entry(7, "unused")].into();
        let index = Arc::new(DirectoryIndex::from_entries(&entries));
        let weak = Arc::downgrade(&entries);
        p.apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation: generation,
            entries,
            index,
        });
        assert!(weak.upgrade().is_some());
        let retired = p.take_retired();
        std::thread::spawn(move || drop(retired)).join().unwrap();
        assert!(weak.upgrade().is_none());
    }
}
