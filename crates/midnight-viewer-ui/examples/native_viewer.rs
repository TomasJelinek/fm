//! Native UI smoke probe; deploy its executable beside the three viewer packages.
use midnight_core::*;
use midnight_viewer_ui::ViewerWindows;
use slint::{
    ComponentHandle,
    platform::{Key, WindowEvent},
};
use std::{
    cell::{Cell, RefCell},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

fn key(window: &midnight_viewer_ui::ViewerWindow, text: impl Into<slint::SharedString>) {
    let text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let paths: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if paths.len() != 4 {
        return Err("usage: native_viewer text source markdown screenshot-directory".into());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let service = midnight_viewer_engine::Service::new(runtime.handle().clone());
    let manager = Rc::new(ViewerWindows::new(service));
    let entries: Arc<[Entry]> = paths[..3]
        .iter()
        .enumerate()
        .map(|(i, path)| Entry::new(EntryId(i as u64), path.clone(), EntryKind::File))
        .collect();
    let launch = ViewerLaunch {
        archive: None,
        focused: paths[0].clone(),
        view: Arc::new(DirectoryIndex::from_entries(&entries)),
        entries,
        basic: false,
        disabled_plugins: Arc::from([]),
    };
    let keys = Rc::new(|event: &i_slint_core::input::KeyEvent, search: bool| {
        if event.text == slint::SharedString::from(Key::Escape) {
            Some("viewer.close".into())
        } else if !search && event.text == " " {
            Some("viewer.next-file".into())
        } else {
            None
        }
    });
    let window = manager.open(
        launch,
        ViewerAppearance {
            family: "monospace".into(),
            font_size: 15.0,
            foreground: [225, 229, 236, 255],
            background: [25, 29, 37, 255],
            selection: [55, 86, 124, 255],
            accent: [133, 185, 244, 255],
        },
        slint::LogicalSize::new(1000.0, 700.0),
        keys,
        Rc::new(|_| {}),
    )?;
    let start = Instant::now();
    let stage = Rc::new(Cell::new(0usize));
    let error = Rc::new(RefCell::new(None));
    let result = error.clone();
    let output = paths[3].clone();
    let writer = runtime.handle().clone();
    let writes = Rc::new(RefCell::new(Vec::new()));
    let pending = writes.clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(100),
        move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            if start.elapsed() > Duration::from_secs(40) {
                *result.borrow_mut() = Some(format!(
                    "stage {} timed out: {}",
                    stage.get(),
                    window.get_status()
                ));
                let _ = slint::quit_event_loop();
                return;
            }
            let index = stage.get();
            if index >= 3 {
                return;
            }
            let text = window.get_accessible_text();
            let expected = ["FIRST", "fn main", "Heading"][index];
            if !text.contains(expected) {
                return;
            }
            eprintln!(
                "Native viewport {index}: {:?}, {}",
                start.elapsed(),
                window.get_status()
            );
            match window.window().take_snapshot() {
                Ok(image) => {
                    let path = output.join(format!("viewer-{index}.png"));
                    pending.borrow_mut().push(writer.spawn_blocking(
                        move || -> Result<(), String> {
                            std::fs::create_dir_all(path.parent().unwrap())
                                .map_err(|e| e.to_string())?;
                            let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
                            let mut encoder =
                                png::Encoder::new(file, image.width(), image.height());
                            encoder.set_color(png::ColorType::Rgba);
                            encoder.set_depth(png::BitDepth::Eight);
                            encoder
                                .write_header()
                                .map_err(|e| e.to_string())?
                                .write_image_data(image.as_bytes())
                                .map_err(|e| e.to_string())
                        },
                    ));
                }
                Err(failure) => {
                    *result.borrow_mut() = Some(failure.to_string());
                    let _ = slint::quit_event_loop();
                    return;
                }
            }
            stage.set(index + 1);
            if index < 2 {
                key(&window, " ");
            } else {
                window.invoke_focus_search();
                key(&window, "hello");
                key(&window, " ");
                key(&window, "world");
                if window.get_query() != "hello world" {
                    *result.borrow_mut() = Some("Search input lost spaces".into());
                }
                key(&window, Key::Escape);
                if window.window().is_visible() {
                    *result.borrow_mut() = Some("Escape did not close viewer".into());
                }
                let _ = slint::quit_event_loop();
            }
        },
    );
    slint::run_event_loop()?;
    manager.close_all();
    timer.stop();
    for write in writes.borrow_mut().drain(..) {
        runtime.block_on(write)??;
    }
    if let Some(message) = error.borrow_mut().take() {
        return Err(message.into());
    }
    Ok(())
}
