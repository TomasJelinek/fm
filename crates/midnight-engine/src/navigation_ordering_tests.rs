use super::*;

fn location(id: &str, label: &str, kind: LocationKind) -> NavigationLocation {
    NavigationLocation {
        id: id.into(),
        label: label.into(),
        path: "raw/../same 文档".into(),
        kind,
        drive_letter: None,
    }
}

#[test]
fn network_follows_drives_before_deduplicated_special_locations() {
    for duplicate_label in ["Documents", "Music", "Zulu"] {
        let input = vec![
            location("documents", duplicate_label, LocationKind::Documents),
            location("disk-z", "Zulu", LocationKind::Volume),
            location("pictures", "Pictures", LocationKind::Pictures),
            location("documents", "Documents", LocationKind::Documents),
            location("disk-a", "Alpha", LocationKind::Volume),
        ];
        let (ordered, position) = locations(input, None).unwrap();
        assert_eq!(
            position, 0,
            "duplicate {duplicate_label} must not shift Network into special locations"
        );
        assert_eq!(
            ordered.len(),
            4,
            "publish the same canonical identities consumed by UI"
        );
        assert_eq!(
            ordered
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["disk-a", "disk-z", "documents", "pictures"]
        );
        assert_eq!(ordered[2].label, "Documents");
    }
}

#[test]
fn same_path_aliases_remain_distinct_after_network() {
    let input = vec![
        location("documents", "Documents", LocationKind::Documents),
        location("home", "Home", LocationKind::Home),
        location("pictures", "Pictures", LocationKind::Pictures),
    ];
    let (ordered, position) = locations(input, None).unwrap();
    assert_eq!(position, 0);
    assert_eq!(ordered.len(), 3);
    assert_eq!(
        ordered
            .iter()
            .map(|item| item.id.as_str())
            .collect::<Vec<_>>(),
        ["documents", "home", "pictures"]
    );
    assert!(
        ordered
            .iter()
            .all(|item| item.path.as_os_str() == OsStr::new("raw/../same 文档"))
    );
}
