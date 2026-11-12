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
        self.bytes = 0;
    }

    pub(super) fn touch(&mut self, identity: &str) {
        self.tick = self.tick.saturating_add(1);
        if let Some(used) = self.used.get_mut(identity) {
            *used = self.tick;
        } else {
            self.used.insert(identity.to_owned(), self.tick);
        }
    }

    pub(super) fn uploaded(&mut self, identity: &str, byte_count: usize) {
        self.bytes += byte_count;
        self.touch(identity);
        self.trim();
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
        let mut candidates: Vec<_> = {
            let visible: HashSet<_> = self
                .visible
                .iter()
                .map(|target| target.identity.as_str())
                .collect();
            self.images
                .keys()
                .filter(|key| key.as_str() != identity && !visible.contains(key.as_str()))
                .map(|key| (self.used.get(key).copied().unwrap_or(0), key.clone()))
                .collect()
        };
        candidates.sort_unstable_by_key(|(used, _)| *used);
        for (_, oldest) in candidates {
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
        self.used.remove(identity);
    }

    pub(super) fn trim(&mut self) {
        if self.images.len() <= MAX_IMAGES && self.bytes <= MAX_BYTES {
            return;
        }
        let visible: HashSet<_> = self
            .visible
            .iter()
            .map(|target| target.identity.clone())
            .collect();
        while self.images.len() > MAX_IMAGES || self.bytes > MAX_BYTES {
            let oldest = |offscreen: bool| {
                self.images
                    .keys()
                    .filter(|identity| !offscreen || !visible.contains(identity.as_str()))
                    .min_by_key(|identity| self.used.get(*identity).copied().unwrap_or(0))
                    .cloned()
            };
            let Some(identity) = oldest(true).or_else(|| oldest(false)) else {
                break;
            };
            self.remove(&identity);
        }
    }
}
