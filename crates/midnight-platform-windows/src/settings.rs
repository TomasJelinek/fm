use crate::WindowsFileSystem;
use midnight_core::{ComparatorValidationProof, FsError, SettingsService, UserPreferences};
use midnight_platform_common::icon_snapshot::referenced_names;
use midnight_platform_common::settings::{
    dismissed_notices, encode_preferences, encode_preferences_prepared, publish_atomic,
    read_preferences, read_preferences_for_acceptance,
};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io,
    os::windows::{
        fs::{MetadataExt, OpenOptionsExt},
        io::AsRawHandle,
    },
    path::{Path, PathBuf},
};
use windows_sys::Win32::{
    Storage::FileSystem::{
        FILE_ATTRIBUTE_REPARSE_POINT, FILE_FLAG_OPEN_REPARSE_POINT, FILE_TYPE_DISK, GetFileType,
    },
    System::Com::COINIT_MULTITHREADED,
    UI::Shell::FOLDERID_LocalAppData,
};

const APPLICATION_DIRECTORY: &str = "Midnight Explorer";
// Kept verbatim for migration from releases before the application was renamed.
const LEGACY_APPLICATION_DIRECTORY: &str = "FM";
/// Debug builds only: an absolute folder that replaces LocalAppData for the profile.
#[cfg(debug_assertions)]
const TEST_DATA_ROOT: &str = "MIDNIGHT_TEST_DATA_ROOT";

fn preferences_path(base: &Path) -> io::Result<PathBuf> {
    if !base.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "LocalAppData must be an absolute native path",
        ));
    }
    Ok(base.join(APPLICATION_DIRECTORY).join("preferences.json"))
}

pub(crate) fn native_preferences_path() -> io::Result<PathBuf> {
    // Debug builds only: a local end-to-end run keeps the whole profile (settings and
    // the command artwork beside them) below a test folder and never touches the
    // user's LocalAppData. Release builds ignore the variable.
    #[cfg(debug_assertions)]
    if let Some(root) = std::env::var_os(TEST_DATA_ROOT) {
        return preferences_path(Path::new(&root));
    }
    let _com = crate::desktop::ComGuard::initialize(COINIT_MULTITHREADED as u32)
        .ok_or_else(|| io::Error::other("Cannot initialize native configuration lookup"))?;
    let base = crate::desktop::known_folder(&FOLDERID_LocalAppData)
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LocalAppData is unavailable"))?;
    preferences_path(&base)
}

fn check_metadata(metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "preferences cannot be a reparse point",
        ));
    }
    if !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "preferences must be a regular file",
        ));
    }
    Ok(())
}

fn inspect_existing(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            check_metadata(&metadata)?;
            Ok(Some(metadata))
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

fn load_from(path: &Path) -> io::Result<UserPreferences> {
    load_from_with(path, read_preferences)
}

fn load_from_with(
    path: &Path,
    decode: fn(File) -> io::Result<UserPreferences>,
) -> io::Result<UserPreferences> {
    if inspect_existing(path)?.is_none() {
        return Ok(UserPreferences::default());
    }
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Ok(UserPreferences::default());
        }
        Err(error) => return Err(error),
    };
    check_metadata(&file.metadata()?)?;
    // SAFETY: this is a live handle owned by File; GetFileType only inspects its type.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "preferences cannot be a device or pipe",
        ));
    }
    decode(file)
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
    // create_new refuses an existing final name, including symlinks/reparse points.
    // Default security attributes inherit the user's LocalAppData directory ACL.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

fn save_to(path: &Path, preferences: UserPreferences) -> io::Result<()> {
    let referenced = referenced_names(&preferences);
    save_preferences_encoded(path, &encode_preferences(preferences)?, &referenced)
}

/// Saves the preferences, then removes the command icon snapshots beside them that the
/// saved preferences no longer name. Collection is best effort and never fails a save.
fn save_preferences_encoded(
    path: &Path,
    bytes: &[u8],
    referenced: &BTreeSet<String>,
) -> io::Result<()> {
    save_encoded(path, bytes)?;
    if let Some(parent) = path.parent() {
        let _ = crate::icon_storage::collect_at(parent, referenced);
    }
    Ok(())
}

fn save_encoded(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "preferences need a parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;
    publish_atomic(path, bytes, create_exclusive, |path| {
        inspect_existing(path).map(|_| ())
    })
}

fn dismissed_notices_path() -> io::Result<PathBuf> {
    Ok(native_preferences_path()?.with_file_name(dismissed_notices::FILE_NAME))
}

fn load_dismissed(path: &Path) -> io::Result<Vec<String>> {
    if inspect_existing(path)?.is_none() {
        return Ok(Vec::new());
    }
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)?;
    check_metadata(&file.metadata()?)?;
    // SAFETY: this is a live handle owned by File; GetFileType only inspects its type.
    if unsafe { GetFileType(file.as_raw_handle()) } != FILE_TYPE_DISK {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "dismissed notices cannot be a device or pipe",
        ));
    }
    dismissed_notices::read(file)
}

impl SettingsService for WindowsFileSystem {
    fn load_dismissed_notices(&self) -> Result<Vec<String>, FsError> {
        let path = dismissed_notices_path()
            .map_err(|error| FsError::from_io("resolve notices", Path::new(""), error))?;
        load_dismissed(&path).map_err(|error| FsError::from_io("load notices", &path, error))
    }
    fn save_dismissed_notices(&self, keys: &[String]) -> Result<(), FsError> {
        let path = dismissed_notices_path()
            .map_err(|error| FsError::from_io("resolve notices", Path::new(""), error))?;
        dismissed_notices::encode(keys)
            .and_then(|bytes| save_encoded(&path, &bytes))
            .map_err(|error| FsError::from_io("save notices", &path, error))
    }
    fn font_families(&self) -> Result<Vec<String>, FsError> {
        crate::fonts::families()
    }
    fn load_preferences(&self) -> Result<UserPreferences, FsError> {
        let path = native_preferences_path()
            .map_err(|error| FsError::from_io("resolve preferences", Path::new(""), error))?;
        let path = load_path(&path)
            .map_err(|error| FsError::from_io("resolve preferences", &path, error))?;
        load_from(&path).map_err(|error| FsError::from_io("load preferences", &path, error))
    }
    fn load_preferences_for_acceptance(&self) -> Result<UserPreferences, FsError> {
        let path = native_preferences_path()
            .map_err(|error| FsError::from_io("resolve preferences", Path::new(""), error))?;
        let path = load_path(&path)
            .map_err(|error| FsError::from_io("resolve preferences", &path, error))?;
        load_from_with(&path, read_preferences_for_acceptance)
            .map_err(|error| FsError::from_io("load preferences", &path, error))
    }
    fn save_preferences(&self, preferences: UserPreferences) -> Result<(), FsError> {
        let path = native_preferences_path()
            .map_err(|error| FsError::from_io("resolve preferences", Path::new(""), error))?;
        save_to(&path, preferences)
            .map_err(|error| FsError::from_io("save preferences", &path, error))
    }
    fn save_preferences_prepared(
        &self,
        preferences: UserPreferences,
        proof: ComparatorValidationProof<'_>,
    ) -> Result<(), FsError> {
        let path = native_preferences_path()
            .map_err(|error| FsError::from_io("resolve preferences", Path::new(""), error))?;
        let referenced = referenced_names(&preferences);
        let bytes = encode_preferences_prepared(preferences, proof)
            .map_err(|error| FsError::from_io("save preferences", &path, error))?;
        save_preferences_encoded(&path, &bytes, &referenced)
            .map_err(|error| FsError::from_io("save preferences", &path, error))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use midnight_core::{PreferenceTheme, UiDensity};
    use std::{fs, os::windows::ffi::OsStrExt};

    #[test]
    fn resolves_native_local_app_data_path_without_profile_writes() {
        assert_eq!(
            preferences_path(Path::new("C:\\Users\\文档👩🏽‍💻\\AppData\\Local")).unwrap(),
            PathBuf::from("C:\\Users\\文档👩🏽‍💻\\AppData\\Local\\Midnight Explorer\\preferences.json")
        );
        assert!(preferences_path(Path::new("relative")).is_err());
    }

    #[test]
    fn renamed_application_reads_legacy_only_until_current_preferences_exist() {
        let temporary = crate::test_fixtures::build_temp("fixture-");
        let current = temporary.path().join("Midnight Explorer/preferences.json");
        let legacy = temporary
            .path()
            .join(LEGACY_APPLICATION_DIRECTORY)
            .join("preferences.json");
        let saved = UserPreferences {
            dark_theme: PreferenceTheme::Dark,
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
    fn renamed_application_rejects_current_or_legacy_directory_without_fallback() {
        let temporary = crate::test_fixtures::build_temp("fixture-");
        let current = temporary.path().join("Midnight Explorer/preferences.json");
        let legacy = temporary
            .path()
            .join(LEGACY_APPLICATION_DIRECTORY)
            .join("preferences.json");
        fs::create_dir_all(&legacy).unwrap();
        fs::write(legacy.join("untouched"), b"keep legacy directory").unwrap();
        assert!(load_path(&current).is_err());
        fs::remove_file(legacy.join("untouched")).unwrap();
        fs::remove_dir(&legacy).unwrap();
        save_to(&legacy, UserPreferences::default()).unwrap();
        let bytes = fs::read(&legacy).unwrap();
        fs::create_dir_all(&current).unwrap();
        fs::write(current.join("untouched"), b"keep current directory").unwrap();
        assert!(load_path(&current).is_err());
        assert_eq!(fs::read(&legacy).unwrap(), bytes);
        assert_eq!(
            fs::read(current.join("untouched")).unwrap(),
            b"keep current directory"
        );
    }

    #[test]
    fn unicode_settings_beyond_max_path_roundtrip_and_replace_atomically() {
        let temporary = crate::test_fixtures::build_temp("fixture-");
        let component = "dlouhá_中文_🧪_".repeat(5);
        let path = temporary
            .path()
            .join(&component)
            .join(&component)
            .join(&component)
            .join(&component)
            .join(APPLICATION_DIRECTORY)
            .join("preferences.json");
        assert!(path.as_os_str().encode_wide().count() > 260);
        assert_eq!(load_from(&path).unwrap(), UserPreferences::default());
        for (dark_theme, light_theme) in [
            (PreferenceTheme::Blue, PreferenceTheme::Letterhead),
            (PreferenceTheme::Dark, PreferenceTheme::Light),
            (PreferenceTheme::PlumSmoke, PreferenceTheme::CanaryCopy),
        ] {
            for density in [UiDensity::Desktop, UiDensity::Touch] {
                let preferences = UserPreferences {
                    dark_theme,
                    light_theme,
                    density,
                    font_family: "文档 é 🧪".into(),
                    desktop_font_size: 12,
                    touch_font_size: 19,
                    ..UserPreferences::default()
                };
                save_to(&path, preferences.clone()).unwrap();
                assert_eq!(load_from(&path).unwrap(), preferences);
            }
        }
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        fs::write(&path, b"invalid JSON").unwrap();
        assert!(load_from(&path).is_err());
    }

    #[test]
    fn directory_destination_is_rejected_and_its_contents_preserved() {
        let temporary = crate::test_fixtures::build_temp("fixture-");
        let path = temporary.path().join("preferences.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), b"preserved").unwrap();
        assert!(load_from(&path).is_err());
        assert!(save_to(&path, UserPreferences::default()).is_err());
        assert_eq!(fs::read(path.join("keep")).unwrap(), b"preserved");
    }

    #[test]
    fn invalid_font_preferences_preserve_the_existing_file_without_temporary_files() {
        let temporary = crate::test_fixtures::build_temp("fixture-");
        let path = temporary.path().join("preferences.json");
        save_to(&path, UserPreferences::default()).unwrap();
        let original = fs::read(&path).unwrap();
        let invalid = UserPreferences {
            font_family: "invalid\0family".into(),
            ..Default::default()
        };
        assert!(save_to(&path, invalid).is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 1);
    }

    #[test]
    fn rejects_actual_junction_reparse_point_without_touching_outside_target() {
        let temporary = crate::test_fixtures::build_temp("settings-junction-");
        let target = temporary.path().join("outside_文档");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"preserved outside").unwrap();
        let path = temporary.path().join("preferences.json");
        fs::create_dir(&path).unwrap();
        crate::test_fixtures::create_junction(&path, &target).unwrap();
        assert_eq!(
            load_from(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(
            save_to(&path, UserPreferences::default())
                .unwrap_err()
                .kind(),
            io::ErrorKind::InvalidInput
        );
        assert_eq!(fs::read(target.join("keep")).unwrap(), b"preserved outside");
        fs::remove_dir(path).unwrap();
    }

    #[test]
    fn failed_replace_preserves_locked_preferences_and_cleans_own_temp() {
        let temporary = crate::test_fixtures::build_temp("fixture-");
        let path = temporary.path().join("preferences.json");
        let old =
            midnight_platform_common::settings::encode_preferences(UserPreferences::default())
                .unwrap();
        fs::write(&path, &old).unwrap();
        let held = OpenOptions::new()
            .read(true)
            .share_mode(windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ)
            .open(&path)
            .unwrap();
        let result = save_to(
            &path,
            UserPreferences {
                dark_theme: PreferenceTheme::Dark,
                density: UiDensity::Touch,
                ..UserPreferences::default()
            },
        );
        drop(held);
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), old);
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 1);
    }

    #[test]
    fn valid_preferences_at_byte_limit_load_and_one_byte_over_is_rejected() {
        use midnight_platform_common::settings::MAX_PREFERENCES_BYTES;

        let temporary = crate::test_fixtures::build_temp("fixture-");
        let path = temporary.path().join("preferences.json");
        let expected = UserPreferences::default();
        let mut bytes = encode_preferences(expected.clone()).unwrap();
        // Adding only JSON whitespace preserves the valid document at both lengths.
        bytes.resize(MAX_PREFERENCES_BYTES, b' ');
        fs::write(&path, &bytes).unwrap();
        assert_eq!(load_from(&path).unwrap(), expected);

        bytes.push(b' ');
        fs::write(&path, &bytes).unwrap();
        let error = load_from(&path).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidData);
        assert!(error.to_string().contains("preferences exceed"));
        assert_eq!(fs::read(path).unwrap(), bytes);
    }

    /// ICON-01: a full folder of snapshots that no saved setting names no longer refuses
    /// every further import once the preferences are saved; a named snapshot stays.
    #[test]
    fn saving_preferences_frees_unreferenced_snapshots_so_an_import_succeeds_again() {
        use midnight_core::{
            Cancellation, UserCommand, UserCommandId, UserCommandInvocation, UserCommandLaunch,
        };
        use midnight_platform_common::icon_snapshot::SnapshotPixels;
        use std::time::{Duration, SystemTime};

        let temporary = crate::test_fixtures::build_temp("fixture-");
        let application = temporary.path().join(APPLICATION_DIRECTORY);
        let icons = application.join("icons");
        fs::create_dir_all(&icons).unwrap();
        let weeks_ago = SystemTime::now() - Duration::from_secs(30 * 24 * 60 * 60);
        for index in 0..1024 {
            let file = File::create(icons.join(format!("{index:064x}.png"))).unwrap();
            file.set_modified(weeks_ago).unwrap();
        }
        let pixels = || SnapshotPixels::new(1, 1, vec![1, 2, 3, 255]).unwrap();
        let cancel = Cancellation::default();
        let full = crate::icon_storage::save_test(&application, &pixels(), &cancel).unwrap_err();
        assert!(full.to_string().contains("is full"), "{full}");

        let named = icons.join(format!("{:064x}.png", 7));
        let command = UserCommand {
            output: Default::default(),
            id: UserCommandId::new("named").unwrap(),
            name: "Named".into(),
            enabled: true,
            toolbar_label: None,
            icon: Some(named.clone()),
            working_directory: "${active.path}".into(),
            launch: UserCommandLaunch::Single,
            invocation: UserCommandInvocation::Program {
                executable: std::env::current_exe().unwrap(),
                arguments: Vec::new(),
            },
        };
        let preferences = UserPreferences {
            user_commands: vec![command].into(),
            ..Default::default()
        };
        save_to(&application.join("preferences.json"), preferences).unwrap();

        let imported = crate::icon_storage::save_test(&application, &pixels(), &cancel).unwrap();
        assert!(imported.is_file());
        assert!(named.is_file(), "a snapshot a saved command names stays");
        let snapshots = fs::read_dir(&icons)
            .unwrap()
            .filter(|entry| entry.as_ref().unwrap().path().extension() == Some("png".as_ref()))
            .count();
        assert_eq!(snapshots, 2);
    }
}
