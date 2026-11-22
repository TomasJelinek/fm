//! Windows link stages: a junction is recreated without privilege, a directory symbolic
//! link as a symbolic link where this process may create one and otherwise as a junction
//! with the reason, a file symbolic link only where symbolic links may be created.
use super::*;
use crate::test_fixtures::{build_temp, create_junction};
use midnight_core::TransferService;
use std::fs;

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

fn destination(path: &Path) -> Arc<dyn TransferDirectory> {
    crate::WindowsFileSystem::new(None)
        .prepare_transfer_destination(path, &Cancellation::default())
        .unwrap()
}

/// Stages `representation` (as `plan` planned it) in `folder`, publishes it as `name` and
/// checks every step, as the engine's link copy does.
fn publish(folder: &Arc<dyn TransferDirectory>, plan: &TransferLinkPlan, name: &str) {
    let cancel = Cancellation::default();
    let mut stage = folder
        .stage_link(&plan.representation, &cancel)
        .map_err(|failure| failure.error)
        .unwrap();
    stage.sync(&cancel).unwrap();
    assert_eq!(
        stage.link_representation(&cancel).unwrap(),
        plan.representation
    );
    let identity = stage
        .revalidate_stage(&plan.representation, &cancel)
        .unwrap();
    let published = stage.publish_no_replace(OsStr::new(name), &cancel).unwrap();
    assert!(
        matches!(&published, TransferPublication::Published(path) if *path == folder.path().join(name))
    );
    assert_eq!(
        stage
            .revalidate_publication(&plan.representation, &cancel)
            .unwrap(),
        identity
    );
    stage.cleanup().unwrap();
}

#[test]
fn a_junction_is_recreated_exactly_without_privilege() {
    let temp = build_temp("link-stage");
    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    fs::write(target.join("sentinel"), b"data").unwrap();
    let source = temp.path().join("source-junction");
    fs::create_dir(&source).unwrap();
    create_junction(&source, &target).unwrap();
    let held = source_links::open_link(
        &native::open_parent(temp.path()).unwrap(),
        OsStr::new("source-junction"),
        true,
    )
    .unwrap();
    let representation = source_links::read_link(&held, &source).unwrap();
    assert_eq!(representation.kind(), TransferLinkKind::WindowsJunction);
    let out = temp.path().join("out");
    fs::create_dir(&out).unwrap();
    let folder = destination(&out);
    let plan = folder
        .plan_link(&representation, &Cancellation::default())
        .unwrap();
    assert_eq!(
        plan.representation, representation,
        "a junction stays a junction"
    );
    assert_eq!(plan.note, None);
    publish(&folder, &plan, "copied");
    // The recreated junction leads to the same folder; nothing of the target was copied.
    assert_eq!(fs::read(out.join("copied/sentinel")).unwrap(), b"data");
    let names: Vec<_> = fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        names,
        [OsString::from("copied")],
        "no private stage remains"
    );
    fs::remove_dir(out.join("copied")).unwrap();
    assert_eq!(fs::read(target.join("sentinel")).unwrap(), b"data");
}

#[test]
fn a_directory_symbolic_link_is_recreated_or_becomes_a_junction_with_the_reason() {
    let temp = build_temp("link-stage-symlink");
    let out = temp.path().join("out");
    fs::create_dir_all(out.join("sub")).unwrap();
    let folder = destination(&out);
    let absolute = format!(r"\??\{}", temp.path().join("target").display());
    let representation = TransferLinkRepresentation::windows_symbolic_link(
        true,
        0,
        &wide(&absolute),
        &wide(&absolute[4..]),
    )
    .unwrap();
    let plan = folder
        .plan_link(&representation, &Cancellation::default())
        .unwrap();
    if symbolic_links_allowed() {
        assert_eq!(
            plan.representation.kind(),
            TransferLinkKind::WindowsDirectorySymbolicLink
        );
        assert_eq!(plan.note, None);
    } else {
        assert_eq!(
            plan.representation.kind(),
            TransferLinkKind::WindowsJunction
        );
        assert_eq!(
            plan.representation.windows_names().unwrap().0,
            wide(&absolute).as_slice()
        );
        let note = plan.note.clone().unwrap();
        assert!(note.starts_with("was created as a junction to "), "{note}");
        assert!(
            note.contains("Developer Mode or administrator rights"),
            "{note}"
        );
    }
    publish(&folder, &plan, "link");
    fs::remove_dir(out.join("link")).unwrap();
    // A relative target stays relative where symbolic links may be created; as a junction
    // it is resolved from the destination folder, never above its drive.
    let relative = TransferLinkRepresentation::windows_symbolic_link(
        true,
        1,
        &wide(r"..\sub"),
        &wide(r"..\sub"),
    )
    .unwrap();
    let plan = folder
        .plan_link(&relative, &Cancellation::default())
        .unwrap();
    if symbolic_links_allowed() {
        assert_eq!(plan.representation.meaningful_flags(), 1);
        assert_eq!(
            plan.representation.windows_names().unwrap().0,
            wide(r"..\sub")
        );
    } else {
        let expected = format!(r"\??\{}", temp.path().join("sub").display());
        assert_eq!(
            plan.representation.windows_names().unwrap().0,
            wide(&expected)
        );
        assert!(
            plan.note
                .unwrap()
                .contains("relative target became this absolute one")
        );
    }
}

#[test]
fn a_job_reads_symbolic_link_rights_once_and_each_volume_once() {
    // A job's folders: the one it starts from, folders it creates below it and one it
    // merges into, with many links planned in each. Every read of the system's link facts
    // on this thread is counted, whichever folder made it.
    let reads = || LINK_FACT_READS.with(std::cell::Cell::get);
    LINK_FACT_READS.with(|reads| reads.set((0, 0)));
    let temp = build_temp("link-stage-facts");
    let out = temp.path().join("out");
    fs::create_dir_all(out.join("existing")).unwrap();
    let root = destination(&out);
    let cancel = Cancellation::default();
    let created = root.create_directory(OsStr::new("a"), &cancel).unwrap();
    let nested = created.create_directory(OsStr::new("b"), &cancel).unwrap();
    let existing = root
        .prepare_target(OsStr::new("existing"), &cancel)
        .unwrap()
        .unwrap();
    let merged = root
        .open_merge_directory(OsStr::new("existing"), existing.entry(), &cancel)
        .unwrap();
    let absolute = format!(r"\??\{}", temp.path().join("target").display());
    let junction =
        TransferLinkRepresentation::windows_junction(&wide(&absolute), &wide(&absolute[4..]))
            .unwrap();
    let symbolic =
        TransferLinkRepresentation::windows_symbolic_link(true, 1, &wide(r"..\x"), &wide(r"..\x"))
            .unwrap();
    for folder in [&root, &created, &nested, &merged] {
        for _ in 0..25 {
            folder.plan_link(&junction, &cancel).unwrap();
            folder.admit_link(&symbolic, &cancel).unwrap();
        }
    }
    assert_eq!(
        reads(),
        (1, 1),
        "200 links in four folders of one job on one volume: each fact was read once"
    );
    // Another job reads them anew.
    destination(&out).plan_link(&symbolic, &cancel).unwrap();
    assert_eq!(reads(), (2, 2));
}

#[test]
fn a_file_symbolic_link_or_a_network_target_needs_symbolic_link_rights() {
    let temp = build_temp("link-stage-file");
    let folder = destination(temp.path());
    for (directory, text) in [
        (false, r"\??\C:\Windows\win.ini"),
        (true, r"\??\UNC\server\share"),
    ] {
        let representation =
            TransferLinkRepresentation::windows_symbolic_link(directory, 0, &wide(text), &[])
                .unwrap();
        let planned = folder.plan_link(&representation, &Cancellation::default());
        if symbolic_links_allowed() {
            let plan = planned.unwrap();
            assert_eq!(plan.note, None);
            // `CreateSymbolicLinkW` names the target by its Win32 path.
            let print = plan.representation.windows_names().unwrap().1.to_vec();
            assert!(!print.starts_with(&wide(r"\??\")), "{print:?}");
        } else {
            let error = planned.unwrap_err();
            assert_eq!(error.kind, io::ErrorKind::Unsupported);
            assert!(
                error
                    .message
                    .contains("Developer Mode or administrator rights")
            );
            assert!(!error.message.contains("os error"), "{}", error.message);
        }
    }
}

#[test]
fn an_unpublished_stage_is_removed_and_an_existing_name_is_never_replaced() {
    let temp = build_temp("link-stage-cleanup");
    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    let out = temp.path().join("out");
    fs::create_dir(&out).unwrap();
    fs::write(out.join("taken"), b"keep").unwrap();
    let folder = destination(&out);
    let substitute = wide(&format!(r"\??\{}", target.display()));
    let representation =
        TransferLinkRepresentation::windows_junction(&substitute, &substitute[4..]).unwrap();
    let cancel = Cancellation::default();
    let mut stage = folder
        .stage_link(&representation, &cancel)
        .map_err(|failure| failure.error)
        .unwrap();
    let error = stage
        .publish_no_replace(OsStr::new("taken"), &cancel)
        .unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(out.join("taken")).unwrap(), b"keep");
    stage.cleanup().unwrap();
    let names: Vec<_> = fs::read_dir(&out)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(
        names,
        [OsString::from("taken")],
        "the stage was removed, not its target"
    );
    assert!(target.is_dir());
}

#[test]
fn a_verbatim_link_path_uses_backslashes_only() {
    // CreateSymbolicLinkW gets the verbatim spelling, which the system does not parse: a `/`
    // left in it (a destination typed with `/`, a TEMP set with `/`) is an invalid name.
    let text = |wide: Vec<u16>| String::from_utf16(&wide[..wide.len() - 1]).unwrap();
    let mixed = verbatim(Path::new(r"C:\work/tree\links/.me-0123")).unwrap();
    assert_eq!(mixed.last(), Some(&0));
    assert_eq!(text(mixed), r"\\?\C:\work\tree\links\.me-0123");
    assert_eq!(
        text(verbatim(Path::new(r"\\?\C:\work\tree")).unwrap()),
        r"\\?\C:\work\tree"
    );
    assert!(verbatim(Path::new("relative/name")).is_err());
}

#[test]
fn relative_targets_resolve_name_by_name_and_never_above_the_drive() {
    let base = Path::new(r"D:\dest\sub");
    let text = |relative: &str| {
        resolve_relative(base, &wide(relative)).map(|units| String::from_utf16_lossy(&units))
    };
    assert_eq!(text(r"..\x").as_deref(), Some(r"D:\dest\x"));
    assert_eq!(text(r".\y\..\z").as_deref(), Some(r"D:\dest\sub\z"));
    assert_eq!(text("../a/b").as_deref(), Some(r"D:\dest\a\b"));
    assert_eq!(text(r"..\..\..\x"), None);
    assert_eq!(
        resolve_relative(Path::new(r"\\server\share"), &wide("x")),
        None
    );
}

#[test]
fn a_symbolic_link_is_created_with_its_win32_target() {
    let target = |text: &str, flags| {
        let representation =
            TransferLinkRepresentation::windows_symbolic_link(true, flags, &wide(text), &[])
                .unwrap();
        symbolic_target(&representation, Path::new(r"C:\d"))
            .map(|units| String::from_utf16_lossy(&units))
    };
    assert_eq!(target(r"\??\C:\x", 0).unwrap(), r"C:\x");
    assert_eq!(
        target(r"\??\UNC\server\share\x", 0).unwrap(),
        r"\\server\share\x"
    );
    assert_eq!(target(r"..\x", 1).unwrap(), r"..\x");
    assert!(target(r"\Device\x", 0).is_err());
}
