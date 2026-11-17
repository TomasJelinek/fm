use super::*;

struct Residual(Status);
impl crate::copy_nested_retirement::CheckedRetirement for Residual {
    fn retirement_is_checked(&self) -> bool {
        true
    }
}
impl Drop for Residual {
    fn drop(&mut self) {
        let mut status = self.0.lock().unwrap();
        let path = status.destination_directory.join("retained-partial");
        retain_error(
            &mut status,
            error(
                &path,
                io::ErrorKind::PermissionDenied,
                "Partial cleanup refused; retained at this location.",
            ),
            true,
        );
    }
}

#[tokio::test]
async fn cancelled_input_keeps_ordinary_residual_reported_during_actual_teardown() {
    let admission = Admission::default();
    let capacity = admission.capacity();
    let status = Arc::new(Mutex::new(CopyProgress::queued(
        OperationId(1),
        Path::new("target").to_owned(),
        1,
        TransferIntent::Copy,
    )));
    let (mut owned, sender) = admission
        .reserve()
        .unwrap()
        .attach(Residual(status.clone()))
        .unwrap();
    owned.resources_mut().attach_status(status.clone());
    drop(sender);
    let result = retire_input(owned, cancelled(&status), &status).await;
    assert_eq!(result.state, CopyOperationState::Cancelled);
    assert_eq!(result.failed, 1);
    assert_eq!(result.errors.len(), 1);
    assert!(result.errors[0].path.ends_with("retained-partial"));
    assert!(result.message.contains("retained-partial"));
    assert_eq!(capacity.available_permits(), MAX_JOBS);
}
