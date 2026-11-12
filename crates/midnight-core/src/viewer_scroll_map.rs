//! Bounded visual-to-source coordinates shared by the viewer worker and presentation camera.
use std::sync::Arc;

/// One visual row boundary and its original source byte fraction.
#[derive(Clone, Copy, Debug)]
pub struct ViewerScrollAnchor {
    pub y: f64,
    pub fraction: f64,
}

/// Immutable coordinates for one decoded layout; never an accumulated file index.
#[derive(Clone, Debug)]
pub struct ViewerScrollMap {
    anchors: Arc<[ViewerScrollAnchor]>,
    origin: f64,
}

impl ViewerScrollMap {
    /// Input is bounded by the decoded page's visual rows and two endpoint anchors.
    pub fn new(anchors: Vec<ViewerScrollAnchor>) -> Option<Self> {
        if anchors.len() < 2
            || anchors.len() > 256 * 1024 + 2
            || anchors.iter().any(|a| {
                !a.y.is_finite() || !a.fraction.is_finite() || !(0.0..=1.0).contains(&a.fraction)
            })
            || anchors
                .windows(2)
                .any(|a| a[0].y >= a[1].y || a[0].fraction > a[1].fraction)
        {
            return None;
        }
        Some(Self {
            anchors: anchors.into(),
            origin: 0.0,
        })
    }

    /// Rebase coordinates without copying rows or retaining an older page.
    pub fn translated(&self, origin: f64) -> Self {
        Self {
            anchors: self.anchors.clone(),
            origin,
        }
    }

    fn fraction(&self, y: f64) -> Option<f64> {
        if !y.is_finite() {
            return None;
        }
        let y = y - self.origin;
        if !y.is_finite() {
            return None;
        }
        let first = self.anchors.first()?;
        let last = self.anchors.last()?;
        // Only real source endpoints may extend into blank viewport space.
        if y < first.y {
            return (first.fraction == 0.0).then_some(0.0);
        }
        if y > last.y {
            return (last.fraction == 1.0).then_some(1.0);
        }
        let i = self.anchors.partition_point(|a| a.y < y);
        if i == 0 {
            return Some(first.fraction);
        }
        let [a, b] = [
            self.anchors[i - 1],
            self.anchors[i.min(self.anchors.len() - 1)],
        ];
        if a.y == b.y {
            return Some(b.fraction);
        }
        Some(a.fraction + (b.fraction - a.fraction) * (y - a.y) / (b.y - a.y))
    }

    /// Couple the thumb position and extent using the actual visible source interval.
    pub fn metrics(&self, origin: f64, viewport: f64) -> Option<(f64, f64)> {
        if !viewport.is_finite() || viewport <= 0.0 {
            return None;
        }
        let top = self.fraction(origin)?;
        let bottom = self.fraction(origin + viewport)?;
        let extent = (bottom - top).clamp(0.0, 1.0);
        let remaining = top + (1.0 - bottom);
        let position = if remaining > 0.0 {
            top / remaining
        } else {
            0.0
        };
        Some((position.clamp(0.0, 1.0), extent))
    }

    /// Keep an already displayed interior position on its current visual occurrence.
    /// Blank/generated spans can have equal source fractions. Accept its exact
    /// value or its UI float representation; endpoints still select the real edge.
    pub fn inverse_near(
        &self,
        position: f64,
        viewport: f64,
        minimum: f64,
        maximum: f64,
        preferred: f64,
    ) -> Option<f64> {
        if position > 0.0
            && position < 1.0
            && preferred >= minimum
            && preferred <= maximum
            && let Some((current, _)) = self.metrics(preferred, viewport)
            && (position == current || position == f64::from(current as f32))
        {
            return Some(preferred);
        }
        self.inverse(position, viewport, minimum, maximum)
    }

    /// Invert only a caller's covered camera interval. Unloaded positions need a bounded read.
    fn inverse(&self, position: f64, viewport: f64, minimum: f64, maximum: f64) -> Option<f64> {
        if !position.is_finite()
            || !viewport.is_finite()
            || viewport <= 0.0
            || !(0.0..=1.0).contains(&position)
            || !minimum.is_finite()
            || !maximum.is_finite()
            || minimum > maximum
        {
            return None;
        }
        // A source plateau at an endpoint can cover many blank visual rows.
        // Only the real layout edge answers Home/End, never a clipped cache edge.
        if position == 0.0 || position == 1.0 {
            let first = self.anchors.first()?;
            let last = self.anchors.last()?;
            let edge = if position == 0.0 {
                if first.fraction != 0.0 {
                    return None;
                }
                self.origin + first.y
            } else {
                if last.fraction != 1.0 {
                    return None;
                }
                self.origin + (last.y - viewport).max(first.y)
            };
            return (edge >= minimum && edge <= maximum).then_some(edge);
        }
        let low_position = self.metrics(minimum, viewport)?.0;
        let high_position = self.metrics(maximum, viewport)?.0;
        if position < low_position - 1e-12 || position > high_position + 1e-12 {
            return None;
        }
        if position <= low_position {
            return Some(minimum);
        }
        if position >= high_position {
            return Some(maximum);
        }
        let (mut low, mut high) = (minimum, maximum);
        for _ in 0..48 {
            let mid = low + (high - low) / 2.0;
            if self.metrics(mid, viewport)?.0 < position {
                low = mid;
            } else {
                high = mid;
            }
        }
        Some(low + (high - low) / 2.0)
    }
}

#[cfg(test)]
#[path = "viewer_scroll_map_tests.rs"]
mod tests;
