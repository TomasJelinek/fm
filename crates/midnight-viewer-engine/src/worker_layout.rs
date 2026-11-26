//! Retain one resolved source/route while its frontend establishes the canvas.
use super::*;

pub(super) struct PreparedOpening {
    pub file: midnight_plugin_host::CapturedFile,
    pub route: crate::routing::Route,
    /// An image provider started while the frontend creates its canvas. Decoding
    /// needs no viewport, so it overlaps native window and swapchain creation;
    /// the first viewport is still requested only after the acknowledgement.
    pub image: Option<crate::image_worker::ImageWorker>,
    /// A video helper started likewise: it starts and loads its decoders meanwhile, and
    /// opens the video when the canvas size is known.
    pub video: Option<VideoView>,
    /// The helper that opens a video first, chosen by its container.
    pub video_first: VideoWorker,
}

impl Worker {
    fn video_process_index(&self) -> usize {
        self.images.len() + self.web_providers.len()
    }

    pub(super) fn invalidate_layout(&mut self) {
        self.layout.invalidate();
        self.retire_pending_layout();
    }

    /// A started image provider closes like a displayed one: the next opening waits
    /// for its actor instead of joining it on this worker.
    pub(super) fn retire_pending_layout(&mut self) {
        let Some(opening) = self.pending_layout.take() else {
            return;
        };
        if let Some(mut image) = opening.image {
            image.cancel();
            self.closing_image = Some(image);
        }
        if let Some(mut video) = opening.video {
            video.cancel();
            self.closing_video = Some(video);
        }
    }

    fn image_process(&self, route: crate::routing::Route) -> Option<usize> {
        match route {
            crate::routing::Route::Process { index } if index < self.images.len() => Some(index),
            _ => None,
        }
    }

    /// The HTML provider's index among process candidates, which also renders Office.
    fn office_renderer(&self) -> Option<usize> {
        self.web_providers
            .iter()
            .position(|provider| {
                provider.id == crate::routing::HTML_ID
                    && !self.disabled_plugins.contains(&provider.id)
            })
            .map(|index| index + self.images.len())
    }

    /// The viewer that would show `path`, by its name.
    pub(super) fn route(&self, path: &std::path::Path) -> Option<crate::routing::Route> {
        crate::routing::choose(
            self.runtime.catalog(),
            self.images
                .iter()
                .map(|provider| crate::routing::ProcessCandidate {
                    id: &provider.id,
                    extensions: &provider.extensions,
                })
                .chain(
                    self.web_providers
                        .iter()
                        .map(|provider| crate::routing::ProcessCandidate {
                            id: &provider.id,
                            extensions: &provider.extensions,
                        }),
                )
                .chain(self.video_installation.iter().map(|installation| {
                    crate::routing::ProcessCandidate {
                        id: VIDEO_PROVIDER_ID,
                        extensions: installation.extensions(),
                    }
                })),
            path,
            self.basic,
            &self.disabled_plugins,
            &self.associations,
            self.office_renderer(),
        )
    }

    pub(super) fn prepare_opening(
        &mut self,
        path: &std::path::Path,
        file: midnight_plugin_host::CapturedFile,
    ) -> Result<PreparedOpening, String> {
        self.link_document(&file);
        let office_renderer = self.office_renderer();
        let route = self.route(path);
        if office_renderer.is_none()
            && crate::routing::office_requested(
                path,
                self.basic,
                &self.disabled_plugins,
                &self.associations,
            )
            && route.is_none_or(|route| match route {
                crate::routing::Route::Native { index, .. } => {
                    let plugin = &self.runtime.catalog()[index];
                    plugin.extensions.is_empty()
                        && self
                            .associations
                            .suffix_match(
                                &plugin.id,
                                &plugin.extensions,
                                path.file_name()
                                    .and_then(|name| name.to_str())
                                    .unwrap_or(""),
                            )
                            .is_none()
                }
                _ => false,
            })
        {
            return Err(
                "Office preview requires an installed and enabled Chromium HTML viewer.".into(),
            );
        }
        let route = route.ok_or_else(|| {
            "No compatible viewer plugin is installed beside the executable in viewers/.".to_owned()
        })?;
        let route = match route {
            crate::routing::Route::Office { .. } => {
                self.mode = VIEWER_MODE_HTML;
                self.modes = vec![VIEWER_MODE_HTML];
                self.binary = false;
                route
            }
            crate::routing::Route::Process { index } => {
                self.mode =
                    if self.video_installation.is_some() && index == self.video_process_index() {
                        VIEWER_MODE_VIDEO
                    } else if index >= self.images.len() {
                        VIEWER_MODE_HTML
                    } else {
                        VIEWER_MODE_IMAGE
                    };
                self.modes = vec![self.mode];
                self.binary = false;
                route
            }
            crate::routing::Route::Native {
                index,
                mut mode,
                detect_binary,
            } => {
                let plugin = &self.runtime.catalog()[index];
                if detect_binary {
                    mode = file
                        .preferred_basic_mode()
                        .map_err(|error| error.to_string())?;
                }
                if let Some(selected) = self
                    .selected_mode
                    .filter(|mode| plugin.modes.contains(mode))
                {
                    mode = selected;
                }
                self.mode = mode;
                self.modes = plugin.modes.clone();
                self.binary = mode == viewer::MODE_BINARY;
                self.length = file.length();
                crate::routing::Route::Native {
                    index,
                    mode,
                    detect_binary: false,
                }
            }
        };
        let video_first = self
            .video_installation
            .as_ref()
            .filter(|_| self.mode == VIEWER_MODE_VIDEO)
            .map_or(VideoWorker::Bundled, |installation| {
                installation.first_worker(path)
            });
        Ok(PreparedOpening {
            file,
            route,
            image: None,
            video: None,
            video_first,
        })
    }

    pub(super) fn select_opening(
        &mut self,
        opening: PreparedOpening,
        output: &Mailbox,
    ) -> Result<(), String> {
        if let Some(selected) = self.layout.select(self.mode, self.positions.generation)? {
            let mut opening = opening;
            if let Some(index) = self.image_process(opening.route) {
                opening.image = Some(crate::image_worker::ImageWorker::open(
                    self.images[index].clone(),
                    Arc::new(opening.file.clone()),
                    self.wake.clone(),
                    self.image_startup.clone(),
                )?);
            }
            if self.mode == VIEWER_MODE_VIDEO
                && let Some(installation) = self.video_installation.clone()
            {
                // A failure to start is reported when the opening starts it again.
                opening.video = VideoView::start(
                    installation,
                    opening.file.clone(),
                    self.positions.generation,
                    opening.video_first,
                    self.wake.clone(),
                    self.audio_volume.clone(),
                )
                .ok();
            }
            self.pending_layout = Some(opening);
            output.put(ViewerOutput::LayoutSelected {
                token: selected.token,
                mode: selected.mode,
                content_generation: selected.content_generation,
            });
            self.status("Opening…".into(), output);
            Ok(())
        } else {
            self.start_opening(opening, output)
        }
    }

    pub(super) fn take_layout_ready(
        &mut self,
        token: u64,
        geometry: ViewerGeometry,
    ) -> Option<PreparedOpening> {
        if geometry.width == 0
            || geometry.height == 0
            || !geometry.scale.is_finite()
            || geometry.scale <= 0.0
        {
            return None;
        }
        self.layout.accept(token)?;
        let opening = self.pending_layout.take()?;
        // Image/PDF Fit uses the raw physical canvas; text and HTML retain the
        // existing independently bounded surface and effective device scale.
        self.image_size = [geometry.width, geometry.height];
        self.geometry = bounded_geometry(geometry);
        Some(opening)
    }

    pub(super) fn start_opening(
        &mut self,
        opening: PreparedOpening,
        output: &Mailbox,
    ) -> Result<(), String> {
        let PreparedOpening {
            file,
            route,
            image,
            video,
            video_first,
        } = opening;
        self.chains.clear();
        self.session = match route {
            crate::routing::Route::Office { renderer } => {
                let provider = renderer
                    .checked_sub(self.images.len())
                    .and_then(|index| self.web_providers.get(index))
                    .filter(|provider| {
                        provider.id == crate::routing::HTML_ID
                            && !self.disabled_plugins.contains(&provider.id)
                    })
                    .ok_or("Office HTML renderer is no longer available.")?
                    .clone();
                self.open_office(provider, file, output)?;
                None
            }
            crate::routing::Route::Process { index }
                if self.video_installation.is_some() && index == self.video_process_index() =>
            {
                let installation = self
                    .video_installation
                    .clone()
                    .ok_or("Selected video viewer is unavailable.")?;
                let generation = self.positions.generation;
                let video = match video {
                    Some(video) if video.session_id() == MediaSessionId(generation) => Ok(video),
                    stale => {
                        if let Some(mut stale) = stale {
                            stale.cancel();
                            self.closing_video = Some(stale);
                        }
                        VideoView::start(
                            installation,
                            file,
                            generation,
                            video_first,
                            self.wake.clone(),
                            self.audio_volume.clone(),
                        )
                    }
                };
                match video {
                    Ok(mut video) => {
                        video.show(self.geometry.width, self.geometry.height);
                        output.put(ViewerOutput::MediaSnapshot(video.presentation()));
                        self.video = Some(video);
                    }
                    Err(error) => {
                        let mut snapshot = crate::video_actor::opening(MediaSessionId(generation));
                        snapshot.phase = MediaPhase::Failed(error.clone());
                        output.put(ViewerOutput::MediaSnapshot(snapshot));
                        return Err(error);
                    }
                }
                None
            }
            crate::routing::Route::Process { index } if index >= self.images.len() => {
                let provider = self
                    .web_providers
                    .get(index - self.images.len())
                    .ok_or("Selected HTML viewer is no longer available.")?
                    .clone();
                self.open_web(provider, file)?;
                None
            }
            crate::routing::Route::Process { index } => {
                let worker = match image {
                    Some(worker) => worker,
                    None => crate::image_worker::ImageWorker::open(
                        self.images[index].clone(),
                        Arc::new(file),
                        self.wake.clone(),
                        self.image_startup.clone(),
                    )?,
                };
                self.image = Some(crate::image_view::ImageView::new(
                    worker,
                    self.image_ids.clone(),
                ));
                None
            }
            crate::routing::Route::Native { index, mode, .. } => Some(
                self.runtime
                    .open(
                        &self.runtime.catalog()[index].id,
                        file,
                        self.generation,
                        mode,
                    )
                    .map_err(|error| error.to_string())?,
            ),
        };
        self.apply_font();
        if let Some(session) = &self.session {
            session.notify_on_ready(self.wake.clone());
        }
        self.status("Opening…".into(), output);
        Ok(())
    }
}
