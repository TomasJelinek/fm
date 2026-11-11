//! Admit native preference writes only after the first window is successfully shown.
use crate::startup_gate::{Gate, WaitError};
use midnight_core::{FsError, SettingsService, UserPreferences};
use std::{io, path::Path, sync::Arc, time::Duration};

pub(crate) fn for_startup(
    native: Arc<dyn SettingsService>,
    gate: Arc<Gate>,
) -> Arc<dyn SettingsService> {
    Arc::new(StartupSettings { native, gate })
}

struct StartupSettings {
    native: Arc<dyn SettingsService>,
    gate: Arc<Gate>,
}

impl SettingsService for StartupSettings {
    fn font_families(&self) -> Result<Vec<String>, FsError> {
        self.native.font_families()
    }

    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        self.native.load_preferences()
    }

    fn save_preferences(&self, preferences: UserPreferences) -> Result<(), FsError> {
        self.gate.wait(Duration::from_secs(30)).map_err(|error| {
            let (kind, message) = match error {
                WaitError::Aborted => (
                    io::ErrorKind::Interrupted,
                    "Initial window did not open; saved preferences were preserved.",
                ),
                WaitError::TimedOut => (
                    io::ErrorKind::TimedOut,
                    "Initial window is not ready; saved preferences were preserved.",
                ),
            };
            FsError::from_io(
                "save preferences",
                Path::new(""),
                io::Error::new(kind, message),
            )
        })?;
        self.native.save_preferences(preferences)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::startup_gate::Owner;
    use std::{
        sync::{
            atomic::{AtomicUsize, Ordering},
            mpsc,
        },
        thread,
    };

    #[derive(Default)]
    struct RecordingSettings {
        reads: AtomicUsize,
        writes: AtomicUsize,
    }
    impl SettingsService for RecordingSettings {
        fn font_families(&self) -> Result<Vec<String>, FsError> {
            Ok(vec!["Installed family".into()])
        }
        fn load_preferences(&self) -> Result<UserPreferences, FsError> {
            self.reads.fetch_add(1, Ordering::SeqCst);
            Ok(UserPreferences::default())
        }
        fn save_preferences(&self, _: UserPreferences) -> Result<(), FsError> {
            self.writes.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }

    #[test]
    fn startup_settings_reads_pass_immediately_and_save_once_after_reveal() {
        let owner = Owner::new();
        let native = Arc::new(RecordingSettings::default());
        let settings = for_startup(native.clone(), owner.gate());
        assert_eq!(settings.font_families().unwrap(), ["Installed family"]);
        let preferences = settings.load_preferences().unwrap();
        assert_eq!(native.reads.load(Ordering::SeqCst), 1);
        let (done, received) = mpsc::channel();
        let worker =
            thread::spawn(move || done.send(settings.save_preferences(preferences)).unwrap());
        assert!(received.recv_timeout(Duration::from_millis(40)).is_err());
        assert_eq!(native.writes.load(Ordering::SeqCst), 0);
        owner.gate().reveal();
        assert!(
            received
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_ok()
        );
        worker.join().unwrap();
        assert_eq!(native.writes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn startup_settings_failed_first_reveal_never_writes_existing_preferences() {
        let owner = Owner::new();
        let native = Arc::new(RecordingSettings::default());
        let settings = for_startup(native.clone(), owner.gate());
        let (done, received) = mpsc::channel();
        let worker = thread::spawn(move || {
            done.send(settings.save_preferences(UserPreferences::default()))
                .unwrap()
        });
        assert!(received.recv_timeout(Duration::from_millis(40)).is_err());
        drop(owner);
        assert!(
            received
                .recv_timeout(Duration::from_secs(1))
                .unwrap()
                .is_err()
        );
        worker.join().unwrap();
        assert_eq!(native.writes.load(Ordering::SeqCst), 0);
    }
}
