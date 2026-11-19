//! Native image acceptance: run only from a private test installation with its
//! trusted image worker. These tests do not deploy or launch the application.
use super::*;
use crate::{IdentityVerdict, ImageMode};

fn bmp(metadata: u8) -> Vec<u8> {
    let mut bytes = vec![
        0x42, 0x4d, 58, 0, 0, 0, 0, 0, 0, 0, 54, 0, 0, 0, 40, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1,
        0, 24, 0, 0, 0, 0, 0, 4, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 255, 0,
        0, 0,
    ];
    bytes[38] = metadata;
    bytes
}

fn bmp_32() -> Vec<u8> {
    let mut bytes = bmp(0);
    bytes[28] = 32;
    bytes[57] = 255;
    bytes
}

fn image_fixture() -> tempfile::TempDir {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .join("_build/verification/viewer-refresh-20261117/viewer/compare-identity/images");
    std::fs::create_dir_all(&root).unwrap();
    tempfile::tempdir_in(root).unwrap()
}

fn require_installed_image_worker() {
    let application = midnight_plugin_host::installed_executable().unwrap();
    let providers = midnight_image_viewer_host::discover(&application).unwrap();
    assert!(
        providers
            .iter()
            .any(|provider| !provider.is_document() && provider.supports_extension("bmp")),
        "Run this test executable from a trusted private installation with the image worker"
    );
}

fn settled(session: &Session) -> Frame {
    wait_for(|last_status| match session.mailbox.take() {
        Some(Output::Frame { frame, .. }) if frame.identity != IdentityVerdict::Pending => {
            Some(frame)
        }
        Some(Output::Status(status)) => {
            *last_status = Some(status);
            None
        }
        Some(Output::ChooseFiles(choice)) => panic!("Image comparison failed: {choice:?}"),
        Some(Output::ExternalStarted { .. }) => {
            panic!("Built-in fixture launched an external program")
        }
        _ => None,
    })
}

#[test]
#[ignore = "Requires the test executable in a trusted private installation with the image worker"]
fn actual_images_publish_pending_before_file_identity_and_keep_generation_across_zoom() {
    require_installed_image_worker();
    let root = image_fixture();
    let paths = [root.path().join("left.bmp"), root.path().join("right.bmp")];
    for path in &paths {
        std::fs::write(path, bmp(0)).unwrap();
    }
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::new(runtime.handle().clone(), spawner);
    let session = open(&service, launch(&paths)).unwrap();
    let initial = frame(&session);
    assert!(initial.image);
    assert!(initial.layers.is_some());
    assert_eq!(initial.identity, IdentityVerdict::Pending);
    let complete = settled(&session);
    assert!(
        Arc::ptr_eq(&initial.find, &complete.find),
        "image paints share one empty Find snapshot"
    );
    assert_eq!(complete.identity, IdentityVerdict::IdenticalFiles);
    assert_eq!(
        complete.comparison_generation,
        initial.comparison_generation
    );
    assert!(session.send(Input::Zoom(2.0)).is_ok());
    let zoomed = frame(&session);
    assert!(Arc::ptr_eq(&complete.find, &zoomed.find));
    assert_eq!(zoomed.comparison_generation, complete.comparison_generation);
    assert_eq!(zoomed.identity, IdentityVerdict::IdenticalFiles);
    assert!(session.send(Input::Mode(ImageMode::OnionSkin)).is_ok());
    let mode = frame(&session);
    assert_eq!(mode.comparison_generation, complete.comparison_generation);
    assert_eq!(mode.identity, IdentityVerdict::IdenticalFiles);
    session.close();
}

#[test]
#[ignore = "Requires the test executable in a trusted private installation with the image worker"]
fn actual_equal_pixels_with_different_bitmap_encoding_never_claim_identical_files() {
    require_installed_image_worker();
    let root = image_fixture();
    let paths = [root.path().join("left.bmp"), root.path().join("right.bmp")];
    std::fs::write(&paths[0], bmp(0)).unwrap();
    std::fs::write(&paths[1], bmp_32()).unwrap();
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (_owner, spawner) = crate::resident_test_support::workers();
    let service = Service::new(runtime.handle().clone(), spawner);
    let session = open(&service, launch(&paths)).unwrap();
    let initial = frame(&session);
    assert!(initial.image);
    assert_eq!(initial.identity, IdentityVerdict::Pending);
    let layers = initial.layers.as_ref().unwrap();
    assert_eq!(layers.frames[0].rgba, layers.frames[1].rgba);
    let result = settled(&session);
    assert_eq!(result.identity, IdentityVerdict::Different);
    assert_eq!(result.comparison_generation, initial.comparison_generation);
    session.close();
}
