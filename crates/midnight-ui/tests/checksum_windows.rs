use crate::support::modeless;
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
fn checksum_window_offers_file_verification_and_browse() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 1000, 620, "checksum-verify-page");
    click(&native, "Verify file");
    draw(&native, 1000, 620, "checksum-verify-input");
    assert_eq!(window.get_page(), 1);
    assert!(
        modeless::accessible_item(
            &native,
            i_slint_core::items::AccessibleRole::Button,
            "Browse checksum file",
        )
        .is_some()
    );
    window.hide().unwrap();
}

#[test]
fn empty_panel_can_choose_manifest_then_explicitly_start_verification() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let root = std::env::temp_dir();
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    midnight_ui::install(
        &main,
        state.clone(),
        Rc::new({
            let commands = commands.clone();
            move |command| {
                commands.borrow_mut().push(command);
                true
            }
        }),
    );
    main.show().unwrap();
    midnight_ui::refresh(&main, &state.borrow());
    main.invoke_calculate_checksums(0);
    assert_eq!(windows.borrow().len(), 2);
    let native = windows.borrow()[1].clone();
    // The shown folder is the Calculate target; Verify file is the other page.
    draw(&native, 1000, 620, "checksum-calculate-empty-panel");
    click(&native, "Verify file");
    draw(&native, 1000, 620, "checksum-verify-empty-panel");
    click(&native, "Browse checksum file");
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PickPath { request, input } if input.kind == PathPickerKind::ChecksumOpen => {
                Some(*request)
            }
            _ => None,
        })
        .unwrap();
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::VerifyChecksumManifest(_)))
    );
    let manifest = root.join("checksums 日本語.sfv");
    state.borrow_mut().apply(Event::PathPicked {
        request,
        result: Ok(Some(manifest.clone())),
    });
    midnight_ui::refresh(&main, &state.borrow());
    draw(&native, 1000, 620, "checksum-verify-picked");
    click(&native, "Verify");
    let verify = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::VerifyChecksumManifest(request) => Some(request.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(verify.manifest, manifest);
    assert_eq!(verify.generation, ChecksumGeneration(1));
    state
        .borrow_mut()
        .apply(Event::Checksum(Box::new(ChecksumEvent {
            job: verify.job,
            generation: verify.generation,
            update: ChecksumUpdate::Verifying(ChecksumAlgorithm::Crc32),
        })));
    midnight_ui::refresh(&main, &state.borrow());
    draw(&native, 1000, 620, "checksum-verify-running");
    // Completed row states, not the presence of a digest, determine the result icon.
    let cases = [
        (ChecksumRowState::Verified, "Passed"),
        (ChecksumRowState::Mismatch, "Failed"),
        (ChecksumRowState::Missing, "Failed"),
        (ChecksumRowState::SourceChanged, "Failed"),
        (ChecksumRowState::ReadError, "Failed"),
        (ChecksumRowState::Calculated, ""),
        (ChecksumRowState::Skipped, ""),
        (ChecksumRowState::Cancelled, ""),
    ];
    let retention = ChecksumRetention::new(());
    let rows = cases
        .iter()
        .enumerate()
        .map(|(index, (status, _))| {
            Arc::new(ChecksumRow {
                id: ChecksumRowId(index as u64 + 1),
                path: root.join(format!("{status:?}.txt")),
                identity: None,
                kind: Some(EntryKind::File),
                state: *status,
                bytes_read: 3,
                digests: Vec::new(),
                error: None,
                retention: retention.clone(),
            })
        })
        .collect::<Vec<_>>();
    state
        .borrow_mut()
        .apply(Event::Checksum(Box::new(ChecksumEvent {
            job: verify.job,
            generation: verify.generation,
            update: ChecksumUpdate::Batch(rows.into()),
        })));
    midnight_ui::refresh(&main, &state.borrow());
    draw(&native, 1000, 620, "checksum-verification-icons");
    for (status, expected) in cases {
        for label in ["Passed", "Failed"] {
            assert_eq!(
                modeless::accessible_item(
                    &native,
                    i_slint_core::items::AccessibleRole::Image,
                    &format!("{label}: {status:?}.txt"),
                )
                .is_some(),
                label == expected,
                "Wrong {label} indicator for {status:?}"
            );
        }
    }
    click(&native, "Stop");
    assert!(commands.borrow().iter().any(|command| matches!(command,
        Command::CancelChecksums { job, generation } if *job == verify.job && *generation == verify.generation)));
    native.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    assert!(!native.window().is_visible());
    main.hide().unwrap();
    state.borrow_mut().retire_all();
}

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
        crc32: true,
        recursive: false,
    });
    window.set_generation_token("7".into());
    window.set_output_available(Rc::new(slint::VecModel::from(vec![true; 5])).into());
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
            crc32: "46".repeat(4).into(),
            calculated: true,
            verified: false,
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
    for (index, label) in ["SHA-256", "SHA-512", "MD5", "SHA-1", "CRC32", "full row"]
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
                if index == 5 { 0 } else { index as i32 },
                index == 5
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
    assert_eq!(copies.borrow().len(), 6);
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
    state
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(
            midnight_core::AcceptedPreferences {
                preferences: UserPreferences {
                    checksum_algorithms: ChecksumAlgorithms::new(&[
                        ChecksumAlgorithm::Md5,
                        ChecksumAlgorithm::Sha256,
                    ])
                    .unwrap(),
                    ..Default::default()
                },
                comparators: midnight_core::FileComparatorSnapshot::default(),
            },
        ))));
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
    // Opening calculates nothing; the user presses Calculate in the new window.
    let child = windows.borrow()[1].clone();
    draw(&child, 1000, 620, "checksum-before-calculate");
    let calculate = modeless::accessible_items(
        &child,
        i_slint_core::items::AccessibleRole::Button,
        "Calculate",
    );
    modeless::click_item(&child, calculate.last().unwrap(), "Calculate");
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
    // The MD5 + SHA-256 set saved by an earlier version is not the window's default.
    assert_eq!(request.algorithms, ChecksumAlgorithms::default());
    let retention = ChecksumRetention::new(());
    let rows: Arc<[Arc<ChecksumRow>]> = vec![Arc::new(ChecksumRow {
        id: ChecksumRowId(1),
        path,
        identity: None,
        kind: Some(EntryKind::File),
        state: ChecksumRowState::Calculated,
        bytes_read: 3,
        digests: vec![
            ChecksumDigest {
                algorithm: ChecksumAlgorithm::Sha256,
                bytes: vec![0x42; 32],
            },
            ChecksumDigest {
                algorithm: ChecksumAlgorithm::Md5,
                bytes: vec![0x43; 16],
            },
        ],
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
    let completed = Arc::new(ChecksumRun {
        job: request.job,
        generation: request.generation,
        algorithms: request.algorithms,
        captured_roots: vec![root.clone()].into(),
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
    });
    state
        .borrow_mut()
        .apply(Event::Checksum(Box::new(ChecksumEvent {
            job: request.job,
            generation: request.generation,
            update: ChecksumUpdate::Finished(completed.clone()),
        })));
    midnight_ui::refresh(&main, &state.borrow());
    draw(&native, 1000, 620, "checksum-default-format");
    let pick_request = || {
        commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::PickPath { request, input } => {
                    assert_eq!(
                        input.kind,
                        PathPickerKind::ChecksumSave(ChecksumAlgorithm::Sha256)
                    );
                    assert_eq!(input.initial_directory.as_ref(), Some(&root));
                    Some(*request)
                }
                _ => None,
            })
            .unwrap()
    };
    click(&native, "Save manifest…");
    state.borrow_mut().apply(Event::PathPicked {
        request: pick_request(),
        result: Ok(None),
    });
    midnight_ui::refresh(&main, &state.borrow());
    draw(&native, 1000, 620, "checksum-save-cancelled");
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|command| matches!(command, Command::SaveChecksumManifest { .. }))
    );
    // Choosing a file saves at once; an existing file is asked about, not refused.
    click(&native, "Save manifest…");
    let chosen = root.join("Šek 日本語.sha256");
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    state.borrow_mut().apply_with_sender(
        Event::PathPicked {
            request: pick_request(),
            result: Ok(Some(chosen.clone())),
        },
        &send,
    );
    midnight_ui::refresh(&main, &state.borrow());
    draw(&native, 1000, 620, "checksum-save");
    let saves = || {
        commands
            .borrow()
            .iter()
            .filter_map(|c| match c {
                Command::SaveChecksumManifest {
                    save_id,
                    job,
                    generation,
                    request: save,
                } if *job == request.job
                    && *generation == request.generation
                    && save.destination == chosen
                    && save.algorithm == ChecksumAlgorithm::Sha256 =>
                {
                    Some((*save_id, save.overwrite))
                }
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let first = saves();
    assert_eq!(first.len(), 1);
    assert!(!first[0].1, "the first attempt never replaces");
    state
        .borrow_mut()
        .apply(Event::ChecksumManifest(Box::new(ChecksumManifestEvent {
            save_id: first[0].0,
            job: request.job,
            generation: request.generation,
            update: ChecksumManifestUpdate::Finished(Err(FsError {
                operation: "save checksum manifest",
                path: chosen.clone(),
                message: "Manifest destination already exists.".into(),
                kind: std::io::ErrorKind::AlreadyExists,
            })),
        })));
    midnight_ui::refresh(&main, &state.borrow());
    draw(&native, 1000, 620, "checksum-replace-prompt");
    click(&native, "Replace");
    let all = saves();
    assert_eq!(all.len(), 2);
    assert!(all[1].1, "Replace confirms the overwrite");
    accepting.set(false);
    click(&native, "Close");
    assert!(native.window().is_visible());
    native.window().dispatch_event(WindowEvent::CloseRequested);
    assert!(
        native.window().is_visible(),
        "native X must preserve an unreleased result owner"
    );
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
    // Themes select colours only (ui/theme.slint), so every theme is laid out and
    // clicked, but only the first one is software-rendered per density, size and page.
    let themes = midnight_core::PreferenceTheme::ALL.map(|theme| theme.index());
    for theme in themes {
        for density in 0..2 {
            window.invoke_apply_preferences(theme, density, "".into(), 11, 15);
            for (width, height) in [(860, 620), (320, 200)] {
                for page in 0..2 {
                    window.set_page(page);
                    modeless::draw_or_layout(
                        &native,
                        width,
                        height,
                        &format!("checksum-layout-theme{theme}-density{density}-page{page}"),
                        theme == themes[0],
                    );
                    if page == 0 {
                        let calculate = modeless::accessible_items(
                            &native,
                            i_slint_core::items::AccessibleRole::Button,
                            "Calculate",
                        );
                        modeless::click_item(&native, calculate.last().unwrap(), "Calculate");
                    }
                    let previous_closes = closed.get();
                    click(&native, "Close");
                    assert_eq!(closed.get(), previous_closes + 1);
                }
            }
        }
    }
    window.hide().unwrap();
}
