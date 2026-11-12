use super::*;
use std::{ffi::OsString, fs};

#[test]
fn default_is_notepad_in_os_system_directory() {
    let service = WindowsFileSystem::new(None);
    let editor = service.default_editor().unwrap();
    assert_eq!(
        editor.executable,
        crate::desktop::system_directory()
            .unwrap()
            .join("notepad.exe")
    );
    assert_eq!(editor.arguments, ["{file}"]);
}

#[test]
fn preview_preserves_unicode_spaces_and_metacharacters_without_a_shell() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("文档 e\u{301} & $HOME; ' test.txt");
    fs::write(&file, b"text").unwrap();
    let executable = std::env::current_exe().unwrap();
    let preferences = EditorPreferences {
        rules: Vec::new(),
        executable: Some(executable.clone()),
        arguments: vec![
            "--literal={file}".into(),
            "{file}".into(),
            "quote\" & %PATH%".into(),
        ],
    };
    let command = prepare(&WindowsFileSystem::new(None), &file, &preferences).unwrap();
    assert_eq!(command.get_program(), executable.as_os_str());
    assert_eq!(
        command.get_args().map(OsString::from).collect::<Vec<_>>(),
        vec![
            OsString::from("--literal={file}"),
            file.into_os_string(),
            OsString::from("quote\" & %PATH%")
        ]
    );
}

#[test]
fn missing_program_and_batch_files_fail_without_default_fallback() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("file.txt");
    fs::write(&file, b"text").unwrap();
    for name in ["missing.exe", "script.cmd", "script.bat"] {
        let executable = directory.path().join(name);
        if name != "missing.exe" {
            fs::write(&executable, b"echo wrong").unwrap();
        }
        let preferences = EditorPreferences {
            rules: Vec::new(),
            executable: Some(executable),
            ..Default::default()
        };
        assert!(
            prepare(&WindowsFileSystem::new(None), &file, &preferences).is_err(),
            "accepted {name}"
        );
    }
}

#[test]
fn directories_missing_files_and_nonlocal_paths_are_rejected_before_launch() {
    let directory = tempfile::tempdir().unwrap();
    let preferences = EditorPreferences {
        rules: Vec::new(),
        executable: Some(std::env::current_exe().unwrap()),
        ..Default::default()
    };
    for file in [
        directory.path().to_path_buf(),
        directory.path().join("missing.txt"),
        "archive://test/file".into(),
    ] {
        assert!(
            prepare(&WindowsFileSystem::new(None), &file, &preferences).is_err(),
            "accepted {file:?}"
        );
    }
}

#[test]
fn safe_harness_child_is_started_without_opening_a_desktop_editor() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("文档 & $HOME .txt");
    fs::write(&file, b"test").unwrap();
    let preferences = EditorPreferences {
        rules: Vec::new(),
        executable: Some(std::env::current_exe().unwrap()),
        arguments: vec!["--list".into(), "{file}".into()],
    };
    let mut child = WindowsFileSystem::new(None)
        .launch_editor(&file, &preferences)
        .unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "safe child did not exit"
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn selected_editor_rule_controls_program_argv_and_working_directory() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("文档 & $HOME.txt");
    fs::write(&file, b"text").unwrap();
    let executable = std::env::current_exe().unwrap();
    let preferences = EditorPreferences {
        executable: Some(directory.path().join("fallback-must-not-run.exe")),
        rules: vec![midnight_core::EditorRule {
            expression: "*.txt".into(),
            executable: Some(executable.clone()),
            arguments: vec!["literal two words".into(), "{file}".into(), "".into()],
            working_directory: Some(directory.path().to_owned()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let command = prepare(&WindowsFileSystem::new(None), &file, &preferences).unwrap();
    assert_eq!(command.get_program(), executable.as_os_str());
    assert_eq!(command.get_current_dir(), Some(directory.path()));
    assert_eq!(
        command.get_args().map(OsString::from).collect::<Vec<_>>(),
        vec![
            OsString::from("literal two words"),
            file.into_os_string(),
            OsString::new()
        ]
    );
}

#[test]
fn unpaired_utf16_filename_uses_fallback_without_lossy_matching() {
    use std::os::windows::ffi::OsStringExt;
    let file = std::env::current_dir().unwrap().join(OsString::from_wide(&[
        0xd800,
        b'.' as u16,
        b't' as u16,
        b'x' as u16,
        b't' as u16,
    ]));
    let executable = std::env::current_exe().unwrap();
    let preferences = EditorPreferences {
        executable: Some(executable.clone()),
        rules: vec![midnight_core::EditorRule {
            expression: "*.txt".into(),
            executable: Some(executable.with_file_name("must-not-match.exe")),
            ..Default::default()
        }],
        ..Default::default()
    };
    let selected = midnight_platform_common::editor_rules::select(&preferences, &file).unwrap();
    let command = selected.command.unwrap();
    assert_eq!(command.executable, executable);
    assert_eq!(
        command.arguments_for(&file).unwrap(),
        vec![file.into_os_string()]
    );
}
