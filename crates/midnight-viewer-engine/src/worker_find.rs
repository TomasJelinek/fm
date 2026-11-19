//! Literal Find keeps provider byte anchors and verifies word boundaries in bounded context.
use super::*;
use crate::search_cursor::{Candidate, Context, Request};
use unicase::UniCase;
use unicode_segmentation::UnicodeSegmentation;

impl Worker {
    pub(super) fn start_find(
        &mut self,
        query: String,
        backwards: bool,
        sensitive: bool,
        whole_words: bool,
        output: &Mailbox,
    ) {
        self.interrupt_find();
        // Binary display maps byte anchors into hexadecimal columns, not source words.
        let whole_words = whole_words && self.mode != viewer::MODE_BINARY;
        self.search_cursor.configure(Request {
            query,
            backwards,
            sensitive,
            whole_words,
        });
        if self
            .search_cursor
            .request
            .as_ref()
            .is_none_or(|request| request.query.is_empty())
        {
            self.found = None;
            self.selection = None;
            self.reset_raster_context();
            self.frames.invalidate();
            self.status.clear();
            self.dirty = true;
            return;
        }
        if self.viewport_request.is_none() {
            self.continuation = None;
            self.motion = RelativeMotion::default();
            self.position_context(output);
        }
        // An unrelated initial/seek/refill remains authoritative while the search runs.
        self.submit_find(output);
    }

    fn submit_find(&mut self, output: &Mailbox) {
        let Some(request) = self.search_cursor.request.clone() else {
            return;
        };
        let anchor = self
            .search_cursor
            .resume
            .take()
            .unwrap_or_else(|| {
                self.search_cursor.begin(
                    &request.query,
                    request.sensitive,
                    request.backwards,
                    self.anchor,
                )
            })
            .min(self.length);
        #[cfg(test)]
        {
            self.search_cursor.submissions += 1;
            self.search_cursor.submitted_anchor = Some(anchor);
        }
        let Some(session) = &self.session else {
            return;
        };
        match session.search(messages::Search {
            query: request.query,
            source_anchor: anchor,
            backwards: request.backwards,
            case_sensitive: request.sensitive,
        }) {
            Ok(id) => {
                self.search_request = Some(id);
                self.status(SEARCHING.into(), output);
            }
            Err(error) => self.error(error.to_string(), output),
        }
    }

    pub(super) fn found_match(&mut self, source: viewer::SourceRange, output: &Mailbox) {
        if self.viewport_request.is_some() && self.search_cursor.candidate.is_none() {
            // Keep one result until an ordinary load/seek reaches its terminal publication.
            self.search_cursor.waiting = Some(source);
            return;
        }
        if self.search_cursor.last().is_some_and(|last| {
            self.search_cursor.request.as_ref().is_some_and(|request| {
                if request.backwards {
                    source.offset >= last
                } else {
                    source.offset <= last
                }
            })
        }) {
            self.error(
                "The viewer plugin returned a match without advancing the search.".into(),
                output,
            );
            return;
        }
        self.search_cursor.found(source.offset);
        let whole = self
            .search_cursor
            .request
            .as_ref()
            .is_some_and(|request| request.whole_words);
        if let Some(page) = &self.page
            && let Some(selection) = selection(page, &source)
            && (!whole || self.valid_find(page, selection).is_some())
        {
            if whole && self.valid_find(page, selection) == Some(false) {
                let plan = local_words(
                    page,
                    self.search_cursor.request.as_ref().unwrap(),
                    source.offset,
                );
                let page = matches!(plan, Some(LocalWords::Found(_))).then(|| page.clone());
                self.continue_find(plan, page, output);
                return;
            }
            self.search_cursor.accept(source.offset);
            self.found = Some(source);
            self.selection = Some(selection);
            self.center_find();
            self.clamp_origin();
            self.reset_raster_context();
            self.frames.invalidate();
            self.dirty = true;
            self.status.clear();
            return;
        }
        if let Some(page) = &self.search_cursor.validation
            && let Some(selected) = selection(page, &source)
            && let Some(valid) = if whole {
                self.valid_find(page, selected)
            } else {
                Some(true)
            }
        {
            if valid {
                let page = page.clone();
                self.search_cursor.accept(source.offset);
                self.found = Some(source);
                self.search_cursor.reveal = true;
                self.set_page(page, output);
            } else {
                let plan = local_words(
                    page,
                    self.search_cursor.request.as_ref().unwrap(),
                    source.offset,
                );
                let page = matches!(plan, Some(LocalWords::Found(_))).then(|| page.clone());
                self.continue_find(plan, page, output);
            }
            return;
        }
        let offset = source.offset;
        self.search_cursor.candidate = Some(Candidate {
            source,
            context: if offset == 0 {
                Context::Content
            } else {
                Context::Prefix
            },
        });
        // First obtain preceding rows, then request from a proven logical row start.
        // Starting the content packet at the match would discard its indentation.
        let mut viewport = self.page_viewport(offset, offset != 0);
        viewport.max_lines = viewport.max_lines.max(128);
        self.request_viewport(viewport, None, output);
        self.status(SEARCHING.into(), output);
    }

    /// Consume a context page without publishing it until it proves the candidate.
    pub(super) fn find_page(&mut self, page: &Page, output: &Mailbox) -> bool {
        let Some(candidate) = self.search_cursor.candidate.take() else {
            return false;
        };
        if candidate.context == Context::Prefix {
            // Include enough logical rows above the result for actual shaped centering.
            // Wrapping and document styles can make rows taller, so this conservative
            // font-em estimate overfetches a few rows within the unchanged byte budget.
            let rows = (self.geometry.height as f32
                / (self.appearance.font_size * self.geometry.scale).max(1.0))
            .ceil()
            .max(1.0) as usize
                / 2
                + 2;
            let anchor = context_start(page, candidate.source.offset, rows);
            self.search_cursor.candidate = Some(Candidate {
                context: Context::Content,
                ..candidate
            });
            let mut viewport = self.page_viewport(anchor, false);
            viewport.max_lines = viewport.max_lines.max(1024);
            self.request_viewport(viewport, None, output);
            return true;
        }
        let selected = selection(page, &candidate.source);
        let whole = self
            .search_cursor
            .request
            .as_ref()
            .is_some_and(|request| request.whole_words);
        match selected.and_then(|selected| {
            if whole {
                self.valid_find(page, selected)
            } else {
                Some(true)
            }
        }) {
            Some(true) => {
                self.search_cursor.accept(candidate.source.offset);
                self.found = Some(candidate.source);
                self.search_cursor.reveal = true;
                false
            }
            Some(false) => {
                let plan = local_words(
                    page,
                    self.search_cursor.request.as_ref().unwrap(),
                    candidate.source.offset,
                );
                self.search_cursor.validation = Some(page.clone());
                let page = matches!(plan, Some(LocalWords::Found(_))).then(|| page.clone());
                self.continue_find(plan, page, output);
                true
            }
            None => {
                if candidate.context != Context::Neighbor {
                    // A long logical row can fill the prefix packet before the match,
                    // and an ordinary page can end exactly on a match boundary. Word
                    // validation needs only adjacent scalars, not the whole row. Eight
                    // source bytes cover a neighbor plus UTF-8/UTF-16 resynchronization.
                    let anchor = candidate.source.offset.saturating_sub(8);
                    self.search_cursor.candidate = Some(Candidate {
                        context: Context::Neighbor,
                        ..candidate
                    });
                    let mut viewport = self.page_viewport(anchor, false);
                    viewport.max_lines = viewport.max_lines.max(1024);
                    self.request_viewport(viewport, None, output);
                } else {
                    self.error(
                        "Cannot reveal this match with sufficient text context.".into(),
                        output,
                    );
                }
                true
            }
        }
    }

    fn continue_find(&mut self, plan: Option<LocalWords>, page: Option<Page>, output: &Mailbox) {
        match plan {
            Some(LocalWords::Found(source)) => {
                self.search_cursor.accept(source.offset);
                self.found = Some(source);
                self.search_cursor.reveal = true;
                self.set_page(
                    page.expect("a local match retains its bounded page"),
                    output,
                );
            }
            Some(LocalWords::Skip(anchor)) => {
                self.search_cursor.resume = Some(anchor);
                self.submit_find(output);
            }
            None => self.submit_find(output),
        }
    }

    fn valid_find(&self, page: &Page, selected: (usize, usize)) -> Option<bool> {
        if !word_match(page, selected)? {
            return Some(false);
        }
        let request = self.search_cursor.request.as_ref()?;
        let actual = page.text.get(selected.0..selected.1)?;
        Some(if request.sensitive {
            actual == request.query
        } else {
            UniCase::unicode(actual) == UniCase::unicode(&request.query)
        })
    }

    pub(super) fn interrupt_find(&mut self) {
        if self.search_cursor.candidate.is_some()
            && let Some(id) = self.viewport_request.take()
            && let Some(session) = &self.session
        {
            session.cancel(id);
        }
        if let Some(id) = self.search_request.take()
            && let Some(session) = &self.session
        {
            session.cancel(id);
        }
        self.search_cursor.cancel_candidate();
        if self.status == SEARCHING {
            self.status.clear();
        }
    }

    /// Center the actual shaped range vertically; pan only when its real column is hidden.
    pub(super) fn center_find(&mut self) {
        let Some((start, end)) = self.selection else {
            return;
        };
        let Some(shaped) = &self.shaped else {
            return;
        };
        let (Ok(first), Ok(last)) = (shaped.caret_geometry(start), shaped.caret_geometry(end))
        else {
            return;
        };
        let width = self.text_width();
        self.origin[1] = ((first.y0 + first.y1) as f32 - self.geometry.height as f32) / 2.0;
        let left = first.x0.min(last.x0) as f32;
        let right = first.x1.max(last.x1) as f32;
        if left < self.origin[0] || right > self.origin[0] + width {
            self.origin[0] = (left + right - width) / 2.0;
        }
        self.origin = self.origin.map(|value| value.max(0.0));
    }
}

fn selection(page: &Page, source: &viewer::SourceRange) -> Option<(usize, usize)> {
    let end = source.offset.checked_add(source.length)?;
    let at = |source| {
        page.anchors
            .iter()
            .find(|anchor| anchor.source_offset == source)
            .map(|anchor| anchor.text_offset as usize)
    };
    Some((at(source.offset)?, at(end)?))
}

enum LocalWords {
    Found(viewer::SourceRange),
    Skip(u64),
}

/// A dense page of rejected substrings needs one linear word pass, not one provider scan
/// per substring. Preserve source overlap at unknown fragment edges. Phrase searches keep
/// the provider's literal semantics; this fast path applies only to a single Unicode word.
fn local_words(page: &Page, request: &Request, rejected: u64) -> Option<LocalWords> {
    if request.query.is_empty() || !request.query.chars().all(word_character) {
        return None;
    }
    let source_at = |index| {
        page.anchors
            .binary_search_by_key(&(index as u32), |anchor| anchor.text_offset)
            .ok()
            .map(|index| page.anchors[index].source_offset)
    };
    let mut start = None;
    let mut nearest = None;
    for (at, character) in page
        .text
        .char_indices()
        .chain(std::iter::once((page.text.len(), ' ')))
    {
        if at < page.text.len() && word_character(character) {
            start.get_or_insert(at);
            continue;
        }
        let Some(first) = start.take() else {
            continue;
        };
        let text = &page.text[first..at];
        let equal = if request.sensitive {
            text == request.query
        } else {
            UniCase::unicode(text) == UniCase::unicode(&request.query)
        };
        if !equal || word_match(page, (first, at)) != Some(true) {
            continue;
        }
        let (offset, end) = (source_at(first)?, source_at(at)?);
        if if request.backwards {
            end <= rejected
        } else {
            offset > rejected
        } {
            let source = viewer::SourceRange {
                offset,
                length: end.checked_sub(offset)?,
            };
            // Semantic/safe display text may share source anchors. A local token
            // is usable only if its range maps back to exactly the same text.
            if source.length == 0 || selection(page, &source) != Some((first, at)) {
                return None;
            }
            if !request.backwards {
                return Some(LocalWords::Found(source));
            }
            nearest = Some(source);
        }
    }
    if let Some(source) = nearest {
        return Some(LocalWords::Found(source));
    }
    let overlap = (request.query.len() as u64)
        .saturating_mul(4)
        .saturating_add(8);
    let anchor = if request.backwards {
        if page.before {
            page.start.saturating_add(overlap).min(page.end)
        } else {
            page.start
        }
    } else if page.after {
        page.end.saturating_sub(overlap).max(page.start)
    } else {
        page.end
    };
    (if request.backwards {
        anchor < rejected
    } else {
        anchor > rejected.saturating_add(1)
    })
    .then_some(LocalWords::Skip(anchor))
}

fn context_start(page: &Page, before: u64, rows: usize) -> u64 {
    let end = page
        .anchors
        .iter()
        .rfind(|anchor| anchor.source_offset <= before)
        .map_or(0, |anchor| anchor.text_offset as usize);
    page.text[..end]
        .char_indices()
        .rev()
        .filter(|(_, ch)| matches!(ch, '\n' | '\u{2028}' | '\u{2029}'))
        .nth(rows)
        .and_then(|(at, ch)| {
            page.anchors
                .iter()
                .find(|anchor| anchor.text_offset as usize == at + ch.len_utf8())
        })
        .map_or(page.start, |anchor| anchor.source_offset)
}

/// Unknown fragment boundaries are never silently treated as file/word boundaries.
fn word_match(page: &Page, selected: (usize, usize)) -> Option<bool> {
    let (start, end) = selected;
    let before = page.text.get(..start)?.chars().next_back();
    let after = page.text.get(end..)?.chars().next();
    if before.is_none() && page.before || after.is_none() && page.after {
        return None;
    }
    Some(!before.is_some_and(word_character) && !after.is_some_and(word_character))
}

fn word_character(value: char) -> bool {
    if value.is_ascii() {
        return value.is_ascii_alphanumeric() || value == '_';
    }
    if value.is_alphanumeric() {
        return true;
    }
    let mut bytes = [0; 5];
    bytes[0] = b'a';
    let length = 1 + value.encode_utf8(&mut bytes[1..]).len();
    std::str::from_utf8(&bytes[..length])
        .ok()
        .is_some_and(|text| {
            text.unicode_words()
                .next()
                .is_some_and(|word| word.len() == text.len())
        })
}

#[cfg(test)]
#[path = "worker_find_tests.rs"]
mod tests;
