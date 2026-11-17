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

/// The software child's status becomes this process's status. A status that does not fit
/// a portable exit byte (a Windows NTSTATUS, a POSIX signal) becomes a generic failure.
pub(crate) fn child_status(code: Option<i32>) -> u8 {
    code.and_then(|code| u8::try_from(code).ok()).unwrap_or(1)
}

/// Run the software-rendering child to completion and return the status to forward.
/// Threads of the failed attempt that outlived its bounded runtime shutdown may still start
/// workers, and the child inherits every inheritable handle, so it is spawned under the
/// worker process-creation lock. The lock covers only the spawn, never the child's session.
/// Tests show only that the lock is free while the child runs. This crate forbids unsafe
/// code, so no test here can observe the spawn waiting for the lock; that it does rests on
/// the `spawn_command` call below alone.
pub(crate) fn run_child(command: &mut Command) -> Result<u8, String> {
    let mut child =
        midnight_worker_platform::spawn_command(command).map_err(|error| error.to_string())?;
    let status = child.wait().map_err(|error| error.to_string())?;
    Ok(child_status(status.code()))
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
    fn software_child_status_is_forwarded_when_it_fits_an_exit_byte() {
        assert_eq!(child_status(Some(0)), 0);
        assert_eq!(
            child_status(Some(101)),
            101,
            "a panicking child keeps its status"
        );
        assert_eq!(child_status(Some(255)), 255);
        // NTSTATUS crash codes, negative values and signal termination are failures.
        assert_eq!(child_status(Some(0xC000_0005_u32 as i32)), 1);
        assert_eq!(child_status(Some(256)), 1);
        assert_eq!(child_status(None), 1);
    }

    const CHILD_ENV: &str = "MIDNIGHT_SOFTWARE_CHILD_FIXTURE";
    const CHILD_READY: &str = "midnight-software-child-ready";
    const CHILD_STATUS: u8 = 7;

    #[test]
    fn software_child_runs_without_the_creation_lock_and_its_status_is_forwarded() {
        use std::{
            io::{BufRead, BufReader},
            process::Stdio,
            sync::mpsc,
            time::Duration,
        };
        const DEADLINE: Duration = Duration::from_secs(30);
        let (input, release) = std::io::pipe().unwrap();
        let (output, written) = std::io::pipe().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args([
                "--exact",
                "startup_recovery::tests::software_child_fixture",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ENV, "1")
            .stdin(input)
            .stdout(written)
            .stderr(Stdio::null());
        let (finished, result) = mpsc::channel();
        let runner = std::thread::spawn(move || {
            let _ = finished.send(run_child(&mut command));
        });
        let (ready, started) = mpsc::channel();
        std::thread::spawn(move || {
            // Ends at end of output, when the runner has dropped the command after the child.
            for line in BufReader::new(output).lines().map_while(Result::ok) {
                if line.contains(CHILD_READY) {
                    let _ = ready.send(());
                }
            }
        });
        started
            .recv_timeout(DEADLINE)
            .expect("the software child did not start");
        // run_child now waits for the running child; worker creations must not wait with it.
        // This does not show that the spawn itself waited for the lock (see run_child).
        #[cfg(windows)]
        {
            let (acquired, lock) = mpsc::channel();
            std::thread::spawn(move || {
                drop(midnight_worker_platform::lock_process_creation());
                let _ = acquired.send(());
            });
            lock.recv_timeout(DEADLINE)
                .expect("the creation lock was held while the software child ran");
        }
        drop(release);
        let status = result
            .recv_timeout(DEADLINE)
            .expect("the software child did not exit");
        assert_eq!(status, Ok(CHILD_STATUS));
        runner.join().unwrap();
    }

    #[test]
    #[ignore = "Owned subprocess fixture; started only by the software child test."]
    fn software_child_fixture() {
        use std::io::{Read, Write};
        if std::env::var_os(CHILD_ENV).is_none() {
            return;
        }
        println!("{CHILD_READY}");
        std::io::stdout().flush().unwrap();
        // Runs until the parent test closes this input, then exits with a distinctive status.
        std::io::stdin().read_to_end(&mut Vec::new()).unwrap();
        std::process::exit(CHILD_STATUS.into());
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
