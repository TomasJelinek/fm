use crate::casefold::FOLD;
use midnight_plugin_sdk::api::viewer::{SourceRange, TextPage};
use std::collections::VecDeque;

pub struct Matcher {
    query: Vec<char>,
    failure: Vec<usize>,
    matched: usize,
    sensitive: bool,
    recent: VecDeque<(u64, u64)>,
}
impl Matcher {
    pub fn new(query: &str, sensitive: bool) -> Self {
        let query: Vec<_> = query.chars().flat_map(|c| fold(c, sensitive)).collect();
        let mut failure = vec![0; query.len()];
        for i in 1..query.len() {
            let mut n = failure[i - 1];
            while n > 0 && query[n] != query[i] {
                n = failure[n - 1];
            }
            if query[n] == query[i] {
                n += 1;
            }
            failure[i] = n;
        }
        Self {
            query,
            failure,
            matched: 0,
            sensitive,
            recent: VecDeque::new(),
        }
    }
    pub fn feed(
        &mut self,
        page: &TextPage,
        mut cancelled: impl FnMut() -> bool,
    ) -> Result<Vec<SourceRange>, ()> {
        let mut found = Vec::new();
        if self.query.is_empty() {
            return Ok(found);
        }
        for pair in page.anchors.windows(2) {
            if cancelled() {
                return Err(());
            }
            let (start, end) = (pair[0].source_offset, pair[1].source_offset);
            for c in page.text[pair[0].text_offset as usize..pair[1].text_offset as usize].chars() {
                for c in fold(c, self.sensitive) {
                    self.recent.push_back((start, end));
                    if self.recent.len() > self.query.len() {
                        self.recent.pop_front();
                    }
                    while self.matched > 0 && self.query[self.matched] != c {
                        self.matched = self.failure[self.matched - 1];
                    }
                    if self.query[self.matched] == c {
                        self.matched += 1;
                    }
                    if self.matched == self.query.len() {
                        let offset = self.recent.front().unwrap().0;
                        let range = SourceRange {
                            offset,
                            length: end - offset,
                        };
                        if found.last() != Some(&range) {
                            found.push(range);
                        }
                        self.matched = self.failure[self.matched - 1];
                    }
                }
            }
        }
        Ok(found)
    }
}

fn fold(c: char, sensitive: bool) -> Vec<char> {
    if !sensitive && let Ok(i) = FOLD.binary_search_by_key(&c, |entry| entry.0) {
        return FOLD[i].1.chars().collect();
    }
    vec![c]
}
