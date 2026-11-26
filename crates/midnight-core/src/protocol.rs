use crate::{
    DesktopActionResult, DesktopRequestId, DirectoryIndex, Entry, EntryId, EntryMetadata,
    FolderSize, FolderSizeScanId, FsError, IconKey, IconPixels, NavigationLocation,
    PreferencesRevision, SearchRequest, SearchResult, SearchRevision, SelectionRequest,
    SelectionRevision, SelectionSnapshot, ShellFlavor, SortSpec, SpaceInfo, TerminalRequest,
    UserPreferences, VisibleDirectory, Volume, VolumeDetails,
};
use std::{path::PathBuf, sync::Arc};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PaneId {
    Left,
    Right,
}

impl PaneId {
    pub const fn index(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
    pub const fn other(self) -> Self {
        match self {
            Self::Left => Self::Right,
            Self::Right => Self::Left,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NavigationId(pub u64);

/// Shared selection captured at the instant of the user's action. Expansion belongs to workers.
#[derive(Clone, Debug)]
pub struct SelectedFiles {
    pub selection: Arc<SelectionSnapshot>,
    pub view: Arc<DirectoryIndex>,
    /// Used only when the captured selection is empty, never when selected entries disappeared.
    pub fallback: Option<PathBuf>,
}

#[derive(Debug)]
pub enum Command {
    SetCommandIconView {
        generation: u64,
        targets: Vec<crate::CommandIconTarget>,
    },
    /// The visible tiles whose files carry their own artwork and still need it at
    /// their edge. A newer generation replaces and cancels the previous view.
    SetFileIconView {
        generation: u64,
        targets: Vec<crate::FileIconTarget>,
    },
    /// The device-pixel edges at which the frontend now draws native icons.
    SetIconEdges(crate::IconEdges),
    EnqueueCapturedTransfer(crate::CapturedTransferRequest),
    RunFileList(Box<crate::FileListRequest>),
    ConfirmFileList {
        id: DesktopRequestId,
        generation: u64,
    },
    CancelFileList {
        id: DesktopRequestId,
        generation: u64,
    },
    CopyGeneratedText {
        request: DesktopRequestId,
        content: Arc<dyn crate::GeneratedTextSource>,
        cancel: crate::Cancellation,
    },
    ReadDiagnosticText {
        request: DesktopRequestId,
        content: Arc<dyn crate::GeneratedTextSource>,
        cancel: crate::Cancellation,
    },
    OpenSizeWarnings {
        request: DesktopRequestId,
        report: crate::SizeWarningReport,
        cancel: crate::Cancellation,
    },
    Tree(Box<crate::TreeCommand>),
    ProbeFolderPresence(Arc<crate::FolderPresenceRequest>),
    NetworkNavigate {
        pane: PaneId,
        navigation: NavigationId,
        action: crate::NetworkAction,
        sort: SortSpec,
    },
    PreviewUserCommand(Box<crate::UserCommandRequest>),
    ExecuteUserCommand(Box<crate::UserCommandRequest>),
    CancelUserCommand(crate::UserCommandRequestId),
    /// Save an output window's text (Save as…); answered by Event::UserCommandOutputSaved.
    SaveUserCommandOutput(Box<crate::UserCommandOutputSaveRequest>),
    StartChecksums(Box<crate::ChecksumRequest>),
    StartSelectedChecksums(Box<crate::SelectedChecksumRequest>),
    StartCapturedChecksums(Box<crate::CapturedChecksumRequest>),
    VerifyChecksumManifest(Box<crate::ChecksumVerifyRequest>),
    CancelChecksums {
        job: crate::ChecksumJobId,
        generation: crate::ChecksumGeneration,
    },
    ReleaseChecksums {
        job: crate::ChecksumJobId,
        generation: crate::ChecksumGeneration,
    },
    SaveChecksumManifest {
        save_id: crate::ChecksumManifestId,
        job: crate::ChecksumJobId,
        generation: crate::ChecksumGeneration,
        request: Box<crate::ChecksumManifestRequest>,
    },
    CancelChecksumManifest {
        save_id: crate::ChecksumManifestId,
    },
    StartFind(Box<crate::FindRequest>),
    SortFind(crate::FindSortRequest),
    CancelFind {
        session: crate::FindSessionId,
        generation: crate::FindGeneration,
    },
    CloseFind(crate::FindSessionId),
    ValidateFindResult(crate::FindValidationRequest),
    CaptureFindFile(crate::FindValidationRequest),
    ExecuteFindFile {
        request: crate::DesktopRequestId,
        source: Arc<dyn crate::CapturedFindFile>,
        action: crate::FindFileOperation,
        cancel: crate::Cancellation,
    },
    ArchiveSelect {
        pane: PaneId,
        navigation: NavigationId,
        revision: crate::SelectionRevision,
        entries: Arc<[Entry]>,
        view: Arc<DirectoryIndex>,
        selected: Arc<std::collections::HashSet<EntryId>>,
        /// The totals of `selected`, which a removal updates member by member.
        totals: crate::ArchiveSelectionTotals,
        action: crate::ArchiveSelectionAction,
    },
    ArchiveNavigate {
        pane: PaneId,
        navigation: NavigationId,
        action: crate::ArchiveAction,
        sort: SortSpec,
        /// The session (`ArchiveMount::id`) of an archive opened from a native file that
        /// this tab let go of for this navigation and that no other tab, panel or viewer
        /// holds. When the navigation opens another such archive, it takes that session's
        /// place once the session's teardown ends, instead of counting beside it.
        replaces: Option<u64>,
    },
    /// Verify the password of one archive mount on a backend worker
    /// (`ArchiveMount::unlock`), answered by `Event::ArchiveUnlocked`. The password
    /// is zeroized when the command is dropped and never enters an event.
    UnlockArchive {
        request: DesktopRequestId,
        mount: Arc<dyn crate::ArchiveMount>,
        password: crate::ArchivePassword,
    },
    ListViewerPlugins,
    /// Metadata-only effective image coverage for the established startup-disabled snapshot.
    ListComparatorCapabilities {
        disabled_plugins: Arc<[String]>,
    },
    LoadLocale,
    ManageNetworkConnections {
        disconnect: bool,
    },
    PrepareOperation(crate::PrepareOperation),
    PrepareCapturedDelete(crate::PrepareCapturedDelete),
    PrepareCapturedRename(crate::PrepareCapturedRename),
    PrepareCapturedMetadata(crate::PrepareCapturedMetadata),
    ConfirmOperation {
        id: crate::OperationId,
        /// Basename for Rename/CreateFile, relative path for CreateDirectory, absent for Delete.
        new_name: Option<String>,
        /// The user confirmed replacing an existing file with the new empty file.
        replace_existing: bool,
    },
    CancelOperation(crate::OperationId),
    PreviewMetadata {
        id: crate::OperationId,
        revision: u64,
        patch: crate::MetadataPatch,
    },
    ConfirmMetadata {
        id: crate::OperationId,
        revision: u64,
        conflicts: crate::MetadataConflictPolicy,
    },
    ResolveMetadataConflict {
        id: crate::OperationId,
        token: u64,
        choice: crate::MetadataConflictPolicy,
    },
    CompareDirectories(crate::CompareRequest),
    CancelComparison(crate::CompareId),
    ValidatePanelFilter(crate::FilterValidationRequest),
    /// Whether a Select/Deselect by mask pattern compiles within the selection
    /// workers' budget, answered by `Event::SelectionMaskValidated`.
    ValidateSelectionMask(crate::MaskValidationRequest),
    /// Rebuild a fed result view on a bounded, coalescing worker for its pane.
    ProjectFindFeed(Arc<crate::FindFeedProjectionRequest>),
    BuildFindFeed(Arc<crate::FindFeedBuildRequest>),
    ComputeFindFeedSizes(Arc<crate::FindFeedSizeRequest>),
    ValidateFilenameColors(crate::FilenameColorsValidation),
    MatchFilenameColors(crate::FilenameColorsRequest),
    ClearFilenameColors {
        pane: PaneId,
        id: u64,
    },
    /// Whether a typed Copy/Move destination is an existing folder, answered by
    /// `Event::TransferDestinationChecked` on a backend worker.
    CheckTransferDestination(crate::DestinationCheckRequest),
    /// What the Copy/Move dialog's destination does with links, for its default link
    /// handling, answered by `Event::TransferLinksInspected` on a backend worker.
    InspectTransferLinks(crate::TransferLinksRequest),
    EnqueueCopy(crate::CopyRequest),
    EnqueueSelectedCopy(crate::SelectedCopyRequest),
    EnqueueMove(crate::CopyRequest),
    EnqueueSelectedMove(crate::SelectedCopyRequest),
    EnqueueClipboardMove(crate::ClipboardMoveRequest),
    EnqueueTransferWithMetadata(Box<crate::MetadataTransferRequest>),
    EnqueueCopyWithOptions(Box<crate::CopyOptionsRequest>),
    EnqueueArchiveCopy(crate::ArchiveCopyRequest),
    DecideCopy(crate::CopyDecision),
    DecideRename(crate::CopyDecision),
    /// Answers a `DeleteFailure` of the same job and version: Retry, Skip,
    /// SkipAllErrors (skip this and every later failure of the job without asking)
    /// or Cancel. Other choices are ignored.
    DecideDelete(crate::CopyDecision),
    /// Answers a versioned permanent-delete offer for the job's pinned, unrecycled items.
    AnswerTrashFallback {
        id: crate::OperationId,
        version: u64,
        confirm: bool,
    },
    CancelCopy(crate::OperationId),
    CopyCancelConfirmation {
        id: crate::OperationId,
        token: u64,
        action: crate::CopyCancelConfirmationAction,
    },
    SetCopyPaused {
        id: crate::OperationId,
        paused: bool,
    },
    SetOperationPaused {
        id: crate::OperationId,
        paused: bool,
    },
    AttachDragDrop {
        host: crate::DragHost,
        targets: crate::SharedDropTargets,
    },
    DetachDragDrop {
        host_generation: u64,
    },
    PrepareDrag {
        id: crate::DragId,
        pane: PaneId,
        navigation: NavigationId,
        files: SelectedFiles,
    },
    PrepareCapturedDrag {
        id: crate::DragId,
        pane: PaneId,
        navigation: NavigationId,
        source: Arc<dyn crate::CapturedClipboardFiles>,
    },
    /// Hand the prepared drag to the operating system. A release before `earliest_drop`
    /// (the press that started the drag plus the minimum drag duration) cancels it there.
    StartDrag {
        id: crate::DragId,
        earliest_drop: std::time::Instant,
    },
    CancelDrag(crate::DragId),
    AcceptDrop {
        verification: crate::TransferVerification,
        token: crate::DropToken,
        id: crate::OperationId,
    },
    RejectDrop(crate::DropToken),
    InternalDrop(Box<crate::InternalDropRequest>),
    PreviewInternalDrop(Box<crate::InternalDropPreviewRequest>),
    CompleteInternalDropPreview {
        id: crate::DragId,
        revision: crate::DragPreviewRevision,
        target: Arc<dyn crate::CapturedDropDirectory>,
    },
    CompleteInternalDrop {
        id: crate::DragId,
        target: Arc<dyn crate::CapturedDropDirectory>,
    },
    Search(SearchRequest),
    CancelSearch {
        pane: PaneId,
        revision: SearchRevision,
    },
    SummarizeRectangle(Arc<crate::RectangleSummaryRequest>),
    Selection(SelectionRequest),
    ListVolumes,
    /// Read the free space of the folder `pane` shows for `navigation` again: a job or
    /// another program may have changed it. The backend coalesces these requests,
    /// starts a read no sooner than a second after the pane's previous one ended,
    /// reports only a changed value and ignores a request for another navigation.
    RefreshSpace {
        pane: PaneId,
        navigation: NavigationId,
    },
    /// Whether the main window is visible (not minimized). While it is, the backend
    /// also reads a fixed local disk's free space periodically for the panes.
    SetWindowVisible {
        visible: bool,
    },
    ListDesktopContext,
    OpenSystemLocation {
        request: DesktopRequestId,
        location: crate::SystemLocation,
    },
    /// Hand a followed web or mail link to the system's default handler on a backend
    /// worker (`OpenService::open_url`). Answered by `Event::DesktopActionFinished`:
    /// `DesktopActionResult::UrlOpened` once the handler took it, or the reason it did not.
    OpenUrl {
        request: DesktopRequestId,
        url: crate::ExternalUrl,
    },
    LoadPreferences,
    /// Already armed backend ownership; all sender/permit paths must use guarded admission.
    LaunchExternalComparison(crate::ExternalCompareRequest),
    ListFontFamilies,
    /// The About dialog's notices for one library; a newer request replaces a queued one.
    ReadLicenseNotice(Box<crate::LicenseNoticeRequest>),
    /// A button of the shared-installation notice.
    Installation(crate::InstallationRequest),
    ResolveDefaultEditor,
    EditCapturedFile {
        request: DesktopRequestId,
        source: Arc<dyn crate::CapturedEditorFile>,
        editor: crate::EditorPreferences,
        cancel: crate::Cancellation,
    },
    EditFile {
        request: DesktopRequestId,
        path: PathBuf,
        editor: crate::EditorPreferences,
    },
    SavePreferences {
        revision: PreferencesRevision,
        preferences: Box<UserPreferences>,
    },
    /// Export configuration: write one settings file on a backend worker.
    ExportPreferences(Box<crate::PreferencesExportRequest>),
    /// Import configuration: read and completely validate one settings file. The
    /// result changes nothing until the UI stages it in its Options transaction.
    ImportPreferences {
        request: DesktopRequestId,
        source: PathBuf,
    },
    /// Acknowledge the listing the UI actually accepted; queued stale listings do not count.
    PaneCommitted(VisibleDirectory),
    LaunchTerminal {
        request: DesktopRequestId,
        terminal: TerminalRequest,
    },
    CopyText {
        request: DesktopRequestId,
        text: String,
    },
    ReadPathClipboardText {
        request: DesktopRequestId,
    },
    PickPath {
        request: DesktopRequestId,
        input: crate::PathPickerRequest,
    },
    CancelPathPicker {
        request: DesktopRequestId,
    },
    CopyFiles {
        request: DesktopRequestId,
        files: SelectedFiles,
    },
    CopyCapturedFiles {
        request: DesktopRequestId,
        source: Arc<dyn crate::CapturedClipboardFiles>,
        cancel: crate::Cancellation,
    },
    CutFiles {
        request: DesktopRequestId,
        files: SelectedFiles,
    },
    ReadFileClipboard {
        request: DesktopRequestId,
        destination: PathBuf,
    },
    ShowShellMenu {
        request: DesktopRequestId,
        files: SelectedFiles,
        owner: usize,
        position: crate::ShellMenuPosition,
        modifiers: crate::ShellMenuModifiers,
        target: crate::ShellMenuTarget,
    },
    /// Resolve retained native tree targets on a backend worker before opening their menu.
    ShowCapturedShellMenu {
        request: DesktopRequestId,
        source: Arc<dyn crate::CapturedClipboardFiles>,
        cancel: crate::Cancellation,
        owner: usize,
        position: crate::ShellMenuPosition,
        modifiers: crate::ShellMenuModifiers,
    },
    /// Retain provider ownership through the native Properties operation.
    ShowCapturedProperties {
        request: DesktopRequestId,
        source: Arc<dyn crate::CapturedClipboardFiles>,
        cancel: crate::Cancellation,
    },
    /// Resolve the exact captured occurrence before invoking its native association.
    OpenCapturedEntry {
        request: DesktopRequestId,
        pane: PaneId,
        navigation: NavigationId,
        row: crate::TreeRowId,
        source: Arc<dyn crate::CapturedClipboardFiles>,
        cancel: crate::Cancellation,
    },
    /// Read the clipboard for one retained native folder, never a display path.
    ReadCapturedFileClipboard {
        request: DesktopRequestId,
        source: Arc<dyn crate::CapturedClipboardFiles>,
        cancel: crate::Cancellation,
    },
    ShowProperties {
        request: DesktopRequestId,
        paths: Vec<PathBuf>,
    },
    QueryVolumeInformation(crate::VolumeInformationRequest),
    /// Expand the immutable selected paths on a backend worker, never in UI.
    /// `cancel` stops the background count of a single folder's contents that a
    /// metadata summary continues with (`Event::PropertiesUpdated`).
    ShowSelectionProperties {
        request: DesktopRequestId,
        selection: Arc<SelectionSnapshot>,
        view: Arc<DirectoryIndex>,
        fallback: Option<PathBuf>,
        cancel: crate::Cancellation,
    },
    OpenEntry {
        request: DesktopRequestId,
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
    },
    /// Directory-only activation must never invoke a host file association,
    /// including when the captured link has since been replaced with a file.
    ResolveDirectoryLink {
        request: DesktopRequestId,
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
    },
    /// Read the stored target of the exact native directory-link row under the pointer.
    /// The caller scopes the token to the row, view revision and active tab.
    ReadDirectoryLinkTarget {
        request: DesktopRequestId,
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
    },
    ComputeFolderSizes {
        pane: PaneId,
        navigation: NavigationId,
        scan: FolderSizeScanId,
        /// None scans all ordinary directories. Some captures an exact selection;
        /// an empty or files-only snapshot never falls back to all directories.
        selection: Option<Arc<SelectionSnapshot>>,
    },
    ArchiveSizes(crate::ArchiveSizeCommand),
    /// Observe cached and future folder sizes for this immutable selection.
    SummarizeSelectedFolders(crate::SelectionSizeRequest),
    ComputeEntrySize {
        pane: PaneId,
        navigation: NavigationId,
        scan: FolderSizeScanId,
        entry: EntryId,
    },
    CancelFolderSizes {
        pane: PaneId,
        scan: FolderSizeScanId,
    },
    /// Explicit retry of a backend-retained denied navigation. UI supplies no privileged path.
    RetryDirectoryWithElevation {
        pane: PaneId,
        denied_navigation: NavigationId,
        navigation: NavigationId,
    },
    Navigate {
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        sort: SortSpec,
    },
    /// Reload the same pane location, optionally continuing its full scan or latest
    /// individual scan still tracked by the UI, including completion events in flight.
    /// A new tab or different location uses Navigate, even for an identical path.
    Refresh {
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        sort: SortSpec,
        scan: Option<FolderSizeScanId>,
    },
    /// Sort the committed in-memory listing on a backend worker.
    /// A normal Navigate/Refresh requests a fresh filesystem listing instead.
    Resort {
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        sort: SortSpec,
        source: Option<VisibleDirectory>,
    },
    /// Whether later listings, archive listings and inline-tree projections of
    /// both panes include hidden entries. Listings already shown keep their rows
    /// until the UI re-reads them; a Resort reuses its rows as they were listed.
    ShowHiddenEntries(bool),
    Shutdown,
}

/// One step of `Event::ListingChanges`. Rows are positions in the listing as
/// it is after the preceding steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RowEdit {
    /// `count` rows starting at `row` are gone.
    Removed { row: usize, count: usize },
    /// `count` new rows are inserted at `row`.
    Added { row: usize, count: usize },
    /// The entry at `row` kept its identity but shows other metadata.
    Changed { row: usize },
}

#[derive(Clone, Debug)]
pub enum Event {
    RectangleSummary {
        pane: PaneId,
        token: Arc<()>,
        totals: crate::ArchiveSelectionTotals,
    },
    FileList(Box<crate::FileListEvent>),
    ArchiveSizes(Box<crate::ArchiveSizeEvent>),
    Tree(Box<crate::TreeEvent>),
    FolderPresence(Box<crate::FolderPresenceSnapshot>),
    NetworkAvailability(bool),
    NetworkUpdated {
        pane: PaneId,
        navigation: NavigationId,
        update: Box<crate::NetworkUpdate>,
    },
    UserCommand(Box<crate::UserCommandEvent>),
    /// A Run directly command with an output window has one, or news for it.
    UserCommandOutput(Box<crate::UserCommandOutputEvent>),
    UserCommandOutputSaved {
        request: DesktopRequestId,
        result: Result<crate::UserCommandOutputSaveReceipt, FsError>,
    },
    Checksum(Box<crate::ChecksumEvent>),
    ChecksumRejected(Box<crate::ChecksumRejected>),
    ChecksumManifest(Box<crate::ChecksumManifestEvent>),
    Find(Box<crate::FindEvent>),
    FindRejected(Box<crate::FindRejected>),
    FindResultValidated(Box<crate::FindValidationResult>),
    FindFileCaptured(Box<crate::FindFileCaptureResult>),
    ArchiveSelection {
        pane: PaneId,
        navigation: NavigationId,
        revision: crate::SelectionRevision,
        view: Arc<DirectoryIndex>,
        result: Result<crate::ArchiveSelectionResult, FsError>,
    },
    ArchiveListed {
        pane: PaneId,
        navigation: NavigationId,
        result: Result<crate::ArchiveListing, FsError>,
    },
    /// The answer to `Command::UnlockArchive`.
    ArchiveUnlocked {
        request: DesktopRequestId,
        result: Result<crate::ArchiveUnlock, FsError>,
    },
    /// Member bytes decoded so far while `navigation` opens an archive inside an archive.
    ArchiveProgress {
        pane: PaneId,
        navigation: NavigationId,
        staged: u64,
    },
    /// `navigation` did not open its archive because every archive opened from a native
    /// file that may be open is open: these are `sessions`. Its failed `ArchiveListed`
    /// follows. `retry` opens the same archive again once one of them is closed.
    ArchiveSessionsFull {
        pane: PaneId,
        navigation: NavigationId,
        sessions: Vec<crate::ArchiveSession>,
        retry: crate::ArchiveAction,
    },
    ArchiveAvailability(Vec<crate::ArchiveFormat>),
    ViewerPluginsListed(Result<Vec<crate::ViewerPluginInfo>, String>),
    ComparatorCapabilitiesListed(crate::ComparatorCapabilities),
    FilesystemPluginsListed(Result<Vec<crate::FilesystemPluginInfo>, String>),
    ThumbnailerPluginsListed(Result<Vec<crate::ThumbnailerPluginInfo>, String>),
    DirectoriesCompared {
        id: crate::CompareId,
        result: Result<Arc<crate::CompareResult>, FsError>,
    },
    /// A running directory comparison's progress; never a result.
    ComparisonProgress {
        id: crate::CompareId,
        progress: crate::CompareProgress,
    },
    PanelFilterValidated {
        request: crate::FilterValidationRequest,
        result: Result<(), FsError>,
        diagnostic: Option<crate::FilterDiagnostic>,
    },
    /// The answer to `Command::ValidateSelectionMask`: the reason the dialog shows
    /// when the mask cannot be applied.
    SelectionMaskValidated {
        request: crate::MaskValidationRequest,
        result: Result<(), String>,
    },
    FindFeedProjected(Box<crate::FindFeedProjectionResult>),
    FindFeedBuilt(Box<crate::FindFeedBuildResult>),
    FindFeedSize(Box<crate::FindFeedSizeEvent>),
    FilenameColorsValidated {
        request: crate::FilenameColorsValidation,
        result: Result<(), String>,
    },
    FilenameColorsReady {
        request: crate::FilenameColorsRequest,
        result: Result<Arc<crate::FilenameColorsResult>, String>,
    },
    Copy(crate::CopyEvent),
    DragDrop(crate::DragDropEvent),
    ClipboardFilesRead {
        request: DesktopRequestId,
        destination: PathBuf,
        result: Result<crate::FileClipboard, FsError>,
    },
    PathClipboardTextRead {
        request: DesktopRequestId,
        result: Result<Arc<str>, FsError>,
    },
    PathPicked {
        request: DesktopRequestId,
        result: Result<Option<PathBuf>, FsError>,
    },
    ShellMenuFinished {
        request: DesktopRequestId,
        result: Result<crate::ShellMenuOutcome, FsError>,
    },
    LocaleLoaded(Result<crate::NumberLocale, crate::FsError>),
    NetworkConnectionsFinished(Result<(), crate::FsError>),
    Operation(crate::OperationEvent),
    SearchReady {
        pane: PaneId,
        navigation: NavigationId,
        revision: SearchRevision,
        result: Arc<SearchResult>,
    },
    SearchFailed {
        pane: PaneId,
        navigation: NavigationId,
        revision: SearchRevision,
        error: FsError,
    },
    SelectionChanged {
        pane: PaneId,
        navigation: NavigationId,
        revision: SelectionRevision,
        selection: Arc<SelectionSnapshot>,
        /// Selection defaults for paths absent from the snapshot's basis until
        /// the complete-view rebase supplies explicit bits for every row.
        deferred: crate::SelectionDefaults,
    },
    SelectionFailed {
        pane: PaneId,
        navigation: NavigationId,
        revision: SelectionRevision,
        error: FsError,
    },
    FontFamiliesLoaded(Result<Vec<String>, FsError>),
    EditorDefaultResolved(Result<crate::EditorCommand, FsError>),
    PreferencesLoaded(Result<Box<crate::AcceptedPreferences>, crate::PreferencesApplyError>),
    PreferencesSaved {
        revision: PreferencesRevision,
        result: Result<crate::FileComparatorSnapshot, crate::PreferencesApplyError>,
    },
    PreferencesExported {
        request: DesktopRequestId,
        result: Result<crate::PreferencesExportReceipt, FsError>,
    },
    PreferencesImported {
        request: DesktopRequestId,
        result: Result<Box<crate::ImportedPreferences>, FsError>,
    },
    DirectoryChanged {
        pane: PaneId,
        navigation: NavigationId,
    },
    /// How a pane's accepted flat view or tree session is watched. Non-fatal:
    /// rows, actions and manual Refresh stay usable, and it is never a panel
    /// error. It is sent only when the status of `view` changes; present it only
    /// while `view` is the pane's current view (tree views by pane, navigation
    /// and session). `TreeEvent::WatchFailed` keeps reporting scope enrollment
    /// failures and epoch exhaustion.
    WatchStatus {
        view: crate::WatchedView,
        status: crate::WatchStatus,
    },
    FolderSizeUpdated {
        pane: PaneId,
        navigation: NavigationId,
        scan: FolderSizeScanId,
        entry: EntryId,
        size: FolderSize,
    },
    DiagnosticTextReady {
        request: DesktopRequestId,
        result: Result<crate::DiagnosticTextPreview, FsError>,
    },
    LicenseNoticeReady(Box<crate::LicenseNotice>),
    Installation(Box<crate::InstallationEvent>),
    SizeWarningsReady {
        request: DesktopRequestId,
        result: Result<Arc<dyn crate::GeneratedTextSource>, FsError>,
    },
    FolderSizesFinished {
        pane: PaneId,
        navigation: NavigationId,
        scan: FolderSizeScanId,
        cancelled: bool,
    },
    SelectedFolderSizes {
        request: crate::SelectionSizeId,
        pane: PaneId,
        navigation: NavigationId,
        selection_revision: SelectionRevision,
        /// Monotonic backend stamp rejects out-of-order observer delivery.
        version: u64,
        summary: crate::SelectedFolderSizes,
    },
    DesktopContext {
        shell: ShellFlavor,
        locations: Vec<NavigationLocation>,
        /// Virtual Network position within the special-location section. Zero
        /// places it after all drives and before every local special folder.
        network_position: usize,
        /// Finite, capability-filtered entries; currently at most one Recycle Bin.
        system_locations: Vec<crate::SystemLocationChoice>,
    },
    DesktopContextFailed(FsError),
    CommandIconLoaded {
        generation: u64,
        identity: String,
        pixels: Arc<IconPixels>,
    },
    /// A file's own artwork for a target of the file icon view `generation`.
    FileIconLoaded {
        generation: u64,
        identity: String,
        edge: u16,
        pixels: Arc<IconPixels>,
    },
    IconLoaded {
        key: IconKey,
        pixels: Arc<IconPixels>,
    },
    DesktopActionFinished {
        request: DesktopRequestId,
        result: Result<DesktopActionResult, FsError>,
    },
    /// A newer text for the metadata Properties summary of `request`, while or after
    /// its single folder's contents are counted. It replaces the shown text only;
    /// it never reopens a dismissed summary.
    PropertiesUpdated {
        request: DesktopRequestId,
        title: String,
        body: String,
    },
    DirectoryLinkTargetRead {
        request: DesktopRequestId,
        pane: PaneId,
        navigation: NavigationId,
        target: Option<PathBuf>,
    },
    VolumesUpdated(Vec<Volume>),
    VolumeDetailsUpdated {
        volume_id: String,
        root: PathBuf,
        details: VolumeDetails,
    },
    VolumeInformationLoaded(Box<crate::VolumeInformationResult>),
    TransferDestinationChecked(Box<crate::DestinationCheckResult>),
    TransferLinksInspected(Box<crate::TransferLinksResult>),
    VolumesFailed(FsError),
    ListingStarted {
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
    },
    EntriesBatch {
        pane: PaneId,
        navigation: NavigationId,
        entries: Vec<Entry>,
    },
    MetadataBatch {
        pane: PaneId,
        navigation: NavigationId,
        entries: Vec<(EntryId, EntryMetadata)>,
    },
    /// Sent just before `ListingFinished` of a re-read of the folder whose
    /// rows were `base`: applying `edits` in order to `base` gives that
    /// listing's rows, and every other row keeps its entry (identity, path,
    /// kind and metadata). Absent when the change cannot be described cheaply;
    /// a consumer whose rows are not `base` ignores it.
    ListingChanges {
        pane: PaneId,
        navigation: NavigationId,
        base: Arc<[Entry]>,
        edits: Arc<[RowEdit]>,
    },
    ListingFinished {
        pane: PaneId,
        navigation: NavigationId,
        entries: Arc<[Entry]>,
        index: Arc<DirectoryIndex>,
    },
    /// A native navigation was denied and the backend configured authorized retries. With
    /// [`ElevationAvailability::Available`] it retained the denied navigation for one
    /// explicit authorization retry; otherwise the retry cannot ask for permission, and
    /// the availability says why. Follows the navigation's [`Event::ListingFailed`].
    DirectoryElevationAvailable {
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        availability: crate::ElevationAvailability,
    },
    ListingFailed {
        pane: PaneId,
        navigation: NavigationId,
        error: FsError,
    },
    /// A native listing failed. Retry the raw fallback under a fresh navigation ID
    /// only while this exact request remains pending; otherwise discard the event.
    ListingFallback {
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        error: FsError,
    },
    /// The path of a failed native listing names an existing entry that is not a
    /// folder (a file, a link to one or a special file): `entry`, in the folder
    /// `parent`. `hidden` says whether Show hidden files leaves it out of listings.
    /// Sent just before that listing's `ListingFallback` or `ListingFailed`. A
    /// request that asked for a file (a path typed in the path bar) may list `parent`
    /// with the cursor on `entry` under a fresh navigation ID while this exact
    /// request remains pending; anything else ignores it and handles the failure.
    ListingNotAFolder {
        pane: PaneId,
        navigation: NavigationId,
        parent: PathBuf,
        entry: PathBuf,
        hidden: bool,
    },
    SpaceUpdated {
        pane: PaneId,
        navigation: NavigationId,
        space: SpaceInfo,
    },
    StorageKindUpdated {
        pane: PaneId,
        navigation: NavigationId,
        kind: crate::StorageKind,
    },
}
