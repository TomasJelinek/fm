//! A copy draft retains shared identities; only backend admission expands selected paths.
use super::*;
use midnight_core::{CopyLinkPolicy, CopyOptionsInput, CopyOptionsRequest};
#[path = "copy_dialog_items.rs"]
mod items;
#[path = "copy_dialog_links.rs"]
mod links;
#[path = "copy_dialog_picker.rs"]
mod picker;
pub(crate) use items::DraftItems;
pub(crate) use links::{DraftLinks, LinkChoiceView};

#[derive(Clone)]
pub(super) struct CopyContext {
    pub(super) source: PathBuf,
    pub(super) destination: PathBuf,
    pub(super) intent: TransferIntent,
}

#[cfg(test)]
mod owned_draft_tests {
    use super::*;
    use std::{
        cell::Cell,
        sync::atomic::{AtomicUsize, Ordering},
    };

    #[derive(Debug)]
    struct Sources(Arc<AtomicUsize>);
    impl Drop for Sources {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }
    impl CapturedTransferSources for Sources {
        fn intent(&self) -> TransferIntent {
            TransferIntent::Copy
        }
        fn root_count(&self) -> usize {
            1
        }
        fn metadata_available(&self) -> bool {
            false
        }
        fn prepare(
            &self,
            _: &dyn TransferService,
            _: &Cancellation,
        ) -> Result<PreparedCapturedTransfer, FsError> {
            panic!("UI must not prepare captured sources")
        }
    }
    fn draft() -> (Presenter, Arc<AtomicUsize>) {
        let path = std::env::temp_dir().join("owned-copy-draft");
        let mut p = Presenter::new(path.clone(), path.clone());
        let drops = Arc::new(AtomicUsize::new(0));
        p.copies.draft = Some(CopyDraft {
            token: "copy-draft:1".into(),
            input: CopyDraftInput::Tree {
                capture: TreeTransferCapture {
                    intent: TransferIntent::Copy,
                    root_count: 1,
                    metadata_available: false,
                    sources: Arc::new(Sources(drops.clone())),
                },
                directory_links_available: true,
            },
            source: "source".into(),
            destination: path,
            items: DraftItems::One {
                name: "one item".into(),
                bytes: None,
            },
            error: String::new(),
            intent: TransferIntent::Copy,
            retire_requested: false,
        });
        (p, drops)
    }

    #[test]
    fn deferred_cancel_keeps_exact_draft_until_an_owned_flush() {
        let (mut p, drops) = draft();
        p.require_owned_command_scope();
        p.defer_cancel_copy_dialog("stale-token");
        assert!(!p.copies.draft.as_ref().unwrap().retire_requested);
        for _ in 0..64 {
            p.defer_cancel_copy_dialog("copy-draft:1");
        }
        assert!(p.copies.draft.as_ref().unwrap().retire_requested);
        assert!(p.retired.is_empty());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        p.flush_deferred_copy_draft();
        assert!(p.copies.draft.is_some());
        assert!(p.retired.is_empty());
        p.set_owned_command_scope_active(true);
        p.flush_deferred_copy_draft();
        assert!(p.copies.draft.is_none());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        let retired = p.take_retired();
        assert_eq!(retired.len(), 1);
        drop(retired);
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn installed_presenter_rejects_repeated_unguarded_submit_before_cloning() {
        let (mut p, drops) = draft();
        p.require_owned_command_scope();
        let next_id = p.operations.next_id;
        let destination = std::env::temp_dir().join("destination");
        for _ in 0..64 {
            p.confirm_copy_dialog("copy-draft:1", &destination.to_string_lossy(), &|_| {
                panic!("an installed presenter requires the caller's owned scope")
            });
        }
        assert_eq!(p.operations.next_id, next_id);
        assert!(p.copies.progress.is_empty());
        assert!(p.retired.is_empty());
        assert!(!p.copies.draft.as_ref().unwrap().error.is_empty());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        p.cancel_copy_dialog("copy-draft:1");
        assert!(p.copies.draft.as_ref().unwrap().retire_requested);
        assert!(p.retired.is_empty());
        p.force_retire_copy_draft();
        drop(p.take_retired());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn busy_shutdown_hides_draft_in_place_and_keeps_cancel_controls_responsive() {
        let (mut p, drops) = draft();
        p.require_owned_command_scope();
        p.copies.progress.push(Arc::new(CopyProgress::queued(
            OperationId(9),
            std::env::temp_dir(),
            1,
            TransferIntent::Copy,
        )));
        let cancels = Cell::new(0);
        let send = |command| {
            assert!(matches!(command, Command::CancelCopy(OperationId(9))));
            cancels.set(cancels.get() + 1);
            true
        };
        p.request_operation_shutdown(&send);
        assert!(!p.continue_operation_shutdown(&send));
        assert_eq!(cancels.get(), 2);
        assert!(p.operations.closing);
        assert!(p.copies.draft.as_ref().unwrap().retire_requested);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        p.flush_deferred_copy_draft();
        assert!(p.copies.draft.is_some());
        p.set_owned_command_scope_active(true);
        p.flush_deferred_copy_draft();
        assert!(p.copies.draft.is_none());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(p.take_retired());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn successful_send_retires_original_draft_after_a_fast_backend_releases_its_clone() {
        let (mut p, drops) = draft();
        // The proposed destination: a typed other folder would be checked first.
        let destination = std::env::temp_dir().join("owned-copy-draft");
        p.confirm_copy_dialog("copy-draft:1", &destination.to_string_lossy(), &|command| {
            drop(command);
            true
        });
        assert!(p.copies.draft.is_none());
        assert_eq!(p.copies.progress.len(), 1);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        drop(p.take_retired());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn exhausted_operation_id_retires_unused_builder_without_replacing_draft() {
        let (mut p, drops) = draft();
        p.operations.next_id = u64::MAX;
        // The proposed destination: a typed other folder would be checked first.
        let destination = std::env::temp_dir().join("owned-copy-draft");
        p.confirm_copy_dialog("copy-draft:1", &destination.to_string_lossy(), &|_| {
            panic!("exhausted ids must not send")
        });
        assert_eq!(p.operations.next_id, u64::MAX);
        assert!(p.copies.progress.is_empty());
        assert!(p.copies.draft.is_some());
        let retired = p.take_retired();
        assert_eq!(retired.len(), 1);
        drop(retired);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        p.retire_copy_draft();
        drop(p.take_retired());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn hidden_draft_cannot_submit_or_consume_an_operation_id() {
        let (mut p, drops) = draft();
        p.defer_cancel_copy_dialog("copy-draft:1");
        let next_id = p.operations.next_id;
        let destination = std::env::temp_dir().join("destination");
        for _ in 0..64 {
            p.confirm_copy_dialog("copy-draft:1", &destination.to_string_lossy(), &|_| {
                panic!("a hidden draft must not build or send a request")
            });
        }
        assert_eq!(p.operations.next_id, next_id);
        assert!(p.copies.progress.is_empty());
        assert!(p.retired.is_empty());
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        p.retire_copy_draft();
        drop(p.take_retired());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn deferred_cancel_prevents_replacing_the_draft_owner() {
        let (mut p, drops) = draft();
        p.defer_cancel_copy_dialog("copy-draft:1");
        let destination = std::env::temp_dir().join("replacement");
        p.open_clipboard_move(
            FileClipboard {
                paths: vec![destination.join("source.txt")],
                intent: FileClipboardIntent::Move,
            },
            destination,
        );
        assert_eq!(p.copies.draft.as_ref().unwrap().token, "copy-draft:1");
        assert!(p.copies.draft.as_ref().unwrap().retire_requested);
        assert_eq!(drops.load(Ordering::SeqCst), 0);
        p.retire_copy_draft();
        drop(p.take_retired());
        assert_eq!(drops.load(Ordering::SeqCst), 1);
    }
}

pub(crate) struct CopyDraft {
    pub(crate) token: String,
    pub(super) input: CopyDraftInput,
    /// Where the items come from, as the dialog's first line names it in its tooltip.
    pub(crate) source: String,
    pub(crate) destination: PathBuf,
    /// What the draft holds, as the dialog's first line names it.
    pub(crate) items: DraftItems,
    pub(crate) error: String,
    pub(crate) intent: TransferIntent,
    /// Hidden in place until an owned callback can detach the captured sources.
    pub(crate) retire_requested: bool,
}

/// What the dialog says when its destination is not an existing folder.
pub(crate) const DESTINATION_FOLDER_REQUIRED: &str =
    "Enter the full path of an existing destination folder.";

/// A typed destination that no panel shows, while the backend checks that it is an
/// existing folder. Its answer resumes the confirmation with these choices.
pub(crate) struct DestinationCheck {
    pub(crate) token: String,
    request: DesktopRequestId,
    path: PathBuf,
    cancel: Cancellation,
    metadata: TransferMetadataPolicy,
    start_policy: TransferStartPolicy,
    links: Option<CopyLinkPolicy>,
}

impl CopyDraft {
    pub(crate) fn directory_link_options_available(&self) -> bool {
        match &self.input {
            CopyDraftInput::Archive(_) => false,
            CopyDraftInput::Tree {
                directory_links_available,
                ..
            } => *directory_links_available,
            CopyDraftInput::Native(_) | CopyDraftInput::Clipboard(_) => true,
        }
    }

    /// Cached facts only; unknown native kinds still require backend preflight.
    pub(crate) fn metadata_available(&self) -> bool {
        if !(1..=64).contains(&self.input.roots()) {
            return false;
        }
        match &self.input {
            CopyDraftInput::Native(files) => {
                files.selection.folder_count() == 0
                    && !(files.selection.is_empty()
                        && files.fallback.as_ref().is_some_and(|path| {
                            files
                                .view
                                .by_path
                                .get(path)
                                .is_some_and(|id| files.view.is_directory(*id))
                        }))
            }
            CopyDraftInput::Clipboard(_) => true,
            CopyDraftInput::Archive(_) => false,
            CopyDraftInput::Tree { capture, .. } => capture.metadata_available,
        }
    }
}

#[derive(Clone)]
pub(super) enum CopyDraftInput {
    Native(SelectedFiles),
    Archive(Box<ArchiveCopyRequest>),
    Clipboard(Arc<FileClipboard>),
    Tree {
        capture: TreeTransferCapture,
        directory_links_available: bool,
    },
}

struct DraftTransferOptions {
    metadata: TransferMetadataPolicy,
    verification: TransferVerification,
    start_policy: TransferStartPolicy,
    links: Option<CopyLinkPolicy>,
}

fn native_options(request: MetadataTransferRequest, links: Option<CopyLinkPolicy>) -> Command {
    match links {
        Some(links) => Command::EnqueueCopyWithOptions(Box::new(CopyOptionsRequest {
            input: CopyOptionsInput::Native(request),
            links,
        })),
        None => Command::EnqueueTransferWithMetadata(Box::new(request)),
    }
}

impl CopyDraftInput {
    fn roots(&self) -> usize {
        match self {
            Self::Native(files) => files
                .selection
                .len()
                .max(usize::from(files.fallback.is_some())),
            Self::Archive(files) => files
                .selected
                .len()
                .max(usize::from(files.fallback.is_some())),
            Self::Clipboard(files) => files.paths.len(),
            Self::Tree { capture, .. } => capture.root_count,
        }
    }

    fn command(
        self,
        id: OperationId,
        destination: PathBuf,
        intent: TransferIntent,
        options: DraftTransferOptions,
    ) -> Command {
        let DraftTransferOptions {
            metadata,
            verification,
            start_policy,
            links,
        } = options;
        match self {
            Self::Tree { capture, .. } => {
                let request = CapturedTransferRequest {
                    start_policy,
                    verification,
                    id,
                    sources: capture.sources,
                    destination,
                    metadata,
                };
                match links {
                    Some(links) => Command::EnqueueCopyWithOptions(Box::new(CopyOptionsRequest {
                        input: CopyOptionsInput::Captured(request),
                        links,
                    })),
                    None => Command::EnqueueCapturedTransfer(request),
                }
            }
            Self::Native(files) => {
                let request = SelectedCopyRequest {
                    start_policy,
                    verification,
                    id,
                    files,
                    destination,
                };
                if links.is_some() || metadata != TransferMetadataPolicy::ContentOnly {
                    native_options(
                        MetadataTransferRequest {
                            input: MetadataTransferInput::Selected(request),
                            intent,
                            metadata,
                        },
                        links,
                    )
                } else if intent == TransferIntent::Move {
                    Command::EnqueueSelectedMove(request)
                } else {
                    Command::EnqueueSelectedCopy(request)
                }
            }
            Self::Archive(mut request) => {
                request.start_policy = start_policy;
                request.verification = verification;
                request.id = id;
                request.destination = destination;
                request.metadata = metadata;
                Command::EnqueueArchiveCopy(*request)
            }
            Self::Clipboard(clipboard) => {
                let request = ClipboardMoveRequest {
                    start_policy,
                    verification,
                    id,
                    clipboard,
                    destination,
                };
                if links.is_some() || metadata != TransferMetadataPolicy::ContentOnly {
                    native_options(
                        MetadataTransferRequest {
                            input: MetadataTransferInput::Clipboard(request),
                            intent,
                            metadata,
                        },
                        links,
                    )
                } else {
                    Command::EnqueueClipboardMove(request)
                }
            }
        }
    }
}

impl Presenter {
    pub(crate) fn request_copy_dialog(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        self.request_transfer_dialog(pane, TransferIntent::Copy, send);
    }
    pub(crate) fn request_move_dialog(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        self.request_transfer_dialog(pane, TransferIntent::Move, send);
    }
    fn request_transfer_dialog(
        &mut self,
        pane: PaneId,
        intent: TransferIntent,
        send: &dyn Fn(Command) -> bool,
    ) {
        // Search results fed to the panel copy or move their marked results.
        if self.feed(pane).is_some() {
            self.feed_action(pane, find::feed::FeedAction::Transfer(intent), send);
            return;
        }
        if self.comparison_refuses(if intent == TransferIntent::Move {
            "Move"
        } else {
            "Copy"
        }) {
            return;
        }
        if self.operations.closing
            || self.copies.draft.is_some()
            || self.copies.tree_pending.is_some()
            || !self.native_action_allowed(pane.other())
        {
            return;
        }
        if !self.owned_command_scope_available() {
            self.desktop.report(
                "Copy/Move queue busy",
                "Wait for the previous transfer request to finish. Try again.".into(),
            );
            return;
        }
        if intent == TransferIntent::Move && self.panes[pane.index()].archive.is_some() {
            self.desktop.report(
                "Move unavailable",
                "Archive members cannot be moved. Use Copy to extract them.".into(),
            );
            return;
        }
        if self.panes[pane.index()].tree_view().is_some() {
            self.request_tree_transfer(pane, intent, send);
            return;
        }
        if self.panes[pane.index()].archive.is_some() {
            self.request_archive_copy_dialog(pane);
            return;
        }
        let context = CopyContext {
            source: self.panes[pane.index()].display_path().to_owned(),
            destination: self.panes[pane.other().index()].display_path().to_owned(),
            intent,
        };
        self.request_selection_action(
            pane,
            selection_actions::SelectionIntent::CopyDialog(context),
            send,
        );
    }

    pub(super) fn open_captured_copy(
        &mut self,
        request: DesktopRequestId,
        context: CopyContext,
        files: SelectedFiles,
    ) {
        if self.operations.closing || self.copies.draft.is_some() {
            self.retired.push(Box::new(files));
            return;
        }
        let items = DraftItems::selected(&files);
        self.copies.draft = Some(CopyDraft {
            token: format!("copy-draft:{}", request.0),
            input: CopyDraftInput::Native(files),
            source: context.source.to_string_lossy().into_owned(),
            destination: context.destination,
            items,
            error: String::new(),
            intent: context.intent,
            retire_requested: false,
        });
    }

    /// Test shorthand: confirm content only, starting at once.
    #[cfg(test)]
    pub(crate) fn confirm_copy_dialog(
        &mut self,
        token: &str,
        destination: &str,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.confirm_copy_dialog_metadata(
            token,
            destination,
            TransferMetadataPolicy::ContentOnly,
            send,
        );
    }

    /// Test shorthand: confirm with `metadata`, starting at once. The Copy/Move form
    /// confirms through [`Self::confirm_copy_dialog_with_directory_links`].
    #[cfg(test)]
    pub(crate) fn confirm_copy_dialog_metadata(
        &mut self,
        token: &str,
        destination: &str,
        metadata: TransferMetadataPolicy,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.confirm_copy_dialog_options(
            token,
            destination,
            metadata,
            TransferStartPolicy::Immediate,
            send,
        );
    }

    #[cfg(test)]
    pub(crate) fn confirm_copy_dialog_options(
        &mut self,
        token: &str,
        destination: &str,
        metadata: TransferMetadataPolicy,
        start_policy: TransferStartPolicy,
        send: &dyn Fn(Command) -> bool,
    ) {
        self.confirm_copy_dialog_with_directory_links(
            token,
            destination,
            metadata,
            start_policy,
            None,
            send,
        );
    }

    pub(crate) fn confirm_copy_dialog_with_directory_links(
        &mut self,
        token: &str,
        destination: &str,
        metadata: TransferMetadataPolicy,
        start_policy: TransferStartPolicy,
        links: Option<CopyLinkPolicy>,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.copies.destination_picker.is_some() {
            return;
        }
        if !self.owned_command_scope_available() {
            self.copy_dialog_reclaim_busy(token);
            return;
        }
        let Some(draft) = self
            .copies
            .draft
            .as_mut()
            .filter(|draft| draft.token == token && !draft.retire_requested)
        else {
            return;
        };
        if metadata == TransferMetadataPolicy::RequireModifiedTime && !draft.metadata_available() {
            draft.error = "Exact modified time requires at most 64 native regular files. Folders, links and archive members are unsupported.".into();
            return;
        }
        let destination = match picker::resolve_destination(
            &draft.destination,
            self.copies.picked_destination.as_ref(),
            destination,
        ) {
            Ok(path) => path,
            Err(message) => {
                draft.error = message.into();
                return;
            }
        };
        let intent = draft.intent;
        if let CopyDraftInput::Tree { capture, .. } = &draft.input
            && (capture.intent != intent || capture.sources.intent() != intent)
        {
            draft.error = "The operation cannot change after its sources are captured.".into();
            return;
        }
        if intent == TransferIntent::Move && matches!(draft.input, CopyDraftInput::Archive(_)) {
            draft.error = "These captured items support Copy only.".into();
            return;
        }
        // The folder the dialog proposed (the other panel's) and a panel's folder now
        // exist. A typed folder is checked first, so that a missing one is refused
        // here instead of starting a job that cannot run.
        let known = destination == draft.destination
            || self.panes.iter().any(|pane| pane.path == destination);
        if known {
            // A Move from another drive to a destination that refuses one does not start.
            if let Some(message) = self.copy_move_refusal(&destination, None) {
                if let Some(draft) = self.copies.draft.as_mut() {
                    draft.error = message;
                }
                return;
            }
            // Keeping links where none can be stored is explained once before starting.
            if let Some(message) = self.copy_links_hold(&destination, links, None) {
                if let Some(draft) = self.copies.draft.as_mut() {
                    draft.error = message;
                }
                return;
            }
            self.submit_copy_draft(destination, metadata, start_policy, links, send);
        } else {
            self.check_copy_destination(token, destination, metadata, start_policy, links, send);
        }
    }

    fn submit_copy_draft(
        &mut self,
        destination: PathBuf,
        metadata: TransferMetadataPolicy,
        start_policy: TransferStartPolicy,
        links: Option<CopyLinkPolicy>,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(draft) = self.copies.draft.as_ref() else {
            return;
        };
        let intent = draft.intent;
        let links = links.filter(|_| draft.directory_link_options_available());
        let input = draft.input.clone();
        let roots = input.roots() as u64;
        let target = destination.clone();
        let options = DraftTransferOptions {
            metadata,
            verification: self.transfer_verification(),
            start_policy,
            links,
        };
        if self.submit_transfer(
            destination,
            roots,
            intent,
            move |id| input.command(id, target, intent, options),
            send,
        ) {
            self.retire_copy_draft();
        }
    }

    /// Ask the backend whether `path` is an existing folder; its answer resumes the
    /// confirmation (`transfer_destination_checked`). The dialog stays as it is.
    fn check_copy_destination(
        &mut self,
        token: &str,
        path: PathBuf,
        metadata: TransferMetadataPolicy,
        start_policy: TransferStartPolicy,
        links: Option<CopyLinkPolicy>,
        send: &dyn Fn(Command) -> bool,
    ) {
        if let Some(check) = self
            .copies
            .destination_check
            .as_mut()
            .filter(|check| check.token == token && check.path == path)
        {
            // OK again while the same folder is checked: one answer, the latest choices.
            check.metadata = metadata;
            check.start_policy = start_policy;
            check.links = links;
            return;
        }
        if let Some(previous) = self.copies.destination_check.take() {
            previous.cancel.cancel();
        }
        let request = self.desktop.request();
        let cancel = Cancellation::default();
        let sent = send(Command::CheckTransferDestination(DestinationCheckRequest {
            request,
            path: path.clone(),
            // A link choice needs the folder's own link facts for its default and warning.
            links: links.is_some(),
            source: self.copy_draft_links_source(),
            cancel: cancel.clone(),
        }));
        if !sent {
            if let Some(draft) = self.copies.draft.as_mut() {
                draft.error = "The Copy/Move queue is busy. Try again.".into();
            }
            return;
        }
        if let Some(draft) = self.copies.draft.as_mut() {
            draft.error.clear();
        }
        self.copies.destination_check = Some(DestinationCheck {
            token: token.to_owned(),
            request,
            path,
            cancel,
            metadata,
            start_policy,
            links,
        });
    }

    /// The backend's answer for the destination the dialog is checking. A folder lets
    /// the transfer start; a missing path or something else keeps the dialog open with
    /// its own text. When the backend cannot tell (no permission to look, an
    /// unsupported provider), the transfer starts and reports what stops it.
    pub(super) fn transfer_destination_checked(
        &mut self,
        checked: DestinationCheckResult,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(check) = self
            .copies
            .destination_check
            .take_if(|check| check.request == checked.request && check.path == checked.path)
        else {
            return;
        };
        if !self
            .copies
            .draft
            .as_ref()
            .is_some_and(|draft| draft.token == check.token && !draft.retire_requested)
        {
            return;
        }
        let folder = match &checked.result {
            Ok(folder) => *folder,
            Err(error) => !matches!(
                error.kind,
                std::io::ErrorKind::NotFound | std::io::ErrorKind::NotADirectory
            ),
        };
        if !folder {
            if let Some(draft) = self.copies.draft.as_mut() {
                draft.error = DESTINATION_FOLDER_REQUIRED.into();
            }
        } else if !self.owned_command_scope_available() {
            self.copy_dialog_reclaim_busy(&check.token);
        } else if let Some(message) = self.copy_move_refusal(&check.path, checked.links.as_ref()) {
            if let Some(draft) = self.copies.draft.as_mut() {
                draft.error = message;
            }
        } else if let Some(message) = self.copy_links_hold(&check.path, check.links, checked.links)
        {
            // The typed folder's link handling is shown, or explained, before starting.
            if let Some(draft) = self.copies.draft.as_mut() {
                draft.error = message;
            }
        } else {
            self.submit_copy_draft(
                check.path,
                check.metadata,
                check.start_policy,
                check.links,
                send,
            );
        }
    }

    fn request_archive_copy_dialog(&mut self, pane: PaneId) {
        let Some(draft) = self.archive_copy_draft(pane) else {
            return;
        };
        // Encrypted members of a locked archive: its password is asked for once first,
        // and the dialog then shows this draft, whatever the panel shows by then.
        if let Some(draft) = self.copy_needs_archive_password(pane, draft) {
            self.open_archive_copy_draft(draft);
        }
    }

    /// Show `draft`, the Copy dialog of an archive selection, unless another transfer
    /// dialog took its place meanwhile.
    pub(super) fn open_archive_copy_draft(&mut self, draft: CopyDraft) {
        if self.operations.closing
            || self.copies.draft.is_some()
            || self.copies.tree_pending.is_some()
        {
            self.retired.push(Box::new(draft));
            return;
        }
        self.copies.draft = Some(draft);
    }

    /// The Copy dialog of the archive selection that F5 names now, whether or not its
    /// password is entered: without it the copy skips the protected members with that
    /// reason. `None` when there is nothing to copy; a notice then says why.
    fn archive_copy_draft(&mut self, pane: PaneId) -> Option<CopyDraft> {
        let p = &self.panes[pane.index()];
        let archive = p.archive.as_ref()?;
        if p.loading || archive.selection_pending || !p.rows_settled() {
            self.desktop.report(
                "Selection still loading",
                "Wait for the archive listing and selection to finish before copying.".into(),
            );
            return None;
        }
        // Without its session there is nothing to copy from.
        archive.location.mount.as_ref()?;
        // The focused row only while the panel shows it (a filter may hide it).
        let focused = (!p.parent_cursor && p.focus_actionable())
            .then_some(p.cursor)
            .flatten()
            .and_then(|id| p.entry_by_id(id));
        if archive.selected.is_empty() && focused.is_none() {
            self.note_filter_hides_focus(pane);
            return None;
        }
        // The panel's own counts of the marked members (settled: the selection is not
        // pending here).
        let items = match focused {
            Some(entry) if archive.selected.is_empty() => DraftItems::entry(entry),
            _ => {
                let (files, folders, bytes, _) =
                    p.archive_selection_summary().unwrap_or((0, 0, None, false));
                DraftItems::counted(files, folders, bytes)
            }
        };
        let destination = self.panes[pane.other().index()].display_path().to_owned();
        let source = archive.location.display_text();
        let input = CopyDraftInput::Archive(Box::new(ArchiveCopyRequest {
            start_policy: TransferStartPolicy::Immediate,
            verification: midnight_core::TransferVerification::default(),
            id: OperationId(0), // Assigned only after the user submits the captured draft.
            location: archive.location.clone(),
            entries: p.entries.clone(),
            view: p.display_index().clone(),
            selected: archive.selected.clone(),
            fallback: focused.map(|entry| entry.id),
            destination: destination.clone(),
            metadata: midnight_core::TransferMetadataPolicy::ContentOnly,
        }));
        let request = self.desktop.request();
        Some(CopyDraft {
            token: format!("copy-draft:{}", request.0),
            input,
            source,
            destination,
            items,
            error: String::new(),
            intent: TransferIntent::Copy,
            retire_requested: false,
        })
    }

    pub(crate) fn cancel_copy_dialog(&mut self, token: &str) {
        if !self.owned_command_scope_available() {
            self.defer_cancel_copy_dialog(token);
            return;
        }
        if self
            .copies
            .draft
            .as_ref()
            .is_some_and(|draft| draft.token == token)
        {
            self.retire_copy_draft();
        }
    }

    pub(crate) fn copy_dialog_reclaim_busy(&mut self, token: &str) -> String {
        let message = "Wait for the previous transfer request to finish. Try again.";
        if let Some(draft) = self
            .copies
            .draft
            .as_mut()
            .filter(|draft| draft.token == token && !draft.retire_requested)
        {
            draft.error = message.into();
        }
        message.into()
    }

    /// Cancellation stays responsive without taking the provider-bearing owner.
    pub(crate) fn defer_cancel_copy_dialog(&mut self, token: &str) {
        if let Some(draft) = self
            .copies
            .draft
            .as_mut()
            .filter(|draft| draft.token == token)
        {
            draft.retire_requested = true;
        }
    }

    /// Called only while the current callback owns the existing reclaim batch.
    pub(crate) fn flush_deferred_copy_draft(&mut self) {
        if !self.owned_command_scope_available() {
            return;
        }
        if self
            .copies
            .draft
            .as_ref()
            .is_some_and(|draft| draft.retire_requested)
        {
            self.retire_copy_draft();
        } else if self
            .copies
            .tree_pending
            .as_ref()
            .is_some_and(|pending| pending.is_cancelled())
        {
            self.cancel_tree_transfer(None);
        }
    }

    pub(super) fn open_clipboard_move(&mut self, clipboard: FileClipboard, destination: PathBuf) {
        if self.operations.closing || self.copies.draft.is_some() {
            self.retired.push(Box::new(clipboard));
            return;
        }
        let request = self.desktop.request();
        let items = DraftItems::clipboard(&clipboard.paths);
        self.copies.draft = Some(CopyDraft {
            token: format!("copy-draft:{}", request.0),
            input: CopyDraftInput::Clipboard(Arc::new(clipboard)),
            source: "Files on clipboard".into(),
            destination,
            items,
            error: String::new(),
            intent: TransferIntent::Move,
            retire_requested: false,
        });
    }

    pub(super) fn retire_copy_draft(&mut self) {
        if !self.owned_command_scope_available() {
            if let Some(draft) = &mut self.copies.draft {
                draft.retire_requested = true;
            }
            self.cancel_tree_transfer(None);
            return;
        }
        self.force_retire_copy_draft();
    }

    /// Only the post-window drain may bypass the live callback's ownership claim.
    pub(super) fn force_retire_copy_draft(&mut self) {
        self.copies.destination_picker = None;
        self.copies.picked_destination = None;
        self.retire_tree_transfer(None);
        if let Some(check) = self.copies.destination_check.take() {
            check.cancel.cancel();
        }
        self.retire_copy_draft_links();
        if let Some(draft) = self.copies.draft.take() {
            self.retired.push(Box::new(draft));
        }
    }
}
