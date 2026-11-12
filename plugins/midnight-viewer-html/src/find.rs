//! Serializes native asynchronous searches so late results retain their actual request identity.
use midnight_web_viewer_api::Find;

#[derive(Default)]
pub(crate) struct FindQueue {
    loaded: bool,
    pending: Option<(u64, Find)>,
    active: Option<(u64, Option<i32>)>,
    completed: Option<i32>,
}

impl FindQueue {
    pub(crate) fn request(&mut self, sequence: u64, find: Find) {
        self.pending = Some((sequence, find));
    }
    pub(crate) fn loaded(&mut self, loaded: bool) {
        self.loaded = loaded;
    }
    pub(crate) fn start(&mut self) -> Option<Find> {
        if !self.loaded || self.active.is_some() {
            return None;
        }
        let (sequence, find) = self.pending.take()?;
        self.active = Some((sequence, None));
        Some(find)
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
        assert_eq!(queue.start().unwrap().query, "old");
        queue.request(11, query("superseded"));
        queue.request(12, query("latest"));
        assert!(queue.start().is_none());
        assert_eq!(queue.result(1, false), Some(10));
        assert_eq!(queue.result(1, true), Some(10));
        assert_eq!(queue.start().unwrap().query, "latest");
        assert_eq!(queue.result(1, true), None);
        assert_eq!(queue.result(2, true), Some(12));
    }
}
