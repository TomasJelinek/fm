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

#[cfg(test)]
mod image_spare_ordering_tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        sync::{Arc, Mutex, mpsc},
        time::Duration,
    };

    #[test]
    fn image_spare_acknowledgement_precedes_runtime_shutdown_and_software_retry() {
        let phases = Arc::new(Mutex::new(Vec::new()));
        let recorded = phases.clone();
        let (stopping, stopped) = mpsc::sync_channel(1);
        let (release, reaped) = mpsc::sync_channel(1);
        let (finished, completion) = mpsc::sync_channel(1);
        let task = std::thread::spawn(move || {
            let result = crate::viewers::finish_after_image_spare(
                Ok(Attempt::NativeStartupFailed("renderer unavailable".into())),
                || {
                    recorded.lock().unwrap().push("spare stop");
                    stopping.send(()).unwrap();
                    reaped.recv_timeout(Duration::from_secs(5)).unwrap();
                    recorded.lock().unwrap().push("spare reaped");
                    (41u8, Ok(()))
                },
                |owned_runtime| {
                    assert_eq!(owned_runtime, 41);
                    recorded.lock().unwrap().push("runtime shutdown");
                },
                || {
                    recorded.lock().unwrap().push("software retry");
                    Ok(())
                },
            );
            finished.send(result).unwrap();
        });
        stopped.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_eq!(*phases.lock().unwrap(), ["spare stop"]);
        assert!(completion.try_recv().is_err());
        release.send(()).unwrap();
        completion
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        task.join().unwrap();
        assert_eq!(
            *phases.lock().unwrap(),
            [
                "spare stop",
                "spare reaped",
                "runtime shutdown",
                "software retry"
            ]
        );
    }

    #[test]
    fn image_spare_retirement_runs_for_normal_exit_and_startup_error() {
        for result in [
            Ok(Attempt::Finished),
            Err("startup failed after registration".into()),
        ] {
            let expected_error = result.as_ref().err().cloned();
            let phases = RefCell::new(Vec::new());
            let result = crate::viewers::finish_after_image_spare(
                result,
                || {
                    phases.borrow_mut().push("spare reaped");
                    (41u8, Ok(()))
                },
                |owned_runtime| {
                    assert_eq!(owned_runtime, 41);
                    phases.borrow_mut().push("runtime shutdown");
                },
                || panic!("normal exit and startup errors must not retry"),
            );
            assert_eq!(result.err(), expected_error);
            assert_eq!(*phases.borrow(), ["spare reaped", "runtime shutdown"]);
        }
    }

    #[test]
    fn image_spare_failure_preserves_ownership_through_runtime_shutdown_and_blocks_retry() {
        struct HeldSpare<'a>(&'a Cell<bool>);
        impl Drop for HeldSpare<'_> {
            fn drop(&mut self) {
                self.0.set(true);
            }
        }

        for outcome in [
            Ok(Attempt::NativeStartupFailed("renderer unavailable".into())),
            Ok(Attempt::Finished),
            Err("startup failed after registration".into()),
        ] {
            let released = Cell::new(false);
            let held = HeldSpare(&released);
            let phases = RefCell::new(Vec::new());
            let result = crate::viewers::finish_after_image_spare(
                outcome,
                || {
                    let _retained = &held;
                    phases.borrow_mut().push("spare owner failed");
                    (41u8, Err("OwnerFailed; cleanup unconfirmed".into()))
                },
                |owned_runtime| {
                    assert_eq!(owned_runtime, 41);
                    assert!(!released.get());
                    phases.borrow_mut().push("runtime shutdown");
                },
                || panic!("failed retirement must suppress a renderer-qualified retry"),
            );
            let error = result.unwrap_err();
            assert!(error.contains("OwnerFailed"));
            assert!(error.contains("cleanup unconfirmed"));
            assert_eq!(*phases.borrow(), ["spare owner failed", "runtime shutdown"]);
            assert!(!released.get(), "ownership must survive the fatal result");
            drop(held);
            assert!(released.get());
        }
    }
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
