use super::*;
use std::{
    fs,
    os::unix::{
        ffi::OsStrExt,
        fs::{PermissionsExt, symlink},
    },
    path::PathBuf,
};

struct Fixture {
    _directory: tempfile::TempDir,
    applications: PathBuf,
    bin: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let applications = directory.path().join("applications");
        let bin = directory.path().join("bin");
        fs::create_dir(&applications).unwrap();
        fs::create_dir(&bin).unwrap();
        Self {
            _directory: directory,
            applications,
            bin,
        }
    }
    fn program(&self, name: impl AsRef<Path>) -> PathBuf {
        let path = self.bin.join(name);
        // Invalid executable contents prove lookup never attempts to run the file.
        fs::write(&path, b"icon metadata fixture, not executable code").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        path
    }
    fn desktop(&self, name: &str, keys: &str) {
        fs::write(
            self.applications.join(format!("{name}.desktop")),
            format!("[Desktop Entry]\nType=Application\nName=Fixture\n{keys}\n"),
        )
        .unwrap();
    }
    fn lookup(&self, executable: &Path) -> Option<String> {
        icon_name(
            executable,
            &[&self.applications],
            self.bin.as_os_str(),
            &Cancellation::default(),
        )
    }
}

#[test]
fn absolute_and_bare_exec_match_the_exact_configured_executable() {
    let fixture = Fixture::new();
    let program = fixture.program("editor");
    fixture.desktop(
        "editor",
        &format!("Exec={} %F\nIcon=editor-host", program.display()),
    );
    assert_eq!(fixture.lookup(&program).as_deref(), Some("editor-host"));
    fixture.desktop(
        "editor",
        "Exec=editor --reuse-window %F\nTryExec=editor\nIcon=editor-host",
    );
    assert_eq!(fixture.lookup(&program).as_deref(), Some("editor-host"));
}

#[test]
fn quoted_unicode_path_and_canonical_alias_keep_exact_identity() {
    let fixture = Fixture::new();
    let program = fixture.program("東京 editor");
    let alias = fixture.bin.join("alias");
    symlink(&program, &alias).unwrap();
    fixture.desktop(
        "unicode",
        &format!("Exec=\"{}\" %f\nIcon=unicode-editor", program.display()),
    );
    assert_eq!(fixture.lookup(&alias).as_deref(), Some("unicode-editor"));
}

#[test]
fn basename_collision_does_not_borrow_another_programs_icon() {
    let fixture = Fixture::new();
    let program = fixture.program("editor");
    let other = fixture._directory.path().join("other");
    fs::create_dir(&other).unwrap();
    let other_program = other.join("editor");
    fs::write(&other_program, b"different").unwrap();
    fs::set_permissions(&other_program, fs::Permissions::from_mode(0o700)).unwrap();
    fixture.desktop(
        "other",
        &format!("Exec={}\nIcon=wrong-editor", other_program.display()),
    );
    assert!(fixture.lookup(&program).is_none());
}

#[test]
fn disagreeing_exact_matches_and_tryexec_are_refused() {
    let fixture = Fixture::new();
    let program = fixture.program("editor");
    fixture.program("other");
    fixture.desktop("first", "Exec=editor\nIcon=first");
    fixture.desktop("second", "Exec=editor\nIcon=second");
    assert!(fixture.lookup(&program).is_none());
    fs::remove_file(fixture.applications.join("second.desktop")).unwrap();
    fixture.desktop("first", "Exec=editor\nTryExec=other\nIcon=first");
    assert!(fixture.lookup(&program).is_none());
}

#[test]
fn malformed_hidden_and_non_theme_resources_are_not_interpreted() {
    let fixture = Fixture::new();
    let program = fixture.program("editor");
    for keys in [
        "Exec=editor\nIcon=../outside",
        "Exec=editor\nIcon=/tmp/outside.png",
        "Exec=editor\nIcon=host\nHidden=true",
        "Exec=editor\nExec=other\nIcon=host",
        "Exec=\"editor\nIcon=host",
        "Exec=$PROGRAM\nIcon=host",
        "Exec=editor;other\nIcon=host",
        "Exec=editor %Q\nIcon=host",
        "Exec=%k\nIcon=host",
        "TryExec=editor\nIcon=host",
    ] {
        fixture.desktop("entry", keys);
        assert!(fixture.lookup(&program).is_none(), "accepted {keys}");
    }
}

#[test]
fn native_non_unicode_paths_are_never_lossily_matched() {
    let fixture = Fixture::new();
    let program = fixture.program(Path::new(OsStr::from_bytes(b"editor-\xff")));
    fixture.program("editor-�");
    fixture.desktop("entry", "Exec=editor-�\nIcon=wrong");
    assert!(fixture.lookup(&program).is_none());
}

#[test]
fn metadata_symlink_escape_and_pre_cancelled_lookup_are_refused() {
    let fixture = Fixture::new();
    let program = fixture.program("editor");
    let outside = fixture.bin.join("outside.desktop");
    fs::write(
        &outside,
        "[Desktop Entry]\nType=Application\nExec=editor\nIcon=outside",
    )
    .unwrap();
    symlink(&outside, fixture.applications.join("escape.desktop")).unwrap();
    assert!(fixture.lookup(&program).is_none());
    fixture.desktop("valid", "Exec=editor\nIcon=valid");
    let cancel = Cancellation::default();
    cancel.cancel();
    assert!(
        icon_name(
            &program,
            &[&fixture.applications],
            fixture.bin.as_os_str(),
            &cancel
        )
        .is_none()
    );
}

#[test]
fn enumerating_beyond_the_metadata_budget_never_returns_a_partial_match() {
    let fixture = Fixture::new();
    let program = fixture.program("editor");
    fixture.desktop("valid", "Exec=editor\nIcon=valid");
    for index in 0..512 {
        fixture.desktop(&format!("other-{index}"), "Exec=other\nIcon=other");
    }
    assert!(fixture.lookup(&program).is_none());
}

#[test]
fn desktop_string_and_exec_escapes_are_data_not_shell_expansion() {
    for (input, expected) in [
        (
            r#""/opt/My\sEditor" --mode "literal\\\\path" %F"#,
            "/opt/My Editor",
        ),
        (r#""/opt/price\\$editor" %F"#, "/opt/price$editor"),
        (r#""/opt/100%%-editor" %F"#, "/opt/100%-editor"),
    ] {
        assert_eq!(
            parse_exec(&unescape(input).unwrap()).as_deref(),
            Some(expected)
        );
    }
    for invalid in [
        "editor '%f'",
        "editor \"%f\"",
        "editor \"unterminated",
        "editor %Q",
    ] {
        assert!(parse_exec(invalid).is_none(), "accepted {invalid}");
    }
    assert!(unescape(r"editor\q").is_none());
}

#[test]
fn file_and_aggregate_read_limits_do_not_accept_partial_metadata() {
    let fixture = Fixture::new();
    let program = fixture.program("editor");
    fixture.desktop("valid", "Exec=editor\nIcon=host");
    assert_eq!(fixture.lookup(&program).as_deref(), Some("host"));
    let padded = format!(
        "Exec=editor\nIcon=conflicting\n#{}",
        "x".repeat(MAX_FILE_BYTES)
    );
    fixture.desktop("too-large", &padded);
    assert!(fixture.lookup(&program).is_none());
    fs::remove_file(fixture.applications.join("too-large.desktop")).unwrap();
    fixture.desktop("valid", "Exec=editor\nIcon=host");
    // Each whole file is below the per-file cap; their aggregate exceeds 4 MiB.
    for index in 0..66 {
        fixture.desktop(
            &format!("padding-{index}"),
            &format!("#{}", "x".repeat(64_000)),
        );
    }
    assert!(fixture.lookup(&program).is_none());
}

#[test]
fn relative_search_entries_and_invalid_output_requests_keep_fallback() {
    let fixture = Fixture::new();
    let program = fixture.program("editor");
    fixture.desktop("valid", "Exec=editor\nIcon=host");
    assert!(
        icon_name(
            &program,
            &[&fixture.applications],
            OsStr::new(".:relative:"),
            &Cancellation::default()
        )
        .is_none()
    );
    assert!(
        icon_name(
            Path::new("editor"),
            &[&fixture.applications],
            fixture.bin.as_os_str(),
            &Cancellation::default()
        )
        .is_none()
    );
    assert!(load(None, &program, 0, &Cancellation::default()).is_none());
    assert!(
        super::super::load_named_icon(None, &["../outside"], 32, &Cancellation::default())
            .is_none()
    );
}

#[test]
fn an_existing_unenumerable_catalog_never_returns_another_roots_match() {
    let fixture = Fixture::new();
    let program = fixture.program("editor");
    fixture.desktop("valid", "Exec=editor\nIcon=host");
    assert_eq!(fixture.lookup(&program).as_deref(), Some("host"));
    // A regular file makes read_dir fail independently of test UID or chmod rules.
    let blocked = fixture._directory.path().join("blocked-applications");
    fs::write(&blocked, b"not a directory").unwrap();
    assert!(
        icon_name(
            &program,
            &[&fixture.applications, &blocked],
            fixture.bin.as_os_str(),
            &Cancellation::default()
        )
        .is_none()
    );
    fs::remove_file(&blocked).unwrap();
    symlink(&blocked, &blocked).unwrap();
    assert!(
        icon_name(
            &program,
            &[&fixture.applications, &blocked],
            fixture.bin.as_os_str(),
            &Cancellation::default()
        )
        .is_none(),
        "a catalog resolution error must not become an absent catalog"
    );
}
