#![cfg(windows)]

mod support;

use midnight_core::{Command, PaneId};
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{fs, sync::Arc};
use support::navigate;
use tokio::sync::mpsc;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn engine_navigates_windows_unicode_paths_beyond_260_utf16_units() {
    use std::os::windows::ffi::OsStrExt;

    let root = tempfile::tempdir().unwrap();
    let component = "dlouhá_中文_🧪_".repeat(5);
    assert!(component.encode_utf16().count() < 255);
    let directory = root
        .path()
        .join(&component)
        .join(&component)
        .join(&component)
        .join(&component);
    assert!(directory.as_os_str().encode_wide().count() > 260);
    if let Err(error) = fs::create_dir_all(&directory) {
        if error.kind() == std::io::ErrorKind::Unsupported || error.raw_os_error() == Some(206) {
            eprintln!("SKIP: temporary filesystem does not support this long path: {error}");
            return;
        }
        panic!("long-path fixture creation failed: {error}");
    }
    fs::write(directory.join("ověření_中文_🧪.txt"), b"long Unicode path").unwrap();
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(8);
    let task = tokio::spawn(
        Engine::new(
            Engine::lifetime().expect("engine cleanup worker"),
            Arc::new(LocalFileSystem::new(None)),
            sender,
        )
        .run(receiver),
    );
    let parents = navigate(
        &commands,
        &mut events,
        PaneId::Left,
        1,
        directory.parent().unwrap(),
    )
    .await;
    let returned = parents
        .iter()
        .find(|entry| entry.name == component.as_str())
        .unwrap();
    assert_eq!(returned.path, directory);
    let children = navigate(&commands, &mut events, PaneId::Left, 2, &returned.path).await;
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "ověření_中文_🧪.txt");
    assert_eq!(children[0].metadata.size, Some(17));
    assert_eq!(fs::read(&children[0].path).unwrap(), b"long Unicode path");
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}
