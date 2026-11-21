//! Select/Deselect by mask has the backend compile the mask before it is applied
//! and remembered, as the panel filter dialog has its filter checked.
use super::*;
use std::cell::RefCell;

/// What the backend says of a mask whose matcher exceeds the budget
/// (`midnight_platform_common::filter::MASK_TOO_LARGE`).
const TOO_LARGE: &str =
    "This mask is too large to match names with. Use fewer ? wildcards or shorter masks.";

/// Settings are loaded with defaults and both panes list a folder and two files.
fn listed() -> Presenter {
    let root = std::env::temp_dir().join("mask-selection-checks");
    let mut p = Presenter::new(root.clone(), root.clone());
    p.apply_with_sender(Event::PreferencesLoaded(Ok(Box::default())), &|_| true);
    for pane in [PaneId::Left, PaneId::Right] {
        let sent = RefCell::new(Vec::new());
        assert!(p.navigate(pane, root.clone(), None, &|command| {
            sent.borrow_mut().push(command);
            true
        }));
        let navigation = sent
            .into_inner()
            .into_iter()
            .find_map(|command| match command {
                Command::Navigate { navigation, .. } | Command::Refresh { navigation, .. } => {
                    Some(navigation)
                }
                _ => None,
            })
            .expect("the folder is read");
        let entries: Arc<[Entry]> = [
            Entry::new(EntryId(1), root.join("docs"), EntryKind::Directory),
            Entry::new(EntryId(2), root.join("a.txt"), EntryKind::File),
            Entry::new(EntryId(3), root.join("b.md"), EntryKind::File),
        ]
        .into();
        p.apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
    }
    p
}

/// The commands a step sent, kept for inspection.
#[derive(Default)]
struct Sent(RefCell<Vec<Command>>);

impl Sent {
    fn sender(&self) -> impl Fn(Command) -> bool + '_ {
        |command| {
            self.0.borrow_mut().push(command);
            true
        }
    }

    /// The mask checks asked of the backend since the last call.
    fn checks(&self) -> Vec<MaskValidationRequest> {
        self.0
            .take()
            .into_iter()
            .filter_map(|command| match command {
                Command::ValidateSelectionMask(request) => Some(request),
                _ => None,
            })
            .collect()
    }

    /// Masks sent to a selection worker, and whether settings were saved.
    fn applied(&self) -> (Vec<MaskSelection>, bool) {
        let commands = self.0.take();
        let masks = commands
            .iter()
            .filter_map(|command| match command {
                Command::Selection(request) => match &request.action {
                    SelectionAction::Mask(mask) => Some((**mask).clone()),
                    _ => None,
                },
                _ => None,
            })
            .collect();
        let saved = commands
            .iter()
            .any(|command| matches!(command, Command::SavePreferences { .. }));
        (masks, saved)
    }
}

fn answer(
    p: &mut Presenter,
    request: MaskValidationRequest,
    result: Result<(), &str>,
    send: &dyn Fn(Command) -> bool,
) {
    p.apply_with_sender(
        Event::SelectionMaskValidated {
            request,
            result: result.map_err(str::to_owned),
        },
        send,
    );
}

#[test]
fn a_mask_over_the_matcher_budget_keeps_the_dialog_open_with_the_backends_reason() {
    let mut p = listed();
    p.open_mask_selection(PaneId::Left, true);
    let sent = Sent::default();
    // Thousands of ? pass the dialog's grammar check; only compiling finds the size.
    let large = "?".repeat(MAX_FILTER_BYTES);
    p.submit_mask_selection(&large, true, &sent.sender());
    let [check] = sent.checks().try_into().expect("one backend check");
    assert_eq!(check.pattern, large);
    assert!(p.mask_dialog.open, "the dialog waits for the check");
    assert!(p.mask_dialog.error.is_empty());
    answer(&mut p, check, Err(TOO_LARGE), &sent.sender());
    assert!(p.mask_dialog.open, "a refused mask keeps the dialog open");
    assert_eq!(p.mask_dialog.error, TOO_LARGE);
    assert_eq!(p.mask_dialog.pattern, large, "the typed mask stays");
    assert!(p.mask_dialog.folders, "and so does Include folders");
    assert_eq!(
        sent.applied(),
        (vec![], false),
        "nothing is selected or saved"
    );
    assert_eq!(
        p.preferences.values.mask_selection,
        MaskSelectionPreferences::default(),
        "a refused mask is never remembered"
    );
    assert!(!p.preferences.edited.mask_selection);

    // OK with a mask the backend accepts applies it, remembers it and closes.
    p.submit_mask_selection("*.txt", true, &sent.sender());
    let [check] = sent.checks().try_into().expect("one backend check");
    answer(&mut p, check, Ok(()), &sent.sender());
    assert!(!p.mask_dialog.open);
    assert!(p.mask_dialog.error.is_empty());
    let (masks, saved) = sent.applied();
    assert_eq!(
        masks,
        [MaskSelection {
            pattern: "*.txt".into(),
            select: true,
            folders: true,
        }]
    );
    assert!(saved);
    assert_eq!(
        p.preferences.values.mask_selection,
        MaskSelectionPreferences {
            pattern: "*.txt".into(),
            folders: true,
        }
    );
}

#[test]
fn only_the_answer_to_the_last_ok_of_the_open_dialog_counts() {
    let mut p = listed();
    p.open_mask_selection(PaneId::Left, false);
    let sent = Sent::default();
    p.submit_mask_selection("*.txt", false, &sent.sender());
    p.submit_mask_selection("*.md", false, &sent.sender());
    let [first, second] = sent.checks().try_into().expect("two backend checks");
    assert!(second.id > first.id, "a later check has a later identity");
    // The first OK was replaced: its answer changes nothing.
    answer(&mut p, first, Ok(()), &sent.sender());
    assert!(p.mask_dialog.open);
    assert_eq!(sent.applied(), (vec![], false));
    answer(&mut p, second.clone(), Err(TOO_LARGE), &sent.sender());
    assert_eq!(p.mask_dialog.error, TOO_LARGE);
    // A repeated answer of the same check changes nothing either.
    answer(&mut p, second, Ok(()), &sent.sender());
    assert!(p.mask_dialog.open);
    assert_eq!(sent.applied(), (vec![], false));

    // Cancel drops the check under way.
    p.submit_mask_selection("*.md", false, &sent.sender());
    let [cancelled] = sent.checks().try_into().expect("one backend check");
    p.cancel_mask_selection();
    answer(&mut p, cancelled.clone(), Ok(()), &sent.sender());
    assert!(!p.mask_dialog.open);
    assert_eq!(sent.applied(), (vec![], false));

    // A new opening never reuses an identity, so the old answer cannot apply to it.
    p.open_mask_selection(PaneId::Right, false);
    answer(&mut p, cancelled.clone(), Ok(()), &sent.sender());
    assert!(p.mask_dialog.open);
    assert_eq!(sent.applied(), (vec![], false));
    p.submit_mask_selection("*.md", true, &sent.sender());
    let [check] = sent.checks().try_into().expect("one backend check");
    assert!(check.id > cancelled.id);
    answer(&mut p, check, Ok(()), &sent.sender());
    assert!(!p.mask_dialog.open);
    let (masks, saved) = sent.applied();
    assert_eq!(
        masks,
        [MaskSelection {
            pattern: "*.md".into(),
            select: false,
            folders: true,
        }]
    );
    assert!(saved);
}

#[test]
fn an_accepted_mask_for_a_busy_panel_keeps_the_dialog_open() {
    let mut p = listed();
    p.open_mask_selection(PaneId::Left, true);
    let sent = Sent::default();
    p.submit_mask_selection("*.txt", false, &sent.sender());
    let [check] = sent.checks().try_into().expect("one backend check");
    // The panel's selection worker refuses the request.
    answer(&mut p, check, Ok(()), &|_| false);
    assert!(p.mask_dialog.open);
    assert!(
        p.mask_dialog
            .error
            .starts_with("The panel is still loading")
    );
    assert_eq!(
        p.preferences.values.mask_selection,
        MaskSelectionPreferences::default(),
        "only an applied mask is remembered"
    );
}
