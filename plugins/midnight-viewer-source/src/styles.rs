//! Directional, bounded style collection shared by foreground and state-only scans.
use midnight_plugin_sdk::api::viewer::{MAX_SPANS, STYLE_PLAIN, Span};
use std::{collections::VecDeque, ops::Range};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RoleHint {
    pub start: u64,
    pub end: u64,
    pub role: u16,
}

pub(crate) struct Styles {
    window: Range<usize>,
    origin: usize,
    backwards: bool,
    spans: VecDeque<Span>,
    clipped: bool,
    watch: Option<u64>,
    resolution: Option<RoleHint>,
}

impl Styles {
    pub(crate) fn new(window: Range<usize>, backwards: bool) -> Self {
        Self {
            window,
            origin: 0,
            backwards,
            spans: VecDeque::new(),
            clipped: false,
            watch: None,
            resolution: None,
        }
    }
    pub(crate) fn origin(&mut self, origin: usize) {
        self.origin = origin;
    }
    pub(crate) fn push(&mut self, start: usize, end: usize, role: u16) {
        self.absolute(self.origin + start, self.origin + end, role);
    }
    pub(crate) fn push_back(&mut self, consumed: u64, end: usize, role: u16) {
        self.push_pending(consumed, 0, end, role);
    }
    pub(crate) fn push_pending(&mut self, consumed: u64, trailing: u64, end: usize, role: u16) {
        let end = self.origin + end;
        let raw_end = end as u64;
        let hint = RoleHint {
            start: raw_end.saturating_sub(consumed),
            end: raw_end.saturating_sub(trailing),
            role,
        };
        if self
            .watch
            .is_some_and(|point| hint.start <= point && point <= hint.end)
        {
            self.resolution = Some(hint);
        }
        let consumed = usize::try_from(consumed).unwrap_or(usize::MAX);
        let trailing = usize::try_from(trailing).unwrap_or(usize::MAX);
        self.absolute(
            end.saturating_sub(consumed),
            end.saturating_sub(trailing),
            role,
        );
    }
    pub(crate) fn watch(&mut self, point: u64) {
        self.watch = Some(point);
        self.resolution = None;
    }
    pub(crate) fn take_resolution(&mut self) -> Option<RoleHint> {
        self.resolution.take()
    }
    fn absolute(&mut self, start: usize, end: usize, role: u16) {
        let start = start.max(self.window.start);
        let end = end.min(self.window.end);
        if role == STYLE_PLAIN || start >= end {
            return;
        }
        if let Some(last) = self
            .spans
            .back_mut()
            .filter(|s| s.end as usize == start && s.role == role)
        {
            last.end = end as u32;
            return;
        }
        if self.spans.capacity() == 0 {
            // One allocation for a line's or a small page's spans; an empty window
            // (a state-only replay) never reaches here and allocates nothing.
            self.spans.reserve(16);
        }
        if self.spans.len() == MAX_SPANS {
            self.clipped = true;
            if self.backwards {
                if let Some(first) = self.spans.pop_front() {
                    self.window.start = first.end as usize;
                }
            } else {
                self.window.end = start;
                return;
            }
        }
        self.spans.push_back(Span {
            start: start as u32,
            end: end as u32,
            role,
            flags: 0,
        });
    }
    pub(crate) fn finish(self) -> (Vec<Span>, Range<usize>, bool) {
        (Vec::from(self.spans), self.window, self.clipped)
    }
    pub(crate) fn finish_with_hints(
        mut self,
        hints: [Option<RoleHint>; 2],
    ) -> (Vec<Span>, Range<usize>, bool) {
        let hints = hints.map(|hint| {
            hint.and_then(|hint| {
                let start = hint
                    .start
                    .max(self.window.start as u64)
                    .min(self.window.end as u64) as usize;
                let end = hint
                    .end
                    .min(self.window.end as u64)
                    .max(self.window.start as u64) as usize;
                (start < end).then_some(Span {
                    start: start as u32,
                    end: end as u32,
                    role: hint.role,
                    flags: 0,
                })
            })
        });
        if hints.iter().all(Option::is_none) {
            return self.finish();
        }
        let mut output = Self::new(self.window.clone(), self.backwards);
        output.clipped = self.clipped;
        let mut at = self.window.start;
        while at < self.window.end {
            while self
                .spans
                .front()
                .is_some_and(|span| span.end as usize <= at)
            {
                self.spans.pop_front();
            }
            let current = self.spans.front();
            let mut role = current
                .filter(|span| span.start as usize <= at)
                .map_or(STYLE_PLAIN, |span| span.role);
            let mut end = self.window.end;
            if let Some(span) = current {
                end = end.min(if span.start as usize > at {
                    span.start as usize
                } else {
                    span.end as usize
                });
            }
            for span in hints.iter().flatten() {
                if span.start as usize > at {
                    end = end.min(span.start as usize);
                } else if span.end as usize > at {
                    role = span.role;
                }
                if span.end as usize > at {
                    end = end.min(span.end as usize);
                }
            }
            output.absolute(at, end, role);
            at = end;
        }
        output.finish()
    }
}

#[cfg(test)]
#[path = "styles_tests.rs"]
mod tests;
