use super::*;

fn preferences() -> UserPreferences {
    let root = std::env::current_dir().unwrap();
    let address = midnight_core::ArchiveAddress {
        format: None,
        provider: "midnight.fs.archive".into(),
        container: root.join("日本語.zip"),
        source_version: vec![1, 2, 0, 255],
        components: vec![b"folder".to_vec(), vec![0xff, b'x']],
    };
    let mut preferences = UserPreferences::default();
    preferences.tabs[0] = PaneTabPreferences {
        paths: vec![root],
        active: 0,
        networks: Default::default(),
        archives: [(0, address)].into(),
    };
    preferences
}

#[test]
fn archive_addresses_round_trip_raw_components_without_path_interpretation() {
    let expected = preferences();
    let bytes = encode_preferences(expected.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["version"], 11);
    assert_eq!(json["tabs"][0]["archives"]["0"]["version"], 1);
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
    let native = encode_preferences(UserPreferences::default()).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&native).unwrap()["version"],
        10
    );
}

#[test]
fn archive_tabs_and_nondefault_previews_round_trip_together() {
    let mut expected = preferences();
    expected.thumbnails_enabled = false;
    expected.thumbnails_network = true;
    let bytes = encode_preferences(expected.clone()).unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["version"], 12);
    assert_eq!(json["tabs"][0]["archives"]["0"]["version"], 1);
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
}

#[test]
fn malformed_and_out_of_range_archive_addresses_are_rejected() {
    let base: serde_json::Value =
        serde_json::from_slice(&encode_preferences(preferences()).unwrap()).unwrap();
    for (key, value) in [
        ("version", serde_json::json!(2)),
        ("provider", serde_json::json!("../plugin")),
        ("source_version", serde_json::json!([])),
        ("components", serde_json::json!([[46, 46]])),
        ("components", serde_json::json!([[120, 47, 121]])),
        ("components", serde_json::json!([[67, 58, 120]])),
    ] {
        let mut bad = base.clone();
        bad["tabs"][0]["archives"]["0"][key] = value;
        assert!(
            decode_preferences(&serde_json::to_vec(&bad).unwrap()).is_err(),
            "{key}"
        );
    }
    let mut bad = preferences();
    let address = bad.tabs[0].archives.remove(&0).unwrap();
    bad.tabs[0].archives.insert(32, address);
    assert!(encode_preferences(bad).is_err());
    let mut downgraded = base;
    downgraded["version"] = serde_json::json!(10);
    assert!(decode_preferences(&serde_json::to_vec(&downgraded).unwrap()).is_err());
}
