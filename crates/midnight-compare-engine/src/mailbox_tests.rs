//! The mailbox wake gate: one consumer callback per burst, never a lost output.
use super::*;
use std::time::{Duration, Instant};

fn counting(wakes: &Arc<AtomicUsize>, delivered: bool) -> Mailbox {
    let wakes = wakes.clone();
    Mailbox::with_wake(Some(Arc::new(move || {
        wakes.fetch_add(1, Ordering::SeqCst);
        delivered
    })))
}

fn status(output: Option<Output>) -> Option<String> {
    match output {
        Some(Output::Status(text)) => Some(text),
        Some(_) => panic!("unexpected output kind"),
        None => None,
    }
}

#[test]
fn a_burst_of_outputs_wakes_the_consumer_once_until_it_takes() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let mailbox = counting(&wakes, true);
    for step in 0..100 {
        mailbox.put(Output::Status(step.to_string()));
    }
    assert_eq!(wakes.load(Ordering::SeqCst), 1, "one callback per burst");
    assert_eq!(status(mailbox.take()).as_deref(), Some("99"), "latest only");
    assert_eq!(status(mailbox.take()), None);
    mailbox.put(Output::Status("next".into()));
    mailbox.put(Output::Status("newest".into()));
    assert_eq!(wakes.load(Ordering::SeqCst), 2, "a take re-arms the wake");
    assert_eq!(status(mailbox.take()).as_deref(), Some("newest"));
}

#[test]
fn an_undelivered_wake_is_retried_by_the_next_output() {
    let wakes = Arc::new(AtomicUsize::new(0));
    let mailbox = counting(&wakes, false);
    mailbox.put(Output::Status("first".into()));
    mailbox.put(Output::Status("second".into()));
    assert_eq!(
        wakes.load(Ordering::SeqCst),
        2,
        "a wake that reached nobody must not stay outstanding"
    );
    assert_eq!(status(mailbox.take()).as_deref(), Some("second"));
    // Without a wake the mailbox is still a plain latest-output slot.
    let plain = Mailbox::default();
    plain.put(Output::Status("polled".into()));
    assert_eq!(status(plain.take()).as_deref(), Some("polled"));
}

/// Races `take` against a concurrent `put`; every round starts with a wake outstanding and
/// an empty slot. Either the take sees the output or the put wakes again: both missing it
/// would leave an output nobody is woken for, as when `take` clears the flag after taking.
/// Bounded by rounds and a short time budget. A round that does not finish fails the test
/// at a deadline, and the side threads end with the test, also when it fails.
#[test]
fn a_take_and_a_concurrent_put_never_both_miss_the_output() {
    const ROUNDS: usize = 50_000;
    const STOP: usize = usize::MAX;
    /// Stores `STOP` when dropped, also while the test thread unwinds.
    struct Stop(Arc<AtomicUsize>);
    impl Drop for Stop {
        fn drop(&mut self) {
            self.0.store(STOP, Ordering::Release);
        }
    }
    let woke = Arc::new(AtomicBool::new(false));
    let flag = woke.clone();
    let mailbox = Arc::new(Mailbox::with_wake(Some(Arc::new(move || {
        flag.store(true, Ordering::Relaxed);
        true
    }))));
    let took = Arc::new(AtomicBool::new(false));
    let round = Arc::new(AtomicUsize::new(0));
    let stop = Stop(round.clone());
    let done = Arc::new(AtomicUsize::new(0));
    let side = |action: Box<dyn Fn() + Send>| {
        let (round, done) = (round.clone(), done.clone());
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
                action();
                done.fetch_add(1, Ordering::AcqRel);
                last = current;
            }
        })
    };
    let (consumer, producer) = (mailbox.clone(), mailbox.clone());
    let taken = took.clone();
    let threads = [
        side(Box::new(move || {
            taken.store(consumer.take().is_some(), Ordering::Relaxed)
        })),
        side(Box::new(move || {
            producer.put(Output::Status(String::new()))
        })),
    ];
    let started = Instant::now();
    // Elapses only on failure.
    let deadline = started + Duration::from_secs(30);
    let (mut rounds, mut missed) = (0, 0);
    while rounds < ROUNDS && started.elapsed() < Duration::from_millis(300) {
        rounds += 1;
        mailbox.woken.store(true, Ordering::SeqCst);
        *mailbox.output.lock().unwrap() = None;
        woke.store(false, Ordering::SeqCst);
        round.store(rounds, Ordering::Release);
        while done.load(Ordering::Acquire) != rounds * 2 {
            // A side thread leaves early only by panicking.
            assert!(
                threads.iter().all(|thread| !thread.is_finished()),
                "a side thread ended in round {rounds}"
            );
            assert!(Instant::now() < deadline, "round {rounds} did not finish");
            std::thread::yield_now();
        }
        if !took.load(Ordering::Relaxed) && !woke.load(Ordering::Relaxed) {
            missed += 1;
        }
    }
    drop(stop);
    for thread in threads {
        thread.join().unwrap();
    }
    assert_eq!(
        missed, 0,
        "{missed} of {rounds} outputs were neither taken nor woken for"
    );
}

#[test]
fn a_session_wakes_its_consumer_instead_of_being_polled() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::new(runtime.handle().clone());
    let (woken, wakes) = mpsc::channel();
    let session = service
        .open_with_wake(
            FileCompareLaunch {
                sources: Vec::new(),
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [255; 4],
                background: [30, 30, 30, 255],
                selection: [50, 80, 120, 255],
                accent: [90, 150, 220, 255],
            },
            ViewerGeometry {
                width: 640,
                height: 200,
                scale: 1.0,
            },
            Arc::new(move || woken.send(()).is_ok()),
        )
        .unwrap();
    // Every output arrives with a wake; the deadline only elapses on failure.
    let deadline = Instant::now() + Duration::from_secs(15);
    let choice = loop {
        wakes
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .expect("the worker's output must wake its consumer");
        if let Some(Output::ChooseFiles(choice)) = session.mailbox.take() {
            break choice;
        }
    };
    assert!(choice.paths.iter().all(|path| path.as_os_str().is_empty()));
    // The worker now waits for the user and publishes nothing more.
    assert!(session.mailbox.take().is_none());
    session.close();
}
