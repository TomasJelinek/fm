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
        "x:",
        "x.",
        "x%20",
        "%zz",
    ] {
        assert!(resolve_segments(&[], url).is_err(), "{url}");
    }
    assert_eq!(
        resolve_segments(&[], "styles/theme%20light.css?q=1#x").unwrap(),
        vec!["styles", "theme light.css"]
    );
    assert_eq!(
        resolve_segments(&["styles".into(), "base.css".into()], "icons/a.png").unwrap(),
        vec!["styles", "icons", "a.png"]
    );
}

#[test]
fn parent_segments_resolve_inside_the_scope_but_never_above_it() {
    let stylesheet: Vec<String> = vec!["css".into(), "site.css".into()];
    assert_eq!(
        resolve_segments(&stylesheet, "../img/logo.png").unwrap(),
        vec!["img", "logo.png"]
    );
    assert_eq!(
        resolve_segments(&stylesheet, "fonts/../%2e%2e/img/a.png").unwrap(),
        vec!["img", "a.png"]
    );
    for escape in ["../../secret", "../%2e%2e/secret", "img/../../../x"] {
        assert!(resolve_segments(&stylesheet, escape).is_err(), "{escape}");
    }
    // The main document is the scope root: any parent segment escapes.
    assert!(resolve_segments(&[], "../x").is_err());
    assert!(resolve_segments(&[], "a/../../x").is_err());
    // Resolving back to the root itself names no resource.
    assert!(resolve_segments(&stylesheet, "..").is_err());
}

#[test]
fn permissions_revoke_old_resource_ids_and_never_restore_them() {
    let main = CapturedFile::from_source(Arc::new(Bytes)).unwrap();
    let mut store = Store::new(&main, ResourcePathPolicy::RelativeAssets).unwrap();
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
