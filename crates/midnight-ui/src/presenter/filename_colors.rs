//! Snapshot-bound color results and revision-tagged backend validation.
use super::*;

#[derive(Default)]
pub(crate) struct ColorState {
    rules: Arc<[FilenameColorRule]>,
    inputs: [Option<FilenameColorsRequest>; 2],
    results: [Option<Arc<FilenameColorsResult>>; 2],
    next: u64,
    pub(crate) validation: Option<u64>,
    pub(crate) error: String,
    ready: Option<Vec<FilenameColorRule>>,
    pub(crate) editor: crate::color_options::Editor,
}

impl Presenter {
    pub(crate) fn cancel_color_draft(&mut self) {
        self.colors.validation = None;
        self.retired.push(Box::new(self.colors.ready.take()));
        self.colors.editor = Default::default();
        self.colors.error.clear();
    }
    pub(crate) fn filename_style(&self, pane: PaneId, entry: &Entry) -> Option<FilenameStyle> {
        let panel = &self.panes[pane.index()];
        let result = self.colors.results[pane.index()].as_ref()?;
        if panel.showing_stream()
            || !panel_snapshot(&result.request, pane, panel, &self.colors.rules)
        {
            return None;
        }
        result.styles.get(&entry.id).copied()
    }

    pub(crate) fn validate_colors(
        &mut self,
        rules: Vec<FilenameColorRule>,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.colors.validation.is_some() {
            return;
        }
        self.colors.error.clear();
        if let Err(error) = validate_filename_colors(&rules) {
            self.colors.error = error.into();
            return;
        }
        let Some(id) = self.colors.next.checked_add(1) else {
            return;
        };
        if send(Command::ValidateFilenameColors(FilenameColorsValidation {
            id,
            rules,
        })) {
            self.colors.next = id;
            self.colors.validation = Some(id);
        } else {
            self.colors.error = "The command queue is busy. Try Apply again.".into();
        }
    }

    /// Retry bounded worker admission and publish successfully validated drafts.
    pub fn flush_filename_colors(&mut self, send: &dyn Fn(Command) -> bool) {
        if let Some(rules) = self.colors.ready.take() {
            let mut values = self.preferences.values.clone();
            values.filename_colors = rules.into();
            self.preferences.edited.filename_colors = true;
            self.update_preferences(values, send);
            self.colors.editor.dirty = false;
        }
        if self.colors.rules.as_ref() != self.preferences.values.filename_colors.as_slice() {
            let rules = self.preferences.values.filename_colors.as_slice().into();
            self.retired
                .push(Box::new(mem::replace(&mut self.colors.rules, rules)));
            for result in &mut self.colors.results {
                self.retired.push(Box::new(result.take()));
            }
        }
        for pane in [PaneId::Left, PaneId::Right] {
            let panel = &self.panes[pane.index()];
            if !self.preferences.loaded
                || panel.entries.is_empty()
                || panel.showing_stream()
                || self.colors.rules.is_empty()
                || !tree_current(pane, panel)
            {
                if self.colors.inputs[pane.index()].is_some() {
                    let Some(id) = self.colors.next.checked_add(1) else {
                        continue;
                    };
                    if send(Command::ClearFilenameColors { pane, id }) {
                        self.colors.next = id;
                        self.retired
                            .push(Box::new(self.colors.inputs[pane.index()].take()));
                        self.retired
                            .push(Box::new(self.colors.results[pane.index()].take()));
                    }
                }
                continue;
            }
            if self.colors.inputs[pane.index()]
                .as_ref()
                .is_some_and(|request| panel_snapshot(request, pane, panel, &self.colors.rules))
            {
                continue;
            }
            let Some(id) = self.colors.next.checked_add(1) else {
                continue;
            };
            let request = FilenameColorsRequest {
                id,
                pane,
                source: panel.entries.clone(),
                rules: self.colors.rules.clone(),
                tree: panel.tree_view().cloned(),
            };
            if send(Command::MatchFilenameColors(request.clone())) {
                self.colors.next = id;
                self.retired
                    .push(Box::new(self.colors.inputs[pane.index()].replace(request)));
                self.retired
                    .push(Box::new(self.colors.results[pane.index()].take()));
            }
        }
    }

    pub(super) fn apply_colors_event(&mut self, event: Event) -> Option<Event> {
        match event {
            Event::FilenameColorsValidated { request, result } => {
                if self.colors.validation == Some(request.id) {
                    self.colors.validation = None;
                    match result {
                        Ok(()) => self.colors.ready = Some(request.rules),
                        Err(error) => self.colors.error = error,
                    }
                }
                None
            }
            Event::FilenameColorsReady { request, result } => {
                let slot = request.pane.index();
                let current = self.colors.inputs[slot].as_ref().is_some_and(|current| {
                    current.id == request.id
                        && same_snapshot(current, &request)
                        && panel_snapshot(
                            current,
                            request.pane,
                            &self.panes[slot],
                            &self.colors.rules,
                        )
                });
                match result {
                    Ok(result)
                        if current
                            && result.request.id == request.id
                            && same_snapshot(&result.request, &request) =>
                    {
                        self.retired
                            .push(Box::new(self.colors.results[slot].replace(result)))
                    }
                    Ok(result) => self.retired.push(Box::new(result)),
                    Err(error) if current => self.desktop.report("Filename colors", error),
                    Err(_) => {}
                }
                self.retired.push(Box::new(request));
                None
            }
            other => Some(other),
        }
    }
}

fn same_tree(left: Option<&Arc<TreeProjection>>, right: Option<&Arc<TreeProjection>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => Arc::ptr_eq(left, right) && left.stamp == right.stamp,
        _ => false,
    }
}

fn same_snapshot(left: &FilenameColorsRequest, right: &FilenameColorsRequest) -> bool {
    left.pane == right.pane
        && Arc::ptr_eq(&left.source, &right.source)
        && Arc::ptr_eq(&left.rules, &right.rules)
        && same_tree(left.tree.as_ref(), right.tree.as_ref())
}

fn tree_current(pane: PaneId, panel: &Pane) -> bool {
    panel.tree_view().is_none_or(|tree| {
        tree.stamp.pane == pane
            && tree.root.pane == pane
            && tree.stamp.navigation.0 == panel.generation
            && tree.root.navigation == tree.stamp.navigation
            && Arc::ptr_eq(&tree.root.entries, &panel.entries)
    })
}

fn panel_snapshot(
    request: &FilenameColorsRequest,
    pane: PaneId,
    panel: &Pane,
    rules: &Arc<[FilenameColorRule]>,
) -> bool {
    request.pane == pane
        && Arc::ptr_eq(&request.source, &panel.entries)
        && Arc::ptr_eq(&request.rules, rules)
        && same_tree(request.tree.as_ref(), panel.tree_view())
        && tree_current(pane, panel)
}
