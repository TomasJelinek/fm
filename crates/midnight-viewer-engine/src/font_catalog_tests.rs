use super::*;
use midnight_core::{DirectoryIndex, Entry, EntryId, EntryKind};
use std::time::Duration;

#[test]
fn installed_font_catalog_arrives_without_content_and_is_shared_between_windows() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_resident_owner, resident) =
        midnight_work_budget::resident::Owner::start("viewer-test", crate::RESIDENT_WORKERS)
            .unwrap();
    let service = Service::with_native_capture(
        runtime.handle().clone(),
        resident,
        Arc::new(|_, _| Err("No content is needed for a font chooser".into())),
    );
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        "unavailable-font-catalog-test.md".into(),
        EntryKind::File,
    )]
    .into();
    let launch = ViewerLaunch {
        find: None,
        associations: Default::default(),
        tree: None,
        generated: None,
        focused: entries[0].path.clone(),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
        preferences: Arc::new(Default::default()),
        archive: None,
        basic: false,
        disabled_plugins: Arc::from([]),
    };
    let appearance = ViewerAppearance {
        family: "sans-serif".into(),
        font_size: 16.0,
        foreground: [255; 4],
        background: [0, 0, 0, 255],
        selection: [70; 4],
        selection_text: [0; 4],
        accent: [180; 4],
    };
    let geometry = ViewerGeometry {
        width: 320,
        height: 200,
        scale: 1.0,
    };
    let first = service
        .open(launch.clone(), appearance.clone(), geometry)
        .unwrap();
    // The worker publishes the catalog before its first output; wait for that wake.
    let (notify, notified) = std::sync::mpsc::sync_channel(1);
    first
        .mailbox
        .notify_on_ready(Arc::new(move || notify.try_send(()).is_ok()));
    notified
        .recv_timeout(Duration::from_secs(10))
        .expect("backend did not start");
    let catalog = first
        .font_families()
        .expect("backend published the font catalog before any output");
    assert!(!catalog.is_empty());
    let second = service.open(launch, appearance, geometry).unwrap();
    assert!(Arc::ptr_eq(&catalog, &second.font_families().unwrap()));
    first.close();
    second.close();
}
