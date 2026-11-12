//! Discover installed families using the same font database as the renderer.
//! Call only from a backend worker. A completed catalog is shared for the process.
use parley::FontContext;
use std::sync::{Arc, OnceLock};

static MONOSPACE: OnceLock<Arc<[String]>> = OnceLock::new();
static INSTALLED: OnceLock<Arc<[String]>> = OnceLock::new();

/// Whether a saved family name denotes a renderer-supported CSS generic family.
pub fn is_generic_family(family: &str) -> bool {
    parley::GenericFamily::parse(family).is_some()
}

/// Installed family names for the viewer's font chooser. Backend workers only.
pub fn installed_families(cancelled: impl Fn() -> bool) -> Option<Arc<[String]>> {
    if cancelled() {
        return None;
    }
    if let Some(names) = INSTALLED.get() {
        return Some(names.clone());
    }
    let mut fonts = FontContext::default();
    let mut names: Vec<_> = fonts
        .collection
        .family_names()
        .take(16_384)
        .filter(|name| {
            !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .collect();
    if cancelled() {
        return None;
    }
    names.sort_by_cached_key(|name| (name.to_lowercase(), name.clone()));
    names.dedup_by(|right, left| left.eq_ignore_ascii_case(right));
    if cancelled() {
        return None;
    }
    let names: Arc<[String]> = names.into();
    Some(INSTALLED.get_or_init(|| names).clone())
}

/// Code fonts with fixed-pitch metrics and glyphs for every printable ASCII character.
/// Cancellation never publishes a partial catalog. Names are literal family names.
pub fn monospace_families(cancelled: impl Fn() -> bool) -> Option<Arc<[String]>> {
    if cancelled() {
        return None;
    }
    if let Some(names) = MONOSPACE.get() {
        return Some(names.clone());
    }
    let mut fonts = FontContext::default();
    let candidates: Vec<_> = fonts
        .collection
        .family_names()
        .take(16_384)
        .filter(|name| {
            !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .collect();
    let mut names = Vec::new();
    for name in candidates {
        if cancelled() {
            return None;
        }
        let Some(family) = fonts.collection.family_by_name(&name) else {
            continue;
        };
        let Some(face) = family.fonts().first() else {
            continue;
        };
        let Some(data) = face.load(Some(&mut fonts.source_cache)) else {
            continue;
        };
        if swash::FontRef::from_index(data.as_ref(), face.index() as usize).is_some_and(|font| {
            if !font.metrics(&[]).is_monospace {
                return false;
            }
            // Symbol/emoji faces may report fixed pitch but trigger proportional fallback.
            let charmap = font.charmap();
            (' '..='~').all(|character| charmap.map(character) != 0)
        }) {
            names.push(name);
        }
    }
    if cancelled() {
        return None;
    }
    names.sort_by_cached_key(|name| (name.to_lowercase(), name.clone()));
    names.dedup_by(|right, left| left.eq_ignore_ascii_case(right));
    let names: Arc<[String]> = names.into();
    Some(MONOSPACE.get_or_init(|| names).clone())
}
