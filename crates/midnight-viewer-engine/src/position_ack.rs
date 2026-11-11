//! A displayed scrollbar acknowledges only the exact successfully completed viewport.
#[derive(Default)]
pub(crate) struct Positions {
    pub generation: u64,
    pub acknowledged: [u64; 2],
    latest: [u64; 2],
    pub horizontal: Option<f64>,
    read: Option<(u64, u64, bool)>,
    deferred: Option<(u64, f64)>,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Completion {
    Unrelated,
    Ready,
    Failed,
}

impl Positions {
    pub fn reset(&mut self) -> Result<(), String> {
        let generation = self
            .generation
            .checked_add(1)
            .ok_or("Viewer position context exhausted.")?;
        *self = Self {
            generation,
            ..Self::default()
        };
        Ok(())
    }
    pub fn accept(
        &mut self,
        generation: u64,
        sequence: u64,
        horizontal: bool,
        position: f64,
    ) -> bool {
        let axis = usize::from(horizontal);
        if generation != self.generation
            || sequence == 0
            || sequence <= self.latest[axis]
            || !position.is_finite()
        {
            return false;
        }
        self.latest[axis] = sequence;
        true
    }
    pub fn horizontal(&mut self, sequence: u64, position: f64) {
        self.horizontal = Some(position.clamp(0.0, 1.0));
        self.acknowledged[1] = sequence;
    }
    pub fn vertical(&mut self, sequence: u64) {
        debug_assert!(self.read.is_none());
        self.acknowledged[0] = sequence;
    }
    pub fn requested(&mut self, id: u64, sequence: u64) {
        self.read = Some((id, sequence, false));
    }
    pub fn loaded(&mut self, id: u64) {
        if let Some((request, _, loaded)) = &mut self.read
            && *request == id
        {
            *loaded = true;
        }
    }
    pub fn complete(&mut self, id: u64, success: bool) -> Completion {
        let Some((request, sequence, loaded)) = self.read else {
            return Completion::Unrelated;
        };
        if id != request {
            return Completion::Unrelated;
        }
        self.read = None;
        if !success || !loaded {
            return Completion::Failed;
        }
        self.acknowledged[0] = sequence;
        Completion::Ready
    }
    pub fn has_pending(&self) -> bool {
        self.read.is_some()
    }
    pub fn defer(&mut self, sequence: u64, position: f64) {
        self.deferred = Some((sequence, position.clamp(0.0, 1.0)));
    }
    pub fn take_deferred(&mut self) -> Option<(u64, f64)> {
        self.deferred.take()
    }
    pub fn uncompleted_page(&self) -> bool {
        self.read.is_some_and(|(_, _, loaded)| loaded)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_matching_loaded_successful_completion_acknowledges_the_latest_seek() {
        let mut positions = Positions::default();
        positions.reset().unwrap();
        assert!(positions.accept(1, 1, false, 0.2));
        positions.requested(41, 1);
        positions.loaded(41);
        assert!(positions.accept(1, 2, false, 0.9));
        positions.requested(42, 2);
        assert_eq!(positions.complete(41, true), Completion::Unrelated);
        assert_eq!(positions.acknowledged, [0; 2]);
        assert_eq!(positions.complete(42, true), Completion::Failed);
        assert_eq!(positions.acknowledged, [0; 2]);
        positions.requested(43, 2);
        positions.loaded(42);
        assert_eq!(positions.complete(43, true), Completion::Failed);
        positions.requested(44, 2);
        positions.loaded(44);
        assert_eq!(positions.complete(44, false), Completion::Failed);
        positions.requested(45, 2);
        positions.loaded(45);
        assert_eq!(positions.complete(45, true), Completion::Ready);
        assert_eq!(positions.acknowledged, [2, 0]);
    }
    #[test]
    fn replaced_file_resize_and_invalid_sequences_cannot_move_the_current_context() {
        let mut positions = Positions::default();
        positions.reset().unwrap();
        assert!(positions.accept(1, 10, true, 0.4));
        positions.horizontal(10, 0.4);
        assert!(!positions.accept(1, 9, true, 0.3));
        assert!(!positions.accept(1, 11, true, f64::NAN));
        positions.requested(12, 3);
        positions.reset().unwrap();
        assert!(!positions.accept(1, 20, false, 0.8));
        assert_eq!(positions.complete(12, true), Completion::Unrelated);
        assert_eq!(positions.acknowledged, [0; 2]);
        assert_eq!(positions.horizontal, None);
    }

    #[test]
    fn continuous_drag_keeps_one_active_read_and_only_the_latest_deferred_target() {
        let mut positions = Positions::default();
        positions.reset().unwrap();
        assert!(positions.accept(1, 1, false, 0.1));
        positions.requested(41, 1);
        for sequence in 2..10_000 {
            assert!(positions.accept(1, sequence, false, sequence as f64 / 10_000.0));
            positions.defer(sequence, sequence as f64 / 10_000.0);
        }
        positions.loaded(41);
        assert!(positions.uncompleted_page());
        assert_eq!(positions.complete(41, true), Completion::Ready);
        assert_eq!(positions.acknowledged, [1, 0]);
        assert!(!positions.uncompleted_page());
        assert_eq!(positions.take_deferred(), Some((9999, 0.9999)));
        assert_eq!(positions.take_deferred(), None);
        positions.defer(10_000, 0.7);
        positions.reset().unwrap();
        assert_eq!(positions.take_deferred(), None);
    }
}
