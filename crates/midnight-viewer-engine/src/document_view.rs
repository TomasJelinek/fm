//! App-private document canvas. Ordinary image camera policy remains in image_geometry.
use crate::image_geometry::ImageGeometry;
use midnight_core::{DocumentLayout, ViewerImageInput, ViewerZoomPreference};
use midnight_image_viewer_host::{DocumentMetadata, Metadata};
use std::sync::Arc;

const PAGE_GAP: f64 = 24.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct PagePlacement {
    pub origin: [f64; 2],
    pub size: [u32; 2],
}

impl PagePlacement {
    pub fn bottom(self) -> f64 {
        self.origin[1] + f64::from(self.size[1])
    }
}

#[derive(Clone, Copy)]
struct Published {
    frame: u64,
    layout: DocumentLayout,
    page: usize,
    viewport: [u32; 2],
    origin: [f64; 2],
    scale: f64,
}

#[derive(Clone, Copy)]
struct Anchor {
    page: usize,
    local: [f64; 2],
    viewport: [f64; 2],
}

pub(crate) struct DocumentView {
    pub layout: DocumentLayout,
    pub geometry: ImageGeometry,
    pub page: usize,
    pub placements: Arc<[PagePlacement]>,
    pub canvas: [f64; 2],
    published: Option<Published>,
    pending_page: Option<usize>,
}

impl DocumentView {
    pub fn new(pages: &[Metadata], viewport: [u32; 2]) -> Result<Self, String> {
        // The protocol validates this too. Keep the retained host geometry bounded independently.
        if pages.is_empty() || pages.len() > 4096 {
            return Err("Document page count exceeds its metadata limit.".into());
        }
        DocumentMetadata {
            pages: pages.to_vec(),
        }
        .validate()
        .map_err(|error| error.to_string())?;
        let width = pages
            .iter()
            .map(|page| f64::from(page.width))
            .fold(0.0, f64::max);
        let mut height = 0.0;
        let mut placements = Vec::with_capacity(pages.len());
        for page in pages {
            placements.push(PagePlacement {
                origin: [(width - f64::from(page.width)) / 2.0, height],
                size: [page.width, page.height],
            });
            height += f64::from(page.height) + PAGE_GAP;
            if !height.is_finite() {
                return Err("Document canvas exceeds its geometry limit.".into());
            }
        }
        height -= PAGE_GAP;
        let first = pages[0];
        Ok(Self {
            layout: DocumentLayout::Paged,
            geometry: ImageGeometry::document(first.width, first.height, viewport),
            page: 0,
            placements: placements.into(),
            canvas: [width, height],
            published: None,
            pending_page: None,
        })
    }

    /// A newly opened document in `layout` (the viewer preference chosen last), in Fit,
    /// showing the top of its first page.
    pub fn opened(
        pages: &[Metadata],
        viewport: [u32; 2],
        layout: DocumentLayout,
    ) -> Result<Self, String> {
        let mut view = Self::new(pages, viewport)?;
        if layout == DocumentLayout::Continuous {
            view.layout = layout;
            let size = view.placements[0].size;
            view.restore_anchor(Anchor {
                page: 0,
                local: [f64::from(size[0]) / 2.0, 0.0],
                viewport: [f64::from(view.geometry.viewport[0]) / 2.0, 0.0],
            });
        }
        Ok(view)
    }

    pub fn painted(&mut self, frame: u64) {
        self.published = Some(self.snapshot(frame));
        self.pending_page = None;
    }

    /// The scrollable extent: the current page in the Paged layout, every page in the
    /// Continuous one.
    pub fn extent(&self) -> [f64; 2] {
        if self.layout == DocumentLayout::Paged {
            self.placements[self.page].size.map(f64::from)
        } else {
            self.canvas
        }
    }

    /// The tile canvas of the current layout.
    pub fn tile_canvas(&self) -> crate::image_scene::Canvas {
        if self.layout == DocumentLayout::Paged {
            let size = self.placements[self.page].size;
            crate::image_scene::Canvas {
                key: self.page,
                pages: Arc::from([PagePlacement {
                    origin: [0.0; 2],
                    size,
                }]),
                extent: size.map(f64::from),
            }
        } else {
            crate::image_scene::Canvas {
                key: crate::image_tiles::CANVAS,
                pages: self.placements.clone(),
                extent: self.canvas,
            }
        }
    }

    pub fn set_layout(&mut self, frame: u64, layout: DocumentLayout) -> bool {
        let Some(published) = self
            .published
            .filter(|value| value.frame == frame && frame != 0)
        else {
            return false;
        };
        if self.layout == layout {
            return false;
        }
        let anchor = self.anchor(
            published,
            published.viewport.map(|edge| f64::from(edge) / 2.0),
        );
        self.layout = layout;
        self.pending_page = None;
        self.restore_anchor(anchor);
        true
    }

    pub fn resize(&mut self, viewport: [u32; 2]) {
        let mut anchor = self.refit_anchor();
        self.geometry.viewport = viewport.map(|edge| edge.max(1));
        anchor.viewport = self.geometry.center();
        self.restore_anchor(anchor);
    }

    pub fn change_page(&mut self, delta: i32) -> bool {
        let page = self
            .page
            .saturating_add_signed(delta as isize)
            .min(self.placements.len() - 1);
        if page == self.page {
            return false;
        }
        let before = (self.page, self.geometry.clone());
        self.page = page;
        let size = self.placements[page].size;
        if self.layout == DocumentLayout::Paged {
            self.geometry.change_page(size[0], size[1]);
        } else {
            self.restore_anchor(Anchor {
                page,
                local: size.map(|edge| f64::from(edge) / 2.0),
                viewport: self.geometry.center(),
            });
        }
        let changed = self.page != before.0 || self.geometry != before.1;
        if changed {
            self.pending_page = Some(page);
        }
        changed
    }

    pub fn image_input(&mut self, input: &ViewerImageInput) -> bool {
        match input {
            ViewerImageInput::ViewZoom {
                zoom,
                source_anchor,
                viewport_anchor,
            } => {
                if source_anchor.iter().any(|value| !value.is_finite())
                    || viewport_anchor.iter().any(|value| !value.is_finite())
                {
                    return false;
                }
                let published = self.published.unwrap_or_else(|| self.snapshot(0));
                let anchor =
                    self.source_anchor(published, *source_anchor, viewport_anchor.map(f64::from));
                let accepted = self.zoom_to(*zoom, anchor);
                if accepted {
                    self.pending_page = None;
                }
                accepted
            }
            ViewerImageInput::Fit => self.zoom_to(ViewerZoomPreference::Fit, self.refit_anchor()),
            ViewerImageInput::ActualPixels => {
                let accepted =
                    self.zoom_to(ViewerZoomPreference::ActualPixels, self.center_anchor());
                if accepted {
                    self.pending_page = None;
                }
                accepted
            }
            ViewerImageInput::Zoom { factor, anchor } => {
                if !factor.is_finite() || *factor <= 0.0 {
                    return false;
                }
                let published = self.published.unwrap_or_else(|| self.snapshot(0));
                let viewport = anchor.map_or_else(
                    || published.viewport.map(|edge| f64::from(edge) / 2.0),
                    |anchor| anchor.map(f64::from),
                );
                if viewport.iter().any(|value| !value.is_finite()) {
                    return false;
                }
                let anchor = self.anchor(published, viewport);
                self.geometry.scale = self.clamp_scale(self.geometry.scale * factor);
                self.geometry.fitted = false;
                self.restore_anchor(anchor);
                self.pending_page = None;
                true
            }
            ViewerImageInput::Drag { x, y } => {
                self.scroll([-f64::from(*x), -f64::from(*y)]);
                true
            }
            // Untiled document scrolling uses backend Scroll/Seek/Pan, never retained-image pans.
            ViewerImageInput::PanTo { .. } => false,
        }
    }

    pub fn scroll(&mut self, delta: [f64; 2]) {
        if self.layout == DocumentLayout::Paged {
            self.geometry.scroll(delta);
            return;
        }
        for (axis, delta) in delta.into_iter().enumerate() {
            if delta.is_finite() {
                self.geometry.origin[axis] += delta / self.geometry.scale;
            }
        }
        self.clamp_origin();
        self.update_page();
    }

    pub fn seek(&mut self, axis: usize, position: f64) {
        if axis >= 2 || !position.is_finite() {
            return;
        }
        if self.layout == DocumentLayout::Paged {
            self.geometry.seek(axis, position);
            return;
        }
        let mut origin = self.geometry.origin;
        origin[axis] = position.clamp(0.0, 1.0) * self.maximum(axis).max(0.0);
        self.pan_to(origin);
    }

    pub fn pan_to(&mut self, origin: [f64; 2]) {
        if origin.iter().any(|value| !value.is_finite()) {
            return;
        }
        if self.layout == DocumentLayout::Paged {
            self.geometry.pan_to(origin);
        } else {
            self.geometry.origin = origin;
            self.clamp_origin();
            self.update_page();
        }
    }

    pub fn metrics(&self, axis: usize) -> (f64, f64) {
        if self.layout == DocumentLayout::Paged || axis >= 2 {
            return self.geometry.metrics(axis);
        }
        let maximum = self.maximum(axis);
        if maximum <= 0.0 {
            return (0.0, 1.0);
        }
        (
            (self.geometry.origin[axis] / maximum).clamp(0.0, 1.0),
            (f64::from(self.geometry.viewport[axis]) / (self.canvas[axis] * self.geometry.scale))
                .clamp(0.0, 1.0),
        )
    }

    fn zoom_to(&mut self, zoom: ViewerZoomPreference, anchor: Anchor) -> bool {
        if zoom == ViewerZoomPreference::Fit {
            self.geometry.fitted = true;
        } else {
            let Some(scale) = zoom
                .scale()
                .filter(|scale| (1.0 / 32768.0..=64.0).contains(scale))
            else {
                return false;
            };
            self.geometry.scale = self.clamp_scale(scale);
            self.geometry.fitted = false;
        }
        self.restore_anchor(anchor);
        true
    }

    fn restore_anchor(&mut self, anchor: Anchor) {
        self.page = anchor.page;
        let placement = self.placements[self.page];
        [self.geometry.width, self.geometry.height] = placement.size;
        if self.geometry.fitted {
            self.geometry.fit();
            // Paged Fit preserves the existing centered single-page behavior.
            if self.layout == DocumentLayout::Paged {
                return;
            }
        }
        let placement_origin = if self.layout == DocumentLayout::Paged {
            [0.0; 2]
        } else {
            placement.origin
        };
        for (axis, placement_origin) in placement_origin.into_iter().enumerate() {
            self.geometry.origin[axis] = placement_origin + anchor.local[axis]
                - anchor.viewport[axis].clamp(0.0, f64::from(self.geometry.viewport[axis]))
                    / self.geometry.scale;
        }
        self.clamp_origin();
        self.update_page();
    }

    fn center_anchor(&self) -> Anchor {
        let published = self.published.unwrap_or_else(|| self.snapshot(0));
        let mut anchor = self.anchor(
            published,
            published.viewport.map(|edge| f64::from(edge) / 2.0),
        );
        anchor.viewport = self.geometry.center();
        anchor
    }

    fn refit_anchor(&self) -> Anchor {
        let Some(page) = self.pending_page else {
            return self.center_anchor();
        };
        // An accepted page jump survives resize/refit until its replacement is published.
        // Coordinate-bearing zoom/layout inputs continue to use the published camera above.
        let viewport = self.geometry.center();
        let offset = if self.layout == DocumentLayout::Paged {
            [0.0; 2]
        } else {
            self.placements[page].origin
        };
        Anchor {
            page,
            local: std::array::from_fn(|axis| {
                self.geometry.origin[axis] + viewport[axis] / self.geometry.scale - offset[axis]
            }),
            viewport,
        }
    }

    fn anchor(&self, camera: Published, viewport: [f64; 2]) -> Anchor {
        let viewport =
            std::array::from_fn(|axis| viewport[axis].clamp(0.0, f64::from(camera.viewport[axis])));
        self.source_anchor(
            camera,
            std::array::from_fn(|axis| camera.origin[axis] + viewport[axis] / camera.scale),
            viewport,
        )
    }

    fn source_anchor(&self, camera: Published, source: [f64; 2], viewport: [f64; 2]) -> Anchor {
        let page = if camera.layout == DocumentLayout::Paged {
            camera.page
        } else {
            self.page_at(source[1])
        };
        let offset = if camera.layout == DocumentLayout::Paged {
            [0.0; 2]
        } else {
            self.placements[page].origin
        };
        Anchor {
            page,
            local: std::array::from_fn(|axis| source[axis] - offset[axis]),
            viewport,
        }
    }

    fn snapshot(&self, frame: u64) -> Published {
        Published {
            frame,
            layout: self.layout,
            page: self.page,
            viewport: self.geometry.viewport,
            origin: self.geometry.origin,
            scale: self.geometry.scale,
        }
    }

    fn page_at(&self, y: f64) -> usize {
        let next = self.placements.partition_point(|page| page.bottom() < y);
        if next == self.placements.len() {
            return next - 1;
        }
        if next > 0 && y < self.placements[next].origin[1] {
            let previous_distance = y - self.placements[next - 1].bottom();
            let next_distance = self.placements[next].origin[1] - y;
            if previous_distance <= next_distance {
                return next - 1;
            }
        }
        next
    }

    fn update_page(&mut self) {
        if self.layout == DocumentLayout::Continuous {
            self.page = self.page_at(
                self.geometry.origin[1]
                    + f64::from(self.geometry.viewport[1]) / (2.0 * self.geometry.scale),
            );
            [self.geometry.width, self.geometry.height] = self.placements[self.page].size;
        }
    }

    fn clamp_scale(&self, scale: f64) -> f64 {
        let minimum = 1.0 / (32768.0 * self.geometry.raster_geometry().1);
        scale.clamp(minimum.min(64.0), 64.0)
    }

    fn maximum(&self, axis: usize) -> f64 {
        let dimension = if self.layout == DocumentLayout::Paged {
            f64::from(self.placements[self.page].size[axis])
        } else {
            self.canvas[axis]
        };
        dimension - f64::from(self.geometry.viewport[axis]) / self.geometry.scale
    }

    fn clamp_origin(&mut self) {
        for axis in 0..2 {
            let maximum = self.maximum(axis);
            self.geometry.origin[axis] = if maximum <= 0.0 {
                maximum / 2.0
            } else {
                self.geometry.origin[axis].clamp(0.0, maximum)
            };
        }
    }
}

#[cfg(test)]
#[path = "document_view_tests.rs"]
mod tests;
