//! Final queued capture disposal keeps real native ownership and admission off the caller.
use super::*;
use crate::{
    providers::{
        MountedFilesystem, OpenedLocation, ProviderEntry, ProviderRegistry, ProviderTarget,
    },
    tree_actor::WorkerBudget,
    tree_projection::ProjectionOptions,
};
use std::sync::mpsc;
struct Observed {
    graph: Arc<Graph>,
    _file: std::fs::File,
    entered: mpsc::Sender<std::thread::ThreadId>,
    release: Mutex<mpsc::Receiver<()>>,
}
impl Drop for Observed {
    fn drop(&mut self) {
        let _ = self.entered.send(std::thread::current().id());
        let _ = self
            .release
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5));
    }
}
impl MountedFilesystem for Observed {
    fn list(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        self.graph.list(node, cancel, emit)
    }
    fn parent(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<Option<NodeToken>, ProviderError> {
        self.graph.parent(node, cancel)
    }
    fn resolve_target(
        &self,
        node: &NodeToken,
        cancel: &Cancellation,
    ) -> Result<ProviderTarget, ProviderError> {
        self.graph.resolve_target(node, cancel)
    }
}
#[test]
fn queued_cancelled_rename_capture_retires_native_owner_and_keeps_permit_until_actual_drop() {
    let f = fixture();
    let root = f.session.projection().root.clone();
    let (entered, received) = mpsc::channel();
    let (release, released) = mpsc::channel();
    let mount = Arc::new(Observed {
        graph: f.graph.clone(),
        _file: std::fs::File::create(f.graph.root_path.join("native-pin")).unwrap(),
        entered,
        release: Mutex::new(released),
    });
    let mut session = Session::new(
        root,
        OpenedLocation {
            mount,
            root: token(0),
        },
        Arc::new(ProviderRegistry::default()),
        WorkerBudget::new(Arc::new(Semaphore::new(2))),
        ProjectionOptions::default(),
        &Cancellation::default(),
        crate::ReportArena::testing(),
    )
    .unwrap();
    let row = session.projection().rows[0].id;
    let request = request(&mut session, row);
    let capacity = Arc::new(Semaphore::new(1));
    let retirement = Retirement::testing();
    let source = session
        .capture_rename(&request, &capacity, &retirement, &Cancellation::default())
        .unwrap();
    let event = midnight_core::TreeEvent::RenameCaptured {
        stamp: source.stamp(),
        request: source.request(),
        result: Ok(source),
    };
    request.cancel.cancel();
    drop(session);
    drop(retirement); // No Tokio runtime or domain owner is needed for final disposal.
    drop(event);
    let thread = received
        .recv_timeout(std::time::Duration::from_secs(2))
        .unwrap();
    let held = capacity.available_permits();
    release.send(()).unwrap();
    assert_ne!(thread, std::thread::current().id());
    assert_eq!(
        held, 0,
        "native destructor still owns admission while blocked"
    );
    wait_released(&capacity);
}
