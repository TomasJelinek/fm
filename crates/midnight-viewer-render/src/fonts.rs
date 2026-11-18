//! Discover installed families using the same font database as the renderer.
//! Call only from a backend worker. A completed catalog is shared for the process.
use parley::FontContext;
use std::{
    sync::{Arc, Mutex, OnceLock, TryLockError},
    time::Duration,
};

static MONOSPACE: Catalog = Catalog::new();
static INSTALLED: Catalog = Catalog::new();

struct Catalog {
    names: OnceLock<Arc<[String]>>,
    initialization: Mutex<()>,
}

impl Catalog {
    const fn new() -> Self {
        Self {
            names: OnceLock::new(),
            initialization: Mutex::new(()),
        }
    }

    fn get(
        &self,
        cancelled: impl Fn() -> bool,
        discover: impl FnOnce() -> Option<Arc<[String]>>,
    ) -> Option<Arc<[String]>> {
        if cancelled() {
            return None;
        }
        if let Some(names) = self.names.get() {
            return Some(names.clone());
        }
        let _initialization = loop {
            match self.initialization.try_lock() {
                Ok(guard) => break guard,
                // A failed scan keeps its names local; the gate contains no partial data.
                Err(TryLockError::Poisoned(error)) => break error.into_inner(),
                Err(TryLockError::WouldBlock) => {
                    if cancelled() {
                        return None;
                    }
                    // Wait only on this backend worker, checking cancellation between attempts.
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        };
        if cancelled() {
            return None;
        }
        if let Some(names) = self.names.get() {
            return Some(names.clone());
        }
        let names = discover()?;
        if cancelled() {
            return None;
        }
        Some(self.names.get_or_init(|| names).clone())
    }
}

/// Whether a saved family name denotes a renderer-supported CSS generic family.
pub fn is_generic_family(family: &str) -> bool {
    parley::GenericFamily::parse(family).is_some()
}

/// Installed family names for the viewer's font chooser. Backend workers only.
pub fn installed_families(cancelled: impl Fn() -> bool) -> Option<Arc<[String]>> {
    INSTALLED.get(&cancelled, || discover_installed(&cancelled))
}

fn discover_installed(cancelled: impl Fn() -> bool) -> Option<Arc<[String]>> {
    let mut names = catalog_names(&mut FontContext::default());
    if cancelled() {
        return None;
    }
    sort_catalog(&mut names);
    if cancelled() {
        return None;
    }
    Some(names.into())
}

/// The installed family names a catalog considers: the collection's first 16,384,
/// without blank, overlong or control-character names.
fn catalog_names(fonts: &mut FontContext) -> Vec<String> {
    fonts
        .collection
        .family_names()
        .take(16_384)
        .filter(|name| {
            !name.trim().is_empty() && name.len() <= 256 && !name.chars().any(char::is_control)
        })
        .map(str::to_owned)
        .collect()
}

/// Case-insensitive order; of names that differ only in ASCII case, the first is kept.
fn sort_catalog(names: &mut Vec<String>) {
    names.sort_by_cached_key(|name| (name.to_lowercase(), name.clone()));
    names.dedup_by(|right, left| left.eq_ignore_ascii_case(right));
}

/// Code fonts with fixed-pitch metrics and glyphs for every printable ASCII character.
/// Cancellation never publishes a partial catalog. Names are literal family names.
pub fn monospace_families(cancelled: impl Fn() -> bool) -> Option<Arc<[String]>> {
    MONOSPACE.get(&cancelled, || discover_monospace(&cancelled))
}

fn discover_monospace(cancelled: impl Fn() -> bool) -> Option<Arc<[String]>> {
    let mut fonts = FontContext::default();
    let mut names = Vec::new();
    for name in catalog_names(&mut fonts) {
        if cancelled() {
            return None;
        }
        if fixed_pitch(&mut fonts, &name, &mut || {}) {
            names.push(name);
        }
    }
    if cancelled() {
        return None;
    }
    sort_catalog(&mut names);
    Some(names.into())
}

/// The catalog's predicate: the family's first face has fixed-pitch metrics and glyphs
/// for every printable ASCII character. `loaded` is called before its face is loaded.
fn fixed_pitch(fonts: &mut FontContext, name: &str, loaded: &mut impl FnMut()) -> bool {
    let Some(family) = fonts.collection.family_by_name(name) else {
        return false;
    };
    let Some(face) = family.fonts().first() else {
        return false;
    };
    loaded();
    let Some(data) = face.load(Some(&mut fonts.source_cache)) else {
        return false;
    };
    swash::FontRef::from_index(data.as_ref(), face.index() as usize).is_some_and(|font| {
        if !font.metrics(&[]).is_monospace {
            return false;
        }
        // Symbol/emoji faces may report fixed pitch but trigger proportional fallback.
        let charmap = font.charmap();
        (' '..='~').all(|character| charmap.map(character) != 0)
    })
}

/// The name [`monospace_families`] lists for the first of `candidates` it would contain,
/// matched ASCII case-insensitively as a chooser resolves a saved family, without
/// scanning the other installed families: it applies the catalog's name filters and
/// predicate and loads one face per installed candidate. Lets a caller paint its
/// preferred code font while the catalog is still being scanned. `None` when no
/// candidate qualifies or when cancelled. Backend workers only.
pub fn fixed_pitch_family(candidates: &[&str], cancelled: impl Fn() -> bool) -> Option<String> {
    first_fixed_pitch(candidates, cancelled, &mut || {})
}

fn first_fixed_pitch(
    candidates: &[&str],
    cancelled: impl Fn() -> bool,
    loaded: &mut impl FnMut(),
) -> Option<String> {
    if candidates.is_empty() || cancelled() {
        return None;
    }
    let mut fonts = FontContext::default();
    let names = catalog_names(&mut fonts);
    for candidate in candidates {
        // The catalog keeps the first qualifying name of each case-insensitive group.
        let mut group: Vec<_> = names
            .iter()
            .filter(|name| name.eq_ignore_ascii_case(candidate))
            .collect();
        group.sort_by_cached_key(|name| (name.to_lowercase(), (*name).clone()));
        for name in group {
            if cancelled() {
                return None;
            }
            if fixed_pitch(&mut fonts, name, loaded) {
                return Some(name.clone());
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "font_cache_tests.rs"]
mod cache_tests;
