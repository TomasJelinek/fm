//! Two bounded sampled layouts; decoded sources remain in their restricted workers.
use crate::{ImageLayers, ImageMode, check_cancel};
use midnight_core::ViewerGeometry;
use midnight_image_viewer_host::{Frame, Viewport};
use std::sync::{Arc, atomic::AtomicBool};

#[derive(Clone)]
pub(crate) struct Layout {
    pub(crate) layers: ImageLayers,
    pub(crate) scale: f64,
}

#[derive(Default)]
pub(crate) struct LayoutCache([Option<Layout>; 2]);

pub(crate) struct LayoutRequest {
    pub(crate) geometry: ViewerGeometry,
    pub(crate) mode: ImageMode,
    pub(crate) zoom: Option<f64>,
    pub(crate) origin: [f64; 2],
    pub(crate) bounds: [u32; 2],
}

impl LayoutCache {
    pub(crate) fn clear(&mut self) {
        self.0 = Default::default();
    }

    pub(crate) fn get_or_render(
        &mut self,
        request: LayoutRequest,
        cancel: &AtomicBool,
        mut render: impl FnMut(usize, Viewport) -> Result<Frame, String>,
    ) -> Result<Layout, String> {
        check_cancel(cancel)?;
        let LayoutRequest {
            geometry,
            mode,
            zoom,
            origin,
            bounds,
        } = request;
        let split = mode == ImageMode::SideBySide;
        let slot = usize::from(!split);
        if let Some(layout) = &self.0[slot] {
            return Ok(layout.clone());
        }
        let width = if split {
            geometry.width / 2
        } else {
            geometry.width
        };
        let scale = zoom
            .unwrap_or_else(|| {
                (f64::from(width) / f64::from(bounds[0].max(1)))
                    .min(f64::from(geometry.height) / f64::from(bounds[1].max(1)))
                    .min(1.0)
            })
            .clamp(1.0 / 32768.0, 64.0);
        let viewport = Viewport {
            width,
            height: geometry.height,
            scale,
            origin_x: origin[0] - (f64::from(width) / scale - f64::from(bounds[0])) / 2.0,
            origin_y: origin[1] - (f64::from(geometry.height) / scale - f64::from(bounds[1])) / 2.0,
        };
        let mut left = render(0, viewport)?;
        flatten(&mut left, 0, cancel)?;
        let right_viewport = Viewport {
            width: if split { geometry.width - width } else { width },
            ..viewport
        };
        let mut right = render(1, right_viewport)?;
        flatten(&mut right, if split { width } else { 0 }, cancel)?;
        let layout = Layout {
            layers: ImageLayers {
                frames: [Arc::new(left), Arc::new(right)],
            },
            scale,
        };
        self.0[slot] = Some(layout.clone());
        Ok(layout)
    }
}

fn flatten(frame: &mut Frame, x_offset: u32, cancel: &AtomicBool) -> Result<(), String> {
    let stride = frame.width as usize * 4;
    if stride == 0 || frame.rgba.len() != stride * frame.height as usize {
        return Err("Invalid sampled comparison image.".into());
    }
    for (y, row) in frame.rgba.chunks_exact_mut(stride).enumerate() {
        check_cancel(cancel)?;
        for (x, pixel) in row.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let alpha = u16::from(pixel[3]);
            let background = if ((x + x_offset as usize) / 12 + y / 12).is_multiple_of(2) {
                190
            } else {
                230
            };
            if alpha != 255 {
                for channel in &mut pixel[..3] {
                    *channel = ((u16::from(*channel) * alpha + background * (255 - alpha) + 127)
                        / 255) as u8;
                }
            }
            pixel[3] = 255;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn odd_width_keeps_actual_pixel_scale_shared_origin_and_checkerboard_phase() {
        let mut cache = LayoutCache::default();
        let layout = cache
            .get_or_render(
                LayoutRequest {
                    geometry: ViewerGeometry {
                        width: 25,
                        height: 1,
                        scale: 1.0,
                    },
                    mode: ImageMode::SideBySide,
                    zoom: Some(1.0),
                    origin: [4.0, 2.0],
                    bounds: [12, 1],
                },
                &AtomicBool::new(false),
                |side, viewport| {
                    assert_eq!(viewport.width, if side == 0 { 12 } else { 13 });
                    assert_eq!(
                        (viewport.scale, viewport.origin_x, viewport.origin_y),
                        (1.0, 4.0, 2.0)
                    );
                    Ok(Frame {
                        width: viewport.width,
                        height: 1,
                        rgba: vec![0; viewport.width as usize * 4],
                    })
                },
            )
            .unwrap();
        assert_eq!(&layout.layers.frames[0].rgba[..4], &[190, 190, 190, 255]);
        assert_eq!(&layout.layers.frames[1].rgba[..4], &[230, 230, 230, 255]);
        assert_eq!(&layout.layers.frames[1].rgba[48..52], &[190, 190, 190, 255]);
    }
    #[test]
    fn warmed_modes_reuse_both_sampled_layouts_and_changes_resample() {
        let mut cache = LayoutCache::default();
        let cancel = AtomicBool::new(false);
        let geometry = ViewerGeometry {
            width: 120,
            height: 80,
            scale: 1.0,
        };
        let calls = std::cell::Cell::new(0);
        let paint = |cache: &mut LayoutCache, mode| {
            cache
                .get_or_render(
                    LayoutRequest {
                        geometry,
                        mode,
                        zoom: None,
                        origin: [0.0; 2],
                        bounds: [4096, 3072],
                    },
                    &cancel,
                    |side, viewport| {
                        calls.set(calls.get() + 1);
                        assert_eq!(
                            viewport.scale,
                            if mode == ImageMode::SideBySide {
                                60.0 / 4096.0
                            } else {
                                80.0 / 3072.0
                            }
                        );
                        Ok(Frame {
                            width: viewport.width,
                            height: viewport.height,
                            rgba: [side as u8, 30, 50, 255]
                                .repeat((viewport.width * viewport.height) as usize),
                        })
                    },
                )
                .unwrap()
        };
        let split = paint(&mut cache, ImageMode::SideBySide);
        let overlay = paint(&mut cache, ImageMode::Swipe);
        for mode in [
            ImageMode::OnionSkin,
            ImageMode::SideBySide,
            ImageMode::Swipe,
            ImageMode::OnionSkin,
        ] {
            let expected = if mode == ImageMode::SideBySide {
                &split
            } else {
                &overlay
            };
            let frame = paint(&mut cache, mode);
            for side in 0..2 {
                assert!(Arc::ptr_eq(
                    &expected.layers.frames[side],
                    &frame.layers.frames[side]
                ));
            }
        }
        assert_eq!(
            calls.get(),
            4,
            "Only two worker renders per distinct layout"
        );
        cache.clear();
        let fresh = paint(&mut cache, ImageMode::Swipe);
        assert_eq!(calls.get(), 6);
        assert!(!Arc::ptr_eq(
            &fresh.layers.frames[0],
            &overlay.layers.frames[0]
        ));
    }
    #[test]
    fn transparency_keeps_global_checkerboard_phase_and_honors_cancellation() {
        let mut frame = Frame {
            width: 24,
            height: 1,
            rgba: [255, 0, 0, 0].repeat(24),
        };
        flatten(&mut frame, 6, &AtomicBool::new(false)).unwrap();
        assert_eq!(&frame.rgba[..4], &[190, 190, 190, 255]);
        assert_eq!(&frame.rgba[24..28], &[230, 230, 230, 255]);
        assert!(flatten(&mut frame, 0, &AtomicBool::new(true)).is_err());
    }
}
