//! Backend-only consumer of the independent native filesystem Browse/Read ABI.
#![deny(unsafe_op_in_unsafe_fn)]

use midnight_filesystem_plugin_api::{Failure, Request, Response};
use std::{fmt::Debug, sync::Arc};

mod admission;
mod capture_cache;
mod client;
pub use capture_cache::CaptureCache;
mod content;
mod discovery;
mod ffi;
mod installed;
mod network;
pub use network::NetworkProvider;
mod revision;
mod services;
mod volumes;
pub use discovery::{Catalog, Package, PackageKind};
pub use installed::InstalledProvider;
#[cfg(windows)]
#[path = "paths_windows.rs"]
mod paths;
#[cfg(target_os = "linux")]
#[path = "paths_linux.rs"]
mod paths;
#[cfg(not(any(windows, target_os = "linux")))]
#[path = "paths_unsupported.rs"]
mod paths;

type Cancel<'a> = &'a (dyn Fn() -> bool + Sync);

trait Transport: Debug + Send + Sync {
    fn call(&self, request: &Request, cancelled: Cancel<'_>) -> Result<Response, Failure>;
}

/// A provider instance. All methods and final resource drops belong on backend workers.
#[derive(Clone, Debug)]
pub struct NativeProvider {
    transport: Arc<dyn Transport>,
    capabilities: u64,
}

impl NativeProvider {
    /// Load a discovered, compatible package. Revalidates trust at load time.
    pub fn open(package: &Package) -> Result<Self, Failure> {
        if package.encoding != paths::ENCODING
            || package.capabilities & midnight_filesystem_plugin_api::abi::CAP_NATIVE_ADDRESS == 0
        {
            return Err(discovery::invalid(
                "A virtual discovery provider cannot supply native file access",
            ));
        }
        Ok(Self {
            transport: Arc::new(ffi::Instance::open(package)?),
            capabilities: package.capabilities,
        })
    }

    fn require(&self, capability: u64) -> Result<(), Failure> {
        if self.capabilities & capability == capability {
            Ok(())
        } else {
            Err(Failure::new(
                midnight_filesystem_plugin_api::ErrorCode::Unsupported,
                "Native filesystem does not advertise this capability",
            ))
        }
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(test, windows))]
#[path = "tests_windows.rs"]
mod test_path;
#[cfg(all(test, target_os = "linux"))]
#[path = "tests_linux.rs"]
mod test_path;
