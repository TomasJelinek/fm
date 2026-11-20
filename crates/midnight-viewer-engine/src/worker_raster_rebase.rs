//! Automatic retained-to-viewport/context handoff at the displayed camera.
use super::*;

#[derive(Clone, Copy)]
enum Phase {
    Waiting,
    Reading { prepared: bool },
}

#[derive(Clone, Copy)]
pub(super) struct Handoff {
    transition: u64,
    surface: ViewerSurfaceId,
    requested: [f64; 2],
    restored: [f32; 2],
    phase: Phase,
}

impl Retained {
    fn reset_automatic(&mut self, origin: [f32; 2]) {
        let source = self.selected_source;
        let text = self.selected_text.take();
        let valid = self.selection_valid;
        self.reset(origin);
        self.selected_source = source;
        self.selected_text = text;
        self.selection_valid = valid;
    }
}

impl Worker {
    /// Shows the retained selection's source range on the current page. The retained
    /// selection itself is unchanged, so Copy may still name frames that showed it;
    /// `publish_page` compares the selection of the page it replaces.
    fn restore_rebase_selection(&mut self) {
        if let (Some(page), Some([start, end])) = (&self.page, self.raster.selected_source) {
            self.selection = page
                .anchors
                .iter()
                .find(|a| a.source_offset == start)
                .zip(page.anchors.iter().find(|a| a.source_offset == end))
                .map(|(a, b)| (a.text_offset as usize, b.text_offset as usize));
        }
    }

    pub(in crate::worker) fn raster_loaded_with_handoff(
        &mut self,
        continuing: Option<[f64; 2]>,
        restored: Option<[f32; 2]>,
        raw: [f32; 2],
        same_context: bool,
        continuing_layout: bool,
        output: &Mailbox,
    ) {
        if let Some(mut handoff) = self.raster.handoff
            && matches!(handoff.phase, Phase::Reading { .. })
        {
            let prepared = restored.is_some() && self.rebase_camera_covered(raw);
            self.raster.reset_automatic(self.origin);
            self.restore_rebase_selection();
            handoff.phase = Phase::Reading { prepared };
            self.raster.handoff = Some(handoff);
            return;
        }
        if let Some(requested) = continuing
            && restored.is_some()
            && (!same_context || !self.retained_active())
        {
            let surface = ViewerSurfaceId(self.raster.surface);
            self.raster.transition = self
                .raster
                .transition
                .checked_add(1)
                .expect("raster transition identity exhausted");
            let transition = self.raster.transition;
            self.raster.reset_automatic(self.origin);
            self.restore_rebase_selection();
            self.raster.handoff = Some(Handoff {
                transition,
                surface,
                requested,
                restored: raw,
                phase: Phase::Waiting,
            });
            self.motion.begin_rebase(self.viewport_request, transition);
            output.put(ViewerOutput::RasterRebase {
                surface,
                transition,
            });
            return;
        }
        if continuing.is_some() && restored.is_none() {
            self.raster.continuing = continuing;
            self.abort_raster_refill(
                "The next text page could not preserve the displayed source position.",
                output,
            );
        }
        let acknowledgement = self.raster.rebase_ack.filter(|_| continuing_layout);
        self.raster_page_loaded(
            continuing.filter(|_| restored.is_some() && same_context),
            raw,
        );
        self.raster.rebase_ack = acknowledgement;
    }

    fn rebase_camera_covered(&self, origin: [f32; 2]) -> bool {
        self.page
            .as_ref()
            .zip(self.shaped.as_ref())
            .is_some_and(|(page, shaped)| {
                let maximum =
                    (crate::viewport::available_height(page, shaped, self.geometry.height as f32)
                        - self.geometry.height as f32)
                        .max(0.0);
                (origin[1] >= -0.001 || !page.before)
                    && (origin[1] <= maximum + 0.001 || !page.after)
            })
    }

    pub(in crate::worker) fn raster_rebase_input(
        &mut self,
        transition: u64,
        origin: [f64; 2],
        output: &Mailbox,
    ) {
        let Some(handoff) = self.raster.handoff.filter(|handoff| {
            handoff.transition == transition && matches!(handoff.phase, Phase::Waiting)
        }) else {
            return;
        };
        if origin
            .iter()
            .any(|value| !value.is_finite() || value.abs() > 1e12)
        {
            return;
        }
        let local: [f32; 2] = std::array::from_fn(|axis| {
            (f64::from(handoff.restored[axis]) + origin[axis] - handoff.requested[axis]) as f32
        });
        let (Some(page), Some(shaped)) = (&self.page, &self.shaped) else {
            self.abort_raster_rebase("The displayed text is no longer available.", output);
            return;
        };
        let maximum = [
            (shaped.width() - self.text_width()).max(0.0),
            (crate::viewport::available_height(page, shaped, self.geometry.height as f32)
                - self.geometry.height as f32)
                .max(0.0),
        ];
        let continuation = crate::viewport::Continuation::capture(
            page,
            shaped,
            self.geometry,
            [local[0], self.origin[1]],
            local[1] - self.origin[1],
        );
        if let Some(continuation) = continuation {
            let progresses = if continuation.backwards {
                continuation.anchor < page.end
            } else {
                continuation.anchor > page.start
            };
            if !progresses {
                self.abort_raster_rebase("The bounded text page cannot restore that displayed position. Try scrolling again.",output);
                return;
            }
            let has_session = self.session.is_some();
            self.request_page(continuation.anchor, continuation.backwards, output);
            self.continuation = Some(continuation);
            self.raster.handoff = Some(Handoff {
                phase: Phase::Reading { prepared: false },
                ..handoff
            });
            if has_session && self.viewport_request.is_none() {
                self.abort_raster_rebase("Cannot request the displayed text position.", output);
            }
            return;
        }
        if !self.rebase_camera_covered(local) {
            self.abort_raster_rebase("The bounded text page cannot restore that displayed position. Try scrolling again.",output);
            return;
        }
        self.origin = std::array::from_fn(|axis| local[axis].clamp(0.0, maximum[axis]));
        self.frames.invalidate();
        self.raster.reset_automatic(self.origin);
        self.restore_rebase_selection();
        self.acknowledge_raster_rebase(transition, output);
    }

    pub(in crate::worker) fn finish_raster_rebase(&mut self, completed: bool, output: &Mailbox) {
        let Some(handoff) = self.raster.handoff else {
            return;
        };
        let Phase::Reading { prepared } = handoff.phase else {
            return;
        };
        if !completed || !prepared {
            self.abort_raster_rebase(
                "The displayed text position was not completed. Try scrolling again.",
                output,
            );
            return;
        }
        self.raster.reset_automatic(self.origin);
        self.restore_rebase_selection();
        self.acknowledge_raster_rebase(handoff.transition, output);
    }

    fn acknowledge_raster_rebase(&mut self, transition: u64, output: &Mailbox) {
        self.raster.rebase_ack = Some(transition);
        self.dirty = true;
        self.resume_motion(MotionOwner::Rebase(transition), output);
    }

    pub(in crate::worker) fn abort_raster_rebase(
        &mut self,
        message: &str,
        output: &Mailbox,
    ) -> bool {
        let Some(handoff) = self.raster.handoff.take() else {
            return false;
        };
        self.motion.retire(MotionOwner::Rebase(handoff.transition));
        self.continuation = None;
        self.raster.reset_automatic(self.origin);
        self.restore_rebase_selection();
        self.dirty = true;
        self.status = message.to_owned();
        output.put(ViewerOutput::RasterUnavailable {
            surface: handoff.surface,
            message: message.into(),
        });
        true
    }

    pub(in crate::worker) fn raster_rebase_waiting(&self) -> bool {
        self.pending_raster_rebase().is_some()
    }

    pub(in crate::worker) fn pending_raster_rebase(&self) -> Option<u64> {
        self.raster.handoff.map(|handoff| handoff.transition)
    }

    pub(in crate::worker) fn raster_rebase_ack(&self) -> Option<u64> {
        self.raster.rebase_ack
    }

    pub(in crate::worker) fn reset_raster_for_scroll(&mut self) {
        // Raw input may have been queued before the corrected frame reached
        // the UI. Keep the receipt on replacement frames until a new context
        // or an input carrying the displayed surface makes it obsolete.
        let ack = self.raster.rebase_ack;
        self.reset_raster_context();
        self.raster.rebase_ack = ack;
    }

    pub(in crate::worker) fn cancel_raster_rebase(&mut self) {
        if let Some(handoff) = self.raster.handoff.take() {
            if let Some(id) = self.viewport_request.take() {
                if let Some(session) = &self.session {
                    session.cancel(id);
                }
                self.source_foreground_finished(id, false);
            }
            self.continuation = None;
            self.motion.retire(MotionOwner::Rebase(handoff.transition));
        }
        self.raster.rebase_ack = None;
    }

    pub(in crate::worker) fn raster_rebase_blocks(
        &mut self,
        surface: ViewerSurfaceId,
        input: &ViewerRasterInput,
    ) -> bool {
        if let Some(handoff) = self.raster.handoff {
            if surface == handoff.surface
                && matches!(
                    input,
                    ViewerRasterInput::Select { .. } | ViewerRasterInput::SelectPage { .. }
                )
            {
                self.raster.selection_valid = false;
            }
            return true;
        }
        false
    }

    /// With retained raster enabled, Copy returns the captured selection, even from a
    /// frame without raster: an automatic handoff keeps it although the new page may
    /// not show it, and every context reset recaptures the shown one. An invalid
    /// (rejected) selection copies nothing. Returns false when retained raster is off.
    pub(in crate::worker) fn copy_raster_selection(&self, output: &Mailbox) -> bool {
        if !self.raster.enabled {
            return false;
        }
        if self.raster.selection_valid
            && let Some(text) = &self.raster.selected_text
        {
            output.put(ViewerOutput::CopyText(text.clone()));
        }
        true
    }
}
