//! Notices the user dismissed for good, beside the preferences file but outside its versioned
//! schema: a missing, older or unreadable file only shows a notice again.
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{self, Read},
};

/// The file beside `preferences.json`.
pub const FILE_NAME: &str = "dismissed-notices.json";
const VERSION: u32 = 1;
/// The most recently dismissed keys that are kept.
const MAX_KEYS: usize = 256;
const MAX_KEY_BYTES: usize = 8 * 1024;
const MAX_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u32,
    dismissed: Vec<String>,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// Decode a bounded file of dismissed notice keys.
pub fn read(file: File) -> io::Result<Vec<String>> {
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(invalid("dismissed notices exceed 4 MiB"));
    }
    let stored: Stored =
        serde_json::from_slice(&bytes).map_err(|error| invalid(&error.to_string()))?;
    if stored.version != VERSION {
        return Err(invalid("unsupported dismissed notices version"));
    }
    if stored.dismissed.len() > MAX_KEYS
        || stored.dismissed.iter().any(|key| key.len() > MAX_KEY_BYTES)
    {
        return Err(invalid("dismissed notices exceed their bounds"));
    }
    Ok(stored.dismissed)
}

/// `keys` with `key` added last (once), keeping the most recent [`MAX_KEYS`].
pub fn with_key(mut keys: Vec<String>, key: String) -> io::Result<Vec<String>> {
    if key.len() > MAX_KEY_BYTES {
        return Err(invalid("dismissed notice key too long"));
    }
    keys.retain(|existing| *existing != key);
    keys.push(key);
    let excess = keys.len().saturating_sub(MAX_KEYS);
    keys.drain(..excess);
    Ok(keys)
}

pub fn encode(keys: &[String]) -> io::Result<Vec<u8>> {
    serde_json::to_vec_pretty(&Stored {
        version: VERSION,
        dismissed: keys.to_vec(),
    })
    .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_round_trip_once_each_and_stay_bounded() {
        let mut keys = Vec::new();
        for index in 0..MAX_KEYS + 3 {
            keys = with_key(keys, format!("installation:{index}")).unwrap();
        }
        keys = with_key(keys, "installation:10".into()).unwrap();
        assert_eq!(keys.len(), MAX_KEYS);
        assert_eq!(keys.last().unwrap(), "installation:10");
        assert_eq!(
            keys.iter().filter(|key| *key == "installation:10").count(),
            1
        );
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(FILE_NAME);
        std::fs::write(&path, encode(&keys).unwrap()).unwrap();
        assert_eq!(read(File::open(&path).unwrap()).unwrap(), keys);
        std::fs::write(&path, br#"{"version":2,"dismissed":[]}"#).unwrap();
        assert!(read(File::open(&path).unwrap()).is_err());
        assert!(with_key(Vec::new(), "x".repeat(MAX_KEY_BYTES + 1)).is_err());
    }
}
