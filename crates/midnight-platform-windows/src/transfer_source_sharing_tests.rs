//! A Copy job prepares its selected items before admission grants it, while granted jobs
//! still change them, and then waits. Windows refuses every rename into a folder while
//! any handle on it denies write sharing (another job's publication included), and a
//! rename or removal of an entry while a handle denies delete sharing. So a prepared root
//! holds no handle on its item until the job first uses it, and the folders a transfer
//! lists share writing; only a file being read refuses writers, renames and removal.
use super::*;
use crate::test_fixtures::{build_temp, create_junction, rerun_on_fresh_fixture_hold};
use midnight_core::TransferService;
use std::{fs, io::Write, os::windows::fs::OpenOptionsExt};

const SHARING_VIOLATION: i32 = 32;

/// Copies a file named `name` into `folder` as another Copy job publishes it: a private
/// stage, then a no-replace rename into the folder relative to its held handle.
fn publish(folder: &Path, name: &str) -> Result<(), FsError> {
    let cancel = Cancellation::default();
    let destination =
        crate::WindowsFileSystem::new(None).prepare_transfer_destination(folder, &cancel)?;
    let mut stage = destination.stage_file(&cancel)?;
    stage
        .write_all(name.as_bytes())
        .map_err(|e| error(folder, e))?;
    stage.sync(&cancel)?;
    stage.publish_no_replace(OsStr::new(name), &cancel)?;
    Ok(())
}

/// Opens `path` for writing as another program does, sharing everything.
fn open_writer(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .open(path)
}

#[test]
fn a_prepared_folder_refuses_nothing_until_the_copy_lists_it() {
    let temp = build_temp("prepared-root-");
    let folder = temp.path().join("output 文档");
    fs::create_dir(&folder).unwrap();
    let cancel = Cancellation::default();
    let service = crate::WindowsFileSystem::new(None);
    // Everything preparation does before the grant: the source, its namespace pin and
    // a revalidation.
    let mut source = service
        .prepare_transfer_source_with_links(
            &folder,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    let pin = source.native_namespace_scope(&cancel).unwrap();
    source.revalidate(&cancel).unwrap();
    // A granted job publishes into the folder, and another one moves it away and back.
    publish(&folder, "published.bin").unwrap();
    let moved = temp.path().join("moved");
    fs::rename(&folder, &moved).unwrap();
    fs::rename(&moved, &folder).unwrap();
    source.revalidate(&cancel).unwrap();
    // The copy lists the folder: from then on its name is kept, and other jobs can still
    // publish into it.
    let first = source
        .next_child_or_failure(&cancel)
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first.entry().item.path, folder.join("published.bin"));
    drop(first);
    let kept = fs::rename(&folder, &moved).unwrap_err();
    assert_eq!(kept.raw_os_error(), Some(SHARING_VIOLATION), "{kept:?}");
    publish(&folder, "later.bin").unwrap();
    assert!(folder.join("later.bin").is_file());
    drop(source);
    drop(pin);
}

#[test]
fn a_prepared_file_refuses_nothing_until_it_is_read() {
    // Endpoint protection may briefly hold the freshly written fixture (TEST-04).
    rerun_on_fresh_fixture_hold(|| -> io::Result<()> {
        let temp = build_temp("prepared-root-");
        let file = temp.path().join("copied.bin");
        fs::write(&file, b"original content").unwrap();
        let cancel = Cancellation::default();
        let mut source = crate::WindowsFileSystem::new(None)
            .prepare_transfer_source(&file, &cancel)
            .unwrap();
        let pin = source.native_namespace_scope(&cancel).unwrap();
        // Before the job reads it, a program may open it for writing (and write nothing),
        // and a granted job may rename it away and back.
        drop(open_writer(&file)?);
        let away = temp.path().join("away.bin");
        fs::rename(&file, &away)?;
        fs::rename(&away, &file)?;
        // Reading takes the guard: nobody writes, renames or removes the file any more.
        let mut buffer = [0; 64];
        let read = source.read_chunk(&mut buffer, &cancel).unwrap();
        assert_eq!(&buffer[..read], b"original content");
        let writer = open_writer(&file).unwrap_err();
        assert_eq!(writer.raw_os_error(), Some(SHARING_VIOLATION), "{writer:?}");
        let renamed = fs::rename(&file, &away).unwrap_err();
        assert_eq!(
            renamed.raw_os_error(),
            Some(SHARING_VIOLATION),
            "{renamed:?}"
        );
        drop(source);
        drop(pin);
        Ok(())
    })
    .unwrap();
}

#[test]
fn opening_a_prepared_file_for_reading_takes_its_guard_or_fails_as_a_read_would() {
    // Endpoint protection may briefly hold the freshly written fixture (TEST-04).
    rerun_on_fresh_fixture_hold(|| -> io::Result<()> {
        let temp = build_temp("prepared-root-");
        let file = temp.path().join("copied.bin");
        fs::write(&file, b"original content").unwrap();
        let cancel = Cancellation::default();
        let source = crate::WindowsFileSystem::new(None)
            .prepare_transfer_source(&file, &cancel)
            .unwrap();
        // Another program holds the file without sharing: the open fails as the first
        // read would, so Copy asks before it creates anything in the destination.
        let held = fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(&file)?;
        let refused = source.open_for_reading(&cancel).unwrap_err();
        assert!(refused.message.ends_with("(os error 32)"), "{refused:?}");
        drop(held);
        // Once it is released, the open takes the guard a read takes, once.
        source.open_for_reading(&cancel).unwrap();
        let writer = open_writer(&file).unwrap_err();
        assert_eq!(writer.raw_os_error(), Some(SHARING_VIOLATION), "{writer:?}");
        source.open_for_reading(&cancel).unwrap();
        Ok(())
    })
    .unwrap();
}

#[test]
fn a_prepared_root_refuses_another_object_or_file_revision_when_first_used() {
    let temp = build_temp("prepared-root-");
    let service = crate::WindowsFileSystem::new(None);
    let cancel = Cancellation::default();
    let file = temp.path().join("changed.bin");
    fs::write(&file, b"planned").unwrap();
    let mut changed = service.prepare_transfer_source(&file, &cancel).unwrap();
    fs::write(&file, b"written while the job waited").unwrap();
    assert!(changed.revalidate(&cancel).is_err());
    let error = changed.read_chunk(&mut [0; 64], &cancel).unwrap_err();
    assert!(
        error.message.contains("source content changed"),
        "{error:?}"
    );

    let folder = temp.path().join("replaced");
    fs::create_dir(&folder).unwrap();
    let mut replaced = service.prepare_transfer_source(&folder, &cancel).unwrap();
    fs::remove_dir(&folder).unwrap();
    fs::create_dir(&folder).unwrap();
    let error = replaced.next_child_or_failure(&cancel).err().unwrap();
    assert!(
        error.message.contains("source identity changed"),
        "{error:?}"
    );
}

#[test]
fn discovery_lets_another_job_publish_into_the_folders_it_lists() {
    let temp = build_temp("prepared-root-");
    let folder = temp.path().join("tree");
    let nested = folder.join("nested");
    fs::create_dir_all(&nested).unwrap();
    fs::write(nested.join("inner.bin"), b"inner").unwrap();
    let cancel = Cancellation::default();
    let source = crate::WindowsFileSystem::new(None)
        .prepare_transfer_source_with_links(
            &folder,
            CopyLinkPolicy::FollowDirectoryContents,
            &cancel,
        )
        .unwrap();
    let mut published = Vec::new();
    source
        .discover(&cancel, &mut |item| {
            // Both folders are being listed while their last item is visited.
            if item.path == nested.join("inner.bin") {
                published.push(publish(&folder, "beside.bin"));
                published.push(publish(&nested, "inside.bin"));
            }
        })
        .unwrap();
    assert_eq!(published.len(), 2);
    for result in published {
        result.unwrap();
    }
}

#[test]
fn a_followed_link_target_lets_another_job_publish_into_it() {
    let temp = build_temp("prepared-root-");
    let target = temp.path().join("target");
    fs::create_dir(&target).unwrap();
    let link = temp.path().join("link");
    fs::create_dir(&link).unwrap();
    create_junction(&link, &target).unwrap();
    let cancel = Cancellation::default();
    let mut source = crate::WindowsFileSystem::new(None)
        .prepare_transfer_source_with_links(&link, CopyLinkPolicy::FollowDirectoryContents, &cancel)
        .unwrap();
    publish(&target, "beside.bin").unwrap();
    assert!(source.next_child_or_failure(&cancel).unwrap().is_some());
    publish(&target, "later.bin").unwrap();
}

#[test]
fn a_move_source_folder_lets_another_job_publish_into_it() {
    let temp = build_temp("prepared-root-");
    let folder = temp.path().join("moved");
    fs::create_dir(&folder).unwrap();
    fs::write(folder.join("first.bin"), b"first").unwrap();
    let cancel = Cancellation::default();
    let mut prepared = crate::WindowsFileSystem::new(None)
        .prepare_move_source(&folder, &cancel)
        .unwrap();
    publish(&folder, "before.bin").unwrap();
    // A cross-volume Move lists the folder through a second handle.
    let child = prepared.source.next_move_child(&cancel).unwrap().unwrap();
    publish(&folder, "during.bin").unwrap();
    drop(child);
    drop(prepared);
}
