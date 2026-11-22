//! Archive passwords through the installed archive plugin (7632ce3f): members encrypted
//! with ZipCrypto and WinZip AES-128, AES-192 and AES-256, written here by
//! `encode::zip_entries`. Their listing marks only the encrypted rows; the details name
//! the method or why a member cannot be read. F3 on an encrypted member asks for the
//! archive's password first: an empty field and a wrong password are refused in the
//! dialog, Escape and Cancel close it without a notice and open nothing, and the right
//! (non-ASCII) password, by keyboard or pointer, opens the viewer. The password is asked
//! once per archive and kept until the archive is closed: the other members then open without
//! asking, and leaving and entering the archive again asks again. An encrypted archive
//! inside an encrypted archive asks for the outer password to open it and for its own to
//! view its members. A 7-Zip archive with encrypted names asks before it lists (made with
//! the host's `7z`, otherwise skipped). The password never reaches the settings, the
//! clipboard, the harness's event trace or the child's console.
use super::viewers_archives::{back_to, open_row, recover, shown, wait_listing};
use crate::{
    Context, Scenario,
    app::{App, Config},
    encode::{self, AesStrength, Entry, Method},
    files_support as files, input, inspect, options_support, panel, viewers_stage,
    viewers_support::{self as support, Dismiss, Failures, PasswordDialog, Submit, Viewing},
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::{Model, platform::Key};
use std::{
    path::Path,
    time::{Duration, Instant},
};

pub const TIMEOUT: Duration = Duration::from_secs(10 * 60);

/// Registered here, not in the registry's list: this module owns its entries.
pub fn scenarios() -> Vec<Scenario> {
    vec![Scenario {
        name: "viewers_passwords",
        area: "viewers",
        summary: "Archive passwords through the installed archive plugin: ZipCrypto and WinZip AES-128/192/256 members marked Encrypted in the listing and the details; F3 asks for the archive's password (empty and wrong refused in the dialog, Escape and Cancel close it without a notice and open nothing); the right non-ASCII password by Enter or OK opens the viewer and the other members without asking until the archive is closed; an AES archive inside a ZipCrypto archive asks for each password; unreadable members (Implode, an unsafe name, a link, strong encryption) say why; a 7-Zip archive with encrypted names asks before listing (host 7z); the password reaches no settings, clipboard, event trace or console.",
        run: passwords,
        timeout: TIMEOUT,
    }]
}
const LIMIT: Duration = Duration::from_secs(30);
/// Non-ASCII on purpose: the dialog passes the typed text on unchanged.
const PASSWORD: &str = "Kyrandia-héslo-🔑";
/// The inner archive of `nested.zip` has a password of its own.
const INNER_PASSWORD: &str = "vnitřní-ключ";
const WRONG_PASSWORD: &str = "Wrong password — try again.";
const EMPTY_PASSWORD: &str = "Type the password first.";

fn passwords(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

/// The fixture archives, in `folder`.
fn write_archives(folder: &Path) -> Result<(), String> {
    let write = |name: &str, bytes: Vec<u8>| {
        std::fs::write(folder.join(name), bytes)
            .map_err(|error| format!("{}: {error}", folder.join(name).display()))
    };
    std::fs::create_dir_all(folder).map_err(|error| format!("{}: {error}", folder.display()))?;
    write(
        "zipcrypto.zip",
        encode::zip_entries(&[
            Entry::file("a.txt", "Secret A.\n").zip_crypto(PASSWORD),
            Entry::file("b.txt", "Secret B, deflated.\n")
                .method(Method::Deflated)
                .zip_crypto(PASSWORD),
            Entry::file("plain.txt", "Plain member.\n"),
        ]),
    )?;
    write(
        "aes.zip",
        encode::zip_entries(&[
            Entry::directory("notes"),
            Entry::file("notes/plain.txt", "Plain beside AES.\n"),
            Entry::file("secret.txt", "AES-256 secret.\n").aes(AesStrength::Aes256, PASSWORD),
            Entry::file("deflated.txt", "AES-256, deflated.\n".repeat(40))
                .method(Method::Deflated)
                .aes(AesStrength::Aes256, PASSWORD),
        ]),
    )?;
    write(
        "aes128.zip",
        encode::zip_entries(&[
            Entry::file("a.txt", "AES-128 secret.\n").aes(AesStrength::Aes128, PASSWORD)
        ]),
    )?;
    write(
        "aes192.zip",
        encode::zip_entries(&[
            Entry::file("a.txt", "AES-192 secret.\n").aes(AesStrength::Aes192, PASSWORD)
        ]),
    )?;
    let inner =
        encode::zip_entries(&[Entry::file("inner.txt", "Innermost secret.\n")
            .aes(AesStrength::Aes256, INNER_PASSWORD)]);
    write(
        "nested.zip",
        encode::zip_entries(&[
            Entry::file("inner.zip", inner).zip_crypto(PASSWORD),
            Entry::file("outer.txt", "Outer secret.\n").zip_crypto(PASSWORD),
        ]),
    )?;
    // Members no reader opens: Implode (method 6), an unsafe name, a link and PKWARE
    // strong encryption (bits 0 and 6).
    let mut implode = Entry::file("implode.txt", "Imploded, so to speak.\n");
    implode.method_number = Some(6);
    let mut link = Entry::file("link", "plain.txt");
    link.mode = Some(0o120_777);
    let mut strong = Entry::file("strong.txt", "Strongly encrypted.\n");
    strong.flags = 0x41;
    write(
        "odd.zip",
        encode::zip_entries(&[
            implode,
            Entry::file("../x", "Outside.\n"),
            link,
            strong,
            Entry::file("plain.txt", "Plain member.\n"),
        ]),
    )?;
    Ok(())
}

fn run(context: &mut Context) -> Result<(), String> {
    let home = context.work.join("archives");
    write_archives(&home)?;
    let seven = seven_zip_names(&home, &context.work.join("scratch"));
    let store = options_support::SettingsStore::new();
    let mut config = Config::new(&home, &context.work);
    config.plugins = Some(context.work.join("plugins"));
    config.settings = Some(store.clone());
    config.clipboard = true;
    let mut app = App::start(config);
    panel::focus_list(&mut app, PaneId::Left)?;
    let failures = Failures::default();

    failures.check(
        "zipcrypto.zip: listing and details",
        listing(
            &mut app,
            &home,
            "zipcrypto.zip",
            &[
                ("a.txt", Some("ZipCrypto")),
                ("b.txt", Some("ZipCrypto")),
                ("plain.txt", None),
            ],
        ),
    );
    recover(&mut app, &home)?;
    failures.check(
        "zipcrypto.zip: F3 asks; empty, wrong, Escape",
        refusals(&mut app, context, &home),
    );
    recover(&mut app, &home)?;
    failures.check(
        "zipcrypto.zip: the password is asked once per archive",
        asked_once(&mut app, context, &home),
    );
    recover(&mut app, &home)?;
    failures.check(
        "aes.zip: listing and details",
        listing(
            &mut app,
            &home,
            "aes.zip",
            &[
                ("deflated.txt", Some("AES-256")),
                ("notes", None),
                ("secret.txt", Some("AES-256")),
            ],
        ),
    );
    recover(&mut app, &home)?;
    failures.check(
        "aes.zip: the plain member opens without asking",
        plain_member(&mut app, &home),
    );
    recover(&mut app, &home)?;
    failures.check(
        "aes.zip: AES-256 members after one password",
        aes_members(&mut app, &home),
    );
    recover(&mut app, &home)?;
    for (archive, method, text) in [
        ("aes128.zip", "AES-128", "AES-128 secret."),
        ("aes192.zip", "AES-192", "AES-192 secret."),
    ] {
        failures.check(
            &format!("{archive}: {method}"),
            one_member(&mut app, &home, archive, method, text),
        );
        recover(&mut app, &home)?;
    }
    failures.check(
        "nested.zip: the outer password, then the inner archive's own",
        nested(&mut app, context, &home),
    );
    recover(&mut app, &home)?;
    failures.check("odd.zip: unreadable members", odd(&mut app, context, &home));
    recover(&mut app, &home)?;
    match seven {
        Ok(()) => failures.check(
            "names.7z: encrypted names asked before listing",
            encrypted_names(&mut app, &home),
        ),
        Err(reason) => failures.skip("names.7z: encrypted names asked before listing", &reason),
    }
    recover(&mut app, &home)?;
    files::dismiss_all(&mut app);
    failures.check("no leaks", no_leaks(&app, context, &store));
    app.shutdown()?;
    // The console takes what the application and its workers printed until now.
    failures.check("no leaks: the console", console_clean());
    failures.finish(context)
}

/// Enter `archive`: its rows are exactly `rows`, only the encrypted ones say so (their
/// accessible label ends " · Encrypted" and the row model marks them), and the details
/// strip names each encrypted member's method.
fn listing(
    app: &mut App,
    home: &Path,
    archive: &str,
    rows: &[(&str, Option<&str>)],
) -> Result<(), String> {
    open_row(app, archive)?;
    let names: Vec<&str> = rows.iter().map(|(name, _)| *name).collect();
    wait_listing(app, archive, &names)?;
    let labels = inspect::labels(app.slint(), AccessibleRole::ListItem);
    let mut problems = Vec::new();
    for (name, method) in rows {
        let encrypted = method.is_some();
        let label = labels
            .iter()
            .find(|label| panel::is_row_label(label, name))
            .ok_or_else(|| format!("no row labelled {name:?} among {labels:?}"))?;
        if label.contains(" · Encrypted") != encrypted {
            problems.push(format!("{name}: accessible label {label:?}"));
        }
        let model = app
            .rows(PaneId::Left)
            .into_iter()
            .find(|row| row.name == *name)
            .map(|row| row.encrypted);
        if model != Some(encrypted) {
            problems.push(format!(
                "{name}: the row model's encrypted flag is {model:?}"
            ));
        }
        panel::select_row(app, PaneId::Left, name)?;
        let details = details(app);
        let shown = details
            .iter()
            .find(|(label, _, _)| label == "Encrypted")
            .map(|(_, value, explanation)| (value.clone(), explanation.clone()));
        match (method, shown) {
            (Some(method), Some((value, explanation))) => {
                if value != *method {
                    problems.push(format!("{name}: details say Encrypted {value:?}"));
                }
                if !explanation.contains("its password is asked for once") {
                    problems.push(format!(
                        "{name}: the Encrypted explanation is {explanation:?}"
                    ));
                }
            }
            (Some(_), None) => problems.push(format!("{name}: no Encrypted detail in {details:?}")),
            (None, Some((value, _))) => problems.push(format!(
                "{name}: a plain row has an Encrypted detail {value:?}"
            )),
            (None, None) => {}
        }
    }
    back_to(app, home, 1, archive)?;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// The focused row's details: (label, value, explanation).
fn details(app: &App) -> Vec<(String, String, String)> {
    app.window
        .get_left_details()
        .iter()
        .map(|row| {
            (
                row.label.to_string(),
                row.value.to_string(),
                row.explanation.to_string(),
            )
        })
        .collect()
}

/// The dialog's sentence for viewing `member` of `archive`.
fn view_sentence(archive: &str, member: &str) -> String {
    format!("{archive} is password-protected. Enter its password to view {member}.")
}

fn expect_sentence(dialog: &PasswordDialog, expected: &str) -> Result<(), String> {
    if dialog.sentence() == expected {
        Ok(())
    } else {
        Err(format!(
            "the dialog says {:?}, not {expected:?}",
            dialog.sentence()
        ))
    }
}

/// F3 on `member` with the cursor there, and the dialog it opens; no viewer meanwhile.
fn ask(app: &mut App, member: &str, basic: bool) -> Result<PasswordDialog, String> {
    panel::select_row(app, PaneId::Left, member)?;
    let before = support::viewers_opened(app);
    if basic {
        input::chord(app.slint(), &[Key::Alt], Key::F3);
    } else {
        input::key(app.slint(), Key::F3);
    }
    let dialog = support::wait_password_dialog(app, LIMIT)?;
    if support::viewers_opened(app) != before {
        return Err(format!(
            "F3 on {member} opened a viewer before the password"
        ));
    }
    Ok(dialog)
}

/// F3 on an encrypted member: the dialog's sentence names the archive and the member; an
/// empty field says what is missing; a wrong password says so and keeps the dialog; and
/// Escape closes it without a notice, opens nothing and gives the keyboard back to the
/// member's row.
fn refusals(app: &mut App, context: &mut Context, home: &Path) -> Result<(), String> {
    open_row(app, "zipcrypto.zip")?;
    wait_listing(app, "zipcrypto.zip", &["a.txt", "b.txt", "plain.txt"])?;
    let dialog = ask(app, "a.txt", false)?;
    app.screenshot(context, "password-dialog", &dialog.sentence());
    expect_sentence(&dialog, &view_sentence("zipcrypto.zip", "a.txt"))?;
    let before = support::viewers_opened(app);
    let empty = support::submit_password(app, "", Submit::Enter, LIMIT)?
        .ok_or("an empty password closed the dialog")?;
    if empty.error != EMPTY_PASSWORD {
        return Err(format!("an empty field says {:?}", empty.error));
    }
    let wrong = support::submit_password(app, "wrong", Submit::Enter, LIMIT)?
        .ok_or("a wrong password closed the dialog")?;
    app.screenshot(context, "password-wrong", &wrong.error);
    if wrong.error != WRONG_PASSWORD || wrong.attempt != 1 {
        return Err(format!(
            "a wrong password says {:?} (attempt {})",
            wrong.error, wrong.attempt
        ));
    }
    support::dismiss_password(app, Dismiss::Escape)?;
    dismissed_quietly(app, "a.txt", before)?;
    context.note(format!(
        "F3 on an encrypted member: {:?}; empty field: {:?}; wrong password: {:?} (attempt \
         {}); Escape: closed without a notice",
        dialog.sentence(),
        empty.error,
        wrong.error,
        wrong.attempt
    ));
    back_to(app, home, 1, "zipcrypto.zip")
}

/// How long a dismissal is watched for a notice.
const QUIET: Duration = Duration::from_millis(500);

/// After Escape or Cancel in the password dialog: for `QUIET` no notice appears, no viewer
/// opened (`viewers` were open before), and the keyboard is back in the left list on `member`.
fn dismissed_quietly(app: &mut App, member: &str, viewers: usize) -> Result<(), String> {
    let deadline = Instant::now() + QUIET;
    while Instant::now() < deadline {
        app.idle(Duration::from_millis(20));
        if let Some(notice) = app.notice() {
            files::dismiss_all(app);
            return Err(format!("a notice followed the dismissal: {notice:?}"));
        }
    }
    if support::viewers_opened(app) != viewers {
        return Err("a refused or cancelled password opened a viewer".into());
    }
    let focused = app.window.get_focused_pane();
    let cursor = files::cursor_name(app, PaneId::Left);
    if focused != 0 || cursor.as_deref() != Some(member) {
        return Err(format!(
            "after the dismissal the keyboard is in pane {focused}, its cursor on {cursor:?}, not on {member}"
        ));
    }
    Ok(())
}

/// The member's text in the viewer, which then closes.
fn shows(app: &mut App, viewing: Viewing, text: &str) -> Result<(), String> {
    let shown = viewing.result.clone().and_then(|()| {
        viewing.wait(app, text, LIMIT, |window| {
            window.get_accessible_text().contains(text)
        })
    });
    viewing.close(app)?;
    shown
}

/// The right password by pointer opens the viewer; b.txt and Alt+F3 on a.txt then open
/// without asking; after leaving and entering the archive again F3 asks again, Cancel
/// opens nothing, and the right password by Enter opens the viewer.
fn asked_once(app: &mut App, context: &mut Context, home: &Path) -> Result<(), String> {
    open_row(app, "zipcrypto.zip")?;
    wait_listing(app, "zipcrypto.zip", &["a.txt", "b.txt", "plain.txt"])?;
    panel::select_row(app, PaneId::Left, "a.txt")?;
    let (dialog, viewing) = support::view_unlocking(app, false, PASSWORD, Submit::Pointer, LIMIT)?;
    expect_sentence(&dialog, &view_sentence("zipcrypto.zip", "a.txt"))?;
    app.screenshot_window(
        context,
        &viewing.software,
        "unlocked-member",
        "F3 on a.txt after its archive's password",
    );
    shows(app, viewing, "Secret A.")?;
    // Remembered until the archive closes: no dialog for the other member or Alt+F3.
    for (member, basic, text) in [
        ("b.txt", false, "Secret B, deflated."),
        ("a.txt", true, "Secret A."),
    ] {
        panel::select_row(app, PaneId::Left, member)?;
        let viewing = support::view(app, basic, LIMIT).map_err(|error| {
            format!(
                "{} on {member} after the password: {error}; dialog {:?}",
                if basic { "Alt+F3" } else { "F3" },
                support::password_dialog(app)
            )
        })?;
        shows(app, viewing, text)?;
    }
    back_to(app, home, 1, "zipcrypto.zip")?;
    // Closed and opened again: the password is asked again.
    open_row(app, "zipcrypto.zip")?;
    wait_listing(app, "zipcrypto.zip again", &["a.txt", "b.txt", "plain.txt"])?;
    let dialog = ask(app, "b.txt", false)?;
    expect_sentence(&dialog, &view_sentence("zipcrypto.zip", "b.txt"))?;
    let before = support::viewers_opened(app);
    support::dismiss_password(app, Dismiss::Cancel)?;
    dismissed_quietly(app, "b.txt", before)?;
    panel::select_row(app, PaneId::Left, "b.txt")?;
    let (_, viewing) = support::view_unlocking(app, false, PASSWORD, Submit::Enter, LIMIT)?;
    shows(app, viewing, "Secret B, deflated.")?;
    back_to(app, home, 1, "zipcrypto.zip")
}

/// The plain member of an archive with encrypted ones opens without a dialog.
fn plain_member(app: &mut App, home: &Path) -> Result<(), String> {
    open_row(app, "aes.zip")?;
    wait_listing(app, "aes.zip", &["deflated.txt", "notes", "secret.txt"])?;
    open_row(app, "notes")?;
    wait_listing(app, "aes.zip/notes", &["plain.txt"])?;
    panel::select_row(app, PaneId::Left, "plain.txt")?;
    let viewing = support::view(app, false, LIMIT).map_err(|error| {
        format!(
            "F3 on the plain member: {error}; dialog {:?}",
            support::password_dialog(app)
        )
    })?;
    shows(app, viewing, "Plain beside AES.")?;
    back_to(app, home, 2, "aes.zip")
}

/// AES-256: the password once, then the deflated member opens too.
fn aes_members(app: &mut App, home: &Path) -> Result<(), String> {
    open_row(app, "aes.zip")?;
    wait_listing(app, "aes.zip", &["deflated.txt", "notes", "secret.txt"])?;
    panel::select_row(app, PaneId::Left, "secret.txt")?;
    let (dialog, viewing) = support::view_unlocking(app, false, PASSWORD, Submit::Enter, LIMIT)?;
    expect_sentence(&dialog, &view_sentence("aes.zip", "secret.txt"))?;
    shows(app, viewing, "AES-256 secret.")?;
    panel::select_row(app, PaneId::Left, "deflated.txt")?;
    let viewing = support::view(app, false, LIMIT)?;
    shows(app, viewing, "AES-256, deflated.")?;
    back_to(app, home, 1, "aes.zip")
}

/// One AES member of `archive`: a wrong password is refused, the right one opens it.
fn one_member(
    app: &mut App,
    home: &Path,
    archive: &str,
    method: &str,
    text: &str,
) -> Result<(), String> {
    listing(app, home, archive, &[("a.txt", Some(method))])?;
    open_row(app, archive)?;
    wait_listing(app, archive, &["a.txt"])?;
    ask(app, "a.txt", false)?;
    let wrong = support::submit_password(app, "Kyrandia", Submit::Enter, LIMIT)?
        .ok_or("a wrong password closed the dialog")?;
    if wrong.error != WRONG_PASSWORD {
        return Err(format!("a wrong password says {:?}", wrong.error));
    }
    // The refused text is selected: typing replaces it.
    let before = support::viewers_opened(app);
    if let Some(refused) = support::submit_password(app, PASSWORD, Submit::Enter, LIMIT)? {
        return Err(format!(
            "the right password after a wrong one was refused: {:?}",
            refused.error
        ));
    }
    let viewing = support::next_viewer(app, before, LIMIT)?;
    shows(app, viewing, text)?;
    back_to(app, home, 1, archive)
}

/// Enter on the AES archive inside a ZipCrypto archive asks for the outer password; its
/// member then asks for the inner archive's own password; Backspace out of both and in
/// again asks again.
fn nested(app: &mut App, context: &mut Context, home: &Path) -> Result<(), String> {
    open_row(app, "nested.zip")?;
    wait_listing(app, "nested.zip", &["inner.zip", "outer.txt"])?;
    panel::select_row(app, PaneId::Left, "inner.zip")?;
    input::key(app.slint(), Key::Return);
    let outer = support::wait_password_dialog(app, LIMIT)?;
    expect_sentence(
        &outer,
        "nested.zip is password-protected. Enter its password to open inner.zip.",
    )?;
    if support::submit_password(app, PASSWORD, Submit::Enter, LIMIT)?.is_some() {
        return Err("the outer password was refused".into());
    }
    wait_listing(app, "nested.zip/inner.zip", &["inner.txt"])?;
    // The inner archive's own password: the outer one does not open it.
    let inner = ask(app, "inner.txt", false)?;
    app.screenshot(context, "nested-password", &inner.sentence());
    expect_sentence(&inner, &view_sentence("inner.zip", "inner.txt"))?;
    let refused = support::submit_password(app, PASSWORD, Submit::Enter, LIMIT)?
        .ok_or("the outer archive's password opened the inner archive")?;
    if refused.error != WRONG_PASSWORD {
        return Err(format!(
            "the outer password at the inner archive says {:?}",
            refused.error
        ));
    }
    let before = support::viewers_opened(app);
    if support::submit_password(app, INNER_PASSWORD, Submit::Enter, LIMIT)?.is_some() {
        return Err("the inner archive's password was refused".into());
    }
    let viewing = support::next_viewer(app, before, LIMIT)?;
    shows(app, viewing, "Innermost secret.")?;
    // The outer archive's member opens with the outer password it kept.
    panel::wait_input(app);
    input::key(app.slint(), Key::Backspace);
    wait_listing(
        app,
        "nested.zip after inner.zip",
        &["inner.zip", "outer.txt"],
    )?;
    panel::select_row(app, PaneId::Left, "outer.txt")?;
    let viewing = support::view(app, false, LIMIT)?;
    shows(app, viewing, "Outer secret.")?;
    back_to(app, home, 1, "nested.zip")?;
    // Entered again: the outer password is asked again.
    open_row(app, "nested.zip")?;
    wait_listing(app, "nested.zip again", &["inner.zip", "outer.txt"])?;
    panel::select_row(app, PaneId::Left, "inner.zip")?;
    let before = support::viewers_opened(app);
    input::key(app.slint(), Key::Return);
    support::wait_password_dialog(app, LIMIT)?;
    support::dismiss_password(app, Dismiss::Escape)?;
    dismissed_quietly(app, "inner.zip", before)?;
    if shown(app).len() != 2 {
        return Err(format!(
            "after Escape the panel shows {:?}, not nested.zip",
            shown(app)
        ));
    }
    back_to(app, home, 1, "nested.zip")
}

/// Members no reader opens show why in their details, and no password is asked.
fn odd(app: &mut App, context: &mut Context, home: &Path) -> Result<(), String> {
    open_row(app, "odd.zip")?;
    wait_listing(
        app,
        "odd.zip",
        &["../x", "implode.txt", "link", "plain.txt", "strong.txt"],
    )?;
    let expected = [
        (
            "implode.txt",
            "implode.txt is compressed with Implode, which the archive plugin cannot \
             decompress.",
        ),
        (
            "../x",
            "../x has an unsafe name (an absolute path, `..`, a backslash, a drive colon \
             or NUL), so it is never read.",
        ),
        (
            "link",
            "link is a link inside the archive; links are not opened.",
        ),
        (
            "strong.txt",
            "strong.txt is encrypted with an unsupported method (such as PKWARE strong \
             encryption), which the archive plugin cannot decrypt.",
        ),
    ];
    let mut problems = Vec::new();
    for (name, reason) in expected {
        panel::select_row(app, PaneId::Left, name)?;
        let details = details(app);
        let unreadable = details
            .iter()
            .find(|(label, _, _)| label == "Unreadable")
            .map(|(_, value, _)| value.clone());
        if unreadable.as_deref() != Some(reason) {
            problems.push(format!("{name}: Unreadable {unreadable:?}"));
        }
        context.note(format!("odd.zip/{name}: Unreadable {unreadable:?}"));
    }
    // The strong encryption is named as an unknown method, and asks for nothing.
    panel::select_row(app, PaneId::Left, "strong.txt")?;
    let method = details(app)
        .into_iter()
        .find(|(label, _, _)| label == "Encrypted")
        .map(|(_, value, _)| value);
    if method.as_deref() != Some("unknown method") {
        problems.push(format!("strong.txt: Encrypted {method:?}"));
    }
    input::key(app.slint(), Key::F3);
    app.settle();
    if support::password_dialog(app).is_some() {
        problems.push("F3 on strong.txt asked for a password it cannot use".into());
        let _ = support::dismiss_password(app, Dismiss::Escape);
    }
    files::dismiss_all(app);
    back_to(app, home, 1, "odd.zip")?;
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// A 7-Zip archive whose names are encrypted (`7z -mhe=on`), made with the host's `7z`;
/// Err with the reason when this machine has none.
fn seven_zip_names(home: &Path, scratch: &Path) -> Result<(), String> {
    let tree = scratch.join("names");
    std::fs::create_dir_all(&tree).map_err(|error| format!("{}: {error}", tree.display()))?;
    std::fs::write(tree.join("hidden-name.txt"), "Behind encrypted names.\n")
        .map_err(|error| error.to_string())?;
    let archive = home.join("names.7z");
    let password = format!("-p{PASSWORD}");
    let output = std::process::Command::new("7z")
        .args(["a", "-t7z", "-mhe=on", password.as_str()])
        .arg(&archive)
        .arg("hidden-name.txt")
        .current_dir(&tree)
        .stdin(std::process::Stdio::null())
        .output()
        .map_err(|error| format!("no 7-Zip program here ({error})"))?;
    if !output.status.success() || !archive.is_file() {
        let _ = std::fs::remove_file(&archive);
        return Err(format!("7z could not make names.7z ({})", output.status));
    }
    Ok(())
}

/// Enter on a 7-Zip archive with encrypted names asks before listing; the password
/// lists the names.
fn encrypted_names(app: &mut App, home: &Path) -> Result<(), String> {
    open_row(app, "names.7z")?;
    let dialog = support::wait_password_dialog(app, LIMIT)?;
    expect_sentence(
        &dialog,
        "names.7z is password-protected. Its file names are encrypted: enter its password \
         to list them.",
    )?;
    if support::submit_password(app, PASSWORD, Submit::Enter, LIMIT)?.is_some() {
        return Err("the 7-Zip password was refused".into());
    }
    wait_listing(app, "names.7z", &["hidden-name.txt"])?;
    back_to(app, home, 1, "names.7z")
}

/// The password reached neither the saved settings, the clipboard, nor the trace of
/// recent events and commands (where the unlock command prints it redacted).
fn no_leaks(
    app: &App,
    context: &mut Context,
    store: &options_support::SettingsStore,
) -> Result<(), String> {
    let mut problems = Vec::new();
    let secrets = [PASSWORD, INNER_PASSWORD];
    let settings = format!("{:?}", store.current());
    let clipboard = app
        .clipboard
        .as_ref()
        .map(|clipboard| format!("{:?} {:?}", clipboard.text(), clipboard.files()))
        .unwrap_or_default();
    // The trace keeps its last lines only: those kept as each password was answered hold
    // the unlock commands, without which the trace could show no leak at all.
    let mut trace = support::submission_traces();
    match trace.iter().find(|line| line.contains("UnlockArchive")) {
        Some(line) => context.note(format!("The event trace's unlock command: {line}")),
        None => problems.push(
            "the event trace held no unlock command (UnlockArchive) when the passwords were \
             answered, so it could show no leak"
                .to_string(),
        ),
    }
    trace.extend(app.recent_events());
    let trace = trace.join("\n");
    for (place, text) in [
        ("the saved settings", &settings),
        ("the clipboard", &clipboard),
        ("the event trace", &trace),
    ] {
        if secrets.iter().any(|secret| text.contains(secret)) {
            problems.push(format!("a password appears in {place}"));
        }
    }
    if problems.is_empty() {
        Ok(())
    } else {
        Err(problems.join("; "))
    }
}

/// The staged child's console (its standard output and error, which the parent writes
/// to `staged/console.log` beside the child's run folder) holds no password.
fn console_clean() -> Result<(), String> {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    let run = std::env::var_os("MIDNIGHT_ACCEPTANCE_RUN_DIR")
        .ok_or("this process has no run folder of a staged child")?;
    let console = Path::new(&run)
        .parent()
        .map(|staged| staged.join("console.log"))
        .ok_or("the staged run folder has no parent")?;
    let text =
        std::fs::read(&console).map_err(|error| format!("{}: {error}", console.display()))?;
    let text = String::from_utf8_lossy(&text);
    if [PASSWORD, INNER_PASSWORD]
        .iter()
        .any(|secret| text.contains(secret))
    {
        Err(format!("a password appears in {}", console.display()))
    } else {
        Ok(())
    }
}
