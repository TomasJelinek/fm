//! Links that viewers hand to the system's default handler. The presenter only sends the
//! typed request and turns the backend's answer into the viewer's status text.
use super::*;

/// The most characters of a link's status text, as for the viewers' link descriptions.
const SHOWN_CHARACTERS: usize = 300;
/// An address is never shortened below this many characters, however long the reason.
const SHOWN_ADDRESS: usize = 40;

/// A link handed to the backend, and who hears how it went.
pub(super) struct PendingUrl {
    url: ExternalUrl,
    done: Box<dyn FnOnce(Result<String, String>)>,
}

impl PendingUrl {
    /// Tells `done` how the backend's answer for this link went.
    pub(super) fn finish(self, result: Result<DesktopActionResult, FsError>) {
        let outcome = match result {
            Ok(DesktopActionResult::UrlOpened { .. }) => Ok(opened(&self.url)),
            Ok(_) => Err(not_opened(
                &self.url,
                "The desktop gave an unexpected answer.",
            )),
            Err(error) => Err(not_opened(&self.url, &reason(&error))),
        };
        (self.done)(outcome);
    }
}

impl Presenter {
    /// Hands `url`, a link the user followed in a viewer, to the system's default handler
    /// on a backend worker (`Command::OpenUrl`).
    ///
    /// `done` hears the status text once: `Ok` when the browser or mail program took the
    /// link, `Err` with the reason otherwise. It runs on the UI thread, inside the `apply`
    /// call that delivers the answer, or at once when the request cannot be queued, so
    /// it must not call back into the presenter. Whether the viewer that asked is still
    /// open is for `done` to decide.
    pub fn open_external_url(
        &mut self,
        url: ExternalUrl,
        send: &dyn Fn(Command) -> bool,
        done: Box<dyn FnOnce(Result<String, String>)>,
    ) {
        let request = self.desktop.request();
        let pending = PendingUrl {
            url: url.clone(),
            done,
        };
        if !send(Command::OpenUrl { request, url }) {
            let busy = not_opened(
                &pending.url,
                "Desktop queue busy. Try opening the link again.",
            );
            (pending.done)(Err(busy));
            return;
        }
        self.desktop.urls.insert(request, pending);
    }
}

/// What the viewer says once the browser or mail program took `url`.
fn opened(url: &ExternalUrl) -> String {
    if !url.is_mail() {
        return status_text("Opened in your browser: ", url, "");
    }
    let note = if url.attachments_removed() {
        " (without the file attachments it asked for)"
    } else {
        ""
    };
    status_text("Opened in your mail program: ", url, note)
}

/// What the viewer says when `url` was not opened, and why.
fn not_opened(url: &ExternalUrl, reason: &str) -> String {
    status_text("Could not open ", url, &format!(": {reason}"))
}

/// The first line of the error's message, which says why.
fn reason(error: &FsError) -> String {
    error
        .message
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map_or_else(|| error.kind.to_string(), str::to_owned)
}

/// `before`, the address and `after` as one line of at most [`SHOWN_CHARACTERS`]. The
/// address is shortened first, ending in "…", so that what happened and why stay
/// readable; only a reason too long even then is cut at the end. Bidirectional controls
/// show as escapes (`visible_name`), other control characters as U+FFFD.
fn status_text(before: &str, url: &ExternalUrl, after: &str) -> String {
    let after = visible(after);
    let room = SHOWN_CHARACTERS.saturating_sub(before.chars().count() + after.chars().count());
    let address = at_most(visible(url.as_str()), room.max(SHOWN_ADDRESS));
    at_most(format!("{before}{address}{after}"), SHOWN_CHARACTERS)
}

fn visible(text: &str) -> String {
    visible_name(text)
        .chars()
        .map(|c| if c.is_control() { '\u{fffd}' } else { c })
        .collect()
}

/// `text` cut to at most `limit` characters, ending in "…" when cut.
fn at_most(text: String, limit: usize) -> String {
    if text.chars().count() <= limit {
        return text;
    }
    let mut cut: String = text.chars().take(limit.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

impl Presenter {
    /// Shows `path`, a link's local target that no viewer opens, in the active panel: a
    /// `folder` opens there, and a file is shown in its folder with the cursor on it.
    /// Navigation is read-only: nothing is opened by its program. Returns the viewer's
    /// status text.
    pub fn show_link_target(
        &mut self,
        path: &Path,
        folder: bool,
        send: &dyn Fn(Command) -> bool,
    ) -> String {
        let pane = self.active;
        let shown = at_most(visible(&path.to_string_lossy()), SHOWN_CHARACTERS - 60);
        let (listing, cursor) = if folder {
            (path.to_path_buf(), None)
        } else {
            match path.parent() {
                Some(parent) => (parent.to_path_buf(), Some(path.to_path_buf())),
                None => return format!("{shown} has no folder to be shown in."),
            }
        };
        if !self.navigate(pane, listing, None, send) {
            return "The panel is busy. Follow the link again.".into();
        }
        if let Some(pending) = &mut self.panes[pane.index()].pending {
            pending.wanted_cursor = cursor;
        }
        if folder {
            format!("Opened {shown} in the active panel.")
        } else {
            format!("No viewer shows {shown}; it is shown in the active panel.")
        }
    }
}
