//! Real Properties and serialized rejection share the exact six-slot desktop ledger.
use super::*;
use crate::captured_native::tests::{Gate, Native, Probe, Release, Source};
use crate::desktop_external_compare::{ExternalCompareNativeCapture, ExternalComparisonFrontDoor};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::sync::{Mutex, mpsc as sync};

struct Session;
impl ExternalCompareSessionCredit for Session {}
#[derive(Debug)]
struct Metadata;
impl ContentSource for Metadata {
    fn length(&self) -> u64 {
        0
    }
    fn display_name(&self) -> String {
        "metadata".into()
    }
    fn version(&self) -> Vec<u8> {
        vec![1]
    }
    fn validate(&self) -> Result<(), Error> {
        Ok(())
    }
    fn read_at(&self, _: u64, _: u32) -> Result<Vec<u8>, Error> {
        panic!("unexpected content read")
    }
    fn release_on_worker(self: Arc<Self>) -> Result<(), Error> {
        Ok(())
    }
}
#[derive(Debug)]
struct HeldIntake {
    entered: sync::Sender<()>,
    release: Mutex<sync::Receiver<()>>,
}
impl CapturedClipboardFiles for HeldIntake {
    fn intent(&self) -> FileClipboardIntent {
        FileClipboardIntent::Copy
    }
    fn resolve(&self, _: &Cancellation) -> Result<FileClipboard, FsError> {
        panic!("occupied Properties must reject before source resolution")
    }
}
impl Drop for HeldIntake {
    fn drop(&mut self) {
        let _ = self.entered.send(());
        let _ = self
            .release
            .lock()
            .unwrap()
            .recv_timeout(Duration::from_secs(5));
    }
}
struct ReleaseIntake(Option<sync::Sender<()>>);
impl Drop for ReleaseIntake {
    fn drop(&mut self) {
        if let Some(sender) = self.0.take() {
            let _ = sender.send(());
        }
    }
}

#[tokio::test]
async fn four_external_one_captured_properties_one_serial_rejection_use_all_six_slots() {
    let domain = crate::retirement::RetirementDomain::try_start(6, 1).unwrap();
    let front = ExternalComparisonFrontDoor::new(domain.clone());
    let gate = Arc::new(Gate::default());
    let release_properties = Release(gate.clone());
    let probe = Arc::new(Probe::default());
    let native = Arc::new(Native {
        gate: Some(gate.clone()),
        ..Native::new(probe.clone())
    });
    let (sender, _events) = mpsc::channel(64);
    let mut tasks = JoinSet::new();
    let desktop = Desktop::new(
        TaskLifetime::testing(native.clone()),
        Retirement {
            intake: domain.new_waiter().unwrap(),
            external: front.clone(),
        },
        Processes {
            terminals: ProcessTracker::try_start().unwrap(),
            openers: ProcessTracker::try_start_openers().unwrap(),
        },
        Services {
            desktop: Some(native.clone()),
            locale: None,
            capture: None,
        },
        Outbox {
            sender,
            wakeup: Arc::new(|| {}),
        },
        &mut tasks,
    );
    let root = std::env::current_dir().unwrap();
    let paths = [root.join("left.txt"), root.join("right.txt")];
    let program = FileComparatorProgram {
        executable: root.join("compare.exe"),
        arguments: vec!["{left}".into(), "{right}".into()],
        working_directory: None,
    };
    let capture: ExternalCompareNativeCapture =
        Arc::new(|_, _| Ok(CapturedFile::from_source(Arc::new(Metadata)).unwrap()));
    let mut requests = Vec::new();
    for _ in 0..4 {
        let (reply, _) = sync::sync_channel(1);
        requests.push(
            front
                .prepare(
                    program.process_for(&paths).unwrap(),
                    paths.clone(),
                    &capture,
                    Arc::new(Session),
                    Cancellation::default(),
                    reply,
                )
                .unwrap(),
        );
    }
    assert_eq!(
        desktop.open_budget.available_permits(),
        0,
        "same four-open semaphore"
    );
    desktop
        .properties_captured(
            native.clone(),
            DesktopRequestId(1),
            Arc::new(Source::new(vec![paths[0].clone()], probe)),
            Cancellation::default(),
            &mut tasks,
        )
        .await;
    gate.wait().await;
    assert_eq!(desktop.property_budget.available_permits(), 0);
    let (entered, entered_rx) = sync::channel();
    let (release, release_rx) = sync::channel();
    let release_intake = ReleaseIntake(Some(release));
    {
        let rejected = desktop.properties_captured(
            native,
            DesktopRequestId(2),
            Arc::new(HeldIntake {
                entered,
                release: Mutex::new(release_rx),
            }),
            Cancellation::default(),
            &mut tasks,
        );
        tokio::pin!(rejected);
        std::future::poll_fn(|context| {
            assert!(std::future::Future::poll(rejected.as_mut(), context).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        entered_rx.recv_timeout(Duration::from_secs(3)).unwrap();
        assert!(
            domain.try_reserve().is_err(),
            "real Properties + intake occupy the fifth/sixth slots"
        );
        let never_capture: ExternalCompareNativeCapture =
            Arc::new(|_, _| panic!("capture before open admission"));
        let (reply, _) = sync::sync_channel(1);
        assert!(
            front
                .prepare(
                    program.process_for(&paths).unwrap(),
                    paths,
                    &never_capture,
                    Arc::new(Session),
                    Cancellation::default(),
                    reply
                )
                .is_err()
        );
        drop(release_intake);
        rejected.await;
    }
    drop(release_properties);
    drop(requests);
    drop(desktop);
    // Background icon/context actors are unrelated to the six payload slots.
    tasks.abort_all();
    while tasks.join_next().await.is_some() {}
    tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let leases: Vec<_> = (0..6).filter_map(|_| domain.try_reserve().ok()).collect();
            if leases.len() == 6 {
                break;
            }
            drop(leases);
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
}
