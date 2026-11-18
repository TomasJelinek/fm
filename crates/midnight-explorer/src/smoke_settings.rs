//! Keep automated UI runs independent of the user's persisted preferences.
use midnight_core::{ComparatorValidationProof, FsError, SettingsService, UserPreferences};
use std::{
    io,
    path::Path,
    sync::{Arc, Mutex, MutexGuard},
};

pub(crate) fn for_run(smoke: bool, native: Arc<dyn SettingsService>) -> Arc<dyn SettingsService> {
    if smoke {
        Arc::new(SmokeSettings::default())
    } else {
        native
    }
}

#[derive(Default)]
struct SmokeSettings(Mutex<UserPreferences>);

impl SmokeSettings {
    fn values(&self) -> Result<MutexGuard<'_, UserPreferences>, FsError> {
        self.0.lock().map_err(|_| {
            FsError::from_io(
                "smoke preferences",
                Path::new(""),
                io::Error::other("in-memory preferences lock poisoned"),
            )
        })
    }
}

impl SettingsService for SmokeSettings {
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        Ok(self.values()?.clone())
    }

    fn save_preferences(&self, preferences: UserPreferences) -> Result<(), FsError> {
        *self.values()? = preferences;
        Ok(())
    }

    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        self.load_preferences()
    }

    fn save_preferences_prepared(
        &self,
        preferences: UserPreferences,
        proof: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        if !proof.matches(&preferences.file_comparators) {
            return Err(FsError::from_io(
                "smoke preferences",
                Path::new(""),
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "Comparator proof does not match preferences.",
                ),
            ));
        }
        preferences.validate()?;
        self.save_preferences(preferences)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{FsError, UserPreferences};

    struct NeverAccessNative;

    impl SettingsService for NeverAccessNative {
        fn font_families(&self) -> Result<Vec<String>, FsError> {
            panic!("smoke must not enumerate the user's font settings")
        }

        fn load_preferences(&self) -> Result<UserPreferences, FsError> {
            panic!("smoke must not load the user's settings")
        }

        fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
            panic!("smoke must not write the user's settings")
        }
        fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
            panic!("smoke must not load the user's settings for acceptance")
        }
        fn save_preferences_prepared(
            &self,
            _: UserPreferences,
            _: ComparatorValidationProof<'_>,
        ) -> Result<(), FsError> {
            panic!("smoke must not write prepared preferences to the user's settings")
        }
    }

    #[test]
    fn smoke_preferences_roundtrip_without_accessing_the_native_profile() {
        let settings = for_run(true, Arc::new(NeverAccessNative));
        assert!(settings.font_families().unwrap().is_empty());
        assert_eq!(
            settings.load_preferences().unwrap(),
            UserPreferences::default()
        );
        let preferences = UserPreferences {
            font_family: "Smoke 文档".into(),
            ..Default::default()
        };
        settings.save_preferences(preferences.clone()).unwrap();
        assert_eq!(settings.load_preferences().unwrap(), preferences);
        let next_run = for_run(true, Arc::new(NeverAccessNative));
        assert_eq!(
            next_run.load_preferences().unwrap(),
            UserPreferences::default()
        );
    }

    #[test]
    fn normal_runs_keep_the_native_settings_service() {
        let native: Arc<dyn SettingsService> = Arc::new(NeverAccessNative);
        assert!(Arc::ptr_eq(&native, &for_run(false, native.clone())));
    }
}
