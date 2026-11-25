use super::mount_paths as paths;

#[test]
fn drive_list_hides_efi_partition_without_hiding_prefix_siblings() {
    let input = b"1 0 8:1 / /efi rw - vfat /dev/sda1 rw\n\
2 1 8:2 / /efi/extra rw - vfat /dev/sda2 rw\n\
3 0 8:3 / /efibackup rw - ext4 /dev/sdb1 rw\n";
    assert_eq!(paths(input), [b"/efibackup".to_vec()]);
}

#[test]
fn drive_list_excludes_internal_filesystems_outside_system_directories() {
    for filesystem in [
        "squashfs",
        "nsfs",
        "fuse.portal",
        "fuse.snapfuse",
        "overlay",
        "aufs",
        "fuse.overlayfs",
    ] {
        let input = format!("1 0 0:1 / /mnt/internal ro - {filesystem} internal ro\n");
        assert!(paths(input.as_bytes()).is_empty(), "{filesystem}");
    }
}

#[test]
fn drive_list_excludes_package_namespace_portal_and_boot_mounts() {
    let input = b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
2 1 8:2 / /boot rw - ext4 /dev/sda2 rw\n\
3 2 8:3 / /boot/efi rw - vfat /dev/sda3 rw\n\
4 1 0:1 / /snap/code/271 ro - squashfs /dev/loop0 ro\n\
5 1 0:2 / /run/docker/netns/default rw - nsfs nsfs rw\n\
6 1 0:3 / /run/snapd/ns/slack.mnt rw - nsfs nsfs rw\n\
7 1 0:4 / /run/user/1000/doc rw - fuse.portal portal rw\n\
8 1 0:5 / /opt/sentinelone/ebpffs/bpf_mount rw - bpf bpf rw\n\
9 1 0:6 / /opt/sentinelone/mount rw - tracefs tracefs rw\n\
10 1 0:7 / /var/lib/snapd/snap/code/271 ro - fuse.snapfuse snapfuse ro\n\
11 1 0:8 / /var/lib/docker/overlay2/merged rw - overlay overlay rw\n\
12 1 0:9 / /tmp rw - tmpfs tmpfs rw\n\
13 1 0:10 / /var/tmp rw - tmpfs tmpfs rw\n\
14 1 8:4 / /sn\\141p/private rw - ext4 /dev/sdb1 rw\n";
    assert_eq!(paths(input), [b"/".to_vec()]);
}

#[test]
fn drive_list_preserves_local_removable_network_and_userspace_storage() {
    let input = b"1 0 0:1 / / rw - overlay overlay rw\n\
2 1 8:2 / /data rw - ext4 /dev/sdb1 rw\n\
3 1 8:3 / /media/My\\040USB rw - exfat /dev/sdc1 rw\n\
4 1 8:4 / /run/media/alice/Backup rw - ntfs3 /dev/sdd1 rw\n\
5 1 0:2 / /mnt/share rw - cifs //server/share rw\n\
6 1 0:3 /export /srv/team rw - nfs4 server:/export rw\n\
7 1 0:4 / /home/alice/remote rw - fuse.sshfs alice@server:/ rw\n\
8 1 0:5 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw\n\
9 1 0:6 /@home /home rw - btrfs /dev/sda1 rw\n\
10 1 11:0 / /media/DVD ro - iso9660 /dev/sr0 ro\n";
    assert_eq!(
        paths(input),
        [
            b"/".to_vec(),
            b"/data".to_vec(),
            b"/media/My USB".to_vec(),
            b"/run/media/alice/Backup".to_vec(),
            b"/mnt/share".to_vec(),
            b"/srv/team".to_vec(),
            b"/home/alice/remote".to_vec(),
            b"/home".to_vec(),
            b"/media/DVD".to_vec(),
        ]
    );
}

#[test]
fn drive_list_matches_internal_path_components_and_narrow_runtime_exceptions() {
    let input = b"1 0 8:1 / /bootstrap rw - ext4 /dev/sda1 rw\n\
2 0 8:2 / /snapshots rw - ext4 /dev/sda2 rw\n\
3 0 8:3 / /runner rw - ext4 /dev/sda3 rw\n\
4 0 8:4 / /var/lib/docker-data rw - ext4 /dev/sda4 rw\n\
5 0 8:5 / /run/mediator rw - ext4 /dev/sda5 rw\n\
6 0 8:6 / /run/user/1000/gvfs-other rw - ext4 /dev/sda6 rw\n\
7 0 8:7 / /run/user/not-a-uid/gvfs rw - ext4 /dev/sda7 rw\n\
8 0 0:1 / /run/user/1000/gvfs/share rw - fuse.sshfs remote rw\n";
    assert_eq!(
        paths(input),
        [
            b"/bootstrap".to_vec(),
            b"/snapshots".to_vec(),
            b"/runner".to_vec(),
            b"/var/lib/docker-data".to_vec(),
            b"/run/user/1000/gvfs/share".to_vec(),
        ]
    );
}

#[test]
fn drive_list_always_preserves_the_filesystem_root() {
    for filesystem in ["overlay", "squashfs", "tmpfs"] {
        let input = format!("1 0 0:1 / / ro - {filesystem} root ro\n");
        assert_eq!(paths(input.as_bytes()), [b"/".to_vec()]);
    }
}

#[test]
fn drive_list_omits_local_subdirectory_views_without_hiding_subvolumes() {
    let input = b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
2 1 8:1 /home/alice/project /home/alice/project rw - ext4 /dev/sda1 rw\n\
3 1 8:1 /etc/hosts /etc/hosts rw - ext4 /dev/sda1 rw\n\
4 1 8:2 /@data /data rw - btrfs /dev/sdb1 rw\n\
5 1 8:3 /data /mnt/subvolume rw - bcachefs /dev/sdc1 rw\n";
    assert_eq!(
        paths(input),
        [b"/".to_vec(), b"/data".to_vec(), b"/mnt/subvolume".to_vec()]
    );
}

#[test]
fn drive_list_leaves_out_the_gvfs_bridge() {
    // The native filesystem package lists the locations in the bridge; this list
    // leaves the bridge out without reading it.
    let input = b"1 0 8:1 / / rw - ext4 /dev/sda1 rw\n\
2 1 0:50 / /run/user/1000/gvfs rw,nosuid,nodev - fuse.gvfsd-fuse gvfsd-fuse rw,user_id=1000\n\
3 1 0:51 / /run/user/1001/gvfs rw,nosuid,nodev - fuse.gvfsd-fuse gvfsd-fuse rw,user_id=1001\n";
    assert_eq!(paths(input), [b"/".to_vec()]);
}

#[test]
fn drive_list_keeps_other_mounts_at_or_below_the_gvfs_path_as_they_are() {
    // Only GVfs's own FUSE root is the bridge: another filesystem at its path, a view
    // of one location inside it and a mount below it are ordinary drives.
    let input = b"1 0 8:1 / /run/user/1000/gvfs rw - ext4 /dev/sda1 rw\n\
2 1 0:50 / /run/user/1000/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw\n\
3 0 0:51 /smb-share:server=nas,share=data /run/user/1001/gvfs rw - fuse.gvfsd-fuse gvfsd-fuse rw\n\
4 0 0:52 / /run/user/1002/gvfs/share rw - fuse.sshfs remote rw\n";
    assert_eq!(
        paths(input),
        [
            b"/run/user/1000/gvfs".to_vec(),
            b"/run/user/1001/gvfs".to_vec(),
            b"/run/user/1002/gvfs/share".to_vec(),
        ]
    );
}

/// Each drive's name and raw root, in order.
type Shown = Vec<(String, Vec<u8>)>;
/// Devices as `(major, minor)`.
type Devices = Vec<(u32, u32)>;

/// The names and roots of the drive list built from `input` with filesystem labels
/// from `labels`, and the devices whose labels it looked up, in order.
fn named(input: &[u8], labels: &[((u32, u32), &str)]) -> (Shown, Devices) {
    use std::os::unix::ffi::OsStrExt;
    let mut asked = Vec::new();
    let volumes = super::volumes(input, &mut |major, minor| {
        asked.push((major, minor));
        labels
            .iter()
            .find(|(device, _)| *device == (major, minor))
            .map(|(_, label)| std::ffi::OsString::from(label))
    });
    let shown = volumes
        .into_iter()
        .map(|volume| (volume.label, volume.root.as_os_str().as_bytes().to_vec()))
        .collect();
    (shown, asked)
}

#[test]
fn drive_list_names_drives_by_udev_label_or_mount_folder_reading_each_device_once() {
    let input = b"1 0 0:21 / / rw - overlay overlay rw\n\
2 1 8:17 / /media/alice/USB rw - vfat /dev/sdb1 rw\n\
3 1 8:33 / /media/alice/USB1 rw - vfat /dev/sdc1 rw\n\
4 1 8:17 / /mnt/stick rw - vfat /dev/sdb1 rw\n\
6 1 8:49 / /mnt/data rw - xfs /dev/sdd1 rw\n\
7 1 0:50 / /mnt/camera rw - tmpfs tmpfs rw\n\
8 7 8:65 / /mnt/camera rw - vfat /dev/sde1 rw\n";
    let (shown, asked) = named(
        input,
        &[((8, 17), "USB"), ((8, 33), "USB"), ((8, 65), "CAMERA")],
    );
    assert_eq!(
        shown,
        [
            ("/".to_owned(), b"/".to_vec()),
            ("USB".to_owned(), b"/media/alice/USB".to_vec()),
            ("USB1".to_owned(), b"/media/alice/USB1".to_vec()),
            ("stick".to_owned(), b"/mnt/stick".to_vec()),
            ("data".to_owned(), b"/mnt/data".to_vec()),
            // Mounted over a tmpfs: the filesystem on top is the one shown there.
            ("CAMERA".to_owned(), b"/mnt/camera".to_vec()),
        ]
    );
    // Anonymous devices (major 0) have no udev record; a shared device is read once.
    assert_eq!(asked, [(8, 17), (8, 33), (8, 49), (8, 65)]);
}

#[test]
fn drive_list_adds_the_root_and_keeps_hex_identities_of_raw_paths() {
    let input = b"1 0 8:1 / /mnt/raw\\377 rw - ext4 /dev/sda1 rw\n\
2 0 8:1 / /media/My\\040USB rw - ext4 /dev/sda1 rw\n\
3 0 x:y / /mnt/odd rw - ext4 /dev/sdb1 rw\n";
    let (shown, asked) = named(input, &[((8, 1), "Data")]);
    assert_eq!(
        shown,
        [
            ("/".to_owned(), b"/".to_vec()),
            ("raw\\xFF".to_owned(), b"/mnt/raw\xff".to_vec()),
            ("My USB".to_owned(), b"/media/My USB".to_vec()),
            ("odd".to_owned(), b"/mnt/odd".to_vec()),
        ]
    );
    // One device at two paths gives one label twice: the folders tell them apart.
    assert_eq!(asked, [(8, 1)]);
    let ids: Vec<String> = super::volumes(input, &mut |_, _| None)
        .into_iter()
        .map(|volume| volume.id)
        .collect();
    assert_eq!(
        ids,
        [
            "mount:2f",
            "mount:2f6d6e742f726177ff",
            "mount:2f6d656469612f4d7920555342",
            "mount:2f6d6e742f6f6464",
        ]
    );
}
