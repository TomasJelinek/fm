//! Image gestures retain their displayed-image identity across asynchronous camera repaints.
use crate::image_geometry::ImageGeometry;
use midnight_core::ViewerImageInput;

#[derive(Default)]
pub(crate) struct ImageFrames {
    first: Option<u64>,
    last: u64,
    coordinate_minimum: u64,
    /// The first published frame of the surface shown last, and that surface.
    surface: Option<(u64, midnight_core::ViewerSurfaceId)>,
    /// The first and the latest frame published for this image or document. Page turns
    /// keep them: a relative page step names the document, not the page it was made on.
    published: Option<[u64; 2]>,
    /// The first published frame of the document layout shown last, and that layout.
    layout: Option<(u64, midnight_core::DocumentLayout)>,
}

impl ImageFrames {
    pub fn painted(&mut self, frame: u64) {
        self.first.get_or_insert(frame);
        self.last = frame;
        self.published = Some([self.published.map_or(frame, |[first, _]| first), frame]);
    }

    /// The page changed: no coordinate of a frame published so far holds for the new
    /// page, so every image input waits for its first frame. Page steps keep naming the
    /// document ([`Self::accepts_page_step`]).
    pub fn page_turned(&mut self) {
        *self = Self {
            published: self.published,
            ..Self::default()
        };
    }

    /// A relative page step (Ctrl+PageDown/Up, the page buttons) carries no coordinates,
    /// only the frame the window showed: it holds for every frame published for this
    /// document, so a step made while the previous page is still rendering is not lost.
    /// A frame of another document (another `ImageFrames`) or one never published is
    /// refused.
    pub fn accepts_page_step(&self, frame: u64) -> bool {
        self.published
            .is_some_and(|[first, last]| frame >= first && frame <= last)
    }

    /// Record the surface of the published `frame`.
    pub fn surface(&mut self, frame: u64, surface: midnight_core::ViewerSurfaceId) {
        if self.surface.is_none_or(|(_, shown)| shown != surface) {
            self.surface = Some((frame, surface));
        }
    }

    /// A document's tile coordinates hold for every frame of one surface: an input made
    /// against an earlier frame of the surface shown last refers to the latest frame.
    /// Other frames stay as they are, for the ordinary checks to accept or refuse.
    pub fn surface_frame(&self, frame: u64) -> u64 {
        match self.surface {
            Some((first, _)) if frame >= first && frame <= self.last => self.last,
            _ => frame,
        }
    }

    /// Record the document layout of the published `frame`.
    pub fn layout(&mut self, frame: u64, layout: midnight_core::DocumentLayout) {
        if self.layout.is_none_or(|(_, shown)| shown != layout) {
            self.layout = Some((frame, layout));
        }
    }

    pub fn resized(&mut self) {
        self.coordinate_minimum = self.last.saturating_add(1);
    }

    fn accepts(&self, frame: u64, anchored: bool) -> bool {
        self.first
            .is_some_and(|first| frame >= first && frame <= self.last)
            && (!anchored || frame >= self.coordinate_minimum)
    }

    /// Camera-anchored document inputs (a layout switch, a pointer zoom by a factor) are
    /// resolved against the published camera snapshot, so they hold only for the latest
    /// frame.
    pub fn accepts_document(&self, frame: u64, anchored: bool) -> bool {
        self.accepts(frame, anchored) && (!anchored || frame == self.last)
    }

    /// A source-anchored zoom (`ViewZoom`) names a source point of the frame the window
    /// showed. Its meaning depends only on the page (reset by [`Self::page_turned`]), the
    /// layout and the viewport size ([`Self::resized`]) of that frame, not on its camera:
    /// it holds for every frame since the layout shown last was published, so a zoom key
    /// pressed before the window applied the latest zoomed frame is not lost.
    pub fn accepts_document_image(&self, frame: u64, input: &ViewerImageInput) -> bool {
        match input {
            ViewerImageInput::ViewZoom { .. } => {
                self.accepts(frame, true) && self.layout.is_some_and(|(first, _)| frame >= first)
            }
            ViewerImageInput::Zoom {
                anchor: Some(_), ..
            } => self.accepts_document(frame, true),
            _ => self.accepts_document(frame, false),
        }
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
    ) || matches!(input,ViewerImageInput::ViewZoom {zoom,..} if *zoom!=midnight_core::ViewerZoomPreference::Fit);
    if !frames.accepts(frame, anchored) {
        return false;
    }
    match input {
        ViewerImageInput::ViewZoom {
            zoom,
            source_anchor,
            viewport_anchor,
        } => return view.zoom_from(*zoom, *source_anchor, *viewport_anchor),
        ViewerImageInput::Fit => view.fit(),
        ViewerImageInput::ActualPixels => view.actual(),
        ViewerImageInput::Zoom { factor, anchor } => view.zoom(
            *factor,
            anchor.map_or_else(|| view.center(), |anchor| anchor.map(f64::from)),
        ),
        ViewerImageInput::Drag { x, y } => view.scroll([-f64::from(*x), -f64::from(*y)]),
        ViewerImageInput::PanTo { .. } => return false, // Surface/sequence checked by ImageView.
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn documents_refuse_old_coordinate_frames_but_keep_relative_gesture_bursts() {
        let mut frames = ImageFrames::default();
        let paged = midnight_core::DocumentLayout::Paged;
        frames.painted(6);
        frames.layout(6, midnight_core::DocumentLayout::Continuous);
        frames.painted(7);
        frames.layout(7, paged);
        frames.painted(8);
        frames.layout(8, paged);
        assert!(!frames.accepts_document(7, true));
        assert!(frames.accepts_document(8, true));
        let coordinates = ViewerImageInput::ViewZoom {
            zoom: midnight_core::ViewerZoomPreference::Fit,
            source_anchor: [100.0, 80.0],
            viewport_anchor: [50.0, 40.0],
        };
        let pointer = ViewerImageInput::Zoom {
            factor: 1.25,
            anchor: Some([50.0, 40.0]),
        };
        // Source anchors hold for every frame of the layout shown last; camera anchors
        // only for the latest frame.
        assert!(
            !frames.accepts_document_image(6, &coordinates),
            "other layout"
        );
        assert!(frames.accepts_document_image(7, &coordinates));
        assert!(frames.accepts_document_image(8, &coordinates));
        assert!(!frames.accepts_document_image(7, &pointer));
        assert!(frames.accepts_document_image(8, &pointer));
        assert!(frames.accepts_document_image(7, &ViewerImageInput::Drag { x: 3.0, y: 2.0 }));
        frames.resized();
        assert!(!frames.accepts_document_image(8, &coordinates));
        assert!(frames.accepts_document_image(8, &ViewerImageInput::Fit));
        frames.painted(9);
        assert!(frames.accepts_document_image(9, &coordinates));
        assert!(!frames.accepts_document(8, true));
    }

    #[test]
    fn page_steps_name_the_document_while_page_turns_retire_coordinates() {
        let mut frames = ImageFrames::default();
        assert!(!frames.accepts_page_step(0), "nothing published yet");
        frames.painted(7);
        frames.surface(7, midnight_core::ViewerSurfaceId(1));
        frames.painted(8);
        frames.page_turned();
        // The next page is still rendering: the window shows frame 8 (or 7).
        assert!(frames.accepts_page_step(7) && frames.accepts_page_step(8));
        assert!(!frames.accepts_page_step(9), "not published yet");
        assert!(
            !frames.accepts_document(8, true),
            "coordinates of the old page"
        );
        assert!(!frames.accepts_document_image(8, &ViewerImageInput::Fit));
        assert_eq!(
            frames.surface_frame(7),
            7,
            "the old page's surface is retired"
        );
        frames.painted(9);
        frames.surface(9, midnight_core::ViewerSurfaceId(2));
        assert!(frames.accepts_page_step(7) && frames.accepts_page_step(9));
        assert!(!frames.accepts_page_step(6) && !frames.accepts_page_step(10));
        assert!(frames.accepts_document(9, true));
        assert!(!frames.accepts_document(8, true));
        // Another document starts its own frames: a step made on this one is refused.
        let mut next_document = ImageFrames::default();
        next_document.painted(12);
        assert!(!next_document.accepts_page_step(9));
        assert!(next_document.accepts_page_step(12));
    }

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

    #[test]
    fn displayed_source_anchor_survives_prefetch_camera_zoom_bursts_and_refuses_old_coordinates() {
        let mut view = ImageGeometry::new(4000, 3000, [600, 400]);
        view.actual();
        let displayed = [1200.0, 900.0];
        let anchor = [100.0, 80.0];
        let source = [
            displayed[0] + f64::from(anchor[0]),
            displayed[1] + f64::from(anchor[1]),
        ];
        view.pan_to([1456.0, 1156.0]); // Backend has prefetched a frontier ahead of presentation.
        let mut frames = ImageFrames::default();
        frames.painted(7);
        frames.painted(8);
        for scale in [12500, 15625] {
            assert!(apply(
                &mut view,
                &frames,
                7,
                &ViewerImageInput::ViewZoom {
                    zoom: midnight_core::ViewerZoomPreference::ScaleBasisPoints(scale),
                    source_anchor: source,
                    viewport_anchor: anchor
                }
            ));
            for axis in 0..2 {
                assert!(
                    (view.origin[axis] + f64::from(anchor[axis]) / view.scale - source[axis]).abs()
                        < 0.00001
                );
            }
        }
        frames.resized();
        let before = view.clone();
        assert!(!apply(
            &mut view,
            &frames,
            8,
            &ViewerImageInput::ViewZoom {
                zoom: midnight_core::ViewerZoomPreference::ActualPixels,
                source_anchor: source,
                viewport_anchor: anchor
            }
        ));
        assert_eq!(view, before);
        let mut replacement = ImageFrames::default();
        replacement.painted(9);
        assert!(!apply(
            &mut view,
            &replacement,
            8,
            &ViewerImageInput::ViewZoom {
                zoom: midnight_core::ViewerZoomPreference::ActualPixels,
                source_anchor: source,
                viewport_anchor: anchor
            }
        ));
        assert_eq!(view, before);
    }
}
