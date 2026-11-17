//! Replace still-pending absolute scrollbar positions without crossing input barriers.
//! Backend wakes are coalesced so they cannot crowd user input out of the bounded channel.
use midnight_core::{ViewerInput, ViewerRasterInput};
use std::sync::{
    Arc, Mutex, Weak,
    atomic::{AtomicBool, Ordering},
    mpsc::SyncSender,
};

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

/// Backend wakes share the bounded user-input channel. At most one `Ready` is queued at a
/// time: the worker polls every source after receiving it, so later wakes are redundant.
///
/// Producers publish their state (session output, a session's end, a finished capture),
/// then `wake`. The worker takes `Ready`, calls `received`, then polls every source. Both
/// sides use read-modify-write operations on `pending`, which totally orders them: either
/// the producer's `wake` reads the worker's `false` and queues a new `Ready`, or `received`
/// reads the producer's `true` and synchronizes with it, so the poll that follows sees the
/// published state. With a plain store in `received`, both could miss each other (store
/// buffering), and a worker that waits for a wake without a retry, such as for the end of
/// a closing plugin session, would wait for an unrelated one.
pub(crate) struct Wakes {
    sender: SyncSender<Message>,
    pending: AtomicBool,
}

impl Wakes {
    pub fn new(sender: SyncSender<Message>) -> Self {
        Self {
            sender,
            pending: AtomicBool::new(false),
        }
    }

    /// Nonblocking. Returns whether a wake is queued for the worker.
    pub fn wake(&self) -> bool {
        if self.pending.swap(true, Ordering::AcqRel) {
            return true;
        }
        let queued = self.sender.try_send(Message::Ready).is_ok();
        if !queued {
            // A full queue is drained, and every source polled, before the worker blocks.
            self.pending.store(false, Ordering::Release);
        }
        queued
    }

    /// The worker received `Ready` and polls every source after this call.
    pub fn received(&self) {
        self.pending.swap(false, Ordering::AcqRel);
    }
}

/// The receiver seals this cell before requesting any filesystem or renderer work.
pub(crate) struct PositionCell(Mutex<Option<ViewerInput>>);

impl PositionCell {
    fn replace(&self, input: &ViewerInput) -> bool {
        let mut pending = self.0.lock().unwrap_or_else(|error| error.into_inner());
        match (pending.as_mut(), input) {
            (
                Some(ViewerInput::Raster {
                    surface: old_surface,
                    input:
                        ViewerRasterInput::PanTo {
                            sequence: old_sequence,
                            origin: old_origin,
                            settled: old_settled,
                        },
                }),
                ViewerInput::Raster {
                    surface,
                    input:
                        ViewerRasterInput::PanTo {
                            sequence,
                            origin,
                            settled,
                        },
                },
            ) if old_surface == surface => {
                if sequence > old_sequence {
                    *old_sequence = *sequence;
                    *old_origin = *origin;
                    *old_settled = *settled;
                }
                true
            }
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
            ViewerInput::Seek(_)
                | ViewerInput::Pan(_)
                | ViewerInput::Position { .. }
                | ViewerInput::Raster {
                    input: ViewerRasterInput::PanTo { .. },
                    ..
                }
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
    fn backend_wake_bursts_queue_one_ready_and_leave_room_for_user_input() {
        let (sender, receiver) = mpsc::sync_channel(2);
        let wakes = Wakes::new(sender.clone());
        for _ in 0..10_000 {
            assert!(wakes.wake());
        }
        assert!(
            sender
                .try_send(Message::Input(ViewerInput::NextFile))
                .is_ok(),
            "browser frames must not fill the shared input queue"
        );
        let queued: Vec<_> = receiver.try_iter().collect();
        assert!(matches!(
            queued.as_slice(),
            [Message::Ready, Message::Input(ViewerInput::NextFile)]
        ));
        wakes.received();
        assert!(
            wakes.wake(),
            "a wake after the worker's poll is queued again"
        );
        assert!(matches!(receiver.try_recv(), Ok(Message::Ready)));
        wakes.received();
        sender.try_send(Message::Ready).unwrap();
        sender.try_send(Message::Ready).unwrap();
        assert!(!wakes.wake(), "a full queue reports the dropped wake");
        receiver.try_iter().for_each(drop);
        assert!(wakes.wake(), "a rejected wake does not stay pending");
    }

    /// Store-buffering litmus test of the `received`-then-poll handshake. Each round
    /// starts with a `Ready` taken from the queue: the worker calls `received` and then
    /// checks a published flag while a producer sets the flag and wakes. At least one side
    /// must observe the other: the worker sees the flag, or a new `Ready` is queued. The
    /// run is bounded by a round count and a short time budget so a loaded machine stays
    /// fast.
    #[test]
    fn received_and_poll_never_miss_a_concurrent_publication() {
        use std::sync::atomic::AtomicUsize;
        const ROUNDS: usize = 50_000;
        const STOP: usize = usize::MAX;
        let (sender, receiver) = mpsc::sync_channel(2);
        let wakes = Arc::new(Wakes::new(sender));
        let published = Arc::new(AtomicBool::new(false));
        let observed = Arc::new(AtomicBool::new(false));
        let round = Arc::new(AtomicUsize::new(0));
        let done = Arc::new(AtomicUsize::new(0));
        // Run one side per round until the coordinator stops the test.
        let side = |run: fn(&Wakes, &AtomicBool, &AtomicBool)| {
            let (wakes, published, observed, round, done) = (
                wakes.clone(),
                published.clone(),
                observed.clone(),
                round.clone(),
                done.clone(),
            );
            std::thread::spawn(move || {
                let mut last = 0;
                loop {
                    let current = loop {
                        let current = round.load(Ordering::Acquire);
                        if current != last {
                            break current;
                        }
                        std::thread::yield_now();
                    };
                    if current == STOP {
                        return;
                    }
                    run(&wakes, &published, &observed);
                    done.fetch_add(1, Ordering::AcqRel);
                    last = current;
                }
            })
        };
        fn poll(wakes: &Wakes, published: &AtomicBool, observed: &AtomicBool) {
            wakes.received();
            observed.store(published.load(Ordering::Acquire), Ordering::Relaxed);
        }
        fn publish(wakes: &Wakes, published: &AtomicBool, _: &AtomicBool) {
            published.store(true, Ordering::Release);
            wakes.wake();
        }
        let worker = side(poll);
        let producer = side(publish);
        let started = std::time::Instant::now();
        let (mut rounds, mut missed) = (0, 0);
        while rounds < ROUNDS && started.elapsed() < std::time::Duration::from_millis(300) {
            rounds += 1;
            // The worker took the pending `Ready`; nothing is published or queued yet.
            wakes.pending.store(true, Ordering::SeqCst);
            published.store(false, Ordering::SeqCst);
            round.store(rounds, Ordering::Release);
            while done.load(Ordering::Acquire) != rounds * 2 {
                std::thread::yield_now();
            }
            let queued = receiver.try_iter().count() > 0;
            if !observed.load(Ordering::Relaxed) && !queued {
                missed += 1;
            }
        }
        round.store(STOP, Ordering::Release);
        worker.join().unwrap();
        producer.join().unwrap();
        assert_eq!(
            missed, 0,
            "{missed} of {rounds} publications were neither polled nor woken"
        );
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
    #[test]
    fn retained_absolute_pans_keep_latest_target_without_crossing_selection_barriers() {
        let queue = PositionQueue::default();
        let (sender, receiver) = mpsc::sync_channel(3);
        let surface = midnight_core::ViewerSurfaceId(7);
        let pan = |sequence| ViewerInput::Raster {
            surface,
            input: ViewerRasterInput::PanTo {
                sequence,
                origin: [1.25, sequence as f64 / 4.0],
                settled: sequence == 10_000,
            },
        };
        for sequence in 1..=10_000 {
            assert!(send(&queue, &sender, pan(sequence)));
        }
        assert!(send(
            &queue,
            &sender,
            ViewerInput::Raster {
                surface,
                input: ViewerRasterInput::Select {
                    pixel_id: 2,
                    origin: [1.25, 20.0],
                    x: 5.0,
                    y: 10.0,
                    extend: false,
                }
            }
        ));
        assert!(send(&queue, &sender, pan(10_001)));
        let values: Vec<_> = receiver
            .try_iter()
            .filter_map(Message::into_input)
            .collect();
        assert_eq!(values.len(), 3);
        assert!(matches!(
            values[0],
            ViewerInput::Raster {
                input: ViewerRasterInput::PanTo {
                    sequence: 10_000,
                    settled: true,
                    ..
                },
                ..
            }
        ));
        assert!(matches!(
            values[1],
            ViewerInput::Raster {
                input: ViewerRasterInput::Select { .. },
                ..
            }
        ));
        assert!(matches!(
            values[2],
            ViewerInput::Raster {
                input: ViewerRasterInput::PanTo {
                    sequence: 10_001,
                    ..
                },
                ..
            }
        ));
    }
}
