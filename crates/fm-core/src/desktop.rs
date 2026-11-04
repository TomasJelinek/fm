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

/// Quote a path token without changing its original Unicode spelling.
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
        return Ok(format!("\"{text}\""));
    }
    let mut quoted = String::with_capacity(text.len() + 2);
    quoted.push('\'');
    for ch in text.chars() {
        match (shell, ch) {
            (ShellFlavor::PowerShell, '\'' | '\u{2018}' | '\u{2019}') => {
                quoted.push(ch);
                quoted.push(ch);
            }
            (ShellFlavor::Posix, '\'') => quoted.push_str("'\\''"),
            _ => quoted.push(ch),
        }
    }
    quoted.push('\'');
    Ok(quoted)
}

/// File type or discovered volume icon; never arbitrary browsed file contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IconKind {
    File,
    Folder,
    Link,
    Drive,
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
            "'Žluťoučký/文档/👩🏽‍💻 '' ‘‘ ’’ $x;&'"
        );
    }

    #[test]
    fn posix_quotes_preserve_curly_quotes_and_escape_ascii_apostrophe() {
        assert_eq!(
            quote_shell_path(Path::new("a'文档 ’ $x;&"), ShellFlavor::Posix).unwrap(),
            "'a'\\''文档 ’ $x;&'"
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
