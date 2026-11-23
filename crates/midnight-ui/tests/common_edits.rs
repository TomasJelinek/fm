//! Every single-line text field in the application's windows is the common edit, which
//! clears inline with its ×: the stock `LineEdit` through `HintedLineEdit` (or
//! `HistoryLineEdit`, which adds recent entries), or `ThemedLineEdit` with
//! `clear-button: true` where a field needs its caret offsets or the theme's selection
//! colours. Anything else is listed below with its reason, and the list holds nothing
//! that no longer exists (docs/developer/DIALOG_ERGONOMICS.md, "Common edit").
use std::path::{Path, PathBuf};

/// Text field elements, longest names first so that `LineEdit` is not found inside
/// `HintedLineEdit`.
const FIELDS: [&str; 8] = [
    "HistoryLineEdit",
    "HintedLineEdit",
    "ThemedLineEdit",
    "ThemedSpinBox",
    "TextInput",
    "LineEdit",
    "TextEdit",
    "SpinBox",
];

/// Fields that are not the common edit, each for a reason: (file, element, the field's
/// id or accessible label, reason).
const EXCEPTIONS: &[(&str, &str, &str, &str)] = &[
    (
        "command-output-window.slint",
        "ThemedLineEdit",
        "Command line",
        "the read-only command line an output window ran: selectable, nothing to clear",
    ),
    (
        "content-controls.slint",
        "TextInput",
        "edit",
        "the implementation of ThemedLineEdit itself",
    ),
    (
        "content-controls.slint",
        "ThemedLineEdit",
        "editor",
        "the number of ThemedSpinBox, which steps with − and + instead of clearing",
    ),
    (
        "file-list-dialog.slint",
        "TextInput",
        "input",
        "the multi-line, read-only preview of the generated list",
    ),
    (
        "main.slint",
        "TextInput",
        "command-input",
        "the command line: its own history, completion and path insertion keys",
    ),
    (
        "metadata-time-field.slint",
        "TextInput",
        "input",
        "one segment (day, month, hour…) of a date or time field, stepped by the arrows",
    ),
    (
        "panel-search.slint",
        "TextInput",
        "label",
        "the panel's quick filename search, which Escape and Backspace end",
    ),
    (
        "panel-search.slint",
        "TextInput",
        "receiver",
        "the panel's quick filename search, which Escape and Backspace end",
    ),
    (
        "path-editor.slint",
        "TextInput",
        "input",
        "the path bar's editor, with its own Go button and native path paste",
    ),
    (
        "user-command-form.slint",
        "TextEdit",
        "Custom Command script",
        "a multi-line script",
    ),
    (
        "color-picker-controls.slint",
        "SpinBox",
        "spin",
        "a numeric colour channel stepped with its arrows",
    ),
    (
        "viewer.slint",
        "ThemedLineEdit",
        "Zoom percentage: Enter applies, Escape keeps the zoom",
        "the zoom label's transient editor: leaving it cancels, so an × would only cancel",
    ),
    (
        "viewer.slint",
        "ThemedSpinBox",
        "Viewer font size",
        "a numeric font size stepped with − and +",
    ),
    (
        "viewer-find-bar.slint",
        "ThemedLineEdit",
        "search-edit",
        "the viewer's find bar, whose frame already holds its own inline clear",
    ),
    (
        "comparison-dialog.slint",
        "ThemedLineEdit",
        "tolerance",
        "Compare directories' time tolerance: a short number field with its unit",
    ),
];

struct Field {
    file: String,
    element: &'static str,
    key: String,
    line: usize,
    block: String,
}

fn slint_files(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            slint_files(&path, found);
        } else if path
            .extension()
            .is_some_and(|extension| extension == "slint")
        {
            found.push(path);
        }
    }
}

/// The text from `open` (a `{`) to its matching `}`, skipping string literals.
fn block(text: &str, open: usize) -> &str {
    let bytes = text.as_bytes();
    let (mut depth, mut index, mut quoted) = (0usize, open, false);
    while index < bytes.len() {
        match bytes[index] {
            b'\\' if quoted => index += 1,
            b'"' => quoted = !quoted,
            b'{' if !quoted => depth += 1,
            b'}' if !quoted => {
                depth -= 1;
                if depth == 0 {
                    return &text[open..=index];
                }
            }
            _ => {}
        }
        index += 1;
    }
    &text[open..]
}

fn is_name(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_'
}

/// Every text field element instantiated in `text`.
fn fields(file: &str, text: &str) -> Vec<Field> {
    let mut found: Vec<Field> = Vec::new();
    let bytes = text.as_bytes();
    for element in FIELDS {
        let mut from = 0;
        while let Some(position) = text[from..].find(element) {
            let start = from + position;
            from = start + element.len();
            let end = start + element.len();
            if (start > 0 && is_name(bytes[start - 1]))
                || bytes.get(end).is_some_and(|b| is_name(*b))
            {
                continue;
            }
            let rest = text[end..].trim_start_matches([' ', '\t']);
            if !rest.starts_with('{') {
                continue;
            }
            let line_start = text[..start].rfind('\n').map_or(0, |n| n + 1);
            let before = text[line_start..start].trim_end();
            // `inherits`, `import` and `export component` lines name the element without
            // instantiating it.
            if before.ends_with("inherits") || before.contains("component ") {
                continue;
            }
            let open = text.len() - rest.len();
            let block = block(text, open).to_owned();
            let id = before
                .strip_suffix(":=")
                .map(|head| head.trim_end())
                .and_then(|head| {
                    head.rsplit(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
                        .next()
                })
                .filter(|id| !id.is_empty());
            let label = ["accessible-label: \"", "accessible-description: \""]
                .iter()
                .find_map(|prefix| {
                    let at = block.find(prefix)? + prefix.len();
                    let tail = &block[at..];
                    let end = tail.find(['"', ';'])?;
                    Some(tail[..end].to_owned())
                });
            found.push(Field {
                file: file.to_owned(),
                element,
                key: id.map(str::to_owned).or(label).unwrap_or_default(),
                line: text[..start].lines().count(),
                block,
            });
        }
    }
    found
}

fn all_fields() -> Vec<Field> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    for directory in [
        "ui",
        "../midnight-viewer-ui/ui",
        "../midnight-compare-ui/ui",
    ] {
        slint_files(&manifest.join(directory), &mut files);
    }
    files.sort();
    files
        .iter()
        .flat_map(|path| {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            fields(&name, &std::fs::read_to_string(path).unwrap())
        })
        .collect()
}

fn exception(
    field: &Field,
) -> Option<&'static (&'static str, &'static str, &'static str, &'static str)> {
    EXCEPTIONS.iter().find(|(file, element, key, _)| {
        *file == field.file && *element == field.element && *key == field.key
    })
}

#[test]
fn every_single_line_text_field_is_the_common_edit_or_a_listed_exception() {
    let fields = all_fields();
    assert!(
        fields
            .iter()
            .filter(|field| field.element == "HintedLineEdit")
            .count()
            >= 10,
        "the scan finds the common edits"
    );
    let mut failures = Vec::new();
    for field in &fields {
        let common = match field.element {
            "HintedLineEdit" | "HistoryLineEdit" => true,
            "ThemedLineEdit" => field.block.contains("clear-button: true"),
            _ => false,
        };
        if !common && exception(field).is_none() {
            failures.push(format!(
                "{}:{} {} {:?} is not the common edit (HintedLineEdit, HistoryLineEdit or ThemedLineEdit with clear-button: true) and not a listed exception",
                field.file, field.line, field.element, field.key
            ));
        }
        if common && exception(field).is_some() {
            failures.push(format!(
                "{}:{} {} {:?} is the common edit now: remove its exception",
                field.file, field.line, field.element, field.key
            ));
        }
    }
    for (file, element, key, _) in EXCEPTIONS {
        if !fields
            .iter()
            .any(|field| field.file == *file && field.element == *element && field.key == *key)
        {
            failures.push(format!(
                "the exception {file} {element} {key:?} names no field"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
