//! Options › Comparators: a built-in text override and an external-program override
//! (configured and described as argument help, never run), an invalid regular
//! expression refused by OK with its reason, Cancel, OK and a restart.
use crate::{
    Context,
    app::{App, Config},
    options_support::{self as options, SettingsStore},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{FileComparatorTarget, FilterMode};
use std::{fs, path::Path, sync::Arc, time::Duration};

const RULES: &str = "Comparator rules";

fn config(folder: &Path, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(folder, folder);
    config.settings = Some(store.clone());
    config
}

fn rule_labels(app: &App) -> Vec<String> {
    options::list_rows(app, RULES)
        .into_iter()
        .map(|(label, _)| label)
        .collect()
}

fn wait_ready(app: &mut App) -> Result<(), String> {
    for _ in 0..250 {
        if options::is_enabled(app, AccessibleRole::Button, "Add comparator rule") {
            return Ok(());
        }
        app.idle(Duration::from_millis(20));
    }
    Err(format!(
        "the Comparators page never became ready: {:?}",
        options::texts_with(app, "omparator")
    ))
}

pub fn comparators(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("comparators");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let tool = crate::tool::program(context, "true")?;
    let tool_text = tool.to_string_lossy().into_owned();
    let store = SettingsStore::new();
    let mut app = App::start(config(&folder, &store));
    options::open(&mut app)?;
    options::page(&mut app, "Comparators")?;
    wait_ready(&mut app)?;
    if options::texts_with(&app, "No overrides").is_empty() {
        return Err("the Comparators page does not state that no overrides exist".into());
    }
    // The toolbar keeps keyboard focus as adding rules reveals the priority buttons.
    for count in 1..=2 {
        for _ in 0..count {
            options::press(&mut app, "Add comparator rule")?;
        }
        let priority_shown = count == 2;
        for label in ["Move comparator rule up", "Move comparator rule down"] {
            if options::find_any(&app, AccessibleRole::Button, label).is_some() != priority_shown {
                return Err(format!(
                    "{count} comparator rules: incorrect visibility of {label:?}"
                ));
            }
        }
        let last = if priority_shown {
            "Move comparator rule down"
        } else {
            "Remove comparator rule"
        };
        let right_edge = |role, label| {
            options::find_any(&app, role, label)
                .map(|item| {
                    let (x, _, width, _) = crate::inspect::rect(&item);
                    x + width
                })
                .ok_or_else(|| format!("missing {label:?}"))
        };
        let edge = right_edge(AccessibleRole::Button, last)?;
        let list_edge = right_edge(AccessibleRole::List, RULES)?;
        if (edge - list_edge).abs() > 3.0 {
            return Err(format!(
                "{last:?} ends at {edge}, away from the rules list edge {list_edge}"
            ));
        }
        crate::input::key(app.slint(), slint::platform::Key::Escape);
        if !options::poll(&mut app, Duration::from_secs(2), |app| {
            !app.window.get_options_open()
        }) {
            return Err(format!(
                "Escape did not close Options directly after adding rule {count}"
            ));
        }
        options::open(&mut app)?;
        options::page(&mut app, "Comparators")?;
        wait_ready(&mut app)?;
        if rule_labels(&app) != ["Other regular files → Automatic (fixed)"] {
            return Err(format!(
                "Escape kept a draft comparator rule: {:?}",
                rule_labels(&app)
            ));
        }
    }

    // A built-in text override.
    options::press(&mut app, "Add comparator rule")?;
    options::fill(&mut app, "Comparator filename pattern", "*.cmp")?;
    options::choose(&mut app, "Comparator target", "Built-in text")?;
    let up = "Move comparator rule up";
    let down = "Move comparator rule down";
    // An external program: its arguments are literal rows with {left} and {right}.
    options::press(&mut app, "Add comparator rule")?;
    options::fill(&mut app, "Comparator filename pattern", "*.ext")?;
    options::choose(&mut app, "Comparator target", "External program")?;
    options::fill(&mut app, "Comparator executable", &tool_text)?;
    // A new external rule starts with {left} and {right}; append a literal option.
    options::press(&mut app, "Add argument")?;
    crate::input::type_text(app.slint(), "--brief");
    app.idle(Duration::from_millis(100));
    for help in [
        "Argument 1: {left}",
        "Argument 2: {right}",
        "Argument 3: --brief",
    ] {
        if options::texts_with(&app, help).is_empty() {
            return Err(format!("the argument help does not show {help:?}"));
        }
    }
    let original = [
        "1. *.cmp → Built-in text".to_owned(),
        format!("2. *.ext → External: {tool_text}"),
        "Other regular files → Automatic (fixed)".to_owned(),
    ];
    let wanted = [
        format!("1. *.ext → External: {tool_text}"),
        "2. *.cmp → Built-in text".to_owned(),
        original[2].clone(),
    ];
    for (button, expected) in [(up, &wanted), (down, &original), (up, &wanted)] {
        options::press(&mut app, button)?;
        let labels = rule_labels(&app);
        if labels.as_slice() != expected.as_slice() {
            return Err(format!(
                "{button} produced {labels:?}, expected {expected:?}"
            ));
        }
    }
    options::review_shot(
        &mut app,
        context,
        "comparators-rules",
        "Options › Comparators: two overrides",
    );
    if !options::ok(&mut app, &store)? {
        return Err("OK did not save the comparator rules".into());
    }
    let rules = store.current().file_comparators.rules;
    let external = rules.first().ok_or("the external rule was not saved")?;
    let text_rule = rules.get(1).ok_or("the text rule was not saved")?;
    if rules.len() != 2
        || text_rule.expression != "*.cmp"
        || text_rule.mode != FilterMode::Masks
        || text_rule.target != FileComparatorTarget::BuiltInText
        || external.expression != "*.ext"
    {
        return Err(format!("the saved comparator rules are {rules:?}"));
    }
    match &external.target {
        FileComparatorTarget::External(program)
            if program.executable == tool
                && program.arguments == ["{left}", "{right}", "--brief"] => {}
        other => return Err(format!("the external rule saved {other:?}")),
    }

    // An invalid regular expression: OK keeps Options open and names the problem.
    options::open(&mut app)?;
    options::page(&mut app, "Comparators")?;
    wait_ready(&mut app)?;
    options::press(&mut app, "Add comparator rule")?;
    options::choose(&mut app, "Comparator matching mode", "RegEx")?;
    options::fill(&mut app, "Comparator filename pattern", "([")?;
    let saves = store.saves();
    input_ok(&mut app)?;
    options::review_shot(
        &mut app,
        context,
        "comparators-invalid",
        "Options › Comparators refusing an invalid expression",
    );
    // The reason is shown beside the rules (a Text labelled for accessibility), or in
    // the dialog's error line.
    let error = options::find_any(&app, AccessibleRole::Text, "Comparator validation error")
        .and_then(|item| options::text_of(&item))
        .filter(|text| !text.is_empty())
        .or_else(|| options::error_text(&app));
    if !app.window.get_options_open() || error.is_none() {
        return Err(format!(
            "OK with an invalid expression: open={}, error {error:?}; texts {:?}",
            app.window.get_options_open(),
            options::texts_with(&app, "xpression")
        ));
    }
    options::cancel(&mut app)?;
    if store.saves() != saves || store.current().file_comparators.rules.len() != 2 {
        return Err("Cancel saved the invalid comparator rule".into());
    }

    // Restart: the two overrides are listed again.
    app.shutdown()?;
    drop(app);
    let mut app = App::start(config(&folder, &store));
    options::open(&mut app)?;
    options::page(&mut app, "Comparators")?;
    wait_ready(&mut app)?;
    let labels = rule_labels(&app);
    if labels != wanted {
        return Err(format!(
            "after a restart the comparator rules show {labels:?}"
        ));
    }
    options::cancel(&mut app)?;
    context.note(format!(
        "Comparators: *.cmp → Built-in text and *.ext → External {tool_text} {{left}} {{right}} --brief saved and listed after a restart (the external program was never run); an invalid regular expression kept Options open with {error:?}"
    ));
    app.shutdown()
}

/// Click OK and give validation time; the dialog may stay open.
fn input_ok(app: &mut App) -> Result<(), String> {
    crate::input::click_button(app.slint(), "OK")?;
    for _ in 0..40 {
        app.idle(Duration::from_millis(25));
        if !app.window.get_options_busy() && !app.window.get_options_open() {
            break;
        }
    }
    Ok(())
}
