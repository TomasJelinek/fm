use midnight_thumbnail_api::Request;
use midnight_thumbnail_host::decode::{self, Source};
use std::{io, sync::atomic::AtomicBool, time::Duration};

struct Bytes(Vec<u8>);
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());
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
        Ok(self.0[offset as usize..(offset as usize + length as usize).min(self.0.len())].to_vec())
    }
}

#[test]
#[ignore = "Run with MIDNIGHT_IMAGE_TEST_HELPER pointing to the separately built provider"]
fn decoder_deadline_cancels_a_broker_queued_behind_the_shared_io_gate() {
    let _serial = SERIAL.lock().unwrap();
    struct Unread;
    impl Source for Unread {
        fn length(&self) -> u64 {
            256
        }
        fn version(&self) -> Vec<u8> {
            vec![1]
        }
        fn validate(&self) -> io::Result<()> {
            Ok(())
        }
        fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
            panic!("A queued source read must not be admitted")
        }
    }
    let helper = std::env::var_os("MIDNIGHT_IMAGE_TEST_HELPER").unwrap();
    let gate = midnight_work_budget::shared_io();
    let held = gate
        .acquire(midnight_work_budget::Admission::Exclusive, || false)
        .unwrap();
    let start = std::time::Instant::now();
    let queued = AtomicBool::new(false);
    let error = std::thread::scope(|scope| {
        scope.spawn(|| {
            let deadline = std::time::Instant::now() + Duration::from_secs(4);
            while std::time::Instant::now() < deadline {
                if gate.snapshot().queued > 0 {
                    queued.store(true, std::sync::atomic::Ordering::Release);
                    break;
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        });
        decode::decode(
            std::path::Path::new(&helper),
            &Unread,
            Request {
                id: 1,
                source_length: 256,
                width: 32,
                height: 32,
                frame: 0,
            },
            &AtomicBool::new(false),
            Duration::from_secs(2),
        )
        .unwrap_err()
    });
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(
        queued.load(std::sync::atomic::Ordering::Acquire),
        "The test must exercise a broker queued behind the held gate"
    );
    assert!(start.elapsed() < Duration::from_secs(5));
    drop(held);
}

#[test]
#[ignore = "Run with MIDNIGHT_HANG_TEST_HELPER pointing to midnight-worker-probe"]
fn stalled_helper_is_killed_at_its_deadline() {
    let _serial = SERIAL.lock().unwrap();
    let helper = std::env::var_os("MIDNIGHT_HANG_TEST_HELPER").unwrap();
    let request = Request {
        id: 8,
        source_length: 0,
        width: 32,
        height: 32,
        frame: 0,
    };
    let start = std::time::Instant::now();
    // The containment probe waits for a newline. A binary thumbnail request has none.
    let error = decode::decode(
        std::path::Path::new(&helper),
        &Bytes(Vec::new()),
        request,
        &AtomicBool::new(false),
        Duration::from_millis(100),
    )
    .unwrap_err();
    assert_eq!(error.kind(), io::ErrorKind::TimedOut);
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
#[ignore = "Run with MIDNIGHT_IMAGE_TEST_HELPER pointing to the separately built provider"]
fn actual_sandboxed_image_provider_returns_pixels_and_honors_cancellation() {
    let _serial = SERIAL.lock().unwrap();
    let helper = std::env::var_os("MIDNIGHT_IMAGE_TEST_HELPER").unwrap();
    // 2x1 24-bit uncompressed BMP, authored fixture: red and green pixels, padded to four bytes.
    let mut bmp = b"BM".to_vec();
    bmp.extend_from_slice(&62u32.to_le_bytes());
    bmp.extend_from_slice(&[0; 4]);
    bmp.extend_from_slice(&54u32.to_le_bytes());
    bmp.extend_from_slice(&40u32.to_le_bytes());
    bmp.extend_from_slice(&2i32.to_le_bytes());
    bmp.extend_from_slice(&1i32.to_le_bytes());
    bmp.extend_from_slice(&1u16.to_le_bytes());
    bmp.extend_from_slice(&24u16.to_le_bytes());
    bmp.extend_from_slice(&[0; 24]);
    bmp.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);
    let source = Bytes(bmp);
    let request = Request {
        id: 8,
        source_length: source.length(),
        width: 32,
        height: 32,
        frame: 0,
    };
    let result = decode::decode(
        std::path::Path::new(&helper),
        &source,
        request.clone(),
        &AtomicBool::new(false),
        Duration::from_secs(5),
    )
    .unwrap();
    assert_eq!((result.width, result.height), (2, 1));
    assert_eq!(result.rgba, [255, 0, 0, 255, 0, 255, 0, 255]);
    assert!(
        decode::decode(
            std::path::Path::new(&helper),
            &source,
            request,
            &AtomicBool::new(true),
            Duration::from_secs(5)
        )
        .is_err()
    );
}

#[test]
#[ignore = "Run with MIDNIGHT_IMAGE_TEST_HELPER pointing to the separately built provider"]
fn decoder_cooperative_source_observes_viewport_and_timeout_cancellation() {
    use std::sync::atomic::Ordering;
    let _serial = SERIAL.lock().unwrap();
    struct Blocked {
        entered: AtomicBool,
        observed: AtomicBool,
    }
    impl Source for Blocked {
        fn length(&self) -> u64 {
            256
        }
        fn version(&self) -> Vec<u8> {
            vec![1]
        }
        fn validate(&self) -> io::Result<()> {
            Ok(())
        }
        fn read_at(&self, _: u64, _: u32) -> io::Result<Vec<u8>> {
            panic!("decoder lost cooperative cancellation")
        }
        fn read_at_cooperative(
            &self,
            _: u64,
            _: u32,
            cancelled: &dyn Fn() -> bool,
        ) -> io::Result<Vec<u8>> {
            self.entered.store(true, Ordering::Release);
            let deadline = std::time::Instant::now() + Duration::from_secs(4);
            while !cancelled() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(1));
            }
            assert!(cancelled(), "decoder stop did not reach in-flight source");
            assert_eq!(
                midnight_work_budget::shared_io().snapshot().available,
                0,
                "I/O capacity released before source cleanup"
            );
            self.observed.store(true, Ordering::Release);
            Err(io::ErrorKind::Interrupted.into())
        }
    }
    let helper = std::env::var_os("MIDNIGHT_IMAGE_TEST_HELPER").unwrap();
    for viewport in [true, false] {
        let source = Blocked {
            entered: AtomicBool::new(false),
            observed: AtomicBool::new(false),
        };
        let cancel = AtomicBool::new(false);
        let error = std::thread::scope(|scope| {
            if viewport {
                scope.spawn(|| {
                    let deadline = std::time::Instant::now() + Duration::from_secs(4);
                    while !source.entered.load(Ordering::Acquire)
                        && std::time::Instant::now() < deadline
                    {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                    cancel.store(true, Ordering::Release);
                });
            }
            decode::decode(
                std::path::Path::new(&helper),
                &source,
                Request {
                    id: 1,
                    source_length: 256,
                    width: 32,
                    height: 32,
                    frame: 0,
                },
                &cancel,
                Duration::from_secs(2),
            )
            .unwrap_err()
        });
        assert_eq!(
            error.kind(),
            if viewport {
                io::ErrorKind::Interrupted
            } else {
                io::ErrorKind::TimedOut
            }
        );
        assert!(source.entered.load(Ordering::Acquire));
        assert!(source.observed.load(Ordering::Acquire));
        assert_eq!(
            midnight_work_budget::shared_io().snapshot().available,
            midnight_work_budget::DEFAULT_IO_LIMIT
        );
    }
}
