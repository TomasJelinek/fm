//! Exercise command artwork routing through the actual existing thumbnail pulse.
use super::*;
use midnight_core::{
    Event, UserCommand, UserCommandId, UserCommandInvocation, UserCommandLaunch, UserPreferences,
};
use slint::{
    Model,
    platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
};
use std::{path::PathBuf, sync::Arc, time::Instant};

fn pulse_until(mut ready: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !ready() {
        assert!(Instant::now() < deadline, "command artwork pulse timed out");
        std::thread::sleep(Duration::from_millis(5));
        slint::platform::update_timers_and_animations();
    }
}

#[test]
fn command_artwork_retries_rejected_sends_and_clears_hidden_views_without_decoding() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(super::tests::TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    let root = std::env::current_dir().unwrap();
    let executable = root.join("editor.exe");
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::new(UserPreferences {
            thumbnails_enabled: false,
            user_commands: Arc::new(vec![UserCommand {
                id: UserCommandId::new("editor").unwrap(),
                name: "Editor".into(),
                enabled: true,
                toolbar_label: None,
                icon: None,
                working_directory: "${active.path}".into(),
                launch: UserCommandLaunch::Single,
                invocation: UserCommandInvocation::Program {
                    executable: executable.clone(),
                    arguments: Vec::new(),
                },
            }]),
            ..Default::default()
        }))));
    midnight_ui::install(&window, presenter.clone(), Rc::new(|_| true));
    native.set_size(slint::PhysicalSize::new(1000, 700));
    window.show().unwrap();
    let row = window.get_user_commands().row_data(0).unwrap();
    window.invoke_user_command_icon_visible(row.identity.clone(), row.revision.clone(), true);
    let attempts = Rc::new(RefCell::new(Vec::new()));
    let recorded = attempts.clone();
    // Invalid discovery is harmless here: native artwork must bypass this decoder entirely.
    let service = Rc::new(Service::start(PathBuf::new(), PathBuf::new()));
    let controller = install_service(
        &window,
        presenter,
        service,
        content::Factory {
            native: Arc::new(|_, _| panic!("executable reached image decoder capture")),
            storage: Arc::new(|_| panic!("executable reached image decoder storage lookup")),
        },
        Rc::new(move |command| {
            let Command::SetCommandIconView {
                generation,
                targets,
            } = command
            else {
                panic!("artwork bridge submitted an unrelated command");
            };
            let mut attempts = recorded.borrow_mut();
            attempts.push((generation, targets));
            attempts.len() != 1
        }),
    );
    pulse_until(|| attempts.borrow().len() >= 2);
    {
        let attempts = attempts.borrow();
        assert_eq!(
            attempts[0], attempts[1],
            "failed submission must retry the same view"
        );
        assert_eq!(attempts[1].1.len(), 1);
        assert_eq!(attempts[1].1[0].executable, executable);
    }
    window.invoke_thumbnail_clear_cache();
    pulse_until(|| attempts.borrow().len() >= 3);
    assert!(attempts.borrow()[2].0 > attempts.borrow()[1].0);
    window.invoke_user_command_icon_visible(row.identity, row.revision, false);
    pulse_until(|| attempts.borrow().len() >= 4);
    assert!(
        attempts.borrow()[3].1.is_empty(),
        "hidden commands cancel the backend view"
    );
    controller.stop();
    window.hide().unwrap();
}
