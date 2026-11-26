use super::*;
#[path = "image_integration_tests.rs"]
mod images;
#[path = "pdf_navigation_tests.rs"]
mod pdf_navigation;
#[path = "pdf_scroll_probe_tests.rs"]
mod pdf_scroll_probe;
#[path = "source_position_tests.rs"]
mod source_position;
use midnight_core::{DirectoryIndex, Entry, EntryId, EntryKind, ViewerFrame};
use std::{
    path::Path,
    time::{Duration, Instant},
};

fn appearance() -> ViewerAppearance {
    ViewerAppearance {
        family: "monospace".into(),
        font_size: 14.0,
        foreground: [230, 230, 230, 255],
        background: [25, 25, 25, 255],
        selection: [60, 80, 130, 255],
        selection_text: [0; 4],
        accent: [100, 180, 240, 255],
    }
}

fn launch(root: &Path, names: &[&str], basic: bool) -> ViewerLaunch {
    let entries: Arc<[Entry]> = names
        .iter()
        .enumerate()
        .map(|(index, name)| Entry::new(EntryId(index as u64), root.join(name), EntryKind::File))
        .collect();
    ViewerLaunch {
        find: None,
        associations: Default::default(),
        tree: None,
        generated: None,
        preferences: Arc::new(Default::default()),
        archive: None,
        focused: entries[0].path.clone(),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
        basic,
        disabled_plugins: Arc::from([]),
    }
}

/// Deliver outputs until `accept` returns a value, waking on the mailbox's own signal.
/// The deadline elapses only when the expected output never arrives.
fn await_output<T>(
    session: &WindowSession,
    timeout: Duration,
    mut accept: impl FnMut(ViewerOutput) -> Option<T>,
) -> Option<T> {
    let deadline = Instant::now() + timeout;
    let (notify, notified) = std::sync::mpsc::sync_channel(1);
    session
        .mailbox
        .notify_on_ready(Arc::new(move || notify.try_send(()).is_ok()));
    loop {
        for output in session.mailbox.take() {
            if let Some(value) = accept(output) {
                return Some(value);
            }
        }
        notified
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .ok()?;
    }
}

#[track_caller]
fn frame(session: &WindowSession, predicate: impl Fn(&ViewerFrame) -> bool) -> ViewerFrame {
    let mut messages = Vec::new();
    await_output(session, Duration::from_secs(45), |output| match output {
        ViewerOutput::Frame(frame) if predicate(&frame) => Some(*frame),
        ViewerOutput::Status(status) => {
            messages.push(status);
            None
        }
        ViewerOutput::ImageUnavailable { message, .. } => {
            panic!("Image/document unavailable: {message}");
        }
        _ => None,
    })
    .unwrap_or_else(|| panic!("No expected frame: {messages:?}"))
}

#[track_caller]
fn status(session: &WindowSession, needles: &[&str]) {
    await_output(session, Duration::from_secs(10), |output| match output {
        ViewerOutput::Status(message) => {
            let message = message.to_lowercase();
            needles
                .iter()
                .any(|needle| message.contains(needle))
                .then_some(())
        }
        _ => None,
    })
    .unwrap_or_else(|| panic!("Missing status containing {needles:?}"));
}

#[track_caller]
fn copied(session: &WindowSession) -> String {
    copied_within(session, Duration::from_secs(5))
}

/// The next copied text, which a whole-file Copy hands over after reading the file.
fn copied_within(session: &WindowSession, timeout: Duration) -> String {
    await_output(session, timeout, |output| match output {
        ViewerOutput::CopyText(text) => Some(text),
        _ => None,
    })
    .expect("copied text")
}

/// The frame paints a selection highlight in the test appearance.
fn shows_selection(frame: &ViewerFrame) -> bool {
    frame
        .pixels
        .as_chunks::<4>()
        .0
        .contains(&appearance().selection)
}

#[test]
fn slow_ui_mailbox_retains_only_latest_frame_status_and_copy() {
    let mailbox = Mailbox::default();
    for index in 0..100 {
        mailbox.put(ViewerOutput::Status(index.to_string()));
        mailbox.put(ViewerOutput::CopyText(index.to_string()));
    }
    let output = mailbox.take();
    assert_eq!(output.len(), 2);
    assert!(matches!(&output[1], ViewerOutput::CopyText(text) if text == "99"));
    mailbox.put(ViewerOutput::Status("obsolete".into()));
    mailbox.put(ViewerOutput::Closed);
    assert!(matches!(mailbox.take().as_slice(), [ViewerOutput::Closed]));
}

#[test]
fn capture_uses_visible_order_and_skips_directories_and_filtered_files() {
    let root = tempfile::tempdir().unwrap();
    for name in ["one", "two", "filtered"] {
        std::fs::write(root.path().join(name), name).unwrap();
    }
    let mut launch = launch(root.path(), &["one", "two", "filtered"], false);
    let mut index = (*launch.view).clone();
    index.by_id.remove(&EntryId(2));
    index.by_id.insert(EntryId(0), 1);
    index.by_id.insert(EntryId(1), 0);
    launch.view = Arc::new(index);
    let closed = Arc::new(AtomicBool::new(false));
    let order = crate::capture::prepare(&launch, &closed).unwrap();
    assert_eq!(order.current, 1);
    assert_eq!(order.files.len(), 2);
    assert_eq!(order.files[0].file_name().unwrap(), "two");
    assert!(order.focused.is_ok());
    assert!(order.capture.as_ref().unwrap()(0, &closed).is_ok());
    closed.store(true, Ordering::Release);
    assert!(crate::capture::prepare(&launch, &closed).is_err());
    let mut entries = launch.entries.to_vec();
    entries[0].kind = EntryKind::Directory;
    launch.entries = entries.into();
    closed.store(false, Ordering::Release);
    assert!(crate::capture::prepare(&launch, &closed).is_err());
}

#[test]
fn opening_fails_cleanly_when_the_resident_refuses_the_worker() {
    // New work requires the original transient runtime to remain alive. Resident refusal is
    // the supported backend-unavailable boundary; resident_tests separately covers accepted
    // queued ownership after the original transient runtime exits.
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (resident_owner, resident) = midnight_work_budget::resident::Owner::start(
        "viewer-refusal-credit",
        crate::RESIDENT_WORKERS,
    )
    .unwrap();
    let service = Service::with_native_capture(
        runtime.handle().clone(),
        resident,
        Arc::new(|_, _| panic!("a refused worker captures nothing")),
    );
    resident_owner.seal();
    let root = tempfile::tempdir().unwrap();
    let result = service.open(
        launch(root.path(), &["file.txt"], false),
        appearance(),
        ViewerGeometry {
            width: 200,
            height: 100,
            scale: 1.0,
        },
    );
    assert!(
        result
            .err()
            .unwrap()
            .contains("resident worker is unavailable")
    );
    drop(service);
    runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(5),
            resident_owner.shutdown(runtime.handle()),
        )
        .await
        .unwrap()
        .unwrap();
    });
}

/// The number of viewer windows is the user's choice: eight windows (twice the former
/// maximum of four) open at once, each on its own resident thread, and each publishes Closed.
#[test]
fn eight_windows_open_and_run_at_once_and_each_publishes_closed() {
    const WINDOWS: usize = 8;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let (resident_owner, resident) = midnight_work_budget::resident::Owner::start_elastic(
        "viewer-eight-windows",
        crate::RESIDENT_WORKERS,
    )
    .unwrap();
    let service = Service::new(runtime.handle().clone(), resident);
    let plugins = tempfile::tempdir().unwrap();
    *service.runtime.lock().unwrap() = Some(Arc::new(Runtime::discover(plugins.path()).unwrap()));
    *service.images.lock().unwrap() = Some(Arc::from([]));
    *service.web.lock().unwrap() = Some(Arc::from([]));
    let files = tempfile::tempdir().unwrap();
    std::fs::write(files.path().join("file.txt"), "one of many windows\n").unwrap();
    let sessions: Vec<_> = (0..WINDOWS)
        .map(|_| {
            service
                .open(
                    launch(files.path(), &["file.txt"], true),
                    appearance(),
                    ViewerGeometry {
                        width: 200,
                        height: 100,
                        scale: 1.0,
                    },
                )
                .expect("no count refuses a viewer window")
        })
        .collect();
    // Every window's worker runs while all of them stay open: each answers its opening
    // (no plugin is installed) without waiting for another window to close.
    for session in &sessions {
        status(session, &["no compatible viewer plugin"]);
    }
    for session in &sessions {
        session.close();
    }
    for session in &sessions {
        await_output(session, Duration::from_secs(10), |output| {
            matches!(output, ViewerOutput::Closed).then_some(())
        })
        .expect("every closed window publishes Closed");
    }
    drop(sessions);
    drop(service);
    runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(5),
            resident_owner.shutdown(runtime.handle()),
        )
        .await
        .unwrap()
        .unwrap();
    });
}

#[test]
fn transient_catalog_failure_lets_the_same_image_policy_configure_again() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (resident_owner, resident) = midnight_work_budget::resident::Owner::start(
        "viewer-catalog-policy",
        crate::RESIDENT_WORKERS,
    )
    .unwrap();
    let service = Service::new(runtime.handle().clone(), resident);
    let policy = crate::image_startup::Policy {
        disabled: Arc::from([]),
        associations: Arc::default(),
        catalog_revision: 0,
    };
    *service.image_policy.lock().unwrap() = Some(policy.clone());
    service.configure_image_spare(policy.clone(), Err("discovery failed".into()));
    assert!(
        service.image_policy.lock().unwrap().is_none(),
        "a later identical request must not return early"
    );
    let newer = crate::image_startup::Policy {
        disabled: Arc::from(["midnight.viewer.image".to_owned()]),
        ..policy.clone()
    };
    *service.image_policy.lock().unwrap() = Some(newer.clone());
    service.configure_image_spare(policy, Err("late failure".into()));
    assert!(
        service.image_policy.lock().unwrap().as_ref() == Some(&newer),
        "a stale failure keeps a newer policy"
    );
    drop(service);
    runtime.block_on(async {
        tokio::time::timeout(
            Duration::from_secs(5),
            resident_owner.shutdown(runtime.handle()),
        )
        .await
        .unwrap()
        .unwrap();
    });
}

#[test]
#[ignore = "requires MIDNIGHT_VIEWER_TEST_INSTALL with three independently built native plugins"]
fn real_plugins_render_search_copy_cycle_modes_and_reject_changed_files() {
    let install =
        std::env::var_os("MIDNIGHT_VIEWER_TEST_INSTALL").expect("secure installation path");
    let runtime = Arc::new(Runtime::discover(Path::new(&install)).unwrap());
    assert_eq!(runtime.catalog().len(), 3, "{:?}", runtime.diagnostics());
    let executor = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (_resident_owner, resident) =
        midnight_work_budget::resident::Owner::start("viewer-native-test", crate::RESIDENT_WORKERS)
            .unwrap();
    let service = Service {
        resident: resident.clone(),
        raster_budget: Arc::new(crate::raster_budget::Budget::default()),
        web: Arc::new(Mutex::new(Some(Arc::from([])))),
        handle: executor.handle().clone(),
        capture: crate::capture::native(),
        runtime: Arc::new(Mutex::new(Some(runtime))),
        images: Arc::new(Mutex::new(Some(Arc::from([])))),
        fonts: Default::default(),
        video: Default::default(),
        catalog_preparation: Default::default(),
        image_startup: crate::image_startup::Startup::new(executor.handle().clone(), resident),
        image_policy: Default::default(),
        image_catalog_revision: Arc::new(AtomicU64::new(0)),
    };
    let files = tempfile::tempdir().unwrap();
    {
        use std::io::Write;
        let mut file = std::fs::File::create(files.path().join("first.txt")).unwrap();
        file.write_all("FIRST Žluťoučký\n".as_bytes()).unwrap();
        let chunk = "line café\n".repeat(4000);
        for _ in 0..512 {
            file.write_all(chunk.as_bytes()).unwrap();
        }
        file.write_all(b"LAST needle").unwrap();
    }
    std::fs::write(
        files.path().join("second.rs"),
        "fn main() { println!(\"Second\"); }\n",
    )
    .unwrap();
    std::fs::write(
        files.path().join("third.md"),
        "# Heading\n\nHello **world**!\n",
    )
    .unwrap();
    let opened = Instant::now();
    let session = service
        .open(
            launch(files.path(), &["first.txt", "second.rs", "third.md"], false),
            appearance(),
            ViewerGeometry {
                width: 600,
                height: 300,
                scale: 1.0,
            },
        )
        .unwrap();
    let initial = frame(&session, |frame| frame.accessible_text.starts_with("FIRST"));
    eprintln!(
        "First viewport: {:?}; surface {} bytes",
        opened.elapsed(),
        initial.pixels.len()
    );
    assert_eq!(initial.pixels.len(), 600 * 300 * 4);
    assert!(
        initial.accessible_text.len() <= 12 * 1024,
        "Large files stay paged"
    );
    assert!(session.send(ViewerInput::SelectPage {
        frame: initial.revision
    }));
    let selected = frame(&session, |f| f.revision > initial.revision);
    // Selection may repaint before the UI presents that new frame. Ctrl+A selected the
    // whole file: Copy reads all of its 22 MB through the plugin and copies exactly it.
    assert!(session.send(ViewerInput::Copy {
        frame: initial.revision
    }));
    let copying = Instant::now();
    let whole = copied_within(&session, Duration::from_secs(120));
    eprintln!("Copy of the whole file: {:?}", copying.elapsed());
    assert!(
        whole
            == format!(
                "FIRST Žluťoučký\n{}LAST needle",
                "line café\n".repeat(4000 * 512)
            ),
        "the whole file's text, {} bytes",
        whole.len()
    );
    drop(whole);
    let mut renderer = midnight_viewer_render::TextRenderer::default();
    let shaped = renderer
        .layout_with_options(
            &initial.accessible_text,
            &midnight_viewer_render::TextOptions {
                family: "monospace",
                size: 14.0,
                ..Default::default()
            },
            &[],
        )
        .unwrap();
    session.send(ViewerInput::Scroll {
        x: 0.0,
        y: shaped.height() - 300.0 - 100.0,
    });
    let near_edge = frame(&session, |f| f.revision > selected.revision);
    session.send(ViewerInput::Scroll { x: 0.0, y: 300.0 });
    let bottom = frame(&session, |f| f.revision > near_edge.revision);
    assert!(
        bottom
            .accessible_text
            .contains(initial.accessible_text.lines().rev().nth(1).unwrap()),
        "A demand page must overlap the unseen source tail rather than jumping past it"
    );
    let search_started = Instant::now();
    session.send(ViewerInput::Search {
        query: "needle".into(),
        backwards: false,
        case_sensitive: false,
        whole_words: false,
    });
    let found = frame(&session, |f| {
        f.accessible_text.contains("needle") && f.position > 0.5
    });
    eprintln!("Search across 20 MiB: {:?}", search_started.elapsed());
    session.send(ViewerInput::Seek(1.0));
    let end = frame(&session, |f| {
        f.revision > found.revision && f.accessible_text.ends_with("LAST needle")
    });
    assert!(
        end.accessible_text.len() > 20,
        "EOF must show a page, not one byte"
    );
    session.send(ViewerInput::Seek(0.0));
    frame(&session, |f| {
        f.revision > end.revision && f.accessible_text.starts_with("FIRST")
    });
    session.send(ViewerInput::Search {
        query: "not present in this file".into(),
        backwards: false,
        case_sensitive: false,
        whole_words: false,
    });
    // The search was submitted; NextFile then cancels it if it is still running.
    status(&session, &["searching", "no further match"]);
    let cancelling = Instant::now();
    session.send(ViewerInput::NextFile);
    let source = frame(&session, |f| f.caption.contains("second.rs"));
    eprintln!("Cancel search and cycle: {:?}", cancelling.elapsed());
    assert_eq!(source.mode, midnight_plugin_host::api::viewer::MODE_SOURCE);
    session.send(ViewerInput::Search {
        query: "Second".into(),
        backwards: false,
        case_sensitive: true,
        whole_words: false,
    });
    frame(&session, |f| {
        f.revision > source.revision && f.accessible_text.contains("Second")
    });
    session.send(ViewerInput::PreviousFile);
    frame(&session, |f| {
        f.caption.contains("first.txt") && f.accessible_text.starts_with("FIRST")
    });
    session.send(ViewerInput::PreviousFile);
    frame(&session, |f| {
        f.caption.contains("third.md") && f.accessible_text.contains("Hello world!")
    });
    session.send(ViewerInput::NextFile);
    frame(&session, |f| {
        f.caption.contains("first.txt") && f.accessible_text.starts_with("FIRST")
    });
    session.send(ViewerInput::NextFile);
    frame(&session, |f| {
        f.caption.contains("second.rs") && f.mode == midnight_plugin_host::api::viewer::MODE_SOURCE
    });
    session.send(ViewerInput::NextFile);
    let markdown = frame(&session, |f| f.caption.contains("third.md"));
    assert!(markdown.accessible_text.contains("Hello world!"));
    assert!(!markdown.accessible_text.contains("**"));
    session.send(ViewerInput::Search {
        query: "world".into(),
        backwards: false,
        case_sensitive: true,
        whole_words: false,
    });
    frame(&session, |f| {
        f.revision > markdown.revision && f.accessible_text.contains("world")
    });
    session.send(ViewerInput::Mode(
        midnight_plugin_host::api::viewer::MODE_TEXT,
    ));
    frame(&session, |f| {
        f.caption.contains("third.md") && f.accessible_text.contains("**world**")
    });
    std::fs::write(files.path().join("first.txt"), "REPLACED").unwrap();
    session.send(ViewerInput::NextFile);
    // Navigation captures the current identity, not one prefetched before the change.
    frame(&session, |f| {
        f.caption.contains("first.txt") && f.accessible_text.starts_with("REPLACED")
    });
    // The displayed document keeps its captured identity: a later change is rejected.
    std::fs::write(files.path().join("first.txt"), "CHANGED WHILE DISPLAYED").unwrap();
    session.send(ViewerInput::Mode(
        midnight_plugin_host::api::viewer::MODE_BINARY,
    ));
    status(&session, &["changed"]);
    let closing = Instant::now();
    session.close();
    await_output(&session, Duration::from_secs(5), |output| {
        matches!(output, ViewerOutput::Closed).then_some(())
    })
    .expect("the closed worker publishes Closed");
    eprintln!("Close worker: {:?}", closing.elapsed());

    std::fs::write(files.path().join("binary.bin"), [0, 1, 2, 0xff, b'A']).unwrap();
    let basic = service
        .open(
            launch(files.path(), &["binary.bin"], true),
            appearance(),
            ViewerGeometry {
                width: 600,
                height: 300,
                scale: 1.0,
            },
        )
        .unwrap();
    let text = frame(&basic, |_| true);
    assert_eq!(text.mode, midnight_plugin_host::api::viewer::MODE_TEXT);
    basic.send(ViewerInput::Mode(
        midnight_plugin_host::api::viewer::MODE_BINARY,
    ));
    let hex = frame(&basic, |f| {
        f.mode == midnight_plugin_host::api::viewer::MODE_BINARY
    });
    assert!(hex.accessible_text.contains("00 01 02 FF 41"));
    assert!(hex.accessible_text.contains("│ ····A"));
    assert!(!hex.wrap_available);
    basic.send(ViewerInput::Search {
        query: "A".into(),
        backwards: false,
        case_sensitive: true,
        whole_words: false,
    });
    frame(&basic, |f| {
        f.revision > hex.revision && f.accessible_text.contains("41")
    });
    basic.close();

    for (name, bytes) in [
        ("utf16-le.txt", vec![255, 254, 65, 0, 61, 216, 0, 222]),
        ("utf16-be.txt", vec![254, 255, 0, 65, 216, 61, 222, 0]),
        ("utf8.txt", "A😀 Žluťoučký 世界".as_bytes().to_vec()),
    ] {
        std::fs::write(files.path().join(name), bytes).unwrap();
    }
    let automatic = service
        .open(
            launch(
                files.path(),
                &["binary.bin", "utf16-le.txt", "utf16-be.txt", "utf8.txt"],
                false,
            ),
            appearance(),
            ViewerGeometry {
                width: 600,
                height: 300,
                scale: 1.0,
            },
        )
        .unwrap();
    let binary = frame(&automatic, |_| true);
    assert_eq!(binary.mode, midnight_plugin_host::api::viewer::MODE_BINARY);
    assert!(binary.accessible_text.contains("00 01 02 FF 41"));
    for name in ["utf16-le.txt", "utf16-be.txt", "utf8.txt"] {
        automatic.send(ViewerInput::NextFile);
        let unicode = frame(&automatic, |f| f.caption.contains(name));
        assert_eq!(unicode.mode, midnight_plugin_host::api::viewer::MODE_TEXT);
        assert!(unicode.accessible_text.starts_with("A😀"));
    }
    automatic.close();

    std::fs::write(files.path().join("empty.md"), [0xef, 0xbb, 0xbf]).unwrap();
    let empty = service
        .open(
            launch(files.path(), &["empty.md"], false),
            appearance(),
            ViewerGeometry {
                width: 200,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    frame(&empty, |f| {
        f.mode == midnight_plugin_host::api::viewer::MODE_DOCUMENT
    });
    empty.send(ViewerInput::Mode(
        midnight_plugin_host::api::viewer::MODE_TEXT,
    ));
    frame(&empty, |f| {
        f.mode == midnight_plugin_host::api::viewer::MODE_TEXT && f.accessible_text.is_empty()
    });
    empty.close();

    let mut disabled = launch(files.path(), &["second.rs"], false);
    disabled.disabled_plugins = Arc::from([
        "midnight.viewer.basic".into(),
        "midnight.viewer.source".into(),
        "midnight.viewer.markdown".into(),
    ]);
    let unavailable = service
        .open(
            disabled,
            appearance(),
            ViewerGeometry {
                width: 200,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap();
    status(&unavailable, &["no compatible viewer"]);
    unavailable.close();
}

#[test]
#[ignore = "requires MIDNIGHT_VIEWER_TEST_INSTALL with basic and source plugins"]
fn native_whole_word_find_exhausts_substrings_and_keeps_real_row_prefix() {
    let installation =
        std::path::PathBuf::from(std::env::var_os("MIDNIGHT_VIEWER_TEST_INSTALL").unwrap());
    let executor = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (_owner, resident) =
        midnight_work_budget::resident::Owner::start("viewer-find-native", crate::RESIDENT_WORKERS)
            .unwrap();
    let service = Service::new(executor.handle().clone(), resident);
    *service.runtime.lock().unwrap() = Some(Arc::new(Runtime::discover(&installation).unwrap()));
    *service.images.lock().unwrap() = Some(Arc::from([]));
    let files = tempfile::tempdir().unwrap();
    let text = format!(
        "{}\n    α😀 target value\n{}",
        "hotdog dog_name dog\u{301} dog\u{203f}tail\n".repeat(32),
        "trailing row\n".repeat(40)
    );
    for (name, basic) in [("word-boundaries.txt", true), ("word-boundaries.rs", false)] {
        let path = files.path().join(name);
        std::fs::write(&path, &text).unwrap();
        images::settle_generated_fixture(&path);
        let session = service
            .open(
                launch(files.path(), &[name], basic),
                appearance(),
                ViewerGeometry {
                    width: 600,
                    height: 180,
                    scale: 1.0,
                },
            )
            .unwrap();
        frame(&session, |_| true);
        assert!(session.send(ViewerInput::Search {
            query: "dog".into(),
            backwards: false,
            case_sensitive: false,
            whole_words: true
        }));
        status(&session, &["no further match"]);
        assert!(session.send(ViewerInput::Search {
            query: "dog".into(),
            backwards: true,
            case_sensitive: false,
            whole_words: true
        }));
        status(&session, &["no further match"]);
        assert!(session.send(ViewerInput::Search {
            query: "target".into(),
            backwards: false,
            case_sensitive: false,
            whole_words: true
        }));
        // A frame from before this find, such as a repaint after "No further match",
        // also lacks "Searching"; only the find result shows a selection. Later
        // frames that keep that selection do not retire its Copy.
        let found = frame(&session, |frame| {
            frame.accessible_text.contains("    α😀 target value")
                && !frame.status.contains("Searching")
                && shows_selection(frame)
        });
        assert!(session.send(ViewerInput::Copy {
            frame: found.revision
        }));
        assert_eq!(copied(&session), "target");
        session.close();
    }
}
