//! Installed toolbar callbacks, typed tree capture and real native preparation.
#[path = "support/tree_move_native.rs"]
mod native;
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;
use midnight_core::*;
use slint::Model;
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Default)]
struct Launches(Mutex<Vec<(Vec<OsString>, PathBuf)>>);
impl TerminalService for Launches {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("program fixture")
    }
}
struct Exited;
impl TerminalProcess for Exited {
    fn id(&self) -> u32 {
        0
    }
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        #[cfg(unix)]
        use std::os::unix::process::ExitStatusExt;
        #[cfg(windows)]
        use std::os::windows::process::ExitStatusExt;
        Ok(Some(std::process::ExitStatus::from_raw(0)))
    }
}
impl UserCommandService for Launches {
    fn launch_user_program(
        &self,
        _: &Path,
        args: &[OsString],
        cwd: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        assert!(!cancel.is_cancelled());
        self.0.lock().unwrap().push((args.to_vec(), cwd.to_owned()));
        Ok(Box::new(Exited))
    }
}
fn exercise(pane: PaneId, selected: bool) {
    let fixture = native::Fixture::new();
    let native = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let launches = Arc::new(Launches::default());
    let command = UserCommand {
        id: UserCommandId::new("tree-command").unwrap(),
        name: "Tree command".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: "${active.path}".into(),
        launch: if selected {
            UserCommandLaunch::PerItem
        } else {
            UserCommandLaunch::Single
        },
        invocation: UserCommandInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: vec!["${file.path}".into()],
        },
    };
    let mut h = support::Harness::with_native_user_commands(
        pane,
        fixture.root.clone(),
        native,
        launches.clone(),
        vec![command],
    );
    h.ui.window.invoke_activate(pane.index() as i32);
    for name in if selected {
        vec!["small", "root.txt"]
    } else {
        vec!["small"]
    } {
        let row = h.rows().iter().position(|row| row.name == name).unwrap();
        h.ui.window
            .invoke_row_click(pane.index() as i32, row as i32, selected, false);
    }
    if selected {
        h.until(|h| h.selection.as_ref().is_some_and(|s| s.len() == 2));
    }
    let command = h.ui.window.get_user_commands().row_data(0).unwrap();
    h.ui.window
        .invoke_execute_user_command(command.identity, command.revision);
    let mut captured = false;
    loop {
        let event = h.receive();
        if let Event::Tree(tree) = &event
            && let TreeEvent::UserCommandCaptured { stamp, result, .. } = tree.as_ref()
        {
            assert_eq!(stamp.pane, pane);
            assert!(result.is_ok(), "{result:?}");
            captured = true;
        }
        let finished = if let Event::UserCommand(event) = &event {
            match &event.update {
                UserCommandUpdate::Rejected(error) => panic!("command rejected: {error}"),
                UserCommandUpdate::Finished(summary) => {
                    assert!(
                        summary.error.is_none(),
                        "native command failed: {summary:?}"
                    );
                    assert_eq!(summary.started, if selected { 2 } else { 1 });
                    true
                }
                _ => false,
            }
        } else {
            false
        };
        h.apply_event(event);
        if finished {
            break;
        }
    }
    assert!(captured);
    let calls = launches.0.lock().unwrap();
    let expected = if selected {
        vec![
            fixture.root.join("folder/small"),
            fixture.root.join("root.txt"),
        ]
    } else {
        vec![fixture.root.join("folder/small")]
    };
    assert_eq!(calls.len(), expected.len());
    for ((args, cwd), expected) in calls.iter().zip(expected) {
        assert_eq!(args, &[expected.into_os_string()]);
        assert_eq!(*cwd, fixture.root);
    }
    assert_eq!(
        h.selection.as_ref().unwrap().len(),
        if selected { 2 } else { 0 }
    );
}
#[test]
fn tree_user_command_left_unmarked_nested_folder_once_uses_native_focus() {
    exercise(PaneId::Left, false);
}
#[test]
fn tree_user_command_right_selected_nested_folder_and_root_file_launch_in_display_order() {
    exercise(PaneId::Right, true);
}
