//! Prepared text: retained pixels as world-aligned strips, kept several screens around the
//! camera, prepared ahead of it between inputs and reused across page refills while their
//! rows stay the same.
//!
//! A strip is `BAND` rows of the surface's world, starting at a multiple of `BAND`, as wide
//! as the window's body plus horizontal guards when lines are wider. It is painted once
//! from the page that holds its rows and keeps the hash of everything that decided its
//! pixels: the rows reaching it (their source text, line starts and world positions), their
//! colours, typography, roles, document blocks, the selection over them and the gutter's
//! numbers. A new page, a restyle or a new selection keeps every strip whose hash it
//! reproduces. A strip at a page's edge lacks the ink of the rows beyond it: only a page
//! with the same edge reproduces it. A new font, size or preset paints every strip again.
//! The window moves over the strips it has; the worker publishes the contiguous run around
//! the camera.
use super::*;
use std::collections::BTreeMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Rows of one strip, effective raster pixels.
pub(crate) const BAND: u32 = 256;
/// Screens prepared against the direction of travel and along it; faster travel prepares
/// further. The memory budget can allow fewer.
pub(super) const BEHIND: f32 = 1.0;
pub(super) const AHEAD: f32 = 2.0;
const AHEAD_FAST: f32 = 3.0;
/// A camera request that moves more than this share of a screen at once is fast travel
/// (about four screens per second at 60 Hz).
const FAST_STEP: f64 = 1.0 / 16.0;
/// A refill starts while the page holds fewer screens than the strips want along the
/// direction of travel, and a quarter of a screen against it.
const REFILL_AGAINST: f32 = 0.25;
/// Laid-out screens a page holds beyond the prepared strips, so that a refill comes only
/// every few screens of travel: layout is far cheaper than pixels, text costs no budget.
const PAGE_EXTRA: f32 = 2.0;
/// The newest strip ID in the process: an ID names one strip's pixels in every viewer, so
/// a window keeps its conversions and images by ID across surfaces and files.
static LAST_STRIP_ID: AtomicU64 = AtomicU64::new(0);
/// Preparation beyond the window's own strips takes at most this long per worker turn;
/// inputs are taken between turns.
const TURN: std::time::Duration = std::time::Duration::from_millis(4);

/// The strips' horizontal extent: world x of their first column, body width and gutter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Column {
    pub left: f64,
    pub width: u32,
    pub gutter: u32,
}

/// What all strips share. A new raster context (a seek, a new page) keeps them: they are
/// revalidated by their content hash before they are published again.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Key {
    column: Column,
    palette: u64,
    /// The font, size, mode and preset that shape every row (`Worker::strip_typography`).
    typography: u64,
}

struct Strip {
    id: u64,
    pixels: Arc<[u8]>,
    gutter: Option<Arc<[u8]>>,
    /// What decided these pixels; `None` when the page lacked some of their rows.
    hash: Option<u64>,
}

/// The prepared strips of the shown surface and the travel that sizes their window.
#[derive(Default)]
pub(super) struct Bands {
    key: Option<Key>,
    strips: BTreeMap<i64, Strip>,
    /// The newest strip this worker painted.
    last_id: u64,
    palette: u64,
    /// The direction of travel (1 down, -1 up, 0 not known yet) and whether it is fast.
    direction: i8,
    fast: bool,
    last_target: Option<f64>,
    /// Strips beyond the window remain to be prepared: the worker loop does not wait.
    pub pending: bool,
}

impl Bands {
    pub fn clear(&mut self) {
        self.strips.clear();
        self.key = None;
        self.reset_travel();
    }

    /// A new raster context: the travel starts again; the strips stay for revalidation.
    pub fn reset_travel(&mut self) {
        self.pending = false;
        self.last_target = None;
        self.direction = 0;
        self.fast = false;
    }

    /// Colours changed: every strip is painted again.
    pub fn palette_changed(&mut self) {
        self.palette = self.palette.wrapping_add(1);
        self.strips.clear();
    }

    /// The camera was asked to `target` (world y): the direction and speed of travel.
    pub fn travel(&mut self, target: f64, screen: f64) {
        if let Some(last) = self.last_target {
            let step = target - last;
            if step.abs() > 0.5 {
                self.direction = if step > 0.0 { 1 } else { -1 };
                self.fast = step.abs() > screen * FAST_STEP;
            }
        }
        self.last_target = Some(target);
    }

    /// Screens kept above and below the window: along the travel, against it.
    pub fn screens(&self) -> (f32, f32) {
        let ahead = if self.fast { AHEAD_FAST } else { AHEAD };
        match self.direction {
            -1 => (ahead, BEHIND),
            _ => (BEHIND, ahead),
        }
    }

    /// Screens the page should hold around the window: the prepared strips' and a few more
    /// ahead, so that strips can be prepared and refills are rare.
    pub fn page_screens(&self) -> f32 {
        let (above, below) = self.screens();
        above + below + PAGE_EXTRA
    }

    /// Thresholds below which a refill starts above and below the window, in screens: the
    /// strips wanted along the travel, a quarter screen against it.
    pub fn refill_thresholds(&self) -> (f32, f32) {
        let (above, below) = self.screens();
        match self.direction {
            -1 => (above, REFILL_AGAINST),
            1 => (REFILL_AGAINST, below),
            _ => (above, below),
        }
    }

    /// Thresholds below which a refill of a page that grows with each refill starts (row
    /// groups, `worker_chain::Budget`): the strips' (`refill_thresholds`) and the screens a
    /// page holds beyond them along the travel, so that such a page comes to hold every
    /// screen its request asks for.
    pub fn growth_thresholds(&self) -> (f32, f32) {
        let (above, below) = self.refill_thresholds();
        match self.direction {
            -1 => (above + PAGE_EXTRA, below),
            _ => (above, below + PAGE_EXTRA),
        }
    }

    /// Screens a refill keeps against the direction of travel.
    pub fn refill_keeps(&self) -> f32 {
        BEHIND
    }

    /// Whether a refill towards the start (`backwards`) or the end goes along the travel.
    pub fn along(&self, backwards: bool) -> bool {
        if backwards {
            self.direction < 0
        } else {
            self.direction > 0
        }
    }

    /// The newest strip's ID: IDs grow, so strips with larger IDs were painted later.
    #[cfg(test)]
    pub fn last_id(&self) -> u64 {
        self.last_id
    }
}

impl Worker {
    /// The strips' column for a camera at world `origin`: the body's width, plus the
    /// horizontal guards while lines are wider than the window. The column stays while the
    /// camera does not leave it.
    pub(super) fn strip_column(&self, origin: [f64; 2], maximum: [f32; 2]) -> Column {
        let margin = self.text_margin();
        let gutter = if self.mode == viewer::MODE_SOURCE && self.preferences.line_numbers {
            margin as u32
        } else {
            0
        };
        let view = self.geometry.width.saturating_sub(gutter).max(1);
        let base = self.raster.base[0];
        if maximum[0] <= 0.0 {
            return Column {
                left: base,
                width: view,
                gutter,
            };
        }
        if let Some(key) = &self.raster.bands.key {
            let column = key.column;
            if column.gutter == gutter
                && origin[0] >= column.left
                && origin[0] + f64::from(view) <= column.left + f64::from(column.width)
            {
                return column;
            }
        }
        let guard = f64::from(GUARD) * f64::from(self.geometry.scale);
        let left = (origin[0] - guard).max(base).floor();
        let right = (origin[0] + f64::from(view) + guard)
            .min(base + f64::from(maximum[0]) + f64::from(view))
            .ceil();
        Column {
            left,
            width: ((right - left) as u32).clamp(view, 8192),
            gutter,
        }
    }

    /// Strips may hold rows of the page from world y `top` to `bottom`: where the camera can
    /// show them.
    fn strip_bounds(&self, page: &Page, shaped: &TextPage) -> (f64, f64) {
        let base = self.raster.base[1];
        let view = self.geometry.height as f32;
        let available = crate::viewport::available_height(page, shaped, view);
        let span = self.camera_span(page, shaped);
        (base, base + f64::from(available.max(span.span + view)))
    }

    /// Paint strip `index` of `column` from the shown page; `None` when the strip budget
    /// cannot take it now.
    fn paint_strip(&mut self, index: i64, column: Column) -> Result<Option<Strip>, String> {
        let bytes = (column.width as usize + column.gutter as usize) * BAND as usize * 4;
        let Some(reservation) = self.raster_budget.reserve_strip(self.raster_owner, bytes) else {
            return Ok(None);
        };
        let hash = self.strip_hash(index, column);
        let margin = self.text_margin();
        let local = [
            (column.left - self.raster.base[0]) as f32,
            (index as f64 * f64::from(BAND) - self.raster.base[1]) as f32,
        ];
        let viewport = self.retained_viewport(
            column.width,
            BAND,
            [
                local[0] - if column.gutter == 0 { margin } else { 0.0 },
                local[1],
            ],
        );
        let Some(shaped) = &self.shaped else {
            return Ok(None);
        };
        let selection = crate::worker::whole::painted(
            self.whole.selected(),
            self.selection,
            shaped,
            viewport.origin[1],
            BAND as f32,
        );
        let mut pixels = shared_pixels(column.width, BAND)?;
        self.renderer
            .rasterize_into(shaped, viewport, selection, unique(&mut pixels)?)
            .map_err(|e| format!("Cannot render retained page: {e:?}"))?;
        self.paint_link_focus(unique(&mut pixels)?, column.width, BAND, viewport.origin);
        let gutter = if column.gutter != 0 {
            let mut gutter_pixels = shared_pixels(column.gutter, BAND)?;
            let mut raster = midnight_viewer_render::RasterMut::new(
                column.gutter,
                BAND,
                unique(&mut gutter_pixels)?,
            )
            .map_err(|e| format!("Cannot render retained gutter: {e:?}"))?;
            text::paint_gutter(
                &mut self.renderer,
                &mut self.text,
                &self.appearance,
                self.geometry,
                local,
                margin,
                &mut raster,
            )?;
            Some(gutter_pixels)
        } else {
            None
        };
        super::prepare_counters::raster(
            (u64::from(column.width) + u64::from(column.gutter)) * u64::from(BAND),
        );
        reservation
            .commit(&pixels, gutter.as_ref())
            .map_err(str::to_owned)?;
        let id = LAST_STRIP_ID
            .fetch_add(1, Ordering::Relaxed)
            .checked_add(1)
            .ok_or("Raster strip identity exhausted.")?;
        self.raster.bands.last_id = id;
        Ok(Some(Strip {
            id,
            pixels,
            gutter,
            hash,
        }))
    }

    /// Everything that decides strip `index`'s pixels on the shown page, or `None` when the
    /// page does not hold all rows whose ink can reach it.
    fn strip_hash(&self, index: i64, column: Column) -> Option<u64> {
        let (page, shaped) = (self.page.as_ref()?, self.shaped.as_ref()?);
        if !page.tables.is_empty() {
            return None;
        }
        let base = self.raster.base[1];
        let top = index as f64 * f64::from(BAND);
        // Glyph ink reaches 4 em around its row (`midnight_viewer_render`); headings are
        // larger than the text, so the reach is twice that.
        let reach = 8.0 * f64::from(self.appearance.font_size) * f64::from(self.geometry.scale);
        let (low, high) = (top - reach - base, top + f64::from(BAND) + reach - base);
        let view = self.geometry.height as f32;
        let available = f64::from(crate::viewport::available_height(page, shaped, view));
        // Rows beyond a page's edge would ink a strip that reaches it, so such a strip is
        // painted the same way only by a page with the same edge and the rows up to it.
        let top_edge = page.before && low < 0.0;
        let bottom_edge = page.after && high > available;
        let rows = shaped.scroll_anchors();
        if rows.is_empty() {
            return None;
        }
        let first = rows
            .partition_point(|(_, y)| f64::from(*y) <= low)
            .saturating_sub(1);
        let last = rows
            .partition_point(|(_, y)| f64::from(*y) < high)
            .min(rows.len());
        let (start, _) = *rows.get(first)?;
        let (end, _) = *rows.get(last.min(rows.len() - 1))?;
        let start = if top_edge { 0 } else { start };
        let end = if bottom_edge {
            page.text.len()
        } else {
            end.max(start)
        };
        let mut hasher = DefaultHasher::new();
        (
            top_edge.then_some(page.start),
            bottom_edge.then_some(page.end),
            page.revision,
        )
            .hash(&mut hasher);
        let source = |offset: usize| {
            let at = page
                .anchors
                .partition_point(|anchor| (anchor.text_offset as usize) < offset);
            page.anchors
                .get(at)
                .map(|anchor| (anchor.source_offset, anchor.text_offset as usize - offset))
        };
        source(start).hash(&mut hasher);
        page.text.get(start..end)?.hash(&mut hasher);
        // The logical line that holds the first row decides wrapping and tab stops.
        let line_start = page.text[..start]
            .rfind(['\n', '\r', '\u{2028}', '\u{2029}'])
            .map_or(0, |at| at + 1);
        (start - line_start, page.before && line_start == 0).hash(&mut hasher);
        for (offset, y) in &rows[first..=last.min(rows.len() - 1)] {
            (*offset as i64 - start as i64).hash(&mut hasher);
            (((base + f64::from(*y) - top) * 64.0).round() as i64).hash(&mut hasher);
        }
        let clip = |range: &std::ops::Range<usize>| {
            (range.end > start && range.start < end)
                .then(|| (range.start.max(start) - start, range.end.min(end) - start))
        };
        for span in &page.spans {
            if let Some(range) = clip(&span.range) {
                (range, span.color).hash(&mut hasher);
            }
        }
        for style in &page.styles {
            if let Some(range) = clip(&style.range) {
                (
                    range,
                    style.weight,
                    style.italic,
                    style.underline,
                    style.strikethrough,
                    style.monospace,
                    style.size.to_bits(),
                )
                    .hash(&mut hasher);
            }
        }
        for role in &page.roles {
            if let Some(range) = clip(&role.range) {
                (range, role.role).hash(&mut hasher);
            }
        }
        for block in &page.document_blocks {
            if let Some(range) = clip(&block.range) {
                use midnight_viewer_render::DocumentKind;
                let kind = match block.kind {
                    DocumentKind::Paragraph => 0,
                    DocumentKind::Heading(level) => 1 + u16::from(level),
                    DocumentKind::Code => 20,
                    DocumentKind::Quote => 21,
                    DocumentKind::ListItem => 22,
                    DocumentKind::Rule => 23,
                };
                (
                    range,
                    kind,
                    block.depth,
                    block.range.start < start,
                    block.range.end > end,
                )
                    .hash(&mut hasher);
            }
        }
        if let Some((a, b)) = self.shown_selection()
            && let Some(range) = clip(&(a.min(b)..a.max(b)))
        {
            range.hash(&mut hasher);
        }
        // The keyboard focus ring of a link (`worker_links.rs`).
        if let Some(range) = self.focused_link_range()
            && let Some(range) = clip(&range)
        {
            ("link focus", range).hash(&mut hasher);
        }
        if column.gutter != 0 {
            for line in self.text.gutter.iter().flatten() {
                let y = base + f64::from(line.y);
                if y + reach > top && y < top + f64::from(BAND) + reach {
                    (((y - top) * 64.0).round() as i64, line.line).hash(&mut hasher);
                }
            }
        }
        Some(hasher.finish())
    }

    /// What shapes every row besides the page: the mode, font, size, scale and preset. The
    /// strips' hash holds rows and their positions, which a new font of the same line
    /// height keeps.
    fn strip_typography(&self) -> u64 {
        let mut hasher = DefaultHasher::new();
        (
            self.mode,
            self.binary,
            &self.appearance.family,
            self.appearance.font_size.to_bits(),
            self.geometry.scale.to_bits(),
            self.line_height().map(f32::to_bits),
            self.preferences.markdown_preset.storage_name(),
            self.text_margin().to_bits(),
        )
            .hash(&mut hasher);
        hasher.finish()
    }

    /// Strips are kept only where the shown page reproduces their hash; a changed key
    /// (column, palette or typography) drops them all.
    fn adopt_strips(&mut self, column: Column) {
        let key = Key {
            column,
            palette: self.raster.bands.palette,
            typography: self.strip_typography(),
        };
        if self.raster.bands.key != Some(key) {
            self.raster.bands.strips.clear();
            self.raster.bands.key = Some(key);
        }
    }

    /// Keep each strip the shown page still paints the same way.
    fn revalidate_strips(&mut self, column: Column) {
        let indices: Vec<i64> = self.raster.bands.strips.keys().copied().collect();
        for index in indices {
            let keep = self
                .raster
                .bands
                .strips
                .get(&index)
                .and_then(|strip| strip.hash)
                .is_some_and(|hash| self.strip_hash(index, column) == Some(hash));
            if !keep {
                self.raster.bands.strips.remove(&index);
            }
        }
    }

    /// Strip indices from the window's top to its bottom, for a camera at world `y`.
    fn window_strips(&self, y: f64, screens: (f32, f32), bounds: (f64, f64)) -> (i64, i64) {
        let view = f64::from(self.geometry.height);
        let top = (y - f64::from(screens.0) * view).max(bounds.0);
        let bottom = (y + view * (1.0 + f64::from(screens.1))).min(bounds.1);
        let band = f64::from(BAND);
        (
            (top / band).floor() as i64,
            ((bottom - 1e-6) / band).floor() as i64,
        )
    }

    /// Paint the strips under the window at world `origin` that are missing, and one more
    /// beyond each of its edges that the page holds, so that a frame always lets the camera
    /// move; `false` when the strip budget cannot take them now.
    pub(super) fn strips_under_window(
        &mut self,
        origin: [f64; 2],
        column: Column,
    ) -> Result<bool, String> {
        let band = f64::from(BAND);
        let view = f64::from(self.geometry.height);
        let mut first = (origin[1] / band).floor() as i64;
        let mut last = ((origin[1] + view - 1e-6) / band).floor() as i64;
        if let (Some(page), Some(shaped)) = (&self.page, &self.shaped) {
            let (top, bottom) = self.strip_bounds(page, shaped);
            if (first as f64) * band > top {
                first -= 1;
            }
            if ((last + 1) as f64) * band < bottom {
                last += 1;
            }
        }
        for index in first..=last {
            if self.raster.bands.strips.contains_key(&index) {
                continue;
            }
            match self.paint_strip(index, column)? {
                Some(strip) => {
                    self.raster.bands.strips.insert(index, strip);
                }
                None => {
                    // Give the budget back from strips far from the window, then once more.
                    self.evict_strips(origin[1], false);
                    match self.paint_strip(index, column)? {
                        Some(strip) => {
                            self.raster.bands.strips.insert(index, strip);
                        }
                        None => return Ok(false),
                    }
                }
            }
        }
        Ok(true)
    }

    /// Drop strips outside the window around world `y`. With `keep` the window counts its
    /// longer side in both directions plus a screen, so that turning back and forth keeps
    /// what was prepared; without it (a full budget), only the window itself stays.
    fn evict_strips(&mut self, y: f64, keep: bool) {
        let (Some(page), Some(shaped)) = (&self.page, &self.shaped) else {
            return;
        };
        let bounds = self.strip_bounds(page, shaped);
        let screens = self.raster.bands.screens();
        let screens = if keep {
            let side = screens.0.max(screens.1) + 1.0;
            (side, side)
        } else {
            screens
        };
        let (first, last) = self.window_strips(y, screens, (f64::NEG_INFINITY, f64::INFINITY));
        let (first, last) = (
            first.max((bounds.0 / f64::from(BAND)).floor() as i64 - 1),
            last.min((bounds.1 / f64::from(BAND)).ceil() as i64 + 1),
        );
        self.raster
            .bands
            .strips
            .retain(|index, _| *index >= first && *index <= last);
    }

    /// The published strips: the contiguous run around the window at world y `origin`.
    pub(super) fn strip_run(&self, origin: f64) -> Option<(i64, i64, Arc<[ViewerRasterBand]>)> {
        let band = f64::from(BAND);
        let view = f64::from(self.geometry.height);
        let first = (origin / band).floor() as i64;
        let last = ((origin + view - 1e-6) / band).floor() as i64;
        let strips = &self.raster.bands.strips;
        if (first..=last).any(|index| !strips.contains_key(&index)) {
            return None;
        }
        let mut top = first;
        while strips.contains_key(&(top - 1)) {
            top -= 1;
        }
        let mut bottom = last;
        while strips.contains_key(&(bottom + 1)) {
            bottom += 1;
        }
        let list: Arc<[ViewerRasterBand]> = (top..=bottom)
            .filter_map(|index| {
                strips.get(&index).map(|strip| ViewerRasterBand {
                    id: strip.id,
                    top: index as f64 * band,
                    height: BAND,
                    pixels: strip.pixels.clone(),
                    gutter: strip.gutter.clone(),
                })
            })
            .collect();
        Some((top, bottom, list))
    }

    /// Before publishing: adopt the column, keep the strips the shown page still paints the
    /// same way, and paint the ones under the window. `false` when the budget cannot.
    pub(super) fn prepare_window_strips(
        &mut self,
        origin: [f64; 2],
        column: Column,
    ) -> Result<bool, String> {
        self.adopt_strips(column);
        if self.raster.invalid {
            self.revalidate_strips(column);
            self.raster.invalid = false;
        }
        self.strips_under_window(origin, column)
    }

    /// Paint strips beyond the window in the direction of travel first, within one turn's
    /// time. Returns whether more remain; new strips mark the view dirty to publish them.
    pub(crate) fn prepare_strips(&mut self) -> bool {
        let pending = self.prepare_strips_turn();
        self.raster.bands.pending = pending;
        pending
    }

    fn prepare_strips_turn(&mut self) -> bool {
        if !self.raster.enabled || !self.retained_active() || self.raster_rebase_waiting() {
            return false;
        }
        let Some(key) = self.raster.bands.key else {
            return false;
        };
        let (Some(page), Some(shaped)) = (&self.page, &self.shaped) else {
            return false;
        };
        // Strips of an earlier raster context are revalidated before more are added.
        if self.raster.invalid {
            return false;
        }
        let origin = self.raster.camera(self.origin)[1];
        let bounds = self.strip_bounds(page, shaped);
        let screens = self.raster.bands.screens();
        let (first, last) = self.window_strips(origin, screens, bounds);
        let band = f64::from(BAND);
        let view = f64::from(self.geometry.height);
        let visible = (
            (origin / band).floor() as i64,
            ((origin + view - 1e-6) / band).floor() as i64,
        );
        // Along the travel first, nearest first; then against it.
        let along: Vec<i64> = if self.raster.bands.direction < 0 {
            (first..visible.0).rev().collect()
        } else {
            (visible.1 + 1..=last).collect()
        };
        let against: Vec<i64> = if self.raster.bands.direction < 0 {
            (visible.1 + 1..=last).collect()
        } else {
            (first..visible.0).rev().collect()
        };
        let started = Instant::now();
        let mut painted = false;
        let mut evicted = false;
        for index in along.into_iter().chain(against) {
            if self.raster.bands.strips.contains_key(&index) {
                continue;
            }
            if started.elapsed() >= TURN {
                if painted {
                    self.dirty = true;
                }
                return true;
            }
            let mut strip = self.paint_strip(index, key.column);
            if matches!(strip, Ok(None)) && !evicted {
                // The budget is full: give back the strips beyond the window first.
                evicted = true;
                self.evict_strips(origin, false);
                strip = self.paint_strip(index, key.column);
            }
            match strip {
                Ok(Some(strip)) => {
                    self.raster.bands.strips.insert(index, strip);
                    painted = true;
                }
                // The window is as large as the budget allows now. A strip that cannot be
                // painted reports its error when the window reaches it.
                Ok(None) | Err(_) => break,
            }
        }
        // Strips stay a screen beyond the window in both directions, so turning back and
        // forth reuses them instead of painting them again.
        self.evict_strips(origin, true);
        if painted {
            self.dirty = true;
        }
        false
    }
}
