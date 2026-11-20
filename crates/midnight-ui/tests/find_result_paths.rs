//! A shortened Path cell of the Find results keeps its last folder, as the fed list's
//! Folder cells and every other shortened path do: results in sibling folders stay
//! distinct.
use crate::support::modeless;

use i_slint_core::{
    item_tree::ItemRc,
    items::{ComplexText, Orientation, SimpleText},
    window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter};
use slint::{ComponentHandle, platform::WindowAdapter};
use std::{cell::RefCell, rc::Rc, sync::Arc};

/// The visible text items of `window` that say `caption`.
fn shown(window: &impl WindowAdapter, caption: &str) -> Vec<ItemRc> {
    fn visit(item: ItemRc, caption: &str, found: &mut Vec<ItemRc>) {
        let text = item
            .downcast::<SimpleText>()
            .map(|text| text.as_pin_ref().text())
            .or_else(|| {
                item.downcast::<ComplexText>()
                    .map(|text| text.as_pin_ref().text())
            });
        if item.is_visible() && text.is_some_and(|text| text == caption) {
            found.push(item.clone());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            visit(item, caption, found);
        }
    }
    let mut found = Vec::new();
    visit(
        ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
        caption,
        &mut found,
    );
    found
}

#[test]
fn a_shortened_result_path_keeps_its_last_folder() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(modeless::Windows(windows.clone()))).unwrap();
    let main = MainWindow::new().unwrap();
    let root = [
        "acceptance-find",
        "20261120-135515",
        "work",
        "find_layout",
        "results in a deeply nested folder",
    ]
    .iter()
    .fold(std::env::temp_dir(), |path, name| path.join(name));
    let state = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    midnight_ui::install(
        &main,
        state.clone(),
        Rc::new({
            let commands = commands.clone();
            move |command| {
                commands.borrow_mut().push(command);
                true
            }
        }),
    );
    main.show().unwrap();
    modeless::draw(&windows.borrow()[0], 1000, 700, "find-path-tail-main");
    main.invoke_find_files(0);
    let find = windows.borrow()[1].clone();
    modeless::draw(&find, 860, 620, "find-path-tail");
    modeless::click(&find, "Find");
    let request = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::StartFind(request) => Some(request.clone()),
            _ => None,
        })
        .expect("Find starts a search");
    let results: Vec<_> = ["folder-1", "folder-2"]
        .into_iter()
        .enumerate()
        .map(|(index, folder)| {
            Arc::new(FindResult {
                id: FindResultId(index as u64 + 1),
                duplicate_group: None,
                duplicate_group_size: 0,
                archive: None,
                entry: FindEntry {
                    path: root.join(folder).join("report.txt"),
                    kind: EntryKind::File,
                    metadata: EntryMetadata {
                        size: Some(1),
                        ..Default::default()
                    },
                    identity: TransferIdentity {
                        device: 1,
                        object: [index as u8; 16],
                    },
                    revision: FindRevision([0; 6]),
                },
            })
        })
        .collect();
    state.borrow_mut().apply(Event::Find(Box::new(FindEvent {
        session: request.session,
        generation: request.generation,
        update: FindUpdate::Batch(results.into()),
    })));
    midnight_ui::refresh(&main, &state.borrow());
    for (width, height) in [(860, 620), (640, 520)] {
        modeless::draw(&find, width, height, "find-path-tail");
        assert!(
            !shown(&*find, "...").is_empty(),
            "{width}x{height}: the result paths are shortened"
        );
        let adapter: Rc<dyn WindowAdapter> = find.clone();
        for folder in ["folder-1", "folder-2"] {
            let tail = format!("{}{folder}", std::path::MAIN_SEPARATOR);
            let item = shown(&*find, &tail).pop().unwrap_or_else(|| {
                panic!("{width}x{height}: the shortened path of {folder} keeps {tail}")
            });
            let rect = item.geometry();
            let natural = item
                .borrow()
                .as_ref()
                .layout_info(Orientation::Horizontal, rect.height(), &adapter, &item)
                .preferred;
            assert!(
                rect.width() + 0.5 >= natural,
                "{width}x{height}: {tail} is cut"
            );
        }
    }
    main.invoke_close_find_windows();
    main.hide().unwrap();
    state.borrow_mut().retire_all();
    let retired = state.borrow_mut().take_retired();
    std::thread::spawn(move || drop(retired)).join().unwrap();
}
