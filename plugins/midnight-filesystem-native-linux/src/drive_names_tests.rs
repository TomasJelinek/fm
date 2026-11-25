use super::{Mount, names};

/// The names of mounts given as raw paths and optional raw filesystem labels.
fn shown(mounts: &[(&[u8], Option<&[u8]>)]) -> Vec<String> {
    let mounts: Vec<_> = mounts
        .iter()
        .map(|&(path, label)| Mount { path, label })
        .collect();
    names(&mounts)
}

#[test]
fn a_drive_shows_its_label_or_its_mount_folder_and_the_root_stays_a_slash() {
    assert_eq!(
        shown(&[
            (b"/", Some(b"system")),
            (b"/media/alice/USB", Some(b"Backup")),
            (b"/mnt/data", None),
            (b"/home", None),
        ]),
        ["/", "Backup", "data", "home"]
    );
}

#[test]
fn a_gvfs_location_shows_its_name_shortened_like_any_label() {
    // The native filesystem package names each location in the GVfs bridge.
    assert_eq!(
        shown(&[
            (
                b"/run/user/1000/gvfs/smb-share:server=nas,share=data",
                Some(b"data on nas")
            ),
            (
                b"/run/user/1000/gvfs/smb-share:server=fileserver,share=Documents",
                Some(b"Documents on fileserver")
            ),
            (b"/run/user/1000/gvfs/share", None),
        ]),
        ["data on nas", "Document…eserver", "share"]
    );
}

#[test]
fn blank_labels_count_as_none_and_labels_lose_surrounding_spaces() {
    assert_eq!(
        shown(&[
            (b"/mnt/data", Some(b"   ")),
            (b"/mnt/empty", Some(b"")),
            (b"/mnt/stick", Some(b" KINGSTON ")),
        ]),
        ["data", "empty", "KINGSTON"]
    );
}

#[test]
fn equal_names_take_folders_from_the_path_until_they_differ() {
    assert_eq!(
        shown(&[
            (b"/media/alice/USB", Some(b"USB")),
            (b"/media/alice/USB1", Some(b"USB")),
            (b"/mnt/backup/data", None),
            (b"/srv/data", None),
            (b"/data", None),
        ]),
        ["USB", "USB1", "backup/data", "srv/data", "data"]
    );
}

#[test]
fn a_folder_taken_to_tell_two_labels_apart_never_collides_with_a_third_drive() {
    // Both sticks are labelled USB; the second one's folder, USB1, is a third drive's
    // name. Each round, every drive of a shared name takes one more step: the third
    // drive's next step is its folder again, the second stick's the folder above.
    assert_eq!(
        shown(&[
            (b"/media/alice/USB", Some(b"USB")),
            (b"/media/alice/USB1", Some(b"USB")),
            (b"/mnt/USB1", None),
        ]),
        ["USB", "alice/USB1", "USB1"]
    );
}

#[test]
fn long_names_keep_their_start_and_end_around_one_ellipsis() {
    assert_eq!(
        shown(&[
            (b"/media/alice/3f2a9c1e-5b7d-4e8f-9a0b-1c2d3e4f5a6b", None),
            (b"/mnt/x", Some(b"My Backup Drive 2023")),
            (b"/mnt/y", Some(b"My Backup Drive 2024")),
            (b"/mnt/z", Some(b"Exactly16Columns")),
        ]),
        [
            "3f2a9c1e…e4f5a6b",
            "My Backu…ve 2023",
            "My Backu…ve 2024",
            "Exactly16Columns"
        ]
    );
}

#[test]
fn wide_characters_take_two_columns_and_marks_stay_with_their_letter() {
    assert_eq!(
        shown(&[
            ("/mnt/文档文档文档文档文档".as_bytes(), None),
            ("/mnt/aaaaaaae\u{301}bbbbbbbbbbbbbbb".as_bytes(), None),
            ("/mnt/cccccccccccccccccccce\u{301}dddddd".as_bytes(), None),
            (
                "/mnt/zwj".as_bytes(),
                Some("Team 👩\u{200d}💻 Shared Disk".as_bytes())
            ),
        ]),
        [
            "文档文档…档文档",
            "aaaaaaae\u{301}…bbbbbbb",
            "cccccccc…e\u{301}dddddd",
            "Team 👩\u{200d}💻…ed Disk",
        ]
    );
}

#[test]
fn shortened_names_that_collide_fall_back_to_folders_then_to_the_whole_path() {
    assert_eq!(
        shown(&[
            (b"/mnt/p", Some(b"AAAAAAAAAAxAAAAAAAAAA")),
            (b"/mnt/q", Some(b"AAAAAAAAAAyAAAAAAAAAA")),
            (b"/one/aaaaaaaaaaZaaaaaaaaaa", None),
            (b"/two/aaaaaaaaaaYaaaaaaaaaa", None),
            (b"/aaaaaaaaaaaWaaaaaaaaaaa/bbbbbbbbbbbbbbbbbbbbbbbbb", None),
            (b"/aaaaaaaaaaaVaaaaaaaaaaa/bbbbbbbbbbbbbbbbbbbbbbbbb", None),
        ]),
        [
            "p",
            "q",
            "one/aaaa…aaaaaaa",
            "two/aaaa…aaaaaaa",
            "aaaaaaaaaaaWaaaaaaaaaaa/bbbbbbbbbbbbbbbbbbbbbbbbb",
            "aaaaaaaaaaaVaaaaaaaaaaa/bbbbbbbbbbbbbbbbbbbbbbbbb",
        ]
    );
}

#[test]
fn control_characters_and_bytes_that_are_not_utf8_show_escaped() {
    assert_eq!(
        shown(&[
            (b"/mnt/a\tb", None),
            (b"/mnt/raw\xff", None),
            (b"/mnt/raw\xfe", None),
            (b"/mnt/x", Some(b"line\nbreak")),
        ]),
        ["a\\tb", "raw\\xFF", "raw\\xFE", "line\\nbreak"]
    );
}

#[test]
fn many_equal_names_grow_only_as_far_as_needed_and_keep_their_order() {
    let mut paths: Vec<Vec<u8>> = Vec::new();
    for a in 0..30 {
        for b in 0..10 {
            paths.push(format!("/mnt/{a}/{b}/data").into_bytes());
        }
    }
    paths.push(b"/srv/music".to_vec());
    let mounts: Vec<_> = paths
        .iter()
        .map(|path| Mount { path, label: None })
        .collect();
    let shown = names(&mounts);
    assert_eq!(shown.len(), 301);
    let mut index = 0;
    for a in 0..30 {
        for b in 0..10 {
            assert_eq!(shown[index], format!("{a}/{b}/data"));
            index += 1;
        }
    }
    assert_eq!(shown[300], "music");
}
