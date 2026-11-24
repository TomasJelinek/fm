//! Search inside archives through the installed archive plugin (a staged process, see
//! `viewers_stage`): members of a ZIP, a tar and a tar.gz matched by name (Unicode, a
//! member folder), content (literal and per-line regex), expression and Modified within
//! (tar times carry a zone, ZIP times do not); nested archives are not opened; a broken
//! archive is reported while the search goes on; Go to file opens the archive at the
//! member and Backspace returns to the folder; View, Edit and Open are refused for
//! members; Feed to panel leaves members out (or refuses only-member results); Find
//! cannot start inside an archive. Many archives: 300 ZIPs and one of 20,000 members
//! searched completely, no limit of Find's own; Stop ends a long search inside them
//! while a panel opens an archive; eleven archives of 99,910 members reach the
//! 1,000,000 visited entries.
use crate::{
    Context,
    app::{App, Config},
    encode::{self, Member},
    files_support as files, input, panel,
    scenarios::find_support::{
        self as find, BASE, Findings, Query, Row, all_rows, fill, open_on, relative, run_for,
        search, set, status,
    },
    tools_support::{self as tools, Tool},
    viewers_stage,
};
use i_slint_core::items::AccessibleRole;
use midnight_core::PaneId;
use slint::platform::{Key, WindowAdapter};
use std::{
    collections::BTreeSet,
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const TIMEOUT: Duration = Duration::from_secs(25 * 60);
const LONG: Duration = Duration::from_secs(300);
const MANY: usize = 300;
const LARGE: usize = 20_000;
const BIG: usize = 120_000;

pub fn archives(context: &mut Context) -> Result<(), String> {
    viewers_stage::run(context, TIMEOUT, run)
}

struct Tree {
    arch: PathBuf,
    nested_folders: PathBuf,
    broken: PathBuf,
    many: PathBuf,
    nodes: PathBuf,
    big: PathBuf,
}

fn write(path: &Path, data: &[u8]) -> Result<(), String> {
    tools::write_at(path, data, SystemTime::now())
}

/// Give every member of a ustar archive the modification time `mtime` (seconds).
fn retime_tar(tar: &mut [u8], mtime: u64) {
    let mut offset = 0;
    while offset + 512 <= tar.len() && tar[offset..offset + 512].iter().any(|&byte| byte != 0) {
        let header = &mut tar[offset..offset + 512];
        let size = std::str::from_utf8(&header[124..135])
            .ok()
            .and_then(|text| u64::from_str_radix(text.trim_matches(['\0', ' ']), 8).ok())
            .unwrap_or(0) as usize;
        let text = format!("{mtime:011o}");
        header[136..147].copy_from_slice(text.as_bytes());
        header[147] = 0;
        header[148..156].fill(b' ');
        let sum: u32 = header.iter().map(|&byte| u32::from(byte)).sum();
        header[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
        offset += 512 + size.next_multiple_of(512);
    }
}

/// A ZIP of more than 65,535 members: `encode::zip` with its end record replaced by the
/// ZIP64 end records (APPNOTE 4.3.14 and 4.3.15) that such a count needs; the classic
/// end record then says 0xFFFF entries. Offsets and sizes stay below 4 GiB.
fn zip64(members: &[Member]) -> Result<Vec<u8>, String> {
    let mut data = encode::zip(members);
    let end = data
        .len()
        .checked_sub(22)
        .ok_or("the ZIP has no end record")?;
    let field =
        |at: usize| u32::from_le_bytes([data[at], data[at + 1], data[at + 2], data[at + 3]]);
    if field(end) != 0x0605_4B50 {
        return Err("the ZIP does not end with its end record".into());
    }
    let (size, offset) = (u64::from(field(end + 12)), u64::from(field(end + 16)));
    data.truncate(end);
    let count = members.len() as u64;
    let record = data.len() as u64;
    data.extend_from_slice(&0x0606_4B50_u32.to_le_bytes());
    data.extend_from_slice(&44_u64.to_le_bytes());
    data.extend_from_slice(&45_u16.to_le_bytes());
    data.extend_from_slice(&45_u16.to_le_bytes());
    data.extend_from_slice(&0_u32.to_le_bytes());
    data.extend_from_slice(&0_u32.to_le_bytes());
    data.extend_from_slice(&count.to_le_bytes());
    data.extend_from_slice(&count.to_le_bytes());
    data.extend_from_slice(&size.to_le_bytes());
    data.extend_from_slice(&offset.to_le_bytes());
    data.extend_from_slice(&0x0706_4B50_u32.to_le_bytes());
    data.extend_from_slice(&0_u32.to_le_bytes());
    data.extend_from_slice(&record.to_le_bytes());
    data.extend_from_slice(&1_u32.to_le_bytes());
    data.extend_from_slice(&0x0605_4B50_u32.to_le_bytes());
    data.extend_from_slice(&[0, 0, 0, 0]);
    data.extend_from_slice(&0xFFFF_u16.to_le_bytes());
    data.extend_from_slice(&0xFFFF_u16.to_le_bytes());
    data.extend_from_slice(
        &u32::try_from(size)
            .map_err(|error| error.to_string())?
            .to_le_bytes(),
    );
    data.extend_from_slice(
        &u32::try_from(offset)
            .map_err(|error| error.to_string())?
            .to_le_bytes(),
    );
    data.extend_from_slice(&0_u16.to_le_bytes());
    Ok(data)
}

fn tree(work: &Path) -> Result<Tree, String> {
    let arch = work.join("arch");
    let inner = encode::zip(&[Member::file("inner-quick.txt", "quick inside\n")]);
    let bundle = encode::zip(&[
        Member::directory("docs"),
        Member::file("docs/readme.txt", "Inside the bundle. quick\n"),
        Member::file("docs/notes.md", "notes\n"),
        Member::directory("data"),
        Member::file("data/blob.bin", vec![b'z'; 1500]),
        Member::directory("data/empty"),
        Member::file("ünï 🦀.txt", "unicode member\n"),
        Member::file("nested.zip", inner),
    ]);
    write(&arch.join("bundle.zip"), &bundle)?;
    let mut tar = encode::tar(&[
        Member::directory("tar"),
        Member::file("tar/readme.txt", "tar quick\n"),
        Member::file("tar/size-2000.bin", vec![b't'; 2000]),
        Member::directory("tar/sub"),
    ]);
    let hour_ago = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| error.to_string())?
        .as_secs()
        - 3600;
    retime_tar(&mut tar, hour_ago);
    write(&arch.join("source.tar"), &tar)?;
    write(&arch.join("source.tar.gz"), &encode::gzip(&tar))?;
    write(&arch.join("plain.txt"), b"plain quick\n")?;
    // A single gzip stream: one member, named "content".
    write(&arch.join("notes.txt.gz"), &encode::gzip(b"gz quick\n"))?;

    let broken = work.join("arch-broken");
    write(&broken.join("broken.zip"), b"not a zip archive\n")?;
    write(&broken.join("ok.txt"), b"ok\n")?;

    let many = work.join("arch-many");
    for index in 0..MANY {
        let zip = encode::zip(&[
            Member::directory("inner"),
            Member::file(&format!("m-{index:04}.txt"), "many quick\n"),
        ]);
        write(&many.join(format!("a-{index:04}.zip")), &zip)?;
    }
    let members: Vec<Member> = (0..LARGE)
        .map(|index| Member::file(&format!("member-{index:05}.txt"), "x"))
        .collect();
    write(&many.join("large.zip"), &encode::zip(&members))?;

    // Eleven archives of 10 folders × 9,990 files (99,910 entries each):
    // more than 1,000,000 visited entries without retaining matching results.
    let nodes = work.join("arch-nodes");
    let mut members = Vec::with_capacity(99_910);
    for folder in 0..10 {
        members.push(Member::directory(&format!("d{folder}")));
        for file in 0..9_990 {
            members.push(Member::file(&format!("d{folder}/n{file:05}"), ""));
        }
    }
    let base = nodes.join("nodes-00.zip");
    write(&base, &zip64(&members)?)?;
    for copy in 1..=10 {
        let path = nodes.join(format!("nodes-{copy:02}.zip"));
        fs::hard_link(&base, &path).map_err(|error| format!("{}: {error}", path.display()))?;
    }
    // One folder of 120,000 members: over the archive plugin's former entry limits.
    let big = work.join("arch-big");
    let members: Vec<Member> = (0..BIG)
        .map(|index| Member::file(&format!("big-{index:06}.txt"), ""))
        .collect();
    write(&big.join("big.zip"), &zip64(&members)?)?;
    // An archive at the top of a folder and one in its ordinary subfolder.
    let nested_folders = work.join("arch-folders");
    write(
        &nested_folders.join("top.zip"),
        &encode::zip(&[
            Member::directory("x"),
            Member::file("x/top-member.txt", "top\n"),
        ]),
    )?;
    write(
        &nested_folders.join("folder").join("below.zip"),
        &encode::zip(&[Member::file("below-member.txt", "below\n")]),
    )?;
    Ok(Tree {
        arch,
        nested_folders,
        broken,
        many,
        nodes,
        big,
    })
}

fn inside(query: Query<'_>) -> Query<'_> {
    Query {
        archives: true,
        ..query
    }
}

fn run(context: &mut Context) -> Result<(), String> {
    let tree = tree(&context.work)?;
    let mut config = Config::new(&tree.arch, &tree.many);
    config.plugins = Some(context.work.join("plugins"));
    let mut app = App::start(config);
    let mut findings = Findings::default();
    findings.check("member queries", queries(&mut app, context, &tree));
    find::reset(&mut app);
    findings.check("member actions", member_actions(&mut app, context, &tree));
    find::reset(&mut app);
    findings.check("Subfolders and archives", subfolders(&mut app, &tree));
    find::reset(&mut app);
    findings.check("a broken archive", broken(&mut app, &tree));
    find::reset(&mut app);
    findings.check("Find inside an archive", inside_archive(&mut app, &tree));
    find::reset(&mut app);
    findings.check("many archives", many(&mut app, context, &tree));
    find::reset(&mut app);
    findings.check("Stop inside archives", stop(&mut app, context, &tree));
    find::reset(&mut app);
    findings.check("visited entries", nodes(&mut app, &tree));
    find::reset(&mut app);
    findings.check(
        "an archive over 100,000 entries",
        over_limit(&mut app, context, &tree),
    );
    find::reset(&mut app);
    app.shutdown()?;
    findings.into_result()
}

/// Search `query` over `root` and compare the results with `expected`.
fn expect(
    app: &mut App,
    window: &Tool,
    root: &Path,
    what: &str,
    query: Query<'_>,
    expected: &[&str],
) -> Result<Vec<Row>, String> {
    let (text, rows) = search(app, window, inside(query))?;
    let found = relative(&rows, root);
    if !text.starts_with("Completed") || found != set(expected) {
        return Err(format!(
            "{what}: {text:?}, found {found:?}, expected {expected:?}"
        ));
    }
    Ok(rows)
}

fn queries(app: &mut App, context: &mut Context, tree: &Tree) -> Result<(), String> {
    let root = &tree.arch;
    let window = open_on(app, PaneId::Left)?;
    let mut problems = Findings::default();
    let readme = [
        "bundle.zip/docs/readme.txt",
        "source.tar/tar/readme.txt",
        "source.tar.gz/tar/readme.txt",
    ];
    let rows = expect(
        app,
        &window,
        root,
        "readme.txt in three archives",
        Query {
            masks: "readme.txt",
            ..BASE
        },
        &readme,
    )?;
    app.screenshot_window(
        context,
        &window,
        "members",
        "readme.txt found in a ZIP, a tar and a tar.gz",
    );
    let texts = find::details(app, &window)?;
    if find::detail_count(&texts, "Inside archives:", " archives") != Some(4) {
        return Err(format!(
            "the details do not count four archives (three and the gzip stream): {texts:?}"
        ));
    }
    let member = rows
        .iter()
        .find(|row| Path::new(row.folder.as_str()).ends_with(Path::new("bundle.zip").join("docs")))
        .ok_or("no bundle.zip member row")?;
    if member.size != "25 B" {
        problems.add(format!("the bundle member's size reads {:?}", member.size));
    }
    // Without the option the same query finds nothing inside.
    let (text, rows) = search(
        app,
        &window,
        Query {
            masks: "readme.txt",
            ..BASE
        },
    )?;
    if !text.starts_with("Completed") || !rows.is_empty() {
        problems.add(format!(
            "readme.txt without Search inside archives: {text:?}, {:?}",
            relative(&rows, root)
        ));
    }
    // Subfolders limits recursion into ordinary folders only: an archive in a searched
    // folder is searched completely, member folders included.
    problems.check(
        "Subfolders off inside an archive",
        expect(
            app,
            &window,
            root,
            "readme.txt with Subfolders off",
            Query {
                masks: "readme.txt",
                subfolders: false,
                ..BASE
            },
            &readme,
        )
        .map(|_| ()),
    );
    let cases: Vec<(&str, Query<'_>, Vec<&str>)> = vec![
        (
            "a Unicode member name",
            Query {
                masks: "*🦀*",
                ..BASE
            },
            vec!["bundle.zip/ünï 🦀.txt"],
        ),
        (
            "member folders",
            Query {
                masks: "empty;sub",
                ..BASE
            },
            vec![
                "bundle.zip/data/empty",
                "source.tar/tar/sub",
                "source.tar.gz/tar/sub",
            ],
        ),
        (
            "a member archive is matched, not opened",
            Query {
                masks: "nested.zip;inner-quick.txt",
                ..BASE
            },
            vec!["bundle.zip/nested.zip"],
        ),
        (
            "content in members",
            Query {
                masks: "*.txt",
                content: Some("quick"),
                ..BASE
            },
            vec![
                "plain.txt",
                "bundle.zip/docs/readme.txt",
                "source.tar/tar/readme.txt",
                "source.tar.gz/tar/readme.txt",
            ],
        ),
        (
            "the content of a gzip stream's only member",
            Query {
                masks: "content",
                content: Some("gz quick"),
                ..BASE
            },
            vec!["notes.txt.gz/content"],
        ),
        (
            "a per-line regex in members",
            Query {
                masks: "*.txt",
                content: Some("^tar quick$"),
                content_regex: true,
                ..BASE
            },
            vec!["source.tar/tar/readme.txt", "source.tar.gz/tar/readme.txt"],
        ),
        (
            "an expression over members and files",
            Query {
                mode: "Expression",
                masks: "kind = file and size >= 1500",
                ..BASE
            },
            vec![
                "bundle.zip",
                "source.tar",
                "source.tar.gz",
                "bundle.zip/data/blob.bin",
                "source.tar/tar/size-2000.bin",
                "source.tar.gz/tar/size-2000.bin",
            ],
        ),
        (
            "Modified within: tar times have a zone, ZIP times do not",
            Query {
                masks: "*.txt",
                modified: Some(("1", "Days")),
                ..BASE
            },
            vec![
                "plain.txt",
                "source.tar/tar/readme.txt",
                "source.tar.gz/tar/readme.txt",
            ],
        ),
    ];
    for (what, query, wanted) in cases {
        problems.check(
            what,
            expect(app, &window, root, what, query, &wanted).map(|_| ()),
        );
    }
    find::close(app, &window);
    problems.into_result()
}

fn member_actions(app: &mut App, context: &mut Context, tree: &Tree) -> Result<(), String> {
    let root = &tree.arch;
    let window = open_on(app, PaneId::Left)?;
    let w = window.window();
    expect(
        app,
        &window,
        root,
        "readme.txt",
        Query {
            masks: "readme.txt",
            ..BASE
        },
        &[
            "bundle.zip/docs/readme.txt",
            "source.tar/tar/readme.txt",
            "source.tar.gz/tar/readme.txt",
        ],
    )?;
    let docs = root.join("bundle.zip").join("docs");
    find::select_result(app, &window, "readme.txt", Some(docs.as_path()))?;
    for button in ["View", "Edit", "Open"] {
        if find::button_enabled(&window, button)? {
            return Err(format!("{button} is enabled for an archive member"));
        }
    }
    if !find::button_enabled(&window, "Go to file")? {
        return Err("Go to file is disabled for an archive member".into());
    }
    // F3 asks anyway: the member opens from its archive.
    input::key(w, Key::F3);
    find::wait_error(app, &window, "F3 on a member", |text| {
        text.contains("An archive member opens from its archive")
    })?;
    // Only members found: nothing to feed. The backend's feed preparation refuses, and
    // its details name the operation as every backend refusal does.
    tools::click_last(w, "Feed to panel")?;
    find::wait_error(app, &window, "Feed to panel with only members", |text| {
        text == "Only archive members were found; open them with Go to file.\n\nOperation: feed search results"
    })?;
    if find::feed_view(app, PaneId::Left).open {
        return Err("only members were fed to the panel".into());
    }
    // Go to file opens the archive folder with the cursor on the member.
    find::select_result(app, &window, "readme.txt", Some(docs.as_path()))?;
    tools::click_last(w, "Go to file")?;
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        app.settle();
        let path = app.window.get_left_path().to_string();
        if Path::new(&path).ends_with(Path::new("bundle.zip").join("docs"))
            && !app.loading(PaneId::Left)
            && files::cursor_name(app, PaneId::Left).as_deref() == Some("readme.txt")
        {
            break;
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "Go to file of a member: the panel shows {path:?} with the cursor on {:?}",
                files::cursor_name(app, PaneId::Left)
            ));
        }
        app.idle(Duration::from_millis(20));
    }
    app.screenshot(
        context,
        "member-goto",
        "Go to file opened bundle.zip/docs at readme.txt",
    );
    find::close(app, &window);
    // Backspace leaves the archive for the folder the panel showed before.
    panel::focus_list(app, PaneId::Left)?;
    input::key(app.slint(), Key::Backspace);
    app.until("the archive's root", |app| {
        app.window.get_left_path().ends_with("bundle.zip") && !app.loading(PaneId::Left)
    });
    input::key(app.slint(), Key::Backspace);
    panel::wait_location(app, PaneId::Left, root);
    if files::cursor_name(app, PaneId::Left).as_deref() != Some("bundle.zip") {
        return Err(format!(
            "leaving the archive put the cursor on {:?}",
            files::cursor_name(app, PaneId::Left)
        ));
    }
    // Feed to panel lists the native results only and counts the members.
    let window = open_on(app, PaneId::Left)?;
    expect(
        app,
        &window,
        root,
        "*.txt",
        Query {
            masks: "*.txt",
            ..BASE
        },
        &[
            "plain.txt",
            "bundle.zip/docs/readme.txt",
            "bundle.zip/ünï 🦀.txt",
            "source.tar/tar/readme.txt",
            "source.tar.gz/tar/readme.txt",
        ],
    )?;
    tools::click_last(window.window(), "Feed to panel")?;
    app.until("the results list", |app| {
        find::feed_view(app, PaneId::Left).open
    });
    let names = find::feed_names(app, PaneId::Left);
    let status = find::feed_view(app, PaneId::Left).status.to_string();
    app.screenshot(
        context,
        "feed-members",
        "Feed to panel: members are not listed",
    );
    input::key(app.slint(), Key::Backspace);
    app.until("the list to close", |app| {
        !find::feed_view(app, PaneId::Left).open
    });
    if names != ["plain.txt"] || !status.contains("4 archive members not shown") {
        return Err(format!(
            "the fed list shows {names:?} with status {status:?}"
        ));
    }
    Ok(())
}

/// Subfolders off: the archive at the top of the searched folder is searched completely
/// (its member folder included), the archive in an ordinary subfolder is not reached;
/// Subfolders on reaches both.
fn subfolders(app: &mut App, tree: &Tree) -> Result<(), String> {
    let window = open_on(app, PaneId::Left)?;
    find::add_root(app, &window, &tree.nested_folders, false)?;
    find::tick_root(app, &window, &tree.arch, false)?;
    let root = &tree.nested_folders;
    let mut problems = Findings::default();
    for (subfolders, wanted) in [
        (false, &["top.zip/x/top-member.txt"][..]),
        (
            true,
            &[
                "top.zip/x/top-member.txt",
                "folder/below.zip/below-member.txt",
            ][..],
        ),
    ] {
        let what = format!(
            "*-member.txt with Subfolders {}",
            if subfolders { "on" } else { "off" }
        );
        problems.check(
            &what,
            expect(
                app,
                &window,
                root,
                &what,
                Query {
                    masks: "*-member.txt",
                    subfolders,
                    ..BASE
                },
                wanted,
            )
            .map(|_| ()),
        );
    }
    find::close(app, &window);
    problems.into_result()
}

/// Two folders searched inside archives at once; then a file named .zip that is no
/// archive alone: reported, and the search goes on.
fn broken(app: &mut App, tree: &Tree) -> Result<(), String> {
    let window = open_on(app, PaneId::Left)?;
    find::add_root(app, &window, &tree.broken, false)?;
    // Two folders at once: the members of the first folder's archives and the second
    // folder's file; the broken archive makes the search Incomplete.
    let (text, rows) = search(
        app,
        &window,
        inside(Query {
            masks: "*.txt",
            ..BASE
        }),
    )?;
    let work = tree
        .arch
        .parent()
        .ok_or("the archive folder has no parent")?;
    let wanted = set(&[
        "arch/plain.txt",
        "arch/bundle.zip/docs/readme.txt",
        "arch/bundle.zip/ünï 🦀.txt",
        "arch/source.tar/tar/readme.txt",
        "arch/source.tar.gz/tar/readme.txt",
        "arch-broken/ok.txt",
    ]);
    if !text.starts_with("Incomplete") || relative(&rows, work) != wanted {
        return Err(format!(
            "two folders with archives: {text:?}, {:?}",
            relative(&rows, work)
        ));
    }
    find::tick_root(app, &window, &tree.arch, false)?;
    let (text, rows) = search(
        app,
        &window,
        inside(Query {
            masks: "*.txt",
            ..BASE
        }),
    )?;
    let texts = find::details(app, &window)?;
    find::close(app, &window);
    if !text.starts_with("Incomplete")
        || relative(&rows, &tree.broken) != set(&["ok.txt"])
        || !find::mentions(&texts, "broken.zip")
    {
        return Err(format!(
            "a broken archive: {text:?}, {:?}, details {texts:?}",
            relative(&rows, &tree.broken)
        ));
    }
    Ok(())
}

/// Find starts only in native folders: Alt+F7 inside an archive explains, and the
/// toolbar's Find button is disabled there.
fn inside_archive(app: &mut App, tree: &Tree) -> Result<(), String> {
    panel::select_row(app, PaneId::Left, "bundle.zip")?;
    input::key(app.slint(), Key::Return);
    app.until("bundle.zip to open", |app| {
        app.window.get_left_path().ends_with("bundle.zip") && !app.loading(PaneId::Left)
    });
    let before = app.tool_windows().len();
    input::chord(app.slint(), &[Key::Alt], Key::F7);
    let notice = find::wait_notice(app, "Alt+F7 inside an archive")?;
    find::dismiss_notice(app);
    let disabled = find::main_buttons(app, "Find files and text")
        .iter()
        .all(|(_, button)| !tools::enabled(button));
    input::key(app.slint(), Key::Backspace);
    panel::wait_location(app, PaneId::Left, &tree.arch);
    if notice.0 != "Find files"
        || !notice
            .1
            .starts_with("Recursive search is available in native folders.")
        || app.tool_windows().len() != before
        || !disabled
    {
        return Err(format!(
            "Alt+F7 inside an archive: notice {notice:?}, toolbar Find disabled {disabled}"
        ));
    }
    Ok(())
}

/// 300 ZIPs and a ZIP of 20,000 members, every one opened and searched.
fn many(app: &mut App, context: &mut Context, tree: &Tree) -> Result<(), String> {
    let window = open_on(app, PaneId::Right)?;
    fill(
        app,
        &window,
        inside(Query {
            masks: "m-*.txt;member-19999.txt",
            ..BASE
        }),
    )?;
    let started = Instant::now();
    let text = run_for(app, &window, LONG)?;
    let took = started.elapsed();
    let rows = all_rows(app, &window)?;
    let texts = find::details(app, &window)?;
    app.screenshot_window(context, &window, "many-archives", "301 archives searched");
    find::close(app, &window);
    let found = relative(&rows, &tree.many);
    let mut expected: BTreeSet<String> = (0..MANY)
        .map(|index| format!("a-{index:04}.zip/m-{index:04}.txt"))
        .collect();
    expected.insert("large.zip/member-19999.txt".into());
    if !text.starts_with("Completed") || found != expected {
        return Err(format!(
            "many archives: {text:?}, {} rows ({} expected)",
            found.len(),
            expected.len()
        ));
    }
    let counts = (
        find::detail_count(&texts, "Inside archives:", " archives"),
        find::detail_count(&texts, "Inside archives:", " members"),
    );
    let members = (MANY * 2 + LARGE) as u64;
    if counts != (Some(MANY as u64 + 1), Some(members)) {
        return Err(format!(
            "many archives: the details count {counts:?}, expected {} archives and {members} members",
            MANY + 1
        ));
    }
    context.note(format!(
        "Search inside {} archives ({members} members): {text} in {:.1} s",
        MANY + 1,
        took.as_secs_f64()
    ));
    Ok(())
}

/// Stop ends a long content search inside archives; a panel opens an archive meanwhile.
fn stop(app: &mut App, context: &mut Context, tree: &Tree) -> Result<(), String> {
    let window = open_on(app, PaneId::Right)?;
    fill(
        app,
        &window,
        inside(Query {
            content: Some("absent-needle"),
            ..BASE
        }),
    )?;
    tools::click_last(window.window(), "Find")?;
    let watched = window.clone();
    app.until("the search inside archives to run", move |_| {
        status(&watched).starts_with("Searching")
    });
    // The left panel opens an archive while the search reads members.
    panel::select_row(app, PaneId::Left, "bundle.zip")?;
    input::key(app.slint(), Key::Return);
    app.until("bundle.zip to open beside the search", |app| {
        app.window.get_left_path().ends_with("bundle.zip") && !app.loading(PaneId::Left)
    });
    let panel_names: BTreeSet<String> = app
        .names(PaneId::Left)
        .into_iter()
        .filter(|name| name != "..")
        .collect();
    let running = status(&window);
    if !running.starts_with("Searching") {
        return Err(format!(
            "the content search inside 301 archives ended before Stop: {running:?}"
        ));
    }
    let stopped = Instant::now();
    tools::click_last(window.window(), "Stop")?;
    let watched = window.clone();
    app.until_for("Stop inside archives", Duration::from_secs(30), move |_| {
        status(&watched).starts_with("Cancelled")
            && crate::inspect::find(watched.window(), AccessibleRole::Button, "Find").is_some()
    });
    let took = stopped.elapsed();
    let text = status(&window);
    app.screenshot_window(context, &window, "stopped-archives", "Stop inside archives");
    find::close(app, &window);
    panel::focus_list(app, PaneId::Left)?;
    input::key(app.slint(), Key::Backspace);
    panel::wait_location(app, PaneId::Left, &tree.arch);
    context.note(format!(
        "Stop inside archives: {running:?} → {text:?} in {} ms",
        took.as_millis()
    ));
    if panel_names != set(&["data", "docs", "nested.zip", "ünï 🦀.txt"]) {
        return Err(format!(
            "bundle.zip opened beside the search lists {panel_names:?}"
        ));
    }
    if took > Duration::from_secs(5) {
        return Err(format!("Stop took {} ms inside archives", took.as_millis()));
    }
    Ok(())
}

/// Every member is visited beyond the former million-entry scan ceiling.
fn nodes(app: &mut App, tree: &Tree) -> Result<(), String> {
    let window = open_on(app, PaneId::Left)?;
    find::add_root(app, &window, &tree.nodes, false)?;
    find::tick_root(app, &window, &tree.arch, false)?;
    fill(
        app,
        &window,
        inside(Query {
            masks: "no-such-member",
            ..BASE
        }),
    )?;
    let started = Instant::now();
    let text = run_for(app, &window, LONG)?;
    let took = started.elapsed();
    let texts = find::details(app, &window)?;
    find::close(app, &window);
    if !text.starts_with("Completed")
        || tools::count_before(&text, " examined") != Some(11 * 99_910 + 11)
        || find::mentions(&texts, "Reached limits: visited entries")
    {
        return Err(format!(
            "eleven archives of 99,910 members: {text:?} in {:.1} s, details {texts:?}",
            took.as_secs_f64()
        ));
    }
    Ok(())
}

/// An archive of 120,000 members in one folder is searched completely: Find has no
/// limit of its own, and the archive plugin has no entry limit either.
fn over_limit(app: &mut App, context: &mut Context, tree: &Tree) -> Result<(), String> {
    let window = open_on(app, PaneId::Left)?;
    find::add_root(app, &window, &tree.big, false)?;
    find::tick_root(app, &window, &tree.arch, false)?;
    fill(
        app,
        &window,
        inside(Query {
            masks: "big-119999.txt",
            ..BASE
        }),
    )?;
    let started = Instant::now();
    let text = run_for(app, &window, LONG)?;
    let took = started.elapsed();
    let rows = all_rows(app, &window)?;
    let texts = find::details(app, &window)?;
    find::close(app, &window);
    let found = relative(&rows, &tree.big);
    let members = find::detail_count(&texts, "Inside archives:", " members");
    if !text.starts_with("Completed")
        || found != set(&["big.zip/big-119999.txt"])
        || members != Some(BIG as u64)
    {
        return Err(format!(
            "a 120,000-member archive: {text:?}, found {found:?}, {members:?} members, details {texts:?}"
        ));
    }
    context.note(format!(
        "An archive of 120,000 members searched completely in {:.1} s: {text}",
        took.as_secs_f64()
    ));
    Ok(())
}
