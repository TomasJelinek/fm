use super::*;

/// Both systems' view, so that this runs the same on Windows and on Linux.
#[test]
fn a_working_folder_or_shell_of_the_other_system_makes_a_command_that_systems() {
    use PathSystem::{Unix as Linux, Windows};
    let place = |current, directory: &str, shell| foreign_place(current, directory, shell);
    // On Windows: a Linux folder, or the POSIX shell its terminal refuses.
    assert_eq!(
        place(Windows, "/home/tom/src 文档", Some("powershell")),
        Some((Linux, "/home/tom/src 文档".to_owned()))
    );
    assert_eq!(
        place(Windows, "${active.path}", Some("posix")),
        Some((Linux, "POSIX shell".to_owned()))
    );
    for shell in [Some("powershell"), Some("cmd"), None] {
        assert_eq!(place(Windows, "C:\\src", shell), None, "{shell:?}");
        assert_eq!(place(Windows, "${active.path}\\build", shell), None);
    }
    // On Linux: a Windows folder, or PowerShell or Command Prompt, which its terminal
    // refuses.
    for directory in ["C:\\Users\\tom", "D:/data", "\\\\server\\share"] {
        assert_eq!(
            place(Linux, directory, Some("posix")),
            Some((Windows, directory.to_owned()))
        );
    }
    assert_eq!(
        place(Linux, "${active.path}", Some("cmd")),
        Some((Windows, "Command Prompt".to_owned()))
    );
    assert_eq!(
        place(Linux, "${active.path}", Some("powershell")),
        Some((Windows, "PowerShell".to_owned()))
    );
    for shell in [Some("posix"), None] {
        assert_eq!(place(Linux, "/home/tom", shell), None, "{shell:?}");
        assert_eq!(place(Linux, "${other.path}/out", shell), None);
    }
    // A folder decides before the shell, and an unknown shell is left to validation.
    assert_eq!(
        place(Linux, "C:\\src", Some("cmd")),
        Some((Windows, "C:\\src".to_owned()))
    );
    assert_eq!(place(Windows, "${active.path}", Some("fish")), None);
}
