//! Native smoke: run beside trusted viewers/, pass two sources and an owned output directory.
use midnight_compare_ui::ComparatorWindows;
use midnight_core::{FileCompareLaunch, FileCompareSource, ViewerAppearance};
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    io::Write,
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<_> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.len() != 3 {
        return Err("usage: native_comparator left right output-directory".into());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    let manager = Rc::new(ComparatorWindows::new(
        midnight_compare_engine::Service::new(runtime.handle().clone()),
    ));
    let window = manager.open(
        FileCompareLaunch {
            sources: paths[..2]
                .iter()
                .cloned()
                .map(FileCompareSource::Focused)
                .collect(),
            disabled_plugins: Arc::from([]),
        },
        ViewerAppearance {
            family: "monospace".into(),
            font_size: 15.0,
            foreground: [225, 229, 236, 255],
            background: [25, 29, 37, 255],
            selection: [55, 86, 124, 255],
            accent: [133, 185, 244, 255],
        },
        slint::LogicalSize::new(1100.0, 720.0),
        Rc::new(|chord, control| {
            midnight_core::ShortcutMap::new(&[])
                .ok()?
                .resolve(
                    chord,
                    if control {
                        midnight_core::ShortcutContext::ViewerSearch
                    } else {
                        midnight_core::ShortcutContext::Viewer
                    },
                )
                .map(|action| action.id.clone())
        }),
    )?;
    let start = Instant::now();
    let stage = Rc::new(Cell::new(0));
    let last = Rc::new(Cell::new(start));
    let failure = Rc::new(RefCell::new(None));
    let failed = failure.clone();
    let writes = Rc::new(RefCell::new(Vec::new()));
    let pending = writes.clone();
    let writer = runtime.handle().clone();
    let directory = paths[2].clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            if start.elapsed() > Duration::from_secs(40) {
                *failed.borrow_mut() =
                    Some(format!("Comparison timed out: {}", window.get_status()));
                let _ = slint::quit_event_loop();
                return;
            }
            if window.get_surface().size().width == 0
                || window.get_geometry_revision() != window.get_frame_revision()
                || (window.get_image_mode() && window.get_rendered_mode() != stage.get())
                || last.get().elapsed() < Duration::from_millis(700)
            {
                return;
            }
            let image = match window.window().take_snapshot() {
                Ok(image) => image,
                Err(error) => {
                    *failed.borrow_mut() = Some(error.to_string());
                    let _ = slint::quit_event_loop();
                    return;
                }
            };
            let path = directory.join(format!("comparison-{}.bmp", stage.get()));
            pending
                .borrow_mut()
                .push(writer.spawn_blocking(move || -> Result<(), String> {
                    std::fs::create_dir_all(path.parent().ok_or("Screenshot parent")?)
                        .map_err(|error| error.to_string())?;
                    let mut file = std::fs::OpenOptions::new()
                        .create_new(true)
                        .write(true)
                        .open(path)
                        .map_err(|error| error.to_string())?;
                    let mut bytes = b"BM".to_vec();
                    bytes.extend((54 + image.width() * image.height() * 4).to_le_bytes());
                    bytes.extend([0u8; 4]);
                    bytes.extend(54u32.to_le_bytes());
                    bytes.extend(40u32.to_le_bytes());
                    bytes.extend(image.width().to_le_bytes());
                    bytes.extend((-(image.height() as i32)).to_le_bytes());
                    bytes.extend(1u16.to_le_bytes());
                    bytes.extend(32u16.to_le_bytes());
                    bytes.extend([0u8; 24]);
                    for pixel in image.as_bytes().as_chunks::<4>().0 {
                        bytes.extend([pixel[2], pixel[1], pixel[0], 255]);
                    }
                    file.write_all(&bytes).map_err(|error| error.to_string())
                }));
            eprintln!("Comparator stage {}: {}", stage.get(), window.get_status());
            if window.get_image_mode() && stage.get() < 2 {
                stage.set(stage.get() + 1);
                last.set(Instant::now());
                window.set_mode(stage.get());
                window.invoke_change_mode(stage.get());
            } else {
                window.window().dispatch_event(WindowEvent::KeyPressed {
                    text: Key::Escape.into(),
                });
                window.window().dispatch_event(WindowEvent::KeyReleased {
                    text: Key::Escape.into(),
                });
                if window.window().is_visible() {
                    *failed.borrow_mut() = Some("Escape did not close comparison".into());
                }
                let _ = slint::quit_event_loop();
            }
        },
    );
    slint::run_event_loop()?;
    timer.stop();
    manager.close_all();
    for write in writes.borrow_mut().drain(..) {
        runtime.block_on(write)??;
    }
    if let Some(error) = failure.borrow_mut().take() {
        return Err(error.into());
    }
    Ok(())
}
