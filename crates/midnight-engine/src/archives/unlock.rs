//! `Command::UnlockArchive`: verify the password of one archive mount on a backend
//! worker. The password lives only in the command and in that worker's call; the
//! answer (`Event::ArchiveUnlocked`) carries none of it.
use super::*;

impl Controller {
    pub(crate) fn unlock(
        &self,
        request: DesktopRequestId,
        mount: Arc<dyn ArchiveMount>,
        password: ArchivePassword,
        tasks: &mut JoinSet<()>,
    ) {
        let outbox = self.outbox.clone();
        let scope = self.scope.clone();
        let cancel = self.unlocks.clone();
        tasks.spawn(self.scope.future(async move {
            let result = tokio::task::spawn_blocking(scope.blocking(move || {
                let result = mount.unlock(&password, &cancel);
                // Zeroized here, before the answer leaves the worker.
                drop(password);
                // The command's reference is given up on this backend worker; the pane
                // keeps its own.
                let _ = mount.release_on_worker();
                result
            }))
            .await
            .unwrap_or_else(|error| {
                Err(FsError::from_io(
                    "archive password",
                    Path::new(""),
                    std::io::Error::other(error.to_string()),
                ))
            });
            outbox
                .send(Event::ArchiveUnlocked { request, result })
                .await;
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A mount that accepts one password.
    #[derive(Debug, Default)]
    struct Locked {
        unlocked: AtomicBool,
    }
    impl ArchiveMount for Locked {
        fn id(&self) -> u64 {
            9
        }
        fn list(&self, _: u64, _: &Cancellation) -> Result<Vec<ArchiveNode>, FsError> {
            Ok(Vec::new())
        }
        fn read(&self, _: u64, _: u64, _: u32, _: &Cancellation) -> Result<Vec<u8>, FsError> {
            Err(archive_password_required(Path::new("")))
        }
        fn validate(&self) -> Result<(), FsError> {
            Ok(())
        }
        fn release_on_worker(self: Arc<Self>) -> Result<(), FsError> {
            Ok(())
        }
        fn unlocked(&self) -> bool {
            self.unlocked.load(Ordering::Acquire)
        }
        fn unlock(
            &self,
            password: &ArchivePassword,
            _: &Cancellation,
        ) -> Result<ArchiveUnlock, FsError> {
            if password.as_bytes() == "správné".as_bytes() {
                self.unlocked.store(true, Ordering::Release);
                Ok(ArchiveUnlock::Unlocked)
            } else {
                Ok(ArchiveUnlock::WrongPassword)
            }
        }
    }

    #[tokio::test]
    async fn the_answer_says_whether_the_password_opened_the_mount_and_never_holds_it() {
        let (sender, mut events) = tokio::sync::mpsc::channel(4);
        let controller = Controller::testing(
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            crate::test_snapshots(Default::default()),
            Arc::new(Semaphore::new(1)),
        );
        let mount = Arc::new(Locked::default());
        let mut tasks = JoinSet::new();
        for (id, password, expected) in [
            (1, "špatné", ArchiveUnlock::WrongPassword),
            (2, "správné", ArchiveUnlock::Unlocked),
        ] {
            controller.unlock(
                DesktopRequestId(id),
                mount.clone(),
                ArchivePassword::new(password.into()),
                &mut tasks,
            );
            let event = events.recv().await.unwrap();
            let text = format!("{event:?}");
            assert!(!text.contains(password), "{text}");
            let Event::ArchiveUnlocked { request, result } = event else {
                panic!("{text}");
            };
            assert_eq!(request, DesktopRequestId(id));
            assert_eq!(result.unwrap(), expected);
        }
        assert!(mount.unlocked());
    }
}
