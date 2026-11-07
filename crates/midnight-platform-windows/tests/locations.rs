#![cfg(windows)]
use midnight_core::{LocationKind, LocationService};
use midnight_platform_windows::WindowsFileSystem;

#[test]
fn user_temporary_directory_is_a_native_special_location() {
    let locations = WindowsFileSystem::new(None).locations().unwrap();
    let temporary = locations
        .iter()
        .find(|location| location.kind == LocationKind::Temporary)
        .expect("the existing user temporary directory is offered");
    assert_eq!(temporary.path, std::env::temp_dir());
    assert_eq!(temporary.id, "temporary");
    assert!(temporary.drive_letter.is_none());
    assert!(temporary.path.is_absolute());
}

#[test]
fn special_locations_have_unique_ids_and_keep_real_native_paths() {
    let locations = WindowsFileSystem::new(None).locations().unwrap();
    let mut ids = std::collections::HashSet::new();
    for location in locations {
        assert!(ids.insert(location.id));
        assert!(location.path.is_absolute());
        if location.kind == LocationKind::OneDrive {
            assert!(
                location.path.is_dir(),
                "unconfigured cloud roots stay absent"
            );
        }
    }
}
