//! A navigation to a path that names a file: before its failure, the Engine says which
//! folder lists that file (`Event::ListingNotAFolder`), as the real native provider
//! sees it. Folders, missing paths and separator-terminated paths are as before.
use midnight_core::{Command, Event, NavigationId, PaneId, SortSpec};
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{
    io::ErrorKind,
    path::{MAIN_SEPARATOR_STR, Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::sync::mpsc;

fn navigate(generation: u64, path: PathBuf) -> Command {
    Command::Navigate {
        pane: PaneId::Left,
        navigation: NavigationId(generation),
        path,
        sort: SortSpec::default(),
    }
}

/// The events of one navigation up to its terminal event; other events are skipped.
async fn outcome(events: &mut mpsc::Receiver<Event>, navigation: u64) -> Vec<Event> {
    let mut seen = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(30), events.recv())
            .await
            .expect("the navigation ends")
            .expect("engine output stays connected");
        match &event {
            Event::ListingNotAFolder { navigation: id, .. } if id.0 == navigation => {
                seen.push(event);
            }
            Event::ListingFallback { navigation: id, .. }
            | Event::ListingFailed { navigation: id, .. }
            | Event::ListingFinished { navigation: id, .. }
                if id.0 == navigation =>
            {
                seen.push(event);
                return seen;
            }
            _ => {}
        }
    }
}

/// A hidden file: a dot name on Linux, created with the hidden attribute on Windows.
fn write_hidden(folder: &Path) -> PathBuf {
    let path = folder.join(".skryté 文档.txt");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(windows)]
    std::os::windows::fs::OpenOptionsExt::attributes(&mut options, 0x2);
    options.open(&path).unwrap();
    path
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_path_that_names_a_file_names_its_folder_before_the_failure() {
    let fixture = tempfile::tempdir().unwrap();
    let folder = fixture.path().join("Příliš 中文 e\u{301}");
    std::fs::create_dir(&folder).unwrap();
    let file = folder.join("资料 🧪.txt");
    std::fs::write(&file, b"named").unwrap();
    let hidden = write_hidden(&folder);
    let filesystem = Arc::new(LocalFileSystem::new(None));
    let (sender, mut events) = mpsc::channel(16);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        filesystem.clone(),
        sender,
    );
    let (commands, receiver) = engine.command_channel(8).unwrap();
    engine
        .configure_navigation_fallback()
        .unwrap()
        .install(filesystem);
    let task = tokio::spawn(engine.run(receiver));

    for (navigation, named, expected_hidden) in [(1, &file, false), (2, &hidden, true)] {
        commands
            .send(navigate(navigation, named.clone()))
            .await
            .unwrap();
        let seen = outcome(&mut events, navigation).await;
        assert!(
            matches!(
                &seen[..],
                [
                    Event::ListingNotAFolder { parent, entry, hidden, .. },
                    Event::ListingFallback { path, error, .. },
                ] if parent == &folder && entry == named && *hidden == expected_hidden
                    && path == &folder && error.kind == ErrorKind::NotADirectory
            ),
            "{seen:?}"
        );
    }

    // A missing file, and a file's path that ends with a separator (a folder's path),
    // fail as before without naming an entry.
    let mut separated = file.clone().into_os_string();
    separated.push(MAIN_SEPARATOR_STR);
    for (navigation, path) in [(3, folder.join("chybí.txt")), (4, PathBuf::from(separated))] {
        commands.send(navigate(navigation, path)).await.unwrap();
        let seen = outcome(&mut events, navigation).await;
        assert!(
            matches!(&seen[..], [Event::ListingFallback { path, .. }] if path == &folder),
            "{seen:?}"
        );
    }

    commands.send(navigate(5, folder.clone())).await.unwrap();
    let seen = outcome(&mut events, 5).await;
    assert!(
        matches!(
            &seen[..],
            [Event::ListingFinished { entries, .. }]
                if entries.iter().any(|entry| entry.path == file)
        ),
        "{seen:?}"
    );
    commands.send(Command::Shutdown).await.unwrap();
    task.await.unwrap();
}
