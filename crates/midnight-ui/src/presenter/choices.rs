//! One identity-bearing native navigation snapshot for both UI surfaces.
use super::*;

pub(crate) struct NavigationChoice {
    pub(crate) id: u64,
    pub(crate) location: NavigationLocation,
}

#[derive(Default)]
pub(super) struct ChoicesState {
    pub(super) authoritative_volumes: bool,
    pub(super) revision: u64,
    pub(super) next_id: u64,
    pub(crate) items: Vec<NavigationChoice>,
    preferred: [Option<u64>; 2],
}

fn identity(location: &NavigationLocation) -> (LocationKind, String, PathBuf) {
    (location.kind, location.id.clone(), location.path.clone())
}

impl Presenter {
    pub(crate) fn navigation_choices(&self) -> &[NavigationChoice] {
        &self.choices.items
    }

    pub(crate) fn navigation_choice_token(&self, index: usize) -> String {
        format!("{}:{}", self.choices.revision, self.choices.items[index].id)
    }

    pub(crate) fn navigation_choices_revision(&self) -> u64 {
        self.choices.revision
    }

    pub(super) fn sync_navigation_choices(&mut self) {
        let native_home = self
            .desktop
            .locations
            .iter()
            .find(|location| location.kind == LocationKind::Home && location.id == "home");
        let synthetic_home = |volume: &&Volume| {
            volume.id == "home"
                && volume.label == "Home"
                && native_home.is_none_or(|home| home.path == volume.root)
        };
        let mut locations = Vec::new();
        if self.choices.authoritative_volumes {
            for volume in self.volumes.iter().filter(|volume| !synthetic_home(volume)) {
                let letter = self
                    .desktop
                    .locations
                    .iter()
                    .find(|location| {
                        location.kind == LocationKind::Volume
                            && location.id == volume.id
                            && location.path == volume.root
                    })
                    .and_then(|location| location.drive_letter);
                locations.push(NavigationLocation {
                    id: volume.id.clone(),
                    label: volume.label.clone(),
                    path: volume.root.clone(),
                    kind: LocationKind::Volume,
                    drive_letter: letter,
                });
            }
        } else {
            locations.extend(
                self.desktop
                    .locations
                    .iter()
                    .filter(|location| location.kind == LocationKind::Volume)
                    .cloned(),
            );
        }
        locations.extend(
            self.desktop
                .locations
                .iter()
                .filter(|location| location.kind != LocationKind::Volume)
                .cloned(),
        );
        // list_volumes contains a synthetic Home alias too. Prefer the semantic
        // desktop discovery entry, preserving other same-path aliases intact.
        if !locations
            .iter()
            .any(|location| location.kind == LocationKind::Home && location.id == "home")
            && let Some(home) = self.volumes.iter().find(synthetic_home)
        {
            locations.push(NavigationLocation {
                id: home.id.clone(),
                label: home.label.clone(),
                path: home.root.clone(),
                kind: LocationKind::Home,
                drive_letter: None,
            });
        }
        let mut seen = HashSet::new();
        locations.retain(|location| seen.insert(identity(location)));
        self.retired.push(Box::new(seen));
        let structural = locations.len() != self.choices.items.len()
            || locations
                .iter()
                .zip(&self.choices.items)
                .any(|(next, old)| {
                    next.id != old.location.id
                        || next.kind != old.location.kind
                        || next.path != old.location.path
                });
        let mut revision = self.choices.revision;
        if structural {
            let Some(next) = revision.checked_add(1) else {
                self.retired.push(Box::new(locations));
                self.retired
                    .push(Box::new(mem::take(&mut self.choices.items)));
                self.desktop.report(
                    "Navigation identities exhausted",
                    "Restart Midnight Explorer before navigating through these choices.".into(),
                );
                return;
            };
            revision = next;
        }
        let previous = mem::take(&mut self.choices.items);
        let by_identity: HashMap<_, _> = previous
            .iter()
            .map(|choice| (identity(&choice.location), choice.id))
            .collect();
        let mut items = Vec::with_capacity(locations.len());
        let mut remaining = locations.into_iter();
        while let Some(location) = remaining.next() {
            let id = if let Some(id) = by_identity.get(&identity(&location)) {
                *id
            } else if let Some(id) = self.choices.next_id.checked_add(1) {
                self.choices.next_id = id;
                id
            } else {
                self.retired.push(Box::new(location));
                self.retired.push(Box::new(items));
                self.retired.push(Box::new(previous));
                self.retired.push(Box::new(by_identity));
                self.retired.push(Box::new(remaining));
                self.desktop.report(
                    "Navigation identities exhausted",
                    "Restart Midnight Explorer before navigating through these choices.".into(),
                );
                return;
            };
            items.push(NavigationChoice { id, location });
        }
        self.choices.items = items;
        self.choices.revision = revision;
        self.retired.push(Box::new(previous));
        self.retired.push(Box::new(by_identity));
    }

    pub(crate) fn choose_navigation_choice(
        &mut self,
        pane: PaneId,
        token: &str,
        send: &dyn Fn(Command) -> bool,
    ) -> bool {
        let Some((revision, id)) = token.split_once(':') else {
            return false;
        };
        let (Ok(revision), Ok(id)) = (revision.parse::<u64>(), id.parse::<u64>()) else {
            return false;
        };
        if revision != self.choices.revision {
            return false;
        }
        let Some(choice) = self.choices.items.iter().find(|choice| choice.id == id) else {
            return false;
        };
        let path = if choice.location.kind == LocationKind::Volume {
            self.panes[pane.index()]
                .volume_paths
                .get(&choice.location.path)
                .unwrap_or(&choice.location.path)
        } else {
            &choice.location.path
        }
        .clone();
        if self.navigate(pane, path, None, send) {
            self.choices.preferred[pane.index()] = Some(id);
            true
        } else {
            false
        }
    }

    pub(crate) fn navigation_choice_index(&self, pane: PaneId) -> i32 {
        let path = self.panes[pane.index()].display_path();
        let preferred = self.choices.preferred[pane.index()];
        let mut best: Option<(usize, usize, bool)> = None;
        for (index, choice) in self.choices.items.iter().enumerate() {
            if choice.location.kind != LocationKind::Volume
                || !path.starts_with(&choice.location.path)
            {
                continue;
            }
            let depth = choice.location.path.components().count();
            if best.is_none_or(|(_, previous_depth, previous_preferred)| {
                depth > previous_depth
                    || (depth == previous_depth
                        && Some(choice.id) == preferred
                        && !previous_preferred)
            }) {
                best = Some((index, depth, Some(choice.id) == preferred));
            }
        }
        best.map_or(-1, |(index, _, _)| index as i32)
    }
}
