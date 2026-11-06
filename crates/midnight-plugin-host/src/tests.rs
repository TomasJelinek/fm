use super::*;
static NATIVE_SESSIONS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn fixture_root() -> tempfile::TempDir {
    let root = match std::env::var_os("HOST_TEST_INSTALL_ROOT") {
        Some(path) => tempfile::tempdir_in(path).unwrap(),
        None => tempfile::tempdir().unwrap(),
    };
    #[cfg(windows)]
    crate::trust::protect_fixture(root.path()).unwrap();
    root
}

fn copy_library(source: impl AsRef<std::path::Path>, target: impl AsRef<std::path::Path>) {
    std::fs::copy(source, target.as_ref()).unwrap();
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::fs::PermissionsExt;
        // The fixture installation is private even when artifacts come from a drvfs share.
        std::fs::set_permissions(target, std::fs::Permissions::from_mode(0o600)).unwrap();
    }
}

#[test]
#[ignore = "requires HOST_TEST_PLUGIN pointing to separately built basic DLL/SO"]
fn validated_plugin_code_stays_pinned_after_last_session_reference() {
    let library = std::env::var_os("HOST_TEST_PLUGIN").expect("HOST_TEST_PLUGIN");
    let root = fixture_root();
    let installed = package(root.path(), "basic", "midnight.viewer.basic", "");
    copy_library(library, installed.join("fixture.dll"));
    let runtime = Runtime::discover(root.path()).unwrap();
    assert!(!runtime.is_loaded("midnight.viewer.basic"));
    let loaded = crate::loaded::Loaded::acquire(&runtime.catalog()[0]).unwrap();
    assert!(runtime.is_loaded("midnight.viewer.basic"));
    let weak = std::sync::Arc::downgrade(&loaded);
    drop(loaded);
    assert!(
        weak.upgrade().is_some(),
        "native code was unloaded before process exit"
    );
}

#[test]
fn absent_viewers_directory_is_an_empty_catalog() {
    let root = fixture_root();
    assert!(Runtime::discover(root.path()).is_ok());
}

fn package(root: &std::path::Path, name: &str, id: &str, extra: &str) -> std::path::PathBuf {
    let package = root.join("viewers").join(name);
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("fixture.dll"), b"not loaded by discovery").unwrap();
    let models = if extra.contains("\"document\"") {
        "[\"document-blocks.v1\"]"
    } else if extra.contains("\"binary\"") {
        "[\"text-page.v1\",\"binary-page.v1\"]"
    } else {
        "[\"text-page.v1\"]"
    };
    std::fs::write(package.join("plugin.toml"), format!(
        "schema_version=1\nid=\"{id}\"\nkind=\"viewer\"\nversion=\"0.1.0\"\nentry_library=\"fixture.dll\"\ntarget=\"{}\"\nabi_major=1\nminimum_host_minor=0\nmodels={models}\n{extra}", discovery::host_target()
    )).unwrap();
    package
}

#[test]
fn discovery_rejects_both_duplicate_ids_and_keeps_independent_packages() {
    let root = fixture_root();
    package(root.path(), "one", "viewer.duplicate", "");
    package(root.path(), "two", "viewer.duplicate", "");
    package(root.path(), "three", "viewer.unique", "");
    let runtime = Runtime::discover(root.path()).unwrap();
    assert_eq!(runtime.catalog().len(), 1);
    assert_eq!(runtime.catalog()[0].id, "viewer.unique");
    assert_eq!(runtime.diagnostics().len(), 2);
}

#[test]
fn discovery_rejects_escape_abi_target_and_required_service() {
    for (from, to) in [
        ("fixture.dll", "../fixture.dll"),
        ("abi_major=1", "abi_major=2"),
        (
            "models=[\"text-page.v1\"]",
            "models=[\"binary-page.v1\"]\nmodes=[\"text\"]",
        ),
        (discovery::host_target(), "wrong-target"),
        (
            "models=",
            "required_host_services=[\"execute.shell\"]\nmodels=",
        ),
    ] {
        let root = fixture_root();
        let path = package(root.path(), "bad", "viewer.bad", "").join("plugin.toml");
        let text = std::fs::read_to_string(&path).unwrap().replace(from, to);
        std::fs::write(path, text).unwrap();
        let runtime = Runtime::discover(root.path()).unwrap();
        assert!(runtime.catalog().is_empty(), "accepted {to}");
        assert_eq!(runtime.diagnostics().len(), 1);
    }
}

#[test]
fn manifest_removal_removes_capability_and_routing_respects_modes() {
    let root = fixture_root();
    package(
        root.path(),
        "basic",
        "viewer.basic",
        "modes=[\"text\",\"binary\"]",
    );
    let markdown = package(
        root.path(),
        "markdown",
        "viewer.markdown",
        "modes=[\"document\"]\nextensions=[\"md\"]",
    );
    let runtime = Runtime::discover(root.path()).unwrap();
    assert_eq!(
        runtime
            .choose(std::path::Path::new("doc.md"), false)
            .unwrap()
            .0
            .id,
        "viewer.markdown"
    );
    assert_eq!(
        runtime
            .choose(std::path::Path::new("doc.md"), true)
            .unwrap()
            .0
            .id,
        "viewer.basic"
    );
    assert_eq!(
        runtime
            .choose_enabled(
                std::path::Path::new("doc.md"),
                false,
                &["viewer.markdown".into()]
            )
            .unwrap()
            .0
            .id,
        "viewer.basic"
    );
    assert!(
        runtime
            .choose_enabled(
                std::path::Path::new("doc.md"),
                true,
                &["viewer.basic".into()]
            )
            .is_none()
    );
    std::fs::remove_file(markdown.join("plugin.toml")).unwrap();
    assert_eq!(Runtime::discover(root.path()).unwrap().catalog().len(), 1);
}

#[test]
fn captured_file_detects_replacement_and_in_place_changes() {
    let root = fixture_root();
    let path = root.path().join("Čeština [literal].txt");
    std::fs::write(&path, b"original").unwrap();
    let capture = CapturedFile::capture(&path).unwrap();
    let mut reader = capture.open().unwrap();
    assert_eq!(reader.read_at(0, 8).unwrap(), b"original");
    std::fs::write(&path, b"different content").unwrap();
    assert!(reader.read_at(0, 8).is_err());
    assert!(capture.open().is_err());
    drop(reader);
    std::fs::rename(&path, root.path().join("old")).unwrap();
    std::fs::write(&path, b"original").unwrap();
    assert!(capture.open().is_err());
}

#[test]
fn native_broker_rejects_directories_and_preserves_large_offsets() {
    let root = fixture_root();
    assert!(CapturedFile::capture(root.path()).is_err());
    let path = root.path().join("sparse.bin");
    let mut file = std::fs::File::create(&path).unwrap();
    use std::io::{Seek, SeekFrom, Write};
    file.seek(SeekFrom::Start((1 << 32) + 11)).unwrap();
    file.write_all(b"bytes").unwrap();
    drop(file);
    let capture = CapturedFile::capture(&path).unwrap();
    assert_eq!(
        capture.open().unwrap().read_at((1 << 32) + 11, 5).unwrap(),
        b"bytes"
    );
    assert!(capture.open().unwrap().read_at(u64::MAX, 2).is_err());
}

#[test]
#[ignore = "requires HOST_TEST_PLUGIN pointing to the separately built basic DLL/SO"]
fn real_plugin_loads_reads_cancels_and_quiesces() {
    let _guard = NATIVE_SESSIONS.lock().unwrap();
    use std::time::{Duration, Instant};
    let library = std::env::var_os("HOST_TEST_PLUGIN").expect("HOST_TEST_PLUGIN");
    let root = fixture_root();
    let installed = package(
        root.path(),
        "basic",
        "midnight.viewer.basic",
        "modes=[\"text\",\"binary\"]",
    );
    copy_library(library, installed.join("fixture.dll"));
    let path = root.path().join("input.txt");
    std::fs::write(&path, "first line\r\nČeština 中文 😀\nthird line\n").unwrap();
    let runtime = Runtime::discover(root.path()).unwrap();
    let session = runtime
        .open(
            "midnight.viewer.basic",
            CapturedFile::capture(&path).unwrap(),
            42,
            api::viewer::MODE_TEXT,
        )
        .unwrap();
    let limit = Instant::now() + Duration::from_secs(10);
    let mut opened = false;
    while !opened && Instant::now() < limit {
        opened = session
            .drain(64)
            .iter()
            .any(|e| matches!(e.kind, EventKind::Opened(_)));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(opened, "plugin did not open");
    let request = session
        .viewport(api::messages::Viewport {
            source_anchor: 0,
            backwards: false,
            max_bytes: 4096,
            max_lines: 32,
        })
        .unwrap();
    let mut text = None;
    while text.is_none() && Instant::now() < limit {
        for event in session.drain(64) {
            assert_eq!(event.generation, 42);
            if let EventKind::TextPage(page) = event.kind {
                assert_eq!(event.request_id, request);
                text = Some(page.text);
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(text.expect("text page").contains("Čeština 中文 😀"));
    let find = session
        .search(api::messages::Search {
            query: "Čeština".into(),
            source_anchor: 0,
            backwards: false,
            case_sensitive: true,
        })
        .unwrap();
    let mut found = false;
    while !found && Instant::now() < limit {
        found = session.drain(64).iter().any(|event| {
            event.request_id == find && matches!(event.kind, EventKind::SearchMatch(_))
        });
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(found, "real plugin search result");
    std::fs::write(&path, b"changed native revision").unwrap();
    let changed = session
        .viewport(api::messages::Viewport {
            source_anchor: 0,
            backwards: false,
            max_bytes: 4096,
            max_lines: 32,
        })
        .unwrap();
    let mut rejected = false;
    while !rejected && Instant::now() < limit {
        rejected = session
            .drain(64)
            .iter()
            .any(|event| event.request_id == changed && matches!(event.kind, EventKind::Failed(_)));
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(rejected, "read broker must reject changed revision");
    let search = session
        .search(api::messages::Search {
            query: "absent".into(),
            source_anchor: 0,
            backwards: false,
            case_sensitive: true,
        })
        .unwrap();
    session.cancel(search);
    session.close();
    while !session.is_closed() && Instant::now() < limit {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(session.is_closed(), "library did not quiesce");
}

#[test]
#[ignore = "requires HOST_TEST_PLUGIN pointing to separately built basic DLL/SO"]
fn real_plugin_descriptor_identity_mismatch_is_refused_before_create() {
    let library = std::env::var_os("HOST_TEST_PLUGIN").expect("HOST_TEST_PLUGIN");
    let root = fixture_root();
    let installed = package(root.path(), "mismatch", "incorrect.viewer.id", "");
    copy_library(library, installed.join("fixture.dll"));
    let runtime = Runtime::discover(root.path()).unwrap();
    let error = crate::loaded::Loaded::open(&runtime.catalog()[0])
        .err()
        .expect("must reject descriptor mismatch");
    assert!(error.0.contains("differs from manifest"));
}

#[test]
#[ignore = "requires HOST_BAD_PLUGIN pointing to generic wrong-ABI DLL/SO fixture"]
fn native_loader_refuses_wrong_abi_descriptor() {
    let library = std::env::var_os("HOST_BAD_PLUGIN").expect("HOST_BAD_PLUGIN");
    let root = fixture_root();
    let installed = package(root.path(), "bad", "viewer.bad", "");
    copy_library(library, installed.join("fixture.dll"));
    let runtime = Runtime::discover(root.path()).unwrap();
    let error = crate::loaded::Loaded::open(&runtime.catalog()[0])
        .err()
        .expect("must reject wrong ABI");
    assert!(error.0.contains("Invalid ABI descriptor"));
}

#[test]
#[ignore = "requires HOST_TEST_PLUGIN pointing to separately built basic DLL/SO"]
fn session_quota_is_process_wide_and_closed_sessions_release_retained_output() {
    let _guard = NATIVE_SESSIONS.lock().unwrap();
    let library = std::env::var_os("HOST_TEST_PLUGIN").expect("HOST_TEST_PLUGIN");
    let root = fixture_root();
    let installed = package(root.path(), "basic", "midnight.viewer.basic", "");
    copy_library(library, installed.join("fixture.dll"));
    let path = root.path().join("input.txt");
    std::fs::write(&path, b"sample").unwrap();
    let captured = CapturedFile::capture(&path).unwrap();
    let runtime = Runtime::discover(root.path()).unwrap();
    let other = Runtime::discover(root.path()).unwrap();
    let sessions: Vec<_> = (0..4)
        .map(|generation| {
            runtime
                .open(
                    "midnight.viewer.basic",
                    captured.clone(),
                    generation,
                    api::viewer::MODE_TEXT,
                )
                .unwrap()
        })
        .collect();
    let extra = other.open("midnight.viewer.basic", captured, 5, api::viewer::MODE_TEXT);
    let rejected = extra.is_err();
    drop(extra);
    std::thread::sleep(std::time::Duration::from_millis(30));
    for session in &sessions {
        session.close();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while sessions.iter().any(|session| !session.is_closed())
        && std::time::Instant::now() < deadline
    {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(rejected, "another Runtime bypassed aggregate quota");
    for session in sessions {
        assert!(session.is_closed());
        assert!(
            session.drain(64).is_empty(),
            "closed session retains output after its quota was released"
        );
    }
}
