//! Slint 1.18.1 host coordinates for embedded, input-transparent tooltip popups.
//!
//! Winit uses the default create_child_window_adapter (None), so every app popup
//! shares its host coordinates. Recheck this contract when upgrading Slint or
//! adding a backend with native popup windows. This observer never consumes input.
use crate::{
    AboutWindow, ChecksumWindow, DecisionWindow, FileListWindow, FindWindow, HelpWindow,
    HotPathsWindow, JobProgressWindow, JobsWindow, MainWindow, NoticeWindow, OperationWindow,
    PanelFilterWindow, TooltipPlacement,
};
use i_slint_core::{
    items::TooltipArea,
    lengths::LogicalLength,
    window::{WindowInner, WindowKind},
};
use slint::{
    ComponentHandle, PlatformError,
    platform::{PointerEventButton, WindowEvent},
};
use std::cell::RefCell;

#[cfg(test)]
#[path = "tooltip_tests.rs"]
mod tests;

#[derive(Default)]
struct Registry {
    installed: bool,
    targets: Vec<Target>,
}
thread_local! { static REGISTRY: RefCell<Registry> = RefCell::default(); }

#[derive(Clone)]
enum Target {
    Main(slint::Weak<MainWindow>),
    Find(slint::Weak<FindWindow>),
    Checksum(slint::Weak<ChecksumWindow>),
    FileList(slint::Weak<FileListWindow>),
    Help(slint::Weak<HelpWindow>),
    About(slint::Weak<AboutWindow>),
    Notice(slint::Weak<NoticeWindow>),
    Operation(slint::Weak<OperationWindow>),
    Jobs(slint::Weak<JobsWindow>),
    JobProgress(slint::Weak<JobProgressWindow>),
    Decision(slint::Weak<DecisionWindow>),
    PanelFilter(slint::Weak<PanelFilterWindow>),
    HotPaths(slint::Weak<HotPathsWindow>),
}
enum Host {
    Main(MainWindow),
    Find(FindWindow),
    Checksum(ChecksumWindow),
    FileList(FileListWindow),
    Help(HelpWindow),
    About(AboutWindow),
    Notice(NoticeWindow),
    Operation(OperationWindow),
    Jobs(JobsWindow),
    JobProgress(JobProgressWindow),
    Decision(DecisionWindow),
    PanelFilter(PanelFilterWindow),
    HotPaths(HotPathsWindow),
}
impl Target {
    fn upgrade(&self) -> Option<Host> {
        match self {
            Self::Main(window) => window.upgrade().map(Host::Main),
            Self::Find(window) => window.upgrade().map(Host::Find),
            Self::Checksum(window) => window.upgrade().map(Host::Checksum),
            Self::FileList(window) => window.upgrade().map(Host::FileList),
            Self::Help(window) => window.upgrade().map(Host::Help),
            Self::About(window) => window.upgrade().map(Host::About),
            Self::Notice(window) => window.upgrade().map(Host::Notice),
            Self::Operation(window) => window.upgrade().map(Host::Operation),
            Self::Jobs(window) => window.upgrade().map(Host::Jobs),
            Self::JobProgress(window) => window.upgrade().map(Host::JobProgress),
            Self::Decision(window) => window.upgrade().map(Host::Decision),
            Self::PanelFilter(window) => window.upgrade().map(Host::PanelFilter),
            Self::HotPaths(window) => window.upgrade().map(Host::HotPaths),
        }
    }
}
impl Host {
    fn window(&self) -> &slint::Window {
        match self {
            Self::Main(window) => window.window(),
            Self::Find(window) => window.window(),
            Self::Checksum(window) => window.window(),
            Self::FileList(window) => window.window(),
            Self::Help(window) => window.window(),
            Self::About(window) => window.window(),
            Self::Notice(window) => window.window(),
            Self::Operation(window) => window.window(),
            Self::Jobs(window) => window.window(),
            Self::JobProgress(window) => window.window(),
            Self::Decision(window) => window.window(),
            Self::PanelFilter(window) => window.window(),
            Self::HotPaths(window) => window.window(),
        }
    }
    fn placement(&self) -> TooltipPlacement<'_> {
        match self {
            Self::Main(window) => window.global::<TooltipPlacement>(),
            Self::Find(window) => window.global::<TooltipPlacement>(),
            Self::Checksum(window) => window.global::<TooltipPlacement>(),
            Self::FileList(window) => window.global::<TooltipPlacement>(),
            Self::Help(window) => window.global::<TooltipPlacement>(),
            Self::About(window) => window.global::<TooltipPlacement>(),
            Self::Notice(window) => window.global::<TooltipPlacement>(),
            Self::Operation(window) => window.global::<TooltipPlacement>(),
            Self::Jobs(window) => window.global::<TooltipPlacement>(),
            Self::JobProgress(window) => window.global::<TooltipPlacement>(),
            Self::Decision(window) => window.global::<TooltipPlacement>(),
            Self::PanelFilter(window) => window.global::<TooltipPlacement>(),
            Self::HotPaths(window) => window.global::<TooltipPlacement>(),
        }
    }
    fn synchronize(&self) -> bool {
        synchronize_host(self.window(), self.placement())
    }
}

pub(crate) fn install(window: &MainWindow) -> Result<(), PlatformError> {
    register(Target::Main(window.as_weak()))
}
pub(crate) fn install_find(window: &FindWindow) -> Result<(), PlatformError> {
    register(Target::Find(window.as_weak()))
}
pub(crate) fn install_checksum(window: &ChecksumWindow) -> Result<(), PlatformError> {
    register(Target::Checksum(window.as_weak()))
}
pub(crate) fn install_file_list(window: &FileListWindow) -> Result<(), PlatformError> {
    register(Target::FileList(window.as_weak()))
}
pub(crate) fn install_help(window: &HelpWindow) -> Result<(), PlatformError> {
    register(Target::Help(window.as_weak()))
}
pub(crate) fn install_about(window: &AboutWindow) -> Result<(), PlatformError> {
    register(Target::About(window.as_weak()))
}
pub(crate) fn install_operation(window: &OperationWindow) -> Result<(), PlatformError> {
    register(Target::Operation(window.as_weak()))
}
pub(crate) fn install_job_progress(window: &JobProgressWindow) -> Result<(), PlatformError> {
    register(Target::JobProgress(window.as_weak()))
}
pub(crate) fn install_jobs(window: &JobsWindow) -> Result<(), PlatformError> {
    register(Target::Jobs(window.as_weak()))
}
pub(crate) fn install_decision(window: &DecisionWindow) -> Result<(), PlatformError> {
    register(Target::Decision(window.as_weak()))
}
pub(crate) fn install_notice(window: &NoticeWindow) -> Result<(), PlatformError> {
    register(Target::Notice(window.as_weak()))
}
pub(crate) fn synchronize_file_list(window: &FileListWindow) {
    synchronize_host(window.window(), window.global::<TooltipPlacement>());
}

pub(crate) fn synchronize_checksum(window: &ChecksumWindow) {
    synchronize_host(window.window(), window.global::<TooltipPlacement>());
}
fn register(target: Target) -> Result<(), PlatformError> {
    let Some(host) = target.upgrade() else {
        return Ok(());
    };
    let callback = target.clone();
    host.placement()
        .on_synchronize(move || callback.upgrade().is_some_and(|host| host.synchronize()));
    let first = REGISTRY.with(|registry| {
        let mut registry = registry.borrow_mut();
        registry.targets.retain(|target| target.upgrade().is_some());
        if !registry
            .targets
            .iter()
            .filter_map(Target::upgrade)
            .any(|target| std::ptr::eq(target.window(), host.window()))
        {
            registry.targets.push(target);
        }
        !registry.installed
    });
    if !first {
        return Ok(());
    }
    let previous = i_slint_core::context::set_window_event_hook(None)?;
    i_slint_core::context::set_window_event_hook(Some(Box::new(move |adapter, event, result| {
        // Drop the registry borrow before evaluating any Slint properties.
        let targets = REGISTRY.with(|registry| {
            let mut registry = registry.borrow_mut();
            let mut targets = Vec::with_capacity(registry.targets.len());
            registry.targets.retain(|target| {
                if let Some(target) = target.upgrade() {
                    targets.push(target);
                    true
                } else {
                    false
                }
            });
            targets
        });
        for host in targets {
            if !std::ptr::eq(adapter.window(), host.window()) {
                continue;
            }
            // Flickable may defer child pointer-down by 100ms. Preserve the footer's
            // actual press action/pane before modifiers or focus can change.
            if let Host::Main(window) = &host {
                match event {
                    WindowEvent::PointerPressed {
                        position,
                        button: PointerEventButton::Left,
                    } => {
                        window.invoke_capture_function_press(position.x, position.y);
                    }
                    WindowEvent::PointerReleased {
                        button: PointerEventButton::Left,
                        ..
                    }
                    | WindowEvent::PointerExited
                    | WindowEvent::WindowActiveChanged(false) => {
                        window.invoke_release_function_press();
                    }
                    _ => {}
                }
            }
            let global = host.placement();
            let size = match event {
                WindowEvent::Resized { size } => *size,
                _ => adapter.size().to_logical(host.window().scale_factor()),
            };
            if size.width > 0.0 && size.height > 0.0 {
                global.set_surface_width(size.width);
                global.set_surface_height(size.height);
            }
            match event {
                WindowEvent::PointerMoved { position }
                | WindowEvent::PointerPressed { position, .. }
                | WindowEvent::PointerReleased { position, .. }
                | WindowEvent::PointerScrolled { position, .. } => {
                    global.set_cursor_x(position.x);
                    global.set_cursor_y(position.y);
                }
                WindowEvent::PointerExited => {
                    global.set_cursor_x(-1.0);
                    global.set_cursor_y(-1.0);
                }
                _ => {}
            }
            host.synchronize();
            if let Host::Main(window) = &host {
                crate::panel_pointer::observe(window, event);
                crate::drag_bridge::observe(window, event);
            }
        }
        if let Some(previous) = previous.as_ref() {
            previous(adapter, event, result);
        }
    })))?;
    REGISTRY.with(|registry| registry.borrow_mut().installed = true);
    Ok(())
}

/// Refresh only active anchors after input or a presentation layout update.
/// The show callback also calls this after its first stock placement; a true
/// result requests one stock retry before the first paint, preserving ownership.
pub(crate) fn synchronize(window: &MainWindow) -> bool {
    synchronize_host(window.window(), window.global::<TooltipPlacement>())
}

/// Retire hints before a structural UI change destroys their owning controls.
/// Slint 1.18 materializes popup children while borrowing its popup list; a
/// delegate destructor must not try to close a live tooltip during that borrow.
pub(crate) fn dismiss(window: &slint::Window) {
    // Also retire hover observers and their pending timers. Closing only visible
    // popups lets a due timer recreate a hint before the next layout pass.
    window.dispatch_event(WindowEvent::PointerExited);
    let inner = WindowInner::from_pub(window);
    let tooltips = inner
        .active_popups()
        .iter()
        .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
        .map(|popup| popup.popup_id)
        .collect::<Vec<_>>();
    for tooltip in tooltips {
        inner.close_popup(tooltip);
    }
}
pub(crate) fn synchronize_find(window: &FindWindow) -> bool {
    synchronize_host(window.window(), window.global::<TooltipPlacement>())
}
fn synchronize_host(window: &slint::Window, global: TooltipPlacement<'_>) -> bool {
    let cursor = (global.get_cursor_x(), global.get_cursor_y());
    if cursor.0 < 0.0 || cursor.1 < 0.0 {
        return false;
    }
    // A property write can update popup geometry; release popup storage first.
    let parents = {
        let inner = WindowInner::from_pub(window);
        inner
            .active_popups()
            .iter()
            .filter(|popup| matches!(popup.window_kind, WindowKind::ToolTip))
            .filter_map(|popup| popup.parent_item.upgrade())
            .collect::<Vec<_>>()
    };
    let mut changed = false;
    for parent in parents {
        let Some(area) = parent.downcast::<TooltipArea>() else {
            continue;
        };
        let origin = parent.map_to_native_window(parent.geometry().origin);
        // Ignore sub-pixel rounding when deciding whether initial placement needs retry.
        changed |= (area.as_pin_ref().mouse_x().get() - (cursor.0 - origin.x)).abs() > 0.01
            || (area.as_pin_ref().mouse_y().get() - (cursor.1 - origin.y)).abs() > 0.01;
        area.as_pin_ref()
            .mouse_x
            .set(LogicalLength::new(cursor.0 - origin.x));
        area.as_pin_ref()
            .mouse_y
            .set(LogicalLength::new(cursor.1 - origin.y));
    }
    changed
}

pub(crate) fn install_panel_filter(window: &PanelFilterWindow) -> Result<(), PlatformError> {
    register(Target::PanelFilter(window.as_weak()))
}
pub(crate) fn install_hot_paths(window: &HotPathsWindow) -> Result<(), PlatformError> {
    register(Target::HotPaths(window.as_weak()))
}
