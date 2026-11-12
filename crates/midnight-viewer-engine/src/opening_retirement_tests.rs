use super::*;
use midnight_core::{DirectoryIndex, Entry, EntryId, EntryKind, GeneratedTextSource};
use std::{io, path::PathBuf, thread::ThreadId, time::Duration};

#[derive(Debug)]
struct Source(std::sync::mpsc::Sender<ThreadId>);
impl Drop for Source {
    fn drop(&mut self) {
        let _ = self.0.send(std::thread::current().id());
    }
}
impl GeneratedTextSource for Source {
    fn length(&self) -> u64 {
        0
    }
    fn read_at(&self, _: u64, _: &mut [u8]) -> io::Result<usize> {
        panic!("rejected source must not be read")
    }
}

fn rejected(invalid_preferences: bool) {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = Service::new(runtime.handle().clone());
    let (drop_sender, drops) = std::sync::mpsc::channel();
    let path = PathBuf::from("diagnostic.txt");
    let entries: Arc<[Entry]> = vec![Entry::new(EntryId(1), path.clone(), EntryKind::File)].into();
    let mut preferences = ViewerPreferences::default();
    if invalid_preferences {
        preferences.text_font.size = 0;
    } else {
        service.windows.store(4, Ordering::Release);
    }
    let result = service.open(
        ViewerLaunch {
            find: None,
            associations: Default::default(),
            tree: None,
            archive: None,
            generated: Some(Arc::new(Source(drop_sender))),
            focused: path,
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            entries,
            preferences: Arc::new(preferences),
            basic: true,
            disabled_plugins: Arc::from([]),
        },
        ViewerAppearance {
            family: "monospace".into(),
            font_size: 14.0,
            foreground: [255; 4],
            background: [0, 0, 0, 255],
            selection: [55; 4],
            accent: [150; 4],
        },
        ViewerGeometry {
            width: 320,
            height: 200,
            scale: 1.0,
        },
    );
    assert!(result.is_err());
    assert_ne!(
        drops.recv_timeout(Duration::from_secs(5)).unwrap(),
        std::thread::current().id(),
        "source retirement must not run on the caller/UI thread"
    );
}

#[test]
fn invalid_preferences_retire_generated_source_off_caller() {
    rejected(true);
}

#[test]
fn window_limit_retires_generated_source_off_caller() {
    rejected(false);
}
