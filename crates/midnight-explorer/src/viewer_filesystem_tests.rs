//! Installed app composition, using actual independent native and viewer packages.
use midnight_core::{
    DirectoryIndex, Entry, EntryId, EntryKind, ViewerAppearance, ViewerFrame, ViewerGeometry,
    ViewerImageInput, ViewerInput, ViewerLaunch, ViewerOutput,
};
use midnight_native_filesystem_host::InstalledProvider;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

fn installation() -> PathBuf {
    std::env::var_os("MIDNIGHT_NATIVE_TEST_INSTALLATION")
        .unwrap()
        .into()
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
        focused: entries[0].path.clone(),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
        preferences: Arc::default(),
        archive: None,
        basic,
        disabled_plugins: Arc::from([]),
    }
}

fn open(
    service: &midnight_viewer_engine::Service,
    launch: ViewerLaunch,
) -> midnight_viewer_engine::WindowSession {
    service
        .open(
            launch,
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [240; 4],
                background: [25, 25, 25, 255],
                selection: [80, 100, 180, 255],
                accent: [120; 4],
            },
            ViewerGeometry {
                width: 320,
                height: 200,
                scale: 1.0,
            },
        )
        .unwrap()
}

fn wait_text(session: &midnight_viewer_engine::WindowSession, expected: &str) {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut messages = Vec::new();
    while Instant::now() < deadline {
        for message in session.mailbox.take() {
            match message {
                ViewerOutput::Frame(frame) if frame.accessible_text.contains(expected) => return,
                ViewerOutput::Status(text) => messages.push(text),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("No native-plugin content {expected:?}: {messages:?}");
}

/// Render the exact immutable tree order through real installed native/text plugins.
#[test]
#[ignore = "run inside protected runtime with native/viewer packages and fixtures"]
fn tree_viewer_uses_native_provider_and_cycles_only_visible_root_files() {
    use midnight_core::{
        NavigationId, PaneId, TreeDisclosure, TreeDisplayRow, TreeLayoutRevision, TreeProjection,
        TreeRevision, TreeRootSnapshot, TreeRowId, TreeSessionId, TreeViewStamp, TreeViewer,
    };
    let root = PathBuf::from(std::env::var_os("MIDNIGHT_APP_NATIVE_TEST_FILES").unwrap());
    let mut launch = launch(
        &root,
        &["first 文档.txt", "second.txt", "hidden-missing.txt"],
        true,
    );
    let mut entries = launch.entries.to_vec();
    entries.push(Entry::new(
        EntryId(3),
        root.join("folder"),
        EntryKind::Directory,
    ));
    launch.entries = entries.into();
    launch.view = Arc::new(DirectoryIndex::from_entries(&launch.entries));
    let root_snapshot = TreeRootSnapshot {
        pane: PaneId::Left,
        navigation: NavigationId(1),
        entries: launch.entries.clone(),
        index: launch.view.clone(),
    };
    let make_row = |id: u64, original: Option<usize>, nested: bool| {
        let mut entry = original.map_or_else(
            || Entry::new(EntryId(id), root.join("folder/child"), EntryKind::Directory),
            |index| launch.entries[index].clone(),
        );
        entry.id = EntryId(id);
        TreeDisplayRow {
            id: TreeRowId(id),
            entry,
            parent: nested.then_some(TreeRowId(10)),
            depth: usize::from(nested),
            disclosure: if id == 10 {
                TreeDisclosure::Expanded
            } else {
                TreeDisclosure::None
            },
            expanded: id == 10,
            selectable: true,
            navigable: original == Some(3) || nested,
            context_only: false,
            error: None,
            error_details: None,
            original_root_entry: original.map(|index| launch.entries[index].id),
            prefix_end: 0,
            search_match: false,
        }
    };
    let projection = Arc::new(
        TreeProjection::new(
            root_snapshot,
            TreeViewStamp {
                pane: PaneId::Left,
                navigation: NavigationId(1),
                session: TreeSessionId(1),
                revision: TreeRevision(1),
            },
            TreeLayoutRevision(1),
            vec![
                make_row(10, Some(3), false),
                make_row(11, None, true),
                make_row(12, Some(1), false),
                make_row(13, Some(0), false),
            ],
        )
        .unwrap(),
    );
    launch.tree = Some(TreeViewer {
        view: projection,
        focused: TreeRowId(13),
    });
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let native = Arc::new(InstalledProvider::from_installation(installation(), None));
    let service = super::service(runtime.handle(), native);
    let session = open(&service, launch);
    wait_text(&session, "First native provider content");
    assert!(session.send(ViewerInput::NextFile));
    wait_text(&session, "Second native provider content");
    assert!(session.send(ViewerInput::NextFile));
    wait_text(&session, "First native provider content");
    session.close();
}

#[test]
#[ignore = "run this test executable inside a protected runtime with native/viewer packages and fixtures"]
fn native_plugin_feeds_actual_f3_and_next_file_without_fallback() {
    let root = PathBuf::from(std::env::var_os("MIDNIGHT_APP_NATIVE_TEST_FILES").unwrap());
    let launch = || launch(&root, &["first 文档.txt", "second.txt"], true);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let native = Arc::new(InstalledProvider::from_installation(installation(), None));
    let service = super::service(runtime.handle(), native);
    let session = open(&service, launch());
    wait_text(&session, "First native provider content");
    assert!(session.send(ViewerInput::NextFile));
    wait_text(&session, "Second native provider content");
    session.close();

    let unavailable = Arc::new(InstalledProvider::from_installation(
        installation().join("no-native-provider"),
        None,
    ));
    let provider_error = unavailable
        .capture(&root.join("first 文档.txt"), &|| false)
        .unwrap_err()
        .to_string();
    eprintln!("Unavailable native provider cause: {provider_error}");
    let service = super::service(runtime.handle(), unavailable);
    let session = open(&service, launch());
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut messages = Vec::new();
    loop {
        let mut unavailable = false;
        for output in session.mailbox.take() {
            match output {
                ViewerOutput::Status(message) => {
                    unavailable |=
                        message.contains("filesystem") && message.contains(&provider_error);
                    if messages.len() == 8 {
                        messages.remove(0);
                    }
                    messages.push(message);
                }
                ViewerOutput::Frame(frame) => {
                    panic!(
                        "Unavailable provider emitted a content frame: mode={}, text={:?}, status={:?}",
                        frame.mode, frame.accessible_text, messages
                    );
                }
                _ => {}
            }
        }
        if unavailable {
            session.close();
            return;
        }
        assert!(
            Instant::now() < deadline,
            "Missing explicit unavailable-provider status for {provider_error:?}: {messages:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[track_caller]
fn wait_frame(
    session: &midnight_viewer_engine::WindowSession,
    expected: &str,
    predicate: impl Fn(&ViewerFrame) -> bool,
) -> ViewerFrame {
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut status = String::new();
    let mut last_frame = None;
    while Instant::now() < deadline {
        for output in session.mailbox.take() {
            match output {
                ViewerOutput::Frame(frame) if predicate(&frame) => return *frame,
                ViewerOutput::Frame(frame) => {
                    last_frame = Some((frame.mode, frame.caption, frame.status));
                }
                ViewerOutput::Status(message) => status = message,
                ViewerOutput::ImageUnavailable { message, .. } => {
                    panic!("{expected}: image/document unavailable: {message}");
                }
                ViewerOutput::Closed => panic!("{expected}: viewer closed: {status}"),
                _ => {}
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    panic!("Missing {expected}: status={status:?}, last frame={last_frame:?}");
}

fn bitmap() -> Vec<u8> {
    let mut bytes = b"BM".to_vec();
    bytes.extend_from_slice(&(54 + 64 * 32 * 3u32).to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&64i32.to_le_bytes());
    bytes.extend_from_slice(&32i32.to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&24u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    for _ in 0..64 * 32 {
        bytes.extend_from_slice(&[224, 144, 32]);
    }
    bytes
}

fn two_page_pdf() -> Vec<u8> {
    let mut objects = vec![
        "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
        "<< /Type /Pages /Kids [3 0 R 5 0 R] /Count 2 >>".to_owned(),
    ];
    for (index, (width, height, color)) in [(400, 600, "1 0 0"), (1000, 400, "0 0 1")]
        .into_iter()
        .enumerate()
    {
        let content = format!("{color} rg 0 0 {width} {height} re f");
        objects.push(format!(
            "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] \
             /Resources << >> /Contents {} 0 R >>",
            4 + index * 2
        ));
        objects.push(format!(
            "<< /Length {} >>\nstream\n{content}\nendstream",
            content.len()
        ));
    }
    let mut bytes = b"%PDF-1.7\n".to_vec();
    let mut offsets = Vec::new();
    for (index, object) in objects.into_iter().enumerate() {
        offsets.push(bytes.len());
        bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", index + 1).as_bytes());
    }
    let xref = bytes.len();
    bytes.extend_from_slice(b"xref\n0 7\n0000000000 65535 f \n");
    for offset in offsets {
        bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
    }
    bytes.extend_from_slice(
        format!("trailer\n<< /Size 7 /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n").as_bytes(),
    );
    bytes
}

fn fixtures(native: &InstalledProvider) -> PathBuf {
    let parent = PathBuf::from(std::env::var_os("MIDNIGHT_APP_NATIVE_TEST_FILES").unwrap());
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    // Keep only these small authored artifacts inside the caller's QA directory.
    let root = parent.join(format!("f3-formats-{}-{nonce}", std::process::id()));
    std::fs::create_dir(&root).unwrap();
    let source: String = (1..=300)
        .map(|line| format!("// native row {line:04} α😀\r\n"))
        .collect();
    let data = [
        ("source 文档.rs", source.into_bytes()),
        (
            "document.md",
            b"# Native Markdown\n\nProvider **rendered** content.\n".to_vec(),
        ),
        ("image.bmp", bitmap()),
        ("pages.pdf", two_page_pdf()),
    ];
    for (name, bytes) in &data {
        assert!(bytes.len() < 16 * 1024);
        std::fs::write(root.join(name), bytes).unwrap();
    }
    // Native scanners can add extended attributes several seconds after creation,
    // including without a viewer/read. Observe both minimum age and quiet revisions
    // before launching; production revision validation remains unchanged.
    let versions = || {
        data.iter()
            .map(|(name, _)| {
                native
                    .capture(&root.join(name), &|| false)
                    .unwrap()
                    .stable_version()
            })
            .collect::<Vec<_>>()
    };
    let started = Instant::now();
    let mut quiet = Instant::now();
    let mut previous = versions();
    while started.elapsed() < Duration::from_secs(8) || quiet.elapsed() < Duration::from_secs(2) {
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "fixture revisions did not settle"
        );
        std::thread::sleep(Duration::from_millis(100));
        let current = versions();
        if current != previous {
            previous = current;
            quiet = Instant::now();
        }
    }
    for (name, expected) in &data {
        let captured = native.capture(&root.join(name), &|| false).unwrap();
        assert_eq!(
            captured.read_at(0, expected.len() as u32).unwrap(),
            *expected,
            "authored fixture changed before viewer capture"
        );
    }
    root
}

fn assert_pdf_page(frame: &ViewerFrame, page: u32) {
    let image = frame.image.as_ref().unwrap();
    assert_eq!(frame.mode, midnight_core::VIEWER_MODE_IMAGE);
    assert_eq!((image.page, image.page_count), (page, 2));
    assert_eq!(
        &*frame.accessible_text,
        format!("Document page {page} of 2")
    );
    assert_eq!(
        frame.pixels.len(),
        (frame.width * frame.height * 4) as usize
    );
    let center = ((frame.height / 2 * frame.width + frame.width / 2) * 4) as usize;
    let expected = if page == 1 {
        [255, 0, 0, 255]
    } else {
        [0, 0, 255, 255]
    };
    assert_eq!(&frame.pixels[center..center + 4], &expected);
}

#[test]
#[ignore = "requires protected runtime with native filesystem plus basic, Source, Markdown, image and PDF packages; Linux basic-only fixture is insufficient"]
fn native_plugin_feeds_selected_source_markdown_image_and_pdf() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let native = Arc::new(InstalledProvider::from_installation(installation(), None));
    let root = fixtures(&native);
    let service = super::service(runtime.handle(), native);
    let session = open(
        &service,
        launch(
            &root,
            &["source 文档.rs", "document.md", "image.bmp", "pages.pdf"],
            false,
        ),
    );
    let source = wait_frame(&session, "Source first page", |frame| {
        frame.mode == 2 && frame.status.starts_with("Ln 1, Col 1")
    });
    assert!(source.accessible_text.contains("native row 0001 α😀"));
    assert!(source.image.is_none());
    assert!(!source.pixels.is_empty());
    assert!(session.send(ViewerInput::Search {
        query: "row 0200".into(),
        backwards: false,
        case_sensitive: true,
    }));
    let found = wait_frame(&session, "Source exact search progress", |frame| {
        frame.mode == 2 && frame.status.starts_with("Ln 200, Col ")
    });
    assert!(found.revision > source.revision);
    assert!(found.position > source.position);
    assert!(found.accessible_text.starts_with("row 0200"));

    assert!(session.send(ViewerInput::NextFile));
    let markdown = wait_frame(&session, "rendered Markdown", |frame| {
        frame.mode == 4 && frame.caption.contains("document.md")
    });
    assert!(markdown.accessible_text.contains("Native Markdown"));
    assert!(
        markdown
            .accessible_text
            .contains("Provider rendered content.")
    );
    assert!(!markdown.accessible_text.contains("**"));
    assert!(!markdown.pixels.is_empty());
    assert!(markdown.image.is_none());

    assert!(session.send(ViewerInput::NextFile));
    let image = wait_frame(&session, "decoded bitmap tiles", |frame| {
        frame.caption.contains("image.bmp")
            && frame.tiles.as_ref().is_some_and(|tiles| !tiles.is_empty())
    });
    let info = image.image.as_ref().unwrap();
    assert_eq!(image.mode, midnight_core::VIEWER_MODE_IMAGE);
    assert_eq!((info.width, info.height, info.page), (64, 32, 0));
    assert!(image.tiles.as_ref().unwrap().iter().any(|tile| {
        tile.validate().unwrap();
        tile.pixels
            .rgba()
            .as_chunks::<4>()
            .0
            .contains(&[32, 144, 224, 255])
    }));

    assert!(session.send(ViewerInput::NextFile));
    let mut pdf = wait_frame(&session, "PDF first page", |frame| {
        frame.caption.contains("pages.pdf")
            && frame.image.as_ref().is_some_and(|image| image.page == 1)
    });
    assert_pdf_page(&pdf, 1);
    assert!(session.send(ViewerInput::Image {
        frame: pdf.revision,
        input: ViewerImageInput::ActualPixels,
    }));
    pdf = wait_frame(&session, "PDF manual zoom", |frame| {
        frame
            .image
            .as_ref()
            .is_some_and(|image| image.page == 1 && !image.fitted && image.zoom == 1.0)
    });
    for (delta, page) in [(1, 2), (-1, 1)] {
        assert!(session.send(ViewerInput::DocumentPage {
            frame: pdf.revision,
            delta
        }));
        pdf = wait_frame(&session, "PDF page navigation at manual zoom", |frame| {
            frame.revision > pdf.revision
                && frame.image.as_ref().is_some_and(|image| image.page == page)
        });
        assert_pdf_page(&pdf, page);
        assert_eq!(pdf.image.as_ref().unwrap().zoom, 1.0);
        assert!(!pdf.image.as_ref().unwrap().fitted);
    }
    session.close();
}
