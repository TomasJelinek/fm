//! Actor abort must retain native pins and provider captures until worker retirement.
use super::*;
use std::sync::mpsc as channel;
use std::thread::{ThreadId, current};

type Drops = channel::Sender<(&'static str, ThreadId)>;

struct Service(Drops);
struct Target {
    item: MutationItem,
    drops: Drops,
}
impl Drop for Target {
    fn drop(&mut self) {
        let _ = self.drops.send(("native", current().id()));
    }
}
impl MutationService for Service {
    fn prepare_create(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn PreparedDirectory>, FsError> {
        unreachable!("the retirement fixture only prepares Delete")
    }
    fn prepare(&self, path: &Path, _: &Cancellation) -> Result<Arc<dyn PreparedMutation>, FsError> {
        Ok(Arc::new(Target {
            item: MutationItem {
                path: path.into(),
                kind: EntryKind::File,
                metadata: Default::default(),
            },
            drops: self.0.clone(),
        }))
    }
}
impl PreparedMutation for Target {
    fn item(&self) -> &MutationItem {
        &self.item
    }
    fn rename(&self, _: &OsStr, _: &Cancellation) -> Result<PathBuf, FsError> {
        unreachable!("the retirement fixture never renames")
    }
    fn delete(
        &self,
        _: DeleteMode,
        _: &Cancellation,
        _: &mut dyn FnMut(&Path),
    ) -> Result<(), FsError> {
        panic!("an unconfirmed Delete must not execute")
    }
}

#[derive(Debug)]
struct Capture {
    path: PathBuf,
    drops: Drops,
}
impl Drop for Capture {
    fn drop(&mut self) {
        let _ = self.drops.send(("provider", current().id()));
    }
}
impl CapturedDeleteSources for Capture {
    fn stamp(&self) -> TreeViewStamp {
        TreeViewStamp {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            session: TreeSessionId(1),
            revision: TreeRevision(1),
        }
    }
    fn request(&self) -> TreeRequestId {
        TreeRequestId(1)
    }
    fn mode(&self) -> DeleteMode {
        DeleteMode::Permanent
    }
    fn root_count(&self) -> usize {
        1
    }
    fn prepare(
        &self,
        service: &dyn MutationService,
        cancel: &Cancellation,
    ) -> Result<Vec<Arc<dyn PreparedMutation>>, FsError> {
        Ok(vec![service.prepare(&self.path, cancel)?])
    }
}

fn fixture() -> (
    Harness,
    PrepareCapturedDelete,
    channel::Receiver<(&'static str, ThreadId)>,
) {
    let (drops, received) = channel::channel();
    let h = Harness::using(
        MutationKind::Delete(DeleteMode::Permanent),
        Some(Arc::new(Service(drops.clone()))),
    );
    let request = PrepareCapturedDelete {
        id: h.request.id,
        source_directory: h.request.source.path.clone(),
        sources: Arc::new(Capture {
            path: h.request.source.path.join("nested/owned.txt"),
            drops,
        }),
    };
    (h, request, received)
}

fn assert_worker_retirement(received: channel::Receiver<(&'static str, ThreadId)>) {
    let native = received.recv_timeout(Duration::from_secs(5)).unwrap();
    let provider = received.recv_timeout(Duration::from_secs(5)).unwrap();
    assert_eq!((native.0, provider.0), ("native", "provider"));
    assert_ne!(
        native.1,
        current().id(),
        "native pin dropped on actor executor"
    );
    assert_ne!(
        provider.1,
        current().id(),
        "provider capture dropped on actor executor"
    );
    assert!(received.try_recv().is_err(), "each owner is destroyed once");
}

#[tokio::test]
async fn actor_abort_retires_prepared_confirmation_off_executor_in_native_then_provider_order() {
    let (mut h, request, received) = fixture();
    h.controller.prepare_captured_delete(request).await;
    h.preview().await;
    assert!(received.try_recv().is_err());
    h.tasks.abort_all();
    while h.tasks.join_next().await.is_some() {}
    assert_worker_retirement(received);
}

#[tokio::test]
async fn actor_abort_during_prepared_publication_retires_the_local_job_off_executor() {
    let (mut h, request, received) = fixture();
    h.controller.prepare_captured_delete(request).await;
    let progress = Arc::new(worker::initial(&h.request));
    while h
        .controller
        .outbox
        .sender
        .try_send(Event::Operation(OperationEvent::Progress(progress.clone())))
        .is_ok()
    {}
    // Exact actor state proves preparation finished and publication is pending;
    // the full output channel prevents insertion into the prepared collection.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let awaiting = h
                .controller
                .registry
                .lock()
                .unwrap()
                .get(&h.request.id)
                .unwrap()
                .status
                .lock()
                .unwrap()
                .state
                == OperationState::AwaitingConfirmation;
            if awaiting {
                break;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(received.try_recv().is_err());
    h.tasks.abort_all();
    while h.tasks.join_next().await.is_some() {}
    assert_worker_retirement(received);
}

type PreparationTask = tokio::task::JoinHandle<Result<WorkerOwned<worker::Job>, FsError>>;

fn pending_preparation() -> (PreparationTask, channel::Receiver<(&'static str, ThreadId)>) {
    let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
    let lease = domain.try_reserve().unwrap();
    let (drops, received) = channel::channel();
    let path = std::env::temp_dir().join("owned-actor-retirement");
    let request = PrepareCapturedDelete {
        id: OperationId(1),
        source_directory: path.clone(),
        sources: Arc::new(Capture {
            path: path.join("child.txt"),
            drops: drops.clone(),
        }),
    };
    let scope = DeleteScope::capture(request.sources.as_ref());
    let service: Arc<dyn MutationService> = Arc::new(Service(drops));
    let context = lease.own((Input::Delete(request, scope), Some(service), None));
    let status = Arc::new(Mutex::new(worker::initial_for(
        OperationId(1),
        MutationKind::Delete(DeleteMode::Permanent),
        path,
    )));
    let pending = tokio::task::spawn_blocking(move || {
        prepare_on_worker(
            context,
            Cancellation::default(),
            status,
            Arc::new(Semaphore::new(MAX_METADATA_TARGETS)),
        )
    });
    (pending, received)
}

#[tokio::test]
async fn unconsumed_preparation_output_retires_native_then_provider_off_executor() {
    let (pending, received) = pending_preparation();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !pending.is_finished() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(received.try_recv().is_err());
    drop(pending);
    assert_worker_retirement(received);
}

#[tokio::test]
async fn predecessor_and_mutation_gate_waits_keep_guarded_jobs_when_aborted() {
    use std::future::Future;
    use std::task::{Context, Poll, Waker};
    for predecessor in [false, true] {
        let (pending, received) = pending_preparation();
        let mut job = pending.await.unwrap().unwrap();
        job.confirm(None).unwrap();
        let gate = Arc::new(Semaphore::new(1));
        let held_gate = (!predecessor).then(|| gate.clone().try_acquire_owned().unwrap());
        let copy_capacity = Arc::new(Semaphore::new(1));
        let _copy_permit = copy_capacity.try_acquire_owned().unwrap();
        let copies = CopyFence::default();
        let _receipt = copies.register();
        if predecessor {
            job.predecessors = Arc::new(copies.snapshot());
        }
        let mut waiting = Box::pin(acquire_mutation(job, gate));
        assert!(matches!(
            waiting
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        assert!(received.try_recv().is_err());
        drop(waiting);
        assert_worker_retirement(received);
        drop(held_gate);
    }
}

struct GatedLocale {
    entered: channel::Sender<ThreadId>,
    release: Mutex<channel::Receiver<()>>,
}
impl LocaleService for GatedLocale {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(NumberLocale::default())
    }
}
impl Drop for GatedLocale {
    fn drop(&mut self) {
        let _ = self.entered.send(current().id());
        let _ = self.release.lock().unwrap().recv();
    }
}
struct PanickingService;
impl MutationService for PanickingService {
    fn prepare_create(
        &self,
        _: &Path,
        _: &Cancellation,
    ) -> Result<Arc<dyn PreparedDirectory>, FsError> {
        panic!("injected provider panic")
    }
    fn prepare(&self, _: &Path, _: &Cancellation) -> Result<Arc<dyn PreparedMutation>, FsError> {
        panic!("injected provider panic")
    }
}

#[tokio::test]
async fn failed_or_panicked_preparation_keeps_slot_until_final_locale_owner_drops() {
    let mut available = Vec::new();
    for panic in [false, true] {
        let domain = crate::retirement::RetirementDomain::try_start(1, 0).unwrap();
        let lease = domain.try_reserve().unwrap();
        let (entered, observed) = channel::channel();
        let (release, released) = channel::channel();
        let locale: Arc<dyn LocaleService> = Arc::new(GatedLocale {
            entered,
            release: Mutex::new(released),
        });
        let (drops, _received) = channel::channel();
        let path = std::env::temp_dir().join("owned-actor-locale-retirement");
        let request = PrepareCapturedDelete {
            id: OperationId(1),
            source_directory: path.clone(),
            sources: Arc::new(Capture {
                path: path.join("child.txt"),
                drops,
            }),
        };
        let scope = DeleteScope::capture(request.sources.as_ref());
        let service = panic.then(|| Arc::new(PanickingService) as Arc<dyn MutationService>);
        let context = lease.own((Input::Delete(request, scope), service, Some(locale)));
        let status = Arc::new(Mutex::new(worker::initial_for(
            OperationId(1),
            MutationKind::Delete(DeleteMode::Permanent),
            path,
        )));
        let pending = tokio::task::spawn_blocking(move || {
            prepare_on_worker(
                context,
                Cancellation::default(),
                status,
                Arc::new(Semaphore::new(MAX_METADATA_TARGETS)),
            )
        });
        let thread = observed.recv_timeout(Duration::from_secs(5)).unwrap();
        assert_ne!(thread, current().id());
        let prematurely_available = domain.try_reserve().is_ok();
        release.send(()).unwrap();
        let result = pending.await;
        if panic {
            match result {
                Err(error) => assert!(error.is_panic()),
                Ok(_) => panic!("injected provider panic must be reported"),
            }
        } else {
            assert!(result.unwrap().is_err());
        }
        available.push((panic, prematurely_available));
    }
    assert!(
        available.iter().all(|(_, free)| !free),
        "slot returned before final locale Drop: {available:?}"
    );
}
