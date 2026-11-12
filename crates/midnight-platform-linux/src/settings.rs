use crate::LinuxFileSystem;
use midnight_core::{FsError, SettingsService, UserPreferences};
use midnight_platform_common::settings::{encode_preferences, publish_atomic, read_preferences};
use rustix::fs::{Mode, OFlags, open};
use std::{
    ffi::OsString,
    fs::{self, File},
    io,
    os::unix::fs::DirBuilderExt,
    path::{Path, PathBuf},
};

const APPLICATION_DIRECTORY: &str = "midnight-explorer";
// Kept verbatim for migration from releases before the application was renamed.
const LEGACY_APPLICATION_DIRECTORY: &str = "fm";

pub(crate) fn preferences_path(
    config: Option<OsString>,
    home: Option<&Path>,
) -> io::Result<PathBuf> {
    let root = config
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.filter(|home| home.is_absolute())
                .map(|home| home.join(".config"))
        })
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "No absolute configuration directory or Home is available",
            )
        })?;
    Ok(root.join(APPLICATION_DIRECTORY).join("preferences.json"))
}

fn inspect_existing(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_file() => Ok(Some(metadata)),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "preferences must be a regular non-symlink file",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn load_from(path: &Path) -> io::Result<UserPreferences> {
    if inspect_existing(path)?.is_none() {
        return Ok(UserPreferences::default());
    }
    // Preflight rejects devices before opening. NOFOLLOW/NONBLOCK and opened-descriptor
    // validation also cover a final component swapped to a symlink or FIFO afterwards.
    let descriptor = match open(
        path,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::NOCTTY,
        Mode::empty(),
    ) {
        Ok(descriptor) => descriptor,
        Err(error) if error == rustix::io::Errno::NOENT => return Ok(UserPreferences::default()),
        Err(error) => return Err(error.into()),
    };
    read_preferences(File::from(descriptor))
}

fn load_path(path: &Path) -> io::Result<PathBuf> {
    if inspect_existing(path)?.is_some() {
        return Ok(path.to_owned());
    }
    let root = path.parent().and_then(Path::parent).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "preferences need a configuration root",
        )
    })?;
    let legacy = root
        .join(LEGACY_APPLICATION_DIRECTORY)
        .join("preferences.json");
    // Only absence permits fallback. Malformed or unsafe current preferences
    // remain authoritative, and neither loading nor later saving alters legacy.
    if inspect_existing(&legacy)
        .map_err(|error| {
            io::Error::new(
                error.kind(),
                format!("Legacy preferences {}: {error}", legacy.display()),
            )
        })?
        .is_some()
    {
        Ok(legacy)
    } else {
        Ok(path.to_owned())
    }
}

fn create_exclusive(path: &Path) -> io::Result<File> {
    let descriptor = open(
        path,
        OFlags::WRONLY
            | OFlags::CREATE
            | OFlags::EXCL
            | OFlags::CLOEXEC
            | OFlags::NOFOLLOW
            | OFlags::NONBLOCK
            | OFlags::NOCTTY,
        Mode::RUSR | Mode::WUSR,
    )?;
    Ok(File::from(descriptor))
}

fn save_to(path: &Path, preferences: UserPreferences) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "preferences need a parent directory",
        )
    })?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(parent)?;
    let bytes = encode_preferences(preferences)?;
    publish_atomic(path, &bytes, create_exclusive, |path| {
        inspect_existing(path).map(|_| ())
    })
}

impl SettingsService for LinuxFileSystem {
    fn font_families(&self) -> Result<Vec<String>, FsError> {
        Ok(crate::fonts::families())
    }
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        let path = preferences_path(std::env::var_os("XDG_CONFIG_HOME"), self.home.as_deref())
            .map_err(|error| FsError::from_io("resolve preferences", Path::new(""), error))?;
        let path = load_path(&path)
            .map_err(|error| FsError::from_io("resolve preferences", &path, error))?;
        load_from(&path).map_err(|error| FsError::from_io("load preferences", &path, error))
    }
    fn save_preferences(&self, preferences: UserPreferences) -> Result<(), FsError> {
        let path = preferences_path(std::env::var_os("XDG_CONFIG_HOME"), self.home.as_deref())
            .map_err(|error| FsError::from_io("resolve preferences", Path::new(""), error))?;
        save_to(&path, preferences.clone())
            .map_err(|error| FsError::from_io("save preferences", &path, error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{PreferenceTheme, UiDensity};
    use std::{
        fs,
        os::unix::{
            ffi::OsStringExt,
            fs::{PermissionsExt, symlink},
        },
    };

    #[test]
    fn resolves_only_absolute_xdg_or_native_home_without_profile_writes() {
        assert_eq!(
            preferences_path(
                Some("/tmp/文档配置".into()),
                Some(Path::new("/home/example"))
            )
            .unwrap(),
            PathBuf::from("/tmp/文档配置/midnight-explorer/preferences.json")
        );
        assert_eq!(
            preferences_path(Some("relative".into()), Some(Path::new("/home/👩🏽‍💻"))).unwrap(),
            PathBuf::from("/home/👩🏽‍💻/.config/midnight-explorer/preferences.json")
        );
        let raw_home = PathBuf::from(OsString::from_vec(b"/tmp/raw-\xff".to_vec()));
        assert_eq!(
            preferences_path(None, Some(&raw_home)).unwrap(),
            raw_home.join(".config/midnight-explorer/preferences.json")
        );
        assert!(preferences_path(None, None).is_err());
    }

    #[test]
    fn renamed_application_reads_legacy_only_until_current_preferences_exist() {
        let temporary = tempfile::tempdir().unwrap();
        let current = temporary.path().join("midnight-explorer/preferences.json");
        let legacy = temporary
            .path()
            .join(LEGACY_APPLICATION_DIRECTORY)
            .join("preferences.json");
        let saved = UserPreferences {
            theme: PreferenceTheme::Dark,
            tabs: [
                midnight_core::PaneTabPreferences {
                    networks: Default::default(),
                    archives: Default::default(),
                    paths: vec![temporary.path().join("文档 e\u{301} 🧪")],
                    active: 0,
                },
                Default::default(),
            ],
            ..Default::default()
        };
        assert_eq!(load_path(&current).unwrap(), current);
        save_to(&legacy, saved.clone()).unwrap();
        let legacy_bytes = fs::read(&legacy).unwrap();
        assert_eq!(load_path(&current).unwrap(), legacy);
        assert_eq!(load_from(&load_path(&current).unwrap()).unwrap(), saved);
        assert!(
            !current.exists(),
            "loading never writes or removes preferences"
        );
        save_to(&current, UserPreferences::default()).unwrap();
        assert_eq!(load_path(&current).unwrap(), current);
        assert_eq!(
            load_from(&load_path(&current).unwrap()).unwrap(),
            UserPreferences::default()
        );
        assert_eq!(fs::read(&legacy).unwrap(), legacy_bytes);
        fs::write(&current, b"invalid JSON").unwrap();
        assert!(load_from(&load_path(&current).unwrap()).is_err());
        assert_eq!(fs::read(&legacy).unwrap(), legacy_bytes);
    }

    #[test]
    fn renamed_application_rejects_legacy_or_current_links_without_fallback() {
        let temporary = tempfile::tempdir().unwrap();
        let current = temporary.path().join("midnight-explorer/preferences.json");
        let legacy = temporary
            .path()
            .join(LEGACY_APPLICATION_DIRECTORY)
            .join("preferences.json");
        let outside = temporary.path().join("untouched");
        save_to(&outside, UserPreferences::default()).unwrap();
        let bytes = fs::read(&outside).unwrap();
        fs::create_dir(legacy.parent().unwrap()).unwrap();
        symlink(&outside, &legacy).unwrap();
        assert!(load_path(&current).is_err());
        fs::remove_file(&legacy).unwrap();
        save_to(&legacy, UserPreferences::default()).unwrap();
        fs::create_dir(current.parent().unwrap()).unwrap();
        symlink(&outside, &current).unwrap();
        assert!(load_path(&current).is_err());
        assert_eq!(fs::read(&outside).unwrap(), bytes);
    }

    #[test]
    fn private_unicode_settings_roundtrip_and_replace_atomically() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary
            .path()
            .join("文档👩🏽‍💻e\u{301}")
            .join(APPLICATION_DIRECTORY)
            .join("preferences.json");
        assert_eq!(load_from(&path).unwrap(), UserPreferences::default());
        for theme in [
            PreferenceTheme::Blue,
            PreferenceTheme::Dark,
            PreferenceTheme::Light,
        ] {
            for density in [UiDensity::Desktop, UiDensity::Touch] {
                let preferences = UserPreferences {
                    theme,
                    density,
                    font_family: "文档 é 🧪".into(),
                    desktop_font_size: 12,
                    touch_font_size: 19,
                    ..UserPreferences::default()
                };
                save_to(&path, preferences.clone()).unwrap();
                assert_eq!(load_from(&path).unwrap(), preferences);
                assert_eq!(
                    fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                    0o600
                );
            }
        }
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        fs::write(&path, b"invalid JSON").unwrap();
        assert!(load_from(&path).is_err());
        fs::write(&path, vec![b' '; 64 * 1024 + 1]).unwrap();
        assert!(load_from(&path).is_err());
    }

    #[test]
    fn rejects_symlink_destination_without_touching_target() {
        let temporary = tempfile::tempdir().unwrap();
        let target = temporary.path().join("target");
        let path = temporary.path().join("preferences.json");
        fs::write(&target, b"keep outside target").unwrap();
        symlink(&target, &path).unwrap();
        assert!(load_from(&path).is_err());
        assert!(save_to(&path, UserPreferences::default()).is_err());
        assert_eq!(fs::read(&target).unwrap(), b"keep outside target");
        assert!(
            fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
    }

    #[test]
    fn invalid_font_preferences_preserve_the_existing_file_without_temporary_files() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("preferences.json");
        save_to(&path, UserPreferences::default()).unwrap();
        let original = fs::read(&path).unwrap();
        let invalid = UserPreferences {
            touch_font_size: 33,
            ..Default::default()
        };
        assert!(save_to(&path, invalid).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 1);
    }

    #[test]
    fn rejects_fifo_and_symlink_fifo_without_waiting_for_writer() {
        use rustix::fs::{CWD, Mode, OFlags, mkfifoat, open};
        use std::{sync::mpsc, thread, time::Duration};
        let temporary = tempfile::tempdir().unwrap();
        let fifo = temporary.path().join("fifo");
        mkfifoat(CWD, &fifo, Mode::RUSR | Mode::WUSR).unwrap();
        let link = temporary.path().join("fifo-link");
        symlink(&fifo, &link).unwrap();
        for path in [fifo.clone(), link] {
            let (sender, receiver) = mpsc::channel();
            let worker = thread::spawn(move || {
                sender.send(load_from(&path).is_err()).unwrap();
            });
            let immediate = receiver.recv_timeout(Duration::from_millis(500));
            if immediate.is_err() {
                let writer = open(
                    &fifo,
                    OFlags::WRONLY | OFlags::NONBLOCK | OFlags::CLOEXEC,
                    Mode::empty(),
                )
                .unwrap();
                receiver.recv_timeout(Duration::from_secs(2)).unwrap();
                drop(writer);
            }
            worker.join().unwrap();
            assert!(immediate.unwrap());
        }
        assert!(load_from(Path::new("/dev/null")).is_err());
        assert!(save_to(&fifo, UserPreferences::default()).is_err());
    }
}
