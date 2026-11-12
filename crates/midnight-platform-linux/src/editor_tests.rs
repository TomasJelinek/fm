use super::*;
use std::{ffi::OsString, os::unix::ffi::OsStringExt};

#[test]
fn argv_preserves_raw_non_unicode_filename_and_literal_metacharacters() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join(OsString::from_vec(
        b"\xff spaces & $HOME; ' \".txt".to_vec(),
    ));
    fs::write(&file, b"text").unwrap();
    let executable = std::env::current_exe().unwrap();
    let preferences = EditorPreferences {
        rules: Vec::new(),
        executable: Some(executable.clone()),
        arguments: vec!["{file}".into(), "--label={file} $HOME; &".into(), "".into()],
    };
    let command = prepare(&LinuxFileSystem::new(None), &file, &preferences).unwrap();
    assert_eq!(command.get_program(), executable.as_os_str());
    assert_eq!(
        command.get_args().map(OsString::from).collect::<Vec<_>>(),
        vec![
            file.into_os_string(),
            "--label={file} $HOME; &".into(),
            "".into()
        ]
    );
}

#[test]
fn safe_default_discovery_ignores_relative_missing_and_non_executable_candidates() {
    let directory = tempfile::tempdir().unwrap();
    let non_executable = directory.path().join("gedit");
    fs::write(&non_executable, b"not executable").unwrap();
    fs::set_permissions(&non_executable, fs::Permissions::from_mode(0o600)).unwrap();
    let executable = std::env::current_exe().unwrap();
    let absent = directory.path().join("missing");
    let candidates = [
        Path::new("gedit"),
        absent.as_path(),
        non_executable.as_path(),
    ];
    assert!(discover(candidates.into_iter()).is_err());
    let spec = discover(candidates.into_iter().chain([executable.as_path()])).unwrap();
    assert_eq!(spec.executable, executable);
}

#[test]
fn invalid_target_and_explicit_missing_executable_never_fall_back() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("文档 e\u{301} .txt");
    fs::write(&file, b"text").unwrap();
    let missing = EditorPreferences {
        rules: Vec::new(),
        executable: Some(directory.path().join("missing")),
        ..Default::default()
    };
    assert!(prepare(&LinuxFileSystem::new(None), &file, &missing).is_err());
    let good = EditorPreferences {
        rules: Vec::new(),
        executable: Some(std::env::current_exe().unwrap()),
        ..Default::default()
    };
    for file in [
        directory.path().to_path_buf(),
        directory.path().join("missing"),
        "archive://file".into(),
    ] {
        assert!(prepare(&LinuxFileSystem::new(None), &file, &good).is_err());
    }
}

#[test]
fn safe_harness_child_launches_with_the_literal_native_file_argument() {
    let directory = tempfile::tempdir().unwrap();
    let file = directory.path().join("文档 & $HOME .txt");
    fs::write(&file, b"test").unwrap();
    let preferences = EditorPreferences {
        rules: Vec::new(),
        executable: Some(std::env::current_exe().unwrap()),
        arguments: vec!["--list".into(), "{file}".into()],
    };
    let mut child = LinuxFileSystem::new(None)
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
    let command = prepare(&LinuxFileSystem::new(None), &file, &preferences).unwrap();
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
fn non_unicode_filename_uses_fallback_without_lossy_matching() {
    let file = std::env::current_dir()
        .unwrap()
        .join(OsString::from_vec(b"\xff.txt".to_vec()));
    let executable = std::env::current_exe().unwrap();
    let preferences = EditorPreferences {
        executable: Some(executable.clone()),
        rules: vec![midnight_core::EditorRule {
            expression: "*.txt".into(),
            executable: Some(executable.with_file_name("must-not-match")),
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
