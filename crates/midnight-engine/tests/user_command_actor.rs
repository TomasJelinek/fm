use midnight_core::*;
use midnight_engine::Engine;
use midnight_platform::LocalFileSystem;
use std::{
    io,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use tokio::sync::mpsc;

struct Native(Arc<AtomicUsize>);
impl TerminalService for Native {
    fn shell_flavor(&self) -> ShellFlavor {
        ShellFlavor::Posix
    }
    fn launch_terminal(&self, _: &TerminalRequest) -> Result<Box<dyn TerminalProcess>, FsError> {
        panic!("preview/execution fixture must not launch an ordinary terminal")
    }
}
impl UserCommandService for Native {
    fn launch_user_program(
        &self,
        _: &Path,
        _: &[std::ffi::OsString],
        directory: &Path,
        _: &Cancellation,
    ) -> Result<Box<dyn TerminalProcess>, FsError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(FsError::from_io(
            "owned launch fixture",
            directory,
            io::Error::other("expected fixture failure"),
        ))
    }
}
fn request(id: u64, directory: &Path) -> UserCommandRequest {
    UserCommandRequest {
        id: UserCommandRequestId(id),
        command: UserCommandId::new("test").unwrap(),
        configuration: Arc::new(vec![UserCommand {
            id: UserCommandId::new("test").unwrap(),
            name: "Test".into(),
            enabled: true,
            toolbar_label: None,
            icon: None,
            working_directory: "${active.path}".into(),
            launch: UserCommandLaunch::Single,
            invocation: UserCommandInvocation::Program {
                executable: std::env::current_exe().unwrap(),
                arguments: vec!["literal & $x;".into()],
            },
        }]),
        context: UserCommandContext {
            active: Some(directory.to_owned()),
            other: None,
            files: None,
            focus: None,
        },
    }
}
async fn event(events: &mut mpsc::Receiver<Event>) -> UserCommandEvent {
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Event::UserCommand(event) = events.recv().await.unwrap() {
                return *event;
            }
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn preview_is_read_only_and_execution_failure_reports_an_honest_summary_without_history() {
    let directory = tempfile::tempdir().unwrap();
    let service = Arc::new(LocalFileSystem::new(None));
    let attempts = Arc::new(AtomicUsize::new(0));
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(
        Engine::new(service.clone(), events)
            .with_transfers(service)
            .with_user_commands(Arc::new(Native(attempts.clone())))
            .run(commands),
    );
    send.send(Command::PreviewUserCommand(Box::new(request(
        1,
        directory.path(),
    ))))
    .await
    .unwrap();
    loop {
        match event(&mut receive).await.update {
            UserCommandUpdate::Preview(preview) => {
                assert_eq!(preview.processes.len(), 1);
                assert!(
                    matches!(&preview.processes[0].process.invocation, UserCommandProcessInvocation::Program { arguments, .. }
                    if arguments == &vec![std::ffi::OsString::from("literal & $x;")])
                );
                break;
            }
            UserCommandUpdate::Rejected(error) => panic!("preview rejected: {error}"),
            _ => {}
        }
    }
    assert_eq!(attempts.load(Ordering::SeqCst), 0);
    send.send(Command::ExecuteUserCommand(Box::new(request(
        2,
        directory.path(),
    ))))
    .await
    .unwrap();
    loop {
        let update = event(&mut receive).await;
        if update.request == UserCommandRequestId(2)
            && let UserCommandUpdate::Finished(summary) = update.update
        {
            assert_eq!(
                (summary.planned, summary.started, summary.not_started),
                (1, 0, 1)
            );
            assert!(!summary.cancelled);
            assert!(summary.error.is_some());
            break;
        }
    }
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    drop(send);
    tokio::time::timeout(Duration::from_secs(10), engine)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn unavailable_service_and_reused_request_ids_are_rejected_visibly() {
    let directory = tempfile::tempdir().unwrap();
    let service = Arc::new(LocalFileSystem::new(None));
    let (send, commands) = mpsc::channel(16);
    let (events, mut receive) = mpsc::channel(16);
    let engine = tokio::spawn(Engine::new(service, events).run(commands));
    for id in [3, 3, 2] {
        send.send(Command::PreviewUserCommand(Box::new(request(
            id,
            directory.path(),
        ))))
        .await
        .unwrap();
        assert!(matches!(
            event(&mut receive).await.update,
            UserCommandUpdate::Rejected(_)
        ));
    }
    drop(send);
    tokio::time::timeout(Duration::from_secs(10), engine)
        .await
        .unwrap()
        .unwrap();
}
