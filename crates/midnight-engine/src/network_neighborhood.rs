//! Cancellable discovery on bounded workers, separate from native file operations.
use crate::{
    Outbox, Snapshots,
    retirement::{RetirementLease, WorkerOwned},
    task_lifetime::TaskLifetime,
};
use midnight_core::*;
use std::{
    sync::{Arc, OnceLock},
    time::{Duration, Instant},
};
use tokio::{
    sync::{Notify, Semaphore},
    task::JoinSet,
};

mod publication;
#[cfg(test)]
mod tests;
use publication::Publication;

// Exactly one startup discovery, with no replacement/retry. All requests share
// this one native instance; their existing two-call semaphore remains separate.
pub(crate) const RETIREMENT_CAPACITY: usize = 1;
pub(crate) struct Retirement {
    pub(crate) provider: RetirementLease,
}
#[cfg(test)]
impl Retirement {
    fn testing() -> Self {
        let domain =
            crate::retirement::RetirementDomain::try_start(RETIREMENT_CAPACITY, 0).unwrap();
        Self {
            provider: domain.try_reserve().unwrap(),
        }
    }
}

#[derive(Clone)]
struct Provider(Arc<WorkerOwned<Arc<dyn NetworkNeighborhood>>>);
impl Provider {
    fn new(lease: RetirementLease, provider: Arc<dyn NetworkNeighborhood>) -> Self {
        Self(Arc::new(lease.own(provider)))
    }
    fn get(&self) -> &dyn NetworkNeighborhood {
        let provider: &Arc<dyn NetworkNeighborhood> = &self.0;
        provider.as_ref()
    }
}

pub(crate) struct Controller {
    locale: Option<Arc<dyn LocaleService>>,
    provider: Arc<OnceLock<Result<Provider, NetworkFailure>>>,
    ready: Arc<Notify>,
    budget: Arc<Semaphore>,
    cancellations: [Cancellation; 2],
    high_water: [Option<NavigationId>; 2],
    snapshots: Snapshots,
    outbox: Outbox,
    scope: TaskLifetime,
}

impl Controller {
    pub(crate) fn new(
        retirement: Retirement,
        scope: TaskLifetime,
        locale: Option<Arc<dyn LocaleService>>,
        outbox: Outbox,
        snapshots: Snapshots,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        Self::with_discovery(
            retirement, scope, locale, outbox, snapshots, tasks, discover,
        )
    }
    fn with_discovery(
        retirement: Retirement,
        scope: TaskLifetime,
        locale: Option<Arc<dyn LocaleService>>,
        outbox: Outbox,
        snapshots: Snapshots,
        tasks: &mut JoinSet<()>,
        load: impl FnOnce() -> Result<Arc<dyn NetworkNeighborhood>, NetworkFailure> + Send + 'static,
    ) -> Self {
        let provider = Arc::new(OnceLock::new());
        let ready = Arc::new(Notify::new());
        let result = Self {
            locale,
            provider: provider.clone(),
            ready: ready.clone(),
            budget: Arc::new(Semaphore::new(2)),
            cancellations: Default::default(),
            high_water: [None; 2],
            snapshots,
            outbox: outbox.clone(),
            scope: scope.clone(),
        };
        tasks.spawn(scope.future(async move {
            let loaded = spawn_discovery(retirement, load)
                .await
                .unwrap_or_else(|error| Err(failure(error.to_string())));
            let available = loaded.is_ok();
            let _ = provider.set(loaded);
            ready.notify_waiters();
            outbox.send(Event::NetworkAvailability(available)).await;
        }));
        result
    }
    pub(crate) fn cancel(&mut self, pane: PaneId) {
        self.cancellations[pane.index()].cancel();
    }
    pub(crate) fn navigate(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        action: NetworkAction,
        sort: SortSpec,
        tasks: &mut JoinSet<()>,
    ) {
        // A failed request rolls back visible authority, never replay protection.
        // Reject before canceling the current owner or changing snapshot state.
        if self.high_water[pane.index()].is_some_and(|last| navigation <= last) {
            return;
        }
        self.high_water[pane.index()] = Some(navigation);
        self.cancel(pane);
        let cancel = Cancellation::default();
        self.cancellations[pane.index()] = cancel.clone();
        let previous = {
            let mut states = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
            let state = &mut states[pane.index()];
            // Restore the last published view, never an intervening canceled request.
            let previous = state.network.as_ref().map(|(navigation, _)| *navigation);
            state.network_navigation = Some(navigation);
            state.pending = None;
            previous
        };
        let provider = self.provider.clone();
        let ready = self.ready.clone();
        let publication = Publication::new(
            self.outbox.clone(),
            self.snapshots.clone(),
            pane,
            navigation,
            previous,
            cancel.clone(),
        );
        let budget = self.budget.clone();
        let locale = self.locale.clone();
        let scope = self.scope.clone();
        tasks.spawn(self.scope.future(async move {
            loop {
                let notified = ready.notified();
                if provider.get().is_some() {
                    break;
                }
                notified.await;
            }
            if cancel.is_cancelled() {
                return;
            }
            let result = match (
                provider.get().expect("discovery completed"),
                budget.try_acquire_owned(),
            ) {
                (Ok(provider), Ok(permit)) => {
                    let provider = provider.clone();
                    let worker_cancel = cancel.clone();
                    let worker_publication = publication.clone();
                    tokio::task::spawn_blocking(scope.blocking(move || {
                        // Cancellation of the async waiter never releases a running OS call's permit.
                        let _permit = permit;
                        run(
                            provider.get(),
                            action,
                            sort,
                            locale.as_deref(),
                            &worker_cancel,
                            &mut |update| worker_publication.blocking_send(update),
                        )
                    }))
                    .await
                    .unwrap_or_else(|error| Err(failure(error.to_string())))
                }
                (Err(error), _) => Err(error.clone()),
                (_, Err(_)) => Err(NetworkFailure::new(
                    NetworkFailureKind::Busy,
                    "Network discovery is busy. Retry after the current request finishes.",
                )),
            };
            if let Err(error) = result {
                publication.send_failure(error).await;
            }
        }));
    }
}
impl Drop for Controller {
    fn drop(&mut self) {
        for cancel in &self.cancellations {
            cancel.cancel();
        }
    }
}

fn spawn_discovery(
    retirement: Retirement,
    load: impl FnOnce() -> Result<Arc<dyn NetworkNeighborhood>, NetworkFailure> + Send + 'static,
) -> tokio::task::JoinHandle<Result<Provider, NetworkFailure>> {
    tokio::task::spawn_blocking(move || {
        // Creation and failed-open cleanup run under the already reserved slot.
        // No raw native owner may escape into a completed, unconsumed task result.
        let provider = load()?;
        Ok(Provider::new(retirement.provider, provider))
    })
}

fn discover() -> Result<Arc<dyn NetworkNeighborhood>, NetworkFailure> {
    let executable = midnight_plugin_host::installed_executable().map_err(failure)?;
    let directory = executable
        .parent()
        .ok_or_else(|| failure("Installation directory is missing"))?;
    use midnight_native_filesystem_host::NetworkProvider;
    let catalog = midnight_native_filesystem_host::Catalog::discover(directory)
        .map_err(NetworkProvider::classify_failure)?;
    let package = catalog
        .network_package()
        .map_err(NetworkProvider::classify_failure)?;
    Ok(Arc::new(
        NetworkProvider::open(package).map_err(NetworkProvider::classify_failure)?,
    ))
}

fn run(
    provider: &dyn NetworkNeighborhood,
    action: NetworkAction,
    sort: SortSpec,
    locale: Option<&dyn LocaleService>,
    cancel: &Cancellation,
    emit: &mut dyn FnMut(NetworkUpdate) -> bool,
) -> Result<(), NetworkFailure> {
    check_cancel(cancel)?;
    let address = match action {
        NetworkAction::OpenRoot => {
            let root = provider.root(cancel)?;
            match provider.activate(&root, cancel)? {
                NetworkDestination::Browse(address) => address,
                NetworkDestination::Native(_) => {
                    return Err(failure("Network root cannot redirect to a native path"));
                }
            }
        }
        NetworkAction::Parse(text) => {
            let address = provider.parse(&text, cancel)?;
            return run(
                provider,
                NetworkAction::Activate(address),
                sort,
                locale,
                cancel,
                emit,
            );
        }
        NetworkAction::Activate(address) => match provider.activate(&address, cancel)? {
            NetworkDestination::Browse(address) => address,
            NetworkDestination::Native(path) => {
                check_cancel(cancel)?;
                emit(NetworkUpdate::Native(path));
                return Ok(());
            }
        },
        NetworkAction::Parent(address) => provider
            .parent(&address, cancel)?
            .ok_or_else(|| failure("Network root has no parent"))?,
        NetworkAction::Refresh(address) => {
            provider.refresh(&address, cancel)?;
            address
        }
        NetworkAction::Browse(address) => address,
    };
    check_cancel(cancel)?;
    let parent = provider.parent(&address, cancel)?;
    check_cancel(cancel)?;
    let mut entries = Vec::new();
    let mut sent = 0;
    let mut last = Instant::now();
    let publish = |entries: &[Entry], done, error, emit: &mut dyn FnMut(NetworkUpdate) -> bool| {
        let mut rows = entries.to_vec();
        sort_entries(&mut rows, sort);
        let index = Arc::new(DirectoryIndex::from_entries(&rows));
        emit(NetworkUpdate::Listing {
            listing: NetworkListing {
                address: address.clone(),
                parent: parent.clone(),
                entries: rows.into(),
                index,
            },
            done,
            error,
        })
    };
    let mut ordering_error = None;
    let result = provider.list(&address, cancel, &mut |node| {
        if cancel.is_cancelled() {
            return false;
        }
        let mut entry = Entry::from_network(EntryId(entries.len() as u64 + 1), node);
        if let Some(locale) = locale
            && let Err(error) = prepare_sort_keys(std::slice::from_mut(&mut entry), locale, cancel)
        {
            ordering_error = Some(failure(error));
            return false;
        }
        entries.push(entry);
        if entries.len() == 1
            || entries.len() - sent >= 128
            || last.elapsed() >= Duration::from_millis(100)
        {
            sent = entries.len();
            last = Instant::now();
            return publish(&entries, false, None, emit);
        }
        true
    });
    check_cancel(cancel)?;
    // An error before the first row preserves the old location; partial results
    // already published remain visible with their terminal error.
    let result = result.and_then(|()| ordering_error.map_or(Ok(()), Err));
    if entries.is_empty() {
        result?;
        publish(&entries, true, None, emit);
    } else {
        publish(&entries, true, result.err(), emit);
    }
    Ok(())
}

fn failure(message: impl std::fmt::Display) -> NetworkFailure {
    NetworkFailure::new(NetworkFailureKind::Io, message.to_string())
}

fn check_cancel(cancel: &Cancellation) -> Result<(), NetworkFailure> {
    if cancel.is_cancelled() {
        Err(NetworkFailure::new(
            NetworkFailureKind::Cancelled,
            "Network request canceled",
        ))
    } else {
        Ok(())
    }
}
