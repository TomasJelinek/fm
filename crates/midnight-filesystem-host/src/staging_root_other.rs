//! The platform temporary directory is already per-user here (Windows `%TEMP%`,
//! macOS `$TMPDIR`); the private-root check still verifies ownership and access. A
//! root for this process alone comes last: `%TEMP%` can be redirected to a directory
//! that other accounts share, where one of them may create the fixed name first.
//! The fixed name carries no account label, unlike the engine's report roots: a
//! label would only save a per-run directory in that rare configuration, and the
//! per-process root already keeps staged reads working there.
use midnight_worker_platform::spool::SpoolRoot;

pub(super) fn candidates() -> Vec<SpoolRoot> {
    let temporary = std::env::temp_dir();
    vec![
        SpoolRoot {
            path: temporary.join("MidnightExplorer-archive-spool"),
            per_process: false,
        },
        SpoolRoot {
            path: temporary.join(format!(
                "MidnightExplorer-archive-spool-{}",
                std::process::id()
            )),
            per_process: true,
        },
    ]
}
