//! Notify only visible rows whose retained thumbnail frame changed.
use crate::*;

/// Release mutable presenter borrows before calling: model listeners can read rows.
pub fn refresh_thumbnail_rows(window: &MainWindow, presenter: &Presenter, changed: &[String]) {
    if changed
        .iter()
        .any(|identity| identity.starts_with("command-icon:"))
    {
        user_command_presentation::refresh(window, presenter);
    }
    for (side, model, first, end) in [
        (
            PaneId::Left,
            window.get_left_rows(),
            window.get_left_visible_row_start(),
            window.get_left_visible_row_end(),
        ),
        (
            PaneId::Right,
            window.get_right_rows(),
            window.get_right_visible_row_start(),
            window.get_right_visible_row_end(),
        ),
    ] {
        if let Some(rows) = model.as_any().downcast_ref::<Rows>() {
            refresh_rows(
                rows,
                &presenter.panes[side.index()],
                first.max(0) as usize,
                end.max(0) as usize,
                changed,
            );
        }
    }
}

fn refresh_rows(rows: &Rows, pane: &presenter::Pane, first: usize, end: usize, changed: &[String]) {
    if changed.is_empty() || rows.structure.get() != RowStructure::from_pane(pane) {
        return;
    }
    let length = pane.display_len() + pane.parent_offset();
    for row in first.min(length)..end.min(length) {
        let Some(entry) = row
            .checked_sub(pane.parent_offset())
            .and_then(|index| pane.display_entry(index))
        else {
            continue;
        };
        let identity = thumbnail_presentation::identity(pane, entry, rows.pane);
        if changed.contains(&identity) {
            rows.notify.row_changed(row);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use i_slint_core::model::{ModelChangeListener, ModelChangeListenerBox};
    use std::{pin::Pin, sync::Arc};

    #[derive(Default)]
    struct Changes {
        rows: RefCell<Vec<usize>>,
        resets: Cell<usize>,
    }
    impl ModelChangeListener for Changes {
        fn row_changed(self: Pin<&Self>, row: usize) {
            self.rows.borrow_mut().push(row);
        }
        fn row_added(self: Pin<&Self>, _: usize, _: usize) {
            panic!("thumbnail inserted rows");
        }
        fn row_removed(self: Pin<&Self>, _: usize, _: usize) {
            panic!("thumbnail removed rows");
        }
        fn reset(self: Pin<&Self>) {
            self.resets.set(self.resets.get() + 1);
        }
    }

    #[test]
    fn completed_frame_notifies_only_its_visible_row_and_stale_frame_notifies_nothing() {
        let root = std::env::current_dir().unwrap();
        let mut p = Presenter::new(root.clone(), root.clone());
        p.preferences.values.thumbnails_network = true;
        p.preferences.values.panel_view_modes = [PanelViewMode::Thumbnails; 2];
        let entries: Arc<[Entry]> = (0..4)
            .map(|i| Entry::new(EntryId(i), root.join(format!("{i}.png")), EntryKind::File))
            .collect();
        p.panes[0].index = Arc::new(DirectoryIndex::from_entries(&entries));
        p.panes[0].entries = entries;
        let offset = p.panes[0].parent_offset();
        let targets = p.thumbnail_targets([(offset, offset + 4), (0, 0)], ["", ""]);
        let presenter = Rc::new(RefCell::new(p));
        let rows = Rows::new(presenter.clone(), PaneId::Left);
        let changes = ModelChangeListenerBox::new(Changes::default());
        rows.model_tracker()
            .attach_peer(changes.as_ref().model_peer());

        assert!(
            presenter
                .borrow_mut()
                .receive_thumbnail(&targets[1].identity, 0, 2, 1, &[255; 8])
        );
        refresh_rows(
            &rows,
            &presenter.borrow().panes[0],
            0,
            offset + 4,
            std::slice::from_ref(&targets[1].identity),
        );
        assert_eq!(*changes.rows.borrow(), [offset + 1]);
        assert_eq!(changes.resets.get(), 0);
        assert_eq!(rows.row_data(offset + 1).unwrap().icon.size().width, 2);

        changes.rows.borrow_mut().clear();
        refresh_rows(&rows, &presenter.borrow().panes[0], 0, offset + 4, &[]);
        assert!(
            changes.rows.borrow().is_empty(),
            "viewport scheduling alone must not repaint rows"
        );
        refresh_rows(
            &rows,
            &presenter.borrow().panes[0],
            offset + 2,
            offset + 4,
            std::slice::from_ref(&targets[1].identity),
        );
        assert!(
            changes.rows.borrow().is_empty(),
            "offscreen image is retained without repainting another row"
        );
        presenter.borrow_mut().panes[0].generation += 1;
        refresh_rows(
            &rows,
            &presenter.borrow().panes[0],
            0,
            offset + 4,
            std::slice::from_ref(&targets[1].identity),
        );
        assert!(changes.rows.borrow().is_empty());
        assert_eq!(
            changes.resets.get(),
            0,
            "a thumbnail completion cannot reset listing identities"
        );
    }
}
