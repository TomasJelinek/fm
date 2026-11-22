//! The Copy/Move dialog's link choice: its default from the destination, the reason it
//! shows, the user's choice that stays, and the explanation before keeping links where
//! none can be stored.
use super::*;
use midnight_core::{DestinationCheckResult, TransferLinkDestination, TransferLinkStorage};
use std::cell::RefCell;

fn facts(
    storage: TransferLinkStorage,
    removable: bool,
    filesystem: Option<&str>,
) -> TransferLinkFacts {
    TransferLinkFacts {
        destination: TransferLinkDestination {
            storage,
            filesystem: filesystem.map(str::to_owned),
            network: false,
            removable,
            volume: Some(7),
            refuses_cross_volume_move: false,
        },
        same_volume: false,
    }
}

/// A Move of two clipboard items to `destination`, with the dialog's link question sent.
fn draft(destination: &Path) -> (Presenter, String, TransferLinksRequest) {
    let base = std::env::temp_dir().join("copy-dialog-links");
    let mut p = Presenter::new(base.clone(), base.clone());
    let sent = RefCell::new(Vec::new());
    p.start_copy(
        FileClipboard {
            paths: vec![base.join("source").join("a"), base.join("source").join("b")],
            intent: FileClipboardIntent::Move,
        },
        destination.to_owned(),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    let token = p.copies.draft.as_ref().unwrap().token.clone();
    let sent: [Command; 1] = sent
        .into_inner()
        .try_into()
        .unwrap_or_else(|sent: Vec<Command>| panic!("one command, not {}", sent.len()));
    let [Command::InspectTransferLinks(request)] = sent else {
        panic!("the dialog asks what its destination does with links")
    };
    assert_eq!(request.destination, destination);
    assert_eq!(
        request.source.as_deref(),
        Some(base.join("source").as_path())
    );
    (p, token, request)
}

fn answer(p: &mut Presenter, request: &TransferLinksRequest, facts: TransferLinkFacts) {
    p.transfer_links_inspected(TransferLinksResult {
        request: request.request,
        destination: request.destination.clone(),
        result: Ok(facts),
    });
}

/// Confirms the draft with `links`, returning what it sent.
fn confirm(
    p: &mut Presenter,
    token: &str,
    destination: &Path,
    links: CopyLinkPolicy,
) -> Vec<Command> {
    let sent = RefCell::new(Vec::new());
    p.confirm_copy_dialog_with_directory_links(
        token,
        &destination.to_string_lossy(),
        TransferMetadataPolicy::ContentOnly,
        TransferStartPolicy::Immediate,
        Some(links),
        &|command| {
            sent.borrow_mut().push(command);
            true
        },
    );
    sent.into_inner()
}

fn started_with(commands: &[Command]) -> Option<CopyLinkPolicy> {
    commands.iter().find_map(|command| match command {
        Command::EnqueueCopyWithOptions(options) => Some(options.links),
        _ => None,
    })
}

#[test]
fn a_removable_drive_copies_the_files_by_default_and_the_users_choice_stays() {
    let destination = std::env::temp_dir().join("stick");
    let (mut p, token, request) = draft(&destination);
    // Until the answer arrives, links stay links, as where nothing tells otherwise.
    let view = p.copy_draft_links_view().unwrap();
    assert!(!view.copy_files && view.reason.is_empty() && view.warning.is_empty());
    answer(
        &mut p,
        &request,
        facts(TransferLinkStorage::Supported, true, None),
    );
    let view = p.copy_draft_links_view().unwrap();
    assert!(view.copy_files, "{view:?}");
    assert!(view.reason.starts_with("Removable drive: "), "{view:?}");
    assert!(view.warning.is_empty());
    // The user keeps links: a later answer never changes that choice.
    p.choose_copy_draft_links(&token, false);
    answer(
        &mut p,
        &request,
        facts(TransferLinkStorage::Supported, true, None),
    );
    assert!(!p.copy_draft_links_view().unwrap().copy_files);
    assert_eq!(
        started_with(&confirm(
            &mut p,
            &token,
            &destination,
            CopyLinkPolicy::Preserve
        )),
        Some(CopyLinkPolicy::Preserve)
    );
    assert!(
        p.copies.links.is_none(),
        "the started draft forgets its link state"
    );
}

#[test]
fn a_selection_already_on_the_removable_drive_keeps_links_by_default() {
    let destination = std::env::temp_dir().join("stick");
    let (mut p, _, request) = draft(&destination);
    let mut same = facts(TransferLinkStorage::Supported, true, None);
    same.same_volume = true;
    answer(&mut p, &request, same);
    let view = p.copy_draft_links_view().unwrap();
    assert!(!view.copy_files && view.reason.is_empty(), "{view:?}");
}

#[test]
fn keeping_links_where_none_can_be_stored_is_explained_once_before_starting() {
    let destination = std::env::temp_dir().join("fat-stick");
    let (mut p, token, request) = draft(&destination);
    answer(
        &mut p,
        &request,
        facts(TransferLinkStorage::Unsupported, true, Some("FAT32")),
    );
    let view = p.copy_draft_links_view().unwrap();
    assert!(view.copy_files);
    assert_eq!(
        view.reason,
        "This drive cannot store links (FAT32), so the files they point to are copied."
    );
    assert!(
        view.warning.is_empty(),
        "copying the files needs no warning"
    );
    p.choose_copy_draft_links(&token, false);
    let view = p.copy_draft_links_view().unwrap();
    assert_eq!(
        view.warning,
        "This drive cannot store links (FAT32): each link is left out and stays where it \
         is, and the job details name it."
    );
    // The first confirmation explains and starts nothing.
    let sent = confirm(&mut p, &token, &destination, CopyLinkPolicy::Preserve);
    assert_eq!(started_with(&sent), None);
    let error = &p.copies.draft.as_ref().unwrap().error;
    assert!(error.starts_with(&view.warning), "{error}");
    assert!(
        error.ends_with("or choose \u{201c}Copy the files links point to\u{201d}."),
        "{error}"
    );
    // The dialog shows it under the link choice.
    assert!(p.copy_draft_error_about_links());
    // Confirming again keeps links anyway.
    let sent = confirm(&mut p, &token, &destination, CopyLinkPolicy::Preserve);
    assert_eq!(started_with(&sent), Some(CopyLinkPolicy::Preserve));
}

#[test]
fn copying_the_files_to_a_drive_without_links_starts_at_once() {
    let destination = std::env::temp_dir().join("fat-stick");
    let (mut p, token, request) = draft(&destination);
    answer(
        &mut p,
        &request,
        facts(TransferLinkStorage::Unsupported, false, Some("exFAT")),
    );
    let sent = confirm(&mut p, &token, &destination, CopyLinkPolicy::FollowLinks);
    assert_eq!(started_with(&sent), Some(CopyLinkPolicy::FollowLinks));
}

/// A Move from another drive to a destination that refuses one (here a network location)
/// does not start from the dialog, whichever way it opened (here a paste of a Cut): it
/// says why and suggests Copy, again on every confirmation. On the destination's own
/// drive the Move renames and starts.
#[test]
fn a_move_from_another_drive_to_a_network_location_does_not_start_and_suggests_copy() {
    let destination = std::env::temp_dir().join("share");
    let mut network = facts(TransferLinkStorage::Unsupported, false, None);
    network.destination.network = true;
    network.destination.refuses_cross_volume_move = true;
    let (mut p, token, request) = draft(&destination);
    answer(&mut p, &request, network.clone());
    for _ in 0..2 {
        let sent = confirm(&mut p, &token, &destination, CopyLinkPolicy::FollowLinks);
        assert!(sent.is_empty(), "{sent:?}");
        assert_eq!(
            p.copies.draft.as_ref().unwrap().error,
            "Items from another drive cannot be moved to a network location: Move cannot verify \
             their copies there before removing the originals. Copy them instead."
        );
        // The refusal concerns the destination, not the link choice.
        assert!(!p.copy_draft_error_about_links());
    }
    network.same_volume = true;
    let (mut p, token, request) = draft(&destination);
    answer(&mut p, &request, network);
    let sent = confirm(&mut p, &token, &destination, CopyLinkPolicy::FollowLinks);
    assert_eq!(started_with(&sent), Some(CopyLinkPolicy::FollowLinks));
}

#[test]
fn a_typed_folder_whose_default_differs_shows_its_default_before_starting() {
    let proposed = std::env::temp_dir().join("fixed");
    let (mut p, token, request) = draft(&proposed);
    answer(
        &mut p,
        &request,
        facts(TransferLinkStorage::Supported, false, None),
    );
    assert!(!p.copy_draft_links_view().unwrap().copy_files);
    // The user types a removable drive's folder and confirms what the dialog showed.
    let typed = std::env::temp_dir().join("typed-stick");
    let sent = confirm(&mut p, &token, &typed, CopyLinkPolicy::Preserve);
    let check = match sent.as_slice() {
        [Command::CheckTransferDestination(check)] => check.clone(),
        other => panic!("a typed folder is checked first: {other:?}"),
    };
    assert!(check.links, "the check asks for the folder's links");
    let checked = |p: &mut Presenter, check: &midnight_core::DestinationCheckRequest| {
        let sent = RefCell::new(Vec::new());
        p.transfer_destination_checked(
            DestinationCheckResult {
                request: check.request,
                path: check.path.clone(),
                result: Ok(true),
                links: Some(facts(TransferLinkStorage::Supported, true, None)),
            },
            &|command| {
                sent.borrow_mut().push(command);
                true
            },
        );
        sent.into_inner()
    };
    assert_eq!(started_with(&checked(&mut p, &check)), None);
    let error = p.copies.draft.as_ref().unwrap().error.clone();
    assert!(
        error.starts_with("The link handling changed for this folder. Removable drive: "),
        "{error}"
    );
    assert!(error.ends_with("Select Move again to start."), "{error}");
    assert!(
        p.copy_draft_error_about_links(),
        "the link choice's own note"
    );
    assert!(p.copy_draft_links_view().unwrap().copy_files);
    // Confirming again with the shown default starts.
    let sent = confirm(&mut p, &token, &typed, CopyLinkPolicy::FollowLinks);
    let [Command::CheckTransferDestination(check)] = sent.as_slice() else {
        panic!("the typed folder is checked again")
    };
    assert_eq!(
        started_with(&checked(&mut p, check)),
        Some(CopyLinkPolicy::FollowLinks)
    );
}

#[test]
fn an_answer_for_another_question_or_draft_changes_nothing() {
    let destination = std::env::temp_dir().join("stick");
    let (mut p, _, request) = draft(&destination);
    p.transfer_links_inspected(TransferLinksResult {
        request: DesktopRequestId(request.request.0 + 1),
        destination: destination.clone(),
        result: Ok(facts(TransferLinkStorage::Supported, true, None)),
    });
    assert!(!p.copy_draft_links_view().unwrap().copy_files);
    // An unreadable destination keeps links, as where nothing tells otherwise.
    p.transfer_links_inspected(TransferLinksResult {
        request: request.request,
        destination,
        result: Err(FsError::from_io(
            "inspect links",
            Path::new("x"),
            std::io::ErrorKind::PermissionDenied.into(),
        )),
    });
    let view = p.copy_draft_links_view().unwrap();
    assert!(!view.copy_files && view.reason.is_empty() && view.warning.is_empty());
}
