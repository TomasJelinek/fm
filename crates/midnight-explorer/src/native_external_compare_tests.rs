//! Typed adapter receipts must survive cancellation/error mapping unchanged.
use super::*;

#[test]
fn external_capture_cancelled_before_admission_never_needs_an_installed_provider() {
    let filesystem = Arc::new(InstalledProvider::from_installation(
        std::env::temp_dir().join("no-external-compare-provider"),
        None,
    ));
    let capture = external_comparison_factory(filesystem);
    let cancellation = midnight_core::Cancellation::default();
    cancellation.cancel();
    let path = std::env::temp_dir().join("comparison-input.bin");
    let failure = capture(&path, &cancellation).unwrap_err();
    assert_eq!(failure.error.kind, io::ErrorKind::Interrupted);
    assert!(!failure.cleanup_unconfirmed);
}

#[test]
fn external_capture_error_mapping_preserves_explicit_cleanup_status_even_when_cancelled() {
    let filesystem = InstalledProvider::from_installation(
        std::env::temp_dir().join("no-external-compare-provider"),
        None,
    );
    let path = std::env::temp_dir().join("comparison-input.bin");
    for cleanup_unconfirmed in [false, true] {
        for cancelled in [false, true] {
            // Obtain a real typed native failure without discovery or a filesystem open.
            let mut failure = filesystem.capture_checked(&path, &|| true).unwrap_err();
            failure.cleanup_unconfirmed = cleanup_unconfirmed;
            let cancellation = midnight_core::Cancellation::default();
            if cancelled {
                cancellation.cancel();
            }
            let mapped = external_capture_error(&path, &cancellation, failure);
            assert_eq!(mapped.cleanup_unconfirmed, cleanup_unconfirmed);
            assert_eq!(
                mapped.error.kind,
                if cancelled {
                    io::ErrorKind::Interrupted
                } else {
                    io::ErrorKind::Other
                }
            );
        }
    }
}
