//! Options › Editors: a custom fallback editor and a filename rule, both a harmless
//! `cp` into the work folder (no real editor is ever launched); F4 runs the matching
//! one; an invalid rule keeps Options open with its reason; a restart keeps both.
use crate::{
    Context,
    app::{App, Config},
    input,
    options_support::{self as options, SettingsStore},
    panel,
};
use midnight_core::PaneId;
use slint::platform::Key;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

const DEFAULT_EDITOR: &str = "Use the system default editor";
const RULES: &str = "Editor rules";

struct Setup {
    panel: PathBuf,
    out: PathBuf,
    cp: PathBuf,
    store: Arc<SettingsStore>,
}

impl Setup {
    fn config(&self) -> Config {
        let mut config = Config::new(&self.panel, &self.out);
        config.settings = Some(self.store.clone());
        config.editor = true;
        config
    }
}

fn rule_labels(app: &App) -> Vec<String> {
    options::list_rows(app, RULES)
        .into_iter()
        .map(|(label, _)| label)
        .collect()
}

fn add_argument(app: &mut App, text: &str) -> Result<(), String> {
    options::press(app, "Add argument")?;
    input::type_text(app.slint(), text);
    app.idle(Duration::from_millis(100));
    Ok(())
}

/// F4 on `name` in the left panel; wait for `output` to equal the file's content.
fn edit(app: &mut App, setup: &Setup, name: &str, output: &Path) -> Result<(), String> {
    let _ = fs::remove_file(output);
    panel::select_row(app, PaneId::Left, name)?;
    input::key(app.slint(), Key::F4);
    options::wait_file(app, output, &format!("F4 on {name}"))?;
    let expected = fs::read(setup.panel.join(name)).map_err(|error| error.to_string())?;
    for _ in 0..100 {
        if fs::read(output).is_ok_and(|bytes| bytes == expected) {
            return Ok(());
        }
        app.idle(Duration::from_millis(20));
    }
    Err(format!(
        "F4 on {name} wrote {:?} into {}",
        fs::read_to_string(output).unwrap_or_default(),
        output.display()
    ))
}

pub fn editors(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("editors");
    let setup = Setup {
        panel: root.join("panel"),
        out: root.join("out"),
        cp: options::program(&["/bin/cp", "/usr/bin/cp"])?,
        store: SettingsStore::new(),
    };
    fs::create_dir_all(&setup.panel).map_err(|error| error.to_string())?;
    fs::create_dir_all(&setup.out).map_err(|error| error.to_string())?;
    fs::write(setup.panel.join("notes.txt"), "plain notes\n").map_err(|error| error.to_string())?;
    fs::write(setup.panel.join("readme.md"), "# Read me\n").map_err(|error| error.to_string())?;
    let fallback_copy = setup.out.join("fallback-copy.txt");
    let rule_copy = setup.out.join("rule-copy.md");
    let cp = setup.cp.to_string_lossy().into_owned();
    let mut app = App::start(setup.config());

    options::open(&mut app)?;
    options::page(&mut app, "Editors")?;
    let labels = rule_labels(&app);
    if labels != ["Fallback → System editor"] {
        return Err(format!("the editor rules start as {labels:?}"));
    }
    // The fallback: a custom program with the file and a destination.
    options::set_checked(&mut app, DEFAULT_EDITOR, false)?;
    options::fill(&mut app, "Editor executable", &cp)?;
    add_argument(&mut app, &fallback_copy.to_string_lossy())?;
    // A rule for Markdown files.
    options::press(&mut app, "Add editor rule")?;
    options::fill(&mut app, "Editor filename pattern", "*.md")?;
    options::fill(&mut app, "Editor executable", &cp)?;
    add_argument(&mut app, &rule_copy.to_string_lossy())?;
    let labels = rule_labels(&app);
    let wanted = [format!("1. *.md → {cp}"), format!("Fallback → {cp}")];
    if labels != wanted {
        return Err(format!(
            "the editor rules show {labels:?}, expected {wanted:?}"
        ));
    }
    options::review_shot(
        &mut app,
        context,
        "editors-configured",
        "Options › Editors: a rule and a custom fallback",
    );
    if !options::ok(&mut app, &setup.store)? {
        return Err("OK did not save the editors".into());
    }
    let editor = setup.store.current().editor;
    if editor.executable.as_deref() != Some(setup.cp.as_path())
        || editor.arguments
            != [
                "{file}".to_owned(),
                fallback_copy.to_string_lossy().into_owned(),
            ]
        || editor.rules.len() != 1
        || editor.rules[0].expression != "*.md"
        || editor.rules[0].executable.as_deref() != Some(setup.cp.as_path())
        || editor.rules[0].arguments
            != [
                "{file}".to_owned(),
                rule_copy.to_string_lossy().into_owned(),
            ]
    {
        return Err(format!("the saved editor settings are {editor:?}"));
    }

    // F4 runs the rule's program for Markdown and the fallback for everything else.
    edit(&mut app, &setup, "readme.md", &rule_copy)?;
    edit(&mut app, &setup, "notes.txt", &fallback_copy)?;
    if rule_copy.exists() && fs::read(&rule_copy).ok() != Some(b"# Read me\n".to_vec()) {
        return Err("the fallback overwrote the rule's output".into());
    }

    // An invalid rule (no {file} argument) keeps Options open with the reason.
    options::open(&mut app)?;
    options::page(&mut app, "Editors")?;
    options::click_row(&mut app, RULES, "rule", &|label| label.starts_with("1. "))?;
    options::click_row(&mut app, "Editor arguments", "first argument", &|label| {
        label == "Editor argument 1"
    })?;
    options::press(&mut app, "Remove argument")?;
    let saves = setup.store.saves();
    input::click_button(app.slint(), "OK")?;
    app.idle(Duration::from_millis(400));
    let reasons = options::texts_with(&app, "{file}");
    if !app.window.get_options_open() || reasons.is_empty() {
        return Err(format!(
            "OK with an editor rule lacking {{file}}: open={}, reasons {reasons:?}",
            app.window.get_options_open()
        ));
    }
    options::review_shot(
        &mut app,
        context,
        "editors-invalid",
        "Options › Editors refusing a rule without {file}",
    );
    options::cancel(&mut app)?;
    if setup.store.saves() != saves {
        return Err("the invalid editor rule was saved".into());
    }

    // Restart: both editors remain.
    app.shutdown()?;
    drop(app);
    let mut app = App::start(setup.config());
    edit(&mut app, &setup, "readme.md", &rule_copy)?;
    edit(&mut app, &setup, "notes.txt", &fallback_copy)?;
    context.note("Editors: fallback and *.md rule configured as /bin/cp {file} <work>; F4 copied each file to its own destination before and after a restart; a rule without {file} kept Options open with its reason and Cancel saved nothing. No real editor was launched");
    app.shutdown()
}
