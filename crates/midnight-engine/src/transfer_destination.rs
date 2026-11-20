//! One worker answers whether a typed Copy/Move destination is an existing folder.
//! A newer question supersedes a pending one; nothing here blocks the command loop.
use crate::{Outbox, task_lifetime::TaskLifetime};
use midnight_core::{DestinationCheckRequest, DestinationCheckResult, Event, FileSystem, FsError};
use std::{io, sync::Arc};
use tokio::{sync::watch, task::JoinSet};

pub(crate) struct Controller {
    requests: watch::Sender<Option<DestinationCheckRequest>>,
}

impl Controller {
    pub(crate) fn new(
        scope: TaskLifetime,
        filesystem: Arc<dyn FileSystem>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let (requests, mut receiver) = watch::channel::<Option<DestinationCheckRequest>>(None);
        let worker_scope = scope.clone();
        tasks.spawn(scope.future(async move {
            while receiver.changed().await.is_ok() {
                let Some(request) = receiver.borrow_and_update().clone() else {
                    continue;
                };
                let filesystem = filesystem.clone();
                let worker_outbox = outbox.clone();
                let fallback = request.clone();
                let completed = tokio::task::spawn_blocking(worker_scope.blocking(move || {
                    if request.cancel.is_cancelled() {
                        return;
                    }
                    // Links are followed: a link to a folder is a destination folder.
                    let result = filesystem
                        .browse_directory_identity(&request.path, &request.cancel)
                        .map(|identity| identity.is_some());
                    worker_outbox.blocking_send(
                        Event::TransferDestinationChecked(Box::new(DestinationCheckResult {
                            request: request.request,
                            path: request.path,
                            result,
                        })),
                        &request.cancel,
                    );
                }))
                .await;
                if completed.is_err() && !fallback.cancel.is_cancelled() {
                    let error = FsError::from_io(
                        "check destination folder",
                        &fallback.path,
                        io::Error::other("The destination check failed."),
                    );
                    let outbox = outbox.clone();
                    let _ = tokio::task::spawn_blocking(move || {
                        outbox.blocking_send(
                            Event::TransferDestinationChecked(Box::new(DestinationCheckResult {
                                request: fallback.request,
                                path: fallback.path,
                                result: Err(error),
                            })),
                            &fallback.cancel,
                        );
                    })
                    .await;
                }
            }
        }));
        Self { requests }
    }

    pub(crate) fn check(&self, request: DestinationCheckRequest) {
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
    use std::{path::Path, time::Duration};

    /// A folder, a file and nothing else, as `browse_directory_identity` reports them.
    struct Native;
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
        fn resolve_size_directory(
            &self,
            path: &Path,
            _: &Cancellation,
        ) -> Result<Option<SizeDirectoryTarget>, FsError> {
            match path.file_name().and_then(|name| name.to_str()) {
                Some("folder") => Ok(Some(SizeDirectoryTarget {
                    path: path.to_owned(),
                    identity: SizeDirectoryIdentity {
                        device: 1,
                        object: [2; 16],
                    },
                })),
                Some("file") => Ok(None),
                _ => Err(FsError::from_io(
                    "resolve directory size target",
                    path,
                    io::ErrorKind::NotFound.into(),
                )),
            }
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
    }

    #[tokio::test]
    async fn a_destination_is_a_folder_something_else_or_missing() {
        let native = Arc::new(Native);
        let (sender, mut events) = tokio::sync::mpsc::channel(1);
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            TaskLifetime::testing(native.clone()),
            native.clone(),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        for (id, name) in [(1, "folder"), (2, "file"), (3, "missing")] {
            let path = std::env::temp_dir().join(name);
            controller.check(DestinationCheckRequest {
                request: DesktopRequestId(id),
                path: path.clone(),
                cancel: Cancellation::default(),
            });
            let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap();
            let Event::TransferDestinationChecked(checked) = event else {
                panic!("a destination check answers with its own event")
            };
            assert_eq!(checked.request, DesktopRequestId(id));
            assert_eq!(checked.path, path);
            match name {
                "folder" => assert!(matches!(checked.result, Ok(true))),
                "file" => assert!(matches!(checked.result, Ok(false))),
                _ => assert!(
                    matches!(&checked.result, Err(error) if error.kind == io::ErrorKind::NotFound)
                ),
            }
        }
        drop(controller);
        tasks.join_next().await.unwrap().unwrap();
    }
}
