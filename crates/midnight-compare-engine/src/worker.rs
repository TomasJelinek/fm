use crate::file_choice::{self, Picker, resolve};
use crate::{
    FileChoice, ImageProviders, Input, Mailbox, NativeCapture, Output, StorageHint, check_cancel,
    images::Images, text::TextComparison,
};
use midnight_core::{FileCompareLaunch, ViewerAppearance, ViewerGeometry};
use midnight_plugin_host::CapturedFile;
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, TryRecvError},
    },
    time::Duration,
};

/// While the font catalog is pending, inputs are awaited at most this long between checks.
const FONT_POLL: Duration = Duration::from_millis(20);

enum Content {
    Text(Box<TextComparison>),
    Images(Box<Images>),
}

struct Viewport {
    geometry: ViewerGeometry,
    revision: u64,
}

pub(crate) struct Capture<'a> {
    pub factory: &'a NativeCapture,
    pub storage: Option<&'a StorageHint>,
    pub providers: &'a ImageProviders,
    pub cancel: &'a Arc<AtomicBool>,
}

/// The installed fixed-pitch families are scanned on their own thread: the scan reads
/// every installed family and can take seconds on a loaded machine, so the first frame
/// is painted in the generic monospace family instead of waiting for it.
struct FontCatalog(Receiver<Option<Arc<[String]>>>);
impl FontCatalog {
    fn start(cancel: Arc<AtomicBool>) -> Self {
        let (sender, receiver) = mpsc::sync_channel(1);
        let scan = sender.clone();
        let spawned = std::thread::Builder::new()
            .name("comparator-fonts".into())
            .spawn(move || {
                let _ = scan.send(midnight_viewer_render::monospace_families(|| {
                    cancel.load(Ordering::Acquire)
                }));
            });
        if spawned.is_err() {
            // Without a thread the comparison keeps the generic family and no chooser list.
            let _ = sender.send(Some(Arc::from([])));
        }
        Self(receiver)
    }
}

impl Capture<'_> {
    fn open(&self, path: &std::path::Path) -> Result<CapturedFile, String> {
        check_cancel(self.cancel)?;
        let result = (self.factory)(path, self.cancel);
        check_cancel(self.cancel)?;
        result
    }

    /// Only a positive local-SSD answer admits this source's reads as shared I/O.
    fn confirmed_ssd(&self, path: &std::path::Path) -> bool {
        self.storage.is_some_and(|storage| storage(path))
    }

    /// Image viewers are optional: a missing registered runtime cannot disable text diff.
    /// Like the viewer service, discovery runs once; opening a session revalidates packages.
    fn image_providers(&self, output: &Mailbox) -> Arc<[midnight_image_viewer_host::Provider]> {
        let mut cached = self
            .providers
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        cached
            .get_or_insert_with(|| {
                output.put(Output::Status("Checking installed image viewers…".into()));
                midnight_plugin_host::installed_executable()
                    .ok()
                    .and_then(|executable| midnight_image_viewer_host::discover(&executable).ok())
                    .unwrap_or_default()
                    .into()
            })
            .clone()
    }
}

#[cfg(test)]
#[path = "worker_tests.rs"]
mod tests;

pub(crate) fn run(
    launch: FileCompareLaunch,
    appearance: ViewerAppearance,
    geometry: ViewerGeometry,
    receiver: Receiver<Input>,
    output: &Mailbox,
    capture: Capture<'_>,
    picker: Picker,
) -> Result<(), String> {
    let cancel = capture.cancel;
    check_cancel(cancel)?;
    output.put(Output::Status("Preparing file identities…".into()));
    let mut pair = resolve(&launch, cancel);
    let mut viewport = Viewport {
        geometry,
        revision: 0,
    };
    loop {
        check_cancel(cancel)?;
        pair = match pair {
            Ok(paths) => {
                let result = file_choice::validate(&paths).and_then(|()| {
                    compare(
                        &paths,
                        &launch.disabled_plugins,
                        &appearance,
                        &mut viewport,
                        &receiver,
                        output,
                        &capture,
                    )
                });
                match result {
                    Ok(()) => return Ok(()),
                    Err(message) => Err(FileChoice {
                        paths,
                        message,
                        is_error: true,
                    }),
                }
            }
            Err(choice) => {
                output.put(Output::ChooseFiles(choice));
                // Session::close sets the cancellation flag before it queues Close, so a
                // blocking receive needs no polling interval.
                loop {
                    check_cancel(cancel)?;
                    match receiver.recv() {
                        Ok(Input::Close) | Err(_) => return Ok(()),
                        Ok(Input::Compare(paths)) => break Ok(paths),
                        Ok(Input::Browse { side, paths, owner }) => {
                            break Err(picker.browse(side, paths, owner));
                        }
                        Ok(Input::Resize(value, revision)) => {
                            viewport.geometry = value;
                            viewport.revision = revision;
                        }
                        _ => {}
                    }
                }
            }
        };
    }
}

fn compare(
    paths: &[std::path::PathBuf; 2],
    disabled_plugins: &[String],
    appearance: &ViewerAppearance,
    viewport: &mut Viewport,
    receiver: &Receiver<Input>,
    output: &Mailbox,
    capture: &Capture<'_>,
) -> Result<(), String> {
    let cancel = capture.cancel;
    let left = capture.open(&paths[0]).map_err(|error| {
        format!(
            "File A could not be opened: {error}\n{}",
            paths[0].display()
        )
    })?;
    let right = capture.open(&paths[1]).map_err(|error| {
        format!(
            "File B could not be opened: {error}\n{}",
            paths[1].display()
        )
    })?;
    let sources = [left, right];
    let ssd = paths.each_ref().map(|path| capture.confirmed_ssd(path));
    check_cancel(cancel)?;
    let providers = capture.image_providers(output);
    check_cancel(cancel)?;
    let selected = paths.each_ref().map(|path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| {
                providers.iter().find(|provider| {
                    provider.supports_extension(extension)
                        && !disabled_plugins.contains(&provider.id)
                })
            })
    });
    if selected
        .iter()
        .flatten()
        .any(|provider| provider.is_document())
    {
        return Err(
            "Document comparison is not supported. Open PDFs in the document viewer instead."
                .into(),
        );
    }
    let mut content = match selected {
        [Some(left), Some(right)] => {
            output.put(Output::Status(
                "Decoding images in restricted workers…".into(),
            ));
            Content::Images(Box::new(Images::open(
                [left, right],
                &sources,
                ssd,
                cancel,
            )?))
        }
        [None, None] => {
            output.put(Output::Status("Reading and comparing text…".into()));
            let left = crate::text::read(&sources[0], ssd[0], cancel)?;
            let right = crate::text::read(&sources[1], ssd[1], cancel)?;
            let text = TextComparison::new(&left, &right, cancel)?;
            Content::Text(Box::new(text))
        }
        _ => return Err("Select two supported images or two text files.".into()),
    };
    viewport.geometry = bounded(viewport.geometry);
    let mut fonts =
        matches!(content, Content::Text(_)).then(|| FontCatalog::start(Arc::clone(cancel)));
    let mut dirty = true;
    loop {
        check_cancel(cancel)?;
        if let Some(catalog) = &fonts {
            match catalog.0.try_recv() {
                Ok(Some(families)) => {
                    if let Content::Text(text) = &mut content {
                        // Repaint in the resolved family and publish the chooser list.
                        text.set_font_catalog(families, &appearance.family);
                        dirty = true;
                    }
                    fonts = None;
                }
                // Cancelled scans end the comparison at the next check; a failed scan
                // keeps the generic family without a chooser list.
                Ok(None) | Err(TryRecvError::Disconnected) => fonts = None,
                Err(TryRecvError::Empty) => {}
            }
        }
        if dirty {
            for source in &sources {
                source
                    .revalidate()
                    .map_err(|error| format!("Compared file changed: {error}"))?;
            }
            let mut frame = match &mut content {
                Content::Text(text) => text.paint(viewport.geometry, appearance, cancel)?,
                Content::Images(images) => images.paint(viewport.geometry, cancel)?,
            };
            frame.geometry_revision = viewport.revision;
            check_cancel(cancel)?;
            for source in &sources {
                source
                    .revalidate()
                    .map_err(|error| format!("Compared file changed: {error}"))?;
            }
            check_cancel(cancel)?;
            output.put(Output::Frame {
                frame,
                names: paths.each_ref().map(|path| path.display().to_string()),
            });
            dirty = false;
        }
        // Idle comparisons block until input, Close or the session's disconnection; only a
        // pending font catalog bounds the wait.
        let input = if fonts.is_some() {
            match receiver.recv_timeout(FONT_POLL) {
                Err(RecvTimeoutError::Timeout) => continue,
                result => result.map_err(|_| ()),
            }
        } else {
            receiver.recv().map_err(|_| ())
        };
        match input {
            Ok(Input::Close) | Err(()) => break,
            Ok(input) => {
                dirty |= apply(
                    input,
                    &mut content,
                    &mut viewport.geometry,
                    &mut viewport.revision,
                )
            }
        }
        for input in receiver.try_iter().take(31) {
            if matches!(input, Input::Close) {
                return Ok(());
            }
            dirty |= apply(
                input,
                &mut content,
                &mut viewport.geometry,
                &mut viewport.revision,
            );
        }
    }
    Ok(())
}

fn apply(
    input: Input,
    content: &mut Content,
    geometry: &mut ViewerGeometry,
    geometry_revision: &mut u64,
) -> bool {
    if let Input::Resize(value, revision) = input {
        *geometry = bounded(value);
        *geometry_revision = revision;
        if let Content::Images(images) = content {
            images.invalidate();
        }
        return true;
    }
    match (content, input) {
        (Content::Text(text), Input::FontFamily(family)) => return text.change_font(&family),
        (Content::Text(text), Input::Scroll { x, y }) if x.is_finite() && y.is_finite() => {
            text.scroll_pixels(y);
            text.horizontal = (text.horizontal - x).clamp(0.0, 262144.0);
        }
        (Content::Text(text), Input::Lines(lines)) => text.scroll(i64::from(lines)),
        (Content::Text(text), Input::Pages(pages)) => text.scroll_pages(pages),
        (Content::Text(text), Input::Seek(value)) => text.seek(value),
        (Content::Text(text), Input::Difference { backwards }) => text.difference(backwards),
        (Content::Text(text), Input::DifferenceEdge { last }) => text.difference_edge(last),
        (Content::Images(images), Input::Mode(mode)) => images.set_mode(mode),
        (Content::Images(images), Input::Zoom(factor)) => images.zoom(factor),
        (Content::Images(images), Input::ActualPixels) => images.actual(),
        (Content::Images(images), Input::Fit) => images.fit(),
        (Content::Images(images), Input::Scroll { x, y }) => {
            images.pan(x * geometry.scale, y * geometry.scale)
        }
        // A key line pans 24 logical pixels; a page pans one viewport height.
        (Content::Images(images), Input::Lines(lines)) => {
            images.pan(0.0, -(lines as f32) * 24.0 * geometry.scale)
        }
        (Content::Images(images), Input::Pages(pages)) => {
            images.pan(0.0, -(pages as f32) * geometry.height as f32)
        }
        _ => return false,
    }
    true
}

fn bounded(mut geometry: ViewerGeometry) -> ViewerGeometry {
    geometry.width = geometry.width.clamp(2, 4096);
    geometry.height = geometry.height.clamp(1, 4096);
    geometry.scale = if geometry.scale.is_finite() {
        geometry.scale.clamp(0.5, 4.0)
    } else {
        1.0
    };
    let pixels = f64::from(geometry.width) * f64::from(geometry.height);
    if pixels > 4_194_304.0 {
        let reduction = (4_194_304.0 / pixels).sqrt();
        geometry.width = (f64::from(geometry.width) * reduction).max(2.0) as u32;
        geometry.height = (f64::from(geometry.height) * reduction).max(1.0) as u32;
        geometry.scale *= reduction as f32;
    }
    geometry
}
