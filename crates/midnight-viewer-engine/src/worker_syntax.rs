//! Optional syntax preparation, keyed to one exact foreground view epoch.
use super::*;
use midnight_plugin_host::AdmissionError;
use midnight_plugin_host::api::abi;
use midnight_viewer_render::{ColorSpan, StyleSpan};
use std::ops::Range;
use std::time::Instant;

const MAX_BACKOFF_SHIFT: u8 = 5;
// A Source packet can carry MAX_SPANS colors and the same number of optional
// typography records. Retained overlap replacement obeys those same fixed caps.
const MAX_SOURCE_STYLE_RECORDS: usize = 2 * viewer::MAX_SPANS;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Key {
    generation: u64,
    revision: u64,
    mode: u16,
    epoch: u64,
    pub viewport: messages::Viewport,
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
    corrective_target: bool,
}

impl SourceSyntax {
    pub fn demand(&mut self, generation: u64, revision: u64, viewport: messages::Viewport) {
        if self.epoch == u64::MAX {
            self.latest = None;
            self.eligible = false;
            return;
        }
        self.invalidate();
        self.foreground = None;
        self.latest = Some(Key {
            generation,
            revision,
            mode: viewer::MODE_SOURCE,
            epoch: self.epoch,
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
        self.corrective_target = false;
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
        self.retry_at = Some(Instant::now() + Duration::from_millis(millis));
    }
}

impl Worker {
    /// In-page movement replaces the epoch but leaves useful admitted prefix
    /// work running. Only its exact current target can later authorize refresh.
    pub(super) fn invalidate_source_target(&mut self) {
        self.clear_source_notice();
        if let Some(refresh) = self.text.syntax.refresh.take()
            && let Some(id) = refresh.id
            && let Some(session) = &self.session
        {
            session.cancel(id);
        }
        self.text.syntax.invalidate();
    }

    pub(super) fn close_source_target(&mut self) {
        self.invalidate_source_target();
        self.text.syntax.latest = None;
        self.text.syntax.eligible = false;
        self.text.syntax.active = None;
    }

    pub(super) fn source_requested(&mut self, id: u64, viewport: messages::Viewport) {
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
        self.text.syntax.demand(self.generation, revision, viewport);
        self.text.syntax.foreground = Some((id, false));
        self.text.syntax.eligible = false;
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

    pub(super) fn source_viewport_parameters_changed(&mut self) {
        let Some(previous) = self
            .text
            .syntax
            .latest
            .as_ref()
            .map(|key| key.viewport.clone())
        else {
            return;
        };
        let viewport = self.page_viewport(previous.source_anchor, previous.backwards);
        if viewport != previous {
            self.invalidate_source_target();
            if let Some(key) = &mut self.text.syntax.latest {
                key.viewport = viewport;
            }
        }
    }

    /// A page can stay loaded while its camera moves far beyond its original
    /// request anchor. Prepare the actual displayed source point, not that old
    /// fragment's beginning. Admitted prefix work remains useful and keeps running.
    pub(super) fn source_stationary_target(&mut self) {
        if !self.source_preparation_admissible() {
            return;
        }
        let Some(view) = self.source_view() else {
            return;
        };
        let Some(previous) = self.text.syntax.latest.clone() else {
            return;
        };
        if previous.viewport.source_anchor == view.source && !previous.viewport.backwards {
            return;
        }
        let viewport = self.page_viewport(view.source, false);
        self.invalidate_source_target();
        self.text
            .syntax
            .demand(previous.generation, previous.revision, viewport);
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
        if !self.source_preparation_admissible() {
            return;
        }
        self.source_stationary_target();
        if let Some(refresh) = &self.text.syntax.refresh {
            let key = refresh.key.clone();
            if !self.current_source_key(&key) {
                self.invalidate_source_target();
                return;
            }
            if !self.text.syntax.refresh_due(Instant::now()) {
                return;
            }
            let Some(session) = &self.session else {
                return;
            };
            // The intent already exists before admission. QUEUE_FULL retries
            // reuse it; they do not create a new preparation/refresh cycle.
            let admission = session.try_viewport(key.viewport.clone());
            self.source_refresh_admission(&key, admission, output);
            return;
        }
        if !self.text.syntax.needs_preparation(Instant::now()) {
            return;
        }
        let Some(key) = self.text.syntax.latest.clone() else {
            return;
        };
        if !self.current_source_key(&key) {
            return;
        }
        let Some(session) = &self.session else {
            return;
        };
        let admission = session.try_prepare_source(messages::PrepareSource {
            revision: key.revision,
            viewport: key.viewport.clone(),
            max_scan_bytes: 256 * 1024,
        });
        self.source_prepare_admission(&key, admission, output);
    }

    pub(super) fn source_prepare_admission(
        &mut self,
        key: &Key,
        admission: Result<u64, AdmissionError>,
        output: &Mailbox,
    ) {
        match admission {
            Ok(id) => {
                self.text.syntax.started(id, key.clone());
                self.source_notice(key, "Preparing source highlighting…".into(), output);
            }
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
            Ok(id) => self.text.syntax.refresh_admitted(id),
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
                            self.text.syntax.refresh = Some(Refresh {
                                key: active.key,
                                id: None,
                                page: None,
                            });
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
            EventKind::Completed if self.source_preparation_admissible() => match refresh.page {
                Some(page) if page.revision == refresh.key.revision => {
                    match self.restyle_source_page(Page::text(page, &self.appearance)) {
                        Ok(()) => None,
                        Err(RestyleError::OutsideView) => {
                            self.correct_source_target(&refresh.key, output);
                            return;
                        }
                        Err(RestyleError::Invalid(message)) => Some(message),
                    }
                }
                Some(_) => Some("Prepared source refresh has the wrong revision.".into()),
                None => Some("Prepared source refresh returned no page.".into()),
            },
            EventKind::Failed(failure) => Some(failure.message.clone()),
            _ => None,
        };
        self.finish_source_key(&refresh.key, error, output);
    }

    fn correct_source_target(&mut self, key: &Key, output: &Mailbox) {
        let Some(view) = self.source_view() else {
            self.finish_source_key(
                key,
                Some("Cannot correlate the visible source anchor.".into()),
                output,
            );
            return;
        };
        if self.text.syntax.corrective_target {
            self.finish_source_key(
                key,
                Some("Prepared source fragment still excludes the visible source anchor.".into()),
                output,
            );
            return;
        }
        let viewport = self.page_viewport(view.source, false);
        self.invalidate_source_target();
        self.text
            .syntax
            .demand(key.generation, key.revision, viewport);
        self.text.syntax.corrective_target = true;
        // One corrective target, with bounded retry frequency. Temporary
        // capacity denials never consume another corrective allowance.
        self.text.syntax.retry();
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

    fn source_notice(&mut self, key: &Key, message: String, output: &Mailbox) {
        if self.current_source_key(key)
            && (self.status.is_empty()
                || self
                    .text
                    .syntax
                    .notice
                    .as_ref()
                    .is_some_and(|(_, old)| *old == self.status))
        {
            self.text.syntax.notice = Some((key.epoch, message.clone()));
            self.status(message, output);
            self.dirty = true;
        }
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
        let index = text_at(page, view.source).ok_or(RestyleError::OutsideView)?;
        let shaped = self
            .layout_page(page)
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

    fn restyle_source_page(&mut self, page: Page) -> Result<(), RestyleError> {
        let view = self.source_view().ok_or_else(RestyleError::uncorrelated)?;
        if text_at(&page, view.source).is_none() {
            return Err(RestyleError::OutsideView);
        }
        let previous = self.page.as_ref().ok_or_else(RestyleError::uncorrelated)?;
        let overlap = Overlap::verified(previous, &page)?;
        let selected = self
            .selection
            .and_then(|(start, end)| source_at(previous, start).zip(source_at(previous, end)));
        let selection =
            selected.and_then(|(start, end)| text_at(&page, start).zip(text_at(&page, end)));
        if (self.selection.is_none() || selection.is_some())
            && let Ok((shaped, origin)) = self.source_restyle_layout(&page, &view)
            && crate::viewport::source_minimum_origin(&page, &shaped)
                .is_some_and(|minimum| origin[1] >= minimum)
        {
            self.cancel_locate();
            self.text.page(self.page.as_ref(), &page);
            self.commit_source_restyle(page, shaped, origin, selection);
            return Ok(());
        }

        // The selection can span most of the old bounded page while the style
        // packet covers only its view or lacks its leading logical context.
        // Replace exactly the verified style interval, retaining the previous
        // text/anchors, line context and original selection indices.
        let previous = self.page.as_ref().ok_or_else(RestyleError::uncorrelated)?;
        let roles = replace_overlap(&previous.roles, &page.roles, &overlap)?;
        let styles = replace_overlap(&previous.styles, &page.styles, &overlap)?;
        if roles.len() + styles.len() > MAX_SOURCE_STYLE_RECORDS {
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
    OutsideView,
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
) -> Result<Vec<T>, RestyleError> {
    let capacity = previous
        .len()
        .saturating_mul(2)
        .saturating_add(incoming.len())
        .min(viewer::MAX_SPANS);
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
        if result.len() == viewer::MAX_SPANS {
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
    page.anchors
        .iter()
        .find(|anchor| anchor.text_offset as usize == index)
        .map(|anchor| anchor.source_offset)
}

fn text_at(page: &Page, source: u64) -> Option<usize> {
    page.anchors
        .binary_search_by_key(&source, |anchor| anchor.source_offset)
        .ok()
        .map(|index| page.anchors[index].text_offset as usize)
}
