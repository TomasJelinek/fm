use midnight_core::TerminalProcess;
use std::{
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Returns the probe executable, compiled once per compiler and probe source.
///
/// Every test process shares `_build/goal/commands/native/<key>/<name>`. A build
/// goes to a private staging directory and is published with a hard link, which
/// never replaces a probe another test or process published (or is running) first.
pub(crate) fn helper(workspace: &Path, name: &str) -> PathBuf {
    let probe = workspace.join("scripts/tests/user-command-probe");
    let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let version = Command::new(&compiler).arg("-vV").output().unwrap();
    assert!(version.status.success(), "rustc -vV failed");
    // FNV-1a over length-prefixed parts: a stable key without extra dependencies.
    let mut key = 0xcbf2_9ce4_8422_2325_u64;
    let parts = [
        compiler.as_encoded_bytes().to_vec(),
        version.stdout,
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
    static BUILDS: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let staging = directory.join(format!(
        ".build-{}-{nanos}-{}",
        std::process::id(),
        BUILDS.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir_all(&staging).unwrap();
    let built = staging.join(name);
    let output = Command::new(&compiler)
        .args(["--edition", "2024"])
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
    match fs::hard_link(&built, &executable) {
        Ok(()) => {}
        // Another test or process published the same probe first; both are identical.
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        // A filesystem without hard links: publish by rename unless another test won.
        Err(_) => {
            if !executable.is_file() && fs::rename(&built, &executable).is_err() {
                assert!(
                    executable.is_file(),
                    "probe publish failed: {}",
                    executable.display()
                );
            }
        }
    }
    // Compiler side files (for example a PDB) stay out of the shared directory.
    let _ = fs::remove_dir_all(&staging);
    executable
}
pub(crate) fn wait(child: &mut dyn TerminalProcess) {
    let deadline = Instant::now() + Duration::from_secs(15);
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
