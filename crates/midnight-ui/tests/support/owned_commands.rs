//! Explicit ownership scopes for boolean UI fixtures, not Engine admission tests.
//!
//! The authored boolean sender keeps its existing acceptance/refusal behavior.
//! Retained test-only shared payload anchors protect ownership before invoking it,
//! including an immediately completed or refused send. A bool cannot return the
//! original command: real Full/Closed custody and the fixed four-queued-plus-one-
//! pending bound belong to CommandReclaim tests.
//!
//! This fixture substitutes one shared collection for finite test-action retirements.
//! It releases that collection on one background thread at final fixture teardown,
//! without a runtime, production queue or task per callback/refusal.
use midnight_core::Command;
use midnight_ui::{BeginOwned, DeferredDrop, MainWindow, OwnedCommandScope, Presenter};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
};

#[derive(Default)]
struct FixtureOwner {
    claimed: Cell<bool>,
    retired: RefCell<Vec<DeferredDrop>>,
}

impl Drop for FixtureOwner {
    fn drop(&mut self) {
        let retired = std::mem::take(self.retired.get_mut());
        if retired.is_empty() {
            return;
        }
        let worker = std::thread::spawn(move || drop(retired));
        if let Err(panic) = worker.join()
            && !std::thread::panicking()
        {
            std::panic::resume_unwind(panic);
        }
    }
}

struct FixtureScope {
    owner: Rc<FixtureOwner>,
    send: Rc<dyn Fn(Command) -> bool>,
}

impl OwnedCommandScope for FixtureScope {
    fn send(&self, command: Command) -> bool {
        // Protect the shared owners before the bool sender can drop or refuse
        // the original. Exhaustiveness makes new payload variants need review.
        match &command {
            Command::EnqueueCapturedTransfer(request) => {
                self.retire(Box::new(request.sources.clone()));
            }
            Command::RunFileList(request) => self.retire(Box::new(request.capture.clone())),
            Command::CopyGeneratedText {
                content, cancel, ..
            }
            | Command::ReadDiagnosticText {
                content, cancel, ..
            } => self.retire(Box::new((content.clone(), cancel.clone()))),
            Command::OpenSizeWarnings { report, cancel, .. } => {
                self.retire(Box::new((report.clone(), cancel.clone())));
            }
            Command::Tree(request) => self.retire(Box::new(request.clone())),
            Command::ProbeFolderPresence(request) => self.retire(Box::new(request.clone())),
            Command::NetworkNavigate { action, .. } => self.retire(Box::new(action.clone())),
            Command::PreviewUserCommand(request) | Command::ExecuteUserCommand(request) => {
                self.retire(Box::new(request.clone()));
            }
            Command::StartSelectedChecksums(request) => {
                self.retire(Box::new(request.files.clone()));
            }
            Command::StartCapturedChecksums(request) => {
                self.retire(Box::new(request.sources.clone()));
            }
            Command::ExecuteFindFile { source, cancel, .. } => {
                self.retire(Box::new((source.clone(), cancel.clone())));
            }
            Command::ArchiveSelect {
                entries,
                view,
                selected,
                action,
                ..
            } => self.retire(Box::new((
                entries.clone(),
                view.clone(),
                selected.clone(),
                action.clone(),
            ))),
            Command::ArchiveNavigate { action, .. } => self.retire(Box::new(action.clone())),
            Command::ListComparatorCapabilities { disabled_plugins } => {
                self.retire(Box::new(disabled_plugins.clone()));
            }
            Command::PrepareOperation(request) => self.retire(Box::new(request.clone())),
            Command::PrepareCapturedDelete(request) => {
                self.retire(Box::new(request.sources.clone()));
            }
            Command::PrepareCapturedRename(request) => {
                self.retire(Box::new(request.source.clone()));
            }
            Command::PrepareCapturedMetadata(request) => {
                self.retire(Box::new(request.sources.clone()));
            }
            Command::CompareDirectories(request) => self.retire(Box::new(request.clone())),
            Command::MatchFilenameColors(request) => self.retire(Box::new(request.clone())),
            Command::EnqueueSelectedCopy(request) | Command::EnqueueSelectedMove(request) => {
                self.retire(Box::new(request.files.clone()));
            }
            Command::EnqueueClipboardMove(request) => {
                self.retire(Box::new(request.clipboard.clone()));
            }
            Command::EnqueueTransferWithMetadata(request) => {
                self.retire(Box::new(request.clone()));
            }
            Command::EnqueueCopyWithOptions(request) => self.retire(Box::new(request.clone())),
            Command::EnqueueArchiveCopy(request) => self.retire(Box::new(request.clone())),
            Command::AttachDragDrop { host, targets } => {
                self.retire(Box::new((host.clone(), targets.clone())));
            }
            Command::PrepareDrag { files, .. }
            | Command::CopyFiles { files, .. }
            | Command::CutFiles { files, .. }
            | Command::ShowShellMenu { files, .. } => self.retire(Box::new(files.clone())),
            Command::PrepareCapturedDrag { source, .. } => {
                self.retire(Box::new(source.clone()));
            }
            Command::CopyCapturedFiles { source, cancel, .. }
            | Command::ShowCapturedShellMenu { source, cancel, .. }
            | Command::ShowCapturedProperties { source, cancel, .. }
            | Command::OpenCapturedEntry { source, cancel, .. }
            | Command::ReadCapturedFileClipboard { source, cancel, .. } => {
                self.retire(Box::new((source.clone(), cancel.clone())));
            }
            Command::InternalDrop(request) => self.retire(Box::new(request.clone())),
            Command::PreviewInternalDrop(request) => self.retire(Box::new(request.clone())),
            Command::CompleteInternalDropPreview { target, .. }
            | Command::CompleteInternalDrop { target, .. } => {
                self.retire(Box::new(target.clone()));
            }
            Command::Search(request) => self.retire(Box::new(request.source.clone())),
            Command::SummarizeRectangle(request) => self.retire(Box::new(request.clone())),
            Command::Selection(request) => self.retire(Box::new(request.clone())),
            Command::EditCapturedFile { source, cancel, .. } => {
                self.retire(Box::new((source.clone(), cancel.clone())));
            }
            Command::SavePreferences { preferences, .. } => self.retire(Box::new((
                preferences.user_commands.clone(),
                preferences.viewer_associations.clone(),
                preferences.thumbnailer_associations.clone(),
                preferences.filename_colors.clone(),
            ))),
            Command::ExportPreferences(request) => self.retire(Box::new((
                request.preferences.user_commands.clone(),
                request.preferences.viewer_associations.clone(),
                request.preferences.thumbnailer_associations.clone(),
                request.preferences.filename_colors.clone(),
            ))),
            Command::PaneCommitted(source) => self.retire(Box::new(source.entries.clone())),
            Command::QueryVolumeInformation(request) => {
                self.retire(Box::new(request.cancel.clone()));
            }
            Command::CheckTransferDestination(request) => {
                self.retire(Box::new(request.cancel.clone()));
            }
            Command::InspectTransferLinks(request) => {
                self.retire(Box::new(request.cancel.clone()));
            }
            Command::ShowSelectionProperties {
                selection, view, ..
            } => self.retire(Box::new((selection.clone(), view.clone()))),
            Command::ComputeFolderSizes { selection, .. } => {
                if let Some(selection) = selection {
                    self.retire(Box::new(selection.clone()));
                }
            }
            Command::ArchiveSizes(request) => self.retire(Box::new(request.clone())),
            Command::SummarizeSelectedFolders(request) => {
                self.retire(Box::new(request.selection.clone()));
            }
            Command::Resort { source, .. } => {
                if let Some(source) = source {
                    self.retire(Box::new(source.entries.clone()));
                }
            }
            // Core's ExternalCompareOwnedPayload contract already protects Drop
            // on any thread, including refused commands. Forward its unique
            // armed owner unchanged; it cannot be cloned or reconstructed here.
            Command::LaunchExternalComparison(_) => {}
            // These carry only values/private allocations, with no shared owner
            // needing a last-reference anchor. A deep copy would not protect
            // destruction of the original allocation by a boolean sender.
            Command::SetCommandIconView { .. }
            | Command::SetFileIconView { .. }
            | Command::SetIconEdges(_)
            | Command::ConfirmFileList { .. }
            | Command::CancelFileList { .. }
            | Command::CancelUserCommand(_)
            | Command::StartChecksums(_)
            | Command::VerifyChecksumManifest(_)
            | Command::CancelChecksums { .. }
            | Command::ReleaseChecksums { .. }
            | Command::SaveChecksumManifest { .. }
            | Command::CancelChecksumManifest { .. }
            | Command::StartFind(_)
            | Command::SortFind(_)
            | Command::CancelFind { .. }
            | Command::CloseFind(_)
            | Command::ValidateFindResult(_)
            | Command::CaptureFindFile(_)
            | Command::ListViewerPlugins
            | Command::LoadLocale
            | Command::ManageNetworkConnections { .. }
            | Command::ConfirmOperation { .. }
            | Command::CancelOperation(_)
            | Command::PreviewMetadata { .. }
            | Command::ConfirmMetadata { .. }
            | Command::ResolveMetadataConflict { .. }
            | Command::CancelComparison(_)
            | Command::ValidatePanelFilter(_)
            | Command::ValidateSelectionMask(_)
            | Command::ValidateFilenameColors(_)
            | Command::ClearFilenameColors { .. }
            | Command::EnqueueCopy(_)
            | Command::EnqueueMove(_)
            | Command::DecideCopy(_)
            | Command::DecideRename(_)
            | Command::DecideDelete(_)
            | Command::CancelCopy(_)
            | Command::CopyCancelConfirmation { .. }
            | Command::SetCopyPaused { .. }
            | Command::SetOperationPaused { .. }
            | Command::DetachDragDrop { .. }
            | Command::StartDrag(_)
            | Command::CancelDrag(_)
            | Command::AcceptDrop { .. }
            | Command::RejectDrop(_)
            | Command::CancelSearch { .. }
            | Command::ShowHiddenEntries(_)
            | Command::ListVolumes
            | Command::ListDesktopContext
            | Command::OpenSystemLocation { .. }
            | Command::OpenUrl { .. }
            | Command::LoadPreferences
            | Command::ImportPreferences { .. }
            | Command::ListFontFamilies
            | Command::UnlockArchive { .. }
            | Command::ReadLicenseNotice(_)
            | Command::Installation(_)
            | Command::ResolveDefaultEditor
            | Command::EditFile { .. }
            | Command::LaunchTerminal { .. }
            | Command::CopyText { .. }
            | Command::ReadPathClipboardText { .. }
            | Command::PickPath { .. }
            | Command::CancelPathPicker { .. }
            | Command::ReadFileClipboard { .. }
            | Command::ShowProperties { .. }
            | Command::OpenEntry { .. }
            | Command::ResolveDirectoryLink { .. }
            | Command::ReadDirectoryLinkTarget { .. }
            | Command::ComputeEntrySize { .. }
            | Command::CancelFolderSizes { .. }
            | Command::RetryDirectoryWithElevation { .. }
            | Command::Navigate { .. }
            | Command::Refresh { .. }
            | Command::Shutdown => {}
        }
        (self.send)(command)
    }

    fn retire(&self, value: DeferredDrop) {
        self.owner.retired.borrow_mut().push(value);
    }
}

impl Drop for FixtureScope {
    fn drop(&mut self) {
        self.owner.claimed.set(false);
    }
}

/// Return the same factory installed on the window for explicit backend-event calls.
pub fn install(
    window: &MainWindow,
    presenter: Rc<RefCell<Presenter>>,
    send: Rc<dyn Fn(Command) -> bool>,
) -> BeginOwned {
    let owner = Rc::new(FixtureOwner::default());
    let action_send = send.clone();
    let begin: BeginOwned = Rc::new(move || {
        if owner.claimed.replace(true) {
            return None;
        }
        Some(Box::new(FixtureScope {
            owner: owner.clone(),
            send: action_send.clone(),
        }))
    });
    midnight_ui::install_with_owned(window, presenter, send, begin.clone());
    begin
}

/// Scope a direct fixture event/action just as an installed owned callback does.
pub fn run<R>(
    presenter: &Rc<RefCell<Presenter>>,
    begin: &BeginOwned,
    action: impl FnOnce(&mut Presenter, &dyn Fn(Command) -> bool) -> R,
) -> R {
    midnight_ui::with_owned_commands(presenter, begin, action)
        .expect("a direct fixture action must obtain its non-reentrant scope")
}
