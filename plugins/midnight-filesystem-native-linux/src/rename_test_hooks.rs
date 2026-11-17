//! Unit-only controls at real native commit/recovery boundaries, plus switches
//! that simulate filesystem behavior an owned local fixture cannot show.
use std::{
    cell::RefCell,
    ffi::{CStr, CString},
    fs::File,
    os::{fd::AsRawFd, unix::ffi::OsStrExt},
    path::Path,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Boundary {
    AfterStageCreate,
    BeforeCapture,
    AfterCapture,
    BeforePublish,
    BeforeRestoreValidation,
    AfterCommit,
    BeforeCleanup,
}
type Action = Box<dyn FnOnce(&Path)>;
#[derive(Default)]
struct Hooks {
    actions: Vec<(Boundary, Action)>,
    fail_sync: bool,
    fail_rename: bool,
    refuse_next_rename: Option<i32>,
    refuse_next_mkdir: Option<i32>,
    ignore_private_mode: bool,
    unqualified_filesystem: bool,
    case_insensitive_lookup: bool,
    private_group: Option<bool>,
}
thread_local! {
    static HOOKS: RefCell<Hooks> = RefCell::default();
}
pub(crate) struct Reset;
impl Drop for Reset {
    fn drop(&mut self) {
        HOOKS.with(|hooks| *hooks.borrow_mut() = Hooks::default());
    }
}
pub(crate) fn reset() -> Reset {
    HOOKS.with(|hooks| *hooks.borrow_mut() = Hooks::default());
    Reset
}
pub(crate) fn at(boundary: Boundary, action: impl FnOnce(&Path) + 'static) {
    HOOKS.with(|hooks| {
        hooks
            .borrow_mut()
            .actions
            .push((boundary, Box::new(action)))
    });
}
pub(crate) fn fire(boundary: Boundary, stage: &Path) {
    let action = HOOKS.with(|hooks| {
        let mut hooks = hooks.borrow_mut();
        let index = hooks
            .actions
            .iter()
            .position(|(point, _)| *point == boundary)?;
        Some(hooks.actions.remove(index).1)
    });
    if let Some(action) = action {
        action(stage);
    }
}
pub(crate) fn fail_next_sync() {
    HOOKS.with(|hooks| hooks.borrow_mut().fail_sync = true);
}
pub(crate) fn take_sync_failure() -> bool {
    HOOKS.with(|hooks| std::mem::take(&mut hooks.borrow_mut().fail_sync))
}
pub(crate) fn fail_after_next_rename() {
    HOOKS.with(|hooks| hooks.borrow_mut().fail_rename = true);
}
pub(crate) fn take_rename_failure() -> bool {
    HOOKS.with(|hooks| std::mem::take(&mut hooks.borrow_mut().fail_rename))
}
/// The next renameat2 call fails with `errno` without changing the namespace.
pub(crate) fn refuse_next_rename(errno: i32) {
    HOOKS.with(|hooks| hooks.borrow_mut().refuse_next_rename = Some(errno));
}
pub(crate) fn take_rename_refusal() -> Option<i32> {
    HOOKS.with(|hooks| hooks.borrow_mut().refuse_next_rename.take())
}
/// The next mkdirat call fails with `errno` without creating anything.
pub(crate) fn refuse_next_mkdir(errno: i32) {
    HOOKS.with(|hooks| hooks.borrow_mut().refuse_next_mkdir = Some(errno));
}
pub(crate) fn take_mkdir_refusal() -> Option<i32> {
    HOOKS.with(|hooks| hooks.borrow_mut().refuse_next_mkdir.take())
}
/// Requests to make a new stage owner-only keep its current mode, as on
/// filesystems whose permissions come from mount options.
pub(crate) fn ignore_private_mode() {
    HOOKS.with(|hooks| hooks.borrow_mut().ignore_private_mode = true);
}
pub(crate) fn private_mode_ignored() -> bool {
    HOOKS.with(|hooks| hooks.borrow().ignore_private_mode)
}
/// The fixture filesystem is treated like remote or FUSE storage.
pub(crate) fn treat_filesystem_as_unqualified() {
    HOOKS.with(|hooks| hooks.borrow_mut().unqualified_filesystem = true);
}
pub(crate) fn unqualified_filesystem() -> bool {
    HOOKS.with(|hooks| hooks.borrow().unqualified_filesystem)
}
/// Name lookups fall back to an ASCII case-insensitive match, as on vfat,
/// exfat, CIFS or casefolded ext4 directories. Exact names still win.
pub(crate) fn fold_lookup_case() {
    HOOKS.with(|hooks| hooks.borrow_mut().case_insensitive_lookup = true);
}
pub(crate) fn case_alias(parent: &File, name: &CStr) -> Option<CString> {
    if !HOOKS.with(|hooks| hooks.borrow().case_insensitive_lookup) {
        return None;
    }
    let listing = std::fs::read_dir(format!("/proc/self/fd/{}", parent.as_raw_fd())).ok()?;
    listing
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .find(|entry| entry.as_bytes().eq_ignore_ascii_case(name.to_bytes()))
        .and_then(|entry| CString::new(entry.as_bytes()).ok())
}
/// Overrides whether the caller's effective group is a user-private group.
pub(crate) fn assume_private_group(verdict: bool) {
    HOOKS.with(|hooks| hooks.borrow_mut().private_group = Some(verdict));
}
pub(crate) fn private_group() -> Option<bool> {
    HOOKS.with(|hooks| hooks.borrow().private_group)
}
