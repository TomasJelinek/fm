//! One explicit native detail worker and one coalesced pending request, separate from listings.
use crate::Outbox;
use midnight_core::{
    Event, FileSystem, FsError, VolumeInformationRequest, VolumeInformationResult,
};
use std::{io, sync::Arc};
use tokio::{sync::watch, task::JoinSet};

pub(crate) struct Controller {
    requests: watch::Sender<Option<VolumeInformationRequest>>,
}

impl Controller {
    pub(crate) fn new(
        filesystem: Arc<dyn FileSystem>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (requests, mut receiver) = watch::channel::<Option<VolumeInformationRequest>>(None);
        tasks.spawn(async move {
            while receiver.changed().await.is_ok() {
                let Some(request) = receiver.borrow_and_update().clone() else {
                    continue;
                };
                let filesystem = filesystem.clone();
                let worker_outbox = outbox.clone();
                let fallback = request.clone();
                let completed = tokio::task::spawn_blocking(move || {
                    if request.cancel.is_cancelled() {
                        return;
                    }
                    let result = filesystem.volume_information(&request.path, &request.cancel);
                    worker_outbox.blocking_send(
                        Event::VolumeInformationLoaded(Box::new(VolumeInformationResult {
                            request: request.request,
                            pane: request.pane,
                            navigation: request.navigation,
                            path: request.path,
                            result,
                        })),
                        &request.cancel,
                    );
                })
                .await;
                if completed.is_err() && !fallback.cancel.is_cancelled() {
                    let error = FsError::from_io(
                        "query volume information",
                        &fallback.path,
                        io::Error::other("The native volume information worker failed."),
                    );
                    // Use the same cancellation-aware delivery bound after a worker panic.
                    let outbox = outbox.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        outbox.blocking_send(
                            Event::VolumeInformationLoaded(Box::new(VolumeInformationResult {
                                request: fallback.request,
                                pane: fallback.pane,
                                navigation: fallback.navigation,
                                path: fallback.path,
                                result: Err(error),
                            })),
                            &fallback.cancel,
                        );
                    })
                    .await;
                }
            }
        });
        Self { requests }
    }

    pub(crate) fn query(&self, request: VolumeInformationRequest) {
        self.requests.send_modify(|previous| {
            if let Some(previous) = previous {
                previous.cancel.cancel();
            }
            *previous = Some(request);
        });
    }
}

impl Drop for Controller {
    fn drop(&mut self) {
        if let Some(request) = self.requests.borrow().as_ref() {
            request.cancel.cancel();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::*;
    use std::{
        path::Path,
        sync::{
            Mutex,
            atomic::{AtomicUsize, Ordering},
        },
        time::Duration,
    };

    #[derive(Default)]
    struct Native {
        active: AtomicUsize,
        maximum: AtomicUsize,
        calls: Mutex<Vec<std::path::PathBuf>>,
    }
    impl DirectoryService for Native {
        fn read_directory(
            &self,
            _: &Path,
            _: &Cancellation,
            _: &mut dyn FnMut(Entry) -> bool,
        ) -> Result<(), FsError> {
            unreachable!()
        }
        fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
            unreachable!()
        }
    }
    impl VolumeService for Native {
        fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
            unreachable!()
        }
    }
    impl StorageService for Native {
        fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
            unreachable!()
        }
        fn volume_information(
            &self,
            path: &Path,
            cancel: &Cancellation,
        ) -> Result<VolumeInformation, FsError> {
            let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
            self.maximum.fetch_max(active, Ordering::SeqCst);
            self.calls.lock().unwrap().push(path.to_owned());
            if path.ends_with("slow") {
                while !cancel.is_cancelled() {
                    std::thread::sleep(Duration::from_millis(2));
                }
            }
            self.active.fetch_sub(1, Ordering::SeqCst);
            Ok(VolumeInformation {
                filesystem: Some(path.as_os_str().to_owned()),
                ..Default::default()
            })
        }
    }
    fn request(id: u64, name: &str) -> VolumeInformationRequest {
        VolumeInformationRequest {
            request: DesktopRequestId(id),
            pane: PaneId::Left,
            navigation: NavigationId(id),
            path: std::env::temp_dir().join(name),
            cancel: Cancellation::default(),
        }
    }

    #[tokio::test]
    async fn new_requests_cancel_and_coalesce_without_accumulating_native_workers() {
        let native = Arc::new(Native::default());
        let (sender, mut events) = tokio::sync::mpsc::channel(1);
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            native.clone(),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        controller.query(request(1, "slow"));
        tokio::time::timeout(Duration::from_secs(2), async {
            while native.active.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        for id in 2..=100 {
            controller.query(request(id, "latest"));
        }
        let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
            .await
            .unwrap()
            .unwrap();
        let Event::VolumeInformationLoaded(result) = event else {
            panic!()
        };
        assert_eq!(result.request, DesktopRequestId(100));
        assert_eq!(result.navigation, NavigationId(100));
        assert_eq!(native.maximum.load(Ordering::SeqCst), 1);
        assert_eq!(native.calls.lock().unwrap().len(), 2);
        drop(controller);
        tasks.join_next().await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn shutdown_cancels_a_pending_native_query_without_publishing() {
        let native = Arc::new(Native::default());
        let (sender, mut events) = tokio::sync::mpsc::channel(1);
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            native.clone(),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        controller.query(request(1, "slow"));
        tokio::time::timeout(Duration::from_secs(2), async {
            while native.active.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        drop(controller);
        tokio::time::timeout(Duration::from_secs(2), tasks.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(events.recv().await.is_none());
    }
}
