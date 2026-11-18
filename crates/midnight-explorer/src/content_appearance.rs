//! Copy the displayed application Theme to modeless content windows, without persistence.
use midnight_compare_ui::ComparatorWindows;
use midnight_core::{FONT_SIZE_MAX, FONT_SIZE_MIN, PreferenceTheme, UiDensity, ViewerAppearance};
use midnight_ui::{MainWindow, Theme};
use midnight_viewer_ui::ViewerWindows;
use slint::{ComponentHandle, SharedString};
use std::{cell::RefCell, rc::Rc};

#[cfg(test)]
#[path = "content_appearance_tests.rs"]
mod tests;

type ThemeKey = (i32, i32, SharedString, i32, i32);

pub(crate) fn capture(window: &MainWindow) -> ViewerAppearance {
    let theme = window.global::<Theme>();
    ViewerAppearance {
        family: theme.get_effective_font_family().to_string(),
        font_size: theme.get_font_size(),
        foreground: rgba(theme.get_text()),
        background: rgba(theme.get_pane_background()),
        selection: rgba(theme.get_selection_background()),
        accent: rgba(theme.get_accent()),
    }
}

pub(crate) fn install(
    window: &MainWindow,
    viewers: &Rc<ViewerWindows>,
    comparators: &Rc<ComparatorWindows>,
) {
    let viewers = Rc::downgrade(viewers);
    let comparators = Rc::downgrade(comparators);
    install_to(window, move |appearance, theme, density, desktop, touch| {
        if let Some(viewers) = viewers.upgrade() {
            viewers.apply_appearance(appearance, theme, density, desktop, touch);
        }
        if let Some(comparators) = comparators.upgrade() {
            comparators.apply_appearance(appearance, theme, density, desktop, touch);
        }
    });
}

fn install_to(
    window: &MainWindow,
    apply: impl Fn(&ViewerAppearance, PreferenceTheme, UiDensity, u16, u16) + 'static,
) {
    let weak = window.as_weak();
    let previous = RefCell::<Option<ThemeKey>>::new(None);
    window.on_refresh_content_appearance(move || {
        let Some(window) = weak.upgrade() else {
            return;
        };
        let theme = window.global::<Theme>();
        // These inputs completely determine the displayed semantic palette and typography.
        let key = (
            theme.get_scheme(),
            theme.get_density(),
            theme.get_effective_font_family(),
            theme.get_desktop_font_size(),
            theme.get_touch_font_size(),
        );
        if previous.borrow().as_ref() == Some(&key) {
            return;
        }
        let scheme = PreferenceTheme::from_index(key.0).unwrap_or_default();
        let density = if key.1 == 1 {
            UiDensity::Touch
        } else {
            UiDensity::Desktop
        };
        let font_size =
            |value: i32| value.clamp(i32::from(FONT_SIZE_MIN), i32::from(FONT_SIZE_MAX)) as u16;
        let desktop = font_size(key.3);
        let touch = font_size(key.4);
        *previous.borrow_mut() = Some(key);
        apply(&capture(&window), scheme, density, desktop, touch);
    });
    // Startup and windows opened before the next preference event use this same snapshot.
    window.invoke_refresh_content_appearance();
}

fn rgba(color: slint::Color) -> [u8; 4] {
    let color = color.to_argb_u8();
    [color.red, color.green, color.blue, color.alpha]
}
