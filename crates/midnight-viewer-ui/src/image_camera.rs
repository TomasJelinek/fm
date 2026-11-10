//! Presentation-only camera. Retained pixels move before the decoder supplies new strips.
use midnight_core::{ViewerImageInfo, ViewerSurfaceId};
use midnight_ui_input::WheelMotion;
use std::{collections::HashSet, time::Duration};

const EDGE: f64 = 256.0;

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
    suspended: bool,
}

impl Camera {
    pub fn receive(&mut self, info: &ViewerImageInfo, native_scale: f32, grid: HashSet<[i32; 2]>) {
        let changed = self
            .info
            .as_ref()
            .is_none_or(|old| old.surface != info.surface);
        if changed {
            self.origin = info.origin;
            self.motion.cancel();
            self.waiting = None;
            self.last_request = None;
            self.suspended = false;
        }
        self.factor = f64::from(native_scale) / info.zoom;
        self.info = Some(info.clone());
        self.grid = grid;
        if let Some(target) = self.waiting
            && self.covered(target)
        {
            self.origin = target;
            self.waiting = None;
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
        if direct {
            self.motion.push_direct(x, y, now)
        } else {
            self.motion.push(x, y, now)
        }
    }

    pub fn seek(&mut self, axis: usize, position: f64) -> bool {
        if axis >= 2 || !position.is_finite() || self.suspended {
            return false;
        }
        let Some(info) = &self.info else {
            return false;
        };
        let dimension = if axis == 0 { info.width } else { info.height };
        let maximum = f64::from(dimension) - info.visible_source[axis];
        let mut target = self.waiting.unwrap_or(self.origin);
        target[axis] = if maximum <= 0.0 {
            maximum / 2.0
        } else {
            maximum * position.clamp(0.0, 1.0)
        };
        self.motion.cancel();
        self.waiting = Some(target);
        if self.covered(target) {
            self.origin = target;
            self.waiting = None;
        }
        // A cached seek still needs to tell the backend about its final camera.
        self.last_request = None;
        true
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
        self.last_request = None;
    }
    pub fn suspend(&mut self) {
        self.cancel();
        self.suspended = true;
    }

    pub fn position(&self, axis: usize) -> f64 {
        self.normalized(axis, self.waiting.unwrap_or(self.origin))
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
