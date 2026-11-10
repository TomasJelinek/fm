//! Replace still-pending absolute scrollbar positions without crossing input barriers.
use midnight_core::ViewerInput;
use std::sync::{Arc, Mutex, Weak};

pub(crate) enum Message {
    Input(ViewerInput),
    Position(Arc<PositionCell>),
    Ready,
}

impl Message {
    pub fn into_input(self) -> Option<ViewerInput> {
        match self {
            Self::Input(input) => Some(input),
            Self::Position(position) => position.take(),
            Self::Ready => None,
        }
    }
}

/// The receiver seals this cell before requesting any filesystem or renderer work.
pub(crate) struct PositionCell(Mutex<Option<ViewerInput>>);

impl PositionCell {
    fn replace(&self, input: &ViewerInput) -> bool {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        match (pending.as_mut(), input) {
            (Some(ViewerInput::Seek(old)), ViewerInput::Seek(new))
            | (Some(ViewerInput::Pan(old)), ViewerInput::Pan(new)) => {
                *old = *new;
                true
            }
            (
                Some(ViewerInput::Position {
                    generation: old_generation,
                    sequence: old_sequence,
                    horizontal: old_axis,
                    position: old_position,
                }),
                ViewerInput::Position {
                    generation,
                    sequence,
                    horizontal,
                    position,
                },
            ) if old_generation == generation && old_axis == horizontal => {
                *old_sequence = *sequence;
                *old_position = *position;
                true
            }
            _ => false,
        }
    }

    fn take(&self) -> Option<ViewerInput> {
        self.0
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
    }
}

#[derive(Default)]
pub(crate) struct PositionQueue {
    pending: Mutex<Option<Weak<PositionCell>>>,
}

impl PositionQueue {
    /// `send` must be a nonblocking enqueue; it is invoked under the short input lock.
    pub fn enqueue(&self, input: ViewerInput, send: impl FnOnce(Message) -> bool) -> bool {
        let mut pending = self
            .pending
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if !matches!(
            input,
            ViewerInput::Seek(_) | ViewerInput::Pan(_) | ViewerInput::Position { .. }
        ) {
            *pending = None;
            return send(Message::Input(input));
        }
        if let Some(previous) = pending.as_ref().and_then(Weak::upgrade)
            && previous.replace(&input)
        {
            return true;
        }
        let position = Arc::new(PositionCell(Mutex::new(Some(input))));
        if !send(Message::Position(position.clone())) {
            *pending = None;
            return false;
        }
        *pending = Some(Arc::downgrade(&position));
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    fn send(queue: &PositionQueue, sender: &mpsc::SyncSender<Message>, input: ViewerInput) -> bool {
        queue.enqueue(input, |message| sender.try_send(message).is_ok())
    }

    fn seek(input: ViewerInput) -> f64 {
        let ViewerInput::Seek(value) = input else {
            panic!("expected seek")
        };
        value
    }

    #[test]
    fn stalled_receiver_keeps_last_of_ten_thousand_positions_in_one_slot() {
        let queue = PositionQueue::default();
        let (sender, receiver) = mpsc::sync_channel(1);
        for index in 0..10_000 {
            assert!(
                send(&queue, &sender, ViewerInput::Seek(index as f64 / 10_000.0)),
                "drag position {index} dropped behind obsolete requests"
            );
        }
        assert_eq!(seek(receiver.recv().unwrap().into_input().unwrap()), 0.9999);
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn consumed_position_is_sealed_and_later_move_gets_a_new_delivery() {
        let queue = PositionQueue::default();
        let (sender, receiver) = mpsc::sync_channel(2);
        assert!(send(&queue, &sender, ViewerInput::Seek(0.1)));
        let first = receiver.recv().unwrap();
        let Message::Position(cell) = first else {
            panic!("coalesced cell expected")
        };
        assert!(send(&queue, &sender, ViewerInput::Seek(0.2)));
        assert_eq!(seek(cell.take().unwrap()), 0.2);
        assert!(send(&queue, &sender, ViewerInput::Seek(0.9)));
        assert_eq!(seek(receiver.recv().unwrap().into_input().unwrap()), 0.9);
        assert!(
            cell.take().is_none(),
            "a consumed request cannot change during rendering"
        );
    }

    #[test]
    fn file_mode_search_and_other_axis_are_ordering_barriers() {
        let queue = PositionQueue::default();
        let (sender, receiver) = mpsc::sync_channel(10);
        for input in [
            ViewerInput::Seek(0.1),
            ViewerInput::Seek(0.2),
            ViewerInput::NextFile,
            ViewerInput::Seek(0.3),
            ViewerInput::Seek(0.4),
            ViewerInput::Pan(0.5),
            ViewerInput::Pan(0.6),
            ViewerInput::Mode(2),
            ViewerInput::Seek(0.7),
            ViewerInput::Search {
                query: "needle".into(),
                backwards: false,
                case_sensitive: false,
            },
            ViewerInput::Seek(0.8),
        ] {
            assert!(send(&queue, &sender, input));
        }
        let received: Vec<_> = receiver
            .try_iter()
            .filter_map(Message::into_input)
            .collect();
        assert_eq!(received.len(), 8);
        assert!(matches!(received[0], ViewerInput::Seek(0.2)));
        assert!(matches!(received[1], ViewerInput::NextFile));
        assert!(matches!(received[2], ViewerInput::Seek(0.4)));
        assert!(matches!(received[3], ViewerInput::Pan(0.6)));
        assert!(matches!(received[4], ViewerInput::Mode(2)));
        assert!(matches!(received[5], ViewerInput::Seek(0.7)));
        assert!(matches!(&received[6], ViewerInput::Search { query, .. } if query == "needle"));
        assert!(matches!(received[7], ViewerInput::Seek(0.8)));
    }

    #[test]
    fn rejected_enqueue_does_not_claim_a_pending_position_was_accepted() {
        let queue = PositionQueue::default();
        assert!(!queue.enqueue(ViewerInput::Seek(0.1), |_| false));
        let (sender, receiver) = mpsc::sync_channel(1);
        assert!(send(&queue, &sender, ViewerInput::Seek(0.8)));
        assert_eq!(seek(receiver.recv().unwrap().into_input().unwrap()), 0.8);
    }

    fn position(generation: u64, sequence: u64, horizontal: bool, position: f64) -> ViewerInput {
        ViewerInput::Position {
            generation,
            sequence,
            horizontal,
            position,
        }
    }

    #[test]
    fn scoped_positions_keep_latest_sequence_and_value_in_one_slot() {
        let queue = PositionQueue::default();
        let (sender, receiver) = mpsc::sync_channel(1);
        for sequence in 1..=10_000 {
            assert!(send(
                &queue,
                &sender,
                position(7, sequence, false, sequence as f64 / 10_000.0)
            ));
        }
        assert!(matches!(
            receiver.recv().unwrap().into_input(),
            Some(ViewerInput::Position {
                generation: 7,
                sequence: 10_000,
                horizontal: false,
                position: 1.0
            })
        ));
        assert!(receiver.try_recv().is_err());
    }

    #[test]
    fn scoped_positions_never_cross_generation_axis_or_relative_scroll_barriers() {
        let queue = PositionQueue::default();
        let (sender, receiver) = mpsc::sync_channel(8);
        for input in [
            position(7, 1, false, 0.1),
            position(7, 2, false, 0.2),
            position(7, 3, true, 0.3),
            position(7, 4, true, 0.4),
            position(8, 5, true, 0.5),
            ViewerInput::Scroll { x: 0.0, y: 40.0 },
            position(8, 6, true, 0.6),
            ViewerInput::Pan(0.7),
        ] {
            assert!(send(&queue, &sender, input));
        }
        let received: Vec<_> = receiver
            .try_iter()
            .filter_map(Message::into_input)
            .collect();
        assert_eq!(received.len(), 6);
        assert!(matches!(
            received[0],
            ViewerInput::Position {
                sequence: 2,
                horizontal: false,
                ..
            }
        ));
        assert!(matches!(
            received[1],
            ViewerInput::Position {
                sequence: 4,
                horizontal: true,
                ..
            }
        ));
        assert!(matches!(
            received[2],
            ViewerInput::Position {
                generation: 8,
                sequence: 5,
                ..
            }
        ));
        assert!(matches!(received[3], ViewerInput::Scroll { y: 40.0, .. }));
        assert!(matches!(
            received[4],
            ViewerInput::Position { sequence: 6, .. }
        ));
        assert!(matches!(received[5], ViewerInput::Pan(0.7)));
    }
}
