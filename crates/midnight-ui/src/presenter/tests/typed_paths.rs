//! A path typed or pasted in the path bar that names a file lists the file's folder
//! with the cursor on the file (`path_target`).
use super::*;

const NAME: &str = "ünï 文档 🧪 e\u{301}.txt";

/// A Unicode folder below the test root and a file in it.
fn fixture() -> (PathBuf, PathBuf) {
    let folder = test_root().join("Příliš 资料");
    let file = folder.join(NAME);
    (folder, file)
}

pub(super) fn file_entry(id: u64, path: PathBuf) -> Entry {
    Entry::new(EntryId(id), path, EntryKind::File)
}

fn text(path: &Path) -> &str {
    path.to_str().unwrap()
}

/// What `call` returns and the commands it sends, all of which are accepted.
fn sending<T>(call: impl FnOnce(&dyn Fn(Command) -> bool) -> T) -> (T, Vec<Command>) {
    let sent = RefCell::new(Vec::new());
    let value = call(&|command| {
        sent.borrow_mut().push(command);
        true
    });
    (value, sent.into_inner())
}

/// The navigation that `commands` request for `path`.
fn listing_of(commands: &[Command], path: &Path) -> Option<NavigationId> {
    commands.iter().find_map(|command| match command {
        Command::Navigate {
            navigation,
            path: requested,
            ..
        }
        | Command::Refresh {
            navigation,
            path: requested,
            ..
        } if requested == path => Some(*navigation),
        _ => None,
    })
}

fn not_a_folder(navigation: NavigationId, entry: &Path, hidden: bool) -> Event {
    Event::ListingNotAFolder {
        pane: PaneId::Left,
        navigation,
        parent: entry.parent().unwrap().to_owned(),
        entry: entry.to_owned(),
        hidden,
    }
}

/// The failure that the backend sends after `not_a_folder`.
fn fallback(navigation: NavigationId, entry: &Path) -> Event {
    Event::ListingFallback {
        pane: PaneId::Left,
        navigation,
        path: entry.parent().unwrap().to_owned(),
        error: FsError::from_io("read directory", entry, io::ErrorKind::NotADirectory.into()),
    }
}

/// Type `typed` in the left path bar: the request for `path` it sends.
fn type_path(p: &mut Presenter, typed: &str, path: &Path) -> NavigationId {
    let (accepted, sent) = sending(|send| p.navigate_text(PaneId::Left, typed, send));
    assert!(accepted, "{typed}");
    listing_of(&sent, path).unwrap_or_else(|| panic!("{typed}: {sent:?}"))
}

pub(super) fn note(p: &Presenter) -> Option<panel_details::PanelDetail> {
    p.panel_details(PaneId::Left)
        .into_iter()
        .find(|detail| detail.label == "Not shown")
}

/// Type the path of the file `typed` in the left path bar and answer that it names a
/// file: the listing of its folder that the panel then requests.
pub(super) fn redirected(p: &mut Presenter, typed: &Path) -> NavigationId {
    let navigation = type_path(p, text(typed), typed);
    let ((), sent) =
        sending(|send| p.apply_with_sender(not_a_folder(navigation, typed, false), send));
    listing_of(&sent, typed.parent().unwrap()).expect("the file's folder is listed")
}

#[test]
fn pasted_quotes_and_whitespace_are_not_part_of_a_path() {
    let separator = std::path::MAIN_SEPARATOR;
    for (typed, path) in [
        ("  plain 文档.txt\t".to_owned(), "plain 文档.txt".to_owned()),
        ("\"quoted 文档.txt\"".into(), "quoted 文档.txt".into()),
        (" \"pasted 文档.txt\" ".into(), "pasted 文档.txt".into()),
        // A separator after the closing quote is the user's own text: it stays.
        (
            format!("\"pasted 文档.txt\"{separator}"),
            format!("\"pasted 文档.txt\"{separator}"),
        ),
        (
            format!("\"folder{separator}\""),
            format!("folder{separator}"),
        ),
        ("\"\"twice\"\"".into(), "\"twice\"".into()),
        ("\"unbalanced".into(), "\"unbalanced".into()),
        ("\" kept inside \"".into(), " kept inside ".into()),
        (
            "middle \"quotes\" stay".into(),
            "middle \"quotes\" stay".into(),
        ),
        ("\"".into(), "\"".into()),
        (String::new(), String::new()),
    ] {
        assert_eq!(path_target::typed_path(&typed), path, "{typed:?}");
    }
}

#[test]
fn every_pasted_form_of_a_file_path_requests_that_path() {
    let (folder, file) = fixture();
    let path = text(&file);
    let separator = std::path::MAIN_SEPARATOR;
    #[allow(unused_mut)]
    let mut forms = vec![
        path.to_owned(),
        format!("  {path}\t"),
        format!("\"{path}\""),
        // The file's path with a final separator still names the file.
        format!("{path}{separator}"),
    ];
    #[cfg(windows)]
    forms.push(path.replace('\\', "/"));
    for form in forms {
        let mut p = Presenter::new(folder.join("elsewhere"), "right".into());
        type_path(&mut p, &form, &file);
        assert!(p.panes[0].pending.as_ref().unwrap().accepts_file, "{form}");
    }
}

#[test]
fn a_typed_file_path_lists_its_folder_with_the_cursor_on_the_file() {
    let (folder, file) = fixture();
    let start = test_root().join("start");
    let mut p = Presenter::new(start.clone(), "right".into());
    let first = begin(&mut p, PaneId::Left, text(&start));
    finish(
        &mut p,
        PaneId::Left,
        first,
        vec![file_entry(1, start.join("old.txt"))],
    );
    let typed = type_path(&mut p, &format!("\"{}\"", text(&file)), &file);
    let ((), sent) = sending(|send| p.apply_with_sender(not_a_folder(typed, &file, false), send));
    let listed = listing_of(&sent, &folder).expect("the file's folder is listed");
    assert!(listed > typed);
    // The failure that follows belongs to the superseded request.
    let ((), sent) = sending(|send| p.apply_with_sender(fallback(typed, &file), send));
    assert!(sent.is_empty(), "{sent:?}");
    finish(
        &mut p,
        PaneId::Left,
        listed,
        vec![
            file_entry(2, folder.join("a.txt")),
            file_entry(3, file.clone()),
            file_entry(4, folder.join("z.txt")),
        ],
    );
    let pane = &p.panes[0];
    assert_eq!(pane.path, folder);
    assert_eq!(pane.cursor, Some(EntryId(3)));
    assert_eq!(pane.cursor_index(), Some(2), "`..`, a.txt, then the file");
    assert!(pane.error.is_empty());
    assert!(p.desktop.notice.is_none(), "{:?}", p.desktop.notice);
    assert_eq!(pane.history, [start, folder]);
    let details = p.panel_details(PaneId::Left);
    assert_eq!(
        (details[0].label, details[0].value.as_str()),
        ("Name", NAME)
    );
}

#[test]
fn only_the_pending_path_bar_request_lists_the_folder_of_a_file() {
    let (folder, file) = fixture();
    let mut p = Presenter::new(folder.join("elsewhere"), "right".into());
    let typed = type_path(&mut p, text(&file), &file);
    let ((), sent) = sending(|send| p.apply_with_sender(not_a_folder(typed, &file, false), send));
    assert!(listing_of(&sent, &folder).is_some(), "{sent:?}");

    // A navigation that the path bar did not type (history, a bookmark) fails as before.
    let mut p = Presenter::new(folder.join("elsewhere"), "right".into());
    let plain = begin(&mut p, PaneId::Left, text(&file));
    let ((), sent) = sending(|send| p.apply_with_sender(not_a_folder(plain, &file, false), send));
    assert!(sent.is_empty(), "{sent:?}");
    let ((), sent) = sending(|send| p.apply_with_sender(fallback(plain, &file), send));
    assert!(listing_of(&sent, &folder).is_some(), "the former recovery");
    assert!(
        p.desktop
            .notice
            .as_ref()
            .is_some_and(|(title, _)| title == "Cannot open folder")
    );

    // An answer for a superseded request, or for another path, changes nothing.
    let mut p = Presenter::new(folder.join("elsewhere"), "right".into());
    let typed = type_path(&mut p, text(&file), &file);
    let other = folder.join("other.txt");
    let ((), sent) = sending(|send| p.apply_with_sender(not_a_folder(typed, &other, false), send));
    assert!(sent.is_empty(), "{sent:?}");
    let later = begin(&mut p, PaneId::Left, text(&test_root().join("later")));
    let ((), sent) = sending(|send| p.apply_with_sender(not_a_folder(typed, &file, false), send));
    assert!(sent.is_empty(), "{sent:?}");
    assert_eq!(p.panes[0].generation, later.0);

    // Without a sender the answer is only retired; the failure that follows decides.
    let mut p = Presenter::new(folder.join("elsewhere"), "right".into());
    let typed = type_path(&mut p, text(&file), &file);
    drop(p.take_retired());
    p.apply(not_a_folder(typed, &file, false));
    assert_eq!(p.take_retired().len(), 1);
    assert_eq!(p.panes[0].generation, typed.0);
    assert!(p.panes[0].loading);
}

#[test]
fn a_file_the_panel_lists_gets_the_cursor_without_a_navigation() {
    let (folder, file) = fixture();
    let sub = folder.join("sub 目录");
    let mut p = Presenter::new(folder.clone(), "right".into());
    let listed = begin(&mut p, PaneId::Left, text(&folder));
    finish(
        &mut p,
        PaneId::Left,
        listed,
        vec![
            Entry::new(EntryId(1), sub.clone(), EntryKind::Directory),
            file_entry(2, folder.join("a.txt")),
            file_entry(3, file.clone()),
        ],
    );
    assert_ne!(p.panes[0].cursor, Some(EntryId(3)));
    let quoted = format!("\"{}\"", text(&file));
    assert!(p.navigate_text(PaneId::Left, &quoted, &|command| panic!(
        "a listed file needs no navigation: {command:?}"
    )));
    assert_eq!(p.panes[0].cursor, Some(EntryId(3)));
    assert!(!p.panes[0].parent_cursor);
    assert!(!p.panes[0].loading);
    assert_eq!(p.panes[0].generation, listed.0);
    assert!(note(&p).is_none());
    // A name relative to the shown folder is found the same way.
    assert!(p.navigate_text(PaneId::Left, "a.txt", &|command| panic!(
        "a listed file needs no navigation: {command:?}"
    )));
    assert_eq!(p.panes[0].cursor, Some(EntryId(2)));
    // An automatic re-read keeps the listed rows, and the cursor stays on the file.
    assert!(p.refresh_if_changed(PaneId::Left, listed, &|_| true));
    let reread = NavigationId(p.panes[0].generation);
    assert!(
        p.navigate_text(PaneId::Left, text(&file), &|command| panic!(
            "a listed file needs no navigation: {command:?}"
        ))
    );
    finish(
        &mut p,
        PaneId::Left,
        reread,
        vec![
            Entry::new(EntryId(11), sub.clone(), EntryKind::Directory),
            file_entry(12, folder.join("a.txt")),
            file_entry(13, file.clone()),
        ],
    );
    assert_eq!(p.panes[0].cursor, Some(EntryId(13)));
    // A listed folder still opens.
    type_path(&mut p, text(&sub), &sub);

    // A quick search is left, as by any navigation: the file is listed again.
    let mut p = Presenter::new(folder.clone(), "right".into());
    let listed = begin(&mut p, PaneId::Left, text(&folder));
    let entries = vec![
        file_entry(2, folder.join("a.txt")),
        file_entry(3, file.clone()),
    ];
    finish(&mut p, PaneId::Left, listed, entries.clone());
    p.panes[0].search.query = "a".into();
    let typed = type_path(&mut p, text(&file), &file);
    assert!(p.panes[0].search.query.is_empty());
    let ((), sent) = sending(|send| p.apply_with_sender(not_a_folder(typed, &file, false), send));
    let again = listing_of(&sent, &folder).expect("the shown folder is listed again");
    finish(&mut p, PaneId::Left, again, entries);
    assert_eq!(p.panes[0].cursor, Some(EntryId(3)));
}

#[test]
fn a_hidden_or_vanished_file_says_why_in_the_status_line() {
    let (folder, file) = fixture();
    for (hidden, value, explanation) in [
        (true, "is hidden", "Hidden files are not shown"),
        (false, "is not listed here", "no file of exactly this name"),
    ] {
        let mut p = Presenter::new(test_root().join("start"), "right".into());
        p.preferences.values.show_hidden_files = false;
        let typed = type_path(&mut p, text(&file), &file);
        let ((), sent) =
            sending(|send| p.apply_with_sender(not_a_folder(typed, &file, hidden), send));
        let listed = listing_of(&sent, &folder).unwrap();
        assert!(note(&p).is_none(), "nothing is said while the folder loads");
        finish(
            &mut p,
            PaneId::Left,
            listed,
            vec![file_entry(2, folder.join("a.txt"))],
        );
        assert_eq!(p.panes[0].path, folder);
        assert!(p.panes[0].parent_cursor);
        let details = p.panel_details(PaneId::Left);
        assert_eq!(details[0].label, "Not shown", "the note leads");
        assert_eq!(details[0].value, format!("{NAME} {value}"));
        assert!(
            details[0].explanation.contains(explanation),
            "{}",
            details[0].explanation
        );
        // The next listing of the folder no longer says it.
        let again = begin(&mut p, PaneId::Left, text(&folder));
        finish(
            &mut p,
            PaneId::Left,
            again,
            vec![file_entry(5, folder.join("a.txt"))],
        );
        assert!(note(&p).is_none());
    }
}

#[test]
fn a_file_the_panel_filter_leaves_out_says_so_and_keeps_the_cursor_on_a_shown_row() {
    let (folder, file) = fixture();
    let mut p = Presenter::new(folder.clone(), "right".into());
    p.preferences.values.panel_filters[0].enabled = true;
    let listed = begin(&mut p, PaneId::Left, text(&folder));
    finish(
        &mut p,
        PaneId::Left,
        listed,
        vec![
            file_entry(1, folder.join("a.txt")),
            file_entry(2, file.clone()),
        ],
    );
    // The filter shows a.txt only.
    let source = p.panes[0].entries.clone();
    let projection = Arc::new(DirectoryIndex::from_entries(&[source[0].clone()]));
    let matches: Arc<[SearchMatch]> = vec![SearchMatch {
        source_row: 0,
        display_prefix_end: 0,
    }]
    .into();
    p.apply(Event::SearchReady {
        pane: PaneId::Left,
        navigation: listed,
        revision: p.panes[0].search.revision,
        result: Arc::new(SearchResult {
            source,
            matches: matches.clone(),
            filtered: matches,
            filtered_index: projection.clone(),
            index: projection,
        }),
    });
    assert_eq!(p.panes[0].cursor, Some(EntryId(1)));
    assert!(
        p.navigate_text(PaneId::Left, text(&file), &|command| panic!(
            "a listed file needs no navigation: {command:?}"
        ))
    );
    assert_eq!(
        p.panes[0].cursor,
        Some(EntryId(1)),
        "the cursor stays shown"
    );
    let note = note(&p).expect("the status line says why the file is not shown");
    assert_eq!(note.value, format!("{NAME} is filtered out"));
    assert!(
        note.explanation.contains("panel filter"),
        "{}",
        note.explanation
    );
}
