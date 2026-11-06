use crate::WindowsFileSystem;
use midnight_core::{FsError, SettingsService, UserPreferences};
use midnight_platform_common::settings::{encode_preferences, publish_atomic, read_preferences};
use std::{
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

fn preferences_path(base: &Path) -> io::Result<PathBuf> {
    if !base.is_absolute() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "LocalAppData must be an absolute native path",
        ));
    }
    Ok(base.join(APPLICATION_DIRECTORY).join("preferences.json"))
}

fn native_preferences_path() -> io::Result<PathBuf> {
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
    read_preferences(file)
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
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "preferences need a parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;
    let bytes = encode_preferences(preferences)?;
    publish_atomic(path, &bytes, create_exclusive, |path| {
        inspect_existing(path).map(|_| ())
    })
}

impl SettingsService for WindowsFileSystem {
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
    fn save_preferences(&self, preferences: UserPreferences) -> Result<(), FsError> {
        let path = native_preferences_path()
            .map_err(|error| FsError::from_io("resolve preferences", Path::new(""), error))?;
        save_to(&path, preferences.clone())
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
        let temporary = tempfile::tempdir().unwrap();
        let current = temporary.path().join("Midnight Explorer/preferences.json");
        let legacy = temporary
            .path()
            .join(LEGACY_APPLICATION_DIRECTORY)
            .join("preferences.json");
        let saved = UserPreferences {
            theme: PreferenceTheme::Dark,
            tabs: [
                midnight_core::PaneTabPreferences {
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
        let temporary = tempfile::tempdir().unwrap();
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
        let temporary = tempfile::tempdir().unwrap();
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
            }
        }
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 1);
        fs::write(&path, b"invalid JSON").unwrap();
        assert!(load_from(&path).is_err());
        fs::write(&path, vec![b' '; 64 * 1024 + 1]).unwrap();
        assert!(load_from(&path).is_err());
    }

    #[test]
    fn directory_destination_is_rejected_and_its_contents_preserved() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("preferences.json");
        fs::create_dir(&path).unwrap();
        fs::write(path.join("keep"), b"preserved").unwrap();
        assert!(load_from(&path).is_err());
        assert!(save_to(&path, UserPreferences::default()).is_err());
        assert_eq!(fs::read(path.join("keep")).unwrap(), b"preserved");
    }

    #[test]
    fn invalid_font_preferences_preserve_the_existing_file_without_temporary_files() {
        let temporary = tempfile::tempdir().unwrap();
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
        use std::{os::windows::process::CommandExt, process::Command};
        let temporary = tempfile::tempdir().unwrap();
        let target = temporary.path().join("outside_文档");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("keep"), b"preserved outside").unwrap();
        let path = temporary.path().join("preferences.json");
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .join("midnight-platform/tests/create-junction.ps1");
        let output = Command::new(crate::desktop::powershell_path().unwrap())
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-WindowStyle",
                "Hidden",
                "-File",
            ])
            .arg(fixture)
            .arg("-JunctionPath")
            .arg(&path)
            .arg("-TargetPath")
            .arg(&target)
            .creation_flags(windows_sys::Win32::System::Threading::CREATE_NO_WINDOW)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "junction fixture failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
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
        let temporary = tempfile::tempdir().unwrap();
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
                theme: PreferenceTheme::Dark,
                density: UiDensity::Touch,
                ..UserPreferences::default()
            },
        );
        drop(held);
        assert!(result.is_err());
        assert_eq!(fs::read(&path).unwrap(), old);
        assert_eq!(fs::read_dir(temporary.path()).unwrap().count(), 1);
    }
}
