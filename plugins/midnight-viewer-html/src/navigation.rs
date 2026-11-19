//! Provider-private captured-main navigation admission; no CEF objects or host imports.
//!
//! The bridge performs policy/method/main-frame checks and all effects outside locks.
//! A successful claim transfers the only target lease and body into its response owner.
use midnight_web_viewer_api as api;
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[cfg(test)]
#[path = "navigation_tests.rs"]
mod tests;

const BODY_BYTES: usize = 1_048_576;
const DEADLINE: Duration = Duration::from_secs(30);
// One current binding and one latest intent are bounded separately from body/Data.
const BINDING_BYTES: usize = api::MAX_CONTROL_BYTES / 2;
const FRAME_BYTES: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Binding {
    pub session: u64,
    pub document: u64,
    pub browser: i32,
    pub frame: String,
    pub url: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Intent {
    pub binding: Binding,
    /// Exact original root before query/fragment, never just URL-prefix equality.
    pub root: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    Resolve {
        request: u64,
        base: u64,
        url: String,
    },
    Read {
        request: u64,
        resource: u64,
        offset: u64,
        length: u32,
    },
    Release {
        resource: u64,
    },
    Denied {
        message: &'static str,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Reissue {
    pub attempt: u64,
    pub marker: String,
    pub binding: Binding,
}

#[derive(Debug)]
pub(crate) struct Response {
    pub attempt: u64,
    /// Metadata and lease ownership. The bridge releases a non-main ID once.
    pub resource: api::Resource,
    /// Some = entirely admitted body; None = same resolved capability, streamed.
    pub body: Option<Vec<u8>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Resolve,
    Read,
    Ready,
    Issued,
    Allowed,
    Serving,
}

impl Stage {
    fn protected(self) -> bool {
        matches!(self, Self::Allowed | Self::Serving)
    }
}

struct Candidate {
    attempt: u64,
    intent: Intent,
    deadline: Instant,
}

struct Attempt {
    candidate: Candidate,
    stage: Stage,
    resource: Option<api::Resource>,
    body: Option<Vec<u8>>,
    offset: u64,
}

#[derive(Clone, Copy)]
enum PendingKind {
    Resolve,
    Read {
        resource: u64,
        offset: u64,
        length: u32,
    },
}

struct Pending {
    request: u64,
    attempt: u64,
    kind: PendingKind,
    retired: bool,
    /// An obsolete Read keeps its target lease until the real reply settles.
    retired_resource: Option<u64>,
}

pub(crate) struct Navigation {
    session: u64,
    document: u64,
    source: api::Resource,
    root_url: String,
    last_attempt: u64,
    current: Option<Attempt>,
    latest: Option<Candidate>,
    pending: Option<Pending>,
    // At most the current lease plus one malformed matching Resolved transfer.
    // New public work is blocked until these effects have been taken.
    releases: VecDeque<u64>,
    notice: Option<&'static str>,
    closed: bool,
}

impl Navigation {
    pub(crate) fn new(session: u64, document: u64, source: api::Resource) -> Self {
        Self {
            session,
            document,
            source,
            // The captured document's own URL, below its origin's root, so a binding
            // above the document directory is not admitted (policy.rs).
            root_url: crate::policy::DocumentPolicy::main_url_of(document),
            last_attempt: 0,
            current: None,
            latest: None,
            pending: None,
            releases: VecDeque::new(),
            notice: None,
            closed: false,
        }
    }

    /// Called only for an eligible genuine unmarked user navigation.
    /// Marked callbacks, fragments and initial bootstrap are handled by the bridge.
    pub(crate) fn begin(&mut self, intent: Intent, now: Instant) -> bool {
        self.tick(now);
        if self.closed || !self.valid_intent(&intent) {
            return false;
        }
        self.notice = None;
        let Some(attempt) = self.last_attempt.checked_add(1) else {
            self.latest = None;
            self.discard_unallowed();
            self.notice = Some("Cannot prepare another navigation.");
            return false;
        };
        let Some(deadline) = now.checked_add(DEADLINE) else {
            self.notice = Some("Cannot prepare navigation.");
            return false;
        };
        self.last_attempt = attempt;
        self.discard_unallowed();
        self.latest = Some(Candidate {
            attempt,
            intent,
            deadline,
        });
        self.tick(now);
        true
    }

    /// Unmatched replies are returned without cloning or discarding their owned Data.
    pub(crate) fn receive(
        &mut self,
        reply: api::ResourceReply,
        now: Instant,
    ) -> Result<(), api::ResourceReply> {
        self.receive_with(reply, now, allocate_body)
    }

    // The reservation operation is factored so allocation refusal can exercise the
    // actual state transition deterministically, without a global failing allocator.
    fn receive_with(
        &mut self,
        reply: api::ResourceReply,
        now: Instant,
        allocate: impl FnOnce(usize) -> Option<Vec<u8>>,
    ) -> Result<(), api::ResourceReply> {
        let request = match &reply {
            api::ResourceReply::Resolved { request, .. }
            | api::ResourceReply::Data { request, .. }
            | api::ResourceReply::Denied { request, .. } => *request,
            api::ResourceReply::Released { .. } => return Err(reply),
        };
        if self
            .pending
            .as_ref()
            .is_none_or(|pending| pending.request != request)
        {
            return Err(reply);
        }
        self.tick(now);
        let Some(pending) = self.pending.take() else {
            return Err(reply);
        };
        let current = self
            .current
            .as_ref()
            .map(|attempt| attempt.candidate.attempt);
        if pending.retired || current != Some(pending.attempt) {
            if let Some(resource) = pending.retired_resource {
                self.release(resource);
            }
            if let api::ResourceReply::Resolved { resource, .. } = reply
                && Some(resource.id) != pending.retired_resource
            {
                self.release(resource.id);
            }
            return Ok(());
        }
        match (pending.kind, reply) {
            (PendingKind::Resolve, api::ResourceReply::Resolved { resource, .. }) => {
                self.accept_resource(resource, allocate);
            }
            (
                PendingKind::Read {
                    resource: expected_resource,
                    offset: expected_offset,
                    length,
                },
                api::ResourceReply::Data {
                    resource,
                    offset,
                    bytes,
                    ..
                },
            ) => {
                let valid = self.current.as_ref().is_some_and(|attempt| {
                    let Some(target) = attempt.resource.as_ref() else {
                        return false;
                    };
                    let Some(end) = offset.checked_add(bytes.len() as u64) else {
                        return false;
                    };
                    resource == expected_resource
                        && resource == target.id
                        && offset == expected_offset
                        && offset == attempt.offset
                        && bytes.len() <= length as usize
                        && bytes.len() <= api::MAX_READ_BYTES
                        && bytes.capacity() <= api::MAX_READ_BYTES
                        && end <= target.length
                        && (!bytes.is_empty() || target.length == 0)
                        && attempt.body.as_ref().is_some_and(|body| {
                            body.len()
                                .checked_add(bytes.len())
                                .is_some_and(|end| end <= body.capacity() && end <= BODY_BYTES)
                        })
                });
                if valid {
                    if let Some(attempt) = self.current.as_mut() {
                        if let Some(body) = attempt.body.as_mut() {
                            body.extend_from_slice(&bytes);
                        }
                        attempt.offset += bytes.len() as u64;
                        if attempt
                            .resource
                            .as_ref()
                            .is_some_and(|r| r.length == attempt.offset)
                        {
                            attempt.stage = Stage::Ready;
                        }
                    }
                } else {
                    self.fail("Cannot read the navigation target.");
                }
            }
            (_, api::ResourceReply::Resolved { resource, .. }) => {
                // Even a wrong reply kind can transfer a new genuine lease. Do
                // not return the currently owned ID twice if metadata repeats it.
                if self
                    .current
                    .as_ref()
                    .and_then(|a| a.resource.as_ref())
                    .map(|r| r.id)
                    != Some(resource.id)
                {
                    self.release(resource.id);
                }
                self.fail("Cannot prepare the navigation target.");
            }
            _ => self.fail("Cannot prepare the navigation target."),
        }
        Ok(())
    }

    pub(crate) fn tick(&mut self, now: Instant) {
        if self.closed {
            return;
        }
        if self
            .current
            .as_ref()
            .is_some_and(|a| !a.stage.protected() && now >= a.candidate.deadline)
        {
            self.discard_unallowed();
            self.notice = Some("Navigation preparation timed out.");
        }
        if self
            .latest
            .as_ref()
            .is_some_and(|candidate| now >= candidate.deadline)
        {
            self.latest = None;
            self.notice = Some("Navigation preparation timed out.");
        }
        if self.current.is_none() && self.pending.is_none() && self.releases.is_empty() {
            let Some(candidate) = self.latest.take() else {
                return;
            };
            let root = candidate.intent.root;
            self.current = Some(Attempt {
                candidate,
                stage: Stage::Resolve,
                resource: None,
                body: None,
                offset: 0,
            });
            if root {
                self.accept_resource(self.source.clone(), allocate_body);
            }
        }
    }

    /// request_id is unused and globally unique in this Document. can_request
    /// includes legacy streams plus pending_requests(), including retirement.
    pub(crate) fn next_effect(
        &mut self,
        now: Instant,
        request_id: u64,
        can_request: bool,
    ) -> Option<Effect> {
        self.tick(now);
        if let Some(resource) = self.releases.pop_front() {
            return Some(Effect::Release { resource });
        }
        if let Some(message) = self.notice.take() {
            return Some(Effect::Denied { message });
        }
        if self.closed || !can_request || request_id == 0 || self.pending.is_some() {
            return None;
        }
        let attempt = self.current.as_ref()?;
        let (kind, effect) = match attempt.stage {
            Stage::Resolve => (
                PendingKind::Resolve,
                Effect::Resolve {
                    request: request_id,
                    base: self.source.id,
                    url: attempt
                        .candidate
                        .intent
                        .binding
                        .url
                        .strip_prefix(&self.root_url)?
                        .into(),
                },
            ),
            Stage::Read => {
                let target = attempt.resource.as_ref()?;
                let length = if target.length == 0 {
                    1
                } else {
                    target
                        .length
                        .checked_sub(attempt.offset)?
                        .min(api::MAX_READ_BYTES as u64) as u32
                };
                if length == 0 {
                    return None;
                }
                (
                    PendingKind::Read {
                        resource: target.id,
                        offset: attempt.offset,
                        length,
                    },
                    Effect::Read {
                        request: request_id,
                        resource: target.id,
                        offset: attempt.offset,
                        length,
                    },
                )
            }
            _ => return None,
        };
        self.pending = Some(Pending {
            request: request_id,
            attempt: attempt.candidate.attempt,
            kind,
            retired: false,
            retired_resource: None,
        });
        Some(effect)
    }

    /// Arms Issued before the bridge calls LoadRequest outside its lock.
    pub(crate) fn next_reissue(&mut self, now: Instant) -> Option<Reissue> {
        self.tick(now);
        if self.closed {
            return None;
        }
        let attempt = self.current.as_mut()?;
        if attempt.stage != Stage::Ready {
            return None;
        }
        attempt.stage = Stage::Issued;
        Some(Reissue {
            attempt: attempt.candidate.attempt,
            marker: marker(attempt.candidate.attempt),
            binding: attempt.candidate.intent.binding.clone(),
        })
    }

    /// The bridge has already checked GET/no post, main navigation, current
    /// frame, policy, exactly one marker header and no redirect.
    pub(crate) fn allow(&mut self, value: &str, binding: &Binding, now: Instant) -> bool {
        self.tick(now);
        if self.closed {
            return false;
        }
        let Some(attempt) = self.current.as_mut() else {
            return false;
        };
        if attempt.stage != Stage::Issued || !matches_binding(attempt, value, binding) {
            return false;
        }
        attempt.stage = Stage::Allowed;
        true
    }

    /// A bridge issue/frame failure may refuse only its exact unallowed attempt.
    /// An obsolete callback cannot destroy newer or already accepted delivery.
    pub(crate) fn refuse(&mut self, id: u64, message: &'static str) {
        if self
            .current
            .as_ref()
            .is_some_and(|attempt| attempt.candidate.attempt == id && !attempt.stage.protected())
        {
            self.fail(message);
        }
    }

    /// A genuine ordinary navigation supersedes preparation without announcing
    /// a refusal. An already accepted response retains its ownership boundary.
    pub(crate) fn cancel_preparation(&mut self) {
        self.latest = None;
        self.notice = None;
        self.discard_unallowed();
    }

    /// Called only by a fresh per-request main-navigation handler. URL or native
    /// request ID zero alone never selects a response. A claim is consumed once.
    pub(crate) fn claim(&mut self, value: &str, binding: &Binding) -> Option<Response> {
        if self.closed {
            return None;
        }
        let attempt = self.current.as_mut()?;
        if attempt.stage != Stage::Allowed || !matches_binding(attempt, value, binding) {
            return None;
        }
        let resource = attempt.resource.take()?;
        let body = attempt.body.take();
        attempt.stage = Stage::Serving;
        Some(Response {
            attempt: attempt.candidate.attempt,
            resource,
            body,
        })
    }

    /// Caller must FIRST relinquish the actual body allocation and target lease.
    /// For an Allowed but unclaimed request, call only after its real cancellation
    /// or explicit document teardown; that still-owned lease is released here.
    pub(crate) fn response_finished(&mut self, id: u64) {
        if self
            .current
            .as_ref()
            .is_none_or(|attempt| attempt.candidate.attempt != id || !attempt.stage.protected())
        {
            return;
        }
        if let Some(attempt) = self.current.take() {
            self.discard(attempt);
        }
    }

    pub(crate) fn pending_requests(&self) -> usize {
        usize::from(self.pending.is_some())
    }

    /// Explicit document teardown may finish only a response still owned here.
    /// A Serving response's external owner alone relinquishes its allocation.
    pub(crate) fn unclaimed_allowed_attempt(&self) -> Option<u64> {
        self.current
            .as_ref()
            .filter(|attempt| attempt.stage == Stage::Allowed)
            .map(|attempt| attempt.candidate.attempt)
    }

    /// Stop unallowed work without erasing a response already accepted by CEF.
    /// The bridge explicitly cancels/tears down Allowed/Serving response owners
    /// and then calls response_finished. Retiring host correlation stays live.
    pub(crate) fn close(&mut self) {
        self.closed = true;
        self.latest = None;
        self.notice = None;
        self.discard_unallowed();
    }

    fn valid_intent(&self, intent: &Intent) -> bool {
        let b = &intent.binding;
        let root = b.url.split(['?', '#']).next() == Some(self.root_url.as_str());
        self.session != 0
            && self.document != 0
            && self.source.validate().is_ok()
            && b.session == self.session
            && b.document == self.document
            && b.browser > 0
            && !b.frame.is_empty()
            && b.frame.len() <= FRAME_BYTES
            && b.frame.capacity() <= FRAME_BYTES
            && b.url.len() <= BINDING_BYTES
            && b.url.capacity() <= BINDING_BYTES
            && b.frame
                .len()
                .checked_add(b.url.len())
                .is_some_and(|n| n <= BINDING_BYTES)
            && !b.frame.chars().any(char::is_control)
            && !b.url.chars().any(char::is_control)
            && b.url.starts_with(&self.root_url)
            && intent.root == root
    }

    fn accept_resource(
        &mut self,
        resource: api::Resource,
        allocate: impl FnOnce(usize) -> Option<Vec<u8>>,
    ) {
        let valid =
            resource.validate().is_ok() && resource.mime.capacity() <= api::MAX_CONTROL_BYTES;
        let small = resource.length <= BODY_BYTES as u64;
        if let Some(attempt) = self.current.as_mut() {
            attempt.resource = Some(resource);
        } else {
            self.release(resource.id);
            return;
        }
        if !valid {
            self.fail("Cannot prepare the navigation target.");
            return;
        }
        if small {
            let length = self
                .current
                .as_ref()
                .and_then(|a| a.resource.as_ref())
                .map_or(0, |r| r.length as usize);
            let Some(body) = allocate(length).filter(|body| {
                body.is_empty() && body.capacity() >= length && body.capacity() <= BODY_BYTES
            }) else {
                self.fail("Cannot allocate the navigation target.");
                return;
            };
            if let Some(attempt) = self.current.as_mut() {
                attempt.body = Some(body);
                attempt.stage = Stage::Read;
            }
        } else if let Some(attempt) = self.current.as_mut() {
            attempt.stage = Stage::Ready;
        }
    }

    fn fail(&mut self, message: &'static str) {
        self.discard_unallowed();
        self.notice = Some(message);
    }

    fn discard_unallowed(&mut self) {
        if self
            .current
            .as_ref()
            .is_some_and(|attempt| attempt.stage.protected())
        {
            return;
        }
        if let Some(mut attempt) = self.current.take() {
            if let Some(pending) = self.pending.as_mut() {
                pending.retired = true;
                if pending.attempt == attempt.candidate.attempt
                    && matches!(pending.kind, PendingKind::Read { .. })
                {
                    pending.retired_resource = attempt.resource.take().map(|resource| resource.id);
                }
            }
            self.discard(attempt);
        } else if let Some(pending) = self.pending.as_mut() {
            pending.retired = true;
        }
    }

    fn discard(&mut self, mut attempt: Attempt) {
        // Free the actual body before a later attempt can obtain its one credit.
        drop(attempt.body.take());
        if let Some(resource) = attempt.resource.take() {
            self.release(resource.id);
        }
    }

    fn release(&mut self, id: u64) {
        if id != 0 && id != self.source.id && !self.releases.contains(&id) {
            self.releases.push_back(id);
        }
    }
}

fn allocate_body(length: usize) -> Option<Vec<u8>> {
    let mut body = Vec::new();
    body.try_reserve_exact(length).ok()?;
    (body.capacity() <= BODY_BYTES).then_some(body)
}

fn marker(attempt: u64) -> String {
    format!("nav-{attempt:016x}")
}

fn matches_binding(attempt: &Attempt, value: &str, binding: &Binding) -> bool {
    value == marker(attempt.candidate.attempt) && binding == &attempt.candidate.intent.binding
}
