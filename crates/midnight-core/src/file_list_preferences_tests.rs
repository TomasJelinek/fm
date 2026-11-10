use super::*;

#[test]
fn file_list_defaults_are_salamander_names_to_clipboard_without_append() {
    let preferences = FileListPreferences::default();
    assert_eq!(preferences.template_history, ["$(FileName)$(CRLF)"]);
    assert_eq!(preferences.destination, 0);
    assert_eq!(preferences.path, None);
    assert!(!preferences.append);
    preferences.validate().unwrap();
}

#[test]
fn file_list_history_is_unique_bounded_mru_and_rejected_edits_change_nothing() {
    let mut preferences = FileListPreferences::default();
    for index in 0..35 {
        preferences
            .remember(&format!("{index}: $(FileName)$(LF)"))
            .unwrap();
    }
    assert_eq!(preferences.template_history.len(), 30);
    assert_eq!(preferences.template_history[0], "34: $(FileName)$(LF)");
    assert_eq!(preferences.template_history[29], "5: $(FileName)$(LF)");
    preferences.remember("10: $(FileName)$(LF)").unwrap();
    assert_eq!(preferences.template_history[0], "10: $(FileName)$(LF)");
    assert_eq!(preferences.template_history.len(), 30);
    let accepted = preferences.clone();
    for invalid in [String::new(), "$(Unknown)".into(), "x".repeat(65537)] {
        assert!(preferences.remember(&invalid).is_err());
        assert_eq!(preferences, accepted);
    }
}

#[test]
fn file_list_validation_rejects_duplicate_empty_invalid_and_excessive_histories() {
    for history in [
        vec!["x".into(), "x".into()],
        vec![String::new()],
        vec!["$(Unknown)".into()],
        vec!["x".repeat(65537)],
        (0..31).map(|index| index.to_string()).collect(),
    ] {
        assert!(
            FileListPreferences {
                template_history: history,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
    }
    assert!(
        FileListPreferences {
            template_history: Vec::new(),
            ..Default::default()
        }
        .validate()
        .is_ok()
    );
}

#[test]
fn file_list_paths_and_output_choices_validate_without_io_or_max_path_limits() {
    let path = std::env::current_dir()
        .unwrap()
        .join("資料".repeat(400))
        .join("list.txt");
    for destination in [0, 1, 2] {
        FileListPreferences {
            destination,
            path: Some(path.clone()),
            ..Default::default()
        }
        .validate()
        .unwrap();
    }
    for preferences in [
        FileListPreferences {
            destination: 3,
            ..Default::default()
        },
        FileListPreferences {
            path: Some("relative.txt".into()),
            ..Default::default()
        },
        FileListPreferences {
            path: Some(path.join("\0")),
            ..Default::default()
        },
    ] {
        assert!(preferences.validate().is_err());
    }
}

#[test]
fn file_list_reset_preserves_history_and_last_path_while_resetting_output_choices() {
    let mut preferences = FileListPreferences {
        template_history: vec!["$(Drive)$(Path)$(FileName)$(LF)".into()],
        destination: 2,
        path: Some(std::env::current_dir().unwrap().join("list.txt")),
        append: true,
    };
    let history = preferences.template_history.clone();
    let path = preferences.path.clone();
    preferences.reset_options();
    assert_eq!(preferences.template_history, history);
    assert_eq!(preferences.path, path);
    assert_eq!(preferences.destination, 0);
    assert!(!preferences.append);
}
