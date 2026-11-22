//! The text of a whole-file Copy, joined from forward pages as the viewer shows them.
//!
//! Text and Source pages join end to end: their decoded text is what the viewer shows
//! (line breaks as the decoders show them, every one LF), and wrapping is layout only, so
//! nothing is added between pages. A page that starts before the copied end continues
//! at its exact anchor for that end; one that starts after it, or from another revision of
//! the file, ends the copy, because the text would no longer be the file's.
//!
//! Document (Markdown) pages join block by block as a page shows them
//! (`Page::document`): a blank line between blocks and the list, quote and rule
//! prefixes. A block the previous page already copied is skipped (Markdown pages hold
//! whole table rows around their anchor), and the rest of a block that page cut short
//! continues it without a break.
use midnight_plugin_host::api::{document, viewer};

/// The longest text a whole-file Copy hands to the clipboard: 64 MiB of UTF-8.
///
/// The text exists whole on the viewer worker while it is read, then once more in the
/// platform's clipboard. Windows keeps text as UTF-16 (`CF_UNICODETEXT`, at most two bytes
/// per UTF-8 byte, 128 MiB here) in one movable global allocation with no format limit;
/// X11 serves text longer than one request in incremental (INCR) transfers, so the
/// server's request size does not bound it. Both hold such a text, but each paste copies
/// all of it again into the receiving program, which takes seconds there beyond tens of
/// megabytes. Larger files are better copied as files (F5).
pub(crate) const LIMIT: usize = 64 * 1024 * 1024;

/// What to read after a page joined.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Next {
    /// The next forward page, from this source offset.
    Read(u64),
    /// That was the last page: the text is whole.
    Done,
}

/// Why the pages cannot make the whole file's text.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum JoinError {
    /// The text grew past the limit.
    Limit,
    /// A page came from another revision of the file than the first.
    Changed,
    /// A page does not continue where the text copied so far ends.
    Discontinuous,
    /// Memory for the text could not be reserved.
    Memory,
}

/// The text joined so far.
pub(crate) struct Joined {
    text: String,
    limit: usize,
    revision: Option<u64>,
    /// The source offset the text copied so far ends at; none before the first page.
    end: Option<u64>,
    /// The source end of the last block copied (Document pages).
    block_end: Option<u64>,
    /// The source offset of the last empty block copied, such as a rule, which a page
    /// that starts there shows again.
    empty_at: Option<u64>,
}

impl Joined {
    pub fn new(limit: usize) -> Self {
        Self {
            text: String::new(),
            limit,
            revision: None,
            end: None,
            block_end: None,
            empty_at: None,
        }
    }

    /// Bytes of text joined so far.
    pub fn len(&self) -> usize {
        self.text.len()
    }

    /// The whole text, once the last page joined.
    pub fn finish(self) -> String {
        self.text
    }

    pub fn text_page(&mut self, page: &viewer::TextPage) -> Result<Next, JoinError> {
        self.revision(page.revision)?;
        let (start, end) = range(&page.source)?;
        let from = match self.end {
            None => 0,
            Some(copied) if start == copied => 0,
            Some(copied) if start < copied && copied <= end => {
                anchor_at(&page.anchors, copied).ok_or(JoinError::Discontinuous)?
            }
            Some(_) => return Err(JoinError::Discontinuous),
        };
        let text = page.text.get(from..).ok_or(JoinError::Discontinuous)?;
        self.push(text)?;
        self.advance(end, page.has_after)
    }

    pub fn document_page(&mut self, page: &document::DocumentPage) -> Result<Next, JoinError> {
        self.revision(page.revision)?;
        let (start, end) = range(&page.source)?;
        if self.end.is_some_and(|copied| start > copied) {
            return Err(JoinError::Discontinuous);
        }
        // The previous page ended where its last block did: that block may go on here.
        let cut = self.end.is_some() && self.end == self.block_end;
        let mut first = true;
        for block in &page.blocks {
            let text = &block.text;
            self.revision(text.revision)?;
            let (block_start, block_end) = range(&text.source)?;
            if let Some(copied) = self.end {
                let empty_again = block_start == block_end && self.empty_at == Some(copied);
                if block_end < copied
                    || (block_end == copied && (block_start < copied || empty_again))
                {
                    continue;
                }
                if block_start < copied {
                    // The rest of a block the text holds in part.
                    let at = anchor_at(&text.anchors, copied).ok_or(JoinError::Discontinuous)?;
                    self.push(text.text.get(at..).ok_or(JoinError::Discontinuous)?)?;
                    self.block_end = Some(block_end);
                    first = false;
                    continue;
                }
                if first && cut && block_start == copied && block_start < block_end {
                    self.push(&text.text)?;
                    self.block_end = Some(block_end);
                    first = false;
                    continue;
                }
            }
            if !self.text.is_empty() {
                self.push("\n\n")?;
            }
            self.push(crate::page::block_prefix(block.kind))?;
            self.push(&text.text)?;
            self.block_end = Some(block_end);
            if block_start == block_end {
                self.empty_at = Some(block_start);
            }
            first = false;
        }
        let end = self.block_end.map_or(end, |block_end| end.max(block_end));
        self.advance(end, page.has_after)
    }

    fn revision(&mut self, revision: u64) -> Result<(), JoinError> {
        match self.revision {
            Some(first) if first != revision => Err(JoinError::Changed),
            _ => {
                self.revision = Some(revision);
                Ok(())
            }
        }
    }

    /// The text copied now ends at `end`; ask for more while `after` and the read moved.
    fn advance(&mut self, end: u64, after: bool) -> Result<Next, JoinError> {
        let moved = self.end.is_none_or(|copied| end > copied);
        let end = self.end.map_or(end, |copied| copied.max(end));
        self.end = Some(end);
        match (after, moved) {
            (false, _) => Ok(Next::Done),
            (true, true) => Ok(Next::Read(end)),
            (true, false) => Err(JoinError::Discontinuous),
        }
    }

    /// Append `text` within the limit; the buffer never grows past the limit.
    fn push(&mut self, text: &str) -> Result<(), JoinError> {
        let length = self
            .text
            .len()
            .checked_add(text.len())
            .filter(|&length| length <= self.limit)
            .ok_or(JoinError::Limit)?;
        if length > self.text.capacity() {
            let wanted = length
                .max(self.text.capacity().saturating_mul(2))
                .min(self.limit);
            self.text
                .try_reserve_exact(wanted - self.text.len())
                .map_err(|_| JoinError::Memory)?;
        }
        self.text.push_str(text);
        Ok(())
    }
}

fn range(source: &viewer::SourceRange) -> Result<(u64, u64), JoinError> {
    source
        .offset
        .checked_add(source.length)
        .map(|end| (source.offset, end))
        .ok_or(JoinError::Discontinuous)
}

/// The text offset of the first exact anchor at source offset `source`.
fn anchor_at(anchors: &[viewer::Anchor], source: u64) -> Option<usize> {
    let at = anchors.partition_point(|anchor| anchor.source_offset < source);
    anchors
        .get(at)
        .filter(|anchor| anchor.source_offset == source)
        .map(|anchor| anchor.text_offset as usize)
}

#[cfg(test)]
#[path = "whole_copy_tests.rs"]
mod tests;
