//! A narrow panel gives file names their width before its metadata columns, and a
//! panel without shown entries says why below `..`, which still goes up.
#[path = "support/render.rs"]
mod render;
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, Orientation, SimpleText},
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::{
    ComponentHandle, SharedString,
    platform::{Key, WindowAdapter},
};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};

/// Both themes the acceptance runs use, at desktop and touch density.
const APPEARANCES: [(PreferenceTheme, UiDensity); 4] = [
    (PreferenceTheme::Dark, UiDensity::Desktop),
    (PreferenceTheme::Light, UiDensity::Desktop),
    (PreferenceTheme::Dark, UiDensity::Touch),
    (PreferenceTheme::Light, UiDensity::Touch),
];

struct Panel {
    test: ui::TestUi,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    /// The left panel's folder; it is below the root, so the panel has `..`.
    path: PathBuf,
    appearance: (PreferenceTheme, UiDensity),
}

impl Panel {
    fn new(preferences: UserPreferences) -> Self {
        let test = ui::TestUi::new();
        let path = std::env::temp_dir()
            .join("midnight-panel-fit")
            .join("listing");
        let state = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let sink = commands.clone();
        install(
            &test.window,
            state.clone(),
            Rc::new(move |command| {
                sink.borrow_mut().push(command);
                true
            }),
        );
        state
            .borrow_mut()
            .apply(Event::PreferencesLoaded(Ok(Box::new(
                AcceptedPreferences {
                    preferences,
                    comparators: FileComparatorSnapshot::default(),
                },
            ))));
        Self {
            test,
            state,
            commands,
            path,
            appearance: (PreferenceTheme::Blue, UiDensity::Desktop),
        }
    }

    /// Answer the left panel's latest listing request.
    fn list(&mut self, entries: Vec<Entry>, hidden: usize) {
        let navigation = self
            .commands
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
            .expect("the left panel asks for its listing");
        let entries: Arc<[Entry]> = entries.into();
        self.state.borrow_mut().apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            index: Arc::new(DirectoryIndex {
                hidden,
                ..DirectoryIndex::from_entries(&entries)
            }),
            entries,
        });
        self.draw();
    }

    /// Refresh, then draw in this test's theme and density.
    fn draw(&mut self) {
        refresh(&self.test.window, &self.state.borrow());
        let (theme, density) = self.appearance;
        self.test.window.invoke_apply_preferences(
            theme.index(),
            if density == UiDensity::Touch { 1 } else { 0 },
        );
        self.test.draw();
    }

    fn key(&mut self, key: impl Into<SharedString>) {
        ui::platform_keys::key(&self.test.window, key.into());
        self.draw();
    }

    fn navigated_to(&self, path: &std::path::Path) -> bool {
        self.commands.borrow().iter().any(|command| {
            matches!(command, Command::Navigate { pane: PaneId::Left, path: to, .. } if to == path)
        })
    }
}

fn files(path: &std::path::Path, names: &[String]) -> Vec<Entry> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            let mut entry = Entry::new(EntryId(index as u64 + 1), path.join(name), EntryKind::File);
            entry.metadata.size = Some(1_000 + index as u64);
            entry.metadata.localized_modified = Some(LocalizedDateTime {
                date: "11/20/2026".into(),
                time: "03:00:52 PM".into(),
            });
            entry
        })
        .collect()
}

/// A drawn name is wrapped in left-to-right isolation marks; compare what it says.
fn same_caption(text: &str, caption: &str) -> bool {
    text.strip_prefix('\u{2066}')
        .and_then(|text| text.strip_suffix('\u{2069}'))
        .unwrap_or(text)
        == caption
}

fn texts(item: ItemRc, caption: &str, found: &mut Vec<ItemRc>) {
    let text = item
        .downcast::<SimpleText>()
        .map(|text| text.as_pin_ref().text())
        .or_else(|| {
            item.downcast::<ComplexText>()
                .map(|text| text.as_pin_ref().text())
        });
    if item.is_visible() && text.is_some_and(|text| same_caption(&text, caption)) {
        found.push(item.clone());
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        texts(item, caption, found);
    }
}

/// The left panel's horizontal extent.
fn left_panel(test: &ui::TestUi) -> (f32, f32) {
    let left = test.window.get_left_list_left();
    (left, left + test.window.get_left_list_width())
}

/// The visible text items in the left list that say exactly `caption` (not, say, the
/// status bar's copy of the focused name).
fn shown(test: &ui::TestUi, caption: &str) -> Vec<ItemRc> {
    let mut found = Vec::new();
    texts(
        ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
        caption,
        &mut found,
    );
    let (left, right) = left_panel(test);
    let top = test.window.get_left_list_top();
    let bottom = top + test.window.get_left_list_height();
    found.retain(|item| {
        let at = item.map_to_native_window(item.geometry().origin);
        at.x >= left - 0.5 && at.x < right && at.y >= top - 0.5 && at.y < bottom
    });
    found
}

/// A visible column heading of the left panel (a button labelled "Sort by …").
fn heading(test: &ui::TestUi, label: &str) -> Option<ItemRc> {
    fn find(item: ItemRc, label: &str, found: &mut Vec<ItemRc>) {
        if item.is_visible()
            && item.accessible_role() == AccessibleRole::Button
            && item
                .accessible_string_property(AccessibleStringProperty::Label)
                .is_some_and(|value| value == label)
        {
            found.push(item.clone());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            find(item, label, found);
        }
    }
    let mut found = Vec::new();
    find(
        ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
        label,
        &mut found,
    );
    let (left, right) = left_panel(test);
    found.into_iter().find(|item| {
        let x = item.map_to_native_window(item.geometry().origin).x;
        x >= left - 0.5 && x < right
    })
}

/// The width a text takes unelided, and the width it was given.
fn widths(test: &ui::TestUi, item: &ItemRc) -> (f32, f32) {
    let adapter: Rc<dyn WindowAdapter> = test.native.clone();
    let rect = item.geometry();
    let natural = item
        .borrow()
        .as_ref()
        .layout_info(Orientation::Horizontal, rect.height(), &adapter, item)
        .preferred;
    (natural, rect.width())
}

fn whole(test: &ui::TestUi, item: &ItemRc) -> bool {
    let (natural, given) = widths(test, item);
    given + 0.5 >= natural
}

fn left_edge(item: &ItemRc) -> f32 {
    item.map_to_native_window(item.geometry().origin).x
}

fn right_edge(item: &ItemRc) -> f32 {
    left_edge(item) + item.geometry().width()
}

fn top_edge(item: &ItemRc) -> f32 {
    item.map_to_native_window(item.geometry().origin).y
}

/// The left panel at its narrowest: the splitter dragged to the window's edge.
fn narrow(panel: &mut Panel) {
    panel.test.window.set_panel_ratio(0.0);
    panel.draw();
    let width = panel.test.window.get_left_list_width();
    assert!(
        (250.0..=290.0).contains(&width),
        "the left list is at its minimum width: {width}"
    );
}

fn tag(appearance: (PreferenceTheme, UiDensity)) -> String {
    format!("{:?}-{:?}", appearance.0, appearance.1).to_lowercase()
}

#[test]
fn a_narrow_panel_shows_short_names_whole_and_drops_time_and_date_first() {
    let names: Vec<String> = (0..30)
        .map(|index| format!("n{index:02}.e{:02}", (index * 7) % 50))
        .collect();
    let mut panel = Panel::new(UserPreferences {
        size_display: SizeDisplay::Bytes,
        ..Default::default()
    });
    let entries = files(&panel.path, &names);
    panel.list(entries, 0);
    for appearance in APPEARANCES {
        panel.appearance = appearance;
        narrow(&mut panel);
        let list_right =
            panel.test.window.get_left_list_left() + panel.test.window.get_left_list_width();
        let mut checked = 0;
        for name in &names {
            for item in shown(&panel.test, name) {
                assert!(
                    whole(&panel.test, &item),
                    "{appearance:?}: {name} is cut at the minimum panel width: {:?}",
                    widths(&panel.test, &item)
                );
                checked += 1;
            }
        }
        assert!(checked >= 5, "{appearance:?}: only {checked} names shown");
        // The time of day goes first, then the date; Size stays while it fits.
        assert!(heading(&panel.test, "Sort by Modified time").is_none());
        let size = heading(&panel.test, "Sort by Size");
        if appearance.1 == UiDensity::Desktop {
            assert!(
                size.is_some(),
                "{appearance:?}: Size fits beside short names"
            );
        }
        if let Some(date) = heading(&panel.test, "Sort by Modified") {
            assert!(size.is_some(), "the date never shows without Size");
            assert!(right_edge(&date) <= list_right + 0.5);
        }
        if let Some(size) = &size {
            assert!(right_edge(size) <= list_right + 0.5);
            // A shown size is never elided.
            for item in shown(&panel.test, "1000") {
                assert!(whole(&panel.test, &item), "{appearance:?}: a size is cut");
            }
        }
        render::snapshot(
            &panel.test,
            &format!("panel-narrow-short-{}", tag(appearance)),
        );
        // Wider, every column comes back whole, and so do the names.
        panel.test.window.set_panel_ratio(0.4);
        panel.draw();
        for label in ["Sort by Size", "Sort by Modified", "Sort by Modified time"] {
            assert!(
                heading(&panel.test, label).is_some(),
                "{appearance:?}: {label}"
            );
        }
        for caption in [names[0].as_str(), "1000", "11/20/2026", "03:00:52 PM"] {
            let item = shown(&panel.test, caption)
                .pop()
                .unwrap_or_else(|| panic!("{caption} is shown"));
            assert!(
                whole(&panel.test, &item),
                "{appearance:?}: {caption} is cut"
            );
        }
    }
}

/// Long names cannot fit a narrow panel, but the metadata gives way first: Time, then
/// the date, then Size, as far as each fits after the name's room, which keeps a short
/// name whole. The fonts decide what fits: with Segoe UI on Windows, the date still
/// does at desktop density, beside a long name cut to that room.
#[test]
fn a_narrow_panel_with_long_names_drops_metadata_before_cutting_them() {
    let names: Vec<String> = (0..12)
        .map(|index| format!("a fairly long file name number {index:02} with several words.txt"))
        .collect();
    // Listed first, so that it stays in view at touch density too.
    let short = "1-notes.txt";
    let listed: Vec<String> = std::iter::once(short.to_owned())
        .chain(names.iter().cloned())
        .collect();
    let mut panel = Panel::new(UserPreferences::default());
    let entries = files(&panel.path, &listed);
    panel.list(entries, 0);
    for appearance in APPEARANCES {
        panel.appearance = appearance;
        narrow(&mut panel);
        let list_right =
            panel.test.window.get_left_list_left() + panel.test.window.get_left_list_width();
        // The metadata has degraded: the time is hidden, and the date never shows
        // without Size.
        assert!(heading(&panel.test, "Sort by Modified time").is_none());
        let size = heading(&panel.test, "Sort by Size");
        if let Some(date) = heading(&panel.test, "Sort by Modified") {
            assert!(
                size.is_some(),
                "{appearance:?}: the date never shows without Size"
            );
            assert!(right_edge(&date) <= list_right + 0.5);
        }
        let item = shown(&panel.test, &names[0])
            .pop()
            .expect("the first long name is shown");
        let (natural, given) = widths(&panel.test, &item);
        assert!(
            given < natural,
            "the long name cannot fit the minimum width"
        );
        // The name keeps its room: a short name is whole, and the long one has as much.
        let short_item = shown(&panel.test, short)
            .pop()
            .unwrap_or_else(|| panic!("{appearance:?}: {short} is shown"));
        let (short_natural, _) = widths(&panel.test, &short_item);
        assert!(
            whole(&panel.test, &short_item) && given + 0.5 >= short_natural,
            "{appearance:?}: the metadata leaves the names {given} px, {short} needs \
             {short_natural}"
        );
        if let Some(size) = &size {
            assert!(
                left_edge(size) >= left_edge(&item) + given,
                "Size starts after the name"
            );
        }
        render::snapshot(
            &panel.test,
            &format!("panel-narrow-long-{}", tag(appearance)),
        );
    }
}

/// `message` is shown below the `..` row and takes some room.
fn says_below_parent(panel: &Panel, message: &str) {
    let parent = shown(&panel.test, "..")
        .pop()
        .expect("the `..` row is shown");
    let parent_bottom = top_edge(&parent) + parent.geometry().height();
    let item = shown(&panel.test, message)
        .pop()
        .unwrap_or_else(|| panic!("the panel does not say {message:?}"));
    assert!(
        top_edge(&item) >= parent_bottom,
        "{message:?} must be below `..`: {} < {parent_bottom}",
        top_edge(&item)
    );
    assert!(whole(&panel.test, &item), "{message:?} is cut");
}

#[test]
fn an_empty_folder_says_so_below_parent_which_still_goes_up() {
    let mut panel = Panel::new(UserPreferences::default());
    panel.list(Vec::new(), 0);
    let parent_folder = panel.path.parent().unwrap().to_path_buf();
    let child = Entry::new(EntryId(1), panel.path.clone(), EntryKind::Directory);
    for appearance in APPEARANCES {
        panel.appearance = appearance;
        panel.draw();
        says_below_parent(&panel, "This folder is empty");
        render::snapshot(&panel.test, &format!("panel-empty-{}", tag(appearance)));
        // The message takes no keys: Enter on `..`, and Backspace, still go up; then
        // Enter on the folder left (the cursor is on it) comes back.
        for key in [Key::Return, Key::Backspace] {
            panel.test.window.invoke_focus_files();
            panel.commands.borrow_mut().clear();
            panel.key(key);
            assert!(
                panel.navigated_to(&parent_folder),
                "{appearance:?}: {key:?} in the empty folder does not go up: {:?}",
                panel.commands.borrow()
            );
            panel.list(vec![child.clone()], 0);
            panel.commands.borrow_mut().clear();
            panel.key(Key::Return);
            assert!(
                panel.navigated_to(&panel.path),
                "{appearance:?}: Enter on the folder left does not go back into it"
            );
            panel.list(Vec::new(), 0);
            says_below_parent(&panel, "This folder is empty");
        }
    }
}

#[test]
fn entries_that_are_all_hidden_are_counted_instead_of_an_empty_folder() {
    let mut panel = Panel::new(UserPreferences::default());
    panel.list(Vec::new(), 3);
    for appearance in APPEARANCES {
        panel.appearance = appearance;
        panel.draw();
        says_below_parent(&panel, "All 3 items here are hidden");
        assert!(shown(&panel.test, "This folder is empty").is_empty());
        render::snapshot(
            &panel.test,
            &format!("panel-all-hidden-{}", tag(appearance)),
        );
    }
}

#[test]
fn a_quick_filter_without_matches_says_so_below_parent() {
    let mut panel = Panel::new(UserPreferences {
        quick_search_filter: true,
        ..Default::default()
    });
    let names = ["alpha.txt".to_owned(), "beta.txt".to_owned()];
    let entries = files(&panel.path, &names);
    panel.list(entries, 0);
    panel.test.window.invoke_focus_files();
    panel.commands.borrow_mut().clear();
    panel.key("z");
    let request = panel
        .commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Search(request) => Some(request.clone()),
            _ => None,
        })
        .expect("typing searches the listing");
    panel.state.borrow_mut().apply(Event::SearchReady {
        pane: request.pane,
        navigation: request.navigation,
        revision: request.revision,
        result: Arc::new(SearchResult {
            source: request.source.clone(),
            filtered: request
                .source
                .iter()
                .enumerate()
                .map(|(source_row, _)| SearchMatch {
                    source_row,
                    display_prefix_end: 0,
                })
                .collect::<Vec<_>>()
                .into(),
            filtered_index: Arc::new(DirectoryIndex::from_entries(&request.source)),
            matches: Arc::from([]),
            index: Arc::new(DirectoryIndex::default()),
        }),
    });
    for appearance in APPEARANCES {
        panel.appearance = appearance;
        panel.draw();
        says_below_parent(&panel, "No matching names");
        assert!(shown(&panel.test, "alpha.txt").is_empty());
    }
    panel.key(Key::Escape);
    assert!(shown(&panel.test, "No matching names").is_empty());
}
