#[path = "support/ui.rs"]
mod ui;
use i_slint_core::items::{AccessibleRole, TextInput};
use i_slint_core::{
    input::{InternalKeyEvent, KeyEventType},
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle,
    platform::{Key, PointerEventButton, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

struct Harness {
    ui: ui::TestUi,
    p: Rc<RefCell<Presenter>>,
    sent: Rc<RefCell<Vec<Command>>>,
    accept: Rc<Cell<bool>>,
}
impl Harness {
    fn new() -> Self {
        let ui = ui::TestUi::new();
        let base = std::env::temp_dir().join("paste 资料");
        let p = Rc::new(RefCell::new(Presenter::new(
            base.clone(),
            base.join("right"),
        )));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let events = sent.clone();
        let accept = Rc::new(Cell::new(true));
        let accepts = accept.clone();
        install(
            &ui.window,
            p.clone(),
            Rc::new(move |command| {
                if !accepts.get() {
                    return false;
                }
                events.borrow_mut().push(command);
                true
            }),
        );
        p.borrow_mut().apply(Event::PreferencesLoaded(Ok(Box::new(
            UserPreferences::default(),
        ))));
        refresh(&ui.window, &p.borrow());
        let mut h = Self {
            ui,
            p,
            sent,
            accept,
        };
        h.open();
        h
    }
    fn tick(&mut self) {
        slint::platform::update_timers_and_animations();
        self.ui.draw();
    }
    fn settle(&mut self) {
        for _ in 0..5 {
            std::thread::sleep(Duration::from_millis(10));
            self.tick();
        }
    }
    fn open(&mut self) {
        self.ui.window.invoke_focus_files();
        ui::chord(&self.ui.window, &[Key::Control], "g");
        self.tick();
        self.settle();
        self.sent.borrow_mut().clear();
    }
    fn text(&mut self, text: &str) {
        ui::chord(&self.ui.window, &[Key::Control], "a");
        ui::key(&self.ui.window, text);
        self.settle();
    }
    fn paste(&mut self) -> DesktopRequestId {
        self.sent.borrow_mut().clear();
        ui::chord(&self.ui.window, &[Key::Control], "v");
        self.request()
    }
    fn request(&self) -> DesktopRequestId {
        let ids: Vec<_> = self
            .sent
            .borrow()
            .iter()
            .filter_map(|c| {
                if let Command::ReadPathClipboardText { request } = c {
                    Some(*request)
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(ids.len(), 1, "one explicit text read, no heuristic paste");
        ids[0]
    }
    fn reply(&mut self, id: DesktopRequestId, text: &str) {
        self.p.borrow_mut().apply(Event::PathClipboardTextRead {
            request: id,
            result: Ok(Arc::from(text)),
        });
        refresh(&self.ui.window, &self.p.borrow());
        self.settle();
    }
    fn value(&self) -> String {
        self.ui.window.get_left_path_draft().to_string()
    }
}

#[test]
fn keyboard_paste_replaces_only_selection_keeps_native_undo_and_waits_for_enter() {
    let mut h = Harness::new();
    h.text("beforeSUFFIX");
    let item = ui::root_item(&h.ui.window, "Folder path", AccessibleRole::TextInput);
    let input = item.downcast::<TextInput>().unwrap();
    input
        .as_pin_ref()
        .set_selection_offsets(&item.window_adapter().unwrap(), &item, 0, 6);
    h.settle();
    let id = h.paste();
    let path = format!("资料/e\u{301}{}", std::path::MAIN_SEPARATOR);
    h.reply(id, &path);
    assert_eq!(h.value(), format!("{path}SUFFIX"));
    assert!(
        !h.sent
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::Navigate { .. }))
    );
    ui::chord(&h.ui.window, &[Key::Control], "z");
    ui::chord(&h.ui.window, &[Key::Control], "z");
    assert_eq!(h.value(), "beforeSUFFIX");
    #[cfg(windows)]
    {
        ui::chord(&h.ui.window, &[Key::Control], "y");
        ui::chord(&h.ui.window, &[Key::Control], "y");
    }
    #[cfg(not(windows))]
    {
        ui::chord(&h.ui.window, &[Key::Control, Key::Shift], "z");
        ui::chord(&h.ui.window, &[Key::Control, Key::Shift], "z");
    }
    assert_eq!(h.value(), format!("{path}SUFFIX"));
    ui::key(&h.ui.window, Key::Return);
    assert!(h.sent.borrow().iter().any(|c| matches!(
        c,
        Command::Navigate {
            pane: PaneId::Left,
            ..
        }
    )));
}

#[test]
fn actual_context_menu_paste_uses_same_backend_route_and_preserves_quotes_spaces() {
    let mut h = Harness::new();
    h.text("replace");
    ui::chord(&h.ui.window, &[Key::Control], "a");
    h.settle();
    let position = ui::root_position(&h.ui.window, "Folder path", AccessibleRole::TextInput);
    for down in [true, false] {
        h.ui.window.window().dispatch_event(if down {
            WindowEvent::PointerPressed {
                position,
                button: PointerEventButton::Right,
            }
        } else {
            WindowEvent::PointerReleased {
                position,
                button: PointerEventButton::Right,
            }
        });
    }
    h.tick();
    ui::click_popup(&h.ui.window, "Paste", AccessibleRole::Text);
    let id = h.request();
    let path = format!("\"quoted 资料\" {}", std::path::MAIN_SEPARATOR);
    h.reply(id, &path);
    assert_eq!(h.value(), path);
    assert!(
        !h.sent
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::Navigate { .. }))
    );
}

#[test]
fn late_reply_cannot_overwrite_typing_cancellation_navigation_or_superseding_paste() {
    let mut h = Harness::new();
    h.text("original");
    let old = h.paste();
    ui::key(&h.ui.window, "x");
    h.reply(old, "late/");
    assert_eq!(h.value(), "originalx");
    let old = h.paste();
    ui::key(&h.ui.window, Key::Escape);
    h.reply(old, "cancelled/");
    assert!(!h.value().contains("cancelled"));
    h.open();
    h.text("current");
    let old = h.paste();
    h.ui.window.set_left_path("changed".into());
    h.tick();
    h.reply(old, "obsolete/");
    assert!(!h.value().contains("obsolete"));
    h.open();
    h.text("seed");
    let old = h.paste();
    let new = h.paste();
    h.reply(old, "superseded/");
    assert_eq!(h.value(), "seed");
    h.reply(new, "fresh/");
    assert_eq!(h.value(), "seedfresh/");
}

#[test]
fn shift_insert_empty_paste_is_noop_and_ordinary_typing_is_not_modified() {
    let mut h = Harness::new();
    h.text("literal path ");
    assert_eq!(h.value(), "literal path ");
    assert!(
        !h.sent
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::ReadPathClipboardText { .. }))
    );
    ui::chord(&h.ui.window, &[Key::Shift], Key::Insert);
    let id = h.request();
    h.reply(id, "");
    assert_eq!(h.value(), "literal path ");
}

#[test]
fn rejected_paste_leaves_text_unchanged_and_reports_failure_without_navigation() {
    let mut h = Harness::new();
    let notices = Rc::new(Cell::new(0));
    let captured = notices.clone();
    h.ui.window
        .on_open_notice_window(move || captured.set(captured.get() + 1));
    h.text("unchanged");
    h.sent.borrow_mut().clear();
    h.accept.set(false);
    ui::chord(&h.ui.window, &[Key::Control], "v");
    h.settle();
    assert_eq!(h.value(), "unchanged");
    assert!(h.ui.window.get_message_body().contains("busy"));
    assert_eq!(
        notices.get(),
        1,
        "the failure requests its independent notice window"
    );
    assert!(h.sent.borrow().is_empty());
}

#[test]
fn committed_paste_preserves_private_use_scalars_tabs_and_clears_old_ime_preedit() {
    let mut h = Harness::new();
    h.text("start");
    let item = WindowInner::from_pub(h.ui.window.window())
        .focus_item
        .borrow()
        .upgrade()
        .unwrap();
    let adapter = item.window_adapter().unwrap();
    let event = InternalKeyEvent {
        event_type: KeyEventType::UpdateComposition,
        preedit_text: "かな".into(),
        ..Default::default()
    };
    item.borrow().as_ref().key_event(&event, &adapter, &item);
    h.settle();
    let id = h.paste();
    let path = format!("private\u{f708}🦀\tpath{}", std::path::MAIN_SEPARATOR);
    h.reply(id, &path);
    assert_eq!(h.value(), format!("start{path}"));
    assert!(
        item.downcast::<TextInput>()
            .unwrap()
            .as_pin_ref()
            .preedit_text()
            .is_empty()
    );
}

#[test]
fn pane_switch_cancels_left_reply_and_right_compact_editor_keeps_its_own_payload() {
    let mut h = Harness::new();
    h.text("left");
    let old = h.paste();
    h.ui.window.invoke_activate(1);
    h.ui.window.invoke_focus_files();
    ui::chord(&h.ui.window, &[Key::Control], "g");
    ui::chord(&h.ui.window, &[Key::Control], "a");
    ui::key(&h.ui.window, "right");
    h.settle();
    h.reply(old, "stale/");
    assert_eq!(h.value(), "left");
    h.ui.resize(320, 200);
    h.settle();
    h.sent.borrow_mut().clear();
    ui::chord(&h.ui.window, &[Key::Control], "v");
    let id = h.request();
    let payload = format!(" literal quoted / {}", std::path::MAIN_SEPARATOR);
    h.reply(id, &payload);
    assert_eq!(
        h.ui.window.get_right_path_draft(),
        format!("right{payload}")
    );
    assert!(
        !h.sent
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::Navigate { .. }))
    );
}

#[test]
fn selection_change_and_backend_refusal_cannot_retarget_or_change_the_field() {
    let mut h = Harness::new();
    h.text("unchanged");
    let old = h.paste();
    ui::key(&h.ui.window, Key::Home);
    h.reply(old, "wrong/");
    assert_eq!(h.value(), "unchanged");
    let id = h.paste();
    h.p.borrow_mut().apply(Event::PathClipboardTextRead {
        request: id,
        result: Err(FsError::from_io(
            "clipboard",
            std::path::Path::new(""),
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "Undecodable clipboard text.",
            ),
        )),
    });
    h.settle();
    assert_eq!(h.value(), "unchanged");
    assert!(h.ui.window.get_message_body().contains("Undecodable"));
}

#[test]
fn paste_timer_defers_conservatively_while_presenter_is_borrowed() {
    let mut h = Harness::new();
    h.text("before");
    let id = h.paste();
    h.p.borrow_mut().apply(Event::PathClipboardTextRead {
        request: id,
        result: Ok(Arc::from("after/")),
    });
    let held = h.p.borrow_mut();
    std::thread::sleep(Duration::from_millis(35));
    slint::platform::update_timers_and_animations();
    drop(held);
    h.settle();
    assert_eq!(h.value(), "beforeafter/");
}
