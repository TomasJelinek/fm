use super::*;
use std::cell::RefCell;

#[derive(Debug)]
struct Source {
    request: FindValidationRequest,
    entry: FindEntry,
}
impl CapturedFindFile for Source {
    fn request(&self) -> FindValidationRequest {
        self.request
    }
    fn entry(&self) -> &FindEntry {
        &self.entry
    }
    fn validate(&self, _: &Cancellation) -> Result<(), FsError> {
        panic!("UI must never execute provider validation")
    }
}
fn fixture(kind: EntryKind) -> (Presenter, FindSessionId, FindEntry) {
    let root = std::env::temp_dir();
    let mut p = Presenter::new(root.clone(), root.clone());
    let id = p.create_find(PaneId::Left).unwrap();
    p.start_find(id, FindForm::default(), &|_| true);
    let entry = FindEntry {
        path: root.join("raw result 文档"),
        kind,
        metadata: EntryMetadata::default(),
        identity: TransferIdentity {
            device: 1,
            object: [1; 16],
        },
        revision: FindRevision([1; 6]),
    };
    p.apply_find(FindEvent {
        session: id,
        generation: FindGeneration(1),
        update: FindUpdate::Batch(
            vec![Arc::new(FindResult {
                id: FindResultId(1),
                entry: entry.clone(),
                duplicate_group: None,
                duplicate_group_size: 0,
            })]
            .into(),
        ),
    });
    (p, id, entry)
}
fn capture_request(
    p: &mut Presenter,
    id: FindSessionId,
    action: FileAction,
) -> FindValidationRequest {
    let commands = RefCell::new(Vec::new());
    assert!(p.find_file_action(id, "1:1", action, &|c| {
        commands.borrow_mut().push(c);
        true
    }));
    let Command::CaptureFindFile(request) = commands.borrow_mut().remove(0) else {
        panic!("actions require a separate capability capture, never a navigation receipt")
    };
    request
}
#[test]
fn file_actions_capture_ids_then_pass_capability_and_frozen_editor_to_backend() {
    let (mut p, id, entry) = fixture(EntryKind::File);
    let editor = p.preferences.values.editor.clone();
    let request = capture_request(&mut p, id, FileAction::Edit);
    p.preferences.values.editor.arguments = vec!["changed settings".into()];
    let commands = RefCell::new(Vec::new());
    p.find_file_captured(
        FindFileCaptureResult {
            request,
            result: Ok(Arc::new(Source {
                request,
                entry: entry.clone(),
            })),
        },
        &|c| {
            commands.borrow_mut().push(c);
            true
        },
    );
    let Command::ExecuteFindFile {
        source,
        action: FindFileOperation::Edit(actual),
        cancel,
        ..
    } = commands.borrow_mut().remove(0)
    else {
        panic!()
    };
    assert_eq!(source.entry().path, entry.path);
    assert_eq!(actual, editor);
    assert!(!cancel.is_cancelled());
    p.close_find(id, &|_| true);
    assert!(cancel.is_cancelled());
}
#[test]
fn stale_or_substituted_capture_never_executes_and_queue_rejection_preserves_retry() {
    let (mut p, id, mut entry) = fixture(EntryKind::File);
    assert!(!p.find_file_action(id, "1:1", FileAction::Open, &|_| false));
    let request = capture_request(&mut p, id, FileAction::Open);
    assert!(!p.find_file_action(id, "1:1", FileAction::Edit, &|_| panic!()));
    entry.identity.object = [9; 16];
    p.find_file_captured(
        FindFileCaptureResult {
            request,
            result: Ok(Arc::new(Source {
                request,
                entry: entry.clone(),
            })),
        },
        &|_| panic!("substituted capture must not execute"),
    );
    assert!(p.finds.sessions[&id].error.contains("changed"));
    let request = capture_request(&mut p, id, FileAction::Open);
    p.start_find(id, FindForm::default(), &|_| true);
    p.find_file_captured(
        FindFileCaptureResult {
            request,
            result: Ok(Arc::new(Source { request, entry })),
        },
        &|_| panic!("stale capture must not execute"),
    );
    assert!(p.take_find_viewer().is_none());
}
#[test]
fn folder_actions_only_request_navigation_validation() {
    let (mut p, id, _) = fixture(EntryKind::Directory);
    assert!(p.find_file_action(id, "1:1", FileAction::Open, &|command| {
        assert!(matches!(command, Command::ValidateFindResult(_)));
        true
    }));
}
#[test]
fn find_uses_current_shortcuts_and_never_routes_other_panel_commands() {
    let (mut p, _, _) = fixture(EntryKind::File);
    for (key, action) in [
        ("F3", FileAction::View(false)),
        ("Alt+F3", FileAction::View(true)),
        ("F4", FileAction::Edit),
    ] {
        assert_eq!(
            p.find_action_key(&ShortcutChord::parse(key).unwrap()),
            Some(action)
        );
    }
    assert!(p.change_shortcuts("files.view", vec!["Ctrl+Alt+Y".into()], &|_| true));
    assert_eq!(
        p.find_action_key(&ShortcutChord::parse("F3").unwrap()),
        None
    );
    assert_eq!(
        p.find_action_key(&ShortcutChord::parse("Ctrl+Alt+Y").unwrap()),
        Some(FileAction::View(false))
    );
    assert_eq!(
        p.find_action_key(&ShortcutChord::parse("Delete").unwrap()),
        None
    );
}

#[test]
fn internal_viewer_launch_contains_only_retained_capability_authority() {
    let (mut p, id, entry) = fixture(EntryKind::File);
    p.viewer_plugins.startup_disabled = Some(Arc::from([]));
    let request = capture_request(&mut p, id, FileAction::View(true));
    p.find_file_captured(
        FindFileCaptureResult {
            request,
            result: Ok(Arc::new(Source {
                request,
                entry: entry.clone(),
            })),
        },
        &|_| panic!(),
    );
    let launch = p.take_find_viewer().unwrap();
    assert!(launch.basic);
    assert!(launch.focused.as_os_str().is_empty());
    assert!(launch.entries.is_empty());
    assert!(launch.find.unwrap().entry().matches_capture(&entry));
    assert!(p.take_find_viewer().is_none());
}
