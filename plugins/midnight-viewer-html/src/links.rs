//! Host link routing (web-viewer API 0.3, `MIDNIGHT_WEB_VIEWER_LINKS=host-v1`): link
//! targets in the API's format, change-only hover and bounded activations and refusal
//! notices. CEF-free; the native bridge passes the URLs Chromium resolved and the time.
use crate::policy::DocumentPolicy;
use midnight_web_viewer_api as api;
use std::time::{Duration, Instant};

#[cfg(test)]
#[path = "links_tests.rs"]
mod tests;

/// A second activation this soon after one is dropped: one click or double click hands
/// on one link, and a script cannot turn one user gesture into a burst of host events.
const ACTIVATION_INTERVAL: Duration = Duration::from_millis(500);
/// At most one refusal notice per interval: a page that keeps navigating by itself
/// cannot fill the host's bounded event queue.
const NOTICE_INTERVAL: Duration = Duration::from_secs(1);

pub(crate) const BY_ITSELF: &str =
    "The page tried to open another document by itself; only links you follow are opened.";
pub(crate) const FORM: &str = "Forms are not submitted; only links you follow are opened.";
pub(crate) const ABOVE: &str =
    "This link leads above the 16 folder levels the viewer can tell apart; it is not followed.";
pub(crate) const TOO_LONG: &str =
    "This link's address is longer than 8192 bytes; it is not followed.";
pub(crate) const NO_ADDRESS: &str =
    "This link has no address the viewer can pass on; it is not followed.";

/// Only the exact launch value routes links to the host; any other keeps them in the
/// browser, as without the opt-in.
pub(crate) fn enabled(value: Option<&str>) -> bool {
    value == Some(api::LINKS_HOST_V1)
}

/// The API target of a URL Chromium resolved: relative to the document's folder on the
/// private origin, otherwise the URL itself. Refused with the notice that says why.
pub(crate) fn target(policy: &DocumentPolicy, url: &str) -> Result<String, &'static str> {
    let private = format!("{}/", policy.origin());
    let target = match url.strip_prefix(&private) {
        Some(rest) => relative(rest)?,
        None if scheme(url) => {
            bounded(url)?;
            url.to_owned()
        }
        None => return Err(NO_ADDRESS),
    };
    bounded(&target)?;
    Ok(target)
}

fn bounded(target: &str) -> Result<(), &'static str> {
    if target.len() > api::MAX_LINK_TARGET_BYTES {
        Err(TOO_LONG)
    } else if target.is_empty() || target.chars().any(char::is_control) {
        Err(NO_ADDRESS)
    } else {
        Ok(())
    }
}

/// RFC 3986 `scheme ":"`: a letter, then letters, digits, `+`, `-` or `.`.
fn scheme(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    let mut characters = scheme.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic())
        && characters.all(|next| next.is_ascii_alphanumeric() || matches!(next, '+' | '-' | '.'))
}

/// `rest` follows the private origin's `/`. Its folders are compared with the
/// document's (the synthetic levels below `mewv-document`, `policy.rs`): each one it
/// leaves is a `../`, and a URL outside `mewv-document` lies above the levels the
/// viewer can tell apart.
fn relative(rest: &str) -> Result<String, &'static str> {
    let folders = DocumentPolicy::document_folders();
    let longest = api::MAX_LINK_TARGET_BYTES + folders.iter().map(|f| f.len() + 1).sum::<usize>();
    if rest.len() > longest {
        return Err(TOO_LONG);
    }
    let end = rest.find(['?', '#']).unwrap_or(rest.len());
    let (path, suffix) = rest.split_at(end);
    let mut segments: Vec<&str> = path.split('/').collect();
    let file = segments.pop().unwrap_or_default();
    let shared = folders
        .iter()
        .zip(&segments)
        .take_while(|(folder, segment)| folder == segment)
        .count();
    if shared == 0 {
        return Err(ABOVE);
    }
    let mut target = "../".repeat(folders.len() - shared);
    for segment in &segments[shared..] {
        target.push_str(segment);
        target.push('/');
    }
    target.push_str(file);
    let first = target.split('/').next().unwrap_or_default();
    if shared == folders.len() && (target.is_empty() || first.is_empty() || first.contains(':')) {
        target.insert_str(0, "./");
    }
    target.push_str(suffix);
    Ok(target)
}

/// Link routing state of one document.
#[derive(Debug)]
pub(crate) struct Links {
    /// The hover target the host was last sent; empty at first.
    shown: String,
    /// What the last status message came to, so a repeated one changes nothing.
    hovered: Result<String, &'static str>,
    activated: Option<Instant>,
    noticed: Option<Instant>,
}

impl Default for Links {
    fn default() -> Self {
        Self {
            shown: String::new(),
            hovered: Ok(String::new()),
            activated: None,
            noticed: None,
        }
    }
}

impl Links {
    /// Chromium's status URL of the link under the pointer, empty when it left: a
    /// changed target for the host, or an empty one and a notice when the link's target
    /// cannot be expressed.
    pub(crate) fn hover(
        &mut self,
        policy: &DocumentPolicy,
        status: &str,
        now: Instant,
    ) -> Vec<api::EventKind> {
        let next = if status.is_empty() {
            Ok(String::new())
        } else {
            target(policy, status)
        };
        if next == self.hovered {
            return Vec::new();
        }
        let mut events = Vec::new();
        let shown = next.as_deref().unwrap_or_default();
        if shown != self.shown {
            self.shown = shown.to_owned();
            events.push(api::EventKind::LinkHover {
                target: self.shown.clone(),
            });
        }
        if let Err(message) = next {
            events.extend(self.notice(message, now));
        }
        self.hovered = next;
        events
    }

    /// A link the user followed: its target for the host, or a refusal notice.
    pub(crate) fn activate(
        &mut self,
        policy: &DocumentPolicy,
        url: &str,
        now: Instant,
    ) -> Option<api::EventKind> {
        let target = match target(policy, url) {
            Ok(target) => target,
            Err(message) => return self.notice(message, now),
        };
        if self
            .activated
            .is_some_and(|last| now.saturating_duration_since(last) < ACTIVATION_INTERVAL)
        {
            return None;
        }
        self.activated = Some(now);
        Some(api::EventKind::LinkActivated { target })
    }

    /// A new window or tab: none opens. The user's link goes to the host; anything
    /// else is refused.
    pub(crate) fn open_elsewhere(
        &mut self,
        policy: &DocumentPolicy,
        url: &str,
        user_gesture: bool,
        now: Instant,
    ) -> Option<api::EventKind> {
        if user_gesture {
            self.activate(policy, url, now)
        } else {
            self.refuse(BY_ITSELF, now)
        }
    }

    /// A navigation the browser refused, with its notice when one is due.
    pub(crate) fn refuse(&mut self, message: &'static str, now: Instant) -> Option<api::EventKind> {
        self.notice(message, now)
    }

    fn notice(&mut self, message: &'static str, now: Instant) -> Option<api::EventKind> {
        if self
            .noticed
            .is_some_and(|last| now.saturating_duration_since(last) < NOTICE_INTERVAL)
        {
            return None;
        }
        self.noticed = Some(now);
        Some(api::EventKind::Error {
            code: api::ErrorCode::Denied,
            message: message.into(),
        })
    }
}
