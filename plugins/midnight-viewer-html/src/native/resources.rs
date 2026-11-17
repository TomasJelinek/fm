//! Bounded streaming through host capabilities, with no filesystem access.
use super::*;
use crate::policy::Access;
use std::collections::{HashMap, VecDeque};

/// Host resource requests awaiting a reply. Kept well below the output control
/// capacity, so resource traffic alone can never overflow it.
pub(super) const MAX_IN_FLIGHT: usize = 32;
/// Browser requests waiting for an in-flight slot. Beyond this (bounded memory)
/// further assets are cancelled.
pub(super) const MAX_WAITING: usize = 4096;
/// Longest captured relative URL (path and query) sent for host resolution; longer
/// URLs are denied locally instead of exceeding the protocol's control budget.
pub(super) const MAX_RESOURCE_URL_BYTES: usize = 8 * 1024;

pub(super) enum HostRequest {
    Resolve {
        url: String,
    },
    Read {
        resource: u64,
        offset: u64,
        length: u32,
    },
}

/// In-flight host requests by ID, plus browser requests queued in FIFO order.
#[derive(Default)]
pub(super) struct Requests {
    active: HashMap<u64, Arc<Mutex<Stream>>>,
    waiting: VecDeque<(Arc<Mutex<Stream>>, HostRequest)>,
}

impl Requests {
    pub(super) fn drain(&mut self) -> Vec<Arc<Mutex<Stream>>> {
        let mut streams: Vec<_> = self.active.drain().map(|(_, stream)| stream).collect();
        streams.extend(self.waiting.drain(..).map(|(stream, _)| stream));
        streams
    }
}

impl Document {
    fn request_event(&self, id: u64, request: &HostRequest) -> api::EventKind {
        match request {
            HostRequest::Resolve { url } => api::EventKind::ResourceResolve {
                request: id,
                base: self.source.id,
                url: url.clone(),
            },
            HostRequest::Read {
                resource,
                offset,
                length,
            } => api::EventKind::ResourceRead {
                request: id,
                resource: *resource,
                offset: *offset,
                length: *length,
            },
        }
    }

    /// Sends or queues one host request for `stream`. Excess requests wait for a free
    /// slot instead of failing the asset. Returns false when the request can never be
    /// sent (invalid for the protocol, waiting queue full, retired document); the
    /// caller then cancels the browser request.
    pub(super) fn request(&self, stream: Arc<Mutex<Stream>>, request: HostRequest) -> bool {
        let valid = api::Message::Event(api::Event {
            header: self.header,
            kind: self.request_event(u64::MAX, &request),
        })
        .validate()
        .is_ok();
        if !valid || !self.active.load(Ordering::Acquire) {
            return false;
        }
        let Ok(mut requests) = self.requests.lock() else {
            return false;
        };
        if requests.active.len() >= MAX_IN_FLIGHT {
            if requests.waiting.len() >= MAX_WAITING {
                return false;
            }
            requests.waiting.push_back((stream, request));
            return true;
        }
        let id = self.request_sequence.fetch_add(1, Ordering::Relaxed);
        requests.active.insert(id, stream);
        drop(requests);
        self.emit(self.request_event(id, &request));
        true
    }

    /// Sends queued requests while in-flight slots are free.
    fn admit_waiting(&self) {
        let mut events = Vec::new();
        if let Ok(mut requests) = self.requests.lock() {
            while requests.active.len() < MAX_IN_FLIGHT {
                let Some((stream, request)) = requests.waiting.pop_front() else {
                    break;
                };
                let id = self.request_sequence.fetch_add(1, Ordering::Relaxed);
                requests.active.insert(id, stream);
                events.push(self.request_event(id, &request));
            }
        }
        for event in events {
            self.emit(event);
        }
    }

    /// Removes every active or waiting request of a cancelled browser request.
    fn forget(&self, stream: &Arc<Mutex<Stream>>) {
        if let Ok(mut requests) = self.requests.lock() {
            requests
                .active
                .retain(|_, value| !Arc::ptr_eq(value, stream));
            requests
                .waiting
                .retain(|(value, _)| !Arc::ptr_eq(value, stream));
        }
        self.admit_waiting();
    }
}

/// Relative URL (path and query) of a captured asset below the document root, or
/// None when it must be denied locally: another directory level than the document
/// root, the main document itself, or a URL beyond the resolution budget.
pub(super) fn captured_path(policy: &crate::policy::DocumentPolicy, url: &str) -> Option<String> {
    let path = url.strip_prefix(&policy.main_url())?;
    (!path.is_empty() && path.len() <= MAX_RESOURCE_URL_BYTES).then(|| path.to_owned())
}

pub(super) struct Stream {
    pub resource: Option<api::Resource>,
    pub offset: u64,
    pub bytes: Vec<u8>,
    pub callback: Option<Callback>,
    pub failed: bool,
    pending_offset: u64,
    pending_length: u32,
    owner: std::sync::Weak<Document>,
}

impl Stream {
    fn new(owner: &Arc<Document>) -> Self {
        Self {
            resource: None,
            offset: 0,
            bytes: Vec::new(),
            callback: None,
            failed: false,
            pending_offset: 0,
            pending_length: 0,
            owner: Arc::downgrade(owner),
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        if let (Some(resource), Some(doc)) = (self.resource.take(), self.owner.upgrade())
            && resource.id != doc.source.id
        {
            doc.emit(api::EventKind::ResourceRelease {
                resource: resource.id,
            });
        }
    }
}

pub(super) fn reply(doc: &Document, reply: api::ResourceReply) {
    let request = match &reply {
        api::ResourceReply::Resolved { request, .. }
        | api::ResourceReply::Data { request, .. }
        | api::ResourceReply::Denied { request, .. } => *request,
        api::ResourceReply::Released { .. } => return,
    };
    let stream = doc
        .requests
        .lock()
        .ok()
        .and_then(|mut requests| requests.active.remove(&request));
    let Some(stream) = stream else {
        // Cancellation can win after the host has allocated an asset capability. Its
        // successful reply still transfers ownership, even though no browser request
        // remains to receive it. Return that lease to this active document's broker.
        if let api::ResourceReply::Resolved { resource, .. } = reply
            && resource.id != doc.source.id
        {
            doc.emit(api::EventKind::ResourceRelease {
                resource: resource.id,
            });
        }
        return;
    };
    doc.admit_waiting();
    let Ok(mut stream) = stream.lock() else {
        return;
    };
    match reply {
        api::ResourceReply::Resolved { resource, .. } if stream.resource.is_none() => {
            stream.resource = Some(resource);
        }
        api::ResourceReply::Data {
            resource,
            offset,
            bytes,
            ..
        } if stream.resource.as_ref().is_some_and(|r| r.id == resource)
            && offset == stream.pending_offset
            && bytes.len() <= stream.pending_length as usize
            && !bytes.is_empty() =>
        {
            stream.bytes = bytes;
        }
        _ => stream.failed = true,
    }
    let failed = stream.failed;
    let callback = stream.callback.take();
    drop(stream);
    if let Some(callback) = callback {
        if failed {
            callback.cancel();
        } else {
            callback.cont();
        }
    }
}

wrap_resource_handler! {
    pub(super) struct CapturedResource {
        doc: Arc<Document>,
        stream: Arc<Mutex<Stream>>,
        url: String,
    }
    impl ResourceHandler {
        fn open(&self, _request: Option<&mut Request>, handle_request: Option<&mut i32>, _callback: Option<&mut Callback>) -> i32 {
            // Explicitly request CEF's supported compatibility bridge. The old read callback
            // lets us defer without retaining any native destination pointer across callbacks.
            if let Some(handle) = handle_request { *handle = 0; }
            0
        }
        fn read(&self, _data_out: *mut u8, _bytes_to_read: i32, bytes_read: Option<&mut i32>, _callback: Option<&mut ResourceReadCallback>) -> i32 {
            if let Some(read) = bytes_read { *read = -1; }
            0
        }
        fn process_request(&self, _request: Option<&mut Request>, callback: Option<&mut Callback>) -> i32 {
            if !self.doc.active.load(Ordering::Acquire) { return 0; }
            let Some(callback) = callback else { return 0; };
            if self.url.split(['?', '#']).next() == Some(self.doc.policy.main_url().as_str()) {
                let Ok(mut stream) = self.stream.lock() else { return 0; };
                stream.resource = Some(self.doc.source.clone());
                drop(stream);
                callback.cont();
                return 1;
            }
            // Deny an over-long or out-of-root URL here: it must neither reach the host
            // nor exceed the protocol budget (which used to stop the whole provider).
            let Some(url) = captured_path(&self.doc.policy, &self.url) else { return 0; };
            let Ok(mut stream) = self.stream.lock() else { return 0; };
            stream.callback = Some(callback.clone());
            drop(stream);
            if !self.doc.request(self.stream.clone(), HostRequest::Resolve { url }) {
                if let Ok(mut stream) = self.stream.lock() { stream.callback = None; }
                return 0;
            }
            1
        }
        fn response_headers(&self, response: Option<&mut Response>, response_length: Option<&mut i64>, _redirect_url: Option<&mut CefString>) {
            let Ok(stream) = self.stream.lock() else { return; };
            if let Some(response) = response {
                response.set_status(if stream.failed { 403 } else { 200 });
                if let Some(resource) = &stream.resource {
                    response.set_mime_type(Some(&CefString::from(resource.mime.as_str())));
                    if let Some(length) = response_length { *length = i64::try_from(resource.length).unwrap_or(-1); }
                }
                for (name, value) in [
                    ("Content-Security-Policy", self.doc.policy.csp()),
                    ("Cache-Control", "no-store"), ("X-Content-Type-Options", "nosniff"),
                    ("Referrer-Policy", "no-referrer"),
                    ("Permissions-Policy", "camera=(), microphone=(), geolocation=(), usb=(), serial=(), payment=(), clipboard-read=(), clipboard-write=()"),
                ] { response.set_header_by_name(Some(&CefString::from(name)), Some(&CefString::from(value)), 1); }
            }
        }
        fn read_response(&self, data_out: *mut u8, bytes_to_read: i32, bytes_read: Option<&mut i32>, callback: Option<&mut Callback>) -> i32 {
            let Some(bytes_read) = bytes_read else { return 0; };
            *bytes_read = 0;
            if data_out.is_null() || bytes_to_read <= 0 || !self.doc.active.load(Ordering::Acquire) { return 0; }
            let Ok(mut stream) = self.stream.lock() else { return 0; };
            if stream.failed { return 0; }
            if !stream.bytes.is_empty() {
                let count = stream.bytes.len().min(bytes_to_read as usize);
                // SAFETY: CEF owns data_out for this callback and guarantees bytes_to_read capacity.
                // Only the checked count is copied; no callback output pointer is retained.
                unsafe { std::ptr::copy_nonoverlapping(stream.bytes.as_ptr(), data_out, count); }
                stream.bytes.drain(..count);
                stream.offset += count as u64;
                *bytes_read = count as i32;
                return 1;
            }
            let Some(resource) = stream.resource.clone() else { return 0; };
            if stream.offset >= resource.length { return 0; }
            let Some(callback) = callback else { return 0; };
            if stream.callback.is_some() { return 1; }
            let offset = stream.offset;
            let length = (resource.length-offset).min(bytes_to_read as u64).min(api::MAX_READ_BYTES as u64) as u32;
            stream.pending_offset = offset;
            stream.pending_length = length;
            stream.callback = Some(callback.clone());
            drop(stream);
            if !self.doc.request(self.stream.clone(), HostRequest::Read { resource: resource.id, offset, length }) {
                if let Ok(mut stream) = self.stream.lock() { stream.callback = None; }
                callback.cancel();
            }
            1
        }
        fn cancel(&self) {
            if let Ok(mut stream) = self.stream.lock() { stream.failed = true; stream.callback = None; }
            self.doc.forget(&self.stream);
        }
    }
}

wrap_resource_request_handler! {
    pub(super) struct RequestPolicy { doc: Arc<Document> }
    impl ResourceRequestHandler {
        fn on_before_resource_load(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _callback: Option<&mut Callback>) -> ReturnValue {
            if !self.doc.active.load(Ordering::Acquire) || request.is_none_or(|r| self.doc.policy.access(&CefString::from(&r.url()).to_string()) == Access::Deny) {
                ReturnValue::CANCEL
            } else { ReturnValue::CONTINUE }
        }
        fn resource_handler(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>) -> Option<ResourceHandler> {
            let url = CefString::from(&request?.url()).to_string();
            if self.doc.policy.access(&url) != Access::Captured { return None; }
            Some(CapturedResource::new(self.doc.clone(), Arc::new(Mutex::new(Stream::new(&self.doc))), url))
        }
        fn on_resource_redirect(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _request: Option<&mut Request>, _response: Option<&mut Response>, new_url: Option<&mut CefString>) {
            if let Some(url) = new_url
                && self.doc.policy.access(&url.to_string()) == Access::Deny { *url = CefString::from("https://mewv-denied.invalid/");
            }
        }
        fn on_protocol_execution(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _request: Option<&mut Request>, allow_os_execution: Option<&mut i32>) {
            if let Some(allow) = allow_os_execution { *allow = 0; }
        }
    }
}

#[cfg(test)]
#[path = "resources_tests.rs"]
mod tests;
