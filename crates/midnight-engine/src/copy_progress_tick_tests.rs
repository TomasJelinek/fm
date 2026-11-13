use super::*;
use crate::operation_throughput::{Counters, Sampler};
use std::sync::atomic::AtomicUsize;

fn running() -> Status {
    let mut progress = CopyProgress::queued(
        OperationId(1),
        std::path::PathBuf::from("destination"),
        1,
        TransferIntent::Copy,
    );
    progress.state = CopyOperationState::Running;
    progress.phase = CopyOperationPhase::Copying;
    progress.current_file_size = Some(1_048_576);
    Arc::new(Mutex::new(progress))
}

fn outbox() -> (Outbox, mpsc::Receiver<Event>, Arc<AtomicUsize>) {
    let (sender, receiver) = mpsc::channel(1);
    let wakes = Arc::new(AtomicUsize::new(0));
    let notify = wakes.clone();
    (
        Outbox {
            sender,
            wakeup: Arc::new(move || {
                notify.fetch_add(1, Ordering::Relaxed);
            }),
        },
        receiver,
        wakes,
    )
}

fn receive(receiver: &mut mpsc::Receiver<Event>) -> Arc<CopyProgress> {
    match receiver.try_recv() {
        Ok(Event::Copy(CopyEvent::Progress(progress))) => progress,
        event => panic!("Periodic Copy tick did not publish the latest snapshot: {event:?}"),
    }
}

#[test]
fn worker_presampled_ticks_still_publish_fresh_bytes_elapsed_and_wakeup() {
    let status = running();
    let (outbox, mut receiver, wakes) = outbox();
    let mut sampler = Sampler::default();
    sampler.observe(Duration::ZERO, Counters::default(), true);
    for tick in 1..=3 {
        let copied = tick.min(2) * 262_144;
        let worker_sample = sampler.observe(
            Duration::from_millis(tick * 250),
            Counters {
                transfer: copied,
                ..Counters::default()
            },
            true,
        );
        {
            let mut progress = status.lock().unwrap();
            progress.copied_bytes = copied;
            progress.current_file_copied = copied;
            progress.throughput = worker_sample.clone();
        }
        // The worker already produced this interval's sample. The periodic
        // sampler therefore has no new rate observation of its own to publish.
        publish_progress_tick(&outbox, &status, None);
        let progress = receive(&mut receiver);
        assert_eq!(progress.copied_bytes, copied);
        assert_eq!(progress.current_file_copied, copied);
        assert!(Arc::ptr_eq(&progress.throughput, &worker_sample));
        assert_eq!(
            progress.throughput.samples.last().unwrap().elapsed_millis,
            tick * 250
        );
        if tick == 3 {
            assert_eq!(progress.throughput.transfer_per_second, 0.0);
        }
        assert_eq!(wakes.load(Ordering::Relaxed), tick as usize);
        assert!(receiver.try_recv().is_err());
    }
}

#[test]
fn full_outbox_does_not_block_and_next_tick_retries_latest_status() {
    let status = running();
    let (outbox, mut receiver, wakes) = outbox();
    outbox
        .sender
        .try_send(Event::Copy(CopyEvent::Progress(Arc::new(snapshot(
            &status,
        )))))
        .unwrap();
    {
        let mut progress = status.lock().unwrap();
        progress.copied_bytes = 262_144;
        progress.current_file_copied = 262_144;
    }
    publish_progress_tick(&outbox, &status, Some(snapshot(&status)));
    assert_eq!(wakes.load(Ordering::Relaxed), 0);
    assert_eq!(receive(&mut receiver).copied_bytes, 0);
    publish_progress_tick(&outbox, &status, None);
    assert_eq!(receive(&mut receiver).copied_bytes, 262_144);
    assert_eq!(wakes.load(Ordering::Relaxed), 1);
}

#[test]
fn closed_outbox_does_not_wake_or_block_the_copy_worker() {
    let status = running();
    let (outbox, receiver, wakes) = outbox();
    drop(receiver);
    publish_progress_tick(&outbox, &status, Some(snapshot(&status)));
    publish_progress_tick(&outbox, &status, None);
    assert_eq!(wakes.load(Ordering::Relaxed), 0);
}
