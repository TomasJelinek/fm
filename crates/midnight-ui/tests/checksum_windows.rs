#[path = "support/modeless.rs"]
mod modeless;
use midnight_core::*;
use midnight_ui::{ChecksumDraft, ChecksumWindow, MainWindow, Presenter};
use modeless::{Windows, click, draw};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

#[test]
fn checksum_rows_support_accessible_selection_and_algorithm_context_copy() {
    use i_slint_core::{accessibility::AccessibilityAction, items::AccessibleRole};
    use midnight_ui::ChecksumDisplayRow;
    use slint::platform::PointerEventButton;

    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.invoke_apply_preferences(1, 0, "".into(), 11, 15);
    window.set_draft(ChecksumDraft {
        sha256: true,
        sha512: true,
        md5: true,
        sha1: true,
        recursive: false,
    });
    window.set_generation_token("7".into());
    window.set_output_available(Rc::new(slint::VecModel::from(vec![true; 4])).into());
    window.set_rows(
        Rc::new(slint::VecModel::from(vec![ChecksumDisplayRow {
            token: "7:1".into(),
            name: "日本語.bin".into(),
            path: "fixture".into(),
            status: "Calculated".into(),
            bytes: "3 B".into(),
            digests: "hashes".into(),
            sha256: "42".repeat(32).into(),
            sha512: "43".repeat(64).into(),
            md5: "44".repeat(16).into(),
            sha1: "45".repeat(20).into(),
            calculated: true,
            error: false,
        }]))
        .into(),
    );
    let copies = Rc::new(RefCell::new(Vec::new()));
    window.on_copy_result({
        let copies = copies.clone();
        move |token, algorithm, whole| copies.borrow_mut().push((token, algorithm, whole))
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "checksum-columns");
    let row = modeless::accessible_item(
        &native,
        AccessibleRole::ListItem,
        "日本語.bin · fixture · 3 B · Calculated · hashes",
    )
    .unwrap();
    row.accessible_action(&AccessibilityAction::Default);
    assert_eq!(window.get_selected_row(), 0);
    // Pointer is in the filename cell even when the table extends past the viewport.
    let rect = row.geometry();
    let origin = row.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(origin.x + 16.0, origin.y + 10.0);
    for (index, label) in ["SHA-256", "SHA-512", "MD5", "SHA-1", "full row"]
        .iter()
        .enumerate()
    {
        native.window().dispatch_event(WindowEvent::PointerPressed {
            position,
            button: PointerEventButton::Right,
        });
        native
            .window()
            .dispatch_event(WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Right,
            });
        draw(&native, 860, 620, "checksum-context");
        click(&native, &format!("Copy {label}"));
        assert_eq!(
            copies.borrow().last().cloned(),
            Some((
                "7:1".into(),
                if index == 4 { 0 } else { index as i32 },
                index == 4
            ))
        );
    }
    // An open menu must disappear on generation changes, not redirect to new rows.
    native.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: PointerEventButton::Right,
    });
    native
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: PointerEventButton::Right,
        });
    window.set_generation_token("8".into());
    draw(&native, 860, 620, "checksum-new-generation");
    assert!(
        modeless::accessible_popup_item(&native, AccessibleRole::Button, "Copy SHA-256").is_none()
    );
    assert_eq!(copies.borrow().len(), 5);
    window.hide().unwrap();
}

#[test]
fn checksum_window_captures_selection_streams_and_closes_without_blocking_main() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let root = std::env::temp_dir();
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let accepting = Rc::new(Cell::new(true));
    midnight_ui::install(
        &main,
        state.clone(),
        Rc::new({
            let commands = commands.clone();
            let accepting = accepting.clone();
            move |command| {
                if !accepting.get() {
                    return false;
                }
                commands.borrow_mut().push(command);
                true
            }
        }),
    );
    let navigation = commands
        .borrow()
        .iter()
        .find_map(|c| match c {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    let path = root.join("Žluťoučký 日本語.txt");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    state.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    main.show().unwrap();
    midnight_ui::refresh(&main, &state.borrow());
    draw(&windows.borrow()[0], 1000, 700, "checksum-main");
    main.invoke_row_click(0, 1, false, false); // First non-parent entry.
    main.invoke_calculate_checksums(0);
    assert_eq!(
        windows.borrow().len(),
        2,
        "Checksums need an independent native window"
    );
    let request = commands
        .borrow()
        .iter()
        .find_map(|c| match c {
            Command::StartSelectedChecksums(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(request.files.fallback, Some(path.clone()));
    assert!(!request.recursive);
    let retention = ChecksumRetention::new(());
    let rows: Arc<[Arc<ChecksumRow>]> = vec![Arc::new(ChecksumRow {
        id: ChecksumRowId(1),
        path,
        identity: None,
        kind: Some(EntryKind::File),
        state: ChecksumRowState::Calculated,
        bytes_read: 3,
        digests: vec![ChecksumDigest {
            algorithm: ChecksumAlgorithm::Sha256,
            bytes: vec![0x42; 32],
        }],
        error: None,
        retention: retention.clone(),
    })]
    .into();
    state
        .borrow_mut()
        .apply(Event::Checksum(Box::new(ChecksumEvent {
            job: request.job,
            generation: request.generation,
            update: ChecksumUpdate::Batch(rows.clone()),
        })));
    midnight_ui::refresh(&main, &state.borrow());
    let native = windows.borrow()[1].clone();
    draw(&native, 860, 620, "checksum-running");
    click(&native, "Stop");
    assert!(
        commands
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::CancelChecksums { job, .. } if *job == request.job))
    );
    state
        .borrow_mut()
        .apply(Event::Checksum(Box::new(ChecksumEvent {
            job: request.job,
            generation: request.generation,
            update: ChecksumUpdate::Finished(Arc::new(ChecksumRun {
                job: request.job,
                generation: request.generation,
                algorithms: request.algorithms,
                captured_roots: vec![root].into(),
                rows,
                summary: ChecksumSummary {
                    completion: ChecksumCompletion::Completed,
                    progress: ChecksumProgress {
                        discovered: 1,
                        calculated: 1,
                        bytes_read: 3,
                        ..Default::default()
                    },
                    reached_budgets: vec![],
                    diagnostics: vec![],
                },
                retention,
            })),
        })));
    midnight_ui::refresh(&main, &state.borrow());
    draw(&native, 320, 200, "checksum-completed");
    click(&native, "Save");
    draw(&native, 320, 200, "checksum-save");
    click(&native, "Save");
    assert!(
        commands
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::SaveChecksumManifest { job, .. } if *job == request.job))
    );
    accepting.set(false);
    click(&native, "Close");
    assert!(native.window().is_visible());
    accepting.set(true);
    native.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    assert!(!native.window().is_visible());
    assert!(main.window().is_visible());
    main.invoke_close_checksum_windows();
    midnight_ui::refresh(&main, &state.borrow());
    assert_eq!(windows.borrow().len(), 2);
    main.hide().unwrap();
    state.borrow_mut().retire_all();
}

#[test]
fn checksum_actions_keep_320_by_200_layout_in_every_theme_and_density() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    let closed = Rc::new(Cell::new(0));
    window.on_close_session({
        let closed = closed.clone();
        move || closed.set(closed.get() + 1)
    });
    window.set_draft(ChecksumDraft {
        sha256: true,
        ..Default::default()
    });
    window.set_has_output(true);
    window.set_output_available(Rc::new(slint::VecModel::from(vec![true; 4])).into());
    window.set_error("Access denied\n\nFile: C:\\Žluťoučký 日本語\\Long name.txt".into());
    window.set_attention(true);
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for theme in 0..9 {
        for density in 0..2 {
            window.invoke_apply_preferences(theme, density, "".into(), 11, 15);
            for (width, height) in [(860, 620), (320, 200)] {
                for page in 0..3 {
                    window.set_page(page);
                    draw(&native, width, height, "checksum-layout");
                    click(&native, if width < 460 { "Run" } else { "Calculate" });
                    click(&native, "Close");
                }
            }
        }
    }
    assert_eq!(closed.get(), 108);
    window.hide().unwrap();
}
