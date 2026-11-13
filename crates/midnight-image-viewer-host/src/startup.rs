//! Source-free preparation and bounded supervision of one-use restricted transports.
use crate::Provider;
use midnight_plugin_host::CapturedFile;
use midnight_worker_platform::{Limits, ResourcePolicy};
use serde::Deserialize;
use std::{
    fs::File,
    io::{self, BufReader},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

const IMAGE_LIMITS: Limits = Limits {
    memory_bytes: 512 * 1024 * 1024,
    cpu_seconds: 60,
};
const MAX_MANIFEST_BYTES: u64 = 16 * 1024;

/// One source-free restricted worker that has acknowledged the existing Ready handshake.
/// Preparation, adoption and Drop run on a backend owner: Drop terminates and reaps the child.
/// This object retains installed-code identity only; it never owns source content or authority.
/// It is consumed by `ImageSession::open_prepared` and cannot be reused for another source.
pub struct PreparedImageWorker {
    transport: WorkerTransport,
    identity: ProviderIdentity,
}

impl PreparedImageWorker {
    /// Validate and pin installed code, start the restricted child and wait for Ready.
    /// The deadline includes validation and native startup. Native process creation cannot be
    /// interrupted; cancellation/deadline after it returns terminates and reaps that actual child.
    pub fn prepare(
        provider: &Provider,
        cancel: &AtomicBool,
        timeout: Duration,
    ) -> io::Result<Self> {
        #[cfg(feature = "startup-diagnostics")]
        let mut startup =
            midnight_worker_platform::startup_diagnostics::Trace::new("image_prepare");
        let start = Instant::now();
        remaining(cancel, start, timeout)?;
        let identity = ProviderIdentity::capture(provider)?;
        #[cfg(feature = "startup-diagnostics")]
        startup.mark("provider_pinned");
        remaining(cancel, start, timeout)?;
        let mut transport = WorkerTransport::spawn(provider)?;
        #[cfg(feature = "startup-diagnostics")]
        startup.mark("spawned");
        let timeout_left = remaining(cancel, start, timeout)?;
        transport.exchange(cancel, timeout_left, |_, output, stopped| {
            midnight_worker_sdk::read_ready(output)?;
            check_cancel(stopped)
        })?;
        #[cfg(feature = "startup-diagnostics")]
        startup.mark("ready");
        identity.validate_for(provider)?;
        remaining(cancel, start, timeout)?;
        #[cfg(feature = "startup-diagnostics")]
        startup.mark("complete");
        Ok(Self {
            transport,
            identity,
        })
    }

    /// Cheap exact descriptor/resource-policy comparison; this does not access the filesystem.
    pub fn catalog_matches(&self, provider: &Provider) -> bool {
        self.identity.catalog_matches(provider)
    }

    /// Revalidate the exact pinned code identity and current package policy before adoption.
    /// This checks no process liveness and does not wait or send any protocol message. A worker
    /// that exited after Ready fails the subsequent Open, which must remain a terminal failure.
    pub fn validate_for(&self, provider: &Provider) -> io::Result<()> {
        self.identity.validate_for(provider)
    }

    pub(crate) fn into_parts(self) -> (WorkerTransport, ProviderIdentity) {
        (self.transport, self.identity)
    }
}

pub(crate) struct ProviderIdentity {
    catalog: Provider,
    policy: ResourcePolicy,
    manifest: CapturedFile,
    executable: CapturedFile,
}

impl ProviderIdentity {
    pub(crate) fn capture(provider: &Provider) -> io::Result<Self> {
        #[cfg(feature = "startup-diagnostics")]
        let mut validation =
            midnight_worker_platform::startup_diagnostics::Trace::new("image_identity_capture");
        validate_provider(provider)?;
        #[cfg(feature = "startup-diagnostics")]
        validation.mark("policy_validated");
        let manifest = CapturedFile::capture(&provider.package_dir().join("image-viewer.json"))
            .map_err(io::Error::other)?;
        if manifest.length() > MAX_MANIFEST_BYTES {
            return Err(io::Error::other("Image-viewer manifest exceeds budget"));
        }
        let bytes = manifest
            .read_at(0, manifest.length() as u32)
            .map_err(io::Error::other)?;
        let descriptor: PinnedManifest = serde_json::from_slice(&bytes)?;
        if !descriptor.matches(provider) {
            return Err(io::Error::other(
                "Image-viewer catalog no longer matches its manifest",
            ));
        }
        #[cfg(feature = "startup-diagnostics")]
        validation.mark("manifest_matched");
        let executable = CapturedFile::capture(&provider.executable).map_err(io::Error::other)?;
        manifest.revalidate().map_err(io::Error::other)?;
        executable.revalidate().map_err(io::Error::other)?;
        #[cfg(feature = "startup-diagnostics")]
        validation.mark("complete");
        Ok(Self {
            catalog: provider.clone(),
            policy: IMAGE_LIMITS.into(),
            manifest,
            executable,
        })
    }

    fn catalog_matches(&self, provider: &Provider) -> bool {
        let catalog = &self.catalog;
        self.policy == IMAGE_LIMITS.into()
            && catalog.id == provider.id
            && catalog.name == provider.name
            && catalog.version == provider.version
            && catalog.extensions == provider.extensions
            && catalog.protocol == provider.protocol
            && catalog.executable == provider.executable
            && catalog.package_dir() == provider.package_dir()
    }

    pub(crate) fn validate_for(&self, provider: &Provider) -> io::Result<()> {
        #[cfg(feature = "startup-diagnostics")]
        let mut validation =
            midnight_worker_platform::startup_diagnostics::Trace::new("image_identity_validation");
        if !self.catalog_matches(provider) {
            return Err(io::Error::other("Prepared image worker catalog changed"));
        }
        #[cfg(feature = "startup-diagnostics")]
        validation.mark("catalog_matched");
        validate_provider(provider)?;
        #[cfg(feature = "startup-diagnostics")]
        validation.mark("policy_validated");
        self.manifest.revalidate().map_err(io::Error::other)?;
        #[cfg(feature = "startup-diagnostics")]
        validation.mark("manifest_revalidated");
        self.executable.revalidate().map_err(io::Error::other)?;
        #[cfg(feature = "startup-diagnostics")]
        validation.mark("complete");
        Ok(())
    }
}

// Reparse the bounded pinned manifest rather than accepting a descriptor discovered before its
// bytes changed. Exact equality with the already validated catalog also enforces its schema.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedManifest {
    schema: u32,
    protocol: u32,
    id: String,
    name: String,
    version: String,
    target: String,
    executable: String,
    extensions: Vec<String>,
}

impl PinnedManifest {
    fn matches(&self, provider: &Provider) -> bool {
        self.schema == 1
            && matches!(self.protocol, 1 | 2)
            && self.protocol == provider.protocol
            && self.id == provider.id
            && self.name == provider.name
            && self.version == provider.version
            && self.extensions == provider.extensions
            && self.target == midnight_plugin_host::host_target()
            && provider.package_dir().join(&self.executable) == provider.executable
            && provider
                .executable
                .file_name()
                .and_then(|name| name.to_str())
                == Some(self.executable.as_str())
    }
}

fn validate_provider(provider: &Provider) -> io::Result<()> {
    provider.validate()?;
    if provider.package_dir().join("plugin.toml").try_exists()?
        || provider
            .package_dir()
            .join("web-viewer.json")
            .try_exists()?
    {
        return Err(io::Error::other(
            "Ambiguous native, image or web viewer package",
        ));
    }
    Ok(())
}

pub(crate) struct WorkerTransport {
    input: File,
    output: BufReader<File>,
    process: midnight_worker_platform::Process,
}

impl WorkerTransport {
    pub(crate) fn spawn(provider: &Provider) -> io::Result<Self> {
        let worker = midnight_worker_platform::spawn(&provider.executable, IMAGE_LIMITS)?;
        Ok(Self {
            input: worker.input,
            output: BufReader::new(worker.output),
            process: worker.process,
        })
    }

    pub(crate) fn kill(&mut self) {
        let _ = self.process.kill();
    }

    pub(crate) fn exchange<T: Send>(
        &mut self,
        cancel: &AtomicBool,
        timeout: Duration,
        operation: impl FnOnce(&mut File, &mut BufReader<File>, &AtomicBool) -> io::Result<T> + Send,
    ) -> io::Result<T> {
        let input = &mut self.input;
        let output = &mut self.output;
        let process = &mut self.process;
        supervise(
            cancel,
            timeout,
            || {
                let _ = process.kill();
            },
            |stopped| operation(input, output, stopped),
        )
    }
}

fn supervise<T: Send>(
    cancel: &AtomicBool,
    timeout: Duration,
    mut terminate: impl FnMut(),
    operation: impl FnOnce(&AtomicBool) -> io::Result<T> + Send,
) -> io::Result<T> {
    let start = Instant::now();
    if let Err(error) = remaining(cancel, start, timeout) {
        terminate();
        return Err(error);
    }
    let stopped = AtomicBool::new(false);
    std::thread::scope(|scope| {
        let (sender, receiver) = mpsc::sync_channel(1);
        let stopped = &stopped;
        let broker = scope.spawn(move || {
            let _ = sender.send(operation(stopped));
        });
        let result = loop {
            let timeout_left = match remaining(cancel, start, timeout) {
                Ok(timeout_left) => timeout_left,
                Err(error) => break Err(error),
            };
            match receiver.recv_timeout(timeout_left.min(Duration::from_millis(10))) {
                Ok(result) => break remaining(cancel, start, timeout).and(result),
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    break Err(io::Error::other("Image broker stopped"));
                }
            }
        };
        stopped.store(true, Ordering::Release);
        if result.is_err() {
            terminate();
        }
        broker
            .join()
            .map_err(|_| io::Error::other("Image broker panicked"))?;
        result
    })
}

fn remaining(cancel: &AtomicBool, start: Instant, timeout: Duration) -> io::Result<Duration> {
    check_cancel(cancel)?;
    timeout
        .checked_sub(start.elapsed())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "Image viewer timed out"))
}

pub(crate) fn check_cancel(cancel: &AtomicBool) -> io::Result<()> {
    if cancel.load(Ordering::Acquire) {
        Err(cancelled())
    } else {
        Ok(())
    }
}

pub(crate) fn cancelled() -> io::Error {
    io::Error::new(io::ErrorKind::Interrupted, "Image viewer cancelled")
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;
