//! Opt-in coverage of actual storage admission, provider capture, decoding and persistence.
use midnight_core::{StorageKind, StorageService};
use midnight_native_filesystem_host::InstalledProvider;
use midnight_thumbnail_host::service;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

#[test]
#[ignore = "requires a trusted MIDNIGHT_NATIVE_TEST_INSTALLATION and explicit MIDNIGHT_NATIVE_THUMBNAIL_PNG"]
fn local_png_reaches_the_installed_thumbnail_worker_with_network_previews_disabled() {
    let installation = PathBuf::from(
        std::env::var_os("MIDNIGHT_NATIVE_TEST_INSTALLATION").expect("explicit installation"),
    );
    let path = PathBuf::from(
        std::env::var_os("MIDNIGHT_NATIVE_THUMBNAIL_PNG").expect("explicit read-only PNG source"),
    );
    assert!(path.is_absolute());
    let filesystem = Arc::new(InstalledProvider::from_installation(
        installation.clone(),
        None,
    ));
    let kind = filesystem.storage_kind(path.parent().unwrap()).unwrap();
    assert!(
        matches!(kind, StorageKind::LocalSsd | StorageKind::Rotational),
        "a proven local backing graph must pass preview admission without network opt-in: {kind:?}"
    );
    let factory = super::content::Factory::installed(filesystem);
    let row = midnight_ui::ThumbnailTarget {
        identity: "explicit-native-png-probe".into(),
        content: midnight_ui::ThumbnailContent::Native(path),
        selected: false,
        hovered: false,
        confirmed_ssd: kind == StorageKind::LocalSsd,
        edge: 96,
    };
    let cache = PathBuf::from(
        std::env::var_os("MIDNIGHT_NATIVE_THUMBNAIL_CACHE").expect("explicit disposable cache"),
    );
    assert!(cache.is_absolute() && cache.components().any(|part| part.as_os_str() == "_build"));
    let service = service::Service::start(installation.join("me"), cache);
    service.set_view(service::View {
        generation: 1,
        targets: vec![factory.target(&row, 1, 96).unwrap()],
    });
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(
            service.take_notice().is_none(),
            "thumbnail service initialization failed"
        );
        if let Some(ready) = service.poll() {
            assert_eq!((ready.generation, ready.token), (1, 1));
            assert!(ready.frame.width > 0 && ready.frame.height > 0);
            assert_eq!(
                ready.frame.rgba.len(),
                ready.frame.width as usize * ready.frame.height as usize * 4
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "local PNG did not produce a persisted thumbnail"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    service.stop();
}
