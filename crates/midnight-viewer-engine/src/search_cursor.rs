#[derive(Default)]
pub(crate) struct SearchCursor {
    query: String,
    sensitive: bool,
    last: Option<u64>,
}

impl SearchCursor {
    pub fn begin(&mut self, query: &str, sensitive: bool, backwards: bool, viewport: u64) -> u64 {
        if self.query != query || self.sensitive != sensitive {
            self.query = query.into();
            self.sensitive = sensitive;
            self.last = None;
        }
        self.last.map_or(viewport, |last| {
            if backwards {
                last
            } else {
                last.saturating_add(1)
            }
        })
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
}
