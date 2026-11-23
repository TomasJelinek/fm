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
        let source = source_directory.join("midnight-explorer.exe");
        let target = target_directory.join("midnight-explorer.exe");
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

    /// A launch folder refused for a reason other than shared permissions: an allow entry
    /// with a condition, which the trust check never interprets.
    fn refused() -> Self {
        let fixture = Self::new();
        let user = crate::trust::user_sid_string().unwrap();
        crate::trust::fixture_acl(
            fixture.source.parent().unwrap(),
            &format!(
                "D:P(A;OICI;FA;;;{user})(A;OICI;FA;;;SY)\
                 (XA;OICI;FA;;;WD;(Member_of {{SID(BA)}}))"
            ),
        )
        .unwrap();
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
        resolve_at(
            &self.source,
            Some(&self.registrations),
            RuntimeVerification::Sha256,
        )
    }

    fn write_pointer(&self) -> PathBuf {
        let launch = launch_path::open(&self.source).unwrap();
        let launchers = self.registrations.join("launchers");
        std::fs::create_dir_all(&launchers).unwrap();
        let pointer = launchers.join(format!("{}.toml", launch_path_key(&launch.utf16)));
        std::fs::write(
            &pointer,
            format!(
                "schema_version=1\nlaunch_executable_utf16={:?}\nlaunch_length={}\n\
                 launch_last_write_time_100ns={}\nregistration_sha256=\"{DIGEST}\"\n",
                launch.utf16, launch.length, launch.last_write_time_100ns,
            ),
        )
        .unwrap();
        pointer
    }
}

fn resolve_without_digest(executable: &Path, root: Option<&Path>) -> Result<PathBuf, Error> {
    let mut calls = 0;
    let result = resolve_at_with_digest(executable, root, RuntimeVerification::Disabled, |_, _| {
        calls += 1;
        Err(Error(
            "Disabled runtime verification requested an executable digest".into(),
        ))
    });
    assert_eq!(calls, 0, "OFF must never invoke an executable digester");
    result
}

/// A refused installation explains itself without developer instructions.
fn assert_pointer_update(error: &Error) {
    assert!(error.0.contains("cannot load its plugins"), "{}", error.0);
    assert!(error.0.contains("launcher pointer"), "{}", error.0);
    assert!(!error.0.contains("deploy-private"), "{}", error.0);
    assert!(!error.0.contains("RegisterRuntime"), "{}", error.0);
}

#[test]
fn shared_installation_runs_without_any_registration() {
    let fixture = Fixture::new();
    let source = fixture.source.canonicalize().unwrap();
    let missing = fixture._root.path().join("missing-registrations");
    for root in [None, Some(missing.as_path())] {
        assert_eq!(
            resolve_without_digest(&fixture.source, root).unwrap(),
            source
        );
        assert_eq!(
            resolve_at(&fixture.source, root, RuntimeVerification::Sha256).unwrap(),
            source
        );
    }
    // A stale or absent pointer no longer stops a shared installation either.
    let pointer = fixture.write_pointer();
    std::fs::write(&fixture.source, b"rebuilt").unwrap();
    assert_eq!(
        resolve_without_digest(&fixture.source, Some(&fixture.registrations)).unwrap(),
        source
    );
    std::fs::remove_file(pointer).unwrap();
    assert_eq!(
        resolve_without_digest(&fixture.source, Some(&fixture.registrations)).unwrap(),
        source
    );
}

#[test]
fn registered_private_runtime_still_precedes_a_shared_installation() {
    let fixture = Fixture::new();
    fixture.write_pointer();
    assert_eq!(
        resolve_without_digest(&fixture.source, Some(&fixture.registrations)).unwrap(),
        fixture.target.canonicalize().unwrap()
    );
    assert_eq!(
        fixture.resolve().unwrap(),
        fixture.target.canonicalize().unwrap()
    );
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
        resolve_at(
            &fixture.target,
            Some(&fixture.registrations),
            RuntimeVerification::Sha256,
        )
        .unwrap(),
        fixture.target.canonicalize().unwrap()
    );
    assert_eq!(
        resolve_at(&fixture.target, None, RuntimeVerification::Sha256).unwrap(),
        fixture.target.canonicalize().unwrap()
    );
}

fn case_alias(path: &Path) -> PathBuf {
    let directory = path.parent().unwrap();
    let mut alias = directory
        .parent()
        .unwrap()
        .join(directory.file_name().unwrap().to_ascii_uppercase());
    alias.push(path.file_name().unwrap().to_ascii_lowercase());
    alias
}

/// The volume's 8.3 spelling, when short names are enabled for it.
fn short_alias(path: &Path) -> Option<PathBuf> {
    use windows_sys::Win32::Storage::FileSystem::GetShortPathNameW;
    let wide: Vec<_> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut buffer = vec![0_u16; 32 * 1024];
    // SAFETY: terminated input and a writable output of the stated capacity.
    let length =
        unsafe { GetShortPathNameW(wide.as_ptr(), buffer.as_mut_ptr(), buffer.len() as u32) }
            as usize;
    if length == 0 || length >= buffer.len() {
        return None;
    }
    buffer.truncate(length);
    let short = PathBuf::from(OsString::from_wide(&buffer));
    (short.file_name() != path.file_name()).then_some(short)
}

#[test]
fn trusted_portable_installation_resolves_native_launch_path_aliases() {
    let fixture = Fixture::new();
    let directory = fixture._root.path().join("Private Runtime Directory");
    std::fs::create_dir(&directory).unwrap();
    let target = directory.join("Midnight Explorer.exe");
    std::fs::write(&target, b"abc").unwrap();
    // Adjacent plugins make every unregistered fallback fail: only trust may resolve.
    std::fs::create_dir(directory.join("viewers")).unwrap();
    let expected = target.canonicalize().unwrap();
    let mut aliases = vec![case_alias(&target)];
    match short_alias(&target) {
        Some(short) => aliases.push(short),
        None => eprintln!("8.3 names are disabled on this volume; short alias skipped"),
    }
    for alias in aliases {
        assert_ne!(runtime_path(&alias).unwrap(), expected, "not an alias");
        for verification in [RuntimeVerification::Disabled, RuntimeVerification::Sha256] {
            let resolved = resolve_at_with_digest(
                &alias,
                Some(&fixture.registrations),
                verification,
                |_, _| Err(Error("trusted portable resolution must not hash".into())),
            );
            assert_eq!(resolved.unwrap(), expected, "{alias:?} {verification:?}");
        }
    }
    // Normalization must not launder a link: the entered spelling is checked first.
    let redirect = fixture._root.path().join("redirect");
    junction(&redirect, &directory);
    let result = resolve_without_digest(
        &redirect.join("Midnight Explorer.exe"),
        Some(&fixture.registrations),
    );
    std::fs::remove_dir(&redirect).unwrap();
    assert!(result.unwrap_err().0.contains("reparse point"));
}

#[test]
fn changed_source_does_not_select_an_older_registered_build() {
    let fixture = Fixture::refused();
    std::fs::write(&fixture.source, b"different build").unwrap();
    let error = fixture.resolve().unwrap_err();
    assert!(error.0.contains("No private plugin runtime"), "{}", error.0);
    assert!(error.0.contains("cannot load its plugins"), "{}", error.0);
    // A shared installation runs as it is instead of an older registered build.
    let fixture = Fixture::new();
    std::fs::write(&fixture.source, b"different build").unwrap();
    assert_eq!(
        fixture.resolve().unwrap(),
        fixture.source.canonicalize().unwrap()
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
    let fixture = Fixture::refused();
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
    assert!(resolve_at(&fixture.source, None, RuntimeVerification::Sha256).is_ok());
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
    assert!(resolve_at(&fixture.source, Some(&nested), RuntimeVerification::Sha256).is_err());
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

use crate::tests_windows::junction;

#[test]
fn registered_target_ancestor_reparse_is_rejected_before_canonicalization() {
    let fixture = Fixture::new();
    let redirect = fixture._root.path().join("redirect");
    junction(&redirect, fixture.target.parent().unwrap());
    fixture.write_record(&redirect.join("midnight-explorer.exe"));
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
    let result = resolve_at(
        &fixture.source,
        Some(&redirect),
        RuntimeVerification::Sha256,
    );
    std::fs::remove_dir(&redirect).unwrap();
    assert!(result.unwrap_err().0.contains("reparse point"));
}

#[test]
fn launch_path_reparse_is_rejected_before_canonicalization() {
    let fixture = Fixture::new();
    let redirect = fixture._root.path().join("redirect");
    junction(&redirect, fixture.source.parent().unwrap());
    let result = resolve_at(
        &redirect.join("midnight-explorer.exe"),
        Some(&fixture.registrations),
        RuntimeVerification::Sha256,
    );
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

#[test]
fn executable_snapshot_diagnostics_identifies_role_and_actual_path() {
    let before = crate::native::test_support::snapshot(3, 7, [0; 16], 11, 13);
    for (role, path, expected_context) in [
        (
            ExecutableRole::Launch,
            Path::new(r"\\?\D:\český build\midnight-explorer.exe"),
            r#"role=launch_executable; path="\\\\?\\D:\\český build\\midnight-explorer.exe""#,
        ),
        (
            ExecutableRole::RegisteredTarget,
            Path::new(r"\\?\C:\private runtime\midnight-explorer.exe"),
            r#"role=registered_target; path="\\\\?\\C:\\private runtime\\midnight-explorer.exe""#,
        ),
    ] {
        let error = validate_executable_snapshot(path, role, &before, &before, 2).unwrap_err();
        assert!(
            error
                .0
                .starts_with("Plugin runtime executable changed while hashing")
        );
        assert!(error.0.contains(expected_context), "{}", error.0);
    }
}

#[test]
fn executable_snapshot_diagnostics_reports_changed_fields_and_exact_values() {
    let before = crate::native::test_support::snapshot(3, 7, [0; 16], 11, 13);
    for (after, expected_fields) in [
        (
            crate::native::test_support::snapshot(4, 7, [0; 16], 11, 13),
            "changed_fields=[length(before=3, after=4)]",
        ),
        (
            crate::native::test_support::snapshot(3, 8, [0; 16], 11, 13),
            "changed_fields=[volume(before=7, after=8)]",
        ),
        (
            crate::native::test_support::snapshot(3, 7, [0xab; 16], 11, 13),
            "changed_fields=[file_id(before=00000000000000000000000000000000, \
             after=abababababababababababababababab)]",
        ),
        (
            crate::native::test_support::snapshot(3, 7, [0; 16], 12, 13),
            "changed_fields=[LastWriteTime(before=11, after=12)]",
        ),
        (
            crate::native::test_support::snapshot(3, 7, [0; 16], 11, 14),
            "changed_fields=[ChangeTime(before=13, after=14)]",
        ),
        (
            crate::native::test_support::snapshot(4, 8, [0xab; 16], 12, 14),
            "changed_fields=[length(before=3, after=4), volume(before=7, after=8), \
             file_id(before=00000000000000000000000000000000, \
             after=abababababababababababababababab), LastWriteTime(before=11, after=12), \
             ChangeTime(before=13, after=14)]",
        ),
    ] {
        let error = validate_executable_snapshot(
            Path::new(r"D:\launch\midnight-explorer.exe"),
            ExecutableRole::Launch,
            &before,
            &after,
            3,
        )
        .unwrap_err();
        assert!(error.0.contains(expected_fields), "{}", error.0);
    }
}

#[test]
fn executable_snapshot_diagnostics_reports_actual_bytes_when_metadata_is_unchanged() {
    let before = crate::native::test_support::snapshot(3, 7, [0; 16], 11, 13);
    for (bytes_read, expected_counts) in [
        (2, "changed_fields=[]; bytes_read=2; expected_length=3"),
        (4, "changed_fields=[]; bytes_read=4; expected_length=3"),
    ] {
        let error = validate_executable_snapshot(
            Path::new(r"D:\launch\midnight-explorer.exe"),
            ExecutableRole::Launch,
            &before,
            &before,
            bytes_read,
        )
        .unwrap_err();
        assert!(error.0.contains(expected_counts), "{}", error.0);
    }
}

#[test]
fn executable_snapshot_diagnostics_accepts_identical_snapshots_and_expected_byte_counts() {
    for length in [0, 3] {
        let before = crate::native::test_support::snapshot(length, 7, [0; 16], 11, 13);
        for role in [ExecutableRole::Launch, ExecutableRole::RegisteredTarget] {
            assert!(
                validate_executable_snapshot(
                    Path::new(r"D:\launch\midnight-explorer.exe"),
                    role,
                    &before,
                    &before,
                    length,
                )
                .is_ok()
            );
        }
    }
}

#[test]
fn default_mode_requires_a_launcher_pointer_instead_of_hash_record_fallback() {
    let fixture = Fixture::refused();
    let error = resolve_at(
        &fixture.source,
        Some(&fixture.registrations),
        RuntimeVerification::Disabled,
    )
    .unwrap_err();
    assert_pointer_update(&error);
}

#[test]
fn runtime_verification_defaults_off_and_accepts_only_exact_native_zero_or_one() {
    assert_eq!(
        verification_from_env(None).unwrap(),
        RuntimeVerification::Disabled
    );
    for (value, expected) in [
        ("0", RuntimeVerification::Disabled),
        ("1", RuntimeVerification::Sha256),
    ] {
        assert_eq!(
            verification_from_env(Some(OsStr::new(value))).unwrap(),
            expected
        );
    }
    for value in [
        "", "true", "false", "on", "off", "01", "00", " 1", "1 ", "0\n", "2", "-1",
    ] {
        assert!(
            verification_from_env(Some(OsStr::new(value))).is_err(),
            "accepted {value:?}"
        );
    }
    for units in [&[0xd800][..], &[b'1' as u16, 0][..]] {
        let value = OsString::from_wide(units);
        assert!(verification_from_env(Some(&value)).is_err());
    }
}

#[test]
fn launcher_path_key_uses_exact_raw_utf16le_and_domain() {
    // Independent fixed vectors: .NET SHA256 over ASCII domain plus manually
    // appended low/high bytes, without a Unicode encoder replacing surrogates.
    for (units, expected) in [
        (
            &[][..],
            "368872ff925863ce8c190ad04c32ad34d77ca32b6e9c74a9e03348b67500e6f1",
        ),
        (
            &[0x0041, 0x0100][..],
            "3ea681ad8c92bfffeef682db8698a0baf8cee945f5f869859cc53a48ea3250aa",
        ),
        (
            &[0xd800][..],
            "0bfbac275185a9ebd79380a5353ff52f7eba09ff12f45681cb294003ed45ae95",
        ),
        (
            &[0xfffd][..],
            "fcb261100268a1565b1f5a9ffa0aba980a527573dbd11886d19bdca5dbff892b",
        ),
        (
            &[0xd800, 0x005c, 0xdc00][..],
            "6cf6c18f631bbec14dc2ad8d8a04d470f19058493dc9142c86e0df61234afc75",
        ),
    ] {
        assert_eq!(launch_path_key(units), expected);
    }
}

#[test]
fn disabled_pointer_selects_protected_runtime_without_matching_executable_bytes() {
    let fixture = Fixture::new();
    fixture.write_pointer();
    std::fs::write(
        &fixture.target,
        b"different registered runtime bytes and length",
    )
    .unwrap();
    assert_eq!(
        resolve_without_digest(&fixture.source, Some(&fixture.registrations)).unwrap(),
        fixture.target.canonicalize().unwrap()
    );
}

#[test]
fn disabled_fallbacks_never_digest_executables() {
    let fixture = Fixture::refused();
    let missing = fixture._root.path().join("missing-registrations");
    for root in [
        None,
        Some(fixture.registrations.as_path()),
        Some(missing.as_path()),
    ] {
        assert_pointer_update(&resolve_without_digest(&fixture.source, root).unwrap_err());
    }
    std::fs::remove_dir(fixture.source.parent().unwrap().join("viewers")).unwrap();
    for root in [
        None,
        Some(fixture.registrations.as_path()),
        Some(missing.as_path()),
    ] {
        assert_eq!(
            resolve_without_digest(&fixture.source, root).unwrap(),
            fixture.source.canonicalize().unwrap()
        );
    }
    std::fs::write(fixture.record(), b"malformed registration").unwrap();
    assert_eq!(
        resolve_without_digest(&fixture.target, Some(&fixture.registrations)).unwrap(),
        fixture.target.canonicalize().unwrap()
    );
}

#[test]
fn enabled_mode_hashes_both_executables_and_rejects_changed_target() {
    for (bytes, matching) in [(b"abc", true), (b"abd", false)] {
        let fixture = Fixture::new();
        let pointer = fixture.write_pointer();
        std::fs::write(pointer, b"strict mode does not consume launcher pointers").unwrap();
        std::fs::write(&fixture.target, bytes).unwrap();
        let mut calls = Vec::new();
        let result = resolve_at_with_digest(
            &fixture.source,
            Some(&fixture.registrations),
            RuntimeVerification::Sha256,
            |path, role| {
                calls.push(role);
                executable_digest(path, role)
            },
        );
        assert_eq!(calls.len(), 2);
        assert!(matches!(calls[0], ExecutableRole::Launch));
        assert!(matches!(calls[1], ExecutableRole::RegisteredTarget));
        if matching {
            assert_eq!(result.unwrap(), fixture.target.canonicalize().unwrap());
        } else {
            assert!(result.unwrap_err().0.contains("does not match"));
        }
    }
}

#[test]
fn disabled_mode_requires_fresh_launch_length_and_last_write_time() {
    for modified_only in [false, true] {
        let fixture = Fixture::refused();
        fixture.write_pointer();
        let before = std::fs::metadata(&fixture.source).unwrap();
        if modified_only {
            let file = OpenOptions::new()
                .write(true)
                .open(&fixture.source)
                .unwrap();
            file.set_modified(before.modified().unwrap() + std::time::Duration::from_secs(2))
                .unwrap();
            drop(file);
            let after = std::fs::metadata(&fixture.source).unwrap();
            assert_eq!(after.file_size(), before.file_size());
            assert_ne!(after.last_write_time(), before.last_write_time());
        } else {
            std::fs::write(&fixture.source, b"changed length").unwrap();
            assert_ne!(
                std::fs::metadata(&fixture.source).unwrap().file_size(),
                before.file_size()
            );
        }
        assert_pointer_update(
            &resolve_without_digest(&fixture.source, Some(&fixture.registrations)).unwrap_err(),
        );
    }
}

#[test]
fn launcher_pointer_is_strict_bounded_and_rejects_invalid_fields() {
    let fixture = Fixture::refused();
    let pointer = fixture.write_pointer();
    let valid = std::fs::read_to_string(&pointer).unwrap();
    let launch = launch_path::open(&fixture.source).unwrap();
    let path_field = format!("launch_executable_utf16={:?}", launch.utf16);
    let length_field = format!("launch_length={}", launch.length);
    let time_field = format!(
        "launch_last_write_time_100ns={}",
        launch.last_write_time_100ns
    );
    let mut invalid = vec![
        "not TOML".to_owned(),
        valid.replace("schema_version=1", "schema_version=2"),
        format!("{valid}unknown=1\n"),
        format!("{valid}schema_version=1\n"),
        valid.replace(&length_field, "launch_length=-1"),
        valid.replace(&length_field, "launch_length=\"3\""),
        valid.replace(
            &time_field,
            "launch_last_write_time_100ns=9223372036854775808",
        ),
        valid.replace(&path_field, ""),
    ];
    for encoded in ["[]", "[0]", "[65536]", "[-1]", "[109,101]"] {
        invalid.push(valid.replace(&path_field, &format!("launch_executable_utf16={encoded}")));
    }
    let other: Vec<_> = fixture.target.as_os_str().encode_wide().collect();
    invalid.push(valid.replace(&path_field, &format!("launch_executable_utf16={other:?}")));
    for text in invalid {
        std::fs::write(&pointer, &text).unwrap();
        assert!(
            resolve_without_digest(&fixture.source, Some(&fixture.registrations)).is_err(),
            "accepted launcher pointer: {text}"
        );
    }
    std::fs::write(&pointer, vec![b' '; MAX_REGISTRATION_BYTES as usize + 1]).unwrap();
    assert!(
        resolve_without_digest(&fixture.source, Some(&fixture.registrations))
            .unwrap_err()
            .0
            .contains("exceeds 1 MiB")
    );
}

#[test]
fn launcher_pointer_hash_must_be_lowercase_hex_before_record_selection() {
    let fixture = Fixture::new();
    let pointer = fixture.write_pointer();
    let valid = std::fs::read_to_string(&pointer).unwrap();
    let target: Vec<_> = fixture.target.as_os_str().encode_wide().collect();
    for hash in [
        String::new(),
        DIGEST.to_ascii_uppercase(),
        "g".repeat(64),
        "0".repeat(63),
        "0".repeat(65),
    ] {
        // A matching protected record prevents a missing-file error from hiding
        // acceptance of an invalid pointer hash.
        std::fs::write(
            fixture.registrations.join(format!("{hash}.toml")),
            format!("schema_version=1\nsha256=\"{hash}\"\nexecutable_utf16={target:?}\n"),
        )
        .unwrap();
        std::fs::write(&pointer, valid.replace(DIGEST, &hash)).unwrap();
        assert!(resolve_without_digest(&fixture.source, Some(&fixture.registrations)).is_err());
    }
    let hash = "0".repeat(64);
    std::fs::write(
        fixture.registrations.join(format!("{hash}.toml")),
        format!("schema_version=1\nsha256=\"{hash}\"\nexecutable_utf16={target:?}\n"),
    )
    .unwrap();
    std::fs::write(&pointer, valid.replace(DIGEST, &hash)).unwrap();
    assert_eq!(
        resolve_without_digest(&fixture.source, Some(&fixture.registrations)).unwrap(),
        fixture.target.canonicalize().unwrap()
    );
}

#[test]
fn disabled_pointer_registration_keeps_legacy_record_validation() {
    let fixture = Fixture::refused();
    fixture.write_pointer();
    let valid = std::fs::read_to_string(fixture.record()).unwrap();
    std::fs::remove_file(fixture.record()).unwrap();
    assert_pointer_update(
        &resolve_without_digest(&fixture.source, Some(&fixture.registrations)).unwrap_err(),
    );
    for text in [
        "not TOML".to_owned(),
        valid.replace("schema_version=1", "schema_version=2"),
        valid.replace(DIGEST, &"0".repeat(64)),
        format!("{valid}unknown=1\n"),
    ] {
        std::fs::write(fixture.record(), text).unwrap();
        assert!(resolve_without_digest(&fixture.source, Some(&fixture.registrations)).is_err());
    }
    std::fs::write(
        fixture.record(),
        vec![b' '; MAX_REGISTRATION_BYTES as usize + 1],
    )
    .unwrap();
    assert!(
        resolve_without_digest(&fixture.source, Some(&fixture.registrations))
            .unwrap_err()
            .0
            .contains("exceeds 1 MiB")
    );
}

#[test]
fn disabled_mode_rejects_unsafe_registration_pointer_and_target() {
    for component in [
        "registration root",
        "launchers",
        "pointer",
        "record",
        "target",
        "target parent",
    ] {
        let fixture = Fixture::new();
        let pointer = fixture.write_pointer();
        for path in [&pointer, &fixture.record(), &fixture.target] {
            crate::trust::protect_fixture(path).unwrap();
        }
        let path = match component {
            "registration root" => fixture.registrations.clone(),
            "launchers" => fixture.registrations.join("launchers"),
            "pointer" => pointer,
            "record" => fixture.record(),
            "target" => fixture.target.clone(),
            "target parent" => fixture.target.parent().unwrap().to_owned(),
            _ => unreachable!(),
        };
        crate::trust::fixture_acl(&path, "D:P(A;OICI;FA;;;WD)").unwrap();
        let error =
            resolve_without_digest(&fixture.source, Some(&fixture.registrations)).unwrap_err();
        assert!(
            error.0.contains("writable or replaceable"),
            "{component}: {}",
            error.0
        );
    }
}

#[test]
fn disabled_mode_rejects_reparse_paths_before_normalization() {
    for component in [
        "launch",
        "registration root",
        "launchers",
        "pointer",
        "record",
        "target parent",
    ] {
        let fixture = Fixture::new();
        let pointer = fixture.write_pointer();
        let redirect = fixture._root.path().join("redirect");
        let mut source = fixture.source.clone();
        let mut root = fixture.registrations.clone();
        let (link, target) = match component {
            "launch" => {
                source = redirect.join("midnight-explorer.exe");
                (
                    redirect.clone(),
                    fixture.source.parent().unwrap().to_owned(),
                )
            }
            "registration root" => {
                root = redirect.clone();
                (redirect.clone(), fixture.registrations.clone())
            }
            "launchers" => {
                let launchers = fixture.registrations.join("launchers");
                std::fs::rename(&launchers, &redirect).unwrap();
                (launchers, redirect.clone())
            }
            "pointer" => {
                std::fs::remove_file(&pointer).unwrap();
                (pointer, fixture.target.parent().unwrap().to_owned())
            }
            "record" => {
                std::fs::remove_file(fixture.record()).unwrap();
                (
                    fixture.record(),
                    fixture.target.parent().unwrap().to_owned(),
                )
            }
            "target parent" => {
                fixture.write_record(&redirect.join("midnight-explorer.exe"));
                (
                    redirect.clone(),
                    fixture.target.parent().unwrap().to_owned(),
                )
            }
            _ => unreachable!(),
        };
        junction(&link, &target);
        let result = resolve_without_digest(&source, Some(&root));
        std::fs::remove_dir(&link).unwrap();
        let error = result.unwrap_err();
        assert!(
            error.0.contains("reparse point"),
            "{component}: {}",
            error.0
        );
        assert_eq!(std::fs::read(&fixture.target).unwrap(), b"abc");
    }
}

#[test]
fn launcher_path_aliases_use_one_native_normalized_key() {
    let fixture = Fixture::new();
    fixture.write_pointer();
    let expected = launch_path::open(&fixture.source).unwrap();
    let verbatim = fixture.source.canonicalize().unwrap();
    let units: Vec<_> = verbatim.as_os_str().encode_wide().collect();
    let dos_units = match verbatim.components().next() {
        Some(Component::Prefix(prefix)) => match prefix.kind() {
            Prefix::VerbatimDisk(_) => units[4..].to_vec(),
            Prefix::VerbatimUNC(_, _) => [vec![92, 92], units[8..].to_vec()].concat(),
            _ => panic!("fixture must expose a native DOS or UNC path"),
        },
        _ => panic!("fixture must expose an absolute native path"),
    };
    for alias in [
        PathBuf::from(OsString::from_wide(&dos_units)),
        verbatim,
        fixture.source.with_file_name("MIDNIGHT-EXPLORER.EXE"),
    ] {
        let native = launch_path::open(&alias).unwrap();
        assert_eq!(native.executable, expected.executable);
        assert_eq!(native.utf16, expected.utf16);
        assert_eq!(
            launch_path_key(&native.utf16),
            launch_path_key(&expected.utf16)
        );
        assert_eq!(
            resolve_without_digest(&alias, Some(&fixture.registrations)).unwrap(),
            fixture.target.canonicalize().unwrap()
        );
    }
}

#[test]
fn launcher_pointer_preserves_long_paths_and_raw_utf16() {
    let mut fixture = Fixture::new();
    let mut name: Vec<_> = "český-日本語-😀-".encode_utf16().collect();
    name.push(0xd800);
    name.extend(".exe".encode_utf16());
    let name = OsString::from_wide(&name);
    let mut directory = runtime_path(fixture.source.parent().unwrap()).unwrap();
    while directory.as_os_str().encode_wide().count() <= 320 {
        directory.push("long-launch-component");
    }
    std::fs::create_dir_all(&directory).unwrap();
    let source = directory.join(&name);
    std::fs::rename(&fixture.source, &source).unwrap();
    fixture.source = source;
    let target = fixture.target.parent().unwrap().join(&name);
    std::fs::rename(&fixture.target, &target).unwrap();
    fixture.target = target;
    fixture.write_record(&fixture.target);
    fixture.write_pointer();
    let native = launch_path::open(&fixture.source).unwrap();
    assert!(native.utf16.len() > 260);
    assert!(native.utf16.contains(&0xd800));
    assert_eq!(
        resolve_without_digest(&fixture.source, Some(&fixture.registrations)).unwrap(),
        fixture.target.canonicalize().unwrap()
    );
}

#[test]
fn disabled_mode_needs_no_executable_read_data_permission() {
    let fixture = Fixture::new();
    fixture.write_pointer();
    let before = std::fs::metadata(&fixture.source).unwrap();
    crate::trust::fixture_acl(&fixture.source, "D:P(D;;0x1;;;WD)(A;;FA;;;WD)").unwrap();
    let after = std::fs::metadata(&fixture.source).unwrap();
    assert_eq!(before.file_size(), after.file_size());
    assert_eq!(before.last_write_time(), after.last_write_time());
    assert_eq!(
        OpenOptions::new()
            .read(true)
            .open(&fixture.source)
            .unwrap_err()
            .kind(),
        io::ErrorKind::PermissionDenied
    );
    assert_eq!(
        resolve_without_digest(&fixture.source, Some(&fixture.registrations)).unwrap(),
        fixture.target.canonicalize().unwrap()
    );
    let mut calls = 0;
    let strict = resolve_at_with_digest(
        &fixture.source,
        Some(&fixture.registrations),
        RuntimeVerification::Sha256,
        |path, role| {
            calls += 1;
            executable_digest(path, role)
        },
    );
    assert_eq!(calls, 1);
    assert!(strict.is_err());
}

#[test]
fn installation_prefix_normalization_preserves_native_utf16_without_resolution() {
    let input = PathBuf::from(OsString::from_wide(&[
        67, 58, 92, 0xd800, 92, 109, 101, 46, 101, 120, 101,
    ]));
    let output = installation_path(&input).unwrap();
    let expected: Vec<_> = "\\\\?\\"
        .encode_utf16()
        .chain(input.as_os_str().encode_wide())
        .collect();
    assert_eq!(
        output.as_os_str().encode_wide().collect::<Vec<_>>(),
        expected
    );
    assert_eq!(installation_path(&output).unwrap(), output);
    assert_eq!(runtime_path(&input).unwrap(), output);
    assert_eq!(
        installation_path(Path::new(r"\\server\share\midnight-explorer.exe")).unwrap(),
        PathBuf::from(r"\\?\UNC\server\share\midnight-explorer.exe")
    );
    assert!(installation_path(Path::new(r"\\.\pipe\midnight")).is_err());
}

#[test]
fn recorded_runtime_paths_are_never_resolved_against_the_current_directory() {
    for recorded in [
        r"relative\midnight-explorer.exe",
        r"C:\runtime\..\midnight-explorer.exe",
    ] {
        assert!(runtime_path(Path::new(recorded)).is_err(), "{recorded}");
        // A host's own executable path may be relative; it resolves without following links.
        let resolved = installation_path(Path::new(recorded)).unwrap();
        assert!(
            resolved.is_absolute() && resolved.as_os_str().to_string_lossy().starts_with(r"\\?\"),
            "{resolved:?}"
        );
    }
}
