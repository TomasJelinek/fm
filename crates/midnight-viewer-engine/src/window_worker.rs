//! Concrete queued ownership of one window's backend work.
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
        drop(start);
        work.run();
    }
}

/// How a viewer window's resident job names itself while it runs (`resident::note`): the
/// file it was opened on, so that an exit that outlives its grace names the window.
pub(super) fn label(launch: &ViewerLaunch) -> String {
    match launch.focused.file_name() {
        Some(name) => format!("viewer window {:?}", name.to_string_lossy()),
        None => "viewer window".to_owned(),
    }
}

impl Work {
    fn run(self) {
        midnight_work_budget::resident::note(format!("{}: starting", label(&self.launch)));
        crate::open_trace::mark("worker_start");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            if self.service.fonts.get().is_none()
                && let Some(names) =
                    midnight_viewer_render::installed_families(|| self.stop.load(Ordering::Acquire))
            {
                let _ = self.service.fonts.set(names);
            }
            crate::open_trace::mark("fonts_ready");
            let (runtime, images, web) = self.service.catalogs()?;
            crate::open_trace::mark("catalogs_ready");
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
