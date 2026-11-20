//! The scenario registry. Add a scenario by writing `fn(&mut Context) -> Result<(), String>`
//! in a module here and listing it in [`all`] (docs/developer/ACCEPTANCE.md).
pub mod checksums;
pub mod create_rename;
pub mod find;
pub mod navigation;
pub mod transfers;
pub mod tree;

use crate::{Context, Scenario, app};
use midnight_core::PaneId;
use std::time::Duration;

fn minutes(minutes: u64) -> Duration {
    Duration::from_secs(minutes * 60)
}

pub fn all() -> &'static [Scenario] {
    static ALL: std::sync::OnceLock<Vec<Scenario>> = std::sync::OnceLock::new();
    ALL.get_or_init(|| {
        vec![
            Scenario {
                name: "startup",
                area: "panels",
                summary: "Both panels open on the fixture root and list every top-level folder.",
                run: startup,
                timeout: minutes(2),
            },
            Scenario {
                name: "navigation",
                area: "panels",
                summary: "Enter/Backspace into every fixture folder, a level deeper and down the 12-level chain; each listing equals the disk.",
                run: navigation::every_folder,
                timeout: minutes(10),
            },
            Scenario {
                name: "copy_move_delete",
                area: "transfers",
                summary: "F5 and F6 of a file and a folder, Shift+F8 of a file and a folder, confirmed in the dialogs and verified on disk.",
                run: transfers::copy_move_delete,
                timeout: minutes(5),
            },
            Scenario {
                name: "create_rename",
                area: "operations",
                summary: "Shift+F4 new empty file (and the existing-name prompt), F7 new folder, F2 rename shown at once.",
                run: create_rename::create_rename,
                timeout: minutes(5),
            },
            Scenario {
                name: "tree_expand_all",
                area: "tree",
                summary: "Ctrl+click on a folder expander expands the 12-level branch inline.",
                run: tree::expand_all,
                timeout: minutes(5),
            },
            Scenario {
                name: "folder_sizes",
                area: "tree",
                summary: "Ctrl+Shift+S measures every fixture folder; each size equals the logical bytes on disk.",
                run: tree::folder_sizes,
                timeout: minutes(5),
            },
            Scenario {
                name: "find_by_mask",
                area: "find",
                summary: "Alt+F7 Find by mask returns exactly the matching files on disk; Go to file locates a result.",
                run: find::by_mask,
                timeout: minutes(5),
            },
            Scenario {
                name: "checksums",
                area: "checksums",
                summary: "Ctrl+F8: calculate, save, replace and verify a manifest; verify the fixture's MD5, SFV, mismatch and missing manifests.",
                run: checksums::calculate_save_verify,
                timeout: minutes(5),
            },
            Scenario {
                name: "ui_review",
                area: "ui",
                summary: "Screenshots of the main window and its dialogs across themes, densities and window sizes, with automatic layout checks.",
                run: crate::review::run,
                timeout: minutes(40),
            },
        ]
    })
}

fn startup(context: &mut Context) -> Result<(), String> {
    let root = context.fixture.root.clone();
    let mut app = app::App::start(app::Config::new(&root, &root));
    let expected = context.fixture.top_folders();
    for pane in [PaneId::Left, PaneId::Right] {
        let names = app.names(pane);
        let missing: Vec<&String> = expected
            .iter()
            .filter(|name| !names.contains(name))
            .collect();
        if !missing.is_empty() {
            return Err(format!(
                "{pane:?} panel lacks {missing:?}; it shows {names:?}"
            ));
        }
    }
    app.screenshot(context, "fixture-root", "Both panels on the fixture root");
    app.shutdown()
}
