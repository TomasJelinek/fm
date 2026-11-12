#![cfg(windows)]
use midnight_worker_platform::store::PrivateStore;
use std::{
    ffi::OsString,
    fs,
    os::windows::{
        ffi::{OsStrExt, OsStringExt},
        process::CommandExt,
    },
    path::Path,
};

fn directory() -> tempfile::TempDir {
    let root = std::env::var_os("MIDNIGHT_NATIVE_TEST_ROOT")
        .unwrap_or_else(|| env!("CARGO_TARGET_TMPDIR").into());
    fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

#[test]
fn bucket_keeps_all_ancestors_pinned_after_its_parent_store_is_dropped() {
    let fixture = directory();
    let path = fixture.path().join("cache");
    let store = PrivateStore::open(&path).unwrap();
    let bucket = store.bucket("ab", true).unwrap().unwrap();
    drop(store);
    assert!(fs::rename(&path, fixture.path().join("moved")).is_err());
    assert!(fs::rename(path.join("ab"), path.join("cd")).is_err());
    drop(bucket);
    fs::rename(path.join("ab"), path.join("cd")).unwrap();
}

#[test]
fn buckets_accept_mixed_separators_without_resolving_or_losing_native_path_units() {
    let fixture = directory();
    let units = fixture
        .path()
        .as_os_str()
        .encode_wide()
        .map(|unit| {
            if unit == b'\\' as u16 {
                b'/' as u16
            } else {
                unit
            }
        })
        .collect::<Vec<_>>();
    let mixed = std::path::PathBuf::from(OsString::from_wide(&units))
        .join("nested/cache")
        .join(OsString::from_wide(&[0x79d8, 0x5bc6, 0xd800]));
    let store = PrivateStore::open(&mixed).unwrap();
    let bucket = store.bucket("ab", true).unwrap().unwrap();
    let name = format!("ab{}.mtc", "1".repeat(62));
    bucket.write(&name, b"preserved").unwrap();
    assert_eq!(bucket.read(&name, 32).unwrap().unwrap(), b"preserved");
    assert_eq!(
        store
            .bucket("ab", false)
            .unwrap()
            .unwrap()
            .records()
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn buckets_reject_junctions_nonprivate_directories_and_ordinary_files() {
    let fixture = directory();
    let path = fixture.path().join("cache");
    let outside = fixture.path().join("outside");
    fs::create_dir(&outside).unwrap();
    let store = PrivateStore::open(&path).unwrap();
    let helper = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../midnight-platform/tests/create-junction.ps1");
    let units: Vec<u16> = outside.as_os_str().encode_wide().collect();
    let target = if units.starts_with(&[92, 92, 63, 92]) {
        OsString::from_wide(&units[4..])
    } else {
        outside.clone().into_os_string()
    };
    let output = std::process::Command::new("pwsh")
        .args(["-NoProfile", "-File"])
        .arg(helper)
        .arg(path.join("ab"))
        .arg(target)
        .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(store.bucket("ab", false).is_err());
    assert!(store.bucket("ab", true).is_err());
    assert!(PrivateStore::open(&path.join("ab")).is_err());
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
    // An existing ordinary directory with the surrounding fixture's inherited ACL is refused.
    fs::create_dir(fixture.path().join("public")).unwrap();
    fs::rename(fixture.path().join("public"), path.join("cd")).unwrap();
    assert!(store.bucket("cd", false).is_err());
    assert!(store.bucket("cd", true).is_err());
    assert!(PrivateStore::open(&path.join("cd")).is_err());
    fs::write(path.join("ef"), b"ordinary file").unwrap();
    assert!(store.bucket("ef", false).is_err());
    assert!(PrivateStore::open(&path.join("ef")).is_err());
}
