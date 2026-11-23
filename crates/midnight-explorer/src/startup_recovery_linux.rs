//! Linux processes inherit terminal behavior without invoking a shell.
use std::process::Command;

/// The restarted application starts with the open-file limit this one started with, so
/// it raises the limit itself and gives the original back to the programs it starts for
/// the user.
pub(super) fn configure(command: &mut Command) {
    midnight_platform::with_original_open_file_limit(command);
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        os::unix::ffi::{OsStrExt, OsStringExt},
        path::Path,
    };

    #[test]
    fn startup_recovery_preserves_native_linux_argument_bytes() {
        let bytes = [b'a', 0xff, b' ', b'b'];
        let argument = OsString::from_vec(bytes.to_vec());
        let command = super::super::software_command(
            Path::new("midnight-explorer"),
            &[argument],
            Path::new("."),
        );
        assert_eq!(command.get_args().next().unwrap().as_bytes(), bytes);
    }
}
