//! Independent actors must not become executor-thread final service owners.
use crate::{
    Outbox, find, fonts, preferences, retirement::RetirementDomain, task_lifetime::TaskLifetime,
};
use midnight_core::*;
use std::{path::Path, sync::Arc, thread::ThreadId, time::Duration};
use tokio::{sync::mpsc, task::JoinSet};

struct Service(std::sync::mpsc::Sender<ThreadId>);
impl Drop for Service {
    fn drop(&mut self) {
        let _ = self.0.send(std::thread::current().id());
    }
}
impl SettingsService for Service {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(UserPreferences::default())
    }
    fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
        Ok(())
    }
}
impl FindService for Service {
    fn open_find_source(&self, _: &Path, _: &Cancellation) -> Result<Box<dyn FindSource>, FsError> {
        unreachable!("idle actor must not access files")
    }
}

impl DirectoryService for Service {
    fn read_directory(
        &self,
        _: &Path,
        _: &Cancellation,
        _: &mut dyn FnMut(Entry) -> bool,
    ) -> Result<(), FsError> {
        unreachable!("idle actor must not list")
    }
    fn entry_metadata(&self, _: &Path) -> Result<EntryMetadata, FsError> {
        unreachable!("idle actor must not stat")
    }
}
impl VolumeService for Service {
    fn list_volumes(&self) -> Result<Vec<Volume>, FsError> {
        unreachable!("idle actor must not query volumes")
    }
}
impl StorageService for Service {
    fn space_info(&self, _: &Path) -> Result<SpaceInfo, FsError> {
        unreachable!("idle actor must not query storage")
    }
}
impl PathPickerService for Service {
    fn pick_path(
        &self,
        _: &PathPickerRequest,
        _: &Cancellation,
    ) -> Result<Option<std::path::PathBuf>, FsError> {
        unreachable!("idle actor must not open chooser")
    }
}
impl ShellMenuService for Service {
    fn show_shell_menu(&self, _: &ShellMenuRequest) -> Result<ShellMenuOutcome, FsError> {
        unreachable!("idle actor must not open menu")
    }
}

enum Actor {
    Find,
    Preferences,
    Fonts,
    Picker,
    ShellMenu,
    Volumes,
    VolumeInformation,
    FolderPresence,
    EntrySizes,
}
fn cancelled_actor_retires_service(actor: Actor) {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let domain = RetirementDomain::try_start(1, 0).unwrap();
    let (notice, dropped) = std::sync::mpsc::channel();
    let service = Arc::new(Service(notice));
    let owner = domain.try_reserve().unwrap().own(service.clone());
    let scope = TaskLifetime::new(Arc::new(owner));
    let (sender, _events) = mpsc::channel(2);
    let outbox = Outbox {
        sender,
        wakeup: Arc::new(|| {}),
    };
    let mut tasks = JoinSet::new();
    let controller: Box<dyn std::any::Any> = {
        let _entered = runtime.enter();
        match actor {
            Actor::Find => Box::new(find::Controller::new(
                scope.clone(),
                find::Retirement {
                    domain: domain.clone(),
                },
                Some(service.clone()),
                None,
                outbox,
                &mut tasks,
            )),
            Actor::Preferences => Box::new(preferences::Controller::new(
                scope.clone(),
                service.clone(),
                outbox,
                &mut tasks,
            )),
            Actor::Fonts => Box::new(fonts::Controller::new(
                scope.clone(),
                Some(service.clone()),
                outbox,
                &mut tasks,
            )),
            Actor::Picker => Box::new(crate::path_picker::Controller::new(
                scope.clone(),
                Some(service.clone()),
                outbox,
            )),
            Actor::ShellMenu => Box::new(crate::shell_menu::Controller::new(
                scope.clone(),
                crate::shell_menu::Retirement::testing(),
                Some(service.clone()),
                outbox,
            )),
            Actor::Volumes => Box::new(crate::volumes::Controller::new(
                scope.clone(),
                service.clone(),
                None,
                outbox,
                tokio::sync::watch::channel(Vec::new()).0,
                tokio::sync::watch::channel(0).0,
                &mut tasks,
            )),
            Actor::VolumeInformation => Box::new(crate::volume_information::Controller::new(
                scope.clone(),
                service.clone(),
                outbox,
                &mut tasks,
            )),
            Actor::FolderPresence => Box::new(crate::folder_presence::Controller::new(
                scope.clone(),
                crate::ReportArena::testing(),
                service.clone(),
                crate::test_snapshots(Default::default()),
                std::array::from_fn(|_| Arc::new(tokio::sync::Semaphore::new(1))),
                outbox,
                &mut tasks,
            )),
            Actor::EntrySizes => Box::new(crate::entry_sizes::Controller::new(
                crate::entry_sizes::Services {
                    reports: crate::ReportArena::testing(),
                    filesystem: service.clone(),
                    scope: scope.clone(),
                },
                outbox,
                crate::test_snapshots(Default::default()),
                Arc::new(tokio::sync::Semaphore::new(1)),
                Cancellation::default(),
                Arc::new(crate::folder_sizes::IoBudget::new()),
                Default::default(),
            )),
        }
    };
    drop(service);
    // Release the parent before the current-thread runtime processes child cancellation.
    drop(scope);
    assert!(dropped.try_recv().is_err());
    drop(controller);
    tasks.abort_all();
    runtime.block_on(async { while tasks.join_next().await.is_some() {} });
    let thread = dropped.recv_timeout(Duration::from_secs(2)).unwrap();
    assert_ne!(
        thread,
        std::thread::current().id(),
        "actor dropped its final service on the executor"
    );
}
#[test]
fn find_service_survives_parent_retirement_off_executor() {
    cancelled_actor_retires_service(Actor::Find);
}
#[test]
fn settings_service_survives_parent_retirement_off_executor() {
    cancelled_actor_retires_service(Actor::Preferences);
}
#[test]
fn font_service_survives_parent_retirement_off_executor() {
    cancelled_actor_retires_service(Actor::Fonts);
}

#[test]
fn picker_service_survives_parent_retirement_off_executor() {
    cancelled_actor_retires_service(Actor::Picker);
}

#[test]
fn shell_menu_service_survives_parent_retirement_off_executor() {
    cancelled_actor_retires_service(Actor::ShellMenu);
}

#[test]
fn volumes_service_survives_parent_retirement_off_executor() {
    cancelled_actor_retires_service(Actor::Volumes);
}

#[test]
fn volume_information_service_survives_parent_retirement_off_executor() {
    cancelled_actor_retires_service(Actor::VolumeInformation);
}

#[test]
fn folder_presence_service_survives_parent_retirement_off_executor() {
    cancelled_actor_retires_service(Actor::FolderPresence);
}

#[test]
fn entry_sizes_service_survives_parent_retirement_off_executor() {
    cancelled_actor_retires_service(Actor::EntrySizes);
}

fn recorded_scope() -> (
    Arc<Service>,
    TaskLifetime,
    std::sync::mpsc::Receiver<ThreadId>,
) {
    let (notice, dropped) = std::sync::mpsc::channel();
    let service = Arc::new(Service(notice));
    let scope = TaskLifetime::testing(service.clone());
    (service, scope, dropped)
}
fn retired_off_caller(dropped: std::sync::mpsc::Receiver<ThreadId>) {
    assert_ne!(
        dropped.recv_timeout(Duration::from_secs(2)).unwrap(),
        std::thread::current().id(),
    );
}
#[test]
fn service_anchor_survives_uninvoked_blocking_closure() {
    let (service, scope, dropped) = recorded_scope();
    let work = scope.blocking(move || drop(service));
    drop(scope);
    assert!(dropped.try_recv().is_err());
    drop(work);
    retired_off_caller(dropped);
}
#[test]
fn service_anchor_survives_panicking_blocking_closure() {
    let (service, scope, dropped) = recorded_scope();
    let work = scope.blocking(move || {
        let _owner = service;
        panic!("native failure");
    });
    drop(scope);
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).is_err());
    retired_off_caller(dropped);
}
#[test]
fn service_anchor_survives_immediately_completed_future() {
    let (service, scope, dropped) = recorded_scope();
    let future = scope.future(async move { drop(service) });
    drop(scope);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    runtime.block_on(future);
    retired_off_caller(dropped);
}
#[test]
fn service_anchor_survives_panicking_future() {
    let (service, scope, dropped) = recorded_scope();
    let future = scope.future(async move {
        let _owner = service;
        panic!("actor failure");
    });
    drop(scope);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    assert!(
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime.block_on(future);
        }))
        .is_err()
    );
    retired_off_caller(dropped);
}

#[test]
fn running_native_child_keeps_service_after_runtime_shutdown() {
    let (service, scope, dropped) = recorded_scope();
    let (entered, started) = std::sync::mpsc::channel();
    let (release, gate) = std::sync::mpsc::channel();
    let runtime = tokio::runtime::Builder::new_current_thread()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    let work = runtime.spawn_blocking(scope.blocking(move || {
        entered.send(()).unwrap();
        let _ = gate.recv();
        drop(service);
    }));
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    drop(scope);
    runtime.shutdown_timeout(Duration::from_millis(10));
    assert!(
        dropped.try_recv().is_err(),
        "native work still owns its service"
    );
    release.send(()).unwrap();
    drop(work);
    retired_off_caller(dropped);
}
