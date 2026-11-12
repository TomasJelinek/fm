//! Linux file managers do not expose a common service for embedding their extension menu.
//! The host can offer supported Open/Properties/application operations when unavailable.

use crate::LinuxFileSystem;
use midnight_core::{FsError, ShellMenuOutcome, ShellMenuRequest, ShellMenuService};

impl ShellMenuService for LinuxFileSystem {
    fn show_shell_menu(&self, request: &ShellMenuRequest) -> Result<ShellMenuOutcome, FsError> {
        request.validate()?;
        Ok(ShellMenuOutcome::Unavailable)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{ShellMenuModifiers, ShellMenuPosition, ShellMenuTarget};

    #[test]
    fn linux_shell_menu_reports_its_capability_without_starting_an_external_file_manager() {
        let request = ShellMenuRequest {
            paths: vec!["/tmp/nonexistent-menu-target".into()],
            owner: 0,
            position: ShellMenuPosition::default(),
            modifiers: ShellMenuModifiers::default(),
            target: ShellMenuTarget::Items,
        };
        assert_eq!(
            LinuxFileSystem::new(None)
                .show_shell_menu(&request)
                .unwrap(),
            ShellMenuOutcome::Unavailable
        );
    }
}
