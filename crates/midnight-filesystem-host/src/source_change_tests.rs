//! A captured archive whose file was changed fails its revalidation as `InvalidData`,
//! which Copy does not offer to retry; another failure keeps `Other`.
use super::*;
use midnight_plugin_host::{ContentSource, Error};

#[derive(Debug, Default)]
struct Source {
    failing: AtomicBool,
    gone: AtomicBool,
}
impl ContentSource for Source {
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        let _ = Arc::into_inner(self);
        Ok(())
    }
    fn length(&self) -> u64 {
        4
    }
    fn display_name(&self) -> String {
        "archive.tar".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        if self.failing.load(Ordering::Acquire) {
            return Err(Error("Native file changed after capture".into()));
        }
        Ok(())
    }
    fn read_at(&self, _: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.validate()?;
        Ok(vec![0; length as usize])
    }
    fn changed_since_capture(&self) -> bool {
        self.gone.load(Ordering::Acquire)
    }
}

fn mount(source: &Arc<Source>) -> Mount {
    Mount {
        id: 9,
        provider: "archive.fixture".into(),
        provider_version: "1.0.0".into(),
        stable_entry_ids: true,
        format: Format::Tar,
        source: Some(CapturedFile::from_source(source.clone()).unwrap()),
        container: std::env::temp_dir().join("archive.tar"),
        channel: Mutex::new(Channel {
            input: tempfile::tempfile().unwrap(),
            output: tempfile::tempfile().unwrap(),
            request: 0,
        }),
        supervision: Arc::new(Supervision::new(None)),
        protocol: Protocol::V3,
        staging: Arc::new(Mutex::new(None)),
        close: Arc::new(close::CloseReceipt::new()),
        nested: None,
    }
}

#[test]
fn only_a_lasting_source_change_fails_as_invalid_data() {
    let source = Arc::new(Source::default());
    let mount = mount(&source);
    mount.validate().unwrap();
    // For example an unreachable file: a later Retry can succeed.
    source.failing.store(true, Ordering::Release);
    let error = mount.validate().unwrap_err();
    assert_eq!(error.operation, "archive source changed");
    assert_eq!(error.kind, io::ErrorKind::Other, "{error}");
    source.gone.store(true, Ordering::Release);
    let error = mount.validate().unwrap_err();
    assert_eq!(error.kind, io::ErrorKind::InvalidData, "{error}");
    assert!(error.message.contains("changed after capture"), "{error}");
}
