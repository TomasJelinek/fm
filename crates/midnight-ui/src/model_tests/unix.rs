//! Unix native timestamps can exceed Chrono's calendar range.
use super::*;
use std::time::{Duration, UNIX_EPOCH};

#[test]
fn extreme_native_modified_times_render_unknown_without_changing_raw_sort_values() {
    let root = PathBuf::from("/");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root.clone())));
    let model = Rows::new(presenter.clone(), PaneId::Left);
    let distance = Duration::from_secs(10_000_000_000_000);
    for time in [
        UNIX_EPOCH.checked_add(distance).unwrap(),
        UNIX_EPOCH.checked_sub(distance).unwrap(),
    ] {
        let mut entry = Entry::new(EntryId(1), root.join("extreme date"), EntryKind::File);
        entry.metadata.modified = Some(time);
        presenter.borrow_mut().panes[0].entries = vec![entry].into();
        assert_eq!(model.row_data(0).unwrap().modified.as_str(), "—");
        assert_eq!(
            presenter.borrow().panes[0].entries[0].metadata.modified,
            Some(time)
        );
    }
}
