#![cfg(target_os = "linux")]
use midnight_core::*;
use midnight_platform_linux::LinuxFileSystem;
use std::{
    ffi::OsString,
    fs,
    os::unix::ffi::{OsStrExt, OsStringExt},
    path::Path,
};
#[path = "../../../scripts/tests/user-command-probe/assertions.rs"]
mod probe;

#[test]
fn actual_program_preserves_raw_native_argv_and_cwd_without_a_shell() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let executable = probe::helper(&workspace, "program-probe");
    let temp = tempfile::tempdir().unwrap();
    let directory = temp.path().join("cwd 文档 & $x; '");
    fs::create_dir(&directory).unwrap();
    let output = temp.path().join("argv.bin");
    let expected = vec![
        OsString::from_vec(b"raw-\xff\xfe".to_vec()),
        "".into(),
        "a\"b".into(),
        "& touch sentinel; $x $(touch sentinel) ' 文档".into(),
        "文".repeat(20_000).into(),
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
    let mut child = LinuxFileSystem::new(None)
        .launch_user_command(&process, &Cancellation::default())
        .unwrap();
    probe::wait(child.as_mut());
    let report = probe::report(&output);
    assert!(report.environment.len() <= USER_COMMAND_ENVIRONMENT.len());
    assert_eq!(
        report.arguments,
        expected
            .iter()
            .map(|value| value.as_bytes().to_vec())
            .collect::<Vec<_>>()
    );
    assert_eq!(report.cwd, directory.as_os_str().as_bytes());
    assert!(!directory.join("sentinel").exists());
}
