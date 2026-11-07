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
    destination_editor(Key::F5, false);
}
#[test]
fn f6_move_confirmation_consumes_enter_escape_and_busy_queue_without_using_clipboard() {
    destination_editor(Key::F6, true);
}
fn destination_editor(shortcut: Key, moving: bool) {
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
    let destination = base.join("another target");
    // Both ends wrap inside the modal form: path -> Cancel -> Copy -> path.
    let inner = i_slint_core::window::WindowInner::from_pub(ui.window.window());
    let editor = inner.focus_item.borrow().upgrade().unwrap();
    for reverse in [false, true] {
        for _ in 0..3 {
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
    let request = match &queued[0] {
        Command::EnqueueSelectedCopy(request) if !moving => request,
        Command::EnqueueSelectedMove(request) if moving => request,
        _ => panic!("confirmed destination must enqueue the captured selection directly"),
    };
    assert_eq!(request.destination, destination);
    assert_eq!(
        request.files.fallback.as_ref(),
        Some(&base.join("file.txt"))
    );
    ui.window.hide().unwrap();
}
