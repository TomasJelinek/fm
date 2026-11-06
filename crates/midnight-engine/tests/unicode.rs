mod support;

use midnight_core::{Command, EntryKind, PaneId};
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{fs, sync::Arc};
use support::navigate;
use tokio::sync::mpsc;

const DIRECTORIES: [&str; 6] = [
    "Příliš žluťoučký kůň_ěščřžýáíéůú",
    "中文目录_資料",
    "日本語のフォルダー_ひらがな",
    "العربية_مجلد",
    "עברית_תיקייה",
    "emoji_🧪🚀😀_👩‍💻",
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn real_unicode_directories_navigate_from_returned_paths_with_exact_metadata() {
    let root = tempfile::Builder::new()
        .prefix("Midnight Explorer-Žluťoučký_中文_🧪-")
        .tempdir()
        .unwrap();
    for directory in DIRECTORIES {
        let path = root.path().join(directory);
        fs::create_dir(&path).unwrap();
        fs::write(
            path.join("vnořený_中文_日本語_العربية_עברית_🧪.txt"),
            directory.as_bytes(),
        )
        .unwrap();
    }
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(8);
    let task =
        tokio::spawn(Engine::new(Arc::new(LocalFileSystem::new(None)), sender).run(receiver));
    let entries = navigate(&commands, &mut events, PaneId::Left, 1, root.path()).await;
    assert_eq!(entries.len(), DIRECTORIES.len());
    for (offset, name) in DIRECTORIES.iter().enumerate() {
        let directory = entries.iter().find(|entry| entry.name == *name).unwrap();
        assert_eq!(directory.display_name, *name);
        assert_eq!(directory.kind, EntryKind::Directory);
        assert_eq!(directory.path, root.path().join(name));
        let children = navigate(
            &commands,
            &mut events,
            PaneId::Left,
            offset as u64 + 2,
            &directory.path,
        )
        .await;
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].name, "vnořený_中文_日本語_العربية_עברית_🧪.txt");
        assert_eq!(children[0].metadata.size, Some(name.len() as u64));
        assert_eq!(fs::read(&children[0].path).unwrap(), name.as_bytes());
    }
    // Navigation generations are independent in each pane.
    let right = navigate(&commands, &mut events, PaneId::Right, 1, root.path()).await;
    assert_eq!(right.len(), DIRECTORIES.len());
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn normalization_equivalent_directory_names_keep_distinct_engine_identities() {
    let root = tempfile::tempdir().unwrap();
    let fixtures = [
        ("café", b"composed".as_slice()),
        ("cafe\u{301}", b"decomposed".as_slice()),
    ];
    for (name, contents) in fixtures {
        let directory = root.path().join(name);
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("ověření_🧪.txt"), contents).unwrap();
    }
    let (commands, receiver) = mpsc::channel(8);
    let (sender, mut events) = mpsc::channel(8);
    let task =
        tokio::spawn(Engine::new(Arc::new(LocalFileSystem::new(None)), sender).run(receiver));
    let entries = navigate(&commands, &mut events, PaneId::Left, 1, root.path()).await;
    assert_eq!(entries.len(), 2);
    assert_ne!(entries[0].id, entries[1].id);
    for (offset, (name, contents)) in fixtures.iter().enumerate() {
        let directory = entries.iter().find(|entry| entry.name == *name).unwrap();
        assert_eq!(directory.display_name, *name);
        assert_eq!(directory.path, root.path().join(name));
        let children = navigate(
            &commands,
            &mut events,
            PaneId::Left,
            offset as u64 + 2,
            &directory.path,
        )
        .await;
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].metadata.size, Some(contents.len() as u64));
        assert_eq!(fs::read(&children[0].path).unwrap(), *contents);
    }
    commands.send(Command::Shutdown).await.unwrap();
    drop(events);
    task.await.unwrap();
}
