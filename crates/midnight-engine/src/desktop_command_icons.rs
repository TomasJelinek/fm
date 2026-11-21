//! Visible executable artwork shares the existing serial desktop icon actor.
use super::file_icons::Pending as Tiles;
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

/// What the icon actor serves: icon requests, the visible command artwork and file
/// artwork views, and the edges icons are drawn at.
pub(super) struct Inputs {
    pub icons: mpsc::Receiver<IconKey>,
    pub commands: watch::Receiver<View>,
    pub files: watch::Receiver<file_icons::View>,
    pub edges: watch::Receiver<IconEdges>,
}

pub(super) async fn worker(
    scope: TaskLifetime,
    services: Option<Arc<dyn DesktopServices>>,
    capture: Option<crate::archives::ArchiveCapture>,
    outbox: Outbox,
    inputs: Inputs,
) {
    let Inputs {
        mut icons,
        mut commands,
        mut files,
        mut edges,
    } = inputs;
    let mut cached = RecentIcons::default();
    let mut pending = Pending::default();
    let mut tiles = Tiles::default();
    let mut absent = file_icons::Absent::default();
    // Sized keys still to load. `requested` holds those of the icon received
    // last; nothing new is received until it is empty, so the channel keeps
    // bounding what listings can queue. `reloads` holds, after the drawn edges
    // changed, the missing edges of every remembered icon; a request that
    // arrives meanwhile is served first.
    let mut requested = VecDeque::new();
    let mut reloads = VecDeque::new();
    let mut drawn = *edges.borrow_and_update();
    let (mut icons_open, mut commands_open, mut edges_open) = (true, true, true);
    let mut files_open = true;
    let (mut prefer_command, mut prefer_files) = (false, false);
    loop {
        // Ordinary icons and command artwork take turns while both have work, and
        // file artwork takes every other turn while it has work.
        let others = !prefer_files || tiles.targets.is_empty();
        let turn = (!prefer_command || pending.targets.is_empty()) && others;
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
            changed = files.changed(), if files_open => {
                if changed.is_err() {
                    files_open = false;
                    continue;
                }
                // The replaced view's load in progress is cancelled with it.
                tiles = files.borrow_and_update().clone().into();
            }
            changed = edges.changed(), if edges_open => {
                if changed.is_err() {
                    edges_open = false;
                    continue;
                }
                drawn = *edges.borrow_and_update();
                let missing = cached.resize(|kind| provider_edges(services.as_deref(), drawn, kind));
                // Keys queued for edges that are no longer drawn are dropped;
                // the others keep their place and are not queued twice.
                requested.retain(|key| cached.wants(key));
                reloads.retain(|key| cached.wants(key));
                reloads.extend(missing);
            }
            () = std::future::ready(()), if !requested.is_empty() && turn => {
                (prefer_command, prefer_files) = (true, true);
                let Some(key) = requested.pop_front() else { continue; };
                if !deliver(&scope, &services, &outbox, &cached, key).await {
                    break;
                }
            }
            key = icons.recv(), if icons_open && requested.is_empty() && turn => {
                let Some(key) = key else { icons_open = false; continue; };
                // Requests name an icon; the edges it is drawn at decide its sizes.
                let icon = key.sized(IconKey::DEFAULT_EDGE);
                let wanted = provider_edges(services.as_deref(), drawn, icon.kind);
                let missing = cached.admit(&icon, &wanted);
                // An icon that needs nothing still takes this worker's turn, as a
                // load does, so pending command and file artwork is never starved.
                prefer_command |= missing.is_empty();
                prefer_files |= missing.is_empty();
                requested.extend(missing.into_iter().map(|edge| icon.clone().sized(edge)));
            }
            () = std::future::ready(()), if requested.is_empty() && !reloads.is_empty() && turn => {
                (prefer_command, prefer_files) = (true, true);
                let Some(key) = reloads.pop_front() else { continue; };
                if !deliver(&scope, &services, &outbox, &cached, key).await {
                    break;
                }
            }
            () = std::future::ready(()), if !pending.targets.is_empty() && others => {
                (prefer_command, prefer_files) = (false, true);
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
            () = std::future::ready(()), if !tiles.targets.is_empty() => {
                prefer_files = false;
                let Some(target) = tiles.targets.pop_front() else { continue; };
                if tiles.cancel.is_cancelled() {
                    tiles.targets.clear();
                    continue;
                }
                if absent.contains(&target.identity) {
                    continue;
                }
                let service = services.clone();
                let capture = capture.clone();
                let cancel = tiles.cancel.clone();
                let FileIconTarget { identity, path, edge } = target;
                let result = tokio::task::spawn_blocking(scope.blocking(move || {
                    file_icons::load(service.as_deref(), capture.as_ref(), &path, edge, &cancel)
                })).await;
                if tiles.cancel.is_cancelled() {
                    continue;
                }
                match result {
                    Ok(Some(pixels)) => {
                        if !outbox.send(Event::FileIconLoaded {
                            generation: tiles.generation,
                            identity,
                            edge,
                            pixels: Arc::new(pixels),
                        }).await {
                            break;
                        }
                    }
                    // The tile keeps its type icon; its file is not read again.
                    Ok(None) => absent.insert(identity),
                    Err(_) => {}
                }
            }
            else => break,
        }
    }
}

/// Loads one sized key on a blocking worker and sends its bitmap, or the
/// fallback of its kind. Returns false once the frontend is gone. A key whose
/// icon was forgotten since it was queued is skipped.
async fn deliver(
    scope: &TaskLifetime,
    services: &Option<Arc<dyn DesktopServices>>,
    outbox: &Outbox,
    cached: &RecentIcons,
    key: IconKey,
) -> bool {
    if !cached.wants(&key) {
        return true;
    }
    let service = services.clone();
    let worker_key = key.clone();
    let kind = key.kind;
    let result = tokio::task::spawn_blocking(scope.blocking(move || {
        service
            .and_then(|service| service.load_icon(&worker_key).ok().flatten())
            .filter(valid_pixels)
            .or_else(|| fallback_icon(worker_key.kind))
    }))
    .await;
    match result.unwrap_or_else(|_| fallback_icon(kind)) {
        Some(pixels) => {
            outbox
                .send(Event::IconLoaded {
                    key,
                    pixels: Arc::new(pixels),
                })
                .await
        }
        None => true,
    }
}

/// The edges to ask the provider for when `kind` is drawn at the frontend's
/// `drawn` edges. A provider that serves one size answers every edge with it.
fn provider_edges(
    service: Option<&dyn DesktopServices>,
    drawn: IconEdges,
    kind: IconKind,
) -> Vec<u16> {
    let mut edges = Vec::new();
    for edge in drawn.of(kind) {
        let edge = service.map_or(IconKey::DEFAULT_EDGE, |service| service.icon_edge(edge));
        if (1..=MAX_ICON_EDGE).contains(&edge) && !edges.contains(&edge) {
            edges.push(edge);
        }
    }
    edges
}

/// Icons already delivered this session, bounded by `ICON_CAPACITY`, each with
/// the edges it was delivered at. The least recently requested icon is
/// forgotten when a new one arrives, so many one-off types (rotated logs,
/// numbered archive parts) cannot stop later new types from ever loading; a
/// forgotten icon is simply loaded again if needed.
#[derive(Default)]
struct RecentIcons {
    /// Keyed at `IconKey::DEFAULT_EDGE`.
    icons: std::collections::HashMap<IconKey, Recent>,
    clock: u64,
}

struct Recent {
    used: u64,
    /// Edges loaded or queued for loading.
    edges: Vec<u16>,
}

impl RecentIcons {
    /// The edges of `icon`, out of the `wanted` ones, that must be loaded now.
    fn admit(&mut self, icon: &IconKey, wanted: &[u16]) -> Vec<u16> {
        self.clock += 1;
        if let Some(recent) = self.icons.get_mut(icon) {
            recent.used = self.clock;
            let missing: Vec<u16> = wanted
                .iter()
                .copied()
                .filter(|edge| !recent.edges.contains(edge))
                .collect();
            recent.edges.extend(&missing);
            return missing;
        }
        if self.icons.len() >= ICON_CAPACITY
            && let Some(oldest) = self
                .icons
                .iter()
                .min_by_key(|(_, recent)| recent.used)
                .map(|(key, _)| key.clone())
        {
            self.icons.remove(&oldest);
        }
        self.icons.insert(
            icon.clone(),
            Recent {
                used: self.clock,
                edges: wanted.to_vec(),
            },
        );
        wanted.to_vec()
    }

    /// After the drawn edges changed: forgets edges that are no longer wanted
    /// and returns the sized keys now missing, most recently requested icon first.
    fn resize(&mut self, wanted: impl Fn(IconKind) -> Vec<u16>) -> Vec<IconKey> {
        let mut icons: Vec<_> = self.icons.iter_mut().collect();
        icons.sort_by_key(|(_, recent)| std::cmp::Reverse(recent.used));
        let mut missing = Vec::new();
        for (icon, recent) in icons {
            let wanted = wanted(icon.kind);
            missing.extend(
                wanted
                    .iter()
                    .filter(|edge| !recent.edges.contains(edge))
                    .map(|edge| icon.clone().sized(*edge)),
            );
            recent.edges = wanted;
        }
        missing
    }

    /// False for a key whose icon was forgotten or whose edge is no longer drawn.
    fn wants(&self, key: &IconKey) -> bool {
        self.icons
            .get(&key.clone().sized(IconKey::DEFAULT_EDGE))
            .is_some_and(|recent| recent.edges.contains(&key.size))
    }
}

pub(super) fn valid_pixels(pixels: &IconPixels) -> bool {
    let edges = 1..=u32::from(MAX_ICON_EDGE);
    edges.contains(&pixels.width)
        && edges.contains(&pixels.height)
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
