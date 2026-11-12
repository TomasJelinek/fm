//! Short configuration acceptance, independent of source/renderer locks and I/O.
use midnight_core::{ViewerPreferenceChange, ViewerPreferences};
use std::sync::{Arc, Mutex};

pub(crate) struct Preferences {
    state: Mutex<State>,
}
struct State {
    revision: u64,
    value: Arc<ViewerPreferences>,
}
impl Preferences {
    pub fn new(value: Arc<ViewerPreferences>) -> Self {
        Self {
            state: Mutex::new(State { revision: 1, value }),
        }
    }
    pub fn snapshot(&self) -> (u64, Arc<ViewerPreferences>) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        (state.revision, state.value.clone())
    }
    pub fn change(&self, change: &ViewerPreferenceChange) -> Result<bool, String> {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut value = (*state.value).clone();
        value.apply(change).map_err(str::to_owned)?;
        if value == *state.value {
            return Ok(false);
        }
        let revision = state
            .revision
            .checked_add(1)
            .ok_or("Viewer settings revision exhausted.")?;
        *state = State {
            revision,
            value: Arc::new(value),
        };
        Ok(true)
    }
    pub fn restore(&self, value: Arc<ViewerPreferences>) -> Result<bool, String> {
        value.validate().map_err(str::to_owned)?;
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if *state.value == *value {
            return Ok(false);
        }
        let revision = state
            .revision
            .checked_add(1)
            .ok_or("Viewer settings revision exhausted.")?;
        *state = State { revision, value };
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_reselection_accepts_its_font_reset_before_any_document_work() {
        let mut initial = ViewerPreferences::default();
        initial.markdown_font.family = "monospace".into();
        initial.markdown_font.size = 28;
        let prefs = Preferences::new(Arc::new(initial));
        let preset = ViewerPreferenceChange::MarkdownPreset(midnight_core::MarkdownPreset::GitHub);
        assert!(prefs.change(&preset).unwrap());
        let (revision, accepted) = prefs.snapshot();
        assert_eq!(revision, 2);
        assert_eq!(accepted.markdown_font.family, "system-ui");
        assert_eq!(accepted.markdown_font.size, 16);
        assert!(!prefs.change(&preset).unwrap());
        assert_eq!(prefs.snapshot().0, revision);
    }

    #[test]
    fn accepted_setting_is_available_before_renderer_or_file_capture_has_run() {
        let prefs = Preferences::new(Arc::new(ViewerPreferences::default()));
        assert!(prefs.change(&ViewerPreferenceChange::Wrap(true)).unwrap());
        assert!(prefs.snapshot().1.wrap);
        assert!(!prefs.change(&ViewerPreferenceChange::Wrap(true)).unwrap());
        assert!(
            prefs
                .change(&ViewerPreferenceChange::LineNumbers(false))
                .unwrap()
        );
        assert!(prefs.snapshot().1.wrap);
        assert!(!prefs.snapshot().1.line_numbers);
    }

    #[test]
    fn invalid_restore_or_patch_keeps_the_accepted_snapshot_and_revision() {
        let prefs = Preferences::new(Arc::new(ViewerPreferences::default()));
        let (revision, old) = prefs.snapshot();
        assert!(
            prefs
                .change(&ViewerPreferenceChange::ImageZoom(
                    midnight_core::ViewerZoomPreference::ScaleBasisPoints(0)
                ))
                .is_err()
        );
        let mut invalid = (*old).clone();
        invalid.source_font.size = 0;
        assert!(prefs.restore(Arc::new(invalid)).is_err());
        let (after_revision, after) = prefs.snapshot();
        assert_eq!(revision, after_revision);
        assert!(Arc::ptr_eq(&old, &after));
    }
}
