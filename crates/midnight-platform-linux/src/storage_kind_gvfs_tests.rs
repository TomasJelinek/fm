use super::gvfs_network_location_in as network;
use std::path::Path;

const BRIDGE: &[u8] = b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
2 1 0:50 / /run/user/1000/gvfs rw,nosuid,nodev - fuse.gvfsd-fuse gvfsd-fuse rw\n";

fn location(name: &str) -> String {
    format!("/run/user/1000/gvfs/{name}")
}

#[test]
fn gvfs_locations_of_network_protocols_are_network_locations() {
    for name in [
        "smb-share:server=nas,share=data",
        "sftp:host=example.com,user=alice",
        "ftp:host=ftp.example.org",
        "dav:host=cloud.example.com,ssl=true",
        "nfs:host=server,prefix=%2Fexport",
        "afp-volume:host=server,volume=Shared",
        "google-drive:host=example.com,user=alice",
        "onedrive:host=example.com,user=alice",
    ] {
        assert!(network(BRIDGE, Path::new(&location(name))), "{name}");
    }
}

#[test]
fn other_gvfs_locations_and_other_paths_are_not() {
    for path in [
        location("mtp:host=Vendor_Phone_1A2B3C"),
        location("gphoto2:host=Maker_Camera"),
        location("archive:host=file%253A%252F%252F%252Fa.zip"),
        location("cdda:host=sr0"),
        location("no-specification"),
        // Only a location's own folder is a drive root, not a folder inside it.
        location("smb-share:server=nas,share=data/inner"),
        "/run/user/1000/gvfs".into(),
        "/mnt/smb-share:server=nas,share=data".into(),
    ] {
        assert!(!network(BRIDGE, Path::new(&path)), "{path}");
    }
}

#[test]
fn only_the_bridge_itself_holds_gvfs_locations() {
    let share = location("smb-share:server=nas,share=data");
    for input in [
        // Another filesystem at the bridge's path, stacked over the bridge.
        b"2 1 0:50 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw\n\
3 2 8:1 / /run/user/1000/gvfs rw - ext4 /dev/sda1 rw\n"
            .as_slice(),
        // A view of a folder inside the bridge rather than its root.
        b"2 1 0:50 /inner /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw\n",
        // A mount of its own at the location's path.
        b"2 1 0:50 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw\n\
3 2 0:51 / /run/user/1000/gvfs/smb-share:server=nas,share=data rw - fuse.sshfs x rw\n",
        // No bridge at all.
        b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n",
    ] {
        assert!(!network(input, Path::new(&share)));
    }
}
