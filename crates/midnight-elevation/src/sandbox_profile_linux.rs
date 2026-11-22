//! The HTML viewer's AppArmor profile, `midnight-explorer-html-viewer`, installed from the
//! viewer with administrator authorization where AppArmor restricts unprivileged user
//! namespaces (Ubuntu 23.10 and later). The application, unprivileged, generates the profile
//! text from the viewer executable's installation path by the rules of
//! `scripts/install-html-viewer-apparmor.sh`, whose `--print` shows the same text. Polkit
//! then runs one constant command as root (`INSTALL`), which reads that text from its
//! standard input, writes it to /etc/apparmor.d atomically and loads it. No file of the
//! installation or checkout is executed or read as root: the user may be able to change
//! them, so the installer script there is not run either.
use crate::{
    SandboxProfileError,
    native::{ChildSlot, PKEXEC, trusted_pkexec},
};
use midnight_core::Cancellation;
use std::{
    ffi::{CStr, CString, OsStr},
    fs,
    io::{self, Read, Write},
    os::unix::{ffi::OsStrExt, fs::MetadataExt, process::ExitStatusExt},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    ptr, thread,
    time::Duration,
};

#[cfg(test)]
#[path = "sandbox_profile_linux_tests.rs"]
mod tests;

/// The one profile of this name on a machine serves one installation at a time.
const PROFILE: &str = "midnight-explorer-html-viewer";
/// The viewer executable below an installation folder.
const VIEWER: &str = "viewers/midnight-viewer-html/midnight-viewer-html";
/// The extended attribute of a POSIX access ACL.
const ACCESS_ACL: &CStr = c"system.posix_acl_access";
/// The extended attribute of a folder's POSIX default ACL, which its new entries inherit.
const DEFAULT_ACL: &CStr = c"system.posix_acl_default";
/// The extended attribute of an NFSv4 ACL.
const NFS4_ACL: &CStr = c"system.nfs4_acl";
/// The viewer's package folder, and the name of its executable.
const PACKAGE: &str = "midnight-viewer-html";
/// The shell that pkexec runs as root.
const SHELL: &str = "/bin/sh";
/// How often a running installation checks for cancellation.
const POLL: Duration = Duration::from_millis(50);
/// The start of the root command's error output that an error keeps.
const ERROR_BYTES: usize = 4096;
/// The longest error text shown.
const ERROR_CHARS: usize = 600;

/// What runs as root, as `/bin/sh -c INSTALL` with the profile text on standard input. It
/// is constant, names only root's own tools and files (`PATH` is reset first) and never
/// exits with 126 or 127, which pkexec reports for a dismissed or refused authorization.
/// A temporary file in the profile's folder becomes the profile by one rename, so the
/// folder never holds a partial profile; its name starts with a dot, which AppArmor skips
/// when it loads the folder. A text that AppArmor refuses leaves the installed profile.
const INSTALL: &str = r#"PATH=/usr/sbin:/usr/bin:/sbin:/bin
export PATH
umask 022
fail() {
    printf '%s\n' "$1" >&2
    exit 1
}
target=/etc/apparmor.d/midnight-explorer-html-viewer
command -v apparmor_parser >/dev/null 2>&1 || fail 'apparmor_parser is missing (package apparmor).'
staged=$(mktemp /etc/apparmor.d/.midnight-explorer-html-viewer.XXXXXX) ||
    fail 'Cannot create a file in /etc/apparmor.d.'
trap 'rm -f -- "$staged"' EXIT
cat >"$staged" || fail 'Cannot write the profile.'
[ -s "$staged" ] || fail 'No profile text was received.'
chmod 0644 "$staged" || fail 'Cannot make the profile readable.'
apparmor_parser -QTK "$staged" >/dev/null || fail 'AppArmor refused the profile.'
mv -fT -- "$staged" "$target" || fail "Cannot replace $target."
apparmor_parser -r "$target" || fail 'AppArmor could not load the profile.'
exit 0
"#;

/// The HTML viewer's AppArmor profile for one installation, ready to be installed with
/// administrator authorization.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SandboxProfile {
    /// The profile file's text. Its only variable part is the validated attachment path.
    text: String,
    /// The executable path, or the pattern of a source checkout's builds, it attaches to.
    attachment: String,
    /// Someone other than root can change a folder on the way to that path.
    changeable: bool,
}

impl SandboxProfile {
    /// The profile that lets the refused browser `executable` use its sandbox, if an
    /// administrator can install it from here: `/usr/bin/pkexec` and every folder above it
    /// belong to root and only root can change them, and the executable is the HTML viewer
    /// of an installation, or of a build in a source checkout, whose path AppArmor can name
    /// without reading a character as a pattern and on whose way, from `/` down to the
    /// viewer (to the viewer of every build of a checkout), users other than its owner can
    /// write to nothing (not even a sticky folder such as /tmp). Inspects the file system;
    /// runs nothing.
    pub fn for_browser(executable: &Path) -> Option<Self> {
        offer(executable, trusted_pkexec)
    }

    /// The viewer executable the profile lets use user namespaces: its path or, for a build
    /// in a source checkout, `<checkout>/_build/*/viewers/…`, which names every build there.
    pub fn attachment(&self) -> &str {
        &self.attachment
    }

    /// Whether someone other than root, such as any program the user runs, can change a
    /// folder or viewer on the way to [`Self::attachment`] (a per-user installation, an
    /// extracted package or a checkout), and so put a program at that path, which the
    /// profile then lets use user namespaces as well.
    pub fn changeable(&self) -> bool {
        self.changeable
    }

    /// Install it as root through polkit. Blocks until the administrator answered and the
    /// installation ended, so call it on a backend worker. Cancellation stops a pending
    /// authorization; once authorized, the short root command completes and is waited for.
    pub fn install(&self, cancel: &Cancellation) -> Result<(), SandboxProfileError> {
        // Shares the bound on concurrent authorization prompts with the elevated listing.
        let _slot =
            ChildSlot::acquire().map_err(|error| SandboxProfileError::Failed(error.to_string()))?;
        install_with(self.text.as_bytes(), cancel, trusted_pkexec, run)
    }
}

/// `SandboxProfile::for_browser`, with the trust check of pkexec from `trusted`.
fn offer(executable: &Path, trusted: impl FnOnce() -> io::Result<()>) -> Option<SandboxProfile> {
    trusted().ok()?;
    profile_for(executable).ok()
}

/// The profile for the viewer `executable`, as `install-html-viewer-apparmor.sh --print`
/// writes it for the installation that holds the executable.
fn profile_for(executable: &Path) -> io::Result<SandboxProfile> {
    // AppArmor attaches a profile to the executable's real path.
    let executable = fs::canonicalize(executable)?;
    let (attachment, way) = attachment(&installation(&executable)?)?;
    if !covers(&attachment, &executable) {
        return Err(unusable("its profile would not attach to the HTML viewer"));
    }
    let changeable = changeable(&way)?;
    Ok(SandboxProfile {
        text: profile_text(&attachment),
        attachment,
        changeable,
    })
}

/// The installation whose profile covers the canonical viewer `executable`: the source
/// checkout of a development build (`<checkout>/_build/<build>/viewers/...`), which the
/// installer in the checkout's `scripts/` resolves without an argument and which covers
/// every build, otherwise the folder that holds `viewers/`, as the packaged installer
/// beside it resolves it.
fn installation(executable: &Path) -> io::Result<PathBuf> {
    let root = executable
        .ancestors()
        .nth(3)
        .filter(|root| root.join(VIEWER) == executable)
        .ok_or_else(|| unusable("it is not the HTML viewer of an installation"))?;
    let checkout = root
        .parent()
        .filter(|build| build.file_name() == Some(OsStr::new("_build")))
        .and_then(Path::parent)
        .filter(|checkout| checkout.join("Cargo.toml").is_file());
    Ok(checkout.unwrap_or(root).to_owned())
}

/// The executable path that the profile for `installation` attaches to, by the rules of
/// the installer's `attachment`: the folder's own viewer, the package folder's executable,
/// or every build of a source checkout; with every path on the way from `/` to the viewers
/// it names (see `up` and `down`). Like the installer, it refuses a canonical folder path
/// with any character but letters, digits, spaces and `._+-@:` (see `nameable`), and a way
/// through a path that users other than its owner can write to (see `shared`).
fn attachment(installation: &Path) -> io::Result<(String, Vec<PathBuf>)> {
    if !installation.is_dir() {
        return Err(unusable("its installation is not a folder"));
    }
    let root = fs::canonicalize(installation)?;
    let text = root
        .to_str()
        .filter(|text| nameable(text))
        .ok_or_else(|| unusable("its path contains characters that AppArmor reads as patterns"))?;
    // Every path whose writers decide what runs at the attached path: the viewer, or a
    // checkout's _build, and every folder above it, then each build below that _build.
    let viewer = root.join(VIEWER);
    let (attachment, way) = if viewer.is_file() {
        (format!("{text}/{VIEWER}"), up(&viewer))
    } else if root.join(PACKAGE).is_file() && root.file_name() == Some(OsStr::new(PACKAGE)) {
        (format!("{text}/{PACKAGE}"), up(&root.join(PACKAGE)))
    } else if root.join("_build").is_dir() && root.join("Cargo.toml").is_file() {
        let builds = root.join("_build");
        let mut way = up(&builds);
        way.extend(down(&builds)?);
        (format!("{text}/_build/*/{VIEWER}"), way)
    } else {
        return Err(unusable(
            "its installation holds neither the viewer nor a source checkout's _build",
        ));
    };
    if let Some(shared) = shared(&way) {
        return Err(unusable(format!(
            "users other than its owner can write to {}, so they could put another program \
             at the viewer's path, now or once the installation is gone",
            shared.display()
        )));
    }
    Ok((attachment, way))
}

/// `path` and every folder above it. The installer's `shared_folder` walks them.
fn up(path: &Path) -> Vec<PathBuf> {
    path.ancestors().map(Path::to_owned).collect()
}

/// Below a checkout's `_build` folder `builds`, the way down to the viewer of each build
/// that the checkout's pattern names: every build folder, its `viewers` folder, the
/// viewer's folder and the viewer, as far as they exist and are no links. AppArmor
/// attaches to an executable's real path, which a link here leaves, so what a link names
/// never counts, and a file here holds no build. The installer's `shared_builds`.
fn down(builds: &Path) -> io::Result<Vec<PathBuf>> {
    let mut entries = fs::read_dir(builds)?
        .map(|entry| Ok(entry?.path()))
        .collect::<io::Result<Vec<PathBuf>>>()?;
    entries.sort();
    let real = |path: &Path| fs::symlink_metadata(path).ok().filter(|m| !m.is_symlink());
    let mut way = Vec::new();
    'builds: for build in entries {
        let viewer = build.join(VIEWER);
        // The build folder, `viewers` and the viewer's folder, outermost first.
        let mut folders: Vec<&Path> = viewer
            .ancestors()
            .skip(1)
            .take_while(|folder| folder.starts_with(&build))
            .collect();
        folders.reverse();
        for folder in folders {
            if !real(folder).is_some_and(|metadata| metadata.is_dir()) {
                continue 'builds;
            }
            way.push(folder.to_owned());
        }
        if real(&viewer).is_some() {
            way.push(viewer);
        }
    }
    Ok(way)
}

/// The first path of `way` that users other than its owner can write to (even a sticky
/// folder such as /tmp), or that cannot be inspected. A link is not followed.
fn shared(way: &[PathBuf]) -> Option<&Path> {
    let accounts = Accounts::read();
    way.iter().map(PathBuf::as_path).find(|path| {
        !fs::symlink_metadata(path).is_ok_and(|metadata| {
            only_its_owner_writes(
                metadata.mode(),
                metadata.uid(),
                metadata.gid(),
                &accounts,
                || may_have_acl(path, metadata.is_dir()),
            )
        })
    })
}

/// Whether only the owner `uid` can write to a path of `mode` and group `gid`: no other
/// user can, and its group only if that is the owner's private group and it carries no
/// ACL, which `acl` tells and is asked only then. With a POSIX ACL the group bits are the
/// ACL's mask, which bounds what its named users and groups may do: without group write
/// they cannot write, and with it any ACL refuses. The copy staging's rule
/// (`private_writers_only` in `crates/midnight-platform-linux/src/ownership.rs`).
fn only_its_owner_writes(
    mode: u32,
    uid: u32,
    gid: u32,
    accounts: &Accounts,
    acl: impl FnOnce() -> bool,
) -> bool {
    mode & 0o002 == 0 && (mode & 0o020 == 0 || (accounts.private_group(uid, gid) && !acl()))
}

/// Whether `path` itself, never what a link names, may carry an ACL, which `ls -l` marks
/// with a `+`: a POSIX access ACL or, on a `folder`, a POSIX default ACL, or an NFSv4 ACL.
/// Only a missing attribute (ENODATA) or a filesystem without such ACLs (EOPNOTSUPP) proves
/// that it carries none; any other failure counts as one, as for the copy staging's
/// `may_have_access_acl`.
fn may_have_acl(path: &Path, folder: bool) -> bool {
    let Ok(path) = CString::new(path.as_os_str().as_bytes()) else {
        return true;
    };
    let mut names = [ACCESS_ACL, NFS4_ACL]
        .into_iter()
        .chain(folder.then_some(DEFAULT_ACL));
    names.any(|name| {
        // SAFETY: a terminated path and attribute name; with no buffer the call writes
        // nothing and only reports the value's size.
        let size = unsafe { libc::lgetxattr(path.as_ptr(), name.as_ptr(), ptr::null_mut(), 0) };
        size >= 0
            || !matches!(
                io::Error::last_os_error().raw_os_error(),
                Some(libc::ENODATA | libc::EOPNOTSUPP)
            )
    })
}

/// The local account files, `/etc/passwd` and `/etc/group`, for `Accounts::private_group`.
struct Accounts {
    passwd: Vec<u8>,
    group: Vec<u8>,
}

impl Accounts {
    /// The files as they are now; one that cannot be read, or is implausibly large, is
    /// empty and so names no private group.
    fn read() -> Self {
        fn bounded(path: &str) -> Vec<u8> {
            const LIMIT: u64 = 8 << 20;
            let mut bytes = Vec::new();
            match fs::File::open(path).and_then(|file| file.take(LIMIT + 1).read_to_end(&mut bytes))
            {
                Ok(read) if read as u64 <= LIMIT => bytes,
                _ => Vec::new(),
            }
        }
        Self {
            passwd: bounded("/etc/passwd"),
            group: bounded("/etc/group"),
        }
    }

    /// Whether `gid` is the private group of the user `uid`. Distributions with user
    /// private groups (Debian, Ubuntu, Fedora) give every user a group of their own and,
    /// through pam_umask or the shell profile, a umask of 002, so the folders a user creates
    /// are writable by that group and yet by nobody else: the user's primary group, named
    /// like the user, which is no other user's primary group and lists no other member.
    /// Only the local files tell, as for pam_umask; a user only a directory service knows,
    /// or files that include one (lines starting with `+` or `-`), have none. The
    /// installer's `private_group`.
    fn private_group(&self, uid: u32, gid: u32) -> bool {
        fn lines(text: &[u8]) -> impl Iterator<Item = &[u8]> {
            text.split(|byte| *byte == b'\n')
        }
        fn id(field: Option<&[u8]>) -> Option<u32> {
            let field = field?;
            if field.is_empty() || !field.iter().all(u8::is_ascii_digit) {
                return None;
            }
            std::str::from_utf8(field).ok()?.parse().ok()
        }
        let included =
            |text: &[u8]| lines(text).any(|line| line.starts_with(b"+") || line.starts_with(b"-"));
        if included(&self.passwd) || included(&self.group) {
            return false;
        }
        // name:password:uid:gid:…
        let users = || {
            lines(&self.passwd).filter_map(|line| {
                let mut fields = line.split(|byte| *byte == b':');
                let name = fields.next()?;
                fields.next()?;
                Some((name, id(fields.next())?, id(fields.next())?))
            })
        };
        let Some((name, _, primary)) = users().find(|(_, user, _)| *user == uid) else {
            return false;
        };
        if primary != gid || users().any(|(other, _, group)| group == gid && other != name) {
            return false;
        }
        // name:password:gid:member,member…
        let group = lines(&self.group).find_map(|line| {
            let mut fields = line.split(|byte| *byte == b':');
            let group = fields.next()?;
            fields.next()?;
            (id(fields.next())? == gid).then(|| (group, fields.next().unwrap_or_default()))
        });
        group.is_some_and(|(group, members)| {
            group == name
                && members
                    .split(|byte| *byte == b',')
                    .all(|member| member.is_empty() || member == name)
        })
    }
}

/// Whether someone other than root owns a path of `way` (`attachment` has refused any that
/// others can write to), and could therefore put a program at the attached path.
fn changeable(way: &[PathBuf]) -> io::Result<bool> {
    let mut changeable = false;
    for path in way {
        changeable |= fs::symlink_metadata(path)?.uid() != 0;
    }
    Ok(changeable)
}

/// An absolute path of letters, digits, spaces and `._+-@:` between slashes. AppArmor reads
/// none of them as a pattern, quote, escape or variable, so the quoted attachment names
/// exactly this path; the only `*` is the one a checkout pattern adds after the check.
fn nameable(path: &str) -> bool {
    path.starts_with('/')
        && path
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._+/@: -".contains(&byte))
}

/// Whether `attachment` names the canonical `executable`: the same path, or, for a
/// checkout's pattern, the viewer of a build folder directly in its `_build`.
fn covers(attachment: &str, executable: &Path) -> bool {
    let Some(executable) = executable.to_str() else {
        return false;
    };
    match attachment.split_once("/_build/*/") {
        Some((checkout, viewer)) => executable
            .strip_prefix(checkout)
            .and_then(|rest| rest.strip_prefix("/_build/"))
            .and_then(|rest| rest.split_once('/'))
            .is_some_and(|(build, rest)| !build.is_empty() && rest == viewer),
        None => executable == attachment,
    }
}

/// The installer's `profile_text`: Ubuntu's form of a browser profile, which confines
/// nothing and only grants `userns`. `attachment` is quoted, which keeps its spaces.
fn profile_text(attachment: &str) -> String {
    format!(
        "# Midnight Explorer HTML viewer: lets its Chromium (CEF) runtime create the user
# namespaces of Chromium's sandbox where AppArmor restricts them for unconfined
# programs. Like Ubuntu's profiles for other browsers, it confines nothing else.
# Written by install-html-viewer-apparmor.sh; remove with its --uninstall option.

abi <abi/4.0>,
include <tunables/global>

profile {PROFILE} \"{attachment}\" flags=(unconfined) {{
  userns,

  # Site-specific additions and overrides. See local/README for details.
  include if exists <local/{PROFILE}>
}}
"
    )
}

fn unusable(reason: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!("The HTML viewer's AppArmor profile cannot be installed from here: {reason}."),
    )
}

/// `SandboxProfile::install`: the trust check from `trusted`, then the constant root
/// command through pkexec with `text` on its standard input, run by `run`.
fn install_with(
    text: &[u8],
    cancel: &Cancellation,
    trusted: impl FnOnce() -> io::Result<()>,
    run: impl FnOnce(&mut Command, &[u8], &Cancellation) -> io::Result<Finished>,
) -> Result<(), SandboxProfileError> {
    // Checked again right before use: pkexec may have changed since the offer.
    trusted().map_err(|_| SandboxProfileError::Unavailable)?;
    let mut command = Command::new(PKEXEC);
    // Without its textual agent, pkexec never reads a password from a terminal that
    // started the application: a graphical polkit agent asks, or the request is refused.
    // The C locale keeps pkexec's own messages, which `Finished::outcome` reads, in English
    // (pkexec passes it on to the root command too).
    command
        .args(["--disable-internal-agent", SHELL, "-c", INSTALL])
        .env("LC_ALL", "C");
    match run(&mut command, text, cancel) {
        Ok(finished) => finished.outcome(),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            Err(SandboxProfileError::Unavailable)
        }
        Err(error) => Err(SandboxProfileError::Failed(error.to_string())),
    }
}

/// How the root command ended: its exit status and the start of its error output.
#[derive(Debug)]
struct Finished {
    status: ExitStatus,
    error: Vec<u8>,
}

impl Finished {
    fn outcome(&self) -> Result<(), SandboxProfileError> {
        match self.status.code() {
            Some(0) => Ok(()),
            // pkexec found no authentication agent in this session to ask: asking again
            // cannot succeed here.
            Some(127) if self.no_agent() => Err(SandboxProfileError::NoAgent),
            // pkexec: the authorization dialog was dismissed (126), or the user is not
            // authorized or no authorization could be obtained (127).
            Some(126 | 127) => Err(SandboxProfileError::Declined),
            _ => Err(SandboxProfileError::Failed(self.describe())),
        }
    }

    /// pkexec's message (in the C locale) when no polkit agent runs in the session.
    fn no_agent(&self) -> bool {
        const NO_AGENT: &[u8] = b"No authentication agent found";
        self.error
            .windows(NO_AGENT.len())
            .any(|window| window == NO_AGENT)
    }

    /// The error output as one bounded line without control characters, or the exit
    /// status when there is none.
    fn describe(&self) -> String {
        let text = String::from_utf8_lossy(&self.error);
        let words: Vec<&str> = text.split_whitespace().collect();
        let line: String = words
            .join(" ")
            .chars()
            .filter(|c| !c.is_control())
            .collect();
        if line.is_empty() {
            return format!("The installation ended with {}.", self.status);
        }
        match line.char_indices().nth(ERROR_CHARS) {
            Some((end, _)) => format!("{}…", &line[..end]),
            None => line,
        }
    }
}

/// Run `command` with `input` on its standard input: start it, write and close the input,
/// then wait for its exit while a scoped thread keeps the start of its error output and
/// reads the rest, so the command never waits on a full pipe. Cancellation stops it; a
/// command that pkexec already started as root cannot be stopped and is waited for.
fn run(command: &mut Command, input: &[u8], cancel: &Cancellation) -> io::Result<Finished> {
    let mut child = command
        .current_dir("/")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;
    let stdin = child.stdin.take();
    let stderr = child.stderr.take();
    thread::scope(|scope| {
        let reader = stderr
            .map(|stderr| {
                thread::Builder::new()
                    .name("sandbox-profile-errors".into())
                    .spawn_scoped(scope, move || error_start(stderr))
            })
            .transpose();
        let reader = match reader {
            Ok(reader) => reader,
            Err(error) => {
                reap(&mut child);
                return Err(error);
            }
        };
        // Far below a pipe's capacity, so this never waits for the command. One that ended
        // first has closed the pipe, and its exit status says why.
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(input);
        }
        let status = wait(&mut child, cancel);
        let error = reader
            .and_then(|reader| reader.join().ok())
            .unwrap_or_default();
        Ok(Finished {
            status: status?,
            error,
        })
    })
}

/// The exit status of `child`, checking `cancel` every `POLL`.
fn wait(child: &mut Child, cancel: &Cancellation) -> io::Result<ExitStatus> {
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if cancel.is_cancelled() => return stop(child),
            Ok(None) => thread::sleep(POLL),
            Err(error) => {
                reap(child);
                return Err(error);
            }
        }
    }
}

/// Cancel `child`: an authorization that is still pending ends, reported as cancelled.
/// Once pkexec runs the command as root, this process may not signal it any more: the
/// command is waited for and its own status returned, as is the status of one that ended
/// before the signal, so a completed installation is never reported as cancelled.
fn stop(child: &mut Child) -> io::Result<ExitStatus> {
    let signalled = child.kill().is_ok();
    let status = child.wait()?;
    if signalled && status.signal() == Some(libc::SIGKILL) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "The installation was cancelled.",
        ));
    }
    Ok(status)
}

/// Stop `child` if it can still be stopped, and wait for its exit.
fn reap(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// The first `ERROR_BYTES` of `stream`; the rest is read and dropped.
fn error_start(mut stream: impl Read) -> Vec<u8> {
    let mut start = Vec::new();
    let _ = stream
        .by_ref()
        .take(ERROR_BYTES as u64)
        .read_to_end(&mut start);
    let _ = io::copy(&mut stream, &mut io::sink());
    start
}
