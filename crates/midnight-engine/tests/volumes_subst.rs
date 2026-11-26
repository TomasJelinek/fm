//! End-to-end drive-list latency on Windows: a `subst` drive letter of this test's own
//! temporary folder is added and removed, and the time until the engine publishes the
//! changed drive list is measured. Both tests change the user's session for a moment
//! (another program in the session sees the letter), so they run only when asked:
//! `cargo test -p midnight-engine --test integration volumes_subst -- --ignored --nocapture`.

use midnight_core::{Command, Event, Volume, VolumeService};
use midnight_engine::Engine;
use std::{
    path::{Path, PathBuf},
    process,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::mpsc;

/// A `subst` letter, removed again on drop even when an assertion failed.
struct Subst(Option<char>);

impl Subst {
    fn define(folder: &Path) -> Self {
        // Reads only the session's drive letters, never a drive.
        let used: Vec<PathBuf> = midnight_platform::LocalFileSystem::new(None)
            .list_volumes()
            .unwrap()
            .into_iter()
            .map(|volume| volume.root)
            .collect();
        let letter = (b'M'..=b'Y')
            .rev()
            .map(char::from)
            .find(|letter| !used.contains(&PathBuf::from(format!("{letter}:\\"))))
            .expect("a free drive letter");
        let status = process::Command::new("subst.exe")
            .arg(format!("{letter}:"))
            .arg(folder)
            .status()
            .unwrap();
        assert!(status.success(), "subst {letter}: failed");
        Self(Some(letter))
    }

    fn root(&self) -> PathBuf {
        PathBuf::from(format!("{}:\\", self.0.unwrap()))
    }

    fn remove(&mut self) {
        if let Some(letter) = self.0.take() {
            let _ = process::Command::new("subst.exe")
                .arg(format!("{letter}:"))
                .arg("/d")
                .status();
        }
    }
}

impl Drop for Subst {
    fn drop(&mut self) {
        self.remove();
    }
}

async fn listed(
    events: &mut mpsc::Receiver<Event>,
    deadline: Duration,
    wanted: impl Fn(&[Volume]) -> bool,
) -> Option<Duration> {
    let start = Instant::now();
    loop {
        let left = deadline.checked_sub(start.elapsed())?;
        match tokio::time::timeout(left, events.recv()).await {
            Ok(Some(Event::VolumesUpdated(volumes))) if wanted(&volumes) => {
                return Some(start.elapsed());
            }
            Ok(Some(_)) => {}
            _ => return None,
        }
    }
}

async fn measure(watch: bool, deadline: Duration) -> (Option<Duration>, Option<Duration>) {
    let folder = tempfile::tempdir().unwrap();
    let local = Arc::new(midnight_platform::LocalFileSystem::new(None));
    let (sender, mut events) = mpsc::channel(256);
    let mut engine = Engine::new(
        Engine::lifetime().expect("engine cleanup worker"),
        local.clone(),
        sender,
    );
    if watch {
        engine.configure_watch().unwrap().install(local);
    }
    let (commands, receiver) = engine.command_channel(8).unwrap();
    let engine = tokio::spawn(engine.run(receiver));
    commands.send(Command::ListVolumes).await.unwrap();
    listed(&mut events, Duration::from_secs(5), |_| true)
        .await
        .expect("the first drive list");
    // Let the native watch register before the drive changes.
    tokio::time::sleep(Duration::from_millis(200)).await;
    let mut subst = Subst::define(folder.path());
    let root = subst.root();
    let has = |volumes: &[Volume]| volumes.iter().any(|volume| volume.root == root);
    let added = listed(&mut events, deadline, has).await;
    subst.remove();
    let removed = listed(&mut events, deadline, |volumes| !has(volumes)).await;
    commands.send(Command::Shutdown).await.unwrap();
    engine.await.unwrap();
    (added, removed)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "defines a temporary subst drive letter in the user's session"]
async fn a_subst_drive_is_listed_and_unlisted_within_half_a_second() {
    let (added, removed) = measure(true, Duration::from_secs(5)).await;
    println!("drive watch: listed after {added:?}, unlisted after {removed:?}");
    assert!(added.is_some_and(|latency| latency < Duration::from_millis(500)));
    assert!(removed.is_some_and(|latency| latency < Duration::from_millis(500)));
}

/// The behavior without a drive watch: only the 45-second refresh notices the drive.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "defines a temporary subst drive letter in the user's session; takes up to 90 s"]
async fn without_a_drive_watch_only_the_periodic_refresh_lists_a_subst_drive() {
    let (added, removed) = measure(false, Duration::from_secs(50)).await;
    println!("periodic refresh only: listed after {added:?}, unlisted after {removed:?}");
    assert!(added.is_some_and(|latency| latency > Duration::from_millis(500)));
}
