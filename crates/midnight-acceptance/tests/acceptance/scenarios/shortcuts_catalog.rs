//! The shortcut catalog sweep: every default binding of `shortcut_catalog()`, its
//! alternates included, is pressed in the context its scope names and must do what its
//! action says, and nothing else.
//!
//! The scenario walks the catalog itself, so an action or key added later fails the
//! sweep ("no probe") until it is covered here. Each probe prepares its context (cursor
//! row, marks, a filename search, the command editor, the drives popup, seeded tabs,
//! Hot Paths or view mode), records the application's visible state, presses the key,
//! waits for the action's own effect and then compares the whole state again: any other
//! change fails the probe. A few probes press keys in a context where their action does
//! not apply: Space, a digit and Insert in filename search; F7, Ctrl+A and a digit in
//! the command editor; the number row's + − * and an uppercase letter in the file list,
//! which are typed into a filename search. `footer` clicks the F1–F12 footer cells of
//! each modifier layer with the same probes.
//!
//! Number-row digits, the keypad's + − * and modified bracket keys go through the
//! window's physical-shortcut entry, as the native key hook delivers them (a key the
//! hook does not take then arrives as typed text); every other key is a Slint key event.
//! Viewer and comparison keys belong to viewer windows, which only the staged viewer
//! processes have; so do the services behind F3, Alt+F3 and Ctrl+Shift+C.
use crate::{
    Context,
    app::{self, App, Config},
    input, inspect,
    options_support::SettingsStore,
    panel,
    tools_support::{self as tools},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{
    Bookmark, BookmarkId, CommandHistoryEntry, FileClipboardIntent, PaneId, PanelViewMode,
    ShellFlavor, ShortcutChord, ShortcutContext, ShortcutDefinition, ShortcutMap, ShortcutScope,
    shortcut_catalog,
};
use midnight_ui::{FileListPresentation, HotPathsPresentation, PanelChoices};
use slint::{
    ComponentHandle, Model, SharedString,
    platform::{Key, WindowAdapter},
};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

const LEFT: PaneId = PaneId::Left;
/// Numbered files beyond one page of the list, so paging is not the same as End.
const NUMBERED: usize = 60;
const HISTORY: [&str; 2] = ["echo first-entry", "echo second-entry"];

/// The sweep's folders.
#[derive(Clone)]
pub struct Paths {
    pub root: PathBuf,
    pub left: PathBuf,
    pub right: PathBuf,
    pub sub: PathBuf,
    pub two: PathBuf,
    /// Hot Path folders: index 0 is number 1, index 9 number 10 (the 0 key).
    pub hot: Vec<PathBuf>,
}

fn write(path: &Path, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
    }
    fs::write(path, text).map_err(|error| format!("{}: {error}", path.display()))
}

/// `keys/left` with two folders, a hidden file, three named files and `NUMBERED`
/// numbered ones; `keys/right`; ten Hot Path folders `keys/hot/h1`…`h10`.
pub fn fixture(work: &Path) -> Result<Paths, String> {
    let root = work.join("keys");
    let left = root.join("left");
    let right = root.join("right");
    let paths = Paths {
        sub: left.join("sub"),
        two: left.join("two"),
        hot: (1..=10)
            .map(|slot| root.join("hot").join(format!("h{slot}")))
            .collect(),
        root: root.clone(),
        left: left.clone(),
        right: right.clone(),
    };
    write(&paths.sub.join("inner.txt"), "inner\n")?;
    write(&paths.two.join("x.txt"), "x\n")?;
    for name in [".hidden.txt", "alpha.txt", "beta.txt", "gamma.dat"] {
        write(&left.join(name), name)?;
    }
    for index in 0..NUMBERED {
        write(&left.join(format!("n-{index:03}.txt")), "n\n")?;
    }
    write(&right.join("other.txt"), "other\n")?;
    fs::create_dir_all(right.join("right-sub")).map_err(|error| error.to_string())?;
    for folder in &paths.hot {
        write(&folder.join("here.txt"), "here\n")?;
    }
    Ok(paths)
}

/// What a probe's application starts from, besides the Hot Paths and history every
/// run has.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seed {
    Default,
    Brief,
    /// Two tabs in the left panel (the folder and `sub`), the second active.
    Tabs2,
    /// Nine tabs in the left panel, this one active.
    Tabs9(usize),
}

fn tab_paths(paths: &Paths) -> Vec<PathBuf> {
    let mut tabs = vec![paths.left.clone(), paths.sub.clone(), paths.two.clone()];
    tabs.extend(paths.hot.iter().take(6).cloned());
    tabs
}

pub fn store(paths: &Paths, seed: Seed) -> Arc<SettingsStore> {
    let paths = paths.clone();
    SettingsStore::seeded(move |preferences| {
        preferences.bookmarks = (1..=10_u8)
            .map(|slot| Bookmark {
                id: BookmarkId(u64::from(slot)),
                name: format!("h{slot}"),
                path: paths.hot[usize::from(slot) - 1].clone(),
                archive: None,
                network: None,
                slot: Some(slot),
            })
            .collect();
        preferences.command_history = HISTORY
            .iter()
            .map(|command| CommandHistoryEntry {
                command: (*command).to_owned(),
                shell: ShellFlavor::Posix,
            })
            .collect();
        match seed {
            Seed::Default => {}
            Seed::Brief => preferences.panel_view_modes[0] = PanelViewMode::Brief,
            Seed::Tabs2 => {
                preferences.tabs[0].paths = vec![paths.left.clone(), paths.sub.clone()];
                preferences.tabs[0].active = 1;
            }
            Seed::Tabs9(active) => {
                preferences.tabs[0].paths = tab_paths(&paths);
                preferences.tabs[0].active = active;
            }
        }
    })
}

pub fn start(paths: &Paths, seed: Seed) -> App {
    let mut config = Config::new(&paths.left, &paths.right);
    config.settings = Some(store(paths, seed));
    config.clipboard = true;
    App::start(config)
}

/// Everything a shortcut could visibly change, read from the windows.
#[derive(Clone, Debug, Default)]
pub struct State {
    pub location: [Option<PathBuf>; 2],
    pub cursor: [i32; 2],
    pub names: Vec<String>,
    pub selection: [Vec<String>; 2],
    pub focus: i32,
    pub active: i32,
    pub view: [i32; 2],
    pub sort: [(i32, bool); 2],
    pub tabs: Vec<(String, bool)>,
    pub right_tabs: usize,
    pub bookmarks: Vec<(String, i32)>,
    pub swapped: bool,
    pub zoomed: bool,
    pub hidden: bool,
    pub operation: Option<(String, bool)>,
    pub copy_draft: Option<bool>,
    pub metadata: Option<bool>,
    pub mask: Option<bool>,
    pub options: bool,
    pub about: bool,
    pub notice: Option<String>,
    pub tools: Vec<String>,
    pub volume: i32,
    pub popups: usize,
    pub menu: &'static str,
    pub filter: bool,
    pub comparison: bool,
    pub file_list: bool,
    pub hot_paths: bool,
    pub command_focus: bool,
    pub command_text: String,
    pub search: [(bool, String); 2],
    pub sizes: [usize; 2],
    pub path_editor: bool,
    pub clipboard: Option<(FileClipboardIntent, Vec<PathBuf>)>,
    pub visible: bool,
    pub listings: [usize; 2],
    pub copies: usize,
}

fn selected(app: &App, pane: PaneId) -> Vec<String> {
    let mut names: Vec<String> = app
        .rows(pane)
        .iter()
        .filter(|row| row.selected && !row.parent_entry)
        .map(|row| row.name.to_string())
        .collect();
    names.sort();
    names
}

fn measured(app: &App, pane: PaneId) -> usize {
    app.rows(pane)
        .iter()
        .filter(|row| {
            row.directory
                && !row.parent_entry
                && !row.size_running
                && row.size.chars().any(|c| c.is_ascii_digit())
        })
        .count()
}

impl State {
    pub fn of(app: &App) -> Self {
        let window = &app.window;
        let choices = window.global::<PanelChoices>();
        let popups = inspect::roots(app.slint()).len().saturating_sub(1);
        let texts = if popups > 0 {
            inspect::visible_texts(app.slint())
        } else {
            Vec::new()
        };
        let menu = if tools::has(app.slint(), AccessibleRole::Combobox, "Menu category") {
            "main"
        } else if texts.iter().any(|text| text == "User menu") {
            "user"
        } else {
            ""
        };
        let mut tool_titles = tools::tool_titles(app);
        tool_titles.sort();
        Self {
            location: [app.location(PaneId::Left), app.location(PaneId::Right)],
            cursor: [app.cursor(PaneId::Left), app.cursor(PaneId::Right)],
            names: app.names(PaneId::Left),
            selection: [selected(app, PaneId::Left), selected(app, PaneId::Right)],
            focus: window.get_focused_pane(),
            active: window.get_active_pane(),
            view: [window.get_left_view_mode(), window.get_right_view_mode()],
            sort: [
                (window.get_left_sort(), window.get_left_descending()),
                (window.get_right_sort(), window.get_right_descending()),
            ],
            tabs: window
                .get_left_tabs()
                .iter()
                .map(|tab| (tab.path.to_string(), tab.active))
                .collect(),
            right_tabs: window.get_right_tabs().row_count(),
            bookmarks: window
                .get_bookmarks()
                .iter()
                .map(|bookmark| (bookmark.path.to_string(), bookmark.slot))
                .collect(),
            swapped: window.get_panels_swapped(),
            zoomed: window.get_panel_zoomed(),
            hidden: choices.get_show_hidden_files(),
            operation: (!window.get_operation_token().is_empty()).then(|| {
                (
                    window.get_operation_title().to_string(),
                    window.get_operation_dangerous(),
                )
            }),
            copy_draft: (!window.get_copy_draft_token().is_empty())
                .then(|| window.get_copy_draft_move()),
            metadata: (!window.get_metadata_token().is_empty())
                .then(|| window.get_metadata_start_case()),
            mask: (!choices.get_mask_token().is_empty()).then(|| choices.get_mask_select()),
            options: window.get_options_open(),
            about: window.get_about_open(),
            notice: tools::shown_notice(app).map(|(title, _)| title),
            tools: tool_titles,
            volume: window.get_volume_popup_pane(),
            popups,
            menu,
            filter: window.get_panel_filter_view().open,
            comparison: window.get_comparison_view().open,
            file_list: window.global::<FileListPresentation>().get_active(),
            hot_paths: !window
                .global::<HotPathsPresentation>()
                .get_token()
                .is_empty(),
            command_focus: window.get_command_focused(),
            command_text: window.get_command_text().to_string(),
            search: [
                (
                    window.get_left_search_active(),
                    window.get_left_search_query().to_string(),
                ),
                (
                    window.get_right_search_active(),
                    window.get_right_search_query().to_string(),
                ),
            ],
            sizes: [measured(app, PaneId::Left), measured(app, PaneId::Right)],
            path_editor: tools::has(app.slint(), AccessibleRole::TextInput, "Folder path"),
            clipboard: app
                .clipboard
                .as_ref()
                .and_then(|clipboard| clipboard.files())
                .map(|files| (files.intent, files.paths)),
            visible: app.slint().is_visible(),
            listings: app.listings,
            copies: app.copies.len(),
        }
    }

    /// Every compared field and its value.
    fn fields(&self) -> Vec<(&'static str, String)> {
        vec![
            ("location0", format!("{:?}", self.location[0])),
            ("location1", format!("{:?}", self.location[1])),
            ("cursor0", self.cursor[0].to_string()),
            ("cursor1", self.cursor[1].to_string()),
            ("selection0", format!("{:?}", self.selection[0])),
            ("selection1", format!("{:?}", self.selection[1])),
            ("focus", self.focus.to_string()),
            ("active", self.active.to_string()),
            ("view0", self.view[0].to_string()),
            ("view1", self.view[1].to_string()),
            ("sort0", format!("{:?}", self.sort[0])),
            ("sort1", format!("{:?}", self.sort[1])),
            // A tab's folder follows its panel's location; tabs change by count and
            // active position.
            (
                "tabs0",
                format!(
                    "{} tabs, active {:?}",
                    self.tabs.len(),
                    self.tabs.iter().position(|(_, active)| *active)
                ),
            ),
            ("tabs1", self.right_tabs.to_string()),
            ("bookmarks", format!("{:?}", self.bookmarks)),
            ("swapped", self.swapped.to_string()),
            ("zoomed", self.zoomed.to_string()),
            ("hidden", self.hidden.to_string()),
            ("operation", format!("{:?}", self.operation)),
            ("copy_draft", format!("{:?}", self.copy_draft)),
            ("metadata", format!("{:?}", self.metadata)),
            ("mask", format!("{:?}", self.mask)),
            ("options", self.options.to_string()),
            ("about", self.about.to_string()),
            ("notice", format!("{:?}", self.notice)),
            ("tools", format!("{:?}", self.tools)),
            ("volume", self.volume.to_string()),
            ("popups", self.popups.to_string()),
            ("menu", self.menu.to_owned()),
            ("filter", self.filter.to_string()),
            ("comparison", self.comparison.to_string()),
            ("file_list", self.file_list.to_string()),
            ("hot_paths", self.hot_paths.to_string()),
            ("command_focus", self.command_focus.to_string()),
            ("command_text", format!("{:?}", self.command_text)),
            ("search0", format!("{:?}", self.search[0])),
            ("search1", format!("{:?}", self.search[1])),
            ("sizes0", self.sizes[0].to_string()),
            ("sizes1", self.sizes[1].to_string()),
            ("path_editor", self.path_editor.to_string()),
            ("clipboard", format!("{:?}", self.clipboard)),
            ("visible", self.visible.to_string()),
        ]
    }

    /// The fields that differ from `other`, with both values.
    pub fn diff(&self, other: &Self) -> Vec<(&'static str, String, String)> {
        self.fields()
            .into_iter()
            .zip(other.fields())
            .filter(|((_, a), (_, b))| a != b)
            .map(|((name, a), (_, b))| (name, a, b))
            .collect()
    }

    /// What outlives a probe and must equal a fresh start before the next one.
    fn lasting(&self) -> Vec<(&'static str, String)> {
        const LASTING: [&str; 21] = [
            "location0",
            "location1",
            "selection0",
            "selection1",
            "view0",
            "view1",
            "sort0",
            "sort1",
            "tabs0",
            "tabs1",
            "bookmarks",
            "swapped",
            "zoomed",
            "hidden",
            "command_text",
            "search0",
            "search1",
            "sizes0",
            "sizes1",
            "clipboard",
            "visible",
        ];
        self.fields()
            .into_iter()
            .filter(|(name, _)| LASTING.contains(name))
            .collect()
    }

    pub fn index(&self, name: &str) -> Option<i32> {
        self.names
            .iter()
            .position(|row| row == name)
            .map(|index| index as i32)
    }
}

/// The files (not folders) among `rows`, sorted.
fn file_names(rows: &[(String, bool)]) -> Vec<String> {
    let mut files: Vec<String> = rows
        .iter()
        .filter(|(_, directory)| !directory)
        .map(|(name, _)| name.clone())
        .collect();
    files.sort();
    files
}

/// Where a probe presses its key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    /// The focused left file list.
    Files,
    /// A filename search typed into the left list.
    Search(&'static str),
    /// The command editor, focused by a click, with this draft typed.
    Command(&'static str),
    /// The command editor after Alt+Up recalled a history entry.
    Recalled,
    /// The left drives popup (opened with Alt+F1).
    Volume,
}

/// Navigation history prepared before the key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum History {
    None,
    /// left → sub → left: Back goes to `sub`.
    Back,
    /// The same, then Back: Forward returns to `left`.
    Forward,
}

#[derive(Clone, Copy, Debug)]
struct Setup {
    place: Place,
    row: &'static str,
    marks: &'static [&'static str],
    seed: Seed,
    history: History,
    /// Copy `right/other.txt` to the clipboard first (Ctrl+V).
    clipboard: bool,
}

impl Setup {
    fn at(row: &'static str) -> Self {
        Self {
            place: Place::Files,
            row,
            marks: &[],
            seed: Seed::Default,
            history: History::None,
            clipboard: false,
        }
    }

    fn place(mut self, place: Place) -> Self {
        self.place = place;
        self
    }

    fn marks(mut self, marks: &'static [&'static str]) -> Self {
        self.marks = marks;
        self
    }

    fn seed(mut self, seed: Seed) -> Self {
        self.seed = seed;
        self
    }

    fn history(mut self, history: History) -> Self {
        self.history = history;
        self
    }
}

type Check = Box<dyn Fn(&App, &State, &State) -> Result<(), String>>;

/// One action's probe: its context, the fields it may change and its own effect.
struct Probe {
    setup: Setup,
    changes: &'static [&'static str],
    check: Check,
    /// The probe expects no change at all; it waits a moment instead of an effect.
    inert: bool,
}

fn probe(
    setup: Setup,
    changes: &'static [&'static str],
    check: impl Fn(&App, &State, &State) -> Result<(), String> + 'static,
) -> Option<Probe> {
    Some(Probe {
        setup,
        changes,
        check: Box::new(check),
        inert: false,
    })
}

fn inert(setup: Setup) -> Option<Probe> {
    Some(Probe {
        setup,
        changes: &[],
        check: Box::new(|_, _, _| Ok(())),
        inert: true,
    })
}

fn same<T: PartialEq + std::fmt::Debug>(what: &str, actual: T, wanted: T) -> Result<(), String> {
    if actual == wanted {
        Ok(())
    } else {
        Err(format!("{what} is {actual:?}, expected {wanted:?}"))
    }
}

fn at(path: &Path) -> Option<PathBuf> {
    Some(path.to_path_buf())
}

/// The left rows' names and whether each is a folder, `..` excluded.
fn rows(app: &App) -> Vec<(String, bool)> {
    app.rows(LEFT)
        .iter()
        .filter(|row| !row.parent_entry)
        .map(|row| (row.name.to_string(), row.directory))
        .collect()
}

fn sorted(names: &[&str]) -> Vec<String> {
    let mut names: Vec<String> = names.iter().map(|name| (*name).to_owned()).collect();
    names.sort();
    names
}

/// The probe for one catalog action and key; None when the sweep has none.
fn probe_for(id: &str, paths: &Paths) -> Option<Probe> {
    let left = paths.left.clone();
    let right = paths.right.clone();
    let sub = paths.sub.clone();
    let root = paths.root.clone();
    let files = Setup::at("alpha.txt");
    if let Some(slot) = id
        .strip_prefix("bookmark.open.")
        .and_then(|slot| slot.parse::<usize>().ok())
    {
        let target = paths.hot[slot - 1].clone();
        return probe(files, &["location0", "cursor0"], move |_, _, after| {
            same("the left folder", after.location[0].clone(), at(&target))
        });
    }
    if let Some(slot) = id
        .strip_prefix("bookmark.other.")
        .and_then(|slot| slot.parse::<usize>().ok())
    {
        let target = paths.hot[slot - 1].clone();
        return probe(files, &["location1", "cursor1"], move |_, _, after| {
            same("the right folder", after.location[1].clone(), at(&target))
        });
    }
    if let Some(number) = id
        .strip_prefix("tab.")
        .and_then(|number| number.parse::<usize>().ok())
    {
        let tabs = tab_paths(paths);
        let wanted = tabs[number - 1].clone();
        return probe(
            Setup::at("").seed(Seed::Tabs9(number % 9)),
            &["tabs0", "location0", "cursor0"],
            move |_, _, after| {
                let active = after.tabs.iter().position(|(_, active)| *active);
                same("the active tab", active, Some(number - 1))?;
                same("the left folder", after.location[0].clone(), at(&wanted))
            },
        );
    }
    if id.starts_with("drive.") && id.len() == "drive.a".len() {
        // Linux has no drive letters: Shift+letter switches nothing. Whether it starts a
        // filename search is one scope probe below, not 26 failures here.
        let home = left.clone();
        return probe(files, &["search0", "cursor0"], move |_, before, after| {
            same("the left folder", after.location[0].clone(), at(&home))?;
            same(
                "the right folder",
                after.location[1].clone(),
                before.location[1].clone(),
            )
        });
    }
    Some(match id {
        "panel.as-other" => probe(
            Setup::at("alpha.txt").place(Place::Volume),
            &["location0", "cursor0", "volume", "popups", "focus"],
            move |_, _, after| {
                same("the left folder", after.location[0].clone(), at(&right))?;
                same("the drives popup", after.volume, -1)
            },
        )?,
        "panel.switch" => probe(files, &["focus", "active"], |_, _, after| {
            same("the focused panel", after.focus, 1)
        })?,
        "panel.open" | "panel.enter-folder" => probe(
            Setup::at("sub"),
            &["location0", "cursor0"],
            move |_, _, after| same("the left folder", after.location[0].clone(), at(&sub)),
        )?,
        "panel.up" => probe(Setup::at("beta.txt"), &["cursor0"], |_, before, after| {
            same("the cursor", after.cursor[0], before.cursor[0] - 1)
        })?,
        "panel.down" => probe(Setup::at("beta.txt"), &["cursor0"], |_, before, after| {
            same("the cursor", after.cursor[0], before.cursor[0] + 1)
        })?,
        "panel.previous-selected" => probe(
            Setup::at("n-010.txt").marks(&["alpha.txt", "n-002.txt"]),
            &["cursor0"],
            |_, before, after| {
                same(
                    "the cursor",
                    Some(after.cursor[0]),
                    before.index("n-002.txt"),
                )
            },
        )?,
        "panel.next-selected" => probe(
            Setup::at("beta.txt").marks(&["alpha.txt", "n-002.txt"]),
            &["cursor0"],
            |_, before, after| {
                same(
                    "the cursor",
                    Some(after.cursor[0]),
                    before.index("n-002.txt"),
                )
            },
        )?,
        "panel.page-up" => probe(Setup::at("n-050.txt"), &["cursor0"], |_, before, after| {
            if after.cursor[0] <= before.cursor[0] - 5 && after.cursor[0] > 0 {
                Ok(())
            } else {
                Err(format!(
                    "the cursor moved from row {} to {}, not one page up",
                    before.cursor[0], after.cursor[0]
                ))
            }
        })?,
        "panel.page-down" => probe(files, &["cursor0"], |_, before, after| {
            let last = before.names.len() as i32 - 1;
            if after.cursor[0] >= before.cursor[0] + 5 && after.cursor[0] < last {
                Ok(())
            } else {
                Err(format!(
                    "the cursor moved from row {} to {} of {last}, not one page down",
                    before.cursor[0], after.cursor[0]
                ))
            }
        })?,
        "panel.first" => probe(Setup::at("beta.txt"), &["cursor0"], |_, _, after| {
            same("the cursor", after.cursor[0], 0)
        })?,
        "panel.last" => probe(Setup::at("beta.txt"), &["cursor0"], |_, before, after| {
            same("the cursor", after.cursor[0], before.names.len() as i32 - 1)
        })?,
        "panel.left" => probe(
            Setup::at("n-040.txt").seed(Seed::Brief),
            &["cursor0"],
            |_, before, after| {
                if after.cursor[0] < before.cursor[0] - 1 {
                    Ok(())
                } else {
                    Err(format!(
                        "in Brief view the cursor moved from row {} to {}, not one column left",
                        before.cursor[0], after.cursor[0]
                    ))
                }
            },
        )?,
        "panel.right" => probe(files.seed(Seed::Brief), &["cursor0"], |_, before, after| {
            if after.cursor[0] > before.cursor[0] + 1 {
                Ok(())
            } else {
                Err(format!(
                    "in Brief view the cursor moved from row {} to {}, not one column right",
                    before.cursor[0], after.cursor[0]
                ))
            }
        })?,
        "panel.parent" => probe(files, &["location0", "cursor0"], move |_, _, after| {
            same("the left folder", after.location[0].clone(), at(&root))
        })?,
        "panel.root" => probe(files, &["location0", "cursor0"], |_, _, after| {
            same(
                "the left folder",
                after.location[0].clone(),
                at(Path::new("/")),
            )
        })?,
        "panel.back" => probe(
            Setup::at("").history(History::Back),
            &["location0", "cursor0"],
            move |_, _, after| same("the left folder", after.location[0].clone(), at(&sub)),
        )?,
        "panel.forward" => probe(
            Setup::at("").history(History::Forward),
            &["location0", "cursor0"],
            move |_, _, after| same("the left folder", after.location[0].clone(), at(&left)),
        )?,
        "panel.refresh" => probe(files, &[], |_, before, after| {
            if after.listings[0] > before.listings[0] {
                Ok(())
            } else {
                Err("the left folder was not listed again".into())
            }
        })?,
        "panel.path" => probe(files, &["path_editor", "focus"], |_, _, after| {
            same("the path editor", after.path_editor, true)
        })?,
        "panel.swap" => probe(files, &["swapped", "focus", "active"], |_, _, after| {
            same("the panels swapped", after.swapped, true)
        })?,
        "panel.compare" => probe(files, &["comparison", "popups", "focus"], |_, _, after| {
            same("Compare directories", after.comparison, true)
        })?,
        "panel.zoom" => probe(files, &["zoomed"], |_, _, after| {
            same("the enlarged panel", after.zoomed, true)
        })?,
        "panel.filter" => probe(files, &["filter", "popups", "focus"], |_, _, after| {
            same("the panel filter", after.filter, true)
        })?,
        "panel.drive-info" => probe(files, &["notice", "popups", "focus"], |_, _, after| {
            same(
                "the notice",
                after.notice.as_deref(),
                Some("Drive information"),
            )
        })?,
        "panel.folder-sizes" => probe(files, &["sizes0"], |_, _, after| {
            same("the measured folders", after.sizes[0], 2)
        })?,
        "panel.hidden-files" => probe(files, &["hidden", "cursor0", "cursor1"], |_, _, after| {
            same("Show hidden files", after.hidden, false)?;
            same(
                "the hidden file listed",
                after.names.iter().any(|name| name == ".hidden.txt"),
                false,
            )
        })?,
        "files.find" => probe(files, &["tools", "focus"], |_, _, after| {
            same(
                "the tool windows",
                after.tools.clone(),
                vec!["Find files · Midnight Explorer".to_owned()],
            )
        })?,
        "files.checksums" => probe(files, &["tools", "focus"], |_, _, after| {
            same(
                "the tool windows",
                after.tools.clone(),
                vec!["Checksums · Midnight Explorer".to_owned()],
            )
        })?,
        "files.edit" => probe(files, &["notice", "popups", "focus"], |_, _, after| {
            // No editor service in this run: the attempt must be reported.
            match &after.notice {
                Some(_) => Ok(()),
                None => Err("F4 reported nothing without an editor".into()),
            }
        })?,
        "files.make-list" => probe(files, &["file_list", "popups", "focus"], |_, _, after| {
            same("Make File List", after.file_list, true)
        })?,
        "selection.toggle-size" => probe(
            Setup::at("sub"),
            &["selection0", "cursor0", "sizes0"],
            |_, before, after| {
                same("the marks", after.selection[0].clone(), sorted(&["sub"]))?;
                same("the cursor", after.cursor[0], before.cursor[0] + 1)?;
                same("the measured folders", after.sizes[0], 1)
            },
        )?,
        "selection.toggle" => probe(files, &["selection0", "cursor0"], |_, before, after| {
            same(
                "the marks",
                after.selection[0].clone(),
                sorted(&["alpha.txt"]),
            )?;
            same("the cursor", after.cursor[0], before.cursor[0] + 1)
        })?,
        "selection.all" => probe(files, &["selection0"], |app, _, after| {
            let mut all: Vec<String> = rows(app).into_iter().map(|(name, _)| name).collect();
            all.sort();
            same("the marks", after.selection[0].clone(), all)
        })?,
        "selection.files" => probe(files, &["selection0"], |app, _, after| {
            same(
                "the marks",
                after.selection[0].clone(),
                file_names(&rows(app)),
            )
        })?,
        "selection.clear" => probe(
            files.marks(&["alpha.txt", "sub"]),
            &["selection0"],
            |_, _, after| same("the marks", after.selection[0].clone(), Vec::new()),
        )?,
        "selection.invert" | "selection.mask-invert" => probe(
            files.marks(&["alpha.txt", "sub"]),
            &["selection0"],
            |app, _, after| {
                let mut wanted: Vec<String> = file_names(&rows(app))
                    .into_iter()
                    .filter(|name| name != "alpha.txt")
                    .collect();
                wanted.push("sub".into());
                wanted.sort();
                same("the marks", after.selection[0].clone(), wanted)
            },
        )?,
        "selection.invert-all" => probe(
            files.marks(&["alpha.txt", "sub"]),
            &["selection0"],
            |app, _, after| {
                let mut wanted: Vec<String> = rows(app)
                    .into_iter()
                    .map(|(name, _)| name)
                    .filter(|name| name != "alpha.txt" && name != "sub")
                    .collect();
                wanted.sort();
                same("the marks", after.selection[0].clone(), wanted)
            },
        )?,
        "selection.mask-select" => probe(files, &["mask", "popups", "focus"], |_, _, after| {
            same("the Select files dialog", after.mask, Some(true))
        })?,
        "selection.mask-deselect" => probe(files, &["mask", "popups", "focus"], |_, _, after| {
            same("the Deselect files dialog", after.mask, Some(false))
        })?,
        "sort.name" | "sort.extension" | "sort.modified" | "sort.size" => {
            let wanted = match id {
                "sort.name" => (0, true),
                "sort.size" => (1, false),
                "sort.modified" => (2, false),
                _ => (3, false),
            };
            probe(files, &["sort0", "cursor0"], move |_, _, after| {
                same("the left sort (column, descending)", after.sort[0], wanted)
            })?
        }
        "search.clear" => probe(
            files.place(Place::Search("n-0")),
            &["search0", "cursor0"],
            |_, _, after| same("the search", after.search[0].0, false),
        )?,
        "search.erase" => probe(
            files.place(Place::Search("n-0")),
            &["search0", "cursor0"],
            |_, _, after| {
                same(
                    "the search",
                    after.search[0].clone(),
                    (true, "n-".to_owned()),
                )
            },
        )?,
        "files.rename"
        | "files.create-file"
        | "files.create-directory"
        | "files.trash"
        | "files.delete" => {
            let wanted = match id {
                "files.rename" => ("Rename", false),
                "files.create-file" => ("New empty file", false),
                "files.create-directory" => ("Create directory", false),
                "files.trash" => ("Move to Trash", false),
                _ => ("Permanently delete", true),
            };
            probe(
                files,
                &["operation", "popups", "focus"],
                move |_, _, after| match &after.operation {
                    Some((title, dangerous))
                        if title.contains(wanted.0) && *dangerous == wanted.1 =>
                    {
                        Ok(())
                    }
                    other => Err(format!(
                        "the operation dialog is {other:?}, expected {:?} (dangerous {})",
                        wanted.0, wanted.1
                    )),
                },
            )?
        }
        "files.attributes" | "files.case" => {
            let case = id == "files.case";
            probe(
                files,
                &["metadata", "popups", "focus"],
                move |_, _, after| {
                    same(
                        "the attributes editor (case page)",
                        after.metadata,
                        Some(case),
                    )
                },
            )?
        }
        "files.properties" => {
            probe(
                files,
                &["notice", "popups", "focus"],
                |_, _, after| match &after.notice {
                    Some(title) if !title.is_empty() => Ok(()),
                    other => Err(format!("Properties showed the notice {other:?}")),
                },
            )?
        }
        "files.copy-to" | "files.move-to" => {
            let moving = id == "files.move-to";
            probe(
                files,
                &["copy_draft", "popups", "focus"],
                move |_, _, after| {
                    same(
                        "the Copy/Move dialog (move)",
                        after.copy_draft,
                        Some(moving),
                    )
                },
            )?
        }
        "files.copy" | "files.cut" => {
            let intent = if id == "files.cut" {
                FileClipboardIntent::Move
            } else {
                FileClipboardIntent::Copy
            };
            let file = left.join("alpha.txt");
            probe(files, &["clipboard"], move |_, _, after| {
                same(
                    "the clipboard",
                    after.clipboard.clone(),
                    Some((intent, vec![file.clone()])),
                )
            })?
        }
        "files.paste" => {
            let pasted = left.join("other.txt");
            Probe {
                setup: Setup {
                    clipboard: true,
                    ..files
                },
                changes: &["cursor0"],
                check: Box::new(move |_, before, after| {
                    if after.copies > before.copies && pasted.is_file() {
                        Ok(())
                    } else {
                        Err(format!(
                            "no copy of the clipboard's file arrived ({} copy reports)",
                            after.copies - before.copies
                        ))
                    }
                }),
                inert: false,
            }
        }
        "tab.new" => probe(files, &["tabs0", "cursor0"], |_, before, after| {
            same("the left tabs", after.tabs.len(), before.tabs.len() + 1)
        })?,
        "tab.close" => probe(
            Setup::at("").seed(Seed::Tabs2),
            &["tabs0", "location0", "cursor0"],
            move |_, _, after| {
                same("the left tabs", after.tabs.len(), 1)?;
                same("the left folder", after.location[0].clone(), at(&left))
            },
        )?,
        "tab.previous" => probe(
            Setup::at("").seed(Seed::Tabs2),
            &["tabs0", "location0", "cursor0"],
            move |_, _, after| {
                same(
                    "the active tab",
                    after.tabs.iter().position(|(_, active)| *active),
                    Some(0),
                )?;
                same("the left folder", after.location[0].clone(), at(&left))
            },
        )?,
        "bookmark.add" => {
            let folder = left.display().to_string();
            probe(files, &["bookmarks"], move |_, before, after| {
                same(
                    "the Hot Paths",
                    after.bookmarks.len(),
                    before.bookmarks.len() + 1,
                )?;
                same(
                    "the new Hot Path",
                    after.bookmarks.iter().any(|(path, _)| *path == folder),
                    true,
                )
            })?
        }
        "bookmark.assign.10" => {
            let folder = left.display().to_string();
            probe(files, &["bookmarks"], move |_, _, after| {
                let slot10: Vec<&String> = after
                    .bookmarks
                    .iter()
                    .filter(|(_, slot)| *slot == 10)
                    .map(|(path, _)| path)
                    .collect();
                same("Hot Path 10", slot10, vec![&folder])
            })?
        }
        "command.focus" => probe(files, &["command_focus", "focus"], |_, _, after| {
            same("the command editor focus", after.command_focus, true)
        })?,
        "command.insert-name"
        | "command.insert-full-path"
        | "command.insert-path"
        | "command.insert-left-path"
        | "command.insert-right-path" => {
            let separator = std::path::MAIN_SEPARATOR;
            let wanted = match id {
                "command.insert-name" => "alpha.txt".to_owned(),
                "command.insert-full-path" => left.join("alpha.txt").display().to_string(),
                "command.insert-right-path" => format!("{}{separator}", right.display()),
                _ => format!("{}{separator}", left.display()),
            };
            probe(
                files,
                &["command_text", "command_focus", "focus"],
                move |_, _, after| {
                    same(
                        "the command draft",
                        after.command_text.clone(),
                        wanted.clone(),
                    )
                },
            )?
        }
        "command.history-older" => probe(
            files.place(Place::Command("")),
            &["command_text"],
            |_, _, after| {
                if HISTORY.contains(&after.command_text.as_str()) {
                    Ok(())
                } else {
                    Err(format!(
                        "Alt+Up recalled {:?}, not a stored command",
                        after.command_text
                    ))
                }
            },
        )?,
        "command.history-newer" => probe(
            files.place(Place::Recalled),
            &["command_text"],
            |_, before, after| {
                if after.command_text != before.command_text
                    && (after.command_text.is_empty()
                        || HISTORY.contains(&after.command_text.as_str()))
                {
                    Ok(())
                } else {
                    Err(format!(
                        "Alt+Down changed the draft from {:?} to {:?}",
                        before.command_text, after.command_text
                    ))
                }
            },
        )?,
        // This run has no desktop services (they would start a real terminal): the
        // submitted command must be reported as such, its draft kept.
        "command.execute" => probe(
            files.place(Place::Command("echo midnight-acceptance")),
            &["notice", "popups", "focus", "command_focus"],
            |_, _, after| {
                same(
                    "the notice",
                    after.notice.as_deref(),
                    Some("Terminal unavailable"),
                )?;
                same(
                    "the kept draft",
                    after.command_text.as_str(),
                    "echo midnight-acceptance",
                )
            },
        )?,
        "command.focus-files" => probe(
            files.place(Place::Command("")),
            &["command_focus", "focus"],
            |_, _, after| {
                same("the command editor focus", after.command_focus, false)?;
                same("the focused panel", after.focus, 0)
            },
        )?,
        "drive.left" | "drive.right" => {
            let pane = i32::from(id == "drive.right");
            probe(
                files,
                &["volume", "popups", "focus", "active"],
                move |_, _, after| same("the drives popup's panel", after.volume, pane),
            )?
        }
        "app.help" => probe(files, &["tools", "focus"], |_, _, after| {
            same(
                "the tool windows",
                after.tools.clone(),
                vec!["Keyboard help · Midnight Explorer".to_owned()],
            )
        })?,
        "app.user-menu" => probe(files, &["menu", "popups", "focus"], |_, _, after| {
            same("the open menu", after.menu, "user")
        })?,
        "app.main-menu" => probe(files, &["menu", "popups", "focus"], |_, _, after| {
            same("the open menu", after.menu, "main")
        })?,
        "view.brief" => probe(files, &["view0", "cursor0"], |_, _, after| {
            same("the left view mode", after.view[0], 1)
        })?,
        "view.detailed" => probe(
            files.seed(Seed::Brief),
            &["view0", "cursor0"],
            |_, _, after| same("the left view mode", after.view[0], 0),
        )?,
        "view.thumbnails" => probe(files, &["view0", "cursor0"], |_, _, after| {
            same("the left view mode", after.view[0], 2)
        })?,
        "app.exit" => probe(files, &["visible", "focus"], |_, _, after| {
            same("the main window shown", after.visible, false)
        })?,
        // Linux offers no network drive dialogs: F11 and F12 change nothing.
        "network.connect" | "network.disconnect" => inert(files)?,
        _ => return None,
    })
}

/// Keys whose actions belong to windows or services this run does not have.
fn excluded(definition: &ShortcutDefinition) -> Option<&'static str> {
    if matches!(
        definition.scope,
        ShortcutScope::ViewerAny
            | ShortcutScope::ViewerContent
            | ShortcutScope::ViewerOnlyContent
            | ShortcutScope::ComparisonContent
    ) {
        return Some("viewer and comparison windows (viewers_* scenarios, staged processes)");
    }
    match definition.id.as_str() {
        "files.view" | "files.view-basic" => Some(
            "the viewer service exists only in staged processes (viewers_files, viewers_controls)",
        ),
        "files.compare" => {
            Some("the comparator service exists only in staged processes (viewers_compare)")
        }
        _ => None,
    }
}

/// Keys the native hook resolves by their physical position.
fn physical(chord: &ShortcutChord) -> bool {
    chord.key.starts_with("Digit")
        || chord.key.starts_with("Numpad")
        || (matches!(chord.key.as_str(), "BracketLeft" | "BracketRight")
            && (chord.control || chord.alt || chord.meta))
}

/// What the platform types for a key the hook does not take.
fn typed(chord: &ShortcutChord) -> Option<SharedString> {
    let key = chord.key.as_str();
    let functions = [
        Key::F1,
        Key::F2,
        Key::F3,
        Key::F4,
        Key::F5,
        Key::F6,
        Key::F7,
        Key::F8,
        Key::F9,
        Key::F10,
        Key::F11,
        Key::F12,
    ];
    Some(match key {
        "Enter" => Key::Return.into(),
        "Escape" => Key::Escape.into(),
        "Tab" => Key::Tab.into(),
        "Space" => Key::Space.into(),
        "Backspace" => Key::Backspace.into(),
        "Delete" => Key::Delete.into(),
        "Insert" => Key::Insert.into(),
        "Home" => Key::Home.into(),
        "End" => Key::End.into(),
        "PageUp" => Key::PageUp.into(),
        "PageDown" => Key::PageDown.into(),
        "Up" => Key::UpArrow.into(),
        "Down" => Key::DownArrow.into(),
        "Left" => Key::LeftArrow.into(),
        "Right" => Key::RightArrow.into(),
        "Plus" | "NumpadAdd" => "+".into(),
        "Minus" | "NumpadSubtract" => "-".into(),
        "NumpadMultiply" => "*".into(),
        "Equal" => "=".into(),
        "Backslash" => "\\".into(),
        "Slash" => "/".into(),
        "Period" => ".".into(),
        "Comma" => ",".into(),
        "Semicolon" => ";".into(),
        "Quote" => "'".into(),
        "Backquote" => "`".into(),
        "BracketLeft" => "[".into(),
        "BracketRight" => "]".into(),
        _ => {
            if let Some(digit) = key.strip_prefix("Digit") {
                digit.into()
            } else if let Some(index) = key
                .strip_prefix('F')
                .and_then(|number| number.parse::<usize>().ok())
                .filter(|number| (1..=12).contains(number))
            {
                functions[index - 1].into()
            } else if key.chars().count() == 1 {
                if chord.shift {
                    key.to_uppercase().into()
                } else {
                    key.to_lowercase().into()
                }
            } else {
                return None;
            }
        }
    })
}

fn modifiers(chord: &ShortcutChord) -> Vec<Key> {
    let mut keys = Vec::new();
    if chord.control {
        keys.push(Key::Control);
    }
    if chord.alt {
        keys.push(Key::Alt);
    }
    if chord.shift {
        keys.push(Key::Shift);
    }
    if chord.meta {
        keys.push(Key::Meta);
    }
    keys
}

/// Press a chord as the platform delivers it: physical keys through the window's
/// physical-shortcut entry first (as the native hook does), the rest, and a physical
/// key the hook did not take, as Slint key events.
pub fn press(app: &mut App, chord: &ShortcutChord) -> Result<(), String> {
    let text = typed(chord).ok_or_else(|| format!("no key text for {}", chord.display()))?;
    if physical(chord) {
        let window = &app.window;
        window.set_native_key_text(text.clone());
        let consumed = window.invoke_physical_shortcut(
            chord.key.as_str().into(),
            chord.control,
            chord.alt,
            chord.shift,
            chord.meta,
        );
        if !consumed {
            // The hook marks the number row's logical key as already resolved.
            window.set_native_digit_event(chord.key.starts_with("Digit"));
            input::chord(app.slint(), &modifiers(chord), text);
            app.window.set_native_digit_event(false);
        }
    } else {
        input::chord(app.slint(), &modifiers(chord), text);
    }
    Ok(())
}

/// Click a row with modifiers, waiting out the double-click interval on the same spot.
fn click_row(app: &mut App, name: &str, keys: &[Key]) -> Result<(), String> {
    let position = panel::row_center(app, LEFT, name)
        .ok_or_else(|| format!("no {name:?} row is shown in the left panel"))?;
    if let Some((when, last)) = app.last_click
        && when.elapsed() < Duration::from_millis(650)
        && (last.x - position.x).abs() < 8.0
        && (last.y - position.y).abs() < 8.0
    {
        app.idle(Duration::from_millis(650).saturating_sub(when.elapsed()));
    }
    input::click_with(app.slint(), keys, position);
    app.last_click = Some((Instant::now(), position));
    app.settle();
    Ok(())
}

/// Double-click a row to open it and wait for the panel to show `target`.
fn open_row(app: &mut App, name: &str, target: &Path) -> Result<(), String> {
    let position = panel::row_center(app, LEFT, name)
        .ok_or_else(|| format!("no {name:?} row is shown in the left panel"))?;
    if app
        .last_click
        .is_some_and(|(when, _)| when.elapsed() < Duration::from_millis(650))
    {
        app.idle(Duration::from_millis(650));
    }
    input::double_click_at(app.slint(), position);
    app.last_click = Some((Instant::now(), position));
    wait(
        app,
        &format!("the left panel to show {}", target.display()),
        |app| app.location(LEFT).as_deref() == Some(target) && !app.loading(LEFT),
    )
}

/// Poll `condition` for up to 10 s; an error names `what`.
fn wait(app: &mut App, what: &str, condition: impl Fn(&App) -> bool) -> Result<(), String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.settle();
        if condition(app) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!("{what} did not happen"));
        }
        app.idle(Duration::from_millis(10));
    }
}

fn click_command(app: &mut App) -> Result<(), String> {
    let window = &app.window;
    let position = slint::LogicalPosition::new(
        window.get_command_input_left() + window.get_command_input_width() / 2.0,
        window.get_command_input_top() + window.get_command_input_height() / 2.0,
    );
    input::click_at(app.slint(), position);
    wait(app, "the command editor to take focus", |app| {
        app.window.get_command_focused()
    })
}

fn prepare(app: &mut App, setup: &Setup, paths: &Paths) -> Result<(), String> {
    panel::wait_input(app);
    panel::focus_list(app, LEFT)?;
    if setup.clipboard {
        // Ctrl+C on the right panel's file puts it on the clipboard.
        panel::select_row(app, PaneId::Right, "other.txt")?;
        input::chord(app.slint(), &[Key::Control], "c");
        wait(app, "the clipboard to hold other.txt", |app| {
            State::of(app).clipboard.is_some()
        })?;
        panel::focus_list(app, LEFT)?;
    }
    if setup.history != History::None {
        open_row(app, "sub", &paths.sub)?;
        let parent = app
            .rows(LEFT)
            .iter()
            .find(|row| row.parent_entry)
            .map(|row| row.name.to_string())
            .ok_or("the sub folder lists no parent row")?;
        open_row(app, &parent, &paths.left)?;
        if setup.history == History::Forward {
            input::chord(app.slint(), &[Key::Alt], Key::LeftArrow);
            wait(app, "Back to sub", |app| {
                app.location(LEFT).as_deref() == Some(paths.sub.as_path()) && !app.loading(LEFT)
            })?;
        }
        panel::focus_list(app, LEFT)?;
    }
    for mark in setup.marks {
        click_row(app, mark, &[Key::Control])?;
    }
    if !setup.marks.is_empty() {
        let wanted = sorted(setup.marks);
        wait(app, &format!("the marks {wanted:?}"), |app| {
            selected(app, LEFT) == wanted
        })?;
    }
    if !setup.row.is_empty() {
        panel::select_row(app, LEFT, setup.row)?;
    }
    match setup.place {
        Place::Files => {}
        Place::Search(prefix) => {
            input::type_text(app.slint(), prefix);
            wait(app, &format!("a filename search for {prefix:?}"), |app| {
                app.window.get_left_search_active()
                    && app.window.get_left_search_query().as_str() == prefix
            })?;
        }
        Place::Command(text) => {
            click_command(app)?;
            input::type_text(app.slint(), text);
            wait(app, &format!("the command draft {text:?}"), |app| {
                app.window.get_command_text().as_str() == text
            })?;
        }
        Place::Recalled => {
            click_command(app)?;
            input::chord(app.slint(), &[Key::Alt], Key::UpArrow);
            wait(app, "Alt+Up to recall a command", |app| {
                !app.window.get_command_text().is_empty()
            })?;
        }
        Place::Volume => {
            input::chord(app.slint(), &[Key::Alt], Key::F1);
            wait(app, "the left drives popup", |app| {
                app.window.get_volume_popup_pane() == 0
            })?;
        }
    }
    app.idle(Duration::from_millis(30));
    Ok(())
}

/// Nothing open over the panels.
fn quiet(app: &App) -> bool {
    let state = State::of(app);
    state.popups == 0
        && state.notice.is_none()
        && state.tools.is_empty()
        && state.operation.is_none()
        && state.copy_draft.is_none()
        && state.metadata.is_none()
        && state.mask.is_none()
        && !state.options
        && !state.about
        && !state.filter
        && !state.comparison
        && !state.file_list
        && !state.hot_paths
        && !state.path_editor
        && !state.command_focus
        && state.volume < 0
        && !state.search[0].0
}

/// Close what a probe opened: tool windows, then dialogs and popups with Escape.
fn close_all(app: &mut App) {
    for _ in 0..4 {
        if quiet(app) {
            return;
        }
        for window in app.tool_windows() {
            app::layout_tool(&window, 900, 640);
            input::key(window.window(), Key::Escape);
        }
        app.idle(Duration::from_millis(60));
        if !app.tool_windows().is_empty() {
            for window in app.tool_windows() {
                let _ = tools::click_last(window.window(), "Close");
            }
            app.idle(Duration::from_millis(60));
        }
        input::key(app.slint(), Key::Escape);
        app.idle(Duration::from_millis(80));
    }
}

/// The result of one key.
struct Outcome {
    label: String,
    error: Option<String>,
}

/// Prepare `probe`, perform `act` (a key or a click) and check what happened.
fn run_probe(
    app: &mut App,
    label: String,
    probe: &Probe,
    paths: &Paths,
    act: &mut dyn FnMut(&mut App) -> Result<(), String>,
) -> Outcome {
    let failed = |error: String| Outcome {
        label: label.clone(),
        error: Some(error),
    };
    if let Err(error) = prepare(app, &probe.setup, paths) {
        return failed(format!("setup: {error}"));
    }
    let before = State::of(app);
    if let Err(error) = act(app) {
        return failed(error);
    }
    let deadline = Instant::now() + Duration::from_secs(if probe.inert { 0 } else { 8 });
    let last = loop {
        app.idle(Duration::from_millis(20));
        let now = State::of(app);
        let result = (probe.check)(app, &before, &now);
        if result.is_ok() || Instant::now() >= deadline {
            break result;
        }
    };
    // Late side effects show up too.
    app.idle(Duration::from_millis(200));
    let after = State::of(app);
    if let Err(error) = last.and_then(|()| (probe.check)(app, &before, &after)) {
        return failed(error);
    }
    let unexpected: Vec<String> = before
        .diff(&after)
        .into_iter()
        .filter(|(name, _, _)| !probe.changes.contains(name))
        .map(|(name, was, now)| format!("{name}: {was} → {now}"))
        .collect();
    if !unexpected.is_empty() {
        return failed(format!("it also changed {}", unexpected.join("; ")));
    }
    Outcome { label, error: None }
}

/// Extra probes: keys pressed where their action does not apply.
struct ScopeProbe {
    label: &'static str,
    key: &'static str,
    probe: Probe,
}

fn scope_probes() -> Vec<ScopeProbe> {
    vec![
        ScopeProbe {
            label: "Space during filename search is typed",
            key: "Space",
            probe: Probe {
                setup: Setup::at("alpha.txt").place(Place::Search("n-0")),
                changes: &["search0", "cursor0"],
                check: Box::new(|_, _, after| {
                    same(
                        "the search",
                        after.search[0].clone(),
                        (true, "n-0 ".to_owned()),
                    )?;
                    same("the marks", after.selection[0].clone(), Vec::new())
                }),
                inert: false,
            },
        },
        ScopeProbe {
            label: "a number-row digit during filename search is typed, no Hot Path opens",
            key: "Digit1",
            probe: Probe {
                setup: Setup::at("alpha.txt").place(Place::Search("n-0")),
                changes: &["search0", "cursor0"],
                check: Box::new(|_, _, after| {
                    same(
                        "the search",
                        after.search[0].clone(),
                        (true, "n-01".to_owned()),
                    )
                }),
                inert: false,
            },
        },
        ScopeProbe {
            label: "Insert during filename search ends it and marks the row",
            key: "Insert",
            probe: Probe {
                setup: Setup::at("alpha.txt").place(Place::Search("n-01")),
                changes: &["search0", "selection0", "cursor0"],
                check: Box::new(|_, before, after| {
                    same("the search", after.search[0].0, false)?;
                    let focused = usize::try_from(before.cursor[0])
                        .ok()
                        .and_then(|row| before.names.get(row))
                        .cloned()
                        .unwrap_or_default();
                    same("the marks", after.selection[0].clone(), vec![focused])
                }),
                inert: false,
            },
        },
        ScopeProbe {
            label: "the number row's + is typed into a filename search, not Select by mask",
            key: "Plus",
            probe: Probe {
                setup: Setup::at("alpha.txt"),
                changes: &["search0", "cursor0"],
                check: Box::new(|_, _, after| {
                    same(
                        "the search",
                        after.search[0].clone(),
                        (true, "+".to_owned()),
                    )
                }),
                inert: false,
            },
        },
        ScopeProbe {
            label: "the number row's - is typed into a filename search, not Deselect by mask",
            key: "Minus",
            probe: Probe {
                setup: Setup::at("alpha.txt"),
                changes: &["search0", "cursor0"],
                check: Box::new(|_, _, after| {
                    same(
                        "the search",
                        after.search[0].clone(),
                        (true, "-".to_owned()),
                    )
                }),
                inert: false,
            },
        },
        ScopeProbe {
            label: "the number row's * is typed into a filename search, not the invert",
            key: "*",
            probe: Probe {
                setup: Setup::at("alpha.txt"),
                changes: &["search0", "cursor0"],
                check: Box::new(|_, _, after| {
                    same(
                        "the search",
                        after.search[0].clone(),
                        (true, "*".to_owned()),
                    )
                }),
                inert: false,
            },
        },
        ScopeProbe {
            label: "finding: an uppercase letter does not start a filename search (Linux has no drive letters; Shift+letter is swallowed)",
            key: "Shift+R",
            probe: Probe {
                setup: Setup::at("alpha.txt"),
                changes: &["search0", "cursor0"],
                check: Box::new(|_, _, after| {
                    same(
                        "the search",
                        after.search[0].clone(),
                        (true, "R".to_owned()),
                    )
                }),
                inert: false,
            },
        },
        ScopeProbe {
            label: "F7 in the command editor opens no file dialog",
            key: "F7",
            probe: Probe {
                setup: Setup::at("alpha.txt").place(Place::Command("abc")),
                changes: &[],
                check: Box::new(|_, _, _| Ok(())),
                inert: true,
            },
        },
        ScopeProbe {
            label: "Ctrl+A in the command editor selects text, not files",
            key: "Ctrl+A",
            probe: Probe {
                setup: Setup::at("alpha.txt").place(Place::Command("abc")),
                changes: &[],
                check: Box::new(|_, _, _| Ok(())),
                inert: true,
            },
        },
        ScopeProbe {
            label: "a plain digit in the command editor is typed",
            key: "Digit2",
            probe: Probe {
                setup: Setup::at("alpha.txt").place(Place::Command("abc")),
                changes: &["command_text"],
                check: Box::new(|_, _, after| {
                    same("the command draft", after.command_text.as_str(), "abc2")
                }),
                inert: false,
            },
        },
    ]
}

/// Restart when the next probe needs another seed or the last one left a change.
fn ready(
    app: &mut Option<App>,
    seed: &mut Seed,
    wanted: Seed,
    fresh: &mut Vec<(Seed, Vec<(&'static str, String)>)>,
    paths: &Paths,
    restarts: &mut usize,
) -> Result<(), String> {
    if let Some(current) = app.as_mut() {
        close_all(current);
        let lasting = State::of(current).lasting();
        let clean = fresh
            .iter()
            .find(|(known, _)| *known == *seed)
            .is_some_and(|(_, state)| *state == lasting);
        if *seed == wanted && clean {
            return Ok(());
        }
    }
    if let Some(mut old) = app.take() {
        old.shutdown()?;
        drop(old);
        *restarts += 1;
    }
    // A probe may have pasted a file: the folder is what every probe starts from.
    let _ = fs::remove_file(paths.left.join("other.txt"));
    let mut started = start(paths, wanted);
    panel::focus_list(&mut started, LEFT)?;
    let lasting = State::of(&started).lasting();
    if !fresh.iter().any(|(known, _)| *known == wanted) {
        fresh.push((wanted, lasting));
    }
    *seed = wanted;
    *app = Some(started);
    Ok(())
}

pub fn catalog(context: &mut Context) -> Result<(), String> {
    let paths = fixture(&context.work)?;
    let mut app: Option<App> = None;
    let mut seed = Seed::Default;
    let mut fresh = Vec::new();
    let mut restarts = 0;
    let mut failures: Vec<String> = Vec::new();
    let mut passed = 0;
    let mut skipped: Vec<String> = Vec::new();
    let mut unbound: Vec<String> = Vec::new();
    let mut exit: Option<(&ShortcutDefinition, String)> = None;
    let started = Instant::now();
    for definition in shortcut_catalog() {
        if let Some(reason) = excluded(definition) {
            skipped.push(format!("{} ({reason})", definition.id));
            continue;
        }
        if definition.defaults.is_empty() {
            unbound.push(definition.id.clone());
            continue;
        }
        for key in &definition.defaults {
            if definition.id == "app.exit" {
                exit = Some((definition, key.clone()));
                continue;
            }
            let Some(probe) = probe_for(&definition.id, &paths) else {
                failures.push(format!(
                    "{} {key}: no probe covers this catalog action",
                    definition.id
                ));
                continue;
            };
            ready(
                &mut app,
                &mut seed,
                probe.setup.seed,
                &mut fresh,
                &paths,
                &mut restarts,
            )?;
            let chord = match ShortcutChord::parse(key) {
                Ok(chord) => chord,
                Err(error) => {
                    failures.push(format!(
                        "{} {key}: the key does not parse: {error}",
                        definition.id
                    ));
                    continue;
                }
            };
            let current = app.as_mut().expect("a started application");
            let outcome = run_probe(
                current,
                format!("{} {key}", definition.id),
                &probe,
                &paths,
                &mut |app| press(app, &chord),
            );
            match outcome.error {
                Some(error) => failures.push(format!("{}: {error}", outcome.label)),
                None => passed += 1,
            }
        }
    }
    // Keys pressed where their action does not apply.
    for scope in scope_probes() {
        ready(
            &mut app,
            &mut seed,
            scope.probe.setup.seed,
            &mut fresh,
            &paths,
            &mut restarts,
        )?;
        let current = app.as_mut().expect("a started application");
        let chord = ShortcutChord::parse(scope.key).map_err(|error| error.to_string())?;
        let outcome = run_probe(
            current,
            scope.key.to_owned(),
            &scope.probe,
            &paths,
            &mut |app| press(app, &chord),
        );
        match outcome.error {
            Some(error) => failures.push(format!("{} ({}): {error}", scope.label, scope.key)),
            None => passed += 1,
        }
    }
    if let Some(current) = app.as_mut() {
        current.screenshot(context, "after-sweep", "The panels after the last probe");
    }
    // Exit last: it hides the main window.
    if let Some((definition, key)) = exit {
        let probe = probe_for(&definition.id, &paths).expect("the exit probe");
        ready(
            &mut app,
            &mut seed,
            Seed::Default,
            &mut fresh,
            &paths,
            &mut restarts,
        )?;
        let current = app.as_mut().expect("a started application");
        let chord = ShortcutChord::parse(&key).map_err(|error| error.to_string())?;
        let outcome = run_probe(
            current,
            format!("{} {key}", definition.id),
            &probe,
            &paths,
            &mut |app| press(app, &chord),
        );
        match outcome.error {
            Some(error) => failures.push(format!("{}: {error}", outcome.label)),
            None => passed += 1,
        }
    }
    if let Some(mut current) = app.take() {
        current.shutdown()?;
    }
    context.note(format!(
        "{passed} keys did exactly what their action says in {:.0} s ({restarts} restarts)",
        started.elapsed().as_secs_f64()
    ));
    context.note(format!(
        "actions without a default key (reached from F10 or Options): {unbound:?}"
    ));
    context.note(format!("not pressed here: {skipped:?}"));
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} of {} keys failed:\n{}",
            failures.len(),
            failures.len() + passed,
            failures.join("\n")
        ))
    }
}

/// Hold `held`, click the footer cell of `name` (which must be `enabled`), release.
fn click_cell(app: &mut App, held: &[Key], name: &str, enabled: bool) -> Result<(), String> {
    for key in held {
        input::press(app.slint(), *key);
    }
    app.settle();
    let prefix = format!("{name} ");
    let cell = inspect::controls(app.slint(), AccessibleRole::Button)
        .into_iter()
        .find(|(label, _)| label == name || label.starts_with(&prefix));
    let result = match cell {
        None => Err(format!("no footer cell {name} while {held:?} is held")),
        Some((label, item)) if tools::enabled(&item) != enabled => Err(format!(
            "the footer cell {label:?} is {}",
            if enabled { "disabled" } else { "enabled" }
        )),
        Some((_, item)) => {
            input::click_at(app.slint(), inspect::center(&item));
            Ok(())
        }
    };
    for key in held.iter().rev() {
        input::release(app.slint(), *key);
    }
    app.settle();
    result
}

/// The F1–F12 footer in the plain, Ctrl, Shift, Alt and Ctrl+Shift layers (held as
/// modifiers): every cell names its key and action and a click runs the same probe as
/// the key; a key without an action is a disabled cell that does nothing; the network
/// cells Linux cannot serve are hidden.
pub fn footer(context: &mut Context) -> Result<(), String> {
    let paths = fixture(&context.work)?;
    let map = ShortcutMap::new(&[]).map_err(|error| format!("the default shortcuts: {error}"))?;
    let layers: [&[Key]; 5] = [
        &[],
        &[Key::Control],
        &[Key::Shift],
        &[Key::Alt],
        &[Key::Control, Key::Shift],
    ];
    let mut app: Option<App> = None;
    let mut seed = Seed::Default;
    let mut fresh = Vec::new();
    let mut restarts = 0;
    let mut failures: Vec<String> = Vec::new();
    let mut passed = 0;
    let mut skipped: Vec<String> = Vec::new();
    let mut exit: Option<(&[Key], String)> = None;
    let mut focus_lost: Vec<String> = Vec::new();
    for held in layers {
        for index in 0..12 {
            let chord = ShortcutChord {
                key: format!("F{}", index + 1),
                control: held.contains(&Key::Control),
                alt: held.contains(&Key::Alt),
                shift: held.contains(&Key::Shift),
                meta: false,
            };
            let name = chord.display();
            let action = map.resolve(&chord, ShortcutContext::Files);
            let (probe, enabled) = match action {
                None => (
                    inert(Setup::at("alpha.txt")).expect("an inert probe"),
                    false,
                ),
                Some(action) if action.id.starts_with("network.") => {
                    // Linux has no network drive dialogs: the cell is hidden.
                    ready(
                        &mut app,
                        &mut seed,
                        Seed::Default,
                        &mut fresh,
                        &paths,
                        &mut restarts,
                    )?;
                    let current = app.as_mut().expect("a started application");
                    let prefix = format!("{name} ");
                    if inspect::controls(current.slint(), AccessibleRole::Button)
                        .iter()
                        .any(|(label, _)| *label == name || label.starts_with(&prefix))
                    {
                        failures.push(format!("the footer shows {name} for {}", action.id));
                    } else {
                        passed += 1;
                    }
                    continue;
                }
                Some(action) => {
                    if let Some(reason) = excluded(action) {
                        skipped.push(format!("{name} {} ({reason})", action.id));
                        continue;
                    }
                    if action.id == "app.exit" {
                        exit = Some((held, name));
                        continue;
                    }
                    match probe_for(&action.id, &paths) {
                        Some(probe) => (probe, true),
                        None => {
                            failures.push(format!("footer {name}: no probe for {}", action.id));
                            continue;
                        }
                    }
                }
            };
            ready(
                &mut app,
                &mut seed,
                probe.setup.seed,
                &mut fresh,
                &paths,
                &mut restarts,
            )?;
            let current = app.as_mut().expect("a started application");
            let outcome = run_probe(
                current,
                format!("footer {name}"),
                &probe,
                &paths,
                &mut |app| click_cell(app, held, &name, enabled),
            );
            match outcome.error {
                // One finding for every unavailable cell that does this.
                Some(error) if !enabled && error == "it also changed focus: 0 → -1" => {
                    focus_lost.push(name.clone());
                }
                Some(error) => failures.push(format!("{}: {error}", outcome.label)),
                None => passed += 1,
            }
        }
    }
    if !focus_lost.is_empty() {
        failures.push(format!(
            "finding: a click on an unavailable footer cell takes the keyboard focus from the file list (focused panel 0 → -1), so arrow keys and typing then reach no panel; cells: {focus_lost:?}"
        ));
    }
    if let Some(current) = app.as_mut() {
        input::press(current.slint(), Key::Control);
        current.settle();
        current.screenshot(context, "footer-ctrl", "The footer while Ctrl is held");
        input::release(current.slint(), Key::Control);
        current.settle();
    }
    if let Some((held, name)) = exit {
        let probe = probe_for("app.exit", &paths).expect("the exit probe");
        ready(
            &mut app,
            &mut seed,
            Seed::Default,
            &mut fresh,
            &paths,
            &mut restarts,
        )?;
        let current = app.as_mut().expect("a started application");
        let outcome = run_probe(
            current,
            format!("footer {name}"),
            &probe,
            &paths,
            &mut |app| click_cell(app, held, &name, true),
        );
        match outcome.error {
            Some(error) => failures.push(format!("{}: {error}", outcome.label)),
            None => passed += 1,
        }
    }
    if let Some(mut current) = app.take() {
        current.shutdown()?;
    }
    context.note(format!(
        "{passed} footer cells did what their key does ({restarts} restarts); not clicked: {skipped:?}"
    ));
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} footer cells failed:\n{}",
            failures.len(),
            failures.join("\n")
        ))
    }
}
