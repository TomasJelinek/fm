//! Modeless information tools and embedded blocking notices own presentation only.
use crate::{
    AboutWindow, HelpWindow, MainWindow, Presenter, ShortcutHints,
    dialog_host::{
        CloseDisposition, DialogHost, DialogModality, DialogOwner, DialogSession, DialogSessions,
    },
    dialog_presentation::{self, DialogThemeTarget},
};
use midnight_core::Command;
use slint::{ComponentHandle, PlatformError, SharedString};
use std::{cell::RefCell, rc::Rc};

#[path = "information_notices.rs"]
mod notices;

#[cfg(test)]
#[path = "information_window_tests.rs"]
mod tests;

type ThemeRevision = (i32, i32, String, i32, i32);
type Windows = Rc<RefCell<InformationWindows>>;

struct Record<C> {
    window: C,
    host: DialogHost,
    session: DialogSession,
    generation: Option<i32>,
    theme: ThemeRevision,
}

struct InformationWindows {
    owner: DialogOwner,
    sessions: DialogSessions,
    help: Option<Record<HelpWindow>>,
    about: Option<Record<AboutWindow>>,
    notice: Rc<notices::Context>,
    closing: bool,
}

trait InformationWindow: ComponentHandle + DialogThemeTarget + Sized + 'static {
    const MODALITY: DialogModality = DialogModality::Modeless;
    fn create() -> Result<Self, PlatformError>;
    fn slot(windows: &mut InformationWindows) -> &mut Option<Record<Self>>;
    fn install_tooltips(&self) -> Result<(), PlatformError>;
    fn on_dismiss(&self, callback: impl Fn() + 'static);
    fn on_native_dismiss(&self);
    fn bind_actions(&self, _main: &MainWindow, _windows: &Windows, _session: DialogSession) {}
    fn initialize(&self, main: &MainWindow);
    fn refresh(&self, _main: &MainWindow) {}
    fn closed(&self, _main: &MainWindow) {}
    fn generation(_main: &MainWindow) -> Option<i32> {
        None
    }
}

macro_rules! information_window {
    ($component:ty, $slot:ident, $tooltip:ident, $($implementation:item),* $(,)?) => {
        impl DialogThemeTarget for $component {
            fn apply_theme(&self, scheme: i32, density: i32, family: SharedString, desktop: i32, touch: i32) {
                self.invoke_apply_preferences(scheme, density, family, desktop, touch);
            }
        }
        impl InformationWindow for $component {
            fn create() -> Result<Self, PlatformError> { Self::new() }
            fn slot(windows: &mut InformationWindows) -> &mut Option<Record<Self>> { &mut windows.$slot }
            fn install_tooltips(&self) -> Result<(), PlatformError> { crate::tooltip::$tooltip(self) }
            fn on_dismiss(&self, callback: impl Fn() + 'static) { self.on_request_dismiss(callback); }
            fn on_native_dismiss(&self) { self.invoke_request_dismiss(); }
            $($implementation)*
        }
    };
}

information_window!(
    HelpWindow,
    help,
    install_help,
    fn initialize(&self, main: &MainWindow) {
        self.set_close_icon(main.get_action_icons().close);
        self.refresh(main);
    },
    fn refresh(&self, main: &MainWindow) {
        self.set_topic(main.get_help_topic());
        self.set_shortcuts(main.global::<ShortcutHints>().invoke_help());
    }
);
information_window!(
    AboutWindow,
    about,
    install_about,
    fn initialize(&self, main: &MainWindow) {
        self.set_application_version(main.get_application_version());
        self.set_close_icon(main.get_action_icons().close);
    }
);
pub(crate) fn install(
    main: &MainWindow,
    state: Rc<RefCell<Presenter>>,
    _send: Rc<dyn Fn(Command) -> bool>,
) {
    let owner = DialogOwner::new(main);
    let windows = Rc::new(RefCell::new(InformationWindows {
        owner,
        sessions: DialogSessions::default(),
        help: None,
        about: None,
        notice: notices::install(main),
        closing: false,
    }));
    macro_rules! open_callback {
        ($callback:ident, $component:ty) => {{
            let weak = main.as_weak();
            let windows = windows.clone();
            let state = state.clone();
            main.$callback(move || {
                if let Some(main) = weak.upgrade() {
                    if let Err(error) = open::<$component>(&main, &windows, &state.borrow()) {
                        main.set_desktop_status(format!("Unable to open dialog: {error}").into());
                    }
                }
            });
        }};
    }
    open_callback!(on_open_help_window, HelpWindow);
    open_callback!(on_open_about_window, AboutWindow);
    {
        let weak = main.as_weak();
        let windows = windows.clone();
        main.on_refresh_information_windows(move || {
            let Some(main) = weak.upgrade() else {
                return;
            };
            let presenter = state.borrow();
            let notice = {
                let mut windows = windows.borrow_mut();
                refresh::<HelpWindow>(&main, &mut windows, &presenter);
                refresh::<AboutWindow>(&main, &mut windows, &presenter);
                windows.notice.clone()
            };
            drop(presenter);
            notices::refresh(&notice, &main);
        });
    }
    let weak = main.as_weak();
    main.on_close_information_windows(move || {
        let mut windows = windows.borrow_mut();
        windows.closing = true;
        if let Some(main) = weak.upgrade() {
            notices::close(&windows.notice, &main);
        }
        close_record(windows.about.take(), false);
        close_record(windows.help.take(), false);
    });
}

fn theme_revision(presenter: &Presenter) -> ThemeRevision {
    let values = &presenter.preferences.values;
    (
        presenter.preferences.theme_index(),
        presenter.preferences.density_index(),
        values.font_family.clone(),
        i32::from(values.desktop_font_size),
        i32::from(values.touch_font_size),
    )
}

fn refresh<C: InformationWindow>(
    main: &MainWindow,
    windows: &mut InformationWindows,
    presenter: &Presenter,
) {
    let Some(record) = C::slot(windows).as_mut() else {
        return;
    };
    let theme = theme_revision(presenter);
    if record.theme != theme {
        dialog_presentation::apply(&record.window, presenter);
        record.theme = theme;
    }
    record.window.refresh(main);
}

fn open<C: InformationWindow>(
    main: &MainWindow,
    windows: &Windows,
    presenter: &Presenter,
) -> Result<(), PlatformError> {
    let generation = C::generation(main);
    let (owner, session) = {
        let mut windows = windows.borrow_mut();
        if windows.closing {
            return Ok(());
        }
        if let Some(record) = C::slot(&mut windows)
            && record.generation == generation
        {
            record.host.show(record.window.window())?;
            record.host.activate(record.window.window());
            return Ok(());
        }
        close_record(C::slot(&mut windows).take(), false);
        let session = windows.sessions.allocate()?;
        (windows.owner.clone(), session)
    };
    let (window, host) = DialogHost::create(session, owner, C::MODALITY, C::create)?;
    window.install_tooltips()?;
    dialog_presentation::apply(&window, presenter);
    window.initialize(main);
    window.bind_actions(main, windows, session);
    let weak = Rc::downgrade(windows);
    let main_weak = main.as_weak();
    let dismissed = window.as_weak();
    window.on_dismiss(move || {
        if let (Some(main), Some(window)) = (main_weak.upgrade(), dismissed.upgrade()) {
            window.closed(&main);
        }
        if let Some(windows) = weak.upgrade() {
            dismiss::<C>(&windows, session);
        }
    });
    let weak = window.as_weak();
    window.window().on_close_requested(move || {
        if let Some(window) = weak.upgrade() {
            // The native X uses precisely the same callback as Escape and Close.
            window.on_native_dismiss();
        }
        slint::CloseRequestResponse::KeepWindowShown
    });
    host.show(window.window())?;
    *C::slot(&mut windows.borrow_mut()) = Some(Record {
        window,
        host,
        session,
        generation,
        theme: theme_revision(presenter),
    });
    Ok(())
}

fn dismiss<C: InformationWindow>(windows: &Windows, session: DialogSession) {
    let record = {
        let mut windows = windows.borrow_mut();
        let slot = C::slot(&mut windows);
        if !slot
            .as_ref()
            .is_some_and(|record| record.session == session && record.host.is_current(session))
        {
            return;
        }
        slot.take()
    };
    close_record(record, true);
}

fn close_record<C: ComponentHandle>(record: Option<Record<C>>, explicit: bool) {
    if let Some(record) = record {
        if explicit {
            record
                .host
                .request_close(record.window.window(), || CloseDisposition::Hide);
        } else {
            record.host.close(record.window.window());
        }
    }
}
