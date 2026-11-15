use super::*;
use std::{ffi::OsStr, io::Write, os::unix::fs::symlink, process::Stdio};

fn temporary() -> tempfile::TempDir {
    let directory = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../_build")
        .canonicalize()
        .unwrap();
    tempfile::tempdir_in(directory).unwrap()
}

fn launch_shell(shell: &Path, source: &str, home: &Path, cwd: &Path, input: &[u8]) -> String {
    let request = TerminalRequest {
        shell: ShellFlavor::Posix,
        mode: TerminalMode::Standard,
        command: source.into(),
        working_directory: cwd.into(),
    };
    let terminal =
        terminal_command_with_shell(Path::new("/usr/bin/xterm"), &request, shell).unwrap();
    let mut arguments = terminal.get_args();
    assert_eq!(arguments.next(), Some(OsStr::new("-e")));
    let mut child = Command::new(arguments.next().unwrap())
        .args(arguments)
        .current_dir(cwd)
        .env("HOME", home)
        .env("TERM", "dumb")
        .env("COLUMNS", "4096")
        .env("LC_ALL", "C.UTF-8")
        .env_remove("PS1")
        .env_remove("ENV")
        .env_remove("BASH_ENV")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(input).unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(output.status.success(), "{output:?}");
    String::from_utf8(output.stderr).unwrap()
}

#[test]
fn configured_shell_is_one_absolute_executable_path_not_shell_source() {
    let temporary = temporary();
    let selected = temporary
        .path()
        .join("configured shell ; $(touch sentinel)");
    symlink("/bin/sh", &selected).unwrap();
    assert_eq!(interactive_shell(Some(selected.as_os_str())), selected);
    for invalid in [
        OsStr::new("sh"),
        OsStr::new("/bin/sh -i"),
        OsStr::new("/bin/sh\0ignored"),
        temporary.path().as_os_str(),
    ] {
        let fallback = interactive_shell(Some(invalid));
        assert!(fallback.is_absolute() && is_executable(&fallback));
        assert_ne!(fallback.as_os_str(), invalid);
    }
    let non_executable = temporary.path().join("not-executable");
    fs::write(&non_executable, b"#!/bin/sh\n").unwrap();
    assert_ne!(
        interactive_shell(Some(non_executable.as_os_str())),
        non_executable
    );
    assert!(!temporary.path().join("sentinel").exists());
}

#[test]
fn selected_shell_path_reaches_exec_as_one_argument_without_evaluation() {
    let temporary = temporary();
    let selected = temporary
        .path()
        .join("shell ' \" ; $(touch SHELL_SENTINEL)");
    fs::write(&selected, "#!/bin/sh\nprintf '%s\\n' \"$0\" \"$@\" >&2\n").unwrap();
    fs::set_permissions(&selected, fs::Permissions::from_mode(0o700)).unwrap();
    let output = launch_shell(&selected, "", temporary.path(), temporary.path(), b"");
    assert_eq!(output, format!("{}\n-i\n", selected.display()));
    assert!(!temporary.path().join("SHELL_SENTINEL").exists());
}

#[test]
fn default_interactive_prompt_shows_user_host_and_updates_directory_without_path_evaluation() {
    let temporary = temporary();
    let home = temporary.path().join("home");
    let first = temporary.path().join("cwd 文档 $(touch SENTINEL)");
    let second = first.join("child e\u{301}");
    fs::create_dir(&home).unwrap();
    fs::create_dir_all(&second).unwrap();
    let mut username = Command::new("id").arg("-un").output().unwrap();
    let has_username = username.status.success();
    if !has_username {
        username = Command::new("id").arg("-u").output().unwrap();
    }
    assert!(username.status.success());
    let hostname = Command::new("uname").arg("-n").output().unwrap();
    assert!(hostname.status.success());
    let username = String::from_utf8(username.stdout).unwrap();
    let hostname = String::from_utf8(hostname.stdout).unwrap();
    let hostname = hostname.trim().split('.').next().unwrap();
    let mark = if rustix::process::geteuid().as_raw() == 0 {
        '#'
    } else {
        '$'
    };
    for shell in [
        Path::new("/bin/bash"),
        Path::new("/bin/sh"),
        Path::new("/usr/bin/bash"),
        Path::new("/usr/bin/sh"),
    ] {
        if !is_executable(shell) {
            continue;
        }
        assert_eq!(interactive_shell(Some(shell.as_os_str())), shell);
        // Bash owns its native unknown-user label, including prompts set by its rc files.
        let username = if !has_username && shell.file_name() == Some(OsStr::new("bash")) {
            "I have no name!"
        } else {
            username.trim()
        };
        let prefix = format!("{username}@{hostname}:");
        let output = launch_shell(shell, "", &home, &first, b"cd 'child e\xcc\x81'\nexit\n");
        for path in [&first, &second] {
            assert!(
                output.contains(&format!("{prefix}{}{mark} ", path.display())),
                "{shell:?} did not show {path:?}: {output:?}",
            );
        }
        assert!(!first.join("SENTINEL").exists());
        assert!(!second.join("SENTINEL").exists());
    }
}

#[test]
fn interactive_shell_reads_normal_bash_configuration_and_retains_submitted_command_cwd() {
    let Some(shell) = [Path::new("/bin/bash"), Path::new("/usr/bin/bash")]
        .into_iter()
        .find(|path| is_executable(path))
    else {
        return;
    };
    let temporary = temporary();
    let home = temporary.path().join("home");
    let child = temporary.path().join("child 文档");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&child).unwrap();
    fs::write(home.join(".bashrc"), "PS1='MY CUSTOM PROMPT ${PWD}> '\n").unwrap();
    let output = launch_shell(
        shell,
        "cd 'child 文档'; set -- changed arguments",
        &home,
        temporary.path(),
        b"exit\n",
    );
    assert!(
        output.contains(&format!("MY CUSTOM PROMPT {}> ", child.display())),
        "{output:?}",
    );
}

#[test]
fn explicit_prompt_from_submitted_source_is_respected() {
    let temporary = temporary();
    let output = launch_shell(
        Path::new("/bin/sh"),
        "PS1='EXPLICIT PROMPT> '; export PS1",
        temporary.path(),
        temporary.path(),
        b"exit\n",
    );
    assert!(output.contains("EXPLICIT PROMPT> "), "{output:?}");
}

#[test]
fn fallback_prompt_uses_posix_utilities_and_numeric_uid_when_username_lookup_fails() {
    let temporary = temporary();
    let bin = temporary.path().join("bin");
    fs::create_dir(&bin).unwrap();
    for (name, source) in [
        (
            "id",
            "#!/bin/sh\ncase \"$1\" in\n-un) echo 'unknown user' >&2; exit 1;;\n-u) printf '%s\\n' \"$_ME_TEST_UID\";;\n*) exit 2;;\nesac\n",
        ),
        (
            "uname",
            "#!/bin/sh\n[ \"$#\" = 1 ] && [ \"$1\" = -n ] || exit 2\nprintf '%s\\n' portable-host.example.invalid\n",
        ),
    ] {
        let path = bin.join(name);
        fs::write(&path, source).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
    }
    for (uid, mark) in [("4242", '$'), ("0", '#')] {
        let source =
            format!("_ME_TEST_UID={uid}; export _ME_TEST_UID; PATH=\"$HOME/bin\"; export PATH");
        let output = launch_shell(
            Path::new("/bin/sh"),
            &source,
            temporary.path(),
            temporary.path(),
            b"exit\n",
        );
        assert!(
            output.contains(&format!(
                "{uid}@portable-host:{}{mark} ",
                temporary.path().display()
            )),
            "{output:?}",
        );
        assert!(!output.contains("unknown user"), "{output:?}");
        assert!(!output.contains("not found"), "{output:?}");
    }
}
