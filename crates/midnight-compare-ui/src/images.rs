//! Reuse renderer-owned images while mode/divider/opacity change.
use super::*;
use midnight_compare_engine::{ImageLayers, RasterFrame};
use std::sync::{Arc, Weak};

#[derive(Default)]
pub(super) struct Surfaces([Option<Pair>; 2]);
struct Pair {
    sources: [Weak<RasterFrame>; 2],
    images: [slint::Image; 2],
}

impl Surfaces {
    pub(super) fn present(&mut self, window: &ComparatorWindow, layers: &ImageLayers, split: bool) {
        let slot = usize::from(!split);
        let sources = layers.frames.each_ref().map(Arc::downgrade);
        if self.0[slot].as_ref().is_none_or(|pair| {
            !(0..2).all(|side| Weak::ptr_eq(&pair.sources[side], &sources[side]))
        }) {
            let images =
                layers.frames.each_ref().map(|frame| {
                    slint::Image::from_rgba8_premultiplied(slint::SharedPixelBuffer::<
                        slint::Rgba8Pixel,
                    >::clone_from_slice(
                        &frame.rgba, frame.width, frame.height
                    ))
                });
            self.0[slot] = Some(Pair { sources, images });
        }
        if let Some(pair) = &self.0[slot] {
            window.set_split_position(
                layers.frames[0].width as f32
                    / (layers.frames[0].width + layers.frames[1].width).max(1) as f32,
            );
            window.set_left_surface(pair.images[0].clone());
            window.set_right_surface(pair.images[1].clone());
            // Keep the general surface readiness contract used by the native probe.
            window.set_surface(pair.images[0].clone());
        }
    }
}
