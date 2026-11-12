//! Installed Chromium frontend qualification. Run beside a trusted viewers/ installation.
use midnight_core::*;
use midnight_viewer_ui::{ViewerWindow, ViewerWindows};
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
#[cfg(target_os = "linux")]
#[path = "support/native_html_linux.rs"]
mod native_linux;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if arguments.len() != 2 {
        return Err("usage: native_html_viewer fixture.html report-directory".into());
    }
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    #[cfg(target_os = "linux")]
    let service = native_linux::service(runtime.handle().clone());
    #[cfg(not(target_os = "linux"))]
    let service = midnight_viewer_engine::Service::new(runtime.handle().clone());
    let manager = Rc::new(ViewerWindows::new(service));
    let path = arguments[0].clone();
    let output = arguments[1].clone();
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    let bindings = ShortcutMap::new(&[])?;
    let window = manager.open(
        ViewerLaunch {
            find: None,
            associations: Default::default(),
            tree: None,
            generated: None,
            preferences: Arc::new(Default::default()),
            archive: None,
            focused: path,
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            basic: false,
            disabled_plugins: Arc::from([]),
        },
        ViewerAppearance {
            family: "sans-serif".into(),
            font_size: 14.0,
            foreground: [225, 229, 236, 255],
            background: [25, 29, 37, 255],
            selection: [55, 86, 124, 255],
            accent: [133, 185, 244, 255],
        },
        slint::LogicalSize::new(1000.0, 720.0),
        Rc::new(move |chord, search| {
            bindings
                .resolve(
                    chord,
                    if search {
                        ShortcutContext::ViewerSearch
                    } else {
                        ShortcutContext::Viewer
                    },
                )
                .map(|action| action.id.clone())
        }),
        Rc::new(|_| {}),
    )?;
    let error = Rc::new(RefCell::new(None::<String>));
    let frames = Rc::new(Cell::new((0u64, Instant::now())));
    let first_resized = Rc::new(Cell::new(None::<Instant>));
    let resize_armed = Rc::new(Cell::new(false));
    if let Some(window) = window.upgrade() {
        let frames = frames.clone();
        let first_resized = first_resized.clone();
        let resize_armed = resize_armed.clone();
        let weak = window.as_weak();
        window.on_frame_presented(move || {
            frames.set((frames.get().0 + 1, Instant::now()));
            if resize_armed.get()
                && first_resized.get().is_none()
                && let Some(window) = weak.upgrade()
            {
                let scale = window.window().scale_factor();
                let size = window.get_surface().size();
                let logical = window.window().size().to_logical(scale);
                if (logical.width - 900.0).abs() < 2.0
                    && size.width == (window.get_content_width() * scale) as u32
                    && size.height == (window.get_content_height() * scale) as u32
                    && !window.get_position_pending()
                {
                    first_resized.set(Some(Instant::now()));
                }
            }
        });
    }
    let report = error.clone();
    let stage = Rc::new(Cell::new(0));
    let began = Instant::now();
    let stage_began = Rc::new(Cell::new(began));
    let resize_frame = Rc::new(Cell::new(0u64));
    let resize_diagnostic = Rc::new(Cell::new(false));
    let writes = Rc::new(RefCell::new(Vec::new()));
    let pending = writes.clone();
    let writer = runtime.handle().clone();
    let timer = slint::Timer::default();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(40),
        move || {
            let Some(window) = window.upgrade() else {
                return;
            };
            let failure = |message: String| {
                *report.borrow_mut() = Some(message);
                let _ = slint::quit_event_loop();
            };
            if began.elapsed() > Duration::from_secs(45) {
                failure(format!(
                    "HTML viewer stage {} timed out: {}",
                    stage.get(),
                    window.get_status()
                ));
                return;
            }
            if !window.get_web_mode() {
                return;
            }
            match stage.get() {
                0 => {
                    if window.get_surface().size().width == 0 {
                        return;
                    }
                    if window.get_web_network() || window.get_web_scripts() {
                        failure("Unsafe initial browser policy".into());
                        return;
                    }
                    eprintln!(
                        "Slint HTML initial frame: {:?}; canvas={}x{}",
                        began.elapsed(),
                        window.get_content_width(),
                        window.get_content_height()
                    );
                    if let Err(message) = save(
                        &window,
                        output.join("html-viewer-initial.png"),
                        &writer,
                        &pending,
                    ) { failure(message); return; }
                    window.invoke_focus_search();
                    window.set_query("needle".into());
                    window.invoke_search(false);
                    stage.set(1);
                    stage_began.set(Instant::now());
                }
                1 => {
                    if !window.get_status().contains("Match ")
                        && !window.get_status().contains("matches")
                    {
                        return;
                    }
                    if window.get_status().contains("No matches") {
                        failure("Native Chromium search did not find fixture needle".into());
                        return;
                    }
                    eprintln!(
                        "Slint HTML find: {:?}; {}",
                        stage_began.get().elapsed(),
                        window.get_status()
                    );
                    window.invoke_focus_content();
                    stage_began.set(Instant::now());
                    resize_armed.set(true);
                    first_resized.set(None);
                    window.invoke_image_command(2);
                    window.invoke_scroll(0.0, 240.0);
                    window
                        .window()
                        .set_size(slint::LogicalSize::new(900.0, 650.0));
                    resize_frame.set(frames.get().0);
                    stage.set(2);
                }
                2 => {
                    let scale = window.window().scale_factor();
                    let size = window.get_surface().size();
                    if stage_began.get().elapsed() >= Duration::from_secs(5) && !resize_diagnostic.replace(true) {
                        eprintln!("HTML stage2 pending: surface={}x{} content={}x{} scale={} expected={}x{} pending={} frames={} resize-start={} context={} status={}",
                            size.width, size.height, window.get_content_width(), window.get_content_height(), scale,
                            (window.get_content_width() * scale) as u32, (window.get_content_height() * scale) as u32,
                            window.get_position_pending(), frames.get().0, resize_frame.get(), window.get_position_context(), window.get_status());
                    }
                    if frames.get().0 <= resize_frame.get()
                        || size.width != (window.get_content_width() * scale) as u32
                        || size.height != (window.get_content_height() * scale) as u32
                        || stage_began.get().elapsed() < Duration::from_secs(2)
                        || window.get_position_pending()
                    {
                        return;
                    }
                    if window.get_zoom_percent() <= 100.0 {
                        failure("Chromium zoom did not reach the viewer".into());
                        return;
                    }
                    eprintln!(
                        "Slint HTML scroll/resize/zoom settled: {:?}; first accepted resized frame after {:?}; last accepted frame after {:?}; canvas={}x{} zoom={}%",
                        stage_began.get().elapsed(),
                        first_resized.get().map(|time| time.duration_since(stage_began.get())),
                        frames.get().1.duration_since(stage_began.get()),
                        window.get_content_width(),
                        window.get_content_height(),
                        window.get_zoom_percent()
                    );
                    if let Err(message) = save(
                        &window,
                        output.join("html-viewer-resized.png"),
                        &writer,
                        &pending,
                    ) { failure(message); return; }
                    let text = slint::SharedString::from(Key::Escape);
                    window
                        .window()
                        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
                    window
                        .window()
                        .dispatch_event(WindowEvent::KeyReleased { text });
                    if window.window().is_visible() {
                        failure("Escape did not close HTML viewer".into());
                        return;
                    }
                    eprintln!("Slint HTML Escape closed the viewer");
                    stage.set(3);
                    let _ = slint::quit_event_loop();
                }
                _ => {}
            }
        },
    );
    slint::run_event_loop()?;
    timer.stop();
    manager.close_all();
    for write in writes.borrow_mut().drain(..) {
        runtime.block_on(write)??;
    }
    if let Some(message) = error.borrow_mut().take() {
        return Err(message.into());
    }
    Ok(())
}

type Writes = Rc<RefCell<Vec<tokio::task::JoinHandle<Result<(), String>>>>>;
fn save(
    window: &ViewerWindow,
    path: PathBuf,
    runtime: &tokio::runtime::Handle,
    pending: &Writes,
) -> Result<(), String> {
    match window.window().take_snapshot() {
        Ok(image) => pending.borrow_mut().push(runtime.spawn_blocking(move || {
            std::fs::create_dir_all(path.parent().ok_or("Screenshot directory missing")?)
                .map_err(|e| e.to_string())?;
            let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
            let mut encoder = png::Encoder::new(file, image.width(), image.height());
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder
                .write_header()
                .map_err(|e| e.to_string())?
                .write_image_data(image.as_bytes())
                .map_err(|e| e.to_string())
        })),
        Err(error) => return Err(format!("Screenshot failed: {error}")),
    }
    Ok(())
}
