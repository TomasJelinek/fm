#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{ComponentHandle, platform::Key};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
};

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
    let mut ui = ui::TestUi::new();
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
    let option = ui::root_item(
        &ui.window,
        "Preserve exact modified time",
        AccessibleRole::Checkbox,
    );
    assert_eq!(
        option
            .accessible_string_property(AccessibleStringProperty::Enabled)
            .unwrap(),
        "false"
    );
    ui::click_root(
        &ui.window,
        "Preserve exact modified time",
        AccessibleRole::Checkbox,
    );
    assert_eq!(
        option
            .accessible_string_property(AccessibleStringProperty::Checked)
            .unwrap(),
        "false"
    );
    ui.window.invoke_focus_files();
    ui::key(&ui.window, Key::Return);
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
    let mut ui = ui::TestUi::new();
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
                .starts_with("Up to 64 regular files")
        );
        assert!(
            ui.window
                .get_copy_draft_body()
                .contains("Source disk space is not freed")
        );
        assert!(
            ui.window
                .get_copy_draft_body()
                .contains("Up to 64 regular files")
        );
    }
    assert!(commands.borrow().is_empty());
    render::snapshot(
        &ui,
        if moving {
            "move-recovery-dialog"
        } else {
            "copy-dialog"
        },
    );
    if moving {
        ui.resize(320, 200);
        render::snapshot(&ui, "move-recovery-dialog-small");
    }
    ui::key(&ui.window, Key::Escape);
    ui.draw();
    assert!(ui.window.get_copy_draft_token().is_empty());
    assert!(commands.borrow().is_empty());

    ui::key(&ui.window, shortcut);
    ui.draw();
    let option = ui::root_item(
        &ui.window,
        "Preserve exact modified time",
        i_slint_core::items::AccessibleRole::Checkbox,
    );
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
        ui::click_root(
            &ui.window,
            "Preserve exact modified time",
            i_slint_core::items::AccessibleRole::Checkbox,
        );
        // Restore path focus without activating the form or changing the option.
        ui.window.invoke_focus_files();
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
    let inner = i_slint_core::window::WindowInner::from_pub(ui.window.window());
    let editor = inner.focus_item.borrow().upgrade().unwrap();
    for reverse in [false, true] {
        for _ in 0..4 {
            if reverse {
                ui::chord(&ui.window, &[Key::Shift], Key::Tab);
            } else {
                ui::key(&ui.window, Key::Tab);
            }
        }
        assert_eq!(
            inner.focus_item.borrow().upgrade().unwrap(),
            editor,
            "focus must wrap inside the form"
        );
    }
    ui::chord(&ui.window, &[Key::Control], "a");
    ui::key(&ui.window, destination.to_string_lossy().as_ref());
    accepting.set(false);
    ui::key(&ui.window, Key::Return);
    ui.draw();
    assert!(ui::has_popup(&ui.window), "admission error notice");
    assert!(!ui.window.get_copy_draft_token().is_empty());
    assert!(commands.borrow().is_empty());
    ui::key(&ui.window, Key::Escape);
    ui.draw();
    assert!(!ui::has_popup(&ui.window));
    // An unrelated asynchronous notice also must not discard the edited target.
    ui.window.set_message_title("Background notice".into());
    ui.window
        .set_message_generation(ui.window.get_message_generation() + 1);
    ui.draw();
    ui::key(&ui.window, Key::Escape);
    accepting.set(true);
    ui::key(&ui.window, Key::Return);
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
                TransferIntent::RecoverableMove
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
