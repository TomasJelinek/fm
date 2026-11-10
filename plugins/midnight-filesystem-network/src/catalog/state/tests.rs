use super::*;

#[test]
fn cache_expires_at_thirty_seconds_and_never_crosses_epoch() {
    let budget = Arc::new(Budget::default());
    let created = Instant::now();
    let cache = Cached {
        snapshot: Arc::new(Snapshot {
            records: Vec::new(),
            _charge: budget.reserve(0, 0, 0).unwrap(),
        }),
        created,
        epoch: 7,
    };
    assert!(cache.fresh(created + Duration::from_secs(29), 7));
    assert!(!cache.fresh(created + Duration::from_secs(30), 7));
    assert!(!cache.fresh(created, 8));
}

#[test]
fn byte_node_and_mount_admission_roll_back_and_release_exactly_once() {
    let budget = Arc::new(Budget::default());
    assert!(budget.reserve(1, 4097, 0).is_err());
    assert!(budget.reserve(1, 1, 65).is_err());
    assert_eq!(budget.bytes.load(Ordering::Acquire), 0);
    assert_eq!(budget.nodes.load(Ordering::Acquire), 0);
    let mut charge = budget.reserve(MAX_BYTES, 4096, 64).unwrap();
    assert!(budget.reserve(1, 0, 0).is_err());
    assert!(budget.reserve(0, 1, 0).is_err());
    assert!(budget.reserve(0, 0, 1).is_err());
    assert!(charge.grow(1).is_err());
    drop(charge);
    assert_eq!(budget.bytes.load(Ordering::Acquire), 0);
    assert_eq!(budget.nodes.load(Ordering::Acquire), 0);
    assert_eq!(budget.mounts.load(Ordering::Acquire), 0);
    let mut charge = budget.reserve(1, 1, 1).unwrap();
    charge.grow(100).unwrap();
    drop(charge);
    assert_eq!(budget.bytes.load(Ordering::Acquire), 0);
}

struct Fixture;
impl Adapter for Fixture {
    type Cursor = ();
    fn root() -> Record {
        Record {
            target: Target {
                kind: NodeKind::Root,
                key: b"root".to_vec(),
            },
            display: "Network".into(),
        }
    }
    fn validate(_: &Target) -> Result<(), Failure> {
        Ok(())
    }
    fn manual(_: &str) -> Result<Record, Failure> {
        unreachable!()
    }
    fn begin(_: &Target, _: &Context<'_>) -> Result<(), Failure> {
        unreachable!()
    }
    fn next(_: &mut (), _: &Context<'_>) -> Result<Option<Record>, Failure> {
        unreachable!()
    }
    fn activate(_: &Target, _: &Context<'_>) -> Result<ActivationTarget, Failure> {
        unreachable!()
    }
}

#[test]
fn retained_deep_trail_arrays_fit_inside_the_node_charge() {
    let budget = Arc::new(Budget::default());
    let filesystem = NetworkFilesystem::<Fixture> {
        state: Arc::new(Mutex::new(State::default())),
        budget: budget.clone(),
    };
    let mut trail = Vec::with_capacity(128);
    trail.push(Fixture::root());
    for id in 1..64 {
        trail.push(Record {
            target: Target {
                kind: NodeKind::Container,
                key: vec![id],
            },
            display: String::new(),
        });
    }
    let reply = filesystem.resolve(Location { trail }).unwrap();
    let required = {
        let state = lock(&filesystem.state);
        let mount = state.mounts.values().next().unwrap();
        let nodes = lock(&mount.nodes);
        let node = nodes.by_id.values().next().unwrap();
        let map_key = nodes.by_key.keys().next().unwrap();
        let mut required = node.location.trail.capacity() * size_of::<Record>();
        for record in &node.location.trail {
            required += record.target.key.capacity() + record.display.capacity();
        }
        for identity in [&node.key, map_key] {
            required += identity.capacity() * size_of::<Target>();
            required += identity
                .iter()
                .map(|target| target.key.capacity())
                .sum::<usize>();
        }
        required
    };
    // Independent capacity measurement excludes node/map/Arc headers, so even
    // this lower bound must be covered by the aggregate retained charge.
    assert!(budget.bytes.load(Ordering::Acquire) >= required);
    drop(reply); // unpublished resolve rolls back both node and mount admission
    assert_eq!(budget.bytes.load(Ordering::Acquire), 0);
}

#[test]
fn pending_records_admit_spare_capacity_and_release_each_cursor_charge() {
    let budget = Arc::new(Budget::default());
    let record = || {
        let mut record = Fixture::root();
        record.target.key.reserve(1024 * 1024);
        record.display.reserve(1024 * 1024);
        record
    };
    let occupied = budget.reserve(MAX_BYTES - 1024, 0, 0).unwrap();
    assert!(matches!(
        Pending::new(record(), &budget),
        Err(Failure {
            code: ErrorCode::ResourceLimit,
            ..
        })
    ));
    assert_eq!(budget.bytes.load(Ordering::Acquire), MAX_BYTES - 1024);
    drop(occupied);
    let first = Pending::new(record(), &budget).unwrap();
    let second = Pending::new(record(), &budget).unwrap();
    assert!(budget.bytes.load(Ordering::Acquire) >= 4 * 1024 * 1024);
    drop(first);
    assert!(budget.bytes.load(Ordering::Acquire) >= 2 * 1024 * 1024);
    drop(second);
    assert_eq!(budget.bytes.load(Ordering::Acquire), 0);
}

#[test]
fn rolled_back_page_releases_the_node_map_capacities_with_its_charges() {
    let budget = Arc::new(Budget::default());
    let filesystem = NetworkFilesystem::<Fixture> {
        state: Arc::new(Mutex::new(State::default())),
        budget: budget.clone(),
    };
    let mount = Arc::new(Mount {
        closed: AtomicBool::new(false),
        published: AtomicBool::new(true),
        nodes: Mutex::default(),
        _charge: budget.reserve(512, 0, 1).unwrap(),
    });
    let mut transaction = NodeTransaction::new(mount.clone());
    for id in 0..128_u16 {
        let location = Location {
            trail: vec![
                Fixture::root(),
                Record {
                    target: Target {
                        kind: NodeKind::Server,
                        key: id.to_le_bytes().to_vec(),
                    },
                    display: "server".into(),
                },
            ],
        };
        filesystem.bind(&mount, location, &mut transaction).unwrap();
    }
    assert!(lock(&mount.nodes).by_id.capacity() >= 128);
    drop(transaction);
    assert_eq!(budget.bytes.load(Ordering::Acquire), 512);
    let nodes = lock(&mount.nodes);
    assert_eq!(
        nodes.by_id.capacity(),
        0,
        "uncharged node map survived rollback"
    );
    assert_eq!(
        nodes.by_key.capacity(),
        0,
        "uncharged identity map survived rollback"
    );
}
