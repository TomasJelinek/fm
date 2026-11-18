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
    Unavailable,
}

impl Worker {
    pub(super) fn position_metrics(&self) -> Option<(f64, f64)> {
        if let Some(map) = &self.scroll_map {
            return map.metrics(f64::from(self.origin[1]), f64::from(self.geometry.height));
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
            self.origin[1],
        ))
    }

    fn cached_position(&self, position: f64) -> Option<f32> {
        if self.viewport_request.is_some() {
            return None;
        }
        self.loaded_position(position)
    }

    fn loaded_position(&self, position: f64) -> Option<f32> {
        let page = self.page.as_ref()?;
        let shaped = self.shaped.as_ref()?;
        let maximum =
            (crate::viewport::available_height(page, shaped, self.geometry.height as f32)
                - self.geometry.height as f32)
                .max(0.0);
        if let Some(map) = &self.scroll_map {
            return map
                .inverse_near(
                    position,
                    f64::from(self.geometry.height),
                    0.0,
                    f64::from(maximum),
                    f64::from(self.origin[1]),
                )
                .map(|origin| origin as f32);
        }
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
        let maximum =
            (crate::viewport::available_height(page, shaped, self.geometry.height as f32)
                - self.geometry.height as f32)
                .max(0.0);
        // A byte-limited single row can only represent whole source offsets.
        let rounding = if maximum == 0.0 {
            (4.0 * height / bytes).max(0.001)
        } else {
            0.001
        };
        (origin >= -rounding && origin <= f64::from(maximum) + rounding)
            .then(|| (origin as f32).clamp(0.0, maximum))
    }

    pub(super) fn seek_position(&mut self, sequence: u64, position: f64, output: &Mailbox) {
        if self.positions.has_pending() {
            self.positions.defer(sequence, position);
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
            self.position_context(output);
            self.dirty = true;
        }
    }

    fn position_page_request(&self, position: f64) -> (u64, bool) {
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
            return LoadedPosition::Unavailable;
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
            return LoadedPosition::Unavailable;
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

    /// Scalar protocol metadata only; internal Page does not retain revision/partial_line.
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
                "reset_done={:?} correction={:?}"
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
        );
    }
}
// SOURCE_COMPLETION_OBSERVATION_V2_END observation_module
