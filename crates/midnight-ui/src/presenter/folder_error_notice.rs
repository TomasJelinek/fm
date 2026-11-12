//! Folder notices present failures; only backend receipts authorize a retry.
use super::*;

impl Presenter {
    pub(super) fn show_folder_error(
        &mut self,
        message: &str,
        path: &Path,
        content: Option<Arc<dyn GeneratedTextSource>>,
    ) {
        let reason = crate::error_presentation::folder_reason(message, path);
        self.retire_warning_viewer();
        self.desktop.report("Folder listing error", reason);
        self.desktop.notice_details = crate::error_presentation::folder_details(message, path);
        if let Some(content) = content {
            self.retain_notice_report(content);
            if !self.desktop.notice_details.is_empty() {
                self.desktop
                    .notice_details
                    .push_str("\n\nCopy details includes the full diagnostic report.");
            }
        }
        crate::error_presentation::append_path(&mut self.desktop.notice_path, path);
    }

    pub(super) fn show_navigation_error(
        &mut self,
        pane: PaneId,
        navigation: NavigationId,
        path: &Path,
        error: &FsError,
    ) {
        self.retire_warning_viewer();
        let reason = if error.kind == std::io::ErrorKind::PermissionDenied {
            "Access to this folder was denied.".into()
        } else {
            crate::error_presentation::folder_reason(&error.message, &error.path)
        };
        self.desktop.report("Cannot open folder", reason);
        crate::error_presentation::append_path(&mut self.desktop.notice_path, path);
        self.desktop.notice_copy = Some(crate::error_presentation::describe(error));
        self.desktop.notice_details = format!(
            "Operation: {}\nError kind: {:?}",
            error.operation, error.kind
        );
        let continuation = crate::error_presentation::folder_details(&error.message, &error.path);
        if !continuation.is_empty() {
            self.desktop.notice_details.push('\n');
            self.desktop.notice_details.push_str(&continuation);
        }
        let p = &self.panes[pane.index()];
        self.desktop.notice_navigation =
            Some((pane, navigation, p.error_revision, path.to_owned()));
        self.panel_warnings.modal_errors[pane.index()] = Some((p.generation, p.error_revision));
    }

    /// A modal replaces only its exact error occurrence, never a later panel failure.
    pub(crate) fn panel_error_text(&self, pane: PaneId) -> &str {
        let p = &self.panes[pane.index()];
        if p.error.is_empty()
            || self.panel_warnings.modal_errors[pane.index()]
                == Some((p.generation, p.error_revision))
        {
            &self.volumes_error
        } else {
            &p.error
        }
    }

    pub(super) fn notice_matches_elevation(&self, pane: PaneId) -> bool {
        let Some((notice_pane, navigation, revision, path)) = &self.desktop.notice_navigation
        else {
            return false;
        };
        let p = &self.panes[pane.index()];
        self.desktop.notice.is_some()
            && *notice_pane == pane
            && navigation.0 == p.generation
            && p.error_revision == *revision
            && p.elevation_retry
                .as_ref()
                .is_some_and(|(id, destination)| id == navigation && destination == path)
    }

    pub(crate) fn notice_can_authorize(&self) -> bool {
        self.desktop
            .notice_navigation
            .as_ref()
            .is_some_and(|(pane, ..)| self.can_retry_directory_elevated(*pane))
    }

    pub(crate) fn retry_notice_authorized(
        &mut self,
        generation: i32,
        send: &dyn Fn(Command) -> bool,
    ) {
        if generation != self.desktop.notice_generation || !self.notice_can_authorize() {
            return;
        }
        let (pane, _, _, path) = self
            .desktop
            .notice_navigation
            .clone()
            .expect("validated notice");
        self.retry_directory_elevated(pane, send);
        if self.panes[pane.index()].loading {
            self.desktop.notice_navigation = None;
        } else {
            self.desktop.report(
                "Cannot retry folder",
                "The navigation queue is busy. Try opening the folder again.".into(),
            );
            crate::error_presentation::append_path(&mut self.desktop.notice_path, &path);
        }
    }
}
