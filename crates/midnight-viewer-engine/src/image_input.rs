//! Image gestures retain their displayed-image identity across asynchronous camera repaints.
use crate::image_geometry::ImageGeometry;
use midnight_core::ViewerImageInput;

#[derive(Default)]
pub(crate) struct ImageFrames {
    first: Option<u64>,
    last: u64,
    coordinate_minimum: u64,
}

impl ImageFrames {
    pub fn painted(&mut self, frame: u64) {
        self.first.get_or_insert(frame);
        self.last = frame;
    }

    pub fn resized(&mut self) {
        self.coordinate_minimum = self.last.saturating_add(1);
    }

    fn accepts(&self, frame: u64, anchored: bool) -> bool {
        self.first
            .is_some_and(|first| frame >= first && frame <= self.last)
            && (!anchored || frame >= self.coordinate_minimum)
    }
}

pub(crate) fn apply(
    view: &mut ImageGeometry,
    frames: &ImageFrames,
    frame: u64,
    input: &ViewerImageInput,
) -> bool {
    let anchored = matches!(
        input,
        ViewerImageInput::Zoom {
            anchor: Some(_),
            ..
        }
    );
    if !frames.accepts(frame, anchored) {
        return false;
    }
    match input {
        ViewerImageInput::Fit => view.fit(),
        ViewerImageInput::ActualPixels => view.actual(),
        ViewerImageInput::Zoom { factor, anchor } => view.zoom(
            *factor,
            anchor.map_or_else(|| view.center(), |anchor| anchor.map(f64::from)),
        ),
        ViewerImageInput::Drag { x, y } => view.scroll([-f64::from(*x), -f64::from(*y)]),
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_and_zoom_bursts_keep_every_delta_until_the_image_changes() {
        let mut view = ImageGeometry::new(3000, 2000, [600, 400]);
        view.actual();
        let mut frames = ImageFrames::default();
        frames.painted(7);
        for _ in 0..2 {
            assert!(apply(
                &mut view,
                &frames,
                7,
                &ViewerImageInput::Drag { x: 20.0, y: 30.0 }
            ));
        }
        assert_eq!(view.origin, [1160.0, 740.0]);
        for _ in 0..2 {
            assert!(apply(
                &mut view,
                &frames,
                7,
                &ViewerImageInput::Zoom {
                    factor: 1.25,
                    anchor: None
                }
            ));
        }
        assert_eq!(view.scale, 1.5625);
        frames.painted(8);
        assert!(apply(&mut view, &frames, 7, &ViewerImageInput::Fit));
        let mut next_image = ImageFrames::default();
        next_image.painted(9);
        assert!(!apply(
            &mut view,
            &next_image,
            8,
            &ViewerImageInput::ActualPixels
        ));
    }

    #[test]
    fn resize_requires_fresh_pointer_coordinates_but_keeps_relative_commands() {
        let mut view = ImageGeometry::new(3000, 2000, [600, 400]);
        let mut frames = ImageFrames::default();
        frames.painted(1);
        frames.resized();
        assert!(!apply(
            &mut view,
            &frames,
            1,
            &ViewerImageInput::Zoom {
                factor: 1.25,
                anchor: Some([20.0, 30.0])
            }
        ));
        assert!(apply(
            &mut view,
            &frames,
            1,
            &ViewerImageInput::ActualPixels
        ));
        assert!(apply(
            &mut view,
            &frames,
            1,
            &ViewerImageInput::Drag { x: 20.0, y: 30.0 }
        ));
        frames.painted(2);
        assert!(apply(
            &mut view,
            &frames,
            2,
            &ViewerImageInput::Zoom {
                factor: 1.25,
                anchor: Some([20.0, 30.0])
            }
        ));
    }
}
