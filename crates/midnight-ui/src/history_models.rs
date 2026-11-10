//! History models retain identities and allocate labels only for requested rows.
use crate::*;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Source {
    Navigation(PaneId, bool),
    Commands,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Revision {
    generation: u64,
    pointer: usize,
    length: usize,
    current: Option<usize>,
}

struct HistoryRows {
    presenter: Rc<RefCell<Presenter>>,
    source: Source,
    revision: Cell<Option<Revision>>,
    icon: RefCell<Option<slint::Image>>,
    notify: ModelNotify,
}

impl HistoryRows {
    fn new(presenter: Rc<RefCell<Presenter>>, source: Source) -> Self {
        Self {
            presenter,
            source,
            revision: Cell::new(None),
            icon: RefCell::default(),
            notify: ModelNotify::default(),
        }
    }

    fn refresh(&self, presenter: &Presenter) {
        let (revision, icon) = match self.source {
            Source::Navigation(pane, _) => {
                let pane = &presenter.panes[pane.index()];
                (
                    Revision {
                        generation: pane.generation,
                        pointer: pane.history.as_ptr() as usize,
                        length: pane.history.len(),
                        current: pane.history_index,
                    },
                    presenter.desktop.image(&IconKey::folder()),
                )
            }
            Source::Commands => {
                let preferences = &presenter.preferences;
                (
                    Revision {
                        generation: preferences.revision.0,
                        pointer: preferences.values.command_history.as_ptr() as usize,
                        length: preferences.values.command_history.len(),
                        current: None,
                    },
                    presenter
                        .desktop
                        .image(&IconKey::action(ActionIcon::Terminal)),
                )
            }
        };
        // Slint 1.18.1 considers two empty Images unequal. The desktop cache
        // validates nonzero dimensions, so represent only absent artwork as None.
        let icon = (icon.size().width != 0).then_some(icon);
        let changed = self.revision.replace(Some(revision)) != Some(revision);
        let icon_changed = *self.icon.borrow() != icon;
        *self.icon.borrow_mut() = icon;
        // Notifications can synchronously read rows. Release all mutable borrows first.
        if changed {
            self.notify.reset();
        } else if icon_changed {
            for row in 0..self.row_count() {
                self.notify.row_changed(row);
            }
        }
    }
}

impl Model for HistoryRows {
    type Data = NavigationChoiceRow;

    fn row_count(&self) -> usize {
        let Some(revision) = self.revision.get() else {
            return 0;
        };
        match self.source {
            Source::Commands => revision.length,
            Source::Navigation(_, forward) => match revision.current {
                Some(current) if current < revision.length => {
                    if forward {
                        revision.length - current - 1
                    } else {
                        current
                    }
                }
                _ => 0,
            },
        }
    }

    fn row_data(&self, row: usize) -> Option<Self::Data> {
        if row >= self.row_count() {
            return None;
        }
        let presenter = self.presenter.borrow();
        let (identity, label) = match self.source {
            Source::Navigation(pane, forward) => {
                let choice = presenter.history_entry(pane, forward, row)?;
                (
                    choice.identity.into(),
                    SharedString::from(choice.path.to_string_lossy().as_ref()),
                )
            }
            Source::Commands => {
                let entry = presenter.preferences.values.command_history.get(row)?;
                let shell = match entry.shell {
                    ShellFlavor::PowerShell => "PowerShell",
                    ShellFlavor::Cmd => "cmd",
                    ShellFlavor::Posix => "Terminal",
                };
                (
                    entry.command.as_str().into(),
                    format!("{shell} · {}", entry.command).into(),
                )
            }
        };
        Some(NavigationChoiceRow {
            identity,
            compact_label: label.clone(),
            tooltip: label.clone(),
            label,
            icon: self.icon.borrow().clone().unwrap_or_default(),
            kind: LocationRole::Home,
        })
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        &self.notify
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

fn update(
    current: ModelRc<NavigationChoiceRow>,
    shared: &Rc<RefCell<Presenter>>,
    presenter: &Presenter,
    source: Source,
    set: impl FnOnce(ModelRc<NavigationChoiceRow>),
) {
    if let Some(model) = current.as_any().downcast_ref::<HistoryRows>()
        && model.source == source
        && Rc::ptr_eq(&model.presenter, shared)
    {
        model.refresh(presenter);
    } else {
        let model = HistoryRows::new(shared.clone(), source);
        model.refresh(presenter);
        set(ModelRc::new(model));
    }
}

pub(super) fn refresh(window: &MainWindow, presenter: &Presenter) {
    let rows = window.get_left_rows();
    let Some(rows) = rows.as_any().downcast_ref::<Rows>() else {
        return;
    };
    let shared = &rows.presenter;
    update(
        window.get_command_history(),
        shared,
        presenter,
        Source::Commands,
        |model| window.set_command_history(model),
    );
    update(
        window.get_left_back_history(),
        shared,
        presenter,
        Source::Navigation(PaneId::Left, false),
        |model| window.set_left_back_history(model),
    );
    update(
        window.get_left_forward_history(),
        shared,
        presenter,
        Source::Navigation(PaneId::Left, true),
        |model| window.set_left_forward_history(model),
    );
    update(
        window.get_right_back_history(),
        shared,
        presenter,
        Source::Navigation(PaneId::Right, false),
        |model| window.set_right_back_history(model),
    );
    update(
        window.get_right_forward_history(),
        shared,
        presenter,
        Source::Navigation(PaneId::Right, true),
        |model| window.set_right_forward_history(model),
    );
}
