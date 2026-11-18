//! Windows spool bases: `%TEMP%`, normally the profile's per-user `Local\Temp`.
//! It can be redirected to a directory that other accounts share, so names in it
//! still carry this account's label.
use super::Session;
use std::{
    ffi::{OsStr, OsString},
    path::PathBuf,
};

#[cfg(test)]
#[path = "private_spool_windows_tests.rs"]
mod tests;

pub(super) fn current() -> Session {
    from_environment(std::env::var_os("USERNAME"), std::env::temp_dir())
}

fn from_environment(user: Option<OsString>, temporary: PathBuf) -> Session {
    Session {
        application: "MidnightExplorer",
        cache: None,
        runtime: None,
        account: user
            .filter(|user| !user.is_empty())
            .map(|user| label(&user)),
        temporary,
    }
}

/// The lowercase hex of the account name's encoded bytes: a lossless,
/// file-name-safe label for a name in any script, so two account names never
/// share a root. Ownership is still verified when the root is opened.
fn label(user: &OsStr) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut label = String::with_capacity(user.len() * 2);
    for byte in user.as_encoded_bytes() {
        label.push(char::from(HEX[usize::from(byte >> 4)]));
        label.push(char::from(HEX[usize::from(byte & 0xf)]));
    }
    label
}
