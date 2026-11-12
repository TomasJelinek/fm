//! Actual system Cmd, not a parser mock. All child output belongs to the fixture.
use super::*;
use std::os::windows::ffi::OsStringExt;

fn request(cwd: &Path, script: String) -> TerminalRequest {
    TerminalRequest {
        command: script,
        working_directory: cwd.into(),
        shell: ShellFlavor::Cmd,
        mode: TerminalMode::Standard,
    }
}

#[test]
fn custom_cmd_companions_round_trip_native_argv_and_raw_environment() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let helper = probe::helper(&workspace, "cmd-env-probe.exe");
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("cwd 文档 & %PATH% !bang! ^ (round)");
    fs::create_dir(&cwd).unwrap();
    let mut raw = OsString::from("raw 文档 %PATH% !ME_OTHER_PATH! & ^ ");
    raw.push(OsString::from_wide(&[0xd800, b'x' as u16, 0xdcff]));
    let cases = [
        vec![
            "文档 %PATH% !ME_OTHER_PATH! & ^ (round)".into(),
            "name!".into(),
            "".into(),
        ],
        vec!["C:\\".into(), "C:\\folder with spaces\\".into(), "".into()],
        vec![
            "name\" & echo bad>sentinel & \"tail".into(),
            "quote\"tail".into(),
            "".into(),
        ],
        vec![raw, "literal ${other.path}".into(), "".into()],
        vec![
            "line1\r\nline2\t & echo bad>sentinel".into(),
            "tab\there".into(),
            "".into(),
        ],
    ];
    for (index, values) in cases.into_iter().enumerate() {
        let output = temp.path().join(format!("report-{index}.bin"));
        let environment = ["ME_FILE_PATH", "ME_FILE_NAME", "ME_FILE_EXTENSION"]
            .into_iter()
            .zip(values.iter().cloned())
            .map(|(key, value)| (key.to_owned(), value))
            .collect::<Vec<_>>();
        let script = format!(
            "\"{}\" \"{}\" !ME_FILE_PATH_ARG! !ME_FILE_NAME_ARG! !ME_FILE_EXTENSION_ARG! & exit /b 0",
            helper.display(),
            output.display()
        );
        let request = request(&cwd, script.clone());
        let spec =
            crate::desktop::prepare_user_terminal(&request, &environment, &Cancellation::default())
                .unwrap();
        assert_eq!(
            request.command, script,
            "preparation must not rewrite source"
        );
        let mut child =
            crate::terminal_process::spawn_with_environment_for_test(&spec, &environment).unwrap();
        probe::wait(child.as_mut());
        let report = probe::report(&output);
        assert_eq!(report.cwd, bytes(cwd.as_os_str()));
        assert_eq!(
            report.arguments,
            values.iter().map(|v| bytes(v)).collect::<Vec<_>>()
        );
        for (key, value) in &environment {
            assert_eq!(
                report
                    .environment
                    .iter()
                    .find(|(k, _)| *k == bytes(OsStr::new(key)))
                    .unwrap()
                    .1,
                bytes(value)
            );
        }
        assert!(!cwd.join("sentinel").exists());
    }
}

#[test]
fn custom_cmd_builtins_use_raw_values_instead_of_native_argv_companions() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let helper = probe::helper(&workspace, "cmd-builtins-probe.exe");
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("start");
    let next = temp.path().join("目录 & %PATH% !bang! ^ (round)");
    fs::create_dir(&cwd).unwrap();
    fs::create_dir(&next).unwrap();
    let input = cwd.join("input 文档 & %PATH% !bang! ^ (round).txt");
    let contents = "owned bytes only\r\n";
    fs::write(&input, contents.as_bytes()).unwrap();
    let output = temp.path().join("builtins.bin");
    let text = "文档 %PATH% !ME_OTHER_PATH! & caret^ (round)";
    let mut directory = next.as_os_str().to_owned();
    directory.push("\\");
    let environment = vec![
        ("ME_FILE_PATH".into(), input.into_os_string()),
        ("ME_FILE_NAME".into(), text.into()),
        ("ME_ACTIVE_PATH".into(), directory),
    ];
    let script = format!(
        ">echo.txt echo !ME_FILE_NAME!& type \"!ME_FILE_PATH!\">type.txt & cd /d \"!ME_ACTIVE_PATH!\" & \"{}\" \"{}\" & exit /b 0",
        helper.display(),
        output.display(),
    );
    let spec = crate::desktop::prepare_user_terminal(
        &request(&cwd, script),
        &environment,
        &Cancellation::default(),
    )
    .unwrap();
    let mut child =
        crate::terminal_process::spawn_with_environment_for_test(&spec, &environment).unwrap();
    probe::wait(child.as_mut());
    assert_eq!(
        fs::read(cwd.join("echo.txt")).unwrap(),
        bytes(OsStr::new(&format!("{text}\r\n")))
    );
    assert_eq!(
        fs::read(cwd.join("type.txt")).unwrap(),
        bytes(OsStr::new(contents))
    );
    assert_eq!(probe::report(&output).cwd, bytes(next.as_os_str()));
}

#[test]
fn custom_cmd_refuses_native_size_limits_and_cancelled_launch_without_output() {
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path();
    for (script, value) in [
        ("echo first\necho ignored".to_owned(), "short".into()),
        ("echo first\r\necho ignored".to_owned(), "short".into()),
        ("echo !ME_FILE_PATH_ARG!".to_owned(), "x".repeat(8191)),
        ("x".repeat(8192), "short".into()),
        (
            "echo !me_file_path_arg! !ME_FILE_PATH_ARG! !me_file_path_arg!".into(),
            "x".repeat(3000),
        ),
    ] {
        let result = crate::desktop::prepare_user_terminal(
            &request(cwd, script),
            &[("ME_FILE_PATH".into(), value.into())],
            &Cancellation::default(),
        );
        assert_eq!(result.unwrap_err().kind, std::io::ErrorKind::InvalidInput);
    }
    let request = request(cwd, "echo must-not-run>sentinel & exit /b 0".into());
    let cancelled = Cancellation::default();
    cancelled.cancel();
    let service = crate::WindowsFileSystem::new(None);
    assert_eq!(
        service
            .launch_terminal_with_environment(&request, &[], &cancelled)
            .err()
            .unwrap()
            .kind,
        std::io::ErrorKind::Interrupted
    );
    let spec =
        crate::desktop::prepare_user_terminal(&request, &[], &Cancellation::default()).unwrap();
    // Cancellation after successful preparation is checked again at native launch.
    assert_eq!(
        crate::terminal_process::spawn_with_environment(&spec, &[], &cancelled)
            .err()
            .unwrap()
            .kind(),
        std::io::ErrorKind::Interrupted
    );
    assert!(!cwd.join("sentinel").exists());
}

fn verbatim(path: &Path) -> std::path::PathBuf {
    let mut value = OsString::from(r"\\?\");
    value.push(path);
    value.into()
}

#[test]
fn custom_cmd_verbatim_dos_cwd_keeps_the_captured_directory() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let helper = probe::helper(&workspace, "cmd-verbatim-cwd-probe.exe");
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("verbatim 文档 & %PATH% !bang!");
    fs::create_dir(&cwd).unwrap();
    let output = temp.path().join("verbatim.bin");
    let source = format!(
        "\"{}\" \"{}\" & exit /b 0",
        helper.display(),
        output.display()
    );
    let spec = crate::desktop::prepare_user_terminal(
        &request(&verbatim(&cwd), source),
        &[],
        &Cancellation::default(),
    )
    .unwrap();
    let mut child = crate::terminal_process::spawn_with_environment_for_test(&spec, &[]).unwrap();
    probe::wait(child.as_mut());
    assert_eq!(probe::report(&output).cwd, bytes(cwd.as_os_str()));
}

#[test]
fn custom_cmd_refuses_semantically_changed_dos_cwd_and_device_namespaces() {
    let temp = tempfile::tempdir().unwrap();
    fs::create_dir(temp.path().join("different")).unwrap();
    for name in ["different.", "different "] {
        let cwd = verbatim(&temp.path().join(name));
        fs::create_dir(&cwd).unwrap();
        let result = crate::desktop::prepare_user_terminal(
            &request(&cwd, "exit /b 0".into()),
            &[],
            &Cancellation::default(),
        );
        assert_eq!(result.unwrap_err().kind, std::io::ErrorKind::Unsupported);
    }
    for cwd in [r"\\server\share", r"\\?\UNC\server\share", r"\\.\C:\"] {
        let result = crate::desktop::prepare_user_terminal(
            &request(Path::new(cwd), "exit /b 0".into()),
            &[],
            &Cancellation::default(),
        );
        assert_eq!(result.unwrap_err().kind, std::io::ErrorKind::Unsupported);
    }
}

#[test]
fn custom_cmd_verbatim_root_and_junction_cwd_preserve_directory_authority() {
    use std::{os::windows::process::CommandExt, process::Command};
    use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let helper = probe::helper(&workspace, "cmd-root-alias-probe.exe");
    let temp = tempfile::tempdir().unwrap();
    let target = temp.path().join("target 文档");
    let alias = temp.path().join("alias %PATH% !bang!");
    fs::create_dir(&target).unwrap();
    let output = Command::new(crate::desktop::powershell_path().unwrap())
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../midnight-platform/tests/create-junction.ps1"),
        )
        .arg("-JunctionPath")
        .arg(&alias)
        .arg("-TargetPath")
        .arg(&target)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .unwrap();
    assert!(output.status.success(), "junction fixture: {output:?}");
    let root = temp.path().ancestors().last().unwrap();
    for (index, cwd) in [root, alias.as_path()].into_iter().enumerate() {
        let output = temp.path().join(format!("cwd-{index}.bin"));
        let original = verbatim(cwd);
        let environment = vec![("ME_ACTIVE_PATH".into(), original.as_os_str().to_owned())];
        let source = format!(
            "\"{}\" \"{}\" & exit /b 0",
            helper.display(),
            output.display()
        );
        let spec = crate::desktop::prepare_user_terminal(
            &request(&original, source),
            &environment,
            &Cancellation::default(),
        )
        .unwrap();
        let mut child =
            crate::terminal_process::spawn_with_environment_for_test(&spec, &environment).unwrap();
        probe::wait(child.as_mut());
        let report = probe::report(&output);
        assert_eq!(report.cwd, bytes(cwd.as_os_str()));
        assert_eq!(report.environment[0].1, bytes(original.as_os_str()));
    }
}

#[test]
fn custom_cmd_long_cwd_never_truncates_or_falls_back_after_native_refusal() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let helper = probe::helper(&workspace, "cmd-long-cwd-probe.exe");
    let temp = tempfile::tempdir().unwrap();
    let mut cwd = temp.path().to_owned();
    while cwd.as_os_str().encode_wide().count() < 330 {
        cwd.push("long directory 文档 with spaces");
    }
    fs::create_dir_all(&cwd).unwrap();
    for (index, input) in [cwd.clone(), verbatim(&cwd)].into_iter().enumerate() {
        let output = temp.path().join(format!("long-cwd-{index}.bin"));
        let source = format!(
            "\"{}\" \"{}\" & exit /b 0",
            helper.display(),
            output.display()
        );
        let spec = crate::desktop::prepare_user_terminal(
            &request(&input, source),
            &[],
            &Cancellation::default(),
        )
        .unwrap();
        match crate::terminal_process::spawn_with_environment_for_test(&spec, &[]) {
            Ok(mut child) => {
                probe::wait(child.as_mut());
                assert_eq!(probe::report(&output).cwd, bytes(cwd.as_os_str()));
            }
            Err(error) => {
                assert!(matches!(error.raw_os_error(), Some(267 | 206)), "{error}");
                assert!(
                    !output.exists(),
                    "native refusal must not run the helper elsewhere"
                );
            }
        }
    }
}
