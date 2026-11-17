use std::{ffi::OsString, io, path::PathBuf};

pub(super) fn installation_path(path: &std::path::Path) -> io::Result<PathBuf> {
    std::path::absolute(path)
}
pub(crate) fn cache_root() -> io::Result<PathBuf> {
    cache_root_from(std::env::var_os("XDG_CACHE_HOME"), std::env::var_os("HOME"))
}
/// XDG: an empty or relative `XDG_CACHE_HOME` is ignored in favour of `$HOME/.cache`.
fn cache_root_from(cache_home: Option<OsString>, home: Option<OsString>) -> io::Result<PathBuf> {
    cache_home
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| {
            home.map(|path| PathBuf::from(path).join(".cache"))
                .filter(|path| path.is_absolute())
        })
        .map(|path| path.join("midnight-explorer").join("thumbnails").join("v1"))
        .ok_or_else(|| io::Error::other("An absolute private XDG cache directory is required"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_relative_xdg_cache_home_falls_back_to_the_home_cache() {
        let expected = PathBuf::from("/home/čeněk/.cache/midnight-explorer/thumbnails/v1");
        for ignored in ["", "relative/cache"] {
            assert_eq!(
                cache_root_from(Some(ignored.into()), Some("/home/čeněk".into())).unwrap(),
                expected
            );
        }
        assert_eq!(
            cache_root_from(Some("/var/cache/user".into()), Some("/home/čeněk".into())).unwrap(),
            PathBuf::from("/var/cache/user/midnight-explorer/thumbnails/v1")
        );
        assert!(cache_root_from(Some("".into()), Some("relative".into())).is_err());
        assert!(cache_root_from(None, None).is_err());
    }
}
