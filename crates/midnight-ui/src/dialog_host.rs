//! Native dialog presentation lifetime; backend identities remain in typed adapters.
use crate::{dialog_focus::FocusReturn, dialog_native};
use slint::{ComponentHandle, PlatformError, winit_030::WinitWindowAccessor};
use std::{
    cell::{Cell, RefCell},
    rc::{Rc, Weak},
};

#[cfg(test)]
#[path = "dialog_host_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct DialogSession(u64);

#[derive(Default)]
pub(crate) struct DialogSessions {
    next: u64,
}
impl DialogSessions {
    pub(crate) fn allocate(&mut self) -> Result<DialogSession, String> {
        self.next = self
            .next
            .checked_add(1)
            .ok_or("Dialog session identities exhausted")?;
        Ok(DialogSession(self.next))
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum DialogModality {
    Modeless,
    OwnerModal,
}
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum CloseDisposition {
    KeepOpen,
    Hide,
}

type WindowAccess = dyn Fn(&mut dyn FnMut(&slint::Window));
type InputGate = Rc<dyn Fn(bool)>;
struct OwnerState {
    access: Box<WindowAccess>,
    leases: Cell<usize>,
    input_gate: RefCell<Option<InputGate>>,
    focus_fallback: RefCell<Option<Rc<dyn Fn()>>>,
}
thread_local! {
    static OWNERS: RefCell<Vec<Weak<OwnerState>>> = RefCell::default();
}

/// Weak component access, shared modal counts, and an optional Slint input gate.
#[derive(Clone)]
pub(crate) struct DialogOwner(Rc<OwnerState>);
impl DialogOwner {
    pub(crate) fn new<C: ComponentHandle + 'static>(component: &C) -> Self {
        if let Some(owner) = find_owner(component.window()) {
            return Self(owner);
        }
        let weak = component.as_weak();
        let state = Rc::new(OwnerState {
            access: Box::new(move |callback| {
                if let Some(component) = weak.upgrade() {
                    callback(component.window());
                }
            }),
            leases: Cell::new(0),
            input_gate: RefCell::new(None),
            focus_fallback: RefCell::new(None),
        });
        OWNERS.with(|owners| owners.borrow_mut().push(Rc::downgrade(&state)));
        Self(state)
    }

    pub(crate) fn with_window<T>(&self, f: impl FnOnce(&slint::Window) -> T) -> Option<T> {
        let mut callback = Some(f);
        let mut result = None;
        (self.0.access)(&mut |window| {
            if let Some(callback) = callback.take() {
                result = Some(callback(window));
            }
        });
        result
    }

    /// The callback must capture its component weakly. It gates Slint pointer and keyboard input.
    pub(crate) fn set_input_gate(&self, gate: impl Fn(bool) + 'static) {
        let gate: Rc<dyn Fn(bool)> = Rc::new(gate);
        *self.0.input_gate.borrow_mut() = Some(gate.clone());
        gate(self.0.leases.get() > 0);
    }

    /// Register a safe default focus action, capturing its component weakly.
    pub(crate) fn set_focus_fallback(&self, fallback: impl Fn() + 'static) {
        *self.0.focus_fallback.borrow_mut() = Some(Rc::new(fallback));
    }

    /// True once the live owner can own a native dialog (or a visible software test adapter).
    pub(crate) fn is_ready(&self) -> bool {
        self.with_window(|window| {
            window.is_visible() && (!dialog_native::uses_native_hook() || window.has_winit_window())
        }) == Some(true)
    }

    fn set_blocked(&self, blocked: bool) {
        self.with_window(|window| {
            window.with_winit_window(|native| dialog_native::set_enabled(native, !blocked));
        });
        // Release the RefCell borrow before invoking application callbacks.
        let gate = self.0.input_gate.borrow().clone();
        if let Some(gate) = gate {
            gate(blocked);
        }
    }

    fn acquire(&self) -> Result<bool, PlatformError> {
        let count = self.0.leases.get();
        let next = count.checked_add(1).ok_or("Too many modal dialogs")?;
        self.0.leases.set(next);
        Ok(count == 0)
    }

    fn release(&self) {
        let count = self.0.leases.get();
        if count == 0 {
            return;
        }
        self.0.leases.set(count - 1);
        if count == 1 {
            self.set_blocked(false);
        }
    }
}

fn find_owner(window: &slint::Window) -> Option<Rc<OwnerState>> {
    let candidates = OWNERS.with(|owners| {
        let mut owners = owners.borrow_mut();
        owners.retain(|owner| owner.strong_count() > 0);
        owners.iter().filter_map(Weak::upgrade).collect::<Vec<_>>()
    });
    candidates.into_iter().find(|owner| {
        let mut matched = false;
        (owner.access)(&mut |candidate| matched = std::ptr::eq(window, candidate));
        matched
    })
}

/// Consult before native shortcut handling; Slint controls also use the owner's input gate.
pub(crate) fn dialog_input_blocked(window: &slint::Window) -> bool {
    find_owner(window).is_some_and(|owner| owner.leases.get() > 0)
}

pub(crate) struct DialogHost {
    session: DialogSession,
    owner: DialogOwner,
    child: DialogOwner,
    modality: DialogModality,
    life: Rc<DialogLife>,
    focus: FocusReturn,
    role: dialog_native::WindowRole,
    owner_window_id: Option<slint::winit_030::winit::window::WindowId>,
}

struct DialogLife {
    open: Cell<bool>,
    shown: Cell<bool>,
    leased: Cell<bool>,
    realized: Cell<bool>,
    readiness: slint::Timer,
}

#[derive(Clone, Copy)]
struct InitialGeometry {
    position: slint::PhysicalPosition,
    size: slint::PhysicalSize,
}
impl InitialGeometry {
    fn capture(child: &slint::Window) -> Self {
        Self {
            position: child.position(),
            size: child.size(),
        }
    }
    fn unchanged(&self, child: &slint::Window) -> bool {
        self.position == child.position() && self.size == child.size()
    }
}

struct Showing<'a> {
    host: &'a DialogHost,
    child: &'a slint::Window,
    complete: bool,
}
impl Drop for Showing<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.host.close(self.child);
        }
    }
}

impl DialogHost {
    pub(crate) fn create<C: ComponentHandle + 'static>(
        session: DialogSession,
        owner: DialogOwner,
        modality: DialogModality,
        create: impl FnOnce() -> Result<C, PlatformError>,
    ) -> Result<(C, Self), PlatformError> {
        Self::create_with_role(
            session,
            owner,
            modality,
            dialog_native::WindowRole::Dialog,
            create,
        )
    }

    /// Working tools retain a taskbar entry and a logical owner, while remaining modeless.
    pub(crate) fn create_tool<C: ComponentHandle + 'static>(
        session: DialogSession,
        owner: DialogOwner,
        create: impl FnOnce() -> Result<C, PlatformError>,
    ) -> Result<(C, Self), PlatformError> {
        Self::create_with_role(
            session,
            owner,
            DialogModality::Modeless,
            dialog_native::WindowRole::WorkingTool,
            create,
        )
    }

    fn create_with_role<C: ComponentHandle + 'static>(
        session: DialogSession,
        owner: DialogOwner,
        modality: DialogModality,
        role: dialog_native::WindowRole,
        create: impl FnOnce() -> Result<C, PlatformError>,
    ) -> Result<(C, Self), PlatformError> {
        if !owner.is_ready() {
            return Err("The dialog owner is closed or has no native window".into());
        }
        let focus = owner.with_window(FocusReturn::capture).unwrap_or_default();
        let owner_window_id = owner
            .with_window(|window| window.with_winit_window(|native| native.id()))
            .flatten();
        // Slint snapshots the attributes during adapter construction, before later realization.
        let _scope = dialog_native::CreationScope::enter(owner.clone(), role);
        let component = create()?;
        midnight_ui_input::install_native_scroll(&component, dialog_input_blocked);
        let host = Self {
            session,
            owner,
            child: DialogOwner::new(&component),
            modality,
            life: Rc::new(DialogLife {
                open: Cell::new(true),
                shown: Cell::new(false),
                leased: Cell::new(false),
                realized: Cell::new(false),
                readiness: slint::Timer::default(),
            }),
            focus,
            role,
            owner_window_id,
        };
        Ok((component, host))
    }

    pub(crate) fn show(&self, child: &slint::Window) -> Result<(), PlatformError> {
        if !self.life.open.get() || !self.owner.is_ready() {
            return Err("The dialog or its owner is closed".into());
        }
        let current_owner = self
            .owner
            .with_window(|window| window.with_winit_window(|native| native.id()))
            .flatten();
        if self.role == dialog_native::WindowRole::Dialog && current_owner != self.owner_window_id {
            return Err("The dialog owner's native window was replaced".into());
        }
        if self.life.shown.get() {
            return Ok(());
        }
        let mut showing = Showing {
            host: self,
            child,
            complete: false,
        };
        let _scope = dialog_native::CreationScope::enter(self.owner.clone(), self.role);
        self.owner
            .with_window(|owner| dialog_native::place_initial(owner, child));
        if let Err(error) = child.show() {
            self.close(child);
            return Err(error);
        }
        if self.modality == DialogModality::OwnerModal {
            let first_lease = self.owner.acquire()?;
            self.life.leased.set(true);
            if first_lease {
                self.owner.set_blocked(true);
            }
        }
        self.life.shown.set(true);
        self.finish_or_schedule_realization(child);
        showing.complete = true;
        Ok(())
    }

    fn finish_or_schedule_realization(&self, child: &slint::Window) {
        let initial_geometry = InitialGeometry::capture(child);
        if finish_realization(&self.life, &self.owner, child, self.role, initial_geometry) {
            return;
        }
        let weak_life = Rc::downgrade(&self.life);
        let owner = self.owner.clone();
        let child = self.child.clone();
        let role = self.role;
        let owner_window_id = self.owner_window_id;
        // Slint can realize a Winit window only in a later event-loop turn. The
        // callback never shows a window and owns no component or native Window Arc.
        self.life.readiness.start(
            slint::TimerMode::Repeated,
            std::time::Duration::from_millis(16),
            move || {
                let Some(life) = weak_life.upgrade() else {
                    return;
                };
                if !life.open.get() || !life.shown.get() {
                    life.readiness.stop();
                    return;
                }
                let current_owner = owner
                    .with_window(|window| window.with_winit_window(|native| native.id()))
                    .flatten();
                let owner_changed =
                    role == dialog_native::WindowRole::Dialog && current_owner != owner_window_id;
                let valid = owner.is_ready() && !owner_changed;
                if child
                    .with_window(|window| {
                        if !valid || !window.is_visible() {
                            close_presentation(&life, &owner, window);
                        } else {
                            finish_realization(&life, &owner, window, role, initial_geometry);
                        }
                    })
                    .is_none()
                {
                    life.readiness.stop();
                    life.open.set(false);
                    life.shown.set(false);
                    if life.leased.replace(false) {
                        owner.release();
                    }
                }
            },
        );
    }

    /// Background-safe closure. Explicit user dismissal uses `request_close` for focus return.
    pub(crate) fn close(&self, child: &slint::Window) {
        close_presentation(&self.life, &self.owner, child);
    }

    /// Explicit user reactivation only; ordinary refreshes must continue to call `show`.
    pub(crate) fn activate(&self, child: &slint::Window) -> bool {
        self.is_realized()
            && self.life.shown.get()
            && child.is_visible()
            && !dialog_input_blocked(child)
            && dialog_native::activate(child)
    }

    pub(crate) fn request_close(
        &self,
        child: &slint::Window,
        semantic_close: impl FnOnce() -> CloseDisposition,
    ) {
        if !self.life.open.get() {
            return;
        }
        let focused = dialog_native::has_focus(child);
        if semantic_close() == CloseDisposition::Hide {
            self.close(child);
            if focused && self.owner.0.leases.get() == 0 {
                self.owner.with_window(|owner| {
                    if owner.is_visible() {
                        dialog_native::activate(owner);
                        if !self.focus.restore(owner) {
                            let fallback = self.owner.0.focus_fallback.borrow().clone();
                            if let Some(fallback) = fallback {
                                fallback();
                            }
                        }
                    }
                });
            }
        }
    }

    pub(crate) fn is_current(&self, session: DialogSession) -> bool {
        self.life.open.get() && self.session == session
    }

    /// The first native placement/ownership pass has completed for this live occurrence.
    pub(crate) fn is_realized(&self) -> bool {
        self.life.open.get() && self.life.realized.get()
    }
}

fn finish_realization(
    life: &DialogLife,
    owner: &DialogOwner,
    child: &slint::Window,
    role: dialog_native::WindowRole,
    initial_geometry: InitialGeometry,
) -> bool {
    if life.realized.get() {
        return true;
    }
    if !life.open.get()
        || !child.is_visible()
        || (dialog_native::uses_native_hook() && !child.has_winit_window())
    {
        return false;
    }
    life.realized.set(true);
    life.readiness.stop();
    owner.with_window(|owner| {
        // A user/compositor move or resize after show owns that geometry. Never
        // overwrite it merely because the native adapter became available later.
        if initial_geometry.unchanged(child) {
            dialog_native::place_realized(owner, child);
        }
        dialog_native::attach(owner, child, role);
        owner.with_winit_window(|native| {
            dialog_native::set_enabled(native, !dialog_input_blocked(owner));
        });
    });
    true
}

fn close_presentation(life: &DialogLife, owner: &DialogOwner, child: &slint::Window) {
    life.readiness.stop();
    life.open.set(false);
    life.shown.set(false);
    if life.leased.replace(false) {
        // Windows requires enabling the owner before its last modal child is hidden.
        owner.release();
    }
    let _ = child.hide();
}

impl Drop for DialogHost {
    fn drop(&mut self) {
        self.child.with_window(|child| self.close(child));
        // The component may already have been destroyed.
        self.life.readiness.stop();
        self.life.open.set(false);
        if self.life.leased.replace(false) {
            self.owner.release();
        }
    }
}
