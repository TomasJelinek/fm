use super::*;
use crate::adapter::Target;
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
fn expired_cache_releases_admission_before_replacement_is_allocated() {
    let budget = Arc::new(Budget::default());
    let filesystem = NetworkFilesystem::<Fixture> {
        state: Arc::new(Mutex::new(State::default())),
        budget: budget.clone(),
    };
    let old = Directory {
        epoch: AtomicU64::new(0),
        cache: Mutex::new(Some(Cached {
            snapshot: Arc::new(Snapshot {
                records: Vec::new(),
                _charge: budget.reserve(MAX_BYTES - 512, 0, 0).unwrap(),
            }),
            created: Instant::now() - Duration::from_secs(31),
            epoch: 0,
        })),
        _charge: budget.reserve(512, 0, 0).unwrap(),
    };
    lock(&filesystem.state).directories.insert(
        vec![Target {
            kind: NodeKind::Root,
            key: b"old".to_vec(),
        }],
        Arc::new(old),
    );
    let location = Location {
        trail: vec![Fixture::root()],
    };
    let node = NodeRecord {
        id: 1,
        key: identity(&location),
        location,
        published: AtomicBool::new(true),
        _charge: budget.reserve(0, 0, 0).unwrap(),
    };
    assert!(filesystem.directory(&node).is_ok());
}

#[test]
fn retained_directory_identity_arrays_fit_inside_the_directory_charge() {
    let budget = Arc::new(Budget::default());
    let filesystem = NetworkFilesystem::<Fixture> {
        state: Arc::new(Mutex::new(State::default())),
        budget: budget.clone(),
    };
    let mut trail = vec![Fixture::root()];
    for id in 1..64 {
        trail.push(Record {
            target: Target {
                kind: NodeKind::Container,
                key: vec![id],
            },
            display: String::new(),
        });
    }
    let location = Location { trail };
    let node = NodeRecord {
        id: 1,
        key: identity(&location),
        location,
        published: AtomicBool::new(true),
        _charge: budget.reserve(0, 0, 0).unwrap(),
    };
    let directory = filesystem.directory(&node).unwrap();
    let required = {
        let state = lock(&filesystem.state);
        let key = state.directories.keys().next().unwrap();
        key.capacity() * size_of::<Target>()
            + key
                .iter()
                .map(|target| target.key.capacity())
                .sum::<usize>()
    };
    assert!(budget.reserve(MAX_BYTES - required + 1, 0, 0).is_err());
    drop(directory);
    drop(filesystem);
    assert!(budget.reserve(MAX_BYTES, 0, 0).is_ok());
}
