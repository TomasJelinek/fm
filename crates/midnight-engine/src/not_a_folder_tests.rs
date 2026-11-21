use crate::not_a_folder::{NamedEntry, inspect};
use midnight_core::{
    Cancellation, DirectoryService, Entry, EntryMetadata, FsError, SizeDirectoryIdentity,
    SizeDirectoryTarget,
};
use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
    sync::atomic::{AtomicUsize, Ordering},
};

const FILE: &str = "资料 e\u{301} 🧪.txt";
/// Hidden on every platform: a dot name (Linux) with the hidden attribute (Windows).
const HIDDEN: &str = ".hidden 文档.txt";

/// A folder, two files and nothing else, as a native provider inspects them.
#[derive(Default)]
struct Provider {
    inspected: AtomicUsize,
}

impl DirectoryService for Provider {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        unreachable!("the listing already failed")
    }
    fn entry_metadata(&self, path: &Path) -> Result<EntryMetadata, FsError> {
        let hidden = path.file_name().is_some_and(|name| name == HIDDEN);
        Ok(EntryMetadata {
            native_attributes: Some(if hidden { 0x22 } else { 0x20 }),
            ..EntryMetadata::default()
        })
    }
    fn resolve_size_directory(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Option<SizeDirectoryTarget>, FsError> {
        self.inspected.fetch_add(1, Ordering::SeqCst);
        match path.file_name().and_then(|name| name.to_str()) {
            Some("folder") => Ok(Some(SizeDirectoryTarget {
                path: path.to_owned(),
                identity: SizeDirectoryIdentity {
                    device: 1,
                    object: [2; 16],
                },
            })),
            Some(FILE | HIDDEN) => Ok(None),
            _ => Err(FsError::from_io(
                "resolve directory size target",
                path,
                ErrorKind::NotFound.into(),
            )),
        }
    }
}

fn root() -> PathBuf {
    std::env::temp_dir().join("listed 文件夹")
}

fn failure(path: &Path, kind: ErrorKind) -> FsError {
    FsError::from_io("read directory", path, kind.into())
}

#[test]
fn a_path_that_names_a_file_names_its_folder_and_whether_it_is_hidden() {
    let provider = Provider::default();
    let cancel = Cancellation::default();
    for (name, hidden) in [(FILE, false), (HIDDEN, true)] {
        let path = root().join(name);
        assert_eq!(
            inspect(
                &provider,
                &path,
                &failure(&path, ErrorKind::NotADirectory),
                &cancel
            ),
            Some(NamedEntry {
                parent: root(),
                hidden
            }),
            "{name}"
        );
    }
}

#[test]
fn folders_missing_paths_and_other_failures_name_no_entry() {
    let provider = Provider::default();
    let cancel = Cancellation::default();
    for name in ["folder", "missing.txt"] {
        let path = root().join(name);
        let error = failure(&path, ErrorKind::NotADirectory);
        assert_eq!(inspect(&provider, &path, &error, &cancel), None, "{name}");
    }
    assert_eq!(provider.inspected.swap(0, Ordering::SeqCst), 2);
    // Only "not a directory" asks the provider: other failures cost no request.
    let file = root().join(FILE);
    for kind in [
        ErrorKind::NotFound,
        ErrorKind::PermissionDenied,
        ErrorKind::TimedOut,
        ErrorKind::Other,
    ] {
        assert_eq!(
            inspect(&provider, &file, &failure(&file, kind), &cancel),
            None,
            "{kind:?}"
        );
    }
    let error = failure(&file, ErrorKind::NotADirectory);
    // A relative path or a root names no entry in a folder.
    let relative = Path::new(FILE);
    assert_eq!(inspect(&provider, relative, &error, &cancel), None);
    let top = root().ancestors().last().unwrap().to_owned();
    assert_eq!(inspect(&provider, &top, &error, &cancel), None);
    assert_eq!(provider.inspected.load(Ordering::SeqCst), 0);
    cancel.cancel();
    assert_eq!(inspect(&provider, &file, &error, &cancel), None);
    assert_eq!(provider.inspected.load(Ordering::SeqCst), 0);
}
