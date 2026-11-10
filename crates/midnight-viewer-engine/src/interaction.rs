//! A selection repaint must not invalidate coordinates from the visible page.

#[derive(Default)]
pub(crate) struct Frames {
    revision: u64,
    minimum_input: u64,
}

impl Frames {
    pub fn latest_revision(&self) -> u64 {
        self.revision
    }
    pub fn accepts(&self, frame: u64) -> bool {
        frame != 0 && frame >= self.minimum_input && frame <= self.revision
    }

    pub fn invalidate(&mut self) {
        self.minimum_input = self.revision.saturating_add(1);
    }

    pub fn painted(&mut self) -> Result<u64, String> {
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or("Viewer frame identity exhausted")?;
        Ok(self.revision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copying_and_dragging_remain_valid_until_page_or_camera_changes() {
        let mut frames = Frames::default();
        assert!(!frames.accepts(0));
        let visible = frames.painted().unwrap();
        frames.painted().unwrap(); // Selection was painted; UI still shows `visible`.
        assert!(frames.accepts(visible));
        assert!(!frames.accepts(frames.revision + 1));
        frames.invalidate(); // A page, scroll origin or size changed.
        assert!(!frames.accepts(visible));
        let next = frames.painted().unwrap();
        assert!(frames.accepts(next));
        assert!(!frames.accepts(visible));
    }
}
