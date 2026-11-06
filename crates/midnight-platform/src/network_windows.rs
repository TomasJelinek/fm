use midnight_core::{LocaleService, NetworkService};
use std::sync::Arc;

/// Windows supplies the native network-drive connect/disconnect dialogs.
pub fn network_service() -> Option<Arc<dyn NetworkService>> {
    Some(Arc::new(midnight_platform_windows::WindowsFileSystem::new(
        None,
    )))
}

/// Locale discovery runs through this provider on a startup worker.
pub fn locale_service() -> Arc<dyn LocaleService> {
    Arc::new(midnight_platform_windows::WindowsFileSystem::new(None))
}
