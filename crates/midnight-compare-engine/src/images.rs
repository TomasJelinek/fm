use crate::{
    Frame, ImageMode, check_cancel,
    image_cache::{LayoutCache, LayoutRequest},
};
use midnight_core::ViewerGeometry;
use midnight_image_viewer_host::{ImageSession, Provider, Source};
use midnight_plugin_host::CapturedFile;
use std::{
    io,
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub(crate) struct Images {
    sessions: [ImageSession; 2],
    cache: LayoutCache,
    pub mode: ImageMode,
    zoom: Option<f64>,
    scale: f64,
    origin: [f64; 2],
}

/// A captured comparison file with its positively established storage hint, so the
/// decoder's source reads use the same shared-I/O admission as other readers.
struct StorageSource {
    file: CapturedFile,
    confirmed_ssd: bool,
}
impl Source for StorageSource {
    fn length(&self) -> u64 {
        Source::length(&self.file)
    }
    fn version(&self) -> Vec<u8> {
        Source::version(&self.file)
    }
    fn validate(&self) -> io::Result<()> {
        Source::validate(&self.file)
    }
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        Source::read_at(&self.file, offset, length)
    }
    fn confirmed_local_ssd(&self) -> bool {
        self.confirmed_ssd
    }
}

impl Images {
    pub fn open(
        providers: [&Provider; 2],
        sources: &[CapturedFile; 2],
        confirmed_ssd: [bool; 2],
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        let source = |side: usize| -> Arc<dyn Source> {
            Arc::new(StorageSource {
                file: sources[side].clone(),
                confirmed_ssd: confirmed_ssd[side],
            })
        };
        let left = ImageSession::open(providers[0], source(0), cancel, Duration::from_secs(15))
            .map_err(|error| format!("Left image: {error}"))?;
        check_cancel(cancel)?;
        let right = ImageSession::open(providers[1], source(1), cancel, Duration::from_secs(15))
            .map_err(|error| format!("Right image: {error}"))?;
        Ok(Self {
            sessions: [left, right],
            cache: LayoutCache::default(),
            mode: ImageMode::SideBySide,
            zoom: None,
            scale: 1.0,
            origin: [0.0; 2],
        })
    }
    pub fn invalidate(&mut self) {
        self.cache.clear();
    }
    pub fn set_mode(&mut self, mode: ImageMode) {
        self.mode = mode;
    }
    pub fn zoom(&mut self, factor: f64) {
        if factor.is_finite() && factor > 0.0 {
            self.zoom = Some((self.scale * factor).clamp(1.0 / 32768.0, 64.0));
            self.invalidate();
        }
    }
    pub fn fit(&mut self) {
        self.zoom = None;
        self.origin = [0.0; 2];
        self.invalidate();
    }
    pub fn actual(&mut self) {
        self.zoom = Some(1.0);
        self.invalidate();
    }
    pub fn pan(&mut self, x: f32, y: f32) {
        if x.is_finite() && y.is_finite() {
            self.origin[0] = (self.origin[0] - f64::from(x) / self.scale).clamp(-32768.0, 32768.0);
            self.origin[1] = (self.origin[1] - f64::from(y) / self.scale).clamp(-32768.0, 32768.0);
            self.invalidate();
        }
    }
    pub fn paint(
        &mut self,
        geometry: ViewerGeometry,
        cancel: &AtomicBool,
    ) -> Result<Frame, String> {
        let metadata = self.sessions.each_ref().map(ImageSession::metadata);
        let layout = self.cache.get_or_render(
            LayoutRequest {
                geometry,
                mode: self.mode,
                zoom: self.zoom,
                origin: self.origin,
                bounds: [
                    metadata.iter().map(|value| value.width).max().unwrap_or(1),
                    metadata.iter().map(|value| value.height).max().unwrap_or(1),
                ],
            },
            cancel,
            |side, viewport| {
                self.sessions[side]
                    .render(viewport, cancel, Duration::from_secs(10))
                    .map_err(|error| {
                        format!(
                            "{} image: {error}",
                            if side == 0 { "Left" } else { "Right" }
                        )
                    })
            },
        )?;
        self.scale = layout.scale;
        check_cancel(cancel)?;
        let gesture = if self.mode == ImageMode::Swipe {
            "drag divider"
        } else {
            "drag to pan"
        };
        let status = format!(
            "Left {} × {} · Right {} × {} · {:.0}% · first still frame · {gesture}",
            metadata[0].width,
            metadata[0].height,
            metadata[1].width,
            metadata[1].height,
            self.scale * 100.0
        );
        Ok(Frame {
            fonts: None,
            #[cfg(test)]
            text_family: None,
            geometry_revision: 0,
            comparison_generation: 0,
            identity: crate::IdentityVerdict::Pending,
            mode: Some(self.mode),
            width: geometry.width,
            height: geometry.height,
            pixels: Vec::new(),
            layers: Some(layout.layers),
            accessible_text: status.clone(),
            status,
            image: true,
            position: 0.0,
        })
    }
}
