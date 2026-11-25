//! Private X11 displays for Chromium in parallel Linux acceptance workers.
use std::{
    env,
    fs::{self, File},
    os::unix::{fs::PermissionsExt, process::CommandExt},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::OnceLock,
    thread,
    time::{Duration, Instant},
};

fn launcher() -> Option<&'static Path> {
    static LAUNCHER: OnceLock<Option<PathBuf>> = OnceLock::new();
    LAUNCHER
        .get_or_init(|| {
            let path = env::var_os("PATH")?;
            let tool = |name| {
                env::split_paths(&path)
                    .map(|dir| dir.join(name))
                    .find(|file| {
                        fs::metadata(file)
                            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                    })
            };
            tool("Xvfb")?;
            tool("xauth")?;
            tool("xvfb-run")
        })
        .as_deref()
}

/// With an existing desktop, plugin scenarios can use it exclusively when
/// this host cannot start a private X server.
pub(super) fn needs_shared_desktop() -> bool {
    launcher().is_none()
        && ["DISPLAY", "WAYLAND_DISPLAY"]
            .iter()
            .any(|key| env::var_os(key).is_some_and(|value| !value.is_empty()))
}

pub(super) struct Display(Child);

/// The parent owns the only write end of the keeper's stdin pipe. Closing it,
/// even through abrupt parent exit, lets xvfb-run shut down the X server. The
/// acceptance child remains leader of its own independently supervised group.
pub(super) fn attach(
    worker: &mut Command,
    folder: &Path,
    deadline: Instant,
) -> Result<Option<Display>, String> {
    let Some(launcher) = launcher() else {
        if needs_shared_desktop() {
            return Err("A parallel plugin worker needs a private X display. Run it exclusively or install xvfb and xauth.".into());
        }
        return Ok(None);
    };
    let ready = folder.join("display");
    let log = folder.join("xvfb.log");
    let errors = File::create(&log).map_err(|error| format!("{}: {error}", log.display()))?;
    let mut display = Display(
        Command::new(launcher)
            .args([
                "-a",
                "-e",
                "/dev/stderr",
                "-s",
                "-screen 0 1280x1024x24 -nolisten tcp -noreset",
                "/bin/sh",
                "-c",
                r#"printf '%s\n' "$DISPLAY" "$XAUTHORITY" > "$1" && exec cat >/dev/null"#,
                "xvfb-keeper",
            ])
            .arg(&ready)
            .env_remove("DISPLAY")
            .env_remove("WAYLAND_DISPLAY")
            .env_remove("DBUS_SESSION_BUS_ADDRESS")
            .env_remove("XAUTHORITY")
            // xvfb-run owns its 0700 authorization directory independently of
            // the report, including when an interrupted run's files are deleted.
            .env("TMPDIR", "/tmp")
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(errors)
            .spawn()
            .map_err(|error| format!("cannot start {}: {error}", launcher.display()))?,
    );
    let deadline = deadline.min(Instant::now() + Duration::from_secs(30));
    loop {
        if let Some(status) = display.0.try_wait().map_err(|error| error.to_string())? {
            return Err(format!(
                "xvfb-run exited {status} before the display was ready. See {}",
                log.display()
            ));
        }
        if let Ok(value) = fs::read_to_string(&ready)
            && let Some((screen, authority)) =
                value.strip_suffix('\n').and_then(|s| s.split_once('\n'))
            && let Some(number) = screen.strip_prefix(':')
            && number.parse::<u32>().is_ok()
            && Path::new(authority).is_absolute()
        {
            worker.env("DISPLAY", screen).env("XAUTHORITY", authority);
            return Ok(Some(display));
        }
        if Instant::now() >= deadline {
            return Err(format!(
                "timed out starting a private X server. See {}",
                log.display()
            ));
        }
        thread::sleep(Duration::from_millis(25));
    }
}

impl Drop for Display {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        let deadline = Instant::now() + Duration::from_secs(3);
        while matches!(self.0.try_wait(), Ok(None)) {
            if Instant::now() >= deadline {
                super::viewers_stage::kill_group(self.0.id());
                let _ = self.0.wait();
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}
