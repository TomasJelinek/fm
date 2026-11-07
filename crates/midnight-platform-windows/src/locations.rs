//! Native navigation aliases. Discovery runs on a backend worker and creates no folders.
use crate::{
    WindowsFileSystem,
    desktop::{ComGuard, known_folder},
};
use midnight_core::{FsError, LocationKind, LocationService, NavigationLocation};
use std::{os::windows::ffi::OsStrExt, path::PathBuf};
use windows_sys::Win32::{System::Com::COINIT_MULTITHREADED, UI::Shell::*};

impl LocationService for WindowsFileSystem {
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
            if !locations
                .iter()
                .any(|location| location.kind == LocationKind::OneDrive && location.path == path)
            {
                append(
                    &mut locations,
                    format!("onedrive:{account}"),
                    format!("OneDrive — {display_name}"),
                    LocationKind::OneDrive,
                    path,
                );
            }
        }
        Ok(locations)
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
