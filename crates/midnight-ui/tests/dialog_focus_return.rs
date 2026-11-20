//! A dialog opened with the pointer (a toolbar button, the drive bar's filter, the
//! Hot Paths menu) gives the keyboard back to the active file list when it closes,
//! by Escape or by its header ×, as one opened by its key does: Down then moves the
//! cursor, and a single click on a row focuses the list. The control that opened it
//! never keeps or takes back the keyboard, nor does a toolbar list closed by a press
//! outside it.
use crate::support::modeless;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::AccessibleRole,
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{
    FileListPresentation, HotPathsPresentation, MainWindow, Presenter, install, refresh,
};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc, sync::Arc};

const NAMES: [&str; 4] = ["a.txt", "b.txt", "c.txt", "d.txt"];

struct Harness {
    main: MainWindow,
    native: Rc<MinimalSoftwareWindow>,
    /// Every native window the app created, the main window first.
    windows: Rc<RefCell<Vec<Rc<MinimalSoftwareWindow>>>>,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
}

#[derive(Clone, Copy, Debug)]
enum Dialog {
    Attributes,
    FileList,
    Properties,
    Filter,
    HotPaths,
}

#[derive(Clone, Copy, Debug)]
enum Close {
    Escape,
    Header,
}

impl Harness {
    /// The installed window with the left panel listing four files.
    fn new() -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        main.show().unwrap();
        let native = windows.borrow()[0].clone();
        main.window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        let root = std::env::temp_dir().join("dialog focus return");
        let left = root.join("left");
        let state = Rc::new(RefCell::new(Presenter::new(
            left.clone(),
            root.join("right"),
        )));
        state
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::default())));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sent = commands.clone();
        install(
            &main,
            state.clone(),
            Rc::new(move |command| {
                sent.borrow_mut().push(command);
                true
            }),
        );
        let listings: Vec<_> = commands
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::Navigate {
                    pane, navigation, ..
                } => Some((*pane, *navigation)),
                _ => None,
            })
            .collect();
        assert_eq!(listings.len(), 2, "both panels are listed at start");
        for (pane, navigation) in listings {
            let entries: Vec<Entry> = if pane == PaneId::Left {
                NAMES
                    .iter()
                    .enumerate()
                    .map(|(index, name)| {
                        let mut entry =
                            Entry::new(EntryId(index as u64 + 1), left.join(name), EntryKind::File);
                        entry.metadata.size = Some(5);
                        entry
                    })
                    .collect()
            } else {
                Vec::new()
            };
            state.borrow_mut().apply(Event::ListingFinished {
                pane,
                navigation,
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries: entries.into(),
            });
        }
        refresh(&main, &state.borrow());
        commands.borrow_mut().clear();
        let harness = Self {
            main,
            native,
            windows,
            state,
            commands,
        };
        harness.settle();
        harness
    }

    /// Run due timers, deferred focus restoration and change handlers, then lay out.
    fn settle(&self) {
        for _ in 0..3 {
            slint::platform::update_timers_and_animations();
        }
        refresh(&self.main, &self.state.borrow());
        slint::platform::update_timers_and_animations();
        modeless::layout(&self.native, 1000, 700);
    }

    fn key(&self, key: Key) {
        let text: slint::SharedString = key.into();
        let window = self.main.window();
        window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        window.dispatch_event(WindowEvent::KeyReleased { text });
        self.settle();
    }

    /// The control with this role whose label starts with `label`, in the topmost
    /// form or popup first.
    fn item(&self, role: AccessibleRole, label: &str) -> Option<ItemRc> {
        fn find(item: ItemRc, role: AccessibleRole, label: &str) -> Option<ItemRc> {
            if item.accessible_role() == role
                && item.is_visible()
                && item
                    .accessible_string_property(AccessibleStringProperty::Label)
                    .is_some_and(|value| value.starts_with(label))
            {
                return Some(item);
            }
            let mut child = item.first_child();
            while let Some(item) = child {
                child = item.next_sibling();
                if let Some(found) = find(item, role, label) {
                    return Some(found);
                }
            }
            None
        }
        let inner = WindowInner::from_pub(self.main.window());
        let popups: Vec<_> = inner
            .active_popups()
            .iter()
            .rev()
            .filter(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .map(|popup| ItemRc::new_root(popup.component.clone()))
            .collect();
        popups
            .into_iter()
            .chain([ItemRc::new_root(inner.component())])
            .find_map(|root| find(root, role, label))
    }

    fn click(&self, role: AccessibleRole, label: &str) {
        let item = self
            .item(role, label)
            .unwrap_or_else(|| panic!("no {role:?} {label:?} to click"));
        modeless::click_item(&self.native, &item, label);
        self.settle();
    }

    fn popups(&self) -> usize {
        WindowInner::from_pub(self.main.window())
            .active_popups()
            .iter()
            .filter(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .count()
    }

    /// Whether an open popup covers the centre of `item`.
    fn covered(&self, item: &ItemRc) -> bool {
        let centre = |item: &ItemRc| {
            let rect = item.geometry();
            let origin = item.map_to_native_window(rect.origin);
            (
                origin.x + rect.size.width / 2.0,
                origin.y + rect.size.height / 2.0,
            )
        };
        let (x, y) = centre(item);
        WindowInner::from_pub(self.main.window())
            .active_popups()
            .iter()
            .filter(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .any(|popup| {
                let root = ItemRc::new_root(popup.component.clone());
                let rect = root.geometry();
                let origin = root.map_to_native_window(rect.origin);
                x >= origin.x
                    && x < origin.x + rect.size.width
                    && y >= origin.y
                    && y < origin.y + rect.size.height
            })
    }

    /// The name in the left list's cursor row, as the window shows it.
    fn cursor_name(&self) -> Option<String> {
        use slint::Model;
        let row = usize::try_from(self.main.get_left_cursor()).ok()?;
        self.main
            .get_left_rows()
            .row_data(row)
            .map(|row| row.name.to_string())
    }

    fn open(&self, dialog: Dialog) {
        match dialog {
            Dialog::Attributes => self.click(AccessibleRole::Button, "Attributes, times and case"),
            Dialog::FileList => self.click(AccessibleRole::Button, "Make File List"),
            Dialog::Properties => {
                self.click(AccessibleRole::Button, "Properties");
                let request = self
                    .commands
                    .borrow()
                    .iter()
                    .find_map(|command| match command {
                        Command::ShowSelectionProperties { request, .. } => Some(*request),
                        _ => None,
                    })
                    .expect("Properties asks the backend for the summary");
                self.state.borrow_mut().apply(Event::DesktopActionFinished {
                    request,
                    result: Ok(DesktopActionResult::PropertiesInfo {
                        title: "a.txt".into(),
                        body: "File · 5 bytes".into(),
                    }),
                });
                self.settle();
            }
            Dialog::Filter => self.click(AccessibleRole::Button, "Left panel filter"),
            Dialog::HotPaths => {
                self.click(AccessibleRole::Combobox, "Hot Paths");
                self.click(AccessibleRole::Button, "Manage Hot Paths…");
            }
        }
        assert!(self.is_open(dialog), "{dialog:?} opened from the pointer");
    }

    fn is_open(&self, dialog: Dialog) -> bool {
        match dialog {
            Dialog::Attributes => !self.main.get_metadata_token().is_empty(),
            Dialog::FileList => self.main.global::<FileListPresentation>().get_active(),
            Dialog::Properties => self.main.get_embedded_modal_open(),
            Dialog::Filter => self.main.get_panel_filter_view().open,
            Dialog::HotPaths => !self
                .main
                .global::<HotPathsPresentation>()
                .get_token()
                .is_empty(),
        }
    }

    fn close(&self, dialog: Dialog, close: Close) {
        match close {
            Close::Escape => self.key(Key::Escape),
            Close::Header => self.click(
                AccessibleRole::Button,
                match dialog {
                    Dialog::Attributes | Dialog::Filter => "Cancel dialog",
                    Dialog::FileList => "Cancel file list",
                    Dialog::Properties | Dialog::HotPaths => "Close dialog",
                },
            ),
        }
        assert!(!self.is_open(dialog), "{dialog:?} closed by {close:?}");
        assert_eq!(self.popups(), 0, "{dialog:?} left no popup behind");
    }
}

/// Every pointer-opened dialog, closed both ways: right after the close the
/// keyboard is in the left list (Down moves its cursor), and one click on the row
/// under the cursor keeps it there.
#[test]
fn a_dialog_opened_with_the_pointer_returns_the_keyboard_to_the_file_list() {
    let h = Harness::new();
    for dialog in [
        Dialog::Attributes,
        Dialog::FileList,
        Dialog::Properties,
        Dialog::Filter,
        Dialog::HotPaths,
    ] {
        for close in [Close::Escape, Close::Header] {
            let what = format!("{dialog:?} closed by {close:?}");
            h.click(AccessibleRole::ListItem, "a.txt");
            assert_eq!(
                h.main.get_focused_pane(),
                0,
                "{what}: the click focused the list"
            );
            assert_eq!(h.cursor_name().as_deref(), Some("a.txt"));
            h.commands.borrow_mut().clear();
            h.open(dialog);
            h.close(dialog, close);
            match close {
                // The keyboard is back at once: Down moves the cursor.
                Close::Escape => {
                    assert_eq!(
                        h.main.get_focused_pane(),
                        0,
                        "{what}: the keyboard is back in the file list"
                    );
                    h.key(Key::DownArrow);
                    assert_eq!(h.cursor_name().as_deref(), Some("b.txt"), "{what}: Down");
                }
                // One click on the row under the cursor focuses the list.
                Close::Header => {
                    h.click(AccessibleRole::ListItem, "a.txt");
                    assert_eq!(
                        h.main.get_focused_pane(),
                        0,
                        "{what}: a single click focuses the list"
                    );
                    h.key(Key::DownArrow);
                    assert_eq!(h.cursor_name().as_deref(), Some("b.txt"), "{what}: Down");
                }
            }
        }
    }
}

/// The Hot Paths list closed by a press outside it (on a file row, which that press
/// does not reach) gives the keyboard to the file list, as Escape does: Down then
/// moves the cursor instead of opening the list again, and the next click on a row
/// selects it.
#[test]
fn a_toolbar_list_closed_by_a_press_outside_it_returns_the_keyboard_to_the_file_list() {
    let h = Harness::new();
    h.click(AccessibleRole::ListItem, "a.txt");
    assert_eq!(h.main.get_focused_pane(), 0);
    h.click(AccessibleRole::Combobox, "Hot Paths");
    assert_eq!(h.popups(), 1, "the Hot Paths list is open");
    let row = ["d.txt", "c.txt", "b.txt"].into_iter().find_map(|name| {
        h.item(AccessibleRole::ListItem, name)
            .filter(|item| !h.covered(item))
            .map(|item| (name, item))
    });
    let (name, row) = row.expect("a file row outside the Hot Paths list");
    modeless::click_item(&h.native, &row, name);
    h.settle();
    assert_eq!(h.popups(), 0, "the press closed the list");
    assert_eq!(
        h.cursor_name().as_deref(),
        Some("a.txt"),
        "the press only closed the list"
    );
    assert_eq!(
        h.main.get_focused_pane(),
        0,
        "the keyboard is back in the file list"
    );
    h.key(Key::DownArrow);
    assert_eq!(h.popups(), 0, "Down does not open the list again");
    assert_eq!(h.cursor_name().as_deref(), Some("b.txt"), "Down");
    h.click(AccessibleRole::ListItem, name);
    assert_eq!(h.cursor_name().as_deref(), Some(name), "one click selects");
    assert_eq!(h.main.get_focused_pane(), 0);
}

/// A tool window opened from the toolbar (Find) gives the keyboard back to the file
/// list once it closes and the main window is active again. Closing it restores the
/// Find button that opened it, which keeps nothing.
#[test]
fn a_tool_window_opened_from_the_toolbar_returns_the_keyboard_to_the_file_list() {
    let h = Harness::new();
    h.click(AccessibleRole::ListItem, "a.txt");
    h.click(AccessibleRole::Button, "Find files and text");
    let find = h.windows.borrow().last().cloned().expect("a native window");
    assert!(!Rc::ptr_eq(&find, &h.native), "Find opened its own window");
    // The window manager activates the new window, and the main window again once the
    // tool window closes.
    h.main
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(false));
    find.window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    find.window().dispatch_event(WindowEvent::CloseRequested);
    h.settle();
    h.main
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    h.settle();
    assert_eq!(
        h.main.get_focused_pane(),
        0,
        "the keyboard is back in the file list"
    );
    h.key(Key::DownArrow);
    assert_eq!(h.cursor_name().as_deref(), Some("b.txt"), "Down");
}
