use super::*;

fn command() -> UserCommand {
    UserCommand {
        id: UserCommandId::new("configured-7").unwrap(),
        name: "原命令".into(),
        enabled: true,
        toolbar_label: Some("Tool".into()),
        icon: None,
        working_directory: "${active.path}".into(),
        launch: UserCommandLaunch::Single,
        invocation: UserCommandInvocation::Program {
            executable: absolute("tool.exe"),
            arguments: vec![
                "".into(),
                "${file.path}".into(),
                "a \"quoted\" value".into(),
            ],
        },
    }
}
fn absolute(name: &str) -> PathBuf {
    std::env::temp_dir().join(name)
}

fn assert_raw_parent_edits_are_refused(parent: PathBuf) {
    let mut original = command();
    original.icon = Some(parent.join("original.png"));
    let UserCommandInvocation::Program { executable, .. } = &mut original.invocation else {
        panic!()
    };
    *executable = parent.join("original-tool");
    let unchanged = Form::from_command(&original);
    assert_eq!(unchanged.command(Some(&original)).unwrap(), original);

    let mut executable_form = unchanged.clone();
    executable_form.executable = parent.join("other-tool").to_string_lossy().into_owned();
    executable_form.executable_edited = true;
    assert!(executable_form.command(Some(&original)).is_err());
    let mut icon_form = unchanged;
    icon_form.icon = parent.join("other.png").to_string_lossy().into_owned();
    icon_form.icon_edited = true;
    assert!(icon_form.command(Some(&original)).is_err());

    let root = absolute("source");
    let mut presenter = crate::Presenter::new(root.clone(), root);
    presenter.preferences.loaded = true;
    presenter.preferences.values.user_commands = std::sync::Arc::new(vec![original]);
    presenter.sync_user_command_settings();
    let before = presenter.preferences.values.user_commands.clone();
    let token = presenter.user_command_settings.token();
    let no_submit =
        |_: Command| -> bool { panic!("Ambiguous command configuration was submitted") };
    assert!(!presenter.apply_user_command_setting(&token, &executable_form, &no_submit));
    assert!(!presenter.preview_user_command_setting(&token, &executable_form, &no_submit));
    assert!(!presenter.apply_user_command_setting(&token, &icon_form, &no_submit));
    assert!(std::sync::Arc::ptr_eq(
        &before,
        &presenter.preferences.values.user_commands
    ));
    assert!(presenter.command_launches.jobs.is_empty());
    assert!(!presenter.user_command_settings.error.is_empty());
}

#[test]
fn form_round_trip_keeps_ids_native_literals_unicode_empty_and_quoted_arguments() {
    let original = command();
    let form = Form::from_command(&original);
    assert_eq!(form.command(Some(&original)).unwrap(), original);
    let mut changed = form;
    changed.name = "Renamed 文档".into();
    changed.enabled = false;
    let accepted = changed.command(Some(&original)).unwrap();
    assert_eq!(accepted.id, original.id);
    assert!(!accepted.enabled);
    assert_eq!(accepted.invocation, original.invocation);
}

#[test]
fn form_uses_core_template_group_per_item_and_json_validation_without_mutating_source() {
    let original = command();
    let mut form = Form::from_command(&original);
    form.arguments = "[\"${selected.paths}\"]".into();
    form.launch = 1;
    assert!(
        form.command(Some(&original))
            .unwrap_err()
            .contains("per-item")
    );
    form.launch = 0;
    assert!(form.command(Some(&original)).is_ok());
    form.arguments = "[\"unterminated]".into();
    assert!(form.command(Some(&original)).is_err());
    form.arguments = "[\"${unknown}\"]".into();
    assert!(
        form.command(Some(&original))
            .unwrap_err()
            .contains("Unknown")
    );
    assert_eq!(original.name, "原命令");
}

#[test]
fn terminal_picker_declares_exact_scalar_and_keeps_script_literal() {
    let original = command();
    let mut form = Form::from_command(&original);
    form.kind = 1;
    form.shell = 0;
    form.script = "Write-Output '${literal.source}'".into();
    form.insert_variable(2, UserCommandVariable::FilePath)
        .unwrap();
    let accepted = form.command(Some(&original)).unwrap();
    let UserCommandInvocation::Terminal {
        script,
        variables,
        shell,
    } = accepted.invocation
    else {
        panic!()
    };
    assert_eq!(shell, ShellFlavor::PowerShell);
    assert_eq!(
        script,
        "Write-Output '${literal.source}' \"$env:ME_FILE_PATH\""
    );
    assert_eq!(variables, vec![UserCommandVariable::FilePath]);
    assert!(
        form.insert_variable(2, UserCommandVariable::SelectedPaths)
            .is_err()
    );
    form.shell = 1;
    form.script.clear();
    form.insert_variable(2, UserCommandVariable::ActivePath)
        .unwrap();
    assert_eq!(form.script, "\"$ME_ACTIVE_PATH\"");
}

#[test]
fn program_picker_adds_separate_argument_and_directory_group_is_rejected() {
    let original = command();
    let mut form = Form::from_command(&original);
    form.insert_variable(1, UserCommandVariable::SelectedPaths)
        .unwrap();
    let UserCommandInvocation::Program { arguments, .. } =
        form.command(Some(&original)).unwrap().invocation
    else {
        panic!()
    };
    assert_eq!(arguments.last().unwrap(), "${selected.paths}");
    assert_eq!(arguments[0], "");
    assert!(
        form.insert_variable(0, UserCommandVariable::SelectedPaths)
            .is_err()
    );
    form.working_directory.clear();
    form.insert_variable(0, UserCommandVariable::OtherPath)
        .unwrap();
    assert_eq!(form.working_directory, "${other.path}");
}

#[test]
fn argument_limit_is_checked_before_parsing_and_valid_escaped_configuration_round_trips() {
    let original = command();
    let mut form = Form::from_command(&original);
    form.arguments = serde_json::to_string(&vec![""; 65]).unwrap();
    assert!(form.command(Some(&original)).unwrap_err().contains("64"));
    form.arguments = serde_json::to_string(&vec!["a,b,\\\""; 64]).unwrap();
    assert!(
        form.command(Some(&original)).is_ok(),
        "commas inside strings are not argument separators"
    );
    let mut original = original;
    let UserCommandInvocation::Program { arguments, .. } = &mut original.invocation else {
        panic!()
    };
    *arguments = vec!["\u{1}".repeat(800_000)];
    original.validate().unwrap();
    let form = Form::from_command(&original);
    assert!(
        form.arguments.len() > 4 * 1024 * 1024,
        "JSON escaping is a presentation expansion"
    );
    assert_eq!(form.command(Some(&original)).unwrap(), original);
}

#[cfg(unix)]
#[path = "form_unix_tests.rs"]
mod unix;
#[cfg(windows)]
#[path = "form_windows_tests.rs"]
mod windows;

#[test]
fn cmd_picker_declares_raw_scalar_and_inserts_complete_native_argument_companion() {
    let original = command();
    let mut form = Form::from_command(&original);
    form.kind = 1;
    form.shell = 2;
    form.script = "program.exe literal-${not-a-template}".into();
    form.insert_variable(2, UserCommandVariable::FileExtension)
        .unwrap();
    let accepted = form.command(Some(&original)).unwrap();
    let UserCommandInvocation::Terminal {
        script,
        variables,
        shell,
    } = accepted.invocation
    else {
        panic!()
    };
    assert_eq!(shell, ShellFlavor::Cmd);
    assert_eq!(
        script,
        "program.exe literal-${not-a-template} !ME_FILE_EXTENSION_ARG!"
    );
    assert_eq!(variables, vec![UserCommandVariable::FileExtension]);
    assert!(
        form.insert_variable(2, UserCommandVariable::SelectedPaths)
            .is_err()
    );
}

#[test]
fn separator_mode_is_inert_and_switching_back_keeps_structured_program_data() {
    let original = command();
    let mut form = Form::from_command(&original);
    form.kind = 2;
    form.script = "echo preserved terminal draft".into();
    let separator = form.command(Some(&original)).unwrap();
    assert_eq!(separator.id, original.id);
    assert!(matches!(
        separator.invocation,
        UserCommandInvocation::Separator
    ));
    assert_eq!(Form::from_command(&separator).kind, 2);
    form.kind = 0;
    assert_eq!(form.command(Some(&original)).unwrap(), original);
    form.kind = 1;
    assert!(matches!(form.command(Some(&original)).unwrap().invocation,
        UserCommandInvocation::Terminal { script, .. } if script == "echo preserved terminal draft"));
    let mut blank = Form::new("separator-new".into());
    blank.kind = 2;
    blank.working_directory.clear();
    assert!(
        blank.command(None).is_ok(),
        "A separator requires no executable or directory"
    );
}

#[test]
fn newly_chosen_icon_requires_imported_native_result_and_legacy_unchanged_path_survives() {
    let mut original = command();
    original.icon = Some(absolute("legacy-original.png"));
    let mut draft = Form::from_command(&original);
    assert_eq!(draft.command(Some(&original)).unwrap(), original);
    let imported = absolute("managed-copy.png");
    draft.icon = imported.to_string_lossy().into_owned();
    draft.icon_edited = true;
    assert!(
        draft
            .command(Some(&original))
            .unwrap_err()
            .contains("independent copy")
    );
    assert_eq!(
        draft
            .command_with_native(Some(&original), None, Some(&imported))
            .unwrap()
            .icon,
        Some(imported)
    );
    draft.icon.clear();
    assert!(draft.command(Some(&original)).unwrap().icon.is_none());
}
