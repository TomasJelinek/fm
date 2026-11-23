//! One worker answers whether a typed Copy/Move destination is an existing folder, and
//! another what the dialog's destination does with links. A newer question supersedes a
//! pending one of its kind; nothing here blocks the command loop.
use crate::{Outbox, task_lifetime::TaskLifetime};
use midnight_core::{
    Cancellation, DestinationCheckRequest, DestinationCheckResult, Event, FileSystem, FsError,
    TransferLinkFacts, TransferLinksRequest, TransferLinksResult, TransferService,
};
use std::{io, sync::Arc};
use tokio::{sync::watch, task::JoinSet};

pub(crate) struct Controller {
    requests: watch::Sender<Option<DestinationCheckRequest>>,
    links: watch::Sender<Option<TransferLinksRequest>>,
}

impl Controller {
    pub(crate) fn new(
        scope: TaskLifetime,
        filesystem: Arc<dyn FileSystem>,
        transfers: Option<Arc<dyn TransferService>>,
        outbox: Outbox,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let links = spawn_link_inspection(scope.clone(), transfers.clone(), outbox.clone(), tasks);
        let (requests, mut receiver) = watch::channel::<Option<DestinationCheckRequest>>(None);
        let worker_scope = scope.clone();
        tasks.spawn(scope.future(async move {
            while receiver.changed().await.is_ok() {
                let Some(request) = receiver.borrow_and_update().clone() else {
                    continue;
                };
                let filesystem = filesystem.clone();
                let transfers = transfers.clone();
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
                    // What an existing folder does with links, when the dialog asks.
                    let links = (request.links && matches!(result, Ok(true)))
                        .then(|| {
                            link_facts(
                                transfers.as_deref(),
                                &TransferLinksRequest {
                                    request: request.request,
                                    destination: request.path.clone(),
                                    source: request.source.clone(),
                                    cancel: request.cancel.clone(),
                                },
                            )
                            .ok()
                        })
                        .flatten();
                    worker_outbox.blocking_send(
                        Event::TransferDestinationChecked(Box::new(DestinationCheckResult {
                            request: request.request,
                            path: request.path,
                            result,
                            links,
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
                                links: None,
                            })),
                            &fallback.cancel,
                        );
                    })
                    .await;
                }
            }
        }));
        Self { requests, links }
    }

    pub(crate) fn check(&self, request: DestinationCheckRequest) {
        self.requests.send_modify(|previous| {
            if let Some(previous) = previous {
                previous.cancel.cancel();
            }
            *previous = Some(request);
        });
    }

    pub(crate) fn inspect_links(&self, request: TransferLinksRequest) {
        self.links.send_modify(|previous| {
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
        if let Some(request) = self.links.borrow().as_ref() {
            request.cancel.cancel();
        }
    }
}

/// The worker that answers `Command::InspectTransferLinks`, one question at a time.
fn spawn_link_inspection(
    scope: TaskLifetime,
    transfers: Option<Arc<dyn TransferService>>,
    outbox: Outbox,
    tasks: &mut JoinSet<()>,
) -> watch::Sender<Option<TransferLinksRequest>> {
    let (requests, mut receiver) = watch::channel::<Option<TransferLinksRequest>>(None);
    let worker_scope = scope.clone();
    tasks.spawn(scope.future(async move {
        while receiver.changed().await.is_ok() {
            let Some(request) = receiver.borrow_and_update().clone() else {
                continue;
            };
            let transfers = transfers.clone();
            let worker_outbox = outbox.clone();
            let fallback = request.clone();
            let completed = tokio::task::spawn_blocking(worker_scope.blocking(move || {
                if request.cancel.is_cancelled() {
                    return;
                }
                let result = link_facts(transfers.as_deref(), &request);
                worker_outbox.blocking_send(
                    Event::TransferLinksInspected(Box::new(TransferLinksResult {
                        request: request.request,
                        destination: request.destination,
                        result,
                    })),
                    &request.cancel,
                );
            }))
            .await;
            if completed.is_err() && !fallback.cancel.is_cancelled() {
                let error = FsError::from_io(
                    "inspect destination links",
                    &fallback.destination,
                    io::Error::other("The destination's link support could not be checked."),
                );
                let outbox = outbox.clone();
                let _ = tokio::task::spawn_blocking(move || {
                    outbox.blocking_send(
                        Event::TransferLinksInspected(Box::new(TransferLinksResult {
                            request: fallback.request,
                            destination: fallback.destination,
                            result: Err(error),
                        })),
                        &fallback.cancel,
                    );
                })
                .await;
            }
        }
    }));
    requests
}

/// What the destination's volume reports about links, and whether the selection's folder
/// is on that volume. Without a transfer service nothing is known.
fn link_facts(
    transfers: Option<&dyn TransferService>,
    request: &TransferLinksRequest,
) -> Result<TransferLinkFacts, FsError> {
    move_facts(
        transfers,
        request.source.as_deref(),
        &request.destination,
        &request.cancel,
    )
}

/// What `destination` does with links, and whether the folder `source` is on its volume
/// as a rename sees it (`TransferLinkDestination::same_volume_as`): the one determination
/// that the Copy/Move dialog's facts (O-1), a drop's admission and a Move job's admission
/// share (`move_refusal`). A source folder that cannot be read is not known to share the
/// volume.
pub(crate) fn move_facts(
    transfers: Option<&dyn TransferService>,
    source: Option<&std::path::Path>,
    destination: &std::path::Path,
    cancel: &Cancellation,
) -> Result<TransferLinkFacts, FsError> {
    let Some(transfers) = transfers else {
        return Ok(TransferLinkFacts::default());
    };
    let destination = transfers.link_destination(destination, cancel)?;
    let source = source.and_then(|source| transfers.link_destination(source, cancel).ok());
    Ok(TransferLinkFacts::new(destination, source.as_ref()))
}

/// Why a Move of the items at `sources` to `destination` must not start, as the
/// Copy/Move dialog says it (`move_facts`): the folder of one of them is not on the
/// destination's volume, and the destination refuses a Move from another volume. None
/// where the Move may start, and where the destination's facts cannot be read (the job
/// then decides). Each folder's facts are read once. A folder whose facts cannot be read
/// is not taken for another drive: its read error is the answer. A cancelled check ends
/// as cancelled.
pub(crate) fn move_refusal<'a>(
    transfers: &dyn TransferService,
    sources: impl IntoIterator<Item = &'a std::path::Path>,
    destination: &std::path::Path,
    cancel: &Cancellation,
) -> Result<Option<String>, FsError> {
    let facts = transfers.link_destination(destination, cancel);
    admission_current(cancel, destination, &facts)?;
    let Ok(destination) = facts else {
        return Ok(None);
    };
    // Only a destination that refuses such a Move can refuse this one.
    if destination.cross_volume_move_refusal().is_none() {
        return Ok(None);
    }
    // Each distinct folder is read once, however many selected items it holds.
    let mut folders = std::collections::HashSet::new();
    for source in sources {
        let folder = source.parent().unwrap_or(source);
        if !folders.insert(folder) {
            continue;
        }
        let facts = transfers.link_destination(folder, cancel);
        admission_current(cancel, folder, &facts)?;
        let refusal = TransferLinkFacts::new(destination.clone(), Some(&facts?)).move_refusal();
        if refusal.is_some() {
            return Ok(refusal);
        }
    }
    Ok(None)
}

/// A cancelled read during a Move's admission ends the admission as cancelled, whatever
/// the read answered.
fn admission_current<T>(
    cancel: &Cancellation,
    path: &std::path::Path,
    read: &Result<T, FsError>,
) -> Result<(), FsError> {
    let interrupted = matches!(read, Err(error) if error.kind == io::ErrorKind::Interrupted);
    if cancel.is_cancelled() || interrupted {
        return Err(FsError::from_io(
            "admit Move",
            path,
            io::Error::new(io::ErrorKind::Interrupted, "Move cancelled."),
        ));
    }
    Ok(())
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
            None,
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
                links: true,
                source: None,
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
            // Without a transfer service a folder's links are unknown, never invented.
            assert_eq!(
                checked.links,
                (name == "folder").then(TransferLinkFacts::default)
            );
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

    /// A USB stick (volume 7) and a fixed disk (volume 1), as a native service reports them.
    struct Volumes;
    impl TransferService for Volumes {
        fn link_destination(
            &self,
            path: &Path,
            _: &Cancellation,
        ) -> Result<TransferLinkDestination, FsError> {
            let removable = path.starts_with(std::env::temp_dir().join("stick"));
            if path.ends_with("unreadable") {
                return Err(FsError::from_io(
                    "inspect links",
                    path,
                    io::ErrorKind::PermissionDenied.into(),
                ));
            }
            Ok(TransferLinkDestination {
                storage: TransferLinkStorage::Supported,
                filesystem: None,
                network: false,
                removable,
                volume: Some(if removable { 7 } else { 1 }),
                refuses_cross_volume_move: false,
            })
        }
        fn prepare_transfer_source(
            &self,
            _: &Path,
            _: &Cancellation,
        ) -> Result<Box<dyn TransferSource>, FsError> {
            unreachable!("inspecting links opens no source")
        }
        fn prepare_transfer_destination(
            &self,
            _: &Path,
            _: &Cancellation,
        ) -> Result<Arc<dyn TransferDirectory>, FsError> {
            unreachable!("inspecting links opens no destination")
        }
    }

    #[tokio::test]
    async fn link_facts_name_the_destination_and_whether_the_selection_is_on_its_volume() {
        let native = Arc::new(Native);
        let (sender, mut events) = tokio::sync::mpsc::channel(1);
        let mut tasks = JoinSet::new();
        let controller = Controller::new(
            TaskLifetime::testing(native.clone()),
            native,
            Some(Arc::new(Volumes)),
            Outbox {
                sender,
                wakeup: Arc::new(|| {}),
            },
            &mut tasks,
        );
        let temp = std::env::temp_dir();
        let stick = temp.join("stick").join("backup");
        let cases = [
            // From the fixed disk to the stick: removable, another volume.
            (1, stick.clone(), Some(temp.join("home")), Some(false)),
            // Within the stick: the same volume.
            (
                2,
                stick.clone(),
                Some(temp.join("stick").join("old")),
                Some(true),
            ),
            // No single source folder (a captured tree): not the same volume.
            (3, stick.clone(), None, Some(false)),
            // An unreadable source folder is simply not known to share the volume.
            (4, stick, Some(temp.join("unreadable")), Some(false)),
            // An unreadable destination is the answer's error.
            (5, temp.join("unreadable"), None, None),
        ];
        for (id, destination, source, same_volume) in cases {
            controller.inspect_links(TransferLinksRequest {
                request: DesktopRequestId(id),
                destination: destination.clone(),
                source,
                cancel: Cancellation::default(),
            });
            let event = tokio::time::timeout(Duration::from_secs(2), events.recv())
                .await
                .unwrap()
                .unwrap();
            let Event::TransferLinksInspected(inspected) = event else {
                panic!("a link inspection answers with its own event")
            };
            assert_eq!(inspected.request, DesktopRequestId(id));
            assert_eq!(inspected.destination, destination);
            match same_volume {
                Some(same) => {
                    let facts = inspected.result.unwrap();
                    assert_eq!(facts.same_volume, same, "case {id}");
                    assert_eq!(facts.destination.volume, Some(7));
                    assert!(facts.destination.removable);
                }
                None => assert_eq!(
                    inspected.result.unwrap_err().kind,
                    io::ErrorKind::PermissionDenied
                ),
            }
        }
        drop(controller);
        while let Some(task) = tasks.join_next().await {
            task.unwrap();
        }
    }
}

#[cfg(test)]
#[path = "transfer_places_fixture.rs"]
pub(crate) mod places;

#[cfg(test)]
#[path = "transfer_destination_volume_tests.rs"]
mod volume_tests;
