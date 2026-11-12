use midnight_core::*;
use std::{
    ffi::{OsStr, OsString},
    fs,
    os::windows::ffi::OsStrExt,
    path::Path,
};
#[path = "../../../scripts/tests/user-command-probe/assertions.rs"]
mod probe;
fn bytes(value: &OsStr) -> Vec<u8> {
    value.encode_wide().flat_map(u16::to_le_bytes).collect()
}

#[test]
fn custom_powershell_receives_native_environment_without_evaluating_filename_source() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let helper = probe::helper(&workspace, "powershell-env-probe.exe");
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("cwd 文档 & $x");
    fs::create_dir(&cwd).unwrap();
    let output = temp.path().join("environment.bin");
    let source = format!(
        "& '{}' '{}'; exit $LASTEXITCODE",
        helper.to_str().unwrap().replace('\'', "''"),
        output.to_str().unwrap().replace('\'', "''")
    );
    let hostile = OsString::from(format!(
        "file' ; New-Item '{}'; # $(throw 'evaluated') & \" %PATH% !x! 文档",
        cwd.join("sentinel").display()
    ));
    let environment = vec![
        ("ME_FILE_PATH".to_owned(), hostile.clone()),
        (
            "ME_FILE_NAME".to_owned(),
            OsString::from("literal ${other.path} ; $()"),
        ),
    ];
    let request = TerminalRequest {
        command: source,
        working_directory: cwd.clone(),
        shell: ShellFlavor::PowerShell,
        mode: TerminalMode::Standard,
    };
    let spec =
        crate::desktop::prepare_user_terminal(&request, &environment, &Cancellation::default())
            .unwrap();
    let mut child =
        crate::terminal_process::spawn_with_environment_for_test(&spec, &environment).unwrap();
    probe::wait(child.as_mut());
    let report = probe::report(&output);
    assert_eq!(report.cwd, bytes(cwd.as_os_str()));
    assert!(report.arguments.is_empty());
    assert_eq!(
        report
            .environment
            .iter()
            .find(|(key, _)| *key == bytes(OsStr::new("ME_FILE_PATH")))
            .unwrap()
            .1,
        bytes(&hostile)
    );
    assert!(!cwd.join("sentinel").exists());
}

#[test]
fn custom_cmd_refuses_caller_supplied_companions_before_any_spawn() {
    let process = UserCommandProcess {
        working_directory: std::env::current_dir().unwrap(),
        invocation: UserCommandProcessInvocation::Terminal {
            shell: ShellFlavor::Cmd,
            script: "echo %ME_FILE_PATH%".into(),
            environment: vec![("ME_FILE_PATH_ARG".into(), "& echo unwanted".into())],
        },
    };
    let result = crate::WindowsFileSystem::new(None)
        .validate_user_command(&process, &Cancellation::default());
    assert_eq!(result.unwrap_err().kind, std::io::ErrorKind::InvalidInput);
}

#[path = "user_command_cmd_tests.rs"]
mod cmd;
