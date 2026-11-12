//! Presentation-only camera for bounded retained text/document pixels.
use midnight_core::{ViewerRasterInfo, ViewerSurfaceId};
use midnight_ui_input::WheelMotion;
use std::time::Duration;

const PREFETCH: f64 = 128.0;

#[derive(Default)]
pub(super) struct Camera {
    pub origin: [f64; 2],
    info: Option<ViewerRasterInfo>,
    scale: [f64; 2],
    motion: WheelMotion,
    direct: bool,
    suspended: bool,
    sequence: u64,
    last_request: Option<([f64; 2], bool)>,
    retry_pending: bool,
    resume: bool,
    seeking: bool,
}

impl Camera {
    pub fn receive(&mut self, info: &ViewerRasterInfo, scale: [f64; 2]) -> bool {
        if !valid(info)
            || scale
                .iter()
                .any(|value| !value.is_finite() || *value <= 0.0)
        {
            return false;
        }
        let changed = self
            .info
            .as_ref()
            .is_none_or(|old| old.surface != info.surface);
        let mut origin = if changed {
            info.origin
        } else {
            clamp(info, self.origin)
        };
        let exact_ack = !changed
            && info.settled
            && !self.motion.active()
            && info.pan_sequence == self.sequence
            && self.last_request == Some((self.origin, true))
            && (0..2).all(|axis| (info.origin[axis] - self.origin[axis]).abs() < 0.25);
        if exact_ack {
            origin = info.origin;
        }
        if (!changed && self.suspended) || !covered(info, origin) {
            return false;
        }
        if changed {
            self.cancel();
            self.last_request = Some((origin, info.settled));
            self.retry_pending = false;
            self.suspended = false;
        }
        if !changed && !self.direct && self.motion.active() {
            let pending = self.motion.pending();
            self.resume |= self.info.as_ref().is_some_and(|old| {
                (0..2).any(|axis| {
                    (pending[axis] > 0.0
                        && self.origin[axis] >= old.coverage_max[axis] - 0.0001
                        && origin[axis] < info.coverage_max[axis])
                        || (pending[axis] < 0.0
                            && self.origin[axis] <= old.coverage_min[axis] + 0.0001
                            && origin[axis] > info.coverage_min[axis])
                })
            });
        }
        if exact_ack {
            self.last_request = Some((origin, true));
        }
        self.origin = origin;
        self.scale = scale;
        let mut metadata = info.clone();
        // Pixel ownership stays with frame retirement and the Slint images.
        metadata.gutter = None;
        self.info = Some(metadata);
        true
    }

    pub fn info(&self) -> Option<&ViewerRasterInfo> {
        (!self.suspended).then_some(self.info.as_ref()).flatten()
    }

    /// Invert this frame's normalized mapping only where existing pixels cover it.
    pub fn seek(
        &mut self,
        axis: usize,
        position: f64,
        frame_position: f64,
        dragging: bool,
    ) -> bool {
        if axis > 1
            || !position.is_finite()
            || !(0.0..=1.0).contains(&position)
            || !frame_position.is_finite()
        {
            return false;
        }
        let Some(info) = self.info() else {
            return false;
        };
        let mut target = self.origin;
        target[axis] = if axis == 1
            && let Some(map) = &info.scroll_map
        {
            let Some(origin) = map.inverse_near(
                position,
                f64::from(info.viewport[1]),
                info.coverage_min[1].max(info.limit_min[1].unwrap_or(f64::NEG_INFINITY)),
                info.coverage_max[1].min(info.limit_max[1].unwrap_or(f64::INFINITY)),
                self.origin[1],
            ) else {
                return false;
            };
            origin
        } else {
            let slope = info.scroll_per_pixel[axis];
            if slope <= 0.0 {
                return false;
            }
            info.origin[axis] + (position - frame_position) / slope
        };
        if !target[axis].is_finite() || !covered(info, target) || clamp(info, target) != target {
            return false;
        }
        self.motion.cancel();
        self.resume = false;
        self.direct = true;
        self.seeking = dragging;
        self.origin = target;
        self.retry_pending = self.last_request != Some((target, !dragging));
        true
    }
    pub fn begin_seek(&mut self) -> bool {
        if self.info().is_none() {
            return false;
        }
        self.motion.cancel();
        self.resume = false;
        self.seeking = true;
        self.retry_pending = self.last_request != Some((self.origin, false));
        true
    }
    pub fn end_seek(&mut self) -> bool {
        if !self.seeking {
            return false;
        }
        self.seeking = false;
        self.retry_pending =
            self.info().is_some() && self.last_request != Some((self.origin, true));
        true
    }
    pub fn push(&mut self, x: f32, y: f32, now: Duration, direct: bool) -> bool {
        if self.info().is_none() || !x.is_finite() || !y.is_finite() || (x == 0.0 && y == 0.0) {
            return false;
        }
        if self.direct != direct {
            self.motion.cancel();
        }
        self.direct = direct;
        self.seeking = false;
        self.resume = false;
        if direct {
            self.motion.push_direct(x, y, now)
        } else {
            self.motion.push(x, y, now)
        }
    }

    pub fn tick(&mut self, now: Duration) -> bool {
        if self.suspended {
            return false;
        }
        if self.resume {
            self.motion.restart_remaining();
            self.resume = false;
        }
        let Some(info) = &self.info else {
            return false;
        };
        let pending = self.motion.pending();
        let target = clamp(
            info,
            std::array::from_fn(|axis| self.origin[axis] + pending[axis] * self.scale[axis]),
        );
        for axis in 0..2 {
            if pending[axis] != 0.0 && target[axis] == self.origin[axis] {
                self.motion.discard_axis(axis);
            }
        }
        let bounds = std::array::from_fn(|axis| {
            let low =
                info.coverage_min[axis].max(info.limit_min[axis].unwrap_or(f64::NEG_INFINITY));
            let high = info.coverage_max[axis].min(info.limit_max[axis].unwrap_or(f64::INFINITY));
            (
                ((low - self.origin[axis]) / self.scale[axis]).min(0.0) as f32,
                ((high - self.origin[axis]) / self.scale[axis]).max(0.0) as f32,
            )
        });
        let Some((x, y)) = self.motion.tick_bounded(now, bounds) else {
            return false;
        };
        let old = self.origin;
        self.origin = clamp(
            info,
            std::array::from_fn(|axis| {
                (old[axis] + f64::from([x, y][axis]) * self.scale[axis])
                    .clamp(info.coverage_min[axis], info.coverage_max[axis])
            }),
        );
        let pending = self.motion.pending();
        let target = clamp(
            info,
            std::array::from_fn(|axis| self.origin[axis] + pending[axis] * self.scale[axis]),
        );
        for axis in 0..2 {
            if pending[axis] != 0.0 && target[axis] == self.origin[axis] {
                self.motion.discard_axis(axis);
            }
        }
        self.origin != old
    }

    pub fn request(&mut self) -> Option<(ViewerSurfaceId, u64, [f64; 2], bool)> {
        let info = self.info()?;
        let surface = info.surface;
        let settled = !self.seeking && !self.motion.active();
        let origin = clamp(
            info,
            std::array::from_fn(|axis| {
                // Keep overlap even when a byte-limited page has very little spare height.
                let lookahead =
                    PREFETCH.min((info.coverage_max[axis] - info.coverage_min[axis]) / 2.0);
                self.origin[axis]
                    + (self.motion.pending()[axis] * self.scale[axis]).clamp(-lookahead, lookahead)
            }),
        );
        if self.last_request == Some((origin, settled)) {
            return None;
        }
        self.sequence = self.sequence.checked_add(1)?;
        self.last_request = Some((origin, settled));
        self.retry_pending = false;
        Some((surface, self.sequence, origin, settled))
    }

    pub fn retry(&mut self) {
        self.last_request = None;
        self.retry_pending = true;
    }
    pub fn active(&self) -> bool {
        !self.suspended && (self.motion.active() || self.retry_pending)
    }
    pub fn cancel(&mut self) {
        self.motion.cancel();
        self.seeking = false;
        self.retry_pending =
            self.info().is_some() && self.last_request != Some((self.origin, true));
        self.resume = false;
    }
    /// Transfer only undelivered logical input before suspending the old pixels.
    pub fn take_pending(&mut self) -> (WheelMotion, bool) {
        self.retry_pending = false;
        self.resume = false;
        (std::mem::take(&mut self.motion), self.direct)
    }
    pub fn suspend(&mut self) {
        self.cancel();
        self.suspended = true;
    }
}

fn covered(info: &ViewerRasterInfo, origin: [f64; 2]) -> bool {
    (0..2).all(|axis| {
        origin[axis] >= info.coverage_min[axis] && origin[axis] <= info.coverage_max[axis]
    })
}
fn clamp(info: &ViewerRasterInfo, origin: [f64; 2]) -> [f64; 2] {
    std::array::from_fn(|axis| {
        origin[axis].clamp(
            info.limit_min[axis].unwrap_or(f64::NEG_INFINITY),
            info.limit_max[axis].unwrap_or(f64::INFINITY),
        )
    })
}
fn valid(info: &ViewerRasterInfo) -> bool {
    info.surface.0 != 0
        && info.pixel_id != 0
        && (0..2).all(|axis| {
            info.viewport[axis] != 0
                && info.origin[axis].is_finite()
                && info.raster_origin[axis].is_finite()
                && info.coverage_min[axis].is_finite()
                && info.coverage_max[axis].is_finite()
                && info.coverage_min[axis] <= info.coverage_max[axis]
                && info.scroll_per_pixel[axis].is_finite()
                && info.scroll_per_pixel[axis] >= 0.0
                && info.limit_min[axis].is_none_or(f64::is_finite)
                && info.limit_max[axis].is_none_or(f64::is_finite)
                && info.limit_min[axis].unwrap_or(f64::NEG_INFINITY)
                    <= info.limit_max[axis].unwrap_or(f64::INFINITY)
        })
}

#[cfg(test)]
#[path = "raster_camera_tests.rs"]
mod tests;
