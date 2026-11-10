//! Exercise the bridge's retry trigger against the real retained-pixel budget.
use super::*;
use midnight_core::{
    Command, DirectoryIndex, Entry, EntryId, EntryKind, Event, PaneId, PanelViewMode,
    UserPreferences,
};
use std::sync::Arc;

#[test]
fn rejected_upgrade_waits_for_changed_view_then_retries_with_its_retained_coverage() {
    use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(super::tests::TestPlatform(native))).unwrap();
    let window = MainWindow::new().unwrap();
    let root = std::env::current_dir().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            thumbnails_enabled: true,
            thumbnails_network: true,
            panel_view_modes: [PanelViewMode::Thumbnails; 2],
            thumbnail_size: 256,
            ..UserPreferences::default()
        }))));
    let navigation = Rc::new(Cell::new(None));
    let captured = navigation.clone();
    midnight_ui::install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            if let Command::Navigate {
                pane: PaneId::Left,
                navigation: id,
                ..
            } = command
            {
                captured.set(Some(id));
            }
            true
        }),
    );
    window.invoke_navigate(0, root.to_string_lossy().into_owned().into());
    let mut presenter = presenter.borrow_mut();
    let entries: Arc<[Entry]> = (0..10)
        .map(|index| {
            Entry::new(
                EntryId(index),
                root.join(format!("{index}.png")),
                EntryKind::File,
            )
        })
        .collect();
    presenter.apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation: navigation.get().unwrap(),
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    let rows = presenter.thumbnail_targets([(0, 11), (0, 0)], ["", ""]);
    assert_eq!(rows.len(), 10);
    let large_pixels = vec![255; 512 * 510 * 4];
    for target in &rows[..9] {
        for frame in 0..6 {
            assert!(presenter.receive_thumbnail_at_resolution(
                &target.identity,
                frame,
                512,
                510,
                512,
                &large_pixels
            ));
        }
    }
    let identity = rows[9].identity.clone();
    assert!(presenter.receive_thumbnail_at_resolution(
        &identity,
        0,
        96,
        96,
        96,
        &vec![128; 96 * 96 * 4]
    ));
    let upgraded_pixels = vec![255; 512 * 512 * 4];
    assert!(!presenter.receive_thumbnail_at_resolution(
        &identity,
        0,
        512,
        512,
        512,
        &upgraded_pixels
    ));
    assert_eq!(presenter.thumbnail_coverage(&identity, 0), 96);

    let mut deliveries = Deliveries::default();
    deliveries.set_view(rows.clone(), 2.0, |_| panic!("no previous rejection"));
    let work = Work {
        generation: 1,
        token: 42,
        edge: 512,
        frame: 0,
    };
    deliveries.record(work, presenter.thumbnail_coverage(&identity, 0));
    for _ in 0..20 {
        deliveries.set_view(rows.clone(), 2.0, |_| {
            panic!("unchanged timer pulses must not retry")
        });
    }
    let less_crowded = presenter.thumbnail_targets([(2, 11), (0, 0)], ["", ""]);
    assert_eq!(less_crowded.len(), 9);
    assert!(
        less_crowded
            .iter()
            .any(|target| target.identity == identity)
    );
    let mut requests = Vec::new();
    deliveries.set_view(less_crowded.clone(), 2.0, |retry| {
        requests.push((retry, presenter.thumbnail_coverage(&identity, retry.frame)));
        true
    });
    assert_eq!(
        requests,
        [(work, 96)],
        "the bridge automatically retries the same immutable source token"
    );
    assert!(presenter.receive_thumbnail_at_resolution(
        &identity,
        0,
        512,
        512,
        512,
        &upgraded_pixels
    ));
    deliveries.record(work, presenter.thumbnail_coverage(&identity, 0));
    assert_eq!(presenter.thumbnail_coverage(&identity, 0), 512);
    deliveries.set_view(less_crowded, 1.0, |_| {
        panic!("an accepted upgrade must not be retried again")
    });
}
