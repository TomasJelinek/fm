//! Bounded retained pixels and a camera that survives source-fragment rebasing.
use super::*;

#[path = "worker_raster_rebase.rs"]
mod rebase;

const GUARD: u32 = 256;
const MAX_PIXELS: u64 = 6 * 1024 * 1024;

#[derive(Default)]
pub(super) struct Retained {
    pub enabled: bool,
    transition: u64,
    handoff: Option<rebase::Handoff>,
    rebase_ack: Option<u64>,
    pub fallback_scroll: f32,
    surface: u64,
    pixel: u64,
    layout: u64,
    first_pixel: u64,
    base: [f64; 2],
    target: [f64; 2],
    sequence: u64,
    exact: bool,
    pub continuing: Option<[f64; 2]>,
    refill_attempt: Option<(u64, u64, u64, bool)>,
    cache: Option<Snapshot>,
    invalid: bool,
    selected_source: Option<[u64; 2]>,
    selected_text: Option<String>,
    selection_valid: bool,
}

#[derive(Clone)]
pub(super) struct Snapshot {
    layout: u64,
    pub width: u32,
    pub height: u32,
    pub pixels: Arc<[u8]>,
    pub info: ViewerRasterInfo,
}

impl Retained {
    pub fn reset(&mut self, origin: [f32; 2]) {
        self.surface = self
            .surface
            .checked_add(1)
            .expect("raster context exhausted");
        self.first_pixel = self
            .pixel
            .checked_add(1)
            .expect("raster pixel identity exhausted");
        self.handoff = None;
        self.rebase_ack = None;
        self.fallback_scroll = 0.0;
        self.base = [0.0; 2];
        self.target = origin.map(f64::from);
        self.sequence = 0;
        self.exact = true;
        self.continuing = None;
        self.refill_attempt = None;
        self.cache = None;
        self.invalid = true;
        self.selected_source = None;
        self.selected_text = None;
        self.selection_valid = false;
    }
    pub fn invalidate(&mut self) {
        self.invalid = true;
    }
    fn camera(&self, origin: [f32; 2]) -> [f64; 2] {
        std::array::from_fn(|axis| self.base[axis] + f64::from(origin[axis]))
    }
}

fn inside(origin: [f64; 2], minimum: [f64; 2], maximum: [f64; 2]) -> bool {
    (0..2).all(|axis| {
        origin[axis].is_finite()
            && origin[axis] >= minimum[axis] - 1e-5
            && origin[axis] <= maximum[axis] + 1e-5
    })
}

fn same_phase(a: [f64; 2], b: [f64; 2]) -> bool {
    (0..2).all(|axis| ((a[axis] - b[axis]) - (a[axis] - b[axis]).round()).abs() < 1e-7)
}

pub(super) fn guard(geometry: ViewerGeometry, horizontal: bool) -> [u32; 2] {
    let mut low = 0;
    // Guard distance is logical: high-DPI monitors need the same time to refill
    // while scrolling. Physical allocation still obeys the shared pixel ceiling.
    let mut high = (GUARD as f32 * geometry.scale).ceil() as u32;
    while low < high {
        let middle = (low + high).div_ceil(2);
        let width = u64::from(geometry.width) + if horizontal { u64::from(middle) * 2 } else { 0 };
        let height = u64::from(geometry.height) + u64::from(middle) * 2;
        if width
            .checked_mul(height)
            .is_some_and(|pixels| pixels <= MAX_PIXELS)
        {
            low = middle;
        } else {
            high = middle - 1;
        }
    }
    [if horizontal { low } else { 0 }, low]
}

impl Worker {
    pub(super) fn retained_active(&self) -> bool {
        self.raster.enabled
            && matches!(
                self.mode,
                viewer::MODE_TEXT
                    | viewer::MODE_SOURCE
                    | viewer::MODE_DOCUMENT
                    | viewer::MODE_BINARY
            )
            && !self
                .page
                .as_ref()
                .zip(self.shaped.as_ref())
                .is_some_and(|(page, shaped)| {
                    let maximum = (crate::viewport::available_height(
                        page,
                        shaped,
                        self.geometry.height as f32,
                    ) - self.geometry.height as f32)
                        .max(0.0);
                    // A retained refill must preserve the displayed frontier and
                    // advance a complete source row. Tiny bounded fragments use
                    // the viewport path until sufficient context is available.
                    (page.after
                        && !crate::viewport::Continuation::retained_refill(
                            page,
                            shaped,
                            self.geometry,
                            [0.0, maximum],
                            0.0,
                            false,
                            matches!(self.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE),
                        )
                        .is_some_and(|next| next.anchor > page.start && maximum > 0.0))
                        || (page.before
                            && !crate::viewport::Continuation::retained_refill(
                                page,
                                shaped,
                                self.geometry,
                                [0.0, 0.0],
                                0.0,
                                true,
                                matches!(self.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE),
                            )
                            .is_some_and(|next| next.anchor < page.end && maximum > 0.0))
                })
    }

    pub(super) fn reset_raster_context(&mut self) {
        self.cancel_raster_rebase();
        self.raster.reset(self.origin);
        // No-match searches, font changes and direct scrollbar movement keep the
        // existing accepted selection. A rejected same-context selection never
        // takes this path and cannot silently restore old Copy contents.
        if self.retained_active() && self.selection.is_some() {
            self.capture_raster_selection();
        }
    }

    pub(super) fn raster_input(
        &mut self,
        surface: ViewerSurfaceId,
        input: ViewerRasterInput,
        output: &Mailbox,
    ) {
        if self.raster_rebase_blocks(surface, &input) {
            return;
        }
        if !self.retained_active() || surface.0 != self.raster.surface {
            return;
        }
        match input {
            ViewerRasterInput::PanTo {
                sequence,
                origin,
                settled,
            } => {
                if sequence == 0
                    || sequence <= self.raster.sequence
                    || origin
                        .iter()
                        .any(|value| !value.is_finite() || value.abs() > 1e12)
                {
                    return;
                }
                self.raster.rebase_ack = None;
                self.raster.sequence = sequence;
                self.raster.target = origin;
                self.raster.exact = settled;
                self.apply_raster_target(output);
            }
            ViewerRasterInput::Select {
                pixel_id,
                origin,
                x,
                y,
                extend,
            } => {
                if !x.is_finite() || !y.is_finite() {
                    self.raster.selection_valid = false;
                    return;
                }
                if !self.raster_selection_camera(pixel_id, origin, output) {
                    return;
                }
                self.raster.rebase_ack = None;
                if let Some(page) = &self.shaped
                    && let Ok(index) = page.hit_test(
                        (x - self.text_margin()).max(0.0) + self.origin[0],
                        y + self.origin[1],
                    )
                {
                    let start = if extend {
                        self.selection.map_or(index, |selection| selection.0)
                    } else {
                        index
                    };
                    self.selection = Some((start, index));
                    self.capture_raster_selection();
                    self.raster.invalidate();
                    self.dirty = true;
                }
            }
            ViewerRasterInput::SelectPage { pixel_id, origin } => {
                if !self.raster_selection_camera(pixel_id, origin, output) {
                    return;
                }
                self.raster.rebase_ack = None;
                if let Some(page) = &self.shaped {
                    // Added guard rows are not part of the page the user can see.
                    let first = page
                        .line_at_y(self.origin[1])
                        .map_or(0, |(range, _)| range.start);
                    let last = page
                        .line_at_y(self.origin[1] + self.geometry.height as f32 - 0.01)
                        .map_or(page.text().len(), |(range, _)| range.end);
                    self.selection = Some((first, last));
                    self.capture_raster_selection();
                    self.raster.invalidate();
                    self.dirty = true;
                }
            }
            ViewerRasterInput::Copy => {
                self.raster.rebase_ack = None;
                if self.raster.selection_valid
                    && let Some(text) = &self.raster.selected_text
                {
                    output.put(ViewerOutput::CopyText(text.clone()));
                }
            }
        }
    }

    fn raster_selection_camera(
        &mut self,
        pixel_id: u64,
        origin: [f64; 2],
        output: &Mailbox,
    ) -> bool {
        let valid = self.raster.cache.as_ref().is_some_and(|cache| {
            cache.layout == self.raster.layout
                && pixel_id >= self.raster.first_pixel
                && pixel_id <= self.raster.pixel
        }) && self.page.as_ref().zip(self.shaped.as_ref()).is_some_and(
            |(page, shaped)| {
                let maximum = [
                    self.raster.base[0] + f64::from((shaped.width() - self.text_width()).max(0.0)),
                    self.raster.base[1]
                        + f64::from(
                            (crate::viewport::available_height(
                                page,
                                shaped,
                                self.geometry.height as f32,
                            ) - self.geometry.height as f32)
                                .max(0.0),
                        ),
                ];
                inside(origin, self.raster.base, maximum)
            },
        );
        if !valid {
            // A rejected new selection must not make a subsequent Copy return the old one.
            self.raster.selection_valid = false;
            self.status(
                "The displayed text changed. Try the selection again.".into(),
                output,
            );
            return false;
        }
        self.raster.target = origin;
        self.origin = std::array::from_fn(|axis| (origin[axis] - self.raster.base[axis]) as f32);
        self.raster.exact = true;
        true
    }

    pub(super) fn capture_raster_selection(&mut self) {
        let Some((first, last)) = self.selection else {
            return;
        };
        let (Some(page), Some(shaped)) = (&self.page, &self.shaped) else {
            return;
        };
        self.raster.selected_source = page
            .anchors
            .iter()
            .find(|anchor| anchor.text_offset as usize == first)
            .zip(
                page.anchors
                    .iter()
                    .find(|anchor| anchor.text_offset as usize == last),
            )
            .map(|(first, last)| [first.source_offset, last.source_offset]);
        self.raster.selected_text = shaped
            .selected_text(first.min(last)..first.max(last))
            .ok()
            .map(str::to_owned);
        self.raster.selection_valid = self.raster.selected_text.is_some();
    }

    pub(super) fn raster_page_loaded(&mut self, continuing: Option<[f64; 2]>, restored: [f32; 2]) {
        if !self.retained_active() {
            if self.raster.enabled {
                self.raster.reset(self.origin);
            }
            return;
        }
        self.raster.layout = self
            .raster
            .layout
            .checked_add(1)
            .expect("raster layout exhausted");
        self.raster.first_pixel = self
            .raster
            .pixel
            .checked_add(1)
            .expect("raster pixel identity exhausted");
        if let Some(camera) = continuing {
            // Only Y rebases between source fragments. Derive it before EOF
            // clamping; a clamped camera must not shift the world's real end.
            self.raster.base = [0.0, camera[1] - f64::from(restored[1])];
            if let (Some(page), Some([start, end])) = (&self.page, self.raster.selected_source) {
                self.selection = page
                    .anchors
                    .iter()
                    .find(|anchor| anchor.source_offset == start)
                    .zip(
                        page.anchors
                            .iter()
                            .find(|anchor| anchor.source_offset == end),
                    )
                    .map(|(a, b)| (a.text_offset as usize, b.text_offset as usize));
            }
            self.raster.invalidate();
        } else {
            self.raster.reset(self.origin);
            if self.selection.is_some() {
                self.capture_raster_selection();
            }
        }
    }

    pub(super) fn abort_raster_refill(&mut self, message: &str, output: &Mailbox) -> bool {
        if self.raster.continuing.take().is_none() {
            return false;
        }
        self.continuation = None;
        self.raster.refill_attempt = None;
        output.put(ViewerOutput::RasterUnavailable {
            surface: ViewerSurfaceId(self.raster.surface),
            message: message.into(),
        });
        // Keep target/sequence unchanged. An unavailable target must not receive a
        // false acknowledgement; the next displayed-camera retry can replace it.
        true
    }

    pub(super) fn apply_raster_target(&mut self, output: &Mailbox) {
        self.apply_raster_target_with(output, true);
    }

    pub(super) fn resume_raster_target(&mut self, output: &Mailbox) {
        self.apply_raster_target_with(output, false);
    }

    fn apply_raster_target_with(&mut self, output: &Mailbox, prefetch: bool) {
        if self.raster.continuing.is_some() || self.raster_rebase_waiting() {
            return;
        }
        let (Some(page), Some(shaped)) = (&self.page, &self.shaped) else {
            return;
        };
        let maximum = [
            (shaped.width() - self.text_width()).max(0.0),
            (crate::viewport::available_height(page, shaped, self.geometry.height as f32)
                - self.geometry.height as f32)
                .max(0.0),
        ];
        let mut local: [f32; 2] =
            std::array::from_fn(|axis| (self.raster.target[axis] - self.raster.base[axis]) as f32);
        local[0] = local[0].clamp(0.0, maximum[0]);
        if !page.before {
            local[1] = local[1].max(0.0);
        }
        if !page.after {
            local[1] = local[1].min(maximum[1]);
        }
        self.raster.target = self.raster.camera(local);
        let guard_y = (guard(self.geometry, maximum[0] > 0.0)[1] as f32).min(maximum[1] / 2.0);
        let backwards = local[1] < guard_y && page.before;
        let forwards = local[1] + guard_y > maximum[1] && page.after;
        let covered = local[1] >= 0.0 && local[1] <= maximum[1];
        let refill = ((backwards || forwards) && (!covered || (prefetch && !self.raster.exact)))
            .then(|| {
                crate::viewport::Continuation::retained_refill(
                    page,
                    shaped,
                    self.geometry,
                    local,
                    guard_y,
                    backwards && (!forwards || local[1] < self.origin[1]),
                    matches!(self.mode, viewer::MODE_TEXT | viewer::MODE_SOURCE),
                )
            })
            .flatten();
        if covered {
            self.origin = local;
            self.dirty = true;
        }
        if self.viewport_request.is_none()
            && let Some(continuation) = refill
        {
            let attempt = (
                page.start,
                page.end,
                continuation.anchor,
                continuation.backwards,
            );
            // A byte-limited response may have less than the desired guard. Do not
            // spin on an identical prefetch; later camera progress chooses a new anchor.
            if self.raster.refill_attempt == Some(attempt) {
                return;
            }
            self.raster.refill_attempt = Some(attempt);
            let camera = self.raster.target;
            let has_session = self.session.is_some();
            self.request_page(continuation.anchor, continuation.backwards, output);
            self.continuation = Some(continuation);
            self.raster.continuing = Some(camera);
            if has_session && self.viewport_request.is_none() {
                self.abort_raster_refill("Cannot request the next text page.", output);
            }
        }
    }

    pub(super) fn retained_pixels(&mut self) -> Result<Option<Snapshot>, String> {
        let (Some(page), Some(shaped)) = (&self.page, &self.shaped) else {
            return Ok(None);
        };
        let origin = self.raster.camera(self.origin);
        // Reset contexts also include legacy keyboard/scrollbar and final reflow clamps.
        if self.raster.sequence == 0 && self.raster.continuing.is_none() {
            self.raster.target = origin;
        }
        if (0..2).any(|axis| (origin[axis] - self.raster.target[axis]).abs() > 1e-4) {
            return Ok(None);
        }
        let height = crate::viewport::available_height(page, shaped, self.geometry.height as f32);
        let maximum = [
            (shaped.width() - self.text_width()).max(0.0),
            (height - self.geometry.height as f32).max(0.0),
        ];
        let local_min = self.raster.base;
        let local_max: [f64; 2] =
            std::array::from_fn(|axis| self.raster.base[axis] + f64::from(maximum[axis]));
        let limits_min = [Some(local_min[0]), (!page.before).then_some(local_min[1])];
        let limits_max = [Some(local_max[0]), (!page.after).then_some(local_max[1])];
        let reusable = self.raster.cache.as_ref().is_some_and(|cache| {
            let inside = inside(origin, cache.info.coverage_min, cache.info.coverage_max);
            let near_edge = (0..2).any(|axis| {
                (origin[axis] - cache.info.coverage_min[axis] < 64.0
                    && cache.info.coverage_min[axis] > local_min[axis] + 1.0)
                    || (cache.info.coverage_max[axis] - origin[axis] < 64.0
                        && cache.info.coverage_max[axis] < local_max[axis] - 1.0)
            });
            inside
                && !near_edge
                && (!self.raster.exact || same_phase(origin, cache.info.raster_origin))
        });
        if self.raster.invalid || !reusable {
            let Some(reservation) = self.raster_budget.reserve_for(self.raster_owner) else {
                self.dirty = true;
                return Ok(None);
            };
            let margin = self.text_margin();
            let gutter_width = if self.mode == viewer::MODE_SOURCE && self.preferences.line_numbers
            {
                margin as u32
            } else {
                0
            };
            let guards = guard(self.geometry, maximum[0] > 0.0);
            // Integer padding preserves exact phase at the acknowledged camera, even near BOF.
            let before: [u32; 2] = std::array::from_fn(|axis| {
                guards[axis].min(self.origin[axis].max(0.0).ceil() as u32)
            });
            let after: [u32; 2] = std::array::from_fn(|axis| {
                guards[axis].min((maximum[axis] - self.origin[axis]).max(0.0).ceil() as u32)
            });
            let size = [
                self.geometry.width - gutter_width + before[0] + after[0],
                self.geometry.height + before[1] + after[1],
            ];
            let pixels = u64::from(size[0] + gutter_width) * u64::from(size[1]);
            if pixels > MAX_PIXELS {
                return Err("Retained raster exceeds pixel budget.".into());
            }
            let local_raster: [f32; 2] =
                std::array::from_fn(|axis| self.origin[axis] - before[axis] as f32);
            let raster_origin = self.raster.camera(local_raster);
            let body = self
                .renderer
                .rasterize(
                    shaped,
                    Viewport {
                        width: size[0],
                        height: size[1],
                        origin: [
                            local_raster[0] - if gutter_width == 0 { margin } else { 0.0 },
                            local_raster[1],
                        ],
                        background: self.appearance.background,
                        selection_color: self.appearance.selection,
                    },
                    self.selection.map(|(a, b)| a.min(b)..a.max(b)),
                )
                .map_err(|e| format!("Cannot render retained page: {e:?}"))?;
            let gutter = if gutter_width != 0 {
                let mut raster = midnight_viewer_render::Raster {
                    width: gutter_width,
                    height: size[1],
                    rgba: vec![0; gutter_width as usize * size[1] as usize * 4],
                };
                text::paint_gutter(
                    &mut self.renderer,
                    &self.text,
                    &self.appearance,
                    self.geometry,
                    local_raster,
                    margin,
                    &mut raster,
                )?;
                Some(ViewerRasterGutter {
                    width: raster.width,
                    height: raster.height,
                    pixels: raster.rgba.into(),
                })
            } else {
                None
            };
            let pixels: Arc<[u8]> = body.rgba.into();
            reservation
                .commit(&pixels, gutter.as_ref().map(|gutter| &gutter.pixels))
                .map_err(str::to_owned)?;
            self.raster.pixel = self
                .raster
                .pixel
                .checked_add(1)
                .ok_or("Raster pixel identity exhausted.")?;
            let coverage_min = std::array::from_fn(|axis| raster_origin[axis].max(local_min[axis]));
            let coverage_max = [
                (raster_origin[0] + f64::from(before[0] + after[0])).min(local_max[0]),
                (raster_origin[1] + f64::from(before[1] + after[1])).min(local_max[1]),
            ];
            self.raster.cache = Some(Snapshot {
                layout: self.raster.layout,
                width: body.width,
                height: body.height,
                pixels,
                info: ViewerRasterInfo {
                    surface: ViewerSurfaceId(self.raster.surface),
                    pixel_id: self.raster.pixel,
                    origin,
                    raster_origin,
                    viewport: [self.geometry.width, self.geometry.height],
                    coverage_min,
                    coverage_max,
                    limit_min: limits_min,
                    limit_max: limits_max,
                    pan_sequence: self.raster.sequence,
                    scroll_per_pixel: [0.0; 2],
                    settled: true,
                    gutter,
                },
            });
            self.raster.invalid = false;
        }
        let cache = self
            .raster
            .cache
            .as_mut()
            .expect("prepared retained pixels");
        cache.info.origin = origin;
        cache.info.pan_sequence = self.raster.sequence;
        cache.info.limit_min = limits_min;
        cache.info.limit_max = limits_max;
        cache.info.settled = same_phase(origin, cache.info.raster_origin);
        let (start, end) = if !page.before && !page.after {
            (0, self.length)
        } else {
            (page.start, page.end)
        };
        let bytes = end.saturating_sub(start) as f64;
        let shaped_height = f64::from(shaped.height().max(1.0));
        let visible_bytes = bytes * (f64::from(self.geometry.height) / shaped_height).min(1.0);
        cache.info.scroll_per_pixel = [
            if maximum[0] > 0.0 {
                1.0 / f64::from(maximum[0])
            } else {
                0.0
            },
            if self.length as f64 > visible_bytes {
                bytes / shaped_height / (self.length as f64 - visible_bytes)
            } else {
                0.0
            },
        ];
        Ok(Some(cache.clone()))
    }
}
