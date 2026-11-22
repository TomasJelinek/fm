//! The Copy/Move dialog's link handling: copy the files links point to, or keep links as
//! links. Its default comes from what the destination's volume reports: a drive that
//! cannot store links, a network location where none are created, and a removable drive
//! that the selection is not already on copy the files; elsewhere links stay links. The
//! user's own choice is never changed afterwards. Presentation policy only: the backend
//! answers the question and the transfer reports each link it could not handle.
use super::*;
use midnight_core::{
    CopyLinkPolicy, TransferLinkDefaultReason, TransferLinkFacts, TransferLinksRequest,
    TransferLinksResult,
};

/// What the dialog knows about its destination's links.
pub(crate) struct DraftLinks {
    /// The draft these facts belong to.
    token: String,
    /// The destination folder the facts are for, or are asked about.
    destination: PathBuf,
    /// The folder that holds the selection, when one folder does.
    source: Option<PathBuf>,
    facts: Option<TransferLinkFacts>,
    pending: Option<(DesktopRequestId, Cancellation)>,
    /// The user's own choice (true copies the files links point to); None follows the
    /// destination's default.
    chosen: Option<bool>,
    /// The destination for which keeping links was explained once before starting:
    /// confirming again there starts.
    warned: Option<PathBuf>,
}

/// What the dialog shows about links.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct LinkChoiceView {
    /// The choice shown: true copies the files links point to.
    pub(crate) copy_files: bool,
    /// Why copying the files is the default here; empty where keeping links is.
    pub(crate) reason: String,
    /// Why keeping links cannot work here, while that is the choice; empty otherwise.
    pub(crate) warning: String,
}

use crate::transfer_link_option::COPY_FILES;

/// Why copying the files is the default for `facts`, in words for the dialog.
fn reason_text(facts: &TransferLinkFacts) -> String {
    match facts.copy_files_by_default() {
        None => String::new(),
        Some(TransferLinkDefaultReason::CannotStoreLinks) => format!(
            "This drive cannot store links{}, so the files they point to are copied.",
            filesystem(facts)
        ),
        Some(TransferLinkDefaultReason::NetworkLocation) => {
            "Network location: no links are created there, so the files they point to are \
             copied."
                .into()
        }
        Some(TransferLinkDefaultReason::RemovableDrive) => {
            "Removable drive: the files links point to are copied, so that no link there \
             points back to this computer."
                .into()
        }
    }
}

/// " (FAT32)" when the volume named its filesystem.
fn filesystem(facts: &TransferLinkFacts) -> String {
    facts
        .destination
        .filesystem
        .as_deref()
        .map_or_else(String::new, |name| format!(" ({name})"))
}

/// What keeping links does at a destination that cannot store them.
fn keep_warning(facts: &TransferLinkFacts, intent: TransferIntent) -> String {
    let stays = match intent {
        TransferIntent::Move => " and stays where it is",
        TransferIntent::Copy => "",
    };
    if facts.destination.network {
        format!(
            "No links are created at this network location: each link is left out{stays}, \
             and the job details name it."
        )
    } else {
        format!(
            "This drive cannot store links{}: each link is left out{stays}, and the job \
             details name it.",
            filesystem(facts)
        )
    }
}

impl Presenter {
    /// Asks what the open draft's destination does with links, for its default link
    /// handling. `source` is the folder that holds the selection, when one does. A draft
    /// without link handling (archive members) asks nothing.
    pub(crate) fn inspect_copy_draft_links(
        &mut self,
        source: Option<PathBuf>,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(draft) =
            self.copies.draft.as_ref().filter(|draft| {
                !draft.retire_requested && draft.directory_link_options_available()
            })
        else {
            return;
        };
        if self
            .copies
            .links
            .as_ref()
            .is_some_and(|links| links.token == draft.token)
        {
            return;
        }
        let token = draft.token.clone();
        let destination = draft.destination.clone();
        self.retire_copy_draft_links();
        let request = self.desktop.request();
        let cancel = Cancellation::default();
        let sent = send(Command::InspectTransferLinks(TransferLinksRequest {
            request,
            destination: destination.clone(),
            source: source.clone(),
            cancel: cancel.clone(),
        }));
        self.copies.links = Some(DraftLinks {
            token,
            destination,
            source,
            facts: None,
            pending: sent.then_some((request, cancel)),
            chosen: None,
            warned: None,
        });
    }

    /// The backend's answer for the dialog's destination.
    pub(crate) fn transfer_links_inspected(&mut self, result: TransferLinksResult) {
        let Some(links) = self.copies.links.as_mut().filter(|links| {
            links
                .pending
                .as_ref()
                .is_some_and(|(request, _)| *request == result.request)
                && links.destination == result.destination
        }) else {
            return;
        };
        links.pending = None;
        // Without an answer the dialog keeps links, as where nothing tells otherwise.
        links.facts = Some(result.result.unwrap_or_default());
    }

    /// The folder that holds the draft's selection, for a typed destination's check.
    pub(crate) fn copy_draft_links_source(&self) -> Option<PathBuf> {
        self.copies
            .links
            .as_ref()
            .and_then(|links| links.source.clone())
    }

    /// The link choice the open draft shows, its reason and its warning; None when the
    /// draft has no link handling.
    pub(crate) fn copy_draft_links_view(&self) -> Option<LinkChoiceView> {
        let draft = self
            .copies
            .draft
            .as_ref()
            .filter(|draft| draft.directory_link_options_available())?;
        let links = self
            .copies
            .links
            .as_ref()
            .filter(|links| links.token == draft.token);
        let facts = links.and_then(|links| links.facts.as_ref());
        let default = facts.is_some_and(|facts| facts.copy_files_by_default().is_some());
        let copy_files = links.and_then(|links| links.chosen).unwrap_or(default);
        Some(LinkChoiceView {
            copy_files,
            reason: facts.map(reason_text).unwrap_or_default(),
            warning: facts
                .filter(|facts| !copy_files && facts.cannot_keep_links())
                .map(|facts| keep_warning(facts, draft.intent))
                .unwrap_or_default(),
        })
    }

    /// The user chose the link handling of the draft `token`: it stays as chosen.
    pub(crate) fn choose_copy_draft_links(&mut self, token: &str, copy_files: bool) {
        let Some(draft) = self
            .copies
            .draft
            .as_ref()
            .filter(|draft| draft.token == token && !draft.retire_requested)
        else {
            return;
        };
        match self.copies.links.as_mut() {
            Some(links) if links.token == token => links.chosen = Some(copy_files),
            _ => {
                let destination = draft.destination.clone();
                self.copies.links = Some(DraftLinks {
                    token: token.to_owned(),
                    destination,
                    source: None,
                    facts: None,
                    pending: None,
                    chosen: Some(copy_files),
                    warned: None,
                });
            }
        }
    }

    /// Whether the draft may start with `links` at `destination`, whose link facts are
    /// `checked` when its check just answered (otherwise those the dialog already has).
    /// Returns what the dialog says instead of starting, once: keeping links where none
    /// can be stored, or a typed folder whose default differs from the choice shown
    /// while the user has not chosen (the choice then follows that folder's default).
    pub(crate) fn copy_links_hold(
        &mut self,
        destination: &Path,
        links: Option<CopyLinkPolicy>,
        checked: Option<TransferLinkFacts>,
    ) -> Option<String> {
        let links = links?;
        let draft = self.copies.draft.as_ref()?;
        let (token, intent, verb) = (
            draft.token.clone(),
            draft.intent,
            match draft.intent {
                TransferIntent::Move => "Move",
                TransferIntent::Copy => "Copy",
            },
        );
        let state = self.copies.links.get_or_insert_with(|| DraftLinks {
            token: token.clone(),
            destination: destination.to_owned(),
            source: None,
            facts: None,
            pending: None,
            chosen: None,
            warned: None,
        });
        if state.token != token {
            return None;
        }
        if let Some(checked) = checked {
            // The typed folder's facts replace those of the folder asked about before.
            if let Some((_, cancel)) = state.pending.take() {
                cancel.cancel();
            }
            let moved = state.destination != destination;
            state.destination = destination.to_owned();
            state.facts = Some(checked);
            let facts = state.facts.as_ref()?;
            let default = facts.copy_files_by_default().is_some();
            let shown = links == CopyLinkPolicy::FollowLinks;
            if moved && state.chosen.is_none() && default != shown {
                let reason = if default {
                    reason_text(facts)
                } else {
                    "Links are kept as links here.".into()
                };
                return Some(format!(
                    "The link handling changed for this folder. {reason} Select {verb} again \
                     to start."
                ));
            }
        } else if state.destination != destination {
            // A panel's other folder, not asked about: start with the choice shown.
            return None;
        }
        let facts = state.facts.as_ref()?;
        if links == CopyLinkPolicy::Preserve
            && facts.cannot_keep_links()
            && state.warned.as_deref() != Some(destination)
        {
            state.warned = Some(destination.to_owned());
            return Some(format!(
                "{} Select {verb} again to keep links anyway, or choose \u{201c}{COPY_FILES}\u{201d}.",
                keep_warning(facts, intent)
            ));
        }
        None
    }

    /// Stops a pending question and forgets the draft's link state.
    pub(crate) fn retire_copy_draft_links(&mut self) {
        if let Some(links) = self.copies.links.take()
            && let Some((_, cancel)) = links.pending
        {
            cancel.cancel();
        }
    }
}

#[cfg(test)]
#[path = "copy_dialog_links_tests.rs"]
mod tests;
