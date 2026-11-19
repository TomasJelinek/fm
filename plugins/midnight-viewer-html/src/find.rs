//! Serializes native asynchronous searches so late results retain their actual request identity.
use midnight_web_viewer_api::Find;

#[derive(Default)]
pub(crate) struct FindQueue {
    loaded: bool,
    pending: Option<(u64, Find)>,
    active: Option<(u64, Option<i32>)>,
    completed: Option<i32>,
    native_query: Option<(String, bool)>,
}

pub(crate) struct FindAction {
    pub(crate) find: Find,
    pub(crate) restart: bool,
}

impl FindQueue {
    pub(crate) fn request(&mut self, sequence: u64, find: Find) {
        self.pending = Some((sequence, find));
    }
    pub(crate) fn loaded(&mut self, loaded: bool) {
        self.loaded = loaded;
    }
    pub(crate) fn start(&mut self) -> Option<FindAction> {
        if !self.loaded || self.active.is_some() {
            return None;
        }
        let (sequence, mut find) = self.pending.take()?;
        // Chromium removes CR before comparing its current query and case flag.
        find.query.retain(|character| character != '\r');
        let restart = !find.find_next
            && self
                .native_query
                .as_ref()
                .is_some_and(|(query, match_case)| {
                    *query == find.query && *match_case == find.match_case
                });
        self.native_query = if find.query.is_empty() {
            None
        } else {
            Some((find.query.clone(), find.match_case))
        };
        self.active = Some((sequence, None));
        Some(FindAction { find, restart })
    }
    pub(crate) fn finish_without_native(&mut self) -> Option<u64> {
        self.active.take().map(|(sequence, _)| sequence)
    }
    pub(crate) fn result(&mut self, identifier: i32, complete: bool) -> Option<u64> {
        if self
            .completed
            .is_some_and(|previous| identifier <= previous)
        {
            return None;
        }
        let (sequence, native) = self.active.as_mut()?;
        if native.is_some_and(|native| native != identifier) {
            return None;
        }
        *native = Some(identifier);
        let sequence = *sequence;
        if complete {
            self.active = None;
            self.completed = Some(identifier);
        }
        Some(sequence)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn query(text: &str) -> Find {
        Find {
            query: text.into(),
            forward: true,
            match_case: false,
            find_next: false,
        }
    }
    #[test]
    fn load_and_previous_completion_gate_latest_search_without_retagging_results() {
        let mut queue = FindQueue::default();
        queue.request(10, query("old"));
        assert!(queue.start().is_none());
        queue.loaded(true);
        assert_eq!(queue.start().unwrap().find.query, "old");
        queue.request(11, query("superseded"));
        queue.request(12, query("latest"));
        assert!(queue.start().is_none());
        assert_eq!(queue.result(1, false), Some(10));
        assert_eq!(queue.result(1, true), Some(10));
        assert_eq!(queue.start().unwrap().find.query, "latest");
        assert_eq!(queue.result(1, true), None);
        assert_eq!(queue.result(2, true), Some(12));
    }

    #[test]
    fn repeated_initial_search_requires_fresh_native_work_and_keeps_result_identity() {
        let mut queue = FindQueue::default();
        queue.loaded(true);
        queue.request(20, query("same"));
        let first = queue.start().unwrap();
        assert_eq!(first.find, query("same"));
        assert!(!first.restart);
        assert_eq!(queue.result(35, true), Some(20));

        queue.request(27, query("same"));
        let repeated = queue.start().unwrap();
        assert_eq!(repeated.find, query("same"));
        assert!(
            repeated.restart,
            "Chromium suppresses an unchanged initial search"
        );
        assert!(queue.start().is_none());
        assert_eq!(
            queue.result(35, true),
            None,
            "the old result cannot settle seq 27"
        );
        assert_eq!(queue.result(36, false), Some(27));
        assert_eq!(
            queue.result(37, true),
            None,
            "the active native ID stays fenced"
        );
        assert_eq!(queue.result(36, true), Some(27));
    }

    #[test]
    fn find_next_keeps_direction_and_never_restarts_the_current_native_session() {
        let mut queue = FindQueue::default();
        queue.loaded(true);
        queue.request(1, query("same"));
        queue.start().unwrap();
        assert_eq!(queue.result(1, true), Some(1));
        for (sequence, forward) in [(2, true), (3, false)] {
            let find = Find {
                forward,
                find_next: true,
                ..query("same")
            };
            queue.request(sequence, find.clone());
            let action = queue.start().unwrap();
            assert_eq!(action.find, find);
            assert!(
                !action.restart,
                "Find Next must retain Chromium's active match"
            );
            assert_eq!(queue.result(sequence as i32, true), Some(sequence));
        }
        queue.request(4, query("same"));
        assert!(
            queue.start().unwrap().restart,
            "initial after Next is still explicit"
        );
    }

    #[test]
    fn changed_query_or_case_uses_native_initial_search_without_a_reset() {
        let mut queue = FindQueue::default();
        queue.loaded(true);
        let searches = [
            query("first"),
            query("second"),
            Find {
                match_case: true,
                ..query("second")
            },
            query("Second"),
        ];
        for (index, find) in searches.into_iter().enumerate() {
            let sequence = index as u64 + 1;
            queue.request(sequence, find.clone());
            let action = queue.start().unwrap();
            assert_eq!(action.find, find);
            assert!(!action.restart, "a changed native key already starts work");
            assert_eq!(queue.result(sequence as i32, true), Some(sequence));
        }
    }

    #[test]
    fn chromium_cr_normalization_controls_duplicate_detection_but_keeps_lf_and_unicode() {
        let mut queue = FindQueue::default();
        queue.loaded(true);
        queue.request(1, query("a\rβ\r\nc"));
        let first = queue.start().unwrap();
        assert_eq!(first.find.query, "aβ\nc");
        assert!(!first.restart);
        assert_eq!(queue.result(1, true), Some(1));

        queue.request(2, query("aβ\nc\r"));
        let repeated = queue.start().unwrap();
        assert_eq!(repeated.find.query, "aβ\nc");
        assert!(repeated.restart);
        assert_eq!(queue.result(2, true), Some(2));

        queue.request(3, query("aβnc"));
        assert!(
            !queue.start().unwrap().restart,
            "LF is part of the native key"
        );
    }

    #[test]
    fn normalized_empty_search_settles_without_native_and_clears_duplicate_key() {
        let mut queue = FindQueue::default();
        queue.loaded(true);
        queue.request(1, query("same"));
        queue.start().unwrap();
        assert_eq!(queue.result(7, true), Some(1));

        queue.request(2, query("\r\r"));
        let empty = queue.start().unwrap();
        assert!(empty.find.query.is_empty());
        assert_eq!(queue.finish_without_native(), Some(2));
        assert_eq!(queue.finish_without_native(), None);
        queue.request(3, query("same"));
        let after_clear = queue.start().unwrap();
        assert!(
            !after_clear.restart,
            "empty search already stops the native session"
        );
        assert_eq!(queue.result(7, true), None);
        assert_eq!(queue.result(8, true), Some(3));
    }

    #[test]
    fn only_a_dispatched_search_changes_the_duplicate_key() {
        let mut queue = FindQueue::default();
        queue.loaded(true);
        queue.request(1, query("same"));
        queue.start().unwrap();
        queue.request(2, query("discarded"));
        queue.request(3, query("same"));
        assert!(queue.start().is_none());
        assert_eq!(queue.result(1, true), Some(1));
        let action = queue.start().unwrap();
        assert_eq!(action.find.query, "same");
        assert!(
            action.restart,
            "superseded pending requests never reached Chromium"
        );
    }
}
