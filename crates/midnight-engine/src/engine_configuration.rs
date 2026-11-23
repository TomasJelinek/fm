//! One-time service admission before accepting native/provider ownership.
use crate::transfer_composition::{QualifiedLocaleInstallation, QualifiedTransferInstallation};
use crate::{ArchiveCapture, Engine};
use midnight_core::{
    ClipboardService, DesktopServices, EditorService, ElevatedDirectoryService,
    FileDragDropService, FindService, LocaleService, MutationService, NavigationFallbackService,
    NetworkService, PathPickerService, SettingsService, ShellMenuService, TransferService,
    UserCommandService, WatchService,
};
use std::{fmt, sync::Arc};

/// One-time typed admission of the reviewed native transfer graph.
pub struct AuditedTransferSlot<'a> {
    slot: ServiceSlot<'a, Arc<dyn TransferService>>,
    qualified: &'a mut Option<QualifiedTransferInstallation>,
}
impl AuditedTransferSlot<'_> {
    pub fn install<T: midnight_core::AuditedSynchronousTransferService + 'static>(
        self,
        service: Arc<T>,
    ) {
        *self.qualified = Some(QualifiedTransferInstallation::audited(service.clone()));
        self.slot.install(service);
    }
}

/// Locale comes from the same reviewed concrete native implementation.
pub struct AuditedLocaleSlot<'a> {
    slot: ServiceSlot<'a, Arc<dyn LocaleService>>,
    qualified: &'a mut Option<QualifiedLocaleInstallation>,
}
impl AuditedLocaleSlot<'_> {
    pub fn install<
        T: LocaleService + midnight_core::AuditedSynchronousTransferService + 'static,
    >(
        self,
        service: Arc<T>,
    ) {
        *self.qualified = Some(QualifiedLocaleInstallation::audited(service.clone()));
        self.slot.install(service);
    }
}

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
    /// Check at startup whether other accounts can change the running installation, report
    /// it once ([`midnight_core::InstallationEvent::Checked`]) and serve the notice's
    /// buttons. Off by default, so engines made by tests and tools never inspect their
    /// executable's folder.
    pub fn enable_installation_notice(&mut self) {
        self.services.installation_notice = true;
    }

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
    /// Retain audited native teardown provenance before erasing the service.
    /// An ordinary erased transfer installation never obtains this witness.
    pub fn configure_audited_transfers(
        &mut self,
    ) -> Result<AuditedTransferSlot<'_>, ServiceAlreadyConfigured> {
        let services = &mut *self.services;
        let slot = ServiceSlot::vacant(&mut services.transfers, "transfers")?;
        Ok(AuditedTransferSlot {
            slot,
            qualified: &mut services.qualified_transfers,
        })
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
    ///
    /// Transfer adapters may report raw modification times, so the engine formats
    /// the transfer entries a prompt displays. This locale formats those of Rename
    /// conflicts and Make File List save confirmations. Copy/Move conflicts need
    /// [`Self::configure_audited_locale`], because a copy job retains only audited
    /// owners; without it they show only adapter-supplied text or the UI's UTC
    /// fallback.
    pub fn configure_locale(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn LocaleService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.locale, "locale")
    }

    /// Install locale from a concrete reviewed synchronous native service.
    ///
    /// It serves everything [`Self::configure_locale`] does and also formats both
    /// entries of a Copy/Move conflict prompt, and only those: copy jobs never
    /// format enumerated, inspected or automatically resolved entries.
    pub fn configure_audited_locale(
        &mut self,
    ) -> Result<AuditedLocaleSlot<'_>, ServiceAlreadyConfigured> {
        let services = &mut *self.services;
        let slot = ServiceSlot::vacant(&mut services.locale, "locale")?;
        Ok(AuditedLocaleSlot {
            slot,
            qualified: &mut services.qualified_locale,
        })
    }

    /// Admit native recursive search once; duplicate configuration accepts no owner.
    pub fn configure_find(
        &mut self,
    ) -> Result<ServiceSlot<'_, Arc<dyn FindService>>, ServiceAlreadyConfigured> {
        ServiceSlot::vacant(&mut self.services.find, "find")
    }
}
