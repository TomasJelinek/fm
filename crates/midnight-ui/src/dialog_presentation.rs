//! Shared presentation input; typed adapters keep their own revision caches.
use crate::Presenter;
use slint::SharedString;

pub(crate) trait DialogThemeTarget {
    fn apply_theme(
        &self,
        scheme: i32,
        density: i32,
        family: SharedString,
        desktop: i32,
        touch: i32,
    );
}

/// Apply before first show and whenever the adapter's preferences revision changes.
pub(crate) fn apply<C: DialogThemeTarget>(component: &C, presenter: &Presenter) {
    let values = &presenter.preferences.values;
    component.apply_theme(
        presenter.preferences.theme_index(),
        presenter.preferences.density_index(),
        values.font_family.as_str().into(),
        i32::from(values.desktop_font_size),
        i32::from(values.touch_font_size),
    );
}
