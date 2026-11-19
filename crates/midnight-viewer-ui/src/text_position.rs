//! Latest absolute scrollbar intent, independent of rendering and bounded queue availability.

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Target {
    pub generation: u64,
    pub sequence: u64,
    pub horizontal: bool,
    pub position: f64,
}

#[derive(Default)]
pub(crate) struct TextPosition {
    generation: Option<u64>,
    blocked_generation: Option<u64>,
    sequence: u64,
    authoritative: [f64; 2],
    acknowledged: [u64; 2],
    desired: [Option<Target>; 2],
    queued: [bool; 2],
}

impl TextPosition {
    pub fn context_available(&self) -> bool {
        self.generation.is_some()
    }
    /// Read the same context/sequence fence before asynchronous pixel-error delivery.
    pub fn accepts_frame(
        &self,
        generation: u64,
        acknowledged: [u64; 2],
        positions: [f64; 2],
    ) -> bool {
        if self.blocked_generation.is_some_and(|old| generation <= old)
            || self.generation.is_some_and(|current| generation < current)
            || positions.iter().any(|value| !value.is_finite())
        {
            return false;
        }
        // A new generation cancels the old pending targets and resets both
        // acknowledged axes before accepting its first finite position.
        if self.generation != Some(generation) {
            return true;
        }
        let pending = self.desired.iter().any(Option::is_some);
        let progress = (0..2).any(|axis| acknowledged[axis] > self.acknowledged[axis]);
        !(0..2).any(|axis| acknowledged[axis] < self.acknowledged[axis]) && (!pending || progress)
    }

    /// Axes are vertical, horizontal. Present completed progress without moving pending thumbs.
    pub fn frame(&mut self, generation: u64, acknowledged: [u64; 2], positions: [f64; 2]) -> bool {
        if !self.accepts_frame(generation, acknowledged, positions) {
            return false;
        }
        if self.generation != Some(generation) {
            self.cancel();
            self.generation = Some(generation);
            self.blocked_generation = None;
            self.acknowledged = [0; 2];
        }
        self.authoritative = positions.map(|value| value.clamp(0.0, 1.0));
        self.acknowledged = acknowledged;
        for (axis, sequence) in acknowledged.into_iter().enumerate() {
            if self.desired[axis].is_some_and(|wanted| sequence >= wanted.sequence) {
                self.desired[axis] = None;
                self.queued[axis] = false;
            }
        }
        true
    }

    /// Settle the exact failed vertical seek without advancing its acknowledgement.
    pub fn vertical_unavailable(&mut self, generation: u64, sequence: u64, displayed: f64) -> bool {
        if self.generation != Some(generation)
            || self.desired[0].map(|wanted| wanted.sequence) != Some(sequence)
        {
            return false;
        }
        self.authoritative[0] = displayed;
        self.desired[0] = None;
        self.queued[0] = false;
        true
    }

    pub fn request(&mut self, horizontal: bool, position: f64) -> Option<Target> {
        let generation = self.generation?;
        if !position.is_finite() {
            return None;
        }
        let sequence = self.sequence.checked_add(1)?;
        self.sequence = sequence;
        let target = Target {
            generation,
            sequence,
            horizontal,
            position: position.clamp(0.0, 1.0),
        };
        let axis = usize::from(horizontal);
        self.desired[axis] = Some(target);
        self.queued[axis] = false;
        Some(target)
    }

    /// Retry unaccepted final positions; only two small records can be retained.
    pub fn flush(&mut self, mut send: impl FnMut(Target) -> bool) {
        let mut axes = [0, 1];
        if self.desired[0].map(|target| target.sequence)
            > self.desired[1].map(|target| target.sequence)
        {
            axes.swap(0, 1);
        }
        for axis in axes {
            if !self.queued[axis]
                && let Some(target) = self.desired[axis]
            {
                if !send(target) {
                    break;
                }
                self.queued[axis] = true;
            }
        }
    }

    pub fn positions(&self) -> [f64; 2] {
        std::array::from_fn(|axis| {
            self.desired[axis].map_or(self.authoritative[axis], |target| target.position)
        })
    }

    pub fn has_pending(&self) -> bool {
        self.desired.iter().any(Option::is_some)
    }

    /// A resize, cancellation or failed viewport discards unsent drag intent.
    pub fn cancel(&mut self) {
        self.desired = [None; 2];
        self.queued = [false; 2];
    }

    /// Reject old file/mode frames until the worker publishes its new generation.
    pub fn changing_content(&mut self) {
        self.cancel();
        if let Some(generation) = self.generation.take() {
            self.blocked_generation = Some(generation);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ready() -> TextPosition {
        let mut state = TextPosition::default();
        assert!(state.frame(7, [0; 2], [0.25, 0.0]));
        state
    }

    #[test]
    fn final_intent_survives_full_queue_and_old_replies_until_matching_frame() {
        let mut state = ready();
        let target = state.request(false, 0.8).unwrap();
        state.flush(|_| false);
        assert_eq!(state.positions(), [0.8, 0.0]);
        assert!(!state.frame(7, [0; 2], [0.3, 0.0]));
        let mut sent = Vec::new();
        state.flush(|next| {
            sent.push(next);
            true
        });
        assert_eq!(sent, [target]);
        state.flush(|_| panic!("accepted request must not be resubmitted"));
        assert!(!state.frame(7, [0; 2], [0.4, 0.0]));
        assert_eq!(state.positions(), [0.8, 0.0]);
        assert!(state.frame(7, [target.sequence, 0], [0.801, 0.0]));
        assert_eq!(state.positions(), [0.801, 0.0]);
        assert!(!state.frame(7, [0; 2], [0.2, 0.0]));
    }

    #[test]
    fn thousands_of_failed_positions_keep_only_latest_per_axis_in_submission_order() {
        let mut state = ready();
        for index in 0..10_000 {
            state.request(false, index as f64 / 10_000.0).unwrap();
            state.flush(|_| false);
        }
        let horizontal = state.request(true, 0.4).unwrap();
        let vertical = state.request(false, 0.2).unwrap();
        let mut sent = Vec::new();
        state.flush(|target| {
            sent.push(target);
            true
        });
        assert_eq!(sent, [horizontal, vertical]);
        assert_eq!(state.positions(), [0.2, 0.4]);
    }

    #[test]
    fn file_or_mode_change_rejects_old_frames_and_does_not_retry_old_gesture() {
        let mut state = ready();
        let old = state.request(false, 0.8).unwrap();
        state.changing_content();
        state.flush(|_| panic!("old file must not receive retry"));
        assert!(!state.frame(7, [old.sequence, 0], [0.8, 0.0]));
        assert!(state.request(false, 0.5).is_none());
        assert!(state.frame(8, [0; 2], [0.0; 2]));
        assert!(state.request(false, 0.1).unwrap().sequence > old.sequence);
    }

    #[test]
    fn resize_or_error_cancels_retry_and_invalid_positions_do_not_mutate_state() {
        let mut state = ready();
        state.request(false, 0.7).unwrap();
        state.cancel();
        state.flush(|_| panic!("cancelled gesture must not retry"));
        assert_eq!(state.positions(), [0.25, 0.0]);
        assert!(state.request(false, f64::NAN).is_none());
        assert!(!state.frame(7, [0; 2], [f64::INFINITY, 0.0]));
        assert_eq!(state.positions(), [0.25, 0.0]);
    }

    #[test]
    fn completed_intermediate_content_does_not_starve_or_move_the_latest_thumb() {
        let mut state = ready();
        let first = state.request(false, 0.4).unwrap();
        state.flush(|_| true);
        let last = state.request(false, 0.9).unwrap();
        assert!(state.frame(7, [first.sequence, 0], [0.401, 0.0]));
        assert!(state.has_pending());
        assert_eq!(state.positions(), [0.9, 0.0]);
        assert!(!state.frame(7, [0, 0], [0.1, 0.0]));
        assert!(!state.frame(7, [first.sequence, 0], [0.5, 0.0]));
        let mut retry = Vec::new();
        state.flush(|target| {
            retry.push(target);
            true
        });
        assert_eq!(
            retry,
            [last],
            "intermediate content lost the last unsubmitted target"
        );
        assert!(state.frame(7, [last.sequence, 0], [0.901, 0.0]));
        assert!(!state.has_pending());
        assert_eq!(state.positions(), [0.901, 0.0]);
        assert!(!state.frame(7, [first.sequence, 0], [0.4, 0.0]));
    }

    #[test]
    fn intermediate_reply_clears_only_completed_axes_and_preserves_other_axis_retry() {
        let mut state = ready();
        let first = state.request(false, 0.4).unwrap();
        let horizontal = state.request(true, 0.7).unwrap();
        state.flush(|_| true);
        let last = state.request(false, 0.8).unwrap();
        assert!(state.frame(7, [first.sequence, horizontal.sequence], [0.401, 0.701]));
        assert_eq!(state.positions(), [0.8, 0.701]);
        let mut retry = Vec::new();
        state.flush(|target| {
            retry.push(target);
            true
        });
        assert_eq!(retry, [last]);
        assert!(!state.frame(7, [last.sequence, 0], [0.8, 0.0]));
        assert!(state.frame(7, [last.sequence, horizontal.sequence], [0.801, 0.701]));
        assert_eq!(state.positions(), [0.801, 0.701]);
    }
}

#[cfg(test)]
mod receiver_tests {
    use crate::{
        OpenWindow, Service, ViewerInput, ViewerOutput, flush_position, image_presentation_tests,
        pixel_delivery, pixel_preparation, text_presentation_tests,
    };
    use std::{cell::RefCell, rc::Rc};

    fn source_frame(revision: u64, sequence: u64, position: f64) -> ViewerOutput {
        let ViewerOutput::Frame(mut frame) =
            text_presentation_tests::frame(revision, sequence, position)
        else {
            unreachable!()
        };
        frame.mode = 2;
        frame.modes = vec![2, 1];
        ViewerOutput::Frame(frame)
    }

    fn unavailable(generation: u64, sequence: u64) -> ViewerOutput {
        ViewerOutput::VerticalPositionUnavailable {
            generation,
            sequence,
        }
    }

    fn deliver(open: &OpenWindow, service: &Service, outputs: Vec<ViewerOutput>) {
        let pixels = outputs
            .iter()
            .find_map(|output| match output {
                ViewerOutput::Frame(frame) => Some(pixel_preparation::prepare(frame)),
                _ => None,
            })
            .unwrap_or(Ok(None));
        pixel_delivery::deliver_batch(
            open,
            service,
            &|_| panic!("position updates cannot copy content"),
            pixel_preparation::PreparedBatch { outputs, pixels },
        );
    }

    #[test]
    fn unavailable_vertical_reconciles_displayed_thumb_and_keeps_horizontal_retry() {
        let (_runtime, manager, open, service) = image_presentation_tests::open();
        open.timer.stop();
        image_presentation_tests::present(
            &open,
            &service,
            &|_| {},
            vec![source_frame(10, 0, 0.25)],
        );
        let submitted = Rc::new(RefCell::new(Vec::new()));
        let capture = submitted.clone();
        *open.input.borrow_mut() = Rc::new(move |input| {
            let accepted = !matches!(
                &input,
                ViewerInput::Position {
                    horizontal: true,
                    ..
                }
            );
            capture.borrow_mut().push(input);
            accepted
        });
        let window = &open.window;
        // A retained camera can advance the displayed position between worker frames.
        window.set_displayed_position(0.3125);
        image_presentation_tests::drag_seek(window, false, 0.625);
        image_presentation_tests::drag_seek(window, false, 0.875);
        image_presentation_tests::drag_seek(window, true, 0.5);
        deliver(
            &open,
            &service,
            vec![unavailable(7, 1), unavailable(7, 3), unavailable(6, 2)],
        );
        assert_eq!(window.get_vertical_thumb_position(), 0.875);
        assert_eq!(window.get_horizontal_position(), 0.5);
        assert!(window.get_position_pending());

        deliver(&open, &service, vec![unavailable(7, 2)]);
        assert_eq!(window.get_vertical_thumb_position(), 0.3125);
        assert_eq!(window.get_displayed_position(), 0.3125);
        assert_eq!(window.get_horizontal_position(), 0.5);
        assert!(window.get_position_pending());
        assert_eq!(open.frame.get(), 10);

        submitted.borrow_mut().clear();
        flush_position(&open);
        assert!(matches!(
            submitted.borrow().as_slice(),
            [ViewerInput::Position {
                generation: 7,
                sequence: 3,
                horizontal: true,
                position: 0.5,
            }]
        ));
        manager.close_all();
    }

    #[test]
    fn unavailable_vertical_precedes_frame_admission_and_ignores_retired_intent() {
        let (_runtime, manager, open, service) = image_presentation_tests::open();
        open.timer.stop();
        *open.input.borrow_mut() = Rc::new(|_| true);
        image_presentation_tests::present(
            &open,
            &service,
            &|_| {},
            vec![source_frame(10, 0, 0.25)],
        );
        let window = &open.window;

        image_presentation_tests::drag_seek(window, false, 0.875);
        deliver(&open, &service, vec![source_frame(11, 0, 0.375)]);
        assert_eq!(open.frame.get(), 10);
        deliver(
            &open,
            &service,
            vec![unavailable(7, 1), source_frame(11, 0, 0.375)],
        );
        assert_eq!(open.frame.get(), 11);
        assert_eq!(window.get_displayed_position(), 0.375);
        assert_eq!(window.get_vertical_thumb_position(), 0.375);
        assert!(!window.get_position_pending());

        image_presentation_tests::drag_seek(window, false, 0.625);
        deliver(&open, &service, vec![source_frame(12, 2, 0.625)]);
        assert!(!window.get_position_pending());
        window.set_displayed_position(0.75);
        deliver(&open, &service, vec![unavailable(7, 2)]);
        assert_eq!(window.get_vertical_thumb_position(), 0.625);
        assert!(!window.get_position_pending());

        image_presentation_tests::drag_seek(window, false, 0.875);
        window.invoke_change_mode(1);
        assert!(!open.text_position.borrow().context_available());
        let previous = (window.get_position(), window.get_position_pending());
        deliver(&open, &service, vec![unavailable(7, 3)]);
        assert_eq!(
            (window.get_position(), window.get_position_pending()),
            previous
        );
        manager.close_all();
    }
}
