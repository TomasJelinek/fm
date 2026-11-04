//! Explicit worker-result fixtures for tests of the installed UI bridge.
use fm_core::*;
use fm_ui::Presenter;
use std::{cell::RefCell, path::Path, rc::Rc, sync::Arc};

pub fn acknowledge(
    presenter: &Rc<RefCell<Presenter>>,
    commands: &RefCell<Vec<Command>>,
    pane: PaneId,
    expected_paths: &[&Path],
) -> SelectionRequest {
    let request = commands
        .borrow()
        .iter()
        .rev()
        .find_map(|command| match command {
            Command::Selection(request) if request.pane == pane => Some(request.clone()),
            _ => None,
        })
        .expect("the UI must request a backend selection change");
    let index = request
        .view
        .clone()
        .expect("fixture has a complete listing");
    let mut bits = vec![0; index.by_id.len().div_ceil(64)];
    for path in expected_paths {
        let position = index.by_id[&index.by_path[*path]];
        bits[position / 64] |= 1 << (position % 64);
    }
    commands
        .borrow_mut()
        .retain(|command| !matches!(command, Command::Selection(value) if value.pane == pane));
    presenter.borrow_mut().apply(Event::SelectionChanged {
        pane,
        navigation: request.navigation,
        revision: request.revision,
        selection: Arc::new(SelectionSnapshot::from_bits(index, bits).unwrap()),
        deferred: false,
    });
    request
}
