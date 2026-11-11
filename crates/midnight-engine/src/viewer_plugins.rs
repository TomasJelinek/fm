//! Executable-relative catalog discovery is bounded backend work, independent of browsing.
use crate::Outbox;
use midnight_core::{Event, ViewerPluginInfo, viewer_plugin_name};
use std::path::Path;
use tokio::{sync::mpsc, task::JoinSet};
#[path = "filesystem_plugins.rs"]
mod filesystems;

pub(crate) struct Controller {
    sender: mpsc::Sender<()>,
}

impl Controller {
    pub(crate) fn new(outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        let (sender, mut requests) = mpsc::channel(1);
        tasks.spawn(async move {
            while requests.recv().await.is_some() {
                let result =
                    tokio::task::spawn_blocking(
                        || match midnight_plugin_host::installed_executable() {
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
                        },
                    )
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
        });
        Self { sender }
    }
    pub(crate) fn refresh(&self) {
        let _ = self.sender.try_send(());
    }
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
        match catalog(&root.path().join("me.exe")) {
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
