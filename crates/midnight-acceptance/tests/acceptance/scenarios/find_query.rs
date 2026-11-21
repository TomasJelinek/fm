//! Find's query language end to end, beyond `tools_find_criteria`, on a generated tree
//! with known answers: masks (`;;`, `*.*` with extensionless names, several
//! exclusions, an excluded folder that is still searched, Unicode letter case), regular
//! expressions (`(?i)` against Match name case), typed expressions (decimal and binary
//! size units, `=~`, `not`, kind, times with a UTC offset at the boundary,
//! present/missing), containing text in every supported encoding (UTF-8 with and
//! without BOM, UTF-16 LE/BE with BOM; BOM-less UTF-16 is not decoded; malformed UTF-8
//! is reported, also after a match), Unicode letter case, whole words at combining
//! marks and connectors, CRLF and per-line regular expressions, Modified within in
//! minutes, hours and days with its validation and the collapsed More options summary,
//! and Identical files (empty and binary groups, hard links counted once, groups kept
//! together under every sort, Search inside archives disabled with it).
use crate::{
    Context,
    app::{App, Config},
    input,
    scenarios::find_support::{
        self as find, BASE, Findings, Query, Row, all_rows, fill, mentions, open_find, ordered,
        relative, run, search, set, status,
    },
    tools_support::{self as tools, Tool},
};
use i_slint_core::items::AccessibleRole;
use slint::platform::{Key, WindowAdapter};
use std::{
    collections::BTreeSet,
    fs,
    path::Path,
    time::{Duration, SystemTime},
};

const INVALID_AGE: &str = "Enter a positive whole number, no more than 366 days.";

fn utf16(text: &str, big_endian: bool, bom: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    if bom {
        bytes.extend_from_slice(if big_endian {
            &[0xFE, 0xFF]
        } else {
            &[0xFF, 0xFE]
        });
    }
    for unit in text.encode_utf16() {
        bytes.extend_from_slice(&if big_endian {
            unit.to_be_bytes()
        } else {
            unit.to_le_bytes()
        });
    }
    bytes
}

/// The generated tree: `names/`, `sizes/`, `text/`, `bad/`, `times/` and `dupes/`.
fn query_tree(root: &Path) -> Result<(), String> {
    let now = SystemTime::now();
    let mut utf8_bom = vec![0xEF, 0xBB, 0xBF];
    utf8_bom.extend_from_slice(b"A quick start\n");
    let files: Vec<(&str, Vec<u8>)> = vec![
        ("names/README", b"readme\n".to_vec()),
        ("names/Makefile", b"all:\n".to_vec()),
        ("names/notes;draft.txt", b"draft\n".to_vec()),
        ("names/über.txt", b"umlaut name\n".to_vec()),
        ("names/ÜBERSICHT.md", b"overview\n".to_vec()),
        ("names/keep.bak", b"backup\n".to_vec()),
        ("names/temp.tmp", b"temporary\n".to_vec()),
        ("names/excluded/inside.txt", b"inside\n".to_vec()),
        ("sizes/k1000.dat", vec![b'k'; 1000]),
        ("sizes/k1010.dat", vec![b'k'; 1010]),
        ("sizes/k1024.dat", vec![b'k'; 1024]),
        ("sizes/dated.txt", b"dated\n".to_vec()),
        ("text/utf8.txt", b"The quick fox\n".to_vec()),
        ("text/utf8-bom.txt", utf8_bom),
        ("text/utf16le.txt", utf16("quick one\n", false, true)),
        ("text/utf16be.txt", utf16("very quick\n", true, true)),
        ("text/utf16-nobom.txt", utf16("quick\n", false, false)),
        ("text/crlf.txt", b"quick\r\nbrown\r\n".to_vec()),
        ("text/twolines.txt", b"quick\nbrown fox\n".to_vec()),
        ("text/oneline.txt", b"quick brown\n".to_vec()),
        ("text/umlaut.txt", "Grüße aus Köln\n".as_bytes().to_vec()),
        (
            "text/combining.txt",
            "cafe\u{301} au lait\n".as_bytes().to_vec(),
        ),
        ("text/connector.txt", b"foo_bar baz\n".to_vec()),
        ("bad/latin1.enc", b"caf\xE9 quick\n".to_vec()),
        ("bad/trailing.enc", b"quick then \xFF\n".to_vec()),
    ];
    for (path, data) in &files {
        tools::write_at(&root.join(path), data, now)?;
    }
    // 2021-06-15T12:00:00Z
    tools::set_modified(
        &root.join("sizes/dated.txt"),
        SystemTime::UNIX_EPOCH + Duration::from_secs(1_623_758_400),
    )?;
    let minutes = |count: u64| Duration::from_secs(count * 60);
    for (name, earlier, later) in [
        ("m30.age", minutes(30), Duration::ZERO),
        ("h3.age", minutes(3 * 60), Duration::ZERO),
        ("d2.age", minutes(2 * 24 * 60), Duration::ZERO),
        ("d40.age", minutes(40 * 24 * 60), Duration::ZERO),
        ("y2.age", minutes(400 * 24 * 60), Duration::ZERO),
        ("future.age", Duration::ZERO, minutes(2 * 24 * 60)),
    ] {
        tools::write_at(
            &root.join("times").join(name),
            name.as_bytes(),
            now - earlier + later,
        )?;
    }
    // Identical files: groups of empty, 100-byte and 5000-byte files whose members'
    // names interleave with the other groups', a same-size different file, a hard-link
    // pair without a copy, and a unique file.
    let hundred = vec![b'h'; 100];
    let mut binary = Vec::with_capacity(5000);
    let mut state = 0x2545_F491_u32;
    for _ in 0..5000 {
        state ^= state << 13;
        state ^= state >> 17;
        state ^= state << 5;
        binary.push((state >> 24) as u8);
    }
    let mut linked = binary[..4096].to_vec();
    linked.reverse();
    let mut different = hundred.clone();
    different[50] = b'H';
    for (path, data) in [
        ("dupes/e-empty", &[][..]),
        ("dupes/x-empty.txt", &[][..]),
        ("dupes/a-hundred.dat", &hundred[..]),
        ("dupes/sub/m-hundred.bin", &hundred[..]),
        ("dupes/z-hundred.txt", &hundred[..]),
        ("dupes/c-different.dat", &different[..]),
        ("dupes/b-binary.bin", &binary[..]),
        ("dupes/y-binary.bin", &binary[..]),
        ("dupes/d-link-a.bin", &linked[..]),
        ("dupes/u-unique.dat", &b"unique\n"[..]),
    ] {
        tools::write_at(&root.join(path), data, now)?;
    }
    fs::hard_link(
        root.join("dupes/d-link-a.bin"),
        root.join("dupes/w-link-b.bin"),
    )
    .map_err(|error| format!("hard link: {error}"))
}

/// Every entry below `root` (files and folders, links not followed), relative.
fn walk(root: &Path) -> BTreeSet<String> {
    fn visit(root: &Path, folder: &Path, found: &mut BTreeSet<String>) {
        for entry in fs::read_dir(folder).into_iter().flatten().flatten() {
            let path = entry.path();
            if let Ok(relative) = path.strip_prefix(root) {
                found.insert(crate::files_support::slashed(relative));
            }
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                visit(root, &path, found);
            }
        }
    }
    let mut found = BTreeSet::new();
    visit(root, root, &mut found);
    found
}

/// Search `query` and compare the results (relative to `root`) with `expected` and the
/// status line with `completion` and the expected count.
fn expect(
    app: &mut App,
    window: &Tool,
    root: &Path,
    what: &str,
    query: Query<'_>,
    expected: &BTreeSet<String>,
    completion: &str,
) -> Result<Vec<Row>, String> {
    let (text, rows) = search(app, window, query)?;
    let found = relative(&rows, root);
    if !text.starts_with(completion) {
        return Err(format!(
            "{what}: the search ended {text:?}, not {completion}"
        ));
    }
    if found != *expected || rows.len() != expected.len() {
        let missing: Vec<_> = expected.difference(&found).collect();
        let extra: Vec<_> = found.difference(expected).collect();
        return Err(format!(
            "{what}: {} rows; missing {missing:?}, unexpected {extra:?}",
            rows.len()
        ));
    }
    if tools::count_before(&text, " found") != Some(expected.len() as u64) {
        return Err(format!("{what}: the status reads {text:?}"));
    }
    Ok(rows)
}

pub fn query_syntax(context: &mut Context) -> Result<(), String> {
    let root = context.work.join("query");
    query_tree(&root)?;
    let mut app = App::start(Config::new(&root, &context.work));
    let window = open_find(&mut app)?;
    let mut findings = Findings::default();
    let all = walk(&root);
    let without = |excluded: &[&str]| -> BTreeSet<String> {
        all.iter()
            .filter(|path| !excluded.contains(&path.as_str()))
            .cloned()
            .collect()
    };
    let mut checked = 0;
    let mut case = |app: &mut App,
                    context: &mut Context,
                    what: &str,
                    query: Query<'_>,
                    expected: BTreeSet<String>,
                    findings: &mut Findings| {
        checked += 1;
        let result = expect(app, &window, &root, what, query, &expected, "Completed");
        if result.is_err() {
            let name = format!("failed-{checked:02}");
            app.screenshot_window(context, &window, &name, &format!("Failed: {what}"));
        }
        findings.check(what, result.map(|_| ()));
    };

    // Masks.
    case(
        &mut app,
        context,
        "*.* lists extensionless names and folders too",
        BASE,
        all.clone(),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "a doubled ;; is a literal semicolon",
        Query {
            masks: "notes;;draft.txt",
            ..BASE
        },
        set(&["names/notes;draft.txt"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "two exclusions after |",
        Query {
            masks: "*.*|*.bak;*.tmp",
            ..BASE
        },
        without(&["names/keep.bak", "names/temp.tmp"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "an excluded folder is still searched",
        Query {
            masks: "*.*|excluded",
            ..BASE
        },
        without(&["names/excluded"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "exact extensionless names",
        Query {
            masks: "README;Makefile",
            ..BASE
        },
        set(&["names/README", "names/Makefile"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "Unicode letters in any case",
        Query {
            masks: "über*",
            ..BASE
        },
        set(&["names/über.txt", "names/ÜBERSICHT.md"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "Unicode letters, Match name case (lower)",
        Query {
            masks: "über*",
            name_case: true,
            ..BASE
        },
        set(&["names/über.txt"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "Unicode letters, Match name case (upper)",
        Query {
            masks: "ÜBER*",
            name_case: true,
            ..BASE
        },
        set(&["names/ÜBERSICHT.md"]),
        &mut findings,
    );

    // Regular expressions over names.
    case(
        &mut app,
        context,
        "regex ^readme$",
        Query {
            mode: "Regex",
            masks: "^readme$",
            ..BASE
        },
        set(&["names/README"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "regex ^readme$ with Match name case",
        Query {
            mode: "Regex",
            masks: "^readme$",
            name_case: true,
            ..BASE
        },
        BTreeSet::new(),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "regex (?i) overrides Match name case",
        Query {
            mode: "Regex",
            masks: "(?i)^readme$",
            name_case: true,
            ..BASE
        },
        set(&["names/README"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "regex alternatives at the end",
        Query {
            mode: "Regex",
            masks: "[.](bak|tmp)$",
            ..BASE
        },
        set(&["names/keep.bak", "names/temp.tmp"]),
        &mut findings,
    );

    // Typed expressions.
    let expression = |masks| Query {
        mode: "Expression",
        masks,
        ..BASE
    };
    case(
        &mut app,
        context,
        "decimal kB",
        expression("kind = file and name like \"k*.dat\" and size >= 1kB"),
        set(&["sizes/k1000.dat", "sizes/k1010.dat", "sizes/k1024.dat"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "binary KiB",
        expression("kind = file and name like \"k*.dat\" and size >= 1KiB"),
        set(&["sizes/k1024.dat"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "bytes with B between two sizes",
        expression("kind = file and size > 1000B and size < 1KiB"),
        set(&["sizes/k1010.dat"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "name =~ regex",
        expression("name =~ \"^k10[01][0-9][.]dat$\""),
        set(&["sizes/k1000.dat", "sizes/k1010.dat"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "not with parentheses",
        expression("kind = file and name like \"*.age\" and not (name like \"*d*\")"),
        set(&[
            "times/m30.age",
            "times/h3.age",
            "times/future.age",
            "times/y2.age",
        ]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "a time with a UTC offset, inclusive bound",
        expression(
            "modified >= \"2021-06-15T14:00:00+02:00\" and modified < \"2021-06-16T00:00:00Z\"",
        ),
        set(&["sizes/dated.txt"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "a time with a UTC offset, exclusive bound",
        expression(
            "modified > \"2021-06-15T14:00:00+02:00\" and modified < \"2021-06-16T00:00:00Z\"",
        ),
        BTreeSet::new(),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "folders by kind and name regex",
        expression("kind = directory and name =~ \"^(names|text|bad)$\""),
        set(&["names", "text", "bad"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "missing(modified)",
        expression("missing(modified)"),
        BTreeSet::new(),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "present(size) and an exact name",
        expression("present(size) and name = \"Makefile\""),
        set(&["names/Makefile"]),
        &mut findings,
    );

    // Containing text: only *.txt files, all valid UTF-8 or BOM-marked UTF-16.
    let content = |text, case_sensitive, whole_words, regex| Query {
        masks: "*.txt",
        content: Some(text),
        content_case: case_sensitive,
        whole_words,
        content_regex: regex,
        ..BASE
    };
    case(
        &mut app,
        context,
        "quick in UTF-8, UTF-8 BOM, UTF-16 LE and BE (not BOM-less UTF-16)",
        content("quick", false, false, false),
        set(&[
            "text/utf8.txt",
            "text/utf8-bom.txt",
            "text/utf16le.txt",
            "text/utf16be.txt",
            "text/crlf.txt",
            "text/twolines.txt",
            "text/oneline.txt",
        ]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "QUICK with Match case",
        content("QUICK", true, false, false),
        BTreeSet::new(),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "Unicode letters in any case",
        content("KÖLN", false, false, false),
        set(&["text/umlaut.txt"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "Unicode letters, Match case (other case)",
        content("KÖLN", true, false, false),
        BTreeSet::new(),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "Unicode letters, Match case (same case)",
        content("Köln", true, false, false),
        set(&["text/umlaut.txt"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "a prefix before a combining mark",
        content("cafe", false, false, false),
        set(&["text/combining.txt"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "whole words: a combining mark continues the word",
        content("cafe", false, true, false),
        BTreeSet::new(),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "whole words: an underscore continues the word",
        content("foo", false, true, false),
        BTreeSet::new(),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "whole words: a word after a space",
        content("baz", false, true, false),
        set(&["text/connector.txt"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "regex lines without their CR",
        content("^quick$", false, false, true),
        set(&["text/crlf.txt", "text/twolines.txt"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "regex $ before CRLF",
        content("brown$", false, false, true),
        set(&["text/crlf.txt", "text/oneline.txt"]),
        &mut findings,
    );
    case(
        &mut app,
        context,
        "regex never spans lines",
        content("quick\\sbrown", false, false, true),
        set(&["text/oneline.txt"]),
        &mut findings,
    );

    // Malformed UTF-8: reported per file, also after a match; nothing listed.
    findings.check("malformed UTF-8", malformed(&mut app, &window, &root));

    // Modified within, in each unit.
    let age = |number, unit| Query {
        masks: "*.age",
        modified: Some((number, unit)),
        ..BASE
    };
    for (number, unit, expected) in [
        ("1", "Hours", &["times/m30.age"][..]),
        ("45", "Minutes", &["times/m30.age"][..]),
        ("29", "Minutes", &[][..]),
        ("4", "Hours", &["times/m30.age", "times/h3.age"][..]),
        (
            "3",
            "Days",
            &["times/m30.age", "times/h3.age", "times/d2.age"][..],
        ),
        (
            "366",
            "Days",
            &[
                "times/m30.age",
                "times/h3.age",
                "times/d2.age",
                "times/d40.age",
            ][..],
        ),
        (
            "8784",
            "Hours",
            &[
                "times/m30.age",
                "times/h3.age",
                "times/d2.age",
                "times/d40.age",
            ][..],
        ),
    ] {
        case(
            &mut app,
            context,
            &format!("modified within {number} {unit} (never the future file)"),
            age(number, unit),
            set(expected),
            &mut findings,
        );
    }
    findings.check(
        "Modified within validation",
        age_validation(&mut app, context, &window),
    );

    // Identical files.
    findings.check(
        "Identical files",
        identical(&mut app, context, &window, &root),
    );
    find::close(&mut app, &window);
    context.note(format!(
        "{checked} searches matched their known answers exactly: masks (;;, extensionless, exclusions, an excluded folder still searched, Unicode case), regex ((?i) against Match name case), expressions (kB/KiB/B, =~, not, kind, offset times at the bound, present/missing), content (UTF-8 ± BOM, UTF-16 LE/BE, Unicode case, whole words at combining marks and underscores, CRLF and per-line regex), Modified within in minutes, hours and days"
    ));
    app.shutdown()?;
    findings.into_result()
}

/// Two malformed UTF-8 files, one of them after a match: both are errors, neither is
/// listed, and the search is Incomplete with both named in the details.
fn malformed(app: &mut App, window: &Tool, root: &Path) -> Result<(), String> {
    let (text, rows) = search(
        app,
        window,
        Query {
            masks: "*.enc",
            content: Some("quick"),
            ..BASE
        },
    )?;
    if !text.starts_with("Incomplete") || !rows.is_empty() {
        return Err(format!(
            "malformed files ended {text:?} with {:?}",
            relative(&rows, root)
        ));
    }
    let texts = find::details(app, window)?;
    for needle in [
        "2 errors",
        "unsupported or malformed encoding",
        "latin1.enc",
        "trailing.enc",
    ] {
        if !mentions(&texts, needle) {
            return Err(format!("the details do not mention {needle:?}: {texts:?}"));
        }
    }
    Ok(())
}

/// Invalid Modified within values disable Find and say why; the collapsed More options
/// line summarizes the options.
fn age_validation(app: &mut App, context: &mut Context, window: &Tool) -> Result<(), String> {
    let w = window.window();
    fill(
        app,
        window,
        Query {
            masks: "*.age",
            modified: Some(("1", "Days")),
            ..BASE
        },
    )?;
    for (number, unit) in [
        ("0", "Days"),
        ("1.5", "Days"),
        ("abc", "Days"),
        ("-5", "Days"),
        ("367", "Days"),
        ("8785", "Hours"),
        ("527041", "Minutes"),
    ] {
        input::fill(w, "Modified age", number)?;
        tools::choose_combo(app, w, "Modified age unit", &find::UNITS, unit)?;
        app.settle();
        let shown = inspect_texts(window);
        if !shown.iter().any(|text| text == INVALID_AGE) {
            return Err(format!("{number} {unit} shows no error: {shown:?}"));
        }
        if find::button_enabled(window, "Find")? {
            return Err(format!("Find stays enabled for {number} {unit}"));
        }
        // Enter in the name field does not start a search either.
        let before = status(window);
        input::click(w, AccessibleRole::TextInput, "File name masks")?;
        input::key(w, Key::Return);
        app.settle();
        if status(window) != before {
            return Err(format!(
                "Enter started a search with {number} {unit}: {:?}",
                status(window)
            ));
        }
    }
    app.screenshot_window(
        context,
        window,
        "age-invalid",
        "Modified within 527041 minutes: more than 366 days",
    );
    // The collapsed options line says why.
    input::click_button(w, "Fewer options")?;
    app.settle();
    if !inspect_texts(window)
        .iter()
        .any(|text| text == "Modified interval needs a positive whole number")
    {
        return Err(format!(
            "the collapsed summary of an invalid age: {:?}",
            inspect_texts(window)
        ));
    }
    find::more_options(app, window)?;
    input::fill(w, "Modified age", "527040")?;
    app.settle();
    if inspect_texts(window).iter().any(|text| text == INVALID_AGE)
        || !find::button_enabled(window, "Find")?
    {
        return Err("527040 minutes (366 days) is refused".into());
    }
    // Summaries: the age with Inside archives, then with Identical files, which
    // replaces (and disables) Search inside archives.
    input::fill(w, "Modified age", "2")?;
    tools::choose_combo(app, w, "Modified age unit", &find::UNITS, "Hours")?;
    tools::set_check(app, w, "Search inside archives", true)?;
    input::click_button(w, "Fewer options")?;
    app.settle();
    let summary = |window: &Tool| inspect_texts(window);
    if !summary(window)
        .iter()
        .any(|text| text == "Modified: last 2 hours · Inside archives")
    {
        return Err(format!("the summary with archives: {:?}", summary(window)));
    }
    find::more_options(app, window)?;
    tools::set_check(app, w, "Identical files", true)?;
    if find::check_enabled(window, "Search inside archives")? {
        return Err("Search inside archives stays enabled with Identical files".into());
    }
    if !inspect_texts(window)
        .iter()
        .any(|text| text == "Files only · identical contents · grouped results")
    {
        return Err("Identical files shows no description".into());
    }
    input::click_button(w, "Fewer options")?;
    app.settle();
    if !summary(window)
        .iter()
        .any(|text| text == "Modified: last 2 hours · Identical files")
    {
        return Err(format!(
            "the summary with Identical files: {:?}",
            summary(window)
        ));
    }
    app.screenshot_window(
        context,
        window,
        "options-summary",
        "Fewer options: the summary of Modified within and Identical files",
    );
    find::more_options(app, window)?;
    tools::set_check(app, w, "Identical files", false)?;
    tools::set_check(app, w, "Search inside archives", false)?;
    tools::set_check(app, w, "Modified within", false)?;
    Ok(())
}

fn inspect_texts(window: &Tool) -> Vec<String> {
    crate::inspect::visible_texts(window.window())
}

/// Identical files: three groups in a known order, hard links counted once, groups
/// kept together under Name and Size in both directions.
fn identical(
    app: &mut App,
    context: &mut Context,
    window: &Tool,
    root: &Path,
) -> Result<(), String> {
    // A fresh window: the default order (Name ascending).
    find::close(app, window);
    let window = &open_find(app)?;
    let query = Query {
        mode: "Expression",
        masks: "name =~ \"^[a-z]-\"",
        identical: true,
        ..BASE
    };
    fill(app, window, query)?;
    let text = run(app, window)?;
    let rows = all_rows(app, window)?;
    if !text.starts_with("Completed")
        || tools::count_before(&text, " groups") != Some(3)
        || tools::count_before(&text, " candidates") != Some(10)
    {
        return Err(format!(
            "the status of three groups among 10 candidates reads {text:?}"
        ));
    }
    let g1 = "Group 1 · 2 files";
    let g2 = "Group 2 · 3 files";
    let g3 = "Group 3 · 2 files";
    let expect_order = |rows: &[Row], wanted: &[(&str, &str)], what: &str| {
        let shown: Vec<(String, String)> = ordered(rows, root)
            .into_iter()
            .zip(rows)
            .map(|(path, row)| (row.group.clone().unwrap_or_default(), path))
            .collect();
        let wanted: Vec<(String, String)> = wanted
            .iter()
            .map(|(group, path)| ((*group).to_owned(), format!("dupes/{path}")))
            .collect();
        if shown != wanted {
            return Err(format!("{what}: {shown:?}, expected {wanted:?}"));
        }
        Ok(())
    };
    let ascending = [
        (g2, "a-hundred.dat"),
        (g2, "sub/m-hundred.bin"),
        (g2, "z-hundred.txt"),
        (g3, "b-binary.bin"),
        (g3, "y-binary.bin"),
        (g1, "e-empty"),
        (g1, "x-empty.txt"),
    ];
    let deadline = std::time::Instant::now() + Duration::from_secs(20);
    let mut rows = rows;
    while let Err(error) = expect_order(&rows, &ascending, "Name ascending") {
        if std::time::Instant::now() >= deadline {
            return Err(error);
        }
        app.idle(Duration::from_millis(50));
        rows = all_rows(app, window)?;
    }
    app.screenshot_window(
        context,
        window,
        "identical-groups",
        "Identical files: three groups, the hard-link pair and the different file not listed",
    );
    let texts = find::details(app, window)?;
    if !mentions(&texts, "Identical files: 3 verified groups · 10 candidates") {
        return Err(format!("the details of the identical files: {texts:?}"));
    }
    for (header, wanted, what) in [
        (
            "Sort by Name",
            [
                (g2, "z-hundred.txt"),
                (g2, "sub/m-hundred.bin"),
                (g2, "a-hundred.dat"),
                (g3, "y-binary.bin"),
                (g3, "b-binary.bin"),
                (g1, "x-empty.txt"),
                (g1, "e-empty"),
            ],
            "Name descending",
        ),
        (
            "Sort by Size",
            [
                (g1, "e-empty"),
                (g1, "x-empty.txt"),
                (g2, "a-hundred.dat"),
                (g2, "sub/m-hundred.bin"),
                (g2, "z-hundred.txt"),
                (g3, "b-binary.bin"),
                (g3, "y-binary.bin"),
            ],
            "Size ascending",
        ),
        (
            "Sort by Size",
            [
                (g3, "b-binary.bin"),
                (g3, "y-binary.bin"),
                (g2, "a-hundred.dat"),
                (g2, "sub/m-hundred.bin"),
                (g2, "z-hundred.txt"),
                (g1, "e-empty"),
                (g1, "x-empty.txt"),
            ],
            "Size descending (equal sizes by path)",
        ),
    ] {
        input::click_button(window.window(), header)?;
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        loop {
            let rows = all_rows(app, window)?;
            match expect_order(&rows, &wanted, what) {
                Ok(()) => break,
                Err(error) if std::time::Instant::now() >= deadline => return Err(error),
                Err(_) => app.idle(Duration::from_millis(50)),
            }
        }
    }
    find::close(app, window);
    Ok(())
}
