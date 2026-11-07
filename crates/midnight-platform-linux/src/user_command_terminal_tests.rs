use super::*;
use std::{
    ffi::OsString,
    os::unix::ffi::{OsStrExt, OsStringExt},
};
#[path = "../../../scripts/tests/user-command-probe/assertions.rs"]
mod probe;

#[test]
fn custom_posix_terminal_transports_raw_environment_without_evaluating_filename_source() {
    let workspace = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let helper = probe::helper(&workspace, "posix-terminal-probe");
    let temp = tempfile::tempdir().unwrap();
    let cwd = temp.path().join("cwd 文档 & $x");
    fs::create_dir(&cwd).unwrap();
    let output = temp.path().join("environment.bin");
    let source = format!(
        "'{}' '{}'; exit",
        helper.to_str().unwrap().replace('\'', "'\\''"),
        output.to_str().unwrap().replace('\'', "'\\''")
    );
    let mut raw = format!(
        "file'; touch '{}'; # $(touch '{}') ",
        cwd.join("sentinel").display(),
        cwd.join("sentinel").display()
    )
    .into_bytes();
    raw.extend_from_slice(&[0xff, 0xfe]);
    let value = OsString::from_vec(raw);
    let environment = vec![("ME_FILE_PATH".to_owned(), value.clone())];
    let request = TerminalRequest {
        command: source,
        working_directory: cwd.clone(),
        shell: ShellFlavor::Posix,
        mode: TerminalMode::Standard,
    };
    let mut command = prepare_user_terminal_with(
        &helper,
        &request,
        &environment,
        &midnight_core::Cancellation::default(),
    )
    .unwrap();
    let mut child = command.spawn().unwrap();
    probe::wait(&mut child);
    let report = probe::report(&output);
    assert_eq!(report.cwd, cwd.as_os_str().as_bytes());
    assert!(report.arguments.is_empty());
    assert_eq!(
        report
            .environment
            .iter()
            .find(|(key, _)| key == b"ME_FILE_PATH")
            .unwrap()
            .1,
        value.as_bytes()
    );
    assert!(!cwd.join("sentinel").exists());
}
