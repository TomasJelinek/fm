//! A failed renderer startup may restart once after bounded application cleanup.
use std::{ffi::OsString, path::Path, process::Command};

#[cfg(windows)]
#[path = "startup_recovery_windows.rs"]
mod native;
#[cfg(target_os = "linux")]
#[path = "startup_recovery_linux.rs"]
mod native;

pub(crate) enum Attempt {
    Finished,
    NativeStartupFailed(String),
}

pub(crate) fn retry_allowed(software: bool, explicit_renderer: bool, revealed: bool) -> bool {
    !software && !explicit_renderer && !revealed
}

pub(crate) fn native_failure(error: String, retry: bool) -> Result<Attempt, String> {
    if retry {
        Ok(Attempt::NativeStartupFailed(error))
    } else {
        Err(error)
    }
}

pub(crate) fn software_command(executable: &Path, arguments: &[OsString], cwd: &Path) -> Command {
    let mut command = Command::new(executable);
    command.args(arguments).arg("--software").current_dir(cwd);
    native::configure(&mut command);
    command
}

pub(crate) fn finish(
    result: Result<Attempt, String>,
    cleanup: impl FnOnce(),
    retry: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    // This callback consumes the runtime after run_window has drained the engine.
    // Nothing from the unsuccessful attempt can submit commands to the child.
    cleanup();
    match result? {
        Attempt::Finished => Ok(()),
        Attempt::NativeStartupFailed(error) => {
            eprintln!("Midnight Explorer: {error}; retrying with software rendering.");
            retry().map_err(|retry_error| format!("{error}\nSoftware retry failed: {retry_error}"))
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, path::PathBuf};

    #[test]
    fn startup_recovery_covers_selector_preflight_failure_without_overriding_explicit_choice() {
        let message = "WGPU rendering was required but no GPU adapter is available".to_owned();
        let result = native_failure(message.clone(), retry_allowed(false, false, false));
        assert!(matches!(result, Ok(Attempt::NativeStartupFailed(error)) if error == message));
        assert!(matches!(native_failure(message.clone(), false), Err(error) if error == message));
    }

    #[test]
    fn startup_recovery_retries_only_the_default_renderer_before_first_reveal() {
        assert!(retry_allowed(false, false, false));
        for (software, explicit, revealed) in [
            (true, false, false),
            (false, true, false),
            (false, false, true),
            (true, true, false),
        ] {
            assert!(!retry_allowed(software, explicit, revealed));
        }
    }

    #[test]
    fn startup_recovery_preserves_arguments_and_working_directory_without_a_shell() {
        let executable = PathBuf::from("private runtime/me");
        let cwd = PathBuf::from("original working directory");
        let arguments: Vec<OsString> = [
            "--left",
            "folder with spaces/文档",
            "--right",
            "literal $() ` & path",
            "--ui-smoke-test",
            "--snapshot",
            "qa/image.png",
        ]
        .into_iter()
        .map(Into::into)
        .collect();
        let command = software_command(&executable, &arguments, &cwd);
        assert_eq!(command.get_program(), executable.as_os_str());
        assert_eq!(command.get_current_dir(), Some(cwd.as_path()));
        let mut expected = arguments;
        expected.push("--software".into());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            expected.iter().map(OsString::as_os_str).collect::<Vec<_>>()
        );
    }

    #[test]
    fn startup_recovery_shuts_down_before_starting_the_software_child() {
        let events = RefCell::new(Vec::new());
        finish(
            Ok(Attempt::NativeStartupFailed("device unavailable".into())),
            || events.borrow_mut().push("workers and runtime stopped"),
            || {
                assert_eq!(*events.borrow(), ["workers and runtime stopped"]);
                events.borrow_mut().push("software child");
                Ok(())
            },
        )
        .unwrap();
        assert_eq!(
            *events.borrow(),
            ["workers and runtime stopped", "software child"]
        );
    }

    #[test]
    fn startup_recovery_does_not_replay_normal_exit_or_runtime_errors() {
        for result in [Ok(Attempt::Finished), Err("file operation failed".into())] {
            let expected_error = result.as_ref().err().cloned();
            let cleaned = std::cell::Cell::new(false);
            let actual = finish(result, || cleaned.set(true), || panic!("must not restart"));
            assert!(cleaned.get());
            assert_eq!(actual.err(), expected_error);
        }
    }

    #[test]
    fn startup_recovery_reports_child_failure_instead_of_looping() {
        let attempts = std::cell::Cell::new(0);
        let result = finish(
            Ok(Attempt::NativeStartupFailed("device unavailable".into())),
            || {},
            || {
                attempts.set(attempts.get() + 1);
                Err("software also unavailable".into())
            },
        );
        assert_eq!(attempts.get(), 1);
        let error = result.unwrap_err();
        assert!(error.contains("device unavailable"));
        assert!(error.contains("software also unavailable"));
    }
}
