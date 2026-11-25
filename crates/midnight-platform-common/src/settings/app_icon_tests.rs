use super::{decode_preferences, encode_preferences};
use midnight_core::{AppIcon, FindPreferences, ScrollGlide, UserPreferences};
use serde_json::{Value, json};
use std::io;

#[test]
fn app_icon_uses_stable_picker_indices_and_storage_names() {
    let choices = [
        (0, AppIcon::Moon, "moon"),
        (1, AppIcon::HarvestNavigator, "harvest-navigator"),
        (2, AppIcon::OrigamiTrail, "origami-trail"),
        (3, AppIcon::Interleave, "interleave"),
    ];
    for (index, icon, name) in choices {
        assert_eq!(AppIcon::from_index(index), Some(icon));
        assert_eq!(icon.index(), index);
        let preferences = UserPreferences {
            app_icon: icon,
            ..Default::default()
        };
        let bytes = encode_preferences(preferences.clone()).unwrap();
        let document: Value = serde_json::from_slice(&bytes).unwrap();
        if icon == AppIcon::Moon {
            assert_eq!(document["version"], 10);
            assert!(document.get("app_icon").is_none());
        } else {
            assert_eq!(document["version"], 44);
            assert_eq!(document["app_icon"], name);
        }
        assert_eq!(decode_preferences(&bytes).unwrap(), preferences);
    }
    assert_eq!(AppIcon::from_index(-1), None);
    assert_eq!(AppIcon::from_index(4), None);
}

#[test]
fn schema_43_migrates_to_moon_and_explicit_schema_44_moon_preserves_other_choices() {
    let previous = UserPreferences {
        copy_preserve_metadata: false,
        scroll_glide: ScrollGlide::Short,
        find: FindPreferences {
            mask_history: vec!["*.rs".into()],
            ..Default::default()
        },
        ..Default::default()
    };
    let bytes = encode_preferences(previous.clone()).unwrap();
    let mut document: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(document["version"], 43);
    assert!(document.get("app_icon").is_none());
    assert_eq!(decode_preferences(&bytes).unwrap().app_icon, AppIcon::Moon);
    document["version"] = 44.into();
    document["app_icon"] = "moon".into();
    let loaded = decode_preferences(&serde_json::to_vec(&document).unwrap()).unwrap();
    assert_eq!(loaded, previous);
    assert_eq!(encode_preferences(loaded).unwrap(), bytes);
}

#[test]
fn app_icon_rejects_invalid_values_and_wrong_schema_versions() {
    let bytes = encode_preferences(UserPreferences {
        app_icon: AppIcon::Interleave,
        copy_preserve_metadata: false,
        ..Default::default()
    })
    .unwrap();
    let mut document: Value = serde_json::from_slice(&bytes).unwrap();
    let invalid = |value: &Value| {
        assert_eq!(
            decode_preferences(&serde_json::to_vec(value).unwrap())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidData
        );
    };
    for value in [json!(null), json!(1), json!("future-icon"), json!("Moon")] {
        document["app_icon"] = value;
        invalid(&document);
    }
    document.as_object_mut().unwrap().remove("app_icon");
    invalid(&document);
    document["app_icon"] = "interleave".into();
    document["version"] = 43.into();
    invalid(&document);
}
