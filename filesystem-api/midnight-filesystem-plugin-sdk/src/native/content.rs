use super::*;
use state::ReadLease;
use std::sync::atomic::{AtomicBool, Ordering};

impl<A: NativeAdapter> NativeFilesystem<A> {
    pub(super) fn open_read(
        &self,
        mount: u64,
        node: u64,
        context: &CallContext<'_>,
    ) -> Result<Response, Failure> {
        let (mount, node) = self.node(mount, node)?;
        if node.kind != Kind::File {
            return Err(failure(ErrorCode::Unsupported));
        }
        let info = A::stat(&node.path, context)?;
        if info.kind != node.kind {
            return Err(failure(ErrorCode::SourceChanged));
        }
        let slot = self.slots.reader()?;
        let reader = A::open_reader(&node.path, context)?;
        reader.validate(context)?;
        if reader.revision().is_empty() || reader.revision().len() > MAX_REVISION {
            return Err(failure(ErrorCode::Invalid));
        }
        let id = crate::allocate_handle()?;
        let length = reader.length();
        let revision = reader.revision().to_vec();
        let lease = Arc::new(ReadLease {
            _mount: mount,
            reader,
            closed: AtomicBool::new(false),
            _slot: slot,
        });
        {
            let mut state = lock(&self.state);
            if state.readers.len() >= 64 {
                return Err(failure(ErrorCode::ResourceLimit));
            }
            state
                .readers
                .try_reserve(1)
                .map_err(|_| failure(ErrorCode::ResourceLimit))?;
            state.readers.insert(id, lease);
        }
        Ok(Response::Opened {
            reader: id,
            length,
            revision,
        })
    }
    fn reader(&self, id: u64) -> Result<Arc<ReadLease<A>>, Failure> {
        lock(&self.state)
            .readers
            .get(&id)
            .cloned()
            .ok_or_else(|| failure(ErrorCode::StaleReference))
    }
    pub(super) fn close_read(&self, id: u64) -> Result<Response, Failure> {
        let retired = {
            let mut state = lock(&self.state);
            let lease = state
                .readers
                .remove(&id)
                .ok_or_else(|| failure(ErrorCode::StaleReference))?;
            lease.closed.store(true, Ordering::Release);
            lease
        };
        drop(retired);
        Ok(Response::Unit)
    }
    pub(super) fn validate_read(
        &self,
        id: u64,
        context: &CallContext<'_>,
    ) -> Result<Response, Failure> {
        let lease = self.reader(id)?;
        if lease.closed.load(Ordering::Acquire) {
            return Err(failure(ErrorCode::StaleReference));
        }
        lease.reader.validate(context)?;
        if lease.closed.load(Ordering::Acquire) {
            return Err(failure(ErrorCode::StaleReference));
        }
        Ok(Response::Unit)
    }
    pub(super) fn read_at(
        &self,
        id: u64,
        offset: u64,
        length: u32,
        context: &CallContext<'_>,
    ) -> Result<Response, Failure> {
        let lease = self.reader(id)?;
        if lease.closed.load(Ordering::Acquire) {
            return Err(failure(ErrorCode::StaleReference));
        }
        let total = lease.reader.length();
        if length == 0
            || length > MAX_READ
            || offset > total
            || offset.checked_add(u64::from(length)).is_none()
        {
            return Err(failure(ErrorCode::Invalid));
        }
        lease.reader.validate(context)?;
        let expected = (total - offset).min(u64::from(length)) as usize;
        let bytes = if expected == 0 {
            vec![]
        } else {
            lease.reader.read_at(offset, expected as u32, context)?
        };
        if bytes.len() != expected {
            return Err(failure(ErrorCode::SourceChanged));
        }
        lease.reader.validate(context)?;
        if context.is_cancelled() {
            return Err(failure(ErrorCode::Cancelled));
        }
        if lease.closed.load(Ordering::Acquire) {
            return Err(failure(ErrorCode::StaleReference));
        }
        let eof = offset + expected as u64 == total;
        Ok(Response::Content { bytes, eof })
    }
}
