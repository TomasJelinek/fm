//! Checksums (Ctrl+F8) beyond `checksums`: every algorithm of the dropdown (tick
//! boxes) calculated at once and checked against independent implementations, a
//! manifest saved for each, the Replace prompt's Cancel keeping the file, Include
//! subfolders, Verify preselected for every manifest kind (GNU, BSD, SFV, hash-only
//! sidecars, UTF-16) with mismatching and missing entries, a mixed manifest's error,
//! and progress and Stop on the large file. Nothing starts until a button is pressed.
//! In the results, Ctrl+C copies the shown digest, Ctrl+Shift+C the whole row, and
//! Shift+F10 and a right click open the per-algorithm copy menu.
use crate::{
    Context,
    app::{App, Config},
    encode, input, inspect, panel,
    scenarios::transfers::copy_tree,
    tools_support::{self as tools, Tool},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{
    LogicalPosition,
    platform::{Key, PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{
    fs,
    time::{Duration, Instant},
};

const SIZE: (u32, u32) = (1100, 720);
const INPUTS: [&str; 4] = ["alpha.bin", "beta.bin", "delta empty.txt", "gamma.txt"];
/// Dropdown order: (label, manifest extension).
const ALGORITHMS: [(&str, &str); 5] = [
    ("SHA-256", "sha256"),
    ("SHA-512", "sha512"),
    ("MD5", "md5"),
    ("SHA-1", "sha1"),
    ("CRC32", "sfv"),
];

/// The text of the newest clipboard copy the presenter requested since `before` was
/// taken from the trace (`→ CopyText { .., text: "…" }`).
fn copied_since(before: &[String], app: &App) -> Option<String> {
    let after = app.recent_events();
    let start = before
        .last()
        .and_then(|last| after.iter().rposition(|line| line == last))
        .map_or(0, |index| index + 1);
    after[start..]
        .iter()
        .rev()
        .filter(|line| line.starts_with("→ CopyText"))
        .find_map(|line| {
            let text = line.split_once("text: \"")?.1;
            Some(text.split('"').next().unwrap_or_default().to_owned())
        })
}

/// Wait (bounded) until the in-process clipboard holds text `accept` takes.
fn wait_clipboard(
    app: &mut App,
    what: &str,
    accept: impl Fn(&str) -> bool,
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.settle();
        let text = app
            .clipboard
            .as_ref()
            .and_then(|clipboard| clipboard.text());
        if let Some(text) = &text
            && accept(text)
        {
            return Ok(text.clone());
        }
        if Instant::now() >= deadline {
            return Err(format!("{what}: the clipboard holds {text:?}"));
        }
        app.idle(Duration::from_millis(20));
    }
}

/// Wait (bounded) for a copy request since `before`; returns its text.
fn wait_copy(app: &mut App, before: &[String], what: &str) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        app.settle();
        if let Some(text) = copied_since(before, app) {
            return Ok(text);
        }
        if Instant::now() >= deadline {
            return Err(format!("{what} requested no clipboard copy"));
        }
        app.idle(Duration::from_millis(20));
    }
}

/// The results' copy keys and menu on the row of `name`: Ctrl+C copies the shown
/// (SHA-256) digest, Ctrl+Shift+C the whole row, Shift+F10 and a right click open the
/// copy menu, whose "Copy MD5" copies that digest.
fn copy_actions(
    app: &mut App,
    window: &Tool,
    path: &str,
    name: &str,
    sha256: &str,
    md5: &str,
) -> Result<(), String> {
    let w = window.window();
    let prefix = format!("{name} · ");
    let row = inspect::controls(w, AccessibleRole::ListItem)
        .into_iter()
        .find(|(label, _)| label.starts_with(&prefix))
        .map(|(_, item)| item)
        .ok_or_else(|| format!("no result row for {name} to copy from"))?;
    let center = inspect::center(&row);
    input::click_at(w, center);
    app.settle();
    let before = app.recent_events();
    input::chord(w, &[Key::Control], "c");
    wait_copy(app, &before, "Ctrl+C on a result")?;
    let text = wait_clipboard(app, "Ctrl+C on a result", |text| {
        text.to_lowercase() == sha256
    })?;
    if text.to_lowercase() != sha256 {
        return Err(format!(
            "Ctrl+C on {name} copied {text:?}, not its SHA-256 {sha256}"
        ));
    }
    // The whole row: the file's path, then its digests.
    let before = app.recent_events();
    input::chord(w, &[Key::Control, Key::Shift], "c");
    wait_copy(app, &before, "Ctrl+Shift+C on a result")?;
    let text = wait_clipboard(app, "Ctrl+Shift+C on a result", |text| {
        text.starts_with(path) && text.to_lowercase().contains(sha256)
    })?;
    if !text.starts_with(path) || !text.to_lowercase().contains(sha256) {
        return Err(format!(
            "Ctrl+Shift+C on {name} copied {text:?}, not the whole row starting with {path}"
        ));
    }
    for (how, open) in [("Shift+F10", true), ("a right click", false)] {
        if open {
            input::chord(w, &[Key::Shift], Key::F10);
        } else {
            let button = PointerEventButton::Right;
            input::move_to(w, center);
            w.dispatch_event(WindowEvent::PointerPressed {
                position: center,
                button,
            });
            w.dispatch_event(WindowEvent::PointerReleased {
                position: center,
                button,
            });
        }
        app.settle();
        if !tools::popup_open(w) {
            return Err(format!("{how} on a result opens no copy menu"));
        }
        let offered = inspect::labels(w, AccessibleRole::Button);
        for wanted in [
            "Copy SHA-256",
            "Copy SHA-512",
            "Copy MD5",
            "Copy SHA-1",
            "Copy CRC32",
            "Copy full row",
        ] {
            if !offered.iter().any(|label| label == wanted) {
                return Err(format!("{how}'s copy menu lacks {wanted:?}: {offered:?}"));
            }
        }
        let before = app.recent_events();
        tools::click_shown(w, "Copy MD5")?;
        wait_copy(app, &before, &format!("{how}'s Copy MD5"))?;
        let text = wait_clipboard(app, &format!("{how}'s Copy MD5"), |text| {
            text.to_lowercase() == md5
        })?;
        if text.to_lowercase() != md5 {
            return Err(format!("{how}'s Copy MD5 copied {text:?}, not {md5}"));
        }
        if tools::popup_open(w) {
            return Err(format!("{how}'s copy menu stays open after copying"));
        }
    }
    Ok(())
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

fn progress(window: &Tool) -> String {
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

fn open(app: &mut App) -> Tool {
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

fn open_menu(app: &mut App, window: &Tool) -> Result<(), String> {
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
fn close_menu(app: &mut App, window: &Tool) -> Result<(), String> {
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
    let copy = format!("Copy {label}");
    if !tools::has(window.window(), AccessibleRole::Button, &copy) {
        return Err(format!(
            "choosing {label} did not make it the shown column (no {copy:?} button)"
        ));
    }
    Ok(())
}

fn wait_progress(app: &mut App, window: &Tool, what: &str, prefix: &str) -> String {
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
    if !folder.join("checksums.sha256").exists() {
        app.shutdown()
    } else {
        Err("a manifest appeared without Save".into())
    }
}
