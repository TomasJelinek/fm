#[path = "../src/file_targets.rs"]
mod file_targets;
#[path = "../src/user_command_prepare.rs"]
mod prepare;

use midnight_core::*;
use std::{fs, path::Path, sync::Arc};

fn configuration(
    executable: &Path,
    arguments: &[&str],
    launch: UserCommandLaunch,
) -> Arc<Vec<UserCommand>> {
    Arc::new(vec![UserCommand {
        id: UserCommandId::new("tool").unwrap(),
        name: "Tool".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch,
        invocation: UserCommandInvocation::Program {
            executable: executable.to_owned(),
            arguments: arguments.iter().map(|value| (*value).to_owned()).collect(),
        },
    }])
}
fn capture(directory: &Path, selected: &[&str], focus: &str) -> UserCommandContext {
    let entries = selected
        .iter()
        .enumerate()
        .map(|(index, name)| {
            Entry::new(EntryId(index as u64), directory.join(name), EntryKind::File)
        })
        .collect::<Vec<_>>();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let bits = if selected.is_empty() {
        vec![]
    } else {
        vec![(1u64 << selected.len()) - 1]
    };
    let selection = Arc::new(SelectionSnapshot::from_bits(view.clone(), bits).unwrap());
    UserCommandContext {
        active: Some(directory.to_owned()),
        other: Some(directory.to_owned()),
        files: Some(SelectedFiles {
            selection,
            view,
            fallback: Some(directory.join(focus)),
        }),
        focus: Some(Entry::new(
            EntryId(99),
            directory.join(focus),
            EntryKind::File,
        )),
    }
}

#[test]
fn single_focus_and_group_selection_are_captured_literally_without_focus_substitution() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["selected & $x;", "focus"] {
        fs::write(temp.path().join(name), b"data").unwrap();
    }
    let request = UserCommandRequest {
        id: UserCommandRequestId(1),
        command: UserCommandId::new("tool").unwrap(),
        configuration: configuration(
            &std::env::current_exe().unwrap(),
            &["${file.name}", "${selected.paths}"],
            UserCommandLaunch::Single,
        ),
        context: capture(temp.path(), &["selected & $x;"], "focus"),
    };
    let service = midnight_platform::LocalFileSystem::new(None);
    let prepared = prepare::prepare(&request, &service, &Cancellation::default()).unwrap();
    assert_eq!(prepared.processes.len(), 1);
    if let UserCommandProcessInvocation::Program { arguments, .. } =
        &prepared.processes[0].invocation
    {
        assert_eq!(
            arguments,
            &[
                "focus".into(),
                temp.path().join("selected & $x;").into_os_string()
            ]
        );
    } else {
        panic!("unexpected terminal plan");
    }
    fs::remove_file(temp.path().join("selected & $x;")).unwrap();
    assert!(prepared.revalidate(&Cancellation::default()).is_err());
    assert!(prepare::prepare(&request, &service, &Cancellation::default()).is_err());
}

#[test]
fn per_item_prepares_every_plan_and_refuses_a_later_missing_input_before_any_spawn() {
    let temp = tempfile::tempdir().unwrap();
    for name in ["first", "second", "focus"] {
        fs::write(temp.path().join(name), b"data").unwrap();
    }
    let mut request = UserCommandRequest {
        id: UserCommandRequestId(2),
        command: UserCommandId::new("tool").unwrap(),
        configuration: configuration(
            &std::env::current_exe().unwrap(),
            &["${file.path}"],
            UserCommandLaunch::PerItem,
        ),
        context: capture(temp.path(), &["first", "second"], "focus"),
    };
    let service = midnight_platform::LocalFileSystem::new(None);
    let prepared = prepare::prepare(&request, &service, &Cancellation::default()).unwrap();
    assert_eq!(prepared.processes.len(), 2);
    fs::remove_file(temp.path().join("second")).unwrap();
    assert!(prepared.revalidate_for(0, &Cancellation::default()).is_ok());
    assert!(
        prepared
            .revalidate_for(1, &Cancellation::default())
            .is_err()
    );
    drop(prepared);
    assert!(prepare::prepare(&request, &service, &Cancellation::default()).is_err());
    request.context.active = None;
    assert!(prepare::prepare(&request, &service, &Cancellation::default()).is_err());
}

#[test]
fn terminal_source_is_literal_and_only_declared_captured_values_become_native_environment() {
    let temp = tempfile::tempdir().unwrap();
    fs::write(temp.path().join("focus"), b"data").unwrap();
    let mut commands = (*configuration(
        &std::env::current_exe().unwrap(),
        &[],
        UserCommandLaunch::Single,
    ))
    .clone();
    commands[0].invocation = UserCommandInvocation::Terminal {
        shell: ShellFlavor::PowerShell,
        script: "'${user.authored.script}' ; $env:ME_FILE_PATH".into(),
        variables: vec![UserCommandVariable::FilePath],
    };
    let mut request = UserCommandRequest {
        id: UserCommandRequestId(3),
        command: UserCommandId::new("tool").unwrap(),
        configuration: Arc::new(commands),
        context: capture(temp.path(), &[], "focus"),
    };
    let service = midnight_platform::LocalFileSystem::new(None);
    let prepared = prepare::prepare(&request, &service, &Cancellation::default()).unwrap();
    assert!(
        matches!(&prepared.processes[0].invocation, UserCommandProcessInvocation::Terminal { script, environment, .. }
        if script == "'${user.authored.script}' ; $env:ME_FILE_PATH" && environment == &vec![("ME_FILE_PATH".to_owned(), temp.path().join("focus").into_os_string())])
    );
    drop(prepared);
    request.context.focus = None;
    assert!(prepare::prepare(&request, &service, &Cancellation::default()).is_err());
}

#[test]
fn grouped_selection_can_exceed_stored_template_count_without_truncating_native_argv() {
    let temp = tempfile::tempdir().unwrap();
    let entries = (0..65)
        .map(|index| {
            let path = temp.path().join(format!("item-{index}"));
            fs::write(&path, b"").unwrap();
            Entry::new(EntryId(index), path, EntryKind::File)
        })
        .collect::<Vec<_>>();
    let view = Arc::new(DirectoryIndex::from_entries(&entries));
    let selection =
        Arc::new(SelectionSnapshot::from_bits(view.clone(), vec![u64::MAX, 1]).unwrap());
    let request = UserCommandRequest {
        id: UserCommandRequestId(4),
        command: UserCommandId::new("tool").unwrap(),
        configuration: configuration(
            &std::env::current_exe().unwrap(),
            &["${selected.paths}"],
            UserCommandLaunch::Single,
        ),
        context: UserCommandContext {
            active: Some(temp.path().to_owned()),
            other: None,
            files: Some(SelectedFiles {
                selection,
                view,
                fallback: None,
            }),
            focus: None,
        },
    };
    let prepared = prepare::prepare(
        &request,
        &midnight_platform::LocalFileSystem::new(None),
        &Cancellation::default(),
    )
    .unwrap();
    if let UserCommandProcessInvocation::Program { arguments, .. } =
        &prepared.processes[0].invocation
    {
        assert_eq!(arguments.len(), 65);
        assert_eq!(arguments[64], entries[64].path.as_os_str());
    } else {
        panic!("unexpected terminal");
    }
}

#[test]
fn disabled_command_preview_is_allowed_but_execution_remains_disabled() {
    let temp = tempfile::tempdir().unwrap();
    let mut commands = (*configuration(
        &std::env::current_exe().unwrap(),
        &["literal"],
        UserCommandLaunch::Single,
    ))
    .clone();
    commands[0].enabled = false;
    let request = UserCommandRequest {
        id: UserCommandRequestId(5),
        command: UserCommandId::new("tool").unwrap(),
        configuration: Arc::new(commands),
        context: UserCommandContext {
            active: Some(temp.path().to_owned()),
            other: None,
            files: None,
            focus: None,
        },
    };
    let native = midnight_platform::LocalFileSystem::new(None);
    let preview = prepare::prepare_preview(&request, &native, &Cancellation::default())
        .unwrap()
        .into_preview(UserCommandRetention::new(()))
        .unwrap();
    assert_eq!(preview.processes.len(), 1);
    assert!(prepare::prepare(&request, &native, &Cancellation::default()).is_err());
}
