#![cfg(windows)]
use midnight_thumbnail_host::{
    decode::Source,
    service::{Service, Target, View},
};
use std::{
    io,
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
#[path = "../../../plugins/midnight-thumbnail-video/tests/fixtures/avi.rs"]
mod avi;

struct Bytes(Vec<u8>);
impl Source for Bytes {
    fn length(&self) -> u64 {
        self.0.len() as u64
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> io::Result<()> {
        Ok(())
    }
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        Ok(self.0[offset as usize..offset as usize + length as usize].to_vec())
    }
}
fn service() -> (Service, tempfile::TempDir) {
    let root = PathBuf::from(
        std::env::var_os("MIDNIGHT_DEPLOYMENT_TEST_ROOT")
            .expect("A tested private installation is required"),
    );
    let cache = tempfile::tempdir_in(env!("CARGO_TARGET_TMPDIR")).unwrap();
    (
        Service::start(root.join("me.exe"), cache.path().join("cache")),
        cache,
    )
}
#[test]
#[ignore = "Run after deploy-private.ps1 with MIDNIGHT_DEPLOYMENT_TEST_ROOT"]
fn private_installed_runtime_passes_discovery_and_decodes_through_the_service() {
    let (service, _cache) = service();
    let source: Arc<dyn Source> = Arc::new(Bytes(avi::video()));
    service.set_view(View {
        generation: 1,
        targets: vec![Target {
            token: 1,
            edge: 96,
            identity: b"fixture".to_vec(),
            extension: "avi".into(),
            selected: false,
            hovered: false,
            capture: Arc::new(move |_| Ok(source.clone())),
        }],
    });
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Some(notice) = service.take_notice() {
            panic!("{notice}");
        }
        if let Some(ready) = service.poll() {
            assert_eq!((ready.frame.width, ready.frame.height), (2, 2));
            break;
        }
        assert!(
            Instant::now() < deadline,
            "Private installed provider did not return a frame"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    service.stop();
}
#[test]
#[ignore = "Run with a deliberately writable mixed-case DLL in a disposable private fixture"]
fn mixed_case_runtime_with_weak_write_acl_is_rejected_during_discovery() {
    let (service, _cache) = service();
    let deadline = Instant::now() + Duration::from_secs(3);
    while service.take_notice().is_none() {
        assert!(
            Instant::now() < deadline,
            "An untrusted uppercase DLL bypassed discovery trust"
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    service.stop();
}
