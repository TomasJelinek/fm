//! Exercise the real timer, presenter-backed models and Slint redraw together.
use super::*;
use midnight_core::Event;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{path::PathBuf, time::Instant};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

#[test]
fn preview_initialization_failure_keeps_the_main_window_alive() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    let root = std::env::current_dir().unwrap();
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    midnight_ui::install(&window, presenter.clone(), Rc::new(|_| true));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(1000, 700));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1000 * 700];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1000);
    });

    // An empty executable identity is invalid on every platform. Discovery fails
    // before cache creation and delivers its normal asynchronous error notice.
    let service = Rc::new(Service::start(PathBuf::new(), PathBuf::new()));
    let controller = install_service(&window, presenter, service);
    let deadline = Instant::now() + Duration::from_secs(5);
    while window.get_message_generation() == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
        slint::platform::update_timers_and_animations();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 1000);
        });
    }
    assert_eq!(window.get_message_title(), "Previews");
    assert!(
        window
            .get_message_body()
            .contains("Previews are unavailable")
    );
    assert!(window.window().is_visible());
    controller.stop();
    window.hide().unwrap();
}
