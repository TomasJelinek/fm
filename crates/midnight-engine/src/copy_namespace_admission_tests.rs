//! Admission is an atomic check-and-claim decided at once from committed facts: a busy
//! peer cannot delay it, and an overlap with a granted job waits per item.
use super::*;

/// One turn of `job` that announces `pin`. Returns whether the change was accepted.
fn announce(job: &Job, pin: &Arc<dyn TransferNamespacePin>) -> bool {
    let mut change = job
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let accepted = change
        .reserve_private_child(Some(pin.clone()), &Cancellation::default())
        .unwrap();
    change.finish().unwrap();
    accepted
}

/// A transfer job whose endpoints are (source, destination) pairs, as copies plan them.
fn transfer(
    registry: &Registry,
    sequence: u64,
    pairs: Vec<(Arc<dyn TransferNamespacePin>, Arc<dyn TransferNamespacePin>)>,
) -> Job {
    let endpoints = pairs
        .into_iter()
        .flat_map(|(source, destination)| [(Read, Some(source)), (Write, Some(destination))])
        .collect();
    planned(registry, sequence, endpoints)
}

fn planned(
    registry: &Registry,
    sequence: u64,
    endpoints: Vec<(
        TransferNamespaceAccess,
        Option<Arc<dyn TransferNamespacePin>>,
    )>,
) -> Job {
    let (claim, owners) = registry.new_job(sequence).unwrap();
    let io = Arc::new(
        registry
            .inner
            .io
            .acquire(midnight_work_budget::Admission::Shared, || false)
            .unwrap(),
    );
    claim
        .plan_transfer(endpoints, io.clone(), &Cancellation::default())
        .unwrap();
    Job {
        claim: Some(claim),
        owners: Some(owners),
        io: Some(io),
    }
}

/// A source whose geometry is unknown (a followed link, a provider without native
/// geometry) is only read, so it stays unclaimed: the job is not exclusive and blocks
/// nobody. A destination of unknown geometry still excludes every other job.
#[test]
fn unknown_read_root_is_unprotected_but_an_unknown_written_root_excludes_all() {
    let registry = registry();
    let (_, output) = pin(vec![entry(parent(2, &[2, 0]), b"photos", None, true)]);
    let reader = planned(&registry, 1, vec![(Read, None), (Write, Some(output))]);
    assert!(reader.grant());
    assert_eq!(registry.state(1), FactsState::Stable);
    let (_, source) = pin(vec![entry(parent(3, &[3, 0]), b"small", Some(31), true)]);
    let (_, sibling) = pin(vec![entry(parent(4, &[4, 0]), b"small", None, true)]);
    let other = transfer(&registry, 2, vec![(source, sibling)]);
    assert!(other.grant(), "an unknown read scope holds nothing");
    let (_, known) = pin(vec![entry(parent(5, &[5, 0]), b"else", Some(51), true)]);
    let writer = planned(&registry, 3, vec![(Read, Some(known)), (Write, None)]);
    let Grant::Conflict(_) = writer.claim().admit(&Cancellation::default()).unwrap() else {
        panic!("an unknown written footprint must exclude the granted jobs");
    };
}

fn deferred(registry: &Registry, sequence: u64) -> Vec<usize> {
    registry.inner.state.lock().unwrap().records[&sequence]
        .deferred
        .clone()
}

/// One turn of `job` that announces its selected root, as a native publication inside
/// that root does, and a fresh private binding named `key` below it.
fn publish_below(job: &Job, root: &Arc<dyn TransferNamespacePin>, key: &[u8]) {
    let mut change = job
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        change
            .reserve_private_child(Some(root.clone()), &Cancellation::default())
            .unwrap()
    );
    let (_, fresh) = pin(vec![entry(parent(10, &[10, 1, 0]), key, None, false)]);
    assert!(
        change
            .reserve_private_child(Some(fresh), &Cancellation::default())
            .unwrap()
    );
    change.finish().unwrap();
}

/// A granted job that has published thousands of files and keeps changing its namespace
/// turn after turn cannot keep a disjoint candidate waiting: the first attempt grants.
#[test]
fn back_to_back_peer_turns_cannot_starve_a_disjoint_admission() {
    const PUBLISHED: usize = 3_000;
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let (_, root) = pin(vec![entry(shared.clone(), b"busy", Some(10), true)]);
    let busy = Job::new(&registry, 1, vec![(Write, Some(root.clone()))]);
    assert!(busy.grant());
    let mut change = busy
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert!(
        change
            .reserve_private_child(Some(root.clone()), &Cancellation::default())
            .unwrap()
    );
    let mut published = Vec::with_capacity(PUBLISHED);
    for index in 0..PUBLISHED {
        let (control, file) = pin(vec![entry(
            parent(10, &[10, 1, 0]),
            format!("{index:05}.bin").as_bytes(),
            Some(100_000 + index as u64),
            false,
        )]);
        assert!(
            change
                .reserve_private_child(Some(file), &Cancellation::default())
                .unwrap()
        );
        published.push(control);
    }
    change.finish().unwrap();
    assert_eq!(busy.claim().alias_count(), PUBLISHED);
    let probed = |published: &[Arc<Control>]| {
        published
            .iter()
            .map(|control| control.calls.load(Ordering::SeqCst))
            .sum::<usize>()
    };
    let before = probed(&published);
    let candidate = one(&registry, 2, Write, entry(shared, b"other", None, false));
    let stop = AtomicBool::new(false);
    let turns = AtomicUsize::new(0);
    let (attempts, granted) = thread::scope(|threads| {
        threads.spawn(|| {
            while !stop.load(Ordering::SeqCst) {
                let turn = turns.fetch_add(1, Ordering::SeqCst);
                publish_below(&busy, &root, format!("next-{turn}").as_bytes());
            }
        });
        while turns.load(Ordering::SeqCst) < 10 {
            thread::yield_now();
        }
        let mut attempts = 0;
        let granted = loop {
            attempts += 1;
            if candidate.grant() {
                break true;
            }
            if attempts == 50 {
                break false;
            }
            thread::sleep(Duration::from_millis(2));
        };
        stop.store(true, Ordering::SeqCst);
        (attempts, granted)
    });
    assert!(
        granted,
        "a disjoint candidate was refused {attempts} times while the peer kept changing"
    );
    assert_eq!(attempts, 1, "admission does not wait for a quiet peer");
    assert_eq!(
        probed(&published),
        before,
        "admission never probes what a peer has published"
    );
}

/// A grant is decided while a granted peer is in the middle of a change: the peer's
/// announced bindings are already in its committed facts.
#[test]
fn disjoint_grant_completes_during_a_peer_change_and_finish_restores_stable() {
    let registry = registry();
    let first = one(
        &registry,
        1,
        Write,
        entry(parent(1, &[1, 0]), b"one", None, false),
    );
    let second = one(
        &registry,
        2,
        Write,
        entry(parent(1, &[1, 0]), b"two", None, false),
    );
    assert!(first.grant());
    let transaction = first
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    assert_eq!(registry.state(1), FactsState::InFlight);
    assert!(second.grant());
    transaction.finish().unwrap();
    assert_eq!(registry.state(1), FactsState::Stable);
    assert_eq!(registry.granted_for_test(), [1, 2]);
}

/// Admission probes only the candidate's own pins. A peer pin that blocks or cannot be
/// refreshed right now neither delays the candidate nor is latched by its admission.
#[test]
fn admission_probes_only_the_candidates_own_pins() {
    let registry = registry();
    let (control, native) = pin(vec![entry(parent(1, &[1, 0]), b"one", None, false)]);
    let first = Job::new(&registry, 1, vec![(Write, Some(native))]);
    assert!(first.grant());
    let calls = control.calls.load(Ordering::SeqCst);
    let (_entered, _release) = block(&control);
    let second = one(
        &registry,
        2,
        Write,
        entry(parent(1, &[1, 0]), b"two", None, false),
    );
    assert!(second.grant(), "a blocked peer probe would hang here");
    *control.probe.lock().unwrap() = Err(failure(
        io::ErrorKind::InvalidInput,
        "original namespace leaf binding changed",
    ));
    let third = one(
        &registry,
        3,
        Write,
        entry(parent(9, &[9, 0]), b"unrelated", None, false),
    );
    assert!(third.grant());
    assert_eq!(control.calls.load(Ordering::SeqCst), calls);
    assert_eq!(registry.state(1), FactsState::Stable);
    assert!(first.claim().take_diagnostics().is_empty());
}

/// Writers of an object or name that granted jobs read start at once: only their change
/// of that item waits, until the readers are gone. Readers still coexist.
#[test]
fn writers_of_a_read_object_or_name_start_and_wait_per_item() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let mut first = one(
        &registry,
        1,
        Read,
        entry(shared.clone(), b"one", Some(10), false),
    );
    let mut second = one(
        &registry,
        2,
        Read,
        entry(shared.clone(), b"alias", Some(10), false),
    );
    let (_, hardlink) = pin(vec![entry(shared.clone(), b"third-name", Some(10), false)]);
    let writer = Job::new(&registry, 3, vec![(Write, Some(hardlink.clone()))]);
    let (_, name) = pin(vec![entry(shared, b"one", None, false)]);
    let target = Job::new(&registry, 4, vec![(Write, Some(name.clone()))]);
    assert!(first.grant());
    assert!(second.grant(), "read and read coexist");
    assert!(writer.grant());
    assert!(target.grant());
    assert_eq!(deferred(&registry, 3), [0]);
    assert_eq!(deferred(&registry, 4), [0]);
    assert!(
        !announce(&writer, &hardlink),
        "the read object stays excluded"
    );
    assert!(!announce(&target, &name), "the read name stays excluded");
    first.close();
    second.close();
    assert!(announce(&writer, &hardlink));
    assert!(announce(&target, &name));
    assert!(deferred(&registry, 3).is_empty() && deferred(&registry, 4).is_empty());
}

/// Two copies of the same name into one folder: the later job starts, its overlapping
/// destination is deferred, and its change of it waits until the earlier job is gone.
/// The earlier job's own changes there never wait for the later one.
#[test]
fn same_destination_name_waits_per_item_and_never_blocks_the_earlier_job() {
    let registry = registry();
    let destination = parent(2, &[2, 0]);
    let (_, first_source) = pin(vec![entry(parent(1, &[1, 0]), b"a", Some(11), true)]);
    let (_, first_output) = pin(vec![entry(destination.clone(), b"photos", None, true)]);
    let (_, other_source) = pin(vec![entry(parent(3, &[3, 0]), b"photos", Some(31), true)]);
    let (_, second_output) = pin(vec![entry(destination.clone(), b"photos", None, true)]);
    let (_, extra_source) = pin(vec![entry(parent(3, &[3, 0]), b"notes", Some(32), false)]);
    let (_, extra_output) = pin(vec![entry(destination, b"notes", None, false)]);
    let mut first = transfer(&registry, 1, vec![(first_source, first_output.clone())]);
    let second = transfer(
        &registry,
        2,
        vec![
            (extra_source, extra_output.clone()),
            (other_source, second_output.clone()),
        ],
    );
    assert!(first.grant());
    assert!(second.grant(), "the later job starts at once");
    assert_eq!(
        deferred(&registry, 2),
        [3],
        "only its overlapping item waits"
    );
    assert!(announce(&second, &extra_output), "its other items proceed");
    assert!(!announce(&second, &second_output));
    assert!(
        announce(&first, &first_output),
        "a later reservation never refuses the earlier job"
    );
    first.close();
    assert!(announce(&second, &second_output));
}

/// Deferred items keep their order: a job granted later never takes an item before an
/// earlier job that deferred it, so waits between jobs cannot form a cycle.
#[test]
fn deferred_items_are_claimed_in_grant_order() {
    let registry = registry();
    let destination = parent(2, &[2, 0]);
    let output = |key: &[u8]| pin(vec![entry(destination.clone(), key, None, false)]).1;
    let source =
        |key: &[u8], object| pin(vec![entry(parent(1, &[1, 0]), key, Some(object), false)]).1;
    let held = output(b"x");
    let mut holder = transfer(&registry, 1, vec![(source(b"h", 11), held.clone())]);
    assert!(holder.grant());
    let earlier_output = output(b"x");
    let mut earlier = transfer(
        &registry,
        2,
        vec![(source(b"e", 12), earlier_output.clone())],
    );
    let later_output = output(b"x");
    let later = transfer(&registry, 3, vec![(source(b"l", 13), later_output.clone())]);
    assert!(earlier.grant() && later.grant());
    assert_eq!(deferred(&registry, 2), [1]);
    assert_eq!(deferred(&registry, 3), [1]);
    holder.close();
    assert!(
        !announce(&later, &later_output),
        "the earlier job's reservation comes first"
    );
    assert!(announce(&earlier, &earlier_output));
    assert!(
        !announce(&later, &later_output),
        "now the earlier job claims it"
    );
    earlier.close();
    assert!(announce(&later, &later_output));
}

/// Reading what a granted job changes cannot be interleaved item by item (its partial
/// output may be visible), so such a job waits as a whole and says so.
#[test]
fn reading_what_another_job_writes_waits_as_a_whole() {
    let registry = registry();
    let destination = parent(2, &[2, 0]);
    let (_, source) = pin(vec![entry(parent(1, &[1, 0]), b"a", Some(11), true)]);
    let (_, output) = pin(vec![entry(destination.clone(), b"photos", Some(21), true)]);
    let mut writer = transfer(&registry, 1, vec![(source, output)]);
    assert!(writer.grant());
    let (_, read) = pin(vec![entry(destination, b"photos", Some(21), true)]);
    let (_, elsewhere) = pin(vec![entry(parent(4, &[4, 0]), b"photos", None, true)]);
    let reader = transfer(&registry, 2, vec![(read, elsewhere)]);
    let Grant::Conflict(seen) = reader.claim().admit(&Cancellation::default()).unwrap() else {
        panic!("a reader of another job's output must wait for it");
    };
    let watch = reader.claim().watch();
    let structural = watch.changed(seen, true);
    tokio::pin!(structural);
    assert!(!poll_once(structural.as_mut()));
    writer.close();
    assert!(poll_once(structural.as_mut()));
    assert!(reader.grant());
}

/// Closing waits for the job's own counted probe (here, its final revalidation) and a
/// cleanup turn while closing keeps the record closing. Other jobs' admissions never
/// probe it, so they never hold its closing back.
#[test]
fn closing_waits_for_its_own_probe_and_cleanup_turn_preserves_closing() {
    let registry = registry();
    let (control, native) = pin(vec![entry(parent(1, &[1, 0]), b"one", None, false)]);
    let first = Job::new(&registry, 1, vec![(Write, Some(native))]);
    assert!(first.grant());
    let (entered, release) = block(&control);
    thread::scope(|threads| {
        let claim = first.claim();
        let revalidation = threads.spawn(move || claim.revalidate_before_close());
        entered.recv_timeout(Duration::from_secs(5)).unwrap();
        let second = one(
            &registry,
            2,
            Write,
            entry(parent(1, &[1, 0]), b"two", None, false),
        );
        assert!(second.grant(), "admission does not wait for the probe");
        let (closed, observed) = std::sync::mpsc::channel();
        let closing = threads.spawn(move || {
            claim.begin_closing().unwrap();
            closed.send(()).unwrap();
        });
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while registry.state(1) != FactsState::Closing {
            assert!(std::time::Instant::now() < deadline);
            thread::yield_now();
        }
        assert!(observed.try_recv().is_err(), "the probe is still counted");
        drop(release);
        revalidation.join().unwrap();
        closing.join().unwrap();
        observed.recv_timeout(Duration::from_secs(5)).unwrap();
    });
    first
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap()
        .finish()
        .unwrap();
    assert_eq!(registry.state(1), FactsState::Closing);
}

/// A turn that did not announce every binding it affected (here only a fresh private pin)
/// refreshes every pin of its job when it finishes, so the committed facts that later
/// grants compare (without probing the job) include an object materialized meanwhile.
#[test]
fn unannounced_finish_refreshes_every_pin_and_grants_compare_without_probing() {
    let registry = registry();
    let shared = parent(1, &[1, 0]);
    let (control, native) = pin(vec![entry(shared.clone(), b"selected", None, true)]);
    let job = Job::new(&registry, 1, vec![(Write, Some(native))]);
    assert!(job.grant());
    let before = control.calls.load(Ordering::SeqCst);
    let mut transaction = job
        .claim()
        .changes()
        .begin(&Cancellation::default())
        .unwrap();
    let (_, alias) = pin(vec![entry(shared.clone(), b"temp", None, true)]);
    assert!(
        transaction
            .reserve_private_child(Some(alias), &Cancellation::default())
            .unwrap()
    );
    *control.probe.lock().unwrap() = Ok(TransferNamespaceProbe::Known(
        vec![entry(shared, b"selected", Some(50), true)].into(),
    ));
    transaction.finish().unwrap();
    let refreshed = control.calls.load(Ordering::SeqCst);
    assert!(refreshed > before, "the unannounced root was refreshed");
    let descendant = one(
        &registry,
        2,
        Write,
        entry(parent(50, &[50, 1, 0]), b"child", None, false),
    );
    assert!(!descendant.grant());
    assert_eq!(control.calls.load(Ordering::SeqCst), refreshed);
}
