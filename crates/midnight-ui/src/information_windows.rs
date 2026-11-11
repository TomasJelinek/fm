//! Informational native windows own presentation only, never backend resources.
use crate::{
    AboutWindow, HelpWindow, MainWindow, NoticeWindow, Presenter, ShortcutHints,
    dialog_host::{
        CloseDisposition, DialogHost, DialogModality, DialogOwner, DialogSession, DialogSessions,
    },
    dialog_presentation::{self, DialogThemeTarget},
};
use midnight_core::Command;
use slint::{ComponentHandle, PlatformError, SharedString};
use std::{cell::RefCell, rc::Rc};

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
    notice: Option<Record<NoticeWindow>>,
    pending_notice: Option<i32>,
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
information_window!(
    NoticeWindow,
    notice,
    install_notice,
    const MODALITY: DialogModality = DialogModality::OwnerModal;,
    fn initialize(&self, main: &MainWindow) {
        self.set_message_title(main.get_message_title());
        self.set_body(main.get_message_body());
        self.set_error(main.get_message_error());
        self.set_generation(main.get_message_generation());
    },
    fn refresh(&self, main: &MainWindow) {
        if self.get_generation() == main.get_message_generation() {
            self.initialize(main);
        }
    },
    fn generation(main: &MainWindow) -> Option<i32> {
        Some(main.get_message_generation())
    },
    fn bind_actions(&self, main: &MainWindow, windows: &Windows, session: DialogSession) {
        let weak = main.as_weak();
        let windows = Rc::downgrade(windows);
        let generation = main.get_message_generation();
        self.on_copy_details(move || {
            let (Some(main), Some(windows)) = (weak.upgrade(), windows.upgrade()) else {
                return;
            };
            let current = windows.borrow().notice.as_ref().is_some_and(|record| {
                record.session == session
                    && record.host.is_current(session)
                    && record.generation == Some(generation)
            });
            if current && main.get_message_generation() == generation {
                main.invoke_copy_message();
            }
        });
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
        notice: None,
        pending_notice: None,
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
        let state = state.clone();
        main.on_open_notice_window(move || {
            let Some(main) = weak.upgrade() else {
                return;
            };
            if windows.borrow().closing {
                return;
            }
            // Preferences/backend startup can report a notice before the main
            // adapter is visible. Retain only the latest requested generation.
            if !windows.borrow().owner.is_ready() {
                windows.borrow_mut().pending_notice = Some(main.get_message_generation());
                return;
            }
            windows.borrow_mut().pending_notice = None;
            if let Err(error) = open::<NoticeWindow>(&main, &windows, &state.borrow()) {
                main.set_desktop_status(format!("Unable to open dialog: {error}").into());
            }
        });
    }
    {
        let weak = main.as_weak();
        let windows = windows.clone();
        main.on_refresh_information_windows(move || {
            let Some(main) = weak.upgrade() else {
                return;
            };
            let presenter = state.borrow();
            let pending = {
                let mut windows = windows.borrow_mut();
                refresh::<HelpWindow>(&main, &mut windows, &presenter);
                refresh::<AboutWindow>(&main, &mut windows, &presenter);
                refresh::<NoticeWindow>(&main, &mut windows, &presenter);
                windows.owner.is_ready()
                    && !windows.closing
                    && windows.pending_notice.take().is_some()
            };
            if pending && let Err(error) = open::<NoticeWindow>(&main, &windows, &presenter) {
                main.set_desktop_status(format!("Unable to open dialog: {error}").into());
            }
        });
    }
    main.on_close_information_windows(move || {
        let mut windows = windows.borrow_mut();
        windows.closing = true;
        windows.pending_notice = None;
        close_record(windows.notice.take(), false);
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
    window.on_dismiss(move || {
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
