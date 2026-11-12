//! One-time service admission before accepting native/provider ownership.
use crate::{ArchiveCapture, Engine};
use midnight_core::{
    ClipboardService, DesktopServices, EditorService, ElevatedDirectoryService,
    FileDragDropService, FindService, LocaleService, MutationService, NavigationFallbackService,
    NetworkService, PathPickerService, SettingsService, ShellMenuService, TransferService,
    UserCommandService, WatchService,
};
use std::{fmt, sync::Arc};

/// A service is already installed in this engine. No incoming owner was accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ServiceAlreadyConfigured {
    service: &'static str,
}
impl fmt::Display for ServiceAlreadyConfigured {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Engine {} service is already configured", self.service)
    }
}
impl std::error::Error for ServiceAlreadyConfigured {}

/// Exclusive admission to one vacant service field in an already guarded engine.
///
/// Obtain this token before constructing or transferring the service. Installation
/// cannot fail or replace another owner. Dropping an unused token leaves the field
/// vacant, and the borrow prevents running or dropping the engine before installation.
/// This is an internal Rust composition API; it changes no plugin ABI.
#[must_use = "install the service, or drop this token to leave it unconfigured"]
pub struct ServiceSlot<'a, T> {
    field: &'a mut Option<T>,
}
impl<T> fmt::Debug for ServiceSlot<'_, T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceSlot").finish_non_exhaustive()
    }
}
impl<'a, T> ServiceSlot<'a, T> {
    fn vacant(
        field: &'a mut Option<T>,
        service: &'static str,
    ) -> Result<Self, ServiceAlreadyConfigured> {
        if field.is_some() {
            return Err(ServiceAlreadyConfigured { service });
        }
        Ok(Self { field })
    }

    /// Transfer ownership into the engine's startup-reserved service guard.
    ///
    /// The field was checked before this value was accepted, and the exclusive
    /// token cannot be reused. Final service cleanup uses the existing plain worker.
    pub fn install(self, service: T) {
        *self.field = Some(service);
    }
}

impl Engine {
    /// Admit the service used only for explicit correlated authorization retries.
    /// Returns an error if it is already configured, before accepting any service.
    pub fn configure_elevated_directories(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn ElevatedDirectoryService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.elevation, "elevated directories")
    }

    /// Admit the native archive-content capture authority before constructing it.
    /// Returns an error if it is already configured, without accepting a closure.
    pub fn configure_archive_capture(
        &mut self,
    ) -> Result<ServiceSlot<'_, ArchiveCapture>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.archive_capture, "archive capture")
    }

    /// Admit native navigation fallback once; duplicate configuration accepts no owner.
    pub fn configure_navigation_fallback(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn NavigationFallbackService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(
            &mut self.services.navigation_fallback,
            "navigation fallback",
        )
    }

    /// Admit native desktop services once; duplicate configuration accepts no owner.
    pub fn configure_desktop(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn DesktopServices>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.desktop, "desktop")
    }

    /// Admit filesystem watching once; duplicate configuration accepts no owner.
    pub fn configure_watch(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn WatchService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.watch, "watch")
    }

    /// Admit settings persistence once; duplicate configuration accepts no owner.
    pub fn configure_settings(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn SettingsService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.settings, "settings")
    }

    /// Admit the native editor service once; duplicate configuration accepts no owner.
    pub fn configure_editor(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn EditorService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.editor, "editor")
    }

    /// Admit native clipboard services once; duplicate configuration accepts no owner.
    pub fn configure_clipboard(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn ClipboardService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.clipboard, "clipboard")
    }

    /// Admit native shell menus once; duplicate configuration accepts no owner.
    pub fn configure_shell_menu(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn ShellMenuService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.shell_menu, "shell menu")
    }

    /// Admit native path selection once; duplicate configuration accepts no owner.
    pub fn configure_path_picker(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn PathPickerService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.path_picker, "path picker")
    }

    /// Admit native transfer services once; duplicate configuration accepts no owner.
    pub fn configure_transfers(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn TransferService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.transfers, "transfers")
    }

    /// Admit Custom Command services once; duplicate configuration accepts no owner.
    pub fn configure_user_commands(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn UserCommandService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.user_commands, "user commands")
    }

    /// Admit native drag/drop services once; duplicate configuration accepts no owner.
    pub fn configure_drag_drop(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn FileDragDropService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.drag_drop, "drag and drop")
    }

    /// Admit native mutation services once; duplicate configuration accepts no owner.
    pub fn configure_mutations(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn MutationService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.mutations, "mutations")
    }

    /// Admit native network dialogs once; duplicate configuration accepts no owner.
    /// An unavailable optional service is left unconfigured rather than installed as None.
    pub fn configure_network(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn NetworkService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.network, "network")
    }

    /// Admit native locale services once; duplicate configuration accepts no owner.
    /// New engines start without a locale service and preserve their existing fallback.
    pub fn configure_locale(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn LocaleService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.locale, "locale")
    }

    /// Admit native recursive search once; duplicate configuration accepts no owner.
    pub fn configure_find(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn FindService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.find, "find")
    }
}
