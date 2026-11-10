//! Retain the latest accepted zoom for the current source, page and viewport.
use midnight_core::{ViewerImageInfo, ViewerZoomPreference};

#[derive(Default)]
pub(crate) struct Intent(Option<Pending>);

struct Pending {
    generation: u64,
    page: u32,
    viewport: [f32; 2],
    zoom: ViewerZoomPreference,
}

impl Intent {
    pub fn clear(&mut self) {
        self.0 = None;
    }

    pub fn scale(&self, generation: u64, info: &ViewerImageInfo, viewport: [f32; 2]) -> f64 {
        let Some(pending) = self.0.as_ref().filter(|pending| {
            pending.generation == generation
                && pending.page == info.page
                && pending.viewport == viewport
        }) else {
            return info.zoom;
        };
        pending.zoom.scale().unwrap_or_else(|| {
            (f64::from(viewport[0]) / f64::from(info.width))
                .min(f64::from(viewport[1]) / f64::from(info.height))
                .min(if info.page > 0 { 64.0 } else { 1.0 })
        })
    }

    pub fn accepted(
        &mut self,
        generation: u64,
        info: &ViewerImageInfo,
        viewport: [f32; 2],
        zoom: ViewerZoomPreference,
    ) {
        self.0 = Some(Pending {
            generation,
            page: info.page,
            viewport,
            zoom,
        });
    }

    pub fn receive(&mut self, generation: u64, info: Option<&ViewerImageInfo>) {
        let Some(pending) = &self.0 else {
            return;
        };
        let Some(info) = info else {
            return self.clear();
        };
        // Numeric scale equality is not a command acknowledgment: an older pan
        // frame can happen to match a later zoom-back request. Keep the intent
        // until its context changes or the presenter explicitly cancels it.
        if pending.generation != generation || pending.page != info.page {
            self.clear();
        }
    }
}
