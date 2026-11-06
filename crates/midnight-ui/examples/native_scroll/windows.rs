use i_slint_core::{
    item_tree::ItemRc, items::Flickable, lengths::LogicalPoint, window::WindowInner,
};
use midnight_core::*;
use midnight_ui::{MainWindow, Presenter, Theme, install, refresh};
use slint::winit_030::{CustomApplicationHandler, EventResult, winit};
use slint::{ComponentHandle, winit_030::WinitWindowAccessor};
use std::{
    cell::{Cell, RefCell},
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

#[link(name = "user32")]
unsafe extern "system" {
    fn FindWindowW(class: *const u16, name: *const u16) -> *mut std::ffi::c_void;
    fn PostMessageW(hwnd: *mut std::ffi::c_void, msg: u32, w: usize, l: isize) -> i32;
}

type Frames = Rc<RefCell<Vec<(f64, f32, bool, f64)>>>;

struct ProbeEvents {
    window: Rc<RefCell<Option<slint::Weak<MainWindow>>>>,
    list: Rc<RefCell<Option<ItemRc>>>,
    samples: Frames,
    capturing: Rc<Cell<bool>>,
    started: Instant,
    pending: Option<Instant>,
}

impl ProbeEvents {
    fn finish_redraw(&mut self) {
        if let Some(before) = self.pending.take()
            && self.capturing.get()
            && let Some(window) = self.window.borrow().as_ref().and_then(slint::Weak::upgrade)
            && let Some(item) = self.list.borrow().as_ref()
            && let Some(flick) = item.downcast::<Flickable>()
        {
            self.samples.borrow_mut().push((
                self.started.elapsed().as_secs_f64() * 1000.0,
                flick.as_pin_ref().content_y().get(),
                window.get_scroll_interaction_active(),
                before.elapsed().as_secs_f64() * 1000.0,
            ));
        }
    }
}

impl CustomApplicationHandler for ProbeEvents {
    fn window_event(
        &mut self,
        _: &winit::event_loop::ActiveEventLoop,
        _: winit::window::WindowId,
        _: Option<&winit::window::Window>,
        _: Option<&slint::Window>,
        event: &winit::event::WindowEvent,
    ) -> EventResult {
        self.finish_redraw();
        // Injected WM_MOUSEMOVE does not move the desktop cursor. Ignore the OS
        // exit notification in this fixture so it cannot cancel the test gesture.
        if matches!(event, winit::event::WindowEvent::CursorLeft { .. }) {
            return EventResult::PreventDefault;
        }
        if matches!(event, winit::event::WindowEvent::MouseWheel { .. }) {
            eprintln!(
                "native input at {:.3}ms: {event:?}",
                self.started.elapsed().as_secs_f64() * 1000.0
            );
        }
        if matches!(event, winit::event::WindowEvent::RedrawRequested) {
            self.pending = Some(Instant::now());
        }
        EventResult::Propagate
    }

    fn about_to_wait(&mut self, _: &winit::event_loop::ActiveEventLoop) -> EventResult {
        self.finish_redraw();
        EventResult::Propagate
    }
}

fn find_list(item: ItemRc, x: f32, y: f32) -> Option<ItemRc> {
    if item.downcast::<Flickable>().is_some() {
        let position = item.map_to_window(LogicalPoint::default());
        if (position.x - x).abs() < 1.0 && (position.y - y).abs() < 1.0 {
            return Some(item);
        }
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(list) = find_list(item, x, y) {
            return Some(list);
        }
    }
    None
}

fn row_background(window: &MainWindow, color: slint::Color) -> bool {
    let snapshot = window.window().take_snapshot().unwrap();
    let scale = window.window().scale_factor();
    let x = ((window.get_left_list_left() + 8.0) * scale) as usize;
    let y = ((window.get_left_list_top() + 1.5 * window.global::<Theme>().get_row_height()) * scale)
        as usize;
    let pixel = snapshot.as_slice()[y * snapshot.width() as usize + x];
    (pixel.r, pixel.g, pixel.b) == (color.red(), color.green(), color.blue())
}

fn capture_views(window: &MainWindow) {
    let snapshot = window.window().take_snapshot().unwrap();
    let width = snapshot.width();
    let height = snapshot.height();
    let stride = (width * 3 + 3) & !3;
    let length = 54 + stride * height;
    let mut bytes = Vec::with_capacity(length as usize);
    bytes.extend_from_slice(b"BM");
    bytes.extend_from_slice(&length.to_le_bytes());
    bytes.extend_from_slice(&[0; 4]);
    bytes.extend_from_slice(&54u32.to_le_bytes());
    bytes.extend_from_slice(&40u32.to_le_bytes());
    bytes.extend_from_slice(&width.to_le_bytes());
    bytes.extend_from_slice(&(-(height as i32)).to_le_bytes());
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&24u16.to_le_bytes());
    bytes.extend_from_slice(&[0; 24]);
    for row in snapshot.as_slice().chunks(width as usize) {
        for pixel in row {
            bytes.extend_from_slice(&[pixel.b, pixel.g, pixel.r]);
        }
        bytes.resize(bytes.len() + (stride - width * 3) as usize, 0);
    }
    let directory =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/qa/native-scroll");
    std::fs::create_dir_all(&directory).unwrap();
    std::fs::write(directory.join("panel-views.bmp"), bytes).unwrap();
}

fn seed_capture_icons(presenter: &Rc<RefCell<Presenter>>) {
    // The benchmark has no desktop backend. Supply existing fixture artwork
    // only after timing capture, so the visual check includes nonempty icons.
    for (key, name) in [
        (IconKey::file(), "documents.svg"),
        (IconKey::folder(), "folder-sizes.svg"),
    ] {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("ui/icons")
            .join(name);
        let image = slint::Image::load_from_path(&path)
            .unwrap()
            .to_rgba8()
            .unwrap();
        let pixels =
            IconPixels::new(image.width(), image.height(), image.as_bytes().to_vec()).unwrap();
        presenter.borrow_mut().apply(Event::IconLoaded {
            key,
            pixels: Arc::new(pixels),
        });
    }
}

fn check_frames(injections: &[f64], frames: &[(f64, f32, bool, f64)], row_height: f32) {
    for (index, &start) in injections.iter().enumerate() {
        let end = injections.get(index + 1).copied().unwrap_or(f64::INFINITY);
        let mut previous = frames
            .iter()
            .rfind(|frame| frame.0 < start)
            .map_or(0.0, |frame| frame.1);
        let initial = previous;
        let mut positions = Vec::new();
        for &(_, offset, active, _) in frames
            .iter()
            .filter(|frame| frame.0 >= start && frame.0 < end)
        {
            if active && (offset - previous).abs() > 0.01 {
                assert!(
                    (offset - previous).abs() < row_height,
                    "notch {} jumps a full row between rendered frames: {previous} -> {offset}",
                    index + 1
                );
                positions.push(offset);
                previous = offset;
            }
        }
        eprintln!(
            "notch {}: {} distinct moving frames {positions:?}",
            index + 1,
            positions.len()
        );
        assert!(
            positions.len() >= 4,
            "notch {} must render at least four pixel intermediates",
            index + 1
        );
        let last = frames
            .iter()
            .rfind(|frame| frame.0 >= start && frame.0 < end)
            .unwrap();
        assert!(
            !last.2 && (last.1 / row_height - (last.1 / row_height).round()).abs() < 0.001,
            "notch {} must settle on a row only after input becomes idle",
            index + 1
        );
        assert!(
            last.1 < initial - row_height / 2.0,
            "notch {} must advance past its starting band after snapping: {initial} -> {}",
            index + 1,
            last.1
        );
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let observed_window = Rc::new(RefCell::new(None));
    let list = Rc::new(RefCell::new(None::<ItemRc>));
    let samples: Frames = Default::default();
    let capturing = Rc::new(Cell::new(false));
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("femtovg-wgpu".into())
        .with_winit_custom_application_handler(ProbeEvents {
            window: observed_window.clone(),
            list: list.clone(),
            samples: samples.clone(),
            capturing: capturing.clone(),
            started,
            pending: None,
        })
        .select()?;
    let window = MainWindow::new()?;
    *observed_window.borrow_mut() = Some(window.as_weak());
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
            .collect();
        presenter.borrow_mut().apply(Event::ListingFinished {
            pane,
            navigation,
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        });
    }
    refresh(&window, &presenter.borrow());
    let requested_mode = std::env::args()
        .find_map(|argument| {
            argument
                .strip_prefix("--mode=")
                .and_then(|value| value.parse::<i32>().ok())
        })
        .unwrap_or(0);
    assert!((0..=2).contains(&requested_mode));
    window.invoke_view_mode(0, requested_mode);
    let row_height = window.get_left_view_row_height();
    window
        .window()
        .set_size(slint::LogicalSize::new(1340.0, 820.0));
    let timer = slint::Timer::default();
    let weak = window.as_weak();
    let mut step = 0;
    let mut injections = Vec::new();
    let capture_injections = Rc::new(RefCell::new(Vec::new()));
    let store_injections = capture_injections.clone();
    let validation = Rc::new(RefCell::new(Vec::new()));
    let validation_capture = validation.clone();
    let capture = std::env::args().any(|argument| argument == "--capture-views");
    let capture_presenter = presenter.clone();
    timer.start(
        slint::TimerMode::Repeated,
        Duration::from_millis(200),
        move || {
            let Some(window) = weak.upgrade() else {
                return;
            };
            if step == 8 {
                *list.borrow_mut() = find_list(
                    ItemRc::new_root(WindowInner::from_pub(window.window()).component()),
                    window.get_left_list_left(),
                    window.get_left_list_top(),
                );
                assert!(list.borrow().is_some());
                capturing.set(true);
            }
            if [9, 16, 23, 30].contains(&step) {
                let title = format!(
                    "Midnight Explorer native scroll probe {}",
                    std::process::id()
                );
                window
                    .window()
                    .with_winit_window(|native| native.set_title(&title));
                let title: Vec<u16> = title.encode_utf16().chain([0]).collect();
                let scale = window.window().scale_factor();
                let x = ((window.get_left_list_left() + 100.0) * scale) as u32;
                let y = ((window.get_left_list_top() + 100.0) * scale) as u32;
                unsafe {
                    let hwnd = FindWindowW(std::ptr::null(), title.as_ptr());
                    assert!(!hwnd.is_null());
                    assert_ne!(PostMessageW(hwnd, 0x0200, 0, (x | y << 16) as isize), 0);
                    assert_ne!(
                        PostMessageW(hwnd, 0x020A, (-120i16 as u16 as usize) << 16, 0),
                        0
                    );
                }
                injections.push(started.elapsed().as_secs_f64() * 1000.0);
            }
            if step == 37 {
                *store_injections.borrow_mut() = injections.clone();
                capturing.set(false);
                window.invoke_view_mode(0, 0);
                window.set_left_cursor(0);
                window.invoke_reveal_left();
            }
            if step == 38 {
                let theme = window.global::<Theme>();
                validation_capture.borrow_mut().push((
                    "ordinary row",
                    row_background(&window, theme.get_alternate_row()),
                ));
                window.invoke_activate(0);
                window.invoke_row_click(0, 1, true, false);
            }
            if step == 39 {
                let theme = window.global::<Theme>();
                validation_capture.borrow_mut().push((
                    "selected cursor row",
                    row_background(&window, theme.get_cursor_background()),
                ));
                theme.set_scheme(2);
            }
            if step == 40 {
                let theme = window.global::<Theme>();
                validation_capture.borrow_mut().push((
                    "changed theme",
                    row_background(&window, theme.get_cursor_background()),
                ));
            }
            if step == 41 && capture {
                seed_capture_icons(&capture_presenter);
                window.invoke_view_mode(0, 1);
                window.invoke_view_mode(1, 2);
            }
            if step == 43 && capture {
                capture_views(&window);
            }
            if step == (if capture { 44 } else { 41 }) {
                slint::quit_event_loop().unwrap();
            }
            step += 1;
        },
    );
    window.run()?;
    println!(
        "native WM_MOUSEWHEEL injections_ms={:?} row_height={}",
        capture_injections.borrow(),
        row_height
    );
    println!("elapsed_ms,content_y,interaction_active,render_ms");
    for &(elapsed, offset, active, render) in samples.borrow().iter() {
        println!("{elapsed:.3},{offset:.4},{active},{render:.3}");
    }
    if std::env::args().any(|arg| arg == "--check") {
        check_frames(&capture_injections.borrow(), &samples.borrow(), row_height);
    }
    assert!(
        samples
            .borrow()
            .iter()
            .any(|&(_, offset, _, _)| offset < -1.0),
        "native wheel must move the list"
    );
    for &(phase, valid) in validation.borrow().iter() {
        eprintln!("native row validation {phase}: {valid}");
        assert!(valid, "native row must reflect {phase}");
    }
    Ok(())
}
