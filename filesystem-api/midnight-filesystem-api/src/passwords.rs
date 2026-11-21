//! Protocol 4 additions: member encryption and unreadability reasons in listings,
//! `Unlock`/`UnlockReply` and the password error codes. See PROTOCOL.md.
use crate::{wire::*, *};
use std::io::Cursor;
use zeroize::{Zeroize, Zeroizing};

/// Tag of the `Unlock` request, whose frame carries a password.
pub(crate) const UNLOCK: u8 = 22;
const UNLOCK_REPLY: u8 = 23;

/// How the content of an archive member is encrypted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(u8)]
pub enum Encryption {
    #[default]
    None = 0,
    /// Traditional PKWARE ("ZipCrypto") encryption.
    ZipCrypto = 1,
    /// WinZip AES (AE-1 or AE-2) with a 128-bit key.
    Aes128 = 2,
    /// WinZip AES with a 192-bit key.
    Aes192 = 3,
    /// WinZip AES with a 256-bit key.
    Aes256 = 4,
    /// 7-Zip's AES-256 with a SHA-256 key derivation.
    SevenZipAes256 = 5,
    /// Another method, such as PKWARE strong encryption.
    Unknown = 255,
}

/// Why a member that is not a directory cannot be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Unreadable {
    /// It is encrypted with a method that this worker cannot decrypt.
    Encryption,
    /// It is compressed with a method that this worker cannot decode, named by
    /// 1–`MAX_METHOD_NAME` bytes (`Implode`, `PPMd`, `method 14`).
    Compression(String),
    /// A symbolic or hard link.
    Link,
    /// A name that cannot be a safe basename (`..`, a drive colon, a backslash, NUL).
    UnsafeName,
    /// Several records share its name, and the reader cannot tell them apart.
    AmbiguousName,
    /// A device, pipe, socket or other special record.
    SpecialFile,
    /// Decoding it needs more than this worker's bounds (such as a dictionary).
    Limit,
}

/// The terminal reply to `Unlock`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum UnlockStatus {
    /// The password opens this mount's encrypted members; the worker keeps it.
    Unlocked = 1,
    /// The password does not open them; nothing changed.
    WrongPassword = 2,
    /// The mount has no encrypted member; nothing changed.
    NotEncrypted = 3,
    /// Its encrypted members use only methods this worker cannot decrypt.
    Unsupported = 4,
}

/// A password in transit. Its bytes are zeroized when it is dropped, and `Debug` never
/// prints them. Encoders also zeroize the frame that carried it.
#[derive(Clone, PartialEq, Eq)]
pub struct Password(Zeroizing<Vec<u8>>);
impl Password {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(Zeroizing::new(bytes))
    }
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }
}
impl std::fmt::Debug for Password {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Password(<redacted>)")
    }
}

/// A frame buffer that is zeroized when dropped if it carries a password.
pub(crate) struct Frame {
    pub(crate) bytes: Vec<u8>,
    pub(crate) secret: bool,
}
impl Drop for Frame {
    fn drop(&mut self) {
        if self.secret {
            self.bytes.zeroize();
        }
    }
}

pub(crate) fn tag(message: &Message) -> Option<u8> {
    Some(match message {
        Message::Unlock { .. } => UNLOCK,
        Message::UnlockReply { .. } => UNLOCK_REPLY,
        _ => return None,
    })
}

/// Whether `message` uses anything that protocols before 4 do not have.
pub(crate) fn needs_v4(message: &Message) -> bool {
    tag(message).is_some()
        || matches!(
            message,
            Message::Failed {
                code: ErrorCode::PasswordRequired | ErrorCode::WrongPassword,
                ..
            }
        )
}

pub(crate) fn write(out: &mut Vec<u8>, message: &Message) -> io::Result<()> {
    match message {
        Message::Unlock { password, .. } => bytes(out, password.as_bytes(), MAX_PASSWORD),
        Message::UnlockReply { status, .. } => {
            out.push(*status as u8);
            Ok(())
        }
        _ => Err(invalid()),
    }
}

pub(crate) fn read(tag: u8, request: u64, input: &mut Cursor<&[u8]>) -> io::Result<Message> {
    Ok(match tag {
        UNLOCK => Message::Unlock {
            request,
            password: Password::new(blob(input, MAX_PASSWORD)?),
        },
        UNLOCK_REPLY => Message::UnlockReply {
            request,
            status: match byte(input)? {
                1 => UnlockStatus::Unlocked,
                2 => UnlockStatus::WrongPassword,
                3 => UnlockStatus::NotEncrypted,
                4 => UnlockStatus::Unsupported,
                _ => return Err(invalid()),
            },
        },
        _ => return Err(invalid()),
    })
}

/// The protocol-4 fields that follow a node's timestamp.
pub(crate) fn write_node(out: &mut Vec<u8>, node: &Node) -> io::Result<()> {
    out.push(node.encryption as u8);
    out.push(u8::from(node.contains_encrypted));
    match &node.unreadable {
        None => out.push(0),
        Some(Unreadable::Encryption) => out.push(1),
        Some(Unreadable::Compression(method)) => {
            out.push(2);
            bytes(out, method.as_bytes(), MAX_METHOD_NAME)?;
        }
        Some(Unreadable::Link) => out.push(3),
        Some(Unreadable::UnsafeName) => out.push(4),
        Some(Unreadable::AmbiguousName) => out.push(5),
        Some(Unreadable::SpecialFile) => out.push(6),
        Some(Unreadable::Limit) => out.push(7),
    }
    Ok(())
}

pub(crate) fn read_node(input: &mut Cursor<&[u8]>, node: &mut Node) -> io::Result<()> {
    node.encryption = match byte(input)? {
        0 => Encryption::None,
        1 => Encryption::ZipCrypto,
        2 => Encryption::Aes128,
        3 => Encryption::Aes192,
        4 => Encryption::Aes256,
        5 => Encryption::SevenZipAes256,
        255 => Encryption::Unknown,
        _ => return Err(invalid()),
    };
    node.contains_encrypted = boolean(input)?;
    node.unreadable = match byte(input)? {
        0 => None,
        1 => Some(Unreadable::Encryption),
        2 => Some(Unreadable::Compression(
            String::from_utf8(blob(input, MAX_METHOD_NAME)?).map_err(|_| invalid())?,
        )),
        3 => Some(Unreadable::Link),
        4 => Some(Unreadable::UnsafeName),
        5 => Some(Unreadable::AmbiguousName),
        6 => Some(Unreadable::SpecialFile),
        7 => Some(Unreadable::Limit),
        _ => return Err(invalid()),
    };
    Ok(())
}

/// Protocol-4 rules: a directory has no encryption or reason of its own; every other
/// node is readable exactly when it has no reason; only directories contain encrypted
/// members; a password is nonempty.
pub(crate) fn validate(message: &Message) -> io::Result<()> {
    match message {
        Message::Entries { entries, .. } => {
            for node in entries {
                let valid = if node.kind == Kind::Directory {
                    node.encryption == Encryption::None && node.unreadable.is_none()
                } else {
                    !node.contains_encrypted
                        && node.readable == node.unreadable.is_none()
                        && !matches!(&node.unreadable, Some(Unreadable::Compression(method)) if method.is_empty())
                };
                if !valid {
                    return Err(invalid());
                }
            }
        }
        Message::Unlock { password, .. } if password.as_bytes().is_empty() => {
            return Err(invalid());
        }
        _ => {}
    }
    Ok(())
}
