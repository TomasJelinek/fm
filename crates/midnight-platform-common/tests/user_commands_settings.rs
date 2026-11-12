use midnight_core::*;
use midnight_platform_common::settings::{decode_preferences, encode_preferences};

fn command() -> UserCommand {
    UserCommand {
        id: UserCommandId::new("stable-id").unwrap(),
        name: "Tool 文档".into(),
        enabled: false,
        toolbar_label: Some("Label".into()),
        icon: Some(std::env::current_exe().unwrap()),
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: vec!["${selected.paths}".into(), "literal & $x;".into()],
        },
    }
}

#[test]
fn schema17_round_trips_ordered_native_commands_and_preserves_checksum_settings_and_defaults() {
    let legacy = encode_preferences(UserPreferences::default()).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&legacy).unwrap()["version"],
        10
    );
    assert!(
        decode_preferences(&legacy)
            .unwrap()
            .user_commands
            .is_empty()
    );
    let mut second = command();
    second.id = UserCommandId::new("second").unwrap();
    let mut preferences = UserPreferences {
        user_commands: vec![command(), second].into(),
        checksum_algorithms: ChecksumAlgorithms::all(),
        ..Default::default()
    };
    let encoded = encode_preferences(preferences.clone()).unwrap();
    let stored: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(stored["version"], 17);
    assert_eq!(
        stored["checksum_algorithms"],
        serde_json::json!(["md5", "sha1", "sha256", "sha512", "crc32"])
    );
    assert_eq!(decode_preferences(&encoded).unwrap(), preferences);
    assert_eq!(
        encode_preferences(decode_preferences(&encoded).unwrap()).unwrap(),
        encoded
    );
    preferences.checksum_algorithms = ChecksumAlgorithms::default();
    let encoded = encode_preferences(preferences.clone()).unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&encoded).unwrap()["checksum_algorithms"],
        serde_json::json!(["sha256"])
    );
    assert_eq!(decode_preferences(&encoded).unwrap(), preferences);
}

#[test]
fn malformed_duplicate_and_cross_version_commands_fail_instead_of_dropping_configuration() {
    let preferences = UserPreferences {
        user_commands: vec![command()].into(),
        ..Default::default()
    };
    let encoded = encode_preferences(preferences).unwrap();
    let baseline: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    for mutation in [
        "null",
        "missing",
        "duplicate",
        "unknown",
        "legacy",
        "template",
    ] {
        let mut stored = baseline.clone();
        match mutation {
            "null" => stored["user_commands"] = serde_json::Value::Null,
            "missing" => {
                stored.as_object_mut().unwrap().remove("user_commands");
            }
            "duplicate" => {
                let command = stored["user_commands"][0].clone();
                stored["user_commands"]
                    .as_array_mut()
                    .unwrap()
                    .push(command);
            }
            "unknown" => stored["user_commands"][0]["unexpected"] = true.into(),
            "legacy" => stored["version"] = 16.into(),
            _ => stored["user_commands"][0]["working_directory"] = "${unknown}".into(),
        }
        assert!(
            decode_preferences(&serde_json::to_vec(&stored).unwrap()).is_err(),
            "{mutation}"
        );
    }
}

#[test]
fn terminal_source_and_explicit_scalar_declarations_round_trip_without_template_interpretation() {
    let mut command = command();
    command.invocation = UserCommandInvocation::Terminal {
        shell: ShellFlavor::PowerShell,
        script: "'${literal.source}' ; $env:ME_FILE_PATH".into(),
        variables: vec![
            UserCommandVariable::ActivePath,
            UserCommandVariable::FilePath,
        ],
    };
    let preferences = UserPreferences {
        user_commands: vec![command].into(),
        ..Default::default()
    };
    let encoded = encode_preferences(preferences.clone()).unwrap();
    assert_eq!(decode_preferences(&encoded).unwrap(), preferences);
    let mut stored: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    stored["user_commands"][0]["invocation"]["variables"] = serde_json::json!(["selected.paths"]);
    assert!(decode_preferences(&serde_json::to_vec(&stored).unwrap()).is_err());
    stored["user_commands"][0]["invocation"]
        .as_object_mut()
        .unwrap()
        .remove("variables");
    let decoded = decode_preferences(&serde_json::to_vec(&stored).unwrap()).unwrap();
    assert!(matches!(&decoded.user_commands[0].invocation,
        UserCommandInvocation::Terminal { variables, .. } if variables.is_empty()));
}
#[test]
fn separator_round_trips_order_and_stable_id_without_executable_or_directory_requirements() {
    let encoded = encode_preferences(UserPreferences {
        user_commands: vec![command()].into(),
        ..Default::default()
    })
    .unwrap();
    let mut stored: serde_json::Value = serde_json::from_slice(&encoded).unwrap();
    let mut separator = stored["user_commands"][0].clone();
    separator["id"] = "group-break".into();
    separator["name"] = "Tools".into();
    separator["working_directory"] = "".into();
    separator["invocation"] = serde_json::json!({"mode":"separator"});
    stored["user_commands"]
        .as_array_mut()
        .unwrap()
        .push(separator);
    let decoded = decode_preferences(&serde_json::to_vec(&stored).unwrap())
        .expect("a separator is a valid ordered configuration item without process fields");
    assert_eq!(decoded.user_commands[1].id.as_str(), "group-break");
    assert_eq!(decoded.user_commands[1].name, "Tools");
    let round_trip: serde_json::Value =
        serde_json::from_slice(&encode_preferences(decoded).unwrap()).unwrap();
    assert_eq!(round_trip["user_commands"], stored["user_commands"]);
    // The separator variant is still strict; process data cannot silently accompany it.
    stored["user_commands"][1]["invocation"]["executable"] = "ignored.exe".into();
    assert!(decode_preferences(&serde_json::to_vec(&stored).unwrap()).is_err());
}
#[test]
fn separator_stale_shortcut_is_pruned_on_load_without_losing_other_preferences() {
    let mut second = command();
    second.id = UserCommandId::new("other").unwrap();
    let preferences = UserPreferences {
        user_commands: vec![command(), second.clone()].into(),
        shortcuts: vec![
            ShortcutBinding {
                action: command().id.action(),
                keys: vec!["Ctrl+Alt+Shift+F11".into()],
            },
            ShortcutBinding {
                action: second.id.action(),
                keys: vec!["Ctrl+Alt+Shift+F12".into()],
            },
        ],
        ..Default::default()
    };
    let mut stored: serde_json::Value =
        serde_json::from_slice(&encode_preferences(preferences.clone()).unwrap()).unwrap();
    stored["user_commands"][0]["invocation"] = serde_json::json!({"mode":"separator"});
    stored["user_commands"][0]["working_directory"] = "".into();
    let decoded = decode_preferences(&serde_json::to_vec(&stored).unwrap()).unwrap();
    assert_eq!(decoded.shortcuts, vec![preferences.shortcuts[1].clone()]);
    assert_eq!(decoded.user_commands[1], second);
    assert_eq!(decoded.checksum_algorithms, preferences.checksum_algorithms);
    assert_eq!(
        decode_preferences(&encode_preferences(decoded.clone()).unwrap()).unwrap(),
        decoded
    );
}
