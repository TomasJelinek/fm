//! Installed fonts arrive from a worker; choices and preview use only cached data.
use super::*;

#[derive(Default)]
pub(crate) struct FontsState {
    pub(crate) catalog: Vec<String>,
    pub(crate) loaded: bool,
    pub(crate) pending: bool,
}

impl Presenter {
    /// Ask once for installed families, retrying only if the bounded queue was full.
    pub(crate) fn request_fonts(&mut self, send: &dyn Fn(Command) -> bool) {
        if self.fonts.loaded || self.fonts.pending {
            return;
        }
        if send(Command::ListFontFamilies) {
            self.fonts.pending = true;
        } else {
            self.desktop.report(
                "Font list queue busy",
                "Try opening Options again to load installed font families.".into(),
            );
        }
    }

    pub(super) fn apply_font_families(&mut self, result: Result<Vec<String>, FsError>) {
        if self.fonts.loaded {
            self.retired.push(Box::new(result));
            return;
        }
        self.fonts.pending = false;
        self.fonts.loaded = true;
        match result {
            Ok(catalog) => {
                self.retired
                    .push(Box::new(mem::replace(&mut self.fonts.catalog, catalog)));
            }
            Err(error) => self.desktop.report(
                "Could not load font families",
                format!("{error}\nApplication default and the configured font remain available."),
            ),
        }
    }

    /// Catalog order is fixed after loading; absent configured names remain selectable.
    #[cfg(test)]
    pub(crate) fn font_choices(&self) -> Vec<String> {
        let family = &self.preferences.values.font_family;
        let mut choices = Vec::with_capacity(self.fonts.catalog.len() + 2);
        choices.push("Application default".into());
        choices.extend(self.fonts.catalog.iter().cloned());
        if !family.is_empty() && !self.fonts.catalog.contains(family) {
            choices.push(family.clone());
        }
        choices
    }

    pub(crate) fn font_index(&self) -> i32 {
        let family = &self.preferences.values.font_family;
        if family.is_empty() {
            return 0;
        }
        let index = self
            .fonts
            .catalog
            .iter()
            .position(|installed| installed == family)
            .unwrap_or(self.fonts.catalog.len());
        i32::try_from(index + 1).unwrap_or(0)
    }

    /// A failed enumeration also settles the catalog, enabling the retained fallbacks.
    pub(crate) fn fonts_ready(&self) -> bool {
        self.fonts.loaded
    }

    pub(crate) fn change_fonts(
        &mut self,
        index: i32,
        desktop: i32,
        touch: i32,
        field: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        let size_range = i32::from(FONT_SIZE_MIN)..=i32::from(FONT_SIZE_MAX);
        if !size_range.contains(&desktop) || !size_range.contains(&touch) {
            return;
        }
        // Size edits can be previewed while fonts load, using the current family.
        if !self.fonts_ready() && index != self.font_index() {
            return;
        }
        let Ok(index) = usize::try_from(index) else {
            return;
        };
        let family = if index == 0 {
            String::new()
        } else if let Some(family) = self.fonts.catalog.get(index - 1) {
            family.clone()
        } else if index == self.fonts.catalog.len() + 1
            && !self.preferences.values.font_family.is_empty()
            && !self
                .fonts
                .catalog
                .contains(&self.preferences.values.font_family)
        {
            self.preferences.values.font_family.clone()
        } else {
            return;
        };
        let preferences = UserPreferences {
            font_family: family,
            desktop_font_size: desktop as u16,
            touch_font_size: touch as u16,
            ..self.preferences.values.clone()
        };
        if preferences.validate().is_ok() {
            match field {
                0 => self.preferences.edited.font_family = true,
                1 => self.preferences.edited.desktop_font_size = true,
                2 => self.preferences.edited.touch_font_size = true,
                _ => {}
            }
            self.update_preferences(preferences, send);
        }
    }
}
