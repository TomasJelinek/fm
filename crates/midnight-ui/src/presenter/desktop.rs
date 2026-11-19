//! Native desktop requests remain messages; this module performs no I/O.
use super::*;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::collections::VecDeque;

/// The backend loads each icon once and does not resend one among its
/// `DESKTOP_ICON_CAPACITY` most recent ones, so this cache holds that many, each
/// at every edge it was delivered at; an evicted icon falls back to its generic
/// kind icon.
const IMAGE_CACHE_CAPACITY: usize = midnight_core::DESKTOP_ICON_CAPACITY;
/// A ceiling on bitmap bytes that the icons the backend remembers cannot reach:
/// each of them as a row and as a tile of the largest edge. Dropping one of
/// those would lose it for good, because the backend does not send it again.
/// The count above therefore bounds memory in normal use (256 file types with
/// tiles of 96, 192 or 256 device pixels hold about 10, 38 or 66 MiB; rows
/// alone well under 1 MiB); this only stops bitmaps of edges nobody draws from
/// piling up.
const IMAGE_CACHE_BYTES: usize =
    IMAGE_CACHE_CAPACITY * 2 * (MAX_ICON_EDGE as usize) * (MAX_ICON_EDGE as usize) * 4;

/// Where an icon is drawn, which decides the bitmap that is used for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum IconPlace {
    /// A row or tile of this pane's listing.
    Entry(PaneId),
    /// A location, drive or bookmark choice.
    Location,
    /// Toolbar, function-key, menu and dialog actions, which share one bitmap.
    Action,
}

/// The device-pixel edges at which the window draws native icons, and the panes
/// whose entries are tiles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct IconView {
    pub edges: IconEdges,
    pub tiles: [bool; 2],
}

/// One delivered bitmap of an icon.
struct IconVariant {
    /// The edge it was requested at. A provider or fallback may deliver other pixels.
    edge: u16,
    image: Image,
    generic: bool,
    bytes: usize,
}

#[derive(Default)]
pub(crate) struct DesktopState {
    pub(crate) shell: Option<ShellFlavor>,
    pub(crate) locations: Vec<NavigationLocation>,
    pub(crate) system_locations: Vec<SystemLocationChoice>,
    pub(crate) network_position: Option<usize>,
    pub(crate) error: String,
    pub(crate) status: String,
    pub(crate) notice: Option<(String, String)>,
    pub(crate) notice_error: bool,
    pub(crate) notice_path: String,
    pub(crate) notice_details: String,
    pub(super) notice_copy: Option<String>,
    pub(super) notice_navigation: Option<(PaneId, NavigationId, u64, PathBuf)>,
    pub(crate) notice_generation: i32,
    pub(crate) terminal_pending: Option<DesktopRequestId>,
    pub(crate) terminal_mode: TerminalMode,
    pub(crate) draft_nonempty: bool,
    pub(crate) draft_shell: Option<ShellFlavor>,
    pub(crate) command_history_cursor: Option<String>,
    pub(crate) terminal_guidance: Option<&'static str>,
    /// Keyed at `IconKey::DEFAULT_EDGE`; each icon's bitmaps by ascending edge.
    images: HashMap<IconKey, Vec<IconVariant>>,
    /// Icons in the order of their first delivery.
    image_order: VecDeque<IconKey>,
    image_bytes: usize,
    icon_view: IconView,
    /// The edges the backend has accepted.
    icon_edges_sent: Option<IconEdges>,
    pub(super) requests: HashSet<DesktopRequestId>,
    pub(super) system_location_requests: HashMap<DesktopRequestId, SystemLocation>,
    open_requests: HashMap<DesktopRequestId, (PaneId, NavigationId, PathBuf)>,
    open_directories: [Option<(NavigationId, PathBuf)>; 2],
    generation: u64,
}

impl DesktopState {
    pub(super) fn request(&mut self) -> DesktopRequestId {
        self.generation = self.generation.saturating_add(1);
        DesktopRequestId(self.generation)
    }
    pub(crate) fn report(&mut self, title: &str, body: String) {
        self.notice_path.clear();
        self.notice_details.clear();
        self.notice_copy = None;
        self.notice_navigation = None;
        self.notice_error = true;
        self.notice_generation = self.notice_generation.wrapping_add(1);
        self.notice = Some((title.into(), body));
    }
    pub(crate) fn inform(&mut self, title: &str, body: String) {
        self.report(title, body);
        self.notice_error = false;
    }
    pub(crate) fn image(&self, key: &IconKey, place: IconPlace) -> Image {
        self.entry_image(key, place, false)
    }
    pub(crate) fn entry_image(&self, key: &IconKey, place: IconPlace, archive: bool) -> Image {
        let edge = self.edge(place);
        self.variant(key, edge)
            .or_else(|| {
                self.variant(
                    &IconKey {
                        kind: if matches!(key.kind, IconKind::Location(_)) {
                            IconKind::Folder
                        } else {
                            key.kind
                        },
                        extension: None,
                        drive_root: None,
                        size: IconKey::DEFAULT_EDGE,
                    },
                    edge,
                )
            })
            .filter(|variant| !archive || !variant.generic)
            .map(|variant| variant.image.clone())
            .unwrap_or_default()
    }
    /// The edge in device pixels of an icon drawn at `place`.
    fn edge(&self, place: IconPlace) -> u16 {
        let IconView { edges, tiles } = self.icon_view;
        match place {
            IconPlace::Entry(pane) if tiles[pane.index()] && edges.tile != 0 => edges.tile,
            IconPlace::Entry(_) => edges.entry,
            IconPlace::Location => edges.location,
            IconPlace::Action => IconKey::DEFAULT_EDGE,
        }
    }
    /// The bitmap of exactly `edge`, else the next larger one, which scales down
    /// well, else the largest. A provider with one size answers every edge.
    fn variant(&self, key: &IconKey, edge: u16) -> Option<&IconVariant> {
        let variants = if key.size == IconKey::DEFAULT_EDGE {
            self.images.get(key)
        } else {
            self.images.get(&key.clone().sized(IconKey::DEFAULT_EDGE))
        }?;
        variants
            .iter()
            .find(|variant| variant.edge >= edge)
            .or(variants.last())
    }
    fn receive_image(&mut self, key: IconKey, pixels: &IconPixels) {
        // Validate again at the UI boundary before creating a Slint image. Models
        // clone these handles; row_data never decodes or constructs an image.
        let edges = 1..=u32::from(MAX_ICON_EDGE);
        if !edges.contains(&pixels.width) || !edges.contains(&pixels.height) {
            return;
        }
        // Within the bounds just checked this cannot overflow.
        let bytes = pixels.width as usize * pixels.height as usize * 4;
        if pixels.rgba.len() != bytes {
            return;
        }
        let edge = key.size;
        let icon = key.sized(IconKey::DEFAULT_EDGE);
        match self.images.get(&icon) {
            // The first delivery stays: models compare images by identity.
            Some(variants) if variants.iter().any(|variant| variant.edge == edge) => return,
            Some(_) => {}
            None => {
                if self.images.len() == IMAGE_CACHE_CAPACITY {
                    self.evict(&icon);
                }
                self.image_order.push_back(icon.clone());
            }
        }
        // Premultiplied pixels keep the edges of artwork clean wherever a bitmap
        // is not drawn at its own size: the hidden color of transparent pixels
        // then never mixes in.
        let mut buffer = SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
            &pixels.rgba,
            pixels.width,
            pixels.height,
        );
        for pixel in buffer.make_mut_slice() {
            let alpha = u16::from(pixel.a);
            if alpha != 255 {
                let premultiply = |channel: u8| ((u16::from(channel) * alpha + 127) / 255) as u8;
                (pixel.r, pixel.g, pixel.b) = (
                    premultiply(pixel.r),
                    premultiply(pixel.g),
                    premultiply(pixel.b),
                );
            }
        }
        let drawn: Vec<u16> = self.icon_view.edges.of(icon.kind).collect();
        let variants = self.images.entry(icon.clone()).or_default();
        if drawn.contains(&edge) {
            // Bitmaps of edges that are no longer drawn only stood in for this one.
            variants.retain(|variant| {
                let keep = drawn.contains(&variant.edge);
                if !keep {
                    self.image_bytes -= variant.bytes;
                }
                keep
            });
        }
        let at = variants.partition_point(|variant| variant.edge < edge);
        variants.insert(
            at,
            IconVariant {
                edge,
                image: Image::from_rgba8_premultiplied(buffer),
                generic: pixels.generic,
                bytes,
            },
        );
        self.image_bytes += bytes;
        self.trim(&icon, IMAGE_CACHE_BYTES);
    }
    /// Drops the oldest icons other than `keep` until at most `budget` bytes remain.
    fn trim(&mut self, keep: &IconKey, budget: usize) {
        while self.image_bytes > budget && self.evict(keep) {}
    }
    /// Drops the oldest icon of a specific file type, else the oldest icon, but
    /// never `keep`. Returns false when nothing else is left.
    fn evict(&mut self, keep: &IconKey) -> bool {
        let victim = self
            .image_order
            .iter()
            .position(|icon| icon.extension.is_some() && icon != keep)
            .or_else(|| self.image_order.iter().position(|icon| icon != keep));
        let Some(old) = victim.and_then(|victim| self.image_order.remove(victim)) else {
            return false;
        };
        if let Some(variants) = self.images.remove(&old) {
            self.image_bytes -= variants.iter().map(|variant| variant.bytes).sum::<usize>();
        }
        true
    }
}

impl Presenter {
    /// Records where the window draws icons and tells the backend the edges it
    /// has not accepted yet. Returns whether icons must be looked up again:
    /// another bitmap may now fit a row, tile or choice.
    pub fn sync_icon_view(&mut self, view: IconView, send: &dyn Fn(Command) -> bool) -> bool {
        let changed = self.desktop.icon_view != view;
        self.desktop.icon_view = view;
        if self.desktop.icon_edges_sent != Some(view.edges)
            && send(Command::SetIconEdges(view.edges))
        {
            self.desktop.icon_edges_sent = Some(view.edges);
        }
        changed
    }
}

impl Presenter {
    pub(super) fn apply_desktop(&mut self, event: Event) -> Option<Event> {
        match event {
            Event::VolumeInformationLoaded(loaded) => self.volume_information_loaded(*loaded),
            Event::ViewerPluginsListed(result) => self.viewer_plugins_listed(result),
            Event::ComparatorCapabilitiesListed(snapshot) => {
                self.comparator_capabilities_listed(snapshot)
            }
            Event::FilesystemPluginsListed(result) => self.filesystem_plugins_listed(result),
            Event::ThumbnailerPluginsListed(result) => self.thumbnailer_plugins_listed(result),
            Event::EditorDefaultResolved(result) => self.editor_default_resolved(result),
            Event::FontFamiliesLoaded(result) => self.apply_font_families(result),
            Event::PreferencesLoaded(result) => {
                self.accepted_preferences_loaded(result.map(|values| *values))
            }
            Event::PreferencesSaved { revision, result } => {
                self.accepted_preferences_saved(revision, result)
            }
            // The app coordinates watch hints through refresh_if_changed so
            // they cannot trigger nested sends while apply owns this state.
            Event::DirectoryChanged { .. } => {}
            Event::DesktopContext {
                shell,
                locations,
                network_position,
                system_locations,
            } => {
                self.desktop.shell = Some(shell);
                self.desktop.network_position = Some(network_position);
                self.track_draft(self.desktop.draft_nonempty);
                self.retired.push(Box::new(mem::replace(
                    &mut self.desktop.locations,
                    locations,
                )));
                self.retired.push(Box::new(mem::replace(
                    &mut self.desktop.system_locations,
                    system_locations,
                )));
                self.desktop.error.clear();
                self.sync_navigation_choices();
            }
            Event::DesktopContextFailed(error) => {
                self.desktop.error = crate::error_presentation::describe(&error);
                self.retired
                    .push(Box::new(mem::take(&mut self.desktop.system_locations)));
                self.sync_navigation_choices();
            }
            Event::CommandIconLoaded {
                generation,
                identity,
                pixels,
            } => {
                self.receive_command_icon(generation, &identity, &pixels);
                self.retired.push(Box::new(pixels));
            }
            Event::IconLoaded { key, pixels } => {
                self.desktop.receive_image(key, &pixels);
                self.retired.push(Box::new(pixels));
            }
            Event::DesktopActionFinished { request, result } => {
                let captured_open = self.tree_open_request(request);
                self.tree_clipboard_finished(request);
                self.tree_editor_finished(request);
                if !self.desktop.requests.remove(&request) {
                    return None;
                }
                if self.desktop.terminal_pending == Some(request) {
                    self.desktop.terminal_pending = None;
                }
                let opened = self.desktop.open_requests.remove(&request);
                let system_location = self.desktop.system_location_requests.remove(&request);
                match result {
                    Ok(DesktopActionResult::SystemLocationOpened { location }) => {
                        if system_location == Some(location) {
                            self.desktop.status = "System Recycle Bin/Trash opened".into();
                        }
                    }
                    Ok(DesktopActionResult::TerminalStarted { history, .. }) => {
                        let status = if history.command.is_empty() {
                            "Terminal opened"
                        } else {
                            "Command opened in terminal"
                        };
                        self.record_command_history(history);
                        self.desktop.status = status.into()
                    }
                    Ok(DesktopActionResult::ClipboardCopied) => {
                        self.desktop.status = "Copied to clipboard".into()
                    }
                    Ok(DesktopActionResult::EditorStarted) => {
                        self.desktop.status = "External editor started".into();
                    }
                    Ok(DesktopActionResult::PropertiesOpened) => {
                        self.desktop.status = "Properties opened".into()
                    }
                    Ok(DesktopActionResult::PropertiesInfo { title, body }) => {
                        self.desktop.inform(&title, body)
                    }
                    Ok(DesktopActionResult::FileOpened) => {
                        if opened.is_some() || captured_open.is_some() {
                            self.desktop.status = "File opened".into();
                        }
                    }
                    Ok(DesktopActionResult::OpenDirectory {
                        pane,
                        navigation,
                        path,
                    }) => {
                        if (opened == Some((pane, navigation, path.clone()))
                            || captured_open == Some((pane, navigation)))
                            && self.panes[pane.index()].generation == navigation.0
                            && path.is_absolute()
                            && !path.as_os_str().as_encoded_bytes().contains(&0)
                        {
                            self.desktop.open_directories[pane.index()] = Some((navigation, path));
                        }
                    }
                    Ok(DesktopActionResult::OpenDirectoryTarget {
                        pane,
                        navigation,
                        path,
                        target,
                    }) => {
                        if (opened == Some((pane, navigation, path))
                            || captured_open == Some((pane, navigation)))
                            && self.panes[pane.index()].generation == navigation.0
                            && target.is_absolute()
                            && !target.as_os_str().as_encoded_bytes().contains(&0)
                        {
                            self.desktop.open_directories[pane.index()] =
                                Some((navigation, target));
                        }
                    }
                    Err(error) => self.desktop.report(
                        "Desktop action failed",
                        crate::error_presentation::describe(&error),
                    ),
                }
            }
            Event::FolderSizeUpdated {
                pane,
                navigation,
                scan,
                entry,
                size,
            } => {
                let p = &mut self.panes[pane.index()];
                if (p.size_scan == Some(scan) || p.cancelled_size_scan == Some(scan))
                    && let Some(scope) = &p.size_scan_scope
                    && scope.navigation == navigation
                    && scope.path == p.display_path()
                    && let Some(path) = scope
                        .index
                        .by_id
                        .get(&entry)
                        .and_then(|index| scope.entries.get(*index))
                        .map(|entry| &entry.path)
                    && p.index.by_path.contains_key(path)
                    && scope.contains(path)
                {
                    p.folder_column_samples.observe(size.bytes);
                    self.retired
                        .push(Box::new(p.folder_sizes.insert(path.clone(), size)));
                } else {
                    self.retired.push(Box::new(size));
                }
            }
            Event::FolderSizesFinished {
                pane,
                navigation,
                scan,
                cancelled,
            } => {
                let p = &mut self.panes[pane.index()];
                if p.size_scan == Some(scan)
                    && p.size_scan_scope.as_ref().is_some_and(|scope| {
                        scope.navigation == navigation && scope.path == p.display_path()
                    })
                {
                    p.size_scan = None;
                    p.cancelled_size_scan = cancelled.then_some(scan);
                    if !cancelled {
                        self.retired.push(Box::new(p.size_scan_scope.take()));
                    }
                    self.desktop.status = if cancelled {
                        "Folder size scan cancelled; unfinished sizes are partial"
                    } else {
                        "Folder sizes updated (logical lengths; links skipped)"
                    }
                    .into();
                }
            }
            event => return Some(event),
        }
        None
    }

    pub(super) fn open_native_entry(
        &mut self,
        pane: PaneId,
        path: PathBuf,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        self.activate_native(pane, path, false, send)
    }

    pub(super) fn open_native_directory(
        &mut self,
        pane: PaneId,
        path: PathBuf,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        self.activate_native(pane, path, true, send)
    }

    fn activate_native(
        &mut self,
        pane: PaneId,
        path: PathBuf,
        directory_only: bool,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let request = self.desktop.request();
        let navigation = NavigationId(self.panes[pane.index()].generation);
        let command = if directory_only {
            Command::ResolveDirectoryLink {
                request,
                pane,
                navigation,
                path: path.clone(),
            }
        } else {
            Command::OpenEntry {
                request,
                pane,
                navigation,
                path: path.clone(),
            }
        };
        if !send(command) {
            self.desktop
                .report("Open queue busy", "Try opening the entry again.".into());
            return false;
        }
        self.desktop.requests.insert(request);
        self.desktop
            .open_requests
            .insert(request, (pane, navigation, path));
        self.desktop.status = "Opening entry…".into();
        true
    }

    pub(super) fn dispatch_open_directories(&mut self, send: &dyn Fn(Command) -> bool) {
        for pane in [PaneId::Left, PaneId::Right] {
            if let Some((navigation, path)) = self.desktop.open_directories[pane.index()].take()
                && self.panes[pane.index()].generation == navigation.0
            {
                self.navigate(pane, path, None, send);
            }
        }
    }

    pub(crate) fn submit_command(&mut self, text: &str, send: &dyn Fn(Command) -> bool) -> bool {
        self.track_draft(!text.is_empty());
        if text.trim().is_empty() {
            return false;
        }
        self.launch_terminal(text, send)
    }

    /// Open an interactive shell without submitting or changing the editor draft.
    pub(crate) fn open_terminal(&mut self, send: &dyn Fn(Command) -> bool) -> bool {
        self.launch_terminal("", send)
    }

    fn launch_terminal(&mut self, text: &str, send: &dyn Fn(Command) -> bool) -> bool {
        if !self.native_action_allowed(self.active) {
            return false;
        }
        if self.desktop.terminal_pending.is_some() {
            return false;
        }
        let Some(shell) = self.effective_shell() else {
            self.desktop.report(
                "Terminal unavailable",
                "Desktop services are still loading or unavailable.".into(),
            );
            return false;
        };
        let request = self.desktop.request();
        let terminal = TerminalRequest {
            command: text.into(),
            working_directory: self.panes[self.active.index()].display_path().to_owned(),
            shell,
            mode: self.effective_terminal_mode(),
        };
        if !send(Command::LaunchTerminal { request, terminal }) {
            self.desktop.report(
                "Terminal queue busy",
                "Try opening the terminal again.".into(),
            );
            return false;
        }
        self.desktop.requests.insert(request);
        self.desktop.terminal_pending = Some(request);
        self.desktop.status = "Opening terminal…".into();
        true
    }

    pub(crate) fn properties(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        self.request_selection_properties(pane, send);
    }

    pub(crate) fn choose_location(
        &mut self,
        pane: PaneId,
        index: usize,
        send: &dyn Fn(Command) -> bool,
    ) {
        let Some(location) = self.desktop.locations.get(index) else {
            return;
        };
        let path = if location.kind == LocationKind::Volume {
            self.panes[pane.index()]
                .volume_paths
                .get(&location.path)
                .unwrap_or(&location.path)
        } else {
            &location.path
        }
        .clone();
        self.navigate(pane, path, None, send);
    }

    pub(crate) fn drive_letter(
        &mut self,
        pane: PaneId,
        letter: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let mut chars = letter.chars();
        let Some(letter) = chars.next().filter(char::is_ascii_alphabetic) else {
            return false;
        };
        if chars.next().is_some() {
            return false;
        }
        if let Some(index) = self.navigation_choices().iter().position(|choice| {
            choice
                .location
                .drive_letter
                .is_some_and(|drive| drive.eq_ignore_ascii_case(&letter))
        }) {
            let token = self.navigation_choice_token(index);
            self.choose_navigation_choice(pane, &token, send);
            return true;
        }
        false
    }

    pub(crate) fn folder_sizes(&mut self, pane: PaneId, send: &dyn Fn(Command) -> bool) {
        if self.panes[pane.index()].tree_view().is_some() {
            self.tree_folder_sizes(pane, send);
            return;
        }
        if self.panes[pane.index()].archive.is_some() {
            self.archive_folder_sizes(pane, send);
            return;
        }
        if !self.native_action_allowed(pane) {
            return;
        }
        let p = &mut self.panes[pane.index()];
        if let Some(scan) = p.size_scan {
            if send(Command::CancelFolderSizes { pane, scan }) {
                self.desktop.status = "Cancelling folder size scan…".into();
            } else {
                self.desktop.report(
                    "Folder size queue busy",
                    "The scan is still running. Try cancelling again.".into(),
                );
            }
            return;
        }
        if p.loading || p.successful_generation.is_none() {
            return;
        }
        self.request_selection_action(
            pane,
            selection_actions::SelectionIntent::FolderSizes(pane),
            send,
        );
    }

    pub(super) fn start_captured_folder_sizes(
        &mut self,
        pane: PaneId,
        selection: Arc<SelectionSnapshot>,
        send: &dyn Fn(Command) -> bool,
    ) {
        if !selection.is_empty() && selection.folder_count() == 0 {
            self.desktop.status = "No selected directories to measure.".into();
            return;
        }
        let selection = (!selection.is_empty()).then_some(selection);
        let p = &mut self.panes[pane.index()];
        let Some(navigation) = p.successful_generation.filter(|_| !p.loading) else {
            return;
        };
        let Some(generation) = p.size_scan_generation.checked_add(1) else {
            self.desktop.report(
                "Folder size scan unavailable",
                "The scan identifier limit was reached. Restart the application.".into(),
            );
            return;
        };
        p.size_scan_generation = generation;
        let scan = FolderSizeScanId(generation);
        if send(Command::ComputeFolderSizes {
            pane,
            navigation,
            scan,
            selection: selection.clone(),
        }) {
            if selection.is_none() {
                p.folder_column_samples = SizeColumnSamples::default();
                self.retired.push(Box::new(mem::take(&mut p.folder_sizes)));
                // A full scan covers every folder listed now.
                p.size_scan_attempted = true;
                self.retired
                    .push(Box::new(mem::take(&mut p.size_scan_new_folders)));
            }
            self.retired
                .push(Box::new(mem::take(&mut p.entry_size_scans)));
            p.size_scan = Some(scan);
            p.cancelled_size_scan = None;
            self.retired
                .push(Box::new(p.size_scan_scope.replace(SizeScanScope {
                    navigation,
                    path: p.path.clone(),
                    entries: p.entries.clone(),
                    index: p.index.clone(),
                    selection: selection.clone(),
                })));
        } else {
            self.desktop
                .report("Folder size queue busy", "Try scanning again.".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROW: IconPlace = IconPlace::Entry(PaneId::Left);
    const TILE: IconPlace = IconPlace::Entry(PaneId::Right);

    /// A left pane of rows beside a right pane of tiles.
    fn rows_and_tiles() -> DesktopState {
        DesktopState {
            icon_view: IconView {
                edges: IconEdges {
                    entry: 20,
                    tile: 96,
                    location: 26,
                },
                tiles: [false, true],
            },
            ..DesktopState::default()
        }
    }

    /// An opaque bitmap whose width tells the tests which one was chosen.
    fn pixels(width: u32) -> IconPixels {
        IconPixels::new(width, 1, vec![255; width as usize * 4]).unwrap()
    }

    fn width(image: Image) -> u32 {
        image.size().width
    }

    #[test]
    fn archive_art_replaces_only_generic_document_icons() {
        let mut state = DesktopState::default();
        let mut generic = IconPixels::new(1, 1, vec![30, 60, 90, 255]).unwrap();
        generic.generic = true;
        state.receive_image(IconKey::file(), &generic);
        let mut archive_key = IconKey::file();
        archive_key.extension = Some("whl".into());
        assert_eq!(width(state.entry_image(&archive_key, ROW, true)), 0);
        assert_eq!(width(state.entry_image(&archive_key, ROW, false)), 1);
        let artwork = IconPixels::new(2, 2, vec![200; 16]).unwrap();
        state.receive_image(archive_key.clone(), &artwork);
        assert_eq!(width(state.entry_image(&archive_key, ROW, true)), 2);
        assert_eq!(width(state.image(&IconKey::file(), ROW)), 1);
    }

    #[test]
    fn image_cache_remains_bounded_and_preserves_generic_fallbacks() {
        let mut state = DesktopState::default();
        state.receive_image(IconKey::folder(), &pixels(1));
        state.receive_image(IconKey::file(), &pixels(1));
        for extension in 0..300 {
            state.receive_image(
                IconKey {
                    extension: Some(format!("ext{extension}").into()),
                    ..IconKey::file()
                },
                &pixels(1),
            );
        }
        assert_eq!(state.images.len(), IMAGE_CACHE_CAPACITY);
        assert_eq!(state.image_order.len(), IMAGE_CACHE_CAPACITY);
        assert_eq!(state.image_bytes, IMAGE_CACHE_CAPACITY * 4);
        assert_eq!(width(state.image(&IconKey::folder(), ROW)), 1);
        let unseen = IconKey {
            extension: Some("unseen".into()),
            ..IconKey::file()
        };
        assert_eq!(width(state.image(&unseen, ROW)), 1);
        state.receive_image(
            IconKey::drive(),
            &IconPixels {
                width: 257,
                height: 1,
                rgba: Arc::from(vec![0; 1028]),
                generic: false,
            },
        );
        assert_eq!(state.images.len(), IMAGE_CACHE_CAPACITY);
        assert_eq!(
            width(state.image(&IconKey::drive(), IconPlace::Location)),
            0
        );
    }

    #[test]
    fn each_place_gets_the_bitmap_of_its_edge_or_the_nearest_substitute() {
        let mut state = rows_and_tiles();
        let folder = IconKey::folder();
        // Until the exact bitmap arrives, the only delivered one stands in.
        state.receive_image(folder.clone().sized(96), &pixels(96));
        assert_eq!(width(state.image(&folder, ROW)), 96);
        state.receive_image(folder.clone().sized(20), &pixels(20));
        state.receive_image(folder.clone().sized(26), &pixels(26));
        assert_eq!(width(state.image(&folder, ROW)), 20);
        assert_eq!(width(state.image(&folder, TILE)), 96);
        assert_eq!(width(state.image(&folder, IconPlace::Location)), 26);
        // Actions ask for the default edge: the next larger bitmap scales down.
        assert_eq!(width(state.image(&folder, IconPlace::Action)), 96);
        // A lookup names the icon whatever edge its key carries.
        assert_eq!(width(state.image(&folder.clone().sized(7), ROW)), 20);

        // A location without art of its own uses folder art of the location edge,
        // and a type without art uses the generic file icon of the place's edge.
        let home = IconKey {
            kind: IconKind::Location(LocationKind::Home),
            ..IconKey::folder()
        };
        assert_eq!(width(state.image(&home, IconPlace::Location)), 26);
        state.receive_image(IconKey::file().sized(20), &pixels(20));
        state.receive_image(IconKey::file().sized(96), &pixels(96));
        let text = IconKey {
            extension: Some("txt".into()),
            ..IconKey::file()
        };
        assert_eq!(width(state.image(&text, ROW)), 20);
        assert_eq!(width(state.image(&text, TILE)), 96);
    }

    #[test]
    fn a_single_size_provider_serves_every_place_from_its_one_bitmap() {
        let mut state = rows_and_tiles();
        state.receive_image(IconKey::folder(), &pixels(32));
        for place in [ROW, TILE, IconPlace::Location, IconPlace::Action] {
            assert_eq!(
                width(state.image(&IconKey::folder(), place)),
                32,
                "{place:?}"
            );
        }
        // Its bitmap is not at a drawn edge, and is still never discarded.
        state.receive_image(IconKey::folder(), &pixels(5));
        assert_eq!(width(state.image(&IconKey::folder(), ROW)), 32);
        assert_eq!(state.image_bytes, 32 * 4);
    }

    #[test]
    fn bitmaps_of_edges_no_longer_drawn_are_replaced_by_the_new_ones() {
        let mut state = DesktopState::default();
        let file = IconKey::file();
        // Startup: the default edge, before the window reported its own.
        state.receive_image(file.clone(), &pixels(32));
        state.icon_view = rows_and_tiles().icon_view;
        assert_eq!(width(state.image(&file, ROW)), 32, "stands in meanwhile");
        state.receive_image(file.clone().sized(20), &pixels(20));
        assert_eq!(state.images[&file].len(), 1, "the stand-in is dropped");
        state.receive_image(file.clone().sized(96), &pixels(96));
        assert_eq!(width(state.image(&file, ROW)), 20);
        assert_eq!(width(state.image(&file, TILE)), 96);
        assert_eq!(state.image_bytes, (20 + 96) * 4);

        // A denser display: the old bitmaps serve until the new ones arrive.
        state.icon_view.edges = IconEdges {
            entry: 25,
            tile: 120,
            location: 33,
        };
        assert_eq!(width(state.image(&file, ROW)), 96);
        state.receive_image(file.clone().sized(25), &pixels(25));
        assert_eq!(width(state.image(&file, ROW)), 25);
        assert_eq!(width(state.image(&file, TILE)), 25, "the only bitmap left");
        state.receive_image(file.clone().sized(120), &pixels(120));
        assert_eq!(width(state.image(&file, TILE)), 120);
        assert_eq!(state.image_bytes, (25 + 120) * 4);
        assert_eq!(state.image_order.len(), 1);
    }

    #[test]
    fn a_repeated_delivery_keeps_the_first_image() {
        let mut state = rows_and_tiles();
        let key = IconKey::folder().sized(20);
        state.receive_image(key.clone(), &pixels(20));
        let first = state.image(&IconKey::folder(), ROW);
        state.receive_image(key, &pixels(20));
        assert!(
            crate::presentation_eq::same_image(&first, &state.image(&IconKey::folder(), ROW)),
            "models would be published again for an identical icon"
        );
        assert_eq!(state.image_bytes, 20 * 4);
    }

    #[test]
    fn every_icon_the_backend_remembers_fits_under_the_byte_ceiling() {
        // A file type is drawn at two edges, as a row and as a tile. The backend
        // never resends a remembered icon, so none of them may be dropped.
        let largest = usize::from(MAX_ICON_EDGE).pow(2) * 4;
        assert!(IMAGE_CACHE_BYTES >= midnight_core::DESKTOP_ICON_CAPACITY * 2 * largest);

        // The largest tiles: a full cache keeps its first icon.
        let mut state = rows_and_tiles();
        state.icon_view.edges.tile = MAX_ICON_EDGE;
        let edge = u32::from(MAX_ICON_EDGE);
        let tile = IconPixels::new(edge, edge, vec![255; largest]).unwrap();
        let typed = |index: usize| IconKey {
            extension: Some(format!("type{index}").into()),
            ..IconKey::file()
        };
        for index in 0..IMAGE_CACHE_CAPACITY {
            state.receive_image(typed(index).sized(20), &pixels(20));
            state.receive_image(typed(index).sized(MAX_ICON_EDGE), &tile);
        }
        assert_eq!(state.images.len(), IMAGE_CACHE_CAPACITY);
        assert_eq!(state.image_bytes, IMAGE_CACHE_CAPACITY * (largest + 20 * 4));
        for index in [0, IMAGE_CACHE_CAPACITY - 1] {
            assert_eq!(width(state.image(&typed(index), ROW)), 20, "{index}");
            assert_eq!(width(state.image(&typed(index), TILE)), edge, "{index}");
        }
    }

    #[test]
    fn trimming_drops_the_oldest_specific_icons_first_and_keeps_the_count_exact() {
        let mut state = rows_and_tiles();
        let typed = |index: usize| IconKey {
            extension: Some(format!("type{index}").into()),
            ..IconKey::file()
        };
        state.receive_image(IconKey::folder().sized(96), &pixels(96));
        for index in 0..10 {
            state.receive_image(typed(index).sized(20), &pixels(20));
            state.receive_image(typed(index).sized(96), &pixels(96));
        }
        let each = (20 + 96) * 4;
        assert_eq!(state.image_bytes, 96 * 4 + 10 * each);
        // Room for the folder and three types: the seven oldest types go, with
        // all their bitmaps, and the icon just delivered is never the victim.
        state.trim(&typed(0), 96 * 4 + 3 * each);
        assert_eq!(state.image_bytes, 96 * 4 + 3 * each);
        assert_eq!(state.images.len(), 4);
        assert_eq!(state.image_order.len(), 4);
        assert_eq!(width(state.image(&IconKey::folder(), TILE)), 96);
        for kept in [0, 8, 9] {
            assert_eq!(width(state.image(&typed(kept), ROW)), 20, "{kept}");
        }
        for dropped in 1..8 {
            assert_eq!(width(state.image(&typed(dropped), TILE)), 0, "{dropped}");
        }
        // Nothing but the kept icon is left to drop: the loop ends.
        state.trim(&typed(0), 0);
        assert_eq!(state.images.len(), 1);
        assert_eq!(state.image_bytes, each);
    }

    #[test]
    fn icons_are_uploaded_premultiplied() {
        let mut state = DesktopState::default();
        let translucent = IconPixels::new(2, 1, vec![200, 100, 50, 128, 255, 255, 255, 0]).unwrap();
        state.receive_image(IconKey::folder(), &translucent);
        let image = state.image(&IconKey::folder(), ROW);
        let buffer = image.to_rgba8_premultiplied().unwrap();
        let stored: Vec<[u8; 4]> = buffer
            .as_slice()
            .iter()
            .map(|pixel| [pixel.r, pixel.g, pixel.b, pixel.a])
            .collect();
        assert_eq!(stored, [[100, 50, 25, 128], [0, 0, 0, 0]]);
    }
}
