use super::*;
#[test]
fn linux_move_refuses_unenforceable_source_finalization_before_publication() {
    let (_temp, parent) = protected_fixture();
    let path = parent.join("source");
    let destination = parent.join("destination");
    fs::write(&path, b"original").unwrap();
    fs::create_dir(&destination).unwrap();
    let request = request(1, std::slice::from_ref(&path), &destination);
    let status = Arc::new(Mutex::new(initial(&CopyInput::move_paths(request.clone()))));
    let service = midnight_platform::LocalFileSystem::new(None);
    let result = worker::Job::prepare_move(request, &service, Cancellation::default(), status);
    assert!(
        result.is_err(),
        "a timeout-dependent lease probe must not authorize deletion"
    );
    assert_eq!(fs::read(path).unwrap(), b"original");
    assert_eq!(fs::read_dir(destination).unwrap().count(), 0);
}
