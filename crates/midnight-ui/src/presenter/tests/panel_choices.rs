//! Show hidden files and Select/Deselect by mask: commands, re-reads and the dialog.
use super::*;
use std::rc::Rc;

fn file(id: u64, path: PathBuf) -> Entry {
    Entry::new(EntryId(id), path, EntryKind::File)
}

/// Both panes list the same four rows; settings are loaded with defaults.
fn loaded() -> (Presenter, PathBuf) {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    p.apply_with_sender(Event::PreferencesLoaded(Ok(Box::default())), &|_| true);
    for pane in [PaneId::Left, PaneId::Right] {
        let navigation = begin(&mut p, pane, root.to_str().unwrap());
        finish(
            &mut p,
            pane,
            navigation,
            vec![
                file(1, root.join(".a")),
                file(2, root.join("b.txt")),
                file(3, root.join(".c")),
                file(4, root.join("d.md")),
            ],
        );
    }
    (p, root)
}

fn recorder() -> (Rc<RefCell<Vec<Command>>>, impl Fn(Command) -> bool) {
    let commands = Rc::new(RefCell::new(Vec::new()));
    let sink = commands.clone();
    (commands, move |command| {
        sink.borrow_mut().push(command);
        true
    })
}

fn position(commands: &[Command], wanted: impl Fn(&Command) -> bool) -> Option<usize> {
    commands.iter().position(wanted)
}

#[test]
fn toggle_tells_the_backend_first_then_rereads_both_panels_and_saves_the_choice() {
    let (mut p, _) = loaded();
    assert!(p.preferences.values.show_hidden_files);
    let (commands, send) = recorder();
    p.toggle_hidden_files(&send);
    let sent = commands.take();
    let told = position(&sent, |c| matches!(c, Command::ShowHiddenEntries(false)))
        .expect("the backend is told to leave hidden entries out");
    for pane in [PaneId::Left, PaneId::Right] {
        let reread = position(
            &sent,
            |c| matches!(c, Command::Refresh { pane: refreshed, .. } if *refreshed == pane),
        )
        .unwrap_or_else(|| panic!("{pane:?} is re-read in place: {sent:?}"));
        assert!(told < reread, "listings use the new choice");
    }
    assert!(sent.iter().any(|c| matches!(c,
        Command::SavePreferences { preferences, .. } if !preferences.show_hidden_files)));
    assert!(!p.preferences.values.show_hidden_files);
    assert_eq!(p.desktop.status, "Hidden files are not shown");

    // Showing them again while those re-reads are pending: each pane reads again
    // once its pending listing commits.
    p.toggle_hidden_files(&send);
    let again = commands.take();
    assert!(matches!(
        again.first(),
        Some(Command::ShowHiddenEntries(true))
    ));
    assert!(p.preferences.values.show_hidden_files);
    assert!(p.panes.iter().all(|pane| pane.loading && pane.refresh_hint));
    for command in sent {
        if let Command::Refresh {
            pane, navigation, ..
        } = command
        {
            let index = p.panes[pane.index()].index.clone();
            let entries = p.panes[pane.index()].entries.clone();
            p.apply_with_sender(
                Event::ListingFinished {
                    pane,
                    navigation,
                    entries,
                    index,
                },
                &send,
            );
        }
    }
    let rereads = commands
        .take()
        .into_iter()
        .filter(|c| matches!(c, Command::Refresh { .. }))
        .count();
    assert_eq!(rereads, 2, "both panels are read again with the new choice");
}

#[test]
fn a_refused_backend_choice_is_retried_before_any_reread() {
    let (mut p, _) = loaded();
    let refused = RefCell::new(Vec::new());
    p.toggle_hidden_files(&|command| {
        let accepted = !matches!(command, Command::ShowHiddenEntries(_));
        refused.borrow_mut().push(command);
        accepted
    });
    assert!(
        !refused
            .borrow()
            .iter()
            .any(|c| matches!(c, Command::Refresh { .. } | Command::Navigate { .. })),
        "no re-read before the backend knows the choice"
    );
    let (commands, send) = recorder();
    p.flush_selection(&send);
    let sent = commands.take();
    let told = position(&sent, |c| matches!(c, Command::ShowHiddenEntries(false)))
        .expect("the refused choice is sent again");
    let rereads: Vec<_> = sent
        .iter()
        .enumerate()
        .filter(|(_, c)| matches!(c, Command::Refresh { .. }))
        .map(|(position, _)| position)
        .collect();
    assert_eq!(rereads.len(), 2);
    assert!(rereads.iter().all(|reread| told < *reread));
    // Nothing more is owed once both panes were asked.
    p.flush_selection(&send);
    assert!(
        !commands
            .take()
            .iter()
            .any(|c| matches!(c, Command::ShowHiddenEntries(_) | Command::Refresh { .. }))
    );
}

#[test]
fn rereads_move_the_cursor_to_the_nearest_shown_row_and_rebase_marks_and_count_hidden() {
    let (mut p, root) = loaded();
    p.panes[0].cursor = Some(EntryId(3));
    set_selected(&mut p, PaneId::Left, &[EntryId(1), EntryId(4)]);
    let (commands, send) = recorder();
    p.toggle_hidden_files(&send);
    let navigation = commands
        .take()
        .into_iter()
        .find_map(|c| match c {
            Command::Refresh {
                pane: PaneId::Left,
                navigation,
                ..
            } => Some(navigation),
            _ => None,
        })
        .unwrap();
    let shown = vec![file(2, root.join("b.txt")), file(4, root.join("d.md"))];
    let index = Arc::new(DirectoryIndex {
        hidden: 2,
        ..DirectoryIndex::from_entries(&shown)
    });
    p.apply_with_sender(
        Event::ListingFinished {
            pane: PaneId::Left,
            navigation,
            entries: shown.into(),
            index: index.clone(),
        },
        &send,
    );
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(4)),
        "the cursor on a hidden row moves to the next shown row"
    );
    assert_eq!(p.panes[0].hidden_count(), 2);
    assert_eq!(p.panes[1].hidden_count(), 0);
    // The status line (the panel's details) says how many are left out.
    let hidden = |pane| {
        p.panel_details(pane)
            .into_iter()
            .filter(|field| field.label == "Hidden")
            .map(|field| field.value)
            .collect::<Vec<_>>()
    };
    assert_eq!(hidden(PaneId::Left), ["2 hidden"]);
    assert!(hidden(PaneId::Right).is_empty());
    let rebase = commands
        .take()
        .into_iter()
        .find_map(|c| match c {
            Command::Selection(request) if matches!(request.action, SelectionAction::Rebase) => {
                Some(request)
            }
            _ => None,
        })
        .expect("marks are rebased onto the shown rows");
    assert!(Arc::ptr_eq(rebase.view.as_ref().unwrap(), &index));
}

#[test]
fn saved_hidden_choice_lists_the_first_folders_again_before_showing_rows() {
    let root = test_root();
    let mut p = Presenter::new(root.clone(), root.clone());
    let (commands, send) = recorder();
    for pane in [PaneId::Left, PaneId::Right] {
        assert!(p.navigate(pane, root.clone(), None, &send));
    }
    let first: Vec<_> = commands
        .take()
        .into_iter()
        .filter_map(|c| match c {
            Command::Navigate { navigation, .. } => Some(navigation),
            _ => None,
        })
        .collect();
    assert_eq!(first.len(), 2);
    p.apply_with_sender(
        Event::PreferencesLoaded(Ok(Box::new(AcceptedPreferences {
            preferences: UserPreferences {
                show_hidden_files: false,
                ..Default::default()
            },
            comparators: FileComparatorSnapshot::default(),
        }))),
        &send,
    );
    let sent = commands.take();
    let told = position(&sent, |c| matches!(c, Command::ShowHiddenEntries(false))).unwrap();
    let again: Vec<_> = sent
        .iter()
        .enumerate()
        .filter_map(|(position, c)| match c {
            Command::Navigate { navigation, .. } => Some((position, *navigation)),
            _ => None,
        })
        .collect();
    assert_eq!(again.len(), 2, "{sent:?}");
    for (position, navigation) in again {
        assert!(told < position);
        assert!(!first.contains(&navigation));
    }
}

#[test]
fn options_preview_applies_at_once_and_cancel_restores_the_listing_choice() {
    let (mut p, _) = loaded();
    p.begin_options();
    let (commands, send) = recorder();
    p.change_show_hidden_files(false, &send);
    let sent = commands.take();
    assert!(
        sent.iter()
            .any(|c| matches!(c, Command::ShowHiddenEntries(false)))
    );
    assert!(
        !sent
            .iter()
            .any(|c| matches!(c, Command::SavePreferences { .. })),
        "Options previews until OK"
    );
    p.cancel_options(&send);
    assert!(p.preferences.values.show_hidden_files);
    let sent = commands.take();
    assert!(
        sent.iter()
            .any(|c| matches!(c, Command::ShowHiddenEntries(true)))
    );
    // The previewed re-reads are still pending: each pane reads again after them.
    assert!(p.panes.iter().all(|pane| pane.loading && pane.refresh_hint));
}

#[test]
fn an_adopted_tree_is_refreshed_once_without_reentering_the_reread() {
    let (mut p, root) = super::tree::fixture();
    p.apply_with_sender(
        Event::Tree(Box::new(TreeEvent::Published(super::tree::publication(
            root, 1, true, false,
        )))),
        &|_| true,
    );
    let (commands, send) = recorder();
    // The tree's maintenance refresh flushes background refreshes, which reach
    // the hidden-entries re-read again before it returns.
    p.toggle_hidden_files(&send);
    let sent = commands.take();
    let told = position(&sent, |c| matches!(c, Command::ShowHiddenEntries(false)))
        .expect("the backend is told first");
    let refreshes: Vec<usize> = sent
        .iter()
        .enumerate()
        .filter(|(_, c)| {
            matches!(c, Command::Tree(tree) if matches!(**tree, TreeCommand::RefreshLoaded(_)))
        })
        .map(|(index, _)| index)
        .collect();
    assert_eq!(refreshes.len(), 1, "one tree refresh: {sent:?}");
    assert!(told < refreshes[0]);
    p.toggle_hidden_files(&send);
    assert!(
        commands
            .take()
            .iter()
            .any(|c| matches!(c, Command::ShowHiddenEntries(true)))
    );
}

fn mask_request(commands: &RefCell<Vec<Command>>) -> (SelectionRequest, MaskSelection) {
    commands
        .borrow()
        .iter()
        .find_map(|c| match c {
            Command::Selection(request) => match &request.action {
                SelectionAction::Mask(mask) => Some((request.clone(), (**mask).clone())),
                _ => None,
            },
            _ => None,
        })
        .expect("a mask selection request")
}

/// The backend accepts the mask of the dialog's last OK (`Command::ValidateSelectionMask`).
fn accept_mask(
    p: &mut Presenter,
    commands: &RefCell<Vec<Command>>,
    send: &dyn Fn(Command) -> bool,
) {
    let request = commands
        .borrow()
        .iter()
        .rev()
        .find_map(|c| match c {
            Command::ValidateSelectionMask(request) => Some(request.clone()),
            _ => None,
        })
        .expect("OK has the backend check the mask");
    p.apply_with_sender(
        Event::SelectionMaskValidated {
            request,
            result: Ok(()),
        },
        send,
    );
}

#[test]
fn select_by_mask_remembers_its_choices_and_sends_the_shown_view() {
    let (mut p, _) = loaded();
    p.open_mask_selection(PaneId::Left, true);
    assert!(p.mask_dialog.open);
    assert!(p.mask_dialog.select);
    assert_eq!(p.mask_dialog.pattern, "*");
    assert!(!p.mask_dialog.folders);
    assert_eq!(
        p.mask_invert_operation(),
        SelectionOperation::Invert,
        "keypad * inverts files only by default"
    );

    let (commands, send) = recorder();
    p.submit_mask_selection("  ", false, &send);
    assert!(p.mask_dialog.open, "an empty mask keeps the dialog open");
    assert!(p.mask_dialog.error.starts_with("Enter a mask"));
    assert!(commands.borrow().is_empty());

    p.submit_mask_selection("*.txt;*.MD|draft*", true, &send);
    accept_mask(&mut p, &commands, &send);
    assert!(!p.mask_dialog.open);
    let (request, mask) = mask_request(&commands);
    assert_eq!(
        mask,
        MaskSelection {
            pattern: "*.txt;*.MD|draft*".into(),
            select: true,
            folders: true,
        }
    );
    assert_eq!(request.pane, PaneId::Left);
    assert!(Arc::ptr_eq(
        request.view.as_ref().unwrap(),
        p.panes[0].display_index()
    ));
    assert!(commands.borrow().iter().any(|c| matches!(c,
        Command::SavePreferences { preferences, .. }
            if preferences.mask_selection.pattern == "*.txt;*.MD|draft*"
                && preferences.mask_selection.folders)));
    assert_eq!(p.mask_invert_operation(), SelectionOperation::InvertAll);

    // Deselect starts from the remembered mask and folder choice.
    p.open_mask_selection(PaneId::Right, false);
    assert!(!p.mask_dialog.select);
    assert_eq!(p.mask_dialog.pattern, "*.txt;*.MD|draft*");
    assert!(p.mask_dialog.folders);
    commands.borrow_mut().clear();
    p.submit_mask_selection("*.md", false, &send);
    accept_mask(&mut p, &commands, &send);
    let (request, mask) = mask_request(&commands);
    assert_eq!(request.pane, PaneId::Right);
    assert!(!mask.select);
    assert!(!mask.folders);
}

#[test]
fn an_invalid_mask_keeps_the_dialog_open_with_the_reason_and_is_not_remembered() {
    let (mut p, _) = loaded();
    p.open_mask_selection(PaneId::Left, true);
    let (commands, send) = recorder();
    let too_many = vec!["*.x"; 65].join(";");
    for (pattern, reason) in [
        ("a|b|c", "Use at most one | before excluded masks."),
        ("*.txt|", "Add at least one excluded mask after |."),
        (
            too_many.as_str(),
            "A selection mask may contain at most 64 masks.",
        ),
    ] {
        p.submit_mask_selection(pattern, true, &send);
        assert!(p.mask_dialog.open, "{pattern:?} keeps the dialog open");
        assert_eq!(p.mask_dialog.error, reason, "{pattern:?}");
        assert_eq!(p.mask_dialog.pattern, pattern, "the typed mask stays");
        assert!(p.mask_dialog.folders, "and so does Include folders");
    }
    assert!(
        commands.borrow().is_empty(),
        "nothing is selected or saved: {:?}",
        commands.borrow()
    );
    assert_eq!(
        p.preferences.values.mask_selection,
        MaskSelectionPreferences::default(),
        "an invalid mask is never remembered"
    );
    assert!(!p.preferences.edited.mask_selection);

    // The corrected mask applies, is remembered, and the reason goes.
    p.submit_mask_selection("*.txt|draft*", true, &send);
    accept_mask(&mut p, &commands, &send);
    assert!(!p.mask_dialog.open);
    assert!(p.mask_dialog.error.is_empty());
    let (_, mask) = mask_request(&commands);
    assert_eq!(mask.pattern, "*.txt|draft*");
    assert_eq!(p.preferences.values.mask_selection.pattern, "*.txt|draft*");
}

#[test]
fn a_busy_panel_or_cancel_keeps_the_selection_unchanged() {
    let (mut p, _) = loaded();
    p.open_mask_selection(PaneId::Left, true);
    p.submit_mask_selection("*.txt", false, &|_| false);
    assert!(p.mask_dialog.open, "a refused request keeps the dialog");
    assert!(!p.mask_dialog.error.is_empty());
    assert_eq!(
        p.preferences.values.mask_selection,
        MaskSelectionPreferences::default(),
        "only an accepted selection is remembered"
    );
    p.cancel_mask_selection();
    assert!(!p.mask_dialog.open);
    let (commands, send) = recorder();
    p.submit_mask_selection("*.txt", false, &send);
    assert!(
        commands.borrow().is_empty(),
        "a closed dialog cannot select"
    );
}
