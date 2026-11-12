use super::*;
use state::{Charge, Cursor, CursorData};
use std::{
    collections::HashSet,
    path::Component,
    sync::{
        TryLockError,
        atomic::{AtomicBool, Ordering},
    },
};

impl<A: NativeAdapter> NativeFilesystem<A> {
    pub(super) fn list_begin(
        &self,
        mount: u64,
        node: u64,
        maximum: u32,
        context: &CallContext<'_>,
    ) -> Result<Response, Failure> {
        let (mount, node) = self.node(mount, node)?;
        // Calling ListBegin is explicit directory navigation. Linux may know a
        // link without knowing its target type; the OS directory open decides.
        if !matches!(
            node.kind,
            Kind::Directory | Kind::DirectoryLink | Kind::Symlink
        ) {
            return Err(failure(ErrorCode::Unsupported));
        }
        if A::stat(&node.path, context)?.kind != node.kind {
            return Err(failure(ErrorCode::SourceChanged));
        }
        let slot = self.slots.cursor()?;
        let directory = A::list(&node.path, context)?;
        let id = crate::allocate_handle()?;
        let cursor = Arc::new(Cursor {
            mount,
            directory: node.path.clone(),
            closed: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            data: Mutex::new(CursorData {
                directory,
                next: 0,
                pending: None,
                seen: HashSet::new(),
                charge: Charge::new(self.budget.clone()),
                done: false,
            }),
            _slot: slot,
        });
        {
            let mut state = lock(&self.state);
            if state.cursors.len() >= 32 {
                return Err(failure(ErrorCode::ResourceLimit));
            }
            state
                .cursors
                .try_reserve(1)
                .map_err(|_| failure(ErrorCode::ResourceLimit))?;
            state.cursors.insert(id, cursor);
        }
        self.list_next(id, 0, maximum, context)
    }
    pub(super) fn close_list(&self, id: u64) -> Result<Response, Failure> {
        let retired = {
            let mut state = lock(&self.state);
            let cursor = state
                .cursors
                .remove(&id)
                .ok_or_else(|| failure(ErrorCode::StaleReference))?;
            cursor.closed.store(true, Ordering::Release);
            cursor
        };
        drop(retired);
        Ok(Response::Unit)
    }
    pub(super) fn list_next(
        &self,
        id: u64,
        page: u64,
        maximum: u32,
        context: &CallContext<'_>,
    ) -> Result<Response, Failure> {
        let cursor = lock(&self.state)
            .cursors
            .get(&id)
            .cloned()
            .ok_or_else(|| failure(ErrorCode::StaleReference))?;
        if cursor
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(failure(ErrorCode::Busy));
        }
        let mut data = match cursor.data.try_lock() {
            Ok(data) => data,
            Err(TryLockError::WouldBlock) => {
                cursor.busy.store(false, Ordering::Release);
                return Err(failure(ErrorCode::Busy));
            }
            Err(TryLockError::Poisoned(error)) => error.into_inner(),
        };
        if cursor.closed.load(Ordering::Acquire) || data.next != page {
            cursor.busy.store(false, Ordering::Release);
            return Err(failure(ErrorCode::StaleReference));
        }
        let result = (|| {
            let mut entries = Vec::new();
            entries
                .try_reserve_exact(maximum.min(MAX_BATCH) as usize)
                .map_err(|_| failure(ErrorCode::ResourceLimit))?;
            let mut bytes = 31usize;
            while entries.len() < maximum.min(MAX_BATCH) as usize && !data.done {
                if context.is_cancelled() {
                    return Err(failure(ErrorCode::Cancelled));
                }
                let node = if let Some(node) = data.pending.take() {
                    node
                } else {
                    let entry = match A::next(&mut data.directory, context) {
                        Ok(Some(entry)) => entry,
                        Ok(None) => {
                            data.done = true;
                            break;
                        }
                        Err(error) if error.code == ErrorCode::NotFound => continue,
                        Err(error) => return Err(error),
                    };
                    let mut components = Path::new(&entry.name).components();
                    if !matches!(components.next(), Some(Component::Normal(_)))
                        || components.next().is_some()
                    {
                        return Err(failure(ErrorCode::Invalid));
                    }
                    let record = cursor.mount.bind(
                        cursor.directory.join(&entry.name),
                        entry.info.kind,
                        self.budget.clone(),
                    )?;
                    model::<A>(&record, entry.info)
                };
                let size = 256usize
                    .checked_add(node.address.bytes.len())
                    .and_then(|size| size.checked_add(node.raw_name.len()))
                    .and_then(|size| size.checked_add(node.display.len()))
                    .ok_or_else(|| failure(ErrorCode::ResourceLimit))?;
                if bytes
                    .checked_add(size)
                    .is_none_or(|bytes| bytes > context.config().reply_bytes as usize)
                {
                    if entries.is_empty() {
                        return Err(failure(ErrorCode::ResourceLimit));
                    }
                    data.pending = Some(node);
                    break;
                }
                if data.seen.contains(&node.id) {
                    return Err(failure(ErrorCode::SourceChanged));
                }
                data.charge.grow(32)?;
                data.seen
                    .try_reserve(1)
                    .map_err(|_| failure(ErrorCode::ResourceLimit))?;
                data.seen.insert(node.id);
                bytes += size;
                entries.push(node);
            }
            if context.is_cancelled() {
                return Err(failure(ErrorCode::Cancelled));
            }
            if cursor.closed.load(Ordering::Acquire) {
                return Err(failure(ErrorCode::StaleReference));
            }
            data.next = data
                .next
                .checked_add(1)
                .ok_or_else(|| failure(ErrorCode::ResourceLimit))?;
            Ok(Response::Page {
                cursor: id,
                page,
                done: data.done,
                entries,
            })
        })();
        if result.is_err() {
            cursor.closed.store(true, Ordering::Release);
            drop(data);
            let retired = lock(&self.state).cursors.remove(&id);
            drop(retired);
        }
        result
    }
}
