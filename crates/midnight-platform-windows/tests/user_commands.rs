#![cfg(windows)]
#[path = "../src/test_fixtures.rs"]
mod test_fixtures;
use midnight_core::*;
use midnight_platform_windows::WindowsFileSystem;
use std::{
    ffi::{OsStr, OsString},
    fs, io,
    os::windows::ffi::OsStrExt,
    path::Path,
};
#[path = "../../../scripts/tests/user-command-probe/assertions.rs"]
mod probe;
fn bytes(value: &OsStr) -> Vec<u8> {
    value.encode_wide().flat_map(u16::to_le_bytes).collect()
}

#[test]
fn actual_program_receives_native_argv_and_cwd_without_shell_execution_or_truncation() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let executable = probe::helper(&workspace, "program-probe.exe");
    let temp = test_fixtures::build_temp("fixture-");
    let directory = temp.path().join("cwd 文档 & $x; '");
    fs::create_dir(&directory).unwrap();
    let output = temp.path().join("argv.bin");
    let expected = vec![
        OsString::from("a\"b"),
        "".into(),
        "spaces trailing\\".into(),
        "& echo injected > sentinel; $x %PATH% !x! 文档 e\u{301}".into(),
        "文".repeat(1000).into(),
    ];
    let mut arguments = vec![output.clone().into_os_string()];
    arguments.extend(expected.clone());
    let process = UserCommandProcess {
        working_directory: directory.clone(),
        invocation: UserCommandProcessInvocation::Program {
            executable,
            arguments,
        },
    };
    let mut child = WindowsFileSystem::new(None)
        .launch_user_command(&process, &Cancellation::default())
        .unwrap();
    probe::wait(child.as_mut());
    let report = probe::report(&output);
    assert!(report.environment.len() <= USER_COMMAND_ENVIRONMENT.len());
    assert_eq!(
        report.arguments,
        expected
            .iter()
            .map(|value| bytes(value))
            .collect::<Vec<_>>()
    );
    assert_eq!(report.cwd, bytes(directory.as_os_str()));
    assert!(!directory.join("sentinel").exists());
}

#[test]
fn run_directly_accepts_bat_and_cmd_in_the_chosen_working_directory() {
    let temp = test_fixtures::build_temp("batch-");
    for extension in ["BaT", "CmD"] {
        let script = temp.path().join(format!("build example.{extension}"));
        fs::write(
            &script,
            b"@echo off\r\n>received.txt echo %1\r\nexit /b 0\r\n",
        )
        .unwrap();
        let mut child = WindowsFileSystem::new(None)
            .launch_user_program(
                &script,
                &["one & two".into()],
                temp.path(),
                &Cancellation::default(),
            )
            .unwrap();
        probe::wait(child.as_mut());
        assert_eq!(
            fs::read_to_string(temp.path().join("received.txt"))
                .unwrap()
                .trim(),
            "\"one & two\""
        );
        fs::remove_file(temp.path().join("received.txt")).unwrap();
    }
}

#[test]
fn cancelled_missing_and_unsupported_programs_never_launch_a_fallback() {
    let temp = test_fixtures::build_temp("fixture-");
    // Each refusal has its own cause: a missing program is NotFound, an unsupported file is
    // refused as input before any spawn (no cmd.exe fallback), and a cancelled
    // request is Interrupted. An unrelated failure no longer satisfies the test.
    for (name, kind) in [
        ("missing.exe", io::ErrorKind::NotFound),
        ("missing.cmd", io::ErrorKind::NotFound),
        ("script.txt", io::ErrorKind::InvalidInput),
    ] {
        let executable = temp.path().join(name);
        if name == "script.txt" {
            fs::write(&executable, b"echo unwanted").unwrap();
        }
        let process = UserCommandProcess {
            working_directory: temp.path().to_owned(),
            invocation: UserCommandProcessInvocation::Program {
                executable,
                arguments: vec![],
            },
        };
        let error = WindowsFileSystem::new(None)
            .launch_user_command(&process, &Cancellation::default())
            .err()
            .expect("the program must not launch");
        assert_eq!(error.kind, kind, "{name}: {error:?}");
    }
    let cancel = Cancellation::default();
    cancel.cancel();
    let process = UserCommandProcess {
        working_directory: temp.path().to_owned(),
        invocation: UserCommandProcessInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: vec!["--list".into()],
        },
    };
    let error = WindowsFileSystem::new(None)
        .launch_user_command(&process, &cancel)
        .err()
        .expect("a cancelled request must not launch");
    assert_eq!(error.kind, io::ErrorKind::Interrupted, "{error:?}");
}

#[test]
fn malformed_executable_returns_a_native_error_and_restores_thread_error_mode() {
    use std::{
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    let temp = test_fixtures::build_temp("fixture-");
    let report = temp.path().join("bad-image-result");
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "bad_image_probe_fixture",
            "--ignored",
            "--nocapture",
        ])
        .env("ME_COMMAND_BAD_IMAGE_PROBE", &report)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("owned malformed-image probe timed out");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(fs::read(report).unwrap(), b"native-error-and-restored-mode");
}

#[test]
#[ignore = "Owned subprocess fixture; invoked only by the bounded malformed-image test."]
fn bad_image_probe_fixture() {
    use windows_sys::Win32::System::Diagnostics::Debug::GetThreadErrorMode;
    let Some(report) = std::env::var_os("ME_COMMAND_BAD_IMAGE_PROBE") else {
        return;
    };
    let report = std::path::PathBuf::from(report);
    let executable = report.with_extension("exe");
    fs::write(&executable, b"echo unwanted").unwrap();
    // SAFETY: read-only query of the current owned fixture thread's native error mode.
    let before = unsafe { GetThreadErrorMode() };
    let process = UserCommandProcess {
        working_directory: report.parent().unwrap().to_owned(),
        invocation: UserCommandProcessInvocation::Program {
            executable,
            arguments: vec![],
        },
    };
    assert!(
        WindowsFileSystem::new(None)
            .launch_user_command(&process, &Cancellation::default())
            .is_err()
    );
    // SAFETY: the same read-only query after the synchronous native launch returned.
    assert_eq!(unsafe { GetThreadErrorMode() }, before);
    fs::write(report, b"native-error-and-restored-mode").unwrap();
}
