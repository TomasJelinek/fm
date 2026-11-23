//! Chromium's sandbox needs unprivileged user namespaces unless `chrome-sandbox` is setuid
//! root. Ubuntu 23.10 and later deny those namespaces to programs that run without an
//! AppArmor profile granting `userns` (`kernel.apparmor_restrict_unprivileged_userns=1`),
//! and a kernel can also deny them to every unprivileged program
//! (`kernel.unprivileged_userns_clone=0`, `user.max_user_namespaces=0`). Chromium then exits
//! before its first message, which the host could only report as a stopped transport.
//! These checks name the cause and what allows the sandbox instead: for AppArmor,
//! `install-html-viewer-apparmor.sh` installs a profile for the viewer executable; for the
//! kernel's own switches, only an administrator can, and no profile helps.
use super::super::SandboxRefusal;
use std::{
    fs, io,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

#[cfg(test)]
#[path = "browser_sandbox_linux_tests.rs"]
mod tests;

const SCRIPT: &str = "install-html-viewer-apparmor.sh";
const RESTRICTION: &str = "/proc/sys/kernel/apparmor_restrict_unprivileged_userns";
/// Debian's and Ubuntu's kernels: 0 denies user namespaces to unprivileged programs.
const USERNS_CLONE: &str = "/proc/sys/kernel/unprivileged_userns_clone";
/// 0 allows no new user namespaces at all.
const MAX_NAMESPACES: &str = "/proc/sys/user/max_user_namespaces";

/// What may deny Chromium's namespace sandbox here, read before it starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Namespaces {
    /// Nothing, as far as the system tells, or a setuid-root `chrome-sandbox` replaces
    /// them: nothing is checked once Chromium runs.
    Unchecked,
    /// AppArmor denies them to programs without a profile that grants them.
    Restricted,
    /// The kernel denies them to every unprivileged program: this setting is 0.
    Disabled(&'static str),
}

/// Before Chromium starts: what may deny its namespace sandbox, which it needs because
/// `chrome-sandbox` beside it is not setuid root.
pub(super) fn namespaces(executable: &Path) -> Namespaces {
    expected(
        |path| fs::read_to_string(path).ok(),
        || {
            executable
                .parent()
                .is_some_and(|package| setuid_root(&package.join("chrome-sandbox")))
        },
    )
}

/// Once the started Chromium process `pid` has executed, only after `namespaces`: the
/// error that replaces its stopped transport when the system denies its sandbox, with the
/// typed refusal for the viewer. A process with an AppArmor profile of its own, such as the
/// installed one, or one inherited from this process, may use the namespaces, so under
/// AppArmor only a process without any profile counts; and in every case only if a
/// namespace probe in a child of this process (never this process itself) is refused too.
pub(super) fn refusal(pid: u32, namespaces: Namespaces, executable: &Path) -> Option<io::Error> {
    decide(namespaces, || label(pid), probe).map(|denial| unavailable(executable, denial))
}

/// Why the system denies the sandbox.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Denial {
    /// AppArmor, for a browser without a profile.
    AppArmor,
    /// The kernel, through this setting.
    Disabled(&'static str),
}

/// The error the viewer shows instead of a stopped transport: the cause and what allows
/// the sandbox (for AppArmor, the one-time step for this installation, with the command
/// only where running it as root is safe, see `message`), with the typed refusal for a
/// host that offers to install the profile itself.
fn unavailable(executable: &Path, denial: Denial) -> io::Error {
    let (refusal, message) = match denial {
        Denial::AppArmor => {
            let refusal = SandboxRefusal::UserNamespaces {
                executable: executable.to_owned(),
            };
            let message = message(executable, &refusal);
            (refusal, message)
        }
        Denial::Disabled(setting) => {
            let refusal = SandboxRefusal::NamespacesDisabled { setting };
            let message = disabled_message(&refusal, setting);
            (refusal, message)
        }
    };
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        super::super::SandboxUnavailable::new(message, refusal),
    )
}

/// `namespaces` with the system's files read by `read`. The kernel's own switches come
/// first: under them no AppArmor profile helps. Systems without any of them never pay for
/// the setuid check.
fn expected(
    read: impl Fn(&str) -> Option<String>,
    setuid_sandbox: impl FnOnce() -> bool,
) -> Namespaces {
    let zero = |path| read(path).is_some_and(|value| value.trim() == "0");
    let denial = if zero(USERNS_CLONE) {
        Namespaces::Disabled("kernel.unprivileged_userns_clone")
    } else if zero(MAX_NAMESPACES) {
        Namespaces::Disabled("user.max_user_namespaces")
    } else if restricted(read(RESTRICTION).as_deref()) {
        Namespaces::Restricted
    } else {
        return Namespaces::Unchecked;
    };
    if setuid_sandbox() {
        Namespaces::Unchecked
    } else {
        denial
    }
}

fn decide(
    namespaces: Namespaces,
    label: impl FnOnce() -> Option<String>,
    probe: impl FnOnce() -> Probe,
) -> Option<Denial> {
    match namespaces {
        Namespaces::Unchecked => None,
        // Whatever profile the browser runs under: the switch is refused (EPERM) or the
        // limit reached (ENOSPC) for any unprivileged program.
        Namespaces::Disabled(setting) => {
            matches!(probe(), Probe::Denied | Probe::Exhausted).then_some(Denial::Disabled(setting))
        }
        Namespaces::Restricted => (label().as_deref().is_some_and(unconfined)
            && probe() == Probe::Denied)
            .then_some(Denial::AppArmor),
    }
}

/// `kernel.apparmor_restrict_unprivileged_userns` reads 1. Kernels without it have no
/// such restriction.
fn restricted(value: Option<&str>) -> bool {
    value.is_some_and(|value| value.trim() == "1")
}

/// A regular file owned by root with the setuid bit: Chromium then uses it as its sandbox.
fn setuid_root(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && setuid_by_root(metadata.uid(), metadata.mode()))
}

fn setuid_by_root(owner: u32, mode: u32) -> bool {
    owner == 0 && mode & 0o4000 != 0
}

/// The AppArmor label of a process, such as `unconfined` or `name (unconfined)`.
fn label(pid: u32) -> Option<String> {
    ["attr/apparmor/current", "attr/current"]
        .iter()
        .find_map(|file| fs::read_to_string(format!("/proc/{pid}/{file}")).ok())
}

/// A process without any AppArmor profile.
fn unconfined(label: &str) -> bool {
    label.trim_end_matches(['\n', '\0']) == "unconfined"
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Probe {
    Allowed,
    /// Refused: "Operation not permitted".
    Denied,
    /// No namespace left to create: "No space left on device".
    Exhausted,
    Unknown,
}

/// `unshare --user --map-root-user`, the mapping Chromium's namespace sandbox writes, in a
/// child process. Only a refusal counts; a missing tool or any other failure leaves the
/// generic transport error.
fn probe() -> Probe {
    let tool = |candidates: [&'static str; 2]| {
        candidates
            .into_iter()
            .find(|path| Path::new(path).is_file())
    };
    let (Some(unshare), Some(program)) = (
        tool(["/usr/bin/unshare", "/bin/unshare"]),
        tool(["/usr/bin/true", "/bin/true"]),
    ) else {
        return Probe::Unknown;
    };
    // No environment: the C locale keeps the system error text in English.
    match Command::new(unshare)
        .args(["--user", "--map-root-user", program])
        .env_clear()
        .current_dir("/")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
    {
        Ok(output) => classify(output.status.success(), &output.stderr),
        Err(_) => Probe::Unknown,
    }
}

fn classify(success: bool, stderr: &[u8]) -> Probe {
    let stderr = String::from_utf8_lossy(stderr);
    if success {
        Probe::Allowed
    } else if stderr.contains("Operation not permitted") {
        Probe::Denied
    } else if stderr.contains("No space left on device") {
        Probe::Exhausted
    } else {
        Probe::Unknown
    }
}

/// The kernel denies the namespaces: what does not help, and what an administrator can do.
fn disabled_message(refusal: &SandboxRefusal, setting: &str) -> String {
    format!(
        "{} ({setting} is 0.) An administrator can allow them by changing that setting or, \
         for an installation that only root can change, by making the HTML viewer's \
         chrome-sandbox setuid root. An AppArmor profile does not help here.",
        refusal.cause()
    )
}

fn message(executable: &Path, refusal: &SandboxRefusal) -> String {
    message_with(executable, refusal, |path| {
        fs::symlink_metadata(path).ok().map(|metadata| Ownership {
            owner: metadata.uid(),
            mode: metadata.mode(),
        })
    })
}

/// `message`, with the ownership of each path (never following a link) from `lookup`.
/// The `sudo` command is suggested only for an installer that only root can change: a
/// user-writable one (a per-user installation, an extracted package or a checkout) could
/// have been replaced by any program the user runs, and would then run as root. The
/// command names the installer itself in its folder's real path; an installer that is a
/// link is not suggested (it could name any file).
fn message_with(
    executable: &Path,
    refusal: &SandboxRefusal,
    lookup: impl Fn(&Path) -> Option<Ownership>,
) -> String {
    let cause = refusal.cause();
    match install_script(executable)
        .and_then(|script| Some(fs::canonicalize(script.parent()?).ok()?.join(SCRIPT)))
        .filter(|script| only_root_can_change(script, &lookup))
    {
        Some(script) => format!("{cause} Run once: sudo {}", shell_word(&script)),
        None => format!(
            "{cause} An administrator can allow it once by installing the package's AppArmor \
             profile for this installation with {SCRIPT} (see README.txt and \
             SYSTEM_REQUIREMENTS.md)."
        ),
    }
}

/// The owner and mode (`st_mode`, with the file type) of a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Ownership {
    owner: u32,
    mode: u32,
}

/// Whether `script`, an absolute path, is a regular file (no link) that only root can
/// change: it and every folder above it belong to root and are neither group- nor
/// other-writable (a POSIX ACL granting write shows as group write; a link's mode shows
/// as writable by all, so no link on the way passes).
fn only_root_can_change(script: &Path, lookup: &impl Fn(&Path) -> Option<Ownership>) -> bool {
    const FILE_TYPE: u32 = 0o170_000;
    const REGULAR: u32 = 0o100_000;
    let root_only =
        |path: &Path| lookup(path).filter(|found| found.owner == 0 && found.mode & 0o022 == 0);
    script.is_absolute()
        && root_only(script).is_some_and(|found| found.mode & FILE_TYPE == REGULAR)
        && script
            .ancestors()
            .skip(1)
            .all(|folder| root_only(folder).is_some())
}

/// The installer of this installation: beside `midnight-explorer` in a package
/// (`<root>/viewers/<package>/<executable>`), or in the checkout's `scripts` directory for a
/// development build (`<checkout>/_build/<profile>/viewers/<package>/<executable>`).
fn install_script(executable: &Path) -> Option<PathBuf> {
    let root = executable.parent()?.parent()?.parent()?;
    let packaged = root.join(SCRIPT);
    if packaged.is_file() {
        return Some(packaged);
    }
    let build = root.parent()?;
    let script = build.parent()?.join("scripts").join(SCRIPT);
    (build.file_name()? == "_build" && script.is_file()).then_some(script)
}

/// `path` as one shell word, quoted only when it needs to be.
fn shell_word(path: &Path) -> String {
    let text = path.to_string_lossy();
    if text
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"/._+-@:,".contains(&byte))
    {
        text.into_owned()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}
