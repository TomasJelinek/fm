//! Run explicitly on a desktop: cargo run -p fm-ui --example native_resize.
//! Measures resize request to render submission, not compositor presentation latency.
use fm_core::*;
use fm_ui::{MainWindow, Presenter, install, refresh};
use slint::{ComponentHandle, RenderingState};
use std::{
    cell::RefCell,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
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
    let pending = Rc::new(RefCell::new(None::<Instant>));
    let samples = Rc::new(RefCell::new(Vec::<f64>::new()));
    let render_samples = Rc::new(RefCell::new(Vec::<f64>::new()));
    let render_capture = render_samples.clone();
    let mut render_started = None;
    let capture = samples.clone();
    let timing = pending.clone();
    window.window().set_rendering_notifier(move |state, _| {
        if matches!(state, RenderingState::BeforeRendering) {
            render_started = Some(Instant::now());
        }
        if matches!(state, RenderingState::AfterRendering)
            && let Some(start) = timing.borrow_mut().take()
        {
            capture
                .borrow_mut()
                .push(start.elapsed().as_secs_f64() * 1000.0);
            if let Some(render) = render_started.take() {
                render_capture
                    .borrow_mut()
                    .push(render.elapsed().as_secs_f64() * 1000.0);
            }
        }
    })?;
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
    println!(
        "native WGPU, debug, 10k rows/pane, resize request to render submission: n={} median={:.3}ms p95={:.3}ms",
        samples.len(),
        samples[samples.len() / 2],
        samples[samples.len() * 95 / 100]
    );
    let mut rendering = render_samples.borrow_mut();
    rendering.sort_by(f64::total_cmp);
    println!(
        "render callback span: median={:.3}ms p95={:.3}ms",
        rendering[rendering.len() / 2],
        rendering[rendering.len() * 95 / 100]
    );
    Ok(())
}
