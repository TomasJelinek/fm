use super::*;
use midnight_core::*;
use midnight_work_budget::resident::{Rejection, Spawner};
use std::{
    cell::RefCell,
    path::Path,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc as blocking,
    },
    thread::ThreadId,
    time::{Duration, Instant},
};
use tokio::runtime::Handle;

struct ReportExit(blocking::Sender<ThreadId>);

impl Drop for ReportExit {
    fn drop(&mut self) {
        let _ = self.0.send(std::thread::current().id());
    }
}

thread_local! {
    static REPORT_RESIDENT_EXIT: RefCell<Option<ReportExit>> = const { RefCell::new(None) };
}

pub(super) struct HeldWorkers {
    pub(super) releases: Vec<blocking::Sender<()>>,
    pub(super) exits: blocking::Receiver<ThreadId>,
}

pub(super) fn hold_workers(spawner: &Spawner, transient: &Handle, count: usize) -> HeldWorkers {
    let (entered, ready) = blocking::channel();
    let (exit, exits) = blocking::channel();
    let mut releases = Vec::with_capacity(count);
    for _ in 0..count {
        let (release, gate) = blocking::channel();
        let entered = entered.clone();
        let exit = exit.clone();
        assert!(
            spawner
                .spawn(
                    transient,
                    Box::new(move || {
                        REPORT_RESIDENT_EXIT.with(|state| {
                            *state.borrow_mut() = Some(ReportExit(exit));
                        });
                        entered.send(()).unwrap();
                        let _ = gate.recv_timeout(Duration::from_secs(15));
                    }),
                )
                .is_ok()
        );
        releases.push(release);
        ready.recv_timeout(Duration::from_secs(5)).unwrap();
    }
    HeldWorkers { releases, exits }
}

pub(super) fn assert_sealed(spawner: &Spawner, transient: &Handle) {
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Err(refused) = spawner.spawn(transient, Box::new(|| {}))
            && refused.reason() == Rejection::Sealed
        {
            return;
        }
        assert!(Instant::now() < deadline, "Engine owner did not seal");
        std::thread::sleep(Duration::from_millis(1));
    }
}

struct Files;

impl DirectoryService for Files {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        Ok(())
    }

    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        unreachable!("empty listing has no entries")
    }
}

impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        Ok(Vec::new())
    }
}

impl StorageService for Files {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        Ok(SpaceInfo {
            available: 0,
            total: 0,
        })
    }
}

#[derive(Default)]
struct Settings {
    prepared: AtomicUsize,
}

impl SettingsService for Settings {
    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        self.load_preferences()
    }

    fn save_preferences_prepared(
        &self,
        value: UserPreferences,
        proof: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        assert!(proof.matches(&value.file_comparators));
        self.prepared.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(UserPreferences::default())
    }

    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        panic!("the Engine must use prepared persistence")
    }
}

#[test]
fn engine_navigation_and_prepared_settings_progress_while_resident_controllers_are_idle() {
    let lifetime = Engine::lifetime().unwrap();
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .max_blocking_threads(1)
        .enable_all()
        .build()
        .unwrap();
    let settings = Arc::new(Settings::default());
    let (outbox, mut events) = mpsc::channel(16);
    let mut engine = Engine::new(lifetime, Arc::new(Files), outbox);
    engine
        .configure_settings()
        .unwrap()
        .install(settings.clone());
    let (commands, input) = engine.command_channel(4).unwrap();
    let run = runtime.spawn(engine.run(input));
    let navigation = NavigationId(1);
    let revision = PreferencesRevision(1);
    let progress = runtime.block_on(async {
        commands
            .send(Command::Navigate {
                pane: PaneId::Left,
                navigation,
                path: PathBuf::from("test-root"),
                sort: SortSpec::default(),
            })
            .await
            .unwrap();
        commands
            .send(Command::SavePreferences {
                revision,
                preferences: Box::new(UserPreferences::default()),
            })
            .await
            .unwrap();
        let progress = tokio::time::timeout(Duration::from_secs(5), async {
            let (mut listed, mut saved) = (false, false);
            while !(listed && saved) {
                match events.recv().await.unwrap() {
                    Event::ListingFinished {
                        navigation: actual, ..
                    } if actual == navigation => {
                        listed = true;
                    }
                    Event::PreferencesSaved {
                        revision: actual,
                        result,
                    } if actual == revision => {
                        result.unwrap();
                        saved = true;
                    }
                    _ => {}
                }
            }
        })
        .await;
        commands.send(Command::Shutdown).await.unwrap();
        let shutdown = tokio::time::timeout(Duration::from_secs(5), run).await;
        (progress, shutdown)
    });
    runtime.shutdown_timeout(Duration::from_millis(100));
    progress
        .0
        .expect("active Engine blocked finite navigation or settings work");
    progress
        .1
        .expect("Engine shutdown exceeded its test bound")
        .unwrap();
    assert_eq!(settings.prepared.load(Ordering::SeqCst), 1);
}
