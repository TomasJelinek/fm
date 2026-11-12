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
    Some(names.into())
}

/// Code fonts with fixed-pitch metrics and glyphs for every printable ASCII character.
/// Cancellation never publishes a partial catalog. Names are literal family names.
pub fn monospace_families(cancelled: impl Fn() -> bool) -> Option<Arc<[String]>> {
    MONOSPACE.get(&cancelled, || discover_monospace(&cancelled))
}

fn discover_monospace(cancelled: impl Fn() -> bool) -> Option<Arc<[String]>> {
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
    Some(names.into())
}

#[cfg(test)]
#[path = "font_cache_tests.rs"]
mod cache_tests;
