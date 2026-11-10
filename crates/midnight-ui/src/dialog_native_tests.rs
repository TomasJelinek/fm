use super::*;
use slint::{
    ComponentHandle,
    platform::{
        Platform, WindowAdapter,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::rc::Rc;

struct SoftwareBoundary;
impl Platform for SoftwareBoundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer))
    }
}

#[test]
fn initial_dialog_can_extend_beyond_owner_and_stays_in_monitor_work_area() {
    let (position, size) =
        centered_bounds((-1850, 80, 500, 400), (900, 600), (-1920, 0, 1920, 1040));
    assert_eq!(position, (-1920, 0));
    assert_eq!(size, (900, 600));
}

#[test]
fn oversized_preferred_size_is_limited_to_monitor_not_owner() {
    let (position, size) = centered_bounds((200, 100, 640, 480), (2000, 1500), (0, 0, 1280, 720));
    assert_eq!(position, (0, 0));
    assert_eq!(size, (1280, 720));
}

#[test]
fn client_size_reserves_actual_decoration_extents_inside_work_area() {
    let (position, client) =
        decorated_bounds((100, 100, 400, 300), (900, 650), (16, 39), (0, 0, 800, 600));
    assert_eq!(position, (0, 0));
    assert_eq!(client, (784, 561));
    let (position, client) = decorated_bounds(
        (-1850, 100, 400, 300),
        (400, 240),
        (24, 58),
        (-1920, 0, 1920, 1040),
    );
    assert_eq!(position, (-1862, 101));
    assert_eq!(client, (400, 240));
}

#[test]
fn failed_or_unwinding_creation_never_leaks_its_native_owner_scope() {
    std::thread::spawn(|| {
        slint::platform::set_platform(Box::new(SoftwareBoundary)).unwrap();
        let main = crate::MainWindow::new().unwrap();
        main.show().unwrap();
        let mut sessions = crate::dialog_host::DialogSessions::default();
        let result = crate::dialog_host::DialogHost::create::<crate::MainWindow>(
            sessions.allocate().unwrap(),
            DialogOwner::new(&main),
            crate::dialog_host::DialogModality::Modeless,
            || Err("injected construction failure".into()),
        );
        assert!(result.is_err());
        assert!(CREATION_OWNER.with(|owner| owner.borrow().is_none()));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            crate::dialog_host::DialogHost::create::<crate::MainWindow>(
                sessions.allocate().unwrap(),
                DialogOwner::new(&main),
                crate::dialog_host::DialogModality::Modeless,
                || panic!("injected construction unwind"),
            )
        }));
        assert!(result.is_err());
        assert!(CREATION_OWNER.with(|owner| owner.borrow().is_none()));
        main.hide().unwrap();
    })
    .join()
    .unwrap();
}
