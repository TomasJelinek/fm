//! Run explicitly on a desktop: cargo run -p midnight-ui --example native_about.
//! Opens the About box over 10,000 rows per pane and measures the redraws its moving sky
//! asks for: frames per second, the redraw span (redraw request to the next event-loop
//! boundary, which includes render submission but not compositor presentation) and the
//! gaps between redraws. `MIDNIGHT_PROBE_RENDERER=software` selects the software renderer;
//! the default is FemtoVG on WGPU, as in the application. On native Wayland the window
//! asks for frame callbacks as the application's does (`NativePresentation`).
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{
    ComponentHandle,
    winit_030::{CustomApplicationHandler, EventResult, winit},
};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

/// Ticks of [`TICK`]: warm-up with the window idle, then the About box open, then closed.
const TICK: Duration = Duration::from_millis(100);
const IDLE_FROM: u32 = 5;
const OPEN_AT: u32 = 15;
const CLOSE_AT: u32 = 75;
const QUIT_AT: u32 = 85;
/// The box's opening frames (layout, glyphs, cached stars) are not the loop's.
const SETTLE: Duration = Duration::from_millis(500);

struct FrameObserver {
    presentation: midnight_ui_input::NativePresentation,
    redraws: Rc<RefCell<Vec<(Instant, f64)>>>,
    redraw_started: Option<Instant>,
}

impl FrameObserver {
    fn finish_redraw(&mut self) {
        if let Some(start) = self.redraw_started.take() {
            self.redraws
                .borrow_mut()
                .push((start, start.elapsed().as_secs_f64() * 1000.0));
        }
    }
}

impl CustomApplicationHandler for FrameObserver {
    fn window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        id: winit::window::WindowId,
        native: Option<&winit::window::Window>,
        window: Option<&slint::Window>,
        event: &winit::event::WindowEvent,
    ) -> EventResult {
        self.finish_redraw();
        self.presentation.observe(id, native, window, event);
        if matches!(event, winit::event::WindowEvent::RedrawRequested) {
            self.redraw_started = Some(Instant::now());
        }
        EventResult::Propagate
    }

    fn about_to_wait(&mut self, _: &winit::event_loop::ActiveEventLoop) -> EventResult {
        self.finish_redraw();
        EventResult::Propagate
    }
}

fn percentile(sorted: &[f64], percent: usize) -> f64 {
    sorted
        .get((sorted.len() * percent / 100).min(sorted.len().saturating_sub(1)))
        .copied()
        .unwrap_or(f64::NAN)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let renderer =
        std::env::var("MIDNIGHT_PROBE_RENDERER").unwrap_or_else(|_| "femtovg-wgpu".into());
    let redraws = Rc::new(RefCell::new(Vec::<(Instant, f64)>::new()));
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name(renderer.clone())
        .with_winit_custom_application_handler(FrameObserver {
            presentation: midnight_ui_input::NativePresentation::default(),
            redraws: redraws.clone(),
            redraw_started: None,
        })
        .select()?;
    let window = MainWindow::new()?;
    let path = std::env::current_dir()?;
    let presenter = Rc::new(RefCell::new(Presenter::new(path.clone(), path.clone())));
    let commands = Rc::new(RefCell::new(Vec::new()));
    let capture = commands.clone();
    install(
        &window,
        presenter.clone(),
        Rc::new(move |command| {
            capture.borrow_mut().push(command);
            true
        }),
    );
    for pane in [PaneId::Left, PaneId::Right] {
        window.invoke_action(pane.index() as i32, 3);
        let navigation = commands
            .borrow()
            .iter()
            .rev()
            .find_map(|command| match command {
                Command::Navigate {
                    pane: id,
                    navigation,
                    ..
                }
                | Command::Refresh {
                    pane: id,
                    navigation,
                    ..
                } if *id == pane => Some(*navigation),
                _ => None,
            })
            .ok_or("no navigation for a pane")?;
        let entries: Arc<[Entry]> = (0..10_000)
            .map(|id| {
                Entry::new(
                    EntryId(id),
                    path.join(format!("entry-{id:06}-文档.txt")),
                    EntryKind::File,
                )
            })
            .collect::<Vec<_>>()
            .into();
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
    }
    refresh(&window, &presenter.borrow());
    window
        .window()
        .set_size(slint::LogicalSize::new(1340.0, 820.0));
    let marks = Rc::new(Cell::new([None::<Instant>; 4]));
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    let mut step = 0u32;
    let tick_marks = marks.clone();
    timer.start(slint::TimerMode::Repeated, TICK, move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let mut set = tick_marks.get();
        match step {
            IDLE_FROM => set[0] = Some(Instant::now()),
            OPEN_AT => {
                set[1] = Some(Instant::now());
                window.invoke_show_about();
            }
            CLOSE_AT => {
                set[2] = Some(Instant::now());
                window.set_about_open(false);
            }
            QUIT_AT => {
                set[3] = Some(Instant::now());
                let _ = slint::quit_event_loop();
            }
            _ => {}
        }
        tick_marks.set(set);
        step += 1;
    });
    window.run()?;
    let [Some(idle), Some(open), Some(close), Some(_)] = marks.get() else {
        return Err("the probe ended before its last step".into());
    };
    let redraws = redraws.borrow();
    let idle_frames = redraws
        .iter()
        .filter(|(start, _)| *start >= idle && *start < open)
        .count();
    let measured: Vec<_> = redraws
        .iter()
        .filter(|(start, _)| *start >= open + SETTLE && *start < close)
        .collect();
    let seconds = close.duration_since(open + SETTLE).as_secs_f64();
    let mut spans: Vec<f64> = measured.iter().map(|(_, span)| *span).collect();
    spans.sort_by(f64::total_cmp);
    let mut gaps: Vec<f64> = measured
        .windows(2)
        .map(|pair| pair[1].0.duration_since(pair[0].0).as_secs_f64() * 1000.0)
        .collect();
    gaps.sort_by(f64::total_cmp);
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    let display = if std::env::var_os("WAYLAND_DISPLAY").is_some_and(|value| !value.is_empty()) {
        "Wayland"
    } else {
        "X11"
    };
    println!(
        "About sky, {renderer}, {display}, {profile}, 10k rows/pane: {} frames in {seconds:.2} s \
         = {:.1} fps; redraw span median={:.2}ms p95={:.2}ms max={:.2}ms; gap median={:.2}ms \
         p95={:.2}ms max={:.2}ms; idle window before About: {idle_frames} frames in {:.1} s",
        measured.len(),
        measured.len() as f64 / seconds,
        percentile(&spans, 50),
        percentile(&spans, 95),
        spans.last().copied().unwrap_or(f64::NAN),
        percentile(&gaps, 50),
        percentile(&gaps, 95),
        gaps.last().copied().unwrap_or(f64::NAN),
        open.duration_since(idle).as_secs_f64(),
    );
    Ok(())
}
