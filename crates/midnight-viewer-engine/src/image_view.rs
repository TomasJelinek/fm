use crate::{
    document_render::BandPlan, document_view::DocumentView, image_geometry::ImageGeometry,
    image_scene::Scene, image_worker::ImageWorker,
};
use midnight_core::{ViewerSurfaceId, ViewerTile};
use std::sync::{Arc, atomic::AtomicU64};

pub(crate) enum Paint {
    Tiles(Vec<ViewerTile>),
}

pub(crate) struct ImageView {
    pub worker: ImageWorker,
    pub geometry: Option<ImageGeometry>,
    pub revision: u64,
    pub pending: Option<u64>,
    pub frames: crate::image_input::ImageFrames,
    pub pages: Vec<midnight_image_viewer_host::Metadata>,
    pub page: usize,
    pub document: bool,
    pub document_view: Option<DocumentView>,
    pub scene: Scene,
    pub validated: Option<u64>,
    /// The revision of the latest document camera change other than a pan (a new surface).
    /// A source validation requested at or after it covers every pan of that surface.
    surface_revision: u64,
    published: Option<(u64, usize)>,
    failure: Option<String>,
    pub waiting_budget: bool,
    /// A document camera the UI asked for while the visible tiles of the camera in
    /// progress were still rendering: surface, sequence and origin. It is applied once
    /// those tiles are published, so a fast scrollbar drag shows the cameras it passes
    /// instead of chasing every new one and showing none.
    pub deferred_pan: Option<(ViewerSurfaceId, u64, [f64; 2])>,
    /// Relative page steps made while the visible tiles of the camera in progress were
    /// still rendering, summed and clamped to the document's pages. They turn the page
    /// once that camera is published, so a held page key shows the pages it passes
    /// instead of chasing every step and showing none. At most the page count.
    deferred_pages: i32,
}

impl ImageView {
    pub fn new(worker: ImageWorker, ids: Arc<AtomicU64>) -> Self {
        Self {
            worker,
            geometry: None,
            revision: 0,
            pending: None,
            frames: Default::default(),
            pages: Vec::new(),
            page: 0,
            document: false,
            document_view: None,
            scene: Scene::new(ids),
            validated: None,
            surface_revision: 0,
            published: None,
            failure: None,
            waiting_budget: false,
            deferred_pan: None,
            deferred_pages: 0,
        }
    }

    /// A camera change the UI did not choose itself (anything but a pan of its retained
    /// tiles): a document gets a new surface, so the UI adopts this camera.
    pub fn changed(&mut self) {
        self.changed_by(false);
    }

    /// `pan`: the UI's tiled camera asked for this camera (`ViewerImageInput::PanTo`).
    pub fn changed_by(&mut self, pan: bool) {
        match self.revision.checked_add(1) {
            Some(revision) => self.revision = revision,
            None => self.failure = Some("Image camera identity exhausted.".into()),
        }
        if !pan {
            self.surface_revision = self.revision;
        }
        if let Some(document) = &self.document_view {
            self.geometry = Some(document.geometry.clone());
            self.page = document.page;
            if let Err(error) =
                self.scene
                    .update_document(&document.tile_canvas(), &document.geometry, !pan)
            {
                self.failure = Some(error);
            }
            return;
        }
        if let Some(geometry) = &self.geometry {
            let result = if self.document {
                Err("Document camera without its pages.".into())
            } else {
                self.scene.update(self.page, geometry)
            };
            if let Err(error) = result {
                self.failure = Some(error);
            }
        }
    }

    /// The UI's tiled camera asked for a document camera (`PanTo`). True when the camera
    /// moved now. While the visible tiles of the camera in progress are still rendering,
    /// the target waits (`deferred_pan`) until `published`; older sequences, replaced
    /// surfaces and nonfinite origins are refused.
    pub fn pan_document(
        &mut self,
        surface: ViewerSurfaceId,
        sequence: u64,
        origin: [f64; 2],
    ) -> bool {
        let latest = self
            .deferred_pan
            .filter(|(deferred, ..)| *deferred == self.scene.surface)
            .map_or(self.scene.sequence, |(_, sequence, _)| sequence);
        if surface != self.scene.surface
            || sequence <= latest
            || origin.iter().any(|value| !value.is_finite())
            || self.document_view.is_none()
        {
            return false;
        }
        if self.pending.is_some() && !self.scene.visible_ready() {
            self.deferred_pan = Some((surface, sequence, origin));
            return false;
        }
        self.deferred_pan = None;
        self.apply_pan(surface, sequence, origin)
    }

    /// A relative document page step (Ctrl+PageDown/Up, the page buttons). Steps add up
    /// against the latest requested page, not the page shown, clamped to the document: n
    /// steps move n pages and never past the first or the last page, and no step waits in
    /// a queue. While the visible tiles of the camera in progress are still rendering, the
    /// step waits with those before it (`deferred_pages`) until that camera is published.
    /// True when the page changed now (`changed_by` already ran).
    pub fn page_step(&mut self, delta: i32) -> bool {
        let Some(document) = &self.document_view else {
            return false;
        };
        let page = document.page as i64;
        let last = document.placements.len() as i64 - 1;
        let target = (page + i64::from(self.deferred_pages) + i64::from(delta)).clamp(0, last);
        // `DocumentView` holds at most 4096 pages, so the difference fits.
        self.deferred_pages = (target - page) as i32;
        // Wait only for a camera that is still to be shown: its publication turns the page.
        let unpublished = self
            .published
            .is_none_or(|(revision, _)| revision != self.revision);
        if unpublished && self.pending.is_some() && !self.scene.visible_ready() {
            return false;
        }
        self.turn_deferred_pages()
    }

    /// Turn the page by the steps that waited for the camera in progress, if any. Another
    /// camera input calls this first, so that it applies to the page the steps reach,
    /// as it would had they not waited. True when the page changed (`changed_by` ran).
    pub fn turn_deferred_pages(&mut self) -> bool {
        let delta = std::mem::take(&mut self.deferred_pages);
        let Some(document) = &mut self.document_view else {
            return false;
        };
        if delta == 0 || !document.change_page(delta) {
            return false;
        }
        // A pan of the old page's surface is moot: the page gets a new surface.
        self.deferred_pan = None;
        self.frames.page_turned();
        self.changed();
        true
    }

    /// A frame was published: move on to the camera that waited for it, if any. True when
    /// the camera changed (`changed_by` already ran).
    pub fn published(&mut self) -> bool {
        if self.turn_deferred_pages() {
            return true;
        }
        let Some((surface, sequence, origin)) = self.deferred_pan.take() else {
            return false;
        };
        if !self.apply_pan(surface, sequence, origin) {
            return false;
        }
        self.changed_by(true);
        true
    }

    fn apply_pan(&mut self, surface: ViewerSurfaceId, sequence: u64, origin: [f64; 2]) -> bool {
        let Some(document) = &mut self.document_view else {
            return false;
        };
        if !self.scene.accept_pan_sequence(surface, sequence, origin) {
            return false;
        }
        document.pan_to(origin);
        true
    }

    pub fn pan_to(&mut self, surface: ViewerSurfaceId, sequence: u64, origin: [f64; 2]) -> bool {
        let Some(geometry) = &mut self.geometry else {
            return false;
        };
        self.scene.accept_pan(geometry, surface, sequence, origin)
    }

    /// Request only the next missing tiles: one image tile or one band of document tiles.
    /// No source/decode operation runs here.
    pub fn request(&mut self) -> Result<Option<Paint>, String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if self.geometry.is_none() {
            return Ok(None);
        }
        self.waiting_budget = false;
        if self.pending.is_none() {
            if self.document {
                self.request_band()?;
            } else if let Some((key, charge)) = self.scene.next()? {
                if self.worker.render(self.revision, key, charge) {
                    self.pending = Some(self.revision);
                }
            } else if self.scene.needs_tiles() {
                self.waiting_budget = true;
            } else if self.validated != Some(self.revision) && self.worker.validate(self.revision) {
                self.pending = Some(self.revision);
            }
        }
        // A document camera reached by a pan shows tiles of a surface whose source a band or
        // a validation already proved current (the UI shows the same retained tiles without
        // asking): it is published without waiting for its own source check, which follows
        // when the actor is idle. Every other camera waits for its validation.
        let validated = self.validated.is_some_and(|validated| {
            validated == self.revision || (self.document && validated >= self.surface_revision)
        });
        if !validated {
            return Ok(None);
        }
        let Some(tiles) = self.scene.tiles() else {
            return Ok(None);
        };
        let signature = (self.revision, tiles.len());
        if self.published == Some(signature) {
            return Ok(None);
        }
        self.published = Some(signature);
        Ok(Some(Paint::Tiles(tiles)))
    }

    /// One band of missing document tiles, visible ones first. While the actor renders it,
    /// newer cameras only move the plan; whatever the band completes that the latest plan
    /// still needs is kept (`Scene::complete_band`).
    fn request_band(&mut self) -> Result<(), String> {
        let Some(document) = &self.document_view else {
            return Ok(());
        };
        if let Some(band) = self.scene.next_band()? {
            let plan = BandPlan::new(document, band)?;
            if self.worker.band(self.revision, plan) {
                self.pending = Some(self.revision);
            }
        } else if self.scene.needs_tiles() {
            self.waiting_budget = true;
        } else if self.validated != Some(self.revision) && self.worker.validate(self.revision) {
            self.pending = Some(self.revision);
        }
        Ok(())
    }
}
