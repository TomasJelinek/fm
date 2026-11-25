//! Render pipelines retained across flushes, bounded by least-recent use.
//!
//! Upstream kept only the pipelines that the latest flush used. Slint flushes a
//! window's clear separately whenever a rendering notifier is installed, so the
//! next flush rebuilt every pipeline it drew with: 16 per frame and about 25 ms
//! with an NVIDIA Vulkan driver, which halved the frame rate of native Wayland
//! windows. An application draws with few distinct pipeline states (22 in the
//! viewer's scrolling probe), so the cache keeps them all up to `MAX_PIPELINES`.
//! Beyond that it drops the pipelines least recently used by earlier flushes;
//! WGPU keeps a dropped pipeline alive while recorded commands reference it.
use std::{collections::HashMap, hash::Hash};

/// About three times the distinct states of a scrolling viewer window.
const MAX_PIPELINES: usize = 64;

#[derive(Debug)]
struct Entry<V> {
    value: V,
    /// Flush in which this entry was last used.
    flush: u64,
}

#[derive(Debug)]
pub(super) struct PipelineCache<K, V> {
    entries: HashMap<K, Entry<V>>,
    flush: u64,
    #[cfg(test)]
    materializations: usize,
}

impl<K, V> Default for PipelineCache<K, V> {
    fn default() -> Self {
        Self {
            entries: HashMap::new(),
            flush: 0,
            #[cfg(test)]
            materializations: 0,
        }
    }
}

impl<K: Clone + Eq + Hash, V> PipelineCache<K, V> {
    /// Returns the value for `key`, materializing it on first use, and marks it
    /// used by the current flush.
    pub(super) fn get_or_insert_with(&mut self, key: K, materialize: impl FnOnce() -> V) -> &V {
        let flush = self.flush;
        let entry = self.entries.entry(key).or_insert_with(|| {
            #[cfg(test)]
            {
                self.materializations += 1;
            }
            Entry {
                value: materialize(),
                flush,
            }
        });
        entry.flush = flush;
        &entry.value
    }

    /// Called once after each nonempty render. Past the cap, drops the entries
    /// least recently used by earlier flushes. Entries this flush used always stay,
    /// so a flush with more distinct states than the cap keeps them all until a
    /// later flush uses fewer.
    pub(super) fn end_flush(&mut self) {
        let excess = self.entries.len().saturating_sub(MAX_PIPELINES);
        if excess > 0 {
            let current = self.flush;
            let mut stale: Vec<(u64, K)> = self
                .entries
                .iter()
                .filter(|(_, entry)| entry.flush != current)
                .map(|(key, entry)| (current.wrapping_sub(entry.flush), key.clone()))
                .collect();
            // Oldest first: the largest age since last use.
            stale.sort_unstable_by_key(|(age, _)| std::cmp::Reverse(*age));
            for (_, key) in stale.into_iter().take(excess) {
                self.entries.remove(&key);
            }
        }
        self.flush = self.flush.wrapping_add(1);
    }

    #[cfg(test)]
    fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    fn contains(&self, key: &K) -> bool {
        self.entries.contains_key(key)
    }

    #[cfg(test)]
    fn materializations(&self) -> usize {
        self.materializations
    }
}

#[cfg(test)]
#[path = "pipeline_tests.rs"]
mod tests;
