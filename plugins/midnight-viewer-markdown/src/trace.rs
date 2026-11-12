//! Opt-in backend diagnostics; numeric bounds only, never document text or paths.
use crate::{page_cache, retained_context::Retained};
use midnight_plugin_sdk::api::{document::DocumentPage, messages::Viewport};
use std::sync::OnceLock;

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os("MIDNIGHT_MARKDOWN_TRACE").is_some())
}

pub(crate) fn request(kind: &str, viewport: &Viewport) {
    if enabled() {
        eprintln!(
            "markdown_trace kind={kind} anchor={} backwards={} bytes={} lines={}",
            viewport.source_anchor, viewport.backwards, viewport.max_bytes, viewport.max_lines
        );
    }
}

pub(crate) fn retained(raw: &DocumentPage, retained: Option<&Retained>) {
    if enabled() {
        let (start, length, bytes, blocks) = retained.map_or((0, 0, 0, 0), |retained| {
            let page = retained.page();
            (
                page.source.offset,
                page.source.length,
                retained.bytes(),
                page.blocks.len(),
            )
        });
        eprintln!(
            "markdown_trace kind=retained raw_start={} raw_length={} raw_bytes={} \
             kept_start={start} kept_length={length} kept_bytes={bytes} kept_blocks={blocks}",
            raw.source.offset,
            raw.source.length,
            page_cache::size(raw)
        );
    }
}
