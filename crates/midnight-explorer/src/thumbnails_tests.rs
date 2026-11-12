//! Exercise the real timer, presenter-backed models and Slint redraw together.
use super::*;
use midnight_core::Event;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{path::PathBuf, time::Instant};

pub(super) struct TestPlatform(pub(super) Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

#[test]
fn panel_decode_edges_follow_dpi_while_custom_command_icons_keep_their_physical_size() {
    let mut target = ThumbnailTarget {
        identity: "panel:fixture".into(),
        content: midnight_ui::ThumbnailContent::Native("picture.png".into()),
        edge: 72,
        selected: false,
        hovered: false,
        confirmed_ssd: true,
    };
    assert_eq!(physical_edge(&target, 1.0), 96);
    assert_eq!(physical_edge(&target, 2.0), 192);
    target.edge = 256;
    assert_eq!(physical_edge(&target, 1.5), 384);
    assert_eq!(physical_edge(&target, 4.0), 512);
    assert_eq!(physical_edge(&target, f32::NAN), 256);
    target.identity = "command-icon:fixture".into();
    assert_eq!(physical_edge(&target, 4.0), 128);
}

#[test]
fn selection_and_size_keep_source_tokens_but_changed_content_replaces_them() {
    let old = ThumbnailTarget {
        identity: "row".into(),
        content: midnight_ui::ThumbnailContent::Native("picture.png".into()),
        edge: 96,
        selected: false,
        hovered: false,
        confirmed_ssd: true,
    };
    let mut changed = old.clone();
    changed.edge = 256;
    changed.selected = true;
    changed.hovered = true;
    let mut tokens = HashMap::from([(old.identity.clone(), 1)]);
    retain_source_tokens(&mut tokens, std::slice::from_ref(&old), &[changed.clone()]);
    assert_eq!(tokens.get("row"), Some(&1));
    changed.content = midnight_ui::ThumbnailContent::Native("replacement.png".into());
    retain_source_tokens(&mut tokens, &[old], &[changed]);
    assert!(
        tokens.is_empty(),
        "captured source must not survive a changed authority"
    );
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
    native.set_size(slint::PhysicalSize::new(1000, 700));
    let mut pixels = vec![slint::Rgb8Pixel::default(); 1000 * 700];
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 1000);
    });

    // An empty executable identity is invalid on every platform. Discovery fails
    // before cache creation and delivers its normal asynchronous error notice.
    let service = Rc::new(Service::start(PathBuf::new(), PathBuf::new()));
    let controller = install_service(
        &window,
        presenter,
        service.clone(),
        content::Factory {
            native: std::sync::Arc::new(|_, _| panic!("no content is requested during discovery")),
            storage: std::sync::Arc::new(|_| panic!("no storage is queried during discovery")),
        },
        Rc::new(|_| true),
    );
    // Startup waits hidden for preferences. A timer pulse in that interval must
    // not permanently stop thumbnail workers before the first visible frame.
    std::thread::sleep(Duration::from_millis(150));
    slint::platform::update_timers_and_animations();
    assert!(service.set_view(View {
        generation: 1,
        targets: Vec::new()
    }));
    window.show().unwrap();
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
