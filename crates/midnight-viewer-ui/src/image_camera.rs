//! Presentation-only camera. Retained pixels move before the decoder supplies new strips.
use midnight_core::{DocumentLayout, ViewerImageInfo, ViewerSurfaceId};
use midnight_ui_input::WheelMotion;
use std::{collections::HashSet, time::Duration};

const EDGE: f64 = 256.0;

/// The scrollbar context of an image or document frame: a scrollbar drag lasts while it
/// stays the same. A new surface means a new zoom, viewport or page, except that a
/// document in the Continuous layout gets a new surface for every page its view reaches
/// while it scrolls. Its context is the document, zoom and viewport instead, so a drag
/// across pages keeps going to where the thumb is released.
pub(super) fn position_context(generation: u64, info: &ViewerImageInfo, size: [u32; 2]) -> String {
    if info.document_layout == Some(DocumentLayout::Continuous) {
        format!(
            "document:{generation}:{:x}:{}x{}",
            info.zoom.to_bits(),
            size[0],
            size[1]
        )
    } else {
        format!("image:{}", info.surface.0)
    }
}

#[derive(Default)]
pub(super) struct Camera {
    info: Option<ViewerImageInfo>,
    pub origin: [f64; 2],
    grid: HashSet<[i32; 2]>,
    factor: f64,
    motion: WheelMotion,
    direct: bool,
    sequence: u64,
    last_request: Option<[f64; 2]>,
    waiting: Option<[f64; 2]>,
    /// The scrollbar positions (per axis, 0..=1) the waiting target came from. A new
    /// surface maps them onto its own extent, so the newest thumb position wins over a
    /// frame painted for an earlier step of the drag.
    thumb: [Option<f64>; 2],
    suspended: bool,
}

impl Camera {
    pub fn receive(&mut self, info: &ViewerImageInfo, native_scale: f32, grid: HashSet<[i32; 2]>) {
        let changed = self
            .info
            .as_ref()
            .is_none_or(|old| old.surface != info.surface);
        let thumb = (changed && self.waiting.is_some()).then_some(self.thumb);
        if changed {
            self.origin = info.origin;
            self.motion.cancel();
            self.waiting = None;
            self.thumb = [None; 2];
            self.last_request = None;
            self.suspended = false;
        }
        self.info = Some(info.clone());
        self.set_native_scale(native_scale);
        self.grid = grid;
        if let Some(thumb) = thumb {
            // The new surface was not asked for this target: keep aiming at it there.
            self.aim(info.origin, thumb);
        } else if let Some(target) = self.waiting
            && self.covered(target)
        {
            self.origin = target;
            self.waiting = None;
            self.thumb = [None; 2];
        }
    }

    pub fn set_native_scale(&mut self, native_scale: f32) {
        if let Some(info) = &self.info {
            self.factor = f64::from(native_scale) / info.zoom;
        }
    }

    pub fn incoming(&self, info: &ViewerImageInfo) -> Option<([f64; 2], bool)> {
        if self
            .info
            .as_ref()
            .is_none_or(|old| old.surface != info.surface)
        {
            return Some((info.origin, true));
        }
        if self.suspended {
            return None;
        }
        if self.waiting.is_some() {
            // An intermediate scrollbar reply must neither move the thumb nor
            // replace the retained view with a far-away, already superseded page.
            if info.pan_sequence < self.sequence {
                return None;
            }
            return Some((self.waiting.unwrap_or(self.origin), true));
        }
        Some((self.origin, false))
    }

    pub fn push(&mut self, x: f32, y: f32, now: Duration, direct: bool) -> bool {
        if self.info.is_none() || self.suspended {
            return false;
        }
        if self.direct != direct {
            self.motion.cancel();
        }
        self.direct = direct;
        self.waiting = None;
        self.thumb = [None; 2];
        if direct {
            self.motion.push_direct(x, y, now)
        } else {
            self.motion.push(x, y, now)
        }
    }

    pub fn seek(&mut self, axis: usize, position: f64) -> bool {
        if axis >= 2 || !position.is_finite() || self.suspended || self.info.is_none() {
            return false;
        }
        let mut thumb = if self.waiting.is_some() {
            self.thumb
        } else {
            [None; 2]
        };
        thumb[axis] = Some(position.clamp(0.0, 1.0));
        self.aim(self.waiting.unwrap_or(self.origin), thumb);
        true
    }

    /// Aim at scrollbar positions; an axis without one keeps `base`. Retained tiles
    /// that cover the target show it at once, otherwise it waits for them.
    fn aim(&mut self, base: [f64; 2], thumb: [Option<f64>; 2]) {
        let Some(info) = &self.info else {
            return;
        };
        let mut target = base;
        for (axis, position) in thumb.into_iter().enumerate() {
            let Some(position) = position else {
                continue;
            };
            let dimension = if axis == 0 { info.width } else { info.height };
            let maximum = f64::from(dimension) - info.visible_source[axis];
            target[axis] = if maximum <= 0.0 {
                maximum / 2.0
            } else {
                maximum * position
            };
        }
        self.motion.cancel();
        if self.covered(target) {
            self.origin = target;
            self.waiting = None;
            self.thumb = [None; 2];
        } else {
            self.waiting = Some(target);
            self.thumb = thumb;
        }
        // A cached seek still needs to tell the backend about its final camera.
        self.last_request = None;
    }

    pub fn tick(&mut self, now: Duration) -> bool {
        if self.suspended || self.waiting.is_some() {
            return false;
        }
        let Some(info) = &self.info else {
            return false;
        };
        let pending = self.motion.pending();
        let target = clamp(
            info,
            std::array::from_fn(|axis| self.origin[axis] + pending[axis] * self.factor),
        );
        for axis in 0..2 {
            if pending[axis] != 0.0 && target[axis] == self.origin[axis] {
                self.motion.discard_axis(axis);
            }
        }
        let bounds = self.bounds();
        let Some((x, y)) = self.motion.tick_bounded(now, bounds) else {
            return false;
        };
        let before = self.origin;
        self.origin = clamp(
            info,
            [
                self.origin[0] + f64::from(x) * self.factor,
                self.origin[1] + f64::from(y) * self.factor,
            ],
        );
        for axis in 0..2 {
            if self.origin[axis] == before[axis] && [x, y][axis] != 0.0 {
                self.motion.discard_axis(axis);
            }
        }
        self.origin != before
    }

    pub fn request(&mut self) -> Option<(ViewerSurfaceId, u64, [f64; 2])> {
        if self.suspended {
            return None;
        }
        let info = self.info.as_ref()?;
        let origin = if let Some(target) = self.waiting {
            target
        } else {
            let frontier = EDGE / info.raster_scale;
            clamp(
                info,
                std::array::from_fn(|axis| {
                    self.origin[axis]
                        + (self.motion.pending()[axis] * self.factor).clamp(-frontier, frontier)
                }),
            )
        };
        if self.last_request == Some(origin) {
            return None;
        }
        self.sequence = self.sequence.checked_add(1)?;
        self.last_request = Some(origin);
        Some((info.surface, self.sequence, origin))
    }

    pub fn retry(&mut self) {
        self.last_request = None;
    }
    pub fn active(&self) -> bool {
        !self.suspended && (self.motion.active() || self.waiting.is_some())
    }
    pub fn cancel(&mut self) {
        self.motion.cancel();
        self.waiting = None;
        self.thumb = [None; 2];
        self.last_request = None;
    }
    pub fn suspend(&mut self) {
        self.cancel();
        self.suspended = true;
    }

    pub fn position(&self, axis: usize) -> f64 {
        self.normalized(axis, self.waiting.unwrap_or(self.origin))
    }

    /// The axis Home and End move along: down when the image is taller than the view,
    /// else across when it is wider; none when it fits.
    pub fn scroll_axis(&self) -> Option<usize> {
        let info = self.info.as_ref().filter(|_| !self.suspended)?;
        [1, 0]
            .into_iter()
            .find(|&axis| f64::from([info.width, info.height][axis]) > info.visible_source[axis])
    }

    pub fn displayed_position(&self, axis: usize) -> f64 {
        self.normalized(axis, self.origin)
    }

    fn normalized(&self, axis: usize, origin: [f64; 2]) -> f64 {
        let Some(info) = &self.info else {
            return 0.0;
        };
        let dimension = if axis == 0 { info.width } else { info.height };
        let maximum = f64::from(dimension) - info.visible_source[axis];
        if maximum <= 0.0 {
            0.0
        } else {
            (origin[axis] / maximum).clamp(0.0, 1.0)
        }
    }

    fn covered(&self, origin: [f64; 2]) -> bool {
        let Some(info) = &self.info else {
            return false;
        };
        let (low, high) = grid_bounds(origin, info);
        (low[1]..high[1]).all(|y| (low[0]..high[0]).all(|x| self.grid.contains(&[x, y])))
    }

    fn bounds(&self) -> [(f32, f32); 2] {
        let Some(info) = &self.info else {
            return [(0.0, 0.0); 2];
        };
        let (mut low, mut high) = grid_bounds(self.origin, info);
        if !self.covered(self.origin) {
            return [(0.0, 0.0); 2];
        }
        while (low[1]..high[1]).all(|y| self.grid.contains(&[low[0] - 1, y])) {
            low[0] -= 1;
        }
        while (low[1]..high[1]).all(|y| self.grid.contains(&[high[0], y])) {
            high[0] += 1;
        }
        while (low[0]..high[0]).all(|x| self.grid.contains(&[x, low[1] - 1])) {
            low[1] -= 1;
        }
        while (low[0]..high[0]).all(|x| self.grid.contains(&[x, high[1]])) {
            high[1] += 1;
        }
        std::array::from_fn(|axis| {
            (
                ((f64::from(low[axis]) * EDGE / info.raster_scale - self.origin[axis])
                    / self.factor)
                    .min(0.0) as f32,
                ((f64::from(high[axis]) * EDGE / info.raster_scale
                    - info.visible_source[axis]
                    - self.origin[axis])
                    / self.factor)
                    .max(0.0) as f32,
            )
        })
    }
}

pub(super) fn grid_bounds(origin: [f64; 2], info: &ViewerImageInfo) -> ([i32; 2], [i32; 2]) {
    (
        std::array::from_fn(|axis| (origin[axis] * info.raster_scale / EDGE).floor() as i32),
        std::array::from_fn(|axis| {
            ((origin[axis] + info.visible_source[axis]) * info.raster_scale / EDGE).ceil() as i32
        }),
    )
}

fn clamp(info: &ViewerImageInfo, origin: [f64; 2]) -> [f64; 2] {
    std::array::from_fn(|axis| {
        let maximum = f64::from([info.width, info.height][axis]) - info.visible_source[axis];
        if maximum <= 0.0 {
            maximum / 2.0
        } else {
            origin[axis].clamp(0.0, maximum)
        }
    })
}

#[cfg(test)]
#[path = "image_camera_tests.rs"]
mod tests;
