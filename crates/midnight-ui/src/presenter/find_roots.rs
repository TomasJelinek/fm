//! Find search folders: a list of captured native roots, each ticked or not.
//! Added folders come from typed text or the native folder chooser; existing
//! roots keep their original native path, including names that cannot be typed.
use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct FindRoot {
    pub path: PathBuf,
    pub searched: bool,
}
impl FindRoot {
    pub(crate) fn new(path: PathBuf) -> Self {
        Self {
            path,
            searched: true,
        }
    }
}

impl Presenter {
    /// Add a typed absolute folder. Typed text is only ever a new root.
    pub(crate) fn add_find_root(&mut self, id: FindSessionId, text: &str) {
        self.add_find_root_path(id, PathBuf::from(text));
    }

    fn add_find_root_path(&mut self, id: FindSessionId, path: PathBuf) {
        let Some(s) = self.finds.session_mut(&id) else {
            return;
        };
        if s.running {
            // Only a chooser opened before Find was pressed can arrive here.
            s.error = "Search folders cannot change while a search runs.".into();
            return;
        }
        if let Some(row) = s.roots.iter().position(|root| root.path == path) {
            // Adding a listed folder again searches it rather than duplicating it.
            s.roots[row].searched = true;
            s.error.clear();
            s.added_root = (s.added_root.0.wrapping_add(1), row);
            return;
        }
        let mut paths: Vec<_> = s.roots.iter().map(|root| root.path.clone()).collect();
        paths.push(path);
        // Every listed root counts toward the request bounds, ticked or not.
        let request = FindRequest::new(id, s.generation, paths);
        match request.validate() {
            Ok(()) => {
                if let Some(path) = request.roots.into_iter().last() {
                    s.roots.push(FindRoot::new(path));
                    s.added_root = (s.added_root.0.wrapping_add(1), s.roots.len() - 1);
                }
                s.error.clear();
            }
            Err(error) => s.error = error.into(),
        }
    }

    pub(crate) fn remove_find_root(&mut self, id: FindSessionId, row: i32) {
        let Some(s) = self.finds.session_mut(&id) else {
            return;
        };
        if !s.running
            && let Ok(row) = usize::try_from(row)
            && row < s.roots.len()
        {
            s.roots.remove(row);
        }
    }

    pub(crate) fn set_find_root_searched(&mut self, id: FindSessionId, row: i32, searched: bool) {
        let Some(s) = self.finds.session_mut(&id) else {
            return;
        };
        if !s.running
            && let Some(root) = usize::try_from(row)
                .ok()
                .and_then(|row| s.roots.get_mut(row))
        {
            root.searched = searched;
        }
    }

    /// Open the native folder chooser for Add; its answer is bound to this session.
    pub(crate) fn request_find_root_picker(
        &mut self,
        id: FindSessionId,
        text: &str,
        owner: PathPickerOwner,
        send: &dyn Fn(Command) -> bool,
    ) {
        if self.operations.closing {
            return;
        }
        let request = self.desktop.request();
        let Some(s) = self.finds.session_mut(&id) else {
            return;
        };
        if s.running || s.picker.is_some() {
            return;
        }
        let path = Path::new(text);
        let initial_directory = if path.is_absolute() {
            Some(path.to_owned())
        } else {
            s.roots.last().map(|root| {
                if text.is_empty() {
                    root.path.clone()
                } else {
                    root.path.join(path)
                }
            })
        };
        if send(Command::PickPath {
            request,
            input: PathPickerRequest {
                kind: PathPickerKind::Directory,
                title: "Add search folder".into(),
                initial_directory,
                initial_name: None,
                owner,
            },
        }) {
            s.picker = Some(request);
            s.error.clear();
        } else {
            s.error = "Folder chooser queue is busy. Try Add again.".into();
        }
    }

    pub(crate) fn owns_find_picker(&self, request: DesktopRequestId) -> bool {
        self.finds
            .sessions
            .values()
            .any(|s| s.picker == Some(request))
    }

    pub(crate) fn find_path_picked(
        &mut self,
        request: DesktopRequestId,
        result: Result<Option<PathBuf>, FsError>,
    ) {
        let id = self
            .finds
            .sessions
            .iter()
            .find_map(|(id, s)| (s.picker == Some(request)).then_some(*id));
        let Some(s) = id.and_then(|id| self.finds.session_mut(&id)) else {
            self.retired.push(Box::new(result));
            return;
        };
        s.picker = None;
        if self.operations.closing {
            self.retired.push(Box::new(result));
            return;
        }
        match result {
            Ok(Some(path)) => match validate_picked_path(&path) {
                Ok(()) => {
                    if let Some(id) = id {
                        self.add_find_root_path(id, path);
                    }
                }
                Err(error) => s.error = crate::error_presentation::describe(&error),
            },
            Ok(None) => {}
            Err(error) => s.error = crate::error_presentation::describe(&error),
        }
    }
}
