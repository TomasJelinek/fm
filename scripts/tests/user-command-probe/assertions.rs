use midnight_core::TerminalProcess;
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

pub(crate) fn helper(workspace: &Path, name: &str) -> PathBuf {
    let directory = workspace
        .join("_build/goal/commands/native")
        .join(std::process::id().to_string());
    fs::create_dir_all(&directory).unwrap();
    let executable = directory.join(name);
    let compiler = std::env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let output = Command::new(compiler)
        .args(["--edition", "2024"])
        .arg(workspace.join("scripts/tests/user-command-probe/main.rs"))
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "probe compile: {}",
        String::from_utf8_lossy(&output.stderr)
    );
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
