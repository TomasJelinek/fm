//! Injects SMB file-ID query responses on a test's own directory tree. All
//! opens, sharing, enumeration, I/O and publication still go through Windows.
use std::{
    fs::File,
    io,
    path::{Path, PathBuf},
    sync::Mutex,
};
use windows_sys::Win32::Storage::FileSystem::VOLUME_NAME_DOS;

#[derive(Clone, Copy)]
pub enum Response {
    Unsupported,
    AllZero,
    AccessDenied,
    /// An all-zero `FileIdInfo` ID and file index 0 for every entry, with the real volume
    /// serial number: the WebDAV redirector and SMB servers without file IDs.
    ZeroIndex,
}

enum Entries {
    All,
    Children,
    Root,
}

struct Rule {
    root: PathBuf,
    response: Response,
    entries: Entries,
}

static ROOTS: Mutex<Vec<Rule>> = Mutex::new(Vec::new());

/// Active until dropped, including for the engine's worker threads.
pub struct Scope(PathBuf);

impl Scope {
    pub fn new(root: &Path, response: Response) -> io::Result<Self> {
        Self::register(root, response, Entries::All)
    }

    /// A network folder with a usable ID whose children have no usable IDs.
    pub fn children(root: &Path, response: Response) -> io::Result<Self> {
        Self::register(root, response, Entries::Children)
    }

    /// A network folder without a usable ID whose children return their native IDs.
    pub fn root_only(root: &Path, response: Response) -> io::Result<Self> {
        Self::register(root, response, Entries::Root)
    }

    fn register(root: &Path, response: Response, entries: Entries) -> io::Result<Self> {
        let root = std::fs::canonicalize(root)?;
        let mut roots = ROOTS.lock().unwrap();
        if roots
            .iter()
            .any(|rule| root.starts_with(&rule.root) || rule.root.starts_with(&root))
        {
            return Err(io::ErrorKind::AlreadyExists.into());
        }
        roots.push(Rule {
            root: root.clone(),
            response,
            entries,
        });
        Ok(Self(root))
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        ROOTS.lock().unwrap().retain(|rule| rule.root != self.0);
    }
}

pub(crate) fn response(file: &File) -> Option<Response> {
    matching_rule(file, |rule, path| match rule.entries {
        Entries::All => true,
        Entries::Children => rule.root != path,
        Entries::Root => rule.root == path,
    })
}

pub(crate) fn is_network(file: &File) -> bool {
    matching_rule(file, |_, _| true).is_some()
}

fn matching_rule(file: &File, matches: impl Fn(&Rule, &Path) -> bool) -> Option<Response> {
    let roots = ROOTS.lock().unwrap();
    if roots.is_empty() {
        return None;
    }
    let path = super::destination::final_path(file, VOLUME_NAME_DOS).ok()?;
    roots
        .iter()
        .find(|rule| path.starts_with(&rule.root) && matches(rule, &path))
        .map(|rule| rule.response)
}
