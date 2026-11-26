//! One bounded, cancellable image actor per viewer window. Owned and reaped by the backend.
use crate::document_render::BandPlan;
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
    /// The tiles of one document band, rendered for the camera of `revision`.
    Tiles {
        revision: u64,
        tiles: Vec<(Key, Arc<ViewerPixels>)>,
    },
    Failed(String),
}

pub(crate) enum Render {
    Tile {
        revision: u64,
        key: Key,
        charge: Arc<Charge>,
    },
    Validate(u64),
    Band {
        revision: u64,
        plan: BandPlan,
    },
}

/// Publishes the actor thread's exit, then wakes the viewer worker. Created first on that
/// thread, it is dropped last, also when the thread unwinds, so `finished` is true when
/// the worker handles this wake.
struct Exit {
    exited: Arc<AtomicBool>,
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
}
impl Drop for Exit {
    fn drop(&mut self) {
        self.exited.store(true, Ordering::Release);
        (self.wake)();
    }
}

/// Run `body` on the actor thread behind the exit guard. `body` owns every other capture,
/// so all of them are dropped when it returns, before the exit is published.
fn spawn(
    exited: Arc<AtomicBool>,
    wake: Arc<dyn Fn() -> bool + Send + Sync>,
    body: impl FnOnce() + Send + 'static,
) -> std::io::Result<JoinHandle<()>> {
    std::thread::Builder::new()
        .name("viewer-image".into())
        .spawn(move || {
            let _exit = Exit { exited, wake };
            body();
        })
}

pub(crate) struct ImageWorker {
    /// Dropped on cancellation, which also ends the idle actor's blocking receive.
    sender: Option<SyncSender<Render>>,
    pub events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
    /// The actor thread published its exit (see `Exit`).
    exited: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl ImageWorker {
    /// An actor stand-in that delivers `events`. A `body` stands for the actor's thread,
    /// for example one still closing its session after cancellation; it runs behind the
    /// exit guard, which then calls no wake.
    #[cfg(test)]
    pub(crate) fn for_test(
        events: Receiver<Event>,
        body: Option<Box<dyn FnOnce() + Send>>,
    ) -> Self {
        Self::waking_for_test(events, Arc::new(|| true), body)
    }

    /// As `for_test`, with the wake the exit guard calls.
    #[cfg(test)]
    pub(crate) fn waking_for_test(
        events: Receiver<Event>,
        wake: Arc<dyn Fn() -> bool + Send + Sync>,
        body: Option<Box<dyn FnOnce() + Send>>,
    ) -> Self {
        let (sender, _) = mpsc::sync_channel(1);
        let exited = Arc::new(AtomicBool::new(false));
        let thread = body.map(|body| spawn(exited.clone(), wake, body).unwrap());
        Self {
            sender: Some(sender),
            events,
            cancel: Arc::new(AtomicBool::new(false)),
            exited,
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
        let exited = Arc::new(AtomicBool::new(false));
        let thread = spawn(exited.clone(), wake.clone(), move || {
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
                    crate::open_trace::mark(if prepared.is_some() {
                        "image_spare_claimed"
                    } else {
                        "image_cold_start"
                    });
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
                    crate::open_trace::mark("image_session_opened");
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
                                            crate::open_trace::mark("image_tile_rendered");
                                            crate::image_pixels::premultiply(&mut frame.rgba);
                                            let pixels =
                                                Arc::new(ViewerPixels::new(frame.rgba, charge)?);
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
                                        Render::Band { revision, plan } => {
                                            if !session.is_document() {
                                                return Err(
                                                    "Unexpected document render request.".into()
                                                );
                                            }
                                            // Only close/source retirement owns cancel: a newer
                                            // camera never ends the SDK session or this band.
                                            let tiles = crate::document_render::render_band(
                                                &mut session,
                                                plan,
                                                &stopped,
                                                Duration::from_secs(15),
                                            )?;
                                            Event::Tiles { revision, tiles }
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
                        let status = midnight_image_viewer_host::cleanup_status(&error).unwrap_or(
                            midnight_image_viewer_host::CleanupStatus::ProcessUnconfirmed,
                        );
                        if let Some(startup) = &startup {
                            startup.cleanup_failed(status);
                        }
                        if status == midnight_image_viewer_host::CleanupStatus::ProcessUnconfirmed {
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
            exited,
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

    /// Render one band of document tiles for the camera of `revision`.
    pub fn band(&self, revision: u64, plan: BandPlan) -> bool {
        self.send(Render::Band { revision, plan })
    }

    pub fn cancel(&mut self) {
        self.cancel.store(true, Ordering::Release);
        self.sender = None;
    }

    /// The actor thread ended its work; dropping the worker then only joins the thread's
    /// return. The thread wakes the viewer worker once this is true.
    pub fn finished(&self) -> bool {
        self.thread.is_none() || self.exited.load(Ordering::Acquire)
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
pub(crate) fn test_worker() -> (ImageWorker, Receiver<Render>) {
    let (sender, requests) = mpsc::sync_channel(1);
    let (_, events) = mpsc::sync_channel(1);
    (
        ImageWorker {
            sender: Some(sender),
            events,
            cancel: Arc::new(AtomicBool::new(false)),
            exited: Arc::new(AtomicBool::new(false)),
            thread: None,
        },
        requests,
    )
}

#[cfg(test)]
#[path = "document_band_tests.rs"]
mod document_band_tests;

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
        let (worker, requests) = test_worker();
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
        let (worker, requests) = test_worker();
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
}
