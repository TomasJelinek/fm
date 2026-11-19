//! Pure, constant-space latest-wheel diagnostic window. All times are caller-supplied microseconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct WheelId {
    pub document: u64,
    pub sequence: u64,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Summary {
    pub id: WheelId,
    pub at_us: u64,
    pub elapsed_us: u64,
    pub replaced: u64,
    pub pump_count: u64,
    pub pump_gap_us: u64,
    pub pump_completed_us: u64,
    pub nav_completed_us: u64,
    pub pump_active: bool,
    pub pump_active_us: u64,
}

struct Pending {
    summary: Summary,
    started_us: u64,
    last_pump_start_us: Option<u64>,
}

#[derive(Clone, Copy)]
struct Call {
    started_us: u64,
    window: Option<WheelId>,
}

#[derive(Default)]
pub(crate) struct Tracker {
    pending: Option<Pending>,
    pump: Option<Call>,
    navigation: Option<Call>,
}

impl Tracker {
    pub fn wheel(&mut self, id: WheelId, at_us: u64) {
        let replaced = self.pending.as_ref().map_or(0, |previous| {
            if previous.summary.id.document == id.document {
                previous.summary.replaced.saturating_add(1)
            } else {
                0
            }
        });
        self.pending = Some(Pending {
            summary: Summary {
                id,
                at_us,
                elapsed_us: 0,
                replaced,
                pump_count: 0,
                pump_gap_us: 0,
                pump_completed_us: 0,
                nav_completed_us: 0,
                pump_active: false,
                pump_active_us: 0,
            },
            started_us: at_us,
            last_pump_start_us: None,
        });
    }

    pub fn pump_start(&mut self, at_us: u64) {
        self.pump = Some(self.call(at_us));
        if let Some(pending) = self.pending.as_mut() {
            let previous = pending.last_pump_start_us.unwrap_or(pending.started_us);
            pending.summary.pump_count = pending.summary.pump_count.saturating_add(1);
            pending.summary.pump_gap_us = pending
                .summary
                .pump_gap_us
                .max(at_us.saturating_sub(previous));
            pending.last_pump_start_us = Some(at_us);
        }
    }

    pub fn pump_end(&mut self, at_us: u64) {
        if let Some(call) = self.pump.take()
            && let Some(pending) = self.pending.as_mut()
            && call.window == Some(pending.summary.id)
        {
            pending.summary.pump_completed_us = pending
                .summary
                .pump_completed_us
                .max(at_us.saturating_sub(call.started_us));
        }
    }

    pub fn navigation_start(&mut self, at_us: u64) {
        self.navigation = Some(self.call(at_us));
    }

    pub fn navigation_end(&mut self, at_us: u64) {
        if let Some(call) = self.navigation.take()
            && let Some(pending) = self.pending.as_mut()
            && call.window == Some(pending.summary.id)
        {
            pending.summary.nav_completed_us = pending
                .summary
                .nav_completed_us
                .max(at_us.saturating_sub(call.started_us));
        }
    }

    fn call(&self, started_us: u64) -> Call {
        Call {
            started_us,
            window: self.pending.as_ref().map(|pending| pending.summary.id),
        }
    }

    pub fn clear(&mut self) {
        *self = Self::default();
    }

    /// Only a validated view paint consumes a window. No pixel/input ACK is implied.
    pub fn paint(&mut self, document: u64, at_us: u64, valid: bool) -> Option<Summary> {
        if !valid || self.pending.as_ref()?.summary.id.document != document {
            return None;
        }
        let mut pending = self.pending.take()?;
        pending.summary.at_us = at_us;
        pending.summary.elapsed_us = at_us.saturating_sub(pending.started_us);
        pending.summary.pump_active = self.pump.is_some();
        pending.summary.pump_active_us = self
            .pump
            .map_or(0, |call| at_us.saturating_sub(call.started_us));
        Some(pending.summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(sequence: u64) -> WheelId {
        WheelId {
            document: 7,
            sequence,
        }
    }

    #[test]
    fn paint_inside_pump_reports_unfinished_age_before_end() {
        let mut tracker = Tracker::default();
        tracker.wheel(id(11), 100);
        tracker.navigation_start(101);
        tracker.navigation_end(108);
        tracker.pump_start(110);
        let summary = tracker.paint(7, 610, true).unwrap();
        assert_eq!(
            summary,
            Summary {
                id: id(11),
                at_us: 610,
                elapsed_us: 510,
                replaced: 0,
                pump_count: 1,
                pump_gap_us: 10,
                pump_completed_us: 0,
                nav_completed_us: 7,
                pump_active: true,
                pump_active_us: 500,
            }
        );
        tracker.pump_end(700);
        assert!(tracker.paint(7, 710, true).is_none());
    }

    #[test]
    fn completed_work_and_start_to_start_gap_are_distinct() {
        let mut tracker = Tracker::default();
        tracker.wheel(id(1), 10);
        tracker.pump_start(20);
        tracker.pump_end(70);
        tracker.pump_start(100);
        tracker.pump_end(130);
        let summary = tracker.paint(7, 1000, true).unwrap();
        assert_eq!(summary.pump_count, 2);
        assert_eq!(summary.pump_gap_us, 80);
        assert_eq!(summary.pump_completed_us, 50);
        assert!(!summary.pump_active);
        assert_eq!(summary.pump_active_us, 0);
        assert_eq!(summary.elapsed_us, 990);
    }

    #[test]
    fn replacement_resets_metrics_and_preserves_only_coalescing_count() {
        let mut tracker = Tracker::default();
        tracker.wheel(id(1), 10);
        tracker.pump_start(20);
        tracker.navigation_start(21);
        tracker.wheel(id(2), 30);
        tracker.navigation_end(40);
        tracker.pump_end(50);
        tracker.wheel(id(3), 60);
        let summary = tracker.paint(7, 70, true).unwrap();
        assert_eq!(summary.id, id(3));
        assert_eq!(summary.replaced, 2);
        assert_eq!(summary.pump_count, 0);
        assert_eq!(summary.pump_completed_us, 0);
        assert_eq!(summary.nav_completed_us, 0);
    }

    #[test]
    fn old_pump_ending_after_paint_cannot_contaminate_successive_window() {
        let mut tracker = Tracker::default();
        tracker.wheel(id(1), 10);
        tracker.pump_start(20);
        assert!(tracker.paint(7, 30, true).is_some());
        tracker.wheel(id(2), 30);
        tracker.pump_end(900);
        let summary = tracker.paint(7, 910, true).unwrap();
        assert_eq!(summary.pump_completed_us, 0);
        assert_eq!(summary.pump_count, 0);
        assert_eq!(summary.replaced, 0);
    }

    #[test]
    fn pump_started_before_latest_wheel_has_explicit_full_active_age() {
        let mut tracker = Tracker::default();
        tracker.pump_start(10);
        tracker.wheel(id(1), 100);
        let summary = tracker.paint(7, 150, true).unwrap();
        assert!(summary.pump_active);
        assert_eq!(summary.pump_active_us, 140);
        assert_eq!(summary.elapsed_us, 50);
        assert_eq!(summary.pump_count, 0);
    }

    #[test]
    fn invalid_or_foreign_paint_does_not_consume_window() {
        let mut tracker = Tracker::default();
        tracker.wheel(id(1), 10);
        assert!(tracker.paint(7, 20, false).is_none());
        assert!(tracker.paint(8, 30, true).is_none());
        assert_eq!(tracker.paint(7, 40, true).unwrap().elapsed_us, 30);
    }

    #[test]
    fn clear_removes_document_window_and_active_calls() {
        let mut tracker = Tracker::default();
        tracker.wheel(id(1), 10);
        tracker.pump_start(20);
        tracker.navigation_start(21);
        tracker.clear();
        tracker.pump_end(50);
        tracker.navigation_end(60);
        assert!(tracker.paint(7, 70, true).is_none());
        let foreign = WheelId {
            document: 8,
            sequence: 1,
        };
        tracker.wheel(foreign, 80);
        let summary = tracker.paint(8, 90, true).unwrap();
        assert_eq!(summary.id, foreign);
        assert!(!summary.pump_active);
        assert_eq!(summary.replaced, 0);
    }

    #[test]
    fn unfinished_navigation_is_not_reported_as_completed_or_as_a_pump() {
        let mut tracker = Tracker::default();
        tracker.wheel(id(1), 10);
        tracker.navigation_start(20);
        let summary = tracker.paint(7, 200, true).unwrap();
        assert_eq!(summary.nav_completed_us, 0);
        assert!(!summary.pump_active);
        tracker.navigation_end(300);
        assert!(tracker.paint(7, 310, true).is_none());
    }

    #[test]
    fn new_document_without_a_pending_paint_does_not_inherit_replacements() {
        let mut tracker = Tracker::default();
        tracker.wheel(id(1), 10);
        tracker.wheel(
            WheelId {
                document: 8,
                sequence: 2,
            },
            20,
        );
        assert!(tracker.paint(7, 30, true).is_none());
        let summary = tracker.paint(8, 40, true).unwrap();
        assert_eq!(summary.replaced, 0);
        assert_eq!(summary.elapsed_us, 20);
    }

    #[test]
    fn no_wheel_idle_pumps_never_emit() {
        let mut tracker = Tracker::default();
        for at in 0..10 {
            tracker.pump_start(at * 10);
            tracker.pump_end(at * 10 + 5);
            assert!(tracker.paint(7, at * 10 + 6, true).is_none());
        }
    }
}
