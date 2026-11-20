//! Panel navigation into every fixture folder with real clicks and keys.
use crate::{
    Context,
    app::{App, Config},
    panel,
};
use midnight_core::PaneId;
use std::{collections::BTreeSet, path::Path};

/// The panel's names without `..`, compared with the folder on disk.
fn compare_with_disk(app: &App, pane: PaneId, folder: &Path) -> Result<(), String> {
    let shown: BTreeSet<String> = app
        .names(pane)
        .into_iter()
        .filter(|name| name != "..")
        .collect();
    let disk: BTreeSet<String> = panel::disk_names(folder).into_iter().collect();
    if shown == disk {
        return Ok(());
    }
    let missing: Vec<&String> = disk.difference(&shown).take(20).collect();
    let extra: Vec<&String> = shown.difference(&disk).take(20).collect();
    Err(format!(
        "{} lists {} entries, disk has {}; missing {missing:?}, unexpected {extra:?}",
        folder.display(),
        shown.len(),
        disk.len()
    ))
}

pub fn every_folder(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut app = App::start(Config::new(&root, &root));
    let pane = PaneId::Left;
    panel::focus_list(&mut app, pane)?;
    compare_with_disk(&app, pane, &root)?;
    let mut visited = 0;
    for folder in context.fixture.top_folders() {
        let path = panel::enter(&mut app, pane, &folder)?;
        compare_with_disk(&app, pane, &path)?;
        app.screenshot(
            context,
            &format!("folder-{folder}"),
            &format!("Left panel in {folder}/"),
        );
        visited += 1;
        // One level deeper where the fixture has sub-folders worth opening.
        for child in [
            "page_files",
            "level-01",
            "folder with spaces",
            ".hidden-folder",
            "group-00",
            "locked-folder",
        ] {
            if path.join(child).is_dir() && app.names(pane).iter().any(|name| name == child) {
                let inner = panel::enter(&mut app, pane, child)?;
                compare_with_disk(&app, pane, &inner)?;
                panel::leave(&mut app, pane)?;
                visited += 1;
            }
        }
        panel::leave(&mut app, pane)?;
    }
    // The twelve-level chain down and back up.
    panel::enter(&mut app, pane, "deep")?;
    for level in 1..=12 {
        let path = panel::enter(&mut app, pane, &format!("level-{level:02}"))?;
        compare_with_disk(&app, pane, &path)?;
        visited += 1;
    }
    app.screenshot(context, "deep-level-12", "Twelve levels down");
    for _ in 0..13 {
        panel::leave(&mut app, pane)?;
    }
    if app.location(pane).as_deref() != Some(root.as_path()) {
        return Err(format!(
            "expected to be back at the root, at {:?}",
            app.location(pane)
        ));
    }
    context.note(format!(
        "visited {visited} folders with Enter/Backspace and verified each listing against the disk"
    ));
    app.shutdown()
}
