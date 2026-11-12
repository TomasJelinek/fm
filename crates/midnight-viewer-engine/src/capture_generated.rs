//! Generated documents are immutable content leases, never reopened filesystem paths.
use midnight_core::{GeneratedTextSource, ViewerLaunch};
use midnight_plugin_host::{CapturedFile, ContentSource, Error};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc,
    },
};

const DEFAULT_NAME: &str = "Generated text.txt";
const MAX_READ: u32 = 256 * 1024;
static REVISIONS: AtomicU64 = AtomicU64::new(1);

#[derive(Debug)]
struct Generated {
    name: String,
    source: Arc<dyn GeneratedTextSource>,
    length: u64,
    revision: u64,
    closed: Arc<AtomicBool>,
}

pub(crate) fn prepare(
    launch: &ViewerLaunch,
    closed: &Arc<AtomicBool>,
) -> Result<super::capture::Order, String> {
    let source = launch
        .generated
        .as_ref()
        .ok_or("Missing generated text source")?;
    if launch.archive.is_some() {
        return Err("Ambiguous generated and archive viewer sources".into());
    }
    let revision = REVISIONS
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .map_err(|_| "Generated text revision IDs exhausted")?;
    // This label is presentation only. The source was captured by its producer.
    let name = if launch.focused.as_os_str().is_empty() {
        DEFAULT_NAME.to_owned()
    } else {
        launch.focused.to_string_lossy().into_owned()
    };
    let content = CapturedFile::from_source(Arc::new(Generated {
        name: name.clone(),
        length: source.length(),
        source: source.clone(),
        revision,
        closed: closed.clone(),
    }))
    .map_err(|error| error.to_string())?;
    // There is one already captured file and no producer or native path to open.
    let (sender, ready) = mpsc::channel();
    drop(sender);
    Ok(super::capture::Order {
        files: vec![(PathBuf::from(name), Some(Ok(content)))],
        current: 0,
        ready,
    })
}

impl ContentSource for Generated {
    fn length(&self) -> u64 {
        self.length
    }

    fn display_name(&self) -> String {
        self.name.clone()
    }

    fn version(&self) -> Vec<u8> {
        let mut version = b"generated-file-list-v1".to_vec();
        version.extend_from_slice(&self.revision.to_le_bytes());
        version.extend_from_slice(&self.length.to_le_bytes());
        version
    }

    fn validate(&self) -> Result<(), Error> {
        if self.closed.load(Ordering::Acquire) {
            return Err(Error("Viewer closed".into()));
        }
        if self.source.length() != self.length {
            return Err(Error("Generated text length changed".into()));
        }
        Ok(())
    }

    fn read_at(&self, offset: u64, length: u32) -> Result<Vec<u8>, Error> {
        self.validate()?;
        if length == 0 || length > MAX_READ || offset.checked_add(u64::from(length)).is_none() {
            return Err(Error("Invalid generated text read bounds".into()));
        }
        let wanted = self.length.saturating_sub(offset).min(u64::from(length)) as usize;
        let mut bytes = vec![0; wanted];
        let mut read = 0;
        while read < wanted {
            self.validate()?;
            let count = self
                .source
                .read_at(offset + read as u64, &mut bytes[read..])?;
            if count == 0 || count > wanted - read {
                return Err(Error(
                    "Generated text returned an incomplete or invalid read".into(),
                ));
            }
            read += count;
        }
        self.validate()?;
        Ok(bytes)
    }
}

#[cfg(test)]
#[path = "capture_generated_tests.rs"]
mod tests;
