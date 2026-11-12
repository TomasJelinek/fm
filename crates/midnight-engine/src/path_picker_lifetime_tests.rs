use super::*;
use std::thread::ThreadId;

struct RetiringPicker(std::sync::mpsc::Sender<ThreadId>);
impl Drop for RetiringPicker {
    fn drop(&mut self) {
        let _ = self.0.send(std::thread::current().id());
    }
}
impl PathPickerService for RetiringPicker {
    fn pick_path(
        &self,
        _: &PathPickerRequest,
        _: &Cancellation,
    ) -> Result<Option<PathBuf>, FsError> {
        unreachable!("idle controller must not open a chooser")
    }
}

#[test]
fn idle_picker_retires_its_final_service_off_caller() {
    let (notice, retired) = std::sync::mpsc::channel();
    let (picker, _events) = controller(Some(Arc::new(RetiringPicker(notice))));
    drop(picker);
    assert_ne!(
        retired.recv_timeout(Duration::from_secs(2)).unwrap(),
        std::thread::current().id(),
        "final picker service must retire outside the caller"
    );
}

#[tokio::test]
async fn dropping_picker_cancels_native_work_and_a_full_live_result_queue() {
    for wait in [false, true] {
        let service = Arc::new(FakePicker {
            calls: AtomicUsize::new(0),
            wait,
            bad: false,
        });
        let (mut picker, mut events) = controller(Some(service.clone()));
        for _ in 0..4 {
            picker
                .outbox
                .sender
                .try_send(Event::LocaleLoaded(Ok(Default::default())))
                .unwrap();
        }
        let cancellation = picker.shutdown.clone();
        let budget = picker.budget.clone();
        let mut tasks = JoinSet::new();
        picker.show(DesktopRequestId(77), input(), &mut tasks).await;
        tokio::time::timeout(Duration::from_secs(2), async {
            while service.calls.load(Ordering::SeqCst) == 0 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert_eq!(budget.available_permits(), 0);
        drop(picker);
        let cancelled_on_drop = cancellation.is_cancelled();
        // Release the old implementation too, so a RED assertion cannot hang teardown.
        cancellation.cancel();
        tokio::time::timeout(Duration::from_secs(2), tasks.join_next())
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert_eq!(budget.available_permits(), 1);
        for _ in 0..4 {
            assert!(matches!(events.try_recv(), Ok(Event::LocaleLoaded(_))));
        }
        assert!(
            events.try_recv().is_err(),
            "no reply after global cancellation"
        );
        assert!(
            cancelled_on_drop,
            "Controller Drop must cancel the native chooser/reply"
        );
    }
}
