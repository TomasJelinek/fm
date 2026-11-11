//! Linux processes inherit terminal behavior without invoking a shell.
use std::process::Command;

pub(super) fn configure(_command: &mut Command) {}

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
        let command = super::super::software_command(Path::new("me"), &[argument], Path::new("."));
        assert_eq!(command.get_args().next().unwrap().as_bytes(), bytes);
    }
}
