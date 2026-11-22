//! Copy/Move adapter: backend captures stay in Presenter, edits stay in the form.
use crate::{
    MainWindow, OperationPresentation, Presenter, presenter::LinkChoiceView, transfer_link_option,
};
use midnight_core::{Command, TransferMetadataPolicy, TransferStartPolicy};
use slint::{ModelRc, SharedString, VecModel};
use std::rc::Rc;

/// `links` is the draft's link choice, None when it has none (archive members).
pub(super) fn initialize(
    window: &OperationPresentation<'_>,
    main: &MainWindow,
    links: Option<LinkChoiceView>,
) {
    let title = if main.get_copy_draft_move() {
        "Move"
    } else {
        "Copy"
    };
    window.set_title_text(title.into());
    window.set_confirm_text(title.into());
    window.set_body(main.get_copy_draft_body());
    window.set_policy(main.get_copy_draft_policy());
    window.set_wait_visible(true);
    window.set_wait_checked(false);
    window.set_links_visible(links.is_some());
    window.set_link_choices(ModelRc::from(Rc::new(VecModel::from(vec![
        SharedString::from(transfer_link_option::COPY_FILES),
        SharedString::from(transfer_link_option::KEEP_LINKS),
    ]))));
    window.set_link_hint(transfer_link_option::HINT.into());
    window.set_link_reason("".into());
    window.set_link_warning("".into());
    refresh_links(window, links.as_ref());
    window.set_has_name(true);
    window.set_initial_name(main.get_copy_draft_destination());
    window.set_selection_end(main.get_copy_draft_selection_end());
    window.set_input_label("Destination folder".into());
    window.set_native_icon(main.get_action_icons().copy);
    window.set_option_visible(true);
    window.set_option_enabled(main.get_copy_draft_metadata_enabled());
    window.set_option_checked(false);
    window.set_option_label("Preserve exact modified time".into());
    window.set_option_hint("Verify each destination file’s exact modified time. Up to 64 native regular files; folders, links and archive members are unsupported. Unsupported precision fails instead of rounding; earlier completed copies may remain.".into());
}

/// Shows the presenter's link choice, why the default was chosen and what keeping links
/// does where it cannot work. Unchanged values are not set again.
pub(super) fn refresh_links(window: &OperationPresentation<'_>, links: Option<&LinkChoiceView>) {
    let Some(links) = links else {
        return;
    };
    let index = if links.copy_files { 0 } else { 1 };
    if window.get_link_choice() != index {
        window.set_link_choice(index);
    }
    if window.get_link_reason().as_str() != links.reason {
        window.set_link_reason(links.reason.as_str().into());
    }
    if window.get_link_warning().as_str() != links.warning {
        window.set_link_warning(links.warning.as_str().into());
    }
}

/// `copy_files` is the dialog's link choice, None when it shows none.
pub(super) fn confirm(
    p: &mut Presenter,
    token: &str,
    destination: &str,
    preserve_time: bool,
    wait_for_earlier: bool,
    copy_files: Option<bool>,
    send: &dyn Fn(Command) -> bool,
) -> String {
    let generation = p.desktop.notice_generation;
    let links = transfer_link_option::policy(copy_files);
    p.confirm_copy_dialog_with_directory_links(
        token,
        destination,
        if preserve_time {
            TransferMetadataPolicy::RequireModifiedTime
        } else {
            TransferMetadataPolicy::ContentOnly
        },
        if wait_for_earlier {
            TransferStartPolicy::WaitForEarlier
        } else {
            TransferStartPolicy::Immediate
        },
        links,
        send,
    );
    // This synchronous call is the only writer under the Presenter borrow.
    // Route only its newly generated admission notice to the still-open child;
    // an unrelated earlier notice is not consumed or assigned a new identity.
    let notice = if p.desktop.notice_generation != generation
        && p.copies
            .draft
            .as_ref()
            .is_some_and(|draft| !draft.retire_requested)
    {
        p.desktop.notice.take().map(|(_, message)| message)
    } else {
        None
    };
    let checking = p
        .copies
        .destination_check
        .as_ref()
        .is_some_and(|check| check.token == token);
    if let Some(draft) = p
        .copies
        .draft
        .as_mut()
        .filter(|draft| draft.token == token && !draft.retire_requested)
    {
        if let Some(message) = notice {
            draft.error = message;
        }
        // The backend is checking the typed folder: its answer admits the transfer
        // or shows the dialog's refusal.
        if draft.error.is_empty() && checking {
            String::new()
        } else if draft.error.is_empty() {
            "The transfer was not admitted. Try again.".into()
        } else {
            draft.error.clone()
        }
    } else {
        String::new()
    }
}
