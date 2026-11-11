use super::*;
#[path = "image_integration_tests.rs"]
mod images;
#[path = "pdf_navigation_tests.rs"]
mod pdf_navigation;
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

#[track_caller]
fn frame(session: &WindowSession, predicate: impl Fn(&ViewerFrame) -> bool) -> ViewerFrame {
    let deadline = Instant::now() + Duration::from_secs(45);
    let mut messages = Vec::new();
    while Instant::now() < deadline {
        for output in session.mailbox.take() {
            match output {
                ViewerOutput::Frame(frame) if predicate(&frame) => return *frame,
                ViewerOutput::Status(status) => messages.push(status),
                ViewerOutput::ImageUnavailable { message, .. } => {
                    panic!("Image/document unavailable: {message}");
                }
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("No expected frame: {messages:?}")
}

fn status(session: &WindowSession, needle: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        for output in session.mailbox.take() {
            if let ViewerOutput::Status(message) = output
                && message.to_lowercase().contains(needle)
            {
                return;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("Missing status containing {needle:?}");
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

#[tokio::test]
async fn capture_uses_visible_order_and_skips_directories_and_filtered_files() {
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
    assert_eq!(order.files[0].0.file_name().unwrap(), "two");
    let (index, identity) = order.ready.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_eq!(index, 0);
    assert!(identity.is_ok());
    closed.store(true, Ordering::Release);
    assert!(crate::capture::prepare(&launch, &closed).is_err());
    let mut entries = launch.entries.to_vec();
    entries[0].kind = EntryKind::Directory;
    launch.entries = entries.into();
    closed.store(false, Ordering::Release);
    assert!(crate::capture::prepare(&launch, &closed).is_err());
}

#[test]
fn enormous_panel_capture_is_rejected_before_native_identity_work() {
    let entries: Arc<[Entry]> = (0..20_000)
        .map(|id| {
            Entry::new(
                EntryId(id),
                format!("{}-{id}", "x".repeat(128)).into(),
                EntryKind::File,
            )
        })
        .collect();
    let launch = ViewerLaunch {
        associations: Default::default(),
        tree: None,
        generated: None,
        preferences: Arc::new(Default::default()),
        archive: None,
        focused: entries[0].path.clone(),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
        basic: false,
        disabled_plugins: Arc::from([]),
    };
    let error = crate::capture::prepare(&launch, &Arc::new(AtomicBool::new(false)))
        .err()
        .unwrap();
    assert!(error.contains("8 MiB"));
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
    let service = Service {
        raster_budget: Arc::new(crate::raster_budget::Budget::default()),
        web: Arc::new(Mutex::new(Some(Arc::from([])))),
        handle: executor.handle().clone(),
        capture: crate::capture::native(),
        runtime: Arc::new(Mutex::new(Some(runtime))),
        images: Arc::new(Mutex::new(Some(Arc::from([])))),
        windows: Arc::new(AtomicUsize::new(0)),
        fonts: Default::default(),
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
    // Selection may repaint before the UI presents that new frame.
    assert!(session.send(ViewerInput::Copy {
        frame: initial.revision
    }));
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut copied = false;
    while Instant::now() < deadline && !copied {
        for event in session.mailbox.take() {
            if let ViewerOutput::CopyText(text) = event {
                assert!(text.starts_with("FIRST Žluťoučký"));
                copied = true;
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(copied);
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
    });
    std::thread::sleep(Duration::from_millis(30));
    let cancelling = Instant::now();
    session.send(ViewerInput::NextFile);
    let source = frame(&session, |f| f.caption.contains("second.rs"));
    eprintln!("Cancel search and cycle: {:?}", cancelling.elapsed());
    assert_eq!(source.mode, midnight_plugin_host::api::viewer::MODE_SOURCE);
    session.send(ViewerInput::Search {
        query: "Second".into(),
        backwards: false,
        case_sensitive: true,
    });
    frame(&session, |f| {
        f.revision > source.revision && f.accessible_text.contains("Second")
    });
    session.send(ViewerInput::NextFile);
    let markdown = frame(&session, |f| f.caption.contains("third.md"));
    assert!(markdown.accessible_text.contains("Hello world!"));
    assert!(!markdown.accessible_text.contains("**"));
    session.send(ViewerInput::Search {
        query: "world".into(),
        backwards: false,
        case_sensitive: true,
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
    status(&session, "changed");
    let closing = Instant::now();
    session.close();
    while service.windows.load(Ordering::Acquire) != 0 && closing.elapsed() < Duration::from_secs(5)
    {
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(service.windows.load(Ordering::Acquire), 0);
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
    status(&unavailable, "no compatible viewer");
    unavailable.close();
}
