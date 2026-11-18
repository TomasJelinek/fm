use super::*;
use slint::platform::{
    Platform, PlatformError, WindowAdapter,
    software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};

struct TestPlatform;

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer))
    }
}

#[test]
fn displayed_app_theme_primes_coalesces_and_rolls_back_through_the_installed_callback() {
    slint::platform::set_platform(Box::new(TestPlatform)).unwrap();
    let window = MainWindow::new().unwrap();
    window.invoke_apply_fonts("Selected UI 文档".into(), 18, 24);
    window.invoke_apply_preferences(PreferenceTheme::SlateTeal.index(), 0);
    let received = Rc::new(RefCell::new(Vec::new()));
    let observed = received.clone();
    install_to(
        &window,
        move |appearance, theme, density, desktop, touch| {
            observed
                .borrow_mut()
                .push((appearance.clone(), theme, density, desktop, touch));
        },
    );
    {
        let updates = received.borrow();
        assert_eq!(updates.len(), 1, "installation must prime before any open");
        let (appearance, theme, density, desktop, touch) = &updates[0];
        assert_eq!(appearance.family, "Selected UI 文档");
        assert_eq!(appearance.font_size, 18.0);
        assert_eq!(*theme, PreferenceTheme::SlateTeal);
        assert_eq!(*density, UiDensity::Desktop);
        assert_eq!((*desktop, *touch), (18, 24));
        let actual = window.global::<Theme>();
        assert_eq!(appearance.foreground, rgba(actual.get_text()));
        assert_eq!(appearance.background, rgba(actual.get_pane_background()));
        assert_eq!(
            appearance.selection,
            rgba(actual.get_selection_background())
        );
        assert_eq!(appearance.accent, rgba(actual.get_accent()));
    }
    window.invoke_refresh_content_appearance();
    window.invoke_refresh_content_appearance();
    assert_eq!(received.borrow().len(), 1, "unchanged refresh is coalesced");

    window.invoke_apply_fonts("Preview UI".into(), 20, 28);
    window.invoke_apply_preferences(PreferenceTheme::SolarizedLight.index(), 1);
    window.invoke_refresh_content_appearance();
    {
        let updates = received.borrow();
        assert_eq!(updates.len(), 2);
        let (appearance, theme, density, desktop, touch) = &updates[1];
        assert_eq!(appearance.family, "Preview UI");
        assert_eq!(appearance.font_size, 28.0);
        assert_eq!(*theme, PreferenceTheme::SolarizedLight);
        assert_eq!(*density, UiDensity::Touch);
        assert_eq!((*desktop, *touch), (20, 28));
        assert_ne!(appearance.background, updates[0].0.background);
        assert_ne!(appearance.accent, updates[0].0.accent);
    }

    window.invoke_apply_fonts("Selected UI 文档".into(), 18, 24);
    window.invoke_apply_preferences(PreferenceTheme::SlateTeal.index(), 0);
    window.invoke_refresh_content_appearance();
    {
        let updates = received.borrow();
        assert_eq!(updates.len(), 3);
        assert_eq!(updates[2].0.family, updates[0].0.family);
        assert_eq!(updates[2].0.font_size, updates[0].0.font_size);
        assert_eq!(updates[2].0.background, updates[0].0.background);
        assert_eq!(updates[2].0.accent, updates[0].0.accent);
        assert_eq!(updates[2].1, updates[0].1);
    }
    let weak = window.as_weak();
    drop(window);
    assert!(
        weak.upgrade().is_none(),
        "the callback must not retain its window"
    );
}
