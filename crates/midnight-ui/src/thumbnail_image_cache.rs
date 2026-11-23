//! Retain visited thumbnails within fixed entry and decoded-pixel budgets.
use super::ThumbnailState;
use slint::Image;
use std::collections::HashSet;

const MAX_IMAGES: usize = 4096;
const MAX_BYTES: usize = 54 * 1024 * 1024;

fn bytes(image: &Image) -> usize {
    let size = image.size();
    size.width as usize * size.height as usize * 4
}

impl ThumbnailState {
    pub(super) fn clear(&mut self) {
        self.images.clear();
        self.coverage.clear();
        self.archive_network_policy.clear();
        self.used.clear();
        self.order.clear();
        self.cycles.clear();
        self.bytes = 0;
    }

    /// Ticks stay unique: `order` holds exactly one entry per `used` identity.
    pub(super) fn touch(&mut self, identity: &str) {
        if self.tick == u64::MAX {
            self.renumber();
        }
        self.tick += 1;
        let tick = self.tick;
        let key = match self.used.get_mut(identity) {
            Some(used) => {
                let previous = std::mem::replace(used, tick);
                self.order
                    .remove(&previous)
                    .unwrap_or_else(|| identity.to_owned())
            }
            None => {
                self.used.insert(identity.to_owned(), tick);
                identity.to_owned()
            }
        };
        self.order.insert(tick, key);
    }

    /// Compact the use clock, preserving order (unreachable in practice).
    fn renumber(&mut self) {
        let order = std::mem::take(&mut self.order);
        for (tick, identity) in (1..).zip(order.into_values()) {
            self.used.insert(identity.clone(), tick);
            self.order.insert(tick, identity);
        }
        self.tick = self.order.len() as u64;
    }

    pub(super) fn uploaded(&mut self, identity: &str, byte_count: usize) {
        self.bytes += byte_count;
        self.touch(identity);
        self.trim();
    }

    /// The least recently used retained image after `after`, excluding `keep`.
    fn oldest(&self, after: Option<u64>, keep: impl Fn(&str) -> bool) -> Option<(u64, String)> {
        let start = after.map_or(std::ops::Bound::Unbounded, std::ops::Bound::Excluded);
        self.order
            .range((start, std::ops::Bound::Unbounded))
            .find(|(_, identity)| self.images.contains_key(*identity) && !keep(identity))
            .map(|(tick, identity)| (*tick, identity.clone()))
    }

    /// Upgrades may reclaim offscreen images, but never evict visible fallbacks to
    /// make one visible image sharper. A later cache hit can retry when space exists.
    pub(super) fn reserve(
        &mut self,
        identity: &str,
        previous_bytes: usize,
        incoming_bytes: usize,
    ) -> bool {
        let new_record = usize::from(!self.images.contains_key(identity));
        let fits = |state: &Self| {
            state.bytes - previous_bytes + incoming_bytes <= MAX_BYTES
                && state.images.len() + new_record <= MAX_IMAGES
        };
        if fits(self) {
            return true;
        }
        let visible: HashSet<String> = self
            .visible
            .iter()
            .map(|target| target.identity.clone())
            .collect();
        let mut after = None;
        while let Some((tick, oldest)) = self.oldest(after, |candidate| {
            candidate == identity || visible.contains(candidate)
        }) {
            after = Some(tick);
            self.remove(&oldest);
            if fits(self) {
                return true;
            }
        }
        false
    }

    pub(super) fn remove(&mut self, identity: &str) {
        if let Some(frames) = self.images.remove(identity) {
            self.bytes = self
                .bytes
                .saturating_sub(frames.iter().flatten().map(bytes).sum());
        }
        self.coverage.remove(identity);
        self.archive_network_policy.remove(identity);
        self.cycles.remove(identity);
        if let Some(tick) = self.used.remove(identity) {
            self.order.remove(&tick);
        }
    }

    #[cfg(test)]
    fn use_order(&self) -> Vec<&str> {
        assert!(
            self.order.len() == self.used.len()
                && self.order.iter().all(|(tick, id)| self.used[id] == *tick),
            "each used identity has exactly one ordered tick"
        );
        self.order.values().map(String::as_str).collect()
    }

    pub(super) fn trim(&mut self) {
        if self.images.len() <= MAX_IMAGES && self.bytes <= MAX_BYTES {
            return;
        }
        let visible: HashSet<String> = self
            .visible
            .iter()
            .map(|target| target.identity.clone())
            .collect();
        while self.images.len() > MAX_IMAGES || self.bytes > MAX_BYTES {
            let Some((_, identity)) = self
                .oldest(None, |candidate| visible.contains(candidate))
                .or_else(|| self.oldest(None, |_| false))
            else {
                break;
            };
            self.remove(&identity);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn use_order_is_least_recent_first_and_survives_clock_compaction() {
        let mut state = ThumbnailState {
            tick: u64::MAX - 1,
            ..Default::default()
        };
        for identity in ["a", "b", "c"] {
            state.touch(identity);
        }
        state.touch("a");
        assert_eq!(state.use_order(), ["b", "c", "a"]);
        state.remove("c");
        assert_eq!(state.use_order(), ["b", "a"]);
        state.clear();
        assert!(state.use_order().is_empty());
    }
}
