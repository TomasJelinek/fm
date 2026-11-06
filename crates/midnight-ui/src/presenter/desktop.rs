//! Native desktop requests remain messages; this module performs no I/O.
use super::*;
use slint::{Image, Rgba8Pixel, SharedPixelBuffer};
use std::collections::VecDeque;

const IMAGE_CACHE_CAPACITY: usize = 256;

#[derive(Default)]
pub(crate) struct DesktopState {
    pub(crate) shell: Option<ShellFlavor>,
    pub(crate) locations: Vec<NavigationLocation>,
    pub(crate) error: String,
    pub(crate) status: String,
    pub(crate) notice: Option<(String, String)>,
    pub(crate) notice_generation: i32,
    pub(crate) terminal_pending: Option<DesktopRequestId>,
    pub(crate) terminal_mode: TerminalMode,
    pub(crate) draft_nonempty: bool,
    pub(crate) draft_shell: Option<ShellFlavor>,
    pub(crate) command_history_cursor: Option<String>,
    pub(crate) terminal_guidance: Option<&'static str>,
    images: HashMap<IconKey, Image>,
    image_order: VecDeque<IconKey>,
    pub(super) requests: HashSet<DesktopRequestId>,
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
        self.notice_generation = self.notice_generation.wrapping_add(1);
        self.notice = Some((title.into(), body));
    }
    pub(crate) fn image(&self, key: &IconKey) -> Image {
        self.images
            .get(key)
            .or_else(|| {
                self.images.get(&IconKey {
                    kind: if matches!(key.kind, IconKind::Location(_)) {
                        IconKind::Folder
                    } else {
                        key.kind
                    },
                    extension: None,
                    drive_root: None,
                    size: key.size,
                })
            })
            .cloned()
            .unwrap_or_default()
    }
    fn receive_image(&mut self, key: IconKey, pixels: &IconPixels) {
        // Validate again at the UI boundary before creating a Slint image. Models
        // clone these handles; row_data never decodes or constructs an image.
        if self.images.contains_key(&key)
            || pixels.width == 0
            || pixels.height == 0
            || pixels.width > 64
            || pixels.height > 64
            || pixels.rgba.len() != pixels.width as usize * pixels.height as usize * 4
        {
            return;
        }
        if self.images.len() == IMAGE_CACHE_CAPACITY {
            let victim = self
                .image_order
                .iter()
                .position(|key| key.extension.is_some())
                .unwrap_or(0);
            if let Some(old) = self.image_order.remove(victim) {
                self.images.remove(&old);
            }
        }
        let image = Image::from_rgba8(SharedPixelBuffer::<Rgba8Pixel>::clone_from_slice(
            &pixels.rgba,
            pixels.width,
            pixels.height,
        ));
        self.image_order.push_back(key.clone());
        self.images.insert(key, image);
    }
}

impl Presenter {
    pub(super) fn apply_desktop(&mut self, event: Event) -> Option<Event> {
        match event {
            Event::FontFamiliesLoaded(result) => self.apply_font_families(result),
            Event::PreferencesLoaded(result) => self.preferences_loaded(result),
            Event::PreferencesSaved { revision, result } => {
                self.preferences_saved(revision, result)
            }
            // The app coordinates watch hints through refresh_if_changed so
            // they cannot trigger nested sends while apply owns this state.
            Event::DirectoryChanged { .. } => {}
            Event::DesktopContext { shell, locations } => {
                self.desktop.shell = Some(shell);
                self.track_draft(self.desktop.draft_nonempty);
                self.retired.push(Box::new(mem::replace(
                    &mut self.desktop.locations,
                    locations,
                )));
                self.desktop.error.clear();
                self.sync_navigation_choices();
            }
            Event::DesktopContextFailed(error) => {
                self.desktop.error = error.to_string();
            }
            Event::IconLoaded { key, pixels } => {
                self.desktop.receive_image(key, &pixels);
                self.retired.push(Box::new(pixels));
            }
            Event::DesktopActionFinished { request, result } => {
                if !self.desktop.requests.remove(&request) {
                    return None;
                }
                if self.desktop.terminal_pending == Some(request) {
                    self.desktop.terminal_pending = None;
                }
                let opened = self.desktop.open_requests.remove(&request);
                match result {
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
                    Ok(DesktopActionResult::PropertiesOpened) => {
                        self.desktop.status = "Properties opened".into()
                    }
                    Ok(DesktopActionResult::PropertiesInfo { title, body }) => {
                        self.desktop.report(&title, body)
                    }
                    Ok(DesktopActionResult::FileOpened) => {
                        if opened.is_some() {
                            self.desktop.status = "File opened".into();
                        }
                    }
                    Ok(DesktopActionResult::OpenDirectory {
                        pane,
                        navigation,
                        path,
                    }) => {
                        if opened == Some((pane, navigation, path.clone()))
                            && self.panes[pane.index()].generation == navigation.0
                        {
                            self.desktop.open_directories[pane.index()] = Some((navigation, path));
                        }
                    }
                    Err(error) => self
                        .desktop
                        .report("Desktop action failed", error.to_string()),
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
                if p.size_scan == Some(scan)
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
                {
                    p.folder_sizes.insert(path.clone(), size);
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
                    self.retired.push(Box::new(p.size_scan_scope.take()));
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
        let request = self.desktop.request();
        let navigation = NavigationId(self.panes[pane.index()].generation);
        if !send(Command::OpenEntry {
            request,
            pane,
            navigation,
            path: path.clone(),
        }) {
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
        }) {
            self.retired.push(Box::new(mem::take(&mut p.folder_sizes)));
            self.retired
                .push(Box::new(mem::take(&mut p.entry_size_scans)));
            p.size_scan = Some(scan);
            p.size_scan_scope = Some(SizeScanScope {
                navigation,
                path: p.path.clone(),
                entries: p.entries.clone(),
                index: p.index.clone(),
            });
            p.size_scan_attempted = true;
        } else {
            self.desktop
                .report("Folder size queue busy", "Try scanning again.".into());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_cache_remains_bounded_and_preserves_generic_fallbacks() {
        let mut state = DesktopState::default();
        let pixels = IconPixels::new(1, 1, vec![30, 60, 90, 255]).unwrap();
        state.receive_image(IconKey::folder(), &pixels);
        state.receive_image(IconKey::file(), &pixels);
        for extension in 0..300 {
            state.receive_image(
                IconKey {
                    extension: Some(format!("ext{extension}").into()),
                    ..IconKey::file()
                },
                &pixels,
            );
        }
        assert!(state.images.len() <= IMAGE_CACHE_CAPACITY);
        assert_eq!(state.image(&IconKey::folder()).size().width, 1);
        assert_eq!(
            state
                .image(&IconKey {
                    extension: Some("unseen".into()),
                    ..IconKey::file()
                })
                .size()
                .width,
            1
        );
        let before = state.images.len();
        state.receive_image(
            IconKey::drive(),
            &IconPixels {
                width: 65,
                height: 1,
                rgba: Arc::from(vec![0; 260]),
            },
        );
        assert_eq!(state.images.len(), before);
        assert_eq!(state.image(&IconKey::drive()).size().width, 0);
    }
}
