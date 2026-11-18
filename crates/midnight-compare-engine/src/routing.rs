//! Session-pinned route selection before decoder discovery or content capture.
use crate::OwnedFileComparatorPolicy;
use midnight_core::{FileComparatorProgram, FileComparatorTarget};
use midnight_image_viewer_host::Provider;
use std::path::PathBuf;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BuiltInTarget {
    Automatic,
    Text,
    Images,
}

pub(crate) enum Target<'a> {
    BuiltIn(BuiltInTarget),
    External(&'a FileComparatorProgram),
}

/// Selection uses both ORIGINAL native filenames through the pinned policy.
/// Never use CapturedFile::path/display_name or consult the live policy store.
pub(crate) fn select<'a>(
    policy: &'a dyn OwnedFileComparatorPolicy,
    paths: &[PathBuf; 2],
) -> Target<'a> {
    let target = policy
        .select(paths)
        .and_then(|index| policy.preferences().rules.get(index))
        .map(|rule| &rule.target);
    match target {
        Some(FileComparatorTarget::BuiltInText) => Target::BuiltIn(BuiltInTarget::Text),
        Some(FileComparatorTarget::BuiltInImages) => Target::BuiltIn(BuiltInTarget::Images),
        Some(FileComparatorTarget::External(program)) => Target::External(program),
        // A selected explicit Automatic row stops matching just like every other row.
        Some(FileComparatorTarget::Automatic) | None => Target::BuiltIn(BuiltInTarget::Automatic),
    }
}

pub(crate) enum BuiltInPlan {
    Text,
    Images {
        providers: Vec<Provider>,
        selected: [usize; 2],
    },
}

/// The one discovery closure keeps the existing installed provider implementation.
/// This private seam permits observable zero-discovery tests without a public factory API.
pub(crate) fn plan(
    target: BuiltInTarget,
    paths: &[PathBuf; 2],
    disabled_plugins: &[String],
    discover: impl FnOnce() -> Result<Vec<Provider>, String>,
) -> Result<BuiltInPlan, String> {
    if target == BuiltInTarget::Text {
        return Ok(BuiltInPlan::Text);
    }
    let providers = match discover() {
        Ok(providers) => providers,
        // Preserve the existing text path when optional image runtime is unavailable.
        Err(_) if target == BuiltInTarget::Automatic => Vec::new(),
        Err(error) => return Err(format!("Image comparison is unavailable: {error}")),
    };
    let selected = paths.each_ref().map(|path| {
        path.extension()
            .and_then(|extension| extension.to_str())
            .and_then(|extension| {
                providers.iter().position(|provider| {
                    !disabled_plugins.contains(&provider.id)
                        && provider.supports_extension(extension)
                        && (target != BuiltInTarget::Images || !provider.is_document())
                })
            })
    });
    if selected
        .iter()
        .flatten()
        .any(|index| providers[*index].is_document())
    {
        return Err(
            "Document comparison is not supported. Open PDFs in the document viewer instead."
                .into(),
        );
    }
    match selected {
        [Some(left), Some(right)] => Ok(BuiltInPlan::Images {
            providers,
            selected: [left, right],
        }),
        [None, None] if target == BuiltInTarget::Automatic => Ok(BuiltInPlan::Text),
        _ if target == BuiltInTarget::Images => Err(
            "Image comparison requires two files supported by enabled installed image decoders."
                .into(),
        ),
        _ => Err("Select two supported images or two text files.".into()),
    }
}

pub(crate) fn discover() -> Result<Vec<Provider>, String> {
    let executable =
        midnight_plugin_host::installed_executable().map_err(|error| error.to_string())?;
    midnight_image_viewer_host::discover(&executable).map_err(|error| error.to_string())
}

#[cfg(test)]
#[path = "routing_tests.rs"]
mod tests;
