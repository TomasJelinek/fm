use crate::{image_geometry::ImageGeometry, image_scene::Scene, image_worker::ImageWorker};
use midnight_core::{ViewerSurfaceId, ViewerTile};
use std::sync::{Arc, atomic::AtomicU64};

pub(crate) enum Paint {
    Tiles(Vec<ViewerTile>),
    Raster(midnight_image_viewer_host::Frame),
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
    pub scene: Scene,
    pub validated: Option<u64>,
    published: Option<(u64, usize)>,
    failure: Option<String>,
    pub waiting_budget: bool,
    pub document_frame: Option<(u64, midnight_image_viewer_host::Frame)>,
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
            scene: Scene::new(ids),
            validated: None,
            published: None,
            failure: None,
            waiting_budget: false,
            document_frame: None,
        }
    }

    pub fn changed(&mut self) {
        match self.revision.checked_add(1) {
            Some(revision) => self.revision = revision,
            None => self.failure = Some("Image camera identity exhausted.".into()),
        }
        if let Some(geometry) = &self.geometry {
            let result = if self.document {
                self.scene.update_document(self.page, geometry)
            } else {
                self.scene.update(self.page, geometry)
            };
            if let Err(error) = result {
                self.failure = Some(error);
            }
        }
    }

    pub fn pan_to(&mut self, surface: ViewerSurfaceId, sequence: u64, origin: [f64; 2]) -> bool {
        let Some(geometry) = &mut self.geometry else {
            return false;
        };
        self.scene.accept_pan(geometry, surface, sequence, origin)
    }

    /// Request only the next missing tile. No source/decode operation runs here.
    pub fn request(&mut self) -> Result<Option<Paint>, String> {
        if let Some(error) = &self.failure {
            return Err(error.clone());
        }
        if self.geometry.is_none() {
            return Ok(None);
        }
        if self.document {
            return Ok(self.request_document());
        }
        self.waiting_budget = false;
        if self.pending.is_none() {
            if let Some((key, charge)) = self.scene.next()? {
                if self.worker.render(self.revision, key, charge) {
                    self.pending = Some(self.revision);
                }
            } else if self.scene.needs_tiles() {
                self.waiting_budget = true;
            } else if self.validated != Some(self.revision) && self.worker.validate(self.revision) {
                self.pending = Some(self.revision);
            }
        }
        if self.validated != Some(self.revision) {
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

    fn request_document(&mut self) -> Option<Paint> {
        if let Some((revision, frame)) = self.document_frame.take()
            && revision == self.revision
        {
            self.published = Some((revision, 0));
            return Some(Paint::Raster(frame));
        }
        if self.pending.is_none() && self.published != Some((self.revision, 0)) {
            let geometry = self.geometry.as_ref()?;
            let (size, density) = geometry.raster_geometry();
            if self.worker.document(
                self.revision,
                self.page,
                midnight_image_viewer_host::Viewport {
                    width: size[0],
                    height: size[1],
                    scale: geometry.scale * density,
                    origin_x: geometry.origin[0],
                    origin_y: geometry.origin[1],
                },
            ) {
                self.pending = Some(self.revision);
            }
        }
        None
    }
}
