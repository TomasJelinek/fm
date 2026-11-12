use midnight_core::*;
use std::ffi::OsString;

#[test]
fn expanded_process_data_rejects_nul_relative_paths_and_environment_injection() {
    let directory = std::env::current_dir().unwrap();
    let mut process = UserCommandProcess {
        working_directory: directory,
        invocation: UserCommandProcessInvocation::Program {
            executable: std::env::current_exe().unwrap(),
            arguments: vec!["a & $b; ' \"".into()],
        },
    };
    assert!(process.validate().is_ok());
    if let UserCommandProcessInvocation::Program { arguments, .. } = &mut process.invocation {
        arguments.push("bad\0data".into());
    }
    assert!(process.validate().is_err());
    process.invocation = UserCommandProcessInvocation::Terminal {
        shell: ShellFlavor::PowerShell,
        script: "Write-Output $env:ME_FILE_NAME".into(),
        environment: vec![("PATH".into(), OsString::from("hostile"))],
    };
    assert!(process.validate().is_err());
    if let UserCommandProcessInvocation::Terminal { environment, .. } = &mut process.invocation {
        environment[0].0 = "ME_FILE_NAME".into();
    }
    assert!(process.validate().is_ok());
    if let UserCommandProcessInvocation::Terminal { environment, .. } = &mut process.invocation {
        environment.push(environment[0].clone());
    }
    assert!(process.validate().is_err());
}
