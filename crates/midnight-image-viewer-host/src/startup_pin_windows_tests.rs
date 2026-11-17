use super::*;

#[test]
fn executable_pin_blocks_data_writes_and_replacement_until_consuming_release() {
    let fixture = Fixture::new();
    let identity = ProviderIdentity::capture(&fixture.provider).unwrap();
    let path = &fixture.provider.executable;
    let original = fs::read(path).unwrap();
    let write = fs::OpenOptions::new().write(true).open(path).unwrap_err();
    assert_eq!(
        write.raw_os_error(),
        Some(32),
        "data-write sharing must be denied"
    );
    let rename = fs::rename(path, path.with_extension("previous")).unwrap_err();
    assert_eq!(
        rename.raw_os_error(),
        Some(32),
        "replacement must be denied"
    );
    assert_eq!(fs::read(path).unwrap(), original);
    identity.validate_for(&fixture.provider).unwrap();
    lifetime::Release::release_on_worker(identity).unwrap();
    fs::rename(path, path.with_extension("previous")).unwrap();
    fs::write(path, original).unwrap();
    // The installed descriptor remains valid; a later capture receives its own new native pin.
    let replacement = ProviderIdentity::capture(&fixture.provider).unwrap();
    replacement.validate_for(&fixture.provider).unwrap();
    lifetime::Release::release_on_worker(replacement).unwrap();
}
