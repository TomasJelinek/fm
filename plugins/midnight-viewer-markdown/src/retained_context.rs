//! One compact semantic context; complete groups share the existing 1 MiB budget.
use crate::{
    markdown::{self, Window},
    page_cache, semantic_window,
};
use midnight_plugin_sdk::api::{
    document::DocumentPage,
    viewer::{Anchor, SourceRange},
};
use std::{mem::size_of_val, ops::Range};

const MAX_BYTES: usize = 1024 * 1024;

pub(crate) struct Retained {
    /// Original parse coverage, before trimming for retained memory.
    pub source: SourceRange,
    pub has_after: bool,
    page: DocumentPage,
    boundaries: Option<Box<[u8]>>,
}

#[cfg(test)]
#[path = "retained_context_tests.rs"]
mod tests;

impl Retained {
    pub fn new(
        raw: &DocumentPage,
        window: &Window,
        source_anchors: &[Anchor],
        cancelled: impl Fn() -> bool,
    ) -> Option<Self> {
        if cancelled() {
            return None;
        }
        let (page, boundaries) = if page_cache::size(raw) <= MAX_BYTES {
            // Compact parser scratch capacities, as in the original retention path.
            (raw.clone(), None)
        } else {
            let boundaries = source_boundaries(&raw.source, source_anchors, &cancelled)?;
            let selected = semantic_window::select(raw, window)?;
            let groups = semantic_window::groups(raw);
            let first = groups
                .iter()
                .position(|group| group.start == selected.start)?;
            let last = groups.iter().position(|group| group.end == selected.end)? + 1;
            let mut costs = Vec::with_capacity(groups.len());
            for group in &groups {
                if cancelled() {
                    return None;
                }
                costs.push(
                    raw.blocks[group.clone()]
                        .iter()
                        .map(page_cache::block_size)
                        .sum::<usize>(),
                );
            }
            let mut retained = first..last;
            let mut bytes =
                size_of_val(raw) + boundaries.len() + costs[retained.clone()].iter().sum::<usize>();
            if bytes > MAX_BYTES {
                // An oversized table/paragraph is never split to populate this cache.
                return None;
            }
            let mut before = true;
            loop {
                if cancelled() {
                    return None;
                }
                let left = retained
                    .start
                    .checked_sub(1)
                    .filter(|&index| bytes + costs[index] <= MAX_BYTES);
                let right = (retained.end < groups.len())
                    .then_some(retained.end)
                    .filter(|&index| bytes + costs[index] <= MAX_BYTES);
                let next = if before {
                    left.or(right)
                } else {
                    right.or(left)
                };
                let Some(index) = next else { break };
                retained.start = retained.start.min(index);
                retained.end = retained.end.max(index + 1);
                bytes += costs[index];
                before = !before;
            }
            (
                semantic_window::slice_ref(
                    raw,
                    groups[retained.start].start..groups[retained.end - 1].end,
                ),
                Some(boundaries),
            )
        };
        if cancelled() {
            return None;
        }
        Some(Self {
            source: raw.source.clone(),
            has_after: raw.has_after,
            page,
            boundaries,
        })
    }

    pub fn page(&self) -> &DocumentPage {
        &self.page
    }

    pub fn bytes(&self) -> usize {
        page_cache::size(&self.page) + self.boundaries.as_ref().map_or(0, |map| map.len())
    }

    pub fn crop(&self, window: &Window) -> Option<DocumentPage> {
        let end = self.page.source.end().ok()?;
        let trimmed_before = self.page.source.offset > self.source.offset;
        let trimmed_after = end < self.source.end().ok()?;
        if trimmed_before || trimmed_after {
            if window.anchor < self.page.source.offset || window.anchor > end {
                return None;
            }
            // Display anchors omit valid source boundaries around removed markers
            // and leading table-cell spaces. The decoder proves every raw boundary
            // without rounding a limit across a scalar or a normalized CRLF pair.
            let offset = usize::try_from(window.anchor.checked_sub(self.source.offset)?).ok()?;
            let boundaries = self.boundaries.as_ref()?;
            if boundaries.get(offset / 8)? & (1 << (offset % 8)) == 0 {
                return None;
            }
            let selected: Range<usize> = semantic_window::select(&self.page, window)?;
            if (window.backwards && trimmed_before && selected.start == 0)
                || (!window.backwards && trimmed_after && selected.end == self.page.blocks.len())
            {
                // The original parser saw more complete groups. An artificial cache
                // edge cannot satisfy a request that may still need those groups.
                return None;
            }
        }
        markdown::crop_context(&self.page, window)
    }
}

/// One bit per source byte boundary in the already bounded read, at most 8,193 bytes.
fn source_boundaries(
    source: &SourceRange,
    anchors: &[Anchor],
    cancelled: impl Fn() -> bool,
) -> Option<Box<[u8]>> {
    let length = usize::try_from(source.length).ok()?;
    if length > 65536 || cancelled() {
        return None;
    }
    let mut map = vec![0u8; (length + 1).div_ceil(8)];
    for (index, anchor) in anchors.iter().enumerate() {
        if index.is_multiple_of(128) && cancelled() {
            return None;
        }
        let offset = usize::try_from(anchor.source_offset.checked_sub(source.offset)?).ok()?;
        if offset > length {
            return None;
        }
        map[offset / 8] |= 1 << (offset % 8);
    }
    (!cancelled()).then(|| map.into_boxed_slice())
}
