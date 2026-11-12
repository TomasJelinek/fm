//! Exercise actual tree-produced ownership through every pre-admission rejection.
use super::*;
use crate::{
    providers::{
        MountedFilesystem, OpenedLocation, ProviderError, ProviderRegistry, ProviderTarget,
    },
    tree_actor::{Session, WorkerBudget},
    tree_projection::ProjectionOptions,
};
use std::{path::PathBuf, sync::mpsc as sync_mpsc, thread::ThreadId};

struct Provider {
    root: PathBuf,
    dropped: sync_mpsc::Sender<ThreadId>,
}
impl Drop for Provider {
    fn drop(&mut self) {
        let _ = self.dropped.send(std::thread::current().id());
    }
}
impl MountedFilesystem for Provider {
    fn list(
        &self,
        _: &NodeToken,
        _: &Cancellation,
        _: &mut dyn FnMut(crate::providers::ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        Ok(())
    }
    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        Ok(None)
    }
    fn resolve_target(
        &self,
        _: &NodeToken,
        _: &Cancellation,
    ) -> Result<ProviderTarget, ProviderError> {
        Ok(ProviderTarget::Native(self.root.clone()))
    }
}

fn captured() -> (
    Arc<dyn CapturedUserCommandSources>,
    Arc<Semaphore>,
    sync_mpsc::Receiver<ThreadId>,
) {
    let (sender, receiver) = sync_mpsc::channel();
    let entries: Arc<[Entry]> = Arc::from([]);
    let mut session = Session::new(
        TreeRootSnapshot {
            pane: PaneId::Left,
            navigation: NavigationId(1),
            index: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
        },
        OpenedLocation {
            mount: Arc::new(Provider {
                root: std::env::current_dir().unwrap(),
                dropped: sender,
            }),
            root: NodeToken::new(&[0]).unwrap(),
        },
        Arc::new(ProviderRegistry::default()),
        WorkerBudget::new(Arc::new(Semaphore::new(1))),
        ProjectionOptions::default(),
        &Cancellation::default(),
        crate::ReportArena::testing(),
    )
    .unwrap();
    let publication = session.publication().unwrap();
    assert!(session.commit(&publication.view));
    let request = TreeUserCommandRequest {
        view: publication.view,
        request: TreeRequestId(1),
        selection_revision: publication.selection_revision,
        selection: publication.selection,
        focus: None,
        cancel: Cancellation::default(),
    };
    let capacity = Arc::new(Semaphore::new(1));
    let source = session
        .capture_user_command(
            &request,
            &capacity,
            &crate::tree_rename::Retirement::testing(),
            &Cancellation::default(),
        )
        .unwrap();
    // The captured source must be the only remaining provider owner. Neither a UI
    // retirement clone nor the original session may mask rejection destruction.
    drop(session);
    assert!(receiver.try_recv().is_err());
    (source, capacity, receiver)
}

#[tokio::test]
async fn tree_user_command_early_rejections_retire_actual_captured_provider_off_dispatcher() {
    let caller = std::thread::current().id();
    for cause in [
        "invalid",
        "unavailable",
        "stopped",
        "cleanup-full",
        "queue-full",
    ] {
        let (input, _queued) = mpsc::channel(1);
        let controller = Controller {
            input,
            retirement: RetirementDomain::try_start(2, 0).unwrap(),
            state: Arc::default(),
            stop: Cancellation::default(),
            available: cause != "unavailable",
            task: None,
        };
        let mut occupied = Vec::new();
        if cause == "cleanup-full" {
            for _ in 0..2 {
                occupied.push(controller.retirement.try_reserve().unwrap());
            }
        }
        if cause == "queue-full" {
            controller
                .input
                .try_send(controller.retirement.try_reserve().unwrap().own(Work {
                    request: tests::request(1, &std::env::current_dir().unwrap()),
                    preview: false,
                    cancel: Cancellation::default(),
                    retention: UserCommandRetention::new(()),
                }))
                .unwrap_or_else(|_| panic!("fixture queue was not empty"));
        }
        if cause == "stopped" {
            controller.stop();
        }
        let (source, capacity, dropped) = captured();
        let mut request = tests::request(2, &std::env::current_dir().unwrap());
        request.captured = Some(source);
        if cause == "invalid" {
            request.id = UserCommandRequestId(0);
        }
        let error = controller.start(request, false).unwrap_err();
        let expected = match cause {
            "invalid" => io::ErrorKind::InvalidInput,
            "unavailable" => io::ErrorKind::Unsupported,
            "stopped" => io::ErrorKind::Interrupted,
            _ => io::ErrorKind::WouldBlock,
        };
        assert_eq!(error.kind, expected, "{cause}: {error}");
        assert_ne!(
            dropped.recv_timeout(Duration::from_secs(5)).unwrap(),
            caller,
            "{cause}: final provider owner dropped on the engine dispatcher"
        );
        let _permit = tokio::time::timeout(Duration::from_secs(5), capacity.acquire())
            .await
            .expect("capture permit must return after provider destruction")
            .unwrap();
        assert!(controller.state.lock().unwrap().requests.is_empty());
    }
}

#[test]
fn tree_user_command_captured_provider_retires_without_a_tokio_runtime() {
    let caller = std::thread::current().id();
    let (source, _capacity, dropped) = captured();
    drop(source);
    assert_ne!(
        dropped.recv_timeout(Duration::from_secs(5)).unwrap(),
        caller,
        "final captured provider destruction must use the reserved plain worker"
    );
}
