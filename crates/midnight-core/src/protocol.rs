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

#[derive(Clone, Debug)]
pub enum Command {
    SetCommandIconView {
        generation: u64,
        targets: Vec<crate::CommandIconTarget>,
    },
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
        action: crate::ArchiveSelectionAction,
    },
    ArchiveNavigate {
        pane: PaneId,
        navigation: NavigationId,
        action: crate::ArchiveAction,
        sort: SortSpec,
    },
    ListViewerPlugins,
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
    ValidateFilenameColors(crate::FilenameColorsValidation),
    MatchFilenameColors(crate::FilenameColorsRequest),
    ClearFilenameColors {
        pane: PaneId,
        id: u64,
    },
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
    CancelCopy(crate::OperationId),
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
    StartDrag(crate::DragId),
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
    ListDesktopContext,
    LoadPreferences,
    ListFontFamilies,
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
    ShowSelectionProperties {
        request: DesktopRequestId,
        selection: Arc<SelectionSnapshot>,
        view: Arc<DirectoryIndex>,
        fallback: Option<PathBuf>,
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
    Shutdown,
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
    ArchiveAvailability(Vec<crate::ArchiveFormat>),
    ViewerPluginsListed(Result<Vec<crate::ViewerPluginInfo>, String>),
    FilesystemPluginsListed(Result<Vec<crate::FilesystemPluginInfo>, String>),
    ThumbnailerPluginsListed(Result<Vec<crate::ThumbnailerPluginInfo>, String>),
    DirectoriesCompared {
        id: crate::CompareId,
        result: Result<Arc<crate::CompareResult>, FsError>,
    },
    PanelFilterValidated {
        request: crate::FilterValidationRequest,
        result: Result<(), FsError>,
        diagnostic: Option<crate::FilterDiagnostic>,
    },
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
    PreferencesLoaded(Result<Box<UserPreferences>, FsError>),
    PreferencesSaved {
        revision: PreferencesRevision,
        result: Result<(), FsError>,
    },
    DirectoryChanged {
        pane: PaneId,
        navigation: NavigationId,
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
    },
    DesktopContextFailed(FsError),
    CommandIconLoaded {
        generation: u64,
        identity: String,
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
    VolumesUpdated(Vec<Volume>),
    VolumeDetailsUpdated {
        volume_id: String,
        root: PathBuf,
        details: VolumeDetails,
    },
    VolumeInformationLoaded(Box<crate::VolumeInformationResult>),
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
    ListingFinished {
        pane: PaneId,
        navigation: NavigationId,
        entries: Arc<[Entry]>,
        index: Arc<DirectoryIndex>,
    },
    /// Backend retained one denied native navigation for an explicit authorization retry.
    DirectoryElevationAvailable {
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
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
