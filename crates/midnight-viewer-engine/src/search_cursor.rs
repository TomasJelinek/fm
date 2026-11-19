use midnight_plugin_host::api::viewer::SourceRange;

#[derive(Clone)]
pub(crate) struct Request {
    pub query: String,
    pub sensitive: bool,
    pub backwards: bool,
    pub whole_words: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Context {
    Prefix,
    Content,
    Neighbor,
}

pub(crate) struct Candidate {
    pub source: SourceRange,
    pub context: Context,
}

#[derive(Default)]
pub(crate) struct SearchCursor {
    query: String,
    sensitive: bool,
    last: Option<u64>,
    accepted: Option<u64>,
    pub request: Option<Request>,
    pub candidate: Option<Candidate>,
    pub waiting: Option<SourceRange>,
    pub reveal: bool,
    pub validation: Option<crate::page::Page>,
    pub resume: Option<u64>,
    #[cfg(test)]
    pub submissions: usize,
    #[cfg(test)]
    pub submitted_anchor: Option<u64>,
}

impl SearchCursor {
    pub fn configure(&mut self, request: Request) {
        if self.query != request.query
            || self.sensitive != request.sensitive
            || self
                .request
                .as_ref()
                .is_some_and(|old| old.whole_words != request.whole_words)
        {
            self.accepted = None;
        }
        // Rejected literal candidates belong only to the current scan. F3 and a
        // direction change resume from the last visible whole-word match.
        self.last = self.accepted;
        self.query.clone_from(&request.query);
        self.sensitive = request.sensitive;
        self.request = Some(request);
        self.validation = None;
        self.candidate = None;
        self.waiting = None;
        self.resume = None;
        self.reveal = false;
    }

    pub fn cancel_candidate(&mut self) {
        self.candidate = None;
        self.waiting = None;
        self.resume = None;
        self.validation = None;
        self.reveal = false;
    }

    pub fn last(&self) -> Option<u64> {
        self.last
    }

    pub fn begin(&mut self, query: &str, sensitive: bool, backwards: bool, viewport: u64) -> u64 {
        if self.query != query || self.sensitive != sensitive {
            self.query = query.into();
            self.sensitive = sensitive;
            self.last = None;
            self.accepted = None;
        }
        self.last.map_or(viewport, |last| {
            if backwards {
                last
            } else {
                last.saturating_add(1)
            }
        })
    }

    pub fn accept(&mut self, offset: u64) {
        self.accepted = Some(offset);
        self.last = Some(offset);
    }

    pub fn found(&mut self, offset: u64) {
        self.last = Some(offset);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_search_includes_byte_zero_and_next_does_not_repeat_the_match() {
        let mut cursor = SearchCursor::default();
        assert_eq!(cursor.begin("x", false, false, 0), 0);
        cursor.found(0);
        assert_eq!(cursor.begin("x", false, false, 0), 1);
        assert_eq!(cursor.begin("different", false, false, 0), 0);
    }

    #[test]
    fn backwards_and_changed_case_keep_exact_large_anchors() {
        let mut cursor = SearchCursor::default();
        let offset = 1_u64 << 42;
        assert_eq!(cursor.begin("文", false, false, offset), offset);
        cursor.found(offset + 45);
        assert_eq!(cursor.begin("文", false, true, offset), offset + 45);
        assert_eq!(cursor.begin("文", true, false, offset), offset);
    }

    #[test]
    fn changing_whole_word_option_restarts_at_viewport_without_old_candidate_history() {
        let mut cursor = SearchCursor::default();
        cursor.configure(Request {
            query: "cat".into(),
            sensitive: false,
            backwards: false,
            whole_words: false,
        });
        assert_eq!(cursor.begin("cat", false, false, 0), 0);
        cursor.found(500);
        cursor.configure(Request {
            query: "cat".into(),
            sensitive: false,
            backwards: false,
            whole_words: true,
        });
        assert_eq!(cursor.begin("cat", false, false, 20), 20);
        assert!(cursor.candidate.is_none());
        assert!(cursor.validation.is_none());
    }

    #[test]
    fn next_user_search_does_not_resume_from_invisible_rejected_candidates() {
        let mut cursor = SearchCursor::default();
        let request = Request {
            query: "cat".into(),
            sensitive: false,
            backwards: false,
            whole_words: true,
        };
        cursor.configure(request.clone());
        cursor.accept(20);
        cursor.found(1000);
        assert_eq!(cursor.begin("cat", false, false, 0), 1001);
        cursor.configure(Request {
            backwards: true,
            ..request
        });
        assert_eq!(cursor.begin("cat", false, true, 0), 20);
    }
}
