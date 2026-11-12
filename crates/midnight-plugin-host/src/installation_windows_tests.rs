use super::*;
use std::os::windows::ffi::OsStrExt;

const DIGEST: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

struct Fixture {
    _root: tempfile::TempDir,
    source: PathBuf,
    target: PathBuf,
    registrations: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let base =
            std::env::var_os("HOST_TEST_INSTALL_ROOT").expect("protected HOST_TEST_INSTALL_ROOT");
        let root = tempfile::tempdir_in(base).unwrap();
        crate::trust::protect_fixture(root.path()).unwrap();
        let source_directory = root.path().join("shared");
        let target_directory = root.path().join("private");
        let registrations = root.path().join("registrations");
        for directory in [&source_directory, &target_directory, &registrations] {
            std::fs::create_dir(directory).unwrap();
        }
        let source = source_directory.join("me.exe");
        let target = target_directory.join("me.exe");
        std::fs::write(&source, b"abc").unwrap();
        std::fs::write(&target, b"abc").unwrap();
        std::fs::create_dir(source_directory.join("viewers")).unwrap();
        crate::trust::fixture_acl(&source_directory, "D:P(A;OICI;FA;;;WD)").unwrap();
        let fixture = Self {
            _root: root,
            source,
            target,
            registrations,
        };
        fixture.write_record(&fixture.target);
        fixture
    }

    fn record(&self) -> PathBuf {
        self.registrations.join(format!("{DIGEST}.toml"))
    }

    fn write_record(&self, executable: &Path) {
        let wide: Vec<_> = executable.as_os_str().encode_wide().collect();
        std::fs::write(
            self.record(),
            format!("schema_version=1\nsha256=\"{DIGEST}\"\nexecutable_utf16={wide:?}\n"),
        )
        .unwrap();
    }

    fn resolve(&self) -> Result<PathBuf, Error> {
        resolve_at(&self.source, Some(&self.registrations))
    }
}

#[test]
fn exact_registered_build_resolves_from_a_shared_launch_directory() {
    let fixture = Fixture::new();
    assert_eq!(
        fixture.resolve().unwrap(),
        fixture.target.canonicalize().unwrap()
    );
}

#[test]
fn trusted_portable_installation_precedes_registration_and_needs_no_plugin_directories() {
    let fixture = Fixture::new();
    std::fs::write(fixture.record(), b"malformed registration").unwrap();
    assert_eq!(
        resolve_at(&fixture.target, Some(&fixture.registrations)).unwrap(),
        fixture.target.canonicalize().unwrap()
    );
    assert_eq!(
        resolve_at(&fixture.target, None).unwrap(),
        fixture.target.canonicalize().unwrap()
    );
}

#[test]
fn changed_source_does_not_select_an_older_registered_build() {
    let fixture = Fixture::new();
    std::fs::write(&fixture.source, b"different build").unwrap();
    assert!(
        fixture
            .resolve()
            .unwrap_err()
            .0
            .contains("No private plugin runtime")
    );
}

#[test]
fn registered_executable_must_match_the_launch_executable_bytes() {
    let fixture = Fixture::new();
    std::fs::write(&fixture.target, b"abd").unwrap();
    assert!(fixture.resolve().unwrap_err().0.contains("does not match"));
}

#[test]
fn absent_registration_allows_only_a_standalone_executable() {
    let fixture = Fixture::new();
    std::fs::remove_file(fixture.record()).unwrap();
    assert!(fixture.resolve().is_err());
    std::fs::remove_dir(fixture.source.parent().unwrap().join("viewers")).unwrap();
    assert_eq!(
        fixture.resolve().unwrap(),
        fixture.source.canonicalize().unwrap()
    );
    for directory in ["viewers", "filesystems", "thumbnailers"] {
        let path = fixture.source.parent().unwrap().join(directory);
        std::fs::create_dir(&path).unwrap();
        assert!(
            fixture.resolve().is_err(),
            "unregistered {directory} accepted"
        );
        std::fs::remove_dir(path).unwrap();
    }
    std::fs::remove_dir(&fixture.registrations).unwrap();
    assert!(fixture.resolve().is_ok());
    assert!(resolve_at(&fixture.source, None).is_ok());
}

#[test]
fn shared_writable_registration_directory_or_record_is_rejected() {
    for directory in [true, false] {
        let fixture = Fixture::new();
        let path = if directory {
            fixture.registrations.clone()
        } else {
            fixture.record()
        };
        crate::trust::fixture_acl(&path, "D:P(A;OICI;FA;;;WD)").unwrap();
        assert!(
            fixture
                .resolve()
                .unwrap_err()
                .0
                .contains("writable or replaceable")
        );
    }
}

#[test]
fn shared_writable_target_or_replaceable_target_parent_is_rejected() {
    for directory in [true, false] {
        let fixture = Fixture::new();
        // Protect the file itself so the ancestor case proves the parent check.
        crate::trust::protect_fixture(&fixture.target).unwrap();
        let path = if directory {
            fixture.target.parent().unwrap()
        } else {
            &fixture.target
        };
        crate::trust::fixture_acl(path, "D:P(A;OICI;FA;;;WD)").unwrap();
        assert!(
            fixture
                .resolve()
                .unwrap_err()
                .0
                .contains("writable or replaceable")
        );
    }
}

#[test]
fn protected_registration_child_cannot_hide_a_replaceable_parent() {
    let fixture = Fixture::new();
    let nested = fixture.registrations.join("private");
    std::fs::create_dir(&nested).unwrap();
    crate::trust::protect_fixture(&nested).unwrap();
    std::fs::rename(fixture.record(), nested.join(format!("{DIGEST}.toml"))).unwrap();
    crate::trust::fixture_acl(&fixture.registrations, "D:P(A;OICI;FA;;;WD)").unwrap();
    assert!(resolve_at(&fixture.source, Some(&nested)).is_err());
}

#[test]
fn registration_is_strict_bounded_and_rejects_invalid_native_paths() {
    let fixture = Fixture::new();
    let wide: Vec<_> = fixture.target.as_os_str().encode_wide().collect();
    for record in [
        "not TOML".to_owned(),
        format!("schema_version=2\nsha256=\"{DIGEST}\"\nexecutable_utf16={wide:?}"),
        format!(
            "schema_version=1\nsha256=\"{}\"\nexecutable_utf16={wide:?}",
            "0".repeat(64)
        ),
        format!("schema_version=1\nsha256=\"{DIGEST}\"\nexecutable_utf16={wide:?}\nunknown=1"),
        format!("schema_version=1\nsha256=\"{DIGEST}\"\nexecutable_utf16=[]"),
        format!("schema_version=1\nsha256=\"{DIGEST}\"\nexecutable_utf16=[67,58,92,0]"),
        format!("schema_version=1\nsha256=\"{DIGEST}\"\nexecutable_utf16=[109,101]"),
        format!("schema_version=1\nsha256=\"{DIGEST}\"\nexecutable_utf16=[65536]"),
        format!(
            "schema_version=1\nsha256=\"{DIGEST}\"\nexecutable_utf16={wide:?}\nschema_version=1"
        ),
    ] {
        std::fs::write(fixture.record(), &record).unwrap();
        assert!(
            fixture.resolve().is_err(),
            "accepted registration: {record}"
        );
    }
    std::fs::write(
        fixture.record(),
        vec![b' '; MAX_REGISTRATION_BYTES as usize + 1],
    )
    .unwrap();
    assert!(fixture.resolve().unwrap_err().0.contains("exceeds 1 MiB"));
}

#[test]
fn registration_preserves_unicode_and_unpaired_utf16_path_units() {
    let fixture = Fixture::new();
    let mut name: Vec<_> = "český-日本語-😀-".encode_utf16().collect();
    name.push(0xd800);
    name.extend(".exe".encode_utf16());
    let target = fixture
        .target
        .parent()
        .unwrap()
        .join(OsString::from_wide(&name));
    std::fs::write(&target, b"abc").unwrap();
    fixture.write_record(&target);
    assert_eq!(fixture.resolve().unwrap(), target.canonicalize().unwrap());
}

fn junction(path: &Path, target: &Path) {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../midnight-platform/tests/create-junction.ps1");
    let powershell = PathBuf::from(std::env::var_os("SystemRoot").unwrap())
        .join("System32/WindowsPowerShell/v1.0/powershell.exe");
    let output = std::process::Command::new(powershell)
        .args(["-NoProfile", "-NonInteractive", "-File"])
        .arg(script)
        .arg("-JunctionPath")
        .arg(path)
        .arg("-TargetPath")
        .arg(target)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn registered_target_ancestor_reparse_is_rejected_before_canonicalization() {
    let fixture = Fixture::new();
    let redirect = fixture._root.path().join("redirect");
    junction(&redirect, fixture.target.parent().unwrap());
    fixture.write_record(&redirect.join("me.exe"));
    let result = fixture.resolve();
    std::fs::remove_dir(&redirect).unwrap();
    assert!(result.unwrap_err().0.contains("reparse point"));
    assert_eq!(std::fs::read(&fixture.target).unwrap(), b"abc");
}

#[test]
fn registration_root_reparse_is_rejected_before_canonicalization() {
    let fixture = Fixture::new();
    let redirect = fixture._root.path().join("redirect");
    junction(&redirect, &fixture.registrations);
    let result = resolve_at(&fixture.source, Some(&redirect));
    std::fs::remove_dir(&redirect).unwrap();
    assert!(result.unwrap_err().0.contains("reparse point"));
}

#[test]
fn launch_path_reparse_is_rejected_before_canonicalization() {
    let fixture = Fixture::new();
    let redirect = fixture._root.path().join("redirect");
    junction(&redirect, fixture.source.parent().unwrap());
    let result = resolve_at(&redirect.join("me.exe"), Some(&fixture.registrations));
    std::fs::remove_dir(&redirect).unwrap();
    assert!(result.unwrap_err().0.contains("reparse point"));
}

#[test]
fn hashing_and_registration_reads_deny_concurrent_writes_and_replacement() {
    let fixture = Fixture::new();
    for path in [&fixture.source, &fixture.target, &fixture.record()] {
        let _locked = locked_file(path).unwrap();
        assert!(std::fs::write(path, b"replacement").is_err());
        assert!(std::fs::rename(path, path.with_extension("renamed")).is_err());
    }
}
