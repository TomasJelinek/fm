use midnight_core::{
    FileComparatorPreferences, FileComparatorProgram, FileComparatorRule, FileComparatorTarget,
    FilterMode, PreparedFileComparatorPolicy, UserPreferences,
};
use midnight_platform_common::{
    comparator_rules,
    settings::{
        decode_preferences, decode_preferences_for_acceptance, encode_preferences,
        encode_preferences_prepared, read_preferences_for_acceptance,
    },
};
use serde_json::{Value, json};
use std::{fs::File, sync::Arc};

fn rule(target: FileComparatorTarget) -> FileComparatorRule {
    FileComparatorRule {
        enabled: true,
        mode: FilterMode::Masks,
        expression: "*.txt;*.rs|secret*".into(),
        case_sensitive: false,
        target,
    }
}

fn preferences() -> UserPreferences {
    let program = FileComparatorProgram {
        executable: std::env::current_exe().unwrap(),
        arguments: vec![
            "{left}".into(),
            "".into(),
            "--arg={left}".into(),
            "{right}".into(),
        ],
        working_directory: Some(std::env::current_dir().unwrap()),
    };
    UserPreferences {
        file_comparators: FileComparatorPreferences {
            rules: vec![
                rule(FileComparatorTarget::Automatic),
                rule(FileComparatorTarget::BuiltInText),
                rule(FileComparatorTarget::BuiltInImages),
                FileComparatorRule {
                    enabled: false,
                    ..rule(FileComparatorTarget::External(program))
                },
            ],
        },
        ..Default::default()
    }
}

fn document() -> Value {
    serde_json::from_slice(&encode_preferences(preferences()).unwrap()).unwrap()
}

fn decode(value: &Value) -> std::io::Result<UserPreferences> {
    decode_preferences(&serde_json::to_vec(value).unwrap())
}

#[test]
fn schema33_roundtrips_ordered_targets_and_cascades_required_defaults() {
    let expected = preferences();
    let bytes = encode_preferences(expected.clone()).unwrap();
    let stored: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(stored["version"], 33);
    assert_eq!(stored["disk_io_concurrency"], json!({"mode":"automatic"}));
    assert_eq!(
        stored["previews"],
        json!({"enabled":true,"network":false,"archives":true})
    );
    assert_eq!(stored["editor"]["rules"], json!([]));
    assert_eq!(stored["panel_filter_history"], json!([]));
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);
    assert_eq!(decode_preferences_for_acceptance(&bytes).unwrap(), expected);
}

#[test]
fn empty_comparators_preserve_old_minimum_schema() {
    let expected = UserPreferences::default();
    let stored: Value =
        serde_json::from_slice(&encode_preferences(expected.clone()).unwrap()).unwrap();
    assert_eq!(stored["version"], 10);
    assert!(stored.get("file_comparators").is_none());
    assert_eq!(decode(&stored).unwrap(), expected);

    let mut empty33 = document();
    empty33["file_comparators"]["rules"] = json!([]);
    assert!(decode(&empty33).unwrap().file_comparators.rules.is_empty());
}

#[test]
fn schema33_requires_comparators_and_both_historical_extensions() {
    for missing in [
        "file_comparators",
        "disk_io_concurrency",
        "previews",
        "viewer",
        "file_list",
        "panel_filter_history",
    ] {
        let mut value = document();
        value.as_object_mut().unwrap().remove(missing);
        assert!(decode(&value).is_err(), "accepted missing {missing}");
    }
    let mut value = document();
    value["previews"]
        .as_object_mut()
        .unwrap()
        .remove("archives");
    assert!(decode(&value).is_err());
    for version in [0, 34, u32::MAX] {
        let mut value = document();
        value["version"] = version.into();
        assert!(decode(&value).is_err());
    }
}

#[test]
fn older_schemas_forbid_even_empty_comparators_and_keep_both_schema31_shapes() {
    let base = document();
    for version in [30, 31, 32] {
        let mut legacy = base.clone();
        legacy["version"] = version.into();
        legacy.as_object_mut().unwrap().remove("file_comparators");
        if version <= 31 {
            legacy["previews"]
                .as_object_mut()
                .unwrap()
                .remove("archives");
        }
        if version == 30 {
            legacy
                .as_object_mut()
                .unwrap()
                .remove("disk_io_concurrency");
        }
        assert!(decode(&legacy).unwrap().file_comparators.rules.is_empty());
        legacy["file_comparators"] = json!({"rules":[]});
        assert!(decode(&legacy).is_err());
    }
    let mut archive31 = base;
    archive31["version"] = 31.into();
    archive31
        .as_object_mut()
        .unwrap()
        .remove("file_comparators");
    archive31
        .as_object_mut()
        .unwrap()
        .remove("disk_io_concurrency");
    archive31["previews"]["archives"] = false.into();
    assert!(!decode(&archive31).unwrap().thumbnails_archives);
}

#[test]
fn strict_dto_rejects_unknown_null_missing_and_cross_target_fields() {
    for comparators in [
        json!(null),
        json!([]),
        json!({}),
        json!({"rules":null}),
        json!({"rules":[],"other":1}),
    ] {
        let mut value = document();
        value["file_comparators"] = comparators;
        assert!(decode(&value).is_err());
    }
    for target in [
        json!(null),
        json!("automatic"),
        json!({}),
        json!({"kind":"unknown"}),
        json!({"kind":"automatic","arguments":[]}),
        json!({"kind":"built_in_text","other":true}),
        json!({"kind":"external","executable":"relative","arguments":["{left}","{right}"]}),
    ] {
        let mut value = document();
        value["file_comparators"]["rules"][0]["target"] = target;
        assert!(decode(&value).is_err());
    }
    for missing in ["enabled", "mode", "expression", "case_sensitive", "target"] {
        let mut value = document();
        value["file_comparators"]["rules"][0]
            .as_object_mut()
            .unwrap()
            .remove(missing);
        assert!(decode(&value).is_err());
    }
    let mut value = document();
    value["file_comparators"]["rules"][0]["extra"] = true.into();
    assert!(decode(&value).is_err());
    let mut value = document();
    value["file_comparators"]["rules"][3]["target"]["working_directory"] = Value::Null;
    assert!(decode(&value).is_err());
}

#[test]
fn duplicate_comparator_objects_rules_and_tags_are_rejected() {
    let text = serde_json::to_string(&document()).unwrap();
    let duplicate = text.replacen(
        "\"file_comparators\":",
        "\"file_comparators\":{\"rules\":[]},\"file_comparators\":",
        1,
    );
    assert!(decode_preferences(duplicate.as_bytes()).is_err());
    let duplicate = text.replacen(
        "\"file_comparators\":{\"rules\":[",
        "\"file_comparators\":{\"rules\":[],\"rules\":[",
        1,
    );
    assert_ne!(duplicate, text);
    assert!(decode_preferences(duplicate.as_bytes()).is_err());
    let duplicate = text.replacen(
        "\"kind\":\"automatic\"",
        "\"kind\":\"automatic\",\"kind\":\"automatic\"",
        1,
    );
    assert!(decode_preferences(duplicate.as_bytes()).is_err());
}

#[test]
fn every_validated_disk_policy_is_accepted_by_the_publication_gate() {
    use midnight_core::DiskIoConcurrency;
    let gate = midnight_work_budget::Gate::new(4);
    for policy in
        std::iter::once(DiskIoConcurrency::Automatic).chain((1..=32).map(DiskIoConcurrency::Manual))
    {
        let validated = policy.effective_limit().unwrap();
        gate.set_limit(validated).unwrap();
        assert_eq!(gate.effective_limit(), validated);
    }
    let prior = gate.effective_limit();
    for invalid in [0, 33, 255] {
        assert!(
            DiskIoConcurrency::Manual(invalid)
                .effective_limit()
                .is_err()
        );
        assert_eq!(gate.effective_limit(), prior);
    }
}

#[test]
fn acceptance_defers_only_semantic_compilation_to_the_serialized_worker() {
    let mut value = document();
    value["file_comparators"]["rules"][3]["mode"] = "regex".into();
    value["file_comparators"]["rules"][3]["expression"] = "[".into();
    let bytes = serde_json::to_vec(&value).unwrap();
    assert!(decode_preferences(&bytes).is_err());
    let accepted_input = decode_preferences_for_acceptance(&bytes).unwrap();
    let error = match comparator_rules::compile(Arc::new(accepted_input.file_comparators)) {
        Ok(_) => panic!("invalid disabled rule compiled"),
        Err(error) => error,
    };
    assert_eq!(error.rule, Some(3));
    value["file_comparators"]["rules"][3]["mode"] = "expression".into();
    assert!(decode_preferences_for_acceptance(&serde_json::to_vec(&value).unwrap()).is_err());
}

#[test]
fn prepared_persistence_requires_exact_policy_and_still_validates_other_preferences() {
    let expected = preferences();
    let compiled = comparator_rules::compile(Arc::new(expected.file_comparators.clone())).unwrap();
    let prepared = PreparedFileComparatorPolicy::new(compiled);
    let bytes = encode_preferences_prepared(expected.clone(), prepared.validation_proof()).unwrap();
    assert_eq!(decode_preferences(&bytes).unwrap(), expected);

    let mut changed = expected.clone();
    changed.file_comparators.rules.swap(0, 1);
    assert!(encode_preferences_prepared(changed, prepared.validation_proof()).is_err());
    let mut changed = expected;
    changed.desktop_font_size = 0;
    assert!(encode_preferences_prepared(changed, prepared.validation_proof()).is_err());
}

#[test]
fn acceptance_decoder_and_file_reader_enforce_the_four_mib_input_budget() {
    let too_large = vec![b' '; midnight_platform_common::settings::MAX_PREFERENCES_BYTES + 1];
    assert!(decode_preferences_for_acceptance(&too_large).is_err());
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("preferences.json");
    std::fs::write(&path, too_large).unwrap();
    assert!(read_preferences_for_acceptance(File::open(&path).unwrap()).is_err());
}

#[cfg(windows)]
#[path = "file_comparator_persistence_windows.rs"]
mod native;
#[cfg(unix)]
#[path = "file_comparator_persistence_unix.rs"]
mod native;
