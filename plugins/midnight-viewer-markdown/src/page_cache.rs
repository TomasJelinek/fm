//! Revision-local, bounded semantic pages; scrolling back does not reread content.
use midnight_plugin_sdk::api::{
    document::{Block, DocumentPage},
    messages::Viewport,
};
use std::{collections::VecDeque, mem::size_of_val, sync::Arc};

const MAX_PAGES: usize = 16;
const MAX_BYTES: usize = 1024 * 1024;

#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    anchor: u64,
    backwards: bool,
    bytes: u32,
    lines: u32,
}

impl From<&Viewport> for Key {
    fn from(viewport: &Viewport) -> Self {
        Self {
            anchor: viewport.source_anchor,
            backwards: viewport.backwards,
            bytes: viewport.max_bytes,
            lines: viewport.max_lines,
        }
    }
}

struct Entry {
    key: Key,
    page: Arc<DocumentPage>,
    bytes: usize,
}

#[derive(Default)]
pub struct Cache {
    pages: VecDeque<Entry>,
    bytes: usize,
}

impl Cache {
    pub fn get(&mut self, viewport: &Viewport) -> Option<Arc<DocumentPage>> {
        let key = Key::from(viewport);
        let index = self.pages.iter().position(|entry| entry.key == key)?;
        let entry = self.pages.remove(index)?;
        let page = entry.page.clone();
        self.pages.push_back(entry);
        Some(page)
    }

    pub fn insert(&mut self, viewport: &Viewport, page: &DocumentPage) {
        let key = Key::from(viewport);
        // A concurrent request for the same immutable revision cannot replace
        // an already published slice with a different parser context.
        if self.pages.iter().any(|entry| entry.key == key) {
            return;
        }
        let bytes = size(page);
        if bytes > MAX_BYTES {
            return;
        }
        while self.pages.len() >= MAX_PAGES || self.bytes + bytes > MAX_BYTES {
            if let Some(old) = self.pages.pop_front() {
                self.bytes -= old.bytes;
            }
        }
        // Clone only the final crop. Vec/String clones retain the current
        // lengths, rather than scratch allocations for the whole parse window.
        self.pages.push_back(Entry {
            key,
            page: Arc::new(page.clone()),
            bytes,
        });
        self.bytes += bytes;
    }
}

pub(crate) fn size(page: &DocumentPage) -> usize {
    size_of_val(page) + page.blocks.iter().map(block_size).sum::<usize>()
}

pub(crate) fn block_size(block: &Block) -> usize {
    size_of_val(block)
        + block.text.text.len()
        + size_of_val(block.text.anchors.as_slice())
        + size_of_val(block.text.spans.as_slice())
        + size_of_val(block.links.as_slice())
        + block
            .links
            .iter()
            .map(|link| link.target.len())
            .sum::<usize>()
        + block
            .table
            .as_ref()
            .map_or(0, |table| size_of_val(table.cells.as_slice()))
        + block
            .strikethrough
            .as_ref()
            .map_or(0, |ranges| size_of_val(ranges.as_slice()))
}
