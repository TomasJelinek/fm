pub fn create(link: &std::path::Path, target: &std::path::Path) {
    use std::os::windows::process::CommandExt;
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../midnight-platform/tests/create-junction.ps1");
    let output = std::process::Command::new("powershell.exe")
        .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
        .arg(script)
        .arg("-JunctionPath")
        .arg(link)
        .arg("-TargetPath")
        .arg(target)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW; the fixture needs no visible shell.
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
