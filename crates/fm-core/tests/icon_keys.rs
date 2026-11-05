use fm_core::{ActionIcon, IconKey, IconKind, LocationKind, NavigationLocation};
use std::{collections::HashSet, path::PathBuf};

#[test]
fn finite_action_palette_keeps_each_semantic_role_distinct() {
    let keys: HashSet<_> = ActionIcon::ALL.into_iter().map(IconKey::action).collect();
    assert_eq!(keys.len(), 33);
    assert!(keys.contains(&IconKey::action(ActionIcon::Network)));
    assert!(keys.iter().all(|key| {
        matches!(key.kind, IconKind::Action(_))
            && key.extension.is_none()
            && key.drive_root.is_none()
            && key.size == 32
    }));
}

#[test]
fn location_keys_preserve_native_volume_identity_and_same_path_aliases() {
    let path = PathBuf::from(format!("/文档/{}", "👩🏽‍💻e\u{301}".repeat(400)));
    let mut location = NavigationLocation {
        id: "native-volume".into(),
        label: "A label is never an icon path".into(),
        path: path.clone(),
        kind: LocationKind::Volume,
        drive_letter: None,
    };
    assert_eq!(IconKey::for_location(&location), IconKey::for_drive(&path));
    let mut aliases = HashSet::new();
    for kind in [
        LocationKind::Home,
        LocationKind::Desktop,
        LocationKind::Documents,
        LocationKind::Downloads,
    ] {
        location.kind = kind;
        let key = IconKey::for_location(&location);
        assert_eq!(key.kind, IconKind::Location(kind));
        assert!(key.drive_root.is_none());
        assert!(aliases.insert(key));
    }
}
