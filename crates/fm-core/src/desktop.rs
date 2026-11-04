//! Platform-neutral contracts for optional native desktop integration.

use crate::{Entry, EntryKind, FsError, NavigationId, PaneId};
use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process::Child,
    sync::Arc,
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

/// Format a literal shell argument without changing its Unicode spelling.
/// Ordinary paths remain bare; shell syntax uses escaped double quotes.
pub fn quote_shell_path(path: &Path, shell: ShellFlavor) -> Result<String, &'static str> {
    let text = path
        .to_str()
        .ok_or("The path cannot be represented in shell Unicode text.")?;
    if text.contains('\0') {
        return Err("A shell path cannot contain NUL.");
    }
    if shell == ShellFlavor::Cmd {
        // cmd expands percent expressions even inside double quotes. Refuse an
        // unsafe literal instead of silently changing its identity; /V:OFF in
        // the launcher keeps exclamation marks literal.
        if text.chars().any(|ch| matches!(ch, '%' | '"' | '\r' | '\n')) {
            return Err("This path cannot be inserted safely in cmd; choose PowerShell.");
        }
        // A quoted trailing backslash has incompatible meanings for cmd
        // builtins and the CRT argv parser used by native programs.
        if !bare_shell_path(text, shell) && text.ends_with('\\') {
            return Err("This path cannot be inserted safely in cmd; choose PowerShell.");
        }
    }
    if bare_shell_path(text, shell) {
        return Ok(text.to_owned());
    }
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('"');
    for ch in text.chars() {
        match (shell, ch) {
            (ShellFlavor::PowerShell, '`' | '$' | '"' | '\u{201c}' | '\u{201d}' | '\u{201e}') => {
                quoted.push('`');
            }
            (ShellFlavor::Posix, '\\' | '$' | '`' | '"') => quoted.push('\\'),
            _ => {}
        }
        quoted.push(ch);
    }
    quoted.push('"');
    Ok(quoted)
}

fn bare_shell_path(text: &str, shell: ShellFlavor) -> bool {
    if text.is_empty() || text.starts_with('-') {
        return false;
    }
    if shell == ShellFlavor::PowerShell
        && (text.starts_with(['\u{2013}', '\u{2014}', '\u{2015}'])
            || text.as_bytes()[0].is_ascii_digit()
            || (text.starts_with('.') && text.as_bytes().get(1).is_some_and(u8::is_ascii_digit)))
    {
        // PowerShell can parse numeric basenames as values (001 becomes 1)
        // and leading dash forms as parameter names instead of literal text.
        return false;
    }
    text.chars().all(|ch| {
        if ch.is_ascii() {
            ch.is_ascii_alphanumeric()
                || matches!(ch, '_' | '.' | '/' | '-')
                || (shell != ShellFlavor::Posix && matches!(ch, '\\' | ':'))
        } else {
            !ch.is_whitespace()
                && !ch.is_control()
                && (shell != ShellFlavor::PowerShell || !matches!(ch, '\u{2018}'..='\u{201e}'))
        }
    })
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
    Edit,
    Copy,
    Move,
    NewFolder,
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
}

impl ActionIcon {
    /// Bounded prefetch palette. Consumers use named roles rather than numeric indexes.
    pub const ALL: [Self; 32] = [
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
        Self::Edit,
        Self::Copy,
        Self::Move,
        Self::NewFolder,
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
    ];
}

/// File type, discovered volume or allowlisted semantic role; never browsed file contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconKind {
    File,
    Folder,
    Link,
    Drive,
    Action(ActionIcon),
    Location(LocationKind),
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
    pub fn action(role: ActionIcon) -> Self {
        Self {
            kind: IconKind::Action(role),
            extension: None,
            drive_root: None,
            size: 32,
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
            size: 32,
        }
    }
    pub fn file() -> Self {
        Self {
            kind: IconKind::File,
            extension: None,
            drive_root: None,
            size: 32,
        }
    }
    pub fn drive() -> Self {
        Self {
            kind: IconKind::Drive,
            extension: None,
            drive_root: None,
            size: 32,
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
        Self {
            kind: match entry.kind {
                EntryKind::Directory => IconKind::Folder,
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
            size: 32,
        }
    }
}

/// Owned straight-alpha RGBA8 pixels; Slint handles are created on the UI thread.
#[derive(Clone, Debug)]
pub struct IconPixels {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

impl IconPixels {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, &'static str> {
        if !(1..=64).contains(&width) || !(1..=64).contains(&height) {
            return Err("Icon dimensions must be in 1..=64.");
        }
        let expected = (width as usize) * (height as usize) * 4;
        if rgba.len() != expected {
            return Err("Icon RGBA buffer has an invalid length.");
        }
        Ok(Self {
            width,
            height,
            rgba: rgba.into(),
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
    },
    PropertiesOpened,
    PropertiesInfo {
        title: String,
        body: String,
    },
    FileOpened,
    OpenDirectory {
        pane: PaneId,
        navigation: NavigationId,
        path: PathBuf,
    },
}

/// Native activation outcome. Directories stay inside the file manager;
/// only explicitly activated regular files use desktop associations.
pub enum OpenEntryResult {
    Directory,
    File {
        process: Option<Box<dyn TerminalProcess>>,
    },
}

impl std::fmt::Debug for OpenEntryResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Directory => f.write_str("Directory"),
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
}

/// All native icon extraction and file decoding happen on a backend worker.
pub trait IconService: Send + Sync + 'static {
    fn load_icon(&self, key: &IconKey) -> Result<Option<IconPixels>, FsError>;
}

pub trait TerminalService: Send + Sync + 'static {
    /// Pure configuration query, with no I/O or process creation.
    fn shell_flavor(&self) -> ShellFlavor;
    /// Launch only after explicit user submission; caller reaps the returned child.
    fn launch_terminal(
        &self,
        request: &TerminalRequest,
    ) -> Result<Box<dyn TerminalProcess>, FsError>;
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
}

pub trait LocationService: Send + Sync + 'static {
    /// Real volumes and supported physical special locations; no UI-side lookup.
    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError>;
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
    fn powershell_quotes_preserve_unicode_and_all_apostrophes() {
        let path = Path::new("Žluťoučký/文档/👩🏽‍💻 ' ‘ ’ $x;&");
        assert_eq!(
            quote_shell_path(path, ShellFlavor::PowerShell).unwrap(),
            "\"Žluťoučký/文档/👩🏽‍💻 ' ‘ ’ `$x;&\""
        );
    }

    #[test]
    fn posix_quotes_preserve_curly_quotes_and_ascii_apostrophe() {
        assert_eq!(
            quote_shell_path(Path::new("a'文档 ’ $x;&"), ShellFlavor::Posix).unwrap(),
            "\"a'文档 ’ \\$x;&\""
        );
    }

    #[test]
    fn icon_shape_requires_exact_bounded_rgba_buffer() {
        assert!(IconPixels::new(2, 2, vec![0; 16]).is_ok());
        assert!(IconPixels::new(2, 2, vec![0; 15]).is_err());
        assert!(IconPixels::new(0, 2, vec![]).is_err());
        assert!(IconPixels::new(65, 1, vec![0; 260]).is_err());
        assert!(IconPixels::new(u32::MAX, u32::MAX, vec![]).is_err());
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
    fn cmd_quotes_unicode_metacharacters_and_rejects_percent_expansion() {
        let path = Path::new("C:\\文档\\Žluťoučký & ! ^ (folder)");
        assert_eq!(
            quote_shell_path(path, ShellFlavor::Cmd).unwrap(),
            "\"C:\\文档\\Žluťoučký & ! ^ (folder)\""
        );
        for input in ["C:\\%PATH%", "C:\\quoted\"path", "C:\\line\nnext"] {
            assert!(quote_shell_path(Path::new(input), ShellFlavor::Cmd).is_err());
        }
    }
}
