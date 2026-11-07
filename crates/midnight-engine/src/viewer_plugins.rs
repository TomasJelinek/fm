//! Executable-relative catalog discovery is bounded backend work, independent of browsing.
use crate::Outbox;
use midnight_core::{Event, ViewerPluginInfo};
use std::path::Path;
use tokio::{sync::mpsc, task::JoinSet};

pub(crate) struct Controller {
    sender: mpsc::Sender<()>,
}

impl Controller {
    pub(crate) fn new(outbox: Outbox, tasks: &mut JoinSet<()>) -> Self {
        let (sender, mut requests) = mpsc::channel(1);
        tasks.spawn(async move {
            while requests.recv().await.is_some() {
                let result = tokio::task::spawn_blocking(|| {
                    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
                    catalog(&executable)
                })
                .await
                .unwrap_or_else(|error| Err(format!("Plugin discovery worker failed: {error}")));
                if !outbox.send(Event::ViewerPluginsListed(result)).await {
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
            id: plugin.id.clone(),
            name: if plugin.description.is_empty() {
                plugin.id.clone()
            } else {
                plugin.description.clone()
            },
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
        id: provider.id.clone(),
        name: provider.name.clone(),
        version: provider.version.clone(),
        status: "Available · isolated image worker".into(),
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
    let mut diagnostics = runtime
        .diagnostics()
        .iter()
        .chain(images.diagnostics.iter())
        .map(|diagnostic| ViewerPluginInfo {
            id: String::new(),
            name: diagnostic
                .package
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| "Viewer package".into()),
            version: String::new(),
            status: format!(
                "Unavailable: {}",
                diagnostic.message.chars().take(2048).collect::<String>()
            ),
            location: diagnostic.package.clone(),
        })
        .collect::<Vec<_>>();
    // Both hosts can report the same untrusted installation root.
    diagnostics.dedup_by(|a, b| a.location == b.location && a.status == b.status);
    rows.extend(diagnostics);
    Ok(rows)
}

#[cfg(test)]
mod tests {
    use super::*;
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
