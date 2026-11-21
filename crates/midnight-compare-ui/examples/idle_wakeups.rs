//! Idle wakeup probe for a desktop session, run from the repository root:
//! `cargo run -p midnight-compare-ui --example idle_wakeups -- [seconds]`.
//!
//! Opens one text comparison in a real window, waits until its frame, font catalog and
//! geometry are presented, lets it settle, then counts winit event-loop wakeups while
//! nothing happens. Keep the pointer outside the window while it measures.
use midnight_compare_ui::{ComparatorWindow, ComparatorWindows};
use midnight_core::{FileCompareLaunch, FileCompareSource, ViewerAppearance};
use slint::winit_030::{CustomApplicationHandler, EventResult, winit};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

#[derive(Default)]
struct Counts {
    wakeups: Cell<u64>,
    timers: Cell<u64>,
    events: Cell<u64>,
}

#[derive(Default)]
struct Probe {
    counts: Counts,
    window: RefCell<Option<slint::Weak<ComparatorWindow>>>,
    started: Cell<bool>,
    seconds: Cell<u64>,
    opened: Cell<Option<Instant>>,
}

struct Observe(Rc<Probe>);
impl CustomApplicationHandler for Observe {
    fn new_events(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        cause: winit::event::StartCause,
    ) -> EventResult {
        let counts = &self.0.counts;
        counts.wakeups.set(counts.wakeups.get() + 1);
        match cause {
            winit::event::StartCause::ResumeTimeReached { .. } => {
                counts.timers.set(counts.timers.get() + 1)
            }
            winit::event::StartCause::WaitCancelled { .. } => {
                counts.events.set(counts.events.get() + 1)
            }
            _ => {}
        }
        EventResult::Propagate
    }

    fn about_to_wait(&mut self, _: &winit::event_loop::ActiveEventLoop) -> EventResult {
        let probe = self.0.clone();
        if probe.started.get() {
            return EventResult::Propagate;
        }
        let Some(window) = probe
            .window
            .borrow()
            .as_ref()
            .and_then(slint::Weak::upgrade)
        else {
            return EventResult::Propagate;
        };
        let ready = window.get_fonts_ready()
            && window.get_surface().size().width > 0
            && window.get_frame_revision() == window.get_geometry_revision()
            && window.get_status().contains("difference");
        if !ready {
            return EventResult::Propagate;
        }
        probe.started.set(true);
        eprintln!(
            "presented after {:.0} ms",
            probe
                .opened
                .get()
                .map_or(0.0, |opened| opened.elapsed().as_secs_f64() * 1000.0)
        );
        // Let trailing redraws and input batching finish before measuring.
        slint::Timer::single_shot(Duration::from_secs(1), move || {
            let counts = &probe.counts;
            let before = (
                counts.wakeups.get(),
                counts.timers.get(),
                counts.events.get(),
            );
            let started = Instant::now();
            let seconds = probe.seconds.get();
            let measured = probe.clone();
            slint::Timer::single_shot(Duration::from_secs(seconds), move || {
                let elapsed = started.elapsed().as_secs_f64();
                let counts = &measured.counts;
                // The wakeup that runs this timer ends the measurement; it is not idle work.
                let wakeups = counts.wakeups.get() - before.0 - 1;
                let timers = counts.timers.get() - before.1 - 1;
                let events = counts.events.get() - before.2;
                println!(
                    "idle_wakeups={wakeups} timer_wakeups={timers} event_wakeups={events} \
                     seconds={elapsed:.2} per_second={:.1}",
                    wakeups as f64 / elapsed
                );
                let _ = slint::quit_event_loop();
            });
        });
        EventResult::Propagate
    }
}

#[path = "../tests/support/resident.rs"]
mod resident;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let seconds = std::env::args()
        .nth(1)
        .map(|value| value.parse::<u64>())
        .transpose()?
        .unwrap_or(5);
    let probe = Rc::new(Probe::default());
    probe.seconds.set(seconds.max(1));
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .with_winit_custom_application_handler(Observe(probe.clone()))
        .select()?;
    let directory = std::env::current_dir()?.join("_build/qa/idle-wakeups");
    std::fs::create_dir_all(&directory)?;
    let paths = [directory.join("left.txt"), directory.join("right.txt")];
    let text = |changed: &str| -> String {
        (0..200)
            .map(|row| match row {
                50 | 120 => format!("{changed} row {row}\n"),
                _ => format!("shared row {row}\n"),
            })
            .collect()
    };
    std::fs::write(&paths[0], text("old"))?;
    std::fs::write(&paths[1], text("new"))?;
    let runtime = resident::TestRuntime::new(tokio::runtime::Runtime::new()?);
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
        runtime.spawner(),
    ));
    probe.opened.set(Some(Instant::now()));
    let policy = midnight_core::FileComparatorSnapshot::default();
    let window = manager.open(
        FileCompareLaunch {
            sources: paths.into_iter().map(FileCompareSource::Focused).collect(),
            disabled_plugins: Arc::from([]),
            comparators: policy.preferences,
            comparator_policy: policy.id,
        },
        ViewerAppearance {
            family: "Consolas".into(),
            font_size: 14.0,
            foreground: [225, 229, 236, 255],
            background: [25, 29, 37, 255],
            selection: [55, 86, 124, 255],
            selection_text: [0; 4],
            accent: [133, 185, 244, 255],
        },
        slint::LogicalSize::new(640.0, 420.0),
        Rc::new(|_, _| None),
    )?;
    *probe.window.borrow_mut() = Some(window);
    slint::run_event_loop()?;
    manager.close_all();
    Ok(())
}
