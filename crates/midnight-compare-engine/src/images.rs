use crate::{Frame, ImageMode, check_cancel};
use midnight_core::ViewerGeometry;
use midnight_image_viewer_host::{Frame as ImageFrame, ImageSession, Provider, Viewport};
use midnight_plugin_host::CapturedFile;
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::Duration,
};

pub(crate) struct Images {
    sessions: [ImageSession; 2],
    cache: Option<[ImageFrame; 2]>,
    pub mode: ImageMode,
    pub amount: f64,
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
            cache: None,
            mode: ImageMode::SideBySide,
            amount: 0.5,
            zoom: None,
            scale: 1.0,
            origin: [0.0; 2],
        })
    }
    pub fn invalidate(&mut self) {
        self.cache = None;
    }
    pub fn set_mode(&mut self, mode: ImageMode) {
        if self.mode != mode {
            let layout_changed =
                (self.mode == ImageMode::SideBySide) != (mode == ImageMode::SideBySide);
            self.mode = mode;
            if layout_changed {
                self.invalidate();
            }
        }
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
        let side_by_side = self.mode == ImageMode::SideBySide;
        let width = if side_by_side {
            geometry.width / 2
        } else {
            geometry.width
        };
        let metadata = self.sessions.each_ref().map(ImageSession::metadata);
        if self.cache.is_none() {
            let max_width = metadata.iter().map(|value| value.width).max().unwrap_or(1);
            let max_height = metadata.iter().map(|value| value.height).max().unwrap_or(1);
            self.scale = self
                .zoom
                .unwrap_or_else(|| {
                    (f64::from(width) / f64::from(max_width))
                        .min(f64::from(geometry.height) / f64::from(max_height))
                        .min(1.0)
                })
                .clamp(1.0 / 32768.0, 64.0);
            let viewport = Viewport {
                width,
                height: geometry.height,
                scale: self.scale,
                // Center the shared bounds, retaining exactly the same top-left origin
                // for both sources. A 50% swipe then intersects even small images.
                origin_x: self.origin[0]
                    - (f64::from(width) / self.scale - f64::from(max_width)) / 2.0,
                origin_y: self.origin[1]
                    - (f64::from(geometry.height) / self.scale - f64::from(max_height)) / 2.0,
            };
            let left = self.sessions[0]
                .render(viewport, cancel, Duration::from_secs(10))
                .map_err(|error| format!("Left image: {error}"))?;
            let right = self.sessions[1]
                .render(viewport, cancel, Duration::from_secs(10))
                .map_err(|error| format!("Right image: {error}"))?;
            self.cache = Some([left, right]);
        }
        check_cancel(cancel)?;
        let images = self
            .cache
            .as_ref()
            .ok_or("Image comparison frame unavailable.")?;
        let pixels = compose(images, geometry.width, self.mode, self.amount, cancel)?;
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
            geometry_revision: 0,
            mode: Some(self.mode),
            width: geometry.width,
            height: geometry.height,
            pixels,
            accessible_text: status.clone(),
            status,
            image: true,
            position: 0.0,
        })
    }
}

fn compose(
    images: &[ImageFrame; 2],
    width: u32,
    mode: ImageMode,
    amount: f64,
    cancel: &AtomicBool,
) -> Result<Vec<u8>, String> {
    let height = images[0].height;
    let mut pixels = vec![0; width as usize * height as usize * 4];
    let amount = amount.clamp(0.0, 1.0);
    for y in 0..height {
        check_cancel(cancel)?;
        for x in 0..width {
            let background = if (x / 12 + y / 12) % 2 == 0 { 190 } else { 230 };
            let sample = |side: usize, x: u32| {
                if x >= images[side].width {
                    return [background, background, background, 255];
                }
                let offset = (y as usize * images[side].width as usize + x as usize) * 4;
                let pixel = &images[side].rgba[offset..offset + 4];
                std::array::from_fn::<_, 4, _>(|channel| {
                    if channel == 3 {
                        255
                    } else {
                        ((u16::from(pixel[channel]) * u16::from(pixel[3])
                            + u16::from(background) * (255 - u16::from(pixel[3]))
                            + 127)
                            / 255) as u8
                    }
                })
            };
            let pixel = match mode {
                ImageMode::SideBySide => {
                    if x < width / 2 {
                        sample(0, x)
                    } else {
                        sample(1, x - width / 2)
                    }
                }
                ImageMode::Swipe => {
                    if f64::from(x) < f64::from(width) * amount {
                        sample(0, x)
                    } else {
                        sample(1, x)
                    }
                }
                ImageMode::OnionSkin => {
                    let left = sample(0, x);
                    let right = sample(1, x);
                    std::array::from_fn(|channel| {
                        (f64::from(left[channel]) * (1.0 - amount)
                            + f64::from(right[channel]) * amount)
                            .round() as u8
                    })
                }
            };
            let offset = (y as usize * width as usize + x as usize) * 4;
            pixels[offset..offset + 4].copy_from_slice(&pixel);
        }
    }
    Ok(pixels)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn frames() -> [ImageFrame; 2] {
        [
            ImageFrame {
                width: 2,
                height: 1,
                rgba: vec![255, 0, 0, 255, 255, 0, 0, 255],
            },
            ImageFrame {
                width: 2,
                height: 1,
                rgba: vec![0, 0, 255, 255, 0, 0, 255, 255],
            },
        ]
    }
    #[test]
    fn image_modes_and_endpoints_are_pixel_exact() {
        let cancelled = AtomicBool::new(false);
        assert_eq!(
            compose(&frames(), 2, ImageMode::Swipe, 0.5, &cancelled).unwrap(),
            [255, 0, 0, 255, 0, 0, 255, 255]
        );
        assert_eq!(
            &compose(&frames(), 2, ImageMode::OnionSkin, 0.5, &cancelled).unwrap()[..4],
            [128, 0, 128, 255]
        );
        assert_eq!(
            compose(&frames(), 2, ImageMode::OnionSkin, 0.0, &cancelled).unwrap(),
            frames()[0].rgba
        );
        assert_eq!(
            compose(&frames(), 2, ImageMode::OnionSkin, 1.0, &cancelled).unwrap(),
            frames()[1].rgba
        );
        assert_eq!(
            &compose(&frames(), 4, ImageMode::SideBySide, 0.5, &cancelled).unwrap()[8..12],
            [0, 0, 255, 255]
        );
    }
    #[test]
    fn transparent_pixels_use_checkerboard_and_cancel_stops_composition() {
        let mut images = frames();
        images[0].rgba[3] = 0;
        assert_eq!(
            &compose(
                &images,
                2,
                ImageMode::OnionSkin,
                0.0,
                &AtomicBool::new(false)
            )
            .unwrap()[..4],
            [190, 190, 190, 255]
        );
        assert!(compose(&images, 2, ImageMode::Swipe, 0.5, &AtomicBool::new(true)).is_err());
    }
}
