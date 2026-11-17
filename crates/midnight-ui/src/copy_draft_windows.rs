//! Copy/Move adapter: backend captures stay in Presenter, edits stay in the form.
use crate::{MainWindow, OperationPresentation, Presenter};
use midnight_core::{Command, TransferMetadataPolicy, TransferStartPolicy};

pub(super) fn initialize(window: &OperationPresentation<'_>, main: &MainWindow) {
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

pub(super) fn confirm(
    p: &mut Presenter,
    token: &str,
    destination: &str,
    preserve_time: bool,
    wait_for_earlier: bool,
    send: &dyn Fn(Command) -> bool,
) -> String {
    let generation = p.desktop.notice_generation;
    p.confirm_copy_dialog_options(
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
    if let Some(draft) = p
        .copies
        .draft
        .as_mut()
        .filter(|draft| draft.token == token && !draft.retire_requested)
    {
        if let Some(message) = notice {
            draft.error = message;
        }
        if draft.error.is_empty() {
            "The transfer was not admitted. Try again.".into()
        } else {
            draft.error.clone()
        }
    } else {
        String::new()
    }
}
