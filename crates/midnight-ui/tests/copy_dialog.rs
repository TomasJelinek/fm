#[path = "support/window_input.rs"]
mod input;
#[allow(dead_code)]
#[path = "support/modeless.rs"]
mod modeless;
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable, TextInput},
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{
        Clipboard, Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::MinimalSoftwareWindow,
    },
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

type NativeWindows = Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>;

struct ClipboardWindows {
    windows: NativeWindows,
    clipboard: Rc<RefCell<String>>,
}

impl Platform for ClipboardWindows {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        modeless::Windows(self.windows.clone()).create_window_adapter()
    }

    fn clipboard_text(&self, clipboard: Clipboard) -> Option<String> {
        matches!(clipboard, Clipboard::DefaultClipboard).then(|| self.clipboard.borrow().clone())
    }

    fn set_clipboard_text(&self, text: &str, clipboard: Clipboard) {
        if matches!(clipboard, Clipboard::DefaultClipboard) {
            *self.clipboard.borrow_mut() = text.to_owned();
        }
    }
}

fn multi_window_ui() -> (ui::TestUi, NativeWindows) {
    let (test, windows, _) = multi_window_ui_with_clipboard();
    (test, windows)
}

fn multi_window_ui_with_clipboard() -> (ui::TestUi, NativeWindows, Rc<RefCell<String>>) {
    let windows = Rc::new(RefCell::new(Vec::new()));
    let clipboard = Rc::new(RefCell::new(String::new()));
    slint::platform::set_platform(Box::new(ClipboardWindows {
        windows: windows.clone(),
        clipboard: clipboard.clone(),
    }))
    .unwrap();
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
    (test, windows, clipboard)
}

fn accessible_item(
    window: &MinimalSoftwareWindow,
    role: i_slint_core::items::AccessibleRole,
    label: &str,
) -> Option<i_slint_core::item_tree::ItemRc> {
    modeless::accessible_popup_item(window, role, label)
        .or_else(|| modeless::accessible_item(window, role, label))
}

fn draft_child(windows: &NativeWindows) -> Rc<MinimalSoftwareWindow> {
    slint::platform::update_timers_and_animations();
    assert_eq!(
        windows.borrow().len(),
        1,
        "copy draft created a native window"
    );
    let main = windows.borrow()[0].clone();
    assert!(
        !i_slint_core::window::WindowInner::from_pub(main.window())
            .active_popups()
            .is_empty()
    );
    modeless::draw(&main, 720, 520, "transfer-draft");
    main
}

fn dismiss_native_notice(windows: &NativeWindows) {
    let main = windows.borrow()[0].clone();
    let count = i_slint_core::window::WindowInner::from_pub(main.window())
        .active_popups()
        .len();
    assert!(count >= 2, "notice should overlay the retained copy draft");
    input::key(&main, Key::Escape);
    slint::platform::update_timers_and_animations();
    assert!(main.window().is_visible());
    assert_eq!(
        i_slint_core::window::WindowInner::from_pub(main.window())
            .active_popups()
            .len(),
        count - 1
    );
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
        let source = accessible_item(&child, AccessibleRole::Text, body).unwrap();
        let label = accessible_item(&child, AccessibleRole::Text, "Destination folder").unwrap();
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
            accessible_item(&child, AccessibleRole::TextInput, "Destination folder").unwrap();
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
        let item = accessible_item(&child, role, label).unwrap();
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

#[test]
fn compact_copy_controls_remain_reachable_with_large_touch_font() {
    compact_transfer_controls(Key::F5, false);
}

#[test]
fn compact_move_controls_remain_reachable_with_large_touch_font() {
    compact_transfer_controls(Key::F6, true);
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
    let option = accessible_item(
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
    let editor = accessible_item(&child, AccessibleRole::TextInput, "Destination folder").unwrap();
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
    let option = accessible_item(
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
        let editor = accessible_item(
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
        "admission error stays in the same editable embedded draft"
    );
    assert!(ui.window.get_embedded_modal_open());
    assert!(
        !ui.window.get_dialog_input_blocked(),
        "a rejected draft must keep main input enabled"
    );
    assert!(!ui.window.get_copy_draft_token().is_empty());
    assert!(commands.borrow().is_empty());
    assert!(child.window().is_visible());
    let error = ui.window.get_copy_draft_error();
    assert!(!error.is_empty());
    assert!(
        accessible_item(
            &child,
            i_slint_core::items::AccessibleRole::Text,
            error.as_str()
        )
        .is_some_and(|item| item.is_visible()),
        "admission error must be visible in the draft"
    );
    assert!(
        ui::has_popup(&ui.window),
        "admission error retains the same embedded draft"
    );
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

fn compact_settle(test: &mut ui::TestUi) {
    for _ in 0..3 {
        test.draw();
    }
}

fn compact_item(test: &ui::TestUi, role: AccessibleRole, label: &str) -> ItemRc {
    modeless::accessible_popup_item(&test.native, role, label)
        .unwrap_or_else(|| panic!("missing compact transfer control: {label}"))
}

fn compact_bounds(item: &ItemRc, indicator: bool) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    // The checkbox caption is deliberately wider than this viewport. Its left
    // indicator and full vertical span must remain visible and actionable.
    let width = if indicator {
        rect.width().min(24.0)
    } else {
        rect.width()
    };
    (
        origin.x,
        origin.y,
        origin.x + width,
        origin.y + rect.height(),
    )
}

fn compact_clip_ancestors(item: &ItemRc) -> Vec<ItemRc> {
    let mut clips = Vec::new();
    let mut parent = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if ancestor.borrow().as_ref().clips_children() {
            clips.push(ancestor.clone());
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    clips
}

fn compact_contains(outer: (f32, f32, f32, f32), inner: (f32, f32, f32, f32)) -> bool {
    inner.0 >= outer.0 - 0.5
        && inner.1 >= outer.1 - 0.5
        && inner.2 <= outer.2 + 0.5
        && inner.3 <= outer.3 + 0.5
}

fn compact_assert_visible(test: &ui::TestUi, item: &ItemRc, indicator: bool, label: &str) {
    let area = compact_bounds(item, indicator);
    assert!(area.2 > area.0 && area.3 > area.1, "{label} has no area");
    let clips = compact_clip_ancestors(item);
    assert!(!clips.is_empty(), "{label} lost its clipping dialog frame");
    for clip in clips {
        let bounds = compact_bounds(&clip, false);
        assert!(
            compact_contains(bounds, area),
            "{label} is clipped by its dialog/scroll ancestor: control={area:?}, clip={bounds:?}"
        );
    }
    assert!(
        compact_contains((0.0, 0.0, test.width as f32, test.height as f32), area),
        "{label} extends beyond the main viewport: {area:?}"
    );
    assert!(item.is_visible(), "{label} is hidden");
}

fn compact_fixed_controls(test: &ui::TestUi, action: &str) -> Vec<(ItemRc, (f32, f32, f32, f32))> {
    // Check the footer first: the old layout exceeds DialogFrame's body clip
    // while the main-sized popup and semantic primary-focus proxy still exist.
    [
        (AccessibleRole::Button, "Cancel"),
        (AccessibleRole::None, action),
        (AccessibleRole::Button, action),
        (AccessibleRole::Text, action),
        (AccessibleRole::Button, "Cancel dialog"),
    ]
    .into_iter()
    .map(|(role, label)| {
        let item = compact_item(test, role, label);
        compact_assert_visible(test, &item, false, label);
        let bounds = compact_bounds(&item, false);
        (item, bounds)
    })
    .collect()
}

fn compact_assert_stationary(test: &ui::TestUi, fixed: &[(ItemRc, (f32, f32, f32, f32))]) {
    for (item, before) in fixed {
        compact_assert_visible(test, item, false, "fixed header/footer");
        let after = compact_bounds(item, false);
        assert!(
            compact_contains(*before, after) && compact_contains(after, *before),
            "scrolling the form moved its fixed header/footer: {before:?} -> {after:?}"
        );
    }
}

fn compact_reveal(test: &mut ui::TestUi, item: &ItemRc, indicator: bool, label: &str) -> usize {
    let ancestors = compact_clip_ancestors(item);
    let mut wheels = 0;
    for viewport in ancestors.iter().rev() {
        let Some(flickable) = viewport.downcast::<Flickable>() else {
            continue;
        };
        for _ in 0..32 {
            let mut clip = compact_bounds(viewport, false);
            for ancestor in compact_clip_ancestors(viewport) {
                let bounds = compact_bounds(&ancestor, false);
                clip.0 = clip.0.max(bounds.0);
                clip.1 = clip.1.max(bounds.1);
                clip.2 = clip.2.min(bounds.2);
                clip.3 = clip.3.min(bounds.3);
            }
            if flickable.as_pin_ref().content_width().get() > viewport.geometry().width() + 0.5 {
                clip.3 -= 14.0;
            }
            let area = compact_bounds(item, indicator);
            let delta = if area.1 < clip.1 - 0.5 {
                clip.1 - area.1
            } else if area.3 > clip.3 + 0.5 {
                clip.3 - area.3
            } else {
                break;
            };
            assert!(
                clip.2 - clip.0 > 16.0 && clip.3 - clip.1 > 16.0,
                "{label} has no visible wheel target: {clip:?}"
            );
            test.window
                .window()
                .dispatch_event(WindowEvent::PointerScrolled {
                    position: slint::LogicalPosition::new(clip.0 + 8.0, clip.3 - 8.0),
                    delta_x: 0.0,
                    delta_y: delta.clamp(-120.0, 120.0),
                });
            wheels += 1;
            compact_settle(test);
        }
    }
    compact_assert_visible(test, item, indicator, label);
    wheels
}

fn compact_assert_focus(test: &ui::TestUi, control: &ItemRc) {
    let mut focused = WindowInner::from_pub(test.window.window())
        .focus_item
        .borrow()
        .upgrade();
    while let Some(item) = focused {
        if &item == control {
            return;
        }
        focused = item.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    panic!("keyboard focus did not reach the compact transfer control");
}

fn compact_assert_edit(input: &ItemRc, value: &str, caret: usize) {
    let input = input
        .downcast::<TextInput>()
        .expect("destination text input");
    let input = input.as_pin_ref();
    assert_eq!(input.text().as_str(), value);
    assert_eq!(input.cursor_position_byte_offset() as usize, caret);
    assert_eq!(input.anchor_position_byte_offset() as usize, caret);
}

fn compact_transfer_controls(shortcut: Key, moving: bool) {
    let (mut test, windows, clipboard) = multi_window_ui_with_clipboard();
    let base = std::env::temp_dir().join("compact transfer 文档");
    let paths = [base.join("source"), base.join("destination")];
    let source = paths[0].join("captured native file 文档.txt");
    let state = Rc::new(RefCell::new(Presenter::new(
        paths[0].clone(),
        paths[1].clone(),
    )));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new({
        let commands = commands.clone();
        move |command| {
            commands.borrow_mut().push(command);
            true
        }
    });
    install(&test.window, state.clone(), send.clone());
    let mut preferences = UserPreferences {
        density: UiDensity::Touch,
        desktop_font_size: 32,
        touch_font_size: 32,
        ..Default::default()
    };
    for (slot, path) in paths.iter().enumerate() {
        preferences.tabs[slot].paths = vec![path.clone()];
    }
    state.borrow_mut().apply_with_sender(
        Event::PreferencesLoaded(Ok(Box::new(preferences))),
        send.as_ref(),
    );
    for pane in [PaneId::Left, PaneId::Right] {
        let navigation = commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: target,
                    navigation,
                    ..
                } if *target == pane => Some(*navigation),
                _ => None,
            })
            .expect("preferences must request the latest pane listing");
        let entries: Arc<[Entry]> = if pane == PaneId::Left {
            vec![Entry::new(EntryId(1), source.clone(), EntryKind::File)].into()
        } else {
            Arc::from([])
        };
        state.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
    }
    refresh(&test.window, &state.borrow());
    test.window.invoke_activate(0);
    test.window.invoke_focus_files();
    ui::key(&test.window, Key::End);
    test.resize(320, 200);
    compact_settle(&mut test);
    assert_eq!(test.window.get_density_index(), 1);
    assert_eq!(test.window.get_touch_font_size(), 32);
    commands.borrow_mut().clear();
    let action = if moving { "Move" } else { "Copy" };

    ui::key(&test.window, shortcut);
    compact_settle(&mut test);
    assert!(!test.window.get_copy_draft_token().is_empty());
    assert_eq!(test.window.get_copy_draft_move(), moving);
    assert_eq!(windows.borrow().len(), 1, "draft created a native window");
    assert!(test.window.get_embedded_modal_open());
    assert!(!test.window.get_dialog_input_blocked());
    let fixed = compact_fixed_controls(&test, action);
    let editor = compact_item(&test, AccessibleRole::TextInput, "Destination folder");
    let option = compact_item(
        &test,
        AccessibleRole::Checkbox,
        "Preserve exact modified time",
    );
    let before = compact_bounds(&editor, false);
    let mut wheels = compact_reveal(&mut test, &editor, false, "Destination folder");
    let mut moved = before != compact_bounds(&editor, false);
    compact_assert_stationary(&test, &fixed);
    let before = compact_bounds(&option, true);
    wheels += compact_reveal(&mut test, &option, true, "metadata checkbox indicator");
    moved |= before != compact_bounds(&option, true);
    assert!(
        wheels > 0 && moved,
        "the compact middle must be reachable through wheel input"
    );
    compact_assert_stationary(&test, &fixed);
    let indicator = compact_bounds(&option, true);
    ui::click(
        &test.window,
        (indicator.0 + indicator.2) / 2.0,
        (indicator.1 + indicator.3) / 2.0,
    );
    compact_settle(&mut test);
    assert_eq!(
        option
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap(),
        "true"
    );
    let cancel = compact_item(&test, AccessibleRole::Button, "Cancel");
    compact_assert_visible(&test, &cancel, false, "pointer Cancel");
    input::click(&test.native, &cancel);
    compact_settle(&mut test);
    assert!(test.window.get_copy_draft_token().is_empty());
    assert!(!test.window.get_embedded_modal_open());
    assert!(
        commands.borrow().is_empty(),
        "pointer Cancel admitted a request"
    );
    assert_eq!(test.window.get_operations().row_count(), 0);
    assert!(!test.window.get_operations_visible());

    ui::key(&test.window, shortcut);
    compact_settle(&mut test);
    assert!(!test.window.get_copy_draft_token().is_empty());
    assert!(test.window.get_embedded_modal_open());
    compact_assert_focus(
        &test,
        &compact_item(&test, AccessibleRole::TextInput, "Destination folder"),
    );
    input::key(&test.native, Key::Escape);
    compact_settle(&mut test);
    assert!(test.window.get_copy_draft_token().is_empty());
    assert!(!test.window.get_embedded_modal_open());
    assert!(commands.borrow().is_empty(), "Escape admitted a request");
    assert_eq!(test.window.get_operations().row_count(), 0);
    assert!(!test.window.get_operations_visible());

    ui::key(&test.window, shortcut);
    compact_settle(&mut test);
    assert_eq!(windows.borrow().len(), 1);
    let token = test.window.get_copy_draft_token();
    assert!(!token.is_empty());
    compact_fixed_controls(&test, action);
    let editor = compact_item(&test, AccessibleRole::TextInput, "Destination folder");
    let option = compact_item(
        &test,
        AccessibleRole::Checkbox,
        "Preserve exact modified time",
    );
    assert_eq!(
        option
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap(),
        "false"
    );
    compact_reveal(&mut test, &editor, false, "Destination folder");
    input::click(&test.native, &editor);
    let destination = base
        .join("edited target e\u{301} 文档")
        .to_str()
        .expect("the Unicode fixture destination")
        .to_owned();
    input::chord(&test.native, &[Key::Control], "a");
    // Key-event text is NFC-normalized by the pinned Slint input dispatcher.
    // A pasted native path must retain its actual decomposed Unicode bytes.
    *clipboard.borrow_mut() = destination.clone();
    input::chord(&test.native, &[Key::Control], "v");
    input::key(&test.native, Key::LeftArrow);
    compact_settle(&mut test);
    let input = WindowInner::from_pub(test.window.window())
        .focus_item
        .borrow()
        .upgrade()
        .unwrap();
    let caret = destination.char_indices().next_back().unwrap().0;
    compact_assert_edit(&input, &destination, caret);
    input::key(&test.native, Key::Tab);
    compact_settle(&mut test);
    compact_assert_focus(&test, &option);
    // Keyboard focus itself must reveal this control; a preparatory wheel here
    // would hide a broken focus/scroll interaction.
    compact_assert_visible(&test, &option, true, "keyboard metadata checkbox indicator");
    input::key(&test.native, Key::Space);
    compact_settle(&mut test);
    assert_eq!(
        option
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap(),
        "true"
    );
    refresh(&test.window, &state.borrow());
    compact_settle(&mut test);
    test.resize(390, 300);
    compact_settle(&mut test);
    test.resize(320, 200);
    compact_settle(&mut test);
    assert_eq!(test.window.get_copy_draft_token(), token);
    assert_eq!(
        compact_item(&test, AccessibleRole::TextInput, "Destination folder"),
        editor
    );
    compact_assert_edit(&input, &destination, caret);
    assert_eq!(
        option
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap(),
        "true"
    );
    compact_assert_focus(&test, &option);
    input::chord(&test.native, &[Key::Shift], Key::Tab);
    compact_settle(&mut test);
    compact_assert_visible(&test, &editor, false, "keyboard Destination folder");
    assert_eq!(
        WindowInner::from_pub(test.window.window())
            .focus_item
            .borrow()
            .upgrade(),
        Some(input.clone())
    );
    input::key(&test.native, "X");
    let mut edited = destination;
    edited.insert(caret, 'X');
    compact_assert_edit(&input, &edited, caret + 1);
    compact_fixed_controls(&test, action);
    for _ in 0..3 {
        input::key(&test.native, Key::Tab);
        compact_settle(&mut test);
    }
    compact_assert_focus(&test, &compact_item(&test, AccessibleRole::Button, action));
    assert!(
        commands.borrow().is_empty(),
        "editing admitted a request before confirmation"
    );
    input::key(&test.native, Key::Return);
    compact_settle(&mut test);
    assert!(test.window.get_copy_draft_token().is_empty());
    let queued = commands.borrow();
    let [Command::EnqueueTransferWithMetadata(request)] = queued.as_slice() else {
        panic!(
            "metadata confirmation must admit exactly one captured {action} request: {queued:?}"
        );
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
        panic!("metadata confirmation changed its captured source kind");
    };
    assert_eq!(request.destination, std::path::PathBuf::from(edited));
    assert_eq!(request.files.fallback.as_ref(), Some(&source));
    test.window.hide().unwrap();
}
