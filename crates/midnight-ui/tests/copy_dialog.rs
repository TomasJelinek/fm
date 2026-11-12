#[path = "support/window_input.rs"]
mod input;
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

type NativeWindows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

fn multi_window_ui() -> (ui::TestUi, NativeWindows) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let window = midnight_ui::MainWindow::new().unwrap();
    window.set_preferences_ready(true);
    midnight_ui::install_file_keys(&window);
    window.show().unwrap();
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let native = windows.borrow()[0].clone();
    let mut test = ui::TestUi {
        window,
        native,
        pixels: Vec::new(),
        width: 1340,
        height: 820,
    };
    test.resize(1340, 820);
    (test, windows)
}

fn draft_child(windows: &NativeWindows) -> Rc<MinimalSoftwareWindow> {
    let child = windows
        .borrow()
        .iter()
        .skip(1)
        .rev()
        .find(|window| window.window().is_visible())
        .cloned()
        .expect("native draft child");
    modeless::draw(&child, 720, 520, "transfer-draft");
    child
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    child
}

fn dismiss_native_notice(windows: &NativeWindows) {
    let child = windows.borrow().last().unwrap().clone();
    assert!(child.window().is_visible());
    child.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Escape.into(),
    });
    child.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Escape.into(),
    });
    assert!(!child.window().is_visible());
}

#[test]
fn copy_confirmation_shows_the_complete_source_above_the_destination() {
    use i_slint_core::items::AccessibleRole;
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let dialog = midnight_ui::OperationWindow::new().unwrap();
    let body = "checksums.sha256\n\nFrom\nD:\\downloads\\119115";
    dialog.set_title_text("Copy".into());
    dialog.set_confirm_text("Copy".into());
    dialog.set_body(body.into());
    dialog.set_initial_name("D:\\temp".into());
    dialog.set_has_name(true);
    dialog.set_input_label("Destination folder".into());
    dialog.set_option_visible(true);
    dialog.set_option_enabled(true);
    dialog.set_option_label("Preserve exact modified time".into());
    dialog.show().unwrap();
    let child = windows.borrow()[0].clone();
    for (density, font) in [(0, 13), (1, 15), (0, 22)] {
        dialog.invoke_apply_preferences(3, density, "".into(), font, font);
        dialog.invoke_begin();
        modeless::draw(&child, 720, 520, &format!("copy-source-{density}-{font}"));
        let source = modeless::accessible_item(&child, AccessibleRole::Text, body).unwrap();
        let label =
            modeless::accessible_item(&child, AccessibleRole::Text, "Destination folder").unwrap();
        let source_rect = source.geometry();
        let source_top = source.map_to_native_window(source_rect.origin).y;
        let label_top = label.map_to_native_window(label.geometry().origin).y;
        assert!(source.is_visible() && label.is_visible());
        assert!(
            source_top + source_rect.size.height <= label_top,
            "Source path is clipped beneath destination: bottom={}, label={label_top}, font={font}, density={density}",
            source_top + source_rect.size.height
        );
        let editor =
            modeless::accessible_item(&child, AccessibleRole::TextInput, "Destination folder")
                .unwrap();
        assert!(
            editor.geometry().size.height < font as f32 * 3.0,
            "Destination must stay single-line"
        );
    }
    dialog.invoke_apply_preferences(3, 0, "".into(), 13, 15);
    modeless::draw(&child, 320, 200, "copy-source-compact");
    for (label, role) in [
        ("Destination folder", AccessibleRole::TextInput),
        ("Cancel", AccessibleRole::Button),
        ("Copy", AccessibleRole::Button),
    ] {
        let item = modeless::accessible_item(&child, role, label).unwrap();
        let rect = item.geometry();
        let origin = item.map_to_native_window(rect.origin);
        assert!(
            item.is_visible()
                && origin.x >= 0.0
                && origin.y >= 0.0
                && origin.x + rect.size.width <= 320.5
                && origin.y + rect.size.height <= 200.5,
            "{label} must remain reachable in a small child: {origin:?} {:?}",
            rect.size
        );
    }
    dialog.hide().unwrap();
}

#[test]
fn f5_destination_editor_consumes_enter_and_escape_without_using_the_clipboard() {
    destination_editor(Key::F5, false, false);
}
#[test]
fn f6_move_confirmation_consumes_enter_escape_and_busy_queue_without_using_clipboard() {
    destination_editor(Key::F6, true, false);
}
#[test]
fn exact_modified_time_copy_requires_explicit_option_and_survives_queue_refusal() {
    destination_editor(Key::F5, false, true);
}
#[test]
fn exact_modified_time_move_requires_explicit_option_and_survives_queue_refusal() {
    destination_editor(Key::F6, true, true);
}

#[derive(Debug)]
struct NoIoArchive;
impl ArchiveMount for NoIoArchive {
    fn id(&self) -> u64 {
        42
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("the dialog must not enumerate an archive")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("the dialog must not read archive content")
    }
    fn validate(&self) -> Result<(), FsError> {
        panic!("the dialog must not validate a source")
    }
}

#[test]
fn archive_dialog_disables_required_modified_time_and_keeps_ordinary_copy() {
    use i_slint_core::{accessibility::AccessibleStringProperty, items::AccessibleRole};
    let (mut ui, windows) = multi_window_ui();
    let root = std::env::temp_dir().join("archive dialog 文档");
    let state = Rc::new(RefCell::new(Presenter::new(
        root.clone(),
        root.join("target"),
    )));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let captured = commands.clone();
    install(
        &ui.window,
        state.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    let navigation = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    state.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        entries: Arc::from([]),
        index: Arc::default(),
    });
    let entries: Arc<[Entry]> = vec![Entry::from_archive(ArchiveNode {
        modified: None,
        id: VirtualEntryId {
            mount: 42,
            entry: 2,
        },
        parent: 1,
        raw_name: Arc::from(&b"raw.txt"[..]),
        display_name: "caption.txt".into(),
        kind: EntryKind::File,
        size: Some(5),
        readable: true,
    })]
    .into();
    state.borrow_mut().apply(Event::ArchiveListed {
        pane: PaneId::Left,
        navigation,
        result: Ok(ArchiveListing {
            location: ArchiveLocation {
                mount: Some(Arc::new(NoIoArchive)),
                address: ArchiveAddress {
                    format: None,
                    provider: "fake.archive".into(),
                    container: root.join("source.zip"),
                    source_version: vec![1],
                    components: vec![],
                },
                directory: 1,
                trail: vec![(1, "source.zip".into())],
            },
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        }),
    });
    refresh(&ui.window, &state.borrow());
    ui.window.invoke_focus_files();
    ui::key(&ui.window, Key::End);
    commands.borrow_mut().clear();
    ui::key(&ui.window, Key::F5);
    ui.draw();
    assert!(!ui.window.get_copy_draft_token().is_empty());
    let child = draft_child(&windows);
    let option = modeless::accessible_item(
        &child,
        AccessibleRole::Checkbox,
        "Preserve exact modified time",
    )
    .unwrap();
    assert_eq!(
        option
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .unwrap(),
        "false"
    );
    input::click(&child, &option);
    assert_eq!(
        option
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap(),
        "false"
    );
    let editor =
        modeless::accessible_item(&child, AccessibleRole::TextInput, "Destination folder").unwrap();
    input::click(&child, &editor);
    input::key(&child, Key::Return);
    ui.draw();
    let commands = commands.borrow();
    let [Command::EnqueueArchiveCopy(request)] = commands.as_slice() else {
        panic!("disabled metadata must leave ordinary archive copy available");
    };
    assert_eq!(request.fallback, Some(EntryId(2)));
    assert_eq!(request.location.mount_id(), Some(42));
    ui.window.hide().unwrap();
}

fn destination_editor(shortcut: Key, moving: bool, preserve: bool) {
    let (mut ui, windows) = multi_window_ui();
    let base = std::env::temp_dir().join("copy dialog 文档");
    let state = Rc::new(RefCell::new(Presenter::new(
        base.clone(),
        base.join("target"),
    )));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let captured = commands.clone();
    let accepting = Rc::new(Cell::new(true));
    let gate = accepting.clone();
    install(
        &ui.window,
        state.clone(),
        Rc::new(move |command| {
            if !gate.get() {
                return false;
            }
            captured.borrow_mut().push(command);
            true
        }),
    );
    let navigation = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(1),
        base.join("file.txt"),
        EntryKind::File,
    )]
    .into();
    state.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
    });
    refresh(&ui.window, &state.borrow());
    ui.window.invoke_focus_files();
    ui::key(&ui.window, Key::End);
    commands.borrow_mut().clear();
    ui::key(&ui.window, shortcut);
    ui.draw();
    assert!(!ui.window.get_copy_draft_token().is_empty());
    assert_eq!(ui.window.get_copy_draft_move(), moving);
    if moving {
        assert!(
            ui.window
                .get_copy_draft_body()
                .starts_with("Same-volume items are renamed.")
        );
        assert!(
            ui.window
                .get_copy_draft_body()
                .contains("verified before each source is removed")
        );
        assert!(
            ui.window
                .get_copy_draft_body()
                .contains("Verification: full SHA-256.")
        );
    }
    assert!(commands.borrow().is_empty());
    let child = draft_child(&windows);
    if moving {
        modeless::draw(&child, 320, 200, "move-dialog-small");
    }
    input::key(&child, Key::Escape);
    ui.draw();
    assert!(ui.window.get_copy_draft_token().is_empty());
    assert!(commands.borrow().is_empty());
    assert!(
        !ui.window.get_operations_visible(),
        "cancelled transfer confirmation must not open Jobs"
    );
    assert_eq!(
        ui.window.get_operations().row_count(),
        0,
        "cancelled transfer confirmation has no history receipt"
    );

    ui::key(&ui.window, shortcut);
    ui.draw();
    let child = draft_child(&windows);
    let option = modeless::accessible_item(
        &child,
        i_slint_core::items::AccessibleRole::Checkbox,
        "Preserve exact modified time",
    )
    .unwrap();
    assert_eq!(
        option
            .accessible_string_property(
                i_slint_core::accessibility::AccessibleStringProperty::Checked
            )
            .unwrap(),
        "false",
        "every new draft must start unchecked",
    );
    if preserve {
        input::click(&child, &option);
        // Restore path focus without activating the form or changing the option.
        let editor = modeless::accessible_item(
            &child,
            i_slint_core::items::AccessibleRole::TextInput,
            "Destination folder",
        )
        .unwrap();
        input::click(&child, &editor);
        assert_eq!(
            option
                .accessible_string_property(
                    i_slint_core::accessibility::AccessibleStringProperty::Checked
                )
                .unwrap(),
            "true",
        );
    }
    let destination = base.join("another target");
    // Both directions include the optional checkbox and stay within the modal form.
    let inner = i_slint_core::window::WindowInner::from_pub(child.window());
    let editor = inner.focus_item.borrow().upgrade().unwrap();
    for reverse in [false, true] {
        for _ in 0..4 {
            if reverse {
                input::chord(&child, &[Key::Shift], Key::Tab);
            } else {
                input::key(&child, Key::Tab);
            }
        }
        assert_eq!(
            inner.focus_item.borrow().upgrade().unwrap(),
            editor,
            "focus must wrap inside the form"
        );
    }
    input::chord(&child, &[Key::Control], "a");
    input::key(&child, destination.to_string_lossy().as_ref());
    let before_retry = windows.borrow().len();
    accepting.set(false);
    input::key(&child, Key::Return);
    ui.draw();
    assert_eq!(
        windows.borrow().len(),
        before_retry,
        "admission error stays in the same editable native draft"
    );
    assert!(ui.window.get_dialog_input_blocked());
    assert!(!ui.window.get_copy_draft_token().is_empty());
    assert!(commands.borrow().is_empty());
    assert!(child.window().is_visible());
    let error = ui.window.get_copy_draft_error();
    assert!(!error.is_empty());
    assert!(
        modeless::accessible_item(
            &child,
            i_slint_core::items::AccessibleRole::Text,
            error.as_str()
        )
        .is_some_and(|item| item.is_visible()),
        "admission error must be visible in the draft"
    );
    assert!(!ui::has_popup(&ui.window));
    // An unrelated asynchronous notice also must not discard the edited target.
    ui.window.set_message_title("Background notice".into());
    ui.window
        .set_message_generation(ui.window.get_message_generation() + 1);
    ui.draw();
    dismiss_native_notice(&windows);
    accepting.set(true);
    input::key(&child, Key::Return);
    ui.draw();
    let queued = commands.borrow();
    assert_eq!(queued.len(), 1);
    let command = if preserve {
        let Command::EnqueueTransferWithMetadata(request) = &queued[0] else {
            panic!("An explicitly checked option must enqueue a required-metadata transfer");
        };
        assert_eq!(
            request.metadata,
            TransferMetadataPolicy::RequireModifiedTime
        );
        assert_eq!(
            request.intent,
            if moving {
                TransferIntent::Move
            } else {
                TransferIntent::Copy
            }
        );
        let MetadataTransferInput::Selected(request) = &request.input else {
            panic!("The captured selection must remain shared");
        };
        assert_eq!(request.destination, destination);
        assert_eq!(
            request.files.fallback.as_ref(),
            Some(&base.join("file.txt"))
        );
        None
    } else {
        Some(&queued[0])
    };
    let request = match command {
        Some(Command::EnqueueSelectedCopy(request)) if !moving => Some(request),
        Some(Command::EnqueueSelectedMove(request)) if moving => Some(request),
        None => None,
        _ => panic!("The unchecked default must enqueue the existing content-only request"),
    };
    if let Some(request) = request {
        assert_eq!(request.destination, destination);
        assert_eq!(
            request.files.fallback.as_ref(),
            Some(&base.join("file.txt"))
        );
    }
    ui.window.hide().unwrap();
}
