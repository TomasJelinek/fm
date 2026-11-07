//! Delete confirmation renders a bounded summary of the worker's captured metadata.
#[path = "support/render.rs"]
mod render;
#[path = "support/ui.rs"]
mod ui;

use midnight_core::*;
use midnight_ui::{Presenter, install, refresh};
use std::{cell::RefCell, path::PathBuf, rc::Rc, sync::Arc};
use ui::TestUi;

#[test]
fn delete_confirmation_shows_bounded_localized_metadata_and_preserves_safety() {
    let mut ui = TestUi::new();
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
    let body = show(&mut ui, &state, &commands, preview.clone());
    assert!(
        !body.contains("žluťoučký"),
        "multiple targets must not list names: {body}"
    );
    assert!(!body.contains("second.txt"));
    assert!(body.contains("2 files"));
    assert!(body.contains("Files size: 3 000 bytes"), "{body}");
    assert!(!body.contains("0 folders"));
    assert!(body.contains("system Trash"));
    assert!(!ui.window.get_operation_dangerous());
    render::snapshot(&ui, "delete-multiple-files");

    preview.files = 1;
    preview.file_bytes = Some(1_000);
    preview.sample = vec![sample[0].clone()];
    let body = show(&mut ui, &state, &commands, preview.clone());
    assert!(body.contains("Name: žluťoučký 文档 🦀.txt"), "{body}");
    assert!(body.contains("Size: 1 000 bytes"), "{body}");
    assert!(
        body.contains("Modified: 6. listopadu 2026 9:05 PM"),
        "{body}"
    );
    assert!(!body.contains(sample[0].path.to_string_lossy().as_ref()));
    render::snapshot(&ui, "delete-single-file");

    preview.kind = MutationKind::Delete(DeleteMode::Permanent);
    preview.files = 2;
    preview.folders = 3;
    preview.file_bytes = Some(3_000);
    preview.sample = sample.clone();
    let body = show(&mut ui, &state, &commands, preview.clone());
    assert!(body.contains("2 files and 3 folders"), "{body}");
    assert!(body.contains("Files size: 3 000 bytes"), "{body}");
    assert!(body.contains("cannot be undone"));
    assert!(!body.contains("žluťoučký"));
    assert!(ui.window.get_operation_dangerous());
    render::snapshot(&ui, "delete-mixed-permanent");

    preview.files = 0;
    preview.folders = 1;
    preview.file_bytes = Some(0);
    preview.sample[0].kind = EntryKind::Directory;
    preview.sample[0].metadata.size = Some(999_999);
    let body = show(&mut ui, &state, &commands, preview.clone());
    assert!(body.contains("1 folder"), "{body}");
    assert!(!body.contains("0 files"));
    assert!(!body.contains("bytes"));
    assert!(!body.contains("žluťoučký"));

    preview.files = u64::MAX;
    preview.folders = u64::MAX;
    preview.file_bytes = None;
    preview.sample.clear();
    let body = show(&mut ui, &state, &commands, preview.clone());
    assert!(body.contains("18 446 744 073 709 551 615 files"), "{body}");
    assert!(
        body.contains("18 446 744 073 709 551 615 folders"),
        "{body}"
    );
    assert!(body.contains("Files size: Unavailable"), "{body}");

    preview.files = 1;
    preview.folders = 0;
    preview.sample = vec![sample[0].clone()];
    preview.sample[0].metadata = EntryMetadata::default();
    let body = show(&mut ui, &state, &commands, preview.clone());
    assert!(body.contains("Size: Unavailable"), "{body}");
    assert!(body.contains("Modified: Unavailable"), "{body}");
    assert!(!body.contains("0 bytes"));

    preview.sample[0].metadata.modified = Some(std::time::UNIX_EPOCH);
    let body = show(&mut ui, &state, &commands, preview);
    assert!(body.contains("Modified: 1970-01-01 00:00:00 UTC"), "{body}");
}

fn show(
    ui: &mut TestUi,
    state: &Rc<RefCell<Presenter>>,
    commands: &Rc<RefCell<Vec<Command>>>,
    mut preview: OperationPreview,
) -> String {
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
    ui.draw();
    assert!(
        commands.borrow().iter().all(|command| matches!(
            command,
            Command::PrepareOperation(_) | Command::CancelOperation(_)
        )),
        "opening confirmation must not start folder scans or confirm deletion"
    );
    ui.window.get_operation_body().to_string()
}
