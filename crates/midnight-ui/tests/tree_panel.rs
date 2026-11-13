//! Installed panel bridge tests; commands are the real application boundary.
#[path = "support/ui.rs"]
mod ui;

use i_slint_core::items::AccessibleRole;
use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use slint::Model;
use std::{cell::RefCell, rc::Rc, sync::Arc};

#[test]
fn disclosure_adopts_exact_projection_and_keeps_navigation_separate() {
    let mut ui = ui::TestUi::new();
    let root_path = std::env::temp_dir().join("tree-panel-root");
    let presenter = Rc::new(RefCell::new(Presenter::new(
        root_path.clone(),
        root_path.clone(),
    )));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let captured = commands.clone();
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new(move |command| {
        captured.borrow_mut().push(command);
        true
    });
    install(&ui.window, presenter.clone(), send.clone());
    ui.window.invoke_action(0, 3);
    let navigation = commands
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
        .unwrap();
    let entries: Arc<[Entry]> = vec![Entry::new(
        EntryId(42),
        root_path.join("Folder 文档"),
        EntryKind::Directory,
    )]
    .into();
    let index = Arc::new(DirectoryIndex::from_entries(&entries));
    presenter.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        entries: entries.clone(),
        index: index.clone(),
    });
    refresh(&ui.window, &presenter.borrow());
    commands.borrow_mut().clear();
    let identity = ui.window.get_left_rows().row_data(1).unwrap().identity;
    ui.draw();
    let button = ui::root_position(
        &ui.window,
        "Discover folders in Folder 文档",
        AccessibleRole::Button,
    );
    ui::click(&ui.window, button.x, button.y);
    let root = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Tree(command) => match command.as_ref() {
                TreeCommand::Enable { root, .. } => Some(root.clone()),
                _ => None,
            },
            _ => None,
        })
        .expect("the installed disclosure callback must request a backend tree session");
    assert!(Arc::ptr_eq(&root.entries, &entries));
    assert!(Arc::ptr_eq(&root.index, &index));
    assert!(
        !commands
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::Navigate { .. } | Command::Selection(_)))
    );

    let mut display = entries[0].clone();
    display.id = EntryId(7);
    let view = Arc::new(
        TreeProjection::new(
            root,
            TreeViewStamp {
                pane: PaneId::Left,
                navigation,
                session: TreeSessionId(19),
                revision: TreeRevision(1),
            },
            TreeLayoutRevision(1),
            vec![TreeDisplayRow {
                id: TreeRowId(7),
                entry: display,
                parent: None,
                depth: 0,
                disclosure: TreeDisclosure::Unknown,
                expanded: false,
                selectable: true,
                navigable: true,
                context_only: false,
                error: None,
                error_details: None,
                original_root_entry: Some(EntryId(42)),
                prefix_end: 0,
                search_match: false,
            }],
        )
        .unwrap(),
    );
    let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
    commands.borrow_mut().clear();
    presenter.borrow_mut().apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(
            TreePublication::new(view.clone(), SelectionRevision(0), selection).unwrap(),
        ))),
        send.as_ref(),
    );
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    let commands_ref = commands.borrow();
    assert!(
        matches!(&commands_ref[0], Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Commit(actual) if Arc::ptr_eq(actual, &view)))
    );
    assert!(commands_ref.iter().any(|command| matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Branch { view: actual, row: TreeRowId(7), action: TreeBranchAction::Expand, .. } if Arc::ptr_eq(actual, &view)))));
    drop(commands_ref);
    commands.borrow_mut().clear();
    ui.window.invoke_tree_disclosure(0, identity);
    assert!(
        commands.borrow().is_empty(),
        "a stale flat-row identity cannot control the adopted view"
    );
    ui.window.invoke_open(0, 1);
    assert!(commands.borrow().iter().any(|command| matches!(command, Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Navigate { view: actual, row: TreeRowId(7), .. } if Arc::ptr_eq(actual, &view)))));
}

#[test]
fn ctrl_disclosure_reaches_recursive_branch_after_exact_commit_and_queue_backpressure() {
    use slint::{
        ComponentHandle,
        platform::{Key, WindowEvent},
    };
    use std::cell::Cell;

    let mut ui = ui::TestUi::new();
    let base = std::env::temp_dir().join("recursive-tree-installed");
    let paths = [base.join("left"), base.join("right")];
    let presenter = Rc::new(RefCell::new(Presenter::new(
        paths[0].clone(),
        paths[1].clone(),
    )));
    let attempts = Rc::new(RefCell::new(Vec::new()));
    let allow_commit = Rc::new(Cell::new(true));
    let allow_branch = Rc::new(Cell::new(true));
    let send: Rc<dyn Fn(Command) -> bool> = Rc::new({
        let attempts = attempts.clone();
        let allow_commit = allow_commit.clone();
        let allow_branch = allow_branch.clone();
        move |command| {
            let accepted = match &command {
                Command::Tree(command) => match command.as_ref() {
                    TreeCommand::Commit(_) => allow_commit.get(),
                    TreeCommand::Branch { .. } => allow_branch.get(),
                    _ => true,
                },
                _ => true,
            };
            attempts.borrow_mut().push((command, accepted));
            accepted
        }
    });
    install(&ui.window, presenter.clone(), send.clone());
    let mut preferences = UserPreferences::default();
    for (slot, path) in paths.iter().enumerate() {
        preferences.tabs[slot].paths = vec![path.clone()];
    }
    presenter.borrow_mut().apply_with_sender(
        Event::PreferencesLoaded(Ok(Box::new(preferences))),
        send.as_ref(),
    );
    let entries: [Arc<[Entry]>; 2] = [
        vec![Entry::new(
            EntryId(42),
            paths[0].join("Ctrl folder 文档"),
            EntryKind::Directory,
        )]
        .into(),
        vec![Entry::new(
            EntryId(42),
            paths[1].join("Ctrl link Ω"),
            EntryKind::DirectoryLink,
        )]
        .into(),
    ];
    let indices: [Arc<DirectoryIndex>; 2] =
        std::array::from_fn(|slot| Arc::new(DirectoryIndex::from_entries(&entries[slot])));
    let navigations: [NavigationId; 2] = std::array::from_fn(|slot| {
        attempts
            .borrow()
            .iter()
            .rev()
            .find_map(|(command, accepted)| match command {
                Command::Navigate {
                    pane,
                    navigation,
                    path,
                    ..
                } if *accepted && pane.index() == slot && path == &paths[slot] => Some(*navigation),
                _ => None,
            })
            .expect("preferences must establish the latest exact pane navigation")
    });
    for pane in [PaneId::Left, PaneId::Right] {
        let slot = pane.index();
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation: navigations[slot],
            entries: entries[slot].clone(),
            index: indices[slot].clone(),
        });
    }
    refresh(&ui.window, &presenter.borrow());
    ui.draw();
    assert!(ui.window.get_preferences_ready());

    for pane in [PaneId::Left, PaneId::Right] {
        let slot = pane.index();
        let label = format!("Discover folders in {}", entries[slot][0].display_name);
        attempts.borrow_mut().clear();
        ui.window.window().dispatch_event(WindowEvent::KeyPressed {
            text: Key::Control.into(),
        });
        ui::click_root(&ui.window, &label, AccessibleRole::Button);
        ui.window.window().dispatch_event(WindowEvent::KeyReleased {
            text: Key::Control.into(),
        });
        let root = attempts
            .borrow()
            .iter()
            .find_map(|(command, accepted)| match command {
                Command::Tree(command) if *accepted => match command.as_ref() {
                    TreeCommand::Enable {
                        root,
                        selected: TreeRootSelection::Empty,
                        ..
                    } if root.pane == pane => Some(root.clone()),
                    _ => None,
                },
                _ => None,
            })
            .expect("Ctrl-click must enable the exact folder or directory-link tree");
        assert_eq!(root.navigation, navigations[slot]);
        assert!(Arc::ptr_eq(&root.entries, &entries[slot]));
        assert!(Arc::ptr_eq(&root.index, &indices[slot]));
        assert_eq!(
            attempts
                .borrow()
                .iter()
                .filter(|(command, _)| matches!(command,
            Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Enable { root, .. }
                if root.pane == pane)))
                .count(),
            1,
            "Ctrl selects only one disclosure route"
        );
        assert_only_disclosure_commands(&attempts.borrow());

        let mut entry = entries[slot][0].clone();
        entry.id = EntryId(7);
        let view = Arc::new(
            TreeProjection::new(
                root,
                TreeViewStamp {
                    pane,
                    navigation: navigations[slot],
                    session: TreeSessionId(90 + slot as u64),
                    revision: TreeRevision(1),
                },
                TreeLayoutRevision(1),
                vec![TreeDisplayRow {
                    id: TreeRowId(7),
                    entry,
                    parent: None,
                    depth: 0,
                    disclosure: TreeDisclosure::Unknown,
                    expanded: false,
                    selectable: true,
                    navigable: true,
                    context_only: false,
                    error: None,
                    error_details: None,
                    original_root_entry: Some(EntryId(42)),
                    prefix_end: 0,
                    search_match: false,
                }],
            )
            .unwrap(),
        );
        let selection = Arc::new(TreeSelectionSnapshot::from_bits(view.clone(), vec![0]).unwrap());
        allow_commit.set(false);
        allow_branch.set(false);
        attempts.borrow_mut().clear();
        presenter.borrow_mut().apply_with_sender(
            Event::Tree(Box::new(TreeEvent::Published(
                TreePublication::new(view.clone(), SelectionRevision(0), selection).unwrap(),
            ))),
            send.as_ref(),
        );
        assert!(attempts.borrow().iter().any(|(command, accepted)| !*accepted
            && matches!(command, Command::Tree(command)
                if matches!(command.as_ref(), TreeCommand::Commit(actual) if Arc::ptr_eq(actual, &view)))));
        assert!(
            !attempts
                .borrow()
                .iter()
                .any(|(command, _)| matches!(command,
            Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Branch { .. })))
        );

        attempts.borrow_mut().clear();
        allow_commit.set(true);
        presenter.borrow_mut().flush_selection(send.as_ref());
        let attempted = attempts.borrow();
        let commit = attempted.iter().position(|(command, accepted)| *accepted
            && matches!(command, Command::Tree(command)
                if matches!(command.as_ref(), TreeCommand::Commit(actual) if Arc::ptr_eq(actual, &view))))
            .expect("the exact publication must commit before its recursive branch");
        let branch = attempted
            .iter()
            .position(|(command, accepted)| {
                !*accepted
                    && matches!(command, Command::Tree(command)
                if matches!(command.as_ref(), TreeCommand::Branch {
                    view: actual, row: TreeRowId(7), action: TreeBranchAction::ExpandRecursive, ..
                } if Arc::ptr_eq(actual, &view)))
            })
            .expect(
                "the recursive branch reaches queue backpressure without becoming ordinary Expand",
            );
        assert!(commit < branch);
        assert_only_disclosure_commands(&attempted);
        drop(attempted);

        attempts.borrow_mut().clear();
        allow_branch.set(true);
        presenter.borrow_mut().flush_selection(send.as_ref());
        assert_eq!(
            attempts
                .borrow()
                .iter()
                .filter(|(command, accepted)| *accepted
                    && matches!(command, Command::Tree(command)
                if matches!(command.as_ref(), TreeCommand::Branch {
                    view: actual, row: TreeRowId(7), action: TreeBranchAction::ExpandRecursive, ..
                } if Arc::ptr_eq(actual, &view))))
                .count(),
            1
        );
        assert_eq!(
            attempts
                .borrow()
                .iter()
                .filter(|(command, _)| matches!(command,
            Command::Tree(command) if matches!(command.as_ref(), TreeCommand::Branch { .. })))
                .count(),
            1
        );
        assert_only_disclosure_commands(&attempts.borrow());
        refresh(&ui.window, &presenter.borrow());
        ui.draw();
    }
}

fn assert_only_disclosure_commands(attempts: &[(Command, bool)]) {
    assert!(
        !attempts.iter().any(|(command, _)| match command {
            Command::Navigate { .. }
            | Command::Refresh { .. }
            | Command::Resort { .. }
            | Command::NetworkNavigate { .. }
            | Command::ArchiveNavigate { .. }
            | Command::Selection(_) => true,
            Command::Tree(command) => matches!(
                command.as_ref(),
                TreeCommand::Select(_) | TreeCommand::Navigate { .. }
            ),
            _ => false,
        }),
        "Ctrl-expander input must not toggle marks or navigate"
    );
}
