//! Optional shared native-author policy. All OS implementation bodies stay in plugins.
use crate::{
    CallContext, Filesystem, Reply,
    api::{abi::Config, *},
};
use std::{
    ffi::{OsStr, OsString},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
mod content;
mod listing;
mod state;
use state::{Budget, Mount, Slots, State, failure, lock, model};

pub struct NativeInfo {
    pub kind: Kind,
    pub metadata: Metadata,
}
pub struct NativeEntry {
    pub name: OsString,
    pub info: NativeInfo,
}
pub trait NativeReader: Send + Sync + 'static {
    fn length(&self) -> u64;
    fn revision(&self) -> &[u8];
    fn validate(&self, context: &CallContext<'_>) -> Result<(), Failure>;
    fn read_at(
        &self,
        offset: u64,
        length: u32,
        context: &CallContext<'_>,
    ) -> Result<Vec<u8>, Failure>;
}
pub trait NativeAdapter: Send + Sync + 'static {
    type Directory: Send + 'static;
    type Reader: NativeReader;
    fn decode(address: &Address) -> Result<PathBuf, Failure>;
    fn encode(path: &Path) -> Address;
    fn name(name: &OsStr) -> (NameEncoding, Vec<u8>);
    fn stat(path: &Path, context: &CallContext<'_>) -> Result<NativeInfo, Failure>;
    fn list(path: &Path, context: &CallContext<'_>) -> Result<Self::Directory, Failure>;
    fn next(
        directory: &mut Self::Directory,
        context: &CallContext<'_>,
    ) -> Result<Option<NativeEntry>, Failure>;
    fn open_reader(path: &Path, context: &CallContext<'_>) -> Result<Self::Reader, Failure>;
    fn volumes(context: &CallContext<'_>) -> Result<Vec<Volume>, Failure>;
    fn storage_kind(path: &Path, context: &CallContext<'_>) -> Result<StorageKind, Failure>;
    fn space_info(path: &Path, context: &CallContext<'_>) -> Result<(u64, u64), Failure>;
    /// Optional capability: the default preserves existing independent adapters.
    fn volume_label(
        _path: &Path,
        _context: &CallContext<'_>,
    ) -> Result<Option<NativeText>, Failure> {
        Err(Failure::new(
            ErrorCode::Unsupported,
            "Volume labels are unavailable.",
        ))
    }
    fn volume_information(
        _path: &Path,
        _context: &CallContext<'_>,
    ) -> Result<VolumeInformation, Failure> {
        Err(Failure::new(
            ErrorCode::Unsupported,
            "Volume information is unavailable.",
        ))
    }
}
pub struct NativeFilesystem<A: NativeAdapter> {
    state: Arc<Mutex<State<A>>>,
    budget: Arc<Budget>,
    slots: Slots,
}
impl<A: NativeAdapter> Filesystem for NativeFilesystem<A> {
    fn create(_: Config) -> Result<Self, Failure> {
        Ok(Self {
            state: Arc::default(),
            budget: Arc::default(),
            slots: Slots::default(),
        })
    }
    fn handle(&self, request: Request, context: &CallContext<'_>) -> Result<Reply, Failure> {
        let response = self.handle_response(request, context)?;
        let state = self.state.clone();
        match &response {
            Response::Resolved { mount, .. } => {
                let mount = *mount;
                Ok(Reply::owned(response, move |published| {
                    if !published {
                        let retired = lock(&state).mounts.remove(&mount);
                        drop(retired);
                    }
                    Ok(())
                }))
            }
            Response::Opened { reader, .. } => {
                let reader = *reader;
                Ok(Reply::owned(response, move |published| {
                    if !published {
                        let retired = lock(&state).readers.remove(&reader);
                        if let Some(lease) = &retired {
                            lease
                                .closed
                                .store(true, std::sync::atomic::Ordering::Release);
                        }
                        drop(retired);
                    }
                    Ok(())
                }))
            }
            Response::Page { cursor, .. } => {
                let id = *cursor;
                let cursor = lock(&state)
                    .cursors
                    .get(&id)
                    .cloned()
                    .ok_or_else(|| failure(ErrorCode::StaleReference))?;
                Ok(Reply::owned(response, move |published| {
                    use std::sync::atomic::Ordering;
                    if !published {
                        cursor.closed.store(true, Ordering::Release);
                        let retired = lock(&state).cursors.remove(&id);
                        drop(retired);
                    }
                    cursor.busy.store(false, Ordering::Release);
                    Ok(())
                }))
            }
            _ => Reply::stateless(response),
        }
    }
}
impl<A: NativeAdapter> NativeFilesystem<A> {
    fn handle_response(
        &self,
        request: Request,
        context: &CallContext<'_>,
    ) -> Result<Response, Failure> {
        match request {
            Request::Resolve { address } => self.resolve(&address, context),
            Request::Stat { mount, node } => {
                let (_mount, node) = self.node(mount, node)?;
                let info = A::stat(&node.path, context)?;
                if info.kind != node.kind {
                    return Err(failure(ErrorCode::SourceChanged));
                }
                Ok(Response::Node(model::<A>(&node, info)))
            }
            Request::ListBegin {
                mount,
                directory,
                max_entries,
            } => self.list_begin(mount, directory, max_entries, context),
            Request::ListNext {
                cursor,
                expected_page,
                max_entries,
            } => self.list_next(cursor, expected_page, max_entries, context),
            Request::CloseList { cursor } => self.close_list(cursor),
            Request::Parent { mount, node } => {
                let (mount, node) = self.node(mount, node)?;
                let Some(path) = node.path.parent() else {
                    return Ok(Response::Parent(None));
                };
                let info = A::stat(path, context)?;
                let record = mount.bind(path.to_owned(), info.kind, self.budget.clone())?;
                Ok(Response::Parent(Some(model::<A>(&record, info))))
            }
            Request::OpenRead { mount, node } => self.open_read(mount, node, context),
            Request::ValidateRead { reader } => self.validate_read(reader, context),
            Request::ReadAt {
                reader,
                offset,
                length,
            } => self.read_at(reader, offset, length, context),
            Request::CloseRead { reader } => self.close_read(reader),
            Request::CloseMount { mount } => self.close_mount(mount),
            Request::Volumes => Ok(Response::Volumes(A::volumes(context)?)),
            Request::StorageKind { address } => Ok(Response::StorageKind(A::storage_kind(
                &Self::path(&address)?,
                context,
            )?)),
            Request::SpaceInfo { address } => {
                let (available, total) = A::space_info(&Self::path(&address)?, context)?;
                Ok(Response::SpaceInfo { available, total })
            }
            Request::VolumeLabel { address } => Ok(Response::VolumeLabel(A::volume_label(
                &Self::path(&address)?,
                context,
            )?)),
            Request::VolumeInformation { address } => Ok(Response::VolumeInformation(
                A::volume_information(&Self::path(&address)?, context)?,
            )),
            Request::Activate { .. } | Request::Refresh { .. } | Request::ResolveText { .. } => {
                Err(Failure::new(
                    ErrorCode::Unsupported,
                    "Native browsing does not provide virtual-location activation.",
                ))
            }
        }
    }
}
impl<A: NativeAdapter> NativeFilesystem<A> {
    fn path(address: &Address) -> Result<PathBuf, Failure> {
        let path = A::decode(address)?;
        if !path.is_absolute() {
            return Err(failure(ErrorCode::Invalid));
        }
        Ok(path)
    }
    fn resolve(&self, address: &Address, context: &CallContext<'_>) -> Result<Response, Failure> {
        let path = Self::path(address)?;
        let slot = self.slots.mount()?;
        let info = A::stat(&path, context)?;
        let mount = Arc::new(Mount::new(slot));
        let node = mount.bind(path, info.kind, self.budget.clone())?;
        let id = crate::allocate_handle()?;
        {
            let mut state = lock(&self.state);
            if state.mounts.len() >= 64 {
                return Err(failure(ErrorCode::ResourceLimit));
            }
            state
                .mounts
                .try_reserve(1)
                .map_err(|_| failure(ErrorCode::ResourceLimit))?;
            state.mounts.insert(id, mount);
        }
        Ok(Response::Resolved {
            mount: id,
            node: model::<A>(&node, info),
        })
    }
    fn node(&self, mount: u64, node: u64) -> Result<(Arc<Mount>, Arc<state::Record>), Failure> {
        let mount = lock(&self.state)
            .mounts
            .get(&mount)
            .cloned()
            .ok_or_else(|| failure(ErrorCode::StaleReference))?;
        let node = lock(&mount.nodes)
            .by_id
            .get(&node)
            .cloned()
            .ok_or_else(|| failure(ErrorCode::StaleReference))?;
        Ok((mount, node))
    }
    fn close_mount(&self, id: u64) -> Result<Response, Failure> {
        let retired = {
            let mut state = lock(&self.state);
            let mount = state
                .mounts
                .get(&id)
                .ok_or_else(|| failure(ErrorCode::StaleReference))?;
            // Every active operation/cursor/reader owns a clone. Initial clones
            // are acquired under this same state lock, so close cannot race one.
            if Arc::strong_count(mount) != 1 {
                return Err(failure(ErrorCode::Busy));
            }
            state.mounts.remove(&id)
        };
        drop(retired);
        Ok(Response::Unit)
    }
}
#[cfg(test)]
mod tests;
