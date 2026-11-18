use std::sync::Arc;
use tokio::sync::mpsc;

pub type Retired = Vec<Box<dyn Send>>;

/// Retain the closed receiver so late reserved sends are reclaimed off the UI thread.
pub fn retire_receiver<T: Send + 'static>(mut receiver: mpsc::Receiver<T>, retired: &mut Retired) {
    receiver.close();
    retired.push(Box::new(receiver));
}

/// Backend service releasing large listing allocations away from the UI thread.
pub fn start(
    runtime: &tokio::runtime::Handle,
    wake: Arc<dyn Fn() + Send + Sync>,
) -> mpsc::Sender<Retired> {
    let (sender, mut receiver) = mpsc::channel::<Retired>(4);
    runtime.spawn(async move {
        while let Some(retired) = receiver.recv().await {
            let _ = tokio::task::spawn_blocking(move || drop(retired)).await;
            wake();
        }
    });
    sender
}

/// A FIFO barrier lets shutdown finish the last reclamation before stopping
/// the runtime. It is used after the window has closed, never in a UI callback.
pub async fn barrier(queue: &mpsc::Sender<Retired>) -> Result<(), String> {
    struct Completion(Option<tokio::sync::oneshot::Sender<()>>);
    impl Drop for Completion {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(());
            }
        }
    }
    let (sender, receiver) = tokio::sync::oneshot::channel();
    queue
        .send(vec![Box::new(Completion(Some(sender)))])
        .await
        .map_err(|_| "reclaimer stopped".to_string())?;
    receiver
        .await
        .map_err(|_| "reclaimer did not complete".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::oneshot;

    struct DropProbe(Option<oneshot::Sender<std::thread::ThreadId>>);
    impl Drop for DropProbe {
        fn drop(&mut self) {
            if let Some(sender) = self.0.take() {
                let _ = sender.send(std::thread::current().id());
            }
        }
    }

    #[tokio::test]
    async fn retiring_a_listing_destroys_it_on_a_background_worker() {
        let caller = std::thread::current().id();
        let (dropped, receiver) = oneshot::channel();
        let queue = start(&tokio::runtime::Handle::current(), Arc::new(|| {}));
        assert!(
            queue
                .send(vec![Box::new(DropProbe(Some(dropped)))])
                .await
                .is_ok(),
            "reclaimer must accept ownership"
        );
        let thread = tokio::time::timeout(std::time::Duration::from_secs(3), receiver)
            .await
            .unwrap()
            .unwrap();
        assert_ne!(
            thread, caller,
            "directory destruction must not run on the UI caller"
        );
    }

    #[tokio::test]
    async fn a_send_reserved_before_close_is_retired_with_the_receiver_on_a_backend_thread() {
        let caller = std::thread::current().id();
        let (sender, receiver) = mpsc::channel(1);
        let permit = sender.reserve().await.unwrap();
        let (dropped, mut notification) = oneshot::channel();
        let mut retired = Retired::new();
        retire_receiver(receiver, &mut retired);
        // Tokio permits remain usable after close. This arrives after a one-time
        // drain would have observed an empty queue.
        permit.send(DropProbe(Some(dropped)));
        drop(sender);
        assert!(
            notification.try_recv().is_err(),
            "late payload was released on the UI caller"
        );
        tokio::task::spawn_blocking(move || drop(retired))
            .await
            .unwrap();
        assert_ne!(notification.await.unwrap(), caller);
    }

    #[tokio::test]
    async fn a_closed_reclamation_queue_keeps_receiver_and_buffered_payload_ownership() {
        let caller = std::thread::current().id();
        let (sender, receiver) = mpsc::channel(1);
        let (dropped, mut notification) = oneshot::channel();
        assert!(sender.send(DropProbe(Some(dropped))).await.is_ok());
        drop(sender);
        let mut retired = Retired::new();
        retire_receiver(receiver, &mut retired);
        // A stopped reclaimer hands the batch back instead of dropping it here.
        let (queue, closed) = mpsc::channel(1);
        drop(closed);
        let retired = queue.try_send(retired).unwrap_err().into_inner();
        assert!(
            notification.try_recv().is_err(),
            "the buffered payload was released on the caller"
        );
        tokio::task::spawn_blocking(move || drop(retired))
            .await
            .unwrap();
        assert_ne!(notification.await.unwrap(), caller);
    }

    #[tokio::test]
    async fn shutdown_barrier_waits_for_prior_retired_objects() {
        let queue = start(&tokio::runtime::Handle::current(), Arc::new(|| {}));
        let (dropped, mut notification) = oneshot::channel();
        assert!(
            queue
                .send(vec![Box::new(DropProbe(Some(dropped)))])
                .await
                .is_ok()
        );
        barrier(&queue).await.unwrap();
        assert!(notification.try_recv().is_ok());
    }
}
