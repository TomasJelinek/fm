//! The viewer's remaining controls, used as a person would, each checked on screen:
//!
//! - The Find bar opened with the Find button: Enter, F3, Shift+F3 and the Next and
//!   Previous match buttons walk four matches in order (the highlight moves to each
//!   one's line and column), "No further match." past the last, Whole words narrows them
//!   to two, Aa (case sensitive) to none, and Clear find text empties the query and
//!   removes the highlight. Escape closes the viewer while the search field has focus.
//! - The mode picker: Source ↔ Text (syntax colours go and come back), Markdown ↔ Text
//!   (the raw markup shows).
//! - Lines (source line numbers) off removes the gutter, and stays off for the next
//!   source file; the Markdown presets GitHub, Compact and Comfortable move the text;
//!   the font size's + button makes rows taller and the family picker changes the
//!   glyphs, both kept for the next text file.
//! - PDF pages: Ctrl+PageDown, Ctrl+PageUp and the page buttons, the page counter, no
//!   page before the first or after the last. Paged and Continuous are kept for the
//!   next PDF, in both directions.
//! - Images: 1:1, Ctrl+0 (Fit) and dragging the picture at actual pixels.
//! - The typed zoom: Ctrl+Y or a click on the percentage opens a field with the shown
//!   zoom; "150", "62.5" and "150%" apply, "0" and "7000" apply the range's ends with a
//!   hint, text that is no number keeps the field open with a hint, Escape keeps the zoom
//!   and the viewer open. HTML takes 10 % to 1000 %. In a decimal-comma number format
//!   (as the Engine's read of the regional settings delivers it) the field opens with
//!   "62,5", takes "62,5", a dot and space-grouped thousands, and its hints say "0,01%"
//!   and "150,5".
//! - HTML: the zoom buttons, Ctrl+Plus, Ctrl+Minus and Ctrl+0, and the Online and
//!   JavaScript boxes (unchecked; JavaScript needs Online).
//! - A restart: the viewer choices made before it (Wrap, Lines, the Markdown preset, the
//!   text font and size, Aa, the image's 1:1 zoom, the PDF's Continuous layout and the
//!   comparator's font) are what the next start shows. Both starts share one profile
//!   (`options_support::SettingsStore`, which keeps the saved preferences without the
//!   settings file's encoding).
use crate::{
    Context,
    app::{App, Config},
    encode, input, inspect,
    options_support::SettingsStore,
    panel, tools_support as tools, viewers_stage,
    viewers_support::{self as support, Canvas, Failures, Viewing, generated, probe},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::{DocumentLayout, MarkdownPreset, NumberLocale, PaneId, ViewerZoomPreference};
use slint::{
    ComponentHandle, LogicalPosition, Model,
    platform::{Key, WindowEvent},
};
use std::{
    sync::Arc,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(20 * 60);
const OPEN_LIMIT: Duration = Duration::from_secs(60);
const STEP: Duration = Duration::from_secs(15);
const NO_MATCH: &str = "No further match";

pub fn presentation(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

fn prepare(context: &Context) -> Result<std::path::PathBuf, String> {
    let root = context.work.join("presentation");
    generated::write(
        &root,
        "words.txt",
        "Cat first line\nconcAtenate second\nCAT third\nscaT fourth\nplain tail\n",
    )?;
    generated::write(&root, "sample.rs", generated::rust(200))?;
    generated::write(&root, "second.rs", generated::rust(60))?;
    generated::write(&root, "doc.md", generated::markdown(4))?;
    generated::write(&root, "other.txt", "Other text file.\n".repeat(20))?;
    generated::write(&root, "pages.pdf", generated::pdf(6))?;
    generated::write(&root, "second.pdf", generated::pdf(3))?;
    let tall = encode::png(800, 3000, |x, y| {
        [
            (y % 256) as u8,
            ((y / 256) * 20 % 256) as u8,
            (x * 7 % 256) as u8,
        ]
    });
    generated::write(&root, "tall.png", tall)?;
    generated::write(&root, "page.html", generated::html(200))?;
    let mut random = generated::Random::new(0x5eed_0006);
    let mut bytes: Vec<u8> = (0..4096)
        .map(|_| (random.next() % 0x7F) as u8 | 0x80)
        .collect();
    bytes[2000..2006].copy_from_slice(b"needle");
    generated::write(&root, "bytes.bin", bytes)?;
    Ok(root)
}

/// A start of the application on `store`, the profile both starts share.
fn config(root: &std::path::Path, context: &Context, store: &Arc<SettingsStore>) -> Config {
    let mut config = Config::new(root, &context.work);
    config.plugins = Some(context.work.join("plugins"));
    config.settings = Some(store.clone());
    config
}

fn run(context: &mut Context) -> Result<(), String> {
    let root = prepare(context)?;
    let store = SettingsStore::new();
    let mut app = App::start(config(&root, context, &store));
    // The typed zoom reads and writes numbers in the user's number format, which the
    // Engine reads from the machine's regional settings: the checks below choose theirs
    // (the invariant format here, a decimal comma in `comma_zoom`).
    support::use_number_locale(&mut app, NumberLocale::default())?;
    let failures = Failures::default();
    type Section = fn(&mut App, &mut Context, &Viewing) -> Result<(), String>;
    // The typed zoom comes after the image section: every zoom is saved and the next
    // image opens with it, while the image section starts from Fit.
    let sections: [(&str, &str, Section); 9] = [
        ("Find bar", "words.txt", find_bar),
        ("Source and Text modes, Lines", "sample.rs", source_modes),
        ("Markdown and Text modes, presets", "doc.md", markdown_modes),
        ("font size and family", "words.txt", fonts),
        ("PDF pages", "pages.pdf", pdf_pages),
        ("image 1:1, Fit and dragging", "tall.png", image_controls),
        ("typed zoom", "tall.png", typed_zoom),
        (
            "HTML zoom, typed zoom, policy and Find",
            "page.html",
            html_controls,
        ),
        ("Find in the hex view", "bytes.bin", hex_find),
    ];
    for (what, file, section) in sections {
        if file.ends_with(".html")
            && let Some(reason) = probe::html_unavailable()
        {
            failures.skip(what, &reason);
            continue;
        }
        let result = open(&mut app, file).and_then(|viewing| {
            let result = section(&mut app, context, &viewing);
            // A section may close its viewer itself (Escape in the Find bar). One that
            // stopped early may leave the zoom field open, where Escape closes only the
            // field; the section's own failure is the one reported.
            let closed = if viewing.slint().is_visible() {
                if viewing.window.get_zoom_editing() {
                    input::key(viewing.slint(), Key::Escape);
                    let _ =
                        viewing.wait(&mut app, "Escape to close the zoom field", STEP, |window| {
                            !window.get_zoom_editing()
                        });
                }
                viewing.close(&mut app)
            } else {
                Ok(())
            };
            result.and(closed)
        });
        if file.ends_with(".html") {
            probe::check_html(&failures, what, result);
        } else {
            failures.check(what, result);
        }
    }
    failures.check(
        "preferences kept for the next file",
        preferences_kept(&mut app, context),
    );
    failures.check(
        "the PDF layout kept for the next PDF",
        layout_for_next_pdf(&mut app, context),
    );
    failures.check(
        "the typed zoom in a decimal-comma number format",
        comma_zoom(&mut app, context),
    );
    let chosen = choose_for_restart(&mut app, context);
    app.shutdown()?;
    match chosen {
        Ok(chosen) => {
            failures.check(
                "viewer preferences saved in the profile",
                saved(&store, &chosen),
            );
            let mut app = App::start(config(&root, context, &store));
            failures.check(
                "viewer preferences after a restart",
                restored(&mut app, context, &chosen),
            );
            app.shutdown()?;
        }
        Err(error) => failures.check("viewer preferences chosen before a restart", Err(error)),
    }
    failures.finish(context)
}

fn open(app: &mut App, file: &str) -> Result<Viewing, String> {
    panel::select_row(app, PaneId::Left, file)?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    if let Err(error) = viewing.result.clone() {
        // The window shows the viewer's report: closed before the next file opens.
        let _ = viewing.close(app);
        return Err(format!("{file}: {error}"));
    }
    viewing.wait(app, "the first content", STEP, |window| {
        let status = window.get_status();
        !status.is_empty() && !status.starts_with("Opening")
    })?;
    Ok(viewing)
}

/// The bounding box of the pixels that differ between two renders: (left, top, right,
/// bottom), exclusive.
fn changed(base: &Canvas, now: &Canvas) -> Option<(usize, usize, usize, usize)> {
    let mut box_: Option<(usize, usize, usize, usize)> = None;
    for y in 0..base.height.min(now.height) {
        for x in 0..base.width.min(now.width) {
            if base.at(x, y) != now.at(x, y) {
                box_ = Some(match box_ {
                    None => (x, y, x + 1, y + 1),
                    Some((l, t, r, b)) => (l.min(x), t.min(y), r.max(x + 1), b.max(y + 1)),
                });
            }
        }
    }
    box_
}

/// Where the match highlight is: the top-left of the pixels painted in the highlight's
/// colour where the unsearched page showed its background. The pointer leaves the
/// window first: the tooltip of a find bar control it rests on ("Next match (F3)",
/// "Find whole words") would cover the page's corner and be taken for the highlight.
fn highlight(app: &mut App, viewing: &Viewing, base: &Canvas) -> Result<(usize, usize), String> {
    super::ui_gate::pointer_out(viewing.slint());
    let now = probe::still_canvas(app, viewing, STEP)?;
    let background = probe::dominant(base);
    let (colour, _) = probe::painted_over(base, &now, background)
        .ok_or_else(|| format!("no match is highlighted; status {:?}", viewing.status()))?;
    let mut corner: Option<(usize, usize)> = None;
    for y in 0..base.height.min(now.height) {
        for x in 0..base.width.min(now.width) {
            if base.at(x, y) == background && now.at(x, y) == colour {
                corner = Some(match corner {
                    None => (x, y),
                    Some((left, top)) => (left.min(x), top.min(y)),
                });
            }
        }
    }
    corner.ok_or_else(|| format!("no match is highlighted; status {:?}", viewing.status()))
}

/// Start a search with `act` and wait for its answer: the status no longer says
/// "Searching…" and the page or the status differs from before (each search here moves
/// the highlight, or turns "No further match." on or off), then the page holds still.
/// The status then read is the search's own, never the previous one.
fn search_settled(
    app: &mut App,
    viewing: &Viewing,
    act: impl FnOnce(&slint::Window),
) -> Result<String, String> {
    let before = (viewing.status(), viewing.canvas().fingerprint());
    act(viewing.slint());
    viewing.wait(app, "the search's answer", STEP, |window| {
        let status = window.get_status();
        !status.contains("Searching")
            && (status.as_str() != before.0 || viewing.canvas().fingerprint() != before.1)
    })?;
    probe::still_canvas(app, viewing, STEP)?;
    Ok(viewing.status())
}

/// Enter in the find field.
fn enter(window: &slint::Window) {
    input::key(window, Key::Return);
}

/// F3 (Shift+F3 `backwards`).
fn next(window: &slint::Window, backwards: bool) {
    if backwards {
        input::chord(window, &[Key::Shift], Key::F3);
    } else {
        input::key(window, Key::F3);
    }
}

fn find_bar(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    let window = &viewing.window;
    input::click_button(viewing.slint(), "Find")?;
    viewing.wait(
        app,
        "the Find button to open the find bar",
        STEP,
        |window| window.get_search_visible(),
    )?;
    // The page as shown with the find bar open and nothing found yet.
    let base = probe::still_canvas(app, viewing, STEP)?;
    input::type_text(viewing.slint(), "cat");
    search_settled(app, viewing, enter)?;
    let mut spots = vec![highlight(app, viewing, &base)?];
    // F3, the Next match button, F3: the second, third and fourth match.
    search_settled(app, viewing, |window| next(window, false))?;
    spots.push(highlight(app, viewing, &base)?);
    let mut clicked = Ok(());
    search_settled(app, viewing, |window| {
        clicked = input::click_button(window, "Next match");
    })?;
    clicked?;
    spots.push(highlight(app, viewing, &base)?);
    search_settled(app, viewing, |window| next(window, false))?;
    spots.push(highlight(app, viewing, &base)?);
    app.screenshot_window(
        context,
        &viewing.software,
        "find-fourth-match",
        "Find \"cat\": the fourth match (scaT) after Enter, F3, Next match, F3",
    );
    // Matches: "Cat" line 1 column 1, "cAt" line 2 column 4, "CAT" line 3 column 1,
    // "caT" line 4 column 2: each lower than the one before, columns as written.
    let rows: Vec<usize> = spots.iter().map(|(_, top)| *top).collect();
    let columns: Vec<usize> = spots.iter().map(|(left, _)| *left).collect();
    if !(rows.windows(2).all(|pair| pair[0] < pair[1])
        && columns[0].abs_diff(columns[2]) <= 1
        && columns[1] > columns[3]
        && columns[3] > columns[0])
    {
        return Err(format!(
            "the four matches were highlighted at {spots:?} (left, top); expected lines going \
             down and columns 1, 4, 1, 2"
        ));
    }
    let status = search_settled(app, viewing, |window| next(window, false))?;
    if !status.contains(NO_MATCH) {
        return Err(format!("F3 after the last match: status {status:?}"));
    }
    search_settled(app, viewing, |window| next(window, true))?;
    let back = highlight(app, viewing, &base)?;
    let mut clicked = Ok(());
    search_settled(app, viewing, |window| {
        clicked = input::click_button(window, "Previous match");
    })?;
    clicked?;
    let before = highlight(app, viewing, &base)?;
    if back.1 >= spots[3].1 || before.1 >= back.1 {
        return Err(format!(
            "Shift+F3 and Previous match highlighted {back:?} and {before:?} after the fourth \
             match at {:?}",
            spots[3]
        ));
    }
    // Whole words: only "Cat" and "CAT".
    if !window.get_whole_words_available() {
        return Err("Whole words is unavailable in a text file".into());
    }
    input::click(
        viewing.slint(),
        AccessibleRole::Checkbox,
        "Find whole words",
    )?;
    input::chord(viewing.slint(), &[Key::Control], "f");
    app.settle();
    input::replace_text(viewing.slint(), "cat");
    search_settled(app, viewing, enter)?;
    // Collect every whole-word match from wherever the search stands: forwards to the
    // end, then backwards to the start.
    let mut whole = std::collections::BTreeSet::new();
    for backwards in [false, true] {
        for _ in 0..4 {
            if !viewing.status().contains(NO_MATCH) {
                whole.insert(highlight(app, viewing, &base)?.1);
            }
            if search_settled(app, viewing, |window| next(window, backwards))?.contains(NO_MATCH) {
                break;
            }
        }
    }
    let whole_rows: Vec<usize> = whole.iter().copied().collect();
    let wanted = [rows[0], rows[2]];
    if whole_rows.len() != 2
        || whole_rows
            .iter()
            .zip(wanted)
            .any(|(a, b)| a.abs_diff(b) > 1)
    {
        return Err(format!(
            "Whole words highlighted rows {whole_rows:?}; expected the first and third lines \
             {wanted:?}"
        ));
    }
    input::click(
        viewing.slint(),
        AccessibleRole::Checkbox,
        "Find whole words",
    )?;
    // Aa: "cat" in lower case appears nowhere. Found first without Aa, so that the
    // case-sensitive search's "No further match." is its own answer.
    input::chord(viewing.slint(), &[Key::Control], "f");
    app.settle();
    input::replace_text(viewing.slint(), "cat");
    let status = search_settled(app, viewing, enter)?;
    if status.contains(NO_MATCH) {
        return Err(format!("\"cat\" without Aa found nothing: {status:?}"));
    }
    input::click(viewing.slint(), AccessibleRole::Checkbox, "Case sensitive")?;
    input::chord(viewing.slint(), &[Key::Control], "f");
    app.settle();
    input::replace_text(viewing.slint(), "cat");
    let status = search_settled(app, viewing, enter)?;
    if !status.contains(NO_MATCH) {
        return Err(format!(
            "a case-sensitive \"cat\" found something: {status:?}"
        ));
    }
    input::click(viewing.slint(), AccessibleRole::Checkbox, "Case sensitive")?;
    // Clear find text empties the query and removes the highlight.
    input::chord(viewing.slint(), &[Key::Control], "f");
    app.settle();
    input::replace_text(viewing.slint(), "third");
    search_settled(app, viewing, enter)?;
    highlight(app, viewing, &base)?;
    input::click_button(viewing.slint(), "Clear find text")?;
    viewing.wait(app, "Clear find text to empty the query", STEP, |window| {
        window.get_query().is_empty()
    })?;
    let cleared = probe::still_canvas(app, viewing, STEP)?;
    if let Some(left) = changed(&base, &cleared) {
        return Err(format!(
            "after Clear find text the page still differs from the unsearched page at {left:?}"
        ));
    }
    // Escape closes the viewer even while the search field has the focus.
    input::chord(viewing.slint(), &[Key::Control], "f");
    app.settle();
    input::type_text(viewing.slint(), "x");
    input::key(viewing.slint(), Key::Escape);
    let deadline = std::time::Instant::now() + STEP;
    while support::software_window(app, viewing.slint()).is_some() {
        if std::time::Instant::now() >= deadline {
            return Err("Escape in the search field did not close the viewer".into());
        }
        app.idle(Duration::from_millis(10));
    }
    app.slint()
        .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
    app.settle();
    context.note(format!(
        "Find: matches at {spots:?}; Shift+F3 {back:?}, Previous {before:?}; whole words \
         {whole_rows:?}"
    ));
    Ok(())
}

/// Pick `mode` in the mode picker.
fn choose_mode(app: &mut App, viewing: &Viewing, mode: &str) -> Result<(), String> {
    let modes: Vec<String> = viewing
        .window
        .get_modes()
        .iter()
        .map(|mode| mode.to_string())
        .collect();
    let index = modes
        .iter()
        .position(|candidate| candidate == mode)
        .ok_or_else(|| format!("no {mode} mode among {modes:?}"))?;
    input::click(viewing.slint(), AccessibleRole::Combobox, "Viewer mode")?;
    app.settle();
    input::click(viewing.slint(), AccessibleRole::Button, mode)?;
    viewing.wait(app, &format!("the {mode} mode"), STEP, |window| {
        window.get_mode_index() == index as i32 && !window.get_status().starts_with("Opening")
    })
}

/// The number of text colours on the page.
fn text_colours(app: &mut App, viewing: &Viewing) -> Result<usize, String> {
    let canvas = probe::still_canvas(app, viewing, STEP)?;
    Ok(probe::glyph_cores(&canvas, probe::dominant(&canvas), 24).len())
}

/// The leftmost column with ink in the canvas.
fn leftmost_ink(canvas: &Canvas) -> Option<usize> {
    let background = probe::dominant(canvas);
    (0..canvas.width).find(|x| (0..canvas.height).any(|y| canvas.at(*x, y) != background))
}

fn source_modes(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    viewing.wait(app, "the source", STEP, |window| window.get_source_mode())?;
    let coloured = text_colours(app, viewing)?;
    choose_mode(app, viewing, "Text")?;
    viewing.wait(app, "plain text", STEP, |window| !window.get_source_mode())?;
    let plain = text_colours(app, viewing)?;
    app.screenshot_window(
        context,
        &viewing.software,
        "source-as-text",
        "sample.rs in the Text mode: no syntax colours",
    );
    choose_mode(app, viewing, "Source")?;
    viewing.wait(app, "the source again", STEP, |window| {
        window.get_source_mode()
    })?;
    let again = text_colours(app, viewing)?;
    if plain >= coloured || again < coloured {
        return Err(format!(
            "text colours: Source {coloured}, Text {plain}, Source again {again}"
        ));
    }
    // Lines off: the gutter goes, the text moves to the left edge.
    let with_numbers = leftmost_ink(&probe::still_canvas(app, viewing, STEP)?);
    input::click(
        viewing.slint(),
        AccessibleRole::Checkbox,
        "Source line numbers",
    )?;
    viewing.wait(app, "Lines to turn off", STEP, |window| {
        !window.get_line_numbers()
    })?;
    let canvas = probe::still_canvas(app, viewing, STEP)?;
    let without = leftmost_ink(&canvas);
    app.screenshot_window(
        context,
        &viewing.software,
        "lines-off",
        "Lines off: sample.rs without its gutter",
    );
    let text_start = canvas_text_start(&canvas);
    context.note(format!(
        "Lines: leftmost ink {with_numbers:?} with numbers, {without:?} without; text starts at \
         {text_start:?}"
    ));
    match (with_numbers, text_start) {
        (Some(numbers), Some(start)) if start + 4 < numbers => {}
        _ => {
            return Err(format!(
                "Lines off did not remove the gutter: leftmost ink {with_numbers:?} with \
                 numbers, text from {text_start:?} without"
            ));
        }
    }
    Ok(())
}

/// Where text starts without a gutter: the leftmost ink column.
fn canvas_text_start(canvas: &Canvas) -> Option<usize> {
    leftmost_ink(canvas)
}

fn markdown_modes(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    viewing.wait(app, "rendered Markdown", STEP, |window| {
        window.get_markdown_mode() && window.get_accessible_text().contains("Long document")
    })?;
    let mut lefts = Vec::new();
    for (index, preset) in ["GitHub", "Compact", "Comfortable"].iter().enumerate() {
        if viewing.window.get_markdown_preset() != index as i32 {
            input::click(
                viewing.slint(),
                AccessibleRole::Combobox,
                "Markdown presentation",
            )?;
            app.settle();
            input::click(viewing.slint(), AccessibleRole::Button, preset)?;
            viewing.wait(app, preset, STEP, |window| {
                window.get_markdown_preset() == index as i32
            })?;
        }
        let canvas = probe::still_canvas(app, viewing, STEP)?;
        lefts.push((preset, leftmost_ink(&canvas), canvas.fingerprint()));
    }
    app.screenshot_window(
        context,
        &viewing.software,
        "markdown-comfortable",
        "doc.md with the Comfortable preset",
    );
    context.note(format!("Markdown presets: leftmost ink {lefts:?}"));
    if lefts[0].2 == lefts[1].2 || lefts[1].2 == lefts[2].2 || lefts[1].1 >= lefts[0].1 {
        return Err(format!(
            "the presets do not change the layout as described (Compact has the narrowest \
             margin): {lefts:?}"
        ));
    }
    input::click(
        viewing.slint(),
        AccessibleRole::Combobox,
        "Markdown presentation",
    )?;
    app.settle();
    input::click(viewing.slint(), AccessibleRole::Button, "GitHub")?;
    viewing.wait(app, "GitHub again", STEP, |window| {
        window.get_markdown_preset() == 0
    })?;
    // The Text mode shows the Markdown source.
    choose_mode(app, viewing, "Text")?;
    viewing.wait(app, "the Markdown source", STEP, |window| {
        !window.get_markdown_mode() && window.get_accessible_text().contains("# Long document")
    })?;
    choose_mode(app, viewing, "Markdown")?;
    viewing.wait(app, "rendered Markdown again", STEP, |window| {
        window.get_markdown_mode()
    })
}

/// The median height of the canvas's text rows.
fn row_height(canvas: &Canvas) -> usize {
    let background = probe::dominant(canvas);
    let mut heights = Vec::new();
    let mut start = None;
    for y in 0..canvas.height {
        let ink = (0..canvas.width).any(|x| canvas.at(x, y) != background);
        match (ink, start) {
            (true, None) => start = Some(y),
            (false, Some(top)) => {
                heights.push(y - top);
                start = None;
            }
            _ => {}
        }
    }
    heights.sort_unstable();
    heights.get(heights.len() / 2).copied().unwrap_or(0)
}

/// Click the + of the font size spin box.
fn bigger(app: &mut App, viewing: &Viewing) -> Result<(), String> {
    let item = inspect::find(viewing.slint(), AccessibleRole::Spinbox, "Viewer font size")
        .ok_or("no Viewer font size spin box")?;
    let (x, y, width, height) = inspect::rect(&item);
    input::click_at(
        viewing.slint(),
        LogicalPosition::new(x + width - 12.0, y + height / 2.0),
    );
    app.settle();
    Ok(())
}

fn fonts(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    if !viewing.window.get_typography_visible() {
        input::click(
            viewing.slint(),
            AccessibleRole::Button,
            "Font and presentation",
        )?;
        viewing.wait(app, "the font row", STEP, |window| {
            window.get_typography_visible()
        })?;
    }
    let size = viewing.window.get_viewer_font_size();
    let before = probe::still_canvas(app, viewing, STEP)?;
    let short = row_height(&before);
    bigger(app, viewing)?;
    bigger(app, viewing)?;
    viewing.wait(app, "the + button to enlarge the font", STEP, |window| {
        window.get_viewer_font_size() == size + 2
    })?;
    let after = probe::still_canvas(app, viewing, STEP)?;
    let tall = row_height(&after);
    if tall <= short {
        return Err(format!(
            "font size {size} → {}: text rows {short} px → {tall} px",
            size + 2
        ));
    }
    let family = viewing.window.get_viewer_font_family().to_string();
    let wanted = if family == "serif" {
        "sans-serif"
    } else {
        "serif"
    };
    input::click(
        viewing.slint(),
        AccessibleRole::Combobox,
        "Viewer font family",
    )?;
    app.settle();
    input::click(viewing.slint(), AccessibleRole::Button, wanted)?;
    viewing.wait(app, "the new font family", STEP, |window| {
        window.get_viewer_font_family() == wanted
    })?;
    let other = probe::still_canvas(app, viewing, STEP)?;
    if other.fingerprint() == after.fingerprint() {
        return Err(format!("the {wanted} family renders like {family}"));
    }
    app.screenshot_window(
        context,
        &viewing.software,
        "font-family",
        &format!("words.txt in {wanted} at {} px", size + 2),
    );
    context.note(format!(
        "Fonts: {family} {size} px rows {short} px; {} px rows {tall} px; then {wanted}",
        size + 2
    ));
    Ok(())
}

/// What the next text file shows of the choices made before: Lines off for source,
/// the bigger font and the family for text. Restores the defaults afterwards.
fn preferences_kept(app: &mut App, context: &mut Context) -> Result<(), String> {
    let viewing = open(app, "second.rs")?;
    let lines = viewing.window.get_line_numbers();
    if !lines {
        input::click(
            viewing.slint(),
            AccessibleRole::Checkbox,
            "Source line numbers",
        )?;
        viewing.wait(app, "Lines back on", STEP, |window| {
            window.get_line_numbers()
        })?;
    }
    viewing.close(app)?;
    let viewing = open(app, "other.txt")?;
    let (size, family) = (
        viewing.window.get_viewer_font_size(),
        viewing.window.get_viewer_font_family().to_string(),
    );
    app.screenshot_window(
        context,
        &viewing.software,
        "preferences-kept",
        &format!("other.txt opened after the font changes: {family} {size} px"),
    );
    viewing.close(app)?;
    if lines {
        return Err("Lines turned off in one source file is on again in the next".into());
    }
    if size == 14 || family == "monospace" {
        return Err(format!(
            "the next text file shows {family} {size} px instead of the changed font"
        ));
    }
    Ok(())
}

fn pdf_pages(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    viewing.wait(app, "six pages", STEP, |window| {
        window.get_document_pages() == 6
    })?;
    if viewing.window.get_document_layout() != 0 {
        input::click(
            viewing.slint(),
            AccessibleRole::Button,
            "Paged document layout",
        )?;
        viewing.wait(app, "the Paged layout", STEP, |window| {
            window.get_document_layout() == 0
        })?;
    }
    viewing.click_content(app);
    let mut prints = vec![probe::still_canvas(app, viewing, STEP)?.fingerprint()];
    let steps: [(&str, i32); 5] = [
        ("Ctrl+PageDown", 2),
        ("Next document page (Ctrl+PageDown)", 3),
        ("Ctrl+PageUp", 2),
        ("Previous document page (Ctrl+PageUp)", 1),
        ("Ctrl+PageUp at the first page", 1),
    ];
    for (action, page) in steps {
        match action {
            "Ctrl+PageDown" => input::chord(viewing.slint(), &[Key::Control], Key::PageDown),
            "Ctrl+PageUp" | "Ctrl+PageUp at the first page" => {
                input::chord(viewing.slint(), &[Key::Control], Key::PageUp)
            }
            button => input::click(viewing.slint(), AccessibleRole::Button, button)?,
        }
        viewing.wait(app, action, STEP, |window| {
            window.get_document_page() == page
        })?;
        prints.push(probe::still_canvas(app, viewing, STEP)?.fingerprint());
    }
    let previous = inspect::find(
        viewing.slint(),
        AccessibleRole::Button,
        "Previous document page (Ctrl+PageUp)",
    )
    .ok_or("no Previous document page button")?;
    if tools::enabled(&previous) {
        return Err("Previous document page is enabled on the first page".into());
    }
    // The pages differ; going back shows the same page again.
    if prints[0] == prints[1] || prints[1] == prints[2] || prints[1] != prints[3] {
        return Err(format!("page renders: {prints:?}"));
    }
    for _ in 0..6 {
        input::chord(viewing.slint(), &[Key::Control], Key::PageDown);
        app.idle(Duration::from_millis(100));
    }
    viewing.wait(app, "the last page", STEP, |window| {
        window.get_document_page() == 6
    })?;
    let next = inspect::find(
        viewing.slint(),
        AccessibleRole::Button,
        "Next document page (Ctrl+PageDown)",
    )
    .ok_or("no Next document page button")?;
    if tools::enabled(&next) {
        return Err("Next document page is enabled on the last page".into());
    }
    let counter = inspect::find(
        viewing.slint(),
        AccessibleRole::Text,
        "Document page counter",
    )
    .and_then(|item| {
        item.accessible_string_property(
            i_slint_core::accessibility::AccessibleStringProperty::Value,
        )
    })
    .map(|value| value.to_string());
    if counter.as_deref() != Some("6 / 6") {
        return Err(format!(
            "the page counter reads {counter:?} on the last page"
        ));
    }
    app.screenshot_window(
        context,
        &viewing.software,
        "pdf-last-page",
        "pages.pdf on its last page (Paged layout)",
    );
    Ok(())
}

fn image_controls(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    viewing.wait(app, "the image", STEP, |window| window.get_image_mode())?;
    let fit = viewing.window.get_zoom_percent();
    input::click(
        viewing.slint(),
        AccessibleRole::Button,
        "Actual image pixels",
    )?;
    viewing.wait(app, "1:1", STEP, |window| {
        (window.get_zoom_percent() - 100.0).abs() < 0.5
    })?;
    // Dragging the picture up by 120 pixels shows 120 pixels further down.
    let before = probe::still_canvas(app, viewing, STEP)?;
    let middle = probe::content_middle(viewing);
    probe::drag(
        app,
        viewing.slint(),
        middle,
        LogicalPosition::new(middle.x, middle.y - 120.0),
        6,
    );
    let after = probe::still_canvas(app, viewing, STEP)?;
    let shift = probe::vertical_shift(&before, &after);
    if !shift.is_some_and(|(pixels, share)| {
        pixels.abs_diff(120) <= 1 && share >= probe::SHIFT_CONFIDENCE
    }) {
        return Err(format!(
            "dragging the picture 120 px up moved it {shift:?} (pixels, share of rows)"
        ));
    }
    viewing.click_content(app);
    input::chord(viewing.slint(), &[Key::Control], "0");
    viewing.wait(app, "Ctrl+0 to fit", STEP, |window| {
        (window.get_zoom_percent() - fit).abs() < 0.5
    })?;
    app.screenshot_window(
        context,
        &viewing.software,
        "image-fit-again",
        &format!("tall.png fitted again with Ctrl+0 ({fit:.1}%)"),
    );
    if fit >= 100.0 {
        return Err(format!(
            "an 800 x 3000 image fits a smaller window at {fit:.1}%"
        ));
    }
    Ok(())
}

/// The shown zoom as the zoom field starts with it: at most `decimals` decimal places,
/// without trailing zeros (`zoom_entry::format`, invariant number format).
fn zoom_text(percent: f32, decimals: usize) -> String {
    let text = format!("{percent:.decimals$}");
    if text.contains('.') {
        text.trim_end_matches('0').trim_end_matches('.').to_owned()
    } else {
        text
    }
}

/// Type `text` into the open zoom field and press Enter; wait until the field closed
/// with the zoom at `percent` (within `tolerance`) and the hint `hint`, or, for `None`,
/// until the field stayed open with the hint and the zoom unchanged.
fn type_zoom(
    app: &mut App,
    viewing: &Viewing,
    text: &str,
    percent: Option<(f32, f32)>,
    hint: &str,
) -> Result<(), String> {
    if !viewing.window.get_zoom_editing() {
        return Err(format!("the zoom field is not open to type {text:?}"));
    }
    let before = viewing.window.get_zoom_percent();
    input::replace_text(viewing.slint(), text);
    input::key(viewing.slint(), Key::Return);
    let what = format!("{text:?} typed into the zoom field");
    viewing
        .wait(app, &what, STEP, |window| {
            let zoom = window.get_zoom_percent();
            window.get_zoom_hint() == hint
                && match percent {
                    Some((wanted, tolerance)) => {
                        !window.get_zoom_editing() && (zoom - wanted).abs() <= tolerance
                    }
                    None => window.get_zoom_editing() && zoom == before,
                }
        })
        .map_err(|error| {
            format!(
                "{error}; zoom {:.2}% (wanted {percent:?}), field open {}, hint {:?} (wanted \
                 {hint:?})",
                viewing.window.get_zoom_percent(),
                viewing.window.get_zoom_editing(),
                viewing.window.get_zoom_hint()
            )
        })
}

/// Open the zoom field with Ctrl+Y from the content, or with a click on the shown
/// percentage (`label`); it starts with the shown zoom (`decimals` places).
fn open_zoom_field(
    app: &mut App,
    viewing: &Viewing,
    click: Option<&str>,
    decimals: usize,
) -> Result<(), String> {
    open_zoom_field_in(app, viewing, click, decimals, ".")
}

/// [`open_zoom_field`] in a number format whose decimal separator is `decimal`.
fn open_zoom_field_in(
    app: &mut App,
    viewing: &Viewing,
    click: Option<&str>,
    decimals: usize,
    decimal: &str,
) -> Result<(), String> {
    let shown = zoom_text(viewing.window.get_zoom_percent(), decimals).replace('.', decimal);
    match click {
        Some(label) => input::click(viewing.slint(), AccessibleRole::Button, label)?,
        None => input::chord(viewing.slint(), &[Key::Control], "y"),
    }
    let how = click.map_or("Ctrl+Y".to_owned(), |label| format!("a click on {label:?}"));
    viewing.wait(
        app,
        &format!("{how} to open the zoom field"),
        STEP,
        |window| window.get_zoom_editing(),
    )?;
    let entry = viewing.window.get_zoom_entry().to_string();
    if entry != shown {
        return Err(format!(
            "{how} opened the zoom field with {entry:?}, not the shown zoom {shown:?}"
        ));
    }
    Ok(())
}

/// The typed zoom of an image: Ctrl+Y and a click on the percentage open the field with
/// the shown zoom; "150", "62.5" and "150%" apply; "0" and "7000" apply the range's ends
/// (0.01 % and 6400 %) and say so; text without a number keeps the field open with the
/// range in its hint and changes nothing; Escape closes only the field, keeping the
/// zoom. Ctrl+0 fits the picture again for the sections after this one.
fn typed_zoom(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    viewing.wait(app, "the image", STEP, |window| window.get_image_mode())?;
    viewing.click_content(app);
    open_zoom_field(app, viewing, None, 1)?;
    type_zoom(app, viewing, "150", Some((150.0, 0.05)), "")?;
    open_zoom_field(app, viewing, Some("Image zoom percentage"), 1)?;
    type_zoom(app, viewing, "62.5", Some((62.5, 0.05)), "")?;
    open_zoom_field(app, viewing, None, 1)?;
    type_zoom(app, viewing, "150%", Some((150.0, 0.05)), "")?;
    open_zoom_field(app, viewing, None, 1)?;
    type_zoom(
        app,
        viewing,
        "0",
        Some((0.01, 0.005)),
        "The smallest zoom is 0.01%",
    )?;
    open_zoom_field(app, viewing, None, 1)?;
    type_zoom(
        app,
        viewing,
        "7000",
        Some((6400.0, 0.5)),
        "The largest zoom is 6400%",
    )?;
    open_zoom_field(app, viewing, Some("Image zoom percentage"), 1)?;
    type_zoom(
        app,
        viewing,
        "abc",
        None,
        "Type a zoom percentage from 0.01% to 6400%, for example 150.5",
    )?;
    app.screenshot_window(
        context,
        &viewing.software,
        "typed-zoom-hint",
        "tall.png: \"abc\" typed into the zoom field keeps it open with the range",
    );
    // Escape closes only the field; the zoom and the viewer stay.
    input::key(viewing.slint(), Key::Escape);
    viewing.wait(app, "Escape to close only the zoom field", STEP, |window| {
        !window.get_zoom_editing()
    })?;
    if support::software_window(app, viewing.slint()).is_none() {
        return Err("Escape in the zoom field closed the viewer".into());
    }
    if (viewing.window.get_zoom_percent() - 6400.0).abs() > 0.5 {
        return Err(format!(
            "Escape in the zoom field changed the zoom to {:.2}%",
            viewing.window.get_zoom_percent()
        ));
    }
    viewing.click_content(app);
    input::chord(viewing.slint(), &[Key::Control], "0");
    viewing.wait(app, "Ctrl+0 to fit the picture again", STEP, |window| {
        window.get_zoom_percent() < 100.0
    })
}

/// The typed zoom in a decimal-comma number format with space-grouped thousands
/// (`support::comma_locale`), made the user's format as the Engine's read of the regional
/// settings delivers it: the field opens with the shown zoom in that format; "62,5"
/// applies 62.5 % and the field opens again with "62,5"; a dot works too there ("75.5":
/// the format does not group digits with dots), and so do grouped thousands ("1 000,5");
/// the limit and refusal hints write the comma ("0,01%", "150,5"). The format the
/// application had before is restored.
fn comma_zoom(app: &mut App, context: &mut Context) -> Result<(), String> {
    let before = app.presenter.borrow().number_locale().clone();
    support::use_number_locale(app, support::comma_locale())?;
    let result = open(app, "tall.png").and_then(|viewing| {
        let result = comma_entries(app, context, &viewing);
        if viewing.slint().is_visible() {
            viewing.close(app)?;
        }
        result
    });
    let restored = support::use_number_locale(app, before);
    result.and(restored)
}

fn comma_entries(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    const FIELD: &str = "Image zoom percentage";
    viewing.wait(app, "the image", STEP, |window| window.get_image_mode())?;
    viewing.click_content(app);
    open_zoom_field_in(app, viewing, None, 1, ",")?;
    type_zoom(app, viewing, "62,5", Some((62.5, 0.05)), "")?;
    open_zoom_field_in(app, viewing, Some(FIELD), 1, ",")?;
    type_zoom(app, viewing, "75.5", Some((75.5, 0.05)), "")?;
    open_zoom_field_in(app, viewing, None, 1, ",")?;
    type_zoom(app, viewing, "1 000,5", Some((1000.5, 0.05)), "")?;
    open_zoom_field_in(app, viewing, None, 1, ",")?;
    type_zoom(
        app,
        viewing,
        "0",
        Some((0.01, 0.005)),
        "The smallest zoom is 0,01%",
    )?;
    open_zoom_field_in(app, viewing, Some(FIELD), 1, ",")?;
    type_zoom(
        app,
        viewing,
        "abc",
        None,
        "Type a zoom percentage from 0,01% to 6400%, for example 150,5",
    )?;
    app.screenshot_window(
        context,
        &viewing.software,
        "typed-zoom-comma",
        "tall.png in a decimal-comma number format: \"abc\" keeps the field open with the range",
    );
    input::key(viewing.slint(), Key::Escape);
    viewing.wait(app, "Escape to close only the zoom field", STEP, |window| {
        !window.get_zoom_editing()
    })?;
    viewing.click_content(app);
    input::chord(viewing.slint(), &[Key::Control], "0");
    viewing.wait(app, "Ctrl+0 to fit the picture again", STEP, |window| {
        window.get_zoom_percent() < 100.0
    })
}

/// The typed zoom of HTML: 10 % to 1000 %, through a click on the percentage.
fn html_typed_zoom(app: &mut App, viewing: &Viewing) -> Result<(), String> {
    const FIELD: &str = "HTML zoom percentage";
    open_zoom_field(app, viewing, Some(FIELD), 0)?;
    type_zoom(app, viewing, "250", Some((250.0, 0.5)), "")?;
    open_zoom_field(app, viewing, Some(FIELD), 0)?;
    type_zoom(
        app,
        viewing,
        "5",
        Some((10.0, 0.5)),
        "The smallest zoom is 10%",
    )?;
    open_zoom_field(app, viewing, Some(FIELD), 0)?;
    type_zoom(
        app,
        viewing,
        "2000",
        Some((1000.0, 0.5)),
        "The largest zoom is 1000%",
    )?;
    open_zoom_field(app, viewing, Some(FIELD), 0)?;
    type_zoom(
        app,
        viewing,
        "x",
        None,
        "Type a zoom percentage from 10% to 1000%, for example 150.5",
    )?;
    input::key(viewing.slint(), Key::Escape);
    viewing.wait(app, "Escape to close only the zoom field", STEP, |window| {
        !window.get_zoom_editing()
    })?;
    input::chord(viewing.slint(), &[Key::Control], "0");
    viewing.wait(app, "Ctrl+0 to reset the HTML zoom", STEP, |window| {
        window.get_zoom_percent().round() as i32 == 100
    })
}

/// The PDF layout chosen last is the next PDF's: Continuous in pages.pdf opens
/// second.pdf Continuous, and Paged there opens pages.pdf Paged.
fn layout_for_next_pdf(app: &mut App, context: &mut Context) -> Result<(), String> {
    let steps: [(&str, i32, i32); 3] = [
        ("pages.pdf", 6, 1),
        ("second.pdf", 3, 0),
        ("pages.pdf", 6, -1),
    ];
    let mut expected: Option<i32> = None;
    let mut seen = Vec::new();
    for (file, pages, choose) in steps {
        let viewing = open(app, file)?;
        let result = (|| {
            viewing.wait(app, &format!("the pages of {file}"), STEP, |window| {
                window.get_document_pages() == pages && window.get_document_layout() >= 0
            })?;
            let layout = viewing.window.get_document_layout();
            seen.push((file, layout));
            if let Some(expected) = expected
                && layout != expected
            {
                return Err(format!(
                    "{file} opened in layout {layout}, not the layout {expected} chosen in the \
                     PDF before (0 Paged, 1 Continuous)"
                ));
            }
            if choose >= 0 {
                let label = if choose == 1 {
                    "Continuous document layout"
                } else {
                    "Paged document layout"
                };
                if layout != choose {
                    input::click(viewing.slint(), AccessibleRole::Button, label)?;
                }
                viewing.wait(app, label, STEP, |window| {
                    window.get_document_layout() == choose
                })?;
                expected = Some(choose);
            }
            Ok(())
        })();
        viewing.close(app)?;
        result?;
    }
    context.note(format!(
        "PDF layout for the next PDF (0 Paged, 1 Continuous): {seen:?}"
    ));
    Ok(())
}

fn html_controls(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    viewing.wait(app, "the page", STEP, |window| window.get_web_mode())?;
    let zoom = |viewing: &Viewing| viewing.window.get_zoom_percent().round() as i32;
    let start = zoom(viewing);
    input::click(viewing.slint(), AccessibleRole::Button, "Zoom HTML in")?;
    viewing.wait(app, "Zoom HTML in", STEP, |window| {
        window.get_zoom_percent().round() as i32 > start
    })?;
    let larger = zoom(viewing);
    input::click(viewing.slint(), AccessibleRole::Button, "Zoom HTML out")?;
    viewing.wait(app, "Zoom HTML out", STEP, |window| {
        (window.get_zoom_percent().round() as i32) < larger
    })?;
    input::chord(viewing.slint(), &[Key::Control], "+");
    input::chord(viewing.slint(), &[Key::Control], "+");
    viewing.wait(app, "Ctrl+Plus", STEP, |window| {
        window.get_zoom_percent().round() as i32 > start
    })?;
    input::chord(viewing.slint(), &[Key::Control], "0");
    viewing.wait(app, "Ctrl+0 to reset the zoom", STEP, |window| {
        window.get_zoom_percent().round() as i32 == 100
    })?;
    html_typed_zoom(app, viewing)?;
    let online = inspect::find(
        viewing.slint(),
        AccessibleRole::Checkbox,
        "Allow network for this document only",
    )
    .ok_or("no Online box")?;
    let scripts = inspect::find(
        viewing.slint(),
        AccessibleRole::Checkbox,
        "Allow scripts to read and transmit this document",
    )
    .ok_or("no JavaScript box")?;
    if viewing.window.get_web_network() || viewing.window.get_web_scripts() {
        return Err("a new document starts online or with scripts".into());
    }
    if tools::enabled(&scripts) {
        return Err("JavaScript can be ticked while Online is off".into());
    }
    context.note(format!(
        "HTML: zoom {start}% → {larger}% → back; Ctrl+0 100%; Online enabled {}, policy change \
         allowed {}",
        tools::enabled(&online),
        viewing.window.get_web_policy_change_allowed()
    ));
    // Find in the page: Chromium's own search, reported on the status line.
    let top = probe::still_canvas(app, viewing, STEP)?;
    input::chord(viewing.slint(), &[Key::Control], "f");
    viewing.wait(app, "the find bar", STEP, |window| {
        window.get_search_visible()
    })?;
    input::type_text(viewing.slint(), "Paragraph 150:");
    input::key(viewing.slint(), Key::Return);
    // Worded as the comparator's Find (worker_web.rs `find_status`): the count alone
    // ("1 match") until Chromium activates the match, then "Match 1 of 1", after the
    // page's policy (`web_presentation_status`; the boxes are unchecked again here).
    const FOUND: &str = "Offline · JavaScript off · Match 1 of 1";
    const ABSENT: &str = "Offline · JavaScript off · No matches";
    viewing.wait(app, &format!("the status {FOUND:?}"), STEP, |window| {
        window.get_status() == FOUND
    })?;
    let reported = viewing.status();
    let found = probe::still_canvas(app, viewing, STEP)?;
    if found.fingerprint() == top.fingerprint() {
        return Err("Find did not bring paragraph 150 into view".into());
    }
    if viewing.window.get_whole_words_available() {
        return Err("Whole words is offered in the HTML viewer, which cannot do it".into());
    }
    input::replace_text(viewing.slint(), "absent-term");
    input::key(viewing.slint(), Key::Return);
    viewing.wait(app, &format!("the status {ABSENT:?}"), STEP, |window| {
        window.get_status() == ABSENT
    })?;
    context.note(format!("HTML Find: found {reported:?}"));
    app.screenshot_window(
        context,
        &viewing.software,
        "html-controls",
        "page.html after the zoom controls and Find",
    );
    Ok(())
}

/// Find in the hex view matches source bytes; Whole words is not offered there.
fn hex_find(app: &mut App, context: &mut Context, viewing: &Viewing) -> Result<(), String> {
    let modes: Vec<String> = viewing
        .window
        .get_modes()
        .iter()
        .map(|mode| mode.to_string())
        .collect();
    let index = usize::try_from(viewing.window.get_mode_index()).unwrap_or(usize::MAX);
    if modes.get(index).map(String::as_str) != Some("Binary") {
        // Bytes without NUL or control characters open as text with escapes: switch.
        context.note(format!(
            "bytes.bin opened in {:?} of {modes:?}; switching to Binary",
            modes.get(index)
        ));
        choose_mode(app, viewing, "Binary")?;
    }
    let base = probe::still_canvas(app, viewing, STEP)?;
    input::chord(viewing.slint(), &[Key::Control], "f");
    viewing.wait(app, "the find bar", STEP, |window| {
        window.get_search_visible()
    })?;
    if viewing.window.get_whole_words_available() {
        return Err("Whole words is offered in the binary view".into());
    }
    input::type_text(viewing.slint(), "needle");
    let status = search_settled(app, viewing, enter)?;
    if status.contains(NO_MATCH) {
        return Err(format!("the bytes \"needle\" were not found: {status:?}"));
    }
    highlight(app, viewing, &base)?;
    app.screenshot_window(
        context,
        &viewing.software,
        "hex-find",
        "Find \"needle\" in the hex view of bytes.bin",
    );
    input::replace_text(viewing.slint(), "absent");
    let status = search_settled(app, viewing, enter)?;
    if !status.contains(NO_MATCH) {
        return Err(format!("absent bytes were found: {status:?}"));
    }
    Ok(())
}

/// Open `file`, act on its viewer, close it.
fn with_viewer(
    app: &mut App,
    file: &str,
    act: impl FnOnce(&mut App, &Viewing) -> Result<(), String>,
) -> Result<(), String> {
    let viewing = open(app, file)?;
    let result = act(app, &viewing).map_err(|error| format!("{file}: {error}"));
    viewing.close(app)?;
    result
}

/// What the first start chose, for the restart to show.
struct Chosen {
    text_family: String,
    text_size: i32,
    /// The comparator's font family, when it offers another one.
    comparison_font: Option<String>,
}

/// The index of the Compact Markdown preset in the preset picker.
const COMPACT: i32 = 1;

/// Before the restart: Wrap and Aa on in a text file (whose family and size stay as the
/// font section left them), Lines off in a source file, the Compact preset, the image's
/// 1:1 zoom, the Continuous layout and another comparator font.
fn choose_for_restart(app: &mut App, context: &mut Context) -> Result<Chosen, String> {
    let mut font = (String::new(), 0);
    with_viewer(app, "words.txt", |app, viewing| {
        if !viewing.window.get_wrap_enabled() {
            input::chord(viewing.slint(), &[Key::Control], "w");
        }
        viewing.wait(app, "Ctrl+W to turn Wrap on", STEP, |window| {
            window.get_wrap_enabled()
        })?;
        input::chord(viewing.slint(), &[Key::Control], "f");
        viewing.wait(app, "the find bar", STEP, |window| {
            window.get_search_visible()
        })?;
        if !viewing.window.get_case_sensitive() {
            input::click(viewing.slint(), AccessibleRole::Checkbox, "Case sensitive")?;
        }
        viewing.wait(app, "Aa to turn on", STEP, |window| {
            window.get_case_sensitive()
        })?;
        font = (
            viewing.window.get_viewer_font_family().to_string(),
            viewing.window.get_viewer_font_size(),
        );
        Ok(())
    })?;
    with_viewer(app, "sample.rs", |app, viewing| {
        if viewing.window.get_line_numbers() {
            input::click(
                viewing.slint(),
                AccessibleRole::Checkbox,
                "Source line numbers",
            )?;
        }
        viewing.wait(app, "Lines off", STEP, |window| !window.get_line_numbers())
    })?;
    with_viewer(app, "doc.md", |app, viewing| {
        viewing.wait(app, "rendered Markdown", STEP, |window| {
            window.get_markdown_mode()
        })?;
        if viewing.window.get_markdown_preset() != COMPACT {
            input::click(
                viewing.slint(),
                AccessibleRole::Combobox,
                "Markdown presentation",
            )?;
            app.settle();
            input::click(viewing.slint(), AccessibleRole::Button, "Compact")?;
        }
        viewing.wait(app, "the Compact preset", STEP, |window| {
            window.get_markdown_preset() == COMPACT
        })
    })?;
    with_viewer(app, "tall.png", |app, viewing| {
        viewing.wait(app, "the image", STEP, |window| window.get_image_mode())?;
        input::click(
            viewing.slint(),
            AccessibleRole::Button,
            "Actual image pixels",
        )?;
        viewing.wait(app, "1:1", STEP, |window| {
            (window.get_zoom_percent() - 100.0).abs() < 0.5
        })
    })?;
    with_viewer(app, "pages.pdf", |app, viewing| {
        viewing.wait(app, "the pages", STEP, |window| {
            window.get_document_pages() == 6 && window.get_document_layout() >= 0
        })?;
        if viewing.window.get_document_layout() != 1 {
            input::click(
                viewing.slint(),
                AccessibleRole::Button,
                "Continuous document layout",
            )?;
        }
        viewing.wait(app, "the Continuous layout", STEP, |window| {
            window.get_document_layout() == 1
        })
    })?;
    let comparison_font = comparator_font(app, context, true)?;
    Ok(Chosen {
        text_family: font.0,
        text_size: font.1,
        comparison_font,
    })
}

/// The profile holds every choice made before the restart.
fn saved(store: &SettingsStore, chosen: &Chosen) -> Result<(), String> {
    let viewer = store.current().viewer;
    let mut problems = Vec::new();
    if !viewer.wrap {
        problems.push("Wrap on".to_owned());
    }
    if viewer.line_numbers {
        problems.push("Lines off".to_owned());
    }
    if !viewer.case_sensitive {
        problems.push("Aa (case sensitive) on".to_owned());
    }
    if viewer.text_font.family != chosen.text_family
        || i32::from(viewer.text_font.size) != chosen.text_size
    {
        problems.push(format!(
            "the text font {} {} px (saved {} {} px)",
            chosen.text_family, chosen.text_size, viewer.text_font.family, viewer.text_font.size
        ));
    }
    if viewer.markdown_preset != MarkdownPreset::Compact {
        problems.push(format!(
            "the Compact preset (saved {:?})",
            viewer.markdown_preset
        ));
    }
    if !matches!(
        viewer.image_zoom,
        ViewerZoomPreference::ActualPixels | ViewerZoomPreference::ScaleBasisPoints(10_000)
    ) {
        problems.push(format!(
            "the image's 1:1 zoom (saved {:?})",
            viewer.image_zoom
        ));
    }
    if viewer.document_layout != DocumentLayout::Continuous {
        problems.push(format!(
            "the Continuous PDF layout (saved {:?})",
            viewer.document_layout
        ));
    }
    // The comparator lists the generic family first, as "System monospace"
    // (`midnight-compare-ui` fonts.rs), and stores it as "monospace".
    if let Some(font) = &chosen.comparison_font
        && viewer.comparison_font
            != if font == "System monospace" {
                "monospace"
            } else {
                font.as_str()
            }
    {
        problems.push(format!(
            "the comparator font {font:?} (saved {:?})",
            viewer.comparison_font
        ));
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(format!("the profile lacks {}", problems.join(", ")))
    }
}

/// After the restart, each viewer opens with what was chosen before it.
fn restored(app: &mut App, context: &mut Context, chosen: &Chosen) -> Result<(), String> {
    let mut problems = Vec::new();
    let mut check = |what: &str, result: Result<(), String>| {
        if let Err(error) = result {
            problems.push(format!("{what}: {error}"));
        }
    };
    let (family, size) = (chosen.text_family.clone(), chosen.text_size);
    check(
        "words.txt",
        with_viewer(app, "words.txt", |app, viewing| {
            viewing.wait(
                app,
                &format!("Wrap on, {family} {size} px and Aa on"),
                STEP,
                |window| {
                    window.get_wrap_enabled()
                        && window.get_viewer_font_family() == family.as_str()
                        && window.get_viewer_font_size() == size
                        && window.get_case_sensitive()
                },
            )
        }),
    );
    check(
        "sample.rs",
        with_viewer(app, "sample.rs", |app, viewing| {
            viewing.wait(app, "Lines off", STEP, |window| {
                window.get_source_mode() && !window.get_line_numbers()
            })
        }),
    );
    check(
        "doc.md",
        with_viewer(app, "doc.md", |app, viewing| {
            viewing.wait(app, "the Compact preset", STEP, |window| {
                window.get_markdown_mode() && window.get_markdown_preset() == COMPACT
            })
        }),
    );
    check(
        "tall.png",
        with_viewer(app, "tall.png", |app, viewing| {
            viewing.wait(app, "the 1:1 zoom", STEP, |window| {
                window.get_image_mode() && (window.get_zoom_percent() - 100.0).abs() < 0.5
            })
        }),
    );
    check(
        "pages.pdf",
        with_viewer(app, "pages.pdf", |app, viewing| {
            let result = viewing.wait(app, "the Continuous layout", STEP, |window| {
                window.get_document_pages() == 6 && window.get_document_layout() == 1
            });
            app.screenshot_window(
                context,
                &viewing.software,
                "restart-pdf-continuous",
                "pages.pdf after a restart: the Continuous layout chosen before it",
            );
            result
        }),
    );
    match comparator_font(app, context, false) {
        Ok(shown) => {
            if let Some(font) = &chosen.comparison_font
                && shown.as_ref() != Some(font)
            {
                check(
                    "the comparator",
                    Err(format!("its font is {shown:?}, not {font:?}")),
                );
            }
        }
        Err(error) => check("the comparator", Err(error)),
    }
    context.note(format!(
        "After a restart: text font {} {} px, comparator font {:?}; {} problem(s)",
        chosen.text_family,
        chosen.text_size,
        chosen.comparison_font,
        problems.len()
    ));
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// Run the application and paint until the comparator meets `condition`.
fn wait_comparator(
    app: &mut App,
    window: &midnight_compare_ui::ComparatorWindow,
    what: &str,
    condition: impl Fn(&midnight_compare_ui::ComparatorWindow) -> bool,
) -> Result<(), String> {
    let deadline = Instant::now() + STEP * 4;
    loop {
        app.settle();
        support::paint_tool_windows(app);
        if condition(window) {
            return Ok(());
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out waiting for {what}; comparator status {:?}",
                window.get_status()
            ));
        }
        app.idle(Duration::from_millis(10));
    }
}

/// Compare words.txt and other.txt (marked with Insert, Ctrl+Shift+C) and read the
/// comparator's text font once its font list is ready. With `change`, choose the first
/// other family offered and return it (None when it offers no other); without, return
/// the family it shows. Escape closes the comparator and Ctrl+D clears the marks.
fn comparator_font(
    app: &mut App,
    context: &mut Context,
    change: bool,
) -> Result<Option<String>, String> {
    panel::focus_list(app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    for name in ["words.txt", "other.txt"] {
        panel::select_row(app, PaneId::Left, name)?;
        input::key(app.slint(), Key::Insert);
        app.settle();
    }
    let before = support::plugins(app)?.compared.borrow().len();
    input::chord(app.slint(), &[Key::Control, Key::Shift], "c");
    app.settle();
    let compared = support::plugins(app)?
        .compared
        .borrow()
        .get(before)
        .cloned()
        .ok_or_else(|| {
            format!(
                "Ctrl+Shift+C opened no comparator; notice {:?}",
                app.notice()
            )
        })?;
    let window = compared
        .window
        .upgrade()
        .ok_or("the comparator closed at once")?;
    window
        .window()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    let result = (|| {
        wait_comparator(app, &window, "the comparator's font list", |window| {
            window.get_fonts_ready()
        })?;
        let families: Vec<String> = window
            .get_font_families()
            .iter()
            .map(|family| family.to_string())
            .collect();
        let shown = usize::try_from(window.get_font_index()).unwrap_or(usize::MAX);
        if !change {
            return Ok(families.get(shown).cloned());
        }
        let Some((index, other)) = families
            .iter()
            .enumerate()
            .find(|(index, _)| *index != shown)
        else {
            context.note(format!(
                "The comparator offers one font family ({families:?}): its font is not changed"
            ));
            return Ok(None);
        };
        input::click(
            window.window(),
            AccessibleRole::Combobox,
            "Text comparison font",
        )?;
        app.settle();
        input::click(window.window(), AccessibleRole::Button, other)?;
        wait_comparator(app, &window, "the chosen font", |window| {
            window.get_font_index() == index as i32
        })?;
        Ok(Some(other.clone()))
    })();
    input::key(window.window(), Key::Escape);
    let deadline = Instant::now() + STEP;
    while support::software_window(app, window.window()).is_some() {
        if Instant::now() >= deadline {
            return Err("Escape did not close the comparator".into());
        }
        app.idle(Duration::from_millis(10));
    }
    app.slint()
        .dispatch_event(WindowEvent::WindowActiveChanged(true));
    panel::focus_list(app, PaneId::Left)?;
    input::chord(app.slint(), &[Key::Control], "d");
    app.settle();
    result
}
