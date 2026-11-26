//! Concrete Engine composition for comparison policies and external admission.
use midnight_compare_engine::{
    ExternalCompareLauncher, FileComparatorPinError, FileComparatorPolicySource,
    OwnedFileComparatorPolicy, Service,
};
use midnight_core::{
    Cancellation, Command, ExternalCompareOutcome, ExternalCompareProcess,
    ExternalCompareSessionCredit, FileComparatorPreferences, FileComparatorSnapshot, FsError,
};
use midnight_engine::{
    CommandSender, ComparisonPolicy, ComparisonPolicyPinError, ComparisonPolicySource, Engine,
    ExternalCompareNativeCapture, ExternalComparisonFrontDoor,
};
use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, mpsc::SyncSender},
};

#[cfg(test)]
#[path = "comparator_external_tests.rs"]
mod tests;

#[cfg(all(test, windows))]
#[path = "comparator_external_native_tests.rs"]
mod native_tests;

#[cfg(all(test, windows))]
#[path = "production_pool_repro.rs"]
mod production_pool_repro;

pub(crate) struct Backend {
    policies: PolicySource,
    launcher: Launcher,
}

impl Backend {
    pub(crate) fn new(
        engine: &Engine,
        commands: CommandSender,
        filesystem: Arc<midnight_native_filesystem_host::InstalledProvider>,
    ) -> Self {
        Self {
            policies: PolicySource(engine.comparator_policy_source()),
            launcher: Launcher {
                front: engine.external_comparisons(),
                commands,
                capture: crate::native_content::external_comparison_factory(filesystem),
            },
        }
    }

    pub(crate) fn configure(self, service: Service) -> Service {
        service
            .with_comparator_policies(Arc::new(self.policies))
            .with_external_launcher(Arc::new(self.launcher))
    }
}

struct PolicySource(ComparisonPolicySource);
struct Policy(ComparisonPolicy);

impl FileComparatorPolicySource for PolicySource {
    fn snapshot(&self) -> FileComparatorSnapshot {
        self.0.snapshot()
    }

    fn pin(
        &self,
        expected: &FileComparatorSnapshot,
        session: Arc<dyn ExternalCompareSessionCredit>,
    ) -> Result<Box<dyn OwnedFileComparatorPolicy>, FileComparatorPinError> {
        let policy = self.0.pin(expected, session).map_err(|error| match error {
            ComparisonPolicyPinError::PolicyChanged => FileComparatorPinError::PolicyChanged,
        })?;
        // The pin keeps Engine's guarded store, which retains and retires its set.
        Ok(Box::new(Policy(policy)))
    }
}

impl OwnedFileComparatorPolicy for Policy {
    fn snapshot(&self) -> FileComparatorSnapshot {
        self.0.snapshot()
    }

    fn preferences(&self) -> &Arc<FileComparatorPreferences> {
        self.0.preferences()
    }

    fn select(&self, original_paths: &[PathBuf; 2]) -> Option<usize> {
        self.0.select(original_paths)
    }
}

// Deliberately holds no policy source, accepted pin or Service.
struct Launcher {
    front: ExternalComparisonFrontDoor,
    commands: CommandSender,
    capture: ExternalCompareNativeCapture,
}

impl ExternalCompareLauncher for Launcher {
    fn submit(
        &self,
        process: ExternalCompareProcess,
        original_paths: [PathBuf; 2],
        session: Arc<dyn ExternalCompareSessionCredit>,
        cancel: Cancellation,
        reply: SyncSender<ExternalCompareOutcome>,
    ) -> Result<(), FsError> {
        let request = self.front.prepare(
            process,
            original_paths,
            &self.capture,
            session,
            cancel,
            reply,
        )?;
        match self
            .commands
            .try_send(Command::LaunchExternalComparison(request))
        {
            Ok(()) => Ok(()),
            Err(tokio::sync::mpsc::error::TrySendError::Full(command)) => {
                drop(command); // Intact armed request transfers cleanup to its actual slot.
                Err(admission_error(
                    io::ErrorKind::WouldBlock,
                    "Engine command queue is full.",
                ))
            }
            Err(tokio::sync::mpsc::error::TrySendError::Closed(command)) => {
                drop(command);
                Err(admission_error(
                    io::ErrorKind::BrokenPipe,
                    "Engine command queue is closed.",
                ))
            }
        }
    }
}

fn admission_error(kind: io::ErrorKind, message: &'static str) -> FsError {
    FsError::from_io(
        "admit external comparison",
        Path::new(""),
        io::Error::new(kind, message),
    )
}
