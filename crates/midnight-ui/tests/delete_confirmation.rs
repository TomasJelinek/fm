//! Delete confirmation renders a bounded summary of the worker's captured metadata.
use crate::support::modeless;
#[path = "support/owned_windows.rs"]
mod owned_windows;
use crate::support::ui;

use i_slint_core::{
    item_tree::ItemRc,
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{OperationPresentation, Presenter, install, refresh};
use slint::platform::{Key, WindowAdapter, software_renderer::MinimalSoftwareWindow};
use slint::{ComponentHandle, Model};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};
use ui::TestUi;

#[test]
fn delete_confirmation_shows_bounded_localized_metadata_and_preserves_safety() {
    let (ui, windows) = owned_windows::with_buffer(
        slint::platform::software_renderer::RepaintBufferType::NewBuffer,
    );
    let path = PathBuf::from(std::path::MAIN_SEPARATOR_STR).join("delete-fixture");
    let state = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sink = commands.clone();
    install(
        &ui.window,
        state.clone(),
        Rc::new(move |command| {
            sink.borrow_mut().push(command);
            true
        }),
    );
    let navigation = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::Navigate {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(*navigation),
            _ => None,
        })
        .unwrap();
    let mut entries = vec![
        Entry::new(
            EntryId(1),
            path.join("žluťoučký 文档 🦀.txt"),
            EntryKind::File,
        ),
        Entry::new(EntryId(2), path.join("second.txt"), EntryKind::File),
    ];
    entries[0].metadata.size = Some(1_000);
    entries[0].metadata.localized_modified = Some(LocalizedDateTime {
        date: "6. listopadu 2026".into(),
        time: "9:05 PM".into(),
    });
    entries[1].metadata.size = Some(2_000);
    state.borrow_mut().apply(Event::ListingFinished {
        pane: PaneId::Left,
        navigation,
        index: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries: entries.clone().into(),
    });
    state
        .borrow_mut()
        .apply(Event::LocaleLoaded(Ok(NumberLocale {
            decimal_separator: ",".into(),
            group_separator: " ".into(),
            grouping: vec![3],
            repeat_last_group: true,
        })));
    refresh(&ui.window, &state.borrow());
    ui.window.invoke_row_click(0, 1, false, false);
    let sample: Vec<_> = entries
        .into_iter()
        .map(|entry| MutationItem {
            path: entry.path,
            kind: entry.kind,
            metadata: entry.metadata,
        })
        .collect();
    let mut preview = OperationPreview {
        id: OperationId(0),
        kind: MutationKind::Delete(DeleteMode::Trash),
        files: 2,
        folders: 0,
        file_bytes: Some(3_000),
        sample: sample.clone(),
    };
    let body = show(&ui, &windows, &state, &commands, preview.clone());
    // Several targets: the question counts them, the details list the first names.
    assert!(body.starts_with("Move 2 items to the Trash?\n\n"), "{body}");
    assert!(body.contains("\nItems: 2 files\n"), "{body}");
    assert!(
        body.contains("\nNames: žluťoučký 文档 🦀.txt\n  second.txt\n"),
        "{body}"
    );
    assert!(body.contains("\nSize: 2,9 KiB (3 000 bytes)\n"), "{body}");
    assert!(!body.contains("0 folders"));
    assert_eq!(
        ui.window.get_operation_warning(),
        "If the Trash is unavailable, permanent deletion requires a separate confirmation."
    );
    assert!(!ui.window.get_operation_dangerous());
    snapshot(&windows, "delete-multiple-files");

    preview.files = 1;
    preview.file_bytes = Some(1_000);
    preview.sample = vec![sample[0].clone()];
    let body = show(&ui, &windows, &state, &commands, preview.clone());
    // One target: the question names it, first.
    assert!(
        body.starts_with("Move “žluťoučký 文档 🦀.txt” to the Trash?\n\n"),
        "{body}"
    );
    assert!(body.contains("Name: žluťoučký 文档 🦀.txt"), "{body}");
    assert!(body.contains("Type: File"), "{body}");
    assert!(body.contains("Size: 1 000 bytes"), "{body}");
    assert!(
        body.contains("Modified: 6. listopadu 2026 9:05 PM"),
        "{body}"
    );
    assert!(
        body.contains(&format!("Location: {}", path.display())),
        "{body}"
    );
    assert!(!body.contains(sample[0].path.to_string_lossy().as_ref()));
    assert_eq!(
        ui.window.get_operation_warning(),
        "If the Trash is unavailable, permanent deletion requires a separate confirmation."
    );
    snapshot(&windows, "delete-single-file");

    preview.kind = MutationKind::Delete(DeleteMode::Permanent);
    preview.files = 2;
    preview.folders = 3;
    preview.file_bytes = Some(3_000);
    preview.sample = sample.clone();
    let body = show(&ui, &windows, &state, &commands, preview.clone());
    assert!(
        body.starts_with("Permanently delete 5 items?\n\n"),
        "{body}"
    );
    assert!(
        body.contains("Items: 2 files and 3 folders, including everything in the folders"),
        "{body}"
    );
    assert!(
        body.contains("Size: 2,9 KiB (3 000 bytes) in the files; folder contents not counted"),
        "{body}"
    );
    // The warning line says it; the question does not repeat it.
    assert_eq!(
        ui.window.get_operation_warning(),
        "Permanent deletion cannot be undone."
    );
    assert!(!body.contains("cannot be undone"), "{body}");
    assert!(ui.window.get_operation_dangerous());
    snapshot(&windows, "delete-mixed-permanent");

    preview.files = 0;
    preview.folders = 1;
    preview.file_bytes = Some(0);
    preview.sample = vec![sample[0].clone()];
    preview.sample[0].path = path.join("složka 文档 🦀");
    preview.sample[0].kind = EntryKind::Directory;
    preview.sample[0].metadata.size = Some(999_999);
    let body = show(&ui, &windows, &state, &commands, preview.clone());
    assert!(
        body.starts_with("Permanently delete the folder “složka 文档 🦀” and everything in it?"),
        "{body}"
    );
    assert!(body.contains("Type: Folder\n"), "{body}");
    assert!(
        !body.contains("bytes"),
        "an unmeasured folder has no size: {body}"
    );
    assert!(body.contains("Name: složka 文档 🦀"), "{body}");
    assert!(
        body.contains("Modified: 6. listopadu 2026 9:05 PM"),
        "{body}"
    );
    snapshot(&windows, "delete-single-folder");

    // A directory link counts with the folders, yet it is named as a link.
    preview.sample[0].kind = EntryKind::DirectoryLink;
    let body = show(&ui, &windows, &state, &commands, preview.clone());
    assert!(
        body.starts_with("Permanently delete the link “složka 文档 🦀”?\n"),
        "{body}"
    );
    assert!(
        body.contains("Type: Folder link (the folder it points to is kept)"),
        "{body}"
    );
    preview.sample[0].kind = EntryKind::Directory;

    preview.folders = 2;
    let body = show(&ui, &windows, &state, &commands, preview.clone());
    assert!(body.starts_with("Permanently delete 2 items?"), "{body}");
    assert!(body.contains("Items: 2 folders"), "{body}");
    assert!(
        body.contains("Names: složka 文档 🦀\n  and 1 more"),
        "{body}"
    );

    preview.files = u64::MAX;
    preview.folders = u64::MAX;
    preview.file_bytes = None;
    preview.sample.clear();
    let body = show(&ui, &windows, &state, &commands, preview.clone());
    assert!(body.contains("18 446 744 073 709 551 615 files"), "{body}");
    assert!(
        body.contains("18 446 744 073 709 551 615 folders"),
        "{body}"
    );
    assert!(body.contains("Size: Unavailable"), "{body}");
    // The total overflows: the question still asks about every item.
    assert!(
        body.starts_with("Permanently delete these items?"),
        "{body}"
    );

    preview.files = 1;
    preview.folders = 0;
    preview.sample = vec![sample[0].clone()];
    preview.sample[0].metadata = EntryMetadata::default();
    let body = show(&ui, &windows, &state, &commands, preview.clone());
    assert!(body.contains("Size: Unavailable"), "{body}");
    assert!(body.contains("Modified: Unavailable"), "{body}");
    assert!(!body.contains("0 bytes"));
    assert!(
        body.starts_with("Permanently delete “žluťoučký 文档 🦀.txt”?"),
        "{body}"
    );

    preview.sample[0].metadata.modified = Some(std::time::UNIX_EPOCH);
    let body = show(&ui, &windows, &state, &commands, preview);
    assert!(body.contains("Modified: 1970-01-01 00:00:00 UTC"), "{body}");
}

fn show(
    ui: &TestUi,
    windows: &owned_windows::Windows,
    state: &Rc<RefCell<Presenter>>,
    commands: &Rc<RefCell<Vec<Command>>>,
    mut preview: OperationPreview,
) -> String {
    if !ui.window.get_operation_token().is_empty() {
        let previous = OperationId(ui.window.get_operation_token().parse().unwrap());
        commands.borrow_mut().clear();
        owned_windows::child_key(windows, Key::Escape);
        assert!(
            matches!(commands.borrow().as_slice(), [Command::CancelOperation(id)] if *id == previous)
        );
        assert!(ui.window.get_operation_token().is_empty());
        slint::platform::update_timers_and_animations();
        assert!(!ui.window.get_embedded_modal_open());
        assert!(
            WindowInner::from_pub(ui.window.window())
                .active_popups()
                .iter()
                .all(|popup| matches!(popup.window_kind, WindowKind::ToolTip)),
            "Escape must retire the actual confirmation popup"
        );
    }
    commands.borrow_mut().clear();
    ui.window.invoke_mutation(
        0,
        if preview.kind == MutationKind::Delete(DeleteMode::Trash) {
            1
        } else {
            2
        },
    );
    let id = commands
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::PrepareOperation(request) => Some(request.id),
            _ => None,
        })
        .unwrap();
    preview.id = id;
    state
        .borrow_mut()
        .apply(Event::Operation(OperationEvent::Preview(Arc::new(preview))));
    refresh(&ui.window, &state.borrow());
    let (main, popup) = confirmation(windows);
    modeless::draw(&main, 540, 360, "summary-layout");
    assert!(ui.window.get_embedded_modal_open());
    assert!(!ui.window.get_dialog_input_blocked());
    // The question, the warning and every detail value are drawn; a folder's path as the
    // path bar's components, its last one always.
    let form = ui.window.global::<OperationPresentation>();
    let body = ui.window.get_operation_body();
    assert!(body.starts_with(form.get_question().as_str()), "{body}");
    for text in [form.get_question(), ui.window.get_operation_warning()] {
        assert!(has_text(popup.clone(), &text), "{text} is drawn");
    }
    for row in form.get_details().iter() {
        assert!(
            has_text(popup.clone(), &row.label),
            "{} is drawn",
            row.label
        );
        if row.identity == "path" {
            let parts: Vec<_> = form.get_path_parts().iter().collect();
            let joined: String = parts.iter().map(|part| part.label.as_str()).collect();
            assert_eq!(
                joined,
                row.value.as_str(),
                "the components join into the path"
            );
            let last = format!("\u{2066}{}\u{2069}", parts.last().unwrap().label);
            assert!(has_text(popup.clone(), &last), "{last} is drawn");
        } else {
            assert!(
                has_text(popup.clone(), &row.value),
                "{} is drawn",
                row.value
            );
        }
        assert!(
            body.contains(&format!(
                "{}: {}",
                row.label,
                row.value.replace('\n', "\n  ")
            )),
            "{body}"
        );
    }
    assert!(
        commands.borrow().iter().all(|command| matches!(
            command,
            Command::PrepareOperation(_) | Command::CancelOperation(_)
        )),
        "opening confirmation must not start folder scans or confirm deletion"
    );
    ui.window.get_operation_body().to_string()
}

fn confirmation(windows: &owned_windows::Windows) -> (Rc<MinimalSoftwareWindow>, ItemRc) {
    slint::platform::update_timers_and_animations();
    assert_eq!(
        windows.borrow().len(),
        1,
        "delete confirmation must reuse the main adapter"
    );
    let main = windows.borrow()[0].clone();
    assert!(main.window().is_visible());
    let roots: Vec<_> = WindowInner::from_pub(main.window())
        .active_popups()
        .iter()
        .filter(|popup| !matches!(popup.window_kind, WindowKind::ToolTip))
        .map(|popup| ItemRc::new_root(popup.component.clone()))
        .collect();
    assert_eq!(
        roots.len(),
        1,
        "exactly one confirmation popup must be active"
    );
    (main, roots.into_iter().next().unwrap())
}
fn snapshot(windows: &owned_windows::Windows, label: &str) {
    let (main, _) = confirmation(windows);
    modeless::draw(&main, 540, 360, label);
}
fn has_text(item: ItemRc, value: &str) -> bool {
    use i_slint_core::items::{ComplexText, SimpleText};
    let text = item
        .downcast::<SimpleText>()
        .map(|text| text.as_pin_ref().text())
        .or_else(|| {
            item.downcast::<ComplexText>()
                .map(|text| text.as_pin_ref().text())
        });
    if item.is_visible() && text.is_some_and(|text| text == value) {
        return true;
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if has_text(item, value) {
            return true;
        }
    }
    false
}
