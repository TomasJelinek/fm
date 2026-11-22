//! Whole-file selection (Ctrl+A, `viewer.select-page`) in Text, Source and Markdown views,
//! and its Copy.
//!
//! Ctrl+A selects the whole file: every page the view shows is painted selected while the
//! selection lasts, scrolling and resizing included, and the status line says so. A click
//! or drag, a Find match, another file or mode, or Ctrl+A again replaces it. The hex view
//! keeps selecting the rows it shows, and image, PDF and HTML views keep their own.
//! Painting is unchanged: the renderer gets the rows a raster shows, as for the rows
//! Ctrl+A selected before, and nothing is done per frame without a selection.
//!
//! Copy of the selected file reads it from its start on this worker, in forward viewport
//! requests of the protocol's largest page (256 KiB, 4096 lines), one at a time on the
//! shown session, and joins their text as the viewer shows it (`whole_copy.rs`). The
//! clipboard gets the text once, whole, after the last page; a copy that does not end
//! copies nothing. It is bounded by `whole_copy::LIMIT`, shows its progress after
//! `PROGRESS`, and ends with Esc (`ViewerInput::CancelCopy`, which the window sends while
//! `ViewerOutput::Copying` says a copy runs), another selection, another file or mode, or
//! closing the window. The session reads the revision captured when the file opened, so
//! a file that changes while it is copied fails the read, and the copy says so.
use super::*;
use crate::whole_copy::{JoinError, Joined, LIMIT, Next};
use std::{borrow::Cow, ops::Range, time::Instant};

#[cfg(test)]
#[path = "worker_whole_tests.rs"]
mod tests;

/// One page request of a whole-file copy: the protocol's largest viewport.
const PAGE_BYTES: u32 = viewer::MAX_PAGE_BYTES as u32;
const PAGE_LINES: u32 = 4096;
/// A copy taking longer than this shows its progress.
const PROGRESS: Duration = Duration::from_millis(300);
/// Text and Source decoders give at least one text byte for four source bytes (a UTF-16
/// CRLF); a larger file cannot fit the limit and is refused before it is read.
const SOURCE_PER_TEXT_BYTE: u64 = 4;

#[derive(Default)]
pub(super) struct WholeFile {
    /// Ctrl+A selected the whole file shown.
    selected: bool,
    copy: Option<Copy>,
    /// A copy ended without output at hand; `poll_whole` reports it.
    stopped: bool,
}

struct Copy {
    request: u64,
    /// The request's page, joined once the request completed.
    page: Option<Received>,
    joined: Joined,
    started: Instant,
    shown: Option<u64>,
}

enum Received {
    Text(viewer::TextPage),
    Document(midnight_plugin_host::api::document::DocumentPage),
}

impl WholeFile {
    pub(super) fn selected(&self) -> bool {
        self.selected
    }

    /// Whether `id` is the running copy's page request.
    pub(super) fn copying(&self, id: u64) -> bool {
        self.copy.as_ref().is_some_and(|copy| copy.request == id)
    }
}

/// The selection the renderer paints over a raster whose top is page row `top`, `height`
/// pixels high: the whole file's selection as the rows it shows, any other as it is.
pub(super) fn painted(
    whole: bool,
    selection: Option<(usize, usize)>,
    page: &TextPage,
    top: f32,
    height: f32,
) -> Option<Range<usize>> {
    if !whole {
        return selection.map(|(a, b)| a.min(b)..a.max(b));
    }
    let first = page.line_at_y(top).map_or(0, |(range, _)| range.start);
    let last = page
        .line_at_y(top + height - 0.01)
        .map_or(page.text().len(), |(range, _)| range.end);
    Some(first.min(last)..last)
}

/// `bytes` for the status line.
fn size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["KiB", "MiB", "GiB", "TiB"];
    if bytes < 1024 {
        return format!("{bytes} bytes");
    }
    let mut value = bytes as f64 / 1024.0;
    let mut unit = 0;
    while value >= 1024.0 && unit + 1 < UNITS.len() {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

impl Worker {
    /// Whether Ctrl+A selects the whole file in the shown view.
    fn whole_file_view(&self) -> bool {
        !self.binary
            && matches!(
                self.mode,
                viewer::MODE_TEXT | viewer::MODE_SOURCE | viewer::MODE_DOCUMENT
            )
            && self.shaped.is_some()
    }

    /// Ctrl+A: select the whole file where the view shows text; false in the hex view,
    /// which selects the rows it shows.
    pub(super) fn select_whole_file(&mut self) -> bool {
        if !self.whole_file_view() {
            return false;
        }
        // Another selection ends a copy in progress, this one too.
        self.stop_whole_copy();
        self.whole.selected = true;
        self.selection = None;
        // The last match no longer names the selection a page shows.
        self.found = None;
        self.raster.forget_selection();
        self.frames.selection_changed();
        self.dirty = true;
        true
    }

    /// The whole-file selection gives way to another selection (or none), and its copy
    /// ends. Returns whether there was one.
    pub(super) fn end_whole_selection(&mut self) -> bool {
        if !self.whole.selected {
            return false;
        }
        self.whole.selected = false;
        self.stop_whole_copy();
        self.frames.selection_changed();
        self.dirty = true;
        true
    }

    /// The selection the shown page holds: all of its text while the whole file is.
    pub(super) fn shown_selection(&self) -> Option<(usize, usize)> {
        if self.whole.selected {
            self.shaped.as_ref().map(|page| (0, page.text().len()))
        } else {
            self.selection
        }
    }

    /// The status line's own note: the latest message, or what is selected.
    pub(super) fn status_note(&self) -> Cow<'_, str> {
        if self.status.is_empty() && self.whole.selected {
            Cow::Owned(format!("Whole file selected ({})", size(self.length)))
        } else {
            Cow::Borrowed(&self.status)
        }
    }

    /// Copy of the whole-file selection: start reading the file, or keep the copy that
    /// reads it.
    pub(super) fn copy_whole_file(&mut self, output: &Mailbox) {
        if self.whole.copy.is_some() {
            return;
        }
        if self.mode != viewer::MODE_DOCUMENT
            && self.length > SOURCE_PER_TEXT_BYTE.saturating_mul(LIMIT as u64)
        {
            self.whole_copy_ended(Err(JoinError::Limit), output);
            return;
        }
        match self.request_copy_page(0) {
            Ok(request) => {
                self.whole.copy = Some(Copy {
                    request,
                    page: None,
                    joined: Joined::new(LIMIT),
                    started: Instant::now(),
                    shown: None,
                });
                output.put(ViewerOutput::Copying(true));
            }
            Err(error) => self.status(format!("Cannot copy the whole file: {error}"), output),
        }
    }

    fn request_copy_page(&mut self, from: u64) -> Result<u64, String> {
        let viewport = messages::Viewport {
            source_anchor: from,
            backwards: false,
            max_bytes: PAGE_BYTES,
            max_lines: PAGE_LINES,
        };
        match &self.session {
            Some(session) => session
                .viewport(viewport)
                .map_err(|error| error.to_string()),
            // A simulated provider admits the request as a session would.
            #[cfg(test)]
            None => {
                super::unsent::record(&viewport);
                super::unsent::admitted(&viewport).ok_or_else(|| "no session".to_owned())
            }
            #[cfg(not(test))]
            None => Err("the file is closed".to_owned()),
        }
    }

    /// Esc while a copy runs: stop it. The window always learns that none runs now.
    pub(super) fn cancel_whole_copy(&mut self, output: &Mailbox) {
        self.stop_whole_copy();
        if !self.whole.stopped {
            output.put(ViewerOutput::Copying(false));
        }
        self.poll_whole(output);
    }

    /// Cancel a running copy; `poll_whole` reports it.
    fn stop_whole_copy(&mut self) {
        if let Some(copy) = self.whole.copy.take() {
            if let Some(session) = &self.session {
                session.cancel(copy.request);
            }
            self.whole.stopped = true;
        }
    }

    /// Report a copy that ended where no output was at hand.
    pub(super) fn poll_whole(&mut self, output: &Mailbox) {
        if std::mem::take(&mut self.whole.stopped) {
            self.status(
                "Copy of the whole file stopped; nothing was copied.".into(),
                output,
            );
            output.put(ViewerOutput::Copying(false));
        }
    }

    /// Another file or mode opens, or the session failed: neither the selection nor its
    /// copy goes on.
    pub(super) fn reset_whole_file(&mut self, output: &Mailbox) {
        self.whole.selected = false;
        self.stop_whole_copy();
        self.poll_whole(output);
    }

    /// An event of the copy's page request (`WholeFile::copying`).
    pub(super) fn whole_copy_event(&mut self, event: EventKind, output: &Mailbox) {
        let Some(copy) = self.whole.copy.as_mut() else {
            return;
        };
        match event {
            EventKind::TextPage(page) => copy.page = Some(Received::Text(page)),
            EventKind::DocumentPage(page) => copy.page = Some(Received::Document(page)),
            EventKind::Completed => self.whole_copy_page(output),
            EventKind::Failed(failure) => {
                self.whole.copy = None;
                self.whole_copy_failed(&failure.message, output);
            }
            EventKind::Cancelled => {
                self.whole.copy = None;
                self.whole_copy_failed("the viewer cancelled the read", output);
            }
            // Progress and anything else no page request answers.
            _ => {}
        }
    }

    /// The copy's request completed: join its page and read on, or end.
    fn whole_copy_page(&mut self, output: &Mailbox) {
        let Some(mut copy) = self.whole.copy.take() else {
            return;
        };
        let next = match copy.page.take() {
            Some(Received::Text(page)) => copy.joined.text_page(&page),
            Some(Received::Document(page)) => copy.joined.document_page(&page),
            None => {
                self.whole_copy_failed("the viewer sent no page", output);
                return;
            }
        };
        match next {
            Ok(Next::Done) => self.whole_copy_ended(Ok(copy.joined), output),
            Ok(Next::Read(from)) => match self.request_copy_page(from) {
                Ok(request) => {
                    copy.request = request;
                    self.whole_copy_progress(&mut copy, from, output);
                    self.whole.copy = Some(copy);
                }
                Err(error) => self.whole_copy_failed(&error, output),
            },
            Err(error) => self.whole_copy_ended(Err(error), output),
        }
    }

    fn whole_copy_progress(&mut self, copy: &mut Copy, from: u64, output: &Mailbox) {
        if copy.started.elapsed() < PROGRESS {
            return;
        }
        let percent = (u128::from(from) * 100 / u128::from(self.length.max(1))).min(99) as u64;
        if copy.shown != Some(percent) {
            copy.shown = Some(percent);
            self.status(
                format!(
                    "Copying the whole file… {percent}% ({} of text; Esc stops)",
                    size(copy.joined.len() as u64)
                ),
                output,
            );
        }
    }

    fn whole_copy_failed(&mut self, reason: &str, output: &Mailbox) {
        self.status(
            format!("Cannot copy the whole file: {reason}. Nothing was copied."),
            output,
        );
        output.put(ViewerOutput::Copying(false));
    }

    fn whole_copy_ended(&mut self, result: Result<Joined, JoinError>, output: &Mailbox) {
        match result {
            Ok(joined) => {
                let text = joined.finish();
                self.status(
                    format!(
                        "Copied the whole file: {} of text.",
                        size(text.len() as u64)
                    ),
                    output,
                );
                output.put(ViewerOutput::CopyText(text));
                output.put(ViewerOutput::Copying(false));
            }
            Err(JoinError::Limit) => {
                self.status(
                    format!(
                        "The whole file is too large to copy as text (more than {}); nothing \
                         was copied. Copy the file itself instead (F5 in the file panel).",
                        size(LIMIT as u64)
                    ),
                    output,
                );
                output.put(ViewerOutput::Copying(false));
            }
            Err(JoinError::Changed) => {
                self.whole_copy_failed("the file changed while it was read", output)
            }
            Err(JoinError::Discontinuous) => {
                self.whole_copy_failed("the viewer's pages do not join", output)
            }
            Err(JoinError::Memory) => self.whole_copy_failed("not enough memory", output),
        }
    }
}
