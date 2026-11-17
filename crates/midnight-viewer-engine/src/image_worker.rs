//! One bounded, cancellable image actor per viewer window. Owned and reaped by the backend.
use crate::image_tiles::{Charge, EDGE, Key};
use midnight_core::ViewerPixels;
use midnight_image_viewer_host::{ImageSession, Metadata, Provider, Source, Viewport};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::JoinHandle,
    time::Duration,
};

pub(crate) fn run_actor<R>(
    operation: impl FnOnce(&mut Option<R>) -> Result<(), String>,
    fault: impl FnOnce(),
) -> Result<(), String> {
    // This actual external reservation must outlive unwinding of the entire operation/session.
    let mut permit = None;
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| operation(&mut permit))) {
        Ok(result) => result,
        Err(payload) => {
            std::mem::forget(payload);
            // Retain unknown accounting before even the fault hook can unwind.
            std::mem::forget(permit.take());
            fault();
            Err("Image viewer stopped after an internal error.".into())
        }
    }
}

pub(crate) enum Event {
    Opened {
        pages: Vec<Metadata>,
        document: bool,
    },
    Frame {
        revision: u64,
        key: Key,
        pixels: Arc<ViewerPixels>,
    },
    Validated(u64),
    DocumentFrame {
        revision: u64,
        raster: DocumentRaster,
    },
    Failed(String),
}

/// A premultiplied document viewport. The actor moves its pixels into the shared buffer the
/// UI frame carries, so the viewer worker publishes it without copying.
pub(crate) struct DocumentRaster {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
}

enum Render {
    Tile {
        revision: u64,
        key: Key,
        charge: Arc<Charge>,
    },
    Validate(u64),
    Document {
        revision: u64,
        page: usize,
        viewport: Viewport,
    },
}

pub(crate) struct ImageWorker {
    /// Dropped on cancellation, which also ends the idle actor's blocking receive.
    sender: Option<SyncSender<Render>>,
    pub events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ImageWorker {
    /// An actor stand-in that delivers `events`. A `thread` stands for the actor's thread,
    /// for example one still closing its session after cancellation.
    #[cfg(test)]
    pub(crate) fn for_test(events: Receiver<Event>, thread: Option<JoinHandle<()>>) -> Self {
        let (sender, _) = mpsc::sync_channel(1);
        Self {
            sender: Some(sender),
            events,
            cancel: Arc::new(AtomicBool::new(false)),
            thread,
        }
    }

    #[cfg(test)]
    pub(crate) fn opened_for_test(pages: Vec<Metadata>, document: bool) -> Self {
        let (output, events) = mpsc::sync_channel(1);
        output.send(Event::Opened { pages, document }).unwrap();
        Self::for_test(events, None)
    }

    pub fn open(
        provider: Provider,
        source: Arc<dyn Source>,
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
        startup: Option<crate::image_startup::Startup>,
    ) -> Result<Self, String> {
        let (sender, requests) = mpsc::sync_channel::<Render>(1);
        let (output, events) = mpsc::sync_channel(1);
        let cancel = Arc::new(AtomicBool::new(false));
        let stopped = cancel.clone();
        let thread = std::thread::Builder::new()
            .name("viewer-image".into())
            .spawn(move || {
                let result = run_actor(
                    |permit| {
                        let acquired = startup
                            .as_ref()
                            .map(|startup| startup.acquire(&provider, &stopped))
                            .transpose()
                            .map_err(|error| error.to_string())?;
                        let (acquired_permit, prepared) = match acquired {
                            Some(crate::image_startup::Acquired { permit, prepared }) => {
                                (Some(permit), prepared)
                            }
                            None => (None, None),
                        };
                        *permit = acquired_permit;
                        let opened = if let Some(prepared) = prepared {
                            // The manager already rejected stale unused spares. The host performs
                            // the final identity/source guard immediately before the consuming Open.
                            ImageSession::open_prepared(
                                &provider,
                                prepared,
                                source,
                                &stopped,
                                Duration::from_secs(30),
                            )
                        } else {
                            ImageSession::open_owned(
                                &provider,
                                source,
                                &stopped,
                                Duration::from_secs(30),
                                Box::new(permit.as_ref().cloned()),
                            )
                        };
                        let mut session = match opened {
                            Ok(session) => session,
                            Err(error) => {
                                let status = midnight_image_viewer_host::cleanup_status(&error)
                                    .unwrap_or(
                                    midnight_image_viewer_host::CleanupStatus::ProcessUnconfirmed,
                                );
                                if let Some(startup) = &startup {
                                    startup.cleanup_failed(status);
                                }
                                if status
                                    == midnight_image_viewer_host::CleanupStatus::ProcessUnconfirmed
                                {
                                    std::mem::forget(permit.take());
                                }
                                return Err(error.to_string());
                            }
                        };
                        let result = (|| {
                            if let Some(startup) = &startup {
                                startup.opened(&provider);
                            }
                            let pages = (0..session.page_count())
                                .filter_map(|page| session.page_metadata(page))
                                .collect();
                            if output
                                .try_send(Event::Opened {
                                    pages,
                                    document: session.is_document(),
                                })
                                .is_err()
                            {
                                return Ok(());
                            }
                            wake();
                            // Cancellation drops the only sender, so an idle actor blocks
                            // here without polling and still observes cancellation.
                            while !stopped.load(Ordering::Acquire) {
                                match requests.recv() {
                                    Ok(_) if stopped.load(Ordering::Acquire) => break,
                                    Ok(request) => {
                                        let event = match request {
                                            Render::Tile {
                                                revision,
                                                key,
                                                charge,
                                            } => {
                                                let scale = f64::from_bits(key.scale);
                                                let mut frame = session
                                                    .render_page(
                                                        key.page,
                                                        Viewport {
                                                            width: EDGE,
                                                            height: EDGE,
                                                            scale,
                                                            origin_x: f64::from(key.x)
                                                                * f64::from(EDGE)
                                                                / scale,
                                                            origin_y: f64::from(key.y)
                                                                * f64::from(EDGE)
                                                                / scale,
                                                        },
                                                        &stopped,
                                                        Duration::from_secs(15),
                                                    )
                                                    .map_err(|error| error.to_string())?;
                                                crate::image_pixels::premultiply(&mut frame.rgba);
                                                let pixels = Arc::new(ViewerPixels::new(
                                                    frame.rgba, charge,
                                                )?);
                                                Event::Frame {
                                                    revision,
                                                    key,
                                                    pixels,
                                                }
                                            }
                                            Render::Validate(revision) => {
                                                session
                                                    .validate(&stopped)
                                                    .map_err(|error| error.to_string())?;
                                                Event::Validated(revision)
                                            }
                                            Render::Document {
                                                revision,
                                                page,
                                                viewport,
                                            } => {
                                                if !session.is_document() {
                                                    return Err(
                                                        "Unexpected document render request."
                                                            .into(),
                                                    );
                                                }
                                                let mut frame = session
                                                    .render_page(
                                                        page,
                                                        viewport,
                                                        &stopped,
                                                        Duration::from_secs(15),
                                                    )
                                                    .map_err(|error| error.to_string())?;
                                                crate::image_pixels::premultiply(&mut frame.rgba);
                                                Event::DocumentFrame {
                                                    revision,
                                                    raster: DocumentRaster {
                                                        width: frame.width,
                                                        height: frame.height,
                                                        rgba: Arc::from(frame.rgba),
                                                    },
                                                }
                                            }
                                        };
                                        if stopped.load(Ordering::Acquire)
                                            || output.try_send(event).is_err()
                                        {
                                            break;
                                        }
                                        wake();
                                    }
                                    Err(mpsc::RecvError) => break,
                                }
                            }
                            Ok::<_, String>(())
                        })();
                        if let Err(error) = session.close_checked() {
                            let status = midnight_image_viewer_host::cleanup_status(&error)
                                .unwrap_or(
                                    midnight_image_viewer_host::CleanupStatus::ProcessUnconfirmed,
                                );
                            if let Some(startup) = &startup {
                                startup.cleanup_failed(status);
                            }
                            if status
                                == midnight_image_viewer_host::CleanupStatus::ProcessUnconfirmed
                            {
                                std::mem::forget(permit.take());
                            }
                            return Err(match result {
                                Ok(()) => error.to_string(),
                                Err(original) => format!("{original}; {error}"),
                            });
                        }
                        result
                    },
                    || {
                        if let Some(startup) = &startup {
                            startup.cleanup_failed(
                                midnight_image_viewer_host::CleanupStatus::ProcessUnconfirmed,
                            );
                        }
                    },
                );
                if let Err(error) = result
                    && output.try_send(Event::Failed(error)).is_ok()
                {
                    wake();
                }
            })
            .map_err(|error| error.to_string())?;
        Ok(Self {
            sender: Some(sender),
            events,
            cancel,
            thread: Some(thread),
        })
    }

    fn send(&self, request: Render) -> bool {
        self.sender
            .as_ref()
            .is_some_and(|sender| sender.try_send(request).is_ok())
    }

    pub fn render(&self, revision: u64, key: Key, charge: Arc<Charge>) -> bool {
        self.send(Render::Tile {
            revision,
            key,
            charge,
        })
    }

    pub fn validate(&self, revision: u64) -> bool {
        self.send(Render::Validate(revision))
    }
    pub fn document(&self, revision: u64, page: usize, viewport: Viewport) -> bool {
        self.send(Render::Document {
            revision,
            page,
            viewport,
        })
    }

    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.sender = None;
    }

    pub fn finished(&self) -> bool {
        self.thread.as_ref().is_none_or(JoinHandle::is_finished)
    }
}

impl Drop for ImageWorker {
    fn drop(&mut self) {
        self.cancel();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        image_geometry::ImageGeometry,
        image_view::{ImageView, Paint},
    };
    use std::sync::atomic::AtomicU64;

    #[test]
    fn transparent_scenes_still_require_current_source_validation_before_publication() {
        let (sender, requests) = mpsc::sync_channel(1);
        let (_, events) = mpsc::sync_channel(1);
        let worker = ImageWorker {
            sender: Some(sender),
            events,
            cancel: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let mut view = ImageView::new(worker, Arc::new(AtomicU64::new(0)));
        let mut geometry = ImageGeometry::new(607, 204, [1200, 800]);
        geometry.origin = [4096.0; 2];
        view.geometry = Some(geometry);
        view.changed();
        view.validated = Some(view.revision - 1);
        assert!(view.request().unwrap().is_none());
        let Render::Validate(revision) = requests.try_recv().unwrap() else {
            panic!("transparent scenes must validate without a decoder request")
        };
        assert_eq!(revision, view.revision);
        assert!(view.scene.tiles().is_some());
        assert!(view.request().unwrap().is_none());
        assert!(requests.try_recv().is_err());
        view.pending = None;
        view.validated = Some(revision);
        let Some(Paint::Tiles(tiles)) = view.request().unwrap() else {
            panic!("validated transparent scene must remain presentable")
        };
        assert!(
            tiles
                .iter()
                .all(|tile| tile.pixels.rgba().iter().all(|byte| *byte == 0))
        );
        assert!(requests.try_recv().is_err());
    }

    #[test]
    fn magnification_reuses_tiles_but_revalidates_each_new_camera_revision() {
        let (sender, requests) = mpsc::sync_channel(1);
        let (_, events) = mpsc::sync_channel(1);
        let worker = ImageWorker {
            sender: Some(sender),
            events,
            cancel: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let mut view = ImageView::new(worker, Arc::new(AtomicU64::new(0)));
        view.geometry = Some(ImageGeometry::new(607, 204, [1200, 800]));
        view.changed();
        while let Some((key, charge)) = view.scene.next().unwrap() {
            let pixels = ViewerPixels::new(vec![255; 256 * 256 * 4], charge).unwrap();
            view.scene.complete(key, Arc::new(pixels)).unwrap();
        }
        view.validated = Some(view.revision);
        assert!(matches!(view.request().unwrap(), Some(Paint::Tiles(_))));
        let geometry = view.geometry.as_mut().unwrap();
        geometry.zoom(1.25, geometry.center());
        view.changed();
        assert!(view.request().unwrap().is_none());
        let Render::Validate(revision) = requests.try_recv().unwrap() else {
            panic!("cached magnification must validate without resampling source pixels")
        };
        assert_eq!(revision, view.revision);
        assert!(view.request().unwrap().is_none());
        view.pending = None;
        view.validated = Some(revision);
        assert!(matches!(view.request().unwrap(), Some(Paint::Tiles(_))));
        assert!(requests.try_recv().is_err());
    }
    #[test]
    fn documents_request_one_bounded_viewport_without_tile_or_guard_amplification() {
        let (sender, requests) = mpsc::sync_channel(1);
        let (_, events) = mpsc::sync_channel(1);
        let worker = ImageWorker {
            sender: Some(sender),
            events,
            cancel: Arc::new(AtomicBool::new(false)),
            thread: None,
        };
        let mut view = ImageView::new(worker, Arc::new(AtomicU64::new(0)));
        view.document = true;
        let mut geometry = ImageGeometry::document(2400, 1800, [1000, 700]);
        assert!(geometry.zoom_from(
            midnight_core::ViewerZoomPreference::ScaleBasisPoints(20000),
            [1200.0, 900.0],
            [500.0, 350.0],
        ));
        view.geometry = Some(geometry);
        view.changed();
        assert!(view.request().unwrap().is_none());
        let Render::Document {
            revision,
            page,
            viewport,
        } = requests.try_recv().unwrap()
        else {
            panic!("document must use whole bounded viewport")
        };
        assert_eq!((page, viewport.width, viewport.height), (0, 1000, 700));
        assert_eq!(viewport.scale, 2.0);
        assert_eq!(view.scene.plan.as_ref().unwrap().scale, viewport.scale);
        assert!(view.request().unwrap().is_none());
        assert!(requests.try_recv().is_err());
        view.pending = None;
        view.document_frame = Some((
            revision,
            DocumentRaster {
                width: 1000,
                height: 700,
                rgba: vec![255; 1000 * 700 * 4].into(),
            },
        ));
        assert!(matches!(view.request().unwrap(), Some(Paint::Raster(_))));
        assert!(view.request().unwrap().is_none());
        assert!(requests.try_recv().is_err());
    }
}
