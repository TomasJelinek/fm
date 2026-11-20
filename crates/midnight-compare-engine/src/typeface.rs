//! The fixed-pitch family that text and byte comparisons paint in, and the chooser state.
use crate::FontChoices;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct Typeface {
    /// The chooser's catalog and selection; `None` until the catalog arrives, which keeps
    /// the chooser disabled.
    pub fonts: Option<FontChoices>,
    /// The family painted while `fonts` is `None`.
    pub family: String,
}

impl Default for Typeface {
    fn default() -> Self {
        Self {
            fonts: None,
            family: "monospace".into(),
        }
    }
}

impl Typeface {
    /// Install the fixed-pitch catalog scanned by the service's font thread and resolve the
    /// preferred family against it. Until then frames carry no font choices and paint in
    /// the family of `set_family`, so the first frame never waits for the font scan.
    pub fn set_font_catalog(&mut self, families: Arc<[String]>, preferred: &str) {
        let selected = resolve_family(&families, preferred);
        self.family.clone_from(&selected);
        self.fonts = Some(FontChoices { families, selected });
    }

    /// The family to paint before the catalog arrives, resolved by a check of at most two
    /// families (`FixedPitchFonts::resolve`).
    pub fn set_family(&mut self, family: String) {
        self.family = family;
    }

    /// The family frames are painted in.
    pub fn painted(&self) -> &str {
        self.fonts
            .as_ref()
            .map_or(self.family.as_str(), |fonts| fonts.selected.as_str())
    }

    /// Select a catalog family (or generic monospace). False when nothing changed: before
    /// the catalog, for the current family, and for a family the catalog does not offer.
    pub fn change_font(&mut self, family: &str) -> bool {
        let Some(fonts) = self.fonts.as_mut() else {
            return false;
        };
        if fonts.selected == family
            || (family != "monospace" && !fonts.families.iter().any(|name| name == family))
        {
            return false;
        }
        fonts.selected = family.into();
        true
    }
}

/// The catalog's name for the preferred family (ASCII case-insensitive), else Consolas,
/// else generic monospace.
pub(crate) fn resolve_family(families: &[String], preferred: &str) -> String {
    if preferred == "monospace" {
        return preferred.into();
    }
    families
        .iter()
        .find(|name| name.eq_ignore_ascii_case(preferred))
        .or_else(|| {
            families
                .iter()
                .find(|name| name.eq_ignore_ascii_case("Consolas"))
        })
        .cloned()
        .unwrap_or_else(|| "monospace".into())
}
