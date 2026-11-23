//! The path bar's edit routes: a click anywhere right of the path components opens
//! the editor, and the editor's Go button does what Enter does, for a folder, a file
//! (its folder opens with the cursor on it) and a missing path (the same error).
//! Escape cancels. Clicks use the window geometry of the real layout.
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::AccessibleRole,
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{Key, WindowEvent},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

/// Window rectangle: left, top, width, height.
type Rect = (f32, f32, f32, f32);

fn bounds(item: &ItemRc) -> Rect {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.width(), rect.height())
}

fn walk(item: &ItemRc, found: &mut dyn FnMut(&ItemRc)) {
    found(item);
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        walk(&current, found);
    }
}

/// The shown controls of the main window with this label and role, left to right.
fn shown(window: &midnight_ui::MainWindow, label: &str, role: AccessibleRole) -> Vec<ItemRc> {
    let mut items = Vec::new();
    walk(
        &ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        &mut |item| {
            if item.accessible_role() == role
                && item
                    .accessible_string_property(AccessibleStringProperty::Label)
                    .is_some_and(|value| value == label)
                && item.is_visible()
            {
                items.push(item.clone());
            }
        },
    );
    items.sort_by(|a, b| bounds(a).0.total_cmp(&bounds(b).0));
    items
}

fn editing(window: &midnight_ui::MainWindow) -> bool {
    !shown(window, "Go", AccessibleRole::Button).is_empty()
}

struct Harness {
    ui: ui::TestUi,
    p: Rc<RefCell<Presenter>>,
    sent: Rc<RefCell<Vec<Command>>>,
}

impl Harness {
    fn new(start: PathBuf) -> Self {
        let ui = ui::TestUi::new();
        let p = Rc::new(RefCell::new(Presenter::new(
            start.clone(),
            start.join("right"),
        )));
        let sent = Rc::new(RefCell::new(Vec::new()));
        let commands = sent.clone();
        install(
            &ui.window,
            p.clone(),
            Rc::new(move |command| {
                commands.borrow_mut().push(command);
                true
            }),
        );
        p.borrow_mut().apply(Event::PreferencesLoaded(Ok(Box::new(
            AcceptedPreferences {
                preferences: UserPreferences::default(),
                comparators: FileComparatorSnapshot::default(),
            },
        ))));
        refresh(&ui.window, &p.borrow());
        let mut h = Self { ui, p, sent };
        h.ui.draw();
        h
    }

    fn draw(&mut self) {
        slint::platform::update_timers_and_animations();
        self.ui.draw();
    }

    /// Deliver a backend event as the application does.
    fn answer(&mut self, event: Event) {
        let sent = self.sent.clone();
        self.p.borrow_mut().apply_with_sender(event, &|command| {
            sent.borrow_mut().push(command);
            true
        });
        refresh(&self.ui.window, &self.p.borrow());
        self.draw();
    }

    /// The left panel's navigation requests since the last clear, in order.
    fn requests(&self) -> Vec<(NavigationId, PathBuf)> {
        self.sent
            .borrow()
            .iter()
            .filter_map(|command| match command {
                Command::Navigate {
                    pane: PaneId::Left,
                    navigation,
                    path,
                    ..
                } => Some((*navigation, path.clone())),
                _ => None,
            })
            .collect()
    }

    /// Open the left path editor with Ctrl+G and replace its text with `text`.
    fn edit(&mut self, text: &str) {
        self.ui.window.invoke_focus_files();
        ui::chord(&self.ui.window, &[Key::Control], "g");
        self.draw();
        assert!(editing(&self.ui.window), "Ctrl+G opens the editor");
        ui::chord(&self.ui.window, &[Key::Control], "a");
        ui::key(&self.ui.window, text);
        self.draw();
        assert_eq!(self.ui.window.get_left_path_draft(), text);
        self.sent.borrow_mut().clear();
    }

    fn click_go(&mut self) {
        let go = shown(&self.ui.window, "Go", AccessibleRole::Button).remove(0);
        let (x, y, w, h) = bounds(&go);
        assert!(w >= 24.0 && h >= 12.0, "Go is {w}x{h}");
        ui::click(&self.ui.window, x + w / 2.0, y + h / 2.0);
        self.draw();
    }
}

#[derive(Clone, Copy, Debug)]
enum Commit {
    Enter,
    Go,
    /// Tab moves keyboard focus from the editor to Go; Space presses it.
    TabSpace,
}

fn commit(h: &mut Harness, how: Commit) {
    match how {
        Commit::Enter => ui::key(&h.ui.window, Key::Return),
        Commit::Go => h.click_go(),
        Commit::TabSpace => {
            ui::key(&h.ui.window, Key::Tab);
            h.draw();
            assert!(
                !h.ui.window.get_left_path_draft().is_empty() && editing(&h.ui.window),
                "Tab keeps the editor open"
            );
            ui::key(&h.ui.window, " ");
        }
    }
    h.draw();
}

/// What a committed path does: the requests it sends, whether the editor stays
/// open, and, after the scripted backend answers, the panel's folder, cursor row and
/// error.
#[derive(Debug, PartialEq)]
struct Outcome {
    requests: Vec<PathBuf>,
    editing: bool,
    folder: String,
    cursor_name: Option<String>,
    error: String,
}

/// A listing of `entries` answers the left panel's latest navigation or refresh (the
/// shown folder's own path asks for a refresh).
fn list(h: &mut Harness, entries: Vec<Entry>) {
    let navigation = h
        .sent
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            }
            | Command::Refresh {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .expect("a navigation");
    h.answer(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries: entries.into(),
    });
}

/// Back to `base`, listed empty and with no notice open, so that each commit starts
/// from the same panel.
fn reset(h: &mut Harness, base: &std::path::Path) {
    if h.ui.window.get_embedded_modal_open() {
        // The notice's Close has the keyboard.
        ui::key(&h.ui.window, Key::Escape);
        h.draw();
    }
    assert!(!h.ui.window.get_embedded_modal_open(), "the notice closes");
    h.edit(base.to_str().unwrap());
    ui::key(&h.ui.window, Key::Return);
    h.draw();
    list(h, Vec::new());
    assert_eq!(h.ui.window.get_left_path(), base.to_str().unwrap());
}

fn folder_case(h: &mut Harness, base: &std::path::Path, how: Commit) -> Outcome {
    let folder = base.join("Příliš 文件夹");
    h.edit(folder.to_str().unwrap());
    commit(h, how);
    let requests = h.requests();
    let editing = editing(&h.ui.window);
    list(
        h,
        vec![Entry::new(
            EntryId(1),
            folder.join("a.txt"),
            EntryKind::File,
        )],
    );
    outcome(h, requests, editing)
}

fn file_case(h: &mut Harness, base: &std::path::Path, how: Commit) -> Outcome {
    let folder = base.join("folder");
    let file = folder.join("report 文档.txt");
    h.edit(file.to_str().unwrap());
    commit(h, how);
    let editing = editing(&h.ui.window);
    let (typed, _) = h.requests().last().cloned().expect("a navigation");
    h.answer(Event::ListingNotAFolder {
        pane: PaneId::Left,
        navigation: typed,
        parent: folder.clone(),
        entry: file.clone(),
        hidden: false,
    });
    h.answer(Event::ListingFallback {
        pane: PaneId::Left,
        navigation: typed,
        path: folder.clone(),
        error: FsError::from_io(
            "read directory",
            &file,
            std::io::ErrorKind::NotADirectory.into(),
        ),
    });
    let requests = h.requests();
    list(
        h,
        vec![
            Entry::new(EntryId(1), folder.join("a.txt"), EntryKind::File),
            Entry::new(EntryId(2), file.clone(), EntryKind::File),
        ],
    );
    outcome(h, requests, editing)
}

fn missing_case(h: &mut Harness, base: &std::path::Path, how: Commit) -> Outcome {
    let missing = base.join("no such folder");
    h.edit(missing.to_str().unwrap());
    commit(h, how);
    let requests = h.requests();
    let editing = editing(&h.ui.window);
    let (navigation, _) = requests.last().cloned().expect("a navigation");
    h.answer(Event::ListingFailed {
        pane: PaneId::Left,
        navigation,
        error: FsError::from_io(
            "read directory",
            &missing,
            std::io::ErrorKind::NotFound.into(),
        ),
    });
    outcome(h, requests, editing)
}

/// The error the panel shows and the notice the commit opened (none was open before).
fn outcome(h: &Harness, requests: Vec<(NavigationId, PathBuf)>, editing: bool) -> Outcome {
    let cursor = usize::try_from(h.ui.window.get_left_cursor()).ok();
    let notice = if h.ui.window.get_embedded_modal_open() {
        format!(
            "{}: {}",
            h.ui.window.get_message_title(),
            h.ui.window.get_message_body()
        )
    } else {
        String::new()
    };
    Outcome {
        requests: requests.into_iter().map(|(_, path)| path).collect(),
        editing,
        folder: h.ui.window.get_left_path().to_string(),
        cursor_name: cursor
            .and_then(|row| h.ui.window.get_left_rows().row_data(row))
            .map(|row| row.name.to_string()),
        error: format!("{}|{}", h.ui.window.get_left_error(), notice),
    }
}

type Case = fn(&mut Harness, &std::path::Path, Commit) -> Outcome;

/// Enter, then Go and Tab+Space, each from `base`: the same outcome.
fn same_as_enter(name: &str, case: Case) -> Outcome {
    let base = std::env::temp_dir().join(format!("path bar go {name}"));
    let mut h = Harness::new(base.clone());
    reset(&mut h, &base);
    let enter = case(&mut h, &base, Commit::Enter);
    assert!(!enter.requests.is_empty(), "{name}: {enter:?}");
    assert!(!enter.editing, "{name}: Enter closes the editor");
    for how in [Commit::Go, Commit::TabSpace] {
        reset(&mut h, &base);
        assert_eq!(case(&mut h, &base, how), enter, "{name} through {how:?}");
    }
    enter
}

#[test]
fn go_opens_a_typed_folder_as_enter_does() {
    let enter = same_as_enter("folder", folder_case);
    assert!(enter.folder.ends_with("Příliš 文件夹"), "{enter:?}");
    assert_eq!(enter.error, "|");
}

#[test]
fn go_opens_a_typed_files_folder_with_the_cursor_on_the_file_as_enter_does() {
    let enter = same_as_enter("file", file_case);
    assert_eq!(enter.cursor_name.as_deref(), Some("report 文档.txt"));
    assert!(enter.folder.ends_with("folder"), "{enter:?}");
}

#[test]
fn go_reports_a_missing_path_as_enter_does() {
    let enter = same_as_enter("missing", missing_case);
    assert_ne!(enter.error, "|", "{enter:?}");
}

#[test]
fn escape_cancels_the_editor_with_go_shown_or_focused() {
    let base = std::env::temp_dir().join("path bar go escape");
    let mut h = Harness::new(base);
    let shown_path = h.ui.window.get_left_path().to_string();
    for focus_go in [false, true] {
        h.edit("typed but cancelled");
        if focus_go {
            ui::key(&h.ui.window, Key::Tab);
            h.draw();
            assert!(editing(&h.ui.window), "Tab keeps the editor open");
        }
        ui::key(&h.ui.window, Key::Escape);
        h.draw();
        assert!(!editing(&h.ui.window), "Escape closes the editor");
        assert!(h.requests().is_empty(), "Escape navigates nowhere");
        assert_eq!(h.ui.window.get_left_path_draft(), shown_path.as_str());
    }
}

#[test]
fn a_click_anywhere_right_of_the_path_components_opens_the_editor() {
    // A short path whatever the temporary folder is: commands are only recorded, so
    // nothing is read from it, and a long temporary folder would fill the bar.
    let base = PathBuf::from(if cfg!(windows) {
        r"C:\path bar click"
    } else {
        "/path bar click"
    });
    let mut h = Harness::new(base);
    h.ui.set_size(1600, 800);
    h.draw();
    let edit = shown(&h.ui.window, "Edit folder path", AccessibleRole::Button).remove(0);
    // The clip of the bar that holds the components and the edit area.
    let mut parent = edit.parent_item(ParentItemTraversalMode::StopAtPopups);
    let clip = loop {
        let ancestor = parent.expect("the path bar clips its components");
        if ancestor.borrow().as_ref().clips_children() {
            break bounds(&ancestor);
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    };
    let (x, y, w, height) = bounds(&edit);
    let (clip_x, _, clip_w, _) = clip;
    assert!(
        (x + w - (clip_x + clip_w)).abs() <= 0.5,
        "the edit area {x:.0}..{:.0} reaches the bar's end {:.0}",
        x + w,
        clip_x + clip_w
    );
    // A short path leaves most of the bar empty: the area takes all of it.
    assert!(w > clip_w / 2.0, "edit area {w:.0}px of {clip_w:.0}px");
    let middle_y = y + height / 2.0;
    let shown_path = h.ui.window.get_left_path().to_string();
    for target in [x + 2.0, x + w / 2.0, clip_x + clip_w - 3.0] {
        assert!(!editing(&h.ui.window));
        h.sent.borrow_mut().clear();
        ui::click(&h.ui.window, target, middle_y);
        h.draw();
        assert!(
            editing(&h.ui.window),
            "a click at x={target:.0} opens the editor"
        );
        assert_eq!(
            h.ui.window.get_left_path_draft(),
            shown_path.as_str(),
            "the editor starts from the shown path"
        );
        assert!(
            h.requests().is_empty(),
            "opening the editor navigates nowhere"
        );
        // The whole path is selected, so what is typed (or pasted) replaces it.
        ui::key(&h.ui.window, "Q");
        h.draw();
        assert_eq!(
            h.ui.window.get_left_path_draft(),
            "Q",
            "a click at x={target:.0} selects the whole path"
        );
        ui::key(&h.ui.window, Key::Escape);
        h.ui.window
            .window()
            .dispatch_event(WindowEvent::PointerMoved {
                position: LogicalPosition::new(0.0, 0.0),
            });
        h.draw();
    }
    // A component still navigates and does not open the editor.
    let first =
        h.ui.window
            .get_left_path_parts()
            .row_data(0)
            .unwrap()
            .prefix;
    let token = shown(
        &h.ui.window,
        &format!("Open {first}"),
        AccessibleRole::Button,
    )
    .remove(0);
    let (tx, ty, tw, th) = bounds(&token);
    h.sent.borrow_mut().clear();
    ui::click(&h.ui.window, tx + tw / 2.0, ty + th / 2.0);
    h.draw();
    assert!(!editing(&h.ui.window), "a component click does not edit");
    assert!(
        !h.requests().is_empty(),
        "a component click navigates: {:?}",
        h.sent
    );
}
