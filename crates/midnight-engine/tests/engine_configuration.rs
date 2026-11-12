//! Admission and final native ownership at the public configuration boundary.
use midnight_core::{
    Cancellation, DirectoryService, Entry, EntryMetadata, FsError, LocaleService, NetworkService,
    NumberLocale, SettingsService, SpaceInfo, StorageService, UserPreferences, Volume,
    VolumeService,
};
use midnight_engine::Engine;
use std::{
    fs::File,
    io,
    path::Path,
    sync::{Arc, Condvar, Mutex},
    thread::{self, ThreadId},
    time::Duration,
};
use tokio::sync::mpsc;

const DEADLINE: Duration = Duration::from_secs(5);
type Gate = Arc<(Mutex<bool>, Condvar)>;

struct Files;
impl DirectoryService for Files {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        unreachable!("configuration must not list files")
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        unreachable!("configuration must not inspect files")
    }
}
impl VolumeService for Files {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        unreachable!("configuration must not inspect volumes")
    }
}
impl StorageService for Files {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        unreachable!("configuration must not inspect storage")
    }
}
fn engine() -> Engine {
    let (events, _receiver) = mpsc::channel(1);
    Engine::new(Engine::lifetime().unwrap(), Arc::new(Files), events)
}

struct Release(Gate);
impl Release {
    fn open(&self) {
        *self.0.0.lock().unwrap() = true;
        self.0.1.notify_all();
    }
}
impl Drop for Release {
    fn drop(&mut self) {
        self.open();
    }
}
struct Completion(std::sync::mpsc::Sender<()>);
impl Drop for Completion {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}
struct NativeOwner {
    // Actual file and allocation remain alive while Drop waits at its gate.
    // Completion is declared afterward and acknowledges their final destruction.
    _file: File,
    _bytes: Vec<u8>,
    _completion: Completion,
    entered: std::sync::mpsc::Sender<ThreadId>,
    gate: Gate,
}
impl Drop for NativeOwner {
    fn drop(&mut self) {
        let _ = self.entered.send(thread::current().id());
        let _released = self
            .gate
            .1
            .wait_while(self.gate.0.lock().unwrap(), |released| !*released)
            .unwrap();
    }
}
struct Observer {
    entered: std::sync::mpsc::Receiver<ThreadId>,
    completed: std::sync::mpsc::Receiver<()>,
    release: Release,
}
impl Observer {
    fn finish(&self) {
        self.release.open();
        self.completed.recv_timeout(DEADLINE).unwrap();
    }
}
fn observed(blocked: bool) -> (NativeOwner, Observer) {
    let (entered, receiver) = std::sync::mpsc::channel();
    let (completed, completion) = std::sync::mpsc::channel();
    let gate = Arc::new((Mutex::new(!blocked), Condvar::new()));
    (
        NativeOwner {
            _file: tempfile::tempfile().unwrap(),
            _bytes: vec![0; 2 * 1024 * 1024],
            _completion: Completion(completed),
            entered,
            gate: gate.clone(),
        },
        Observer {
            entered: receiver,
            completed: completion,
            release: Release(gate),
        },
    )
}
struct Service {
    _owner: NativeOwner,
}
impl SettingsService for Service {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(UserPreferences::default())
    }
    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        Ok(())
    }
}
impl LocaleService for Service {
    fn number_locale(&self) -> Result<NumberLocale, FsError> {
        Ok(NumberLocale::default())
    }
}
impl NetworkService for Service {
    fn manage_connections(&self, _: bool) -> Result<(), FsError> {
        unreachable!("configuration must not open a native dialog")
    }
}

#[test]
fn repeated_configuration_rejects_before_accepting_another_owner() {
    let mut engine = engine();
    let slot = engine.configure_settings().unwrap();
    let (owner, observer) = observed(false);
    let original = Arc::new(Service { _owner: owner });
    let retained = Arc::downgrade(&original);
    slot.install(original);

    // A rejected admission takes no native argument or closure capture. Only
    // successful admission permits the caller to construct a replacement owner.
    let constructed = std::cell::Cell::new(false);
    match engine.configure_settings() {
        Ok(slot) => {
            constructed.set(true);
            let (owner, _observer) = observed(false);
            slot.install(Arc::new(Service { _owner: owner }));
        }
        Err(error) => assert_eq!(
            error.to_string(),
            "Engine settings service is already configured"
        ),
    }
    assert!(
        !constructed.get(),
        "duplicate admission constructed another owner"
    );
    assert!(
        retained.upgrade().is_some(),
        "the original service was displaced"
    );
    assert!(observer.entered.try_recv().is_err());
    drop(engine);
    assert_ne!(
        observer.entered.recv_timeout(DEADLINE).unwrap(),
        thread::current().id()
    );
    observer.finish();
    assert!(retained.upgrade().is_none());
}

#[test]
fn unused_admission_leaves_the_default_locale_vacant() {
    let mut engine = engine();
    {
        let _unused = engine.configure_locale().unwrap();
    }
    let slot = engine.configure_locale().unwrap();
    let (owner, observer) = observed(false);
    slot.install(Arc::new(Service { _owner: owner }));
    assert!(engine.configure_locale().is_err());
    drop(engine);
    assert_ne!(
        observer.entered.recv_timeout(DEADLINE).unwrap(),
        thread::current().id()
    );
    observer.finish();
}

#[test]
fn absent_network_is_unconfigured_and_an_installed_network_cannot_be_replaced() {
    let mut engine = engine();
    // There is no owner-taking None/clear operation. An unavailable network
    // service simply leaves admission unused until a real service is supplied.
    assert!(engine.configure_network().is_ok());
    let slot = engine.configure_network().unwrap();
    let (owner, observer) = observed(false);
    slot.install(Arc::new(Service { _owner: owner }));
    assert!(engine.configure_network().is_err());
    assert!(observer.entered.try_recv().is_err());
    drop(engine);
    assert_ne!(
        observer.entered.recv_timeout(DEADLINE).unwrap(),
        thread::current().id()
    );
    observer.finish();
}

#[test]
fn archive_capture_final_file_cleanup_does_not_block_engine_drop() {
    let mut engine = engine();
    let slot = engine.configure_archive_capture().unwrap();
    let (owner, observer) = observed(true);
    slot.install(Arc::new(move |path, _, _| {
        let _retained = &owner;
        Err(FsError::from_io(
            "unused capture",
            path,
            io::ErrorKind::Unsupported.into(),
        ))
    }));
    assert!(engine.configure_archive_capture().is_err());
    let (returned, completion) = std::sync::mpsc::channel();
    let caller = thread::spawn(move || {
        let caller = thread::current().id();
        drop(engine);
        returned.send(caller).unwrap();
    });
    // Release's Drop opens the gate on assertion failure, avoiding stranded tests.
    let caller_id = completion.recv_timeout(DEADLINE).unwrap();
    let cleanup_id = observer.entered.recv_timeout(DEADLINE).unwrap();
    assert_ne!(cleanup_id, caller_id);
    assert!(
        observer.completed.try_recv().is_err(),
        "native owner passed its gate early"
    );
    observer.finish();
    caller.join().unwrap();
}

#[test]
fn unpolled_run_retires_configured_service_after_tokio_runtime_shutdown() {
    let mut engine = engine();
    let slot = engine.configure_settings().unwrap();
    let (owner, observer) = observed(false);
    slot.install(Arc::new(Service { _owner: owner }));
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let (_commands, receiver) = mpsc::channel(1);
    let run = engine.run(receiver);
    drop(runtime);
    assert!(observer.entered.try_recv().is_err());
    drop(run);
    assert_ne!(
        observer.entered.recv_timeout(DEADLINE).unwrap(),
        thread::current().id()
    );
    observer.finish();
}
