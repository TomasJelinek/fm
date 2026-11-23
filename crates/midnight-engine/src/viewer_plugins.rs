//! Executable-relative catalog discovery is bounded backend work, independent of browsing.
use crate::Outbox;
use midnight_core::{
    ComparatorCapabilities, ComparatorImageCapability, Event, ViewerPluginInfo, viewer_plugin_name,
};
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::{sync::mpsc, task::JoinSet};
#[path = "filesystem_plugins.rs"]
mod filesystems;

pub(crate) struct Controller {
    sender: mpsc::Sender<()>,
    pending: Arc<Mutex<Pending>>,
}

#[derive(Default)]
struct Pending {
    catalog: bool,
    comparators: Option<Arc<[String]>>,
}

impl Controller {
    pub(crate) fn new(outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        let (sender, mut requests) = mpsc::channel(1);
        let pending = Arc::new(Mutex::new(Pending::default()));
        let worker_pending = pending.clone();
        tasks.spawn(async move {
            let mut revision = 0_u64;
            while requests.recv().await.is_some() {
                let pending = {
                    let mut pending = worker_pending
                        .lock()
                        .unwrap_or_else(|error| error.into_inner());
                    std::mem::take(&mut *pending)
                };
                if pending.catalog {
                    let result = tokio::task::spawn_blocking(|| {
                        match midnight_plugin_host::installed_executable() {
                            Ok(executable) => (
                                catalog(&executable),
                                filesystems::catalog(&executable),
                                midnight_thumbnail_host::catalog(&executable)
                                    .map_err(|error| error.to_string()),
                            ),
                            Err(error) => (
                                Err(error.to_string()),
                                Err(error.to_string()),
                                Err(error.to_string()),
                            ),
                        }
                    })
                    .await
                    .unwrap_or_else(|error| {
                        let message = format!("Plugin discovery worker failed: {error}");
                        (Err(message.clone()), Err(message.clone()), Err(message))
                    });
                    if !outbox.send(Event::ViewerPluginsListed(result.0)).await
                        || !outbox.send(Event::FilesystemPluginsListed(result.1)).await
                        || !outbox.send(Event::ThumbnailerPluginsListed(result.2)).await
                    {
                        break;
                    }
                }
                if let Some(disabled_plugins) = pending.comparators {
                    let Some(next_revision) = revision.checked_add(1) else {
                        break;
                    };
                    revision = next_revision;
                    let disabled = disabled_plugins.clone();
                    let images = tokio::task::spawn_blocking(move || {
                        midnight_core::validate_disabled_viewers(&disabled)
                            .map_err(str::to_owned)?;
                        let executable = midnight_plugin_host::installed_executable()
                            .map_err(|error| error.to_string())?;
                        comparator_images(&executable, &disabled)
                    })
                    .await
                    .unwrap_or_else(|error| {
                        Err(format!("Comparator coverage worker failed: {error}"))
                    });
                    if !outbox
                        .send(Event::ComparatorCapabilitiesListed(
                            ComparatorCapabilities {
                                revision,
                                disabled_plugins,
                                images,
                            },
                        ))
                        .await
                    {
                        break;
                    }
                }
            }
        });
        Self { sender, pending }
    }
    pub(crate) fn refresh(&self) {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .catalog = true;
        let _ = self.sender.try_send(());
    }
    pub(crate) fn comparator_capabilities(&self, disabled_plugins: Arc<[String]>) {
        self.pending
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .comparators = Some(disabled_plugins);
        let _ = self.sender.try_send(());
    }
}

fn comparator_images(
    executable: &Path,
    disabled: &[String],
) -> Result<Arc<[ComparatorImageCapability]>, String> {
    let discovery = midnight_image_viewer_host::discover_with_diagnostics(executable)
        .map_err(|error| error.to_string())?;
    if !discovery.diagnostics.is_empty() {
        return Err(format!(
            "Image comparison coverage is incomplete: {}",
            discovery
                .diagnostics
                .iter()
                .map(|diagnostic| diagnostic.message.as_str())
                .collect::<Vec<_>>()
                .join("; ")
        ));
    }
    Ok(effective_comparator_images(
        discovery.providers.iter().map(|provider| {
            (
                provider.id.as_str(),
                provider.extensions.as_slice(),
                provider.is_document(),
            )
        }),
        disabled,
    ))
}

fn effective_comparator_images<'a>(
    providers: impl IntoIterator<Item = (&'a str, &'a [String], bool)>,
    disabled: &[String],
) -> Arc<[ComparatorImageCapability]> {
    providers
        .into_iter()
        .filter(|(id, _, document)| {
            !*document && !disabled.iter().any(|disabled| disabled.as_str() == *id)
        })
        .map(|(id, extensions, _)| ComparatorImageCapability {
            provider_id: id.to_owned(),
            extensions: extensions.to_vec(),
        })
        .collect()
}

fn catalog(executable: &Path) -> Result<Vec<ViewerPluginInfo>, String> {
    let directory = executable
        .parent()
        .ok_or("Executable directory is unavailable")?;
    let runtime =
        midnight_plugin_host::Runtime::discover(directory).map_err(|error| error.to_string())?;
    let mut rows = runtime
        .catalog()
        .iter()
        .map(|plugin| ViewerPluginInfo {
            extensions: plugin.extensions.clone(),
            fallback: plugin.extensions.is_empty()
                && plugin
                    .modes
                    .contains(&midnight_plugin_host::api::viewer::MODE_TEXT),
            id: plugin.id.clone(),
            name: viewer_plugin_name(&plugin.id, plugin.name.as_deref()),
            description: description(&plugin.id, &plugin.description),
            version: plugin.version.clone(),
            status: if runtime.is_loaded(&plugin.id) {
                "Loaded"
            } else {
                "Available · not loaded"
            }
            .into(),
            location: plugin.package_dir.clone(),
        })
        .collect::<Vec<_>>();
    let images = midnight_image_viewer_host::discover_with_diagnostics(executable)
        .map_err(|error| error.to_string())?;
    rows.extend(images.providers.iter().map(|provider| ViewerPluginInfo {
        extensions: provider.extensions.clone(),
        fallback: false,
        id: provider.id.clone(),
        name: viewer_plugin_name(&provider.id, Some(&provider.name)),
        description: match provider.id.as_str() {
            "midnight.viewer.image" => "Displays image files.".into(),
            "midnight.viewer.pdf" => "Displays PDF documents.".into(),
            _ => String::new(),
        },
        version: provider.version.clone(),
        status: "Available".into(),
        location: provider.package_dir().to_path_buf(),
    }));
    let web = midnight_web_viewer_host::discover_with_diagnostics(executable)
        .map_err(|error| error.to_string())?;
    rows.extend(web.providers.iter().map(|provider| ViewerPluginInfo {
        extensions: provider.extensions.clone(),
        fallback: false,
        id: provider.id.clone(),
        name: viewer_plugin_name(&provider.id, Some(&provider.name)),
        description: "Displays HTML pages.".into(),
        version: provider.version.clone(),
        status: "Available".into(),
        location: provider.package_dir().to_path_buf(),
    }));
    if let Ok(video) =
        midnight_media_host::video_installation::VideoInstallation::discover(executable)
    {
        rows.push(ViewerPluginInfo {
            extensions: video.extensions().to_vec(),
            fallback: false,
            id: midnight_media_host::video_installation::VIDEO_PROVIDER_ID.into(),
            name: viewer_plugin_name(
                midnight_media_host::video_installation::VIDEO_PROVIDER_ID,
                Some("Video"),
            ),
            description: "Plays videos with FFmpeg.".into(),
            version: midnight_media_host::video_installation::VIDEO_RUNTIME_PROFILE.into(),
            status: "Available".into(),
            location: directory.join("thumbnailers"),
        });
    }
    let mut counts = std::collections::HashMap::new();
    for row in &rows {
        *counts.entry(row.id.clone()).or_insert(0_usize) += 1;
    }
    for row in &mut rows {
        if counts[&row.id] > 1 {
            row.id.clear();
            row.status = "Unavailable: duplicate viewer identity".into();
        }
    }
    let mut by_package = std::collections::BTreeMap::<_, Vec<String>>::new();
    for diagnostic in runtime
        .diagnostics()
        .iter()
        .chain(images.diagnostics.iter())
        .chain(web.diagnostics.iter())
    {
        let messages = by_package.entry(diagnostic.package.clone()).or_default();
        if !messages.contains(&diagnostic.message) {
            messages.push(diagnostic.message.clone());
        }
    }
    rows.extend(by_package.into_iter().map(|(package, messages)| {
        ViewerPluginInfo {
            extensions: Vec::new(),
            fallback: false,
            id: String::new(),
            name: package
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Viewer package".into()),
            description: String::new(),
            version: String::new(),
            status: format!("Unavailable: {}", messages.join("\n")),
            location: package,
        }
    }));
    Ok(rows)
}

fn description(id: &str, supplied: &str) -> String {
    match (id, supplied) {
        ("midnight.viewer.basic", "Bounded UTF-8 / BOM UTF-16 text and binary viewer") =>
            "Displays text and binary files; supports UTF-8 and BOM-marked UTF-16.".into(),
        ("midnight.viewer.source", "Bounded source highlighting: 17 documented lexical syntax profiles") =>
            "Syntax highlighting for Rust, C, C++, JavaScript, TypeScript, Python, POSIX shell/Bash/Zsh, JSON, TOML, Java, C#, Go, Swift, Kotlin, Ruby, SQL and YAML (17 lexical profiles).".into(),
        ("midnight.viewer.markdown", "Bounded CommonMark semantic preview; inert HTML and safe link metadata") =>
            "Displays Markdown documents. HTML stays inert and links are shown as metadata.".into(),
        _ => supplied.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn comparator_coverage_uses_only_enabled_non_document_discovered_metadata() {
        let images = vec!["png".into(), "webp".into()];
        let other = vec!["avif".into()];
        let documents = vec!["pdf".into()];
        let providers = || {
            [
                ("images.primary", images.as_slice(), false),
                ("images.optional", other.as_slice(), false),
                ("documents.pdf", documents.as_slice(), true),
            ]
        };
        let enabled = effective_comparator_images(providers(), &["images.optional".into()]);
        assert_eq!(enabled.len(), 1);
        assert_eq!(enabled[0].provider_id, "images.primary");
        assert_eq!(enabled[0].extensions, images);
        let none = effective_comparator_images(
            providers(),
            &["images.primary".into(), "images.optional".into()],
        );
        assert!(none.is_empty());
    }

    #[test]
    fn failed_or_untrusted_discovery_is_reported_as_unknown_coverage() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("viewers/broken");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("image-viewer.json"), br#"{"schema":1}"#).unwrap();
        let result = comparator_images(&root.path().join("midnight-explorer.exe"), &[]);
        assert!(
            result.is_err(),
            "unavailable discovery cannot claim known empty coverage"
        );
    }
    #[test]
    #[ignore = "requires PLUGIN_CATALOG_TEST_EXECUTABLE in a trusted installed package"]
    fn installed_viewer_catalog_has_one_html_row_and_readable_viewer_names() {
        let executable = std::path::PathBuf::from(
            std::env::var_os("PLUGIN_CATALOG_TEST_EXECUTABLE")
                .expect("PLUGIN_CATALOG_TEST_EXECUTABLE"),
        )
        .canonicalize()
        .unwrap();
        let rows = catalog(&executable).unwrap();
        let html: Vec<_> = rows
            .iter()
            .filter(|row| row.location.file_name().is_some_and(|name| name == "html"))
            .collect();
        assert_eq!(
            html.len(),
            1,
            "one web bundle must create one real row: {html:?}"
        );
        assert_eq!(html[0].id, "midnight.viewer.html");
        for row in rows.iter().filter(|row| !row.id.is_empty()) {
            assert!(!row.name.starts_with("Bounded"), "{}", row.name);
            assert!(
                row.name
                    .split_whitespace()
                    .any(|word| word.eq_ignore_ascii_case("viewer")),
                "friendly title must identify the viewer: {}",
                row.name
            );
        }
        let source = rows
            .iter()
            .find(|row| row.id == "midnight.viewer.source")
            .unwrap();
        for profile in [
            "Rust",
            "C++",
            "JavaScript",
            "TypeScript",
            "Python",
            "POSIX shell",
            "JSON",
            "TOML",
            "Java",
            "C#",
            "Go",
            "Swift",
            "Kotlin",
            "Ruby",
            "SQL",
            "YAML",
        ] {
            assert!(
                source.description.contains(profile),
                "missing {profile}: {}",
                source.description
            );
        }
    }

    #[test]
    fn invalid_or_untrusted_installation_reports_a_diagnostic_without_loading_code() {
        let root = tempfile::tempdir().unwrap();
        let package = root.path().join("viewers/broken");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("plugin.toml"), "not valid = [").unwrap();
        match catalog(&root.path().join("midnight-explorer.exe")) {
            Ok(rows) => {
                assert_eq!(rows.len(), 1);
                assert!(rows[0].id.is_empty());
                assert!(rows[0].status.contains("Unavailable"));
                let package = package.canonicalize().unwrap();
                assert!(
                    rows[0].location == package
                        || Some(rows[0].location.as_path()) == package.parent()
                );
            }
            // An installation in an insecure temporary root must still report its refusal.
            Err(error) => assert!(!error.is_empty()),
        }
    }
}
