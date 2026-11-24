//! The update controller: one background thread runs checks and downloads; callers
//! (the UI thread) only queue requests and read snapshots, so they never wait for the
//! network or the disk.
use crate::{
    apply::{Outcome, StartupReport},
    archive,
    client::{FetchError, UpdateClient, unix_now},
    install::{self, Installation, NotUpdatable, Staged, remove_tree},
    notes,
    release::{self, Finding, Platform, Release},
    state::{CheckState, StateStore},
    trust::{self, MAX_SIGNATURE_BYTES, MAX_SUMS_BYTES, TrustAnchor},
};
use semver::Version;
use std::{
    fs,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

/// The startup check asks GitHub at most this often.
pub const STARTUP_INTERVAL: Duration = Duration::from_secs(20 * 60 * 60);
/// A failed automatic check waits this long, doubling up to a day.
const FIRST_BACKOFF_SECONDS: u64 = 30 * 60;
const MAX_BACKOFF_SECONDS: u64 = 24 * 60 * 60;

/// What the update button and dialog show.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Phase {
    Idle,
    Checking,
    UpToDate,
    Available,
    Downloading {
        received: u64,
        total: u64,
    },
    Verifying,
    /// Downloaded, verified and staged: "Restart to update".
    Ready,
    Failed {
        message: String,
    },
}

/// The newer release found by the last check.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Offer {
    pub version: String,
    pub title: String,
    /// Plain-text excerpt of the release notes.
    pub notes: String,
    pub notes_cut: bool,
    pub page_url: String,
    pub preview: bool,
    /// The user chose "Skip this version" for it.
    pub skipped: bool,
    pub published: String,
    /// Why it cannot be installed automatically, when it cannot.
    pub blocked: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Snapshot {
    pub phase: Phase,
    pub current: String,
    pub offer: Option<Offer>,
    /// Why this copy installs no update itself; the release page is offered instead.
    pub unavailable: Option<String>,
    pub last_checked: Option<u64>,
    /// How the last update went, shown once.
    pub notice: Option<String>,
}

/// Everything the controller works with; tests build their own.
pub struct Environment {
    pub client: UpdateClient,
    pub anchor: Option<TrustAnchor>,
    pub current: Version,
    pub platform: Option<Platform>,
    pub installation: Result<Installation, NotUpdatable>,
    /// The user's private folder for downloads and `update-state.json`.
    pub private_folder: PathBuf,
    pub startup: StartupReport,
}

enum Job {
    Check {
        manual: bool,
        include_previews: bool,
    },
    StartupCheck {
        include_previews: bool,
        interval: Duration,
    },
    Download,
    Skip,
    Stop,
}

pub type Publisher = Box<dyn Fn(Snapshot) + Send>;

pub struct UpdateController {
    jobs: mpsc::Sender<Job>,
    snapshot: Arc<Mutex<Snapshot>>,
    cancel: Arc<AtomicBool>,
    staged: Arc<Mutex<Option<(Installation, Staged)>>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl UpdateController {
    /// Starts the worker thread. `publish` hears every snapshot, on the worker thread.
    pub fn start(environment: Environment, publish: Publisher) -> Self {
        Self::start_with(move || environment, publish)
    }

    /// Like `start`, but `environment` runs on the worker thread: finding the
    /// installation and the private folder touches the disk, which the UI thread must
    /// not wait for. Requests made meanwhile wait in the queue.
    pub fn start_with(
        environment: impl FnOnce() -> Environment + Send + 'static,
        publish: Publisher,
    ) -> Self {
        let snapshot = Arc::new(Mutex::new(Snapshot {
            phase: Phase::Idle,
            current: crate::release::current_version().to_string(),
            offer: None,
            unavailable: None,
            last_checked: None,
            notice: None,
        }));
        let staged = Arc::new(Mutex::new(None));
        let cancel = Arc::new(AtomicBool::new(false));
        let (jobs, receiver) = mpsc::channel();
        let shared = (snapshot.clone(), staged.clone(), cancel.clone());
        let worker = thread::Builder::new()
            .name("midnight-update".into())
            .spawn(move || {
                let environment = environment();
                let (snapshot, staged, cancel) = shared;
                if let Ok(mut slot) = snapshot.lock() {
                    *slot = initial_snapshot(&environment);
                }
                if let Ok(mut slot) = staged.lock() {
                    *slot = initial_staged(&environment);
                }
                let mut worker = Worker {
                    store: StateStore::new(&environment.private_folder),
                    environment,
                    snapshot,
                    staged,
                    cancel,
                    publish,
                    offer: None,
                    last_progress: None,
                };
                worker.publish_current();
                while let Ok(job) = receiver.recv() {
                    if matches!(job, Job::Stop) {
                        break;
                    }
                    worker.run(job);
                }
            })
            .ok();
        Self {
            jobs,
            snapshot,
            cancel,
            staged,
            worker,
        }
    }

    /// The latest snapshot; never waits for the worker's network or disk work.
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.clone())
            .unwrap_or_else(|poisoned| poisoned.into_inner().clone())
    }

    /// "Check now" (`manual`) or a check that only shows something when it finds a
    /// newer version that is not skipped.
    pub fn check(&self, manual: bool, include_previews: bool) {
        let _ = self.jobs.send(Job::Check {
            manual,
            include_previews,
        });
    }

    /// The startup check: at most once per `interval`, respecting GitHub's limit.
    pub fn startup_check(&self, include_previews: bool, interval: Duration) {
        let _ = self.jobs.send(Job::StartupCheck {
            include_previews,
            interval,
        });
    }

    /// "Download and install": downloads, verifies and stages the offered release.
    pub fn download(&self) {
        let _ = self.jobs.send(Job::Download);
    }

    /// "Skip this version".
    pub fn skip(&self) {
        let _ = self.jobs.send(Job::Skip);
    }

    /// The staged release that "Restart now" installs.
    pub fn staged(&self) -> Option<(Installation, Staged)> {
        self.staged.lock().ok().and_then(|staged| staged.clone())
    }

    /// Stops a download in progress and the worker; returns at once.
    pub fn stop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        let _ = self.jobs.send(Job::Stop);
        // The worker is detached: a request blocked on the network ends with its
        // timeout, and the process may exit before that.
        drop(self.worker.take());
    }
}

impl Drop for UpdateController {
    fn drop(&mut self) {
        self.stop();
    }
}

fn unavailable(environment: &Environment) -> Option<String> {
    if environment.platform.is_none() {
        return Some("Updates are not published for this system.".into());
    }
    if environment.anchor.is_none() {
        return Some(
            "This build has no update signing key, so it cannot install updates. Download new versions from the release page."
                .into(),
        );
    }
    environment
        .installation
        .as_ref()
        .err()
        .map(ToString::to_string)
}

fn initial_staged(environment: &Environment) -> Option<(Installation, Staged)> {
    let installation = environment.installation.as_ref().ok()?;
    let staged = install::read_staged(installation)?;
    (staged.version()? > environment.current).then(|| (installation.clone(), staged))
}

fn initial_snapshot(environment: &Environment) -> Snapshot {
    let staged = initial_staged(environment);
    // The new version learns of its update while confirming it; the outcome file that
    // the updater writes afterwards is read at a later start, where only a failure is news.
    let notice = match (&environment.startup.confirm, &environment.startup.outcome) {
        (Some(to), _) => Some(format!("Updated to version {to}.")),
        (None, Some(Outcome::RolledBack { to, reason, .. })) => Some(format!(
            "Version {to} did not start correctly, so the previous version was restored. {reason}"
        )),
        (None, Some(Outcome::NotApplied { reason })) => {
            Some(format!("The update was not installed. {reason}"))
        }
        _ => None,
    };
    Snapshot {
        phase: if staged.is_some() {
            Phase::Ready
        } else {
            Phase::Idle
        },
        current: environment.current.to_string(),
        offer: staged.map(|(_, staged)| Offer {
            version: staged.version.clone(),
            title: format!("Midnight Explorer {}", staged.version),
            notes: String::new(),
            notes_cut: false,
            page_url: staged.page_url.clone(),
            preview: false,
            skipped: false,
            published: String::new(),
            blocked: None,
        }),
        unavailable: unavailable(environment),
        last_checked: None,
        notice,
    }
}

struct Worker {
    environment: Environment,
    store: StateStore,
    snapshot: Arc<Mutex<Snapshot>>,
    staged: Arc<Mutex<Option<(Installation, Staged)>>>,
    cancel: Arc<AtomicBool>,
    publish: Publisher,
    offer: Option<Release>,
    last_progress: Option<Instant>,
}

impl Worker {
    fn update(&mut self, change: impl FnOnce(&mut Snapshot)) {
        let snapshot = {
            let mut guard = self
                .snapshot
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            change(&mut guard);
            guard.clone()
        };
        (self.publish)(snapshot);
    }

    fn publish_current(&mut self) {
        self.update(|_| {});
    }

    fn phase(&self) -> Phase {
        self.snapshot
            .lock()
            .map(|snapshot| snapshot.phase.clone())
            .unwrap_or(Phase::Idle)
    }

    fn run(&mut self, job: Job) {
        let busy = matches!(
            self.phase(),
            Phase::Downloading { .. } | Phase::Verifying | Phase::Ready
        );
        match job {
            Job::Check {
                manual,
                include_previews,
            } => {
                if !busy {
                    self.check(manual, include_previews);
                }
            }
            Job::StartupCheck {
                include_previews,
                interval,
            } => {
                let state = self.store.load();
                let now = unix_now();
                let due = now >= state.retry_after
                    && now.saturating_sub(state.last_check) >= interval.as_secs();
                if !busy && due {
                    self.check(false, include_previews);
                }
            }
            Job::Download => {
                if !busy {
                    self.download();
                }
            }
            Job::Skip => {
                if let Some(release) = &self.offer {
                    let mut state = self.store.load();
                    state.skipped = Some(release.version.to_string());
                    self.store.save(&state);
                    self.update(|snapshot| {
                        if let Some(offer) = snapshot.offer.as_mut() {
                            offer.skipped = true;
                        }
                        if snapshot.phase == Phase::Available {
                            snapshot.phase = Phase::Idle;
                        }
                    });
                }
            }
            Job::Stop => {}
        }
    }

    fn check(&mut self, manual: bool, include_previews: bool) {
        self.update(|snapshot| snapshot.phase = Phase::Checking);
        let mut state = self.store.load();
        let result = self.environment.client.releases(&self.cancel);
        let now = unix_now();
        let releases = match result {
            Ok(releases) => releases,
            Err(error) => {
                state.failures = state.failures.saturating_add(1);
                state.retry_after = match &error {
                    FetchError::RateLimited { retry_at } => *retry_at,
                    _ => {
                        let shift = state.failures.saturating_sub(1).min(6);
                        now + (FIRST_BACKOFF_SECONDS << shift).min(MAX_BACKOFF_SECONDS)
                    }
                };
                self.store.save(&state);
                let message = error.to_string();
                self.update(|snapshot| {
                    snapshot.phase = if manual {
                        Phase::Failed { message }
                    } else {
                        // A quiet startup check keeps the button calm; it tries later.
                        Phase::Idle
                    };
                });
                return;
            }
        };
        state.last_check = now;
        state.failures = 0;
        state.retry_after = 0;
        self.store.save(&state);
        let skipped = state
            .skipped
            .as_deref()
            .and_then(|version| Version::parse(version).ok());
        let finding = release::newest(
            &releases,
            &self.environment.current,
            include_previews,
            skipped.as_ref(),
        );
        match finding {
            Finding::UpToDate => {
                self.offer = None;
                self.update(|snapshot| {
                    snapshot.phase = if manual { Phase::UpToDate } else { Phase::Idle };
                    snapshot.offer = None;
                    snapshot.last_checked = Some(now);
                });
            }
            Finding::Newer { release, skipped } => {
                let blocked = self
                    .environment
                    .platform
                    .as_ref()
                    .and_then(|platform| release.missing_files(platform));
                let (text, cut) = notes::excerpt(&release.notes);
                let offer = Offer {
                    version: release.version.to_string(),
                    title: release.title.clone(),
                    notes: text,
                    notes_cut: cut,
                    page_url: release.page_url.clone(),
                    preview: release.preview,
                    skipped,
                    published: release.published.clone(),
                    blocked,
                };
                self.offer = Some(release);
                self.update(|snapshot| {
                    snapshot.phase = if skipped && !manual {
                        Phase::Idle
                    } else {
                        Phase::Available
                    };
                    snapshot.offer = Some(offer);
                    snapshot.last_checked = Some(now);
                });
            }
        }
    }

    fn fail(&mut self, message: String) {
        self.update(|snapshot| snapshot.phase = Phase::Failed { message });
    }

    fn download(&mut self) {
        let Some(release) = self.offer.clone() else {
            return self.fail("Check for updates first.".into());
        };
        let Some(platform) = self.environment.platform else {
            return self.fail("Updates are not published for this system.".into());
        };
        let installation = match &self.environment.installation {
            Ok(installation) => installation.clone(),
            Err(reason) => return self.fail(reason.to_string()),
        };
        if let Some(reason) = release.missing_files(&platform) {
            return self.fail(reason);
        }
        let archive_name = platform.archive_name(&release.version);
        let (Some(archive_asset), Some(sums_asset), Some(signature_asset)) = (
            release.asset(&archive_name).cloned(),
            release.asset(release::SUMS_NAME).cloned(),
            release.asset(release::SIGNATURE_NAME).cloned(),
        ) else {
            return self.fail("The release is missing files.".into());
        };
        let folder = self
            .environment
            .private_folder
            .join("download")
            .join(release.version.to_string());
        if let Err(error) = fs::create_dir_all(&folder) {
            return self.fail(format!("The download folder cannot be created: {error}"));
        }
        self.update(|snapshot| {
            snapshot.phase = Phase::Downloading {
                received: 0,
                total: archive_asset.size,
            };
        });
        // The signature first: an unsigned or forged release downloads nothing large.
        let client = self.environment.client.clone();
        let fetched = client
            .fetch_small(
                &sums_asset.url,
                "application/octet-stream",
                MAX_SUMS_BYTES,
                &self.cancel,
            )
            .and_then(|sums| {
                client
                    .fetch_small(
                        &signature_asset.url,
                        "application/octet-stream",
                        MAX_SIGNATURE_BYTES,
                        &self.cancel,
                    )
                    .map(|signature| (sums, signature))
            });
        let (sums, signature) = match fetched {
            Ok(files) => files,
            Err(error) => return self.fail(error.to_string()),
        };
        let signature = String::from_utf8_lossy(&signature);
        let signed = match trust::verify_sums(
            self.environment.anchor.as_ref(),
            &sums,
            &signature,
            &release.version,
        ) {
            Ok(signed) => signed,
            Err(error) => return self.fail(error.to_string()),
        };
        if signed.digest(&archive_name).is_none() {
            return self.fail(trust::TrustError::NotListed(archive_name).to_string());
        }
        let archive = folder.join(&archive_name);
        let total = archive_asset.size;
        let snapshot = self.snapshot.clone();
        let publish = &self.publish;
        let last = &mut self.last_progress;
        let mut progress = |received: u64, total: u64| {
            let now = Instant::now();
            if last.is_some_and(|last| now.duration_since(last) < Duration::from_millis(100))
                && received < total
            {
                return;
            }
            *last = Some(now);
            let current = {
                let mut guard = snapshot
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                guard.phase = Phase::Downloading { received, total };
                guard.clone()
            };
            publish(current);
        };
        if !archive.is_file()
            && let Err(error) = client.download(
                &archive_asset.url,
                &archive,
                total,
                &self.cancel,
                &mut progress,
            )
        {
            return self.fail(error.to_string());
        }
        self.update(|snapshot| snapshot.phase = Phase::Verifying);
        // Extracted through the handle that was checked (`SignedSums::open_checked`).
        let checked = match signed.open_checked(&archive_name, &archive) {
            Ok(file) => file,
            Err(error) => {
                let _ = fs::remove_file(&archive);
                return self.fail(error.to_string());
            }
        };
        let root = platform.archive_root(&release.version);
        let staging = installation.staging();
        let staged_folder = staging.join(&root);
        let staged = remove_tree(&staging)
            .and_then(|()| fs::create_dir_all(&staging))
            .map_err(|error| error.to_string())
            .and_then(|()| {
                archive::extract(checked, &platform, &root, &staged_folder)
                    .map_err(|error| error.to_string())
            })
            .and_then(|()| {
                archive::verify_staged(&staged_folder, &release.version, &platform)
                    .map_err(|error| error.to_string())
            });
        if let Err(message) = staged {
            let _ = remove_tree(&staging);
            return self.fail(message);
        }
        let staged = Staged {
            version: release.version.to_string(),
            folder: root,
            page_url: release.page_url.clone(),
        };
        if let Err(error) = install::write_staged(&installation, &staged) {
            let _ = remove_tree(&staging);
            return self.fail(format!("The update cannot be prepared: {error}"));
        }
        // The staged copy is what installs; the archive is no longer needed.
        let _ = remove_tree(&folder);
        if let Ok(mut slot) = self.staged.lock() {
            *slot = Some((installation, staged));
        }
        self.update(|snapshot| snapshot.phase = Phase::Ready);
    }
}

/// The state file is kept beside the downloads.
pub fn check_state(private_folder: &std::path::Path) -> CheckState {
    StateStore::new(private_folder).load()
}
