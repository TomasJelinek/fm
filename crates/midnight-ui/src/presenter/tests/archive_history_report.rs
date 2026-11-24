//! The reported cases: Back from an archive's root, Back into an archive after leaving it,
//! and a Hot Path inside an archive inside an archive.
use super::*;

fn last_navigate(sent: &RefCell<Vec<Command>>) -> Option<(NavigationId, PathBuf)> {
    sent.borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Navigate {
                navigation, path, ..
            } => Some((*navigation, path.clone())),
            _ => None,
        })
}

#[test]
fn report_back_from_the_archive_root_returns_to_the_native_folder() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    assert!(
        p.history(PaneId::Left, -1, &send),
        "Back from the archive's root went nowhere"
    );
    assert_eq!(
        last_navigate(&sent).map(|(_, path)| path),
        Some(test_root())
    );
}

#[test]
fn report_back_after_leaving_an_archive_opens_it_again() {
    let sent = RefCell::new(Vec::new());
    let send = |command| {
        sent.borrow_mut().push(command);
        true
    };
    let mut p = outer_pane(&sent);
    assert!(p.up(PaneId::Left, &send));
    let (navigation, _) = last_navigate(&sent).unwrap();
    finish(&mut p, PaneId::Left, navigation, vec![]);
    let before = sent.borrow().len();
    p.history(PaneId::Left, -1, &send);
    assert!(
        sent.borrow()[before..].iter().any(|command| matches!(
            command,
            Command::ArchiveNavigate {
                action: ArchiveAction::List(location),
                ..
            } if location.address.container == test_root().join("sample.zip")
        )),
        "Back after leaving the archive did not open it again"
    );
}

#[test]
fn report_a_hot_path_inside_an_archive_inside_an_archive_keeps_the_whole_path() {
    let sent = RefCell::new(Vec::new());
    let mut p = outer_pane(&sent);
    enter_inner(&mut p, &sent, 43);
    let bookmark = p
        .capture_bookmark_location(PaneId::Left, "inside".into())
        .unwrap();
    let separator = std::path::MAIN_SEPARATOR;
    assert!(
        bookmark
            .display_text()
            .ends_with(&format!("sample.zip{separator}inner.zip")),
        "the Hot Path is shown as {:?}",
        bookmark.display_text()
    );
}
