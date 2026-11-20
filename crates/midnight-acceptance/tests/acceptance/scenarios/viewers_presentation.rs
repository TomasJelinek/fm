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
//!   page before the first or after the last.
//! - Images: 1:1, Ctrl+0 (Fit) and dragging the picture at actual pixels.
//! - HTML: the zoom buttons, Ctrl+Plus, Ctrl+Minus and Ctrl+0, and the Online and
//!   JavaScript boxes (unchecked; JavaScript needs Online).
use crate::{
    Context,
    app::{App, Config},
    encode, input, inspect, panel, tools_support as tools, viewers_stage,
    viewers_support::{self as support, Canvas, Failures, Viewing, generated, probe},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{LogicalPosition, Model, platform::Key};
use std::time::Duration;

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

fn run(context: &mut Context) -> Result<(), String> {
    let root = prepare(context)?;
    let mut config = Config::new(&root, &context.work);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let failures = Failures::default();
    let sections: [(
        &str,
        &str,
        fn(&mut App, &mut Context, &Viewing) -> Result<(), String>,
    ); 8] = [
        ("Find bar", "words.txt", find_bar),
        ("Source and Text modes, Lines", "sample.rs", source_modes),
        ("Markdown and Text modes, presets", "doc.md", markdown_modes),
        ("font size and family", "words.txt", fonts),
        ("PDF pages", "pages.pdf", pdf_pages),
        ("image 1:1, Fit and dragging", "tall.png", image_controls),
        ("HTML zoom, policy and Find", "page.html", html_controls),
        ("Find in the hex view", "bytes.bin", hex_find),
    ];
    for (what, file, section) in sections {
        let result = open(&mut app, file).and_then(|viewing| {
            let result = section(&mut app, context, &viewing);
            // A section may close its viewer itself (Escape in the Find bar).
            if viewing.slint().is_visible() {
                viewing.close(&mut app)?;
            }
            result
        });
        failures.check(what, result);
    }
    failures.check(
        "preferences kept for the next file",
        preferences_kept(&mut app, context),
    );
    app.shutdown()?;
    failures.finish(context)
}

fn open(app: &mut App, file: &str) -> Result<Viewing, String> {
    panel::select_row(app, PaneId::Left, file)?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    viewing
        .result
        .clone()
        .map_err(|error| format!("{file}: {error}"))?;
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
/// colour where the unsearched page showed its background.
fn highlight(app: &mut App, viewing: &Viewing, base: &Canvas) -> Result<(usize, usize), String> {
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

fn search_settled(app: &mut App, viewing: &Viewing) -> Result<String, String> {
    app.idle(Duration::from_millis(150));
    probe::still_canvas(app, viewing, STEP)?;
    Ok(viewing.status())
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
    input::key(viewing.slint(), Key::Return);
    search_settled(app, viewing)?;
    let mut spots = vec![highlight(app, viewing, &base)?];
    // F3, the Next match button, F3: the second, third and fourth match.
    input::key(viewing.slint(), Key::F3);
    search_settled(app, viewing)?;
    spots.push(highlight(app, viewing, &base)?);
    input::click_button(viewing.slint(), "Next match")?;
    search_settled(app, viewing)?;
    spots.push(highlight(app, viewing, &base)?);
    input::key(viewing.slint(), Key::F3);
    search_settled(app, viewing)?;
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
    input::key(viewing.slint(), Key::F3);
    let status = search_settled(app, viewing)?;
    if !status.contains(NO_MATCH) {
        return Err(format!("F3 after the last match: status {status:?}"));
    }
    input::chord(viewing.slint(), &[Key::Shift], Key::F3);
    search_settled(app, viewing)?;
    let back = highlight(app, viewing, &base)?;
    input::click_button(viewing.slint(), "Previous match")?;
    search_settled(app, viewing)?;
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
    input::key(viewing.slint(), Key::Return);
    search_settled(app, viewing)?;
    // Collect every whole-word match from wherever the search stands: forwards to the
    // end, then backwards to the start.
    let mut whole = std::collections::BTreeSet::new();
    for backwards in [false, true] {
        for _ in 0..4 {
            if !viewing.status().contains(NO_MATCH) {
                whole.insert(highlight(app, viewing, &base)?.1);
            }
            if backwards {
                input::chord(viewing.slint(), &[Key::Shift], Key::F3);
            } else {
                input::key(viewing.slint(), Key::F3);
            }
            if search_settled(app, viewing)?.contains(NO_MATCH) {
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
    // Aa: "cat" in lower case appears nowhere.
    input::click(viewing.slint(), AccessibleRole::Checkbox, "Case sensitive")?;
    input::chord(viewing.slint(), &[Key::Control], "f");
    app.settle();
    input::replace_text(viewing.slint(), "cat");
    input::key(viewing.slint(), Key::Return);
    let status = search_settled(app, viewing)?;
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
    input::key(viewing.slint(), Key::Return);
    search_settled(app, viewing)?;
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
    Ok(probe::glyph_colours(&canvas, probe::dominant(&canvas), 24).len())
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
    viewing.wait(app, "Chromium's find result", STEP, |window| {
        let status = window.get_status();
        status.contains("Match 1 of 1")
            || status.contains("1 matches")
            || status.contains("1 match")
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
    viewing.wait(app, "no match for an absent term", STEP, |window| {
        window.get_status().contains("No matches")
    })?;
    context.note(format!("HTML Find: found {reported:?}"));
    if reported.contains("1 matches") {
        let shot = app.screenshot_window(
            context,
            &viewing.software,
            "html-find-grammar",
            "HTML Find reports one match",
        );
        support::flag_shot(
            context,
            &shot,
            "status text",
            format!("one match is reported as {reported:?}"),
        );
    }
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
    input::key(viewing.slint(), Key::Return);
    let status = search_settled(app, viewing)?;
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
    input::key(viewing.slint(), Key::Return);
    let status = search_settled(app, viewing)?;
    if !status.contains(NO_MATCH) {
        return Err(format!("absent bytes were found: {status:?}"));
    }
    Ok(())
}
