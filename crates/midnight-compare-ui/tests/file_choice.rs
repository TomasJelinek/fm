//! Real Slint form, worker session and keyboard input; fixtures never touch user files.
mod support;
use midnight_compare_ui::{ComparatorWindow, ComparatorWindows};
use midnight_core::{FileCompareLaunch, FileCompareSource, ViewerAppearance};
use slint::{
    ComponentHandle,
    platform::{
        Key, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{
    rc::Rc,
    sync::Arc,
    time::{Duration, Instant},
};

struct TestPicker {
    requests: std::sync::mpsc::Sender<midnight_core::PathPickerRequest>,
    responses: std::sync::Mutex<std::sync::mpsc::Receiver<Option<std::path::PathBuf>>>,
}
impl midnight_core::PathPickerService for TestPicker {
    fn pick_path(
        &self,
        request: &midnight_core::PathPickerRequest,
        _: &midnight_core::Cancellation,
    ) -> Result<Option<std::path::PathBuf>, midnight_core::FsError> {
        self.requests.send(request.clone()).unwrap();
        Ok(self
            .responses
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5))
            .unwrap())
    }
}

fn wait_until(mut condition: impl FnMut() -> bool) {
    let start = Instant::now();
    while !condition() {
        slint::platform::update_timers_and_animations();
        assert!(
            start.elapsed() < Duration::from_secs(15),
            "Comparator UI timed out"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn key(window: &ComparatorWindow, text: slint::SharedString) {
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn render(native: &MinimalSoftwareWindow, window: &ComparatorWindow, width: u32, height: u32) {
    native.set_size(slint::PhysicalSize::new(width, height));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(width as f32, height as f32),
    });
    let mut pixels = vec![slint::Rgb8Pixel::default(); (width * height) as usize];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, width as usize);
    });
    assert_eq!(
        window.window().size(),
        slint::PhysicalSize::new(width, height)
    );
    // Optional visual evidence; ordinary test runs need no artifact encoding dependencies.
    if let Some(directory) = std::env::var_os("ME_COMPARE_SCREENSHOTS") {
        let directory = std::path::PathBuf::from(directory);
        std::fs::create_dir_all(&directory).unwrap();
        let mut bytes = format!("P6\n{width} {height}\n255\n").into_bytes();
        for pixel in pixels {
            bytes.extend([pixel.r, pixel.g, pixel.b]);
        }
        std::fs::write(
            directory.join(format!("file-choice-{width}x{height}.ppm")),
            bytes,
        )
        .unwrap();
    }
}

#[test]
fn manual_form_is_resizable_edits_paths_retries_and_closes_on_escape() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(support::TestPlatform(native.clone()))).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (requests, requested) = std::sync::mpsc::channel();
    let (responses, respond) = std::sync::mpsc::channel();
    let manager = ComparatorWindows::new(
        midnight_compare_engine::Service::new(runtime.handle().clone()).with_path_picker(Arc::new(
            TestPicker {
                requests,
                responses: std::sync::Mutex::new(respond),
            },
        )),
    )
    .with_picker_owner(|_| midnight_core::PathPickerOwner::X11(42));
    let known = std::env::current_dir().unwrap().join("Cargo.toml");
    let open_window = || {
        manager
            .open(
                FileCompareLaunch {
                    sources: vec![
                        FileCompareSource::Focused(known.clone()),
                        FileCompareSource::Unspecified,
                    ],
                    disabled_plugins: Arc::from([]),
                },
                ViewerAppearance {
                    family: "sans-serif".into(),
                    font_size: 14.0,
                    foreground: [240, 240, 240, 255],
                    background: [28, 30, 38, 255],
                    selection: [55, 80, 120, 255],
                    accent: [100, 150, 220, 255],
                },
                slint::LogicalSize::new(1000.0, 700.0),
                Rc::new(|_, _| None),
            )
            .unwrap()
            .upgrade()
            .unwrap()
    };
    let window = open_window();
    render(&native, &window, 1000, 700);
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    wait_until(|| window.get_choosing_files());
    assert_eq!(window.get_file_a().as_str(), known.to_str().unwrap());
    assert!(window.get_file_b().is_empty());
    assert!(!window.get_content_focused());
    render(&native, &window, 1000, 700);
    render(&native, &window, 320, 200);
    render(&native, &window, 760, 360);
    window.invoke_browse_file(1);
    assert!(window.get_choice_busy());
    // Repeated activation while a picker owns the form must not queue another request.
    window.invoke_browse_file(0);
    window.invoke_compare_chosen_files();
    let mut request = None;
    wait_until(|| {
        requested
            .try_recv()
            .map(|value| {
                request = Some(value);
            })
            .is_ok()
    });
    let request = request.unwrap();
    assert_eq!(request.owner, midnight_core::PathPickerOwner::X11(42));
    assert!(request.title.contains("B"));
    responses.send(None).unwrap();
    wait_until(|| !window.get_choice_busy());
    assert!(window.get_file_b().is_empty());
    assert!(requested.try_recv().is_err());
    window.invoke_focus_files();
    key(&window, "invalid-relative.txt".into());
    assert_eq!(window.get_file_b(), "invalid-relative.txt");
    key(&window, Key::Return.into());
    assert!(window.get_choice_busy());
    wait_until(|| !window.get_choice_busy());
    assert!(window.get_choosing_files());
    assert!(window.get_choice_message().contains("File B"));
    assert!(window.get_choice_error());
    assert_eq!(window.get_file_b(), "invalid-relative.txt");
    // Compare can retry in the same native window; backend opens only explicitly submitted paths.
    window.invoke_browse_file(1);
    wait_until(|| requested.try_recv().is_ok());
    responses.send(Some(known.clone())).unwrap();
    wait_until(|| !window.get_choice_busy());
    assert_eq!(window.get_file_b(), known.to_str().unwrap());
    window.invoke_compare_chosen_files();
    wait_until(|| !window.get_choosing_files());
    assert!(window.get_surface().size().width > 0);
    assert!(window.get_content_focused());
    key(&window, Key::Escape.into());
    assert!(!window.window().is_visible());
    manager.close_all();
    let window = open_window();
    render(&native, &window, 320, 200);
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    wait_until(|| window.get_choosing_files());
    assert!(!window.get_content_focused());
    key(&window, Key::Escape.into());
    assert!(
        !window.window().is_visible(),
        "Escape closes the chooser with an edit focused"
    );
    manager.close_all();
}
