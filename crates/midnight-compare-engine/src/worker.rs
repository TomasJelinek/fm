use crate::{Input, Mailbox, Output, check_cancel, images::Images, text::TextComparison};
use midnight_core::{
    EntryKind, FileCompareLaunch, FileCompareSource, ViewerAppearance, ViewerGeometry,
};
use midnight_plugin_host::CapturedFile;
use std::{
    sync::{
        atomic::AtomicBool,
        mpsc::{Receiver, RecvTimeoutError},
    },
    time::Duration,
};

enum Content {
    Text(Box<TextComparison>),
    Images(Box<Images>),
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
    cancel: &AtomicBool,
) -> Result<(), String> {
    output.put(Output::Status("Preparing file identities…".into()));
    let paths = resolve(&launch, cancel)?;
    check_cancel(cancel)?;
    let left = CapturedFile::capture(&paths[0])
        .map_err(|error| format!("{}: {error}", paths[0].display()))?;
    check_cancel(cancel)?;
    let right = CapturedFile::capture(&paths[1])
        .map_err(|error| format!("{}: {error}", paths[1].display()))?;
    let sources = [left, right];
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let providers = midnight_image_viewer_host::discover(&executable).unwrap_or_default();
    let selected = paths.each_ref().map(|path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| {
                providers.iter().find(|provider| {
                    provider.supports_extension(extension)
                        && !launch.disabled_plugins.contains(&provider.id)
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
            Content::Images(Box::new(Images::open([left, right], &sources, cancel)?))
        }
        [None, None] => {
            output.put(Output::Status("Reading and comparing text…".into()));
            let left = crate::text::read(&sources[0], cancel)?;
            let right = crate::text::read(&sources[1], cancel)?;
            Content::Text(Box::new(TextComparison::new(&left, &right, cancel)?))
        }
        _ => return Err("Select two supported images or two text files.".into()),
    };
    let mut geometry = bounded(geometry);
    let mut geometry_revision = 0;
    let mut dirty = true;
    loop {
        check_cancel(cancel)?;
        if dirty {
            for source in &sources {
                source
                    .revalidate()
                    .map_err(|error| format!("Compared file changed: {error}"))?;
            }
            let mut frame = match &mut content {
                Content::Text(text) => text.paint(geometry, &appearance, cancel)?,
                Content::Images(images) => images.paint(geometry, cancel)?,
            };
            frame.geometry_revision = geometry_revision;
            check_cancel(cancel)?;
            for source in &sources {
                source
                    .revalidate()
                    .map_err(|error| format!("Compared file changed: {error}"))?;
            }
            output.put(Output::Frame {
                frame,
                names: paths.each_ref().map(|path| path.display().to_string()),
            });
            dirty = false;
        }
        match receiver.recv_timeout(Duration::from_millis(30)) {
            Ok(Input::Close) | Err(RecvTimeoutError::Disconnected) => break,
            Err(RecvTimeoutError::Timeout) => continue,
            Ok(input) => dirty |= apply(input, &mut content, &mut geometry, &mut geometry_revision),
        }
        for input in receiver.try_iter().take(31) {
            if matches!(input, Input::Close) {
                return Ok(());
            }
            dirty |= apply(input, &mut content, &mut geometry, &mut geometry_revision);
        }
    }
    Ok(())
}

fn resolve(
    launch: &FileCompareLaunch,
    cancel: &AtomicBool,
) -> Result<[std::path::PathBuf; 2], String> {
    if launch.sources.len() > 2 {
        return Err("Select exactly two regular files.".into());
    }
    let mut paths = Vec::with_capacity(2);
    for source in &launch.sources {
        match source {
            FileCompareSource::Focused(path) => paths.push(path.clone()),
            FileCompareSource::Selected {
                entries,
                view,
                selection,
            } => {
                let mut selected = Vec::with_capacity(2);
                for entry in entries.iter() {
                    check_cancel(cancel)?;
                    if selection.contains_path(&entry.path)
                        && let Some(row) = view.by_id.get(&entry.id)
                    {
                        if entry.kind != EntryKind::File || entry.virtual_node.is_some() {
                            return Err("Comparison requires ordinary native files.".into());
                        }
                        selected.push((*row, entry.path.clone()));
                        if selected.len() > 2 {
                            return Err("Select exactly two regular files.".into());
                        }
                    }
                }
                selected.sort_unstable_by_key(|(row, _)| *row);
                paths.extend(selected.into_iter().map(|(_, path)| path));
            }
        }
    }
    paths.try_into().map_err(|_| {
        "Select two regular files in one pane, or one selected/focused file in each pane.".into()
    })
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
        (Content::Text(text), Input::Scroll { x, y }) if x.is_finite() && y.is_finite() => {
            text.scroll_pixels(y);
            text.horizontal = (text.horizontal - x).clamp(0.0, 262144.0);
        }
        (Content::Text(text), Input::Seek(value)) => text.seek(value),
        (Content::Text(text), Input::Difference { backwards }) => text.difference(backwards),
        (Content::Images(images), Input::Mode(mode)) => images.set_mode(mode),
        (Content::Images(images), Input::Amount(value)) if value.is_finite() => {
            images.amount = value.clamp(0.0, 1.0)
        }
        (Content::Images(images), Input::Zoom(factor)) => images.zoom(factor),
        (Content::Images(images), Input::ActualPixels) => images.actual(),
        (Content::Images(images), Input::Fit) => images.fit(),
        (Content::Images(images), Input::Scroll { x, y }) => {
            images.pan(x * geometry.scale, y * geometry.scale)
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
