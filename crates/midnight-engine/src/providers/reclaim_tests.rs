//! Lister-owned reclamation: released references fail closed, kept and resolved
//! ones keep working, and nothing is released while a listing is in flight.
use super::*;
use midnight_core::{
    DirectoryService, Entry, EntryId, FsError, NodeRef, ProviderId, ProviderInstanceId, SpaceInfo,
    StorageService, Volume, VolumeService,
};
use std::{
    io,
    num::NonZeroU32,
    path::{Path, PathBuf},
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, sync_channel},
    },
    time::Duration,
};

const DEADLINE: Duration = Duration::from_secs(5);

fn token(value: u64) -> NodeToken {
    NodeToken::new(&value.to_le_bytes()).unwrap()
}

fn entry(value: u64, kind: EntryKind) -> ProviderEntry {
    ProviderEntry {
        token: token(value),
        raw_name: Arc::from(value.to_le_bytes()),
        display_name: format!("node {value}"),
        kind,
        metadata: EntryMetadata::default(),
    }
}

/// A generic provider listing a mutable table. It keeps every record (like the
/// default `retain_nodes`) but records each requested set, observes the lease
/// re-entrantly from that call and can hold one listing or parent lookup.
#[derive(Default)]
struct Table {
    rows: Mutex<HashMap<u64, Vec<ProviderEntry>>>,
    retained: Mutex<Vec<HashSet<NodeToken>>>,
    observer: Mutex<Option<MountedLocation>>,
    observed: Mutex<Vec<usize>>,
    hold: Mutex<Option<(SyncSender<()>, Receiver<()>)>>,
}

impl Table {
    fn with_root(rows: Vec<ProviderEntry>) -> Arc<Self> {
        let table = Arc::new(Self::default());
        table.rows.lock().unwrap().insert(0, rows);
        table
    }

    /// The next listing or parent lookup waits inside the provider call.
    fn hold_next(&self) -> (Receiver<()>, SyncSender<()>) {
        let (entered, on_entry) = sync_channel(1);
        let (resume, on_resume) = sync_channel(1);
        *self.hold.lock().unwrap() = Some((entered, on_resume));
        (on_entry, resume)
    }

    fn pause(&self) {
        let held = self.hold.lock().unwrap().take();
        if let Some((entered, resume)) = held {
            entered.send(()).unwrap();
            resume.recv_timeout(DEADLINE).unwrap();
        }
    }
}

impl MountedFilesystem for Table {
    fn list(
        &self,
        node: &NodeToken,
        _: &Cancellation,
        emit: &mut dyn FnMut(ProviderEntry) -> bool,
    ) -> Result<(), ProviderError> {
        self.pause();
        let value = u64::from_le_bytes(node.as_bytes().try_into().unwrap());
        let rows = self.rows.lock().unwrap().get(&value).cloned();
        for row in rows.into_iter().flatten() {
            if !emit(row) {
                break;
            }
        }
        Ok(())
    }

    fn parent(&self, _: &NodeToken, _: &Cancellation) -> Result<Option<NodeToken>, ProviderError> {
        self.pause();
        Ok(Some(token(0)))
    }

    fn resolve_target(
        &self,
        node: &NodeToken,
        _: &Cancellation,
    ) -> Result<ProviderTarget, ProviderError> {
        let value = u64::from_le_bytes(node.as_bytes().try_into().unwrap());
        Ok(ProviderTarget::Native(PathBuf::from(format!(
            "node {value}"
        ))))
    }

    fn retain_nodes(&self, keep: &HashSet<NodeToken>) -> Result<(), ProviderError> {
        // Would deadlock if the lease still held its issued-table lock.
        if let Some(location) = self.observer.lock().unwrap().as_ref() {
            self.observed
                .lock()
                .unwrap()
                .push(location.issued_count().unwrap());
        }
        self.retained.lock().unwrap().push(keep.clone());
        Ok(())
    }
}

struct Fixture {
    registry: ProviderRegistry,
    address: StoredLocation,
    instance: ProviderInstanceId,
    location: MountedLocation,
}

fn registered(name: &str, mount: Arc<dyn MountedFilesystem>, root: NodeToken) -> Fixture {
    struct Provider(Arc<dyn MountedFilesystem>, NodeToken);
    impl FilesystemProvider for Provider {
        fn resolve(
            &self,
            _: &StoredLocation,
            _: &Cancellation,
        ) -> Result<OpenedLocation, ProviderError> {
            Ok(OpenedLocation {
                mount: self.0.clone(),
                root: self.1.clone(),
            })
        }
    }
    let registry = ProviderRegistry::default();
    let address = StoredLocation::new(
        ProviderId::new(name).unwrap(),
        NonZeroU32::MIN,
        Arc::from([]),
    );
    let instance = registry
        .register(address.provider().clone(), Arc::new(Provider(mount, root)))
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    Fixture {
        registry,
        address,
        instance,
        location,
    }
}

fn fixture(table: &Arc<Table>) -> Fixture {
    registered("test.reclaim", table.clone(), token(0))
}

fn tokens<'a>(nodes: impl IntoIterator<Item = &'a NodeRef>) -> HashSet<NodeToken> {
    nodes.into_iter().map(|node| node.token.clone()).collect()
}

#[test]
fn retain_issued_releases_unreferenced_references_and_keeps_the_root() {
    let table = Table::with_root(vec![
        entry(1, EntryKind::Directory),
        entry(2, EntryKind::Directory),
        entry(3, EntryKind::File),
    ]);
    let fixture = fixture(&table);
    let location = &fixture.location;
    let cancel = Cancellation::default();
    let rows = location.list(location.root(), &cancel).unwrap();
    assert_eq!(location.issued_count(), Ok(4));
    let (kept, released) = (&rows[0].node, &rows[1].node);

    assert_eq!(
        location.retain_issued(&tokens([kept]), &cancel),
        Ok(Some(2))
    );
    assert_eq!(location.issued_count(), Ok(2));
    assert_eq!(
        *table.retained.lock().unwrap(),
        [tokens([kept, location.root()])],
        "the provider keeps the lister's references and the root"
    );
    assert_eq!(
        location.validate_reference(released),
        Err(ProviderError::StaleReference)
    );
    assert_eq!(
        location.list_folders(released, &cancel, 8).unwrap_err(),
        ProviderError::StaleReference
    );
    assert_eq!(
        location.has_folders(released, &cancel),
        Err(ProviderError::StaleReference)
    );
    assert_eq!(
        location.parent(released, &cancel).unwrap_err(),
        ProviderError::StaleReference
    );
    assert!(matches!(
        location.resolve_target(released, &cancel),
        Err(ProviderError::StaleReference)
    ));
    location.validate_reference(kept).unwrap();
    location.validate_reference(location.root()).unwrap();

    // This provider kept its records, so listing again issues the same tokens.
    let again = location.list(location.root(), &cancel).unwrap();
    assert_eq!(again[1].node, *released);
    location.validate_reference(released).unwrap();

    // A kept reference still remembers its kind.
    location.retain_issued(&tokens([kept]), &cancel).unwrap();
    table
        .rows
        .lock()
        .unwrap()
        .insert(0, vec![entry(1, EntryKind::File)]);
    assert_eq!(
        location.list(location.root(), &cancel).unwrap_err(),
        ProviderError::InvalidResponse("issued node changed kind")
    );
}

#[test]
fn retain_issued_calls_the_provider_outside_the_issued_table_lock_with_the_root_included() {
    let table = Table::with_root(vec![entry(1, EntryKind::Directory)]);
    let fixture = fixture(&table);
    let location = &fixture.location;
    location
        .list(location.root(), &Cancellation::default())
        .unwrap();
    *table.observer.lock().unwrap() = Some(location.clone());
    let (done, finished) = sync_channel(1);
    let worker = location.clone();
    std::thread::spawn(move || {
        done.send(worker.retain_issued(&HashSet::new(), &Cancellation::default()))
            .unwrap();
    });
    assert_eq!(finished.recv_timeout(DEADLINE).unwrap(), Ok(Some(1)));
    assert_eq!(*table.observed.lock().unwrap(), [1]);
    assert_eq!(*table.retained.lock().unwrap(), [tokens([location.root()])]);
    // Break the provider -> location -> lease -> provider cycle.
    table.observer.lock().unwrap().take();
}

#[test]
fn reclamation_skips_while_a_listing_probe_or_parent_lookup_is_in_flight() {
    let table = Table::with_root(vec![entry(1, EntryKind::Directory)]);
    table.rows.lock().unwrap().insert(1, Vec::new());
    let fixture = fixture(&table);
    let location = &fixture.location;
    let cancel = Cancellation::default();
    let folder = location.list(location.root(), &cancel).unwrap()[0]
        .node
        .clone();
    type Call = fn(&MountedLocation, &NodeRef) -> Result<(), ProviderError>;
    let calls: [(&str, Call); 4] = [
        ("list", |location, _| {
            location
                .list(location.root(), &Cancellation::default())
                .map(drop)
        }),
        ("folder listing", |location, node| {
            location
                .list_folders(node, &Cancellation::default(), 8)
                .map(drop)
        }),
        ("presence probe", |location, node| {
            location
                .has_folders(node, &Cancellation::default())
                .map(drop)
        }),
        ("parent lookup", |location, node| {
            location.parent(node, &Cancellation::default()).map(drop)
        }),
    ];
    for (name, call) in calls {
        let (entered, resume) = table.hold_next();
        let (worker, node) = (location.clone(), folder.clone());
        let running = std::thread::spawn(move || call(&worker, &node));
        entered.recv_timeout(DEADLINE).unwrap();
        let before = table.retained.lock().unwrap().len();
        assert_eq!(
            location.retain_issued(&HashSet::new(), &cancel),
            Ok(None),
            "{name} in flight must defer reclamation"
        );
        assert_eq!(table.retained.lock().unwrap().len(), before);
        location.validate_reference(&folder).unwrap();
        resume.send(()).unwrap();
        running.join().unwrap().unwrap();
        location.validate_reference(&folder).unwrap();
    }
    assert_eq!(
        location.retain_issued(&tokens([&folder]), &cancel),
        Ok(Some(2))
    );
}

#[test]
fn resolved_target_survives_reference_release_and_fails_only_on_revocation() {
    let table = Table::with_root(vec![entry(1, EntryKind::File)]);
    let fixture = fixture(&table);
    let location = &fixture.location;
    let cancel = Cancellation::default();
    let file = location.list(location.root(), &cancel).unwrap()[0]
        .node
        .clone();
    let target = location.resolve_target(&file, &cancel).unwrap();
    location.retain_issued(&HashSet::new(), &cancel).unwrap();
    assert_eq!(
        location.validate_reference(&file),
        Err(ProviderError::StaleReference)
    );
    assert!(
        matches!(
            location.resolve_target(&file, &cancel),
            Err(ProviderError::StaleReference)
        ),
        "resolution still requires an issued reference"
    );
    assert!(matches!(
        target.value(),
        Ok(ProviderTarget::Native(path)) if path == Path::new("node 1")
    ));
    assert!(
        fixture
            .registry
            .unregister(fixture.address.provider(), fixture.instance)
            .unwrap()
    );
    assert!(matches!(target.value(), Err(ProviderError::StaleReference)));
}

#[test]
fn lease_returns_spare_issued_capacity_after_a_burst() {
    let table = Table::with_root(
        (1..=10_000)
            .map(|value| entry(value, EntryKind::File))
            .collect(),
    );
    let fixture = fixture(&table);
    let location = &fixture.location;
    let cancel = Cancellation::default();
    location.list(location.root(), &cancel).unwrap();
    assert!(location.issued_capacity() >= 10_001);
    assert_eq!(
        location.retain_issued(&HashSet::new(), &cancel),
        Ok(Some(1))
    );
    assert!(location.issued_capacity() < 64);
}

/// Every enumeration of the root returns 1,000 names it never returned before.
#[derive(Default)]
struct Churn {
    listings: AtomicU64,
}

impl DirectoryService for Churn {
    fn read_directory(
        &self,
        path: &Path,
        _: &Cancellation,
        emit: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        let listing = self.listings.fetch_add(1, Ordering::Relaxed);
        for index in 0..1_000 {
            let name = format!("temporary {listing}-{index}.tmp");
            if !emit(Entry::new(EntryId(index), path.join(name), EntryKind::File)) {
                break;
            }
        }
        Ok(())
    }
    fn listing_metadata_complete(&self, _: &Entry) -> bool {
        true
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        Ok(EntryMetadata::default())
    }
}
impl VolumeService for Churn {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}
impl StorageService for Churn {
    fn space_info(&self, path: &Path) -> Result<SpaceInfo, FsError> {
        Err(FsError::from_io(
            "test",
            path,
            io::ErrorKind::Unsupported.into(),
        ))
    }
}

/// A registry mount of the production native adapter over a churning directory.
fn churning_native_mount() -> (ProviderRegistry, MountedLocation) {
    let registry = ProviderRegistry::default();
    let root = std::env::current_dir()
        .unwrap()
        .join("_build")
        .join("provider-reclaim-test");
    let address = native_location(&root).unwrap();
    registry
        .register(
            address.provider().clone(),
            Arc::new(NativeProvider::new(Arc::new(Churn::default()))),
        )
        .unwrap();
    let location = registry
        .resolve(&address, &Cancellation::default())
        .unwrap();
    (registry, location)
}

/// List the churning root `listings` times, retaining only the latest rows.
fn churn_retaining_rows(location: &MountedLocation, listings: usize) {
    let cancel = Cancellation::default();
    let mut previous = HashSet::new();
    for _ in 0..listings {
        let rows = location.list(location.root(), &cancel).unwrap();
        let keep = tokens(rows.iter().map(|row| &row.node));
        assert!(keep.is_disjoint(&previous), "the listing reused a token");
        assert_eq!(location.retain_issued(&keep, &cancel), Ok(Some(1_001)));
        assert_eq!(location.provider_node_count(), Some(1_001));
        previous = keep;
    }
}

#[test]
fn churning_root_listing_keeps_both_tables_at_the_retained_rows() {
    let (_registry, location) = churning_native_mount();
    churn_retaining_rows(&location, 4);
}

/// 101,000 names through one mount whose native table and issued ledger each
/// admit 100,000 (without reclamation the 100th listing fails), then the cost of
/// one reclamation at the table sizes a tree can reach.
/// Run: cargo test -p midnight-engine --lib reclamation_cost_harness -- --ignored --nocapture
#[test]
#[ignore = "timing harness"]
fn reclamation_cost_harness() {
    let (_registry, location) = churning_native_mount();
    let started = std::time::Instant::now();
    churn_retaining_rows(&location, 101);
    println!("101,000 churned names: {:?}", started.elapsed());
    // The native table and the issued ledger each admit 100,000, root included.
    for (total, live) in [(25_000, 10_000), (99_000, 10_000), (99_000, 1_000)] {
        let (_registry, location) = churning_native_mount();
        let cancel = Cancellation::default();
        let mut keep = HashSet::new();
        for _ in 0..total / 1_000 {
            let rows = location.list(location.root(), &cancel).unwrap();
            if keep.len() < live {
                keep.extend(rows.iter().map(|row| row.node.token.clone()));
            }
        }
        let started = std::time::Instant::now();
        let kept = location.retain_issued(&keep, &cancel).unwrap();
        println!(
            "reclaim {total} issued to {kept:?} kept: {:?} (native {:?})",
            started.elapsed(),
            location.provider_node_count()
        );
    }
}
