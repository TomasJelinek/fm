//! Native navigation aliases. Discovery runs on a backend worker and creates no folders.
use crate::{
    WindowsFileSystem,
    desktop::{ComGuard, known_folder},
};
use midnight_core::{FsError, LocationKind, LocationService, NavigationLocation};
use std::{os::windows::ffi::OsStrExt, path::PathBuf};
use windows_sys::Win32::{System::Com::COINIT_MULTITHREADED, UI::Shell::*};

impl LocationService for WindowsFileSystem {
    fn supports_system_location(&self, location: midnight_core::SystemLocation) -> bool {
        matches!(location, midnight_core::SystemLocation::RecycleBin)
    }

    fn locations(&self) -> Result<Vec<NavigationLocation>, FsError> {
        let _error_mode = crate::volume_details::ErrorMode::new().map_err(|error| {
            FsError::from_io(
                "suppress location media dialogs",
                std::path::Path::new(""),
                error,
            )
        })?;
        let mut locations: Vec<_> = crate::platform_volumes()?
            .into_iter()
            .map(|volume| NavigationLocation {
                drive_letter: volume
                    .root
                    .as_os_str()
                    .encode_wide()
                    .next()
                    .and_then(|ch| char::from_u32(u32::from(ch)))
                    .filter(char::is_ascii_alphabetic),
                id: volume.id,
                label: volume.label,
                path: volume.root,
                kind: LocationKind::Volume,
            })
            .collect();
        let com = ComGuard::initialize(COINIT_MULTITHREADED as u32);
        for (id, label, kind, folder) in [
            ("home", "Home", LocationKind::Home, FOLDERID_Profile),
            (
                "desktop",
                "Desktop",
                LocationKind::Desktop,
                FOLDERID_Desktop,
            ),
            (
                "documents",
                "Documents",
                LocationKind::Documents,
                FOLDERID_Documents,
            ),
            (
                "downloads",
                "Downloads",
                LocationKind::Downloads,
                FOLDERID_Downloads,
            ),
            (
                "pictures",
                "Pictures",
                LocationKind::Pictures,
                FOLDERID_Pictures,
            ),
            ("videos", "Videos", LocationKind::Videos, FOLDERID_Videos),
            ("music", "Music", LocationKind::Music, FOLDERID_Music),
            (
                "onedrive",
                "OneDrive",
                LocationKind::OneDrive,
                FOLDERID_OneDrive,
            ),
        ] {
            let path = com
                .as_ref()
                .and_then(|_| known_folder(&folder))
                .or_else(|| {
                    (kind == LocationKind::Home)
                        .then(|| self.home.clone())
                        .flatten()
                });
            if let Some(path) = path.filter(|path| kind == LocationKind::Home || path.is_dir()) {
                append(&mut locations, id.into(), label.into(), kind, path);
            }
        }
        let temporary = std::env::temp_dir();
        if temporary.is_absolute() && temporary.is_dir() {
            append(
                &mut locations,
                "temporary".into(),
                "TEMP".into(),
                LocationKind::Temporary,
                temporary,
            );
        }
        for (account, display_name, path) in crate::onedrive_locations::accounts() {
            append_onedrive_account(&mut locations, &account, &display_name, path);
        }
        Ok(locations)
    }
}

fn append_onedrive_account(
    locations: &mut Vec<NavigationLocation>,
    account: &str,
    display_name: &str,
    path: PathBuf,
) {
    // Shell and account registrations can differ in casing, path prefix or
    // junction spelling. Compare directory identities without rewriting paths.
    let identity = |path: &std::path::Path| {
        crate::mutation::native::open_parent(path)
            .and_then(|file| crate::mutation::native::identity(&file))
            .ok()
    };
    let target = identity(&path);
    let label = format!("OneDrive — {display_name}");
    if let Some(existing) = locations.iter_mut().find(|location| {
        location.kind == LocationKind::OneDrive
            && (location.path == path
                || target.is_some_and(|id| identity(&location.path) == Some(id)))
    }) {
        existing.label = label;
    } else {
        append(
            locations,
            format!("onedrive:{account}"),
            label,
            LocationKind::OneDrive,
            path,
        );
    }
}

fn append(
    locations: &mut Vec<NavigationLocation>,
    id: String,
    label: String,
    kind: LocationKind,
    path: PathBuf,
) {
    locations.push(NavigationLocation {
        id,
        label,
        kind,
        path,
        drive_letter: None,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cloud_registrations_merge_same_directory_but_keep_distinct_accounts() {
        let temporary = tempfile::tempdir().unwrap();
        let personal = temporary.path().join("Cloud 文档");
        let business = temporary.path().join("Business");
        std::fs::create_dir(&personal).unwrap();
        std::fs::create_dir(&business).unwrap();
        let mut locations = Vec::new();
        append(
            &mut locations,
            "onedrive".into(),
            "OneDrive".into(),
            LocationKind::OneDrive,
            personal.clone(),
        );
        let alias = temporary.path().join("CLOUD 文档");
        append_onedrive_account(&mut locations, "Personal", "Personal", alias);
        assert_eq!(locations.len(), 1);
        assert_eq!(locations[0].id, "onedrive");
        assert_eq!(locations[0].path, personal);
        assert_eq!(locations[0].label, "OneDrive — Personal");
        append_onedrive_account(&mut locations, "Business1", "Work", business.clone());
        append_onedrive_account(&mut locations, "Business2", "Work", business.clone());
        assert_eq!(locations.len(), 2);
        assert_eq!(locations[1].path, business);
    }

    #[test]
    fn recycle_bin_capability_is_semantic_and_requires_no_folder_discovery() {
        let filesystem = WindowsFileSystem::new(None);
        assert!(filesystem.supports_system_location(midnight_core::SystemLocation::RecycleBin));
    }
}
