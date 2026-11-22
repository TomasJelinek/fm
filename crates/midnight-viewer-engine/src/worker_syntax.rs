//! Optional syntax preparation of the shown Source page.
//!
//! The Source viewer colours a page it reads when it holds the lexer state at a line end
//! near before the page; preparation walks the document to a target and keeps such states on
//! the way (`plugins/midnight-viewer-source/src/preparation.rs`). A document that fits one
//! walk (`MAX_SCAN_BYTES`) is prepared whole, once per revision: every page read from it then
//! comes coloured. In a larger one only rows the shown page holds plain are prepared, at rest
//! and nearest the view first, and coloured in place; pages read after coloured ones come
//! coloured too. Scrolling within the page prepares nothing. The notice appears only when
//! the visible rows stay plain at rest for `NOTICE_DELAY`.
use super::*;
use midnight_plugin_host::AdmissionError;
use midnight_plugin_host::api::abi;
use midnight_viewer_render::{ColorSpan, StyleSpan};
use std::ops::Range;
use std::time::Instant;

const MAX_BACKOFF_SHIFT: u8 = 5;
/// The most bytes one preparation scans (the plugin's `PrepareSource::max_scan_bytes`
/// limit). A document no longer than this is prepared whole, once per revision.
pub(super) const MAX_SCAN_BYTES: u32 = 256 * 1024;
/// How long the visible rows stay plain at rest before the status line says that their
/// highlighting is being prepared: rows that are coloured sooner never show the notice.
pub(super) const NOTICE_DELAY: Duration = Duration::from_millis(300);
/// The notice of rows whose highlighting is being prepared.
pub(super) const PREPARING_HIGHLIGHTING: &str = "Preparing source highlighting…";
/// Rows are plain when this many bytes of them carry no colour span: the Source viewer
/// colours keywords, numbers, strings and comments far more densely wherever it held the
/// lexer state, and sends a page it could not colour without spans.
const PLAIN_GAP: usize = 2048;
/// The most rows one refresh asks for: a viewport's largest line budget
/// (`messages::Viewport`). Its reads are joined up to their byte budget
/// (`worker_chain::Budget`).
const MAX_REFRESH_LINES: u32 = 4096;
/// How far before the view's end plain rows may start for one refresh to colour them from
/// their start through the view: half of what one refresh joins (120 KiB,
/// `worker_chain::JOIN_BYTES`), the rest for the rows past the view.
const REFRESH_REACH: usize = 64 * 1024;

#[cfg(test)]
std::thread_local! {
    /// The co-simulations' highlighting clock, which moves a frame of their window at a time.
    static CLOCK: std::cell::Cell<Option<Instant>> = const { std::cell::Cell::new(None) };
}

/// Set this thread's highlighting clock; `None` follows the system's.
#[cfg(test)]
pub(super) fn set_clock(now: Option<Instant>) {
    CLOCK.with(|clock| clock.set(now));
}

/// Move this thread's highlighting clock on by `by`, when it is set.
#[cfg(test)]
pub(super) fn advance_clock(by: Duration) {
    CLOCK.with(|clock| clock.set(clock.get().map(|now| now + by)));
}

/// The time of the notice delay and the retry pacing: the system's, except in the
/// co-simulations (`set_clock`).
fn clock_now() -> Instant {
    #[cfg(test)]
    if let Some(now) = CLOCK.with(std::cell::Cell::get) {
        return now;
    }
    Instant::now()
}

/// What a preparation covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Scope {
    /// The whole document, which fits one scan (`MAX_SCAN_BYTES`).
    Whole,
    /// Plain rows of the shown page, from `Key::viewport`'s anchor and as many rows.
    Rows,
}

/// What the shown page needs prepared (`Worker::wanted_source_target`): the viewport of
/// `rows` rows from `anchor`, or before it when `backwards`.
struct Target {
    scope: Scope,
    anchor: u64,
    backwards: bool,
    rows: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Key {
    generation: u64,
    revision: u64,
    mode: u16,
    epoch: u64,
    pub scope: Scope,
    pub viewport: messages::Viewport,
}

/// A request that source highlighting asks the plugin session to admit
/// (`Worker::source_admission`).
pub(super) enum SourceAdmission {
    Prepare(messages::PrepareSource),
    Refresh(messages::Viewport),
}

struct Active {
    id: u64,
    key: Key,
    result: Option<messages::SourcePrepared>,
}

pub(super) struct Refresh {
    pub key: Key,
    id: Option<u64>,
    page: Option<viewer::TextPage>,
    /// The refresh completed while the view was not at rest: its page waits for the rest.
    complete: bool,
}

#[derive(Default)]
pub(super) struct SourceSyntax {
    epoch: u64,
    pub latest: Option<Key>,
    active: Option<Active>,
    pub refresh: Option<Refresh>,
    finished: Option<Key>,
    foreground: Option<(u64, bool)>,
    eligible: bool,
    pub unavailable: bool,
    retries: u8,
    retry_at: Option<Instant>,
    notice: Option<(u64, String)>,
    /// The generation and revision of the shown Source file (`Worker::source_requested`).
    context: Option<(u64, u64)>,
    /// The generation and revision prepared whole: every page read from it comes coloured.
    whole: Option<(u64, u64)>,
    /// Rows found plain (`Worker::plain_rows`): what a refresh leaves of them stays plain
    /// however few rows it is.
    found: Rows,
    /// Rows refreshed after their preparation: rows without a token stay plain there, and
    /// they are not prepared again.
    settled: Rows,
    /// Since when the visible rows have been plain at rest (`NOTICE_DELAY`).
    plain_since: Option<Instant>,
}

/// Source ranges of the shown page's rows, of one revision, merged.
#[derive(Default)]
struct Rows {
    revision: u64,
    ranges: Vec<Range<u64>>,
}

impl Rows {
    fn add(&mut self, revision: u64, range: Range<u64>) {
        if self.revision != revision {
            *self = Self {
                revision,
                ranges: Vec::new(),
            };
        }
        if range.is_empty() {
            return;
        }
        self.ranges.push(range);
        self.ranges.sort_by_key(|range| range.start);
        let mut merged: Vec<Range<u64>> = Vec::with_capacity(self.ranges.len());
        for range in self.ranges.drain(..) {
            match merged.last_mut() {
                Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
                _ => merged.push(range),
            }
        }
        self.ranges = merged;
    }

    /// Keep only the ranges the source range `start..end` of `revision` reaches.
    fn retain(&mut self, revision: u64, start: u64, end: u64) {
        if self.revision != revision {
            self.ranges.clear();
        }
        self.ranges
            .retain(|range| range.start < end && start < range.end);
    }

    /// The ranges, as text ranges of `page`.
    fn in_text(&self, page: &Page) -> Vec<Range<usize>> {
        if self.revision != page.revision {
            return Vec::new();
        }
        self.ranges
            .iter()
            .map(|range| text_of(page, range.start)..text_of(page, range.end))
            .filter(|range| !range.is_empty())
            .collect()
    }
}

impl SourceSyntax {
    /// Prepare the rows of `viewport`, as `Worker::source_target` would: tests start there.
    #[cfg(test)]
    pub fn demand(&mut self, generation: u64, revision: u64, viewport: messages::Viewport) {
        self.demand_scope(generation, revision, Scope::Rows, viewport);
    }

    fn demand_scope(
        &mut self,
        generation: u64,
        revision: u64,
        scope: Scope,
        viewport: messages::Viewport,
    ) {
        if self.epoch == u64::MAX {
            self.latest = None;
            self.eligible = false;
            return;
        }
        self.invalidate();
        self.foreground = None;
        self.context = Some((generation, revision));
        self.latest = Some(Key {
            generation,
            revision,
            mode: viewer::MODE_SOURCE,
            epoch: self.epoch,
            scope,
            viewport,
        });
        self.eligible = true;
    }

    fn invalidate(&mut self) {
        let Some(epoch) = self.epoch.checked_add(1) else {
            self.latest = None;
            self.eligible = false;
            return;
        };
        self.epoch = epoch;
        if let Some(key) = &mut self.latest {
            key.epoch = epoch;
        }
        self.finished = None;
        self.retries = 0;
        self.retry_at = None;
        self.eligible = self.foreground.is_none();
    }

    pub fn started(&mut self, id: u64, key: Key) {
        self.active = Some(Active {
            id,
            key,
            result: None,
        });
    }

    pub fn refresh_admitted(&mut self, id: u64) {
        if let Some(refresh) = &mut self.refresh {
            refresh.id = Some(id);
            refresh.page = None;
            refresh.complete = false;
        }
    }

    pub fn needs_preparation(&self, now: Instant) -> bool {
        !self.unavailable
            && self.eligible
            && self.active.is_none()
            && self.refresh.is_none()
            && self
                .latest
                .as_ref()
                .is_some_and(|key| self.finished.as_ref() != Some(key))
            && self.retry_at.is_none_or(|retry| now >= retry)
    }

    pub fn refresh_due(&self, now: Instant) -> bool {
        self.refresh
            .as_ref()
            .is_some_and(|refresh| refresh.id.is_none())
            && self.retry_at.is_none_or(|retry| now >= retry)
    }

    fn retry(&mut self) {
        self.retries = self.retries.saturating_add(1).min(MAX_BACKOFF_SHIFT);
        let millis = 20u64 << self.retries;
        self.retry_at = Some(clock_now() + Duration::from_millis(millis));
    }
}

impl Worker {
    /// A page is asked for, or the view navigates: the epoch is replaced, the refresh under
    /// way is cancelled and the notice goes. An admitted preparation keeps running; only its
    /// exact current target can later authorize a refresh.
    pub(super) fn invalidate_source_target(&mut self) {
        self.source_moved();
        if let Some(refresh) = self.text.syntax.refresh.take()
            && let Some(id) = refresh.id
            && let Some(session) = &self.session
        {
            session.cancel(id);
        }
        self.text.syntax.invalidate();
    }

    /// The admitted request of the current source refresh.
    pub(super) fn source_refresh_request(&self) -> Option<u64> {
        self.text
            .syntax
            .refresh
            .as_ref()
            .and_then(|refresh| refresh.id)
    }

    pub(super) fn close_source_target(&mut self) {
        self.invalidate_source_target();
        self.text.syntax.latest = None;
        self.text.syntax.eligible = false;
        self.text.syntax.active = None;
    }

    /// A foreground page is requested: the shown file's revision, and no preparation until
    /// the page is shown (`source_target` then chooses what the page needs).
    pub(super) fn source_requested(&mut self, id: u64) {
        if self.mode != viewer::MODE_SOURCE {
            return;
        }
        let Some(revision) = self
            .captured
            .as_ref()
            .filter(|(index, _)| *index == self.current)
            .and_then(|(_, file)| file.as_ref().ok())
            .map(midnight_plugin_host::CapturedFile::revision)
        else {
            return;
        };
        self.text.syntax.context = Some((self.generation, revision));
        self.text.syntax.foreground = Some((id, false));
        self.text.syntax.eligible = false;
    }

    /// The shown file's revision, as `source_requested` takes it from the captured file.
    #[cfg(test)]
    pub(super) fn source_shown(&mut self, revision: u64) {
        self.text.syntax.context = Some((self.generation, revision));
    }

    /// The camera moved, or a page is asked for (`invalidate_source_target`): the notice goes
    /// and its delay counts again from the next rest. A preparation or refresh under way goes
    /// on; a refresh that completes away from rest waits for it.
    pub(super) fn source_moved(&mut self) {
        self.text.syntax.plain_since = None;
        self.clear_source_notice();
    }

    pub(super) fn source_page_loaded(&mut self, id: u64) {
        if let Some((request, loaded)) = &mut self.text.syntax.foreground
            && *request == id
        {
            *loaded = true;
        }
    }

    pub(super) fn source_foreground_finished(&mut self, id: u64, completed: bool) {
        if let Some((request, loaded)) = self.text.syntax.foreground
            && request == id
        {
            self.text.syntax.foreground = None;
            self.text.syntax.eligible = completed && loaded;
        }
    }

    /// New request parameters (geometry, font): the next rest chooses the target again.
    pub(super) fn source_viewport_parameters_changed(&mut self) {
        if self.text.syntax.latest.is_some() {
            self.invalidate_source_target();
            self.text.syntax.latest = None;
        }
    }

    /// Choose, at rest, what the shown page needs prepared (`wanted_source_target`). A target
    /// that did not change keeps its admitted work; nothing is prepared while every row of
    /// the page is coloured or settled.
    pub(super) fn source_target(&mut self) {
        if !self.source_preparation_admissible() || self.text.syntax.unavailable {
            return;
        }
        let Some((generation, revision)) = self.text.syntax.context else {
            return;
        };
        // The shown page has arrived: what it needs may be prepared.
        if self.text.syntax.foreground.is_none() {
            self.text.syntax.eligible = true;
        }
        let Some(page) = &self.page else {
            return;
        };
        // Rows of earlier pages are read again when shown again: found and settled anew.
        let (start, end) = (page.start, page.end);
        let syntax = &mut self.text.syntax;
        syntax.found.retain(page.revision, start, end);
        syntax.settled.retain(page.revision, start, end);
        let plain = self.plain_rows(page);
        for rows in &plain {
            if let Some((start, end)) = source_of(page, rows.start).zip(source_of(page, rows.end)) {
                self.text.syntax.found.add(page.revision, start..end);
            }
        }
        let wanted = self.wanted_source_target(generation, revision, plain);
        let syntax = &self.text.syntax;
        let current = syntax
            .latest
            .as_ref()
            .filter(|key| key.generation == generation && key.revision == revision)
            .map(|key| {
                (
                    key.scope,
                    key.viewport.source_anchor,
                    key.viewport.backwards,
                )
            });
        let Some(target) = wanted else {
            if syntax.active.is_none() && syntax.refresh.is_none() {
                self.text.syntax.latest = None;
            }
            return;
        };
        if current == Some((target.scope, target.anchor, target.backwards)) {
            return;
        }
        let mut viewport = self.page_viewport(target.anchor, target.backwards);
        if target.scope == Scope::Rows {
            // All the plain rows, as far as one refresh reads (`worker_chain::Budget`).
            viewport.max_lines = target.rows.clamp(1, MAX_REFRESH_LINES);
        }
        self.invalidate_source_target();
        self.text
            .syntax
            .demand_scope(generation, revision, target.scope, viewport);
    }

    /// What the shown page needs prepared: the whole document once per revision when it
    /// fits one walk, unless the page holds all of it coloured already; else the `plain` rows
    /// nearest the visible ones (below them on a tie): from their start when one refresh
    /// reads them through the view, else from a screen above the view, or back from the view
    /// when they all lie above it.
    fn wanted_source_target(
        &self,
        generation: u64,
        revision: u64,
        plain: Vec<Range<usize>>,
    ) -> Option<Target> {
        let page = self.page.as_ref()?;
        let holds_all = !page.before && !page.after;
        if self.length > 0
            && self.length <= u64::from(MAX_SCAN_BYTES)
            && self.text.syntax.whole != Some((generation, revision))
            && (!plain.is_empty() || !holds_all)
        {
            return Some(Target {
                scope: Scope::Whole,
                anchor: self.length,
                backwards: true,
                rows: 0,
            });
        }
        let visible = self.visible_text_range(page)?;
        let nearest = plain.into_iter().min_by_key(|rows| {
            if rows.end <= visible.start {
                (visible.start - rows.end, true)
            } else {
                (rows.start.saturating_sub(visible.end), false)
            }
        })?;
        let text = &page.text;
        let above = nearest.end <= visible.start;
        let from = if above || visible.end.saturating_sub(nearest.start) <= REFRESH_REACH {
            // One refresh reads the plain rows from their start through the view.
            nearest.start
        } else {
            // The screen above the view comes with it: the page holds it to scroll back to.
            let screen = self.shaped.as_ref().and_then(|shaped| {
                let top = (self.origin[1] - self.geometry.height as f32).max(0.0);
                shaped.line_at_y(top).ok().map(|(line, _)| line.start)
            });
            nearest
                .start
                .max(screen.unwrap_or(visible.start).min(visible.start))
        };
        // Line breaks are single bytes, which no other character's bytes repeat.
        let bytes = text.as_bytes();
        let newline = |byte: &u8| *byte == b'\n';
        let first = bytes[..from]
            .iter()
            .rposition(newline)
            .map_or(0, |at| at + 1);
        // The end of the row the plain rows end in.
        let last = nearest.end.saturating_sub(1).max(first);
        let end = bytes[last..]
            .iter()
            .position(newline)
            .map_or(text.len(), |at| last + at + 1);
        let rows = bytes[first..end]
            .iter()
            .filter(|byte| newline(byte))
            .count()
            + 1;
        Some(Target {
            scope: Scope::Rows,
            anchor: source_of(page, if above { end } else { first })?,
            backwards: above,
            rows: u32::try_from(rows).unwrap_or(u32::MAX),
        })
    }

    /// Text ranges of `page` that came without highlighting, less the rows refreshed after
    /// their preparation (`SourceSyntax::settled`): stretches without a colour span of
    /// `PLAIN_GAP` bytes, or all of a shorter page, and any such stretch that holds what a
    /// refresh left of rows found plain before (`SourceSyntax::found`). Rows found plain that
    /// were read again with colours hold colour spans: they no longer count.
    pub(super) fn plain_rows(&self, page: &Page) -> Vec<Range<usize>> {
        let length = page.text.len();
        let minimum = PLAIN_GAP.min(length).max(1);
        let settled = self.text.syntax.settled.in_text(page);
        let mut left = Vec::new();
        for found in self.text.syntax.found.in_text(page) {
            let mut start = found.start;
            for range in &settled {
                if range.end <= start || range.start >= found.end {
                    continue;
                }
                if range.start > start {
                    left.push(start..range.start);
                }
                start = start.max(range.end);
            }
            if found.end > start {
                left.push(start..found.end);
            }
        }
        let mut rows = Vec::new();
        let mut keep = |piece: Range<usize>| {
            if piece.len() >= minimum
                || left
                    .iter()
                    .any(|range| piece.start <= range.start && range.end <= piece.end)
            {
                rows.push(piece);
            }
        };
        let found = &left;
        let mut at = 0;
        let roles = page.roles.iter().map(|role| role.range.clone());
        for role in roles.chain(std::iter::once(length..length)) {
            if role.start > at && (role.start - at >= minimum || !found.is_empty()) {
                let mut start = at;
                for range in &settled {
                    if range.end <= start || range.start >= role.start {
                        continue;
                    }
                    if range.start > start {
                        keep(start..range.start);
                    }
                    start = start.max(range.end);
                }
                if role.start > start {
                    keep(start..role.start);
                }
            }
            at = at.max(role.end);
        }
        rows
    }

    /// The text range of `page` that the window shows.
    fn visible_text_range(&self, page: &Page) -> Option<Range<usize>> {
        let shaped = self.shaped.as_ref()?;
        let top = shaped.line_at_y(self.origin[1]).ok()?.0.start;
        let bottom = shaped
            .line_at_y(self.origin[1] + self.geometry.height as f32)
            .ok()?
            .0
            .end;
        let length = page.text.len();
        Some(top.min(length)..bottom.min(length).max(top.min(length)))
    }

    /// Whether the window shows rows that came without highlighting.
    pub(super) fn visible_rows_plain(&self) -> bool {
        let Some(page) = &self.page else {
            return false;
        };
        let Some(visible) = self.visible_text_range(page) else {
            return false;
        };
        let end = visible.end.max(visible.start + 1);
        self.plain_rows(page)
            .iter()
            .any(|rows| rows.start < end && visible.start < rows.end)
    }

    fn current_source_key(&self, key: &Key) -> bool {
        !self.layout.closed()
            && key.generation == self.generation
            && key.mode == self.mode
            && self.mode == viewer::MODE_SOURCE
            && self.text.syntax.latest.as_ref() == Some(key)
    }

    pub(super) fn source_preparation_admissible(&self) -> bool {
        !self.layout.closed()
            && !self.layout.waiting()
            && !self.pending_open
            && self.mode == viewer::MODE_SOURCE
            && self.viewport_request.is_none()
            && self.search_request.is_none()
            && !self.positions.has_pending()
            && self.continuation.is_none()
            && self.raster.continuing.is_none()
            && !self.raster_rebase_waiting()
            && self.motion.is_idle()
            && self.page.is_some()
            && self.shaped.is_some()
    }

    pub(super) fn source_retry_deadline(&self) -> Option<Instant> {
        let deadline = self.text.syntax.retry_at?;
        (self.session.is_some()
            && self.source_preparation_admissible()
            && (self.text.syntax.needs_preparation(deadline)
                || self.text.syntax.refresh_due(deadline)))
        .then_some(deadline)
    }

    pub(super) fn prepare_source(&mut self, output: &Mailbox) {
        let Some((key, admission)) = self.source_admission(output) else {
            return;
        };
        let Some(session) = &self.session else {
            return;
        };
        // The intent already exists before admission. QUEUE_FULL retries reuse it; they do
        // not create a new preparation/refresh cycle.
        let result = match &admission {
            SourceAdmission::Prepare(request) => session.try_prepare_source(request.clone()),
            SourceAdmission::Refresh(viewport) => session.try_viewport(viewport.clone()),
        };
        self.source_admitted(&key, &admission, result, output);
    }

    /// The preparation or refresh that source highlighting asks the session to admit now,
    /// with its key (`source_admitted` takes the session's answer).
    pub(super) fn source_admission(&mut self, output: &Mailbox) -> Option<(Key, SourceAdmission)> {
        self.source_notice_turn(output);
        if !self.source_preparation_admissible() {
            return None;
        }
        if self
            .text
            .syntax
            .refresh
            .as_ref()
            .is_some_and(|refresh| refresh.complete)
        {
            let refresh = self.text.syntax.refresh.take()?;
            self.finish_source_refresh(refresh, output);
            return None;
        }
        self.source_target();
        let now = clock_now();
        if let Some(refresh) = &self.text.syntax.refresh {
            let key = refresh.key.clone();
            if !self.current_source_key(&key) {
                self.invalidate_source_target();
                return None;
            }
            return self.text.syntax.refresh_due(now).then(|| {
                let viewport = key.viewport.clone();
                (key, SourceAdmission::Refresh(viewport))
            });
        }
        let key = self.text.syntax.latest.clone()?;
        if !self.current_source_key(&key) {
            return None;
        }
        let syntax = &mut self.text.syntax;
        // Every page of a document prepared whole comes coloured when read: its plain rows
        // only need reading again.
        if key.scope == Scope::Rows
            && syntax.whole == Some((key.generation, key.revision))
            && syntax.active.is_none()
            && syntax.finished.as_ref() != Some(&key)
        {
            syntax.refresh = Some(Refresh {
                key: key.clone(),
                id: None,
                page: None,
                complete: false,
            });
            return syntax.refresh_due(now).then(|| {
                let viewport = key.viewport.clone();
                (key, SourceAdmission::Refresh(viewport))
            });
        }
        if !syntax.needs_preparation(now) {
            return None;
        }
        let request = messages::PrepareSource {
            revision: key.revision,
            viewport: key.viewport.clone(),
            max_scan_bytes: MAX_SCAN_BYTES,
        };
        Some((key, SourceAdmission::Prepare(request)))
    }

    /// The session's answer to `admission` (`source_admission`).
    pub(super) fn source_admitted(
        &mut self,
        key: &Key,
        admission: &SourceAdmission,
        result: Result<u64, AdmissionError>,
        output: &Mailbox,
    ) {
        match admission {
            SourceAdmission::Prepare(_) => self.source_prepare_admission(key, result, output),
            SourceAdmission::Refresh(_) => self.source_refresh_admission(key, result, output),
        }
    }

    /// The notice of plain visible rows: shown once they have stayed plain at rest for
    /// `NOTICE_DELAY`, cleared once they are coloured, the camera moves or a page is asked
    /// for (`invalidate_source_target`).
    fn source_notice_turn(&mut self, output: &Mailbox) {
        let plain = self.source_preparation_admissible()
            && !self.text.syntax.unavailable
            && self.text.syntax.context.is_some()
            && self.visible_rows_plain();
        if !plain {
            self.text.syntax.plain_since = None;
            self.clear_source_notice();
            return;
        }
        let now = clock_now();
        let since = *self.text.syntax.plain_since.get_or_insert(now);
        if self.text.syntax.notice.is_none()
            && self.status.is_empty()
            && now.saturating_duration_since(since) >= NOTICE_DELAY
        {
            self.text.syntax.notice = Some((self.text.syntax.epoch, PREPARING_HIGHLIGHTING.into()));
            self.status(PREPARING_HIGHLIGHTING.into(), output);
            self.dirty = true;
        }
    }

    /// When the notice of plain visible rows falls due (`Worker::retry_delay`).
    pub(super) fn source_notice_deadline(&self) -> Option<Instant> {
        let syntax = &self.text.syntax;
        if syntax.notice.is_some() || !self.status.is_empty() {
            return None;
        }
        syntax.plain_since.map(|since| since + NOTICE_DELAY)
    }

    pub(super) fn source_prepare_admission(
        &mut self,
        key: &Key,
        admission: Result<u64, AdmissionError>,
        output: &Mailbox,
    ) {
        match admission {
            Ok(id) => self.text.syntax.started(id, key.clone()),
            Err(AdmissionError::Capacity) => self.text.syntax.retry(),
            Err(AdmissionError::Closing) => self.finish_source_key(key, None, output),
            Err(AdmissionError::Invalid(error)) => {
                self.finish_source_key(key, Some(error.to_string()), output);
            }
        }
    }

    pub(super) fn source_refresh_admission(
        &mut self,
        key: &Key,
        admission: Result<u64, AdmissionError>,
        output: &Mailbox,
    ) {
        match admission {
            Ok(id) => {
                self.text.syntax.refresh_admitted(id);
                self.chain_requested(id, &key.viewport);
            }
            Err(AdmissionError::Capacity) => self.text.syntax.retry(),
            Err(AdmissionError::Closing) => self.finish_source_key(key, None, output),
            Err(AdmissionError::Invalid(error)) => {
                self.finish_source_key(key, Some(error.to_string()), output);
            }
        }
    }

    pub(super) fn source_event(&mut self, id: u64, event: &EventKind, output: &Mailbox) -> bool {
        if self
            .text
            .syntax
            .refresh
            .as_ref()
            .is_some_and(|refresh| refresh.id == Some(id))
        {
            self.source_refresh_event(event, output);
            return true;
        }
        if !self
            .text
            .syntax
            .active
            .as_ref()
            .is_some_and(|active| active.id == id)
        {
            return false;
        }
        match event {
            EventKind::SourcePrepared(result) => {
                if let Some(active) = &mut self.text.syntax.active {
                    active.result = Some(result.clone());
                }
            }
            EventKind::Completed | EventKind::Cancelled | EventKind::Failed(_) => {
                let active = self
                    .text
                    .syntax
                    .active
                    .take()
                    .expect("matched active preparation");
                // Capability absence belongs to the session even when navigation
                // superseded the originating epoch while the old plugin replied.
                if matches!(event, EventKind::Failed(failure) if failure.code == abi::UNSUPPORTED) {
                    self.text.syntax.unavailable = true;
                    self.finish_source_key(&active.key, None, output);
                    return true;
                }
                if !self.current_source_key(&active.key) {
                    return true;
                }
                match event {
                    EventKind::Failed(failure) if failure.code == abi::QUEUE_FULL => {
                        self.text.syntax.retry();
                    }
                    EventKind::Failed(failure) => {
                        self.finish_source_key(&active.key, Some(failure.message.clone()), output);
                    }
                    EventKind::Cancelled => self.finish_source_key(&active.key, None, output),
                    EventKind::Completed => match active.result {
                        Some(result) if result.revision == active.key.revision && result.ready => {
                            self.text.syntax.retries = 0;
                            self.text.syntax.retry_at = None;
                            if active.key.scope == Scope::Whole {
                                // Every page of the revision now comes coloured when read.
                                self.text.syntax.whole =
                                    Some((active.key.generation, active.key.revision));
                                self.finish_source_key(&active.key, None, output);
                            } else {
                                self.text.syntax.refresh = Some(Refresh {
                                    key: active.key,
                                    id: None,
                                    page: None,
                                    complete: false,
                                });
                            }
                        }
                        Some(result) if result.revision == active.key.revision => {
                            // Equal frontiers can be decoder-tail progress. The
                            // plugin owns readiness; never guess from offsets.
                            self.text.syntax.retries = 0;
                            self.text.syntax.retry_at = None;
                        }
                        _ => self.finish_source_key(
                            &active.key,
                            Some("Source preparation returned no valid result.".into()),
                            output,
                        ),
                    },
                    _ => {}
                }
            }
            _ => {}
        }
        true
    }

    fn source_refresh_event(&mut self, event: &EventKind, output: &Mailbox) {
        let Some(refresh) = &mut self.text.syntax.refresh else {
            return;
        };
        if let EventKind::TextPage(page) = event {
            refresh.page = Some(page.clone());
            return;
        }
        if !matches!(
            event,
            EventKind::Completed | EventKind::Cancelled | EventKind::Failed(_)
        ) {
            return;
        }
        let mut refresh = self
            .text
            .syntax
            .refresh
            .take()
            .expect("matched source refresh");
        if !self.current_source_key(&refresh.key) {
            return;
        }
        if let EventKind::Failed(failure) = event
            && failure.code == abi::QUEUE_FULL
        {
            self.text.syntax.retry();
            refresh.id = None;
            refresh.page = None;
            self.text.syntax.refresh = Some(refresh);
            return;
        }
        let error = match event {
            EventKind::Completed if self.source_preparation_admissible() => {
                self.finish_source_refresh(refresh, output);
                return;
            }
            EventKind::Completed => {
                // The camera moves: the page waits for it to rest (`source_admission`).
                refresh.complete = true;
                self.text.syntax.refresh = Some(refresh);
                return;
            }
            EventKind::Failed(failure) => Some(failure.message.clone()),
            _ => None,
        };
        self.finish_source_key(&refresh.key, error, output);
    }

    /// Colour the rows of a completed refresh's page in the shown page, and settle them.
    fn finish_source_refresh(&mut self, refresh: Refresh, output: &Mailbox) {
        if !self.current_source_key(&refresh.key) {
            return;
        }
        let error = match refresh.page {
            Some(page) if page.revision == refresh.key.revision => {
                let page = Page::text(page, &self.appearance);
                let rows = page.start..page.end;
                match self.restyle_source_page(page) {
                    Ok(()) => {
                        self.text.syntax.settled.add(refresh.key.revision, rows);
                        None
                    }
                    Err(RestyleError::Invalid(message)) => Some(message),
                }
            }
            Some(_) => Some("Prepared source refresh has the wrong revision.".into()),
            None => Some("Prepared source refresh returned no page.".into()),
        };
        self.finish_source_key(&refresh.key, error, output);
    }

    fn finish_source_key(&mut self, key: &Key, error: Option<String>, output: &Mailbox) {
        if !self.current_source_key(key) {
            return;
        }
        self.text.syntax.finished = Some(key.clone());
        self.text.syntax.refresh = None;
        self.clear_source_notice();
        if let Some(error) = error {
            self.status(error, output);
        }
        self.dirty = true;
    }

    fn clear_source_notice(&mut self) {
        if let Some((_, message)) = self.text.syntax.notice.take()
            && self.status == message
        {
            self.status.clear();
            self.dirty = true;
        }
    }

    fn source_view(&self) -> Option<SourceView> {
        let page = self.page.as_ref()?;
        let shaped = self.shaped.as_ref()?;
        let index = shaped.hit_test(self.origin[0], self.origin[1] + 0.5).ok()?;
        let caret = shaped.caret_geometry(index).ok()?;
        Some(SourceView {
            source: source_at(page, index)?,
            offset: [
                self.origin[0] - caret.x0 as f32,
                self.origin[1] - caret.y0 as f32,
            ],
        })
    }

    fn source_restyle_layout(
        &mut self,
        page: &Page,
        view: &SourceView,
    ) -> Result<(TextPage, [f32; 2]), RestyleError> {
        let index = text_at(page, view.source).ok_or_else(RestyleError::uncorrelated)?;
        let shaped = self
            .restyled_layout(page)
            .map_err(|_| RestyleError::uncorrelated())?;
        let caret = shaped
            .caret_geometry(index)
            .map_err(|_| RestyleError::uncorrelated())?;
        let origin = [
            caret.x0 as f32 + view.offset[0],
            caret.y0 as f32 + view.offset[1],
        ];
        let maximum = [
            (shaped.width() - self.text_width()).max(0.0),
            (shaped.height() - self.geometry.height as f32).max(0.0),
        ];
        if (0..2).any(|axis| origin[axis] < -0.01 || origin[axis] > maximum[axis] + 0.01) {
            return Err(RestyleError::uncorrelated());
        }
        Ok((
            shaped,
            std::array::from_fn(|axis| origin[axis].clamp(0.0, maximum[axis])),
        ))
    }

    /// The layout of the shown page's text restyled as `page`. Row groups whose colours and
    /// styles stay keep their shaping (`Page::layout_rows`): the text is the shown page's.
    fn restyled_layout(
        &mut self,
        page: &Page,
    ) -> Result<TextPage, midnight_viewer_render::LayoutError> {
        if !self.row_pages() || !page.tables.is_empty() || !page.document_blocks.is_empty() {
            return self.layout_page(page);
        }
        let family = self.text_family().to_owned();
        let options = self.text_options(&family);
        let styles = page.scaled_styles(&options);
        let previous = self.shaped.as_ref().map(|shaped| (shaped, 0));
        let (shaped, rows) =
            self.renderer
                .layout_rows(&page.text, &options, &page.spans, &styles, previous)?;
        prepare_counters::layout(rows.shaped_bytes);
        Ok(shaped)
    }

    /// Colour the rows `page` holds in the shown page. Exactly the verified style interval is
    /// replaced: the shown page keeps its text, anchors, line context, selection and the
    /// prepared window it holds, which a refresh of some of its rows never shrinks.
    fn restyle_source_page(&mut self, page: Page) -> Result<(), RestyleError> {
        let view = self.source_view().ok_or_else(RestyleError::uncorrelated)?;
        let previous = self.page.as_ref().ok_or_else(RestyleError::uncorrelated)?;
        let overlap = Overlap::verified(previous, &page)?;
        // A Source packet carries at most MAX_SPANS colours and as many typography records;
        // a page of row groups holds more (`worker_chain::Budget`).
        let cap = if self.row_pages() {
            midnight_viewer_render::MAX_ROWS_SPANS
        } else {
            viewer::MAX_SPANS
        };
        let roles = replace_overlap(&previous.roles, &page.roles, &overlap, cap)?;
        let styles = replace_overlap(&previous.styles, &page.styles, &overlap, cap)?;
        if roles.len() + styles.len() > 2 * cap {
            return Err(RestyleError::budget());
        }
        let mut retained = self.page.take().ok_or_else(RestyleError::uncorrelated)?;
        let old_roles = std::mem::replace(&mut retained.roles, roles);
        let old_colors = std::mem::take(&mut retained.spans);
        retained.recolor(&self.appearance);
        let old_styles = std::mem::replace(&mut retained.styles, styles);
        match self.source_restyle_layout(&retained, &view) {
            Ok((shaped, origin)) => {
                self.text.gutter = None;
                self.commit_source_restyle(retained, shaped, origin, self.selection);
                Ok(())
            }
            Err(error) => {
                retained.spans = old_colors;
                retained.roles = old_roles;
                retained.styles = old_styles;
                self.page = Some(retained);
                Err(error)
            }
        }
    }

    fn commit_source_restyle(
        &mut self,
        page: Page,
        shaped: TextPage,
        origin: [f32; 2],
        selection: Option<(usize, usize)>,
    ) {
        let previous_origin = self.origin;
        self.scroll_map =
            scroll_map::build(&page, &shaped, self.length, true, self.geometry.height);
        self.anchor = page.start;
        self.page = Some(page);
        self.shaped = Some(shaped);
        self.origin = origin;
        // The callers keep the same source range selected, so this is no selection
        // change: Copy may still name frames that showed it.
        self.selection = selection;
        self.raster.source_refreshed(previous_origin, origin);
        if self.selection.is_some() {
            self.capture_raster_selection();
        }
        self.frames.invalidate();
        self.dirty = true;
    }
}

struct SourceView {
    source: u64,
    offset: [f32; 2],
}

enum RestyleError {
    Invalid(String),
}
impl RestyleError {
    fn uncorrelated() -> Self {
        Self::Invalid("Prepared source refresh has uncorrelated text or source anchors.".into())
    }

    fn budget() -> Self {
        Self::Invalid("Source highlighting style budget exceeded.".into())
    }
}

struct Overlap {
    previous: Range<usize>,
    incoming: Range<usize>,
}
impl Overlap {
    fn verified(previous: &Page, incoming: &Page) -> Result<Self, RestyleError> {
        let start = previous.start.max(incoming.start);
        let end = previous.end.min(incoming.end);
        if start >= end {
            return Err(RestyleError::uncorrelated());
        }
        let old = overlap_anchors(previous, start, end).ok_or_else(RestyleError::uncorrelated)?;
        let new = overlap_anchors(incoming, start, end).ok_or_else(RestyleError::uncorrelated)?;
        let (old_first, old_last) = old
            .first()
            .zip(old.last())
            .ok_or_else(RestyleError::uncorrelated)?;
        let (new_first, new_last) = new
            .first()
            .zip(new.last())
            .ok_or_else(RestyleError::uncorrelated)?;
        let previous_range = old_first.text_offset as usize..old_last.text_offset as usize;
        let incoming_range = new_first.text_offset as usize..new_last.text_offset as usize;
        let old_text = previous
            .text
            .get(previous_range.clone())
            .ok_or_else(RestyleError::uncorrelated)?;
        let new_text = incoming
            .text
            .get(incoming_range.clone())
            .ok_or_else(RestyleError::uncorrelated)?;
        if old_first.source_offset != start
            || new_first.source_offset != start
            || old_last.source_offset != end
            || new_last.source_offset != end
            || old.len() != new.len()
            || old_text != new_text
            || old.iter().zip(new).any(|(old, new)| {
                old.source_offset != new.source_offset
                    || old.text_offset - old_first.text_offset
                        != new.text_offset - new_first.text_offset
            })
        {
            return Err(RestyleError::uncorrelated());
        }
        Ok(Self {
            previous: previous_range,
            incoming: incoming_range,
        })
    }
}

fn overlap_anchors(page: &Page, start: u64, end: u64) -> Option<&[viewer::Anchor]> {
    let first = page
        .anchors
        .partition_point(|anchor| anchor.source_offset < start);
    let last = page
        .anchors
        .partition_point(|anchor| anchor.source_offset <= end);
    page.anchors.get(first..last)
}

trait SourceStyle: Clone {
    fn range(&self) -> &Range<usize>;
    fn range_mut(&mut self) -> &mut Range<usize>;
    fn same_style(&self, other: &Self) -> bool;
}
impl SourceStyle for ColorSpan {
    fn range(&self) -> &Range<usize> {
        &self.range
    }
    fn range_mut(&mut self) -> &mut Range<usize> {
        &mut self.range
    }
    fn same_style(&self, other: &Self) -> bool {
        self.color == other.color
    }
}
impl SourceStyle for crate::page::RoleSpan {
    fn range(&self) -> &Range<usize> {
        &self.range
    }
    fn range_mut(&mut self) -> &mut Range<usize> {
        &mut self.range
    }
    fn same_style(&self, other: &Self) -> bool {
        self.role == other.role
    }
}
impl SourceStyle for StyleSpan {
    fn range(&self) -> &Range<usize> {
        &self.range
    }
    fn range_mut(&mut self) -> &mut Range<usize> {
        &mut self.range
    }
    fn same_style(&self, other: &Self) -> bool {
        self.weight == other.weight
            && self.italic == other.italic
            && self.underline == other.underline
            && self.strikethrough == other.strikethrough
            && self.monospace == other.monospace
            && self.size == other.size
    }
}

fn replace_overlap<T: SourceStyle>(
    previous: &[T],
    incoming: &[T],
    overlap: &Overlap,
    cap: usize,
) -> Result<Vec<T>, RestyleError> {
    let capacity = previous
        .len()
        .saturating_mul(2)
        .saturating_add(incoming.len())
        .min(cap);
    let mut result: Vec<T> = Vec::with_capacity(capacity);
    let mut push = |style: &T, range: Range<usize>| -> Result<(), RestyleError> {
        if range.is_empty() {
            return Ok(());
        }
        if let Some(last) = result.last_mut()
            && last.range().end == range.start
            && last.same_style(style)
        {
            last.range_mut().end = range.end;
            return Ok(());
        }
        if result.len() == cap {
            return Err(RestyleError::budget());
        }
        let mut style = style.clone();
        *style.range_mut() = range;
        result.push(style);
        Ok(())
    };
    for style in previous {
        push(
            style,
            style.range().start..style.range().end.min(overlap.previous.start),
        )?;
    }
    for style in incoming {
        let start = style.range().start.max(overlap.incoming.start);
        let end = style.range().end.min(overlap.incoming.end);
        if start < end {
            push(
                style,
                (start - overlap.incoming.start + overlap.previous.start)
                    ..(end - overlap.incoming.start + overlap.previous.start),
            )?;
        }
    }
    for style in previous {
        push(
            style,
            style.range().start.max(overlap.previous.end)..style.range().end,
        )?;
    }
    Ok(result)
}

fn source_at(page: &Page, index: usize) -> Option<u64> {
    let at = page
        .anchors
        .partition_point(|anchor| (anchor.text_offset as usize) < index);
    page.anchors
        .get(at)
        .filter(|anchor| anchor.text_offset as usize == index)
        .map(|anchor| anchor.source_offset)
}

/// The source offset of `page`'s text offset `index`: of the last anchor at or before it.
fn source_of(page: &Page, index: usize) -> Option<u64> {
    let after = page
        .anchors
        .partition_point(|anchor| anchor.text_offset as usize <= index);
    page.anchors
        .get(after.checked_sub(1)?)
        .map(|anchor| anchor.source_offset)
}

/// The text offset of `page`'s first anchor at or after `source`, or the page's end.
fn text_of(page: &Page, source: u64) -> usize {
    let at = page
        .anchors
        .partition_point(|anchor| anchor.source_offset < source);
    page.anchors
        .get(at)
        .map_or(page.text.len(), |anchor| anchor.text_offset as usize)
}

fn text_at(page: &Page, source: u64) -> Option<usize> {
    page.anchors
        .binary_search_by_key(&source, |anchor| anchor.source_offset)
        .ok()
        .map(|index| page.anchors[index].text_offset as usize)
}
