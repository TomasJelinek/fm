//! Installed command shortcuts use acknowledged native tree occurrences as literal text.
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use midnight_ui::{FileRow, Presenter, install, refresh};
use slint::{ComponentHandle, Model, platform::Key};
use std::{
    cell::{Cell, RefCell},
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
};

const ROOT_FOLDER: EntryId = EntryId(41);
const ROOT_FILE: EntryId = EntryId(42);
const FOLDER: TreeRowId = TreeRowId(102);
const FILE: TreeRowId = TreeRowId(103);
const CONTROL_FILE: TreeRowId = TreeRowId(104);
const VIRTUAL_FILE: TreeRowId = TreeRowId(105);
const MARKED_ROOTS: u64 = (1 << 0) | (1 << 5);
const DRAFT: &str = "echo OLD tail";

#[derive(Clone, Copy, Debug)]
enum SourceFocus {
    Files,
    Command,
}

#[derive(Clone, Copy, Debug)]
enum Insertion {
    Name,
    FullPath,
    PaneRoot,
}
impl Insertion {
    fn default_key(self, window: &midnight_ui::MainWindow) {
        match self {
            Self::Name => ui::chord(window, &[Key::Control], Key::Return),
            Self::FullPath => ui::chord(window, &[Key::Control, Key::Shift], "p"),
            Self::PaneRoot => ui::chord(window, &[Key::Control], "p"),
        }
    }
    fn remapped_key(self, window: &midnight_ui::MainWindow) {
        let key = match self {
            Self::Name => "n",
            Self::FullPath => "p",
            Self::PaneRoot => "i",
        };
        ui::chord(window, &[Key::Control, Key::Alt], key);
    }
}

struct CommitAttempt {
    view: Arc<TreeProjection>,
    accepted: bool,
}

struct Harness {
    ui: ui::TestUi,
    presenter: Rc<RefCell<Presenter>>,
    commands: Rc<RefCell<Vec<Command>>>,
    send: Rc<dyn Fn(Command) -> bool>,
    allow_commits: Rc<Cell<bool>>,
    commits: Rc<RefCell<Vec<CommitAttempt>>>,
    paths: [PathBuf; 2],
    entries: [Arc<[Entry]>; 2],
    indices: [Arc<DirectoryIndex>; 2],
    views: [Option<Arc<TreeProjection>>; 2],
    selection_revisions: [SelectionRevision; 2],
}

impl Harness {
    fn new(remapped: bool) -> Self {
        let ui = ui::TestUi::new();
        let base = std::env::temp_dir().join("cmd02 tree command UI");
        let paths = [
            base.join("left 文档 ' $(root)"),
            base.join("right Ω ' $(root)"),
        ];
        assert!(paths.iter().all(|path| path.is_absolute()));
        let entries: [Arc<[Entry]>; 2] = std::array::from_fn(|slot| {
            vec![
                Entry::new(
                    ROOT_FOLDER,
                    paths[slot].join("expanded root"),
                    EntryKind::Directory,
                ),
                Entry::new(
                    ROOT_FILE,
                    paths[slot].join("marked other root.txt"),
                    EntryKind::File,
                ),
            ]
            .into()
        });
        let indices =
            std::array::from_fn(|slot| Arc::new(DirectoryIndex::from_entries(&entries[slot])));
        let presenter = Rc::new(RefCell::new(Presenter::new(
            paths[0].clone(),
            paths[1].clone(),
        )));
        let commands = Rc::new(RefCell::new(Vec::new()));
        let allow_commits = Rc::new(Cell::new(true));
        let commits = Rc::new(RefCell::new(Vec::new()));
        let send: Rc<dyn Fn(Command) -> bool> = Rc::new({
            let commands = commands.clone();
            let allow_commits = allow_commits.clone();
            let commits = commits.clone();
            move |command| {
                if let Command::Tree(tree) = &command
                    && let TreeCommand::Commit(view) = tree.as_ref()
                {
                    let accepted = allow_commits.get();
                    commits.borrow_mut().push(CommitAttempt {
                        view: view.clone(),
                        accepted,
                    });
                    if !accepted {
                        return false;
                    }
                }
                commands.borrow_mut().push(command);
                true
            }
        });
        install(&ui.window, presenter.clone(), send.clone());
        let mut preferences = UserPreferences::default();
        for (slot, path) in paths.iter().enumerate() {
            preferences.tabs[slot].paths = vec![path.clone()];
        }
        if remapped {
            preferences.shortcuts = [
                ("command.insert-name", "Ctrl+Alt+N"),
                ("command.insert-full-path", "Ctrl+Alt+P"),
                ("command.insert-path", "Ctrl+Alt+I"),
            ]
            .into_iter()
            .map(|(action, key)| ShortcutBinding {
                action: action.into(),
                keys: vec![key.into()],
            })
            .collect();
        }
        preferences.validate().unwrap();
        presenter.borrow_mut().apply_with_sender(
            Event::PreferencesLoaded(Ok(Box::new(preferences))),
            send.as_ref(),
        );
        presenter.borrow_mut().apply(Event::DesktopContext {
            network_position: 0,
            shell: ShellFlavor::Posix,
            locations: Vec::new(),
        });
        let mut h = Self {
            ui,
            presenter,
            commands,
            send,
            allow_commits,
            commits,
            paths,
            entries,
            indices,
            views: Default::default(),
            selection_revisions: [SelectionRevision(0); 2],
        };
        for pane in [PaneId::Left, PaneId::Right] {
            let navigation = h
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
                .expect("installed preferences must request the current pane listing");
            h.apply(Event::ListingFinished {
                pane,
                navigation,
                entries: h.entries[pane.index()].clone(),
                index: h.indices[pane.index()].clone(),
            });
        }
        h.settle();
        for pane in [PaneId::Left, PaneId::Right] {
            h.enable_tree(pane);
        }
        h.settle();
        h.commands.borrow_mut().clear();
        h
    }

    fn apply(&mut self, event: Event) {
        self.presenter
            .borrow_mut()
            .apply_with_sender(event, self.send.as_ref());
        refresh(&self.ui.window, &self.presenter.borrow());
    }

    fn settle(&mut self) {
        refresh(&self.ui.window, &self.presenter.borrow());
        self.ui.draw();
        self.presenter
            .borrow_mut()
            .flush_selection(self.send.as_ref());
        refresh(&self.ui.window, &self.presenter.borrow());
        self.ui.draw();
    }

    fn rows(&self, pane: PaneId) -> Vec<FileRow> {
        if pane == PaneId::Left {
            self.ui.window.get_left_rows()
        } else {
            self.ui.window.get_right_rows()
        }
        .iter()
        .collect()
    }

    fn index(&self, pane: PaneId, id: u64) -> usize {
        let id = id.to_string();
        self.rows(pane)
            .iter()
            .position(|row| {
                !row.parent_entry && row.identity.rsplit(':').next() == Some(id.as_str())
            })
            .expect("the published occurrence must be visible")
    }

    fn enable_tree(&mut self, pane: PaneId) {
        let slot = pane.index();
        // These marks travel through the installed selection bridge before the
        // public backend receipt migrates the exact root snapshot into a tree.
        for id in [ROOT_FOLDER, ROOT_FILE] {
            self.ui.window.invoke_row_click(
                slot as i32,
                self.index(pane, id.0) as i32,
                true,
                false,
            );
        }
        let request = self
            .commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Selection(request) if request.pane == pane => Some(request.clone()),
                _ => None,
            })
            .expect("root marks must use the selection service contract");
        assert!(Arc::ptr_eq(
            request.view.as_ref().unwrap(),
            &self.indices[slot]
        ));
        self.apply(Event::SelectionChanged {
            pane,
            navigation: request.navigation,
            revision: request.revision,
            selection: Arc::new(
                SelectionSnapshot::from_bits(self.indices[slot].clone(), vec![3]).unwrap(),
            ),
            deferred: Default::default(),
        });
        self.settle();
        let identity = self.rows(pane)[self.index(pane, ROOT_FOLDER.0)]
            .identity
            .clone();
        self.ui.window.invoke_tree_disclosure(slot as i32, identity);
        let (root, revision) = self.commands.borrow().iter().rev().find_map(|command| match command {
            Command::Tree(command) => match command.as_ref() {
                TreeCommand::Enable { root, selection_revision, selected } if root.pane == pane => {
                    assert!(matches!(selected, TreeRootSelection::Native(selection)
                        if !selection.is_empty() && Arc::ptr_eq(selection.basis(), &self.indices[slot])));
                    Some((root.clone(), *selection_revision))
                }
                _ => None,
            },
            _ => None,
        }).expect("installed disclosure must capture a backend tree root");
        assert!(Arc::ptr_eq(&root.entries, &self.entries[slot]));
        assert!(Arc::ptr_eq(&root.index, &self.indices[slot]));
        let view = Arc::new(
            TreeProjection::new(
                root.clone(),
                TreeViewStamp {
                    pane,
                    navigation: root.navigation,
                    session: TreeSessionId(700 + slot as u64),
                    revision: TreeRevision(1),
                },
                TreeLayoutRevision(1),
                tree_rows(&root),
            )
            .unwrap(),
        );
        self.selection_revisions[slot] = revision;
        self.publish(view.clone());
        self.views[slot] = Some(view.clone());
        self.settle();
        assert!(
            self.commits
                .borrow()
                .iter()
                .any(|attempt| attempt.accepted && Arc::ptr_eq(&attempt.view, &view))
        );
        assert_eq!(self.rows(pane).iter().filter(|row| row.selected).count(), 2);
    }

    fn publish(&mut self, view: Arc<TreeProjection>) {
        let pane = view.stamp.pane;
        let selection =
            Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![MARKED_ROOTS]).unwrap());
        self.apply(Event::Tree(Box::new(TreeEvent::Published(
            TreePublication::new(view, self.selection_revisions[pane.index()], selection).unwrap(),
        ))));
    }

    fn view(&self, pane: PaneId) -> Arc<TreeProjection> {
        self.views[pane.index()].as_ref().unwrap().clone()
    }

    fn focus(&mut self, pane: PaneId, row: Option<TreeRowId>) {
        self.ui.window.invoke_activate(pane.index() as i32);
        let index = row.map_or(0, |row| self.index(pane, row.0));
        if row.is_none() {
            assert!(self.rows(pane)[0].parent_entry);
        }
        self.ui
            .window
            .invoke_row_click(pane.index() as i32, index as i32, false, false);
        self.ui.window.invoke_focus_files();
    }

    fn stage(&mut self, pane: PaneId, row: Option<TreeRowId>, focus: SourceFocus, selected: bool) {
        self.focus(pane, row);
        self.ui.window.invoke_set_command(DRAFT.into(), 5);
        if selected {
            for _ in 0..3 {
                ui::chord(&self.ui.window, &[Key::Shift], Key::RightArrow);
            }
        }
        assert!(self.ui.window.get_command_focused());
        assert_eq!(self.ui.window.get_command_anchor(), 5);
        let cursor = if selected { 8 } else { 5 };
        assert_eq!(self.ui.window.get_command_cursor(), cursor);
        if matches!(focus, SourceFocus::Files) {
            // TextInput clears its selection on focus loss and keeps the caret.
            self.ui.window.invoke_focus_files();
        }
        self.settle();
        assert_eq!(self.ui.window.get_command_text(), DRAFT);
        assert_eq!(
            self.ui.window.get_command_anchor(),
            if matches!(focus, SourceFocus::Files) {
                cursor
            } else {
                5
            }
        );
        assert_eq!(self.ui.window.get_command_cursor(), cursor);
        assert_eq!(
            self.ui.window.get_command_focused(),
            matches!(focus, SourceFocus::Command)
        );
        self.commands.borrow_mut().clear();
    }

    fn assert_inserted(&self, expected: &str, offset: usize) {
        assert_eq!(self.ui.window.get_command_text().as_str(), expected);
        assert_eq!(self.ui.window.get_command_cursor() as usize, offset);
        assert_eq!(self.ui.window.get_command_anchor() as usize, offset);
        assert!(self.ui.window.get_command_focused());
        assert!(
            self.commands.borrow().is_empty(),
            "literal insertion must not enqueue backend work"
        );
    }

    fn dismiss_notice(&mut self) {
        self.ui.draw();
        if self.ui.window.get_embedded_modal_open() {
            ui::key(&self.ui.window, Key::Escape);
            self.ui.draw();
            assert!(!self.ui.window.get_embedded_modal_open());
        }
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.ui.window.hide().unwrap();
        self.presenter.borrow_mut().retire_all();
    }
}

fn row(
    id: TreeRowId,
    mut entry: Entry,
    parent: Option<TreeRowId>,
    depth: usize,
    expanded: bool,
    original: Option<EntryId>,
) -> TreeDisplayRow {
    entry.id = EntryId(id.0);
    let directory = entry.kind == EntryKind::Directory;
    TreeDisplayRow {
        id,
        entry,
        parent,
        depth,
        disclosure: if expanded {
            TreeDisclosure::Expanded
        } else if directory {
            TreeDisclosure::Unknown
        } else {
            TreeDisclosure::None
        },
        expanded,
        selectable: true,
        navigable: directory,
        context_only: false,
        error: None,
        error_details: None,
        original_root_entry: original,
        prefix_end: 0,
        search_match: false,
    }
}

fn tree_rows(root: &TreeRootSnapshot) -> Vec<TreeDisplayRow> {
    let folder = root.entries[root.index.by_id[&ROOT_FOLDER]].clone();
    let other = root.entries[root.index.by_id[&ROOT_FILE]].clone();
    let nested_path = folder.path.join("nested ' $(folder) & 文档");
    let mut nested = Entry::new(EntryId(FOLDER.0), nested_path.clone(), EntryKind::Directory);
    nested.display_name = "painted folder caption".into();
    let mut file = Entry::new(
        EntryId(FILE.0),
        nested_path.join("file ' $(data) & 文档.txt"),
        EntryKind::File,
    );
    file.display_name = "painted file caption".into();
    let mut control = Entry::new(
        EntryId(CONTROL_FILE.0),
        nested_path.join("unsafe\nline.txt"),
        EntryKind::File,
    );
    control.display_name = "painted control-name caption".into();
    let virtual_file = Entry::from_archive(ArchiveNode {
        modified: None,
        id: VirtualEntryId {
            mount: 77,
            entry: 9,
        },
        parent: 1,
        raw_name: Arc::from(&b"virtual-member.txt"[..]),
        display_name: nested_path
            .join("native-looking provider caption.txt")
            .to_string_lossy()
            .into_owned(),
        kind: EntryKind::File,
        size: Some(9),
        readable: true,
    });
    vec![
        row(TreeRowId(101), folder, None, 0, true, Some(ROOT_FOLDER)),
        row(FOLDER, nested, Some(TreeRowId(101)), 1, true, None),
        row(FILE, file, Some(FOLDER), 2, false, None),
        row(CONTROL_FILE, control, Some(FOLDER), 2, false, None),
        row(VIRTUAL_FILE, virtual_file, Some(FOLDER), 2, false, None),
        row(TreeRowId(106), other, None, 0, false, Some(ROOT_FILE)),
    ]
}

fn native_token(h: &Harness, pane: PaneId, row: TreeRowId, insertion: Insertion) -> String {
    let view = h.view(pane);
    let entry = &view.rows[view.index.row_positions[&row]].entry;
    let path = entry.native_path().unwrap();
    match insertion {
        Insertion::Name => path.file_name().unwrap().to_str().unwrap().to_owned(),
        Insertion::FullPath => path.to_str().unwrap().to_owned(),
        Insertion::PaneRoot => panic!("pane-root insertion does not use a descendant path"),
    }
}

#[test]
fn native_tree_name_and_full_path_append_focused_raw_text_from_files_and_command() {
    let mut h = Harness::new(false);
    for pane in [PaneId::Left, PaneId::Right] {
        for row in [FOLDER, FILE] {
            for focus in [SourceFocus::Files, SourceFocus::Command] {
                for insertion in [Insertion::Name, Insertion::FullPath] {
                    h.stage(pane, Some(row), focus, true);
                    assert_eq!(h.rows(pane).iter().filter(|row| row.selected).count(), 2);
                    assert!(!h.rows(pane)[h.index(pane, row.0)].selected);
                    let token = native_token(&h, pane, row, insertion);
                    insertion.default_key(&h.ui.window);
                    let expected = format!("{DRAFT}{token}");
                    h.assert_inserted(&expected, expected.len());
                }
            }
        }
    }
}

#[test]
fn native_tree_ctrl_p_uses_the_pane_root_at_the_caret_or_selected_text() {
    let mut h = Harness::new(false);
    for pane in [PaneId::Left, PaneId::Right] {
        for row in [FOLDER, FILE] {
            for focus in [SourceFocus::Files, SourceFocus::Command] {
                for selected in [false, true] {
                    h.stage(pane, Some(row), focus, selected);
                    let token = format!(
                        "{}{}",
                        h.paths[pane.index()].display(),
                        std::path::MAIN_SEPARATOR
                    );
                    Insertion::PaneRoot.default_key(&h.ui.window);
                    let (expected, caret) = match (focus, selected) {
                        (SourceFocus::Files, true) => {
                            (format!("echo OLD{token} tail"), 8 + token.len())
                        }
                        (SourceFocus::Command, true) => {
                            (format!("echo {token} tail"), 5 + token.len())
                        }
                        (_, false) => (format!("echo {token}OLD tail"), 5 + token.len()),
                    };
                    h.assert_inserted(&expected, caret);
                }
            }
        }
    }
}

#[test]
fn native_tree_command_insertions_honor_persisted_remaps_without_default_fallbacks() {
    let mut h = Harness::new(true);
    for pane in [PaneId::Left, PaneId::Right] {
        for focus in [SourceFocus::Files, SourceFocus::Command] {
            for insertion in [Insertion::Name, Insertion::FullPath, Insertion::PaneRoot] {
                h.stage(pane, Some(FILE), focus, true);
                insertion.default_key(&h.ui.window);
                assert_eq!(
                    h.ui.window.get_command_text(),
                    DRAFT,
                    "unbound default {insertion:?} inserted text"
                );
                assert!(
                    h.commands.borrow().is_empty(),
                    "an unbound default must not execute or mutate"
                );
                h.stage(pane, Some(FILE), focus, true);
                insertion.remapped_key(&h.ui.window);
                let (expected, caret) = match insertion {
                    Insertion::Name | Insertion::FullPath => {
                        let expected =
                            format!("{DRAFT}{}", native_token(&h, pane, FILE, insertion));
                        let end = expected.len();
                        (expected, end)
                    }
                    Insertion::PaneRoot => {
                        let token = format!(
                            "{}{}",
                            h.paths[pane.index()].display(),
                            std::path::MAIN_SEPARATOR
                        );
                        match focus {
                            SourceFocus::Files => {
                                (format!("echo OLD{token} tail"), 8 + token.len())
                            }
                            SourceFocus::Command => (format!("echo {token} tail"), 5 + token.len()),
                        }
                    }
                };
                h.assert_inserted(&expected, caret);
            }
        }
    }
}

#[test]
fn native_tree_keys_wait_for_the_exact_replacement_commit_before_inserting() {
    let mut h = Harness::new(false);
    let pane = PaneId::Left;
    let previous = h.view(pane);
    let next = Arc::new(
        TreeProjection::new(
            previous.root.clone(),
            TreeViewStamp {
                revision: TreeRevision(previous.stamp.revision.0 + 1),
                ..previous.stamp
            },
            TreeLayoutRevision(previous.layout_revision.0 + 1),
            previous.rows.to_vec(),
        )
        .unwrap(),
    );
    h.allow_commits.set(false);
    h.publish(next.clone());
    h.views[pane.index()] = Some(next.clone());
    assert!(
        h.commits
            .borrow()
            .iter()
            .any(|attempt| !attempt.accepted && Arc::ptr_eq(&attempt.view, &next))
    );
    for focus in [SourceFocus::Files, SourceFocus::Command] {
        for insertion in [Insertion::Name, Insertion::FullPath] {
            h.stage(pane, Some(FILE), focus, true);
            insertion.default_key(&h.ui.window);
            assert_eq!(
                h.ui.window.get_command_text(),
                DRAFT,
                "pending Commit allowed {insertion:?}"
            );
            assert!(h.commands.borrow().is_empty());
        }
    }
    h.allow_commits.set(true);
    h.presenter.borrow_mut().flush_selection(h.send.as_ref());
    assert!(h.commands.borrow().iter().any(|command| matches!(command,
        Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Commit(actual) if Arc::ptr_eq(actual, &next)))));
    for insertion in [Insertion::Name, Insertion::FullPath] {
        h.stage(pane, Some(FILE), SourceFocus::Files, true);
        let expected = format!("{DRAFT}{}", native_token(&h, pane, FILE, insertion));
        insertion.default_key(&h.ui.window);
        h.assert_inserted(&expected, expected.len());
    }
}

#[test]
fn virtual_and_control_names_do_not_insert_or_execute_and_parent_keeps_name_only() {
    let mut h = Harness::new(false);
    for pane in [PaneId::Left, PaneId::Right] {
        let view = h.view(pane);
        let virtual_entry = &view.rows[view.index.row_positions[&VIRTUAL_FILE]].entry;
        assert!(virtual_entry.path.as_os_str().is_empty());
        assert!(Path::new(&virtual_entry.display_name).is_absolute());
        for row in [VIRTUAL_FILE, CONTROL_FILE] {
            for insertion in [Insertion::Name, Insertion::FullPath] {
                h.stage(pane, Some(row), SourceFocus::Files, true);
                insertion.default_key(&h.ui.window);
                assert_eq!(h.ui.window.get_command_text(), DRAFT);
                assert!(
                    h.commands.borrow().is_empty(),
                    "refused insertion must not enqueue work"
                );
                h.dismiss_notice();
            }
        }
        h.stage(pane, None, SourceFocus::Files, true);
        Insertion::Name.default_key(&h.ui.window);
        let expected = format!("{DRAFT}..");
        h.assert_inserted(&expected, expected.len());
        h.stage(pane, None, SourceFocus::Files, true);
        Insertion::FullPath.default_key(&h.ui.window);
        assert_eq!(h.ui.window.get_command_text(), DRAFT);
        assert!(h.commands.borrow().is_empty());
    }
}
