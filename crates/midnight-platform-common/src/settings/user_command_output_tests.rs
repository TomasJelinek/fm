//! Schema 39: where a Run directly command's output goes (Options › Custom Commands).
use super::{decode_preferences, decode_settings_file, encode_preferences, encode_settings_file};
use midnight_core::*;
use serde_json::{Value, json};

fn program(id: &str, output: UserCommandOutput) -> UserCommand {
    UserCommand {
        id: UserCommandId::new(id).unwrap(),
        name: "git clean".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: vec!["clean".into(), "-fxd".into()],
        },
        output,
    }
}

fn with(commands: Vec<UserCommand>) -> UserPreferences {
    UserPreferences {
        user_commands: commands.into(),
        ..Default::default()
    }
}

fn stored(preferences: &UserPreferences) -> Value {
    serde_json::from_slice(&encode_preferences(preferences.clone()).unwrap()).unwrap()
}

fn decoded(value: &Value) -> std::io::Result<UserPreferences> {
    decode_preferences(&serde_json::to_vec(value).unwrap())
}

#[test]
fn hidden_output_keeps_the_previous_schema_and_older_commands_read_as_hidden() {
    let value = stored(&with(vec![program("hidden", UserCommandOutput::default())]));
    assert_eq!(value["version"], 17);
    assert!(
        value["user_commands"][0]["invocation"]
            .get("output")
            .is_none()
    );
    // What a schema 38 (or earlier) file holds is a command without an output window:
    // the migration keeps its behavior.
    let schema_38 = stored(&UserPreferences {
        wheel_speed: WheelSpeed::Fast,
        ..with(vec![program("hidden", UserCommandOutput::default())])
    });
    assert_eq!(schema_38["version"], 38);
    let read = decoded(&schema_38).unwrap();
    assert_eq!(read.user_commands[0].output, UserCommandOutput::default());
    assert_eq!(
        read.user_commands[0].output.mode,
        UserCommandOutputMode::Hidden
    );
}

#[test]
fn an_output_choice_writes_schema_39_and_round_trips() {
    for (mode, window) in [
        (UserCommandOutputMode::Window, "output"),
        (UserCommandOutputMode::Console, "console"),
        (UserCommandOutputMode::Hidden, "hidden"),
    ] {
        for close_on_success in [false, true] {
            let output = UserCommandOutput {
                mode,
                close_on_success,
            };
            if output == UserCommandOutput::default() {
                continue;
            }
            let preferences = with(vec![
                program("plain", UserCommandOutput::default()),
                program("shown", output),
            ]);
            let value = stored(&preferences);
            assert_eq!(value["version"], 39, "{output:?}");
            let invocation = &value["user_commands"][1]["invocation"];
            assert_eq!(invocation["output"]["window"], window);
            assert_eq!(
                invocation["output"].get("close_on_success").is_some(),
                close_on_success
            );
            assert!(
                value["user_commands"][0]["invocation"]
                    .get("output")
                    .is_none()
            );
            // Fields that schemas 30 to 34 require are present.
            assert_eq!(value["file_comparators"], json!({"rules": []}));
            assert_eq!(decoded(&value).unwrap(), preferences, "{output:?}");
        }
    }
}

#[test]
fn the_output_choice_survives_settings_export_and_import() {
    let preferences = with(vec![program("shown", UserCommandOutput::NEW_PROGRAM)]);
    let file = encode_settings_file(preferences.clone()).unwrap();
    let imported = decode_settings_file(&file).unwrap();
    assert_eq!(
        imported.preferences.user_commands,
        preferences.user_commands
    );
}

#[test]
fn schema_39_fields_are_versioned_and_strict() {
    let valid = stored(&with(vec![program(
        "shown",
        UserCommandOutput::NEW_PROGRAM,
    )]));
    assert!(decoded(&valid).is_ok());
    // An earlier schema cannot carry an output choice: a schema 38 file that is valid
    // without it is refused with it.
    let mut premature = valid.clone();
    premature["version"] = 38.into();
    premature["wheel_speed"] = 150.into();
    assert!(decoded(&premature).is_err());
    let mut schema_38 = valid.clone();
    schema_38["version"] = 38.into();
    schema_38["wheel_speed"] = 150.into();
    schema_38["user_commands"][0]["invocation"]
        .as_object_mut()
        .unwrap()
        .remove("output");
    assert!(
        decoded(&schema_38).is_ok(),
        "the same file without the choice"
    );
    // A schema 39 document needs one.
    let mut missing = valid.clone();
    missing["user_commands"][0]["invocation"]
        .as_object_mut()
        .unwrap()
        .remove("output");
    assert!(decoded(&missing).is_err());
    for invalid in [
        Value::Null,
        json!({}),
        json!({"window": "popup"}),
        json!({"window": "output", "close_on_success": "yes"}),
        json!({"window": "output", "colour": true}),
        json!("output"),
    ] {
        let mut mutated = valid.clone();
        mutated["user_commands"][0]["invocation"]["output"] = invalid.clone();
        assert!(decoded(&mutated).is_err(), "{invalid}");
    }
    // Only Run directly commands have one.
    let mut terminal = valid.clone();
    terminal["user_commands"][0]["invocation"] = json!({
        "mode": "terminal",
        "shell": if cfg!(windows) { "powershell" } else { "posix" },
        "script": "git clean -fxd",
        "output": {"window": "output"},
    });
    assert!(decoded(&terminal).is_err());
    let mut future = valid;
    future["version"] = 40.into();
    assert_eq!(
        decoded(&future).unwrap_err().to_string(),
        "unsupported preferences version"
    );
}
