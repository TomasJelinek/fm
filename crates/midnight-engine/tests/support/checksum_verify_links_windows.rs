//! The directory junction of the engine's Windows tests, shared by every test that needs
//! one. The platform crate's fixture sets it with `FSCTL_SET_REPARSE_POINT` on a new folder,
//! so no shell or command line sees either path (SECURITY.md): `%`, `!`, `^`, `&` and
//! spaces in them are only data. Junctions need no symbolic-link privilege.

/// Creates `link` as a new directory junction to `target`, which need not exist; a
/// relative target is made absolute against the working folder, as `mklink /J` does.
pub fn create(link: &std::path::Path, target: &std::path::Path) {
    if let Err(error) = midnight_platform_windows::new_junction(link, target) {
        panic!("junction {link:?} to {target:?}: {error}");
    }
}
