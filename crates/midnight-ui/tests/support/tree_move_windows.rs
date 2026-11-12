//! Private inheritable ACL for this owned Windows fixture only.
use std::{os::windows::process::CommandExt, path::Path, process::Command};

pub fn secure(path: &Path) {
    let script = r#"$ErrorActionPreference='Stop'
$p=$env:MIDNIGHT_TREE_MOVE_FIXTURE
$user=[Security.Principal.WindowsIdentity]::GetCurrent().User
# Change only this fresh directory's DACL; setting its existing owner is unnecessary.
$acl=New-Object Security.AccessControl.DirectorySecurity
$acl.SetAccessRuleProtection($true,$false)
$rule=[Security.AccessControl.FileSystemAccessRule]::new($user,'FullControl','ContainerInherit,ObjectInherit','None','Allow')
$acl.AddAccessRule($rule)
[IO.Directory]::SetAccessControl($p,$acl)
"#;
    let output = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", script])
        .env("MIDNIGHT_TREE_MOVE_FIXTURE", path)
        .creation_flags(0x0800_0000)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "private fixture ACL: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}
