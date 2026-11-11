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
    fn position_metrics(&self) -> Option<(f64, f64)> {
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
            return;
        }
        if let Some(id) = self.search_request.take()
            && let Some(session) = &self.session
        {
            session.cancel(id);
        }
        // Inspect read authority before reset can cancel a speculative correction.
        if let Some(origin) = self.cached_position(position) {
            self.origin[1] = origin;
            // Capture the new origin so completion cannot resume an older camera.
            self.reset_raster_context();
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
                ((page.end - page.start) as f64 * fraction).ceil() as u64
            });
        (anchor.saturating_sub(before), false)
    }

    /// A newly decoded fragment has its own byte-to-height estimate. Resolve the
    /// same absolute intent in that layout before acknowledging it, retaining a
    /// small prefix for density changes. At most one correction read is allowed.
    pub(super) fn refine_position(&mut self) -> LoadedPosition {
        let Some(target) = self.positions.target else {
            return LoadedPosition::Ready;
        };
        // The already queued latest intent owns the next request; do not spend
        // another read refining a superseded intermediate target.
        if self.positions.has_deferred() {
            return LoadedPosition::Ready;
        }
        if let Some(origin) = self.loaded_position(target.position) {
            self.origin[1] = origin;
            self.reset_raster_context();
            self.frames.invalidate();
            self.dirty = true;
            return LoadedPosition::Ready;
        }
        if target.corrected {
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
            return LoadedPosition::Unavailable;
        }
        self.positions.target = Some(crate::position_ack::Target {
            corrected: true,
            ..target
        });
        LoadedPosition::Reload {
            sequence: target.sequence,
            anchor,
            backwards,
        }
    }
}
