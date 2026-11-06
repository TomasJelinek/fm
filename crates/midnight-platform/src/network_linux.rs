use midnight_core::{LocaleService, NetworkService};
use std::sync::Arc;

/// No desktop-independent native network-drive dialog is exposed on Linux.
pub fn network_service() -> Option<Arc<dyn NetworkService>> {
    None
}

/// Locale discovery runs through this provider on a startup worker.
pub fn locale_service() -> Arc<dyn LocaleService> {
    Arc::new(midnight_platform_linux::LinuxFileSystem::new(None))
}
