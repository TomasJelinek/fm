//! Composition tokens identify only the process-pinned factory.
use super::*;
#[test]
fn composition_clones_share_the_lazy_process_owner_without_discovery() {
    let token = InstalledProvider::shared_composition(None);
    let other = token.clone();
    assert!(Arc::ptr_eq(&token.service(), &other.service()));
    assert!(Arc::ptr_eq(
        &token.service(),
        &InstalledProvider::shared(None)
    ));
    assert!(!Arc::ptr_eq(
        &token.service(),
        &Arc::new(InstalledProvider::default())
    ));
    assert!(!Arc::ptr_eq(
        &token.service(),
        &Arc::new(InstalledProvider::from_installation(
            PathBuf::from("unopened-installation"),
            None
        ))
    ));
}
