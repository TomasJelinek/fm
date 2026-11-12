//! Actual native process argv/cwd and Engine-owned handle retirement for tree commands.
#[path = "support/tree_move_native.rs"]
mod native;
#[path = "support/tree_filters.rs"]
mod support;
#[path = "support/ui.rs"]
mod ui;
// Engine owns polling here; reuse the compiler/report helpers, not their direct-child waiter.
#[cfg(windows)]
#[path = "support/tree_command_process_windows.rs"]
mod native_cwd;
#[cfg(unix)]
#[path = "support/tree_command_process_unix.rs"]
mod native_cwd;
#[allow(dead_code)]
#[path = "../../../scripts/tests/user-command-probe/assertions.rs"]
mod probe;
#[cfg(windows)]
#[path = "../../../scripts/tests/user-command-probe/windows.rs"]
mod raw;
#[cfg(unix)]
#[path = "../../../scripts/tests/user-command-probe/unix.rs"]
mod raw;
use midnight_core::*;
use slint::Model;
use std::{
    ffi::OsString,
    fs,
    path::Path,
    process::ExitStatus,
    sync::{Arc, Mutex},
    thread::ThreadId,
};

#[derive(Debug)]
struct Receipt {
    pid: u32,
    success: Option<bool>,
    dropped: Option<ThreadId>,
}
struct ObservedChild {
    native: Option<Box<dyn TerminalProcess>>,
    receipts: Arc<Mutex<Vec<Receipt>>>,
    index: usize,
}
impl TerminalProcess for ObservedChild {
    fn id(&self) -> u32 {
        self.native.as_ref().unwrap().id()
    }
    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let status = self.native.as_mut().unwrap().try_wait()?;
        if let Some(status) = status {
            self.receipts.lock().unwrap()[self.index].success = Some(status.success());
        }
        Ok(status)
    }
}
impl Drop for ObservedChild {
    fn drop(&mut self) {
        // Observe final release only after the actual platform child/handle has been destroyed.
        drop(self.native.take());
        self.receipts.lock().unwrap()[self.index].dropped = Some(std::thread::current().id());
    }
}
struct ObserveNative {
    native: Arc<midnight_platform::LocalFileSystem>,
    receipts: Arc<Mutex<Vec<Receipt>>>,
}
impl TerminalService for ObserveNative {
    fn shell_flavor(&self) -> ShellFlavor {
        self.native.shell_flavor()
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("Program mode must not launch a terminal")
    }
}
impl UserCommandService for ObserveNative {
    fn validate_user_command(
        &self,
        process: &UserCommandProcess,
        cancel: &Cancellation,
    ) -> Result<(), FsError> {
        self.native.validate_user_command(process, cancel)
    }
    fn launch_user_program(
        &self,
        executable: &Path,
        args: &[OsString],
        cwd: &Path,
        cancel: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        let child = self
            .native
            .launch_user_program(executable, args, cwd, cancel)?;
        let mut receipts = self.receipts.lock().unwrap();
        let index = receipts.len();
        receipts.push(Receipt {
            pid: child.id(),
            success: None,
            dropped: None,
        });
        Ok(Box::new(ObservedChild {
            native: Some(child),
            receipts: self.receipts.clone(),
            index,
        }))
    }
}

fn exercise(pane: PaneId, selected: bool) {
    let caller = std::thread::current().id();
    let fixture = native::Fixture::new();
    let folder = "vnořená 文档 🚀 e\u{301}";
    let file = "soubor 文档 🐈 & $x.txt";
    let cwd_name = "pracovní 文档 e\u{301}";
    fs::create_dir(fixture.root.join("folder").join(folder)).unwrap();
    fs::write(fixture.root.join(file), b"native command input").unwrap();
    let cwd = fixture.root.join(cwd_name);
    fs::create_dir(&cwd).unwrap();
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let executable = probe::helper(
        &workspace,
        if selected {
            "tree-per-item-probe.exe"
        } else {
            "tree-once-probe.exe"
        },
    );
    let service = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let receipts = Arc::new(Mutex::new(Vec::new()));
    let observed = Arc::new(ObserveNative {
        native: service.clone(),
        receipts: receipts.clone(),
    });
    let separator = std::path::MAIN_SEPARATOR;
    let command = UserCommand {
        id: UserCommandId::new("native-tree-probe").unwrap(),
        name: "Owned native tree probe".into(),
        enabled: true,
        toolbar_label: None,
        icon: None,
        working_directory: format!("${{active.path}}{separator}{cwd_name}"),
        launch: if selected {
            UserCommandLaunch::PerItem
        } else {
            UserCommandLaunch::Single
        },
        invocation: UserCommandInvocation::Program {
            executable,
            arguments: vec![
                format!("${{active.path}}{separator}destination{separator}${{file.name}}.report"),
                "${file.path}".into(),
                "${file.name}".into(),
                "literal & $$(not-a-shell) 文档".into(),
            ],
        },
    };
    let mut h = support::Harness::with_native_user_commands(
        pane,
        fixture.root.clone(),
        service,
        observed,
        vec![command],
    );
    h.ui.window.invoke_activate(pane.index() as i32);
    for name in if selected {
        vec![folder, file]
    } else {
        vec![folder]
    } {
        let row = h.rows().iter().position(|row| row.name == name).unwrap();
        h.ui.window
            .invoke_row_click(pane.index() as i32, row as i32, selected, false);
    }
    let expected = if selected {
        vec![
            fixture.root.join("folder").join(folder),
            fixture.root.join(file),
        ]
    } else {
        vec![fixture.root.join("folder").join(folder)]
    };
    if selected {
        h.until(|h| {
            h.selection
                .as_ref()
                .is_some_and(|s| s.len() == expected.len())
        });
    }
    let command = h.ui.window.get_user_commands().row_data(0).unwrap();
    h.ui.window
        .invoke_execute_user_command(command.identity, command.revision);
    let mut captured = false;
    let mut job = None;
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
            assert_eq!(event.command.as_str(), "native-tree-probe");
            assert_eq!(*job.get_or_insert(event.request), event.request);
            match &event.update {
                UserCommandUpdate::Rejected(error) => panic!("command rejected: {error}"),
                UserCommandUpdate::Finished(summary) => {
                    assert!(
                        summary.error.is_none(),
                        "native command failed: {summary:?}"
                    );
                    assert_eq!(summary.planned, expected.len() as u64);
                    assert_eq!(summary.started, expected.len() as u64);
                    assert_eq!(summary.not_started, 0);
                    assert!(!summary.cancelled);
                    assert!(summary.error.is_none(), "{summary:?}");
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
    assert!(captured && job.is_some());
    h.until(|_| {
        let receipts = receipts.lock().unwrap();
        receipts.len() == expected.len() && receipts.iter().all(|r| r.dropped.is_some())
    });
    for receipt in receipts.lock().unwrap().iter() {
        assert_ne!(receipt.pid, 0);
        assert_eq!(receipt.success, Some(true), "{receipt:?}");
        assert_ne!(receipt.dropped, Some(caller), "native child retired on UI");
    }
    for path in expected {
        let name = path.file_name().unwrap();
        let mut report_name = name.to_os_string();
        report_name.push(".report");
        let report = probe::report(&fixture.root.join("destination").join(report_name));
        native_cwd::assert_cwd(&report.cwd, &cwd);
        assert_eq!(
            report.arguments,
            vec![
                raw::bytes(path.as_os_str()),
                raw::bytes(name),
                raw::bytes(std::ffi::OsStr::new("literal & $(not-a-shell) 文档"))
            ]
        );
        assert!(report.environment.len() <= USER_COMMAND_ENVIRONMENT.len());
    }
    assert_eq!(
        h.selection.as_ref().unwrap().len(),
        if selected { 2 } else { 0 }
    );
}
#[test]
fn tree_user_command_actual_native_once_preserves_nested_unicode_path_and_retires_child() {
    exercise(PaneId::Left, false);
}
#[test]
fn tree_user_command_actual_native_per_item_preserves_unicode_selection_and_retires_children() {
    exercise(PaneId::Right, true);
}
