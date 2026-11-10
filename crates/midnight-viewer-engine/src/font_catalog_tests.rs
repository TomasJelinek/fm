use super::*;
use midnight_core::{DirectoryIndex, Entry, EntryId, EntryKind};
use std::time::{Duration, Instant};

#[test]
fn installed_font_catalog_arrives_without_content_and_is_shared_between_windows() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::with_native_capture(
        runtime.handle().clone(),
        Arc::new(|_, _| Err("No content is needed for a font chooser".into())),
    );
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        "unavailable-font-catalog-test.md".into(),
        EntryKind::File,
    )]
    .into();
    let launch = ViewerLaunch {
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
    let deadline = Instant::now() + Duration::from_secs(10);
    let catalog = loop {
        if let Some(names) = first.font_families() {
            break names;
        }
        assert!(
            Instant::now() < deadline,
            "backend did not publish the font catalog"
        );
        std::thread::sleep(Duration::from_millis(5));
    };
    assert!(!catalog.is_empty());
    let second = service.open(launch, appearance, geometry).unwrap();
    assert!(Arc::ptr_eq(&catalog, &second.font_families().unwrap()));
    first.close();
    second.close();
}
