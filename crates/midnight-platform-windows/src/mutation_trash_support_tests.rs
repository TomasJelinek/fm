use super::*;
use std::{collections::HashMap, fs::OpenOptions, os::windows::fs::OpenOptionsExt};
use windows_sys::Win32::Storage::FileSystem::{FILE_FLAG_BACKUP_SEMANTICS, FILE_READ_ATTRIBUTES};

/// Settings as a test sets them.
#[derive(Default)]
struct Fake {
    policy: bool,
    volumes: HashMap<&'static str, bool>,
    global: Option<bool>,
    known: Option<(PathBuf, bool)>,
}

impl Settings for Fake {
    fn policy(&self) -> bool {
        self.policy
    }
    fn volume_nuke(&self, guid: &str) -> Option<bool> {
        self.volumes.get(guid).copied()
    }
    fn global_nuke(&self) -> Option<bool> {
        self.global
    }
    fn known_folder(&self, folder: &Path) -> Option<(PathBuf, bool)> {
        self.known
            .clone()
            .filter(|(known, _)| inside(folder, known))
    }
}

fn fixed(name: &str, guid: Option<&str>) -> Volume {
    Volume {
        name: name.into(),
        drive: Drive::Fixed,
        guid: guid.map(Into::into),
    }
}

fn without_bin(name: &str) -> Volume {
    Volume {
        name: name.into(),
        drive: Drive::WithoutBin,
        guid: None,
    }
}

fn off(text: &str) -> TrashSupport {
    TrashSupport::Unavailable(text.into())
}

/// This machine's C: (`NukeOnDelete` 1) and D: (0): each fixed drive follows its own
/// "Don't move files to the Recycle Bin" setting, and only a drive without one the older
/// all-drives setting.
#[test]
fn a_fixed_drive_follows_its_own_setting_then_the_older_all_drives_one() {
    let mut settings = Fake::default();
    settings.volumes.insert("{c}", true);
    settings.volumes.insert("{d}", false);
    let folder = Path::new(r"C:\Temp");
    assert_eq!(
        decide(&fixed("C:", Some("{c}")), folder, &settings),
        off("The Recycle Bin is turned off for C:")
    );
    assert_eq!(
        decide(&fixed("D:", Some("{d}")), folder, &settings),
        TrashSupport::Available
    );
    // A volume mounted in a folder is named by that folder.
    assert_eq!(
        decide(&fixed(r"C:\mnt\data", Some("{c}")), folder, &settings),
        off(r"The Recycle Bin is turned off for C:\mnt\data")
    );
    // Without its own setting (a volume the Shell has not set up yet), the default is on.
    assert_eq!(
        decide(&fixed("E:", Some("{e}")), folder, &settings),
        TrashSupport::Available
    );
    settings.global = Some(true);
    assert_eq!(
        decide(&fixed("E:", Some("{e}")), folder, &settings),
        off("The Recycle Bin is turned off for E:")
    );
    assert_eq!(
        decide(&fixed("D:", Some("{d}")), folder, &settings),
        TrashSupport::Available,
        "a drive's own setting wins"
    );
    assert_eq!(
        decide(&fixed("S:", None), folder, &settings),
        off("The Recycle Bin is turned off for S:")
    );
}

#[test]
fn the_policy_turns_it_off_for_every_drive() {
    let mut settings = Fake {
        policy: true,
        ..Fake::default()
    };
    settings.volumes.insert("{d}", false);
    settings.known = Some((PathBuf::from(r"\\server\home\Documents"), false));
    for volume in [fixed("D:", Some("{d}")), without_bin(r"\\server\home")] {
        assert_eq!(
            decide(&volume, Path::new(r"\\server\home\Documents"), &settings),
            off("A policy turns off the Recycle Bin")
        );
    }
}

/// Network shares and drives, USB sticks, memory cards, CD/DVD and RAM drives have no
/// Recycle Bin, except in a known folder redirected there (the Shell keeps one for it);
/// an unknown drive type is left to the Shell.
#[test]
fn drives_without_a_recycle_bin_have_none_except_in_a_redirected_known_folder() {
    for (drive_type, expected) in [
        (DRIVE_FIXED, Drive::Fixed),
        (DRIVE_REMOVABLE, Drive::WithoutBin),
        (DRIVE_REMOTE, Drive::WithoutBin),
        (DRIVE_CDROM, Drive::WithoutBin),
        (DRIVE_RAMDISK, Drive::WithoutBin),
        (0, Drive::Unknown),
        (1, Drive::Unknown),
    ] {
        assert_eq!(drive(drive_type), expected, "{drive_type}");
    }
    let mut settings = Fake::default();
    let share = without_bin(r"\\server\home");
    assert_eq!(
        decide(&share, Path::new(r"\\server\home\scans"), &settings),
        off(r"There is no Recycle Bin on \\server\home")
    );
    assert_eq!(
        decide(&without_bin("E:"), Path::new(r"E:\photos"), &settings),
        off("There is no Recycle Bin on E:")
    );
    settings.known = Some((PathBuf::from(r"\\server\home\Documents"), false));
    assert_eq!(
        decide(
            &share,
            Path::new(r"\\SERVER\home\documents\taxes"),
            &settings
        ),
        TrashSupport::Available
    );
    assert_eq!(
        decide(&share, Path::new(r"\\server\home\DocumentsOld"), &settings),
        off(r"There is no Recycle Bin on \\server\home")
    );
    settings.known = Some((PathBuf::from(r"\\server\home\Documents"), true));
    assert_eq!(
        decide(&share, Path::new(r"\\server\home\Documents"), &settings),
        off(r"The Recycle Bin is turned off for \\server\home\Documents")
    );
    let unknown = Volume {
        drive: Drive::Unknown,
        ..without_bin("Q:")
    };
    assert_eq!(
        decide(&unknown, Path::new(r"Q:\x"), &settings),
        TrashSupport::Available
    );
}

#[test]
fn paths_guids_and_volume_names_are_read_as_windows_spells_them() {
    assert!(inside(
        Path::new(r"\\?\C:\Users\Me\Documents\Sub"),
        Path::new(r"c:\users\me\documents\")
    ));
    assert!(inside(
        Path::new(r"\\?\UNC\server\home\Documents"),
        Path::new(r"\\server\home\Documents")
    ));
    assert!(!inside(
        Path::new(r"C:\Users\Me\DocumentsX"),
        Path::new(r"C:\Users\Me\Documents")
    ));
    assert!(same(
        &parse_guid("{FDD39AD0-238F-46AF-ADB4-6C85480369C7}").unwrap(),
        &FOLDERID_Documents
    ));
    for text in [
        "FDD39AD0-238F-46AF-ADB4-6C85480369C7",
        "{FDD39AD0-238F-46AF-ADB4-6C85480369C}",
        "{FDD39AD0-238F-46AF-ADB4-6C85480369CG}",
        "{}",
    ] {
        assert!(parse_guid(text).is_none(), "{text}");
    }
    let root: Vec<u16> = "\\\\?\\Volume{128401dd-ba32-4638-8cd3-0ec97c0b82d5}\\"
        .encode_utf16()
        .chain([0])
        .collect();
    assert_eq!(
        guid_of(&root).as_deref(),
        Some("{128401dd-ba32-4638-8cd3-0ec97c0b82d5}")
    );
}

fn open_folder(path: &Path) -> File {
    OpenOptions::new()
        .access_mode(FILE_READ_ATTRIBUTES)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS)
        .open(path)
        .unwrap()
}

#[test]
fn a_share_is_told_by_its_path_without_touching_the_network() {
    let handle = open_folder(Path::new(env!("CARGO_MANIFEST_DIR")));
    for path in [
        r"\\does-not-exist.invalid\share\folder",
        r"\\?\UNC\does-not-exist.invalid\share\folder",
    ] {
        assert_eq!(
            volume(Path::new(path), &handle),
            Some(without_bin(r"\\does-not-exist.invalid\share"))
        );
    }
}

/// The repository's own folder: a fixed disk, whose volume name and settings key come
/// from the folder's handle. Read only.
#[test]
fn the_repository_folder_is_on_a_fixed_volume_named_by_its_handle() {
    let folder = Path::new(env!("CARGO_MANIFEST_DIR"));
    let handle = open_folder(folder);
    let volume = volume(folder, &handle).unwrap();
    assert_eq!(volume.drive, Drive::Fixed, "{volume:?}");
    let guid = volume.guid.as_deref().unwrap_or_default();
    assert!(
        guid.starts_with('{') && guid.ends_with('}') && guid.len() == 38,
        "{volume:?}"
    );
    let letter = plain(folder).to_string_lossy().chars().next().unwrap();
    assert!(volume.name.starts_with(&format!("{letter}:")), "{volume:?}");
}

/// This machine's own settings, read only: C: has "Don't move files to the Recycle Bin"
/// (`NukeOnDelete` 1), D: has a Recycle Bin. Each answer is timed; the second comes from
/// the volume's recent answer.
#[test]
#[ignore = "reads this machine's Recycle Bin settings: MIDNIGHT_TEST_NUKE_DIR names a folder on a volume whose Recycle Bin is turned off, MIDNIGHT_TEST_RECYCLE_DIR one on a volume whose Recycle Bin is on"]
fn this_machines_volumes_answer_as_their_recycle_bin_settings() {
    let mut checked = 0;
    for (variable, turned_off) in [
        ("MIDNIGHT_TEST_NUKE_DIR", true),
        ("MIDNIGHT_TEST_RECYCLE_DIR", false),
    ] {
        let Some(folder) = std::env::var_os(variable).map(PathBuf::from) else {
            continue;
        };
        let handle = open_folder(&folder);
        let started = Instant::now();
        let answer = support(&folder, &handle);
        let first = started.elapsed();
        let started = Instant::now();
        let again = support(&folder, &handle);
        let reused = started.elapsed();
        eprintln!(
            "{}: {answer:?} ({volume:?}); first {first:?}, reused {reused:?}",
            folder.display(),
            volume = volume(&folder, &handle)
        );
        assert_eq!(answer, again);
        if turned_off {
            assert!(
                matches!(&answer, TrashSupport::Unavailable(why)
                    if why.starts_with("The Recycle Bin is turned off for ")),
                "{answer:?}"
            );
        } else {
            assert_eq!(answer, TrashSupport::Available);
        }
        checked += 1;
    }
    assert!(
        checked > 0,
        "set MIDNIGHT_TEST_NUKE_DIR or MIDNIGHT_TEST_RECYCLE_DIR"
    );
}
