//! A displayed scrollbar acknowledges only the exact successfully completed viewport.
use midnight_plugin_host::api::messages::Viewport;

#[derive(Default)]
pub(crate) struct Positions {
    pub generation: u64,
    pub acknowledged: [u64; 2],
    latest: [u64; 2],
    pub horizontal: Option<f64>,
    pub target: Option<Target>,
    pub source: Option<SourceRecovery>,
    read: Option<(u64, u64, bool)>,
    deferred: Option<(u64, f64)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceWindow {
    pub start: u64,
    pub end: u64,
    pub before: bool,
    pub after: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MissingEdge {
    Before,
    After,
}

/// One initial request and at most three strictly growing Source rereads.
pub(crate) struct SourceRecovery {
    requested: Viewport,
    reads: u8,
    expected: Option<(SourceWindow, MissingEdge)>,
    stopped: bool,
}

impl SourceRecovery {
    pub fn new(viewport: Viewport) -> Self {
        Self {
            requested: viewport,
            reads: 1,
            expected: None,
            stopped: false,
        }
    }

    /// Called only after an otherwise successful completed map still misses.
    /// A returned request reserves its read slot; failed admission retires this state.
    pub fn reread(&mut self, window: SourceWindow, edge: MissingEdge) -> Option<Viewport> {
        if self.stopped {
            return None;
        }
        self.stopped = true;
        if window.start >= window.end
            || self.reads >= 4
            || self.requested.max_lines == 0
            || self.requested.max_lines > 512
        {
            return None;
        }
        if let Some((expected, growth)) = self.expected {
            let contains = window.start <= expected.start && window.end >= expected.end;
            let progressed = match growth {
                MissingEdge::Before => window.start < expected.start,
                MissingEdge::After => window.end > expected.end,
            };
            if !contains || !progressed {
                return None;
            }
        }
        if match edge {
            MissingEdge::Before => !window.before,
            MissingEdge::After => !window.after,
        } {
            return None;
        }
        // By induction, this real window contains every previous promised extent,
        // including both sides of a bracket when reshaping flips the miss. It is
        // their byte union: keep the opposite endpoint and require strict growth.
        let mut viewport = self.requested.clone();
        viewport.max_lines = viewport.max_lines.saturating_mul(2).min(512);
        match edge {
            MissingEdge::Before => {
                viewport.source_anchor = window.end;
                viewport.backwards = true;
            }
            MissingEdge::After => {
                viewport.source_anchor = window.start;
                viewport.backwards = false;
            }
        }
        if viewport == self.requested {
            return None;
        }
        self.expected = Some((window, edge));
        self.requested = viewport.clone();
        self.reads += 1;
        self.stopped = false;
        Some(viewport)
    }
}

#[derive(Clone, Copy)]
pub(crate) struct Target {
    pub sequence: u64,
    pub position: f64,
    pub corrected: bool,
}

#[derive(Debug, PartialEq)]
pub(crate) enum Completion {
    Unrelated,
    Ready,
    Failed(u64),
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
        self.target = None;
        self.source = None;
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
        self.target = None;
        self.source = None;
        if !success || !loaded {
            return Completion::Failed(sequence);
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
    pub fn has_deferred(&self) -> bool {
        self.deferred.is_some()
    }
    pub fn take_deferred(&mut self) -> Option<(u64, f64)> {
        self.deferred.take()
    }
    /// Supersession is not an acknowledgement of the unfinished old target.
    pub fn retire_superseded(&mut self, id: u64) -> Option<(u64, f64)> {
        if self.read.is_none_or(|(request, _, _)| request != id) {
            return None;
        }
        let deferred = self.deferred.take()?;
        self.read = None;
        self.target = None;
        self.source = None;
        Some(deferred)
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
        assert_eq!(positions.complete(42, true), Completion::Failed(2));
        assert_eq!(positions.acknowledged, [0; 2]);
        positions.requested(43, 2);
        positions.loaded(42);
        assert_eq!(positions.complete(43, true), Completion::Failed(2));
        positions.requested(44, 2);
        positions.loaded(44);
        assert_eq!(positions.complete(44, false), Completion::Failed(2));
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

    fn source_viewport() -> Viewport {
        Viewport {
            source_anchor: 100,
            backwards: false,
            max_bytes: 4096,
            max_lines: 10,
        }
    }

    fn source_window(start: u64, end: u64) -> SourceWindow {
        SourceWindow {
            start,
            end,
            before: true,
            after: true,
        }
    }

    #[test]
    fn source_forward_growth_preserves_start_and_actual_byte_budget() {
        let mut recovery = SourceRecovery::new(source_viewport());
        let first = recovery
            .reread(source_window(100, 200), MissingEdge::After)
            .unwrap();
        assert_eq!(first.source_anchor, 100);
        assert!(!first.backwards);
        assert_eq!(first.max_lines, 20);
        assert_eq!(first.max_bytes, 4096);
        let next = recovery
            .reread(source_window(100, 300), MissingEdge::After)
            .unwrap();
        assert_eq!(next.source_anchor, 100);
        assert!(!next.backwards);
        assert_eq!(next.max_lines, 40);
        assert_eq!(next.max_bytes, first.max_bytes);
    }

    #[test]
    fn source_backward_growth_preserves_end() {
        let mut viewport = source_viewport();
        viewport.source_anchor = 300;
        viewport.backwards = true;
        let mut recovery = SourceRecovery::new(viewport);
        let first = recovery
            .reread(source_window(200, 300), MissingEdge::Before)
            .unwrap();
        assert_eq!(first.source_anchor, 300);
        assert!(first.backwards);
        assert_eq!(first.max_lines, 20);
        let next = recovery
            .reread(source_window(100, 300), MissingEdge::Before)
            .unwrap();
        assert_eq!(next.source_anchor, 300);
        assert!(next.backwards);
        assert_eq!(next.max_lines, 40);
        assert_eq!(next.max_bytes, first.max_bytes);
    }

    #[test]
    fn source_bracket_flip_preserves_both_observed_boundaries() {
        let mut viewport = source_viewport();
        viewport.source_anchor = 200;
        let mut recovery = SourceRecovery::new(viewport);
        let before = recovery
            .reread(source_window(200, 300), MissingEdge::Before)
            .unwrap();
        assert_eq!((before.source_anchor, before.backwards), (300, true));
        let after = recovery
            .reread(source_window(100, 300), MissingEdge::After)
            .unwrap();
        assert_eq!((after.source_anchor, after.backwards), (100, false));
        assert_eq!(after.max_lines, 40);
        let before = recovery
            .reread(source_window(100, 400), MissingEdge::Before)
            .unwrap();
        assert_eq!((before.source_anchor, before.backwards), (400, true));
        assert_eq!(before.max_lines, 80);
    }

    #[test]
    fn source_repeated_cropped_and_wrong_edge_growth_stop_permanently() {
        for (edge, response) in [
            (MissingEdge::After, source_window(100, 200)),
            (MissingEdge::After, source_window(101, 250)),
            (MissingEdge::After, source_window(99, 200)),
            (MissingEdge::Before, source_window(50, 199)),
            (MissingEdge::Before, source_window(100, 250)),
        ] {
            let mut recovery = SourceRecovery::new(source_viewport());
            assert!(recovery.reread(source_window(100, 200), edge).is_some());
            assert!(recovery.reread(response, edge).is_none());
            assert!(
                recovery
                    .reread(source_window(1, 400), MissingEdge::After)
                    .is_none()
            );
        }
    }

    #[test]
    fn source_recovery_reserves_at_most_three_rereads() {
        let mut recovery = SourceRecovery::new(source_viewport());
        for end in [200, 300, 400] {
            assert!(
                recovery
                    .reread(source_window(100, end), MissingEdge::After)
                    .is_some()
            );
        }
        assert_eq!(recovery.reads, 4);
        assert!(
            recovery
                .reread(source_window(100, 500), MissingEdge::After)
                .is_none()
        );
        assert_eq!(recovery.reads, 4);
    }

    #[test]
    fn source_raw_extent_can_exceed_the_unchanged_decoded_byte_budget() {
        let mut viewport = source_viewport();
        viewport.max_bytes = 100;
        let mut recovery = SourceRecovery::new(viewport);
        let first = recovery
            .reread(source_window(100, 400), MissingEdge::After)
            .unwrap();
        assert_eq!(first.max_bytes, 100);
        let next = recovery
            .reread(source_window(100, 700), MissingEdge::After)
            .unwrap();
        assert_eq!(next.max_bytes, first.max_bytes);
        assert!(
            recovery
                .reread(source_window(100, 700), MissingEdge::After)
                .is_none()
        );
    }

    #[test]
    fn source_recovery_stops_at_line_and_real_file_edges() {
        let mut viewport = source_viewport();
        viewport.max_lines = 400;
        let mut recovery = SourceRecovery::new(viewport);
        assert_eq!(
            recovery
                .reread(source_window(100, 200), MissingEdge::After)
                .unwrap()
                .max_lines,
            512
        );
        assert!(
            recovery
                .reread(source_window(100, 300), MissingEdge::After)
                .is_none()
        );
        for edge in [MissingEdge::Before, MissingEdge::After] {
            let mut window = source_window(100, 200);
            match edge {
                MissingEdge::Before => window.before = false,
                MissingEdge::After => window.after = false,
            }
            let mut recovery = SourceRecovery::new(source_viewport());
            assert!(recovery.reread(window, edge).is_none());
        }
    }

    #[test]
    fn source_line_cap_allows_changed_boundaries_but_refuses_an_identical_request() {
        let mut viewport = source_viewport();
        viewport.max_lines = 512;
        let mut recovery = SourceRecovery::new(viewport.clone());
        let before = recovery
            .reread(source_window(100, 200), MissingEdge::Before)
            .unwrap();
        assert_eq!((before.source_anchor, before.backwards), (200, true));
        assert_eq!(before.max_lines, 512);
        assert_eq!(before.max_bytes, viewport.max_bytes);
        let after = recovery
            .reread(source_window(50, 200), MissingEdge::After)
            .unwrap();
        assert_eq!((after.source_anchor, after.backwards), (50, false));
        assert_eq!(after.max_lines, 512);
        assert_eq!(after.max_bytes, viewport.max_bytes);
        let mut repeated = SourceRecovery::new(viewport);
        assert!(
            repeated
                .reread(source_window(100, 200), MissingEdge::After)
                .is_none()
        );
        assert_eq!(repeated.reads, 1);
    }

    #[test]
    fn retiring_only_a_matching_superseded_read_preserves_context_and_ack() {
        let mut positions = Positions::default();
        positions.reset().unwrap();
        positions.horizontal(7, 0.4);
        positions.acknowledged[0] = 2;
        positions.target = Some(Target {
            sequence: 3,
            position: 0.2,
            corrected: false,
        });
        positions.source = Some(SourceRecovery::new(source_viewport()));
        positions.requested(41, 3);
        assert_eq!(positions.retire_superseded(41), None);
        assert!(positions.target.is_some() && positions.source.is_some());
        positions.defer(4, 0.5);
        positions.defer(5, 0.9);
        assert_eq!(positions.retire_superseded(40), None);
        assert!(positions.has_pending() && positions.has_deferred());
        assert_eq!(positions.retire_superseded(41), Some((5, 0.9)));
        assert_eq!(positions.generation, 1);
        assert_eq!(positions.acknowledged, [2, 7]);
        assert_eq!(positions.horizontal, Some(0.4));
        assert!(!positions.has_pending() && !positions.has_deferred());
        assert!(positions.target.is_none() && positions.source.is_none());
        assert_eq!(positions.complete(41, true), Completion::Unrelated);
    }

    #[test]
    fn source_recovery_is_cleared_by_vertical_completion_and_context_reset() {
        let mut positions = Positions {
            source: Some(SourceRecovery::new(source_viewport())),
            ..Positions::default()
        };
        positions.vertical(1);
        assert!(positions.source.is_none());
        positions.source = Some(SourceRecovery::new(source_viewport()));
        positions.requested(41, 2);
        assert_eq!(positions.complete(40, false), Completion::Unrelated);
        assert!(positions.source.is_some());
        assert_eq!(positions.complete(41, false), Completion::Failed(2));
        assert!(positions.source.is_none());
        positions.source = Some(SourceRecovery::new(source_viewport()));
        positions.requested(42, 2);
        positions.loaded(42);
        assert_eq!(positions.complete(42, true), Completion::Ready);
        assert!(positions.source.is_none());
        positions.source = Some(SourceRecovery::new(source_viewport()));
        positions.reset().unwrap();
        assert!(positions.source.is_none());
    }
}
