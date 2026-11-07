//! Prepare file identities independently of the first viewport and its parser.
use midnight_core::{EntryKind, ViewerLaunch};
use midnight_plugin_host::CapturedFile;
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::Duration,
};

pub(crate) type Captured = Result<CapturedFile, String>;
pub(crate) struct Order {
    pub files: Vec<(PathBuf, Option<Captured>)>,
    pub current: usize,
    pub ready: Receiver<(usize, Captured)>,
}

pub(crate) fn prepare(launch: &ViewerLaunch, closed: &Arc<AtomicBool>) -> Result<Order, String> {
    if launch.archive.is_some() {
        return crate::capture_archive::prepare(launch, closed);
    }
    let mut ordered = Vec::new();
    let mut bytes = 0usize;
    for entry in launch.entries.iter() {
        if closed.load(Ordering::Acquire) {
            return Err("Viewer closed".into());
        }
        if entry.kind != EntryKind::File {
            continue;
        }
        let Some(row) = launch.view.by_id.get(&entry.id) else {
            continue;
        };
        // Includes work descriptors, UTF-16/native path copies and captured identities.
        bytes = bytes
            .saturating_add(
                entry
                    .path
                    .as_os_str()
                    .as_encoded_bytes()
                    .len()
                    .saturating_mul(8),
            )
            .saturating_add(512);
        if bytes > 8 * 1024 * 1024 {
            return Err("The captured file order exceeds the 8 MiB viewer budget.".into());
        }
        ordered.push((*row, &entry.path));
    }
    ordered.sort_unstable_by_key(|entry| entry.0);
    let mut files: Vec<_> = ordered
        .into_iter()
        .map(|(_, path)| (path.clone(), None))
        .collect();
    let current = files
        .iter()
        .position(|(path, _)| *path == launch.focused)
        .ok_or("The focused regular file is no longer in the captured view.")?;
    files[current].1 = Some(CapturedFile::capture(&files[current].0).map_err(|e| e.to_string()));
    let paths: Vec<_> = (1..files.len())
        .map(|step| {
            let index = (current + step) % files.len();
            (index, files[index].0.clone())
        })
        .collect();
    let (sender, ready) = mpsc::sync_channel(16);
    let closed = closed.clone();
    tokio::runtime::Handle::try_current()
        .map_err(|e| e.to_string())?
        .spawn_blocking(move || {
            for (index, path) in paths {
                if closed.load(Ordering::Acquire) {
                    break;
                }
                let result = CapturedFile::capture(&path).map_err(|e| e.to_string());
                let mut event = (index, result);
                loop {
                    if closed.load(Ordering::Acquire) {
                        return;
                    }
                    match sender.try_send(event) {
                        Ok(()) => break,
                        Err(mpsc::TrySendError::Full(value)) => event = value,
                        Err(mpsc::TrySendError::Disconnected(_)) => return,
                    }
                    std::thread::park_timeout(Duration::from_millis(8));
                }
            }
        });
    Ok(Order {
        files,
        current,
        ready,
    })
}
