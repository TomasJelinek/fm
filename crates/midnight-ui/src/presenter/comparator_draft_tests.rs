use super::*;

fn program(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("_fixture")
        .join(name)
}

fn preferences() -> FileComparatorPreferences {
    FileComparatorPreferences {
        rules: vec![FileComparatorRule {
            enabled: true,
            mode: FilterMode::Masks,
            expression: "*.c;*.cpp".into(),
            case_sensitive: false,
            target: FileComparatorTarget::External(FileComparatorProgram {
                executable: program("compare"),
                arguments: vec!["{left}".into(), "{right}".into(), "--literal={left}".into()],
                working_directory: Some(program("working folder")),
            }),
        }],
    }
}

#[test]
fn target_and_selection_changes_retain_incomplete_external_fields_and_literal_arguments() {
    let mut draft = Draft::from_preferences(&preferences());
    assert!(draft.select(0));
    let mut form = draft.selected_rule().unwrap().form();
    form.executable = "".into();
    form.working_directory = "".into();
    form.target = 1;
    assert!(draft.edit_selected(form));
    assert!(draft.select(1)); // The immutable fallback is a selectable summary.
    assert!(draft.select(0));
    let mut form = draft.selected_rule().unwrap().form();
    assert_eq!(form.executable, "");
    form.target = 3;
    assert!(draft.edit_selected(form));
    let rule = draft.selected_rule().unwrap();
    assert_eq!(
        rule.external.arguments,
        ["{left}", "{right}", "--literal={left}"]
    );
    assert_eq!(rule.external.executable, "");
    assert!(draft.candidate().is_err());
}

#[test]
fn reorder_and_remove_keep_rule_identity_and_never_modify_the_fallback() {
    let mut draft = Draft::from_preferences(&preferences());
    assert!(draft.select(0));
    let first = draft.selected_rule().unwrap().id;
    assert!(draft.action(0));
    let second = draft.selected_rule().unwrap().id;
    assert_ne!(first, second);
    assert!(draft.action(2));
    assert_eq!(draft.selected_index(), Some(0));
    assert_eq!(draft.selected_rule().unwrap().id, second);
    assert!(draft.action(1));
    assert_eq!(draft.selected_rule().unwrap().id, first);
    assert!(draft.select(draft.rules.len()));
    assert!(!draft.action(1));
    assert!(!draft.action(2));
    assert!(!draft.action(3));
    assert_eq!(draft.rules.len(), 1);
    assert!(draft.action(0));
    assert_ne!(draft.selected_rule().unwrap().id, second);
}

#[test]
fn argument_rows_preserve_whitespace_quotes_empty_values_and_whole_item_placeholders() {
    let mut draft = Draft::from_preferences(&preferences());
    draft.select(0);
    for value in ["", "one two", "\"quotes\"", "{left}", "--right={right}"] {
        assert!(draft.change_arguments(1, 0, value));
    }
    let candidate = draft.candidate().unwrap();
    let FileComparatorTarget::External(program) = &candidate.rules[0].target else {
        panic!("external target changed");
    };
    assert_eq!(
        program.arguments,
        [
            "{left}",
            "{right}",
            "--literal={left}",
            "",
            "one two",
            "\"quotes\"",
            "{left}",
            "--right={right}"
        ]
    );
}

#[test]
fn invalid_disabled_rule_reports_its_ordered_row_and_arguments_field() {
    let mut draft = Draft::from_preferences(&preferences());
    draft.select(0);
    draft.selected_rule_mut().unwrap().enabled = false;
    draft.selected_rule_mut().unwrap().external.arguments = vec!["{left}".into()];
    let diagnostic = draft.candidate().unwrap_err();
    assert_eq!(diagnostic.rule, Some(0));
    assert_eq!(diagnostic.field, FileComparatorField::Arguments);
}

#[test]
fn malformed_form_indices_do_not_silently_select_a_different_policy() {
    let preferences = preferences();
    let mut draft = Draft::from_preferences(&preferences);
    draft.select(0);
    let mut form = draft.selected_rule().unwrap().form();
    form.target = 4;
    assert!(!draft.edit_selected(form));
    let mut form = draft.selected_rule().unwrap().form();
    form.mode = 2;
    assert!(!draft.edit_selected(form));
    assert_eq!(draft.candidate().unwrap(), preferences);
}

#[test]
fn untouched_native_program_and_cwd_survive_argument_only_edits() {
    let preferences = preferences();
    let mut draft = Draft::from_preferences(&preferences);
    draft.select(0);
    assert!(draft.change_arguments(1, 0, ""));
    let FileComparatorTarget::External(original) = &preferences.rules[0].target else {
        unreachable!();
    };
    let candidate = draft.candidate().unwrap();
    let FileComparatorTarget::External(saved) = &candidate.rules[0].target else {
        unreachable!();
    };
    assert_eq!(
        saved.executable.as_os_str(),
        original.executable.as_os_str()
    );
    assert_eq!(saved.working_directory, original.working_directory);
}

#[cfg(windows)]
#[test]
fn raw_windows_program_and_cwd_are_not_rebuilt_from_lossy_labels() {
    use std::os::windows::ffi::OsStringExt;
    let raw = PathBuf::from(std::ffi::OsString::from_wide(&[
        67, 58, 92, 116, 101, 115, 116, 92, 0xd800,
    ]));
    let mut preferences = preferences();
    let FileComparatorTarget::External(program) = &mut preferences.rules[0].target else {
        unreachable!();
    };
    program.executable = raw.join("compare.exe");
    program.working_directory = Some(raw.clone());
    let mut draft = Draft::from_preferences(&preferences);
    draft.select(0);
    draft.change_arguments(1, 0, "");
    let candidate = draft.candidate().unwrap();
    let FileComparatorTarget::External(saved) = &candidate.rules[0].target else {
        unreachable!();
    };
    assert_eq!(saved.executable, raw.join("compare.exe"));
    assert_eq!(saved.working_directory.as_deref(), Some(raw.as_path()));
    let mut form = draft.selected_rule().unwrap().form();
    form.executable = format!("{}-edited", form.executable).into();
    draft.edit_selected(form);
    let diagnostic = draft.candidate().unwrap_err();
    assert_eq!(diagnostic.field, FileComparatorField::Executable);
}

#[cfg(unix)]
#[test]
fn raw_unix_program_and_cwd_are_not_rebuilt_from_lossy_labels() {
    use std::os::unix::ffi::OsStringExt;
    let raw =
        Path::new(env!("CARGO_MANIFEST_DIR")).join(std::ffi::OsString::from_vec(vec![b'p', 0xff]));
    let mut preferences = preferences();
    let FileComparatorTarget::External(program) = &mut preferences.rules[0].target else {
        unreachable!();
    };
    program.executable = raw.join("compare");
    program.working_directory = Some(raw.clone());
    let mut draft = Draft::from_preferences(&preferences);
    draft.select(0);
    draft.change_arguments(1, 0, "");
    let candidate = draft.candidate().unwrap();
    let FileComparatorTarget::External(saved) = &candidate.rules[0].target else {
        unreachable!();
    };
    assert_eq!(
        saved.executable.as_os_str(),
        raw.join("compare").as_os_str()
    );
    assert_eq!(saved.working_directory.as_deref(), Some(raw.as_path()));
    let mut form = draft.selected_rule().unwrap().form();
    form.working_directory = format!("{}-edited", form.working_directory).into();
    draft.edit_selected(form);
    assert_eq!(
        draft.candidate().unwrap_err().field,
        FileComparatorField::WorkingDirectory
    );
}

#[cfg(any(unix, windows))]
use crate::raw_picker_paths;

fn assert_picked_native_field(draft: &Draft, directory: bool, expected: &Path) {
    let external = &draft.selected_rule().unwrap().external;
    let (display, native) = if directory {
        (
            &external.working_directory,
            external.native_working_directory.as_deref(),
        )
    } else {
        (&external.executable, external.native_executable.as_deref())
    };
    assert_eq!(display.as_str(), expected.to_string_lossy().as_ref());
    assert_eq!(native.unwrap().as_os_str(), expected.as_os_str());
}

#[test]
fn identical_native_picker_values_are_noops_for_program_and_directory() {
    for directory in [false, true] {
        let preferences = preferences();
        let mut draft = Draft::from_preferences(&preferences);
        assert!(draft.select(0));
        let path = program(if directory {
            "working folder"
        } else {
            "compare"
        });
        for _ in 0..2 {
            assert!(!draft.picked(path.clone(), directory));
            assert_picked_native_field(&draft, directory, &path);
            assert_eq!(draft.candidate().unwrap(), preferences);
        }
    }
}

#[test]
fn native_picker_adopts_missing_identity_and_restores_edited_display() {
    for directory in [false, true] {
        let mut draft = Draft::from_preferences(&preferences());
        assert!(draft.select(0));
        let path = program(if directory {
            "working folder"
        } else {
            "compare"
        });
        let external = &mut draft.selected_rule_mut().unwrap().external;
        if directory {
            external.native_working_directory = None;
        } else {
            external.native_executable = None;
        }
        assert!(draft.picked(path.clone(), directory));
        assert_picked_native_field(&draft, directory, &path);
        assert!(!draft.picked(path.clone(), directory));

        let mut form = draft.selected_rule().unwrap().form();
        if directory {
            form.working_directory = program("typed other folder")
                .to_string_lossy()
                .into_owned()
                .into();
        } else {
            form.executable = program("typed other program")
                .to_string_lossy()
                .into_owned()
                .into();
        }
        assert!(draft.edit_selected(form));
        assert!(draft.picked(path.clone(), directory));
        assert_picked_native_field(&draft, directory, &path);
        assert!(!draft.picked(path, directory));
    }
}

#[cfg(any(unix, windows))]
#[test]
fn native_picker_adopts_distinct_raw_paths_with_identical_lossy_displays() {
    let [first, second] = raw_picker_paths::same_display(&program("raw picker"));
    assert_ne!(first.as_os_str(), second.as_os_str());
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    for directory in [false, true] {
        let first = if directory {
            first.clone()
        } else {
            first.join("compare.exe")
        };
        let second = if directory {
            second.clone()
        } else {
            second.join("compare.exe")
        };
        let mut preferences = preferences();
        let FileComparatorTarget::External(external) = &mut preferences.rules[0].target else {
            unreachable!();
        };
        if directory {
            external.working_directory = Some(first);
        } else {
            external.executable = first;
        }
        let mut draft = Draft::from_preferences(&preferences);
        assert!(draft.select(0));
        let before = draft.selected_rule().unwrap().form();
        assert!(draft.picked(second.clone(), directory));
        assert_picked_native_field(&draft, directory, &second);
        let after = draft.selected_rule().unwrap().form();
        assert_eq!(after.executable, before.executable);
        assert_eq!(after.working_directory, before.working_directory);
        assert!(!draft.picked(second, directory));
    }
}

#[test]
fn native_picker_compares_raw_spelling_instead_of_normalized_components() {
    let base = program("picker folder");
    let mut spelling = base.clone().into_os_string();
    spelling.push(std::path::MAIN_SEPARATOR_STR);
    spelling.push(".");
    spelling.push(std::path::MAIN_SEPARATOR_STR);
    spelling.push("chosen");
    let original = PathBuf::from(spelling);
    let chosen = base.join("chosen");
    assert_eq!(
        original, chosen,
        "Path equality normalizes the interior dot"
    );
    assert_ne!(original.as_os_str(), chosen.as_os_str());
    for directory in [false, true] {
        let mut preferences = preferences();
        let FileComparatorTarget::External(external) = &mut preferences.rules[0].target else {
            unreachable!();
        };
        if directory {
            external.working_directory = Some(original.clone());
        } else {
            external.executable = original.clone();
        }
        let mut draft = Draft::from_preferences(&preferences);
        assert!(draft.select(0));
        let mut form = draft.selected_rule().unwrap().form();
        if directory {
            form.working_directory = chosen.to_string_lossy().into_owned().into();
        } else {
            form.executable = chosen.to_string_lossy().into_owned().into();
        }
        assert!(draft.edit_selected(form));
        assert!(draft.picked(chosen.clone(), directory));
        assert_picked_native_field(&draft, directory, &chosen);
        assert!(!draft.picked(chosen.clone(), directory));
    }
}

#[test]
fn native_picker_preserves_unselected_fallback_and_non_external_guards() {
    let mut draft = Draft::from_preferences(&preferences());
    for directory in [false, true] {
        assert!(!draft.picked(program("unselected"), directory));
    }
    assert!(draft.select(0));
    assert!(draft.select(draft.rules.len()));
    let before = draft.candidate().unwrap();
    for directory in [false, true] {
        assert!(!draft.picked(program("fallback"), directory));
        assert_eq!(draft.candidate().unwrap(), before);
    }
    assert!(draft.select(0));
    let mut form = draft.selected_rule().unwrap().form();
    form.target = 1;
    assert!(draft.edit_selected(form));
    let before = draft.candidate().unwrap();
    for directory in [false, true] {
        assert!(!draft.picked(program("not external"), directory));
        assert_eq!(draft.candidate().unwrap(), before);
    }
}
