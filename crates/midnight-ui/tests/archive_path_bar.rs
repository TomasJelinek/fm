//! Inside an archive the path bar names the native folders that hold the archive, then
//! the archive and its folders, each a part of its own: a shortened path keeps the
//! archive's name and the current folder whole, and a folder's part leaves the archive.
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

#[derive(Debug)]
struct Mount;
impl ArchiveMount for Mount {
    fn id(&self) -> u64 {
        42
    }
    fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
        panic!("the UI lists nothing itself")
    }
    fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
        panic!("the UI reads nothing itself")
    }
    fn validate(&self) -> Result<(), FsError> {
        panic!("the UI validates nothing itself")
    }
}

fn files(folder: &std::path::Path, names: &[&str]) -> Vec<Entry> {
    names
        .iter()
        .enumerate()
        .map(|(index, name)| {
            Entry::new(
                EntryId(index as u64 + 1),
                folder.join(name),
                EntryKind::File,
            )
        })
        .collect()
}

struct Panel {
    test: ui::TestUi,
    state: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    /// The deep native folder that holds `bundle.zip`, as in the acceptance runs.
    folder: PathBuf,
    appearance: (PreferenceTheme, UiDensity),
}

impl Panel {
    fn new() -> Self {
        let test = ui::TestUi::new();
        let folder = [
            "midnight-archive-path",
            "acceptance-panels",
            "20261120-143744",
            "work",
            "panels_root_keys",
            "staged",
            "run",
            "fixture",
            "archives",
        ]
        .iter()
        .fold(std::env::temp_dir(), |path, name| path.join(name));
        let state = Rc::new(RefCell::new(Presenter::new(folder.clone(), folder.clone())));
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
        let mut panel = Self {
            test,
            state,
            commands,
            folder,
            appearance: (PreferenceTheme::Blue, UiDensity::Desktop),
        };
        panel.list(files(&panel.folder, &["bundle.zip", "notes.txt"]));
        panel
    }

    /// The left panel's latest request, native or in an archive.
    fn navigation(&self) -> NavigationId {
        self.commands
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
                }
                | Command::ArchiveNavigate {
                    pane: PaneId::Left,
                    navigation,
                    ..
                } => Some(*navigation),
                _ => None,
            })
            .expect("the left panel asks for a listing")
    }

    /// Answer the left panel's latest native listing request.
    fn list(&mut self, entries: Vec<Entry>) {
        let navigation = self.navigation();
        let entries: Arc<[Entry]> = entries.into();
        self.state.borrow_mut().apply(Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
        self.draw();
    }

    /// List `location` of the archive with its folders `names`.
    fn list_archive(&mut self, location: ArchiveLocation, names: &[&str]) {
        let navigation = self.navigation();
        let entries: Arc<[Entry]> = names
            .iter()
            .enumerate()
            .map(|(index, name)| {
                Entry::from_archive(ArchiveNode {
                    id: VirtualEntryId {
                        mount: 42,
                        entry: location.directory * 10 + index as u64 + 1,
                    },
                    parent: location.directory,
                    raw_name: name.as_bytes().into(),
                    display_name: (*name).into(),
                    kind: EntryKind::Directory,
                    size: None,
                    readable: false,
                    modified: None,
                    access: Default::default(),
                })
            })
            .collect::<Vec<_>>()
            .into();
        self.state.borrow_mut().apply(Event::ArchiveListed {
            pane: PaneId::Left,
            navigation,
            result: Ok(ArchiveListing {
                location,
                index: Arc::new(DirectoryIndex::from_entries(&entries)),
                entries,
            }),
        });
        self.draw();
    }

    /// Open the first folder below `..` and list it with its folders `names`.
    fn open_first(&mut self, names: &[&str]) {
        self.test.window.invoke_focus_files();
        self.key(Key::Home);
        self.key(Key::DownArrow);
        self.key(Key::Return);
        let location = self
            .commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::ArchiveNavigate {
                    pane: PaneId::Left,
                    action: ArchiveAction::List(location),
                    ..
                } => Some(location.clone()),
                _ => None,
            })
            .expect("Enter on a folder of the archive lists it");
        self.list_archive(location, names);
    }

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

    fn root(&self) -> ArchiveLocation {
        ArchiveLocation {
            mount: Some(Arc::new(Mount)),
            address: ArchiveAddress {
                provider: "midnight.archive".into(),
                format: None,
                container: self.folder.join("bundle.zip"),
                source_version: vec![1],
                components: Vec::new(),
            },
            directory: 1,
            trail: vec![(1, "bundle.zip".into())],
        }
    }
}

fn visit(item: ItemRc, found: &mut dyn FnMut(&ItemRc)) {
    found(&item);
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        visit(item, found);
    }
}

fn left_edge(item: &ItemRc) -> f32 {
    item.map_to_native_window(item.geometry().origin).x
}

/// Visible buttons above the left list whose accessible label passes `keep`, left
/// to right.
fn bar_buttons(test: &ui::TestUi, keep: &dyn Fn(&str) -> bool) -> Vec<(String, ItemRc)> {
    let mut found = Vec::new();
    visit(
        ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
        &mut |item| {
            if item.is_visible()
                && item.accessible_role() == AccessibleRole::Button
                && let Some(label) =
                    item.accessible_string_property(AccessibleStringProperty::Label)
                && keep(&label)
            {
                found.push((label.to_string(), item.clone()));
            }
        },
    );
    let left = test.window.get_left_list_left();
    let right = left + test.window.get_left_list_width();
    let top = test.window.get_left_list_top();
    found.retain(|(_, item)| {
        let at = item.map_to_native_window(item.geometry().origin);
        at.x >= left - 0.5 && at.x < right && at.y < top
    });
    found.sort_by(|(_, a), (_, b)| left_edge(a).total_cmp(&left_edge(b)));
    found
}

/// The shown parts of the left path bar, which shows `location`: each part's path.
fn parts(test: &ui::TestUi, location: &str) -> Vec<(String, ItemRc)> {
    bar_buttons(test, &|label| {
        label
            .strip_prefix("Open ")
            .is_some_and(|prefix| location.starts_with(prefix))
    })
    .into_iter()
    .map(|(label, item)| (label["Open ".len()..].to_owned(), item))
    .collect()
}

fn markers(test: &ui::TestUi) -> usize {
    bar_buttons(test, &|label| label == "Edit full folder path").len()
}

/// Whether a part shows its whole caption.
fn whole(test: &ui::TestUi, part: &ItemRc) -> bool {
    let mut caption = None;
    visit(part.clone(), &mut |item| {
        if caption.is_none()
            && (item.downcast::<SimpleText>().is_some() || item.downcast::<ComplexText>().is_some())
        {
            caption = Some(item.clone());
        }
    });
    let caption = caption.expect("a part has a caption");
    let adapter: Rc<dyn WindowAdapter> = test.native.clone();
    let rect = caption.geometry();
    let natural = caption
        .borrow()
        .as_ref()
        .layout_info(Orientation::Horizontal, rect.height(), &adapter, &caption)
        .preferred;
    rect.width() + 0.5 >= natural
}

fn shown_whole(test: &ui::TestUi, location: &str, path: &str, case: &str) {
    let shown = parts(test, location);
    let (_, part) = shown
        .iter()
        .find(|(prefix, _)| prefix == path)
        .unwrap_or_else(|| {
            panic!(
                "{case}: {path} is not shown in {:?}",
                shown.iter().map(|(prefix, _)| prefix).collect::<Vec<_>>()
            )
        });
    assert!(whole(test, part), "{case}: {path} is cut");
}

fn text(path: &std::path::Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn a_shortened_archive_path_keeps_the_archive_and_the_current_folder_whole() {
    let mut panel = Panel::new();
    let root = panel.root();
    panel.list_archive(root, &["bundle"]);
    let archive = panel.folder.join("bundle.zip");
    let inner = archive.join("bundle");
    let current = inner
        .join("a second folder with a rather long name")
        .join("current");
    for appearance in [
        (PreferenceTheme::Blue, UiDensity::Desktop),
        (PreferenceTheme::Dark, UiDensity::Touch),
    ] {
        panel.appearance = appearance;
        for ratio in [0.5, 0.0] {
            panel.test.window.set_panel_ratio(ratio);
            panel.draw();
            let case = format!("{appearance:?} at {ratio}, archive root");
            // The archive is the last part; its folders were cut at its end before.
            shown_whole(&panel.test, &text(&archive), &text(&archive), &case);
            assert!(markers(&panel.test) >= 1, "{case}: the middle is omitted");
            for (prefix, part) in parts(&panel.test, &text(&archive)) {
                assert!(whole(&panel.test, &part), "{case}: {prefix} is cut");
            }
        }
    }
    panel.appearance = (PreferenceTheme::Blue, UiDensity::Desktop);
    panel.open_first(&["a second folder with a rather long name"]);
    panel.open_first(&["current"]);
    panel.open_first(&[]);
    for appearance in [
        (PreferenceTheme::Blue, UiDensity::Desktop),
        (PreferenceTheme::Dark, UiDensity::Touch),
    ] {
        panel.appearance = appearance;
        for ratio in [0.5, 0.0] {
            panel.test.window.set_panel_ratio(ratio);
            panel.draw();
            let case = format!("{appearance:?} at {ratio}, {}", text(&current));
            shown_whole(&panel.test, &text(&current), &text(&archive), &case);
            shown_whole(&panel.test, &text(&current), &text(&current), &case);
            let shown = parts(&panel.test, &text(&current));
            for (prefix, part) in &shown {
                assert!(whole(&panel.test, part), "{case}: {prefix} is cut");
            }
            if ratio == 0.0 {
                // The folders between the archive and the current one are omitted
                // too, behind a marker of their own.
                assert!(
                    !shown.iter().any(|(prefix, _)| *prefix == text(&inner)),
                    "{case}: the narrow bar has no room for every folder"
                );
                assert_eq!(markers(&panel.test), 2, "{case}");
            }
            render::snapshot(
                &panel.test,
                &format!("archive-path-{:?}-{:?}-{ratio}", appearance.0, appearance.1)
                    .to_lowercase(),
            );
        }
    }
}

#[test]
fn a_native_folder_of_an_archive_path_leaves_the_archive_for_it() {
    let mut panel = Panel::new();
    let root = panel.root();
    panel.list_archive(root, &["bundle"]);
    panel.open_first(&[]);
    // Wide enough for the whole path: every folder has its part.
    panel.test.set_size(2400, 820);
    panel.test.window.set_panel_ratio(1.0);
    panel.draw();
    let location = text(&panel.folder.join("bundle.zip").join("bundle"));
    assert_eq!(markers(&panel.test), 0, "the whole path fits");
    let shown: Vec<String> = parts(&panel.test, &location)
        .into_iter()
        .map(|(prefix, _)| prefix)
        .collect();
    let expected: Vec<String> = panel
        .folder
        .join("bundle.zip")
        .join("bundle")
        .ancestors()
        .map(text)
        .filter(|prefix| !prefix.is_empty())
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    assert_eq!(
        shown, expected,
        "one part per folder, the archive's own last but one"
    );
    // The folder that holds the archive: it lists natively, cursor on the archive.
    panel.commands.borrow_mut().clear();
    let position = ui::root_position(
        &panel.test.window,
        &format!("Open {}", text(&panel.folder)),
        AccessibleRole::Button,
    );
    ui::click(&panel.test.window, position.x, position.y);
    panel.draw();
    assert!(
        panel.commands.borrow().iter().any(|command| matches!(
            command,
            Command::Navigate { pane: PaneId::Left, path, .. } if *path == panel.folder
        )),
        "the part lists its native folder: {:?}",
        panel.commands.borrow()
    );
    let entries = files(&panel.folder, &["bundle.zip", "notes.txt"]);
    panel.list(entries);
    assert_eq!(
        panel.test.window.get_left_cursor(),
        1,
        "the cursor is on the archive"
    );
    let shown: Vec<String> = parts(&panel.test, &text(&panel.folder))
        .into_iter()
        .map(|(prefix, _)| prefix)
        .collect();
    assert_eq!(shown.last(), Some(&text(&panel.folder)), "{shown:?}");
}
