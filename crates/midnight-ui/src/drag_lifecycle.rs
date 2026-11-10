//! Pure input lifecycle: asynchronous preparation can never revive an ended gesture.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct SourceStamp {
    pub(crate) pane: usize,
    pub(crate) navigation: u64,
    pub(crate) displayed: u64,
    pub(crate) view: u64,
    pub(crate) tree: Option<midnight_core::TreeViewStamp>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    Preparing,
    Ready,
    Native,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Gesture {
    pub(crate) id: u64,
    pub(crate) source: SourceStamp,
    pub(crate) phase: Phase,
}

#[derive(Default)]
pub(crate) struct DragLifecycle {
    next: u64,
    pub(crate) active: Option<Gesture>,
}

impl DragLifecycle {
    pub(crate) fn begin(&mut self, source: SourceStamp) -> Option<u64> {
        if self.active.is_some() {
            return None;
        }
        self.next = self.next.checked_add(1)?;
        self.active = Some(Gesture {
            id: self.next,
            source,
            phase: Phase::Preparing,
        });
        Some(self.next)
    }
    pub(crate) fn prepared(&mut self, id: u64) -> bool {
        let Some(gesture) = self.active.as_mut() else {
            return false;
        };
        if gesture.id != id || gesture.phase != Phase::Preparing {
            return false;
        }
        gesture.phase = Phase::Ready;
        true
    }
    pub(crate) fn native_started(&mut self, id: u64) -> bool {
        let Some(gesture) = self.active.as_mut() else {
            return false;
        };
        if gesture.id != id || gesture.phase != Phase::Ready {
            return false;
        }
        gesture.phase = Phase::Native;
        true
    }
    pub(crate) fn release(&mut self) -> Option<Gesture> {
        if self
            .active
            .is_some_and(|gesture| gesture.phase == Phase::Native)
        {
            return None;
        }
        self.active.take()
    }
    pub(crate) fn cancel(&mut self) -> Option<Gesture> {
        self.active.take()
    }
    pub(crate) fn validate(&mut self, current: SourceStamp) -> Option<Gesture> {
        if self.active.is_some_and(|gesture| gesture.source != current) {
            self.active.take()
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stamp() -> SourceStamp {
        SourceStamp {
            pane: 0,
            navigation: 7,
            displayed: 6,
            view: 3,
            tree: None,
        }
    }

    #[test]
    fn cancelled_or_released_preparation_cannot_revive_a_drag() {
        let mut state = DragLifecycle::default();
        let first = state.begin(stamp()).unwrap();
        assert_eq!(state.release().unwrap().phase, Phase::Preparing);
        assert!(!state.prepared(first));
        let second = state.begin(stamp()).unwrap();
        assert_ne!(second, first);
        assert!(!state.prepared(first));
        assert_eq!(state.cancel().unwrap().id, second);
        assert!(!state.prepared(second));
    }

    #[test]
    fn internal_drop_requires_ready_gesture_and_is_consumed_once() {
        let mut state = DragLifecycle::default();
        let id = state.begin(stamp()).unwrap();
        assert!(state.prepared(id));
        assert_eq!(state.release().unwrap().phase, Phase::Ready);
        assert!(state.release().is_none());
        assert!(!state.native_started(id));
    }

    #[test]
    fn native_release_does_not_cancel_the_operating_system_drag() {
        let mut state = DragLifecycle::default();
        let id = state.begin(stamp()).unwrap();
        assert!(!state.native_started(id), "preparation is required");
        assert!(state.prepared(id));
        assert!(state.native_started(id));
        assert!(!state.native_started(id), "commit only once");
        assert!(state.release().is_none());
        assert_eq!(state.active.unwrap().phase, Phase::Native);
        assert_eq!(
            state.cancel().unwrap().id,
            id,
            "explicit shutdown still cancels"
        );
    }

    #[test]
    fn navigation_projection_and_pane_changes_cancel_only_the_captured_source() {
        for changed in [
            SourceStamp {
                navigation: 8,
                ..stamp()
            },
            SourceStamp {
                displayed: 8,
                ..stamp()
            },
            SourceStamp { view: 4, ..stamp() },
            SourceStamp { pane: 1, ..stamp() },
        ] {
            let mut state = DragLifecycle::default();
            let id = state.begin(stamp()).unwrap();
            assert!(state.validate(stamp()).is_none());
            assert_eq!(state.validate(changed).unwrap().id, id);
            assert!(!state.prepared(id));
        }
    }
}
