pub fn create(link: &std::path::Path, target: &std::path::Path) {
    use std::os::windows::process::CommandExt;
    // `mklink /J` is a cmd.exe builtin. Starting cmd for each fixture junction
    // costs a fraction of a Windows PowerShell start. Both paths are passed
    // quoted and verbatim; cmd would still expand `%NAME%`, so such fixture
    // paths are rejected instead of being reinterpreted.
    for path in [link, target] {
        assert!(
            !path.as_os_str().to_string_lossy().contains(['"', '%']),
            "unsupported junction fixture path: {path:?}"
        );
    }
    let mut command = std::ffi::OsString::from("/d /v:off /c mklink /J \"");
    command.push(link);
    command.push("\" \"");
    command.push(target);
    command.push("\"");
    let output = std::process::Command::new("cmd.exe")
        .raw_arg(command)
        .creation_flags(0x0800_0000) // CREATE_NO_WINDOW; the fixture needs no visible shell.
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "mklink /J {link:?} {target:?}: {}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
