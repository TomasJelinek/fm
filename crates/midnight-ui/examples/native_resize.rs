//! Run explicitly on a desktop: cargo run -p midnight-ui --example native_resize.
//! Measures resize request to render submission, not compositor presentation latency.
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, install, refresh};
use slint::{
    ComponentHandle,
    winit_030::{CustomApplicationHandler, EventResult, winit},
};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

struct RenderObserver {
    pending: Rc<RefCell<Option<Instant>>>,
    samples: Rc<RefCell<Vec<f64>>>,
    render_samples: Rc<RefCell<Vec<f64>>>,
    redraw_started: Option<Instant>,
}

impl RenderObserver {
    fn finish_redraw(&mut self) {
        if let Some(render) = self.redraw_started.take()
            && let Some(start) = self.pending.borrow_mut().take()
        {
            self.samples
                .borrow_mut()
                .push(start.elapsed().as_secs_f64() * 1000.0);
            self.render_samples
                .borrow_mut()
                .push(render.elapsed().as_secs_f64() * 1000.0);
        }
    }
}

impl CustomApplicationHandler for RenderObserver {
    fn window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        _: winit::window::WindowId,
        _: Option<&winit::window::Window>,
        _: Option<&slint::Window>,
        event: &winit::event::WindowEvent,
    ) -> EventResult {
        self.finish_redraw();
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let pending = Rc::new(RefCell::new(None::<Instant>));
    let samples = Rc::new(RefCell::new(Vec::<f64>::new()));
    let render_samples = Rc::new(RefCell::new(Vec::<f64>::new()));
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        // A rendering notifier forces an extra clear-only FemtoVG flush. Its
        // per-flush pipeline eviction then recompiles shaders every frame.
        // Observe event-loop boundaries without changing the rendering path.
        .with_winit_custom_application_handler(RenderObserver {
            pending: pending.clone(),
            samples: samples.clone(),
            render_samples: render_samples.clone(),
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
            .unwrap();
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
    let weak = window.as_weak();
    let timer = slint::Timer::default();
    let mut step = 0u32;
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            if step >= 40 {
                slint::quit_event_loop().unwrap();
                return;
            }
            // Allow glyph and pipeline caches to warm before the first resize sample.
            if step >= 8 && pending.borrow().is_none() {
                *pending.borrow_mut() = Some(Instant::now());
                window.window().set_size(slint::LogicalSize::new(
                    if step.is_multiple_of(2) {
                        1240.0
                    } else {
                        1340.0
                    },
                    820.0,
                ));
            }
            step += 1;
        },
    );
    window.run()?;
    let mut samples = samples.borrow_mut();
    assert!(samples.len() >= 24, "insufficient rendered resize samples");
    samples.sort_by(f64::total_cmp);
    let profile = if cfg!(debug_assertions) {
        "debug"
    } else {
        "release"
    };
    println!(
        "native WGPU, {profile}, 10k rows/pane, resize request to render submission: n={} median={:.3}ms p95={:.3}ms",
        samples.len(),
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100]
    );
    let mut rendering = render_samples.borrow_mut();
    rendering.sort_by(f64::total_cmp);
    println!(
        "native redraw dispatch span: median={:.3}ms p95={:.3}ms",
        rendering[rendering.len() / 2],
        rendering[rendering.len() * 95 / 100]
    );
    Ok(())
}
