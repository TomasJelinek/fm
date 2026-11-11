//! Windows GUI recovery must not create a helper console.
use std::{os::windows::process::CommandExt, process::Command};

pub(super) fn configure(command: &mut Command) {
    command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        os::windows::ffi::{OsStrExt, OsStringExt},
        path::Path,
    };

    #[test]
    fn startup_recovery_preserves_native_windows_argument_code_units() {
        let units = [0x0061, 0xd800, 0x0020, 0x0062];
        let argument = OsString::from_wide(&units);
        let command =
            super::super::software_command(Path::new("me.exe"), &[argument], Path::new("."));
        let actual = command
            .get_args()
            .next()
            .unwrap()
            .encode_wide()
            .collect::<Vec<_>>();
        assert_eq!(actual, units);
    }
}
