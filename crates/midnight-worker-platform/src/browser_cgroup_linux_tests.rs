use super::*;
use std::{
    io::{BufRead, BufReader},
    os::unix::fs::{PermissionsExt, symlink},
    process::Stdio,
};

// Removing the native-filesystem check must allow this ordinary private fixture and fail this test.
#[test]
fn ordinary_directory_cannot_impersonate_a_delegated_cgroup() {
    let directory = tempfile::tempdir().unwrap();
    assert!(open_parent(directory.path()).is_err());
}

// Removing the no-follow ancestry walk must allow this alias and fail this test.
#[test]
fn cgroup_parent_rejects_symlink_ancestry() {
    let directory = tempfile::tempdir().unwrap();
    let alias = directory.path().join("alias");
    symlink("/sys/fs/cgroup", &alias).unwrap();
    assert!(open_parent(&alias).is_err());
}

fn controls() -> (tempfile::TempDir, File) {
    let temporary = tempfile::tempdir().unwrap();
    for name in [
        "memory.max",
        "memory.swap.max",
        "memory.oom.group",
        "pids.max",
    ] {
        File::create(temporary.path().join(name)).unwrap();
    }
    let directory = File::open(temporary.path()).unwrap();
    (temporary, directory)
}

#[test]
fn all_tree_limits_are_written_and_checked_before_launch() {
    let (temporary, directory) = controls();
    configure_limits(&directory).unwrap();
    for (name, expected) in [
        ("memory.max", "2147483648"),
        ("memory.swap.max", "0"),
        ("memory.oom.group", "1"),
        ("pids.max", "256"),
    ] {
        assert_eq!(
            std::fs::read_to_string(temporary.path().join(name)).unwrap(),
            expected
        );
    }
}

#[test]
fn missing_tree_limit_is_an_error_and_never_creates_a_fake_control() {
    let (temporary, directory) = controls();
    std::fs::remove_file(temporary.path().join("pids.max")).unwrap();
    assert_eq!(
        configure_limits(&directory).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    assert!(!temporary.path().join("pids.max").exists());
}

#[test]
fn limit_readback_must_confirm_the_value() {
    let (temporary, directory) = controls();
    // A stale longer value must not be mistaken for the new hard limit. Real cgroup controls
    // replace one value atomically; this ordinary-file fixture deliberately violates that rule.
    std::fs::write(temporary.path().join("memory.max"), "214748364800000").unwrap();
    assert!(configure_limits(&directory).is_err());
}

#[test]
fn controls_reject_links_special_files_and_foreign_write_access() {
    let (temporary, directory) = controls();
    let maximum = temporary.path().join("memory.max");
    std::fs::set_permissions(&maximum, std::fs::Permissions::from_mode(0o666)).unwrap();
    assert!(configure_limits(&directory).is_err());
    std::fs::remove_file(&maximum).unwrap();
    symlink("pids.max", &maximum).unwrap();
    assert!(configure_limits(&directory).is_err());
    std::fs::remove_file(&maximum).unwrap();
    rustix::fs::mknodat(
        &directory,
        "memory.max",
        rustix::fs::FileType::Fifo,
        Mode::RUSR | Mode::WUSR,
        0,
    )
    .unwrap();
    assert!(read_control(&directory, "memory.max").is_err());
}

#[test]
fn cgroup_metadata_is_bounded() {
    let (temporary, directory) = controls();
    std::fs::write(temporary.path().join("memory.max"), vec![b'0'; 4097]).unwrap();
    assert!(read_control(&directory, "memory.max").is_err());
}

#[test]
fn failed_session_setup_removes_only_the_owned_new_directory() {
    let (temporary, directory) = controls();
    let before = std::fs::read_dir(temporary.path()).unwrap().count();
    assert!(Session::create(directory).is_err());
    assert_eq!(std::fs::read_dir(temporary.path()).unwrap().count(), before);
    assert!(temporary.path().join("memory.max").exists());
}

#[test]
fn root_owned_delegation_is_denied_to_regular_users() {
    if geteuid().as_raw() != 0 {
        let error = open_parent(Path::new("/sys/fs/cgroup")).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
    }
}

#[test]
fn unavailable_delegation_diagnostic_explains_the_required_configuration() {
    let diagnostic = unavailable("fixture has no delegation").to_string();
    assert!(diagnostic.contains("cgroup-v2"));
    assert!(diagnostic.contains("memory and pids"));
    assert!(diagnostic.contains("MIDNIGHT_WEB_VIEWER_CGROUP_PARENT"));
}

#[test]
fn invalid_delegation_denies_browser_before_exec() {
    const CHILD: &str = "MIDNIGHT_CGROUP_TEST_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let temporary = tempfile::tempdir().unwrap();
        std::fs::set_permissions(temporary.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        let profile = super::super::super::profile::Profile::new(temporary.path()).unwrap();
        let result = super::super::spawn(Path::new("/bin/true"), profile);
        let error = match result {
            Err(error) => error,
            Ok(_) => panic!("Browser started without an actual cgroup-v2 delegation"),
        };
        assert_eq!(error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.to_string().contains(PARENT_ENV));
        return;
    }
    // Isolate the environment override from concurrently running tests and prove the actual
    // browser launch path refuses an ordinary directory before it executes /bin/true.
    let temporary = tempfile::tempdir().unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "browser::native::cgroup::tests::invalid_delegation_denies_browser_before_exec",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env(PARENT_ENV, temporary.path())
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

// Run explicitly on a machine with an existing usable user delegation. A skipped test is not
// evidence of native Chromium or kernel cgroup qualification.
#[test]
#[ignore = "requires existing writable cgroup-v2 memory/pids delegation"]
fn native_tree_membership_and_termination_include_independent_process_groups() {
    let session = Session::new().unwrap();
    let mut command = Command::new("/bin/sh");
    command
        .arg("-c")
        .arg("/usr/bin/setsid /bin/sleep 30 & echo $!; wait")
        .stdin(Stdio::null())
        .stdout(Stdio::piped());
    session.prepare_command(&mut command).unwrap();
    let child = command.spawn().unwrap();
    // The production process guard kills and reaps on every assertion/error path too.
    let mut process = super::super::Process {
        child,
        cgroup: session,
        temporary: super::super::temporary::Directory::new().unwrap(),
    };
    let mut descendant = String::new();
    BufReader::new(process.child.stdout.take().unwrap())
        .read_line(&mut descendant)
        .unwrap();
    let pid: u32 = descendant.trim().parse().unwrap();
    let membership = std::fs::read_to_string(format!("/proc/{pid}/cgroup")).unwrap();
    assert!(membership.contains(&process.cgroup.name));
    process.kill().unwrap();
    assert!(!process.wait().unwrap());
}
