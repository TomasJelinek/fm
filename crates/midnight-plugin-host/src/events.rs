use crate::api::{document, messages, viewer};

#[derive(Clone, Debug)]
pub struct HostEvent {
    pub request_id: u64,
    pub generation: u64,
    pub kind: EventKind,
    /// Set on the session's fatal failure, always a `Failed` event; a session reports at
    /// most one. The host stopped the session when it recorded it: the session accepts no
    /// further request and completes none of its outstanding ones. `request_id` names the
    /// offending request, whether or not the consumer still tracks it, or
    /// [`OPEN_REQUEST_ID`](crate::OPEN_REQUEST_ID) for a failure of the session itself.
    /// `drain` returns it ahead of any older event still queued; until the dispatcher
    /// discards those, they can follow it in the same or a later drain and belong to
    /// requests that will never complete.
    pub fatal: bool,
}

#[derive(Clone, Debug)]
pub enum EventKind {
    Opened(messages::Opened),
    TextPage(viewer::TextPage),
    BinaryPage {
        page: viewer::BinaryPage,
        bytes: Vec<u8>,
    },
    DocumentPage(document::DocumentPage),
    SearchMatch(messages::SearchMatch),
    TextPosition(viewer::TextPosition),
    Progress(messages::Progress),
    Completed,
    Cancelled,
    Failed(messages::Failure),
}

impl EventKind {
    pub(crate) fn retained_bytes(&self) -> usize {
        let heap = match self {
            Self::TextPage(page) => text_heap(page),
            Self::BinaryPage { bytes, .. } => bytes.capacity(),
            Self::DocumentPage(page) => {
                page.blocks.capacity() * size_of::<document::Block>()
                    + page
                        .blocks
                        .iter()
                        .map(|block| {
                            text_heap(&block.text)
                                + block.links.capacity() * size_of::<document::Link>()
                                + block.table.as_ref().map_or(0, |row| {
                                    row.cells.capacity() * size_of::<document::TableCell>()
                                })
                                + block.strikethrough.as_ref().map_or(0, |ranges| {
                                    ranges.capacity() * size_of::<document::TextRange>()
                                })
                                + block
                                    .links
                                    .iter()
                                    .map(|link| link.target.capacity())
                                    .sum::<usize>()
                        })
                        .sum::<usize>()
            }
            Self::SearchMatch(found) => found.preview.capacity(),
            Self::Failed(error) => error.message.capacity(),
            _ => 0,
        };
        size_of::<HostEvent>() + heap
    }
}
fn text_heap(page: &viewer::TextPage) -> usize {
    page.text.capacity()
        + page.spans.capacity() * size_of::<viewer::Span>()
        + page.anchors.capacity() * size_of::<viewer::Anchor>()
}
