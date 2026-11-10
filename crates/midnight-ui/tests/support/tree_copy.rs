//! In-memory transfer endpoint for the real presenter/actor/copy-queue bridge.
use midnight_core::*;
use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    hash::{Hash, Hasher},
    io::{self, Cursor, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

#[derive(Default)]
pub struct Destination {
    pub files: Mutex<HashMap<PathBuf, Vec<u8>>>,
    directories: Mutex<HashSet<PathBuf>>,
    pub readback: Mutex<usize>,
}
pub struct Transfer(pub Arc<Destination>);
fn identity(path: &Path) -> TransferIdentity {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    path.hash(&mut hash);
    TransferIdentity {
        device: 1,
        object: u128::from(hash.finish()).to_le_bytes(),
    }
}
fn source(path: &Path) -> Source {
    let file = path.file_name() == Some(OsStr::new("payload.bin"));
    let count = if path.components().any(|part| part.as_os_str() == "small") {
        12
    } else {
        1000
    };
    let item = MutationItem {
        path: path.to_owned(),
        kind: if file {
            EntryKind::File
        } else {
            EntryKind::Directory
        },
        metadata: EntryMetadata {
            size: file.then_some(count),
            ..Default::default()
        },
    };
    Source {
        entry: TransferEntry {
            item,
            identity: identity(path),
        },
        bytes: if file {
            vec![42; count as usize]
        } else {
            vec![]
        },
        offset: 0,
        child: false,
    }
}
impl TransferService for Transfer {
    fn prepare_checksum_directory(
        &self,
        path: &Path,
        cancel: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        self.prepare_transfer_destination(path, cancel)
    }
    fn prepare_transfer_source(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TransferSource>, FsError> {
        Ok(Box::new(source(path)))
    }
    fn prepare_transfer_destination(
        &self,
        path: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        Ok(Arc::new(Directory {
            path: path.to_owned(),
            state: self.0.clone(),
        }))
    }
}
struct Source {
    entry: TransferEntry,
    bytes: Vec<u8>,
    offset: usize,
    child: bool,
}
impl TransferSource for Source {
    fn entry(&self) -> &TransferEntry {
        &self.entry
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn read_chunk(&mut self, buffer: &mut [u8], _: &Cancellation) -> Result<usize, FsError> {
        let size = buffer.len().min(self.bytes.len() - self.offset);
        buffer[..size].copy_from_slice(&self.bytes[self.offset..self.offset + size]);
        self.offset += size;
        Ok(size)
    }
    fn next_child(&mut self, _: &Cancellation) -> Result<Option<Box<dyn TransferSource>>, FsError> {
        if self.entry.item.kind != EntryKind::Directory || std::mem::replace(&mut self.child, true)
        {
            return Ok(None);
        }
        Ok(Some(Box::new(source(
            &self.entry.item.path.join("payload.bin"),
        ))))
    }
}
struct Directory {
    path: PathBuf,
    state: Arc<Destination>,
}
impl TransferDirectory for Directory {
    fn path(&self) -> &Path {
        &self.path
    }
    fn identity(&self) -> TransferIdentity {
        identity(&self.path)
    }
    fn revalidate(&self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn inspect(&self, _: &OsStr, _: &Cancellation) -> Result<Option<TransferEntry>, FsError> {
        Ok(None)
    }
    fn reject_source_ancestor(
        &self,
        source: TransferIdentity,
        _: &Cancellation,
    ) -> Result<(), FsError> {
        assert!(!self.path.ancestors().any(|path| identity(path) == source));
        Ok(())
    }
    fn create_directory(
        &self,
        name: &OsStr,
        _: &Cancellation,
    ) -> Result<Arc<dyn TransferDirectory>, FsError> {
        let path = self.path.join(name);
        assert!(self.state.directories.lock().unwrap().insert(path.clone()));
        Ok(Arc::new(Directory {
            path,
            state: self.state.clone(),
        }))
    }
    fn stage_file(&self, _: &Cancellation) -> Result<Box<dyn StagedTransferFile>, FsError> {
        Ok(Box::new(Stage {
            data: Cursor::new(vec![]),
            parent: self.path.clone(),
            state: self.state.clone(),
        }))
    }
}
struct Stage {
    data: Cursor<Vec<u8>>,
    parent: PathBuf,
    state: Arc<Destination>,
}
impl Read for Stage {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.data.read(bytes)?;
        *self.state.readback.lock().unwrap() += count;
        Ok(count)
    }
}
impl Write for Stage {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.data.write(bytes)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Seek for Stage {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.data.seek(position)
    }
}
impl StagedTransferFile for Stage {
    fn sync(&mut self, _: &Cancellation) -> Result<(), FsError> {
        Ok(())
    }
    fn len(&self) -> Result<u64, FsError> {
        Ok(self.data.get_ref().len() as u64)
    }
    fn publish_no_replace(
        &mut self,
        name: &OsStr,
        _: &Cancellation,
    ) -> Result<TransferPublication, FsError> {
        assert!(
            self.state
                .files
                .lock()
                .unwrap()
                .insert(self.parent.join(name), self.data.get_ref().clone())
                .is_none()
        );
        Ok(TransferPublication::Published(self.parent.join(name)))
    }
    fn cleanup(&mut self) -> Result<(), FsError> {
        Ok(())
    }
}
