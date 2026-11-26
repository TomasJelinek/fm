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
#[ignore = "requires HOST_TEST_PLUGIN pointing to separately built basic DLL/SO"]
fn concurrent_first_loads_register_one_pinned_library() {
    let library = std::env::var_os("HOST_TEST_PLUGIN").expect("HOST_TEST_PLUGIN");
    let root = fixture_root();
    let installed = package(root.path(), "concurrent", "midnight.viewer.basic", "");
    copy_library(library, installed.join("fixture.dll"));
    let runtime = Runtime::discover(root.path()).unwrap();
    let info = runtime.catalog()[0].clone();
    let start = std::sync::Arc::new(std::sync::Barrier::new(4));
    let loads: Vec<_> = (0..4)
        .map(|_| {
            let (info, start) = (info.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                crate::loaded::Loaded::acquire(&info).unwrap()
            })
        })
        .collect();
    let loaded: Vec<_> = loads.into_iter().map(|load| load.join().unwrap()).collect();
    // Unlocked opens may race; exactly one registered library is shared by all callers.
    assert!(
        loaded
            .windows(2)
            .all(|pair| std::sync::Arc::ptr_eq(&pair[0], &pair[1]))
    );
    assert!(runtime.is_loaded("midnight.viewer.basic"));
}

#[test]
fn absent_viewers_directory_is_an_empty_catalog() {
    let root = fixture_root();
    assert!(Runtime::discover(root.path()).is_ok());
}

#[test]
fn image_worker_packages_are_left_to_the_process_viewer_host() {
    let root = fixture_root();
    let package = root.path().join("viewers/image");
    std::fs::create_dir_all(&package).unwrap();
    std::fs::write(package.join("image-viewer.json"), "{}").unwrap();
    let runtime = Runtime::discover(root.path()).unwrap();
    assert!(runtime.catalog().is_empty());
    assert!(runtime.diagnostics().is_empty());
}

#[test]
fn web_worker_packages_are_left_to_their_validating_process_host() {
    let root = fixture_root();
    let package = root.path().join("viewers/html");
    std::fs::create_dir_all(&package).unwrap();
    // Validation belongs to the web host, including malformed manifests. Native
    // discovery must not invent a second missing-plugin.toml failure for it.
    std::fs::write(package.join("web-viewer.json"), "{}").unwrap();
    let runtime = Runtime::discover(root.path()).unwrap();
    assert!(runtime.catalog().is_empty());
    assert!(
        runtime.diagnostics().is_empty(),
        "{:?}",
        runtime.diagnostics()
    );
}

#[test]
fn mixed_viewer_transports_are_always_unavailable() {
    for manifests in [
        ["plugin.toml", "web-viewer.json"],
        ["image-viewer.json", "web-viewer.json"],
    ] {
        let root = fixture_root();
        let directory = package(root.path(), "mixed", "viewer.mixed", "");
        std::fs::remove_file(directory.join("plugin.toml")).unwrap();
        for manifest in manifests {
            if manifest == "plugin.toml" {
                package(root.path(), "mixed", "viewer.mixed", "");
            } else {
                std::fs::write(directory.join(manifest), "{}").unwrap();
            }
        }
        let runtime = Runtime::discover(root.path()).unwrap();
        assert!(runtime.catalog().is_empty(), "accepted {manifests:?}");
        assert_eq!(runtime.diagnostics().len(), 1, "{manifests:?}");
    }
}

#[test]
fn viewer_friendly_names_reject_controls_empty_and_oversized_metadata() {
    for name in [
        String::new(),
        "   ".into(),
        "unsafe\u{1b}name".into(),
        "x".repeat(129),
    ] {
        let root = fixture_root();
        // TOML literal strings preserve the control character for the parser's
        // validation; either parser or metadata validation must reject it.
        package(
            root.path(),
            "named",
            "viewer.named",
            &format!("name = '{name}'\n"),
        );
        let runtime = Runtime::discover(root.path()).unwrap();
        assert!(runtime.catalog().is_empty(), "accepted name {name:?}");
        assert_eq!(runtime.diagnostics().len(), 1);
    }
}

#[test]
fn packages_with_native_and_image_worker_entries_are_ambiguous() {
    let root = fixture_root();
    let package = package(root.path(), "ambiguous", "viewer.ambiguous", "");
    std::fs::write(package.join("image-viewer.json"), "{}").unwrap();
    let runtime = Runtime::discover(root.path()).unwrap();
    assert!(runtime.catalog().is_empty());
    assert_eq!(runtime.diagnostics().len(), 1);
    assert!(runtime.diagnostics()[0].message.contains("both"));
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

/// Other accounts' write access alone does not refuse a package (they could replace the
/// application as well); the installation reports it instead.
#[test]
fn discovery_accepts_shared_writable_sibling_libraries_and_reports_them() {
    for name in [
        "helper.dll",
        "HELPER.DlL",
        "libhelper.so",
        "libhelper.so.1",
        "lib/helper.bin",
    ] {
        let root = fixture_root();
        let directory = package(root.path(), "basic", "viewer.basic", "");
        let helper = directory.join(name);
        std::fs::create_dir_all(helper.parent().unwrap()).unwrap();
        std::fs::write(&helper, b"dependency fixture").unwrap();
        assert_eq!(Runtime::discover(root.path()).unwrap().catalog().len(), 1);
        assert!(installation_sharing(root.path()).unwrap().is_private());
        make_shared(&helper);
        let runtime = Runtime::discover(root.path()).unwrap();
        assert_eq!(runtime.catalog().len(), 1, "{:?}", runtime.diagnostics());
        let sharing = installation_sharing(root.path()).unwrap();
        assert_eq!(sharing.shared_count, 1, "{name}: {sharing:?}");
        assert!(sharing.shared[0].ends_with(name), "{sharing:?}");
    }
}

/// The first load validates the package again: a folder redirected out of the installation
/// after discovery is refused before any of its code is loaded.
#[test]
fn first_load_rechecks_dependency_redirection_after_discovery() {
    let root = fixture_root();
    let outside = fixture_root();
    let directory = package(root.path(), "basic", "viewer.basic", "");
    let lib = directory.join("lib");
    std::fs::create_dir(&lib).unwrap();
    std::fs::write(lib.join("helper.dll"), b"dependency fixture").unwrap();
    let runtime = Runtime::discover(root.path()).unwrap();
    assert_eq!(runtime.catalog().len(), 1);
    std::fs::remove_file(lib.join("helper.dll")).unwrap();
    std::fs::remove_dir(&lib).unwrap();
    #[cfg(windows)]
    super::tests_windows::junction(&lib, outside.path());
    #[cfg(unix)]
    std::os::unix::fs::symlink(outside.path(), &lib).unwrap();
    let error = match crate::loaded::Loaded::open(&runtime.catalog()[0]) {
        Ok(_) => panic!("loaded a redirected dependency folder"),
        Err(error) => error.to_string(),
    };
    #[cfg(windows)]
    std::fs::remove_dir(&lib).unwrap();
    assert!(
        error.contains("redirect"),
        "loader ran before dependency trust validation: {error}"
    );
}

#[cfg(target_os = "linux")]
use super::tests_linux::make_shared;
#[cfg(windows)]
use super::tests_windows::make_shared;

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

/// Discovery publishes the manifest modes and extensions that the viewer engine routes on (its
/// `routing_tests` build the catalog by hand); removing a manifest removes its capability.
#[test]
fn discovery_publishes_manifest_modes_and_extensions_until_removal() {
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
    let [basic, document] = runtime.catalog() else {
        panic!("unexpected catalog {:?}", runtime.catalog());
    };
    assert_eq!(basic.id, "viewer.basic");
    assert_eq!(
        basic.modes,
        [api::viewer::MODE_TEXT, api::viewer::MODE_BINARY]
    );
    assert!(basic.extensions.is_empty(), "{:?}", basic.extensions);
    assert_eq!(document.id, "viewer.markdown");
    assert_eq!(document.modes, [api::viewer::MODE_DOCUMENT]);
    assert_eq!(document.extensions, ["md"]);
    std::fs::remove_file(markdown.join("plugin.toml")).unwrap();
    let runtime = Runtime::discover(root.path()).unwrap();
    let ids: Vec<_> = runtime
        .catalog()
        .iter()
        .map(|info| info.id.as_str())
        .collect();
    assert_eq!(ids, ["viewer.basic"]);
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

/// Move only the change time: the read-only attribute and NTFS ChangeTime on Windows, the
/// mode and ctime on Linux. The in-process revision includes neither attributes nor mode.
fn move_change_time(path: &std::path::Path) {
    // Leave the clock past the write's timestamp, which may be as coarse as a tick.
    std::thread::sleep(std::time::Duration::from_millis(50));
    let writable = std::fs::metadata(path).unwrap().permissions();
    let mut read_only = writable.clone();
    read_only.set_readonly(true);
    std::fs::set_permissions(path, read_only).unwrap();
    std::fs::set_permissions(path, writable).unwrap();
}

/// VIEW-05: a viewer capture reads on after a change-time-only drift; an exact capture,
/// and every other change, still fail.
#[test]
fn a_viewer_capture_accepts_only_a_moved_change_time() {
    let root = tempfile::tempdir().unwrap();
    // No extension: endpoint protection on some machines rescans fresh `.txt` files.
    let path = root.path().join("fresh");
    std::fs::write(&path, b"original").unwrap();
    let exact = CapturedFile::capture(&path).unwrap();
    let viewer = CapturedFile::capture_with(&path, RevisionPolicy::AcceptChangeTimeOnly).unwrap();
    assert_eq!(viewer.stable_version(), exact.stable_version());
    let mut held = viewer.open().unwrap();
    move_change_time(&path);
    assert!(exact.revalidate().is_err(), "the change time did not move");
    assert!(exact.read_at(0, 8).is_err());
    viewer.revalidate().unwrap();
    assert_eq!(viewer.read_at(0, 8).unwrap(), b"original");
    // A reader opened before the drift checks before and after each read as well.
    assert_eq!(held.read_at(0, 8).unwrap(), b"original");
    // Same length, new write time.
    std::fs::write(&path, b"replaced").unwrap();
    assert!(viewer.revalidate().is_err());
    assert!(viewer.read_at(0, 8).is_err());
    assert!(held.read_at(0, 8).is_err());
    drop(held);
    // Another file under the same name, with the same length and write time.
    let viewer = CapturedFile::capture_with(&path, RevisionPolicy::AcceptChangeTimeOnly).unwrap();
    let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
    std::fs::rename(&path, root.path().join("renamed")).unwrap();
    std::fs::write(&path, b"replaced").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(modified)
        .unwrap();
    assert!(viewer.revalidate().is_err());
}

#[test]
fn native_broker_rejects_directories_and_preserves_large_offsets() {
    let root = fixture_root();
    assert!(CapturedFile::capture(root.path()).is_err());
    let path = root.path().join("sparse.bin");
    let mut file = std::fs::File::create(&path).unwrap();
    // Linux files are sparse by default; NTFS would otherwise zero-fill 4 GiB.
    #[cfg(windows)]
    super::tests_windows::sparse(&file);
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
    assert!(
        drain_until(&session, limit, |e| matches!(e.kind, EventKind::Opened(_))),
        "plugin did not open"
    );
    let request = session
        .viewport(api::messages::Viewport {
            source_anchor: 0,
            backwards: false,
            max_bytes: 4096,
            max_lines: 32,
        })
        .unwrap();
    let mut text = None;
    drain_until(&session, limit, |event| {
        assert_eq!(event.generation, 42);
        if let EventKind::TextPage(page) = event.kind {
            assert_eq!(event.request_id, request);
            text = Some(page.text);
            return true;
        }
        false
    });
    assert!(text.expect("text page").contains("Čeština 中文 😀"));
    let find = session
        .search(api::messages::Search {
            query: "Čeština".into(),
            source_anchor: 0,
            backwards: false,
            case_sensitive: true,
        })
        .unwrap();
    assert!(
        drain_until(&session, limit, |event| {
            event.request_id == find && matches!(event.kind, EventKind::SearchMatch(_))
        }),
        "real plugin search result"
    );
    std::fs::write(&path, b"changed native revision").unwrap();
    let changed = session
        .viewport(api::messages::Viewport {
            source_anchor: 0,
            backwards: false,
            max_bytes: 4096,
            max_lines: 32,
        })
        .unwrap();
    assert!(
        drain_until(&session, limit, |event| {
            event.request_id == changed && matches!(event.kind, EventKind::Failed(_))
        }),
        "read broker must reject changed revision"
    );
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
    assert!(closed_by(&[session], limit), "library did not quiesce");
}

/// Drain until `accept` takes an event, woken by the session's ready notification
/// rather than a sleep. Returns false only when the failure deadline elapses.
fn drain_until(
    session: &Session,
    deadline: std::time::Instant,
    mut accept: impl FnMut(HostEvent) -> bool,
) -> bool {
    use std::sync::mpsc::{TrySendError, sync_channel};
    let (ready, wake) = sync_channel(1);
    session.notify_on_ready(std::sync::Arc::new(move || {
        !matches!(ready.try_send(()), Err(TrySendError::Disconnected(_)))
    }));
    loop {
        let mut accepted = false;
        for event in session.drain(64) {
            accepted |= accept(event);
        }
        if accepted {
            return true;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() || wake.recv_timeout(remaining).is_err() {
            return false;
        }
    }
}

/// Wait until every session finished its backend cleanup, woken by the final ready
/// signal each session sends when its dispatcher ends rather than by polling. Returns
/// false only when the failure deadline elapses.
fn closed_by(sessions: &[Session], deadline: std::time::Instant) -> bool {
    use std::sync::mpsc::{TrySendError, sync_channel};
    let (ended, wake) = sync_channel(sessions.len().max(1));
    for session in sessions {
        let ended = ended.clone();
        // A full channel already holds a wake, after which every session is checked again.
        session.notify_on_ready(std::sync::Arc::new(move || {
            !matches!(ended.try_send(()), Err(TrySendError::Disconnected(_)))
        }));
    }
    while sessions.iter().any(|session| !session.is_closed()) {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        if remaining.is_zero() || wake.recv_timeout(remaining).is_err() {
            return false;
        }
    }
    true
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
fn sixteen_sessions_of_one_plugin_open_at_once_and_closed_sessions_release_retained_output() {
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
    // Sixteen text viewers of one plugin, through two runtimes: the host refuses none (it
    // refused the fifth). Sixteen is the published SDK's own instance limit per loaded
    // library (`api/midnight-plugin-sdk` `plugin::create`), which a 17th session still meets.
    let sessions: Vec<_> = (0..16)
        .map(|generation| {
            let owner = if generation % 2 == 0 {
                &runtime
            } else {
                &other
            };
            owner
                .open(
                    "midnight.viewer.basic",
                    captured.clone(),
                    generation,
                    api::viewer::MODE_TEXT,
                )
                .unwrap()
        })
        .collect();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    for session in &sessions {
        assert!(
            drain_until(session, deadline, |e| matches!(
                e.kind,
                EventKind::Opened(_)
            )),
            "every session opens"
        );
    }
    for session in &sessions {
        session.close();
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    assert!(closed_by(&sessions, deadline), "sessions did not quiesce");
    for session in sessions {
        assert!(session.is_closed());
        assert!(
            session.drain(64).is_empty(),
            "closed session retains output after it ended"
        );
    }
}
