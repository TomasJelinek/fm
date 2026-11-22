//! Checksums (Ctrl+F8) beyond `checksums`: every algorithm of the dropdown (tick
//! boxes) calculated at once and checked against independent implementations, a
//! manifest saved for each, the Replace prompt's Cancel keeping the file, Include
//! subfolders, Verify preselected for every manifest kind (GNU, BSD, SFV, hash-only
//! sidecars, UTF-16) with mismatching and missing entries, Browse… filling the Verify
//! field, a mixed manifest's error, progress and Stop on the large file, Escape during a
//! calculation, and saving partial results (the question, Cancel, Save, Replace, Details,
//! Copy details). Nothing starts until a button is pressed.
//!
//! The results select several rows (click, Ctrl+click, Ctrl+A, Ctrl+D); Ctrl+C copies the
//! selected rows' shown checksums, Ctrl+Shift+C their full rows, Shift+F10 and a right
//! click open the per-algorithm copy menu (for the selection when its row is selected,
//! else for that row), and the footer's Copy ("Copy 2 SHA-256") copies the selected rows
//! as checksum-file lines; disabled, its hint says why. Copies are read from the
//! in-process clipboard, each told apart by its count of copies.
use crate::{
    Context,
    app::{App, Config},
    encode, input, inspect, panel,
    scenarios::transfers::copy_tree,
    tools_support::{self as tools, MenuBy, Tool},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{
    LogicalPosition,
    platform::{Key, WindowAdapter},
};
use std::{
    fs,
    time::{Duration, Instant},
};

const SIZE: (u32, u32) = (1100, 720);
/// The results list's accessible label.
pub(super) const RESULTS: &str = "Checksum results";
const INPUTS: [&str; 4] = ["alpha.bin", "beta.bin", "delta empty.txt", "gamma.txt"];
/// Dropdown order: (label, manifest extension).
const ALGORITHMS: [(&str, &str); 5] = [
    ("SHA-256", "sha256"),
    ("SHA-512", "sha512"),
    ("MD5", "md5"),
    ("SHA-1", "sha1"),
    ("CRC32", "sfv"),
];

/// The results' copy keys and menu on the row of `name`: Ctrl+C copies the shown
/// (SHA-256) digest, Ctrl+Shift+C the whole row, Shift+F10 and a right click open the
/// copy menu, whose "Copy MD5" copies that digest. Each copy is told apart from the one
/// before by the in-process clipboard's count of copies, not by its text.
fn copy_actions(
    app: &mut App,
    window: &Tool,
    path: &str,
    name: &str,
    sha256: &str,
    md5: &str,
) -> Result<(), String> {
    let w = window.window();
    let row = result_row(window, name)?;
    let center = row_point(&row);
    input::click_at(w, center);
    app.settle();
    let before = tools::clipboard_writes(app);
    input::chord(w, &[Key::Control], "c");
    let text = tools::wait_copied(app, before, "Ctrl+C on a result")?;
    if text.to_lowercase() != sha256 {
        return Err(format!(
            "Ctrl+C on {name} copied {text:?}, not its SHA-256 {sha256}"
        ));
    }
    // The whole row: the file's path, then its digests.
    let before = tools::clipboard_writes(app);
    input::chord(w, &[Key::Control, Key::Shift], "c");
    let text = tools::wait_copied(app, before, "Ctrl+Shift+C on a result")?;
    if !text.starts_with(path) || !text.to_lowercase().contains(sha256) {
        return Err(format!(
            "Ctrl+Shift+C on {name} copied {text:?}, not the whole row starting with {path}"
        ));
    }
    for (how, by) in [
        ("Shift+F10", MenuBy::ShiftF10),
        ("a right click", MenuBy::Pointer(center)),
    ] {
        let offered: Vec<String> = tools::context_menu(app, w, by)
            .map_err(|error| format!("on a result: {error}"))?
            .into_iter()
            .map(|action| action.label)
            .collect();
        let wanted = [
            "Copy SHA-256",
            "Copy SHA-512",
            "Copy MD5",
            "Copy SHA-1",
            "Copy CRC32",
            "Copy full row",
        ];
        if offered != wanted {
            return Err(format!(
                "{how}'s copy menu offers {offered:?}, expected {wanted:?}"
            ));
        }
        let before = tools::clipboard_writes(app);
        tools::choose(app, w, "Copy MD5")?;
        let text = tools::wait_copied(app, before, &format!("{how}'s Copy MD5"))?;
        if text.to_lowercase() != md5 {
            return Err(format!("{how}'s Copy MD5 copied {text:?}, not {md5}"));
        }
        if tools::popup_open(w) {
            return Err(format!("{how}'s copy menu stays open after copying"));
        }
    }
    Ok(())
}

/// The result row of `name` in view.
pub(super) fn result_row(window: &Tool, name: &str) -> Result<tools::ListRow, String> {
    tools::list_rows(window, RESULTS)
        .into_iter()
        .find(|row| row.name() == name)
        .ok_or_else(|| format!("no result row for {name} in view"))
}

/// A point inside a row, clear of its left-edge marks.
pub(super) fn row_point(row: &tools::ListRow) -> LogicalPosition {
    let (x, y, w, h) = row.rect;
    LogicalPosition::new(x + w.min(240.0) / 2.0, y + h / 2.0)
}

/// The footer's Copy action (a button, or in More…), whose caption names the shown
/// algorithm and, with several rows to copy, their count ("Copy 2 SHA-256").
pub(super) fn copy_action(app: &mut App, window: &Tool) -> Result<tools::Action, String> {
    let footer = tools::footer_actions(app, window.window(), "Close")?;
    footer
        .all()
        .find(|action| {
            action.label == "Copy"
                || (action.label.starts_with("Copy ") && action.label != "Copy details")
        })
        .cloned()
        .ok_or_else(|| {
            format!(
                "the footer has no Copy: buttons {:?}, More… {:?}",
                footer.labels(),
                footer.menu_labels()
            )
        })
}

/// Whether a caption is the Copy caption for `algorithm`: "Copy SHA-256" or, with several
/// rows to copy, "Copy 3 SHA-256" (the count with the locale's grouping).
pub(super) fn copy_caption_of(caption: &str, algorithm: &str) -> Option<u64> {
    let rest = caption.strip_prefix("Copy ")?.strip_suffix(algorithm)?;
    if rest.is_empty() {
        return Some(1);
    }
    let count = rest.strip_suffix(' ')?;
    count
        .chars()
        .all(|c| {
            c.is_ascii_digit()
                || c == ','
                || c == '.'
                || c == '\u{a0}'
                || c == '\u{202f}'
                || c == ' '
        })
        .then(|| tools::number(count))
        .flatten()
}

/// The hint of a Copy that has `count` (more than one) selected rows to copy.
fn copy_hint(count: &str) -> String {
    format!(
        "Copy the {count} selected rows as a checksum file, in the list's order: the lines Save manifest… writes, named relative to the folder. Ctrl+C copies the selected checksums, Ctrl+Shift+C their full rows."
    )
}

const NOTHING_TO_COPY: &str = "Nothing to copy: no selected row has a calculated checksum. Click rows to select them (Ctrl+click adds one, Shift+click a range), or press Ctrl+A to select all.";
const VERIFICATION_NOT_COPIED: &str = "A verification's results are not copied as a checksum file. Select rows and press Ctrl+C to copy their checksums.";

/// Copies of several selected rows (all four calculated with every algorithm, SHA-256
/// shown): Ctrl+click selects two, Copy counts them; Ctrl+C copies their checksums, one
/// per line, Ctrl+Shift+C their full rows with a blank line between; Shift+F10 on a
/// selected row offers each algorithm and the full rows of both, a right click on
/// another row that row alone (the selection stays); the footer's Copy writes the two
/// checksum-file lines; Ctrl+D disables Copy and says why, without touching the panel's
/// marks; Ctrl+A counts every row.
fn selection_copies(
    app: &mut App,
    context: &mut Context,
    window: &Tool,
    folder: &std::path::Path,
    expected: &[(&str, [String; 5])],
) -> Result<(), String> {
    let w = window.window();
    let digest = |name: &str, index: usize| -> String {
        expected
            .iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, values)| values[index].clone())
            .unwrap_or_default()
    };
    let marks_before: Vec<String> = app
        .rows(PaneId::Left)
        .iter()
        .filter(|row| row.selected)
        .map(|row| row.name.to_string())
        .collect();
    let (first, second) = ("beta.bin", "gamma.txt");
    input::click_at(w, row_point(&result_row(window, first)?));
    app.settle();
    input::click_with(w, &[Key::Control], row_point(&result_row(window, second)?));
    app.settle();
    let selected = tools::selected_names(window, RESULTS);
    if selected != [first, second] {
        return Err(format!(
            "a click on {first} and a Ctrl+click on {second} selected {selected:?}"
        ));
    }
    let copy = copy_action(app, window)?;
    if copy.label != "Copy 2 SHA-256" || !copy.enabled || copy.description != copy_hint("2") {
        return Err(format!(
            "with two rows selected the footer offers {:?} (enabled {}, hint {:?})",
            copy.label, copy.enabled, copy.description
        ));
    }
    app.screenshot_window(
        context,
        window,
        "two-selected",
        "Two rows selected: Copy 2 SHA-256",
    );
    // Ctrl+C: the shown checksums of both, one per line, in the list's order.
    let before = tools::clipboard_writes(app);
    input::chord(w, &[Key::Control], "c");
    let text = tools::wait_copied(app, before, "Ctrl+C on two selected rows")?;
    let wanted = format!("{}\n{}", digest(first, 0), digest(second, 0));
    if text.to_lowercase() != wanted {
        return Err(format!(
            "Ctrl+C on two selected rows copied {text:?}, expected {wanted:?}"
        ));
    }
    // Ctrl+Shift+C: their full rows, the path and then every checksum, a blank line
    // between the rows.
    let before = tools::clipboard_writes(app);
    input::chord(w, &[Key::Control, Key::Shift], "c");
    let text = tools::wait_copied(app, before, "Ctrl+Shift+C on two selected rows")?;
    let blocks: Vec<&str> = text.split("\n\n").collect();
    let mut problems = Vec::new();
    if blocks.len() != 2 {
        problems.push(format!("{} blocks", blocks.len()));
    }
    for (block, name) in blocks.iter().zip([first, second]) {
        let mut lines = block.lines();
        let path = folder.join(name).display().to_string();
        if lines.next() != Some(path.as_str()) {
            problems.push(format!("{name}'s block does not start with {path}"));
        }
        let mut digests: Vec<(String, String)> = lines
            .map(|line| {
                let (label, hex) = line.split_once(": ").unwrap_or((line, ""));
                (label.to_owned(), hex.to_lowercase())
            })
            .collect();
        digests.sort();
        let mut wanted: Vec<(String, String)> = ALGORITHMS
            .iter()
            .enumerate()
            .map(|(index, (label, _))| ((*label).to_owned(), digest(name, index)))
            .collect();
        wanted.sort();
        if digests != wanted {
            problems.push(format!(
                "{name}'s checksums {digests:?}, expected {wanted:?}"
            ));
        }
    }
    if !problems.is_empty() {
        return Err(format!(
            "Ctrl+Shift+C on two selected rows copied {text:?}: {}",
            problems.join("; ")
        ));
    }
    // Shift+F10 on the cursor's row, which is selected: the menu acts on both rows.
    let offered: Vec<String> = tools::context_menu(app, w, MenuBy::ShiftF10)?
        .into_iter()
        .map(|action| action.label)
        .collect();
    let wanted: Vec<String> = ["SHA-256", "SHA-512", "MD5", "SHA-1", "CRC32", "full rows"]
        .iter()
        .map(|label| format!("Copy 2 {label}"))
        .collect();
    if offered != wanted {
        input::key(w, Key::Escape);
        app.settle();
        return Err(format!(
            "Shift+F10 on a selected row offers {offered:?}, expected {wanted:?}"
        ));
    }
    let before = tools::clipboard_writes(app);
    tools::choose(app, w, "Copy 2 MD5")?;
    let text = tools::wait_copied(app, before, "the menu's Copy 2 MD5")?;
    let wanted = format!("{}\n{}", digest(first, 2), digest(second, 2));
    if text.to_lowercase() != wanted {
        return Err(format!(
            "the menu's Copy 2 MD5 copied {text:?}, expected {wanted:?}"
        ));
    }
    // A right click on a row outside the selection: that row alone, and the selection
    // stays; the row becomes the cursor.
    let other = "alpha.bin";
    let offered: Vec<String> = tools::context_menu(
        app,
        w,
        MenuBy::Pointer(row_point(&result_row(window, other)?)),
    )?
    .into_iter()
    .map(|action| action.label)
    .collect();
    if offered.first().map(String::as_str) != Some("Copy SHA-256")
        || offered.last().map(String::as_str) != Some("Copy full row")
    {
        input::key(w, Key::Escape);
        app.settle();
        return Err(format!(
            "a right click on the unselected {other} offers {offered:?}, not its own row's copies"
        ));
    }
    let before = tools::clipboard_writes(app);
    tools::choose(app, w, "Copy SHA-1")?;
    let text = tools::wait_copied(app, before, "Copy SHA-1 from an unselected row's menu")?;
    if text.to_lowercase() != digest(other, 3) {
        return Err(format!(
            "Copy SHA-1 on the unselected {other} copied {text:?}, not its SHA-1 alone"
        ));
    }
    let selected = tools::selected_names(window, RESULTS);
    let cursor = tools::cursor_row(window, RESULTS).map(|row| row.name().to_owned());
    if selected != [first, second] || cursor.as_deref() != Some(other) {
        return Err(format!(
            "after the menu of the unselected {other}: selected {selected:?}, cursor {cursor:?} (expected the selection kept and the cursor on {other})"
        ));
    }
    // The footer's Copy: the checksum-file lines of the selected rows, as Save manifest…
    // writes them.
    let copy = copy_action(app, window)?;
    let before = tools::clipboard_writes(app);
    input::click_at(
        w,
        LogicalPosition::new(
            copy.rect.0 + copy.rect.2 / 2.0,
            copy.rect.1 + copy.rect.3 / 2.0,
        ),
    );
    let text = tools::wait_copied(app, before, "the footer's Copy 2 SHA-256")?;
    let wanted = format!(
        "{} *{first}\n{} *{second}\n",
        digest(first, 0),
        digest(second, 0)
    );
    if text != wanted {
        return Err(format!(
            "the footer's Copy 2 SHA-256 copied {text:?}, expected {wanted:?}"
        ));
    }
    // Ctrl+D: nothing selected, so nothing to copy, and the panel keeps its marks. The
    // footer's button has the keyboard now: a click on a row gives it back to the list.
    input::click_at(w, row_point(&result_row(window, first)?));
    app.settle();
    input::chord(w, &[Key::Control], "d");
    app.settle();
    let copy = copy_action(app, window)?;
    let selected = tools::selected_names(window, RESULTS);
    if !selected.is_empty()
        || copy.label != "Copy SHA-256"
        || copy.enabled
        || copy.description != NOTHING_TO_COPY
    {
        return Err(format!(
            "after Ctrl+D: selected {selected:?}; Copy {:?} enabled {} with hint {:?}",
            copy.label, copy.enabled, copy.description
        ));
    }
    let marks: Vec<String> = app
        .rows(PaneId::Left)
        .iter()
        .filter(|row| row.selected)
        .map(|row| row.name.to_string())
        .collect();
    if marks != marks_before {
        return Err(format!(
            "Ctrl+D in the Checksums window changed the panel's marks {marks_before:?} → {marks:?}"
        ));
    }
    app.screenshot_window(
        context,
        window,
        "nothing-selected",
        "Ctrl+D: Copy disabled, with its reason",
    );
    // Ctrl+A: every row.
    input::chord(w, &[Key::Control], "a");
    app.settle();
    let copy = copy_action(app, window)?;
    let selected = tools::selected_names(window, RESULTS);
    if selected.len() != expected.len() || copy.label != "Copy 4 SHA-256" || !copy.enabled {
        return Err(format!(
            "after Ctrl+A: selected {selected:?}; Copy {:?} enabled {}",
            copy.label, copy.enabled
        ));
    }
    // Back to one selected row for the checks that follow.
    input::click_at(w, row_point(&result_row(window, "alpha.bin")?));
    app.settle();
    Ok(())
}

/// Four Checksums windows open at once; Ctrl+F8 for a fifth opens none and says why
/// ("Close one of the 4 open checksum windows first."); after one closes, Ctrl+F8 opens
/// a window again.
fn window_limit(app: &mut App) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, "gamma.txt")?;
    let mut windows: Vec<Tool> = (0..4).map(|_| open(app)).collect();
    let open_before = app.tool_windows().len();
    input::chord(app.slint(), &[Key::Control], Key::F8);
    let deadline = Instant::now() + Duration::from_secs(15);
    let notice = loop {
        app.settle();
        if let Some(notice) = tools::shown_notice(app) {
            break Some(notice);
        }
        if app.tool_windows().len() > open_before || Instant::now() >= deadline {
            break None;
        }
        app.idle(Duration::from_millis(20));
    };
    let opened = app.tool_windows().len() - open_before;
    if tools::shown_notice(app).is_some() {
        input::key(app.slint(), Key::Escape);
        app.until("the refusal to close", |app| {
            tools::shown_notice(app).is_none()
        });
    }
    let mut problems = Vec::new();
    match &notice {
        Some((_, body))
            if body == "Close one of the 4 open checksum windows first." && opened == 0 => {}
        other => problems.push(format!(
            "a fifth Ctrl+F8 opened {opened} windows with the notice {other:?}"
        )),
    }
    if let Some(first) = windows.first().cloned() {
        tools::close_tool(app, &first, "a Checksums window");
        windows.remove(0);
        panel::focus_list(app, PaneId::Left)?;
        windows.push(open(app));
    }
    for window in &windows {
        tools::close_tool(app, window, "a Checksums window");
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Ctrl+F8 typed during a filename search: with nothing marked the window captures the
/// focused match only (gamma.txt for "gam"); with alpha.bin and beta.bin marked it
/// captures the marks. The search ends either way.
fn quick_search(app: &mut App) -> Result<(), String> {
    let calculated = |app: &mut App, marks: &[&str]| -> Result<Vec<String>, String> {
        panel::focus_list(app, PaneId::Left)?;
        input::chord(app.slint(), &[Key::Control], "d");
        for name in marks {
            panel::select_row(app, PaneId::Left, name)?;
            input::key(app.slint(), Key::Insert);
        }
        app.settle();
        input::type_text(app.slint(), "gam");
        app.until("the filename search to find gamma.txt", |app| {
            app.window.get_left_search_active()
                && usize::try_from(app.cursor(PaneId::Left))
                    .ok()
                    .and_then(|row| app.names(PaneId::Left).get(row).cloned())
                    .as_deref()
                    == Some("gamma.txt")
        });
        let window = open(app);
        let searching = app.window.get_left_search_active();
        tools::click_last(window.window(), "Calculate")?;
        wait_progress(app, &window, "the calculation", "Completed");
        let rows: Vec<String> = tools::list_rows(&window, RESULTS)
            .iter()
            .map(|row| row.name().to_owned())
            .collect();
        tools::close_tool(app, &window, "the Checksums window");
        if searching {
            return Err(format!(
                "the search stays open after Ctrl+F8 (marks {marks:?})"
            ));
        }
        Ok(rows)
    };
    let focused = calculated(app, &[])?;
    if focused != ["gamma.txt"] {
        return Err(format!(
            "with nothing marked it calculated {focused:?}, not the focused match gamma.txt"
        ));
    }
    let marked = calculated(app, &["alpha.bin", "beta.bin"])?;
    panel::focus_list(app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    if marked != ["alpha.bin", "beta.bin"] {
        return Err(format!(
            "with alpha.bin and beta.bin marked it calculated {marked:?}"
        ));
    }
    Ok(())
}

/// Below 460 px wide Copy is plain "Copy" and Save manifest… is "Save…"; an action that
/// does not fit whole beside Close moves into More…, Save… first, then Copy, while
/// Calculate stays a button; every button is whole and 8 px from the next.
fn narrow_footer(app: &mut App, context: &mut Context, window: &Tool) -> Result<(), String> {
    let mut problems = Vec::new();
    for (width, height) in [(440, 600), (320, 300)] {
        crate::app::layout_tool(window, width, height);
        app.settle();
        let footer = match tools::footer_actions(app, window.window(), "Close") {
            Ok(footer) => footer,
            Err(error) => {
                problems.push(format!("{width} px: {error}"));
                continue;
            }
        };
        app.screenshot_window(
            context,
            window,
            &format!("footer-{width}"),
            &format!("The footer at {width} px"),
        );
        let mut every: Vec<String> = footer
            .labels()
            .into_iter()
            .chain(footer.menu_labels())
            .collect();
        every.sort();
        let moved = footer.menu_labels();
        let order = ["Save…", "Copy"];
        let prefix: Vec<String> = order
            .iter()
            .take(moved.len())
            .map(|label| (*label).to_owned())
            .collect();
        let mut sorted_moved = moved.clone();
        sorted_moved.sort();
        let mut sorted_prefix = prefix.clone();
        sorted_prefix.sort();
        if every != ["Calculate", "Copy", "Save…"]
            || footer.labels().first().map(String::as_str) != Some("Calculate")
            || sorted_moved != sorted_prefix
        {
            problems.push(format!(
                "at {width} px the footer shows {:?} and More… {moved:?}: expected Calculate, Copy and Save…, with Save… moving first",
                footer.labels()
            ));
        }
        let mut row: Vec<&tools::Action> = footer.buttons.iter().collect();
        row.extend(footer.more.as_ref());
        row.push(&footer.dismiss);
        for pair in row.windows(2) {
            let gap = pair[1].rect.0 - (pair[0].rect.0 + pair[0].rect.2);
            if (gap - 8.0).abs() > 0.75 || pair[1].rect.0 + pair[1].rect.2 > width as f32 + 0.5 {
                problems.push(format!(
                    "at {width} px {:?} and {:?} are {gap:.1} px apart (the row ends at {:.1})",
                    pair[0].label,
                    pair[1].label,
                    pair[1].rect.0 + pair[1].rect.2
                ));
            }
        }
        if width == 320
            && moved.first().map(String::as_str) != Some("Save…")
            && !moved.contains(&"Save…".to_owned())
        {
            problems.push(format!("at 320 px Save… is not in More… ({moved:?})"));
        }
    }
    crate::app::layout_tool(window, SIZE.0, SIZE.1);
    app.settle();
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Every algorithm's digest of `data`, lower-case hex, in dropdown order.
fn digests(data: &[u8]) -> [String; 5] {
    [
        encode::hex(&encode::sha256(data)),
        encode::hex(&tools::sha512(data)),
        encode::hex(&encode::md5(data)),
        encode::hex(&tools::sha1(data)),
        format!("{:08x}", encode::crc32(data)),
    ]
}

pub(super) fn progress(window: &Tool) -> String {
    inspect::visible_texts(window.window())
        .into_iter()
        .rfind(|text| {
            [
                "Ready ·",
                "Calculating",
                "Verifying",
                "Stopping",
                "Completed",
                "Incomplete",
                "Cancelled",
                "Failed",
                "Error",
            ]
            .iter()
            .any(|prefix| text.starts_with(prefix))
        })
        .unwrap_or_default()
}

pub(super) fn open(app: &mut App) -> Tool {
    tools::open_tool(app, "the Checksums window", &[Key::Control], Key::F8, SIZE)
}

/// Nothing runs on its own: no result, no run and a Ready line after a pause.
fn assert_idle(app: &mut App, window: &Tool, runs: usize, what: &str) -> Result<(), String> {
    app.idle(Duration::from_millis(400));
    let line = progress(window);
    if app.checksum_runs.len() != runs
        || !line.starts_with("Ready")
        || !inspect::labels(window.window(), AccessibleRole::ListItem).is_empty()
    {
        return Err(format!(
            "{what}: work started without a button ({line:?}, {} runs)",
            app.checksum_runs.len() - runs
        ));
    }
    Ok(())
}

pub(super) fn open_menu(app: &mut App, window: &Tool) -> Result<(), String> {
    if !tools::popup_open(window.window()) {
        input::click_button(window.window(), "Checksum algorithms")?;
        app.settle();
    }
    if !tools::popup_open(window.window()) {
        return Err("the Checksum algorithms dropdown did not open".into());
    }
    Ok(())
}

/// Close the dropdown with a click outside it (Escape would close the window if the
/// popup had already gone).
pub(super) fn close_menu(app: &mut App, window: &Tool) -> Result<(), String> {
    // The middle of the results list's left half: rows only select there.
    let size = window.window().size();
    input::click_at(
        window.window(),
        LogicalPosition::new(size.width as f32 * 0.25, size.height as f32 * 0.5),
    );
    app.settle();
    if tools::popup_open(window.window()) {
        return Err("the dropdown stayed open after a click outside it".into());
    }
    Ok(())
}

/// Click an algorithm's name in the dropdown: that column becomes the shown one.
fn show_algorithm(app: &mut App, window: &Tool, label: &str) -> Result<(), String> {
    open_menu(app, window)?;
    let row = tools::control(window.window(), AccessibleRole::Checkbox, label)?;
    let (x, y, w, h) = inspect::rect(&row);
    input::click_at(
        window.window(),
        LogicalPosition::new(x + w * 0.6, y + h / 2.0),
    );
    app.settle();
    if tools::popup_open(window.window()) {
        close_menu(app, window)?;
    }
    // Copy names the shown algorithm, with a count when several rows are selected
    // ("Copy 2 SHA-512"); it copies the selection, so it is enabled with a calculated row
    // selected.
    let copy = copy_action(app, window)?;
    if copy_caption_of(&copy.label, label).is_none() {
        return Err(format!(
            "choosing {label} did not make it the shown column: Copy reads {:?}",
            copy.label
        ));
    }
    if !copy.enabled {
        return Err(format!(
            "with {label} shown and a calculated row selected, {:?} is disabled ({:?})",
            copy.label, copy.description
        ));
    }
    Ok(())
}

pub(super) fn wait_progress(app: &mut App, window: &Tool, what: &str, prefix: &str) -> String {
    let watched = window.clone();
    let wanted = prefix.to_owned();
    app.until(what, move |_| progress(&watched).starts_with(&wanted));
    progress(window)
}

/// A manifest line's (name, digest) for GNU (`hex *name`) and SFV (`name  hex`).
fn manifest_lines(text: &str, sfv: bool) -> Vec<(String, String)> {
    text.lines()
        .filter(|line| !line.trim().is_empty() && !line.starts_with(';'))
        .map(|line| {
            if sfv {
                let (name, hex) = line.rsplit_once(' ').unwrap_or((line, ""));
                (name.trim_end().to_owned(), hex.to_lowercase())
            } else {
                let (hex, name) = line.split_once(' ').unwrap_or((line, ""));
                (name.trim_start_matches('*').to_owned(), hex.to_lowercase())
            }
        })
        .collect()
}

pub fn checksum_algorithms(context: &mut Context) -> Result<(), String> {
    tools::self_test()?;
    let folder = context.work.join("sums");
    copy_tree(&context.fixture.path("checksums"), &folder)?;
    fs::create_dir_all(folder.join("nested/inner")).map_err(|error| error.to_string())?;
    fs::write(folder.join("nested/top.txt"), b"top\n").map_err(|error| error.to_string())?;
    fs::write(folder.join("nested/inner/deep.txt"), b"deep\n")
        .map_err(|error| error.to_string())?;
    let mut expected = Vec::new();
    for name in INPUTS {
        let data = fs::read(folder.join(name)).map_err(|error| error.to_string())?;
        expected.push((name, digests(&data)));
    }
    let mut config = Config::new(&folder, &context.work);
    // The copy actions reach the in-process clipboard.
    config.clipboard = true;
    let mut app = App::start(config);
    let pane = PaneId::Left;
    for name in INPUTS {
        panel::select_row(&mut app, pane, name)?;
        input::key(app.slint(), Key::Insert);
    }
    let window = open(&mut app);
    let w = window.window();
    assert_idle(&mut app, &window, 0, "opening Checksums on four files")?;

    // The dropdown: five algorithm tick boxes (SHA-256 alone by default) and
    // Include subfolders.
    open_menu(&mut app, &window)?;
    for (index, (label, _)) in ALGORITHMS.iter().enumerate() {
        if tools::is_checked(w, label)? != (index == 0) {
            return Err(format!(
                "{label} is {}ticked by default",
                if index == 0 { "not " } else { "" }
            ));
        }
    }
    if tools::is_checked(w, "Include subfolders")? {
        return Err("Include subfolders is ticked by default".into());
    }
    app.screenshot_window(context, &window, "algorithms", "The algorithm dropdown");
    for (label, _) in &ALGORITHMS[1..] {
        tools::click_check(w, label)?;
        app.settle();
        if !tools::is_checked(w, label)? {
            return Err(format!("clicking {label}'s box did not tick it"));
        }
        if !tools::popup_open(w) {
            return Err(format!("ticking {label} closed the dropdown"));
        }
    }
    close_menu(&mut app, &window)?;
    assert_idle(&mut app, &window, 0, "ticking algorithms")?;

    // Calculate: every row carries all five digests.
    tools::click_last(w, "Calculate")?;
    wait_progress(
        &mut app,
        &window,
        "the calculation",
        "Completed · 4 calculated",
    );
    let rows = inspect::labels(w, AccessibleRole::ListItem);
    for (name, values) in &expected {
        let row = rows
            .iter()
            .find(|row| row.starts_with(&format!("{name} · ")))
            .ok_or_else(|| format!("no result row for {name}: {rows:#?}"))?
            .to_lowercase();
        for ((label, _), digest) in ALGORITHMS.iter().zip(values) {
            if !row.contains(&format!("{}  {digest}", label.to_lowercase())) {
                return Err(format!("{name}: no {label} {digest} in {row:?}"));
            }
        }
    }
    app.screenshot_window(
        context,
        &window,
        "all-calculated",
        "All five algorithms calculated",
    );
    copy_actions(
        &mut app,
        &window,
        &folder.join(expected[0].0).display().to_string(),
        expected[0].0,
        &expected[0].1[0],
        &expected[0].1[2],
    )?;
    selection_copies(&mut app, context, &window, &folder, &expected)?;
    narrow_footer(&mut app, context, &window)?;

    // Save a manifest for each algorithm; the chooser suggests checksums.<ext>.
    for (label, extension) in ALGORITHMS {
        show_algorithm(&mut app, &window, label)?;
        let saves = app.manifests.len();
        let requests = app.picker.requests().len();
        tools::click_last(w, "Save manifest…")?;
        app.until(&format!("the {label} manifest"), move |app| {
            app.manifests.len() > saves
        });
        if let Some(Err(error)) = app.manifests.last() {
            return Err(format!("saving the {label} manifest failed: {error}"));
        }
        let request = app
            .picker
            .requests()
            .get(requests)
            .cloned()
            .ok_or("Save manifest… asked no file chooser")?;
        let suggested = request.kind.suggested_save_name().unwrap_or_default();
        if suggested != format!("checksums.{extension}") {
            return Err(format!("{label}: the chooser suggested {suggested:?}"));
        }
        let path = folder.join(format!("checksums.{extension}"));
        let text =
            fs::read_to_string(&path).map_err(|error| format!("{}: {error}", path.display()))?;
        let mut lines = manifest_lines(&text, extension == "sfv");
        lines.sort();
        let index = ALGORITHMS
            .iter()
            .position(|(name, _)| *name == label)
            .unwrap_or(0);
        let mut wanted: Vec<(String, String)> = expected
            .iter()
            .map(|(name, values)| ((*name).to_owned(), values[index].clone()))
            .collect();
        wanted.sort();
        if lines != wanted {
            return Err(format!(
                "checksums.{extension} holds {lines:?}, expected {wanted:?} ({text:?})"
            ));
        }
    }

    // Saving onto an existing manifest asks first; Cancel leaves it untouched.
    let md5 = folder.join("checksums.md5");
    fs::write(&md5, b"sentinel\n").map_err(|error| error.to_string())?;
    show_algorithm(&mut app, &window, "MD5")?;
    tools::click_last(w, "Save manifest…")?;
    tools::wait_text(&mut app, &window, "the Replace prompt", |text| {
        text == "checksums.md5 already exists. Replace it?"
    });
    // The refused first attempt (the file exists) is what raised the prompt.
    let saves = app.manifests.len();
    app.screenshot_window(
        context,
        &window,
        "replace-prompt",
        "Replace prompt before Cancel",
    );
    tools::click_last(w, "Cancel")?;
    app.idle(Duration::from_millis(400));
    if inspect::visible_texts(w)
        .iter()
        .any(|text| text.ends_with("already exists. Replace it?"))
    {
        return Err("Cancel did not dismiss the Replace prompt".into());
    }
    let kept = fs::read(&md5).map_err(|error| error.to_string())?;
    if kept != b"sentinel\n" || app.manifests.len() != saves {
        return Err(format!(
            "Cancel at the Replace prompt changed checksums.md5 to {:?} ({} more save results)",
            String::from_utf8_lossy(&kept),
            app.manifests.len() - saves
        ));
    }
    tools::click_last(w, "Save manifest…")?;
    tools::wait_text(&mut app, &window, "the Replace prompt again", |text| {
        text == "checksums.md5 already exists. Replace it?"
    });
    let saves = app.manifests.len();
    tools::click_last(w, "Replace")?;
    app.until("the replacing save", move |app| {
        app.manifests[saves..].iter().any(Result::is_ok)
    });
    let replaced = fs::read_to_string(&md5).map_err(|error| error.to_string())?;
    if !replaced.contains(&expected[0].1[2]) {
        return Err(format!("Replace wrote {replaced:?}"));
    }
    // Escape closes a Checksums window. After the prompt's buttons go away the window
    // can be left without keyboard focus; Tab gives it focus again.
    input::key(w, Key::Escape);
    app.idle(Duration::from_millis(300));
    let escape_ignored = window.window().is_visible();
    let mut tab_restores = false;
    if escape_ignored {
        app.screenshot_window(
            context,
            &window,
            "escape-after-replace",
            "Escape after Replace left the window open",
        );
        input::key(w, Key::Tab);
        app.settle();
        input::key(w, Key::Escape);
        app.idle(Duration::from_millis(300));
        tab_restores = !window.window().is_visible();
        if !tab_restores {
            tools::click_last(w, "Close")?;
        }
        let watched = window.clone();
        app.until("the Checksums window to close", move |_| {
            !watched.window().is_visible()
        });
    }

    // Include subfolders: a marked folder is measured file by file only with it.
    // The folder is marked: a folder under the cursor alone captures nothing.
    panel::focus_list(&mut app, pane)?;
    input::chord(app.slint(), &[Key::Control], "d");
    panel::select_row(&mut app, pane, "nested")?;
    input::key(app.slint(), Key::Insert);
    let window = open(&mut app);
    let w = window.window();
    let runs = app.checksum_runs.len();
    assert_idle(&mut app, &window, runs, "opening Checksums on a folder")?;
    tools::click_last(w, "Calculate")?;
    let flat = wait_progress(
        &mut app,
        &window,
        "the folder without subfolders",
        "Completed",
    );
    let flat_rows = inspect::labels(w, AccessibleRole::ListItem);
    open_menu(&mut app, &window)?;
    tools::click_check(w, "Include subfolders")?;
    app.settle();
    if !tools::is_checked(w, "Include subfolders")? {
        return Err("Include subfolders did not tick".into());
    }
    close_menu(&mut app, &window)?;
    tools::click_last(w, "Calculate")?;
    let deep = wait_progress(
        &mut app,
        &window,
        "the folder with subfolders",
        "Completed · 2 calculated",
    );
    let rows = inspect::labels(w, AccessibleRole::ListItem);
    for (name, data) in [("top.txt", &b"top\n"[..]), ("deep.txt", &b"deep\n"[..])] {
        let digest = encode::hex(&encode::sha256(data));
        if !rows
            .iter()
            .any(|row| row.starts_with(&format!("{name} · ")) && row.contains(&digest))
        {
            return Err(format!(
                "Include subfolders: no {name} with {digest} in {rows:#?}"
            ));
        }
    }
    app.screenshot_window(
        context,
        &window,
        "subfolders",
        "Include subfolders measures nested files",
    );
    tools::close_tool(&mut app, &window, "the Checksums window");
    context.note(format!(
        "nested/ without subfolders: {flat:?} ({} rows); with them: {deep:?}",
        flat_rows.len()
    ));
    context.note("five algorithms ticked and calculated together, each digest equal to an independent implementation; a manifest saved per algorithm (sha256, sha512, md5, sha1, sfv); Replace prompt Cancel kept the file, Replace wrote it");
    app.shutdown()?;
    if escape_ignored {
        return Err(format!(
            "finding ACC-TOOLS-05: after Replace was clicked in the save prompt, Escape no longer closed the Checksums window (suspected: keyboard focus is lost when the prompt row goes away); Tab then Escape {}",
            if tab_restores {
                "closed it"
            } else {
                "did not close it either (closed with Close)"
            }
        ));
    }
    Ok(())
}

/// One manifest to verify: (file, contents, final status prefix, passed, failed).
type Manifest = (
    &'static str,
    Vec<u8>,
    &'static str,
    Vec<&'static str>,
    Vec<&'static str>,
);

pub fn checksum_manifests(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("manifests");
    copy_tree(&context.fixture.path("checksums"), &folder)?;
    let mut data = Vec::new();
    for name in INPUTS {
        let bytes = fs::read(folder.join(name)).map_err(|error| error.to_string())?;
        data.push((name, digests(&bytes)));
    }
    let digest = |name: &str, index: usize| -> String {
        data.iter()
            .find(|(candidate, _)| *candidate == name)
            .map(|(_, values)| values[index].clone())
            .unwrap_or_default()
    };
    let bsd: String = INPUTS
        .iter()
        .map(|name| format!("SHA512 ({name}) = {}\n", digest(name, 1)))
        .collect();
    let mut gnu_sha1: String = INPUTS
        .iter()
        .map(|name| {
            let value = if *name == "gamma.txt" {
                "0".repeat(40)
            } else {
                digest(name, 3)
            };
            format!("{value} *{name}\n")
        })
        .collect();
    gnu_sha1.push_str(&format!("{} *gone.bin\n", "1".repeat(40)));
    let mut utf16 = vec![0xFF, 0xFE];
    let gnu_sha256: String = INPUTS
        .iter()
        .map(|name| format!("{}  {name}\n", digest(name, 0)))
        .collect();
    for unit in gnu_sha256.encode_utf16() {
        utf16.extend_from_slice(&unit.to_le_bytes());
    }
    let sfv = format!(
        "; generated\nalpha.bin {}\ngamma.txt 00000000\nmissing.bin {}\n",
        digest("alpha.bin", 4).to_uppercase(),
        digest("beta.bin", 4).to_uppercase()
    );
    let mixed = format!(
        "{}  alpha.bin\n{}  beta.bin\n",
        digest("alpha.bin", 0),
        digest("beta.bin", 2)
    );
    let manifests: Vec<Manifest> = vec![
        (
            "bsd.sha512",
            bsd.into_bytes(),
            "Completed · 4 verified · 0 mismatched · 0 missing",
            INPUTS.to_vec(),
            vec![],
        ),
        (
            "binary.sha1",
            gnu_sha1.into_bytes(),
            "Completed with differences",
            vec!["alpha.bin", "beta.bin", "delta empty.txt"],
            vec!["gamma.txt", "gone.bin"],
        ),
        (
            "alpha.bin.sha1",
            format!("{}\n", digest("alpha.bin", 3)).into_bytes(),
            "Completed · 1 verified",
            vec!["alpha.bin"],
            vec![],
        ),
        (
            "beta.bin.md5",
            format!("{}\n", "f".repeat(32)).into_bytes(),
            "Completed with differences",
            vec![],
            vec!["beta.bin"],
        ),
        (
            "utf16.sha256",
            utf16,
            "Completed · 4 verified",
            INPUTS.to_vec(),
            vec![],
        ),
        (
            "partial.sfv",
            sfv.into_bytes(),
            "Completed with differences",
            vec!["alpha.bin"],
            vec!["gamma.txt", "missing.bin"],
        ),
    ];
    for (name, contents, ..) in &manifests {
        fs::write(folder.join(name), contents).map_err(|error| error.to_string())?;
    }
    fs::write(folder.join("mixed.sha256"), mixed).map_err(|error| error.to_string())?;
    let mut app = App::start(Config::new(&folder, &context.work));
    let pane = PaneId::Left;
    // Escape closes a window that opened on the Verify page, before anything else.
    panel::select_row(&mut app, pane, "all.md5")?;
    let window = open(&mut app);
    input::key(window.window(), Key::Escape);
    app.idle(Duration::from_millis(300));
    let escape_ignored = window.window().is_visible();
    if escape_ignored {
        app.screenshot_window(
            context,
            &window,
            "escape-on-verify-page",
            "Escape right after opening on the Verify page",
        );
        tools::click_last(window.window(), "Close")?;
        let watched = window.clone();
        app.until("the Checksums window to close", move |_| {
            !watched.window().is_visible()
        });
    }
    for (name, _, status, passed, failed) in &manifests {
        panel::select_row(&mut app, pane, name)?;
        let runs = app.checksum_runs.len();
        let window = open(&mut app);
        let w = window.window();
        // Verify is preselected with this manifest, and nothing has run yet.
        let field = tools::control(w, AccessibleRole::TextInput, "Checksum file to verify")
            .map_err(|error| format!("{name}: the Verify page is not preselected: {error}"))?;
        let value = tools::value(&field);
        if !value.ends_with(name) {
            return Err(format!("{name}: the Verify field holds {value:?}"));
        }
        app.idle(Duration::from_millis(300));
        if app.checksum_runs.len() != runs {
            return Err(format!("{name}: verification started without Verify"));
        }
        // The pages are "Calculate" and "Verify" (once "Verify file"): on this page the
        // tab and the page's own button both say "Verify".
        let buttons = inspect::labels(w, AccessibleRole::Button);
        let verify = buttons.iter().filter(|label| *label == "Verify").count();
        if !buttons.iter().any(|label| label == "Calculate")
            || verify < 2
            || buttons.iter().any(|label| label.starts_with("Verify file"))
        {
            return Err(format!(
                "{name}: the page tabs are not Calculate and Verify: buttons {buttons:?}"
            ));
        }
        tools::click_last(w, "Verify")?;
        let line = wait_progress(&mut app, &window, &format!("{name} verification"), status);
        let images: Vec<String> = inspect::labels(w, AccessibleRole::Image)
            .into_iter()
            .filter(|label| label.starts_with("Passed: ") || label.starts_with("Failed: "))
            .collect();
        for entry in passed {
            if !images
                .iter()
                .any(|label| *label == format!("Passed: {entry}"))
            {
                return Err(format!("{name}: {entry} did not pass ({line}); {images:?}"));
            }
        }
        for entry in failed {
            if !images
                .iter()
                .any(|label| *label == format!("Failed: {entry}"))
            {
                return Err(format!("{name}: {entry} did not fail ({line}); {images:?}"));
            }
        }
        if images.len() != passed.len() + failed.len() {
            return Err(format!("{name}: unexpected results {images:?}"));
        }
        // A verification's results are not a checksum file: Copy is disabled and says so.
        let copy = copy_action(&mut app, &window)?;
        if copy.enabled || copy.description != VERIFICATION_NOT_COPIED {
            return Err(format!(
                "{name}: over a verification's results the footer's {:?} is enabled {} with hint {:?}",
                copy.label, copy.enabled, copy.description
            ));
        }
        let rows = inspect::labels(w, AccessibleRole::ListItem);
        if *name == "binary.sha1"
            && !(rows
                .iter()
                .any(|row| row.starts_with("gamma.txt · ") && row.contains("Mismatch"))
                && rows
                    .iter()
                    .any(|row| row.starts_with("gone.bin · ") && row.contains("Missing")))
        {
            return Err(format!(
                "binary.sha1: rows do not say Mismatch and Missing: {rows:#?}"
            ));
        }
        app.screenshot_window(
            context,
            &window,
            &format!("verify-{}", name.replace('.', "-")),
            &format!("Verify {name}"),
        );
        tools::close_tool(&mut app, &window, "the Checksums window");
    }
    // Ctrl+F8 during a filename search acts on the focused match, or on the marks when
    // there are any, and ends the search.
    if let Err(error) = quick_search(&mut app) {
        return Err(format!("Ctrl+F8 during a filename search: {error}"));
    }
    // Four Checksums windows at most: a fifth is refused with the reason, and closing one
    // makes room again.
    if let Err(error) = window_limit(&mut app) {
        return Err(format!("the window limit: {error}"));
    }

    // Browse… on the Verify page asks the file chooser and only fills the field with the
    // answer; Verify then checks that file.
    panel::select_row(&mut app, pane, "all.md5")?;
    let window = open(&mut app);
    let w = window.window();
    let chosen = folder.join("bsd.sha512");
    app.picker.answer(Some(chosen.clone()));
    let requests = app.picker.requests().len();
    let runs = app.checksum_runs.len();
    input::click_button(w, "Browse checksum file")?;
    let wanted = chosen.display().to_string();
    let watched = window.clone();
    let filled = wanted.clone();
    app.until("Browse… to fill the Verify field", move |_| {
        tools::control(
            watched.window(),
            AccessibleRole::TextInput,
            "Checksum file to verify",
        )
        .is_ok_and(|field| tools::value(&field) == filled)
    });
    if app.picker.requests().len() != requests + 1 {
        return Err("Browse checksum file asked no file chooser".into());
    }
    tools::click_last(w, "Verify")?;
    wait_progress(
        &mut app,
        &window,
        "the browsed manifest's verification",
        "Completed · 4 verified · 0 mismatched · 0 missing",
    );
    // Verify ran once: Browse… itself started nothing (a run of its own would be a
    // second finished run, or would have kept Verify from starting).
    let started = app.checksum_runs.len() - runs;
    tools::close_tool(&mut app, &window, "the Checksums window");
    if started != 1 {
        return Err(format!(
            "Browse… then Verify finished {started} runs: Browse… started a verification by itself"
        ));
    }

    // A manifest that mixes algorithms cannot be verified: the run fails, and the
    // window should say why.
    panel::select_row(&mut app, pane, "mixed.sha256")?;
    let window = open(&mut app);
    tools::click_last(window.window(), "Verify")?;
    let line = wait_progress(&mut app, &window, "the mixed manifest to fail", "Failed");
    app.screenshot_window(
        context,
        &window,
        "verify-mixed",
        "A manifest mixing algorithms",
    );
    let texts = inspect::visible_texts(window.window());
    let explained = texts
        .iter()
        .any(|text| text.contains("mixes different algorithms"));
    tools::close_tool(&mut app, &window, "the Checksums window");
    context.note(format!(
        "Verify preselected for BSD SHA-512, GNU binary SHA-1 (mismatch + missing), hash-only SHA-1 and MD5 sidecars, UTF-16 SHA-256 and SFV (wrong CRC + missing); mixed.sha256: {line:?}"
    ));
    app.shutdown()?;
    let mut findings = Vec::new();
    if !explained {
        findings.push(format!(
            "finding ACC-TOOLS-01: verifying mixed.sha256 ends {line:?} but the window never says why (\"Checksum manifest mixes different algorithms.\" is only in the run's diagnostics); it shows {texts:?}"
        ));
    }
    if escape_ignored {
        findings.push("finding ACC-TOOLS-06: a Checksums window that opens on the Verify page (Ctrl+F8 on a manifest) ignores Escape until something else takes focus (suspected: begin() focuses the results list, which that page hides)".into());
    }
    if findings.is_empty() {
        Ok(())
    } else {
        Err(findings.join("; "))
    }
}

pub fn checksum_large_stop(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("large");
    fs::create_dir_all(&folder).map_err(|error| error.to_string())?;
    let huge = context.fixture.path("sizes/huge.bin");
    if fs::hard_link(&huge, folder.join("huge.bin")).is_err() {
        fs::copy(&huge, folder.join("huge.bin")).map_err(|error| error.to_string())?;
    }
    // A sparse companion keeps the run going long enough to stop it at either scale.
    tools::sparse(&folder.join("sparse.bin"), 2 << 30)?;
    let mut app = App::start(Config::new(&folder, &context.work));
    for name in ["huge.bin", "sparse.bin"] {
        panel::select_row(&mut app, PaneId::Left, name)?;
        input::key(app.slint(), Key::Insert);
    }
    let window = open(&mut app);
    let w = window.window();
    assert_idle(&mut app, &window, 0, "opening Checksums on the large files")?;
    open_menu(&mut app, &window)?;
    for (label, _) in &ALGORITHMS[1..] {
        if !tools::is_checked(w, label)? {
            tools::click_check(w, label)?;
            app.settle();
        }
    }
    close_menu(&mut app, &window)?;
    tools::click_last(w, "Calculate")?;
    let started = Instant::now();
    let first = wait_progress(&mut app, &window, "the calculation to start", "Calculating");
    let watched = window.clone();
    let initial = tools::count_before(&first, " B read").unwrap_or(0);
    app.until("bytes to be read", move |_| {
        tools::count_before(&progress(&watched), " B read").unwrap_or(0) > initial
    });
    let moving = progress(&window);
    if !tools::has(w, AccessibleRole::Button, "Stop") {
        return Err(format!("no Stop button while running ({moving:?})"));
    }
    app.screenshot_window(context, &window, "running", "Progress while calculating");
    tools::click_last(w, "Stop")?;
    let stopped = wait_progress(&mut app, &window, "the run to stop", "Cancelled");
    let read = tools::count_before(&stopped, " B read").unwrap_or(0);
    let total = fs::metadata(&huge)
        .map_err(|error| error.to_string())?
        .len()
        + (2 << 30);
    if read >= total {
        return Err(format!("Stop came after every byte was read: {stopped:?}"));
    }
    if !tools::has(w, AccessibleRole::Button, "Calculate") {
        return Err("Calculate did not return after Stop".into());
    }
    let rows = inspect::labels(w, AccessibleRole::ListItem);
    if rows
        .iter()
        .any(|row| row.starts_with("sparse.bin · ") && row.contains("Calculated"))
        || !rows
            .iter()
            .any(|row| row.contains(" · Cancelled") || row.contains(" · Skipped"))
    {
        return Err(format!(
            "the unfinished file is not marked as stopped: {rows:#?}"
        ));
    }
    app.screenshot_window(context, &window, "stopped", "Stop keeps finished results");
    tools::close_tool(&mut app, &window, "the Checksums window");
    context.note(format!(
        "large files: {first:?} → {moving:?} → Stop after {:.1} s → {stopped:?}",
        started.elapsed().as_secs_f64()
    ));
    if folder.join("checksums.sha256").exists() {
        return Err("a manifest appeared without Save".into());
    }
    let mut problems = Vec::new();
    if let Err(error) = escape_while_running(&mut app, &folder) {
        problems.push(format!("Escape during a calculation: {error}"));
    }
    app.shutdown()?;
    let partial = context.work.join("partial");
    if let Err(error) = partial_save(context, &partial) {
        problems.push(format!("saving partial results: {error}"));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("\n"))
    }
}

/// Escape while a calculation runs closes the window and cancels the run: no manifest is
/// written, and Ctrl+F8 opens a new window (the closed one gave its place back).
fn escape_while_running(app: &mut App, folder: &std::path::Path) -> Result<(), String> {
    let window = open(app);
    let w = window.window();
    tools::click_last(w, "Calculate")?;
    wait_progress(app, &window, "the calculation to run", "Calculating");
    input::key(w, Key::Escape);
    let watched = window.clone();
    app.until("Escape to close the running Checksums window", move |_| {
        !watched.window().is_visible()
    });
    let window = open(app);
    let line = progress(&window);
    let rows = inspect::labels(window.window(), AccessibleRole::ListItem);
    tools::close_tool(app, &window, "the new Checksums window");
    if !line.starts_with("Ready") || !rows.is_empty() {
        return Err(format!(
            "Ctrl+F8 after Escape opened a window showing {line:?} with {} rows",
            rows.len()
        ));
    }
    let manifests: Vec<String> = fs::read_dir(folder)
        .map_err(|error| error.to_string())?
        .filter_map(Result::ok)
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("checksums."))
        .collect();
    if !manifests.is_empty() {
        return Err(format!("a cancelled run left {manifests:?}"));
    }
    Ok(())
}

/// Saving partial results: Stop after two of three files, then Save manifest… asks
/// first ("… Save the 2 completed checksums anyway?"); Cancel writes nothing, Save writes
/// the two lines and says so on the save line; saving again asks to replace, and Replace
/// says the replaced file is kept for recovery; Details and Copy details name the file
/// and the recovery copy.
fn partial_save(context: &mut Context, folder: &std::path::Path) -> Result<(), String> {
    fs::create_dir_all(folder).map_err(|error| error.to_string())?;
    for (name, data) in [
        ("a.txt", &b"first file\n"[..]),
        ("b.txt", &b"second file\n"[..]),
    ] {
        fs::write(folder.join(name), data).map_err(|error| error.to_string())?;
    }
    // Read last, long enough to stop it at any speed.
    tools::sparse(&folder.join("c.bin"), 4 << 30)?;
    let mut config = Config::new(folder, &context.work);
    config.clipboard = true;
    let mut app = App::start(config);
    for name in ["a.txt", "b.txt", "c.bin"] {
        panel::select_row(&mut app, PaneId::Left, name)?;
        input::key(app.slint(), Key::Insert);
    }
    let window = open(&mut app);
    let w = window.window();
    tools::click_last(w, "Calculate")?;
    let watched = window.clone();
    app.until(
        "a.txt and b.txt to be calculated while c.bin is read",
        move |_| {
            let rows = inspect::labels(watched.window(), AccessibleRole::ListItem);
            let done = |name: &str| {
                rows.iter()
                    .any(|row| row.starts_with(&format!("{name} · ")) && row.contains("Calculated"))
            };
            done("a.txt") && done("b.txt") && progress(&watched).starts_with("Calculating")
        },
    );
    tools::click_last(w, "Stop")?;
    let stopped = wait_progress(&mut app, &window, "the run to stop", "Cancelled");
    let manifest = folder.join("checksums.sha256");
    let prompt = |window: &Tool| {
        inspect::visible_texts(window.window())
            .into_iter()
            .find(|text| text.ends_with("anyway?"))
    };
    // Save manifest… asks first; Cancel writes nothing.
    tools::click_last(w, "Save manifest…")?;
    let asked = tools::wait_text(&mut app, &window, "the partial-save question", |text| {
        text.ends_with("anyway?")
    });
    // "1 files were excluded. …": the count and the question are checked, the plural of
    // one is a wording matter noted below.
    if !asked.starts_with("1 ")
        || !asked.contains(" excluded")
        || !asked.ends_with("Save the 2 completed checksums anyway?")
    {
        return Err(format!(
            "after {stopped:?} Save manifest… asks {asked:?}, not to save the 2 completed checksums with 1 excluded"
        ));
    }
    app.screenshot_window(
        context,
        &window,
        "partial-question",
        "Save partial results?",
    );
    let saves = app.manifests.len();
    tools::click_last(w, "Cancel")?;
    app.settle();
    if prompt(&window).is_some() || manifest.exists() || app.manifests.len() != saves {
        return Err(format!(
            "Cancel at the partial-save question: question still shown {}, {} exists {}, {} save results",
            prompt(&window).is_some(),
            manifest.display(),
            manifest.exists(),
            app.manifests.len() - saves
        ));
    }
    // Save writes the two completed checksums.
    tools::click_last(w, "Save manifest…")?;
    tools::wait_text(
        &mut app,
        &window,
        "the partial-save question again",
        |text| text.ends_with("anyway?"),
    );
    tools::click_last(w, "Save")?;
    app.until("the partial save", move |app| app.manifests.len() > saves);
    if let Some(Err(error)) = app.manifests.last() {
        return Err(format!("saving the partial manifest failed: {error}"));
    }
    let text = fs::read_to_string(&manifest)
        .map_err(|error| format!("{}: {error}", manifest.display()))?;
    let mut lines = manifest_lines(&text, false);
    lines.sort();
    let wanted: Vec<(String, String)> = [
        ("a.txt", &b"first file\n"[..]),
        ("b.txt", &b"second file\n"[..]),
    ]
    .iter()
    .map(|(name, data)| ((*name).to_owned(), encode::hex(&encode::sha256(data))))
    .collect();
    if lines != wanted {
        return Err(format!(
            "the partial manifest holds {text:?}, expected {wanted:?}"
        ));
    }
    let saved = tools::wait_text(&mut app, &window, "the save line", |text| {
        text.starts_with("Saved checksums.sha256")
    });
    if saved != "Saved checksums.sha256 · 2 included · 1 excluded" {
        return Err(format!("the save line reads {saved:?}"));
    }
    // Saving again: the question, then the file exists, so Replace is asked; Replace keeps
    // the replaced file for recovery.
    let saves = app.manifests.len();
    tools::click_last(w, "Save manifest…")?;
    tools::wait_text(
        &mut app,
        &window,
        "the partial-save question before replacing",
        |text| text.ends_with("anyway?"),
    );
    tools::click_last(w, "Save")?;
    tools::wait_text(&mut app, &window, "the Replace question", |text| {
        text == "checksums.sha256 already exists. Replace it?"
    });
    tools::click_last(w, "Replace")?;
    app.until("the replacing save", move |app| {
        app.manifests[saves..].iter().any(Result::is_ok)
    });
    let replaced = tools::wait_text(&mut app, &window, "the save line after Replace", |text| {
        text.starts_with("Saved checksums.sha256") && text.contains("recovery")
    });
    if replaced
        != "Saved checksums.sha256 · 2 included · 1 excluded · the replaced file is kept for recovery"
    {
        return Err(format!("after Replace the save line reads {replaced:?}"));
    }
    app.screenshot_window(
        context,
        &window,
        "partial-replaced",
        "A partial manifest replaced",
    );
    // Details names the saved file and the recovery copy; Copy details copies them.
    let details = inspect::find_all(w, AccessibleRole::Button, "Details")
        .into_iter()
        .rfind(|item| inspect::visible_rect(item).is_some())
        .ok_or("the save line has no Details button")?;
    input::click_at(w, inspect::center(&details));
    app.settle();
    let shown = inspect::visible_texts(w).join("\n");
    let path = manifest.display().to_string();
    if !shown.contains(&path) || !shown.contains(".midnight-recovery") {
        return Err(format!(
            "Details after a replacing save shows {shown:?}, without {path} and the recovery folder"
        ));
    }
    let before = tools::clipboard_writes(&app);
    tools::click_last(w, "Copy details")?;
    let copied = tools::wait_copied(&mut app, before, "Copy details")?;
    if !copied.contains(&path) || !copied.contains(".midnight-recovery") {
        return Err(format!(
            "Copy details copied {copied:?}, without {path} and the recovery folder"
        ));
    }
    tools::click_last(w, "Back")?;
    app.settle();
    tools::close_tool(&mut app, &window, "the Checksums window");
    context.note(format!("partial save: {asked:?}; {saved:?}; {replaced:?}"));
    app.shutdown()
}
