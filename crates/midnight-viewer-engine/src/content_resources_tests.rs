use super::*;
use midnight_core::*;
use std::{
    ffi::OsStr,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};

#[derive(Default)]
struct State {
    changed: AtomicBool,
    short: AtomicBool,
    opens: AtomicUsize,
}

struct Directory {
    state: Arc<State>,
    seekable: bool,
}
struct Source {
    entry: TransferEntry,
    state: Arc<State>,
    cursor: usize,
    seekable: bool,
}

fn failure() -> FsError {
    FsError::from_io(
        "relative resource",
        Path::new(""),
        std::io::Error::other("changed or denied"),
    )
}

impl Source {
    fn boxed(kind: EntryKind, state: Arc<State>, seekable: bool) -> Box<dyn TransferSource> {
        Box::new(Self {
            state,
            seekable,
            cursor: 0,
            entry: TransferEntry {
                identity: TransferIdentity {
                    device: 1,
                    object: [2; 16],
                },
                item: MutationItem {
                    path: PathBuf::new(),
                    kind,
                    metadata: EntryMetadata {
                        size: Some(6),
                        ..EntryMetadata::default()
                    },
                },
            },
        })
    }
}

impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        if self.state.changed.load(Ordering::Acquire) {
            Err(failure())
        } else {
            Ok(())
        }
    }
    fn open_child(
        &self,
        name: &OsStr,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.state.opens.fetch_add(1, Ordering::Relaxed);
        if self.entry.item.kind == EntryKind::Directory && name == "main.css" {
            Ok(Source::boxed(
                EntryKind::File,
                self.state.clone(),
                self.seekable,
            ))
        } else {
            Err(failure())
        }
    }
    fn read_chunk(&mut self, buffer: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        if self.state.short.load(Ordering::Acquire) {
            return Ok(0);
        }
        let length = buffer.len().min(6 - self.cursor);
        buffer[..length].copy_from_slice(&b"body{}"[self.cursor..self.cursor + length]);
        self.cursor += length;
        Ok(length)
    }
    fn supports_read_retry(&self) -> bool {
        self.seekable
    }
    fn retry_read_from(&mut self, offset: u64, cancel: &Cancellation) -> Result<(), FsError> {
        self.revalidate(cancel)?;
        self.cursor = offset as usize;
        Ok(())
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        panic!("resource lookup must not enumerate")
    }
}

impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        Path::new("")
    }
    fn identity(&self) -> TransferIdentity {
        TransferIdentity {
            device: 1,
            object: [1; 16],
        }
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        if self.state.changed.load(Ordering::Acquire) {
            Err(failure())
        } else {
            Ok(())
        }
    }
    fn open_read_child(
        &self,
        name: &OsStr,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        self.state.opens.fetch_add(1, Ordering::Relaxed);
        let kind = if name == "styles" {
            EntryKind::Directory
        } else if name == "文档.css" {
            EntryKind::File
        } else if name == "link.css" {
            EntryKind::Symlink
        } else {
            return Err(failure());
        };
        Ok(Source::boxed(kind, self.state.clone(), self.seekable))
    }
    fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
        panic!("not an authority")
    }
    fn reject_source_ancestor(&self, _: TransferIdentity, _: &Cancellation) -> Result<(), FsError> {
        panic!("not a write")
    }
    fn create_directory(
        &self,
        _: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        panic!("not a write")
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        panic!("not a write")
    }
}

type Fixture = (
    tempfile::TempDir,
    PathBuf,
    CapturedFile,
    Arc<State>,
    Arc<dyn TransferDirectory>,
);
fn fixture(seekable: bool) -> Fixture {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("main 文档.html");
    std::fs::write(&path, b"<html>").unwrap();
    let main = CapturedFile::capture(&path).unwrap();
    let state = Arc::new(State::default());
    let root = Arc::new(Directory {
        state: state.clone(),
        seekable,
    });
    (directory, path, main, state, root)
}

#[test]
fn directory_assets_use_pinned_child_capabilities_and_unicode_without_paths() {
    let (_directory, _path, main, _state, root) = fixture(true);
    let resources = captured_directory_resources(root, main).unwrap();
    let nested = resources
        .capture(&["styles".into(), "main.css".into()])
        .unwrap();
    assert!(nested.path().as_os_str().is_empty());
    assert_eq!(nested.read_at(2, 2).unwrap(), b"dy");
    assert_eq!(nested.read_at(0, 6).unwrap(), b"body{}");
    assert_eq!(
        resources
            .capture(&["文档.css".into()])
            .unwrap()
            .read_at(0, 6)
            .unwrap(),
        b"body{}"
    );
}

#[test]
fn resource_scope_denies_escapes_links_nonseekable_streams_before_adopting_content() {
    let (_directory, _path, main, state, root) = fixture(true);
    let resources = captured_directory_resources(root, main).unwrap();
    for components in [
        vec![],
        vec![".."],
        vec!["."],
        vec!["a/b"],
        vec!["a\\b"],
        vec!["C:"],
        vec!["a\0b"],
        vec![""],
    ] {
        assert!(
            resources
                .capture(&components.into_iter().map(String::from).collect::<Vec<_>>())
                .is_err()
        );
    }
    assert_eq!(state.opens.load(Ordering::Relaxed), 0);
    assert!(resources.capture(&["link.css".into()]).is_err());
    let (_directory, _path, main, _, root) = fixture(false);
    let resources = captured_directory_resources(root, main).unwrap();
    assert!(resources.capture(&["文档.css".into()]).is_err());
}

#[test]
fn resource_reads_reject_changed_scope_main_document_and_short_stream() {
    let (_directory, path, main, state, root) = fixture(true);
    let resources = captured_directory_resources(root, main).unwrap();
    let file = resources.capture(&["文档.css".into()]).unwrap();
    state.short.store(true, Ordering::Release);
    assert!(file.read_at(0, 6).is_err());
    state.short.store(false, Ordering::Release);
    state.changed.store(true, Ordering::Release);
    assert!(file.read_at(0, 6).is_err());
    assert!(resources.capture(&["文档.css".into()]).is_err());
    state.changed.store(false, Ordering::Release);
    std::fs::write(path, b"changed main document").unwrap();
    assert!(file.read_at(0, 6).is_err());
}
