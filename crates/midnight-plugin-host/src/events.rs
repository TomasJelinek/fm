use crate::api::{document, messages, viewer};

#[derive(Clone, Debug)]
pub struct HostEvent {
    pub request_id: u64,
    pub generation: u64,
    pub kind: EventKind,
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
