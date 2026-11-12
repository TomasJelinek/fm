//! UI lifetime tests use real Slint windows and a controllable display boundary.
use super::*;
use slint::platform::{
    Platform, WindowAdapter,
    software_renderer::{RepaintBufferType, SoftwareRenderer},
};

#[derive(Default)]
struct Display {
    fail_show: Cell<bool>,
    adapters: RefCell<Vec<std::rc::Weak<Adapter>>>,
}

struct Adapter {
    window: slint::Window,
    renderer: SoftwareRenderer,
    display: Rc<Display>,
}

impl WindowAdapter for Adapter {
    fn window(&self) -> &slint::Window {
        &self.window
    }
    fn renderer(&self) -> &dyn slint::platform::Renderer {
        &self.renderer
    }
    fn size(&self) -> slint::PhysicalSize {
        slint::PhysicalSize::new(640, 480)
    }
    fn set_visible(&self, visible: bool) -> Result<(), slint::PlatformError> {
        if visible && self.display.fail_show.get() {
            return Err("injected display failure".into());
        }
        Ok(())
    }
}

struct PlatformBoundary(Rc<Display>);
impl Platform for PlatformBoundary {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        let adapter = Rc::new_cyclic(|weak: &std::rc::Weak<Adapter>| Adapter {
            window: slint::Window::new(weak.clone()),
            renderer: SoftwareRenderer::new_with_repaint_buffer_type(RepaintBufferType::NewBuffer),
            display: self.0.clone(),
        });
        self.0.adapters.borrow_mut().push(Rc::downgrade(&adapter));
        Ok(adapter)
    }
}

fn ui(test: impl FnOnce(Rc<Display>) + Send + 'static) {
    std::thread::spawn(move || {
        let display = Rc::new(Display::default());
        slint::platform::set_platform(Box::new(PlatformBoundary(display.clone()))).unwrap();
        test(display.clone());
        let adapters = display
            .adapters
            .borrow()
            .iter()
            .filter_map(std::rc::Weak::upgrade)
            .collect::<Vec<_>>();
        for adapter in adapters {
            adapter.window.hide().unwrap();
        }
    })
    .join()
    .unwrap();
}

#[test]
fn exhausted_sessions_never_reuse_an_identity() {
    let mut sessions = DialogSessions { next: u64::MAX - 1 };
    let final_session = sessions.allocate().unwrap();
    assert_eq!(final_session, DialogSession(u64::MAX));
    assert!(sessions.allocate().is_err());
    assert!(sessions.allocate().is_err());
}

#[test]
fn nested_modal_lease_restores_only_after_last_child() {
    ui(|_| {
        let main = crate::MainWindow::new().unwrap();
        main.show().unwrap();
        let owner = DialogOwner::new(&main);
        let gates = Rc::new(RefCell::new(Vec::new()));
        let changes = gates.clone();
        owner.set_input_gate(move |blocked| changes.borrow_mut().push(blocked));
        let mut sessions = DialogSessions::default();
        let (first, first_host) = DialogHost::create(
            sessions.allocate().unwrap(),
            owner.clone(),
            DialogModality::OwnerModal,
            crate::MainWindow::new,
        )
        .unwrap();
        let (second, second_host) = DialogHost::create(
            sessions.allocate().unwrap(),
            DialogOwner::new(&main),
            DialogModality::OwnerModal,
            crate::MainWindow::new,
        )
        .unwrap();
        first_host.show(first.window()).unwrap();
        second_host.show(second.window()).unwrap();
        assert!(dialog_input_blocked(main.window()));
        first_host.close(first.window());
        assert!(dialog_input_blocked(main.window()));
        assert_eq!(&*gates.borrow(), &[false, true]);
        second_host.close(second.window());
        assert!(!dialog_input_blocked(main.window()));
        assert_eq!(&*gates.borrow(), &[false, true, false]);
    });
}

#[test]
fn show_failure_releases_modal_owner() {
    ui(|display| {
        let main = crate::MainWindow::new().unwrap();
        main.show().unwrap();
        let mut sessions = DialogSessions::default();
        let (child, host) = DialogHost::create(
            sessions.allocate().unwrap(),
            DialogOwner::new(&main),
            DialogModality::OwnerModal,
            crate::MainWindow::new,
        )
        .unwrap();
        display.fail_show.set(true);
        assert!(host.show(child.window()).is_err());
        assert!(!dialog_input_blocked(main.window()));
        assert!(!child.window().is_visible());
    });
}

#[test]
fn expired_or_hidden_owner_cannot_open_a_child() {
    ui(|_| {
        let main = crate::MainWindow::new().unwrap();
        main.show().unwrap();
        let owner = DialogOwner::new(&main);
        main.hide().unwrap();
        let mut sessions = DialogSessions::default();
        assert!(
            DialogHost::create(
                sessions.allocate().unwrap(),
                owner.clone(),
                DialogModality::OwnerModal,
                crate::MainWindow::new,
            )
            .is_err()
        );
        drop(main);
        assert!(owner.with_window(|_| ()).is_none());
    });
}

#[test]
fn close_semantics_can_reject_then_accept_without_repeating() {
    ui(|_| {
        let main = crate::MainWindow::new().unwrap();
        main.show().unwrap();
        let mut sessions = DialogSessions::default();
        let session = sessions.allocate().unwrap();
        let (child, host) = DialogHost::create(
            session,
            DialogOwner::new(&main),
            DialogModality::OwnerModal,
            crate::MainWindow::new,
        )
        .unwrap();
        host.show(child.window()).unwrap();
        host.request_close(child.window(), || CloseDisposition::KeepOpen);
        assert!(child.window().is_visible());
        assert!(host.is_current(session));
        host.request_close(child.window(), || CloseDisposition::Hide);
        assert!(!child.window().is_visible());
        assert!(!host.is_current(session));
        host.request_close(child.window(), || {
            panic!("closed session called semantic action")
        });
    });
}

#[test]
fn dropped_host_releases_owner_even_without_a_close_callback() {
    ui(|_| {
        let main = crate::MainWindow::new().unwrap();
        main.show().unwrap();
        let mut sessions = DialogSessions::default();
        let (child, host) = DialogHost::create(
            sessions.allocate().unwrap(),
            DialogOwner::new(&main),
            DialogModality::OwnerModal,
            crate::MainWindow::new,
        )
        .unwrap();
        host.show(child.window()).unwrap();
        assert!(dialog_input_blocked(main.window()));
        drop(host);
        assert!(!dialog_input_blocked(main.window()));
    });
}

#[test]
fn only_explicit_focused_close_uses_the_owner_focus_fallback() {
    ui(|_| {
        let main = crate::MainWindow::new().unwrap();
        main.show().unwrap();
        // No invoking item survives: this exercises the adapter's safe default.
        i_slint_core::window::WindowInner::from_pub(main.window())
            .focus_item
            .take();
        let owner = DialogOwner::new(&main);
        let calls = Rc::new(Cell::new(0));
        let observed = calls.clone();
        owner.set_focus_fallback(move || observed.set(observed.get() + 1));
        let mut sessions = DialogSessions::default();
        let (child, host) = DialogHost::create(
            sessions.allocate().unwrap(),
            owner.clone(),
            DialogModality::Modeless,
            crate::MainWindow::new,
        )
        .unwrap();
        host.show(child.window()).unwrap();
        child
            .window()
            .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
        host.request_close(child.window(), || CloseDisposition::Hide);
        assert_eq!(calls.get(), 1);

        let (child, host) = DialogHost::create(
            sessions.allocate().unwrap(),
            owner,
            DialogModality::Modeless,
            crate::MainWindow::new,
        )
        .unwrap();
        host.show(child.window()).unwrap();
        child
            .window()
            .dispatch_event(slint::platform::WindowEvent::WindowActiveChanged(true));
        host.close(child.window());
        assert_eq!(calls.get(), 1, "background close restored foreground focus");
    });
}

#[test]
fn unwinding_during_modal_presentation_releases_the_owner_and_child() {
    ui(|_| {
        let main = crate::MainWindow::new().unwrap();
        main.show().unwrap();
        let owner = DialogOwner::new(&main);
        owner.set_input_gate(|blocked| {
            assert!(!blocked, "injected presentation callback unwind");
        });
        let mut sessions = DialogSessions::default();
        let (child, host) = DialogHost::create(
            sessions.allocate().unwrap(),
            owner,
            DialogModality::OwnerModal,
            crate::MainWindow::new,
        )
        .unwrap();
        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| host.show(child.window())));
        assert!(result.is_err());
        assert!(!dialog_input_blocked(main.window()));
        assert!(!child.window().is_visible());
    });
}

#[test]
fn invoking_item_focus_returns_only_to_its_live_enabled_owner() {
    ui(|_| {
        let main = crate::MainWindow::new().unwrap();
        main.show().unwrap();
        main.invoke_set_command("draft".into(), 2);
        let inner = i_slint_core::window::WindowInner::from_pub(main.window());
        let invoking = inner.focus_item.borrow().upgrade().unwrap();
        let saved = FocusReturn::capture(main.window());
        main.invoke_focus_files();
        assert!(saved.restore(main.window()));
        assert!(
            inner
                .focus_item
                .borrow()
                .upgrade()
                .is_some_and(|focused| focused == invoking)
        );
        let other = crate::MainWindow::new().unwrap();
        other.show().unwrap();
        assert!(
            !saved.restore(other.window()),
            "focus crossed its owner boundary"
        );
        let mut sessions = DialogSessions::default();
        let (modal, host) = DialogHost::create(
            sessions.allocate().unwrap(),
            DialogOwner::new(&main),
            DialogModality::OwnerModal,
            crate::MainWindow::new,
        )
        .unwrap();
        host.show(modal.window()).unwrap();
        assert!(
            !saved.restore(main.window()),
            "blocked owner regained invoking focus"
        );
        host.close(modal.window());
        assert!(saved.restore(main.window()));
        drop(modal);
        drop(host);
        drop(invoking);
        let weak = main.as_weak();
        main.hide().unwrap();
        drop(main);
        assert!(
            weak.upgrade().is_none(),
            "saved focus retained the closed owner"
        );
        assert!(!saved.restore(other.window()));
    });
}

#[test]
fn closing_before_first_show_never_reopens_the_pending_child() {
    ui(|_| {
        let main = crate::MainWindow::new().unwrap();
        main.show().unwrap();
        let mut sessions = DialogSessions::default();
        let (child, host) = DialogHost::create(
            sessions.allocate().unwrap(),
            DialogOwner::new(&main),
            DialogModality::OwnerModal,
            crate::MainWindow::new,
        )
        .unwrap();
        let weak = child.as_weak();
        host.close(child.window());
        assert!(host.show(child.window()).is_err());
        assert!(!host.activate(child.window()));
        assert!(!child.window().is_visible());
        assert!(!dialog_input_blocked(main.window()));
        drop(child);
        drop(host);
        assert!(weak.upgrade().is_none());
    });
}
