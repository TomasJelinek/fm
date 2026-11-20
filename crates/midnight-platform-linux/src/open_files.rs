//! The process's limit on open files (RLIMIT_NOFILE).
//!
//! Desktop sessions start programs with a soft limit of 1024 open files under a much higher
//! hard limit (systemd's default hard limit is 524,288). Every folder a transfer works
//! inside keeps a few descriptors open, so a soft limit that low bounds how deep a copy can
//! go (`transfer::descriptor_budget` takes a quarter of the limit) and leaves little for
//! everything else. Like many runtimes, the application therefore raises its soft limit
//! to the hard limit, at most `MAX_OPEN_FILES`, when it starts, before its workers open
//! files. Programs it starts for the user (an editor, a terminal, a custom command, a
//! document's application) get the limit the application started with back: a program
//! that still uses select() cannot handle descriptors numbered 1024 or above. So does the
//! application when it restarts itself (with software rendering): it raises the limit
//! again and so learns the original one.
use rustix::process::{Resource, Rlimit, getrlimit, setrlimit};
use std::{io, os::unix::process::CommandExt, process::Command, sync::OnceLock};

#[cfg(test)]
#[path = "open_files_tests.rs"]
mod tests;

/// The most open files the application raises its soft limit to: thousands of folder
/// levels for transfers, with ample room for everything else, and far below what the
/// kernel allows one process.
pub const MAX_OPEN_FILES: u64 = 65_536;

/// The limit the process started with, once `raise_open_file_limit` raised it.
static ORIGINAL: OnceLock<Rlimit> = OnceLock::new();

/// Raise the soft limit on open files to the hard limit, at most `MAX_OPEN_FILES`, never
/// lowering it. Call once at application start, before other threads open files. Returns
/// the soft limit now in force (`None`: unlimited).
pub fn raise_open_file_limit() -> io::Result<Option<u64>> {
    let started = getrlimit(Resource::Nofile);
    let Some(target) = raised(started) else {
        return Ok(started.current);
    };
    setrlimit(
        Resource::Nofile,
        Rlimit {
            current: Some(target),
            maximum: started.maximum,
        },
    )?;
    let _ = ORIGINAL.set(started);
    Ok(Some(target))
}

/// The soft limit to raise `limit`'s to: its hard limit, at most `MAX_OPEN_FILES`; `None`
/// when the soft limit is that high already (or unlimited).
fn raised(limit: Rlimit) -> Option<u64> {
    let soft = limit.current?;
    let target = limit
        .maximum
        .map_or(MAX_OPEN_FILES, |hard| hard.min(MAX_OPEN_FILES));
    (target > soft).then_some(target)
}

/// The program that `command` starts gets the open-file limit this process started with,
/// if `raise_open_file_limit` raised it: every program started for the user, and a
/// restart of the application, which then raises the limit itself and remembers the
/// original for the programs it starts.
pub fn with_original_open_file_limit(command: &mut Command) {
    if let Some(original) = ORIGINAL.get() {
        restore_in_child(command, *original);
    }
}

/// `command`'s child sets its open-file limit to `limit` before it executes its program.
/// Lowering a soft limit is always allowed; should it fail anyway, the program still runs.
#[allow(unsafe_code)]
fn restore_in_child(command: &mut Command, limit: Rlimit) {
    // SAFETY: the closure runs in the child between fork and exec, where only
    // async-signal-safe operations are allowed. It makes one setrlimit system call on a
    // copied value, allocates nothing, takes no lock and ignores the result.
    unsafe {
        command.pre_exec(move || {
            let _ = setrlimit(Resource::Nofile, limit);
            Ok(())
        });
    }
}
