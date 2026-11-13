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

#[test]
fn named_volumes_precede_raw_mount_paths_without_mutating_paths() {
    let labels = ["/media/usb/", "Zulu", "/", "Alpha", "/home/"];
    let input = labels
        .iter()
        .map(|label| location(label, label, LocationKind::Volume))
        .collect();
    let (ordered, _) = locations(input, None).unwrap();
    assert_eq!(
        ordered
            .iter()
            .map(|item| item.label.as_str())
            .collect::<Vec<_>>(),
        ["Alpha", "Zulu", "/", "/home/", "/media/usb/"]
    );
    assert!(
        ordered
            .iter()
            .all(|item| item.path == Path::new("raw/../same 文档"))
    );
}

#[test]
fn recycle_bin_rank_excludes_volumes_and_keeps_network_at_the_front() {
    let (ordered, network_position) = locations(
        vec![
            location("disk", "A disk", LocationKind::Volume),
            location("z", "Zulu", LocationKind::Home),
            location("a", "Alpha", LocationKind::Documents),
            location("tie", "Recycle Bin", LocationKind::Pictures),
        ],
        None,
    )
    .unwrap();
    let choices = system_locations(&ordered, true, None).unwrap();
    assert_eq!(network_position, 0);
    assert_eq!(choices.len(), 1);
    assert_eq!(choices[0].location, SystemLocation::RecycleBin);
    assert_eq!(choices[0].label, "Recycle Bin");
    // Equal native labels precede the system entry; the volume is not counted.
    assert_eq!(choices[0].special_position, 2);
    assert!(system_locations(&ordered, false, None).unwrap().is_empty());
}

struct RecycleLocale;
impl LocaleService for RecycleLocale {
    fn number_locale(&self) -> Result<midnight_core::NumberLocale, FsError> {
        Ok(midnight_core::NumberLocale::default())
    }
    fn collation_key(&self, label: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        Ok(match label.to_str().unwrap() {
            "Zulu" => Some(vec![1]),
            "Recycle Bin" => Some(vec![2]),
            "Alpha" => Some(vec![3]),
            _ => None,
        })
    }
}

#[test]
fn recycle_bin_rank_uses_worker_locale_keys_and_raw_name_fallback() {
    let input = vec![
        location("a", "Alpha", LocationKind::Documents),
        location("z", "Zulu", LocationKind::Home),
        location("raw", "/unrepresentable", LocationKind::Temporary),
    ];
    let (ordered, _) = locations(input, Some(&RecycleLocale)).unwrap();
    let choices = system_locations(&ordered, true, Some(&RecycleLocale)).unwrap();
    assert_eq!(choices[0].special_position, 1);
    assert_eq!(
        system_locations(&ordered, true, None).unwrap()[0].special_position,
        1
    );
}

struct OversizedRecycleKey;
impl LocaleService for OversizedRecycleKey {
    fn number_locale(&self) -> Result<midnight_core::NumberLocale, FsError> {
        Ok(midnight_core::NumberLocale::default())
    }
    fn collation_key(&self, label: &OsStr) -> Result<Option<Vec<u8>>, FsError> {
        assert_eq!(label, OsStr::new("Recycle Bin"));
        Ok(Some(vec![0; MAX_COLLATION_KEY_BYTES + 1]))
    }
}

#[test]
fn recycle_bin_key_budget_is_checked_and_unavailable_capability_does_no_label_work() {
    assert!(
        system_locations(&[], false, Some(&OversizedRecycleKey))
            .unwrap()
            .is_empty()
    );
    let error = system_locations(&[], true, Some(&OversizedRecycleKey)).unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::InvalidData);
    assert!(error.path.as_os_str().is_empty());
}
