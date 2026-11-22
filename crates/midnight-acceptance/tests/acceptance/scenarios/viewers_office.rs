//! Office documents through the installed viewers (V5): the Office preview turns a Word
//! document, a PowerPoint presentation or an Excel workbook into a page that the HTML
//! viewer (Chromium) shows. The packages are written here by `encode::ooxml`, with the
//! ZIP extra fields Office itself writes: Microsoft's growth-hint padding (0xA220) on the
//! first parts and extended timestamps, which the preview opens since 500224d6. Each
//! must render, and after Find the status must read "Office preview · Offline ·
//! JavaScript off · Match 1 of 1". A package whose parts are
//! encrypted is refused with "This presentation is password-protected, so the preview
//! cannot open it.", an OLE compound file (an encrypted or legacy Office file) with the
//! older-format sentence, and with the HTML viewer disabled in the settings F3 says
//! "Office preview requires an installed and enabled Chromium HTML viewer.". Where the
//! HTML viewer cannot run (its package or its system libraries are missing, or the
//! machine refuses its browser: AppArmor or no cgroup-v2 delegation, as in
//! `viewers_files`), the checks that need it report SKIP with that reason; the refusals
//! do not need it.
use crate::{
    Context, Scenario,
    app::{App, Config},
    encode::{self, Lock, Office, OfficeStorage},
    files_support as files, input, options_support, panel, viewers_stage,
    viewers_support::{self as support, Failures, Viewing},
};
use midnight_core::PaneId;
use slint::platform::Key;
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(8 * 60);

/// Registered here, not in the registry's list: this module owns its entries.
pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "viewers_office",
        area: "viewers",
        summary: "Office documents through the Office preview and the HTML viewer: a Word document, a PowerPoint presentation and an Excel workbook with Office's ZIP padding (0xA220) and timestamps render and after Find the status reads Office preview · Offline · JavaScript off · Match 1 of 1; a package with encrypted parts says it is password-protected, an OLE compound file names the older format, and with the HTML viewer disabled F3 says the preview requires it.",
        run: office,
        timeout: TIMEOUT,
    }]
}
const OPEN_LIMIT: Duration = Duration::from_secs(60);
const STEP: Duration = Duration::from_secs(20);
/// The documents' text; Find looks for its distinctive part.
const TEXT: &str = "Midnight acceptance Office text";
const NEEDLE: &str = "acceptance Office";
/// The HTML viewer's plugin identity, as Options › Plugins disables it.
const HTML_VIEWER: &str = "midnight.viewer.html";

fn office(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

/// The documents, in `folder`: (file, refusal it must give, if any).
fn write_documents(folder: &Path) -> Result<Vec<(&'static str, Option<&'static str>)>, String> {
    std::fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    let padded = OfficeStorage {
        padded: true,
        lock: Lock::None,
    };
    let locked = OfficeStorage {
        padded: false,
        lock: Lock::ZipCrypto("office-password".into()),
    };
    let files: [(&str, Vec<u8>, Option<&str>); 5] = [
        (
            "padded.docx",
            encode::ooxml(Office::Docx, TEXT, &padded),
            None,
        ),
        (
            "padded.pptx",
            encode::ooxml(Office::Pptx, TEXT, &padded),
            None,
        ),
        (
            "padded.xlsx",
            encode::ooxml(Office::Xlsx, TEXT, &padded),
            None,
        ),
        (
            "locked.pptx",
            encode::ooxml(Office::Pptx, TEXT, &locked),
            Some("This presentation is password-protected, so the preview cannot open it."),
        ),
        (
            "protected.docx",
            encode::compound_file_start(),
            Some(
                "This document is password-protected or saved in the older binary Office \
                 format (.doc, .xls or .ppt), which the preview cannot open.",
            ),
        ),
    ];
    let mut cases = Vec::new();
    for (name, bytes, refusal) in files {
        std::fs::write(folder.join(name), bytes)
            .map_err(|error| format!("{}: {error}", folder.join(name).display()))?;
        cases.push((name, refusal));
    }
    Ok(cases)
}

fn run(context: &mut Context) -> Result<(), String> {
    let folder = context.work.join("office");
    let cases = write_documents(&folder)?;
    let failures = Failures::default();
    let html_missing = viewers_stage::html_viewer_missing();
    let mut config = Config::new(&folder, &context.work);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    panel::focus_list(&mut app, PaneId::Left)?;
    for (name, refusal) in &cases {
        match (refusal, &html_missing) {
            (Some(refusal), _) => failures.check(
                &format!("{name}: refused"),
                refused(&mut app, context, name, refusal),
            ),
            (None, Some(reason)) => failures.skip(&format!("{name}: renders, Find"), reason),
            (None, None) => {
                let check = format!("{name}: renders, Find");
                match rendered(&mut app, context, name) {
                    Ok(Some(reason)) => failures.skip(&check, reason),
                    result => failures.check(&check, result.map(|_| ())),
                }
            }
        }
        files::dismiss_all(&mut app);
    }
    // Where the browser's system libraries are missing (Linux), the viewer names them
    // and how to install them (b8537464).
    let missing = viewers_stage::html_viewer_missing_libraries();
    if !missing.is_empty() {
        failures.check(
            "the HTML viewer names its missing libraries",
            libraries_named(&mut app, context, &missing),
        );
        files::dismiss_all(&mut app);
    }
    app.shutdown()?;
    // Options › Plugins with the HTML viewer disabled, as the settings keep it.
    let store = options_support::SettingsStore::seeded(|preferences| {
        preferences
            .disabled_viewer_plugins
            .push(HTML_VIEWER.to_owned());
    });
    let mut config = Config::new(&folder, &context.work);
    config.plugins = Some(context.work.join("plugins-disabled"));
    config.settings = Some(store);
    let mut app = App::start(config);
    panel::focus_list(&mut app, PaneId::Left)?;
    failures.check(
        "without the HTML viewer",
        refused(
            &mut app,
            context,
            "padded.docx",
            "Office preview requires an installed and enabled Chromium HTML viewer.",
        ),
    );
    files::dismiss_all(&mut app);
    app.shutdown()?;
    failures.finish(context)
}

/// F3 on a document on a machine without the browser's libraries: the viewer reports
/// "The HTML viewer needs system libraries that are not installed: <names>. Install them
/// with[ scripts/setup-linux.sh --install in the source checkout, or with]: sudo apt
/// install <packages>", naming only libraries that are missing (`missing`, as `ldd`
/// finds them) and a command.
fn libraries_named(app: &mut App, context: &mut Context, missing: &[String]) -> Result<(), String> {
    const HEAD: &str = "The HTML viewer needs system libraries that are not installed: ";
    panel::select_row(app, PaneId::Left, "padded.docx")?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let reported = match viewing.result.clone() {
        Err(error) => Ok(error),
        Ok(()) => viewing
            .wait(app, "the viewer's error", STEP, |window| {
                window.get_status().contains(HEAD)
            })
            .map(|()| viewing.status()),
    };
    app.screenshot_window(
        context,
        &viewing.software,
        "missing-libraries",
        &format!(
            "F3 on padded.docx without the browser's libraries: {}",
            viewing.status()
        ),
    );
    viewing.close(app)?;
    let text = reported?;
    let start = text
        .find(HEAD)
        .ok_or_else(|| format!("the viewer reports {text:?}, not the missing libraries"))?;
    let rest = &text[start + HEAD.len()..];
    let (named, command) = rest
        .split_once(". ")
        .ok_or_else(|| format!("no sentence after the libraries in {text:?}"))?;
    let named: Vec<&str> = named
        .split(", ")
        .filter(|name| !name.starts_with("and ") || !name.ends_with(" more"))
        .collect();
    let unknown: Vec<&&str> = named
        .iter()
        .filter(|name| !missing.iter().any(|known| known.as_str() == **name))
        .collect();
    context.note(format!(
        "Missing libraries named: {text:?}; ldd finds {missing:?}"
    ));
    if named.is_empty() || !unknown.is_empty() {
        return Err(format!(
            "the viewer names {named:?}, of which {unknown:?} are not missing (ldd finds \
             {missing:?})"
        ));
    }
    if !(command.starts_with("Install them with") && command.contains(": sudo apt install ")) {
        return Err(format!("the install command is missing: {command:?}"));
    }
    Ok(())
}

/// F3 on `name`: the preview opens in the HTML viewer and Find reports one match.
/// `Ok(Some(reason))` when this machine refuses the browser itself
/// ([`support::browser_refused`]): the case is skipped with that reason.
fn rendered(
    app: &mut App,
    context: &mut Context,
    name: &str,
) -> Result<Option<&'static str>, String> {
    panel::select_row(app, PaneId::Left, name)?;
    let viewing = support::view(app, false, OPEN_LIMIT)?;
    let mut refused = viewing
        .result
        .as_ref()
        .err()
        .and_then(|error| support::browser_refused(error));
    let checked = match refused {
        Some(_) => Ok(()),
        None => (|| {
            viewing.result.clone().map_err(|error| {
                format!(
                    "the preview reported {error:?}; status {:?}",
                    viewing.status()
                )
            })?;
            find(app, &viewing)
        })(),
    };
    if checked.is_err() {
        refused = support::browser_refused(&viewing.status());
    }
    app.screenshot_window(
        context,
        &viewing.software,
        &name.replace('.', "-"),
        &format!("F3 on {name}: {}", viewing.status()),
    );
    let closed = viewing.close(app);
    match refused {
        Some(reason) => {
            context.note(format!("F3 on {name}: {reason}"));
            closed.map(|()| Some(reason))
        }
        None => checked.and(closed).map(|()| None),
    }
}

/// The status after Find: the preview and its policy, then Chromium's count. The reply
/// composes it at once for every format (cee7ecd8; before, a Word document's reply after
/// the page's last frame showed the bare count).
const FOUND: &str = "Office preview · Offline · JavaScript off · Match 1 of 1";

/// Ctrl+F, the needle, Enter: Chromium's search finds the one match and the status line
/// reads exactly [`FOUND`].
fn find(app: &mut App, viewing: &Viewing) -> Result<(), String> {
    input::chord(viewing.slint(), &[Key::Control], "f");
    viewing.wait(app, "the find bar", STEP, |window| {
        window.get_search_visible()
    })?;
    input::type_text(viewing.slint(), NEEDLE);
    input::key(viewing.slint(), Key::Return);
    let found = viewing.wait(app, "Find's result", STEP, |window| {
        let status = window.get_status();
        status.contains("Match 1 of 1") || status.contains("No matches")
    });
    let status = viewing.status();
    found?;
    if status.trim() == FOUND {
        Ok(())
    } else {
        Err(format!(
            "Find for {NEEDLE:?} shows the status {status:?}, not {FOUND:?}"
        ))
    }
}

/// F3 on `name` is refused with exactly `sentence`: as the viewer's report or status
/// line, or (when no viewer opens) as the notice.
fn refused(app: &mut App, context: &mut Context, name: &str, sentence: &str) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, name)?;
    let before = support::viewers_opened(app);
    input::key(app.slint(), Key::F3);
    let deadline = Instant::now() + OPEN_LIMIT;
    loop {
        app.settle();
        if support::viewers_opened(app) > before {
            break;
        }
        if let Some((title, body)) = app.notice() {
            app.screenshot(
                context,
                &format!("{}-refused", name.replace('.', "-")),
                &body,
            );
            return if body == sentence {
                Ok(())
            } else {
                Err(format!("F3 on {name} gave the notice {title:?}: {body:?}"))
            };
        }
        if Instant::now() >= deadline {
            return Err(format!("F3 on {name} opened nothing and said nothing"));
        }
        app.idle(Duration::from_millis(10));
    }
    let viewing = support::next_viewer(app, before, OPEN_LIMIT)?;
    let shown = viewing.wait(app, "the refusal", STEP, |window| {
        window.get_status().contains(sentence)
    });
    let (result, status) = (viewing.result.clone(), viewing.status());
    app.screenshot_window(
        context,
        &viewing.software,
        &format!("{}-refused", name.replace('.', "-")),
        &format!("F3 on {name}: {status}"),
    );
    viewing.close(app)?;
    let reported = result
        .as_ref()
        .err()
        .is_some_and(|error| error.contains(sentence));
    if shown.is_ok() || reported {
        Ok(())
    } else {
        Err(format!(
            "F3 on {name}: the viewer reports {result:?} with the status {status:?}, not \
             {sentence:?}"
        ))
    }
}
