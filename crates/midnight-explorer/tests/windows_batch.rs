#![cfg(windows)]

use midnight_core::{Cancellation, ConsoleLauncher, ProgramExit, UserCommandService};
use midnight_platform::{LocalFileSystem, set_console_launcher};
use std::{
    fs,
    io::Read,
    path::Path,
    time::{Duration, Instant},
};

#[test]
fn captured_batch_inherits_cwd_reports_both_streams_and_preserves_exit_code() {
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("batch capture {}", std::process::id()));
    fs::create_dir_all(&folder).unwrap();
    let services = LocalFileSystem::new(None);
    assert!(set_console_launcher(ConsoleLauncher {
        executable: env!("CARGO_BIN_EXE_midnight-explorer").into(),
        leading: vec![midnight_core::RUN_AND_WAIT_ARGUMENT.into()],
    }));
    for extension in ["bat", "CMD"] {
        let script = folder.join(format!("build example.{extension}"));
        fs::write(&script, b"@echo off\r\nif not exist \"%~nx0\" exit /b 3\r\necho %1\r\n1>&2 echo captured stderr\r\nexit /b 300\r\n").unwrap();
        let mut child = services
            .launch_user_program_captured(
                &script,
                &["one & two".into()],
                &folder,
                &Cancellation::default(),
            )
            .unwrap();
        let mut pipes = child.take_streams().unwrap();
        let deadline = Instant::now() + Duration::from_secs(15);
        let exit = loop {
            if let Some(code) = child.try_wait().unwrap() {
                break code;
            }
            if Instant::now() >= deadline {
                child.terminate_tree(true).unwrap();
                panic!("captured batch did not exit");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        let mut stdout = String::new();
        let mut stderr = String::new();
        pipes.stdout.read_to_string(&mut stdout).unwrap();
        pipes.stderr.read_to_string(&mut stderr).unwrap();
        assert_eq!(exit, ProgramExit::Code(300), "{stdout} {stderr}");
        assert_eq!(stdout.trim(), "\"one & two\"");
        assert_eq!(stderr.trim(), "captured stderr");
        fs::remove_file(script).unwrap();
    }
    fs::remove_dir(folder).unwrap();
}
