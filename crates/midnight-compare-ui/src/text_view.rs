//! The compared text's view on the screen. A text or byte frame carries content painted
//! around its view (`midnight_compare_engine::ScrollView`), so a wheel glide frame,
//! touchpad motion or a scroll key moves the view here at once, on the frame it arrives,
//! as the text viewer moves over its prepared strips. The worker is sent the same scrolling
//! when the view nears the end of the painted margin or comes to rest, and paints a new
//! surface around the new position; the frame says how much of the sent scrolling it
//! includes, so the view stays where it was moved. One view moves both panes: they cannot
//! drift apart. Pure state; `lib.rs` presents it.
use midnight_compare_engine::ScrollView;

#[derive(Default)]
pub(super) struct TextView {
    /// The presented text or byte frame's view; `None` for images and before a frame.
    frame: Option<ScrollView>,
    /// The scrolling this window's session has taken, summed per axis as sent.
    sent: [f64; 2],
    /// Scrolling moved here and not yet queued for the worker.
    unsent: [f64; 2],
}

/// Where the view is shown in the presented surface, in surface pixels, and where it
/// stands in the content.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Shown {
    /// Each pane's view: left edge and width.
    pub x: [i32; 2],
    pub width: [i32; 2],
    /// The view's top edge and height.
    pub y: i32,
    pub height: i32,
    /// The left pane's share of the content's width.
    pub split: f32,
    /// The shown position and the farthest one, logical pixels per axis.
    pub position: [f64; 2],
    pub extent: [f64; 2],
}

impl TextView {
    /// A newly presented frame: `None` for an image comparison.
    pub fn present(&mut self, frame: Option<ScrollView>) {
        self.frame = frame;
    }

    /// Whether a text or byte frame is presented, so scrolling moves the view here.
    pub fn active(&self) -> bool {
        self.frame.is_some()
    }

    /// The session took this scroll input.
    pub fn sent(&mut self, x: f32, y: f32) {
        self.sent[0] += f64::from(x);
        self.sent[1] += f64::from(y);
    }

    /// How far the view stands ahead of the presented frame, logical pixels per axis
    /// (positive: further into the content), with `queued` scrolling waiting in the input
    /// queue (as sent: negative moves down or right).
    fn ahead(&self, frame: &ScrollView, queued: [f64; 2]) -> [f64; 2] {
        std::array::from_fn(|axis| {
            frame.scrolled[axis] - self.sent[axis] - queued[axis] - self.unsent[axis]
        })
    }

    /// Move the view by a scroll delta (negative: down or right), up to the content's
    /// edges. False without a text or byte frame: the delta is then the worker's.
    pub fn scroll(&mut self, x: f32, y: f32, queued: [f64; 2]) -> bool {
        let Some(frame) = self.frame else {
            return false;
        };
        let ahead = self.ahead(&frame, queued);
        for (axis, delta) in [x, y].into_iter().enumerate() {
            if !delta.is_finite() {
                continue;
            }
            // Scrolling already on its way is never pulled back, only limited further.
            let low = (-frame.position[axis]).min(ahead[axis]);
            let high = (frame.extent[axis] - frame.position[axis]).max(ahead[axis]);
            let target = (ahead[axis] - f64::from(delta)).clamp(low, high);
            self.unsent[axis] -= target - ahead[axis];
        }
        true
    }

    /// The scrolling moved here and not yet queued, for the worker.
    pub fn take(&mut self) -> Option<(f32, f32)> {
        if self.unsent == [0.0; 2] {
            return None;
        }
        let [x, y] = std::mem::take(&mut self.unsent);
        Some((x as f32, y as f32))
    }

    /// Whether the view has moved past half the painted margin towards content beyond
    /// it, so the worker should paint around the view now rather than when it rests.
    pub fn due(&self, queued: [f64; 2]) -> bool {
        let Some(frame) = &self.frame else {
            return false;
        };
        let ahead = self.ahead(frame, queued);
        let scale = f64::from(frame.scale.max(f32::MIN_POSITIVE));
        (0..2).any(|axis| {
            let [before, after] = frame.margin[axis].map(|pixels| f64::from(pixels) / scale);
            let more_after = frame.position[axis] + after < frame.extent[axis] - 0.5;
            let more_before = frame.position[axis] - before > 0.5;
            (ahead[axis] > 0.0 && more_after && ahead[axis] > after / 2.0)
                || (ahead[axis] < 0.0 && more_before && -ahead[axis] > before / 2.0)
        })
    }

    /// Where the view is shown: moved by everything not painted yet, within the content's
    /// edges and the painted margin (the view waits at the margin for the next frame, as
    /// the text viewer waits at the end of its strips).
    pub fn shown(&self, queued: [f64; 2]) -> Option<Shown> {
        let frame = self.frame?;
        let ahead = self.ahead(&frame, queued);
        let scale = f64::from(frame.scale.max(f32::MIN_POSITIVE));
        let offset: [i64; 2] = std::array::from_fn(|axis| {
            let wanted = ahead[axis]
                .clamp(
                    -frame.position[axis],
                    frame.extent[axis] - frame.position[axis],
                )
                .max(-frame.position[axis]);
            let pixels = (wanted * scale).round() as i64;
            let [before, after] = frame.margin[axis].map(i64::from);
            pixels.clamp(-before, after)
        });
        let [[left, right], [above, _]] = frame.margin.map(|margin| margin.map(i64::from));
        let panes = frame.panes.map(i64::from);
        let x = [left + offset[0], left + panes[0] + right + left + offset[0]];
        Some(Shown {
            x: x.map(|value| value as i32),
            width: frame.panes.map(|value| value as i32),
            y: (above + offset[1]) as i32,
            height: frame.height as i32,
            split: frame.panes[0] as f32 / (frame.panes[0] + frame.panes[1]).max(1) as f32,
            position: std::array::from_fn(|axis| {
                frame.position[axis] + offset[axis] as f64 / scale
            }),
            extent: frame.extent,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(position: f64, scrolled: f64) -> ScrollView {
        ScrollView {
            scrolled: [0.0, scrolled],
            position: [0.0, position],
            extent: [131_072.0, 10_000.0],
            scale: 1.0,
            panes: [400, 400],
            height: 500,
            margin: [[0, 200], [position.min(500.0) as u32, 500]],
        }
    }

    #[test]
    fn scrolling_moves_the_view_at_once_and_a_frame_that_includes_it_keeps_it_in_place() {
        let mut view = TextView::default();
        assert!(
            !view.scroll(0.0, -10.0, [0.0; 2]),
            "no text frame: the worker's"
        );
        view.present(Some(frame(1000.0, 0.0)));
        assert_eq!(view.shown([0.0; 2]).unwrap().y, 500);
        assert!(view.scroll(0.0, -30.5, [0.0; 2]));
        let shown = view.shown([0.0; 2]).unwrap();
        assert_eq!(shown.y, 531);
        assert_eq!(shown.position[1], 1031.0);
        assert_eq!(shown.x, [0, 600]);
        // Queued for the worker, then sent: the view stays.
        let (x, y) = view.take().unwrap();
        assert_eq!((x, y), (0.0, -30.5));
        assert_eq!(view.shown([0.0, -30.5]).unwrap().y, 531);
        view.sent(x, y);
        assert_eq!(view.shown([0.0; 2]).unwrap().y, 531);
        // The worker's frame painted at the new position includes it.
        view.present(Some(frame(1030.5, -30.5)));
        let shown = view.shown([0.0; 2]).unwrap();
        assert_eq!(shown.y, 500);
        assert_eq!(shown.position[1], 1030.5);
    }

    #[test]
    fn the_view_stops_at_the_content_edges_and_waits_at_the_painted_margin() {
        let mut view = TextView::default();
        view.present(Some(frame(100.0, 0.0)));
        view.scroll(0.0, 300.0, [0.0; 2]);
        let shown = view.shown([0.0; 2]).unwrap();
        assert_eq!((shown.y, shown.position[1]), (0, 0.0));
        assert_eq!(view.take(), Some((0.0, 100.0)), "only up to the start");
        view.sent(0.0, 100.0);
        // Scrolling up beyond the start moves nothing more; down moves at once.
        view.scroll(0.0, 50.0, [0.0; 2]);
        assert_eq!(view.take(), None);
        view.scroll(0.0, -20.0, [0.0; 2]);
        assert_eq!(view.shown([0.0; 2]).unwrap().position[1], 20.0);
        // Far beyond the painted margin the view waits at its end, and is due long before.
        view.present(Some(frame(1000.0, 100.0)));
        view.take();
        assert!(!view.due([0.0; 2]));
        view.scroll(0.0, -200.0, [0.0; 2]);
        assert!(!view.due([0.0; 2]));
        view.scroll(0.0, -100.0, [0.0; 2]);
        assert!(view.due([0.0; 2]));
        view.scroll(0.0, -900.0, [0.0; 2]);
        let shown = view.shown([0.0; 2]).unwrap();
        assert_eq!(shown.y, 1000, "the margin's end");
        // Horizontal scrolling moves both panes alike; the left edge stops it.
        view.scroll(-40.0, 0.0, [0.0; 2]);
        assert_eq!(view.shown([0.0; 2]).unwrap().x, [40, 640]);
        view.scroll(100.0, 0.0, [0.0; 2]);
        assert_eq!(view.shown([0.0; 2]).unwrap().x, [0, 600]);
    }
}
