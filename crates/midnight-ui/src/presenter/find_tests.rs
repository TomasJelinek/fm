use super::*;
use crate::presenter::find::FindForm;

fn presenter() -> Presenter {
    Presenter::new(std::env::temp_dir(), std::env::temp_dir())
}

#[test]
fn admission_and_close_preserve_state_when_queue_is_full() {
    let mut p = presenter();
    let id = p.create_find(PaneId::Left).unwrap();
    assert!(!p.start_find(id, FindForm::default(), &|_| false));
    assert_eq!(p.finds.sessions[&id].generation, FindGeneration(0));
    assert!(!p.finds.sessions[&id].running);
    assert!(p.start_find(id, FindForm::default(), &|_| true));
    assert_eq!(p.finds.sessions[&id].generation, FindGeneration(1));
    assert!(!p.close_find(id, &|_| false));
    assert!(p.finds.sessions.contains_key(&id));
    assert!(p.close_find(id, &|_| true));
    assert!(!p.finds.sessions.contains_key(&id));
    assert!(!p.take_retired().is_empty());
    assert!(p.create_find(PaneId::Left).unwrap().0 > id.0);
}

#[test]
fn four_independent_windows_keep_captured_raw_roots() {
    let mut p = presenter();
    let expected = p.panes[0].path.clone();
    let first = p.create_find(PaneId::Left).unwrap();
    p.panes[0].path.push("changed-panel");
    for _ in 0..3 {
        p.create_find(PaneId::Right).unwrap();
    }
    assert!(p.create_find(PaneId::Left).is_none());
    assert_eq!(p.finds.sessions[&first].roots, [expected]);
    assert!(p.close_find(first, &|_| panic!(
        "unsubmitted sessions need no backend close"
    )));
}

#[test]
fn superseded_generations_and_closed_windows_cannot_receive_results() {
    let mut p = presenter();
    let id = p.create_find(PaneId::Left).unwrap();
    p.start_find(id, FindForm::default(), &|_| true);
    p.start_find(id, FindForm::default(), &|_| true);
    p.apply(Event::Find(Box::new(FindEvent {
        session: id,
        generation: FindGeneration(1),
        update: FindUpdate::Finished(FindSummary {
            completion: FindCompletion::Completed,
            progress: FindProgress::default(),
            reached_budgets: vec![],
            diagnostics: vec![],
        }),
    })));
    assert!(p.finds.sessions[&id].running);
    assert!(p.finds.sessions[&id].summary.is_none());
    p.close_find(id, &|_| true);
    p.apply(Event::Find(Box::new(FindEvent {
        session: id,
        generation: FindGeneration(2),
        update: FindUpdate::Started,
    })));
    assert!(p.finds.sessions.is_empty());
}

#[test]
fn result_validation_never_executes_and_cannot_retarget_a_new_search() {
    let mut p = presenter();
    let id = p.create_find(PaneId::Right).unwrap();
    p.start_find(id, FindForm::default(), &|_| true);
    let path = p.panes[1].path.join("Český soubor.txt");
    let entry = FindEntry {
        path: path.clone(),
        kind: EntryKind::File,
        metadata: EntryMetadata::default(),
        identity: TransferIdentity {
            device: 1,
            object: [1; 16],
        },
        revision: FindRevision([0; 6]),
    };
    p.apply(Event::Find(Box::new(FindEvent {
        session: id,
        generation: FindGeneration(1),
        update: FindUpdate::Batch(
            vec![Arc::new(FindResult {
                id: FindResultId(7),
                duplicate_group: None,
                duplicate_group_size: 0,
                entry: entry.clone(),
            })]
            .into(),
        ),
    })));
    let commands = std::cell::RefCell::new(Vec::new());
    let send = |c| {
        commands.borrow_mut().push(c);
        true
    };
    assert!(p.reveal_find(id, "1:7", &send));
    let Command::ValidateFindResult(request) = commands.borrow_mut().remove(0) else {
        panic!()
    };
    p.apply_with_sender(
        Event::FindResultValidated(Box::new(FindValidationResult {
            request,
            result: Ok(Box::new(entry.clone())),
        })),
        &send,
    );
    assert!(matches!(
        commands.borrow().last(),
        Some(Command::Navigate {
            pane: PaneId::Right,
            ..
        })
    ));
    assert_eq!(
        p.panes[1].pending.as_ref().unwrap().wanted_cursor,
        Some(path)
    );
    commands.borrow_mut().clear();
    p.start_find(id, FindForm::default(), &|_| true);
    p.apply(Event::Find(Box::new(FindEvent {
        session: id,
        generation: FindGeneration(2),
        update: FindUpdate::Batch(
            vec![Arc::new(FindResult {
                id: FindResultId(7),
                duplicate_group: None,
                duplicate_group_size: 0,
                entry: entry.clone(),
            })]
            .into(),
        ),
    })));
    assert!(
        !p.reveal_find(id, "1:7", &send),
        "an old rendered row must not target the replacement generation"
    );
    p.apply_with_sender(
        Event::FindResultValidated(Box::new(FindValidationResult {
            request,
            result: Ok(Box::new(entry)),
        })),
        &send,
    );
    assert!(commands.borrow().is_empty());
}

#[test]
fn invalid_root_and_query_edits_never_start_a_search_or_erase_captured_roots() {
    let mut p = presenter();
    let id = p.create_find(PaneId::Left).unwrap();
    let roots = p.finds.sessions[&id].roots.clone();
    p.add_find_root(id, "relative-folder");
    assert_eq!(p.finds.sessions[&id].roots, roots);
    assert!(!p.finds.sessions[&id].error.is_empty());
    let invalid = FindForm {
        content: Some(FindContentQuery {
            text: String::new(),
            case_sensitive: false,
            whole_words: false,
        }),
        ..Default::default()
    };
    assert!(!p.start_find(id, invalid, &|_| panic!(
        "invalid form must not enter backend queue"
    )));
    assert_eq!(p.finds.sessions[&id].generation, FindGeneration(0));
    assert!(p.start_find(id, FindForm::default(), &|_| true));
    p.remove_find_root(id, 0);
    assert_eq!(
        p.finds.sessions[&id].roots, roots,
        "running request roots remain immutable"
    );
    p.stop_find(id, &|_| false);
    assert!(!p.finds.sessions[&id].stopping);
    assert!(p.finds.sessions[&id].running);
}

#[test]
fn sampled_directory_never_revives_stopped_terminal_or_replaced_searches() {
    let mut p = presenter();
    let id = p.create_find(PaneId::Left).unwrap();
    p.start_find(id, FindForm::default(), &|_| true);
    let path = Arc::new(std::env::temp_dir().join("current 文档"));
    let event = |generation| {
        Event::Find(Box::new(FindEvent {
            session: id,
            generation: FindGeneration(generation),
            update: FindUpdate::CurrentDirectory(path.clone()),
        }))
    };
    p.apply(event(1));
    assert_eq!(
        p.finds.sessions[&id].current_directory.as_ref(),
        Some(&path)
    );
    p.stop_find(id, &|_| true);
    p.apply(event(1));
    assert!(p.finds.sessions[&id].current_directory.is_none());
    p.apply(Event::Find(Box::new(FindEvent {
        session: id,
        generation: FindGeneration(1),
        update: FindUpdate::Finished(FindSummary {
            completion: FindCompletion::Cancelled,
            progress: FindProgress::default(),
            reached_budgets: vec![],
            diagnostics: vec![],
        }),
    })));
    p.apply(event(1));
    assert!(p.finds.sessions[&id].current_directory.is_none());
    p.start_find(id, FindForm::default(), &|_| true);
    p.apply(event(1));
    assert!(p.finds.sessions[&id].current_directory.is_none());
    p.apply(event(2));
    assert_eq!(
        p.finds.sessions[&id].current_directory.as_ref(),
        Some(&path)
    );
}

#[test]
fn sort_queue_refusal_and_stale_revisions_preserve_the_last_accepted_order() {
    let mut p = presenter();
    let id = p.create_find(PaneId::Left).unwrap();
    p.start_find(id, FindForm::default(), &|_| true);
    p.sort_find(id, FindSortColumn::Size, &|_| false);
    assert_eq!(p.finds.sessions[&id].sort, FindSortSpec::default());
    p.sort_find(
        id,
        FindSortColumn::Size,
        &|command| matches!(command,Command::SortFind(request) if request.revision==1),
    );
    assert_eq!(p.finds.sessions[&id].sort.column, FindSortColumn::Size);
    let apply = |revision| {
        Event::Find(Box::new(FindEvent {
            session: id,
            generation: FindGeneration(1),
            update: FindUpdate::Sorted(Arc::new(FindOrder {
                revision,
                sort: FindSortSpec {
                    column: FindSortColumn::Size,
                    descending: false,
                },
                ids: vec![].into(),
                positions: vec![].into(),
            })),
        }))
    };
    p.apply(apply(0));
    assert!(p.finds.sessions[&id].order.is_none());
    p.apply(apply(1));
    assert_eq!(p.finds.sessions[&id].order.as_ref().unwrap().revision, 1);
}

#[test]
fn duplicate_group_larger_than_a_normal_batch_is_retained_atomically_on_stop() {
    let mut p = presenter();
    let session = p.create_find(PaneId::Left).unwrap();
    p.start_find(session, FindForm::default(), &|_| true);
    let group: Vec<_> = (1..=100)
        .map(|id| {
            Arc::new(FindResult {
                id: FindResultId(id),
                duplicate_group: Some(1),
                duplicate_group_size: 100,
                entry: FindEntry {
                    path: std::env::temp_dir().join(format!("duplicate-{id}.txt")),
                    kind: EntryKind::File,
                    metadata: EntryMetadata::default(),
                    identity: TransferIdentity {
                        device: 1,
                        object: [id as u8; 16],
                    },
                    revision: FindRevision([0; 6]),
                },
            })
        })
        .collect();
    p.apply(Event::Find(Box::new(FindEvent {
        session,
        generation: FindGeneration(1),
        update: FindUpdate::Batch(group.into()),
    })));
    p.stop_find(session, &|_| true);
    p.apply(Event::Find(Box::new(FindEvent {
        session,
        generation: FindGeneration(1),
        update: FindUpdate::Finished(FindSummary {
            completion: FindCompletion::Cancelled,
            progress: FindProgress::default(),
            reached_budgets: vec![],
            diagnostics: vec![],
        }),
    })));
    let found = &p.finds.sessions[&session].results;
    assert_eq!(found.len(), 100);
    assert!(
        found
            .iter()
            .all(|row| row.duplicate_group == Some(1) && row.duplicate_group_size == 100)
    );
    assert!(!p.finds.sessions[&session].running);
}
