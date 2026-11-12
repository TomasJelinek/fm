//! Bounded streaming through host capabilities, with no filesystem access.
use super::*;
use crate::policy::Access;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canceled_resolution_releases_late_resource_without_releasing_main_authority() {
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
            pending: Mutex::new(HashMap::new()),
            selection: Mutex::new(crate::selection::Selection::default()),
            mouse_buttons: AtomicU64::new(0),
            host_shortcuts: false,
        });
        let stream = Arc::new(Mutex::new(Stream {
            resource: None,
            offset: 0,
            bytes: Vec::new(),
            callback: None,
            failed: false,
            pending_offset: 0,
            pending_length: 0,
            owner: Arc::downgrade(&doc),
        }));
        doc.pending.lock().unwrap().insert(7, stream.clone());
        let handler =
            CapturedResource::new(doc.clone(), stream, "https://mewv-1.invalid/a.css".into());
        handler.cancel();
        assert!(doc.pending.lock().unwrap().is_empty());
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
            events.controls.try_recv().unwrap().kind,
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
        assert!(events.controls.try_recv().is_err());
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
    let stream = doc.pending.lock().ok().and_then(|mut p| p.remove(&request));
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
            let Ok(mut stream) = self.stream.lock() else { return 0; };
            if self.url.split(['?', '#']).next() == Some(self.doc.policy.main_url().as_str()) {
                stream.resource = Some(self.doc.source.clone());
                drop(stream);
                callback.cont();
            } else {
                stream.callback = Some(callback.clone());
                drop(stream);
                let prefix = format!("{}/", self.doc.policy.origin());
                let path = self.url.strip_prefix(&prefix).unwrap_or("").to_owned();
                if !self.doc.request(self.stream.clone(), |request| api::EventKind::ResourceResolve {
                    request, base: self.doc.source.id, url: path,
                }) { callback.cancel(); }
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
            if !self.doc.request(self.stream.clone(), |request| api::EventKind::ResourceRead {
                request, resource: resource.id, offset, length,
            }) { callback.cancel(); }
            1
        }
        fn cancel(&self) {
            if let Ok(mut stream) = self.stream.lock() { stream.failed = true; stream.callback = None; }
            if let Ok(mut pending) = self.doc.pending.lock() { pending.retain(|_, value| !Arc::ptr_eq(value, &self.stream)); }
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
            Some(CapturedResource::new(self.doc.clone(), Arc::new(Mutex::new(Stream {
                resource: None, offset: 0, bytes: Vec::new(), callback: None, failed: false,
                pending_offset: 0, pending_length: 0, owner: Arc::downgrade(&self.doc),
            })), url))
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
