//! Scrollbar coordinates describe the scrollable visual range, not a raw file fraction.
use super::*;

#[derive(Debug, PartialEq)]
pub(super) enum LoadedPosition {
    Ready,
    Reload {
        sequence: u64,
        anchor: u64,
        backwards: bool,
    },
    ReloadViewport {
        sequence: u64,
        viewport: messages::Viewport,
    },
    Unavailable,
}

pub(super) enum SourcePosition {
    Ready,
    Reload {
        sequence: u64,
        viewport: messages::Viewport,
    },
    Unavailable,
}

/// A reply can stop short of the scrollbar position it was read for, even after its
/// bounded correction reads. A semantic document keeps tables and other layout units
/// whole, so a position can fall between the last camera of one window and the first
/// camera of the next. A byte-capped text or source window of long lines can be barely
/// taller than the viewport, so its cameras cover only a sliver of the scrollbar. The
/// window's camera at `minimum` or `maximum`, whose thumb positions are `low` and `high`,
/// is then the closest view on the side where the file continues. File endpoints need
/// the real edge. A text or source window holds one contiguous byte range, so one farther
/// from the position than its own source span did not load the position's surroundings;
/// beside a document's whole table, the band no window shows can be wider than a small
/// window next to it.
fn nearest_edge(
    page: &Page,
    length: u64,
    mode: u16,
    position: f64,
    (low, high): (f64, f64),
    (minimum, maximum): (f32, f32),
) -> Option<f32> {
    if !(position > 0.0 && position < 1.0) {
        return None;
    }
    let (origin, distance) = if page.after && position > high {
        (maximum, position - high)
    } else if page.before && position < low {
        (minimum, low - position)
    } else {
        return None;
    };
    let span = page.end.saturating_sub(page.start) as f64 / length.max(1) as f64;
    (mode == viewer::MODE_DOCUMENT || distance <= span).then_some(origin)
}

impl Worker {
    pub(super) fn position_metrics(&self) -> Option<(f64, f64)> {
        self.metrics_at(self.origin[1])
    }

    /// Thumb position and extent of the loaded page at vertical camera `origin`.
    fn metrics_at(&self, origin: f32) -> Option<(f64, f64)> {
        if let Some(map) = &self.scroll_map {
            return map.metrics(f64::from(origin), f64::from(self.geometry.height));
        }
        let page = self.page.as_ref()?;
        let shaped = self.shaped.as_ref()?;
        let (start, end) = if !page.before && !page.after {
            (0, self.length)
        } else {
            (page.start, page.end)
        };
        Some(navigation::scroll_metrics(
            start,
            end,
            self.length,
            shaped.height(),
            self.geometry.height as f32,
            origin,
        ))
    }

    /// The loaded window's camera that comes closest to `position` (`nearest_edge`).
    fn nearest_loaded_edge(&self, position: f64) -> Option<f32> {
        let page = self.page.as_ref()?;
        let shaped = self.shaped.as_ref()?;
        let height = self.geometry.height as f32;
        let maximum = (crate::viewport::available_height(page, shaped, height) - height).max(0.0);
        let minimum = if self.mode == viewer::MODE_SOURCE {
            crate::viewport::source_minimum_origin(page, shaped)?
        } else {
            0.0
        };
        if minimum > maximum {
            return None;
        }
        let coverage = (self.metrics_at(minimum)?.0, self.metrics_at(maximum)?.0);
        nearest_edge(
            page,
            self.length,
            self.mode,
            position,
            coverage,
            (minimum, maximum),
        )
    }

    /// Settle a position that the loaded window cannot show at its nearest edge
    /// (`nearest_loaded_edge`); otherwise it stays unavailable.
    fn settle_at_nearest_edge(&mut self, position: f64) -> LoadedPosition {
        let Some(origin) = self.nearest_loaded_edge(position) else {
            return LoadedPosition::Unavailable;
        };
        self.origin[1] = origin;
        self.reset_raster_context();
        self.frames.invalidate();
        self.dirty = true;
        LoadedPosition::Ready
    }

    fn cached_position(&self, position: f64) -> Option<f32> {
        if self.viewport_request.is_some() {
            return None;
        }
        self.loaded_position(position)
    }

    fn loaded_position(&self, position: f64) -> Option<f32> {
        let page = self.page.as_ref()?;
        if self.mode == viewer::MODE_SOURCE && !Self::has_source_edge(page, position) {
            return None;
        }
        let shaped = self.shaped.as_ref()?;
        let maximum =
            (crate::viewport::available_height(page, shaped, self.geometry.height as f32)
                - self.geometry.height as f32)
                .max(0.0);
        if self.scroll_map.is_none()
            && self.mode == viewer::MODE_SOURCE
            && shaped.height() < self.geometry.height as f32
        {
            return self.source_short_byte_position(page, shaped, maximum, position);
        }
        let minimum = if self.mode == viewer::MODE_SOURCE {
            crate::viewport::source_minimum_origin(page, shaped)?
        } else {
            0.0
        };
        if minimum > maximum {
            return None;
        }
        if let Some(map) = &self.scroll_map {
            return map
                .inverse_near(
                    position,
                    f64::from(self.geometry.height),
                    f64::from(minimum),
                    f64::from(maximum),
                    f64::from(self.origin[1]),
                )
                .map(|origin| origin as f32);
        }
        self.byte_position(page, shaped, maximum, position)
            .filter(|origin| *origin >= minimum)
    }

    fn has_source_edge(page: &Page, position: f64) -> bool {
        (position > 0.0 || !page.before) && (position < 1.0 || !page.after)
    }

    fn byte_position(
        &self,
        page: &Page,
        shaped: &TextPage,
        maximum: f32,
        position: f64,
    ) -> Option<f32> {
        let height = f64::from(shaped.height().max(1.0));
        let (start, end) = if !page.before && !page.after {
            (0, self.length)
        } else {
            (page.start, page.end)
        };
        let bytes = end.checked_sub(start)? as f64;
        if bytes == 0.0 {
            return (!page.before && !page.after).then_some(0.0);
        }
        let visible = bytes * (f64::from(self.geometry.height) / height).min(1.0);
        let top = position.clamp(0.0, 1.0) * (self.length as f64 - visible).max(0.0);
        let origin = (top - start as f64) * height / bytes;
        // A byte-limited single row can only represent whole source offsets.
        let rounding = if maximum == 0.0 {
            (4.0 * height / bytes).max(0.001)
        } else {
            0.001
        };
        (origin >= -rounding && origin <= f64::from(maximum) + rounding)
            .then(|| (origin as f32).clamp(0.0, maximum))
    }

    /// A byte-limited packet shorter than the viewport has no row map. Keep
    /// its explicit byte inverse, including inside enormous logical lines,
    /// only when that inverse proves the requested camera is actually covered.
    /// The caller must check source edge flags and have no row map.
    fn source_short_byte_position(
        &self,
        page: &Page,
        shaped: &TextPage,
        maximum: f32,
        position: f64,
    ) -> Option<f32> {
        if self.mode != viewer::MODE_SOURCE || shaped.height() >= self.geometry.height as f32 {
            return None;
        }
        // The byte inverse tolerates scalar rounding. That tolerance cannot
        // turn a clipped packet into authority for an actual file endpoint.
        if (position == 0.0 && page.start != 0)
            || (position == 1.0
                && (page.end != self.length
                    || page.anchors.last().is_none_or(|anchor| {
                        anchor.source_offset != self.length
                            || anchor.text_offset as usize != page.text.len()
                    })))
        {
            return None;
        }
        self.byte_position(page, shaped, maximum, position)
    }

    pub(super) fn seek_position(&mut self, sequence: u64, position: f64, output: &Mailbox) {
        if self.positions.has_pending() {
            let cancel = self.mode == viewer::MODE_SOURCE && !self.positions.has_deferred();
            self.positions.defer(sequence, position);
            if cancel {
                self.position_page = None;
                if let Some(id) = self.viewport_request
                    && let Some(session) = &self.session
                {
                    session.cancel(id);
                }
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN seek_deferred
            #[cfg(test)]
            {
                if observation::enabled(self) {
                    observation::record(
                        self,
                        8,
                        21,
                        self.viewport_request,
                        observation::Extra {
                            intent: Some((sequence, position)),
                            ..Default::default()
                        },
                    );
                }
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_END seek_deferred
            return;
        }
        if let Some(id) = self.search_request.take() {
            if let Some(session) = &self.session {
                session.cancel(id);
            }
            // Its terminal event is now unrelated; a loaded-page seek sets no other status.
            if self.status == SEARCHING {
                self.status.clear();
            }
        }
        // Inspect read authority before reset can cancel a speculative correction.
        if let Some(origin) = self.cached_position(position) {
            self.origin[1] = origin;
            // Capture the new origin so completion cannot resume an older camera.
            self.reset_raster_context();
            // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN seek_cached
            #[cfg(test)]
            {
                if observation::enabled(self) {
                    observation::record(
                        self,
                        8,
                        22,
                        self.viewport_request,
                        observation::Extra {
                            intent: Some((sequence, position)),
                            ..Default::default()
                        },
                    );
                }
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_END seek_cached
            self.positions.vertical(sequence);
            self.frames.invalidate();
            self.dirty = true;
            return;
        }
        self.reset_raster_context();
        self.positions.target = Some(crate::position_ack::Target {
            sequence,
            position: position.clamp(0.0, 1.0),
            corrected: false,
        });
        // For an unloaded fragment this remains a provisional byte estimate. Do not
        // report the requested fraction as displayed until actual content arrives.
        let (anchor, backwards) = self.position_page_request(position);
        self.request_page_for(anchor, backwards, Some(sequence), output);
        if self.viewport_request.is_none() {
            if self.mode == viewer::MODE_SOURCE {
                self.source_position_unavailable(sequence, None, output);
            } else {
                self.position_context(output);
                self.dirty = true;
            }
        }
    }

    pub(super) fn retire_superseded_source_position(&mut self, id: u64, output: &Mailbox) -> bool {
        if self.mode != viewer::MODE_SOURCE || self.viewport_request != Some(id) {
            return false;
        }
        let Some((sequence, position)) = self.positions.retire_superseded(id) else {
            return false;
        };
        self.position_page = None;
        self.viewport_request = None;
        self.motion.retire_viewport();
        self.source_foreground_finished(id, false);
        self.invalidate_source_target();
        self.seek_position(sequence, position, output);
        true
    }

    pub(super) fn source_position_unavailable(
        &mut self,
        sequence: u64,
        message: Option<String>,
        output: &Mailbox,
    ) {
        self.positions.target = None;
        self.motion.retire_viewport();
        self.continuation = None;
        self.invalidate_source_target();
        self.status = message.unwrap_or_else(|| "The requested viewport was not completed.".into());
        self.dirty = true;
        output.put(ViewerOutput::VerticalPositionUnavailable {
            generation: self.positions.generation,
            sequence,
        });
    }

    pub(super) fn cancel_source_position_for_scroll(&mut self, output: &Mailbox) -> bool {
        if self.mode != viewer::MODE_SOURCE {
            return false;
        }
        let Some(id) = self.viewport_request else {
            return false;
        };
        let crate::position_ack::Completion::Failed(sequence) = self.positions.complete(id, false)
        else {
            return false;
        };
        let deferred = self.positions.take_deferred();
        if deferred.is_none()
            && let Some(session) = &self.session
        {
            session.cancel(id);
        }
        self.viewport_request = None;
        self.position_page = None;
        self.source_foreground_finished(id, false);
        let sequence = deferred.map_or(sequence, |(sequence, _)| sequence);
        self.source_position_unavailable(sequence, Some(String::new()), output);
        true
    }

    /// Camera of a Source candidate for `position` once no bounded reread can complete
    /// it. A candidate whose cameras all lie above its first complete logical line, as
    /// an 8 KiB window that starts inside a line and is barely taller than the viewport,
    /// uses its whole camera range, as a packet shorter than the viewport already does.
    /// The position is resolved in that range or settles at its nearer edge; a packet
    /// shorter than the viewport has one camera, at its top.
    fn source_fallback_origin(
        &self,
        page: &Page,
        shaped: &TextPage,
        map: Option<&ViewerScrollMap>,
        position: f64,
        minimum: Option<f32>,
        maximum: f32,
    ) -> Option<f32> {
        if !(position > 0.0 && position < 1.0) {
            return None;
        }
        let Some(map) = map else {
            let height = self.geometry.height as f32;
            if shaped.height() >= height {
                return None;
            }
            let (top, _) = navigation::scroll_metrics(
                page.start,
                page.end,
                self.length,
                shaped.height(),
                height,
                0.0,
            );
            return nearest_edge(
                page,
                self.length,
                self.mode,
                position,
                (top, top),
                (0.0, 0.0),
            );
        };
        let height = f64::from(self.geometry.height);
        let minimum = minimum.filter(|minimum| *minimum <= maximum).unwrap_or(0.0);
        let low = map.metrics(f64::from(minimum), height)?.0;
        let high = map.metrics(f64::from(maximum), height)?.0;
        if (low..=high).contains(&position) {
            return map
                .inverse_near(
                    position,
                    height,
                    f64::from(minimum),
                    f64::from(maximum),
                    f64::from(minimum),
                )
                .map(|origin| origin as f32);
        }
        nearest_edge(
            page,
            self.length,
            self.mode,
            position,
            (low, high),
            (minimum, maximum),
        )
    }

    pub(super) fn resolve_source_position(
        &mut self,
        page: Page,
        output: &Mailbox,
    ) -> SourcePosition {
        use crate::position_ack::{MissingEdge, SourceWindow};

        let Some(target) = self.positions.target else {
            return SourcePosition::Unavailable;
        };
        if !Self::has_source_edge(&page, target.position) {
            return SourcePosition::Unavailable;
        }
        let Ok(shaped) = self.shape_page(&page) else {
            return SourcePosition::Unavailable;
        };
        let height = self.geometry.height as f32;
        let maximum = (crate::viewport::available_height(&page, &shaped, height) - height).max(0.0);
        let map = scroll_map::build(&page, &shaped, self.length, true, self.geometry.height);
        let minimum = crate::viewport::source_minimum_origin(&page, &shaped);
        let coverage = minimum
            .filter(|minimum| *minimum <= maximum)
            .and_then(|minimum| {
                let map = map.as_ref()?;
                let (low, _) = map.metrics(f64::from(minimum), f64::from(height))?;
                let (high, _) = map.metrics(f64::from(maximum), f64::from(height))?;
                Some((low, high))
            });
        // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN candidate_coverage
        #[cfg(test)]
        if let Some((low, high)) = coverage
            && observation::enabled(self)
        {
            observation::record(
                self,
                12,
                0,
                self.viewport_request,
                observation::Extra {
                    candidate: Some((page.start, page.end, low, high)),
                    ..Default::default()
                },
            );
        }
        // SOURCE_COMPLETION_OBSERVATION_V2_END candidate_coverage
        let interior = target.position > 0.0 && target.position < 1.0;
        let origin = match (minimum, &map) {
            (Some(minimum), Some(map))
                if minimum <= maximum
                    && (!interior
                        || coverage.is_some_and(|(low, high)| {
                            (low..=high).contains(&target.position)
                        })) =>
            {
                map.inverse_near(
                    target.position,
                    f64::from(height),
                    f64::from(minimum),
                    f64::from(maximum),
                    f64::from(minimum),
                )
                .map(|origin| origin as f32)
            }
            (_, None) if shaped.height() < height => {
                self.source_short_byte_position(&page, &shaped, maximum, target.position)
            }
            (Some(minimum), None) => self
                .byte_position(&page, &shaped, maximum, target.position)
                .filter(|origin| *origin >= minimum),
            _ => None,
        };
        if let Some(origin) = origin {
            self.publish_page(page, shaped, Some((map, origin)), output);
            return SourcePosition::Ready;
        }
        let leading_context = page.before && page.partial_line;
        if !interior && !leading_context {
            return SourcePosition::Unavailable;
        }
        let edge = match coverage {
            Some((low, _)) if target.position < low => MissingEdge::Before,
            Some((_, high)) if target.position > high => MissingEdge::After,
            None if map.is_none() && shaped.height() < height => {
                // The target camera must contain position * length. Use that
                // to grow a short fragment after its byte inverse missed.
                let pivot = target.position * self.length as f64;
                if page.before && (!page.after || pivot < page.start as f64) {
                    MissingEdge::Before
                } else {
                    MissingEdge::After
                }
            }
            None if leading_context => MissingEdge::Before,
            _ => return SourcePosition::Unavailable,
        };

        // Session validates the page revision. The matching request and position
        // context own the recovery, including observations from prior reads.
        let window = SourceWindow {
            start: page.start,
            end: page.end,
            before: page.before,
            after: page.after,
        };
        let Some(viewport) = self
            .positions
            .source
            .as_mut()
            .and_then(|source| source.reread(window, edge))
        else {
            let origin = self.source_fallback_origin(
                &page,
                &shaped,
                map.as_ref(),
                target.position,
                minimum,
                maximum,
            );
            let Some(origin) = origin else {
                return SourcePosition::Unavailable;
            };
            self.publish_page(page, shaped, Some((map, origin)), output);
            return SourcePosition::Ready;
        };
        SourcePosition::Reload {
            sequence: target.sequence,
            viewport,
        }
    }

    pub(super) fn position_page_request(&self, position: f64) -> (u64, bool) {
        let extent = self.position_metrics().map_or(0.0, |(_, extent)| extent);
        let (anchor, backwards) = navigation::scrollbar_viewport(position, self.length, extent);
        if backwards {
            // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN backward_calculation
            #[cfg(test)]
            {
                if observation::enabled(self) {
                    observation::record(
                        self,
                        1,
                        0,
                        self.viewport_request,
                        observation::Extra {
                            calculation: Some((position, extent, anchor, 0, anchor)),
                            correction: Some((anchor, true)),
                            ..Default::default()
                        },
                    );
                }
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_END backward_calculation
            return (anchor, true);
        }
        let before = self
            .page
            .as_ref()
            .zip(self.shaped.as_ref())
            .map_or(0, |(page, shaped)| {
                if shaped.height() <= self.geometry.height as f32 {
                    return 0;
                }
                let spare =
                    (crate::viewport::available_height(page, shaped, self.geometry.height as f32)
                        - self.geometry.height as f32)
                        .max(0.0);
                let margin =
                    f64::from(raster::guard(self.geometry, false)[1]).min(f64::from(spare) / 2.0);
                let fraction = (margin / f64::from(shaped.height().max(1.0))).min(0.25);
                if fraction == 0.0 {
                    return 0;
                }
                let bytes = (page.end - page.start) as f64;
                let mapped = self.scroll_map.as_ref().and_then(|map| {
                    // This query's extent is F(margin) - F(0), not a thumb
                    // position. It respects actual row density and plateaus.
                    map.metrics(0.0, fraction * f64::from(shaped.height().max(1.0)))
                });
                match mapped {
                    Some((_, source_fraction)) => (source_fraction * self.length as f64)
                        .clamp(0.0, bytes * 0.25)
                        .ceil() as u64,
                    None => (bytes * fraction).ceil() as u64,
                }
            });
        // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN forward_calculation
        #[cfg(test)]
        {
            if observation::enabled(self) {
                observation::record(
                    self,
                    1,
                    0,
                    self.viewport_request,
                    observation::Extra {
                        calculation: Some((
                            position,
                            extent,
                            anchor,
                            before,
                            anchor.saturating_sub(before),
                        )),
                        correction: Some((anchor.saturating_sub(before), false)),
                        ..Default::default()
                    },
                );
            }
        }
        // SOURCE_COMPLETION_OBSERVATION_V2_END forward_calculation
        (anchor.saturating_sub(before), false)
    }

    /// A missed Text edge needs more of the same window rather than another
    /// equally short window shifted past context it already owns.
    fn text_edge_correction(
        &mut self,
        target: crate::position_ack::Target,
    ) -> Option<LoadedPosition> {
        if self.mode != viewer::MODE_TEXT {
            return None;
        }
        let page = self.page.as_ref()?;
        let shaped = self.shaped.as_ref()?;
        let height = self.geometry.height as f32;
        let maximum = (crate::viewport::available_height(page, shaped, height) - height).max(0.0);
        let map = self.scroll_map.as_ref()?;
        let low = map.metrics(0.0, f64::from(height))?.0;
        let high = map.metrics(f64::from(maximum), f64::from(height))?.0;
        let (anchor, backwards, available) = if target.position < low {
            (page.end, true, page.before)
        } else if target.position > high {
            (page.start, false, page.after)
        } else {
            return None;
        };
        if !available {
            return None;
        }
        let mut viewport = self.page_viewport(anchor, backwards);
        let lines = viewport.max_lines.saturating_mul(2).min(512);
        if lines <= viewport.max_lines {
            return None;
        }
        viewport.max_lines = lines;
        self.positions.target = Some(crate::position_ack::Target {
            corrected: true,
            ..target
        });
        Some(LoadedPosition::ReloadViewport {
            sequence: target.sequence,
            viewport,
        })
    }

    /// A newly decoded fragment has its own byte-to-height estimate. Resolve the
    /// same absolute intent in that layout before acknowledging it, retaining a
    /// small prefix for density changes. At most one correction read is allowed.
    pub(super) fn refine_position(&mut self) -> LoadedPosition {
        let Some(target) = self.positions.target else {
            // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN refine_no_target
            #[cfg(test)]
            {
                if observation::enabled(self) {
                    observation::record(self, 8, 20, self.viewport_request, Default::default());
                }
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_END refine_no_target
            return LoadedPosition::Ready;
        };
        // The already queued latest intent owns the next request; do not spend
        // another read refining a superseded intermediate target.
        if self.positions.has_deferred() {
            // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN refine_deferred
            #[cfg(test)]
            {
                if observation::enabled(self) {
                    observation::record(self, 8, 21, self.viewport_request, Default::default());
                }
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_END refine_deferred
            return LoadedPosition::Ready;
        }
        if let Some(origin) = self.loaded_position(target.position) {
            self.origin[1] = origin;
            self.reset_raster_context();
            self.frames.invalidate();
            self.dirty = true;
            // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN refine_loaded
            #[cfg(test)]
            {
                if observation::enabled(self) {
                    observation::record(self, 8, 22, self.viewport_request, Default::default());
                }
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_END refine_loaded
            return LoadedPosition::Ready;
        }
        if target.corrected {
            // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN refine_corrected_uncovered
            #[cfg(test)]
            {
                if observation::enabled(self) {
                    observation::record(self, 8, 5, self.viewport_request, Default::default());
                }
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_END refine_corrected_uncovered
            return self.settle_at_nearest_edge(target.position);
        }
        if let Some(correction) = self.text_edge_correction(target) {
            return correction;
        }
        let (anchor, backwards) = self.position_page_request(target.position);
        if self.page.as_ref().is_none_or(|page| {
            if backwards {
                anchor == page.end
            } else {
                anchor == page.start
            }
        }) {
            // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN refine_no_progress
            #[cfg(test)]
            {
                if observation::enabled(self) {
                    let reason = if self.page.is_none() {
                        6
                    } else if backwards {
                        8
                    } else {
                        7
                    };
                    observation::record(
                        self,
                        8,
                        reason,
                        self.viewport_request,
                        observation::Extra {
                            correction: Some((anchor, backwards)),
                            ..Default::default()
                        },
                    );
                }
            }
            // SOURCE_COMPLETION_OBSERVATION_V2_END refine_no_progress
            return self.settle_at_nearest_edge(target.position);
        }
        self.positions.target = Some(crate::position_ack::Target {
            corrected: true,
            ..target
        });
        // SOURCE_COMPLETION_OBSERVATION_V2_BEGIN refine_reload
        #[cfg(test)]
        {
            if observation::enabled(self) {
                observation::record(
                    self,
                    8,
                    23,
                    self.viewport_request,
                    observation::Extra {
                        correction: Some((anchor, backwards)),
                        ..Default::default()
                    },
                );
            }
        }
        // SOURCE_COMPLETION_OBSERVATION_V2_END refine_reload
        LoadedPosition::Reload {
            sequence: target.sequence,
            anchor,
            backwards,
        }
    }
}
// SOURCE_COMPLETION_OBSERVATION_V2_BEGIN observation_module
#[cfg(test)]
pub(super) mod observation {
    use super::Worker;
    use midnight_plugin_host::{EventKind, api::viewer};
    use std::io::Write;

    /// Incoming protocol metadata, including the revision not retained by Page.
    pub type Incoming = (u64, u64, u64, usize, usize, usize, bool, bool, bool);

    #[derive(Default)]
    pub struct Extra {
        pub event_kind: Option<u8>,
        pub incoming: Option<Incoming>,
        pub failure_code: Option<u32>,
        pub viewport: Option<(u64, bool, u32, u32)>,
        pub calculation: Option<(f64, f64, u64, u64, u64)>,
        pub intent: Option<(u64, f64)>,
        pub admitted: Option<bool>,
        pub admitted_id: Option<u64>,
        pub page_match: Option<bool>,
        pub page_applied: Option<bool>,
        pub completion: Option<u8>,
        pub completion_success: Option<bool>,
        pub cancelled_request: Option<u64>,
        pub reset_done: Option<bool>,
        pub correction: Option<(u64, bool)>,
        pub candidate: Option<(u64, u64, f64, f64)>,
    }

    impl Extra {
        pub fn event(event: &EventKind) -> Self {
            Self {
                event_kind: Some(match event {
                    EventKind::TextPage(_) => 1,
                    EventKind::Completed => 2,
                    EventKind::Cancelled => 3,
                    EventKind::Failed(_) => 4,
                    _ => 0,
                }),
                incoming: match event {
                    EventKind::TextPage(page) => Some((
                        page.revision,
                        page.source.offset,
                        page.source.length,
                        page.text.len(),
                        page.anchors.len(),
                        page.spans.len(),
                        page.has_before,
                        page.has_after,
                        page.partial_line,
                    )),
                    _ => None,
                },
                failure_code: match event {
                    EventKind::Failed(failure) => Some(failure.code),
                    _ => None,
                },
                ..Self::default()
            }
        }
    }

    pub fn enabled(worker: &Worker) -> bool {
        worker.mode == viewer::MODE_SOURCE
            && std::env::var_os("MIDNIGHT_VIEWER_SCROLL_TRACE").is_some()
    }

    /// One bounded line, emitted before destructive completion/reset operations.
    /// All Debug arguments below are explicit scalar options/tuples, never model objects.
    pub fn record(worker: &Worker, stage: u8, reason: u8, request_id: Option<u64>, extra: Extra) {
        if !enabled(worker) {
            return;
        }
        let active = worker.page.as_ref().map(|page| {
            (
                page.start,
                page.end,
                page.text.len(),
                page.anchors.len(),
                page.spans.len(),
                page.before,
                page.after,
            )
        });
        let staged = worker.position_page.as_ref().map(|(id, page)| {
            (
                *id,
                page.start,
                page.end,
                page.text.len(),
                page.anchors.len(),
                page.spans.len(),
                page.before,
                page.after,
            )
        });
        let shaped_height = worker.shaped.as_ref().map(|shaped| shaped.height());
        let available_height =
            worker
                .page
                .as_ref()
                .zip(worker.shaped.as_ref())
                .map(|(page, shaped)| {
                    crate::viewport::available_height(page, shaped, worker.geometry.height as f32)
                });
        let maximum_y =
            available_height.map(|height| (height - worker.geometry.height as f32).max(0.0));
        // Endpoint metrics only. Never run layout, refinement or map inversion here.
        let low_metrics = worker
            .scroll_map
            .as_ref()
            .and_then(|map| map.metrics(0.0, f64::from(worker.geometry.height)));
        let high_metrics = worker.scroll_map.as_ref().and_then(|map| {
            maximum_y.and_then(|maximum| {
                map.metrics(f64::from(maximum), f64::from(worker.geometry.height))
            })
        });
        let target = worker
            .positions
            .target
            .map(|target| (target.sequence, target.position, target.corrected));
        let _ = writeln!(
            std::io::stderr().lock(),
            concat!(
                "source_completion_observation_v2 stage={} reason={} request={:?} ",
                "protocol_generation={} position_generation={} target={:?} ack={:?} ",
                "pending={} loaded={} deferred={} viewport_request={:?} source_length={} ",
                "active={:?} staged={:?} geometry={:?} origin={:?} shaped_height={:?} ",
                "available_height={:?} maximum_y={:?} map={} low_metrics={:?} high_metrics={:?} ",
                "event={:?} incoming={:?} failure_code={:?} viewport={:?} calculation={:?} ",
                "intent={:?} admitted={:?} admitted_id={:?} page_match={:?} page_applied={:?} ",
                "completion={:?} completion_success={:?} cancelled_request={:?} ",
                "reset_done={:?} correction={:?} candidate={:?}"
            ),
            stage,
            reason,
            request_id,
            worker.generation,
            worker.positions.generation,
            target,
            worker.positions.acknowledged,
            worker.positions.has_pending(),
            worker.positions.uncompleted_page(),
            worker.positions.has_deferred(),
            worker.viewport_request,
            worker.length,
            active,
            staged,
            (
                worker.geometry.width,
                worker.geometry.height,
                worker.geometry.scale
            ),
            worker.origin,
            shaped_height,
            available_height,
            maximum_y,
            worker.scroll_map.is_some(),
            low_metrics,
            high_metrics,
            extra.event_kind,
            extra.incoming,
            extra.failure_code,
            extra.viewport,
            extra.calculation,
            extra.intent,
            extra.admitted,
            extra.admitted_id,
            extra.page_match,
            extra.page_applied,
            extra.completion,
            extra.completion_success,
            extra.cancelled_request,
            extra.reset_done,
            extra.correction,
            extra.candidate,
        );
    }
}
// SOURCE_COMPLETION_OBSERVATION_V2_END observation_module
