//! Current application presentation; document typography stays in the existing session.
use super::{ComparatorWindow, ComparatorWindows, OpenWindow, color};
use midnight_core::{PreferenceTheme, UiDensity, ViewerAppearance};
use std::cell::{Cell, RefCell};

#[derive(Clone)]
pub(super) struct Presentation {
    appearance: ViewerAppearance,
    theme: PreferenceTheme,
    density: UiDensity,
    desktop_font_size: u16,
    touch_font_size: u16,
}

impl Presentation {
    pub(super) fn new(
        appearance: &ViewerAppearance,
        theme: PreferenceTheme,
        density: UiDensity,
        desktop_font_size: u16,
        touch_font_size: u16,
    ) -> Self {
        Self {
            appearance: appearance.clone(),
            theme,
            density,
            desktop_font_size,
            touch_font_size,
        }
    }

    pub(super) fn legacy(appearance: &ViewerAppearance) -> Self {
        let light = appearance.background[..3]
            .iter()
            .map(|value| u16::from(*value))
            .sum::<u16>()
            > 384;
        let size = appearance.font_size.round().clamp(1.0, f32::from(u16::MAX)) as u16;
        Self::new(
            appearance,
            if light {
                PreferenceTheme::Light
            } else {
                PreferenceTheme::Dark
            },
            UiDensity::Desktop,
            size,
            size,
        )
    }

    fn same(&self, other: &Self) -> bool {
        self.theme == other.theme
            && self.density == other.density
            && self.desktop_font_size == other.desktop_font_size
            && self.touch_font_size == other.touch_font_size
            && self.appearance.family == other.appearance.family
            && self.appearance.font_size.to_bits() == other.appearance.font_size.to_bits()
            && colors(&self.appearance) == colors(&other.appearance)
    }

    pub(super) fn apply_window(&self, window: &ComparatorWindow) {
        window.invoke_apply_tool_theme(
            self.theme.index(),
            i32::from(self.density == UiDensity::Touch),
            self.appearance.family.as_str().into(),
            i32::from(self.desktop_font_size),
            i32::from(self.touch_font_size),
        );
        window.set_foreground(color(self.appearance.foreground));
        window.set_background_color(color(self.appearance.background));
        window.set_accent(color(self.appearance.accent));
        window.set_ui_font_family(self.appearance.family.as_str().into());
        window.set_ui_font_size(self.appearance.font_size);
    }

    pub(super) fn opening_palette(&self, document: &mut ViewerAppearance) {
        document.foreground = self.appearance.foreground;
        document.background = self.appearance.background;
        document.selection = self.appearance.selection;
        document.selection_text = self.appearance.selection_text;
        document.accent = self.appearance.accent;
    }
}

pub(super) struct PaletteState {
    colors: Cell<[[u8; 4]; 5]>,
    revision: Cell<u64>,
    /// The presentation last applied, and whether the window is active: an inactive
    /// window paints its selection with the theme's inactive pair.
    presentation: RefCell<Option<Presentation>>,
    active: Cell<bool>,
}

impl PaletteState {
    pub(super) fn new(appearance: &ViewerAppearance, revision: u64) -> Self {
        Self {
            colors: Cell::new(colors(appearance)),
            revision: Cell::new(revision),
            presentation: RefCell::new(None),
            active: Cell::new(true),
        }
    }

    /// The presentation last applied to the window, which activation changes keep.
    pub(super) fn presented(&self, presentation: Presentation) {
        *self.presentation.borrow_mut() = Some(presentation);
    }

    pub(super) fn revision(&self) -> u64 {
        self.revision.get()
    }

    pub(super) fn accepts(&self, revision: u64) -> bool {
        self.revision() == revision
    }

    /// The accepted document colors: text, page, selection, selection text, accent.
    #[cfg(test)]
    pub(super) fn colors(&self) -> [[u8; 4]; 5] {
        self.colors.get()
    }
}

impl ComparatorWindows {
    /// Apply current app chrome and palette, also priming windows opened later.
    /// The appearance font is UI-only; existing document-font choices are preserved.
    pub fn apply_appearance(
        &self,
        appearance: &ViewerAppearance,
        theme: PreferenceTheme,
        density: UiDensity,
        desktop_font_size: u16,
        touch_font_size: u16,
    ) {
        let next = Presentation::new(
            appearance,
            theme,
            density,
            desktop_font_size,
            touch_font_size,
        );
        {
            let mut current = self.appearance.borrow_mut();
            if current.as_ref().is_some_and(|old| old.same(&next)) {
                return;
            }
            *current = Some(next.clone());
        }
        // Property callbacks cannot reenter a borrow of the manager's window list.
        let windows = self.windows.borrow().clone();
        for open in windows {
            open.apply_appearance(&next);
        }
    }
}

impl OpenWindow {
    pub(super) fn apply_appearance(&self, presentation: &Presentation) {
        if self.closed.get() {
            return;
        }
        presentation.apply_window(&self.window);
        self.appearance.presented(presentation.clone());
        self.restore_palette(presentation);
    }

    /// Submit the presentation's colors, with the theme's dimmer inactive selection
    /// pair while the window is inactive.
    fn restore_palette(&self, presentation: &Presentation) {
        let mut document = presentation.appearance.clone();
        if !self.appearance.active.get() {
            document.selection = rgba(self.window.get_inactive_selection_background());
            document.selection_text = rgba(self.window.get_inactive_selection_text());
        }
        let next = colors(&document);
        if self.appearance.colors.get() != next {
            // Acceptance fences UI delivery immediately. Queue-full wakes are handled
            // by the session's latest slot, without entering/replaying ordinary input.
            if let Ok(revision) = self.session.restore_palette(&document) {
                self.appearance.colors.set(next);
                self.appearance.revision.set(revision);
            }
            // A stopped session keeps its prior accepted stamp, pixels and status.
            // In particular, appearance failure cannot rewrite an external Unknown.
        }
    }
}

/// The window was activated or deactivated: its selection is painted again with the
/// theme's active or inactive pair. Text, page and accent colors stay.
pub(super) fn activation_changed(open: &OpenWindow, active: bool) {
    if open.closed.get() || open.appearance.active.replace(active) == active {
        return;
    }
    let presentation = open.appearance.presentation.borrow().clone();
    if let Some(presentation) = presentation {
        open.restore_palette(&presentation);
    }
}

fn rgba(color: slint::Color) -> [u8; 4] {
    let color = color.to_argb_u8();
    [color.red, color.green, color.blue, color.alpha]
}

fn colors(appearance: &ViewerAppearance) -> [[u8; 4]; 5] {
    [
        appearance.foreground,
        appearance.background,
        appearance.selection,
        appearance.selection_text,
        appearance.accent,
    ]
}
