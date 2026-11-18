use super::*;
use std::{ffi::OsStr, os::unix::ffi::OsStrExt};

/// Resolves only the listed links (link, target), as if nothing else existed.
fn linked<'a>(links: &'a [(&str, &str)]) -> impl Fn(&Path) -> Option<PathBuf> + 'a {
    move |path: &Path| {
        links
            .iter()
            .find(|(link, _)| Path::new(link) == path)
            .map(|(_, target)| PathBuf::from(target))
    }
}

#[test]
fn links_resolve_before_the_missing_trailing_names_are_appended() {
    let links = [
        ("/home/u", "/var/home/u"),
        ("/data/cache", "/mnt/disk/cache"),
    ];
    for (base, resolved) in [
        ("/data/cache", "/mnt/disk/cache"),
        // A linked home resolves before its missing `.cache` is appended.
        ("/home/u/.cache", "/var/home/u/.cache"),
        ("/home/u/.cache/a/b", "/var/home/u/.cache/a/b"),
    ] {
        assert_eq!(
            resolve_base(Path::new(base), linked(&links)),
            Path::new(resolved),
            "{base}"
        );
    }
    // Missing names, including ones that are not UTF-8, are kept losslessly.
    let mut base = PathBuf::from("/home/u/東京");
    base.push(OsStr::from_bytes(b"\xff-raw"));
    let mut resolved = PathBuf::from("/var/home/u/東京");
    resolved.push(OsStr::from_bytes(b"\xff-raw"));
    assert_eq!(resolve_base(&base, linked(&links)), resolved);
}

#[test]
fn a_base_without_a_resolvable_prefix_is_used_as_entered() {
    for base in ["/run/user/1000", "/"] {
        assert_eq!(resolve_base(Path::new(base), |_| None), Path::new(base));
    }
}

#[test]
fn real_links_in_a_base_resolve_while_the_names_below_it_stay_as_entered() {
    let temporary = tempfile::tempdir().unwrap();
    let real = temporary.path().join("real");
    std::fs::create_dir_all(real.join("existing")).unwrap();
    let link = temporary.path().join("link");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    let canonical = |path: &Path| std::fs::canonicalize(path).ok();
    let real = real.canonicalize().unwrap();
    assert_eq!(
        resolve_base(&link.join("existing"), canonical),
        real.join("existing")
    );
    assert_eq!(
        resolve_base(&link.join("missing/below"), canonical),
        real.join("missing/below")
    );
    // A dangling link is not resolved: it stays a link that the private-root walk,
    // which follows no link, refuses.
    let dangling = temporary.path().join("dangling");
    std::os::unix::fs::symlink(temporary.path().join("nowhere"), &dangling).unwrap();
    assert_eq!(
        resolve_base(&dangling.join("spool"), canonical),
        temporary
            .path()
            .canonicalize()
            .unwrap()
            .join("dangling/spool")
    );
}
