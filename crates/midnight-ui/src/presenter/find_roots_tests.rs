use super::*;
use crate::presenter::find::{FindForm, FindRoot};
use std::cell::RefCell;

fn presenter() -> Presenter {
    Presenter::new(std::env::temp_dir(), std::env::temp_dir())
}

fn started_roots(p: &mut Presenter, id: FindSessionId) -> Option<Vec<PathBuf>> {
    let roots = RefCell::new(None);
    p.start_find(id, FindForm::default(), &|command| {
        if let Command::StartFind(request) = command {
            *roots.borrow_mut() = Some(request.roots.clone());
        }
        true
    });
    roots.into_inner()
}

#[test]
fn only_ticked_folders_are_searched_and_unticking_all_never_starts_a_search() {
    let mut p = presenter();
    let id = p.create_find(PaneId::Left).unwrap();
    let first = p.finds.sessions[&id].roots[0].path.clone();
    let second = std::env::temp_dir().join("second 文档");
    assert_eq!(p.finds.sessions[&id].added_root, (0, 0));
    p.add_find_root(id, &second.to_string_lossy());
    assert_eq!(
        p.finds.sessions[&id].roots,
        [FindRoot::new(first.clone()), FindRoot::new(second.clone())]
    );
    // The window chooses the added folder and closes its Add entry.
    assert_eq!(p.finds.sessions[&id].added_root, (1, 1));
    p.set_find_root_searched(id, 0, false);
    assert_eq!(started_roots(&mut p, id), Some(vec![second.clone()]));
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
    p.set_find_root_searched(id, 1, false);
    assert!(!p.start_find(id, FindForm::default(), &|_| panic!(
        "a Find without a ticked folder must not reach the backend"
    )));
    assert_eq!(
        p.finds.sessions[&id].error,
        "Tick at least one search folder."
    );
    // Adding a listed folder again ticks it instead of listing it twice.
    p.add_find_root(id, &first.to_string_lossy());
    assert_eq!(p.finds.sessions[&id].roots.len(), 2);
    assert!(p.finds.sessions[&id].roots[0].searched);
    assert!(p.finds.sessions[&id].error.is_empty());
    assert_eq!(
        p.finds.sessions[&id].added_root,
        (2, 0),
        "the listed folder is chosen, so the entry closes too"
    );
    assert_eq!(started_roots(&mut p, id), Some(vec![first]));
    // A running search keeps its folder list.
    p.set_find_root_searched(id, 0, false);
    p.remove_find_root(id, 1);
    assert_eq!(p.finds.sessions[&id].roots.len(), 2);
    assert!(p.finds.sessions[&id].roots[0].searched);
}

#[test]
fn unticked_folders_still_count_toward_the_root_limit() {
    let mut p = presenter();
    let id = p.create_find(PaneId::Left).unwrap();
    for index in 1..MAX_FIND_ROOTS {
        p.add_find_root(
            id,
            &std::env::temp_dir()
                .join(format!("root-{index}"))
                .to_string_lossy(),
        );
        p.set_find_root_searched(id, index as i32, false);
    }
    assert_eq!(p.finds.sessions[&id].roots.len(), MAX_FIND_ROOTS);
    assert!(p.finds.sessions[&id].error.is_empty());
    let added = p.finds.sessions[&id].added_root;
    p.add_find_root(
        id,
        &std::env::temp_dir().join("one too many").to_string_lossy(),
    );
    assert_eq!(p.finds.sessions[&id].roots.len(), MAX_FIND_ROOTS);
    assert!(!p.finds.sessions[&id].error.is_empty());
    assert_eq!(
        p.finds.sessions[&id].added_root, added,
        "a refused folder keeps the entry open"
    );
}

#[test]
fn browse_adds_the_exact_chosen_folder_only_to_its_own_open_window() {
    let mut p = presenter();
    let id = p.create_find(PaneId::Left).unwrap();
    let other = p.create_find(PaneId::Right).unwrap();
    let commands = RefCell::new(Vec::new());
    let send = |command| {
        commands.borrow_mut().push(command);
        true
    };
    p.request_find_root_picker(id, "typed", PathPickerOwner::X11(7), &send);
    let request = match commands.borrow_mut().pop() {
        Some(Command::PickPath { request, input }) => {
            assert_eq!(input.kind, PathPickerKind::Directory);
            assert_eq!(input.owner, PathPickerOwner::X11(7));
            assert_eq!(
                input.initial_directory.as_ref(),
                Some(&p.finds.sessions[&id].roots[0].path.join("typed"))
            );
            request
        }
        _ => panic!("Browse must open the native folder chooser"),
    };
    // A second Browse while the chooser is open is ignored.
    p.request_find_root_picker(id, "", PathPickerOwner::None, &send);
    assert!(commands.borrow().is_empty());
    let chosen = std::env::temp_dir().join("chosen 日本語");
    p.apply(Event::PathPicked {
        request,
        result: Ok(Some(chosen.clone())),
    });
    assert_eq!(
        p.finds.sessions[&id].roots.last(),
        Some(&FindRoot::new(chosen.clone()))
    );
    assert_eq!(p.finds.sessions[&id].added_root, (1, 1));
    assert_eq!(p.finds.sessions[&other].roots.len(), 1);
    // A repeated answer for the finished request changes nothing.
    p.apply(Event::PathPicked {
        request,
        result: Ok(Some(std::env::temp_dir().join("late"))),
    });
    assert_eq!(p.finds.sessions[&id].roots.len(), 2);
    // A relative answer is refused with an error, not listed.
    p.request_find_root_picker(id, "", PathPickerOwner::None, &send);
    let Some(Command::PickPath { request, .. }) = commands.borrow_mut().pop() else {
        panic!("Browse must open the native folder chooser");
    };
    p.apply(Event::PathPicked {
        request,
        result: Ok(Some(PathBuf::from("relative"))),
    });
    assert_eq!(p.finds.sessions[&id].roots.len(), 2);
    assert!(!p.finds.sessions[&id].error.is_empty());
}

#[test]
fn closing_a_window_cancels_its_folder_chooser_first() {
    let mut p = presenter();
    let id = p.create_find(PaneId::Left).unwrap();
    let commands = RefCell::new(Vec::new());
    p.request_find_root_picker(id, "", PathPickerOwner::None, &|command| {
        commands.borrow_mut().push(command);
        true
    });
    let Some(Command::PickPath { request, .. }) = commands.borrow_mut().pop() else {
        panic!("Browse must open the native folder chooser");
    };
    assert!(
        !p.close_find(id, &|_| false),
        "a full queue keeps the window"
    );
    assert!(p.owns_find_picker(request));
    assert!(p.close_find(id, &|command| {
        commands.borrow_mut().push(command);
        true
    }));
    assert!(matches!(
        commands.borrow().as_slice(),
        [Command::CancelPathPicker { request: cancelled }] if *cancelled == request
    ));
    assert!(!p.owns_find_picker(request));
    drop(p.take_retired());
    p.apply(Event::PathPicked {
        request,
        result: Ok(Some(std::env::temp_dir())),
    });
    assert!(
        !p.take_retired().is_empty(),
        "a late answer is only retired"
    );
}
