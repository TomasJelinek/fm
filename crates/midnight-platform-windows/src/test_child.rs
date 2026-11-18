//! Bounded reruns of this test binary in a child process, for tests that need a
//! process-wide setting (such as a forced namespace lookup mode) in another run.
use std::{
    io::Read,
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

/// Elapses only on failure: the child runs of these tests take a few seconds.
const CHILD_DEADLINE: Duration = Duration::from_secs(60);
/// After a kill, for the child's pipes to close.
const KILL_GRACE: Duration = Duration::from_secs(5);

/// Runs this test binary again with `args` and the environment variable `env`, and
/// returns its standard output once it has exited successfully. Fails with the
/// child's output when it fails or has not finished by `CHILD_DEADLINE`; a child that
/// hangs is killed and reaped, so it cannot hang this test run.
pub(crate) fn run_test_child(args: &[&str], env: (&str, &str)) -> String {
    run_test_child_within(args, env, CHILD_DEADLINE)
}

fn run_test_child_within(args: &[&str], env: (&str, &str), deadline: Duration) -> String {
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(args)
        .env(env.0, env.1)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    // Drain both pipes while waiting: a child blocked on a full pipe never exits. Each
    // drain ends when the child closes its end of the pipe, normally by exiting.
    let (closed, pipe_closed) = mpsc::channel();
    let drain = |mut pipe: Box<dyn Read + Send>| {
        let closed = closed.clone();
        thread::spawn(move || {
            let mut bytes = Vec::new();
            let _ = pipe.read_to_end(&mut bytes);
            let _ = closed.send(());
            String::from_utf8_lossy(&bytes).into_owned()
        })
    };
    let stdout = drain(Box::new(child.stdout.take().unwrap()));
    let stderr = drain(Box::new(child.stderr.take().unwrap()));
    drop(closed);
    let mut open = 2;
    let mut drained_within = |within: Duration| {
        let until = Instant::now() + within;
        while open > 0
            && pipe_closed
                .recv_timeout(until.saturating_duration_since(Instant::now()))
                .is_ok()
        {
            open -= 1;
        }
        open == 0
    };
    let finished = drained_within(deadline);
    let drained = finished || {
        // Killing closes the child's ends of both pipes, which ends the drains unless
        // another process has inherited one of them.
        let _ = child.kill();
        drained_within(KILL_GRACE)
    };
    let status = child.wait().unwrap();
    let (stdout, stderr) = if drained {
        (stdout.join().unwrap(), stderr.join().unwrap())
    } else {
        Default::default()
    };
    assert!(
        finished,
        "the child test run did not finish within {deadline:?} and was killed:\n\
         {stdout}\n{stderr}"
    );
    assert!(
        status.success(),
        "the child test run failed ({status}):\n{stdout}\n{stderr}"
    );
    stdout
}

#[cfg(test)]
mod tests {
    use super::*;

    const HANG: &str = "MIDNIGHT_TEST_CHILD_HANG";

    /// Hangs only as the child of `a_hanging_child_is_killed_at_its_deadline`, so a run
    /// of every ignored test passes it at once.
    #[test]
    #[ignore = "the child of a_hanging_child_is_killed_at_its_deadline"]
    fn hanging_child() {
        if std::env::var_os(HANG).is_some() {
            loop {
                thread::park();
            }
        }
    }

    #[test]
    fn a_hanging_child_is_killed_at_its_deadline() {
        let deadline = Duration::from_millis(300);
        let failure = std::panic::catch_unwind(|| {
            run_test_child_within(
                &["test_child::tests::hanging_child", "--exact", "--ignored"],
                (HANG, "1"),
                deadline,
            )
        })
        .unwrap_err();
        let message = failure
            .downcast_ref::<String>()
            .map(String::as_str)
            .unwrap_or_default();
        assert!(
            message.contains("did not finish within 300ms and was killed"),
            "{message}"
        );
    }
}
