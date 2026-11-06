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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
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
    LoadLocale,
    ManageNetworkConnections {
        disconnect: bool,
    },
    PrepareOperation(crate::PrepareOperation),
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
    EnqueueCopy(crate::CopyRequest),
    DecideCopy(crate::CopyDecision),
    CancelCopy(crate::OperationId),
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
    StartDrag(crate::DragId),
    CancelDrag(crate::DragId),
    AcceptDrop {
        token: crate::DropToken,
        id: crate::OperationId,
    },
    RejectDrop(crate::DropToken),
    InternalDrop {
        id: crate::DragId,
        target: crate::FileDropTarget,
        targets_revision: u64,
    },
    Search(SearchRequest),
    CancelSearch {
        pane: PaneId,
        revision: SearchRevision,
    },
    Selection(SelectionRequest),
    ListVolumes,
    ListDesktopContext,
    LoadPreferences,
    ListFontFamilies,
    SavePreferences {
        revision: PreferencesRevision,
        preferences: UserPreferences,
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
    CopyFiles {
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
    ShowProperties {
        request: DesktopRequestId,
        paths: Vec<PathBuf>,
    },
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
    ComputeFolderSizes {
        pane: PaneId,
        navigation: NavigationId,
        scan: FolderSizeScanId,
    },
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
    DirectoriesCompared {
        id: crate::CompareId,
        result: Result<Arc<crate::CompareResult>, FsError>,
    },
    PanelFilterValidated {
        request: crate::FilterValidationRequest,
        result: Result<(), FsError>,
    },
    Copy(crate::CopyEvent),
    DragDrop(crate::DragDropEvent),
    ClipboardFilesRead {
        request: DesktopRequestId,
        destination: PathBuf,
        result: Result<crate::FileClipboard, FsError>,
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
        /// All/Invert still applies to unseen rows: paths absent from the basis
        /// are selected until the complete-view rebase supplies explicit bits.
        deferred: bool,
    },
    SelectionFailed {
        pane: PaneId,
        navigation: NavigationId,
        revision: SelectionRevision,
        error: FsError,
    },
    FontFamiliesLoaded(Result<Vec<String>, FsError>),
    PreferencesLoaded(Result<UserPreferences, FsError>),
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
    FolderSizesFinished {
        pane: PaneId,
        navigation: NavigationId,
        scan: FolderSizeScanId,
        cancelled: bool,
    },
    DesktopContext {
        shell: ShellFlavor,
        locations: Vec<NavigationLocation>,
    },
    DesktopContextFailed(FsError),
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
    ListingFailed {
        pane: PaneId,
        navigation: NavigationId,
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
