use super::*;
use std::{cell::RefCell, io};

#[derive(Debug)]
struct Text;
impl GeneratedTextSource for Text {
    fn length(&self) -> u64 {
        3
    }
    fn read_at(&self, offset: u64, out: &mut [u8]) -> io::Result<usize> {
        let bytes = b"abc".get(offset as usize..).unwrap_or_default();
        let len = bytes.len().min(out.len());
        out[..len].copy_from_slice(&bytes[..len]);
        Ok(len)
    }
}

#[derive(Debug)]
struct Report;
impl SizeWarningSource for Report {
    fn snapshot(&self, _: &Cancellation) -> Result<Arc<dyn GeneratedTextSource>, FsError> {
        panic!("UI must not read the report")
    }
}

#[derive(Debug)]
struct BackendOnlyText;
impl GeneratedTextSource for BackendOnlyText {
    fn length(&self) -> u64 {
        panic!("UI must not inspect generated storage")
    }
    fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
        panic!("UI must not read generated storage")
    }
}

fn fixture() -> (Presenter, String, Arc<Report>) {
    let root = std::env::temp_dir();
    let mut p = Presenter::new(root.clone(), root.clone());
    p.viewer_plugins.startup_disabled = Some(Arc::from([]));
    let pane = &mut p.panes[0];
    let entry = Entry::new(EntryId(1), root.join("資料"), EntryKind::Directory);
    let source = Arc::new(Report);
    pane.folder_sizes.insert(
        entry.path.clone(),
        FolderSize {
            errors: 1,
            warnings: Some(SizeWarningReport::new(
                source.clone(),
                SizeWarningSummary {
                    observed: 1,
                    stored: 1,
                    tooltip: "Access denied in child 資料".into(),
                },
            )),
            ..Default::default()
        },
    );
    pane.entries = vec![entry].into();
    pane.index = Arc::new(DirectoryIndex::from_entries(&pane.entries));
    pane.cursor = Some(EntryId(1));
    pane.loading = false;
    let identity = pane.row_identity(&pane.entries[0]);
    (p, identity, source)
}

fn request(p: &mut Presenter, identity: &str) -> (DesktopRequestId, Cancellation) {
    let commands = RefCell::new(Vec::new());
    p.inspect_panel_warning(PaneId::Left, identity, false, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    match commands.into_inner().pop().unwrap() {
        Command::OpenSizeWarnings {
            request, cancel, ..
        } => (request, cancel),
        _ => panic!("only a read-only diagnostic request is permitted"),
    }
}

#[test]
fn warning_uses_exact_retained_report_and_rejects_stale_or_parent_identity() {
    let (mut p, identity, _) = fixture();
    let before_cursor = p.panes[0].cursor;
    let before_selected = p.panes[0].selected.clone();
    for stale in ["parent", "different:1", "exact:99"] {
        p.inspect_panel_warning(PaneId::Left, stale, false, &|_| panic!("stale request"));
    }
    request(&mut p, &identity);
    assert_eq!(p.panes[0].cursor, before_cursor);
    assert!(Arc::ptr_eq(&p.panes[0].selected, &before_selected));
}

#[test]
fn report_result_stays_valid_after_navigation_and_launches_basic_searchable_viewer() {
    let (mut p, identity, _) = fixture();
    let (id, _) = request(&mut p, &identity);
    p.panes[0].generation += 1;
    p.apply(Event::SizeWarningsReady {
        request: id,
        result: Ok(Arc::new(Text)),
    });
    let launch = p.take_warning_viewer().unwrap();
    assert!(launch.basic);
    assert!(launch.generated.is_some());
    assert!(launch.entries.is_empty());
    assert!(launch.focused.to_string_lossy().contains("資料"));
}

#[test]
fn newer_click_cancels_old_request_and_retires_stale_content_off_ui() {
    let (mut p, identity, _) = fixture();
    let (old, cancelled) = request(&mut p, &identity);
    let (new, _) = request(&mut p, &identity);
    assert!(cancelled.is_cancelled());
    assert_ne!(old, new);
    let content = Arc::new(Text);
    let weak = Arc::downgrade(&content);
    p.apply(Event::SizeWarningsReady {
        request: old,
        result: Ok(content),
    });
    assert!(p.take_warning_viewer().is_none());
    assert!(weak.upgrade().is_some());
    let retired = p.take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(weak.upgrade().is_none());
}

#[test]
fn shutdown_retires_ready_content_and_scan_report_owners() {
    let (mut p, identity, source) = fixture();
    let weak_report = Arc::downgrade(&source);
    drop(source);
    let (id, _) = request(&mut p, &identity);
    let content = Arc::new(Text);
    let weak_content = Arc::downgrade(&content);
    p.apply(Event::SizeWarningsReady {
        request: id,
        result: Ok(content),
    });
    p.retire_all();
    assert!(p.take_warning_viewer().is_none());
    assert!(weak_report.upgrade().is_some());
    assert!(weak_content.upgrade().is_some());
    let retired = p.take_retired();
    drop(p);
    std::thread::spawn(move || drop(retired)).join().unwrap();
    assert!(weak_report.upgrade().is_none());
    assert!(weak_content.upgrade().is_none());
}

#[test]
fn busy_worker_keeps_existing_request_and_shows_explicit_error() {
    let (mut p, identity, _) = fixture();
    let (_, cancel) = request(&mut p, &identity);
    p.inspect_panel_warning(PaneId::Left, &identity, false, &|_| false);
    assert!(!cancel.is_cancelled());
    assert!(p.desktop.notice.as_ref().unwrap().1.contains("busy"));
}

#[test]
fn full_terminal_details_are_correlated_with_current_panel_error_and_generation() {
    let (mut p, _, _) = fixture();
    let content: Arc<dyn GeneratedTextSource> = Arc::new(Text);
    let diagnostic = TreeDiagnostic::with_details("Short reason", content.clone());
    p.panes[0].set_tree_error(&diagnostic, "Cannot list: ", &mut p.retired);
    let sent = RefCell::new(None);
    p.inspect_panel_error(PaneId::Left, "Cannot list: Short reason", &|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    let Command::ReadDiagnosticText {
        content: captured, ..
    } = sent.into_inner().unwrap()
    else {
        panic!("full error details must use the modal's backend reader");
    };
    assert!(Arc::ptr_eq(&captured, &content));
    assert!(p.take_warning_viewer().is_none());
    p.panes[0].generation += 1;
    p.inspect_panel_error(PaneId::Left, "Cannot list: Short reason", &|_| {
        panic!("stale source")
    });
    assert!(p.take_warning_viewer().is_none());
    assert_eq!(
        p.desktop.notice.as_ref().unwrap().1,
        "Cannot list: Short reason"
    );
}

#[test]
fn generic_error_replacement_with_identical_text_retires_old_full_details() {
    let (mut p, _, _) = fixture();
    let diagnostic = TreeDiagnostic::with_details("Same reason", Arc::new(Text));
    p.panes[0].set_tree_error(&diagnostic, "", &mut p.retired);
    p.panes[0].set_error("Same reason", &mut p.retired);
    p.inspect_panel_error(PaneId::Left, "Same reason", &|_| panic!("replaced source"));
    assert!(p.take_warning_viewer().is_none());
    assert_eq!(p.desktop.notice.as_ref().unwrap().1, "Same reason");
}

#[test]
fn short_panel_error_cancels_a_previous_report_launch() {
    let (mut p, identity, _) = fixture();
    let (id, cancel) = request(&mut p, &identity);
    p.panes[0].set_error("Another error", &mut p.retired);
    p.inspect_panel_error(PaneId::Left, "Another error", &|_| {
        panic!("short error has no reads")
    });
    assert!(cancel.is_cancelled());
    p.apply(Event::SizeWarningsReady {
        request: id,
        result: Ok(Arc::new(Text)),
    });
    assert!(p.take_warning_viewer().is_none());
}

#[test]
fn copying_panel_error_captures_full_source_and_never_reads_it_on_ui() {
    let (mut p, _, _) = fixture();
    let content: Arc<dyn GeneratedTextSource> = Arc::new(BackendOnlyText);
    let full = format!("{}tail", "long reason ".repeat(600));
    let diagnostic = TreeDiagnostic::with_details(&full, content.clone());
    p.panes[1].set_tree_error(&diagnostic, "", &mut p.retired);
    let summary = p.panes[1].error.clone();
    assert!(summary.len() < full.len());
    let commands = RefCell::new(Vec::new());
    p.copy_panel_error(PaneId::Right, &summary, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    let command = commands.into_inner().pop().unwrap();
    let Command::CopyGeneratedText {
        request,
        content: captured,
        cancel,
    } = command
    else {
        panic!("Copy must retain the full generated source instead of the truncated summary");
    };
    assert!(Arc::ptr_eq(&captured, &content));
    assert!(!cancel.is_cancelled());
    assert!(p.desktop.requests.contains(&request));
}

#[test]
fn stale_panel_error_copy_is_rejected_and_ordinary_errors_copy_their_complete_text() {
    let (mut p, _, _) = fixture();
    p.panes[0].set_error("current error", &mut p.retired);
    p.copy_panel_error(PaneId::Left, "old error", &|_| {
        panic!("stale error gesture")
    });
    let commands = RefCell::new(Vec::new());
    p.copy_panel_error(PaneId::Left, "current error", &|command| {
        commands.borrow_mut().push(command);
        true
    });
    assert!(
        matches!(&commands.borrow()[0], Command::CopyText { text, .. } if text == "current error")
    );
}

#[test]
fn displayed_global_volume_error_copies_text_without_reusing_a_pane_report() {
    let (mut p, _, _) = fixture();
    p.volumes_error = "Cannot list drives".into();
    p.panes[0].set_tree_error(
        &TreeDiagnostic::with_details("Cannot list drives", Arc::new(BackendOnlyText)),
        "",
        &mut p.retired,
    );
    // Legacy/direct pane clearing must not turn a global error into an old source.
    p.panes[0].error.clear();
    let commands = RefCell::new(Vec::new());
    p.copy_panel_error(PaneId::Left, "Cannot list drives", &|command| {
        commands.borrow_mut().push(command);
        true
    });
    assert!(
        matches!(&commands.borrow()[0], Command::CopyText { text, .. } if text == "Cannot list drives")
    );
}

#[test]
fn rejected_generated_error_copy_keeps_current_owner_and_reports_queue_failure() {
    let (mut p, _, _) = fixture();
    let content: Arc<dyn GeneratedTextSource> = Arc::new(Text);
    let weak = Arc::downgrade(&content);
    p.panes[0].set_tree_error(
        &TreeDiagnostic::with_details("error", content),
        "",
        &mut p.retired,
    );
    p.copy_panel_error(PaneId::Left, "error", &|command| {
        assert!(matches!(command, Command::CopyGeneratedText { .. }));
        false
    });
    assert!(weak.upgrade().is_some());
    assert!(p.desktop.requests.is_empty());
    assert!(p.desktop.notice.as_ref().unwrap().0.contains("Clipboard"));
}

#[test]
fn identical_error_text_with_different_details_changes_gesture_revision() {
    let (mut p, _, _) = fixture();
    p.panes[0].set_tree_error(
        &TreeDiagnostic::with_details("same", Arc::new(Text)),
        "",
        &mut p.retired,
    );
    let before = p.panes[0].error_revision();
    p.panes[0].set_tree_error(
        &TreeDiagnostic::with_details("same", Arc::new(Text)),
        "",
        &mut p.retired,
    );
    assert_ne!(before, p.panes[0].error_revision());
}

#[test]
fn missing_size_report_escapes_control_characters_in_fallback_path() {
    let (mut p, old_identity, _) = fixture();
    let (_, cancelled) = request(&mut p, &old_identity);
    let path = std::env::temp_dir().join("line\n\nOperation: forged");
    let entry = Entry::new(EntryId(2), path.clone(), EntryKind::Directory);
    p.panes[0].folder_sizes.insert(
        path,
        FolderSize {
            errors: 1,
            ..Default::default()
        },
    );
    p.panes[0].entries = vec![entry].into();
    p.panes[0].index = Arc::new(DirectoryIndex::from_entries(&p.panes[0].entries));
    let identity = p.panes[0].row_identity(&p.panes[0].entries[0]);
    p.inspect_panel_warning(PaneId::Left, &identity, false, &|_| {
        panic!("unavailable report")
    });
    assert!(cancelled.is_cancelled());
    let body = &p.desktop.notice.as_ref().unwrap().1;
    assert!(!body.contains("\n\nOperation: forged"));
    assert!(body.contains("line\\n\\nOperation: forged"));
}

fn modal_request(
    p: &mut Presenter,
) -> (DesktopRequestId, Cancellation, Arc<dyn GeneratedTextSource>) {
    let content: Arc<dyn GeneratedTextSource> = Arc::new(BackendOnlyText);
    let diagnostic = TreeDiagnostic::with_details("Access denied", content.clone());
    p.panes[0].set_tree_error(&diagnostic, "", &mut p.retired);
    let sent = RefCell::new(None);
    p.inspect_panel_error(PaneId::Left, "Access denied", &|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    let Command::ReadDiagnosticText {
        request,
        cancel,
        content: captured,
    } = sent.into_inner().unwrap()
    else {
        panic!("error click must read captured text for the modal");
    };
    assert!(Arc::ptr_eq(&captured, &content));
    assert!(p.take_warning_viewer().is_none());
    (request, cancel, content)
}

#[test]
fn full_error_opens_notice_and_copy_retains_full_source_without_ui_reads() {
    let (mut p, _, _) = fixture();
    let (request, _, content) = modal_request(&mut p);
    let generation = p.desktop.notice_generation;
    let full = "Access denied\n\nPath\nC:/資料/private\n\nOperation: list";
    p.apply(Event::DiagnosticTextReady {
        request,
        result: Ok(DiagnosticTextPreview {
            text: full.into(),
            total_bytes: full.len() as u64,
            truncated: false,
        }),
    });
    assert_eq!(
        p.desktop.notice.as_ref().unwrap(),
        &("Panel error".into(), format!("Access denied\n\n{full}"))
    );
    assert_eq!(
        p.desktop.notice_generation, generation,
        "update the same modal"
    );
    assert!(p.take_warning_viewer().is_none());
    let sent = RefCell::new(None);
    p.copy_notice_details(&|command| {
        *sent.borrow_mut() = Some(command);
        true
    });
    let Command::CopyGeneratedText {
        content: captured, ..
    } = sent.into_inner().unwrap()
    else {
        panic!("Copy details must copy the original full report");
    };
    assert!(Arc::ptr_eq(&captured, &content));
}

#[test]
fn modal_preview_is_honestly_truncated_and_late_closed_reply_cannot_replace_a_notice() {
    let (mut p, _, _) = fixture();
    let (old, cancelled, _) = modal_request(&mut p);
    let old_generation = p.desktop.notice_generation;
    p.dismiss_diagnostic_notice(old_generation);
    assert!(cancelled.is_cancelled());
    let (request, _, _) = modal_request(&mut p);
    let generation = p.desktop.notice_generation;
    p.apply(Event::DiagnosticTextReady {
        request: old,
        result: Ok(DiagnosticTextPreview {
            text: "late wrong report".into(),
            total_bytes: 17,
            truncated: false,
        }),
    });
    assert_eq!(p.desktop.notice_generation, generation);
    assert!(!p.desktop.notice.as_ref().unwrap().1.contains("late wrong"));
    p.apply(Event::DiagnosticTextReady {
        request,
        result: Ok(DiagnosticTextPreview {
            text: "bounded preview".into(),
            total_bytes: 100000,
            truncated: true,
        }),
    });
    assert!(
        p.desktop
            .notice
            .as_ref()
            .unwrap()
            .1
            .contains("Showing 15 of 100000 bytes")
    );
    assert!(
        p.desktop
            .notice
            .as_ref()
            .unwrap()
            .1
            .contains("complete report")
    );
    assert!(p.take_warning_viewer().is_none());
    let (request, _, _) = modal_request(&mut p);
    p.desktop.report("New notice", "Keep this message".into());
    p.apply(Event::DiagnosticTextReady {
        request,
        result: Ok(DiagnosticTextPreview {
            text: "superseded".into(),
            total_bytes: 10,
            truncated: false,
        }),
    });
    assert_eq!(p.desktop.notice.as_ref().unwrap().1, "Keep this message");
}

#[test]
fn stale_panel_error_click_cannot_open_or_copy_a_replacement_diagnostic() {
    let (mut p, _, _) = fixture();
    p.panes[0].set_error("Current error", &mut p.retired);
    p.inspect_panel_error(PaneId::Left, "Old error", &|_| panic!("stale click"));
    assert!(p.desktop.notice.is_none());
    assert!(p.take_warning_viewer().is_none());
}
