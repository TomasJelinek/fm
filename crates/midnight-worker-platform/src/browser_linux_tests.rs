use std::{
    fs,
    io::{BufRead, BufReader},
    os::unix::fs::{MetadataExt, PermissionsExt},
};

#[test]
fn browser_temporary_cleanup_never_follows_content_symlinks() {
    let outside = tempfile::tempdir().unwrap();
    let retained = outside.path().join("retained");
    fs::write(&retained, b"outside owned temporary tree").unwrap();
    let directory = super::temporary::Directory::new().unwrap();
    let temporary = directory.path().to_path_buf();
    assert!(temporary.as_os_str().len() <= 60);
    std::os::unix::fs::symlink(outside.path(), temporary.join("foreign")).unwrap();
    drop(directory);
    assert!(!temporary.exists());
    assert_eq!(fs::read(retained).unwrap(), b"outside owned temporary tree");
}

#[test]
#[ignore = "requires native Linux delegated user-manager cgroup"]
fn browser_temporary_is_short_private_and_retained_until_tree_reaping() {
    let root = tempfile::tempdir().unwrap();
    let long_parent = root.path().join("long-profile-".repeat(15));
    let profile = super::super::profile::Profile::new(&long_parent).unwrap();
    assert!(profile.path().as_os_str().len() > 160);
    let launcher = root.path().join("browser-fixture");
    fs::write(
        &launcher,
        "#!/bin/sh\nprintf '%s\\n' \"$TMPDIR\"\nread answer\n",
    )
    .unwrap();
    fs::set_permissions(&launcher, fs::Permissions::from_mode(0o700)).unwrap();
    let mut browser = super::spawn(&launcher, profile).unwrap();
    let mut temporary = String::new();
    BufReader::new(&mut browser.output)
        .read_line(&mut temporary)
        .unwrap();
    let temporary = std::path::PathBuf::from(temporary.trim_end());
    assert!(
        temporary.as_os_str().len() <= 60,
        "Chromium needs short AF_UNIX socket paths"
    );
    let metadata = fs::symlink_metadata(&temporary).unwrap();
    assert!(metadata.is_dir());
    assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());
    assert_eq!(metadata.mode() & 0o777, 0o700);
    browser.process.kill().unwrap();
    assert!(
        temporary.is_dir(),
        "temporary lifetime must include process-tree reaping"
    );
    browser.process.wait().unwrap();
    drop(browser);
    assert!(
        !temporary.exists(),
        "owned browser temporary tree must be cleaned"
    );
}
