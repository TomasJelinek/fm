use midnight_compare_ui::ComparatorWindows;
use midnight_core::{FileCompareLaunch, ViewerAppearance};
use slint::{
    ComponentHandle,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{rc::Rc, sync::Arc};

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

#[test]
fn actual_comparator_window_exposes_three_modes_and_always_closes_on_escape() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let manager = ComparatorWindows::new(midnight_compare_engine::Service::new(
        runtime.handle().clone(),
    ));
    for escape in [false, true] {
        let window = manager
            .open(
                FileCompareLaunch {
                    sources: Vec::new(),
                    disabled_plugins: Arc::from([]),
                },
                ViewerAppearance {
                    family: "monospace".into(),
                    font_size: 14.0,
                    foreground: [255; 4],
                    background: [20, 20, 20, 255],
                    selection: [60, 80, 120, 255],
                    accent: [100, 150, 220, 255],
                },
                slint::LogicalSize::new(1000.0, 700.0),
                Rc::new(|chord, control| {
                    midnight_core::ShortcutMap::new(&[midnight_core::ShortcutBinding {
                        action: "viewer.close".into(),
                        keys: vec!["Ctrl+K".into()],
                    }])
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
            )
            .unwrap()
            .upgrade()
            .unwrap();
        native.set_size(slint::PhysicalSize::new(1000, 700));
        window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(1000.0, 700.0),
        });
        window
            .window()
            .dispatch_event(WindowEvent::WindowActiveChanged(true));
        let mut pixels = vec![slint::Rgb8Pixel::default(); 1000 * 700];
        for mode in 0..3 {
            window.set_image_mode(true);
            window.set_mode(mode);
            window.invoke_change_mode(mode);
            native.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, 1000);
            });
            assert!(window.get_content_width() > 900.0 && window.get_content_height() > 550.0);
        }
        window.set_compact(true);
        native.set_size(slint::PhysicalSize::new(320, 200));
        window.window().dispatch_event(WindowEvent::Resized {
            size: slint::LogicalSize::new(320.0, 200.0),
        });
        let mut compact_pixels = vec![slint::Rgb8Pixel::default(); 320 * 200];
        for mode in 0..3 {
            window.set_mode(mode);
            native.draw_if_needed(|renderer| {
                renderer.render(&mut compact_pixels, 320);
            });
            assert_eq!(window.window().size(), slint::PhysicalSize::new(320, 200));
            assert!((270.0..=308.0).contains(&window.get_content_width()));
            assert!(window.get_content_height() > 50.0);
        }
        if escape {
            // Tab moves focus into the actual mode chooser, whose native keys must not swallow Escape.
            window.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Tab.into(),
            });
            window.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Tab.into(),
            });
            assert!(
                !window.get_content_focused(),
                "Escape test must focus a toolbar control"
            );
            window.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Escape.into(),
            });
            window.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Escape.into(),
            });
        } else {
            window.window().dispatch_event(WindowEvent::KeyPressed {
                text: Key::Control.into(),
            });
            window
                .window()
                .dispatch_event(WindowEvent::KeyPressed { text: "k".into() });
            window
                .window()
                .dispatch_event(WindowEvent::KeyReleased { text: "k".into() });
            window.window().dispatch_event(WindowEvent::KeyReleased {
                text: Key::Control.into(),
            });
        }
        assert!(
            !window.window().is_visible(),
            "Escape and the additional configured close chord must both close"
        );
        manager.close_all();
    }
}
