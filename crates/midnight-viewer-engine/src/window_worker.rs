//! Concrete queued ownership: all window work is destroyed before its admission credit.
use crate::{Mailbox, Service, ViewerAppearance, ViewerGeometry, ViewerLaunch, ViewerOutput};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::Receiver,
};

pub(super) struct Work {
    pub service: Service,
    pub launch: ViewerLaunch,
    pub appearance: ViewerAppearance,
    pub geometry: ViewerGeometry,
    pub receiver: Receiver<crate::worker::Message>,
    pub output: Arc<Mailbox>,
    pub stop: Arc<AtomicBool>,
    pub wakes: Arc<crate::input_queue::Wakes>,
    pub preferences: Arc<crate::preferences::Preferences>,
    pub negotiate_layout: bool,
    pub palette_lifetime: crate::palette::Lifetime,
    pub audio_volume: Arc<midnight_media_host::AudioVolume>,
}

pub(super) struct Job {
    pub work: Work,
    pub start: crate::WindowStart,
}

impl Job {
    pub fn run(self) {
        let Self { work, mut start } = self;
        start.started = true;
        work.run(start);
    }
}

impl Work {
    /// `window` is dropped (returning the credit) by the worker before it publishes
    /// Closed, or with the closure on an early error or unwinding.
    fn run(self, window: crate::WindowStart) {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if self.service.fonts.get().is_none()
                && let Some(names) =
                    midnight_viewer_render::installed_families(|| self.stop.load(Ordering::Acquire))
            {
                let _ = self.service.fonts.set(names);
            }
            let (runtime, images, web) = self.service.catalogs()?;
            // Video support is discovered only for an opening that may route to it.
            let service = self.service.clone();
            crate::worker::run(
                crate::worker::Services {
                    runtime,
                    images,
                    web,
                    capture: self.service.capture.clone(),
                    image_startup: self.service.image_startup.clone(),
                    wakes: self.wakes,
                    preferences: self.preferences,
                    raster_budget: self.service.raster_budget.clone(),
                    negotiate_layout: self.negotiate_layout,
                    palette: self.palette_lifetime.state(),
                    video: Box::new(move || service.video_installation()),
                    audio_volume: self.audio_volume,
                    window,
                },
                self.launch,
                self.appearance,
                self.geometry,
                self.receiver,
                &self.output,
                &self.stop,
            )
        }))
        .unwrap_or_else(|_| Err("Viewer backend stopped after an internal error.".into()));
        if let Err(message) = result {
            self.output.finish_open(Err(message.clone()));
            self.output.put(ViewerOutput::Status(message));
        }
    }
}
