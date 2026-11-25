use super::{Location, MAX_LOCATIONS, label, locations};
use crate::rename_test_support::Temp;
use std::{
    ffi::OsStr,
    fs,
    os::unix::{ffi::OsStrExt, fs::PermissionsExt},
    path::Path,
};

fn bytes(path: &Path) -> Vec<u8> {
    path.as_os_str().as_bytes().to_vec()
}

fn location(bridge: &Path, name: &str, label: &str) -> Location {
    Location {
        root: [bytes(bridge), b"/".to_vec(), name.as_bytes().to_vec()].concat(),
        label: label.into(),
    }
}

fn sorted(mut locations: Vec<Location>) -> Vec<Location> {
    locations.sort_by(|left, right| left.root.cmp(&right.root));
    locations
}

#[test]
fn gvfs_folder_names_become_readable_drive_labels() {
    for (name, expected) in [
        (
            "smb-share:server=nas.local,share=public",
            "public on nas.local",
        ),
        (
            "smb-share:domain=WORKGROUP,server=nas,share=My%20Files,user=alice",
            "My Files on nas",
        ),
        ("smb-share:server=server,share=sdílené", "sdílené on server"),
        (
            "smb-share:server=server,share=sd%C3%ADlen%C3%A9",
            "sdílené on server",
        ),
        ("smb-server:server=nas", "nas"),
        ("sftp:host=example.com,port=2222,user=alice", "example.com"),
        ("sftp:host=%5Bfe80%3A%3A1%5D", "[fe80::1]"),
        ("ftp:host=ftp.example.org", "ftp.example.org"),
        (
            "dav:host=cloud.example.com,ssl=true,user=alice,prefix=%2Fremote.php%2Fwebdav",
            "cloud.example.com",
        ),
        ("nfs:host=server,prefix=%2Fexport%2Fdata", "data on server"),
        ("nfs:host=server", "server"),
        (
            "afp-volume:host=server.local,user=alice,volume=Shared",
            "Shared on server.local",
        ),
        (
            "mtp:host=Vendor_Phone_7_1A2B3C4D5E",
            "Vendor Phone 7 1A2B3C4D5E",
        ),
        ("mtp:host=%5Busb%3A001%2C005%5D", "[usb:001,005]"),
        (
            "gphoto2:host=Maker_Inc._Digital_Camera",
            "Maker Inc. Digital Camera",
        ),
        (
            "afc:host=00008030-001A2B3C4D5E802E,port=3",
            "00008030-001A2B3C4D5E802E",
        ),
        (
            "google-drive:host=example.com,user=alice",
            "alice@example.com",
        ),
        ("onedrive:host=example.org,user=alice", "alice@example.org"),
        (
            "archive:host=file%253A%252F%252F%252Fdata%252Fmy%252520photos.zip",
            "my photos.zip",
        ),
        ("cdda:host=sr0", "sr0"),
    ] {
        assert_eq!(label(OsStr::new(name)), expected, "{name}");
    }
}

#[test]
fn unreadable_gvfs_folder_names_keep_their_raw_text() {
    for name in [
        "trash:",
        "no-specification",
        ":host=example.com",
        "sftp:host",
        "sftp:host=",
        "sftp:user=alice",
        "smb-share:server=nas",
        "smb-share:server=nas,share=cut%2",
        "smb-share:server=nas,share=bad%zz",
        "smb-share:server=nas,share=signed%+1",
        "smb-share:server=nas,share=%FF",
        "smb-share:server=nas,share=two%0Alines",
        "google-drive:host=example.com",
        "archive:host=file%253A%252F%252F%252Fdata%252F",
    ] {
        assert_eq!(label(OsStr::new(name)), name, "{name}");
    }
    assert_eq!(
        label(OsStr::from_bytes(b"sftp:host=\xff")),
        "sftp:host=\u{fffd}"
    );
}

#[test]
fn bridge_folders_become_drives_named_after_their_locations() {
    let bridge = Temp::new();
    for name in [
        "smb-share:server=nas,share=data",
        "sftp:host=example.com,user=alice",
        "trash:",
    ] {
        fs::create_dir(bridge.path.join(name)).unwrap();
    }
    let found = locations(&[bytes(&bridge.path)], &[], &|| false);
    assert_eq!(
        sorted(found),
        [
            location(
                &bridge.path,
                "sftp:host=example.com,user=alice",
                "example.com"
            ),
            location(
                &bridge.path,
                "smb-share:server=nas,share=data",
                "data on nas"
            ),
            location(&bridge.path, "trash:", "trash:"),
        ]
    );
}

#[test]
fn locations_are_listed_without_examining_them() {
    // A location's attributes come from its backend, which may be offline. With
    // read but no search permission the folder's entries cannot be examined at all,
    // yet each is still listed.
    let bridge = Temp::new();
    fs::create_dir(bridge.path.join("smb-share:server=offline,share=data")).unwrap();
    fs::set_permissions(&bridge.path, fs::Permissions::from_mode(0o400)).unwrap();
    let found = locations(&[bytes(&bridge.path)], &[], &|| false);
    fs::set_permissions(&bridge.path, fs::Permissions::from_mode(0o700)).unwrap();
    assert_eq!(
        found,
        [location(
            &bridge.path,
            "smb-share:server=offline,share=data",
            "data on offline"
        )]
    );
}

#[test]
fn missing_or_unreadable_bridges_add_no_drives() {
    let parent = Temp::new();
    let file = parent.path.join("file");
    fs::write(&file, b"").unwrap();
    let bridges = [bytes(&parent.path.join("missing")), bytes(&file)];
    assert!(locations(&bridges, &[], &|| false).is_empty());
}

#[test]
fn locations_already_listed_as_mounts_are_not_repeated() {
    let bridge = Temp::new();
    for name in ["sftp:host=a", "sftp:host=b"] {
        fs::create_dir(bridge.path.join(name)).unwrap();
    }
    let listed = [bytes(&bridge.path.join("sftp:host=a"))];
    assert_eq!(
        locations(&[bytes(&bridge.path)], &listed, &|| false),
        [location(&bridge.path, "sftp:host=b", "b")]
    );
}

#[test]
fn at_most_max_locations_join_the_drive_list_across_bridges() {
    let first = Temp::new();
    let second = Temp::new();
    for index in 0..MAX_LOCATIONS / 2 + 10 {
        for bridge in [&first, &second] {
            fs::create_dir(bridge.path.join(format!("sftp:host={index}"))).unwrap();
        }
    }
    let found = locations(&[bytes(&first.path), bytes(&second.path)], &[], &|| false);
    assert_eq!(found.len(), MAX_LOCATIONS);
    // The first bridge is read whole; the second fills the rest.
    let parent = |location: &Location| {
        Path::new(OsStr::from_bytes(&location.root))
            .parent()
            .map(Path::to_path_buf)
    };
    assert_eq!(
        found
            .iter()
            .filter(|location| parent(location).as_deref() == Some(first.path.as_path()))
            .count(),
        MAX_LOCATIONS / 2 + 10
    );
}

#[test]
fn cancellation_stops_reading_the_bridges() {
    let bridge = Temp::new();
    fs::create_dir(bridge.path.join("sftp:host=example.com")).unwrap();
    assert!(locations(&[bytes(&bridge.path)], &[], &|| true).is_empty());
}

#[test]
fn locations_of_one_name_add_what_tells_them_apart() {
    let bridge = Temp::new();
    for name in [
        "sftp:host=example.com,user=alice",
        "sftp:host=example.com,user=bob",
        "sftp:host=example.com,port=2222,user=alice",
        "smb-share:server=nas,share=data,user=alice",
        "smb-share:domain=OFFICE,server=nas,share=data,user=alice",
        "smb-share:server=nas,share=public",
    ] {
        fs::create_dir(bridge.path.join(name)).unwrap();
    }
    let found = locations(&[bytes(&bridge.path)], &[], &|| false);
    assert_eq!(
        sorted(found),
        [
            location(
                &bridge.path,
                "sftp:host=example.com,port=2222,user=alice",
                "example.com (alice, port 2222)"
            ),
            location(
                &bridge.path,
                "sftp:host=example.com,user=alice",
                "example.com (alice)"
            ),
            location(
                &bridge.path,
                "sftp:host=example.com,user=bob",
                "example.com (bob)"
            ),
            location(
                &bridge.path,
                "smb-share:domain=OFFICE,server=nas,share=data,user=alice",
                "data on nas (alice, OFFICE)"
            ),
            location(
                &bridge.path,
                "smb-share:server=nas,share=data,user=alice",
                "data on nas (alice)"
            ),
            location(
                &bridge.path,
                "smb-share:server=nas,share=public",
                "public on nas"
            ),
        ]
    );
}

#[test]
fn locations_of_one_name_without_details_keep_it() {
    // Nothing tells these apart in their names; the drive toolbar then takes folders
    // from their paths.
    let bridge = Temp::new();
    for name in ["ftp:host=example.com", "sftp:host=example.com"] {
        fs::create_dir(bridge.path.join(name)).unwrap();
    }
    let found = locations(&[bytes(&bridge.path)], &[], &|| false);
    assert_eq!(
        sorted(found),
        [
            location(&bridge.path, "ftp:host=example.com", "example.com"),
            location(&bridge.path, "sftp:host=example.com", "example.com"),
        ]
    );
}
