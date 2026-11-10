#![cfg(windows)]
use midnight_worker_platform::spool::PrivateSpoolDirectory;
use std::{
    ffi::OsString,
    fs,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        process::CommandExt,
    },
    path::Path,
};

#[test]
fn spool_keeps_ancestors_pinned_after_factory_drop_and_denies_external_opens() {
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("private");
    let root = PrivateSpoolDirectory::open(&path).unwrap();
    let mut spool = root.create().unwrap();
    spool.append(0, b"private").unwrap();
    spool.finish().unwrap();
    drop(root);
    assert!(fs::rename(&path, temporary.path().join("moved")).is_err());
    let file = fs::read_dir(&path).unwrap().next().unwrap().unwrap().path();
    assert!(fs::File::open(&file).is_err());
    assert!(fs::write(&file, b"tamper").is_err());
    drop(spool);
    assert!(!file.exists());
    fs::rename(&path, temporary.path().join("moved")).unwrap();
}

#[test]
fn spool_root_rejects_junction_and_nonprivate_directory() {
    let temporary = tempfile::tempdir().unwrap();
    let outside = temporary.path().join("outside");
    fs::create_dir(&outside).unwrap();
    assert!(PrivateSpoolDirectory::open(&outside).is_err());
    let junction = temporary.path().join("junction");
    let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../midnight-platform/tests/create-junction.ps1");
    let units: Vec<u16> = outside.as_os_str().encode_wide().collect();
    let target = if units.starts_with(&[92, 92, 63, 92]) {
        OsString::from_wide(&units[4..])
    } else {
        outside.into_os_string()
    };
    let output = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-File"])
        .arg(helper)
        .arg(&junction)
        .arg(target)
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(PrivateSpoolDirectory::open(&junction).is_err());
    assert!(PrivateSpoolDirectory::open(&junction.join("child")).is_err());
    assert!(!temporary.path().join("outside/child").exists());
}
