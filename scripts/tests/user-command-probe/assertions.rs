use midnight_core::TerminalProcess;
use std::{
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Compiler flags of every probe build; they are part of the cache key.
const PROBE_FLAGS: [&str; 2] = ["--edition", "2024"];

/// Returns the probe executable `_build/goal/commands/native/<key>/<name>`.
///
/// `<key>` hashes the compiler path, its `rustc -vV`, the probe flags and the
/// probe sources, so all test threads and processes share one directory per
/// compiler and source. Its first caller compiles `<key>/probe[.exe]` while it
/// holds an OS file lock on `<key>/.lock`; other threads and processes wait for
/// that lock (the OS releases it when a holder exits) and then reuse the file.
/// Every requested name is a hard link to it, or a copy where the filesystem has
/// no hard links. A cold run therefore compiles once per key and a warm run never.
/// Publication never replaces a published (possibly running) probe and stays
/// race-free without the lock, which only prevents duplicate compiles.
pub(crate) fn helper(workspace: &Path, name: &str) -> PathBuf {
    let probe = workspace.join("scripts/tests/user-command-probe");
    let (compiler, version) = compiler_identity();
    // FNV-1a over length-prefixed parts: a stable key without extra dependencies.
    let mut key = 0xcbf2_9ce4_8422_2325_u64;
    let parts = [
        compiler.as_encoded_bytes().to_vec(),
        version.clone(),
        PROBE_FLAGS.join("\0").into_bytes(),
        fs::read(probe.join("main.rs")).unwrap(),
        fs::read(probe.join("unix.rs")).unwrap(),
        fs::read(probe.join("windows.rs")).unwrap(),
    ];
    for part in &parts {
        for byte in (part.len() as u64).to_le_bytes().iter().chain(part) {
            key = (key ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3);
        }
    }
    let directory = workspace
        .join("_build/goal/commands/native")
        .join(format!("{key:016x}"));
    let executable = directory.join(name);
    if executable.is_file() {
        return executable;
    }
    fs::create_dir_all(&directory).unwrap();
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(directory.join(".lock"))
        .unwrap();
    // Without OS file locks, concurrent cold callers may each compile; publication
    // still leaves exactly one identical probe per name.
    let _ = lock.lock();
    let canonical = directory.join(if cfg!(windows) { "probe.exe" } else { "probe" });
    if !canonical.is_file() {
        let staging = private_path(&directory, ".build");
        fs::create_dir_all(&staging).unwrap();
        let built = staging.join(canonical.file_name().unwrap());
        let output = Command::new(compiler)
            .args(PROBE_FLAGS)
            .arg(probe.join("main.rs"))
            .arg("-o")
            .arg(&built)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "probe compile: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        publish(&built, &canonical);
        // Compiler side files (for example a PDB) stay out of the shared directory.
        let _ = fs::remove_dir_all(&staging);
    }
    publish(&canonical, &executable);
    executable
}

/// The compiler and its `rustc -vV` output, queried once per test process.
fn compiler_identity() -> &'static (OsString, Vec<u8>) {
    static IDENTITY: OnceLock<(OsString, Vec<u8>)> = OnceLock::new();
    IDENTITY.get_or_init(|| {
        let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
        let version = Command::new(&compiler).arg("-vV").output().unwrap();
        assert!(version.status.success(), "rustc -vV failed");
        (compiler, version.stdout)
    })
}

/// A path in `directory` that no other thread or process uses.
fn private_path(directory: &Path, prefix: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    directory.join(format!(
        "{prefix}-{}-{nanos}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ))
}

/// Publishes `source` as `destination` unless another thread or process did so
/// first; both are then identical, and a published probe is never replaced.
fn publish(source: &Path, destination: &Path) {
    match fs::hard_link(source, destination) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        // A filesystem without hard links gets a complete copy renamed into place.
        Err(_) => {
            if !destination.is_file() {
                let copy = private_path(destination.parent().unwrap(), ".copy");
                fs::copy(source, &copy).unwrap();
                if fs::rename(&copy, destination).is_err() {
                    let _ = fs::remove_file(&copy);
                }
            }
            assert!(
                destination.is_file(),
                "probe publish failed: {}",
                destination.display()
            );
        }
    }
}
pub(crate) fn wait(child: &mut dyn TerminalProcess) {
    // A guard against a hang, not a duration: the shell and helper end within seconds,
    // but a hosted runner's first Windows PowerShell start once took longer than 15 s
    // (and another over 60 s in test_deploy_build).
    let deadline = Instant::now() + Duration::from_secs(120);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            return;
        }
        assert!(Instant::now() < deadline, "owned helper did not exit");
        std::thread::sleep(Duration::from_millis(10));
    }
}
pub(crate) struct Report {
    pub cwd: Vec<u8>,
    pub arguments: Vec<Vec<u8>>,
    pub environment: Vec<(Vec<u8>, Vec<u8>)>,
}
pub(crate) fn report(path: &Path) -> Report {
    fn count(bytes: &mut &[u8]) -> usize {
        let value = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
        *bytes = &bytes[4..];
        value
    }
    fn record(bytes: &mut &[u8]) -> Vec<u8> {
        let length = count(bytes);
        let value = bytes[..length].to_owned();
        *bytes = &bytes[length..];
        value
    }
    let data = fs::read(path).unwrap();
    assert!(data.starts_with(b"MECMD1\0"));
    let mut bytes = &data[7..];
    let cwd = record(&mut bytes);
    let arguments = (0..count(&mut bytes)).map(|_| record(&mut bytes)).collect();
    let environment = (0..count(&mut bytes))
        .map(|_| (record(&mut bytes), record(&mut bytes)))
        .collect();
    assert!(bytes.is_empty());
    Report {
        cwd,
        arguments,
        environment,
    }
}
