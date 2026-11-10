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
