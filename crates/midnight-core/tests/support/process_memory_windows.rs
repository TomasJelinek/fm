/// This process's private memory in bytes (what Task Manager calls its commit size).
pub fn private_bytes() -> u64 {
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-Command",
            &format!(
                "(Get-Process -Id {}).PrivateMemorySize64",
                std::process::id()
            ),
        ])
        .output()
        .expect("run PowerShell");
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse()
        .expect("a byte count")
}
