use super::*;
use std::ffi::OsString;

fn details() -> VolumeDetails {
    VolumeDetails {
        label: Ok(Some(OsString::from("文档 · Žluťoučký 👩🏽‍💻"))),
        space: Ok(SpaceInfo {
            available: 42,
            total: 100,
        }),
    }
}

#[test]
fn stale_volume_details_are_ignored_without_navigation_or_ui_errors() {
    let root = test_root();
    let mut presenter = Presenter::new(root.clone(), root.clone());
    presenter.apply(Event::VolumesUpdated(vec![Volume {
        id: "visible".into(),
        label: "Disk".into(),
        root: root.clone(),
    }]));
    drop(presenter.take_retired());
    presenter.apply(Event::VolumeDetailsUpdated {
        volume_id: "removed".into(),
        root: root.join("removed"),
        details: details(),
    });
    assert_eq!(presenter.panes[0].path, root);
    assert!(!presenter.has_errors());
    assert_eq!(
        presenter.take_retired().len(),
        1,
        "the stale details are retired, not dropped on the UI thread"
    );
}

#[test]
fn volume_cache_accepts_only_matching_id_and_native_root() {
    let root = test_root();
    let mut presenter = Presenter::new(root.clone(), root.clone());
    presenter.apply(Event::VolumesUpdated(vec![Volume {
        id: "visible".into(),
        label: "Disk".into(),
        root: root.clone(),
    }]));
    presenter.apply(Event::VolumeDetailsUpdated {
        volume_id: "visible".into(),
        root: root.join("wrong"),
        details: details(),
    });
    assert!(presenter.volume_details.is_empty());
    presenter.apply(Event::VolumeDetailsUpdated {
        volume_id: "visible".into(),
        root: root.clone(),
        details: details(),
    });
    let (cached_root, cached) = presenter.volume_details.get("visible").unwrap();
    assert_eq!(cached_root, &root);
    assert_eq!(
        cached.label.as_ref().unwrap().as_ref().unwrap(),
        "文档 · Žluťoučký 👩🏽‍💻"
    );
    assert_eq!(cached.space.as_ref().unwrap().available, 42);
}

#[test]
fn volume_discovery_preserves_only_unchanged_cache_and_retires_removed_values() {
    let root = test_root();
    let mut presenter = Presenter::new(root.clone(), root.clone());
    let volume = Volume {
        id: "visible".into(),
        label: "Disk".into(),
        root: root.clone(),
    };
    presenter.apply(Event::VolumesUpdated(vec![volume.clone()]));
    presenter.apply(Event::VolumeDetailsUpdated {
        volume_id: "visible".into(),
        root: root.clone(),
        details: details(),
    });
    presenter.apply(Event::VolumesUpdated(vec![volume.clone()]));
    assert_eq!(presenter.volume_details.len(), 1);
    let mut replacement = volume.clone();
    replacement.root = root.join("replacement");
    // Identical presenters that differ only in the cached details retire exactly
    // one more value for the same discovery: the removed details.
    let retirements = |cached: bool| {
        let mut presenter = Presenter::new(root.clone(), root.clone());
        presenter.apply(Event::VolumesUpdated(vec![volume.clone()]));
        if cached {
            presenter.apply(Event::VolumeDetailsUpdated {
                volume_id: "visible".into(),
                root: root.clone(),
                details: details(),
            });
        }
        drop(presenter.take_retired());
        presenter.apply(Event::VolumesUpdated(vec![replacement.clone()]));
        assert!(presenter.volume_details.is_empty());
        presenter.take_retired().len()
    };
    assert_eq!(
        retirements(true),
        retirements(false) + 1,
        "the removed volume's details are retired, not dropped on the UI thread"
    );
    presenter.apply(Event::VolumesUpdated(vec![replacement]));
    assert!(presenter.volume_details.is_empty());
    drop(presenter.take_retired());
    presenter.apply(Event::VolumeDetailsUpdated {
        volume_id: "visible".into(),
        root,
        details: details(),
    });
    assert!(presenter.volume_details.is_empty());
    assert_eq!(
        presenter.take_retired().len(),
        1,
        "details for the replaced root are retired"
    );
}

#[test]
fn failed_volume_details_do_not_become_pane_errors_or_fake_capacity() {
    let root = test_root();
    let mut presenter = Presenter::new(root.clone(), root.clone());
    presenter.apply(Event::VolumesUpdated(vec![Volume {
        id: "visible".into(),
        label: "Disk".into(),
        root: root.clone(),
    }]));
    presenter.apply(Event::VolumeDetailsUpdated {
        volume_id: "visible".into(),
        root: root.clone(),
        details: VolumeDetails {
            label: Ok(None),
            space: Err(FsError::from_io(
                "space",
                &root,
                io::Error::other("offline"),
            )),
        },
    });
    assert!(presenter.volume_details["visible"].1.space.is_err());
    assert!(!presenter.has_errors());
    assert!(presenter.panes[0].space.is_none());
    presenter.retire_all();
    assert!(presenter.volume_details.is_empty());
}

fn volume(name: &str) -> Volume {
    Volume {
        id: name.into(),
        label: name.into(),
        root: name.into(),
    }
}

fn refreshed_paths(p: &mut Presenter, volumes: Vec<Volume>) -> Vec<(PaneId, PathBuf)> {
    let sent = RefCell::new(Vec::new());
    p.apply_with_sender(Event::VolumesUpdated(volumes), &|command| {
        if let Command::Refresh { pane, path, .. } = command {
            sent.borrow_mut().push((pane, path));
        }
        true
    });
    sent.into_inner()
}

#[test]
fn a_removed_drive_rereads_only_the_panels_on_it_at_once() {
    let mut p = Presenter::new("usb".into(), "disk".into());
    for (pane, path) in [(PaneId::Left, "usb/docs"), (PaneId::Right, "disk/usb")] {
        let navigation = begin(&mut p, pane, path);
        finish(
            &mut p,
            pane,
            navigation,
            vec![entry(1, &format!("{path}/a"))],
        );
    }
    // The first list removes nothing; a new drive leaves both panels alone.
    assert!(refreshed_paths(&mut p, vec![volume("disk"), volume("usb")]).is_empty());
    assert!(
        refreshed_paths(&mut p, vec![volume("disk"), volume("usb"), volume("card")]).is_empty()
    );
    // The left panel's drive is gone. The right panel's folder only has the
    // removed drive's name and stays.
    assert_eq!(
        refreshed_paths(&mut p, vec![volume("disk"), volume("card")]),
        [(PaneId::Left, PathBuf::from("usb/docs"))]
    );
    assert!(p.panes[0].loading);
    assert!(!p.panes[1].loading);
    // The re-read is in flight; the same list again sends nothing more.
    assert!(refreshed_paths(&mut p, vec![volume("disk"), volume("card")]).is_empty());
}

#[test]
fn a_panel_on_a_remaining_inner_volume_is_not_reread() {
    let mut p = Presenter::new("disk/mnt/usb".into(), "disk".into());
    let navigation = begin(&mut p, PaneId::Left, "disk/mnt/usb/x");
    finish(&mut p, PaneId::Left, navigation, Vec::new());
    assert!(
        refreshed_paths(
            &mut p,
            vec![volume("disk"), volume("disk/mnt/usb"), volume("other")]
        )
        .is_empty()
    );
    // Removing a drive that only contains the inner volume's mount point does not
    // touch a panel on the inner volume.
    assert!(refreshed_paths(&mut p, vec![volume("disk/mnt/usb"), volume("other")]).is_empty());
    assert_eq!(
        refreshed_paths(&mut p, vec![volume("other")]),
        [(PaneId::Left, PathBuf::from("disk/mnt/usb/x"))]
    );
}
