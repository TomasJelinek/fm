//! The administrator terminal as data: no test elevates or shows a consent prompt.
use super::*;
use std::path::PathBuf;

const POWERSHELL: &str = r"C:\Program Files\PowerShell\7\pwsh.exe";
const CMD: &str = r"C:\Windows\System32\cmd.exe";

fn request(shell: ShellFlavor, folder: &str) -> TerminalRequest {
    TerminalRequest {
        command: String::new(),
        working_directory: PathBuf::from(folder),
        shell,
        mode: TerminalMode::Elevated,
    }
}

fn shared(_: char) -> DriveScope {
    DriveScope::Shared
}

#[test]
fn the_shell_its_fixed_arguments_and_the_folder_are_separate_fields() {
    // A folder named `x" & calc & "`, which would add a command to any command line
    // built from it.
    let folder = r#"C:\x" & calc & "\文档 👩🏽‍💻"#;
    for (shell, file, parameters) in [
        (
            ShellFlavor::PowerShell,
            POWERSHELL,
            "-NoLogo -NoProfile -NoExit",
        ),
        (ShellFlavor::Cmd, CMD, "/D /U /V:OFF"),
    ] {
        let launch = elevated_launch(&request(shell, folder), Path::new(file), shared).unwrap();
        assert_eq!(
            launch,
            ElevatedLaunch {
                verb: "runas",
                file: PathBuf::from(file),
                parameters,
                directory: PathBuf::from(folder),
            },
            "{shell:?}"
        );
    }
    // A UNC folder: PowerShell starts in it; Command Prompt cannot.
    let unc = r#"\\server\share\x" & calc & ""#;
    let launch = elevated_launch(
        &request(ShellFlavor::PowerShell, unc),
        Path::new(POWERSHELL),
        shared,
    )
    .unwrap();
    assert_eq!(launch.directory, Path::new(unc));
    let refused = elevated_launch(&request(ShellFlavor::Cmd, unc), Path::new(CMD), shared);
    assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::Unsupported);
}

#[test]
fn only_an_interactive_elevated_shell_with_native_paths_is_started() {
    let refused = |request: &TerminalRequest, file: &str| {
        elevated_launch(request, Path::new(file), shared)
            .unwrap_err()
            .kind()
    };
    let mut with_command = request(ShellFlavor::PowerShell, r"C:\data");
    with_command.command = r"Remove-Item C:\data".into();
    assert_eq!(
        refused(&with_command, POWERSHELL),
        io::ErrorKind::Unsupported
    );
    let mut standard = request(ShellFlavor::Cmd, r"C:\data");
    standard.mode = TerminalMode::Standard;
    assert_eq!(refused(&standard, CMD), io::ErrorKind::InvalidInput);
    assert_eq!(
        refused(&request(ShellFlavor::Posix, r"C:\data"), CMD),
        io::ErrorKind::Unsupported
    );
    for folder in ["relative", "C:\\bad\0path"] {
        assert_eq!(
            refused(&request(ShellFlavor::PowerShell, folder), POWERSHELL),
            io::ErrorKind::InvalidInput
        );
    }
    assert_eq!(
        refused(&request(ShellFlavor::Cmd, r"C:\data"), "cmd.exe"),
        io::ErrorKind::InvalidInput,
        "the shell's path is absolute, never searched"
    );
}

#[test]
fn a_folder_on_a_drive_of_this_sign_in_only_is_refused_before_any_prompt() {
    let scope = |letter| match letter {
        'Z' => DriveScope::Mapped,
        'S' => DriveScope::Substituted,
        _ => DriveScope::Shared,
    };
    let mapped = "network drive mapped for your account only";
    let substituted = "substituted drive defined for your account only";
    for (shell, file) in [
        (ShellFlavor::PowerShell, POWERSHELL),
        (ShellFlavor::Cmd, CMD),
    ] {
        for (folder, drive, reason) in [
            (r"Z:\projects", "drive Z:", mapped),
            (r"z:\projects", "drive Z:", mapped),
            (r"\\?\Z:\projects", "drive Z:", mapped),
            (r"S:\projects", "drive S:", substituted),
        ] {
            let error =
                elevated_launch(&request(shell, folder), Path::new(file), scope).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Unsupported, "{folder}");
            let message = error.to_string();
            assert!(
                message.contains(drive) && message.contains(reason),
                "{shell:?} {folder}: {message}"
            );
        }
        assert!(elevated_launch(&request(shell, r"C:\projects"), Path::new(file), scope).is_ok());
    }
}

#[test]
fn the_system_drive_is_one_an_elevated_process_sees() {
    let system = crate::desktop::system_directory().unwrap();
    let letter = drive_letter(&system).expect("the system directory is on a drive");
    assert_eq!(drive_scope(letter), DriveScope::Shared);
}

#[test]
fn a_declined_consent_prompt_is_the_users_choice_and_other_errors_name_their_cause() {
    let folder = Path::new(r"C:\文档");
    let declined = launch_failure(io::Error::from_raw_os_error(ERROR_CANCELLED as i32), folder);
    assert!(midnight_core::is_terminal_consent_declined(&declined));
    assert_eq!(declined.path, folder);
    // Not found, access denied, an invalid folder: failures that name their cause.
    for code in [2, 5, 267] {
        let failed = launch_failure(io::Error::from_raw_os_error(code), folder);
        assert!(
            !midnight_core::is_terminal_consent_declined(&failed),
            "{code}"
        );
        assert_eq!(failed.operation, "launch administrator terminal");
        assert!(
            failed.message.contains(&format!("os error {code}")),
            "{failed}"
        );
    }
}

#[test]
fn an_elevated_request_takes_the_administrator_path_not_the_normal_spawn() {
    use midnight_core::TerminalService;
    // Refused before any consent prompt could show: a command, in a relative folder.
    let mut request = request(ShellFlavor::Cmd, "relative");
    request.command = "echo never".into();
    let Err(error) = crate::WindowsFileSystem::new(None).launch_terminal(&request) else {
        panic!("an elevated request with a command never starts");
    };
    assert_eq!(error.operation, "launch administrator terminal");
    assert_eq!(error.kind, io::ErrorKind::Unsupported);
}
