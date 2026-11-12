use crate::CapturedFile;
use std::{
    ffi::OsString,
    fs,
    os::windows::ffi::OsStringExt,
    path::{Path, PathBuf},
};

fn fixture() -> tempfile::TempDir {
    let repository = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap();
    let root = repository.join("_build/tmp");
    fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("viewer-literal-")
        .tempdir_in(root)
        .unwrap()
}

#[test]
fn capture_reads_dotted_spaced_and_device_looking_native_names_without_sibling_retargeting() {
    let fixture = fixture();
    let literal_root = fixture.path().canonicalize().unwrap();
    for (name, sibling) in [
        ("dotted.", "dotted"),
        ("spaced ", "spaced"),
        ("NUL", "ordinary"),
    ] {
        fs::write(fixture.path().join(sibling), b"ordinary").unwrap();
        fs::write(literal_root.join(name), b"selected literal").unwrap();
        let raw = fixture.path().join(name);
        let captured = CapturedFile::capture(&raw).unwrap();
        assert_eq!(captured.path(), raw);
        assert_eq!(captured.length(), 16);
        assert_eq!(
            captured.read_at(0, 100).unwrap(),
            b"selected literal",
            "{name}"
        );
        captured.revalidate().unwrap();
        assert_eq!(fs::read(fixture.path().join(sibling)).unwrap(), b"ordinary");
    }
}

#[test]
fn capture_keeps_unpaired_utf16_and_long_native_addresses_lossless() {
    let fixture = fixture();
    let mut raw = fixture.path().to_owned();
    let mut literal = raw.canonicalize().unwrap();
    while raw.as_os_str().len() < 320 {
        raw.push("long native directory");
        literal.push("long native directory");
        fs::create_dir(&literal).unwrap();
    }
    let name = OsString::from_wide(&[b'f' as u16, 0xd800]);
    raw.push(&name);
    literal.push(name);
    fs::write(literal, b"raw long source").unwrap();
    let captured = CapturedFile::capture(&raw).unwrap();
    assert_eq!(captured.path(), raw);
    assert_eq!(captured.read_at(0, 100).unwrap(), b"raw long source");
    captured.revalidate().unwrap();
}

#[test]
fn capture_refuses_device_namespaces_nul_and_dot_steps_before_open() {
    for path in [
        PathBuf::from(r"\\.\NUL"),
        PathBuf::from(r"\\?\GLOBALROOT\Device\Null"),
        PathBuf::from(r"C:\source\..\file"),
        PathBuf::from(OsString::from_wide(&[67, 58, 92, 0, 120])),
    ] {
        assert!(CapturedFile::capture(&path).is_err(), "{}", path.display());
    }
}
