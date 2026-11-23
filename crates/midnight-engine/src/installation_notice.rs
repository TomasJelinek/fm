//! The shared-installation notice's backend: the startup check and the notice's buttons,
//! each on a backend worker. Plugins load from a shared installation; this only reports it
//! once per installation folder and offers to make it private.
use crate::{Outbox, task_lifetime::TaskLifetime};
use midnight_core::{
    Event, InstallationEvent, InstallationRequest, InstallationStatus, SettingsService,
};
use midnight_plugin_host::{CopyInstallationError, InstallationSharing};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use tokio::task::JoinSet;

/// Resolves the running executable's installation folder.
type InstallationFolder = Arc<dyn Fn() -> Result<PathBuf, String> + Send + Sync>;
/// Starts the executable in a copied installation folder.
type Launch = Arc<dyn Fn(&Path) -> Result<(), String> + Send + Sync>;

/// Where the installation is and how dismissals and copies are stored; tests replace them.
#[derive(Clone)]
pub(crate) struct Environment {
    /// The running executable's installation folder.
    pub(crate) installation: InstallationFolder,
    /// The folder "Move to my profile" copies into.
    pub(crate) profile: Option<PathBuf>,
    pub(crate) settings: Option<Arc<dyn SettingsService>>,
    /// Starts the copied executable; the copy's folder is its working directory.
    pub(crate) launch: Launch,
}

impl Environment {
    pub(crate) fn installed(settings: Option<Arc<dyn SettingsService>>) -> Self {
        Self {
            installation: Arc::new(|| {
                let executable = midnight_plugin_host::installed_executable()
                    .map_err(|error| error.to_string())?;
                executable
                    .parent()
                    .map(Path::to_owned)
                    .ok_or_else(|| "The application has no installation folder".to_owned())
            }),
            profile: midnight_plugin_host::profile_installation_folder(),
            settings,
            launch: Arc::new(launch_copy),
        }
    }
}

/// Start the copy's executable (named like this one) without waiting for it.
fn launch_copy(folder: &Path) -> Result<(), String> {
    let name = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .file_name()
        .map(ToOwned::to_owned)
        .ok_or_else(|| "The application has no file name".to_owned())?;
    // Process creation takes the plain drive spelling of a verbatim (`\\?\C:\...`) path.
    let folder = folder
        .to_str()
        .and_then(|text| text.strip_prefix(r"\\?\"))
        .filter(|text| text.as_bytes().get(1) == Some(&b':'))
        .map_or_else(|| folder.to_owned(), PathBuf::from);
    let executable = folder.join(name);
    if !executable.is_file() {
        return Err(format!("{} is missing", executable.display()));
    }
    let mut command = std::process::Command::new(&executable);
    command.current_dir(&folder);
    // The child outlives this session; its handle is released without waiting.
    midnight_worker_platform::spawn_command(&mut command)
        .map(drop)
        .map_err(|error| error.to_string())
}

/// The dismissal key of an installation folder.
pub(crate) fn dismissal_key(folder: &Path) -> String {
    format!("shared-installation:{}", folder.display())
}

pub(crate) struct Controller {
    scope: TaskLifetime,
    outbox: Outbox,
    environment: Environment,
    /// The private copy the last "Move to my profile" made, for the restart.
    copy: Arc<Mutex<Option<PathBuf>>>,
}

impl Controller {
    /// Start the startup check; it always answers with [`InstallationEvent::Checked`].
    pub(crate) fn new(
        scope: TaskLifetime,
        outbox: Outbox,
        environment: Environment,
        tasks: &mut JoinSet<()>,
    ) -> Self {
        let controller = Self {
            scope,
            outbox,
            environment,
            copy: Arc::default(),
        };
        let environment = controller.environment.clone();
        controller.spawn(tasks, move || {
            InstallationEvent::Checked(check(&environment).unwrap_or_else(|error| {
                eprintln!("Midnight Explorer: installation check: {error}");
                None
            }))
        });
        controller
    }

    pub(crate) fn request(&self, request: InstallationRequest, tasks: &mut JoinSet<()>) {
        let environment = self.environment.clone();
        let copy = self.copy.clone();
        match request {
            InstallationRequest::Secure => self.spawn(tasks, move || secure(&environment)),
            InstallationRequest::CopyToProfile { replace } => {
                self.spawn(tasks, move || copy_to_profile(&environment, &copy, replace))
            }
            InstallationRequest::Dismiss => {
                let scope = self.scope.clone();
                tasks.spawn(self.scope.future(async move {
                    let _ = tokio::task::spawn_blocking(scope.blocking(move || {
                        if let Err(error) = dismiss(&environment) {
                            eprintln!("Midnight Explorer: the notice is not remembered: {error}");
                        }
                    }))
                    .await;
                }));
            }
            InstallationRequest::RestartFromCopy => self.spawn(tasks, move || {
                let Some(folder) = copy.lock().unwrap_or_else(|e| e.into_inner()).clone() else {
                    return InstallationEvent::RestartFailed(
                        "No copy of Midnight Explorer was made in this session".into(),
                    );
                };
                match (environment.launch)(&folder) {
                    Ok(()) => InstallationEvent::Restarting,
                    Err(error) => InstallationEvent::RestartFailed(error),
                }
            }),
        }
    }

    fn spawn(
        &self,
        tasks: &mut JoinSet<()>,
        work: impl FnOnce() -> InstallationEvent + Send + 'static,
    ) {
        let (scope, outbox) = (self.scope.clone(), self.outbox.clone());
        tasks.spawn(self.scope.future(async move {
            let event = match tokio::task::spawn_blocking(scope.blocking(work)).await {
                Ok(event) => event,
                Err(error) => InstallationEvent::SecureFailed(error.to_string()),
            };
            outbox.send(Event::Installation(Box::new(event))).await;
        }));
    }
}

fn status(sharing: InstallationSharing) -> InstallationStatus {
    InstallationStatus {
        folder: sharing.folder,
        shared_ancestor: sharing.shared_ancestor,
        shared: sharing.shared,
        shared_count: sharing.shared_count,
        refused: sharing.refused,
    }
}

fn check(environment: &Environment) -> Result<Option<InstallationStatus>, String> {
    let folder = (environment.installation)()?;
    let sharing =
        midnight_plugin_host::installation_sharing(&folder).map_err(|error| error.to_string())?;
    if sharing.is_private() {
        return Ok(None);
    }
    if let Some(settings) = &environment.settings {
        let dismissed = settings
            .load_dismissed_notices()
            .map_err(|error| error.to_string())?;
        if dismissed.contains(&dismissal_key(&folder)) {
            return Ok(None);
        }
    }
    Ok(Some(status(sharing)))
}

fn secure(environment: &Environment) -> InstallationEvent {
    let folder = match (environment.installation)() {
        Ok(folder) => folder,
        Err(error) => return InstallationEvent::SecureFailed(error),
    };
    match midnight_plugin_host::secure_installation(&folder) {
        Ok(secured) => InstallationEvent::Secured {
            status: status(secured.sharing),
            failures: secured.failures,
        },
        Err(error) => InstallationEvent::SecureFailed(error.0),
    }
}

fn copy_to_profile(
    environment: &Environment,
    copy: &Mutex<Option<PathBuf>>,
    replace: bool,
) -> InstallationEvent {
    let folder = match (environment.installation)() {
        Ok(folder) => folder,
        Err(error) => return InstallationEvent::CopyFailed(error),
    };
    let Some(destination) = environment.profile.as_deref() else {
        return InstallationEvent::CopyFailed("Your profile folder is unknown".into());
    };
    match midnight_plugin_host::copy_installation(&folder, destination, replace) {
        Ok(copied) => {
            let private = copied.sharing.is_private();
            *copy.lock().unwrap_or_else(|e| e.into_inner()) =
                private.then(|| copied.folder.clone());
            InstallationEvent::Copied {
                folder: copied.folder,
                files: copied.files,
                bytes: copied.bytes,
                status: status(copied.sharing),
            }
        }
        Err(CopyInstallationError::DestinationExists(path)) => InstallationEvent::CopyExists(path),
        Err(CopyInstallationError::Failed(error)) => InstallationEvent::CopyFailed(error),
    }
}

fn dismiss(environment: &Environment) -> Result<(), String> {
    let folder = (environment.installation)()?;
    let Some(settings) = &environment.settings else {
        return Ok(());
    };
    let keys = settings.load_dismissed_notices().unwrap_or_default();
    let keys = midnight_platform_common::settings::dismissed_notices::with_key(
        keys,
        dismissal_key(&folder),
    )
    .map_err(|error| error.to_string())?;
    settings
        .save_dismissed_notices(&keys)
        .map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "installation_notice_tests.rs"]
mod tests;
