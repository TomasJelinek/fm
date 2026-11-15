//! Explicit content release never turns an opaque destructor into an acknowledgement.
use super::*;
use std::sync::{
    Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Debug)]
struct Unknown(Arc<AtomicUsize>);
impl Drop for Unknown {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl ContentSource for Unknown {
    fn length(&self) -> u64 {
        1
    }
    fn display_name(&self) -> String {
        "unknown".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        Ok(vec![0])
    }
}
#[derive(Debug)]
struct Source {
    order: Arc<Mutex<Vec<&'static str>>>,
    refuse: bool,
    panic: bool,
}
impl ContentSource for Source {
    fn length(&self) -> u64 {
        1
    }
    fn display_name(&self) -> String {
        "source".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        Ok(vec![0])
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        let Some(source) = Arc::into_inner(self) else {
            return Ok(());
        };
        source.order.lock().unwrap().push("source");
        if source.refuse || source.panic {
            let panic = source.panic;
            std::mem::forget(source);
            if panic {
                panic!("source release panic");
            }
            return Err(Error("source release unconfirmed".into()));
        }
        Ok(())
    }
}
#[derive(Debug)]
struct Resources(Arc<Mutex<Vec<&'static str>>>);
impl ContentResources for Resources {
    fn capture(&self, _: &[String]) -> Result<CapturedFile, Error> {
        unreachable!()
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        if let Some(resources) = Arc::into_inner(self) {
            resources.0.lock().unwrap().push("resources");
        }
        Ok(())
    }
}

#[derive(Debug)]
struct UnknownResources(Arc<AtomicUsize>);
impl ContentResources for UnknownResources {
    fn capture(&self, _: &[String]) -> Result<CapturedFile, Error> {
        unreachable!()
    }
}
impl Drop for UnknownResources {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}

#[test]
fn unknown_resources_stay_owned_after_source_release() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let drops = Arc::new(AtomicUsize::new(0));
    let resources = Arc::new(UnknownResources(drops.clone()));
    let retained = Arc::downgrade(&resources);
    let file = CapturedFile::from_source(Arc::new(Source {
        order: order.clone(),
        refuse: false,
        panic: false,
    }))
    .unwrap()
    .with_resources(resources);
    assert!(file.release_on_worker().is_err());
    assert_eq!(*order.lock().unwrap(), ["source"]);
    assert_eq!(drops.load(Ordering::SeqCst), 0);
    assert!(retained.upgrade().is_some());
}

#[test]
fn plain_native_snapshot_metadata_needs_no_content_release_protocol() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("metadata.txt");
    std::fs::write(&path, b"native metadata").unwrap();
    CapturedFile::capture(&path)
        .unwrap()
        .release_on_worker()
        .unwrap();
    std::fs::remove_file(&path).unwrap();
}
#[test]
fn unknown_content_is_retained_instead_of_certifying_its_drop() {
    let drops = Arc::new(AtomicUsize::new(0));
    let source = Arc::new(Unknown(drops.clone()));
    let retained = Arc::downgrade(&source);
    assert!(source.release_on_worker().is_err());
    assert!(retained.upgrade().is_some());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
}

#[test]
fn default_clean_observation_does_not_qualify_an_unknown_content_release() {
    let drops = Arc::new(AtomicUsize::new(0));
    let source = Arc::new(Unknown(drops.clone()));
    let file = CapturedFile::from_source(source).unwrap();
    assert!(!file.cleanup_unconfirmed());
    assert!(file.release_on_worker().is_err());
    assert_eq!(drops.load(Ordering::SeqCst), 0);
}
#[test]
fn captured_file_releases_source_before_resources() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let file = CapturedFile::from_source(Arc::new(Source {
        order: order.clone(),
        refuse: false,
        panic: false,
    }))
    .unwrap()
    .with_resources(Arc::new(Resources(order.clone())));
    file.release_on_worker().unwrap();
    assert_eq!(*order.lock().unwrap(), ["source", "resources"]);
}
#[test]
fn refused_or_panicking_source_preserves_the_resource_tail() {
    for panic in [false, true] {
        let order = Arc::new(Mutex::new(Vec::new()));
        let resources = Arc::new(Resources(order.clone()));
        let retained = Arc::downgrade(&resources);
        let file = CapturedFile::from_source(Arc::new(Source {
            order: order.clone(),
            refuse: !panic,
            panic,
        }))
        .unwrap()
        .with_resources(resources);
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| file.release_on_worker()));
        if panic {
            assert!(result.is_err());
        } else {
            assert!(result.unwrap().is_err());
        }
        assert!(retained.upgrade().is_some());
        assert_eq!(*order.lock().unwrap(), ["source"]);
    }
}
#[test]
fn shared_known_source_does_not_stop_another_capture() {
    let order = Arc::new(Mutex::new(Vec::new()));
    let source = Arc::new(Source {
        order: order.clone(),
        refuse: false,
        panic: false,
    });
    let first = CapturedFile::from_source(source.clone()).unwrap();
    let second = first.clone();
    drop(source);
    first.release_on_worker().unwrap();
    assert!(order.lock().unwrap().is_empty());
    assert_eq!(second.read_at(0, 1).unwrap(), [0]);
    second.release_on_worker().unwrap();
    assert_eq!(*order.lock().unwrap(), ["source"]);
}
