use super::PreparedCatalogs;
use std::path::Path;

fn scratch() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../_build/tests/prepared-catalogs");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::Builder::new()
        .prefix("rejected-path-")
        .tempdir_in(root)
        .unwrap()
}

fn rejection(application: &Path) -> String {
    match PreparedCatalogs::discover(application) {
        Ok(_) => panic!("invalid application path was accepted"),
        Err(error) => error,
    }
}

#[test]
fn relative_application_is_rejected_before_discovery() {
    assert_eq!(
        rejection(Path::new("application-metadata")),
        "Prepared catalog application path must be absolute"
    );
}

#[test]
fn directory_cannot_become_the_application_authority() {
    let scratch = scratch();
    assert_eq!(
        rejection(&scratch.path().canonicalize().unwrap()),
        "Prepared catalog application must be a regular file"
    );
}

#[test]
fn dot_alias_is_not_canonicalized_into_application_authority() {
    let scratch = scratch();
    let metadata = scratch.path().join("application-metadata");
    std::fs::write(&metadata, b"negative path-validation fixture").unwrap();
    let canonical = metadata.canonicalize().unwrap();
    // Joining a component onto a Windows verbatim PathBuf normalizes dots.
    // Assemble the supplied spelling as OsString so the negative fixture keeps its alias.
    let mut raw_alias = canonical.parent().unwrap().as_os_str().to_os_string();
    raw_alias.push(std::path::MAIN_SEPARATOR_STR);
    raw_alias.push(".");
    raw_alias.push(std::path::MAIN_SEPARATOR_STR);
    raw_alias.push(canonical.file_name().unwrap());
    let alias = std::path::PathBuf::from(raw_alias);
    assert_ne!(alias.as_os_str(), canonical.as_os_str());
    // Extended Windows paths may reject the alias before canonicalization can return a path.
    // Either outcome must fail the canonical-path gate, independently of package permissions.
    assert!(super::require_canonical(&alias, "application").is_err());
    assert!(PreparedCatalogs::discover(&alias).is_err());
}
