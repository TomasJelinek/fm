//! Opt-in end-to-end timing of opening and navigating archives of many members through
//! the sandboxed worker; accepts a trusted installation and owned fixture paths.
//! Per run, each archive gets a new mount: the first listing (Open, root listing and the
//! pane's rows, sort and index), a walk down the last folder of each level and back up
//! (each folder listed again, as the pane does), a size sort of the deepest folder and of
//! the root, and a listing of every folder (what an archive folder-size scan requests).
//! `MEASURE_RUNS` sets the runs (default 5); medians are reported. `MEASURE_HOLD_MS`
//! keeps each last mount open that long after its walk, so the worker's memory can be
//! read from outside.
use midnight_core::{
    ArchiveMount, ArchiveNode, Cancellation, DirectoryIndex, Entry, EntryKind, SortColumn,
    SortSpec, sort_entries,
};
use midnight_filesystem_host::{Catalog, Mount};
use midnight_plugin_host::CapturedFile;
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};

/// The pane's rows, name order and index of a listing.
fn pane(nodes: Vec<ArchiveNode>) -> Vec<Entry> {
    let mut entries: Vec<_> = nodes.into_iter().map(Entry::from_archive).collect();
    sort_entries(&mut entries, SortSpec::default());
    let _index = DirectoryIndex::from_entries(&entries);
    entries
}

fn last_folder(nodes: &[ArchiveNode]) -> Option<u64> {
    nodes
        .iter()
        .filter(|node| node.kind == EntryKind::Directory)
        .max_by(|a, b| a.raw_name.cmp(&b.raw_name))
        .map(|node| node.id.entry)
}

fn median(samples: &mut [Duration]) -> Duration {
    samples.sort();
    samples[samples.len() / 2]
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let install = PathBuf::from(
        args.next()
            .ok_or("expected trusted installation directory")?,
    );
    let runs: usize = std::env::var("MEASURE_RUNS")
        .ok()
        .and_then(|runs| runs.parse().ok())
        .unwrap_or(5);
    let hold = std::env::var("MEASURE_HOLD_MS")
        .ok()
        .and_then(|hold| hold.parse().ok())
        .map(Duration::from_millis);
    let catalog = Arc::new(Catalog::discover(&install)?);
    eprintln!("diagnostics={:?}", catalog.diagnostics);
    catalog.prepare_staging()?;
    let cancel = Cancellation::default();
    for path in args.map(PathBuf::from) {
        let (package, format) = catalog.choose(&path).ok_or("missing archive worker")?;
        // Warm the source cache and the worker executable once, untimed.
        Mount::open_captured(
            package,
            format,
            &path,
            CapturedFile::capture(&path)?,
            &cancel,
        )?
        .release_on_worker()?;
        let mut samples: [Vec<Duration>; 7] = Default::default();
        let mut shape = String::new();
        for run in 0..runs {
            let start = Instant::now();
            let mount = Mount::open_captured(
                package,
                format,
                &path,
                CapturedFile::capture(&path)?,
                &cancel,
            )?;
            let opened = start.elapsed();
            let listed_at = Instant::now();
            let root = mount.list(1, &cancel)?;
            let listed = listed_at.elapsed();
            let root_rows = root.len();
            let mut down = last_folder(&root);
            let mut root_entries = pane(root);
            let first = start.elapsed();

            let start = Instant::now();
            let mut trail = Vec::new();
            let mut deep = Vec::new();
            while let Some(folder) = down {
                trail.push(folder);
                let nodes = mount.list(folder, &cancel)?;
                down = last_folder(&nodes);
                deep = pane(nodes);
            }
            for folder in trail.iter().rev().skip(1).copied().chain([1]) {
                pane(mount.list(folder, &cancel)?);
            }
            let navigated = start.elapsed();

            let size = SortSpec {
                column: SortColumn::Size,
                descending: true,
            };
            let start = Instant::now();
            sort_entries(&mut deep, size);
            sort_entries(&mut root_entries, size);
            let sorted = start.elapsed();

            let start = Instant::now();
            let (mut folders, mut members) = (0usize, 0usize);
            let mut pending = vec![1];
            while let Some(folder) = pending.pop() {
                folders += 1;
                for node in mount.list(folder, &cancel)? {
                    members += 1;
                    if node.kind == EntryKind::Directory {
                        pending.push(node.id.entry);
                    }
                }
            }
            let walked = start.elapsed();
            for (list, sample) in samples
                .iter_mut()
                .zip([opened, listed, first, navigated, sorted, walked])
            {
                list.push(sample);
            }
            shape = format!(
                "bytes={} members={members} folders={folders} root_rows={root_rows} depth={} deep_rows={}",
                std::fs::metadata(&path)?.len(),
                trail.len(),
                deep.len()
            );
            if run + 1 == runs
                && let Some(hold) = hold
            {
                eprintln!("holding worker for {hold:?}");
                std::thread::sleep(hold);
            }
            let start = Instant::now();
            mount.release_on_worker()?;
            samples[6].push(start.elapsed());
        }
        let [opened, listed, first, navigated, sorted, walked, released] = &mut samples;
        eprintln!(
            "{}: open={:?} root_list={:?} first_listing={:?} navigate_down_and_back={:?} \
             size_sort={:?} all_folders={:?} release={:?} (medians of {runs}) {shape}",
            path.display(),
            median(opened),
            median(listed),
            median(first),
            median(navigated),
            median(sorted),
            median(walked),
            median(released),
        );
        eprintln!("  first_listing samples: {first:?}");
        eprintln!("  all_folders samples: {walked:?}");
    }
    Ok(())
}
