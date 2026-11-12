//! Opt-in end-to-end archive timing; accepts a trusted installation and owned fixture paths.
use midnight_core::{ArchiveMount, Cancellation, DirectoryIndex, Entry, SortSpec, sort_entries};
use midnight_filesystem_host::{Catalog, Mount};
use midnight_plugin_host::CapturedFile;
use std::{path::PathBuf, time::Instant};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let install = PathBuf::from(
        args.next()
            .ok_or("expected trusted installation directory")?,
    );
    let start = Instant::now();
    let catalog = Catalog::discover(&install)?;
    eprintln!(
        "discovery={:?}, diagnostics={:?}",
        start.elapsed(),
        catalog.diagnostics
    );
    for path in args.map(PathBuf::from) {
        let (package, format) = catalog.choose(&path).ok_or("missing archive worker")?;
        for iteration in 0..3 {
            let start = Instant::now();
            let source = CapturedFile::capture(&path)?;
            let captured = start.elapsed();
            let start = Instant::now();
            let mut offset = 0;
            while offset < source.length() {
                let bytes = source.read_at(offset, 256 * 1024)?;
                if bytes.is_empty() {
                    return Err("unexpected empty source read".into());
                }
                offset += bytes.len() as u64;
            }
            let source_read = start.elapsed();
            let cancel = Cancellation::default();
            let start = Instant::now();
            let mount = Mount::open(package, format, &path, &cancel)?;
            let opened = start.elapsed();
            let start = Instant::now();
            let entries = mount.list(1, &cancel)?;
            let listed = start.elapsed();
            let start = Instant::now();
            let again = mount.list(1, &cancel)?;
            assert_eq!(again.len(), entries.len());
            let relisted = start.elapsed();
            let start = Instant::now();
            let mut pane_entries: Vec<_> = again.into_iter().map(Entry::from_archive).collect();
            sort_entries(&mut pane_entries, SortSpec::default());
            let _index = DirectoryIndex::from_entries(&pane_entries);
            eprintln!(
                "{} iteration={iteration} bytes={} capture={captured:?} source_read={source_read:?} open={opened:?} list={listed:?} relist={relisted:?} pane_index={:?} entries={}",
                path.display(),
                source.length(),
                start.elapsed(),
                entries.len()
            );
            if let Some(entry) = entries.iter().find(|entry| entry.readable) {
                let start = Instant::now();
                let bytes = mount.read(entry.id.entry, 0, 256 * 1024, &cancel)?;
                let first_read = start.elapsed();
                let start = Instant::now();
                let cached = mount.read(entry.id.entry, 0, 256 * 1024, &cancel)?;
                assert_eq!(bytes, cached, "cached read changed member content");
                eprintln!(
                    "first_read={first_read:?} cached_read={:?} read_bytes={}",
                    start.elapsed(),
                    bytes.len()
                );
            }
            drop(mount);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }
    Ok(())
}
