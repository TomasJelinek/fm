use midnight_core::{UserPreferences, WindowGeometry, WindowMonitor, WindowRect};
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn geometry() -> WindowGeometry {
    WindowGeometry {
        normal: WindowRect {
            x: -1755,
            y: 137,
            width: 1231,
            height: 719,
        },
        monitor: Some(WindowMonitor {
            name: Some("Display 文档 🧪".into()),
            bounds: WindowRect {
                x: -1920,
                y: 0,
                width: 1920,
                height: 1080,
            },
        }),
        maximized: true,
        fullscreen: false,
    }
}

#[test]
fn exact_physical_normal_bounds_monitor_and_state_round_trip() {
    let preferences = UserPreferences {
        window_geometry: Some(geometry()),
        ..Default::default()
    };
    let bytes = encode_preferences(preferences.clone()).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["version"], 15);
    assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
}

#[test]
fn legacy_preferences_have_no_geometry_and_keep_their_original_version() {
    let bytes = encode_preferences(UserPreferences::default()).unwrap();
    let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["version"], 10);
    assert!(value.get("window_geometry").is_none());
    assert!(
        decode_preferences(&bytes)
            .unwrap()
            .window_geometry
            .is_none()
    );
}

#[test]
fn malformed_and_foreign_version_geometry_is_rejected_atomically() {
    let bytes = encode_preferences(UserPreferences {
        window_geometry: Some(geometry()),
        ..Default::default()
    })
    .unwrap();
    let original: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    for (pointer, invalid) in [
        ("/window_geometry/normal/width", serde_json::json!(0)),
        ("/window_geometry/normal/height", serde_json::json!(131073)),
        ("/window_geometry/normal/y", serde_json::json!(2147483647)),
        (
            "/window_geometry/monitor/name",
            serde_json::json!("bad\nname"),
        ),
        ("/window_geometry/fullscreen", serde_json::json!(null)),
        ("/version", serde_json::json!(14)),
    ] {
        let mut value = original.clone();
        *value.pointer_mut(pointer).unwrap() = invalid;
        assert!(
            decode_preferences(&serde_json::to_vec(&value).unwrap()).is_err(),
            "{pointer}"
        );
    }
    let mut missing = original.clone();
    missing.as_object_mut().unwrap().remove("window_geometry");
    assert!(decode_preferences(&serde_json::to_vec(&missing).unwrap()).is_err());
    let mut unknown = original;
    unknown["window_geometry"]["work_area_guess"] = serde_json::json!(7);
    assert!(decode_preferences(&serde_json::to_vec(&unknown).unwrap()).is_err());
}
