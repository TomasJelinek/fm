use super::*;
use midnight_core::StorageKind;

#[test]
fn delayed_network_classification_never_admits_a_preview_without_opt_in() {
    let mut p = Presenter::new(test_root(), test_root());
    p.preferences.values.panel_view_modes[0] = PanelViewMode::Thumbnails;
    let navigation = begin(&mut p, PaneId::Left, test_root().to_str().unwrap());
    let entry = Entry::new(EntryId(1), test_root().join("clip.avi"), EntryKind::File);
    finish(&mut p, PaneId::Left, navigation, vec![entry]);
    assert!(p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).is_empty());
    p.apply(Event::StorageKindUpdated {
        pane: PaneId::Left,
        navigation,
        kind: StorageKind::Network,
    });
    assert!(p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).is_empty());
    p.preferences.values.thumbnails_network = true;
    assert_eq!(p.thumbnail_targets([(0, 10), (0, 0)], ["", ""]).len(), 1);
}
