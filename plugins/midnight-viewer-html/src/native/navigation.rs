//! CEF-only admission bridge. Public capability work is performed by the pure state machine.
use super::{admission, *};
use crate::navigation_route::{BrowseFacts, BrowseRoute, classify};
use crate::policy::Access;

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;

const MARKER_HEADER: &str = "X-Midnight-Viewer-Attempt";
const CANNOT_PREPARE: &str = "Cannot prepare navigation; previous contents remain available.";

#[derive(Debug, PartialEq, Eq)]
enum Marker {
    Absent,
    Value(String),
    Invalid,
}

fn marker_from_headers(headers: impl IntoIterator<Item = (String, Vec<String>)>) -> Marker {
    let mut marker = None;
    for (name, values) in headers {
        if !name.eq_ignore_ascii_case(MARKER_HEADER) {
            continue;
        }
        for value in values {
            if marker.is_some()
                || value.is_empty()
                || value.len() > 128
                || value.chars().any(char::is_control)
            {
                return Marker::Invalid;
            }
            marker = Some(value);
        }
        if marker.is_none() {
            return Marker::Invalid;
        }
    }
    marker.map_or(Marker::Absent, Marker::Value)
}

fn headers(request: &Request) -> CefStringMultimap {
    let mut headers = CefStringMultimap::new();
    request.header_map(Some(&mut headers));
    headers
}

fn marker(request: &Request) -> Marker {
    marker_from_headers(headers(request))
}

fn without_marker(
    headers: impl IntoIterator<Item = (String, Vec<String>)>,
) -> Vec<(String, Vec<String>)> {
    headers
        .into_iter()
        .filter(|(name, _)| !name.eq_ignore_ascii_case(MARKER_HEADER))
        .collect()
}

fn strip_marker(request: &Request) -> bool {
    if request.is_read_only() != 0 {
        return false;
    }
    let mut clean = CefStringMultimap::new();
    for (name, values) in without_marker(headers(request)) {
        for value in values {
            if !clean.append(&name, &value) {
                return false;
            }
        }
    }
    request.set_header_map(Some(&mut clean));
    marker(request) == Marker::Absent
}

fn same_document_fragment(current: &str, target: &str) -> bool {
    current != target
        && target.contains('#')
        && current.split('#').next() == target.split('#').next()
}

fn take_bootstrap(bootstrap: &AtomicBool, root: &str, target: &str) -> bool {
    target == root
        && bootstrap
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
}

fn get_no_post(request: &Request) -> bool {
    CefString::from(&request.method()).to_string() == "GET" && request.post_data().is_none()
}

fn root(doc: &Document, target: &str) -> bool {
    target.split(['?', '#']).next() == Some(doc.policy.main_url().as_str())
}

fn binding(
    doc: &Document,
    browser: Option<&Browser>,
    frame: Option<&Frame>,
    url: String,
) -> Option<admission::Binding> {
    let browser = browser?;
    let frame = frame?;
    if browser.is_valid() == 0 || frame.is_valid() == 0 || frame.is_main() == 0 {
        return None;
    }
    let current = browser.main_frame()?;
    let frame_id = CefString::from(&frame.identifier()).to_string();
    if current.is_valid() == 0
        || current.is_main() == 0
        || CefString::from(&current.identifier()).to_string() != frame_id
    {
        return None;
    }
    Some(admission::Binding {
        session: doc.header.session,
        document: doc.header.document,
        browser: browser.identifier(),
        frame: frame_id,
        url,
    })
}

pub(super) fn before_browse(
    doc: &Arc<Document>,
    browser: Option<&mut Browser>,
    frame: Option<&mut Frame>,
    request: Option<&mut Request>,
    user_gesture: i32,
    is_redirect: i32,
) -> i32 {
    if !doc.active.load(Ordering::Acquire) || doc.output.failed.load(Ordering::Acquire) {
        return 1;
    }
    let Some(request) = request else {
        return 1;
    };
    let url = CefString::from(&request.url()).to_string();
    let access = doc.policy.access(&url);
    let main = frame.as_deref().is_some_and(|frame| frame.is_main() != 0);
    let request_marker = marker(request);
    // Under host link routing, every unmarked navigation of the main frame is classified,
    // wherever it leads: a link the browser may not load still goes to the host.
    let routed = doc.links.is_some() && main && request_marker == Marker::Absent;
    if access == Access::Deny && !routed {
        return 1;
    }
    match request_marker {
        Marker::Invalid => return 1,
        Marker::Value(value) => {
            if access != Access::Captured || !main || !get_no_post(request) || is_redirect != 0 {
                return 1;
            }
            let Some(bound) = binding(doc, browser.as_deref(), frame.as_deref(), url) else {
                return 1;
            };
            let allowed = doc
                .navigation
                .lock()
                .ok()
                .is_some_and(|mut navigation| navigation.allow(&value, &bound, Instant::now()));
            return i32::from(!allowed);
        }
        Marker::Absent => {}
    }
    let get_no_post = get_no_post(request);
    let bound = if main && access == Access::Captured && get_no_post {
        binding(doc, browser.as_deref(), frame.as_deref(), url.clone())
    } else {
        None
    };
    let current_main_url = bound
        .as_ref()
        .and(frame.as_deref())
        .map(|frame| CefString::from(&frame.url()).to_string());
    let facts = BrowseFacts {
        unmarked: true,
        target: &url,
        main_frame: main,
        current_main_url: current_main_url.as_deref(),
        get_no_post,
        bootstrap_unused: doc.bootstrap.load(Ordering::Acquire),
        same_document_fragment: current_main_url
            .as_deref()
            .is_some_and(|current| same_document_fragment(current, &url)),
        redirect: is_redirect != 0,
        direct_load: request.transition_type().get_raw()
            & TransitionType::DIRECT_LOAD_FLAG.get_raw()
            != 0,
        user_gesture: user_gesture != 0,
        links: doc.links.is_some(),
    };
    match classify(&doc.policy, &facts) {
        BrowseRoute::Refuse => return 1,
        BrowseRoute::Original => return allow_original(doc, main),
        BrowseRoute::Bootstrap => {
            return i32::from(!take_bootstrap(
                &doc.bootstrap,
                &doc.policy.main_url(),
                &url,
            ));
        }
        BrowseRoute::Activate => {
            route_link(doc, |links, now| links.activate(&doc.policy, &url, now));
            return 1;
        }
        BrowseRoute::Deny(message) => {
            route_link(doc, |links, now| links.refuse(message, now));
            return 1;
        }
        BrowseRoute::Prepare => {}
    }
    let Some(bound) = bound else {
        return 1;
    };
    let intent = admission::Intent {
        root: root(doc, &bound.url),
        binding: bound,
    };
    let accepted = doc
        .navigation
        .lock()
        .ok()
        .is_some_and(|mut navigation| navigation.begin(intent, Instant::now()));
    if !accepted {
        doc.emit(api::EventKind::Error {
            code: api::ErrorCode::Denied,
            message: CANNOT_PREPARE.into(),
        });
    }
    1
}

fn allow_original(doc: &Document, main_frame: bool) -> i32 {
    if main_frame && let Ok(mut navigation) = doc.navigation.lock() {
        navigation.cancel_preparation();
    }
    0
}

/// Host link routing: emit what `decide` makes of a link (a `LinkActivated`, `LinkHover`
/// or bounded `Denied` notice), outside the link state's lock. Nothing without routing.
fn route_link<E: IntoIterator<Item = api::EventKind>>(
    doc: &Document,
    decide: impl FnOnce(&mut crate::links::Links, Instant) -> E,
) {
    if !doc.active.load(Ordering::Acquire) || doc.output.failed.load(Ordering::Acquire) {
        return;
    }
    let Some(links) = doc.links.as_ref() else {
        return;
    };
    let events: Vec<_> = match links.lock() {
        Ok(mut links) => decide(&mut links, Instant::now()).into_iter().collect(),
        Err(_) => return,
    };
    for event in events {
        doc.emit(event);
    }
}

/// OnBeforePopup and OnOpenURLFromTab (`target=_blank`, `window.open`, a middle or
/// modified click): no other window or tab ever opens. Under host link routing a link the
/// user followed goes to the host and anything else is refused with a notice.
pub(super) fn open_elsewhere(doc: &Document, target: Option<&CefString>, user_gesture: i32) -> i32 {
    if doc.links.is_some() {
        let url = target.map(ToString::to_string).unwrap_or_default();
        route_link(doc, |links, now| {
            links.open_elsewhere(&doc.policy, &url, user_gesture != 0, now)
        });
    }
    1
}

/// OnStatusMessage under host link routing: Chromium's URL of the link under the pointer,
/// empty once the pointer leaves it, reported to the host as `LinkHover` when it changes.
pub(super) fn status(doc: &Document, value: Option<&CefString>) {
    if doc.links.is_some() {
        let status = value.map(ToString::to_string).unwrap_or_default();
        route_link(doc, |links, now| links.hover(&doc.policy, &status, now));
    }
}

fn reload_key(key: &api::Key) -> bool {
    !key.modifiers.alt
        && !key.modifiers.meta
        && (key.code == "F5" || key.code == "KeyR" && key.modifiers.control)
}

/// F5/Ctrl+R creates a clean explicit user intent, never reuses a history marker.
pub(super) fn reload(instance: &BrowserInstance, key: &api::Key) -> bool {
    if !reload_key(key) {
        return false;
    }
    let doc = &instance.document;
    let Some(frame) = instance.browser.main_frame() else {
        return false;
    };
    let url = CefString::from(&frame.url()).to_string();
    if doc.policy.access(&url) != Access::Captured {
        return false;
    }
    let Some(bound) = binding(doc, Some(&instance.browser), Some(&frame), url) else {
        return false;
    };
    if key.pressed {
        let intent = admission::Intent {
            root: root(doc, &bound.url),
            binding: bound,
        };
        let accepted = doc
            .navigation
            .lock()
            .ok()
            .is_some_and(|mut navigation| navigation.begin(intent, Instant::now()));
        if !accepted {
            doc.emit(api::EventKind::Error {
                code: api::ErrorCode::Denied,
                message: CANNOT_PREPARE.into(),
            });
        }
    }
    true
}

fn refuse(doc: &Document, attempt: u64) {
    if let Ok(mut navigation) = doc.navigation.lock() {
        navigation.refuse(attempt, CANNOT_PREPARE);
    }
}

pub(super) fn drive(instance: &BrowserInstance) {
    let doc = &instance.document;
    if !doc.active.load(Ordering::Acquire) || doc.output.failed.load(Ordering::Acquire) {
        return;
    }
    for _ in 0..8 {
        let Some(request_id) = doc.next_request_id() else {
            doc.error("Browser resource identities exhausted");
            return;
        };
        // The same lock order is used by ordinary resource request admission.
        let effect = {
            let Ok(requests) = doc.requests.lock() else {
                return;
            };
            let Ok(mut navigation) = doc.navigation.lock() else {
                return;
            };
            let room =
                requests.in_flight() + navigation.pending_requests() < resources::MAX_IN_FLIGHT;
            navigation.next_effect(Instant::now(), request_id, room)
        };
        let Some(effect) = effect else {
            break;
        };
        let event = match effect {
            admission::Effect::Resolve { request, base, url } => {
                api::EventKind::ResourceResolve { request, base, url }
            }
            admission::Effect::Read {
                request,
                resource,
                offset,
                length,
            } => api::EventKind::ResourceRead {
                request,
                resource,
                offset,
                length,
            },
            admission::Effect::Release { resource } => api::EventKind::ResourceRelease { resource },
            admission::Effect::Denied { message } => api::EventKind::Error {
                code: api::ErrorCode::Denied,
                message: message.into(),
            },
        };
        doc.emit(event);
    }
    if doc.output.failed.load(Ordering::Acquire) {
        return;
    }
    let reissue = doc
        .navigation
        .lock()
        .ok()
        .and_then(|mut navigation| navigation.next_reissue(Instant::now()));
    let Some(reissue) = reissue else {
        return;
    };
    let Some(frame) = instance.browser.main_frame() else {
        refuse(doc, reissue.attempt);
        return;
    };
    let current = binding(
        doc,
        Some(&instance.browser),
        Some(&frame),
        reissue.binding.url.clone(),
    );
    let private_origin = format!("{}/", doc.policy.origin());
    if current.as_ref() != Some(&reissue.binding)
        || !CefString::from(&frame.url())
            .to_string()
            .starts_with(&private_origin)
        || !doc.active.load(Ordering::Acquire)
        || doc.output.failed.load(Ordering::Acquire)
    {
        refuse(doc, reissue.attempt);
        return;
    }
    let Some(mut request) = request_create() else {
        refuse(doc, reissue.attempt);
        return;
    };
    let mut request_headers = CefStringMultimap::new();
    if !request_headers.append(MARKER_HEADER, &reissue.marker) {
        refuse(doc, reissue.attempt);
        return;
    }
    request.set_url(Some(&CefString::from(reissue.binding.url.as_str())));
    request.set_method(Some(&CefString::from("GET")));
    request.set_post_data(None);
    request.set_header_map(Some(&mut request_headers));
    // Issued was armed before this CEF call. It may synchronously reenter
    // OnBeforeBrowse; no state/pending/response lock is held here.
    frame.load_request(Some(&mut request));
}

pub(super) struct ResponseLease {
    response: Option<admission::Response>,
    owner: std::sync::Weak<Document>,
}

impl ResponseLease {
    pub(super) fn new(response: admission::Response, owner: &Arc<Document>) -> Self {
        Self {
            response: Some(response),
            owner: Arc::downgrade(owner),
        }
    }
    pub(super) fn resource(&self) -> &api::Resource {
        &self
            .response
            .as_ref()
            .expect("live response lease")
            .resource
    }
    pub(super) fn body(&self) -> Option<&[u8]> {
        self.response
            .as_ref()
            .and_then(|response| response.body.as_deref())
    }
}

impl Drop for ResponseLease {
    fn drop(&mut self) {
        let Some(mut response) = self.response.take() else {
            return;
        };
        // Capacity must actually be relinquished before another candidate can
        // acquire the one body credit. Metadata is not lease ownership.
        drop(response.body.take());
        if let Some(doc) = self.owner.upgrade() {
            if response.resource.id != doc.source.id {
                doc.emit(api::EventKind::ResourceRelease {
                    resource: response.resource.id,
                });
            }
            if let Ok(mut navigation) = doc.navigation.lock() {
                navigation.response_finished(response.attempt);
            }
        }
    }
}

#[derive(Clone)]
pub(super) enum RequestAdmission {
    Ordinary,
    Refused,
    Bound(Arc<BoundRequest>),
}

pub(super) struct BoundRequest {
    binding: admission::Binding,
    marker: String,
    native_request: u64,
    stripped: AtomicBool,
    response: Mutex<Option<ResponseLease>>,
}

pub(super) fn for_request(
    doc: &Arc<Document>,
    browser: Option<&Browser>,
    frame: Option<&Frame>,
    request: Option<&Request>,
    is_navigation: i32,
    is_download: i32,
) -> RequestAdmission {
    let Some(request) = request else {
        return RequestAdmission::Refused;
    };
    let value = match marker(request) {
        Marker::Absent => return RequestAdmission::Ordinary,
        Marker::Invalid => return RequestAdmission::Refused,
        Marker::Value(value) => value,
    };
    let url = CefString::from(&request.url()).to_string();
    if !doc.active.load(Ordering::Acquire)
        || doc.output.failed.load(Ordering::Acquire)
        || is_navigation == 0
        || is_download != 0
        || !get_no_post(request)
        || request.resource_type() != ResourceType::MAIN_FRAME
        || doc.policy.access(&url) != Access::Captured
    {
        return RequestAdmission::Refused;
    }
    let Some(bound) = binding(doc, browser, frame, url) else {
        return RequestAdmission::Refused;
    };
    let response = doc
        .navigation
        .lock()
        .ok()
        .and_then(|mut navigation| navigation.claim(&value, &bound));
    let Some(response) = response else {
        return RequestAdmission::Refused;
    };
    RequestAdmission::Bound(Arc::new(BoundRequest {
        binding: bound,
        marker: value,
        native_request: request.identifier(),
        stripped: AtomicBool::new(false),
        response: Mutex::new(Some(ResponseLease::new(response, doc))),
    }))
}

impl RequestAdmission {
    /// Exact per-request completion may arrive without a body handler, including
    /// a real cancellation. Never infer this boundary from a URL/loading event.
    pub(super) fn completed(&self, native_request: Option<u64>) {
        let Self::Bound(bound) = self else {
            return;
        };
        if bound.native_request != 0
            && native_request.is_some_and(|request| request != bound.native_request)
        {
            return;
        }
        let owner = bound
            .response
            .lock()
            .ok()
            .and_then(|mut response| response.take());
        drop(owner);
    }

    pub(super) fn ordinary(&self) -> bool {
        matches!(self, Self::Ordinary)
    }

    fn matches(
        &self,
        doc: &Document,
        browser: Option<&Browser>,
        frame: Option<&Frame>,
        request: &Request,
    ) -> bool {
        let Self::Bound(bound) = self else {
            return false;
        };
        let url = CefString::from(&request.url()).to_string();
        get_no_post(request)
            && request.resource_type() == ResourceType::MAIN_FRAME
            && doc.policy.access(&url) == Access::Captured
            && (bound.native_request == 0 || request.identifier() == bound.native_request)
            && binding(doc, browser, frame, url).as_ref() == Some(&bound.binding)
    }

    pub(super) fn before_load(
        &self,
        doc: &Document,
        browser: Option<&Browser>,
        frame: Option<&Frame>,
        request: &Request,
    ) -> bool {
        match self {
            Self::Ordinary => marker(request) == Marker::Absent,
            Self::Refused => {
                let _ = strip_marker(request);
                false
            }
            Self::Bound(bound) => {
                let valid = self.matches(doc, browser, frame, request)
                    && marker(request) == Marker::Value(bound.marker.clone())
                    && !bound.stripped.swap(true, Ordering::AcqRel)
                    && strip_marker(request);
                if !valid {
                    let owner = bound
                        .response
                        .lock()
                        .ok()
                        .and_then(|mut response| response.take());
                    drop(owner);
                }
                valid
            }
        }
    }

    pub(super) fn take_response(
        &self,
        doc: &Document,
        browser: Option<&Browser>,
        frame: Option<&Frame>,
        request: &Request,
    ) -> Option<ResponseLease> {
        let Self::Bound(bound) = self else {
            return None;
        };
        if !bound.stripped.load(Ordering::Acquire)
            || marker(request) != Marker::Absent
            || !self.matches(doc, browser, frame, request)
        {
            return None;
        }
        bound.response.lock().ok()?.take()
    }
}
