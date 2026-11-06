//! Audited native host boundary. All public plugin-facing handles are owned.
use crate::{guard::lock, jobs::Registration, outbox::Origin};
use midnight_plugin_api::{abi, viewer, wire};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Usage {
    count: usize,
    bytes: usize,
}

struct State {
    api: abi::HostApi,
    usage: Mutex<Usage>,
}

#[derive(Clone)]
pub(crate) struct Host(Arc<State>);

struct Quota {
    host: Host,
    bytes: usize,
}

impl Drop for Quota {
    fn drop(&mut self) {
        let mut usage = lock(&self.host.0.usage);
        usage.count -= 1;
        usage.bytes -= self.bytes;
    }
}

struct Lease {
    buffer: abi::Buffer,
    quota: Quota,
}

// SAFETY: successful host retention pins immutable mapped bytes until release.
// Host callbacks are thread-safe and outlive every lease by Host::new's contract.
unsafe impl Send for Lease {}
unsafe impl Sync for Lease {}

impl Drop for Lease {
    fn drop(&mut self) {
        let api = &self.quota.host.0.api;
        // SAFETY: exactly one successful retain created this lease; all its Arc
        // copies have ended, and the host's callback table remains valid.
        unsafe { api.release_buffer.unwrap()(api.context, self.buffer.id) };
    }
}

/// Immutable bytes retained from the host. Clones share one native lease.
#[derive(Clone, Default)]
pub struct Content(Option<Arc<Lease>>);

impl AsRef<[u8]> for Content {
    fn as_ref(&self) -> &[u8] {
        let Some(lease) = &self.0 else {
            return &[];
        };
        if lease.buffer.bytes.len == 0 {
            return &[];
        }
        // SAFETY: retain validated metadata and acquired the immutable allocation.
        // The Arc keeps it pinned for this borrow; length is bounded to 256 KiB.
        unsafe {
            std::slice::from_raw_parts(lease.buffer.bytes.data, lease.buffer.bytes.len as usize)
        }
    }
}

impl Host {
    #[cfg(test)]
    pub(crate) fn api(&self) -> abi::HostApi {
        self.0.api
    }

    /// # Safety
    /// Callbacks/context must stay valid until all instances, jobs and leases
    /// quiesce, accept concurrent backend calls, and never unwind through C.
    /// They must implement the API ownership contract, including schedule rejection.
    pub(crate) unsafe fn new(api: abi::HostApi) -> Result<Self, abi::Status> {
        api.validate(size_of::<abi::HostApi>() as u32)?;
        Ok(Self(Arc::new(State {
            api,
            usage: Mutex::new(Usage::default()),
        })))
    }

    /// # Safety
    /// Nonempty bytes must describe the mapped immutable allocation identified by
    /// this host's buffer ID. It must remain live through the retain callback.
    pub(crate) unsafe fn retain(&self, buffer: abi::Buffer) -> Result<Content, abi::Status> {
        buffer.validate(viewer::MAX_PAGE_BYTES as u32)?;
        if buffer.id == 0 {
            return Ok(Content::default());
        }
        let bytes = buffer.bytes.len as usize;
        {
            let mut usage = lock(&self.0.usage);
            if usage.count >= 128 || bytes > 8 * 1024 * 1024 - usage.bytes {
                return Err(abi::QUEUE_FULL);
            }
            usage.count += 1;
            usage.bytes += bytes;
        }
        let quota = Quota {
            host: self.clone(),
            bytes,
        };
        let api = &self.0.api;
        // SAFETY: the caller supplied this host's live buffer; table validated at construction.
        let status = unsafe { api.retain_buffer.unwrap()(api.context, buffer.id) };
        if status != abi::OK {
            return Err(status);
        }
        Ok(Content(Some(Arc::new(Lease { buffer, quota }))))
    }

    pub(crate) fn leases(&self) -> usize {
        lock(&self.0.usage).count
    }

    pub(crate) fn schedule(&self, job: &Registration, id: u64, class: u32) -> abi::Status {
        if id == 0 || !matches!(class, abi::WORK_CPU | abi::WORK_IO) {
            return abi::INVALID;
        }
        let api = &self.0.api;
        let work = job.work(id, class);
        // SAFETY: work references a registered owned job and is borrowed only for
        // this call. On OK the caller must accept registration before dropping it.
        unsafe { api.schedule_work.unwrap()(api.context, &work) }
    }

    pub(crate) fn service(&self, origin: Origin, kind: u32, payload: &[u8]) -> abi::Status {
        if payload.len() > wire::MAX_MESSAGE_BYTES {
            return abi::INVALID;
        }
        let request = abi::Request {
            id: origin.id,
            session: origin.session,
            generation: origin.generation,
            kind,
            flags: 0,
            payload: bytes(payload),
            content: abi::Buffer {
                id: 0,
                bytes: bytes(&[]),
            },
        };
        let api = &self.0.api;
        // SAFETY: the request and its bounded payload stay borrowed until return.
        unsafe { api.submit_service.unwrap()(api.context, &request) }
    }

    pub(crate) fn cancelled(&self, id: u64) -> bool {
        let api = &self.0.api;
        // SAFETY: validated host callback, with no borrowed native data.
        unsafe { api.cancelled.unwrap()(api.context, id) != 0 }
    }

    pub(crate) fn notify(&self) {
        let api = &self.0.api;
        // SAFETY: validated host callback, with no borrowed native data.
        unsafe { api.notify_ready.unwrap()(api.context) };
    }
}

fn bytes(value: &[u8]) -> abi::ByteSlice {
    abi::ByteSlice {
        data: value.as_ptr(),
        len: value.len() as u32,
        reserved: 0,
    }
}

#[cfg(test)]
#[path = "host_tests.rs"]
pub(crate) mod tests;
