//! Scrollbar coordinates describe the scrollable visual range, not a raw file fraction.
use super::*;

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
        (origin >= -0.001 && origin <= f64::from(maximum) + 0.001)
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
        // For an unloaded fragment this remains a provisional byte estimate. Do not
        // report the requested fraction as displayed until actual content arrives.
        let extent = self.position_metrics().map_or(0.0, |(_, extent)| extent);
        let (anchor, backwards) = navigation::scrollbar_viewport(position, self.length, extent);
        self.request_page_for(anchor, backwards, Some(sequence), output);
        if self.viewport_request.is_none() {
            self.position_context(output);
            self.dirty = true;
        }
    }
}
