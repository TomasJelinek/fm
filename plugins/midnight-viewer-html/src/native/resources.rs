//! Bounded streaming through host capabilities, with no filesystem access.
use super::*;
use crate::policy::Access;
use std::collections::{HashMap, VecDeque};

/// Ordinary and navigation host requests awaiting a reply. Kept well below the
/// output control capacity, so resource traffic alone can never overflow it.
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
    pub(super) fn in_flight(&self) -> usize {
        self.active.len()
    }
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
        let Ok(navigation) = self.navigation.lock() else {
            return false;
        };
        if requests.active.len() + navigation.pending_requests() >= MAX_IN_FLIGHT {
            if requests.waiting.len() >= MAX_WAITING {
                return false;
            }
            requests.waiting.push_back((stream, request));
            return true;
        }
        let id = self.request_sequence.fetch_add(1, Ordering::Relaxed);
        requests.active.insert(id, stream);
        drop(navigation);
        drop(requests);
        self.emit(self.request_event(id, &request));
        true
    }

    /// Sends queued requests while in-flight slots are free.
    fn admit_waiting(&self) {
        let mut events = Vec::new();
        if let Ok(mut requests) = self.requests.lock() {
            let Ok(navigation) = self.navigation.lock() else {
                return;
            };
            while requests.active.len() + navigation.pending_requests() < MAX_IN_FLIGHT {
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

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    /// The next event the provider writes, or `None` once every lane is empty. Resource
    /// releases wait in their own lane after the FIFO controls (`output::Receiver::next`).
    pub(in crate::native) fn written(
        doc: &Document,
        events: &mut crate::output::Receiver,
    ) -> Option<api::EventKind> {
        doc.output.finish();
        events.next(&doc.output).map(|event| event.kind)
    }

    pub(in crate::native) fn document() -> (Arc<Document>, crate::output::Receiver) {
        let (output, events) = crate::output::channel();
        let viewport = api::Viewport {
            revision: 1,
            width: 1,
            height: 1,
            scale: 1.0,
        };
        let doc = Arc::new(Document {
            header: api::Header {
                session: 1,
                document: 1,
                sequence: 1,
            },
            source: api::Resource {
                id: 1,
                length: 0,
                mime: "text/html".into(),
            },
            policy: crate::policy::DocumentPolicy::new(1, false, false).unwrap(),
            viewport: Mutex::new(viewport),
            surface: Mutex::new(crate::surface::Surface::new(1, 1, 1).unwrap()),
            layers: Mutex::new(crate::surface::Layers::default()),
            output,
            active: AtomicBool::new(true),
            closed: AtomicBool::new(false),
            finding: Mutex::new(crate::find::FindQueue::default()),
            request_sequence: AtomicU64::new(1),
            requests: Mutex::new(Requests::default()),
            selection: Mutex::new(crate::selection::Selection::default()),
            mouse_buttons: AtomicU64::new(0),
            wheel_gesture: Mutex::default(),
            host_shortcuts: false,
            navigation: Mutex::new(admission::Navigation::new(
                1,
                1,
                api::Resource {
                    id: 1,
                    length: 0,
                    mime: "text/html".into(),
                },
            )),
            bootstrap: AtomicBool::new(true),
        });
        (doc, events)
    }

    #[test]
    fn canceled_resolution_releases_late_resource_without_releasing_main_authority() {
        let (doc, mut events) = document();
        let stream = Arc::new(Mutex::new(Stream::new(&doc, None)));
        doc.requests
            .lock()
            .unwrap()
            .active
            .insert(7, stream.clone());
        let handler = CapturedResource::new(
            doc.clone(),
            stream,
            "https://mewv-1.invalid/mewv-document/a.css".into(),
        );
        handler.cancel();
        assert!(doc.requests.lock().unwrap().active.is_empty());
        reply(
            &doc,
            api::ResourceReply::Resolved {
                request: 7,
                resource: api::Resource {
                    id: 42,
                    length: 20,
                    mime: "text/css".into(),
                },
            },
        );
        assert_eq!(
            written(&doc, &mut events).unwrap(),
            api::EventKind::ResourceRelease { resource: 42 }
        );
        reply(
            &doc,
            api::ResourceReply::Resolved {
                request: 8,
                resource: doc.source.clone(),
            },
        );
        reply(
            &doc,
            api::ResourceReply::Data {
                request: 9,
                resource: 42,
                offset: 0,
                bytes: vec![1],
            },
        );
        reply(
            &doc,
            api::ResourceReply::Denied {
                request: 10,
                reason: "canceled".into(),
            },
        );
        assert!(written(&doc, &mut events).is_none());
    }

    pub(in crate::native) fn admitted(doc: &Arc<Document>) -> navigation::ResponseLease {
        let now = Instant::now();
        let bound = admission::Binding {
            session: 1,
            document: 1,
            browser: 1,
            frame: "main".into(),
            url: "https://mewv-1.invalid/mewv-document/a".into(),
        };
        let response = {
            let mut nav = doc.navigation.lock().unwrap();
            assert!(nav.begin(
                admission::Intent {
                    binding: bound.clone(),
                    root: false
                },
                now
            ));
            assert_eq!(
                nav.next_effect(now, 100, true),
                Some(admission::Effect::Resolve {
                    request: 100,
                    base: 1,
                    url: "a".into(),
                })
            );
            nav.receive(
                api::ResourceReply::Resolved {
                    request: 100,
                    resource: api::Resource {
                        id: 42,
                        length: 3,
                        mime: "text/html".into(),
                    },
                },
                now,
            )
            .unwrap();
            assert_eq!(
                nav.next_effect(now, 101, true),
                Some(admission::Effect::Read {
                    request: 101,
                    resource: 42,
                    offset: 0,
                    length: 3,
                })
            );
            nav.receive(
                api::ResourceReply::Data {
                    request: 101,
                    resource: 42,
                    offset: 0,
                    bytes: b"abc".to_vec(),
                },
                now,
            )
            .unwrap();
            let reissue = nav.next_reissue(now).unwrap();
            assert!(nav.allow(&reissue.marker, &bound, now));
            nav.claim(&reissue.marker, &bound).unwrap()
        };
        navigation::ResponseLease::new(response, doc)
    }

    #[test]
    fn admitted_response_cursor_releases_once_at_eof_without_postallow_host_read() {
        let (doc, mut events) = document();
        let stream = Arc::new(Mutex::new(Stream::new(&doc, Some(admitted(&doc)))));
        let handler = CapturedResource::new(
            doc.clone(),
            stream.clone(),
            "https://mewv-1.invalid/mewv-document/a".into(),
        );
        let mut bytes = [0_u8; 2];
        let mut count = 0;
        assert_eq!(
            handler.read_response(bytes.as_mut_ptr(), 2, Some(&mut count), None),
            1
        );
        assert_eq!(count, 2);
        assert_eq!(&bytes, b"ab");
        assert!(written(&doc, &mut events).is_none());
        assert_eq!(
            handler.read_response(bytes.as_mut_ptr(), 2, Some(&mut count), None),
            1
        );
        assert_eq!(count, 1);
        assert_eq!(bytes[0], b'c');
        assert_eq!(
            written(&doc, &mut events).unwrap(),
            api::EventKind::ResourceRelease { resource: 42 }
        );
        assert_eq!(
            handler.read_response(bytes.as_mut_ptr(), 2, Some(&mut count), None),
            0
        );
        assert!(doc.requests.lock().unwrap().active.is_empty());
        drop(handler);
        drop(stream);
        assert!(
            written(&doc, &mut events).is_none(),
            "metadata must not release the lease twice"
        );
    }

    #[test]
    fn actual_response_owner_drop_unblocks_latest_only_after_serving_allocation_is_gone() {
        let (doc, mut events) = document();
        let stream = Stream::new(&doc, Some(admitted(&doc)));
        let now = Instant::now();
        let bound = admission::Binding {
            session: 1,
            document: 1,
            browser: 1,
            frame: "main".into(),
            url: "https://mewv-1.invalid/mewv-document/b".into(),
        };
        {
            let mut nav = doc.navigation.lock().unwrap();
            assert!(nav.begin(
                admission::Intent {
                    binding: bound,
                    root: false
                },
                now
            ));
            assert!(nav.next_effect(now, 102, true).is_none());
        }
        drop(stream);
        assert_eq!(
            written(&doc, &mut events).unwrap(),
            api::EventKind::ResourceRelease { resource: 42 }
        );
        assert_eq!(
            doc.navigation.lock().unwrap().next_effect(now, 103, true),
            Some(admission::Effect::Resolve {
                request: 103,
                base: 1,
                url: "b".into()
            })
        );
        assert!(written(&doc, &mut events).is_none());
    }
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
    pub admitted: Option<navigation::ResponseLease>,
    owns_resource: bool,
}

impl Stream {
    fn new(doc: &Arc<Document>, admitted: Option<navigation::ResponseLease>) -> Self {
        let resource = admitted.as_ref().map(|owner| owner.resource().clone());
        let owns_resource = admitted.is_none();
        Self {
            resource,
            offset: 0,
            bytes: Vec::new(),
            callback: None,
            failed: false,
            pending_offset: 0,
            pending_length: 0,
            owner: Arc::downgrade(doc),
            admitted,
            owns_resource,
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        drop(self.admitted.take());
        if !self.owns_resource {
            return;
        }
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
    let reply = match doc.navigation.lock() {
        Ok(mut navigation) => navigation.receive(reply, Instant::now()).err(),
        Err(_) => {
            doc.output.failed.store(true, Ordering::Release);
            Some(reply)
        }
    };
    let Some(reply) = reply else {
        // Navigation released an in-flight slot; do not strand queued ordinary assets.
        doc.admit_waiting();
        return;
    };
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
    let response = if failed { stream.admitted.take() } else { None };
    drop(stream);
    drop(response);
    doc.admit_waiting();
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
            let saved_callback = callback.clone();
            let Ok(mut stream) = self.stream.lock() else { return 0; };
            if stream.failed { return 0; }
            if !stream.owns_resource
                || self.url.split(['?', '#']).next() == Some(self.doc.policy.main_url().as_str()) {
                if stream.resource.is_none() { stream.resource = Some(self.doc.source.clone()); }
                drop(stream);
                callback.cont();
            } else {
                // Keep main's URL/protocol guard before admitting a captured asset.
                let Some(path) = captured_path(&self.doc.policy, &self.url) else { return 0; };
                stream.callback = Some(saved_callback);
                drop(stream);
                if !self.doc.request(self.stream.clone(), HostRequest::Resolve { url: path }) {
                    if let Ok(mut stream) = self.stream.lock() { stream.callback = None; }
                    return 0;
                }
            }
            1
        }
        fn response_headers(&self, response: Option<&mut Response>, response_length: Option<&mut i64>, _redirect_url: Option<&mut CefString>) {
            let (failed, resource, finished) = {
                let Ok(mut stream) = self.stream.lock() else { return; };
                let finished = if stream.resource.as_ref().is_some_and(|resource| resource.length == 0) {
                    stream.admitted.take()
                } else { None };
                (stream.failed, stream.resource.clone(), finished)
            };
            drop(finished);
            if let Some(response) = response {
                response.set_status(if failed { 403 } else { 200 });
                if let Some(resource) = &resource {
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
            let saved_callback = callback.as_ref().map(|callback| (**callback).clone());
            let Ok(mut stream) = self.stream.lock() else { return 0; };
            if stream.failed { return 0; }
            if let Some(body) = stream.admitted.as_ref().and_then(navigation::ResponseLease::body) {
                let Ok(offset) = usize::try_from(stream.offset) else { return 0; };
                let Some(available) = body.get(offset..) else { return 0; };
                let count = available.len().min(bytes_to_read as usize);
                // SAFETY: CEF supplies data_out capacity for this callback. The
                // immutable admitted body stays owned through the checked copy.
                unsafe { std::ptr::copy_nonoverlapping(available.as_ptr(), data_out, count); }
                stream.offset += count as u64;
                *bytes_read = count as i32;
                let finished = if stream.resource.as_ref().is_some_and(|resource| stream.offset == resource.length) {
                    stream.admitted.take()
                } else { None };
                drop(stream);
                drop(finished);
                return i32::from(count != 0);
            }
            if !stream.bytes.is_empty() {
                let count = stream.bytes.len().min(bytes_to_read as usize);
                // SAFETY: CEF owns data_out for this callback and guarantees bytes_to_read capacity.
                // Only the checked count is copied; no callback output pointer is retained.
                unsafe { std::ptr::copy_nonoverlapping(stream.bytes.as_ptr(), data_out, count); }
                stream.bytes.drain(..count);
                stream.offset += count as u64;
                *bytes_read = count as i32;
                let finished = if stream.resource.as_ref().is_some_and(|resource| stream.offset == resource.length) {
                    stream.admitted.take()
                } else { None };
                drop(stream);
                drop(finished);
                return 1;
            }
            let Some(resource) = stream.resource.clone() else { return 0; };
            if stream.offset >= resource.length { return 0; }
            let Some(saved_callback) = saved_callback else { return 0; };
            if stream.callback.is_some() { return 1; }
            let offset = stream.offset;
            let length = (resource.length-offset).min(bytes_to_read as u64).min(api::MAX_READ_BYTES as u64) as u32;
            stream.pending_offset = offset;
            stream.pending_length = length;
            stream.callback = Some(saved_callback);
            drop(stream);
            if !self.doc.request(self.stream.clone(), HostRequest::Read { resource: resource.id, offset, length }) {
                if let Ok(mut stream) = self.stream.lock() { stream.callback = None; }
                if let Some(callback) = callback { callback.cancel(); }
            }
            1
        }
        fn cancel(&self) {
            let (callback, response) = if let Ok(mut stream) = self.stream.lock() {
                stream.failed = true;
                (stream.callback.take(), stream.admitted.take())
            } else { (None, None) };
            drop(callback);
            drop(response);
            self.doc.forget(&self.stream);
        }
    }
}

wrap_resource_request_handler! {
    pub(super) struct RequestPolicy { doc: Arc<Document>, admission: navigation::RequestAdmission }
    impl ResourceRequestHandler {
        fn on_before_resource_load(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, request: Option<&mut Request>, _callback: Option<&mut Callback>) -> ReturnValue {
            if !self.doc.active.load(Ordering::Acquire) || request.is_none_or(|r|
                !self.admission.before_load(&self.doc, browser.as_deref(), frame.as_deref(), r)
                    || self.doc.policy.access(&CefString::from(&r.url()).to_string()) == Access::Deny) {
                ReturnValue::CANCEL
            } else { ReturnValue::CONTINUE }
        }
        fn resource_handler(&self, browser: Option<&mut Browser>, frame: Option<&mut Frame>, request: Option<&mut Request>) -> Option<ResourceHandler> {
            let request = request?;
            let url = CefString::from(&request.url()).to_string();
            if self.doc.policy.access(&url) != Access::Captured { return None; }
            let admitted = if self.admission.ordinary() { None } else {
                Some(self.admission.take_response(&self.doc, browser.as_deref(), frame.as_deref(), request)?)
            };
            Some(CapturedResource::new(self.doc.clone(), Arc::new(Mutex::new(Stream::new(&self.doc, admitted))), url))
        }
        fn on_resource_redirect(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _request: Option<&mut Request>, _response: Option<&mut Response>, new_url: Option<&mut CefString>) {
            if let Some(url) = new_url
                && self.doc.policy.access(&url.to_string()) == Access::Deny { *url = CefString::from("https://mewv-denied.invalid/");
            }
        }
        fn on_resource_load_complete(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, request: Option<&mut Request>, _response: Option<&mut Response>, _status: UrlrequestStatus, _received_content_length: i64) {
            self.admission.completed(request.as_deref().map(ImplRequest::identifier));
        }
        fn on_protocol_execution(&self, _browser: Option<&mut Browser>, _frame: Option<&mut Frame>, _request: Option<&mut Request>, allow_os_execution: Option<&mut i32>) {
            if let Some(allow) = allow_os_execution { *allow = 0; }
        }
    }
}

#[cfg(test)]
#[path = "resources_tests.rs"]
mod queue_tests;
