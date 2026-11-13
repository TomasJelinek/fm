use super::paths;

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
            b"/run/user/1000/gvfs".to_vec(),
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
