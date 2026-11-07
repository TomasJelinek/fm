use super::*;
use std::sync::atomic::{AtomicBool, Ordering};

struct UncertainChild {
    polls: mpsc::UnboundedSender<usize>,
    count: usize,
    exited: Arc<AtomicBool>,
}
impl TerminalProcess for UncertainChild {
    fn id(&self) -> u32 {
        1
    }
    fn try_wait(&mut self) -> io::Result<Option<std::process::ExitStatus>> {
        self.count += 1;
        let _ = self.polls.send(self.count);
        if self.count == 1 {
            Err(io::Error::other("Owned child exit is temporarily unknown"))
        } else if self.exited.load(Ordering::Acquire) {
            Ok(Some(std::process::ExitStatus::default()))
        } else {
            Ok(None)
        }
    }
}

#[tokio::test]
async fn fallback_retains_process_quota_after_wait_error_until_confirmed_exit() {
    let (children, reaper) = mpsc::channel(1);
    drop(reaper);
    let processes = Arc::new(Semaphore::new(1));
    let runtime = Runtime::new(Arc::new(Semaphore::new(1)), processes.clone(), children);
    let (polls, mut observed) = mpsc::unbounded_channel();
    let exited = Arc::new(AtomicBool::new(false));
    let child = Box::new(UncertainChild {
        polls,
        count: 0,
        exited: exited.clone(),
    });
    let permit = processes.clone().acquire_owned().await.unwrap();
    assert!(runtime.track(child, permit).await.is_err());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), observed.recv())
            .await
            .unwrap(),
        Some(1)
    );
    assert_eq!(processes.available_permits(), 0, "Wait error is not exit.");
    assert!(processes.clone().try_acquire_owned().is_err());
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), observed.recv())
            .await
            .unwrap(),
        Some(2)
    );
    assert_eq!(processes.available_permits(), 0);
    exited.store(true, Ordering::Release);
    let returned = tokio::time::timeout(Duration::from_secs(5), processes.acquire())
        .await
        .expect("Confirmed exit must return the process slot.")
        .unwrap();
    drop(returned);
    assert_eq!(processes.available_permits(), 1);
}
