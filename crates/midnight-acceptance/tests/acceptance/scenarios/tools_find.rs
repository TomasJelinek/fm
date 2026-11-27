//! Find (Alt+F7) beyond `find_by_mask`: every criterion of the window on a generated
//! tree with known answers, the Search folders picker (tick boxes, Add, Browse,
//! Remove), the syntax help, error details, Stop, large result sets with the result
//! limit, column resizing, sorting and Go to file.
use crate::{
    Context,
    app::{self, App, Config},
    input, inspect, panel,
    scenarios::{
        find_support::{
            self, BASE, MODES, Query, Row, SIZE, all_rows, fill, open_find, relative, result_count,
            run, search, set, status, terminal, to_top, visible_rows, wheel,
        },
        transfers::copy_tree,
    },
    tools_support::{self as tools, Tool},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{PaneId, PathPickerKind};
use slint::{
    LogicalPosition,
    platform::{Key, WindowAdapter},
};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    time::{Duration, Instant, SystemTime},
};

/// The generated tree with known answers (relative paths below it).
fn find_tree(root: &Path) -> Result<(), String> {
    let now = SystemTime::now();
    let files: [(&str, &[u8]); 14] = [
        ("alpha.txt", b"The quick brown fox\n"),
        ("Beta.TXT", b"QUICK silver\n"),
        ("gamma.log", b"slowly\n"),
        ("notes.md", b"quickly done\n"),
        ("report-2024.pdf", b"%PDF-1.4 placeholder\n"),
        ("old.txt", b"quick but old\n"),
        (".hidden.txt", b"hidden quick\n"),
        ("readonly.txt", b"read only\n"),
        ("sub/alpha.txt", b"quick\n"),
        ("sub/deeper/zeta.txt", b"zeta\n"),
        ("dupes/one.dat", b"duplicate payload\n"),
        ("dupes/two.dat", b"duplicate payload\n"),
        ("dupes/three.dat", b"different payload\n"),
        ("gamma-copy.log", b"slowly\n"),
    ];
    for (path, data) in files {
        tools::write_at(&root.join(path), data, now)?;
    }
    tools::write_at(&root.join("big.dat"), &[b'b'; 2048], now)?;
    let mut utf16 = vec![0xFF, 0xFE];
    for unit in "a quick test\n".encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    tools::write_at(&root.join("utf16.txt"), &utf16, now)?;
    // 2020-01-02T03:04:05Z: older than any "modified within" window.
    tools::set_modified(
        &root.join("old.txt"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_577_934_245),
    )?;
    // Mode 0444; on Windows the read-only attribute.
    tools::set_mode(&root.join("readonly.txt"), 0o444)?;
    Ok(())
}

pub fn find_criteria(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("findtree");
    find_tree(&root)?;
    let mut app = App::start(Config::new(&root, &context.work));
    let window = open_find(&mut app)?;
    let all_txt = [
        "alpha.txt",
        "Beta.TXT",
        "utf16.txt",
        "old.txt",
        ".hidden.txt",
        "readonly.txt",
        "sub/alpha.txt",
        "sub/deeper/zeta.txt",
    ];
    let quick = [
        "alpha.txt",
        "Beta.TXT",
        "notes.md",
        "utf16.txt",
        "old.txt",
        ".hidden.txt",
        "sub/alpha.txt",
    ];
    let cases: Vec<(&str, Query, Vec<&str>)> = vec![
        (
            "masks *.txt",
            Query {
                masks: "*.txt",
                ..BASE
            },
            all_txt.to_vec(),
        ),
        (
            "masks *.txt, match name case",
            Query {
                masks: "*.txt",
                name_case: true,
                ..BASE
            },
            all_txt
                .iter()
                .copied()
                .filter(|path| *path != "Beta.TXT")
                .collect(),
        ),
        (
            "masks *.txt without subfolders",
            Query {
                masks: "*.txt",
                subfolders: false,
                ..BASE
            },
            all_txt
                .iter()
                .copied()
                .filter(|path| !path.contains('/'))
                .collect(),
        ),
        (
            "masks with an exclusion",
            Query {
                masks: "*.txt|alpha*",
                ..BASE
            },
            all_txt
                .iter()
                .copied()
                .filter(|path| !path.ends_with("alpha.txt"))
                .collect(),
        ),
        (
            "masks ?eta.*",
            Query {
                masks: "?eta.*",
                ..BASE
            },
            vec!["Beta.TXT", "sub/deeper/zeta.txt"],
        ),
        (
            "two masks",
            Query {
                masks: "*.dat;*.pdf",
                ..BASE
            },
            vec![
                "big.dat",
                "report-2024.pdf",
                "dupes/one.dat",
                "dupes/two.dat",
                "dupes/three.dat",
            ],
        ),
        (
            "a folder name",
            Query {
                masks: "sub",
                ..BASE
            },
            vec!["sub"],
        ),
        (
            "regex",
            Query {
                mode: "Regex",
                masks: "^[ab]",
                ..BASE
            },
            vec!["alpha.txt", "Beta.TXT", "big.dat", "sub/alpha.txt"],
        ),
        (
            "regex, match name case",
            Query {
                mode: "Regex",
                masks: "^[ab]",
                name_case: true,
                ..BASE
            },
            vec!["alpha.txt", "big.dat", "sub/alpha.txt"],
        ),
        (
            "regex alternatives",
            Query {
                mode: "Regex",
                masks: r"\.(log|md)$",
                ..BASE
            },
            vec!["gamma.log", "gamma-copy.log", "notes.md"],
        ),
        (
            "expression: size",
            Query {
                mode: "Expression",
                masks: "kind = file and size >= 2KiB",
                ..BASE
            },
            vec!["big.dat"],
        ),
        (
            "expression: size range",
            Query {
                mode: "Expression",
                masks: "size > 10 and size < 100",
                ..BASE
            },
            vec![
                "alpha.txt",
                "Beta.TXT",
                "notes.md",
                "report-2024.pdf",
                "old.txt",
                ".hidden.txt",
                "utf16.txt",
                "dupes/one.dat",
                "dupes/two.dat",
                "dupes/three.dat",
            ],
        ),
        (
            "expression: date range",
            Query {
                mode: "Expression",
                masks: "modified >= \"2020-01-01T00:00:00Z\" and modified < \"2020-01-03T00:00:00Z\"",
                ..BASE
            },
            vec!["old.txt"],
        ),
        (
            "expression: kind",
            Query {
                mode: "Expression",
                masks: "kind = directory",
                ..BASE
            },
            vec!["sub", "sub/deeper", "dupes"],
        ),
        (
            "expression: date",
            Query {
                mode: "Expression",
                masks: "modified < \"2021-01-01T00:00:00Z\"",
                ..BASE
            },
            vec!["old.txt"],
        ),
        (
            "expression: name and exact size",
            Query {
                mode: "Expression",
                masks: "name like \"*.dat\" and size = 18",
                ..BASE
            },
            vec!["dupes/one.dat", "dupes/two.dat", "dupes/three.dat"],
        ),
        (
            "content",
            Query {
                content: Some("quick"),
                ..BASE
            },
            quick.to_vec(),
        ),
        (
            "content, match case",
            Query {
                content: Some("quick"),
                content_case: true,
                ..BASE
            },
            quick
                .iter()
                .copied()
                .filter(|path| *path != "Beta.TXT")
                .collect(),
        ),
        (
            "content, whole words",
            Query {
                content: Some("quick"),
                whole_words: true,
                ..BASE
            },
            quick
                .iter()
                .copied()
                .filter(|path| *path != "notes.md")
                .collect(),
        ),
        // Regular expressions match within each line; without the option the same
        // text is literal and matches nothing.
        (
            "content regex",
            Query {
                content: Some("qu.ck"),
                content_regex: true,
                ..BASE
            },
            quick.to_vec(),
        ),
        (
            "content qu.ck without regex",
            Query {
                content: Some("qu.ck"),
                ..BASE
            },
            vec![],
        ),
        (
            "content regex, anchored line",
            Query {
                content: Some("^quick"),
                content_regex: true,
                ..BASE
            },
            vec!["Beta.TXT", "notes.md", "old.txt", "sub/alpha.txt"],
        ),
        (
            "content regex, alternatives with case",
            Query {
                content: Some("QUICK|slowly"),
                content_regex: true,
                content_case: true,
                ..BASE
            },
            vec!["Beta.TXT", "gamma.log", "gamma-copy.log"],
        ),
        (
            "content regex, whole words",
            Query {
                content: Some("qu.ck"),
                content_regex: true,
                whole_words: true,
                ..BASE
            },
            quick
                .iter()
                .copied()
                .filter(|path| *path != "notes.md")
                .collect(),
        ),
        (
            "modified within 1 day",
            Query {
                modified: Some(("1", "Days")),
                ..BASE
            },
            vec![
                "alpha.txt",
                "Beta.TXT",
                "gamma.log",
                "gamma-copy.log",
                "notes.md",
                "big.dat",
                "report-2024.pdf",
                "utf16.txt",
                ".hidden.txt",
                "readonly.txt",
                "sub",
                "sub/alpha.txt",
                "sub/deeper",
                "sub/deeper/zeta.txt",
                "dupes",
                "dupes/one.dat",
                "dupes/two.dat",
                "dupes/three.dat",
            ],
        ),
    ];
    let mut checked = 0;
    for (name, query, expected) in cases {
        let (text, rows) = search(&mut app, &window, query)?;
        if !text.starts_with("Completed") {
            return Err(format!("{name}: the search ended {text:?}"));
        }
        let found = relative(&rows, &root);
        if found != set(&expected) {
            return Err(format!(
                "{name}: found {found:?}, expected {:?}",
                set(&expected)
            ));
        }
        if tools::count_before(&text, " found") != Some(expected.len() as u64) {
            return Err(format!("{name}: the status reads {text:?}"));
        }
        checked += 1;
        if name == "content" {
            app.screenshot_window(context, &window, "content", "Containing text \"quick\"");
        }
    }

    // An invalid content pattern fails the search; the status line opens its reason.
    fill(
        &mut app,
        &window,
        Query {
            content: Some("(unclosed"),
            content_regex: true,
            ..BASE
        },
    )?;
    let text = run(&mut app, &window)?;
    if !text.starts_with("Failed · click for details") {
        return Err(format!("an invalid content regex ended {text:?}"));
    }
    let line = tools::control(window.window(), AccessibleRole::Button, &text)?;
    input::click_at(window.window(), inspect::center(&line));
    app.settle();
    if !inspect::visible_texts(window.window())
        .iter()
        .any(|shown| shown.contains("Invalid content regular expression"))
    {
        return Err(format!(
            "the details of an invalid content regex: {:?}",
            inspect::visible_texts(window.window())
        ));
    }
    app.screenshot_window(
        context,
        &window,
        "content-regex-error",
        "An invalid content regular expression, explained in the details",
    );
    input::click_button(window.window(), "Back")?;
    app.settle();

    // Identical files: one group of the two equal .dat files; the same-sized
    // different file and the unique sizes are not listed.
    let (text, rows) = search(
        &mut app,
        &window,
        Query {
            masks: "*.dat",
            identical: true,
            ..BASE
        },
    )?;
    let found = relative(&rows, &root);
    if !text.starts_with("Completed")
        || found != set(&["dupes/one.dat", "dupes/two.dat"])
        || rows
            .iter()
            .any(|row| row.group.as_deref() != Some("Group 1 · 2 files"))
    {
        return Err(format!("identical files: {text:?}, rows {rows:?}"));
    }
    if !tools::has(window.window(), AccessibleRole::Text, "Identical group") {
        return Err("identical files: no Identical group column".into());
    }
    app.screenshot_window(context, &window, "identical", "Identical files: one group");
    resize_columns(&mut app, context, &window, true)?;

    // Attributes are the provider's native flags. Linux listings report none
    // (docs/developer/metadata-filters.md): a dot-name is not `hidden` and mode
    // 0444 is not `readonly`, so the predicates match nothing and
    // missing(attributes) matches every entry.
    #[cfg(target_os = "linux")]
    {
        let (text, rows) = search(
            &mut app,
            &window,
            Query {
                mode: "Expression",
                masks: "attributes has readonly or attributes has hidden",
                ..BASE
            },
        )?;
        if !text.starts_with("Completed") || !rows.is_empty() {
            return Err(format!(
                "attribute predicates on Linux: {text:?}, {:?}",
                relative(&rows, &root)
            ));
        }
        let (text, rows) = search(
            &mut app,
            &window,
            Query {
                mode: "Expression",
                masks: "missing(attributes)",
                ..BASE
            },
        )?;
        // 16 files and 3 folders.
        if relative(&rows, &root).len() != 19 {
            return Err(format!(
                "missing(attributes) found {} entries ({text}), expected all 19",
                rows.len()
            ));
        }
    }
    // Windows listings report the native flags: only readonly.txt has the read-only
    // attribute, and a dot-name is not hidden there either.
    #[cfg(windows)]
    for (masks, wanted) in [
        ("attributes has readonly", &["readonly.txt"][..]),
        ("attributes has hidden", &[][..]),
    ] {
        let (text, rows) = search(
            &mut app,
            &window,
            Query {
                mode: "Expression",
                masks,
                ..BASE
            },
        )?;
        let found = relative(&rows, &root);
        if !text.starts_with("Completed") || found != set(wanted) {
            return Err(format!(
                "{masks:?} on Windows: {text:?}, {found:?}, expected {wanted:?}"
            ));
        }
    }
    tools::close_tool(&mut app, &window, "the Find window");
    context.note(format!(
        "{checked} criteria searches (content regular expressions included) matched their known answers exactly, plus Identical files; an invalid content pattern failed with its reason in the details"
    ));
    app.shutdown()
}

/// Drag each column's header separator: wider by 30 px, then the Name column below
/// its minimum. Columns are resized right to left so every separator stays in view.
fn resize_columns(
    app: &mut App,
    context: &mut Context,
    window: &Tool,
    identical: bool,
) -> Result<(), String> {
    let w = window.window();
    for column in ["Modified", "Size", "Path", "Extension", "Name"] {
        let label = format!("Sort by {column}");
        let before = inspect::rect(&tools::control(w, AccessibleRole::Button, &label)?);
        let grip = LogicalPosition::new(before.0 + before.2 + 5.0, before.1 + before.3 / 2.0);
        input::drag(w, grip, LogicalPosition::new(grip.x + 30.0, grip.y), 6);
        app.settle();
        let after = inspect::rect(&tools::control(w, AccessibleRole::Button, &label)?);
        if (after.2 - before.2 - 30.0).abs() > 1.5 {
            return Err(format!(
                "dragging the {column} separator by 30 px changed its width from {:.0} to {:.0}",
                before.2, after.2
            ));
        }
    }
    if identical {
        let header = tools::control(w, AccessibleRole::Text, "Identical group")?;
        let name_before =
            inspect::rect(&tools::control(w, AccessibleRole::Button, "Sort by Name")?);
        let (x, y, width, height) = inspect::rect(&header);
        let grip = LogicalPosition::new(x + width + 5.0, y + height / 2.0);
        input::drag(w, grip, LogicalPosition::new(grip.x + 30.0, grip.y), 6);
        app.settle();
        let name_after = inspect::rect(&tools::control(w, AccessibleRole::Button, "Sort by Name")?);
        if (name_after.0 - name_before.0 - 30.0).abs() > 1.5 {
            return Err(format!(
                "dragging the Identical group separator moved Name from {:.0} to {:.0}",
                name_before.0, name_after.0
            ));
        }
    }
    // Far to the left: the column keeps a minimum width.
    let before = inspect::rect(&tools::control(w, AccessibleRole::Button, "Sort by Name")?);
    let grip = LogicalPosition::new(before.0 + before.2 + 5.0, before.1 + before.3 / 2.0);
    input::drag(w, grip, LogicalPosition::new(2.0, grip.y), 8);
    app.settle();
    let after = inspect::rect(&tools::control(w, AccessibleRole::Button, "Sort by Name")?);
    if after.2 < 20.0 || after.2 >= before.2 {
        return Err(format!(
            "shrinking Name past its minimum left {:.0} px (was {:.0})",
            after.2, before.2
        ));
    }
    app.screenshot_window(
        context,
        window,
        if identical {
            "columns-identical"
        } else {
            "columns-resized"
        },
        "Every results column resized by dragging its separator",
    );
    Ok(())
}

pub fn find_folders_help_errors(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("findtree");
    find_tree(&root)?;
    let other = context.work.join("second");
    let third = context.work.join("third");
    tools::write_at(&other.join("second-only.txt"), b"two\n", SystemTime::now())?;
    tools::write_at(&third.join("third-only.txt"), b"three\n", SystemTime::now())?;
    let mut app = App::start(Config::new(&root, &context.work));
    let window = open_find(&mut app)?;
    let w = window.window();
    let picker = tools::control(w, AccessibleRole::Combobox, "Search folders")?;
    if !tools::value(&picker).ends_with("findtree") {
        return Err(format!(
            "Search folders shows {:?}, not the panel folder",
            tools::value(&picker)
        ));
    }

    // Add a typed folder.
    input::click_button(w, "Add")?;
    app.settle();
    let field = tools::control(w, AccessibleRole::TextInput, "Folder to add")?;
    input::click_at(w, inspect::center(&field));
    input::type_text(w, &other.display().to_string());
    input::key(w, Key::Return);
    app.settle();
    if !inspect::visible_texts(w)
        .iter()
        .any(|text| text == "Search folders (2)")
    {
        return Err(format!(
            "after Add the label is not \"Search folders (2)\": {:?}",
            inspect::visible_texts(w)
        ));
    }
    // Browse adds the folder the (scripted) chooser returns.
    let requests = app.picker.requests().len();
    app.picker.answer(Some(third.clone()));
    input::click_button(w, "Add")?;
    app.settle();
    input::click_button(w, "Browse…")?;
    app.until("the browsed folder", |_| {
        inspect::visible_texts(w)
            .iter()
            .any(|text| text == "Search folders (3)")
    });
    let request = app
        .picker
        .requests()
        .get(requests)
        .cloned()
        .ok_or("Browse… asked no folder chooser")?;
    if request.kind != PathPickerKind::Directory {
        return Err(format!("Browse… asked for {:?}", request.kind));
    }

    // The dropdown lists every folder with a tick box; untick the panel folder.
    input::click(w, AccessibleRole::Combobox, "Search folders")?;
    app.settle();
    let boxes: Vec<String> = inspect::roots(w)
        .last()
        .map(|popup| {
            let mut found = Vec::new();
            inspect::walk(popup, &mut |item| {
                if item.accessible_role() == AccessibleRole::Checkbox
                    && let Some(label) = inspect::label(item)
                {
                    found.push(label);
                }
            });
            found
        })
        .unwrap_or_default();
    let mut labels = Vec::new();
    for folder in [&root, &other, &third] {
        let name = folder
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        labels.push(
            boxes
                .iter()
                .find(|label| label.ends_with(&name))
                .cloned()
                .ok_or_else(|| format!("no tick box for {name} in Search folders: {boxes:?}"))?,
        );
    }
    if boxes.len() != 3 {
        return Err(format!("Search folders lists {boxes:?}"));
    }
    for label in &labels {
        if !tools::is_checked(w, label)? {
            return Err(format!("{label} is not ticked in Search folders"));
        }
    }
    app.screenshot_window(
        context,
        &window,
        "search-folders",
        "Search folders with tick boxes",
    );
    tools::click_check(w, &labels[0])?;
    app.settle();
    if tools::is_checked(w, &labels[0])? || !tools::popup_open(w) {
        return Err("unticking the panel folder failed or closed the list".into());
    }
    input::key(w, Key::Escape);
    app.settle();
    if tools::popup_open(w) {
        return Err("Escape did not close the Search folders list".into());
    }
    if !inspect::visible_texts(w)
        .iter()
        .any(|text| text == "Search folders (2 of 3)")
    {
        return Err(format!(
            "the label does not count ticked folders: {:?}",
            inspect::visible_texts(w)
        ));
    }
    let (text, rows) = search(
        &mut app,
        &window,
        Query {
            masks: "*.txt",
            ..BASE
        },
    )?;
    let names: BTreeSet<String> = rows.iter().map(|row| row.name.clone()).collect();
    if names != set(&["second-only.txt", "third-only.txt"]) {
        return Err(format!(
            "with the panel folder unticked Find returned {names:?} ({text})"
        ));
    }
    // Choose the third folder by its name and Remove it.
    input::click(w, AccessibleRole::Combobox, "Search folders")?;
    app.settle();
    input::click_button(w, &labels[2])?;
    app.settle();
    if tools::popup_open(w) {
        return Err("choosing a folder by name did not close the list".into());
    }
    input::click_button(w, "Remove")?;
    app.settle();
    if !inspect::visible_texts(w)
        .iter()
        .any(|text| text == "Search folders (1 of 2)")
    {
        return Err(format!(
            "Remove did not remove the chosen folder: {:?}",
            inspect::visible_texts(w)
        ));
    }
    // Nothing ticked: an error line that opens its details on click.
    input::click(w, AccessibleRole::Combobox, "Search folders")?;
    app.settle();
    tools::click_check(w, &labels[1])?;
    app.settle();
    input::key(w, Key::Escape);
    app.settle();
    tools::click_last(w, "Find")?;
    app.settle();
    let error = tools::control(
        w,
        AccessibleRole::Button,
        "Tick at least one search folder.",
    )?;
    if tools::description(&error) != "Click for more details" {
        return Err(format!(
            "the error line's description is {:?}",
            tools::description(&error)
        ));
    }
    input::click_at(w, inspect::center(&error));
    app.settle();
    if !tools::has(w, AccessibleRole::Button, "Back") {
        return Err("clicking the error line did not open its details".into());
    }
    input::click_button(w, "Back")?;
    app.settle();
    // Tick the panel folder again for the rest.
    input::click(w, AccessibleRole::Combobox, "Search folders")?;
    app.settle();
    tools::click_check(w, &labels[0])?;
    app.settle();
    input::key(w, Key::Escape);
    app.settle();

    // Syntax help: the title follows the mode; an example fills the query only.
    let before = status(&window);
    tools::choose_combo(&mut app, w, "Find match mode", &MODES, "Expression")?;
    input::click_button(w, "Help")?;
    app.settle();
    if !inspect::visible_texts(w)
        .iter()
        .any(|text| text == "Expression syntax")
    {
        return Err("Help did not show Expression syntax".into());
    }
    app.screenshot_window(
        context,
        &window,
        "expression-help",
        "Expression syntax help",
    );
    let example = "kind = file and size >= 10MiB";
    input::click_button(w, example)?;
    app.settle();
    let masks = tools::control(w, AccessibleRole::TextInput, "File name masks")?;
    if tools::value(&masks) != example {
        return Err(format!(
            "the example filled {:?} into the query",
            tools::value(&masks)
        ));
    }
    if status(&window) != before {
        return Err(format!(
            "using an example started a search: {:?}",
            status(&window)
        ));
    }
    input::click_button(w, "Close help")?;
    app.settle();
    for (mode, title) in [("Masks", "Masks syntax"), ("Regex", "Regex syntax")] {
        tools::choose_combo(&mut app, w, "Find match mode", &MODES, mode)?;
        let masks = tools::control(w, AccessibleRole::TextInput, "File name masks")?;
        input::click_at(w, inspect::center(&masks));
        input::key(w, Key::F1);
        app.settle();
        if !inspect::visible_texts(w).iter().any(|text| text == title) {
            return Err(format!("F1 in {mode} mode did not show {title}"));
        }
        input::key(w, Key::F1);
        app.settle();
        if inspect::visible_texts(w).iter().any(|text| text == title) {
            return Err(format!("F1 again did not hide {title}"));
        }
    }

    // An invalid expression fails; its status line opens the details on click.
    fill(
        &mut app,
        &window,
        Query {
            mode: "Expression",
            masks: "size >",
            ..BASE
        },
    )?;
    let text = run(&mut app, &window)?;
    if !text.starts_with("Failed · click for details") {
        return Err(format!("an invalid expression ended {text:?}"));
    }
    let line = tools::control(w, AccessibleRole::Button, &text)?;
    if tools::description(&line) != "Click for more details" {
        return Err(format!(
            "the status line's description is {:?}",
            tools::description(&line)
        ));
    }
    let tooltip = tools::hover_tooltip(&mut app, w, &line);
    if !tooltip.iter().any(|tip| tip == "Click for more details") {
        return Err(format!(
            "hovering the status line shows the tooltip {tooltip:?}"
        ));
    }
    app.screenshot_window(
        context,
        &window,
        "invalid-expression",
        "An invalid expression fails",
    );
    if tools::has(w, AccessibleRole::Button, "Details") {
        return Err("a Details button exists although the line opens details".into());
    }
    input::click_at(w, inspect::center(&line));
    app.settle();
    let details: Vec<String> = inspect::visible_texts(w);
    if !details
        .iter()
        .any(|text| text.contains("Operation: find files"))
        || !tools::has(w, AccessibleRole::Button, "Back")
        || !tools::has(w, AccessibleRole::Button, "Copy")
    {
        return Err(format!("the details page shows {details:?}"));
    }
    app.screenshot_window(
        context,
        &window,
        "error-details",
        "Details of the failed search",
    );
    input::click_button(w, "Back")?;
    app.settle();
    if !tools::has(w, AccessibleRole::TextInput, "File name masks") {
        return Err("Back did not return to the query".into());
    }
    tools::close_tool(&mut app, &window, "the Find window");
    context.note("Search folders: Add (typed), Browse (scripted chooser, Directory kind), untick, Remove, nothing-ticked error with details; Masks/Regex/Expression help, example use, F1; invalid expression → Failed, tooltip and details page");
    app.shutdown()
}

pub fn find_results(context: &mut Context) -> Result<(), String> {
    let many = context.fixture.path("many");
    let mut app = App::start(Config::new(&many, &context.work));
    let window = open_find(&mut app)?;
    let w = window.window();

    // A large result set: every entry of many/.
    let (text, rows) = search(&mut app, &window, BASE)?;
    let disk: BTreeSet<String> = panel::disk_names(&many).into_iter().collect();
    let names: BTreeSet<String> = rows.iter().map(|row| row.name.clone()).collect();
    if names != disk || rows.len() != disk.len() {
        return Err(format!(
            "*.* over many/ found {} rows ({} names), the disk has {}",
            rows.len(),
            names.len(),
            disk.len()
        ));
    }
    if tools::count_before(&text, " found") != Some(disk.len() as u64) {
        return Err(format!(
            "the status reads {text:?} for {} entries",
            disk.len()
        ));
    }
    let first = rows.first().map(|row| row.name.clone());
    let last = rows.last().map(|row| row.name.clone());
    app.screenshot_window(context, &window, "many", "Every entry of many/");
    // Sorting by Name again reverses the order (the Engine sorts; wait for it).
    input::click_button(w, "Sort by Name")?;
    to_top(&mut app, &window)?;
    let wanted = last.clone();
    let watched = window.clone();
    app.until("the descending order", move |_| {
        visible_rows(&watched)
            .into_values()
            .find(|row| row.index == 0)
            .map(|row| row.name)
            == wanted
    });
    let reversed = all_rows(&mut app, &window)?;
    if reversed.first().map(|row| row.name.clone()) != last
        || reversed.last().map(|row| row.name.clone()) != first
    {
        return Err(format!(
            "Sort by Name did not reverse: first {:?} last {:?} (ascending {first:?} … {last:?})",
            reversed.first().map(|row| &row.name),
            reversed.last().map(|row| &row.name)
        ));
    }
    if !tools::has(w, AccessibleRole::Text, "Name ↓")
        && !inspect::visible_texts(w)
            .iter()
            .any(|text| text == "Name ↓")
    {
        return Err("the Name header shows no descending arrow".into());
    }
    // Sort by size: sizes never decrease down the list.
    input::click_button(w, "Sort by Size")?;
    to_top(&mut app, &window)?;
    let watched = window.clone();
    app.until("the size order", move |_| {
        let mut rows: Vec<Row> = visible_rows(&watched).into_values().collect();
        rows.sort_by_key(|row| row.index);
        let sizes: Vec<u64> = rows
            .iter()
            .filter(|row| row.size.ends_with(" B"))
            .filter_map(|row| tools::number(&row.size))
            .collect();
        sizes.len() > 1 && sizes.windows(2).all(|pair| pair[0] <= pair[1])
    });
    let by_size = all_rows(&mut app, &window)?;
    let sizes: Vec<u64> = by_size
        .iter()
        .filter(|row| row.size.ends_with(" B"))
        .filter_map(|row| tools::number(&row.size))
        .collect();
    if sizes.windows(2).any(|pair| pair[0] > pair[1]) {
        return Err("Sort by Size does not order the files by size".into());
    }
    resize_columns(&mut app, context, &window, false)?;

    // Go to file with the button: the panel shows the item's folder, cursor on it.
    let target = by_size
        .iter()
        .find(|row| row.name.ends_with(".log"))
        .cloned()
        .ok_or("no .log result")?;
    to_top(&mut app, &window)?;
    let mut located = false;
    for _ in 0..by_size.len() {
        let _ = app::render(&window);
        if let Some((x, y, width, height)) = inspect::controls(w, AccessibleRole::ListItem)
            .into_iter()
            .filter(|(label, _)| label.starts_with(&format!("{} · ", target.name)))
            .find_map(|(_, item)| inspect::visible_rect(&item))
            .filter(|rect| rect.3 >= 8.0)
        {
            input::click_at(w, LogicalPosition::new(x + width / 2.0, y + height / 2.0));
            app.settle();
            located = true;
            break;
        }
        wheel(&mut app, &window, -60.0)?;
    }
    if !located {
        return Err(format!("{} never became visible", target.name));
    }
    tools::click_last(w, "Go to file")?;
    let wanted = target.name.clone();
    let folder = many.clone();
    app.until("Go to file to show the result", move |app| {
        let names = app.names(PaneId::Left);
        app.location(PaneId::Left).as_deref() == Some(folder.as_path())
            && usize::try_from(app.cursor(PaneId::Left))
                .ok()
                .and_then(|row| names.get(row))
                .is_some_and(|name| *name == wanted)
    });
    if !window.window().is_visible() {
        return Err("Go to file closed the Find window".into());
    }
    tools::close_tool(&mut app, &window, "the Find window");

    // Find retains each result's full path. Keep this count-limit fixture under
    // the supervisor's work directory so private-worker nesting does not consume
    // its 64 MiB budget before it can show the 100,000th file.
    let limit = crate::viewers_stage::scenario_work(context).join("f");
    fs::create_dir(&limit).map_err(|error| format!("{}: {error}", limit.display()))?;
    for index in 0..100_050 {
        let path = limit.join(format!("{index:05x}.f"));
        fs::write(&path, b"").map_err(|error| format!("{}: {error}", path.display()))?;
    }
    // Cancelling: a copy of many/ plus a large sparse text file keeps a content
    // search busy long enough to stop it at either scale.
    let busy = context.work.join("many-search");
    copy_tree(&many, &busy)?;
    tools::sparse(&busy.join("zz-sparse.txt"), 1 << 30)?;
    app.shutdown()?;

    let mut app = App::start(Config::new(&limit, &busy));
    let window = open_find(&mut app)?;
    let w = window.window();
    fill(&mut app, &window, BASE)?;
    let text = run(&mut app, &window)?;
    let rows = result_count(&window);
    let path_bytes = limit.join("00000.f").as_os_str().as_encoded_bytes().len();
    // Require the count ceiling on the standard Windows and Linux fixtures:
    // 160 encoded path bytes leave 511 bytes per row for its native metadata
    // and formatted timestamp within the default 64 MiB. Longer custom roots
    // can reach the memory ceiling first; verify the reason reported by Find.
    let reached = if rows == 100_000 {
        "result count"
    } else if path_bytes > 160 && (1..100_000).contains(&rows) {
        "retained memory"
    } else {
        return Err(format!(
            "100,050 files with {path_bytes}-byte paths gave {rows} rows: {text}"
        ));
    };
    if !text.starts_with("Incomplete · click for details")
        || tools::count_before(&text, " found") != Some(rows as u64)
    {
        return Err(format!("100,050 files gave {text:?} with {rows} rows"));
    }
    let visible = visible_rows(&window);
    if visible.is_empty()
        || visible
            .values()
            .any(|row| row.path() != limit.join(&row.name) || !row.path().is_file())
    {
        return Err(
            "the bounded Find list contains no visible rows or an unexpected result".into(),
        );
    }
    app.screenshot_window(
        context,
        &window,
        "limit",
        &format!("Find reached its {reached} limit at {rows} rows"),
    );
    let line = tools::control(w, AccessibleRole::Button, &text)?;
    input::click_at(w, inspect::center(&line));
    app.settle();
    let expected_detail = format!("Reached limits: {reached}");
    if !inspect::visible_texts(w)
        .iter()
        .any(|text| text.lines().any(|line| line == expected_detail))
    {
        return Err(format!(
            "the details do not name the {reached} limit: {:?}",
            inspect::visible_texts(w)
        ));
    }
    input::click_button(w, "Back")?;
    app.settle();
    tools::close_tool(&mut app, &window, "the Find window");

    // Stop a running content search.
    panel::focus_list(&mut app, PaneId::Right)?;
    let window = tools::open_tool(&mut app, "the Find window", &[Key::Alt], Key::F7, SIZE);
    let w = window.window();
    fill(
        &mut app,
        &window,
        Query {
            content: Some("needle-that-is-absent"),
            ..BASE
        },
    )?;
    tools::click_last(w, "Find")?;
    let started = Instant::now();
    let watched = window.clone();
    app.until("the content search to run", move |_| {
        status(&watched).starts_with("Searching")
            && inspect::find(watched.window(), AccessibleRole::Button, "Stop").is_some()
    });
    app.idle(Duration::from_millis(300));
    if terminal(&status(&window)) {
        return Err(format!(
            "the content search over many/ finished before Stop ({:?} after {:?})",
            status(&window),
            started.elapsed()
        ));
    }
    let running = status(&window);
    app.screenshot_window(context, &window, "running", "A running content search");
    tools::click_last(w, "Stop")?;
    let watched = window.clone();
    app.until("the search to stop", move |_| {
        status(&watched).starts_with("Cancelled")
            && inspect::find(watched.window(), AccessibleRole::Button, "Find").is_some()
    });
    let stopped = status(&window);
    app.screenshot_window(context, &window, "stopped", "Stop cancelled the search");
    tools::close_tool(&mut app, &window, "the Find window");
    app.shutdown()?;
    let fed = feed_to_panel(context)?;
    if !context.options.keep_fixture {
        fs::remove_dir_all(&limit).map_err(|error| format!("{}: {error}", limit.display()))?;
    }
    context.note(format!(
        "many/: {} results listed and sorted; 100,050 files: {rows} results, Incomplete ({reached}; {path_bytes}-byte paths); Stop: {running:?} → {stopped:?}; {fed}",
        disk.len()
    ));
    Ok(())
}

/// The names and marks of the search results the left panel shows.
fn feed_rows(app: &App) -> Vec<(String, bool)> {
    find_support::feed_rows(app, PaneId::Left)
        .into_iter()
        .map(|row| (row.name.to_string(), row.selected))
        .collect()
}

/// Mark exactly the rows named in `wanted` with Home, Insert and Down.
fn mark_feed(app: &mut App, wanted: &[&str]) -> Result<(), String> {
    panel::wait_input(app);
    input::key(app.slint(), Key::Home);
    app.settle();
    for (name, selected) in feed_rows(app) {
        let key = if selected != wanted.contains(&name.as_str()) {
            Key::Insert
        } else {
            Key::DownArrow
        };
        input::key(app.slint(), key);
        app.settle();
    }
    let marked: BTreeSet<String> = feed_rows(app)
        .into_iter()
        .filter(|(_, selected)| *selected)
        .map(|(name, _)| name)
        .collect();
    if marked != set(wanted) {
        return Err(format!("the results marked {marked:?}, wanted {wanted:?}"));
    }
    Ok(())
}

/// Feed to panel: the results become a list in the panel that opened the Find; Insert
/// marks, F5 copies and Shift+F8 deletes the real files, and Backspace shows the
/// folder again. Search inside archives without the archive provider (not installed
/// here) says so in the details instead of skipping archives silently.
fn feed_to_panel(context: &mut Context) -> Result<String, String> {
    let root = context.work.join("feed");
    let now = SystemTime::now();
    for (path, data) in [
        ("a/one.txt", "one"),
        ("b/two.txt", "two two"),
        ("b/three.log", "three"),
        ("c/four.txt", "four"),
    ] {
        tools::write_at(&root.join(path), data.as_bytes(), now)?;
    }
    tools::write_at(&root.join("bundle.zip"), b"PK\x05\x06 placeholder", now)?;
    let target = context.work.join("feed-target");
    fs::create_dir_all(&target).map_err(|error| error.to_string())?;
    let mut app = App::start(Config::new(&root, &target));
    let window = open_find(&mut app)?;
    let w = window.window();

    fill(
        &mut app,
        &window,
        Query {
            masks: "*.txt",
            archives: true,
            ..BASE
        },
    )?;
    let text = run(&mut app, &window)?;
    if !text.starts_with("Incomplete · click for details") {
        return Err(format!(
            "Search inside archives without a provider ended {text:?}"
        ));
    }
    let line = tools::control(w, AccessibleRole::Button, &text)?;
    input::click_at(w, inspect::center(&line));
    app.settle();
    if !inspect::visible_texts(w)
        .iter()
        .any(|shown| shown.contains("Search inside archives is unavailable"))
    {
        return Err(format!(
            "the details do not explain the missing archive provider: {:?}",
            inspect::visible_texts(w)
        ));
    }
    input::click_button(w, "Back")?;
    app.settle();

    let (text, rows) = search(
        &mut app,
        &window,
        Query {
            masks: "*.txt",
            ..BASE
        },
    )?;
    let found = relative(&rows, &root);
    if !text.starts_with("Completed") || found != set(&["a/one.txt", "b/two.txt", "c/four.txt"]) {
        return Err(format!("*.txt over feed/: {text:?}, {found:?}"));
    }
    tools::click_last(w, "Feed to panel")?;
    app.until("the results to fill the left panel", |app| {
        find_support::feed_view(app, PaneId::Left).open
    });
    let watched = window.clone();
    app.until("Feed to panel to close the Find window", move |_| {
        !watched.window().is_visible()
    });
    let names: BTreeSet<String> = feed_rows(&app).into_iter().map(|(name, _)| name).collect();
    if names != set(&["one.txt", "two.txt", "four.txt"]) {
        return Err(format!("the panel lists the results {names:?}"));
    }
    let title = find_support::feed_view(&app, PaneId::Left)
        .title
        .to_string();
    if !title.starts_with("Search results (3)") {
        return Err(format!("the results list is titled {title:?}"));
    }
    app.screenshot(context, "feed", "Search results fed to the left panel");

    // Insert marks two results; F5 copies those real files to the other panel.
    mark_feed(&mut app, &["one.txt", "two.txt"])?;
    let run = crate::files_support::transfer(
        &mut app,
        context,
        crate::files_support::TransferSpec::copy(),
    )?;
    crate::files_support::completed(&run, "F5 of two marked search results")?;
    for (name, bytes) in [("one.txt", "one"), ("two.txt", "two two")] {
        let copied = fs::read(target.join(name)).map_err(|error| format!("{name}: {error}"))?;
        if copied != bytes.as_bytes() {
            return Err(format!("{name} was copied as {copied:?}"));
        }
    }
    if target.join("four.txt").exists() {
        return Err("the unmarked four.txt was copied too".into());
    }

    // Shift+F8 deletes the focused result's real file, and its row.
    mark_feed(&mut app, &[])?;
    input::key(app.slint(), Key::Home);
    app.settle();
    let index = feed_rows(&app)
        .iter()
        .position(|(name, _)| name == "four.txt")
        .ok_or("four.txt is not listed")?;
    for _ in 0..index {
        input::key(app.slint(), Key::DownArrow);
        app.settle();
    }
    let progress =
        crate::files_support::delete_permanently(&mut app, context, Some("feed-delete"))?;
    crate::files_support::operation_ok(&progress, "Shift+F8 of a search result")?;
    if root.join("c/four.txt").exists() {
        return Err("Shift+F8 left c/four.txt on disk".into());
    }
    app.until("the deleted result to leave the list", |app| {
        !feed_rows(app).iter().any(|(name, _)| name == "four.txt")
    });

    // Backspace leaves the results: the panel shows its folder again.
    input::key(app.slint(), Key::Backspace);
    app.until("the folder to show again", |app| {
        !find_support::feed_view(app, PaneId::Left).open
    });
    if app.location(PaneId::Left).as_deref() != Some(root.as_path()) {
        return Err(format!(
            "after the results the panel shows {:?}",
            app.location(PaneId::Left)
        ));
    }
    let shown: BTreeSet<String> = app
        .names(PaneId::Left)
        .into_iter()
        .filter(|name| name != "..")
        .collect();
    let disk: BTreeSet<String> = panel::disk_names(&root).into_iter().collect();
    if shown != disk {
        return Err(format!("the folder lists {shown:?}, the disk has {disk:?}"));
    }
    app.shutdown()?;
    Ok("Feed to panel: 3 results listed, 2 marked and copied with F5, 1 deleted with Shift+F8 (row removed), Backspace back to the folder; Search inside archives without a provider explained in the details".into())
}
