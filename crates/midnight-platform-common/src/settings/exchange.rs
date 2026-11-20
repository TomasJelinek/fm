//! Export/Import configuration files. An exchange file is one complete settings
//! document in exactly the native settings-file serialization: a JSON object whose
//! `version` names its schema. Import accepts every schema this build can load and
//! migrates older ones like a saved settings file; it never applies part of a file.
use super::{CURRENT_PREFERENCES_VERSION, MAX_PREFERENCES_BYTES, invalid};
use midnight_core::UserPreferences;
use std::io;

/// The newest settings schema this build reads and writes.
pub const SETTINGS_FILE_VERSION: u32 = CURRENT_PREFERENCES_VERSION;

const UTF8_BOM: &[u8] = b"\xEF\xBB\xBF";

/// Encode a complete settings snapshot exactly like the native settings file.
pub fn encode_settings_file(preferences: UserPreferences) -> io::Result<Vec<u8>> {
    super::encode_preferences(preferences)
}

/// Decode and validate a whole settings file before anything is applied.
///
/// Refuses non-JSON and non-object documents, missing, malformed or newer schema
/// versions, unknown fields and invalid values with a readable `InvalidData` error.
/// An accepted file is also encoded once, so saving it cannot fail on its contents.
/// A leading UTF-8 byte order mark from a text editor is ignored.
pub fn decode_settings_file(bytes: &[u8]) -> io::Result<UserPreferences> {
    if bytes.len() > MAX_PREFERENCES_BYTES {
        return Err(invalid("The file is larger than the 4 MiB settings limit."));
    }
    let bytes = bytes.strip_prefix(UTF8_BOM).unwrap_or(bytes);
    check_version(bytes)?;
    let preferences = super::decode_preferences(bytes)
        .map_err(|error| invalid(format!("The settings are invalid: {}", concise(&error))))?;
    super::encode_preferences_impl(preferences.clone()).map_err(|error| {
        invalid(format!(
            "The settings cannot be saved by this version: {}",
            concise(&error)
        ))
    })?;
    Ok(preferences)
}

/// Identify the document and its schema before the strict decoder reports details:
/// a newer file usually also has fields this build does not know.
fn check_version(bytes: &[u8]) -> io::Result<()> {
    let document: serde_json::Value = serde_json::from_slice(bytes).map_err(|error| {
        invalid(format!(
            "The file is not a JSON settings document: {error}."
        ))
    })?;
    let version = document
        .as_object()
        .ok_or_else(|| invalid("The file is not a Midnight Explorer settings file."))?
        .get("version")
        .ok_or_else(|| {
            invalid(
                "The file is not a Midnight Explorer settings file: it has no settings version.",
            )
        })?;
    match version.as_u64() {
        Some(version) if version > u64::from(SETTINGS_FILE_VERSION) => Err(invalid(format!(
            "The file uses settings version {version} from a newer Midnight Explorer. \
             This version reads settings versions 1 to {SETTINGS_FILE_VERSION}."
        ))),
        Some(1..) => Ok(()),
        _ => Err(invalid(format!(
            "The settings version must be a whole number from 1 to {SETTINGS_FILE_VERSION}."
        ))),
    }
}

/// The strict decoder lists every accepted name after an unknown field or variant.
/// The offending name and its position identify the problem.
fn concise(error: &io::Error) -> String {
    let message = error.to_string();
    // Whole-settings validation reports no path; drop its empty path suffix.
    let message = message.strip_suffix(" ()").unwrap_or(&message);
    let Some(list) = message.find(", expected one of") else {
        return message.to_owned();
    };
    let position = message
        .rfind(" at line ")
        .filter(|position| *position > list)
        .map_or("", |position| &message[position..]);
    format!("{}{position}", &message[..list])
}

#[cfg(test)]
#[path = "exchange_tests.rs"]
mod tests;
