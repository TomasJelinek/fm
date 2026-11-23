use super::*;
use midnight_plugin_host::{ContentSource, Error};

#[derive(Debug)]
struct Bytes;
impl ContentSource for Bytes {
    fn length(&self) -> u64 {
        5
    }
    fn display_name(&self) -> String {
        "index.html".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        Ok(b"hello"[offset as usize..(offset + u64::from(length)).min(5) as usize].to_vec())
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        Ok(())
    }
}

#[test]
fn urls_cannot_escape_authority_or_smuggle_native_paths() {
    for url in [
        "../secret",
        "%2e%2e/secret",
        "file:///a",
        "https://site/a",
        "//site/a",
        "%2fsecret",
        "x%5cy",
        "x%00y",
        "%252e%252e/x",
        "x%25y",
        "x:",
        "x.",
        "x%20",
        "%zz",
    ] {
        assert!(resolve_segments(&[], url, 0).is_err(), "{url}");
    }
    assert_eq!(
        resolve_segments(&[], "styles/theme%20light.css?q=1#x", 0).unwrap(),
        vec!["styles", "theme light.css"]
    );
    assert_eq!(
        resolve_segments(&["styles".into(), "base.css".into()], "icons/a.png", 0).unwrap(),
        vec!["styles", "icons", "a.png"]
    );
}

/// A page saved "complete" keeps its assets in `<title>_files`, whose title may hold `#`
/// or non-ASCII letters, and a saved script is named `<name>.js.download`.
#[test]
fn saved_page_names_with_spaces_unicode_and_hash_resolve_literally() {
    assert_eq!(
        resolve_segments(
            &[],
            "C%23%20notes%20%C5%BElu%C5%A5ou%C4%8Dk%C3%BD_files/a%3Fb.css",
            0
        )
        .unwrap(),
        vec!["C# notes žluťoučký_files", "a?b.css"]
    );
    assert_eq!(mime_type("jquery.min.js.download"), "text/javascript");
    assert_eq!(mime_type("site.CSS"), "text/css");
    assert_eq!(mime_type("analytics.download"), "application/octet-stream");
    assert_eq!(mime_type("search_index.json"), "application/json");
    assert_eq!(mime_type("logo.avif"), "image/avif");
    assert_eq!(mime_type("no-extension"), "application/octet-stream");
}

#[test]
fn parent_segments_resolve_inside_the_scope_but_never_above_it() {
    let stylesheet: Vec<String> = vec!["css".into(), "site.css".into()];
    assert_eq!(
        resolve_segments(&stylesheet, "../img/logo.png", 0).unwrap(),
        vec!["img", "logo.png"]
    );
    assert_eq!(
        resolve_segments(&stylesheet, "fonts/../%2e%2e/img/a.png", 0).unwrap(),
        vec!["img", "a.png"]
    );
    for escape in ["../../secret", "../%2e%2e/secret", "img/../../../x"] {
        assert!(
            resolve_segments(&stylesheet, escape, 0).is_err(),
            "{escape}"
        );
    }
    // The main document is the scope root: any parent segment escapes.
    assert!(resolve_segments(&[], "../x", 0).is_err());
    assert!(resolve_segments(&[], "a/../../x", 0).is_err());
    // Resolving back to the root itself names no resource.
    assert!(resolve_segments(&stylesheet, "..", 0).is_err());
}

/// Generated documentation (rustdoc, MkDocs, Sphinx, Javadoc) names shared assets of
/// folders above a page: a scope that reaches them resolves them to leading `..`.
#[test]
fn parent_segments_reach_as_many_folders_above_the_root_as_the_scope_allows() {
    assert_eq!(
        resolve_segments(&[], "../static.files/rustdoc.css", 2).unwrap(),
        vec!["..", "static.files", "rustdoc.css"]
    );
    assert_eq!(
        resolve_segments(&[], "../../x/../y.css", 2).unwrap(),
        vec!["..", "..", "y.css"]
    );
    // Down and up again stays inside the scope.
    assert_eq!(
        resolve_segments(&[], "a/../../b.css", 1).unwrap(),
        vec!["..", "b.css"]
    );
    // A stylesheet above the root resolves its own relative assets from its folder.
    let above: Vec<String> = vec!["..".into(), "css".into(), "site.css".into()];
    assert_eq!(
        resolve_segments(&above, "../fonts/a.woff2", 2).unwrap(),
        vec!["..", "fonts", "a.woff2"]
    );
    assert_eq!(
        resolve_segments(&above, "../../b.png", 2).unwrap(),
        vec!["..", "..", "b.png"]
    );
    for escape in ["../../../x", "../..", "..", "../"] {
        assert!(resolve_segments(&[], escape, 2).is_err(), "{escape}");
    }
    assert!(resolve_segments(&above, "../../../c.png", 2).is_err());
}

#[test]
fn permissions_revoke_old_resource_ids_and_never_restore_them() {
    let main = CapturedFile::from_source(Arc::new(Bytes)).unwrap();
    let mut store = Store::new(&main, ResourcePathPolicy::RelativeAssets, 0).unwrap();
    let first = store.reset(false).unwrap();
    store.read(1, first.id, 0, 5).unwrap();
    assert!(
        matches!(store.accept_read(1, first.id, 0, b"hello".to_vec()),
        ResourceReply::Data { bytes, .. } if bytes == b"hello")
    );
    let second = store.reset(true).unwrap();
    assert_ne!(first.id, second.id);
    assert!(store.read(2, first.id, 0, 5).is_err());
    assert!(store.resolve(3, second.id, "sibling.html", None).is_err());
    let third = store.reset(false).unwrap();
    assert_ne!(third.id, first.id);
    assert!(store.read(4, second.id, 0, 5).is_err());
    assert!(store.read(5, third.id, u64::MAX, 1).is_err());
    main.release_on_worker().unwrap();
}
