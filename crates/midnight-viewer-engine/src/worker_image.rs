use super::*;
use crate::{document_view::DocumentView, image_geometry::ImageGeometry, image_worker::Event};

impl Worker {
    pub(super) fn image_input(&mut self, input: &ViewerInput) -> bool {
        let Some(image) = &mut self.image else {
            return false;
        };
        let before = image.geometry.clone();
        let page_before = image.page;
        let layout_before = image.document_view.as_ref().map(|view| view.layout);
        let mut page_turn = false;
        let pan_ack = !image.document
            && matches!(
                input,
                ViewerInput::Image {
                    input: ViewerImageInput::PanTo { .. },
                    ..
                }
            );
        if let ViewerInput::Resize(geometry) = input {
            self.image_size = [geometry.width.max(1), geometry.height.max(1)];
            self.geometry = bounded_geometry(*geometry);
            if let Some(document) = &mut image.document_view {
                document.resize(self.image_size);
            } else {
                image.frames.resized();
                if let Some(view) = &mut image.geometry {
                    view.resize(self.image_size);
                }
            }
        } else if let Some(document) = &mut image.document_view {
            match input {
                ViewerInput::DocumentLayout { frame, layout } => {
                    if !image.frames.accepts_document(*frame, true)
                        || !document.set_layout(*frame, *layout)
                    {
                        return true;
                    }
                }
                ViewerInput::DocumentPage { frame, delta } => {
                    if !self.frames.accepts(*frame) || !document.change_page(*delta) {
                        return true;
                    }
                    page_turn = true;
                }
                ViewerInput::Scroll { x, y } => document.scroll([f64::from(*x), f64::from(*y)]),
                ViewerInput::Seek(position) => document.seek(1, *position),
                ViewerInput::Pan(position) => document.seek(0, *position),
                ViewerInput::Image { frame, input } => {
                    if !image.frames.accepts_document_image(*frame, input)
                        || !document.image_input(input)
                    {
                        return true;
                    }
                }
                ViewerInput::NextFile | ViewerInput::PreviousFile | ViewerInput::Close => {
                    return false;
                }
                _ => return true,
            }
        } else if let ViewerInput::Image {
            input:
                ViewerImageInput::PanTo {
                    surface,
                    sequence,
                    origin,
                },
            ..
        } = input
        {
            if !image.pan_to(*surface, *sequence, *origin) {
                return true;
            }
        } else {
            let Some(view) = &mut image.geometry else {
                return !matches!(
                    input,
                    ViewerInput::NextFile | ViewerInput::PreviousFile | ViewerInput::Close
                );
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
                ViewerInput::NextFile | ViewerInput::PreviousFile | ViewerInput::Close => {
                    return false;
                }
                _ => return true,
            }
        }
        if let Some(document) = &image.document_view {
            image.geometry = Some(document.geometry.clone());
            image.page = document.page;
        }
        // A clamped Fit pan does not rerender or upload an unchanged viewport.
        if image.geometry != before
            || image.page != page_before
            || image.document_view.as_ref().map(|view| view.layout) != layout_before
            || pan_ack
        {
            if page_turn {
                image.frames = Default::default();
            } else if image.document_view.is_some() && matches!(input, ViewerInput::Resize(_)) {
                image.frames.resized();
            }
            image.changed();
            self.frames.invalidate();
            self.dirty = true;
        }
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
        let result = match event {
            Event::Opened { pages, document } => (|| {
                if let Some(image) = &mut self.image {
                    let metadata = pages.first().copied().ok_or("Image contains no pages.")?;
                    let document_view = document
                        .then(|| DocumentView::new(&pages, self.image_size))
                        .transpose()?;
                    image.pages = pages;
                    image.document = document;
                    image.document_view = document_view;
                    // Each PDF source begins in Fit; page turns retain live geometry.
                    let view = if let Some(document) = &image.document_view {
                        document.geometry.clone()
                    } else {
                        let mut view =
                            ImageGeometry::new(metadata.width, metadata.height, self.image_size);
                        view.restore_zoom(self.preferences.image_zoom);
                        view
                    };
                    image.geometry = Some(view);
                    image.changed();
                    self.dirty = true;
                }
                Ok(())
            })(),
            Event::Frame {
                revision,
                key,
                pixels,
            } => {
                let Some(image) = &mut self.image else {
                    return;
                };
                image.pending = None;
                image.validated = Some(revision);
                self.dirty = true;
                image.scene.complete(key, pixels)
            }
            Event::Validated(revision) => {
                if let Some(image) = &mut self.image {
                    image.pending = None;
                    image.validated = Some(revision);
                    self.dirty = true;
                }
                Ok(())
            }
            Event::DocumentFrame { revision, raster } => {
                if let Some(image) = &mut self.image {
                    image.complete_document(revision, Some(raster));
                    self.dirty = true;
                }
                Ok(())
            }
            Event::DocumentSuperseded(revision) => {
                if let Some(image) = &mut self.image {
                    image.complete_document(revision, None);
                    self.dirty = true;
                }
                Ok(())
            }
            Event::Failed(error) => Err(error),
        };
        if let Err(error) = result {
            self.frames.invalidate();
            if let Some(mut image) = self.image.take() {
                output.put(ViewerOutput::ImageUnavailable {
                    generation: self.positions.generation,
                    surface: image.scene.surface,
                    message: error.clone(),
                });
                image.worker.cancel();
                self.closing_image = Some(image.worker);
            }
            self.status = error;
        }
    }

    pub(super) fn paint_image(&mut self, output: &Mailbox) -> Result<(), String> {
        let Some(image) = &mut self.image else {
            return Ok(());
        };
        let Some(paint) = image.request()? else {
            self.dirty = image.waiting_budget;
            return Ok(());
        };
        let Some(view) = &image.geometry else {
            return Ok(());
        };
        let Some(plan) = &image.scene.plan else {
            return Ok(());
        };
        let metrics = |axis| {
            image
                .document_view
                .as_ref()
                .map_or_else(|| view.metrics(axis), |document| document.metrics(axis))
        };
        let (position, vertical_extent) = metrics(1);
        let (horizontal_position, horizontal_extent) = metrics(0);
        let revision = self.frames.painted()?;
        image.frames.painted(revision);
        if let Some(document) = &mut image.document_view {
            document.painted(revision);
        }
        let (width, height, pixels, tiles) = match paint {
            crate::image_view::Paint::Tiles(tiles) => (
                plan.raster_size[0],
                plan.raster_size[1],
                Arc::from([]),
                Some(tiles.into()),
            ),
            // The actor already shared these pixels; the frame takes the same buffer.
            crate::image_view::Paint::Raster(raster) => {
                (raster.width, raster.height, raster.rgba, None)
            }
        };
        output.put(ViewerOutput::Frame(Box::new(ViewerFrame {
            generation: self.positions.generation,
            position_sequence: [0; 2],
            revision,
            mode: VIEWER_MODE_IMAGE,
            modes: vec![VIEWER_MODE_IMAGE],
            wrap_enabled: false,
            wrap_available: false,
            image: Some(ViewerImageInfo {
                surface: image.scene.surface,
                origin: view.origin,
                visible_source: view.viewport.map(|value| f64::from(value) / view.scale),
                raster_scale: plan.scale,
                pan_sequence: image.scene.sequence,
                width: view.width,
                height: view.height,
                zoom: view.scale,
                fitted: view.fitted,
                page: if image.document {
                    image.page as u32 + 1
                } else {
                    0
                },
                page_count: image.pages.len() as u32,
                document_layout: image.document_view.as_ref().map(|document| document.layout),
            }),
            raster: None,
            raster_rebase: None,
            width,
            height,
            pixels,
            tiles,
            accessible_text: (if image.document {
                format!("Document page {} of {}", image.page + 1, image.pages.len())
            } else {
                format!("Image {} × {} pixels", view.width, view.height)
            })
            .into(),
            position,
            vertical_extent,
            horizontal_position,
            horizontal_extent,
            caption: self.caption.clone(),
            status: if image.document {
                format!(
                    "Page {} / {}  {:.1}%{}",
                    image.page + 1,
                    image.pages.len(),
                    view.scale * 100.0,
                    if view.fitted { "  Fit" } else { "" }
                )
            } else {
                format!(
                    "{} × {} pixels  {:.1}%{}",
                    view.width,
                    view.height,
                    view.scale * 100.0,
                    if view.fitted { "  Fit" } else { "" }
                )
            },
        })));
        Ok(())
    }
}
