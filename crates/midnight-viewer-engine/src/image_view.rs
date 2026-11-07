use crate::{image_geometry::ImageGeometry, image_worker::ImageWorker};

pub(crate) struct ImageView {
    pub worker: ImageWorker,
    pub geometry: Option<ImageGeometry>,
    pub revision: u64,
    pub pending: Option<u64>,
    pub frames: crate::image_input::ImageFrames,
    pub pages: Vec<midnight_image_viewer_host::Metadata>,
    pub page: usize,
    pub document: bool,
}

impl ImageView {
    pub fn new(worker: ImageWorker) -> Self {
        Self {
            worker,
            geometry: None,
            revision: 0,
            pending: None,
            frames: Default::default(),
            pages: Vec::new(),
            page: 0,
            document: false,
        }
    }

    pub fn changed(&mut self) {
        self.revision = self.revision.saturating_add(1);
    }

    pub fn request(&mut self) -> bool {
        let Some(geometry) = &self.geometry else {
            return false;
        };
        if self.pending.is_some() {
            return false;
        }
        let (surface, density) = geometry.raster_geometry();
        if self.worker.render(
            self.revision,
            self.page,
            midnight_image_viewer_host::Viewport {
                width: surface[0],
                height: surface[1],
                scale: geometry.scale * density,
                origin_x: geometry.origin[0],
                origin_y: geometry.origin[1],
            },
        ) {
            self.pending = Some(self.revision);
            true
        } else {
            false
        }
    }
}
