use super::*;
use crate::{image_geometry::ImageGeometry, image_worker::Event};

impl Worker {
    pub(super) fn image_input(&mut self, input: &ViewerInput) -> bool {
        let Some(image) = &mut self.image else {
            return false;
        };
        if let ViewerInput::Resize(geometry) = input {
            image.frames.resized();
            self.image_size = [geometry.width.max(1), geometry.height.max(1)];
            self.geometry = bounded_geometry(*geometry);
            if let Some(view) = &mut image.geometry {
                view.resize(self.image_size);
            }
        } else {
            let Some(view) = &mut image.geometry else {
                return !matches!(input, ViewerInput::NextFile | ViewerInput::Close);
            };
            match input {
                ViewerInput::Scroll { x, y } => view.scroll([f64::from(*x), f64::from(*y)]),
                ViewerInput::Seek(position) => view.seek(1, *position),
                ViewerInput::Pan(position) => view.seek(0, *position),
                ViewerInput::Image { frame, input } => {
                    if !crate::image_input::apply(view, &image.frames, *frame, input) {
                        return true;
                    }
                }
                ViewerInput::NextFile | ViewerInput::Close => return false,
                _ => return true,
            }
        }
        image.changed();
        self.frames.invalidate();
        self.dirty = true;
        true
    }

    pub(super) fn poll_image(&mut self, output: &Mailbox) {
        let Some(event) = self
            .image
            .as_ref()
            .and_then(|image| image.worker.events.try_recv().ok())
        else {
            return;
        };
        match event {
            Event::Opened(metadata) => {
                if let Some(image) = &mut self.image {
                    image.geometry = Some(ImageGeometry::new(
                        metadata.width,
                        metadata.height,
                        self.image_size,
                    ));
                    image.changed();
                    self.dirty = true;
                }
            }
            Event::Frame { revision, frame } => {
                let Some(image) = &mut self.image else {
                    return;
                };
                if image.pending != Some(revision) {
                    return;
                }
                image.pending = None;
                if image.revision != revision {
                    self.dirty = true;
                    return;
                }
                let Some(view) = &image.geometry else {
                    return;
                };
                let (position, vertical_extent) = view.metrics(1);
                let (horizontal_position, horizontal_extent) = view.metrics(0);
                let revision = match self.frames.painted() {
                    Ok(revision) => revision,
                    Err(error) => {
                        self.error(error, output);
                        return;
                    }
                };
                image.frames.painted(revision);
                output.put(ViewerOutput::Frame(ViewerFrame {
                    revision,
                    mode: VIEWER_MODE_IMAGE,
                    modes: vec![VIEWER_MODE_IMAGE],
                    wrap_enabled: false,
                    wrap_available: false,
                    image: Some(ViewerImageInfo {
                        width: view.width,
                        height: view.height,
                        zoom: view.scale,
                        fitted: view.fitted,
                    }),
                    width: frame.width,
                    height: frame.height,
                    pixels: frame.rgba.into(),
                    accessible_text: format!("Image {} × {} pixels", view.width, view.height)
                        .into(),
                    position,
                    vertical_extent,
                    horizontal_position,
                    horizontal_extent,
                    caption: self.caption.clone(),
                    status: format!(
                        "{} × {} pixels  {:.1}%{}",
                        view.width,
                        view.height,
                        view.scale * 100.0,
                        if view.fitted { "  Fit" } else { "" }
                    ),
                }));
            }
            Event::Failed(error) => {
                self.frames.invalidate();
                if let Some(image) = self.image.take() {
                    image.worker.cancel();
                    self.closing_image = Some(image.worker);
                }
                self.error(error, output);
            }
        }
    }
}
