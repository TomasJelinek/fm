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
