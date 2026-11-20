//! Platform-neutral contracts for optional native desktop integration.

use crate::{Entry, EntryKind, FsError, NavigationId, PaneId};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Child,
    sync::Arc,
    time::Duration,
};

/// Shell syntax used by the intentionally user-submitted command line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellFlavor {
    PowerShell,
    Cmd,
    Posix,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TerminalMode {
    #[default]
    Standard,
    Elevated,
}

/// Finite semantic roles for optional native control icons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ActionIcon {
    Back,
    Forward,
    Parent,
    Refresh,
    OpenPath,
    Bookmarks,
    Terminal,
    Options,
    Run,
    TabAdd,
    CloseTab,
    RemoveBookmark,
    AddBookmark,
    Help,
    Rename,
    View,
    Find,
    Edit,
    Copy,
    Move,
    NewFolder,
    NewFile,
    Delete,
    UserMenu,
    Menu,
    SaveAgain,
    Done,
    Close,
    Properties,
    FolderSizes,
    About,
    Exit,
    SortAscending,
    SortDescending,
    Network,
    RecycleBin,
}

impl ActionIcon {
    /// Bounded prefetch palette. Consumers use named roles rather than numeric indexes.
    pub const ALL: [Self; 36] = [
        Self::Back,
        Self::Forward,
        Self::Parent,
        Self::Refresh,
        Self::OpenPath,
        Self::Bookmarks,
        Self::Terminal,
        Self::Options,
        Self::Run,
        Self::TabAdd,
        Self::CloseTab,
        Self::RemoveBookmark,
        Self::AddBookmark,
        Self::Help,
        Self::Rename,
        Self::View,
        Self::Find,
        Self::Edit,
        Self::Copy,
        Self::Move,
        Self::NewFolder,
        Self::NewFile,
        Self::Delete,
        Self::UserMenu,
        Self::Menu,
        Self::SaveAgain,
        Self::Done,
        Self::Close,
        Self::Properties,
        Self::FolderSizes,
        Self::About,
        Self::Exit,
        Self::SortAscending,
        Self::SortDescending,
        Self::Network,
        Self::RecycleBin,
    ];
}

/// File type, discovered volume or allowlisted semantic role; never browsed file contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconKind {
    File,
    Folder,
    FolderLink,
    Link,
    Drive,
    Action(ActionIcon),
    Location(LocationKind),
}

/// Delivered icons the backend remembers: it does not load or resend an icon,
/// at any edge it was delivered at, while that icon is among this many most
/// recently requested ones. A UI image cache must therefore hold at least this
/// many icons, each at every edge the frontend draws.
pub const DESKTOP_ICON_CAPACITY: usize = 256;

/// Largest edge, in device pixels, of an icon a provider is asked for and of an
/// `IconPixels` bitmap. Thumbnail tiles on dense displays draw icons this large.
pub const MAX_ICON_EDGE: u16 = 256;

/// Edges, in device pixels, at which a frontend draws native icons. The backend
/// delivers every requested icon at the edges of the places its kind is drawn
/// in, as far as the platform's provider serves them (`IconService::icon_edge`).
/// Action art is drawn at several small sizes from one bitmap and stays at
/// `IconKey::DEFAULT_EDGE`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct IconEdges {
    /// File and folder rows.
    pub entry: u16,
    /// Tiles of entries without a thumbnail; zero while no pane shows tiles.
    pub tile: u16,
    /// Location, drive and bookmark choices.
    pub location: u16,
}

impl Default for IconEdges {
    /// The single edge delivered before frontends reported theirs.
    fn default() -> Self {
        Self {
            entry: IconKey::DEFAULT_EDGE,
            tile: 0,
            location: IconKey::DEFAULT_EDGE,
        }
    }
}

impl IconEdges {
    /// The distinct edges at which an icon of `kind` is drawn.
    pub fn of(self, kind: IconKind) -> impl Iterator<Item = u16> {
        let action = IconKey::DEFAULT_EDGE;
        let places = match kind {
            IconKind::File | IconKind::Link => [self.entry, self.tile, 0, 0],
            // Folder art also stands in for a location without art of its own.
            IconKind::Folder | IconKind::FolderLink => [self.entry, self.tile, self.location, 0],
            IconKind::Drive | IconKind::Location(_) => [self.location, 0, 0, 0],
            // Some action art is also the icon of a row or of a location choice.
            IconKind::Action(ActionIcon::Parent) => [action, self.entry, self.tile, 0],
            IconKind::Action(ActionIcon::Network) => [action, self.entry, self.tile, self.location],
            IconKind::Action(
                ActionIcon::RecycleBin
                | ActionIcon::OpenPath
                | ActionIcon::Bookmarks
                | ActionIcon::AddBookmark,
            ) => [action, self.location, 0, 0],
            IconKind::Action(_) => [action, 0, 0, 0],
        };
        (0..places.len())
            .filter(move |&at| places[at] != 0 && !places[..at].contains(&places[at]))
            .map(move |at| places[at])
    }

    /// False for edges no provider is asked for.
    pub fn is_valid(self) -> bool {
        let drawn = 1..=MAX_ICON_EDGE;
        drawn.contains(&self.entry) && drawn.contains(&self.location) && self.tile <= MAX_ICON_EDGE
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IconKey {
    pub kind: IconKind,
    pub extension: Option<OsString>,
    /// Raw root supplied by volume discovery. Native adapters validate it before lookup.
    /// Ordinary files/folders use type-only keys and leave this unset.
    pub drive_root: Option<PathBuf>,
    pub size: u16,
}

impl IconKey {
    /// The edge every constructor assigns. Frontends build keys of this edge to
    /// name an icon; the backend delivers it at the edges it is drawn at.
    pub const DEFAULT_EDGE: u16 = 32;

    /// The same icon at another edge, in device pixels.
    pub fn sized(mut self, size: u16) -> Self {
        self.size = size;
        self
    }

    pub fn action(role: ActionIcon) -> Self {
        Self {
            kind: IconKind::Action(role),
            extension: None,
            drive_root: None,
            size: Self::DEFAULT_EDGE,
        }
    }

    /// Special aliases retain their meaning even when their native paths are identical.
    pub fn for_location(location: &NavigationLocation) -> Self {
        if location.kind == LocationKind::Volume {
            return Self::for_drive(&location.path);
        }
        Self {
            kind: IconKind::Location(location.kind),
            ..Self::folder()
        }
    }

    pub fn folder() -> Self {
        Self {
            kind: IconKind::Folder,
            extension: None,
            drive_root: None,
            size: Self::DEFAULT_EDGE,
        }
    }
    pub fn file() -> Self {
        Self {
            kind: IconKind::File,
            extension: None,
            drive_root: None,
            size: Self::DEFAULT_EDGE,
        }
    }
    pub fn folder_link() -> Self {
        Self {
            kind: IconKind::FolderLink,
            ..Self::folder()
        }
    }
    pub fn drive() -> Self {
        Self {
            kind: IconKind::Drive,
            extension: None,
            drive_root: None,
            size: Self::DEFAULT_EDGE,
        }
    }
    /// Preserve the native volume identity so different drives have independent icons.
    pub fn for_drive(root: &Path) -> Self {
        Self {
            drive_root: Some(root.into()),
            ..Self::drive()
        }
    }
    pub fn for_entry(entry: &Entry) -> Self {
        if entry.network_node.is_some() {
            return Self::action(ActionIcon::Network);
        }
        Self {
            kind: match entry.kind {
                EntryKind::Directory => IconKind::Folder,
                EntryKind::DirectoryLink => IconKind::FolderLink,
                EntryKind::Symlink => IconKind::Link,
                _ => IconKind::File,
            },
            extension: if entry.kind == EntryKind::File {
                entry
                    .path
                    .extension()
                    .filter(|extension| extension.len() <= 128)
                    .map(OsString::from)
            } else {
                None
            },
            drive_root: None,
            size: Self::DEFAULT_EDGE,
        }
    }
}

/// Owned straight-alpha RGBA8 pixels; Slint handles are created on the UI thread.
#[derive(Clone, Debug)]
pub struct IconPixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    /// A generic unassociated document icon, rather than artwork for this type.
    pub generic: bool,
}

impl IconPixels {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, &'static str> {
        let edges = 1..=u32::from(MAX_ICON_EDGE);
        if !edges.contains(&width) || !edges.contains(&height) {
            return Err("Icon dimensions must be in 1..=256.");
        }
        let expected = (width as usize) * (height as usize) * 4;
        if rgba.len() != expected {
            return Err("Icon RGBA buffer has an invalid length.");
        }
        Ok(Self {
            width,
            height,
            rgba: rgba.into(),
            generic: false,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LocationKind {
    Volume,
    Home,
    Desktop,
    Documents,
    Downloads,
    Pictures,
    Videos,
    Music,
    Temporary,
    OneDrive,
}

#[derive(Clone, Debug)]
pub struct NavigationLocation {
    pub id: String,
    pub label: String,
    pub path: PathBuf,
    pub kind: LocationKind,
    /// Native drive letters only; Linux/macOS provide None.
    pub drive_letter: Option<char>,
}

/// Shared display text for locale ordering of the virtual network entry point.
pub const NETWORK_NEIGHBORHOOD_LABEL: &str = "Network Neighborhood";

/// Typed desktop destinations that have no filesystem navigation path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum SystemLocation {
    RecycleBin,
}

/// Backend-prepared, capability-filtered system entry for desktop controls.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SystemLocationChoice {
    pub location: SystemLocation,
    pub label: String,
    /// Insertion rank among non-volume native local special folders only.
    /// Drives and Network do not count; Network remains before all local/system
    /// special folders. The backend applies locale ordering and deterministic ties.
    pub special_position: usize,
}

#[derive(Clone, Debug)]
pub struct TerminalRequest {
    pub command: String,
    pub working_directory: PathBuf,
    pub shell: ShellFlavor,
    pub mode: TerminalMode,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DesktopRequestId(pub u64);

#[derive(Clone, Debug)]
pub enum DesktopActionResult {
    TerminalStarted {
        process_id: u32,
        history: crate::CommandHistoryEntry,
    },
    ClipboardCopied,
    EditorStarted,
    PropertiesOpened,
    PropertiesInfo {
        title: String,
        body: String,
    },
    FileOpened,
    /// Native desktop dispatch completed, or its returned helper exited successfully.
    /// This does not assert that a desktop window is visible or ready.
    SystemLocationOpened {
        location: SystemLocation,
    },
    OpenDirectory {
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
    },
    /// Explicit activation resolved a link to another native directory. `path`
    /// remains the original request identity; `target` is the OS-resolved destination.
    OpenDirectoryTarget {
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
        target: PathBuf,
    },
}

/// Native activation outcome. Directories stay inside the file manager;
/// only explicitly activated regular files use desktop associations.
pub enum OpenEntryResult {
    Directory,
    /// A native directory link whose explicit activation requires navigating to
    /// its validated target rather than enumerating the original alias.
    DirectoryTarget {
        path: PathBuf,
    },
    File {
        process: Option<Box<dyn TerminalProcess>>,
    },
}

impl std::fmt::Debug for OpenEntryResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Directory => f.write_str("Directory"),
            Self::DirectoryTarget { path } => f
                .debug_struct("DirectoryTarget")
                .field("path", path)
                .finish(),
            Self::File { process } => f
                .debug_struct("File")
                .field("has_process", &process.is_some())
                .finish(),
        }
    }
}

pub trait OpenService: Send + Sync + 'static {
    /// Inspect the target on a backend worker, following an explicitly activated
    /// link. Reject missing/special files and use literal native arguments only.
    fn open_entry(&self, path: &Path) -> Result<OpenEntryResult, FsError>;

    /// Read the stored first hop of a link for presentation on a bounded backend
    /// worker. Use the exact absolute path captured from the original native row.
    /// Implementations read the link object directly and preserve native encoding
    /// and relative target semantics. `Ok(None)` means the capability is unavailable.
    fn read_link_target(&self, _link: &Path) -> Result<Option<PathBuf>, FsError> {
        Ok(None)
    }

    /// Explicit typed activation on a backend worker, without filesystem fallback.
    /// None means native dispatch was accepted. A returned helper remains owned
    /// and reaped by the caller; success is published only after its successful exit.
    /// Facility support does not guarantee that the desktop has a working handler.
    fn open_system_location(
        &self,
        _location: SystemLocation,
    ) -> Result<Option<Box<dyn TerminalProcess>>, FsError> {
        Err(FsError::from_io(
            "open system location",
            Path::new(""),
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "This desktop system location is unavailable.",
            ),
        ))
    }

    /// Resolve an explicitly activated directory link without launching any
    /// file association. Ordinary platforms keep the raw navigation path and
    /// let directory enumeration validate it with the caller's permissions.
    fn directory_destination(&self, path: &Path) -> Result<PathBuf, FsError> {
        Ok(path.to_owned())
    }
}

/// All native icon extraction and file decoding happen on a backend worker.
pub trait IconService: Send + Sync + 'static {
    fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError>;

    /// The edge to request from `load_icon` for an icon drawn at `drawn` device
    /// pixels. A provider that renders artwork at the requested size returns
    /// `drawn`; the default is the one edge every icon was delivered at before,
    /// which the frontend then scales itself. Called on the backend's own task:
    /// an implementation computes the answer without I/O.
    fn icon_edge(&self, _drawn: u16) -> u16 {
        IconKey::DEFAULT_EDGE
    }

    /// Explicitly configured Program artwork only. The caller captures and revalidates
    /// the native executable around this backend-only lookup; no program is launched.
    fn load_executable_icon(
        &self,
        _path: &Path,
        _size: u16,
        _cancel: &crate::Cancellation,
    ) -> Result<Option<IconPixels>, FsError> {
        Ok(None)
    }
}

/// Bounded presentation metadata, never a captured native owner.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandIconTarget {
    pub identity: String,
    pub executable: PathBuf,
}

pub trait TerminalService: Send + Sync + 'static {
    /// Pure configuration query, with no I/O or process creation.
    fn shell_flavor(&self) -> ShellFlavor;
    /// Launch only after explicit user submission; caller reaps the returned child.
    fn launch_terminal(
        &self,
        request: &TerminalRequest,
    ) -> Result<Box<dyn TerminalProcess>, FsError>;

    /// Explicit backend environment data; defaults preserve existing adapters and never
    /// interpolate file names into user-authored source. Native implementations override.
    fn launch_terminal_with_environment(
        &self,
        request: &TerminalRequest,
        environment: &[(String, OsString)],
        cancel: &crate::Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        if cancel.is_cancelled() {
            return Err(FsError::from_io(
                "Custom Command terminal",
                &request.working_directory,
                std::io::ErrorKind::Interrupted.into(),
            ));
        }
        if environment.is_empty() {
            return self.launch_terminal(request);
        }
        Err(FsError::from_io(
            "Custom Command terminal",
            &request.working_directory,
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "This terminal cannot accept native environment data.",
            ),
        ))
    }
}

/// Backend-owned process tracking for either Rust Child or a native process handle.
/// This permits Windows to create a real independent console without inheriting
/// redirected file-manager streams. Calls and disposal belong to backend workers.
pub trait TerminalProcess: Send {
    fn id(&self) -> u32;
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>>;
}

impl TerminalProcess for Child {
    fn id(&self) -> u32 {
        Child::id(self)
    }
    fn try_wait(&mut self) -> std::io::Result<Option<std::process::ExitStatus>> {
        Child::try_wait(self)
    }
}

pub trait PropertyService: Send + Sync + 'static {
    /// Returns true when native properties were shown, false for metadata fallback.
    fn show_properties(&self, paths: &[PathBuf]) -> Result<bool, FsError>;

    /// Ownership, times and permissions of `path` itself (a final link is not
    /// followed) for the metadata fallback. Read-only and called on a backend
    /// worker. None when this service has no such details.
    fn property_details(&self, _path: &Path) -> Result<Option<PropertyDetails>, FsError> {
        Ok(None)
    }
}

/// Native details of one entry for the Properties summary. A field is None when the
/// filesystem does not report it; a missing birth time is shown as not available.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PropertyDetails {
    pub owner: Option<PropertyAccount>,
    pub group: Option<PropertyAccount>,
    pub created: Option<std::time::SystemTime>,
    pub modified: Option<std::time::SystemTime>,
    pub accessed: Option<std::time::SystemTime>,
    /// POSIX file type and permission bits of the entry itself.
    pub unix_mode: Option<u32>,
}

/// A numeric owner or group and its name when the account database has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PropertyAccount {
    pub id: u32,
    pub name: Option<String>,
}

pub trait LocationService: Send + Sync + 'static {
    /// Real volumes and supported physical special locations; no UI-side lookup.
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError>;

    /// Discover dispatch capability on a backend worker, never in a UI callback.
    /// False omits the typed entry and refuses activation; legacy adapters opt out.
    fn supports_system_location(&self, _location: SystemLocation) -> bool {
        false
    }
}

/// Ends a [`ClipboardSession::wait_pending`] from any thread; see [`ClipboardSession::waker`].
pub type ClipboardWaker = Arc<dyn Fn() + Send + Sync>;

/// A thread-affine native clipboard owner. Create, use and drop it on one backend worker.
///
/// Threading contract: the backend thread that opened the session (its owner thread)
/// makes every call on it and drops it. Only the [`ClipboardWaker`] returned by
/// [`waker`](Self::waker) may be used on other threads.
pub trait ClipboardSession {
    fn copy_text(&mut self, text: &str) -> Result<(), FsError>;
    /// Read strict Unicode text on this session's backend worker. Native adapters
    /// bound the encoded input before allocation; undecodable text is an error.
    fn read_text(&mut self) -> Result<String, FsError> {
        Err(FsError::from_io(
            "clipboard text",
            Path::new(""),
            std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                "Reading clipboard text is unavailable.",
            ),
        ))
    }
    fn copy_files(&mut self, _files: &crate::FileClipboard) -> Result<(), FsError> {
        Err(unsupported_file_clipboard())
    }
    fn read_files(&mut self) -> Result<crate::FileClipboard, FsError> {
        Err(unsupported_file_clipboard())
    }
    /// Pump bounded native clipboard-owner events on the session's own backend thread.
    fn service_pending(&mut self) -> Result<(), FsError> {
        Ok(())
    }
    /// Whether the session still owns clipboard content or delayed rendering that other
    /// applications may need it to serve. Once false, dropping the session leaves the
    /// clipboard that other applications see unchanged, so an idle owner thread may close
    /// it instead of pumping it. The default, true, keeps a session open and pumped.
    fn owns_clipboard(&self) -> bool {
        true
    }
    /// Block the owner thread until native owner events may be pending, the session's
    /// [`waker`](Self::waker) was called, or `timeout` elapsed. An implementation services
    /// the pending owner events (as [`service_pending`](Self::service_pending) does)
    /// before it returns.
    ///
    /// A wake made at any time after the previous `wait_pending` returned, including
    /// before this call started, ends this wait immediately; no wake is lost between the
    /// caller's last check of its own state and the wait. Spurious early returns are
    /// allowed, so callers re-check their requests, cancellation and
    /// [`owns_clipboard`](Self::owns_clipboard) after every return.
    ///
    /// The default parks the thread for at most `timeout` (`std::thread::park_timeout`)
    /// and services nothing; callers keep calling `service_pending` between waits.
    fn wait_pending(&mut self, timeout: Duration) -> Result<(), FsError> {
        std::thread::park_timeout(timeout);
        Ok(())
    }
    /// A waker that ends the current or the next [`wait_pending`](Self::wait_pending) of
    /// this session, or None (the default) when nothing but the timeout ends that wait;
    /// such a session needs short timeouts to stay responsive.
    ///
    /// The owner thread obtains the waker; any thread may clone, keep and call it, any
    /// number of times (calls may coalesce into one wakeup). A call must be cheap and
    /// non-blocking, because it may run on an async executor, and stays safe after the
    /// session was dropped, when it has no effect.
    fn waker(&self) -> Option<ClipboardWaker> {
        None
    }
}

fn unsupported_file_clipboard() -> FsError {
    FsError::from_io(
        "file clipboard",
        Path::new(""),
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Native file clipboard is unavailable.",
        ),
    )
}

pub trait ClipboardService: Send + Sync + 'static {
    /// Opening may perform native I/O; call only from the dedicated backend worker.
    fn open(&self) -> Result<Box<dyn ClipboardSession>, FsError>;
}

pub trait DesktopServices:
    IconService + TerminalService + PropertyService + LocationService + OpenService
{
}
impl<T: IconService + TerminalService + PropertyService + LocationService + OpenService + ?Sized>
    DesktopServices for T
{
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_shape_requires_exact_bounded_rgba_buffer() {
        assert!(IconPixels::new(2, 2, vec![0; 16]).is_ok());
        assert!(IconPixels::new(2, 2, vec![0; 15]).is_err());
        assert!(IconPixels::new(0, 2, vec![]).is_err());
        assert!(IconPixels::new(256, 1, vec![0; 1024]).is_ok());
        assert!(IconPixels::new(257, 1, vec![0; 1028]).is_err());
        assert!(IconPixels::new(u32::MAX, u32::MAX, vec![]).is_err());
    }

    #[test]
    fn icon_edges_list_each_place_a_kind_is_drawn_in_once() {
        let edges = IconEdges {
            entry: 20,
            tile: 96,
            location: 26,
        };
        let of = |kind| edges.of(kind).collect::<Vec<_>>();
        assert_eq!(of(IconKind::File), [20, 96]);
        assert_eq!(of(IconKind::Link), [20, 96]);
        assert_eq!(of(IconKind::Folder), [20, 96, 26]);
        assert_eq!(of(IconKind::FolderLink), [20, 96, 26]);
        assert_eq!(of(IconKind::Drive), [26]);
        assert_eq!(of(IconKind::Location(LocationKind::Home)), [26]);
        assert_eq!(of(IconKind::Action(ActionIcon::Copy)), [32]);
        assert_eq!(of(IconKind::Action(ActionIcon::Parent)), [32, 20, 96]);
        assert_eq!(of(IconKind::Action(ActionIcon::Network)), [32, 20, 96, 26]);
        assert_eq!(of(IconKind::Action(ActionIcon::RecycleBin)), [32, 26]);

        // No tile view, and rows drawn at the location edge: nothing repeats.
        let rows = IconEdges {
            entry: 20,
            tile: 0,
            location: 20,
        };
        assert_eq!(rows.of(IconKind::Folder).collect::<Vec<_>>(), [20]);
        assert_eq!(
            IconEdges::default()
                .of(IconKind::Action(ActionIcon::Network))
                .collect::<Vec<_>>(),
            [32],
            "the default delivers one bitmap of the default edge per icon"
        );
        assert!(IconEdges::default().is_valid() && edges.is_valid());
        for invalid in [
            IconEdges { entry: 0, ..edges },
            IconEdges {
                location: 0,
                ..edges
            },
            IconEdges {
                entry: 257,
                ..edges
            },
            IconEdges { tile: 257, ..edges },
            IconEdges {
                location: 257,
                ..edges
            },
        ] {
            assert!(!invalid.is_valid(), "{invalid:?}");
        }
    }

    #[test]
    fn per_volume_icons_keep_raw_roots_separate_in_the_cache() {
        let first = Path::new("C:\\");
        let second = Path::new("D:\\");
        let mut icons = std::collections::HashMap::new();
        icons.insert(IconKey::for_drive(first), "system");
        icons.insert(IconKey::for_drive(second), "data");
        icons.insert(IconKey::drive(), "fallback");
        assert_eq!(icons.len(), 3);
        assert_eq!(icons[&IconKey::for_drive(first)], "system");
        assert_eq!(icons[&IconKey::for_drive(second)], "data");
        assert_eq!(icons[&IconKey::drive()], "fallback");
        assert_eq!(IconKey::for_drive(first).drive_root.as_deref(), Some(first));
        assert_eq!(IconKey::folder().drive_root, None);
        assert_eq!(IconKey::file().drive_root, None);
    }

    #[test]
    fn directory_link_icon_keys_are_shared_type_only_keys_without_target_or_extension() {
        let entry = Entry::new(
            crate::EntryId(7),
            PathBuf::from("文档/link.exe"),
            EntryKind::DirectoryLink,
        );
        let key = IconKey::for_entry(&entry);
        assert_eq!(key, IconKey::folder_link());
        assert_ne!(key, IconKey::folder());
        assert_eq!(key.drive_root, None);
        assert_eq!(key.extension, None);
    }

    /// A session written before the event-driven wait existed.
    struct TextOnly;
    impl ClipboardSession for TextOnly {
        fn copy_text(&mut self, _: &str) -> Result<(), FsError> {
            Ok(())
        }
    }

    #[test]
    fn clipboard_session_defaults_assume_ownership_and_offer_no_waker() {
        let session: Box<dyn ClipboardSession> = Box::new(TextOnly);
        // Unknown sessions keep being pumped instead of being closed while they may own data.
        assert!(session.owns_clipboard());
        assert!(session.waker().is_none());
    }

    #[test]
    fn default_clipboard_wait_is_a_bounded_park_that_an_unpark_ends() {
        // Elapses only when the unpark is lost. A timed wait can end up to a timer tick
        // early, so the check below keeps a clear margin below it.
        let failure_wait = Duration::from_secs(60);
        let mut session: Box<dyn ClipboardSession> = Box::new(TextOnly);
        assert!(session.wait_pending(Duration::ZERO).is_ok());
        // The default is `park_timeout`: an unpark before or during the wait ends it.
        let owner = std::thread::current();
        let unparker = std::thread::spawn(move || owner.unpark());
        let started = std::time::Instant::now();
        assert!(session.wait_pending(failure_wait).is_ok());
        unparker.join().unwrap();
        assert!(
            started.elapsed() < failure_wait / 2,
            "the default wait must return when its thread is unparked"
        );
    }
}
