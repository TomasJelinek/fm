//! Stable checksum algorithm names, separate from the calculation implementation.
use midnight_core::{ChecksumAlgorithm, ChecksumAlgorithms};
use std::io;

pub(super) fn decode(version: u32, names: Option<Vec<String>>) -> io::Result<ChecksumAlgorithms> {
    match (version, names) {
        (1..=15, None) => Ok(ChecksumAlgorithms::default()),
        (16.., Some(names)) if (1..=4).contains(&names.len()) => {
            let mut algorithms = Vec::with_capacity(names.len());
            for name in names {
                let algorithm = match name.as_str() {
                    "md5" => ChecksumAlgorithm::Md5,
                    "sha1" => ChecksumAlgorithm::Sha1,
                    "sha256" => ChecksumAlgorithm::Sha256,
                    "sha512" => ChecksumAlgorithm::Sha512,
                    _ => return Err(super::invalid("unknown checksum algorithm")),
                };
                if algorithms.contains(&algorithm) {
                    return Err(super::invalid("duplicate checksum algorithm"));
                }
                algorithms.push(algorithm);
            }
            ChecksumAlgorithms::new(&algorithms).map_err(super::invalid)
        }
        _ => Err(super::invalid(
            "missing, unexpected or empty checksum algorithms",
        )),
    }
}

pub(super) fn encode(algorithms: ChecksumAlgorithms) -> Vec<String> {
    algorithms
        .iter()
        .map(|algorithm| {
            match algorithm {
                ChecksumAlgorithm::Md5 => "md5",
                ChecksumAlgorithm::Sha1 => "sha1",
                ChecksumAlgorithm::Sha256 => "sha256",
                ChecksumAlgorithm::Sha512 => "sha512",
            }
            .to_owned()
        })
        .collect()
}
