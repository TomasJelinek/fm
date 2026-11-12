//! Visible executable artwork shares the existing serial desktop icon actor.
use super::*;

#[derive(Clone, Default)]
pub(super) struct View {
    pub generation: u64,
    pub targets: Vec<CommandIconTarget>,
    pub cancel: Cancellation,
}

pub(super) fn valid_targets(targets: &[CommandIconTarget]) -> bool {
    if targets.len() > MAX_USER_COMMANDS {
        return false;
    }
    let mut identities = HashSet::new();
    let mut bytes = 0usize;
    targets.iter().all(|target| {
        let path = target.executable.as_os_str().as_encoded_bytes();
        bytes = bytes
            .saturating_add(target.executable.capacity())
            .saturating_add(target.identity.capacity());
        target.identity.starts_with("command-icon:")
            && target.identity.capacity() <= 128
            && target.identity.bytes().all(|byte| byte.is_ascii_graphic())
            && target.executable.is_absolute()
            && !path.contains(&0)
            && identities.insert(target.identity.as_str())
            && bytes <= MAX_USER_COMMAND_CONFIG_BYTES
    })
}

#[derive(Default)]
struct Pending {
    generation: u64,
    targets: VecDeque<CommandIconTarget>,
    cancel: Cancellation,
}
impl Drop for Pending {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub(super) async fn worker(
    scope: TaskLifetime,
    services: Option<Arc<dyn DesktopServices>>,
    capture: Option<crate::archives::ArchiveCapture>,
    outbox: Outbox,
    mut icons: mpsc::Receiver<IconKey>,
    mut commands: watch::Receiver<View>,
) {
    let mut cached = HashSet::new();
    let mut pending = Pending::default();
    let (mut icons_open, mut commands_open) = (true, true);
    let mut prefer_command = false;
    loop {
        tokio::select! {
            biased;
            changed = commands.changed(), if commands_open => {
                if changed.is_err() {
                    commands_open = false;
                    continue;
                }
                let view = commands.borrow_and_update().clone();
                pending = Pending {
                    generation: view.generation,
                    targets: view.targets.into(),
                    cancel: view.cancel,
                };
            }
            key = icons.recv(), if icons_open && (!prefer_command || pending.targets.is_empty()) => {
                prefer_command = true;
                let Some(key) = key else { icons_open = false; continue; };
                if cached.contains(&key) || cached.len() == ICON_CAPACITY {
                    continue;
                }
                cached.insert(key.clone());
                let service = services.clone();
                let worker_key = key.clone();
                let kind = key.kind;
                let result = tokio::task::spawn_blocking(scope.blocking(move || {
                    service
                        .and_then(|service| service.load_icon(&worker_key).ok().flatten())
                        .filter(valid_pixels)
                        .or_else(|| fallback_icon(worker_key.kind))
                })).await;
                if let Some(pixels) = result.unwrap_or_else(|_| fallback_icon(kind))
                    && !outbox.send(Event::IconLoaded { key, pixels: Arc::new(pixels) }).await {
                    break;
                }
            }
            () = std::future::ready(()), if !pending.targets.is_empty() => {
                prefer_command = false;
                let Some(target) = pending.targets.pop_front() else { continue; };
                if pending.cancel.is_cancelled() {
                    pending.targets.clear();
                    continue;
                }
                let service = services.clone();
                let capture = capture.clone();
                let path = target.executable;
                let cancel = pending.cancel.clone();
                let result = tokio::task::spawn_blocking(scope.blocking(move || {
                    load_command(service.as_deref(), capture.as_ref(), &path, &cancel)
                })).await;
                if pending.cancel.is_cancelled() {
                    continue;
                }
                if let Ok(Some(pixels)) = result
                    && !outbox.send(Event::CommandIconLoaded {
                        generation: pending.generation,
                        identity: target.identity,
                        pixels: Arc::new(pixels),
                    }).await {
                    break;
                }
            }
            else => break,
        }
    }
}

fn valid_pixels(pixels: &IconPixels) -> bool {
    (1..=64).contains(&pixels.width)
        && (1..=64).contains(&pixels.height)
        && pixels.rgba.len() == pixels.width as usize * pixels.height as usize * 4
        && pixels
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .any(|pixel| pixel[3] != 0)
}

fn load_command(
    service: Option<&dyn DesktopServices>,
    capture: Option<&crate::archives::ArchiveCapture>,
    path: &Path,
    cancel: &Cancellation,
) -> Option<IconPixels> {
    if cancel.is_cancelled() {
        return None;
    }
    let service = service?;
    let source = (capture?)(path, None, cancel).ok()?;
    source.revalidate().ok()?;
    if cancel.is_cancelled() {
        return None;
    }
    let pixels = service.load_executable_icon(path, 64, cancel).ok()??;
    source.revalidate().ok()?;
    (!cancel.is_cancelled() && valid_pixels(&pixels)).then_some(pixels)
}

#[cfg(test)]
#[path = "desktop_command_icons_tests.rs"]
mod tests;
