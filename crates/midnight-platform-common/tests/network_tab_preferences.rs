use midnight_core::{NetworkAddress, UserPreferences};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};
use std::sync::Arc;

#[test]
fn network_tabs_preserve_opaque_bytes_without_native_path_interpretation() {
    let mut expected = UserPreferences::default();
    expected.tabs[0]
        .paths
        .push(std::env::current_dir().unwrap());
    expected.tabs[0].networks.insert(
        0,
        NetworkAddress {
            provider: "network.test".into(),
            opaque: Arc::from([0, 255, 1, 128]),
            display: "文档 server".into(),
        },
    );
    let bytes = encode_preferences(expected.clone()).unwrap();
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
    let mut value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["version"], 24);
    value["tabs"][0]["networks"]["0"]["version"] = 2.into();
    assert!(decode_preferences(&serde_json::to_vec(&value).unwrap()).is_err());
    value["tabs"][0]["networks"]["0"]["version"] = 1.into();
    value["version"] = 23.into();
    assert!(decode_preferences(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn network_tab_must_reference_a_saved_slot_and_bounded_provider_address() {
    let mut preferences = UserPreferences::default();
    let address = NetworkAddress {
        provider: "network.test".into(),
        opaque: Arc::from([1]),
        display: "Network".into(),
    };
    preferences.tabs[0].networks.insert(0, address);
    assert!(encode_preferences(preferences.clone()).is_err());
    preferences.tabs[0]
        .paths
        .push(std::env::current_dir().unwrap());
    preferences.tabs[0].networks.get_mut(&0).unwrap().provider = "../escape".into();
    assert!(encode_preferences(preferences).is_err());
}
