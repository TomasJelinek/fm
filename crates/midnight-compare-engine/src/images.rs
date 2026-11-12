use crate::{
    Frame, ImageMode, check_cancel,
    image_cache::{LayoutCache, LayoutRequest},
};
use midnight_core::ViewerGeometry;
use midnight_image_viewer_host::{ImageSession, Provider};
use midnight_plugin_host::CapturedFile;
use std::{
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

impl Images {
    pub fn open(
        providers: [&Provider; 2],
        sources: &[CapturedFile; 2],
        cancel: &AtomicBool,
    ) -> Result<Self, String> {
        let left = ImageSession::open(
            providers[0],
            Arc::new(sources[0].clone()),
            cancel,
            Duration::from_secs(15),
        )
        .map_err(|error| format!("Left image: {error}"))?;
        check_cancel(cancel)?;
        let right = ImageSession::open(
            providers[1],
            Arc::new(sources[1].clone()),
            cancel,
            Duration::from_secs(15),
        )
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
            geometry_revision: 0,
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
