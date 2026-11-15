use super::*;
use std::{
    ffi::{OsStr, OsString},
    io::Write,
    os::unix::{ffi::OsStringExt, fs::symlink},
};

fn temporary() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../_build")
        .canonicalize()
        .unwrap();
    tempfile::tempdir_in(root).unwrap()
}

fn fixture(path: &Path, bytes: &[u8], mode: u32) {
    fs::write(path, bytes).unwrap();
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).unwrap();
}

#[test]
fn executable_shebang_and_shell_files_run_while_documents_keep_associations() {
    let temp = temporary();
    for (name, contents, mode, expected) in [
        (
            "script without suffix",
            b"#!/bin/sh\nexit 0\n".as_slice(),
            0o744,
            FileActivation::Terminal,
        ),
        (
            "legacy.sh",
            b"# ordinary comment\nexit 0\n".as_slice(),
            0o744,
            FileActivation::Terminal,
        ),
        (
            "read-only.sh",
            b"#!/bin/sh\nexit 0\n".as_slice(),
            0o644,
            FileActivation::Association,
        ),
        (
            "text with inherited execute bits.txt",
            b"ordinary document\n".as_slice(),
            0o777,
            FileActivation::Association,
        ),
        (
            "launcher.desktop",
            b"[Desktop Entry]\nType=Application\n".as_slice(),
            0o755,
            FileActivation::Association,
        ),
        (
            "shebang-launcher.desktop",
            b"#!/usr/bin/env xdg-open\n[Desktop Entry]\n".as_slice(),
            0o755,
            FileActivation::Association,
        ),
        (
            "native program",
            b"\x7fELF".as_slice(),
            0o755,
            FileActivation::Program,
        ),
    ] {
        let path = temp.path().join(name);
        fixture(&path, contents, mode);
        assert_eq!(
            file_activation(&path, &fs::metadata(&path).unwrap()).unwrap(),
            expected,
            "{name}",
        );
    }
}

#[test]
fn explicit_script_symlink_preserves_alias_path_and_changed_probe_identity_is_rejected() {
    let temp = temporary();
    let target = temp.path().join("target");
    fixture(&target, b"#!/bin/sh\nexit 0\n", 0o744);
    let alias = temp.path().join("alias ' ; $(touch NEVER)");
    symlink(&target, &alias).unwrap();
    assert_eq!(
        file_activation(&alias, &fs::metadata(&alias).unwrap()).unwrap(),
        FileActivation::Terminal,
    );
    let original = fs::metadata(&target).unwrap();
    fs::rename(&target, temp.path().join("old-target")).unwrap();
    fixture(&target, b"#!/bin/sh\nexit 0\n", 0o744);
    assert!(file_activation(&target, &original).is_err());
}

fn run_script_fixture(header: &[u8], shell: &Path, alias: bool) -> (Vec<u8>, Vec<u8>) {
    let temp = temporary();
    let cwd = temp.path().join(OsString::from_vec(
        b"cwd_\xff ' \" ; $(touch CWD_SENTINEL)".to_vec(),
    ));
    fs::create_dir(&cwd).unwrap();
    let path = cwd.join(OsString::from_vec(
        b"script_\xfe ' \" ; $(touch SCRIPT_SENTINEL).sh".to_vec(),
    ));
    let mut contents = header.to_vec();
    contents.extend_from_slice(
        b"printf '%s\\0' \"$0\" \"$#\" \"$PWD\" > \"$ME_ACTIVATION_REPORT\"\nprintf 'VISIBLE SCRIPT OUTPUT\\n' >&2\nfalse\nprintf 'not reached with shebang -e' > \"$ME_AFTER_FALSE\"\n",
    );
    fixture(&path, &contents, 0o744);
    let path = if alias {
        let link = cwd.join(OsString::from_vec(b"alias_\xfd ' \".sh".to_vec()));
        symlink(&path, &link).unwrap();
        link
    } else {
        path
    };
    let report = temp.path().join("executed.bin");
    let after_false = temp.path().join("after-false");
    let home = temp.path().join("home");
    fs::create_dir(&home).unwrap();
    fs::write(home.join(".bashrc"), "PS1='SELECTED BASH HANDOFF> '\n").unwrap();
    let terminal =
        crate::desktop::executable_terminal_with(Path::new("/usr/bin/xterm"), &path, shell)
            .unwrap();
    assert_eq!(terminal.get_current_dir(), Some(cwd.as_path()));
    let mut arguments = terminal.get_args();
    assert_eq!(arguments.next(), Some(OsStr::new("-e")));
    let mut child = Command::new(arguments.next().unwrap())
        .args(arguments)
        .current_dir(terminal.get_current_dir().unwrap())
        .env("HOME", home)
        .env("TERM", "dumb")
        .env("LC_ALL", "C.UTF-8")
        .env("COLUMNS", "4096")
        .env("ME_ACTIVATION_REPORT", &report)
        .env("ME_AFTER_FALSE", &after_false)
        .env_remove("PS1")
        .env_remove("ENV")
        .env_remove("BASH_ENV")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"printf 'INTERACTIVE HANDOFF\\n'\nexit\n")
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(
        fs::read(report).unwrap(),
        [
            path.as_os_str().as_bytes(),
            b"\0",
            b"0\0",
            cwd.as_os_str().as_bytes(),
            b"\0"
        ]
        .concat(),
    );
    assert_eq!(output.stdout, b"INTERACTIVE HANDOFF\n");
    assert!(
        output
            .stderr
            .windows(b"VISIBLE SCRIPT OUTPUT".len())
            .any(|part| part == b"VISIBLE SCRIPT OUTPUT")
    );
    assert_eq!(after_false.exists(), !header.starts_with(b"#!/bin/sh -e"));
    assert!(!cwd.join("CWD_SENTINEL").exists());
    assert!(!cwd.join("SCRIPT_SENTINEL").exists());
    (output.stdout, output.stderr)
}

#[test]
fn actual_shebang_execution_honors_interpreter_options_raw_path_and_symlink_alias() {
    run_script_fixture(b"#!/bin/sh -e\n", Path::new("/bin/sh"), false);
    run_script_fixture(b"#!/bin/sh -e\n", Path::new("/bin/sh"), true);
}

#[test]
fn legacy_executable_shell_file_runs_then_hands_off_to_selected_interactive_shell() {
    run_script_fixture(
        b"# Legacy shell file without a shebang.\n",
        Path::new("/bin/sh"),
        false,
    );
    if let Some(shell) = [Path::new("/bin/bash"), Path::new("/usr/bin/bash")]
        .into_iter()
        .find(|path| path.is_file())
    {
        let (_, stderr) = run_script_fixture(b"# Legacy shell file.\n", shell, false);
        assert!(
            stderr
                .windows(b"SELECTED BASH HANDOFF> ".len())
                .any(|part| part == b"SELECTED BASH HANDOFF> ")
        );
    }
}

#[test]
fn nonexecutable_shell_files_and_executable_marked_documents_are_not_run() {
    let temp = temporary();
    let marker = temp.path().join("MUST_NOT_EXIST");
    for (name, mode) in [("not-executable.sh", 0o644), ("document.txt", 0o777)] {
        let path = temp.path().join(name);
        fixture(&path, b"printf 'must not execute' > MUST_NOT_EXIST\n", mode);
        let result = open_with(&path, |path, metadata| {
            assert_eq!(
                file_activation(path, metadata)?,
                FileActivation::Association
            );
            Ok(None)
        })
        .unwrap();
        assert!(matches!(result, OpenEntryResult::File { process: None }));
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, mode);
        assert!(!marker.exists());
    }
}

#[test]
fn native_elf_program_executes_directly_with_parent_as_its_working_directory() {
    let temp = temporary();
    let cwd = temp
        .path()
        .join(OsString::from_vec(b"native_\xff ' \" ; $()".to_vec()));
    fs::create_dir(&cwd).unwrap();
    let system_pwd = [Path::new("/usr/bin/pwd"), Path::new("/bin/pwd")]
        .into_iter()
        .find(|path| path.is_file())
        .unwrap();
    let path = cwd.join("pwd");
    symlink(system_pwd, &path).unwrap();
    assert_eq!(
        file_activation(&path, &fs::metadata(&path).unwrap()).unwrap(),
        FileActivation::Program
    );
    let mut command = program_command(&path).unwrap();
    assert_eq!(command.get_program(), path.as_os_str());
    assert!(command.get_args().next().is_none());
    let output = command.stdout(Stdio::piped()).output().unwrap();
    assert!(output.status.success());
    assert_eq!(output.stdout, [cwd.as_os_str().as_bytes(), b"\n"].concat());
}

#[test]
fn execute_only_program_does_not_require_header_read_permission() {
    let temp = temporary();
    let path = temp.path().join("execute-only");
    fixture(&path, b"\x7fELF", 0o111);
    assert_eq!(
        file_activation(&path, &fs::metadata(&path).unwrap()).unwrap(),
        FileActivation::Program
    );
}

#[test]
fn replacement_fifo_during_header_probe_is_rejected_without_waiting_for_a_writer() {
    use rustix::fs::{CWD, Mode, OFlags, mkfifoat, open};
    use std::{sync::mpsc, thread, time::Duration};

    let temp = temporary();
    let path = temp.path().join("replaced.sh");
    fixture(&path, b"#!/bin/sh\n", 0o744);
    let metadata = fs::metadata(&path).unwrap();
    fs::remove_file(&path).unwrap();
    mkfifoat(CWD, &path, Mode::RUSR | Mode::WUSR).unwrap();
    let fifo = path.clone();
    let (sender, receiver) = mpsc::channel();
    let worker = thread::spawn(move || sender.send(file_activation(&path, &metadata)).unwrap());
    let immediate = receiver.recv_timeout(Duration::from_secs(2));
    if immediate.is_err() {
        let writer = open(
            &fifo,
            OFlags::WRONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .unwrap();
        drop(writer);
        assert!(
            receiver
                .recv_timeout(Duration::from_secs(2))
                .unwrap()
                .is_err()
        );
    }
    worker.join().unwrap();
    assert!(
        immediate.is_ok(),
        "executable inspection waited for a FIFO writer"
    );
    assert!(immediate.unwrap().is_err());
}
