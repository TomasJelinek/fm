//! The Copy/Move dialog as F5 and F6 open it through the installed presenter, with the
//! texts the presenter composes: one file, several items, a destination where no links
//! are created, a Move from another drive to a network location (refused) and a Move of
//! 65 items (refused). Its layout fits the minimum window and 32 px fonts in both
//! densities with nothing cut, the options start at one edge, the link choice shows no
//! caption but keeps its accessible name, Tab follows the order the controls are shown
//! in, Enter in the destination confirms, and at most one note shows, under the control
//! it concerns. The capture tests render each state for review; they are ignored and
//! write only with `MIDNIGHT_UI_RENDER_DIR` set.
#[path = "support/embedded_form.rs"]
mod form;
#[path = "support/owned_commands.rs"]
mod owned_commands;
#[path = "support/window_polish.rs"]
mod polish;
#[path = "support/selection.rs"]
mod selection;
use crate::support::{link_option, modeless, ui};

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::{ItemRc, ParentItemTraversalMode},
    items::{AccessibleRole, Flickable},
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{MainWindow, OperationPresentation, Presenter, refresh};
use slint::{
    ComponentHandle, Model,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{
    cell::RefCell,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

/// A fixed folder that is never read: the presenter only names it.
fn documents() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from(r"C:\Users\Alex\Documents")
    } else {
        PathBuf::from("/home/alex/Documents")
    }
}

/// A name long enough to wrap the dialog's first line at every width.
const LONG_NAME: &str = "Minutes of the quarterly board meeting with every appendix and the signed resolutions 文档.pdf";

/// The left panel's items: four files and a folder.
const LEFT: [(&str, EntryKind, Option<u64>); 5] = [
    ("Q3 report.pdf", EntryKind::File, Some(1_234_567)),
    ("Budget 2027.xlsx", EntryKind::File, Some(48_211)),
    ("Photos", EntryKind::Directory, None),
    ("notes.txt", EntryKind::File, Some(2_048)),
    (LONG_NAME, EntryKind::File, Some(73_400)),
];

/// The right panel holds this many photos, one more than a Move accepts.
const PHOTOS: u64 = 65;

fn photo(index: u64) -> String {
    format!("IMG_{index:04}.jpg")
}

struct Harness {
    main: MainWindow,
    native: Rc<MinimalSoftwareWindow>,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    folders: [PathBuf; 2],
}

impl Harness {
    /// The main window with both panels listed, in `theme` at `font` px and `density`.
    fn new(theme: PreferenceTheme, density: UiDensity, font: u16) -> Self {
        let windows = Rc::new(RefCell::new(Vec::new()));
        slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
        let main = MainWindow::new().unwrap();
        midnight_ui::install_file_keys(&main);
        let folders = [documents().join("Reports"), documents().join("Backup")];
        let state = Rc::new(RefCell::new(Presenter::new(
            folders[0].clone(),
            folders[1].clone(),
        )));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let captured = commands.clone();
        let begin = owned_commands::install(
            &main,
            state.clone(),
            Rc::new(move |command| {
                captured.borrow_mut().push(command);
                true
            }),
        );
        let mut preferences = UserPreferences {
            theme,
            density,
            desktop_font_size: font,
            touch_font_size: font,
            ..Default::default()
        };
        preferences.tabs[0].paths = vec![folders[0].clone()];
        preferences.tabs[1].paths = vec![folders[1].clone()];
        owned_commands::run(&state, &begin, |state, send| {
            state.apply_with_sender(
                Event::PreferencesLoaded(Ok(Box::new(AcceptedPreferences {
                    preferences,
                    comparators: FileComparatorSnapshot::default(),
                }))),
                send,
            )
        });
        let harness = Self {
            native: windows.borrow()[0].clone(),
            main,
            state,
            commands,
            folders,
        };
        harness.list(
            PaneId::Left,
            LEFT.iter()
                .map(|(name, kind, size)| (name.to_string(), *kind, *size))
                .collect(),
        );
        harness.list(
            PaneId::Right,
            (1..=PHOTOS)
                .map(|index| (photo(index), EntryKind::File, Some(3_145_728 + index * 977)))
                .collect(),
        );
        refresh(&harness.main, &harness.state.borrow());
        harness.main.set_preferences_ready(true);
        harness.main.show().unwrap();
        harness
            .main
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        modeless::draw(&harness.native, 1340, 820, "copy-dialog-owner");
        harness
    }

    /// Answer the panel's latest listing request with these items.
    fn list(&self, pane: PaneId, items: Vec<(String, EntryKind, Option<u64>)>) {
        let navigation = self
            .commands
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
            .expect("the panel asks for its folder");
        let folder = &self.folders[pane.index()];
        let entries: Arc<[Entry]> = items
            .into_iter()
            .enumerate()
            .map(|(index, (name, kind, size))| {
                let mut entry = Entry::new(EntryId(index as u64 + 1), folder.join(name), kind);
                entry.metadata.size = size;
                entry
            })
            .collect::<Vec<_>>()
            .into();
        self.state.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
    }

    /// Put the panel's cursor on `name`, with the keyboard in the file list.
    fn focus(&self, pane: PaneId, name: &str) {
        self.main.invoke_activate(pane.index() as i32);
        let rows = if pane == PaneId::Left {
            self.main.get_left_rows()
        } else {
            self.main.get_right_rows()
        };
        let row = rows
            .iter()
            .position(|row| row.name == name)
            .unwrap_or_else(|| panic!("the panel shows {name}"));
        self.main
            .invoke_row_click(pane.index() as i32, row as i32, false, false);
        self.main.invoke_focus_files();
    }

    /// Mark exactly `names` in the panel, as the selection worker answers.
    fn select(&self, pane: PaneId, names: &[String]) {
        self.main.invoke_bulk_selection(pane.index() as i32, 0);
        let paths: Vec<PathBuf> = names
            .iter()
            .map(|name| self.folders[pane.index()].join(name))
            .collect();
        let paths: Vec<&Path> = paths.iter().map(PathBuf::as_path).collect();
        selection::acknowledge(&self.state, &self.commands, pane, &paths);
        refresh(&self.main, &self.state.borrow());
    }

    /// Press `key` (F5 or F6) and answer the dialog's question about its destination's
    /// links with `facts`.
    fn open(&self, key: Key, facts: TransferLinkFacts) {
        self.commands.borrow_mut().clear();
        ui::key(&self.main, key);
        let inspection = self
            .commands
            .borrow()
            .iter()
            .find_map(|command| match command {
                Command::InspectTransferLinks(request) => Some(request.clone()),
                _ => None,
            })
            .expect("the dialog asks what its destination does with links");
        self.state
            .borrow_mut()
            .apply(Event::TransferLinksInspected(Box::new(
                TransferLinksResult {
                    request: inspection.request,
                    destination: inspection.destination.clone(),
                    result: Ok(facts),
                },
            )));
        refresh(&self.main, &self.state.borrow());
        slint::platform::update_timers_and_animations();
        assert!(
            self.main.global::<OperationPresentation>().get_active(),
            "{key:?} opens the dialog"
        );
    }

    /// The user's own link choice in the open dialog: 0 copies the files, 1 keeps links.
    fn choose_links(&self, choice: i32) {
        let form = self.main.global::<OperationPresentation>();
        form.invoke_link_chosen(form.get_token(), choice);
        refresh(&self.main, &self.state.borrow());
    }

    fn key(&self, text: impl Into<slint::SharedString>) {
        slint::platform::update_timers_and_animations();
        let text = text.into();
        self.native
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
        self.native
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text });
        refresh(&self.main, &self.state.borrow());
        slint::platform::update_timers_and_animations();
    }

    /// `text` with `modifier` held.
    fn chord(&self, modifier: Key, text: impl Into<slint::SharedString>) {
        self.native
            .window()
            .dispatch_event(WindowEvent::KeyPressed {
                text: modifier.into(),
            });
        self.key(text);
        self.native
            .window()
            .dispatch_event(WindowEvent::KeyReleased {
                text: modifier.into(),
            });
        slint::platform::update_timers_and_animations();
    }

    /// Escape closes the dialog without starting anything.
    fn close(&self) {
        self.key(Key::Escape);
        assert!(
            !self.main.global::<OperationPresentation>().get_active(),
            "Escape closes the dialog"
        );
        assert!(
            !self.commands.borrow().iter().any(|command| matches!(
                command,
                Command::EnqueueCopyWithOptions(_)
                    | Command::EnqueueSelectedCopy(_)
                    | Command::EnqueueSelectedMove(_)
            )),
            "nothing started"
        );
    }

    /// Render the window at `width`×`height` and keep the open dialog's body, with a
    /// margin, as the artifact `name`.
    fn capture(&self, width: u32, height: u32, name: &str) {
        let mut pixels = Vec::new();
        for _ in 0..3 {
            pixels = render(&self.native, width, height);
        }
        let cancel =
            modeless::accessible_popup_item(&self.native, AccessibleRole::Button, "Cancel")
                .expect("the dialog shows Cancel");
        let (origin, size) = form::body(&cancel);
        let margin = 8.0;
        let left = (origin.x - margin).max(0.0) as usize;
        let top = (origin.y - margin).max(0.0) as usize;
        let right = ((origin.x + size.width + margin) as usize).min(width as usize);
        let bottom = ((origin.y + size.height + margin) as usize).min(height as usize);
        let cropped: Vec<slint::Rgb8Pixel> = (top..bottom)
            .flat_map(|row| {
                let start = row * width as usize;
                pixels[start + left..start + right].to_vec()
            })
            .collect();
        modeless::bitmap::snapshot(&cropped, right - left, bottom - top, name);
    }
}

fn render(window: &MinimalSoftwareWindow, width: u32, height: u32) -> Vec<slint::Rgb8Pixel> {
    window.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    slint::platform::update_timers_and_animations();
    let mut pixels = vec![slint::Rgb8Pixel::default(); width as usize * height as usize];
    window.window().request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    });
    pixels
}

/// A fixed local folder: links stay links, nothing to say.
fn local() -> TransferLinkFacts {
    TransferLinkFacts {
        destination: TransferLinkDestination {
            storage: TransferLinkStorage::Supported,
            filesystem: Some("NTFS".into()),
            volume: Some(1),
            ..Default::default()
        },
        same_volume: false,
    }
}

/// A network location: no links are created there, and a Move from another drive
/// cannot end there.
fn network() -> TransferLinkFacts {
    TransferLinkFacts {
        destination: TransferLinkDestination {
            storage: TransferLinkStorage::Unsupported,
            network: true,
            volume: Some(7),
            refuses_cross_volume_move: true,
            ..Default::default()
        },
        same_volume: false,
    }
}

/// The dialog's states, in an order that needs no marks cleared in between: the
/// single-file states first, then the left panel's marks, then the right panel's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// F5 on one file.
    OneFile,
    /// F5 to a network location, keeping links: no links are created there (W-3).
    LinkNote,
    /// F6 from another drive to a network location, confirmed: refused (O-1).
    MoveRefused,
    /// F5 of two files and a folder.
    Several,
    /// F6 of 65 files: more than a Move once took, now an ordinary Move.
    MoveMany,
}

const STATES: [State; 5] = [
    State::OneFile,
    State::LinkNote,
    State::MoveRefused,
    State::Several,
    State::MoveMany,
];

impl Harness {
    /// Open the dialog in `state`.
    fn show(&self, state: State) {
        match state {
            State::OneFile => {
                self.focus(PaneId::Left, "Q3 report.pdf");
                self.open(Key::F5, local());
            }
            State::LinkNote => {
                self.focus(PaneId::Left, "Q3 report.pdf");
                self.open(Key::F5, network());
                self.choose_links(1);
            }
            State::MoveRefused => {
                self.focus(PaneId::Left, "Q3 report.pdf");
                self.open(Key::F6, network());
                self.key(Key::Return);
            }
            State::Several => {
                self.focus(PaneId::Left, "Q3 report.pdf");
                let marked: Vec<String> = LEFT[..3]
                    .iter()
                    .map(|(name, ..)| name.to_string())
                    .collect();
                self.select(PaneId::Left, &marked);
                self.open(Key::F5, local());
            }
            State::MoveMany => {
                self.focus(PaneId::Right, &photo(1));
                let photos: Vec<String> = (1..=PHOTOS).map(photo).collect();
                self.select(PaneId::Right, &photos);
                self.open(Key::F6, local());
            }
        }
    }

    /// The open dialog's popup root.
    fn popup(&self) -> ItemRc {
        let inner = WindowInner::from_pub(self.native.window());
        let popups = inner.active_popups();
        let popup = popups
            .iter()
            .rev()
            .find(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
            .expect("the dialog is open");
        ItemRc::new_root(popup.component.clone())
    }

    fn item(&self, role: AccessibleRole, label: &str) -> ItemRc {
        modeless::accessible_popup_item(&self.native, role, label)
            .unwrap_or_else(|| panic!("the dialog has no {label}"))
    }

    /// Size the window and lay the dialog out, as a frame does before painting.
    fn layout(&self, width: u32, height: u32) {
        for _ in 0..2 {
            slint::platform::update_timers_and_animations();
            modeless::layout(&self.native, width, height);
        }
    }

    fn tab(&self, backwards: bool) {
        if backwards {
            self.chord(Key::Shift, Key::Tab);
        } else {
            self.key(Key::Tab);
        }
    }

    /// The control that has the keyboard, as (role, label).
    fn focused(&self) -> Option<(AccessibleRole, String)> {
        polish::focused_control(self.native.window()).map(|item| {
            let label = item
                .accessible_string_property(AccessibleStringProperty::Label)
                .unwrap_or_default();
            (item.accessible_role(), label.to_string())
        })
    }

    /// The texts the open dialog draws.
    fn drawn(&self) -> Vec<String> {
        ui::visible_text_values(self.popup())
            .into_iter()
            .map(|text| text.to_string())
            .filter(|text| !text.trim().is_empty())
            .collect()
    }
}

/// Every reviewed state of the dialog, in `theme` at `font` px, each as an artifact
/// named `<state>-<look>`.
fn capture_states(theme: PreferenceTheme, font: u16, look: &str) {
    let h = Harness::new(theme, UiDensity::Desktop, font);
    for (state, name) in [
        (State::OneFile, "1-copy-one-file"),
        (State::LinkNote, "3-link-note"),
        (State::MoveRefused, "4-move-refused-network"),
        (State::Several, "2-copy-several"),
        (State::MoveMany, "5-move-65"),
    ] {
        h.show(state);
        h.capture(1340, 820, &format!("{name}-{look}"));
        h.close();
    }
}

#[test]
#[ignore = "renders the dialog's states for review; run with --ignored and MIDNIGHT_UI_RENDER_DIR"]
fn capture_dark_default_font() {
    capture_states(PreferenceTheme::Dark, 12, "dark-12px");
}

#[test]
#[ignore = "renders the dialog's states for review; run with --ignored and MIDNIGHT_UI_RENDER_DIR"]
fn capture_light_default_font() {
    capture_states(PreferenceTheme::Light, 12, "light-12px");
}

#[test]
#[ignore = "renders the dialog's states for review; run with --ignored and MIDNIGHT_UI_RENDER_DIR"]
fn capture_dark_32px() {
    capture_states(PreferenceTheme::Dark, 32, "dark-32px");
}

#[test]
#[ignore = "renders the dialog's states for review; run with --ignored and MIDNIGHT_UI_RENDER_DIR"]
fn capture_light_32px() {
    capture_states(PreferenceTheme::Light, 32, "light-32px");
}

/// Every state in the minimum window (320×200), whole, as `min-<state>-<look>`.
fn capture_minimum(density: UiDensity, font: u16, look: &str) {
    let h = Harness::new(PreferenceTheme::Dark, density, font);
    for (state, name) in [
        (State::OneFile, "1-copy-one-file"),
        (State::LinkNote, "3-link-note"),
        (State::MoveRefused, "4-move-refused-network"),
        (State::Several, "2-copy-several"),
        (State::MoveMany, "5-move-65"),
    ] {
        h.show(state);
        let mut pixels = Vec::new();
        for _ in 0..3 {
            pixels = render(&h.native, 320, 200);
        }
        modeless::bitmap::snapshot(&pixels, 320, 200, &format!("min-{name}-{look}"));
        h.close();
    }
}

#[test]
#[ignore = "renders the dialog's states for review; run with --ignored and MIDNIGHT_UI_RENDER_DIR"]
fn capture_minimum_desktop_default_font() {
    capture_minimum(UiDensity::Desktop, 12, "desktop-12px");
}

#[test]
#[ignore = "renders the dialog's states for review; run with --ignored and MIDNIGHT_UI_RENDER_DIR"]
fn capture_minimum_touch_32px() {
    capture_minimum(UiDensity::Touch, 32, "touch-32px");
}

/// The part of `item` that the window shows, beside the whole of it: (whole, shown), each
/// as left, top, right and bottom.
type Edges = (f32, f32, f32, f32);
fn whole_and_shown(h: &Harness, item: &ItemRc) -> (Edges, Edges) {
    (
        modeless::edges(item),
        modeless::shown_edges(&h.native, item),
    )
}

/// The height of what the dialog's scrolled middle holds, and of its view.
fn middle_extent(h: &Harness) -> (f32, f32) {
    let mut parent = h
        .item(AccessibleRole::TextInput, "Destination folder")
        .parent_item(ParentItemTraversalMode::StopAtPopups);
    while let Some(ancestor) = parent {
        if let Some(flickable) = ancestor.downcast::<Flickable>() {
            return (
                flickable.as_pin_ref().content_height().get(),
                ancestor.geometry().height(),
            );
        }
        parent = ancestor.parent_item(ParentItemTraversalMode::StopAtPopups);
    }
    panic!("the destination is in the scrolled middle")
}

/// Every state of the dialog at the minimum window size (320×200) and at a regular one,
/// with `font` px in `density`: the layout checks of the UI polish gate find nothing in
/// the dialog, the header's close button and both actions are whole, and Tab reaches
/// every control, each shown whole when it has the keyboard (a check box whose caption
/// wraps taller than the scrolled middle of a tiny window shows its box) and comes back
/// round to the destination.
fn fits(density: UiDensity, font: u16) {
    let h = Harness::new(PreferenceTheme::Dark, density, font);
    for state in STATES {
        h.show(state);
        let action = if matches!(state, State::MoveRefused | State::MoveMany) {
            "Move"
        } else {
            "Copy"
        };
        for (width, height) in [(1340, 820), (320, 200)] {
            let case = format!("{state:?} {density:?} {font} px {width}x{height}");
            h.layout(width, height);
            let findings = polish::item_findings(&h.popup());
            assert!(findings.is_empty(), "{case}: {findings:#?}");
            if width == 1340 {
                // The dialog asks for the height its regular layout needs.
                let (content, view) = middle_extent(&h);
                assert!(
                    content <= view + 0.5,
                    "{case}: the regular dialog scrolls {content} px in {view} px"
                );
            }
            for label in ["Cancel dialog", "Cancel", action] {
                let (whole, shown) = whole_and_shown(&h, &h.item(AccessibleRole::Button, label));
                assert!(
                    (whole.0 - shown.0).abs() < 0.5
                        && (whole.1 - shown.1).abs() < 0.5
                        && (whole.2 - shown.2).abs() < 0.5
                        && (whole.3 - shown.3).abs() < 0.5,
                    "{case}: {label} is cut: {whole:?} shows {shown:?}"
                );
            }
            let start = h.focused();
            assert_eq!(
                start,
                Some((AccessibleRole::TextInput, "Destination folder".to_owned())),
                "{case}: the keyboard is in the destination"
            );
            for press in 1..=8 {
                h.tab(false);
                h.layout(width, height);
                let item = polish::focused_control(h.native.window())
                    .unwrap_or_else(|| panic!("{case}: Tab {press} lost the keyboard"));
                let (whole, shown) = whole_and_shown(&h, &item);
                let (top, bottom) = if item.accessible_role() == AccessibleRole::Checkbox {
                    let middle = (whole.1 + whole.3) / 2.0;
                    (middle - 12.0, middle + 12.0)
                } else {
                    (whole.1, whole.3)
                };
                assert!(
                    shown.0 <= whole.0 + 0.5
                        && shown.2 >= whole.2 - 0.5
                        && shown.1 <= top + 0.5
                        && shown.3 >= bottom - 0.5,
                    "{case}: Tab {press} reaches {:?}, cut: {whole:?} shows {shown:?}",
                    h.focused()
                );
                if h.focused() == start {
                    break;
                }
            }
            assert_eq!(h.focused(), start, "{case}: Tab comes back round");
        }
        h.close();
    }
}

#[test]
fn every_state_fits_desktop_density_with_the_default_font() {
    fits(UiDensity::Desktop, 12);
}

#[test]
fn every_state_fits_desktop_density_with_32_px() {
    fits(UiDensity::Desktop, 32);
}

#[test]
fn every_state_fits_touch_density_with_the_default_font() {
    fits(UiDensity::Touch, 15);
}

#[test]
fn every_state_fits_touch_density_with_32_px() {
    fits(UiDensity::Touch, 32);
}

/// The link choice in `density` at `font` px, at the regular size and at the minimum
/// window size: both its entries name links ("Copy the files links point to", "Keep links
/// as links"), so the dialog draws no "Links" caption for it; a screen reader still reads
/// the box by that name, with the entry shown as its value.
fn link_choice_without_caption(density: UiDensity, font: u16) {
    let h = Harness::new(PreferenceTheme::Light, density, font);
    h.show(State::OneFile);
    // The keyboard in the box, so that a small window's scrolled middle shows it.
    h.tab(false);
    for (width, height) in [(1340, 820), (320, 200)] {
        let case = format!("{density:?} {font} px {width}x{height}");
        h.layout(width, height);
        assert_eq!(
            h.focused(),
            Some((AccessibleRole::Combobox, link_option::LABEL.to_owned())),
            "{case}"
        );
        let texts = h.drawn();
        assert!(
            texts.iter().any(|text| text == link_option::KEEP_LINKS),
            "{case}: the link choice is in view: {texts:?}"
        );
        if width == 1340 {
            assert!(
                texts.iter().any(|text| text == "Options"),
                "{case}: the Options group is drawn: {texts:?}"
            );
        }
        assert!(
            !texts.iter().any(|text| text == link_option::LABEL),
            "{case}: the dialog draws a {:?} caption: {texts:?}",
            link_option::LABEL
        );
        let links = h.item(AccessibleRole::Combobox, link_option::LABEL);
        assert_eq!(
            links
                .accessible_string_property(AccessibleStringProperty::Value)
                .as_deref(),
            Some(link_option::KEEP_LINKS),
            "{case}: the box is read as Links with its entry"
        );
    }
    h.close();
}

#[test]
fn the_link_choice_shows_no_caption() {
    link_choice_without_caption(UiDensity::Desktop, 12);
}

#[test]
fn the_link_choice_shows_no_caption_with_32_px_touch_fonts() {
    link_choice_without_caption(UiDensity::Touch, 32);
}

/// Without a caption the link choice keeps what it says to a screen reader and in its
/// tooltip: its accessible name "Links"; as its description why the destination's default
/// was chosen, or, once keeping links cannot work there, what keeping them does; and as
/// its tooltip that reason, a blank line and what each entry does. The note about keeping
/// links, and the hold of the first confirmation that keeps them, sit under the box.
#[test]
fn the_link_choice_keeps_its_name_description_and_tooltip() {
    let h = Harness::new(PreferenceTheme::Dark, UiDensity::Desktop, 12);
    let form = h.main.global::<OperationPresentation>();
    let description = |item: &ItemRc| {
        item.accessible_string_property(AccessibleStringProperty::Description)
            .map(|text| text.to_string())
    };
    h.focus(PaneId::Left, "Q3 report.pdf");
    h.open(Key::F5, network());
    h.layout(1340, 820);
    let reason = form.get_link_reason().to_string();
    assert!(reason.starts_with("Network location: "), "{reason}");
    let links = h.item(AccessibleRole::Combobox, link_option::LABEL);
    assert_eq!(description(&links).as_deref(), Some(reason.as_str()));
    let hint = form.get_link_hint().to_string();
    let entries = hint
        .strip_prefix(&format!("{reason}\n\n"))
        .unwrap_or_else(|| panic!("the tooltip starts with the reason and a blank line: {hint:?}"));
    assert!(
        entries.starts_with(&format!("{}: ", link_option::COPY_FILES))
            && entries.contains(&format!("\n{}: ", link_option::KEEP_LINKS)),
        "then what each entry does: {entries:?}"
    );
    // Keeping links where none are created: the box describes what that does, and the
    // note says it under the box, from the box's left edge.
    h.choose_links(1);
    h.layout(1340, 820);
    let warning = form.get_link_warning().to_string();
    assert!(!warning.is_empty(), "keeping links cannot work there");
    let links = h.item(AccessibleRole::Combobox, link_option::LABEL);
    assert_eq!(description(&links).as_deref(), Some(warning.as_str()));
    let under = |text: &str| {
        let edges = modeless::edges(&links);
        let note = modeless::edges(&h.item(AccessibleRole::Text, text));
        assert!(
            note.1 >= edges.3
                && note.1 < edges.3 + 24.0
                && note.0 > edges.0
                && note.0 < edges.0 + 48.0,
            "{text:?} sits under the link choice: {note:?} under {edges:?}"
        );
    };
    under(&warning);
    // Enter holds the Copy once, with what keeping links does, still under the box.
    h.key(Key::Return);
    h.layout(1340, 820);
    assert!(form.get_active(), "the first Enter holds");
    let held = form.get_warning().to_string();
    assert!(held.starts_with(&warning), "{held:?}");
    assert!(form.get_warning_about_links(), "the hold is about links");
    under(&held);
    h.close();
}

/// The options in `density` at `font` px, at the regular size and at the minimum window
/// size: one column under the "Options" heading. The link choice and both check boxes
/// start at the heading's left edge, which is the destination's; with the default font the
/// link choice is as wide as its longest entry, not as the form. Exact modified time is
/// left out, not shown disabled, where it cannot apply (a folder among the items).
fn controls_start_at_one_edge(density: UiDensity, font: u16) {
    let h = Harness::new(PreferenceTheme::Light, density, font);
    h.show(State::OneFile);
    for (width, height) in [(1340, 820), (320, 200)] {
        let case = format!("{density:?} {font} px {width}x{height}");
        h.layout(width, height);
        let links = modeless::edges(&h.item(AccessibleRole::Combobox, link_option::LABEL));
        let field = modeless::edges(&h.item(AccessibleRole::TextInput, "Destination folder"));
        let edge = |role, label: &str| modeless::edges(&h.item(role, label)).0;
        for (label, left) in [
            ("the Options heading", edge(AccessibleRole::Text, "Options")),
            ("the destination", field.0),
            (
                "Preserve exact modified time",
                edge(AccessibleRole::Checkbox, "Preserve exact modified time"),
            ),
            (
                "Wait for earlier transfers",
                edge(AccessibleRole::Checkbox, "Wait for earlier transfers"),
            ),
        ] {
            assert!(
                (left - links.0).abs() < 0.5,
                "{case}: {label} starts at {left}, the link choice at {}",
                links.0
            );
        }
        assert!(
            links.2 <= field.2 + 0.5,
            "{case}: the link choice ends within the form: {links:?} {field:?}"
        );
        if width == 1340 && font < 16 {
            assert!(
                links.2 - links.0 >= 159.5 && links.2 < field.2 - 40.0,
                "{case}: the link choice is as wide as its longest entry: {links:?} {field:?}"
            );
        }
    }
    h.close();
    h.show(State::Several);
    h.layout(1340, 820);
    assert!(
        modeless::accessible_popup_item(
            &h.native,
            AccessibleRole::Checkbox,
            "Preserve exact modified time"
        )
        .is_none_or(|item| !item.is_visible()),
        "a folder among the items leaves exact times out"
    );
    h.close();
}

#[test]
fn the_option_controls_start_at_one_edge() {
    controls_start_at_one_edge(UiDensity::Desktop, 12);
}

#[test]
fn the_option_controls_start_at_one_edge_with_32_px_touch_fonts() {
    controls_start_at_one_edge(UiDensity::Touch, 32);
}

/// Tab follows the order the controls are shown in: the destination, the options top to
/// bottom, Cancel, the primary action and the header's close button, then the
/// destination again; Shift+Tab goes back the same way.
#[test]
fn tab_follows_the_visual_order_and_wraps() {
    let h = Harness::new(PreferenceTheme::Dark, UiDensity::Desktop, 12);
    h.show(State::OneFile);
    h.layout(1340, 820);
    let control = |role, label: &str| Some((role, label.to_owned()));
    let cycle = [
        control(AccessibleRole::TextInput, "Destination folder"),
        control(AccessibleRole::Combobox, "Links"),
        control(AccessibleRole::Checkbox, "Preserve exact modified time"),
        control(AccessibleRole::Checkbox, "Wait for earlier transfers"),
        control(AccessibleRole::Button, "Cancel"),
        control(AccessibleRole::Button, "Copy"),
        control(AccessibleRole::Button, "Cancel dialog"),
    ];
    assert_eq!(h.focused(), cycle[0], "the dialog opens in its destination");
    for step in 1..=cycle.len() {
        h.tab(false);
        assert_eq!(h.focused(), cycle[step % cycle.len()], "Tab {step}");
    }
    for (step, expected) in cycle.iter().enumerate().rev() {
        h.tab(true);
        assert_eq!(&h.focused(), expected, "Shift+Tab to {step}");
    }
    h.close();
}

/// Enter in the destination field confirms: the proposed folder starts the Copy at once,
/// and a typed folder is checked first.
#[test]
fn enter_in_the_destination_confirms() {
    let h = Harness::new(PreferenceTheme::Dark, UiDensity::Desktop, 12);
    h.show(State::OneFile);
    assert_eq!(
        h.focused(),
        Some((AccessibleRole::TextInput, "Destination folder".to_owned()))
    );
    h.commands.borrow_mut().clear();
    h.key(Key::Return);
    {
        let commands = h.commands.borrow();
        let [Command::EnqueueCopyWithOptions(options)] = commands.as_slice() else {
            panic!("Enter starts the Copy: {commands:?}");
        };
        let CopyOptionsInput::Native(request) = &options.input else {
            panic!("the dialog keeps its native selection");
        };
        let MetadataTransferInput::Selected(request) = &request.input else {
            panic!("the dialog keeps its selected sources");
        };
        assert_eq!(request.destination, h.folders[1]);
    }
    assert!(
        !h.main.global::<OperationPresentation>().get_active(),
        "the dialog closed"
    );
    // A typed folder: Enter asks whether it is an existing folder first.
    h.show(State::OneFile);
    let typed = documents().join("Archive 2026");
    h.chord(Key::Control, "a");
    h.key(typed.to_string_lossy().as_ref());
    h.commands.borrow_mut().clear();
    h.key(Key::Return);
    assert!(
        h.commands.borrow().iter().any(|command| matches!(
            command,
            Command::CheckTransferDestination(check) if check.path == typed
        )),
        "Enter asks about the typed folder: {:?}",
        h.commands.borrow()
    );
    h.close();
}

/// One note at a time, under the control it concerns: what keeping links does at a
/// network location under the link choice; a refused Move under the destination, in
/// place of the link note; the 65-item refusal in its mandated words. Why the default
/// was chosen and what the job verifies are the tooltips of the link choice and of the
/// primary action, not paragraphs of the dialog.
#[test]
fn one_note_at_a_time_sits_under_its_control() {
    let h = Harness::new(PreferenceTheme::Light, UiDensity::Desktop, 12);
    let form = h.main.global::<OperationPresentation>();
    let link_note = "No links are created at this network location: each link is left out \
                     and stays where it is, and the job details name it.";
    let refusal = network()
        .move_refusal()
        .expect("a Move from another drive to a network location is refused");
    h.focus(PaneId::Left, "Q3 report.pdf");
    h.open(Key::F6, network());
    h.layout(1340, 820);
    let reason = form.get_link_reason().to_string();
    assert!(reason.starts_with("Network location: "), "{reason}");
    assert!(
        form.get_link_hint().starts_with(&format!("{reason}\n\n")),
        "the reason leads the Links tooltip: {}",
        form.get_link_hint()
    );
    assert!(
        form.get_policy()
            .starts_with("Same-volume items are renamed"),
        "what the Move verifies is its button's tooltip: {}",
        form.get_policy()
    );
    let texts = h.drawn();
    for paragraph in ["Network location", "Same-volume", "From: "] {
        assert!(
            !texts.iter().any(|text| text.contains(paragraph)),
            "{paragraph:?} is not drawn: {texts:?}"
        );
    }
    // Keeping links there: the link note, under the link choice.
    h.choose_links(1);
    h.layout(1340, 820);
    let texts = h.drawn();
    assert_eq!(
        texts.iter().filter(|text| **text == link_note).count(),
        1,
        "{texts:?}"
    );
    let links = modeless::edges(&h.item(AccessibleRole::Combobox, "Links"));
    let note = modeless::edges(&h.item(AccessibleRole::Text, link_note));
    assert!(
        note.1 >= links.3 && note.1 < links.3 + 24.0 && note.0 >= links.0 - 0.5,
        "the link note sits under the link choice: {note:?} under {links:?}"
    );
    // Enter: the Move is refused, under the destination, and the link note gives way.
    h.key(Key::Return);
    h.layout(1340, 820);
    let texts = h.drawn();
    assert!(texts.contains(&refusal), "{texts:?}");
    assert!(!texts.iter().any(|text| text == link_note), "{texts:?}");
    let field = modeless::edges(&h.item(AccessibleRole::TextInput, "Destination folder"));
    let note = modeless::edges(&h.item(AccessibleRole::Text, &refusal));
    assert!(
        note.1 >= field.3 && note.1 < field.3 + 24.0,
        "the refusal sits under the destination: {note:?} under {field:?}"
    );
    h.close();
    // 65 items: no note, and Enter starts the Move of all of them.
    h.show(State::MoveMany);
    h.layout(1340, 820);
    let texts = h.drawn();
    assert!(
        !texts.iter().any(|text| text.contains("at most")),
        "{texts:?}"
    );
    h.key(Key::Return);
    assert!(
        !h.main.global::<OperationPresentation>().get_active(),
        "Enter starts the Move"
    );
    let commands = h.commands.borrow();
    let moved = commands.iter().find_map(|command| match command {
        Command::EnqueueSelectedMove(request) => Some(request.files.selection.len()),
        Command::EnqueueCopyWithOptions(request) => match &request.input {
            CopyOptionsInput::Native(request) if request.intent == TransferIntent::Move => {
                Some(match &request.input {
                    MetadataTransferInput::Selected(request) => request.files.selection.len(),
                    MetadataTransferInput::Paths(request) => request.clipboard.paths.len(),
                    MetadataTransferInput::Clipboard(request) => request.clipboard.paths.len(),
                })
            }
            _ => None,
        },
        _ => None,
    });
    assert_eq!(moved, Some(PHOTOS as usize), "{commands:?}");
}

/// The first line names what is copied, with what the draft already knows, and ends right
/// above the destination field it introduces; where the items come from is its tooltip.
#[test]
fn the_first_line_says_what_is_copied_where() {
    let h = Harness::new(PreferenceTheme::Dark, UiDensity::Desktop, 12);
    for (state, line) in [
        (
            State::OneFile,
            "Copy \u{201c}\u{2066}Q3 report.pdf\u{2069}\u{201d} (1.2 MiB) to:",
        ),
        (State::Several, "Copy 2 files and 1 folder to:"),
        (State::MoveMany, "Move 65 files (197.0 MiB) to:"),
    ] {
        h.show(state);
        h.layout(1340, 820);
        let form = h.main.global::<OperationPresentation>();
        assert_eq!(form.get_body(), line, "{state:?}");
        let first = modeless::edges(&h.item(AccessibleRole::Text, line));
        let field = modeless::edges(&h.item(AccessibleRole::TextInput, "Destination folder"));
        assert!(
            first.3 <= field.1 + 0.5 && field.1 - first.3 < 10.0,
            "{state:?}: the line sits right above the destination: {first:?} {field:?}"
        );
        let from = form.get_body_hint();
        let folder = h.folders[usize::from(state == State::MoveMany)]
            .display()
            .to_string();
        assert_eq!(from, format!("From: {folder}"), "{state:?}");
        h.close();
    }
}

/// Window widths below the dialog's desired 540 px narrow the dialog, so every wrap point
/// of a long name sweeps the first line, and a note under the destination, through the
/// widths of the regular and of the compact layout: the gate's layout checks find no text
/// cut anywhere, and the note stays under the field.
#[test]
fn a_long_first_line_and_a_note_wrap_whole_at_every_width() {
    let h = Harness::new(PreferenceTheme::Dark, UiDensity::Desktop, 13);
    h.focus(PaneId::Left, LONG_NAME);
    h.open(Key::F5, local());
    let form = h.main.global::<OperationPresentation>();
    assert!(form.get_body().contains(LONG_NAME), "{}", form.get_body());
    let mut cut = Vec::new();
    for noted in [false, true] {
        if noted {
            // A relative destination is refused, with a note under the field.
            h.chord(Key::Control, "a");
            h.key("relative folder");
            h.key(Key::Return);
            assert_eq!(
                form.get_warning(),
                "Enter the full path of an existing destination folder."
            );
        }
        for (height, step) in [(520, 2), (250, 6)] {
            for width in (320..=600).step_by(step) {
                h.layout(width, height);
                let findings = polish::item_findings(&h.popup());
                if !findings.is_empty() {
                    cut.push(format!("note {noted}, {width}x{height}: {findings:?}"));
                }
                if noted {
                    let field =
                        modeless::edges(&h.item(AccessibleRole::TextInput, "Destination folder"));
                    let note = modeless::edges(&h.item(AccessibleRole::Text, &form.get_warning()));
                    assert!(
                        note.1 >= field.3 && note.1 < field.3 + 24.0,
                        "{width}x{height}: the note sits under the field: {note:?} {field:?}"
                    );
                }
            }
        }
    }
    assert!(cut.is_empty(), "{cut:#?}");
    h.close();
}

/// The Copy and Move dialogs print no path that a narrow dialog would have to shorten:
/// the first line names the items, where they come from is its tooltip, whole, and the
/// destination is the editable field, which keeps every typed character at every width
/// (an edited path is never shortened).
#[test]
fn copy_and_move_print_no_path_and_keep_the_typed_destination_whole() {
    let h = Harness::new(PreferenceTheme::Dark, UiDensity::Desktop, 12);
    let mut long = documents();
    for part in [
        "Projects",
        "Midnight Explorer",
        "release candidates",
        "2026-11-23 nightly build with symbols",
        "artifacts",
        "packages",
    ] {
        long.push(part);
    }
    let long = long.display().to_string();
    for (key, verb) in [(Key::F5, "Copy "), (Key::F6, "Move ")] {
        h.focus(PaneId::Left, "Q3 report.pdf");
        h.open(key, local());
        let form = h.main.global::<OperationPresentation>();
        assert!(form.get_body().starts_with(verb), "{}", form.get_body());
        assert_eq!(
            form.get_body_hint(),
            format!("From: {}", h.folders[0].display())
        );
        h.chord(Key::Control, "a");
        h.key(long.as_str());
        for (width, height) in [(1340, 820), (520, 400), (320, 200)] {
            h.layout(width, height);
            let field = h.item(AccessibleRole::TextInput, "Destination folder");
            assert_eq!(
                field
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .as_deref(),
                Some(long.as_str()),
                "{verb}at {width}: the typed destination stays whole"
            );
            assert!(
                !form.get_body().contains(['\\', '/']),
                "{verb}at {width}: the first line names no path: {}",
                form.get_body()
            );
        }
        h.close();
    }
}
