//! A timing, not a check (run it explicitly): Include subfolders over a native tree of
//! 5,000 folders and 40,000 files per side, with and without Compare contents.
use super::*;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, Instant, SystemTime},
};

const TOP: usize = 50;
const SUB: usize = 100;
const FILES: usize = 8;

fn fixture() -> PathBuf {
    let root = std::env::var_os("MIDNIGHT_COMPARE_BENCH_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            let crates = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
            crates
                .parent()
                .unwrap()
                .join("_build")
                .join("compare-bench")
        });
    let done = root.join("ready");
    if done.exists() {
        return root;
    }
    let time = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    for side in ["left", "right"] {
        for top in 0..TOP {
            for sub in 0..SUB {
                let folder = root
                    .join(side)
                    .join(format!("t{top:02}"))
                    .join(format!("s{sub:03}"));
                fs::create_dir_all(&folder).unwrap();
                for file in 0..FILES {
                    let path = folder.join(format!("f{file}.txt"));
                    let mut bytes = format!("{top} {sub} {file} ").repeat(8).into_bytes();
                    // One differing byte in the last file of one folder per top folder.
                    if side == "right" && sub == SUB - 1 && file == FILES - 1 {
                        bytes[0] ^= 1;
                    }
                    fs::write(&path, &bytes).unwrap();
                    fs::File::options()
                        .write(true)
                        .open(&path)
                        .unwrap()
                        .set_modified(time)
                        .unwrap();
                }
            }
        }
    }
    fs::write(&done, b"").unwrap();
    root
}

fn displayed(root: &Path) -> Vec<Entry> {
    let mut entries: Vec<_> = fs::read_dir(root)
        .unwrap()
        .map(|item| item.unwrap())
        .filter(|item| item.file_name() != "ready")
        .enumerate()
        .map(|(index, item)| {
            let metadata = item.metadata().unwrap();
            let kind = if metadata.is_dir() {
                EntryKind::Directory
            } else {
                EntryKind::File
            };
            let mut entry = Entry::new(EntryId(index as u64 + 1), item.path(), kind);
            entry.metadata.size = metadata.is_file().then_some(metadata.len());
            entry.metadata.modified = metadata.modified().ok();
            entry
        })
        .collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

fn request(root: &Path, options: CompareOptions) -> Arc<CompareRequest> {
    let sides = [(PaneId::Left, "left"), (PaneId::Right, "right")].map(|(pane, side)| {
        let path = root.join(side);
        let entries = displayed(&path);
        CompareSide {
            tree: None,
            view: Arc::new(DirectoryIndex::from_entries(&entries)),
            source: VisibleDirectory {
                pane,
                navigation: NavigationId(1),
                path,
                entries: entries.into(),
            },
            view_revision: 0,
            selection_revision: SelectionRevision(0),
        }
    });
    Arc::new(CompareRequest {
        id: CompareId(1),
        sides,
        options,
    })
}

fn bench_run(service: &midnight_platform::LocalFileSystem, request: Arc<CompareRequest>) -> String {
    let cancel = Cancellation::default();
    let mut samples = 0;
    let mut report = |_: &CompareProgress| samples += 1;
    let result = compute::compare(
        request,
        &|| false,
        Some(compute::Deep {
            find: service,
            transfer: Some(service),
            cancel: &cancel,
            report: &mut report,
        }),
    )
    .unwrap();
    format!(
        "selected {} / {}, differing folders {}, walked {} folders {} files, read {} pairs, \
         errors {}, limited {}, marks {}, {samples} samples",
        result.selections[0].len(),
        result.selections[1].len(),
        result.differing_folders,
        result.walked_folders,
        result.walked_files,
        result.content_files,
        result.errors,
        result.limited,
        result.marks.as_ref().map_or(0, |marks| marks.len()),
    )
}

#[test]
#[ignore = "times a native tree of 5,000 folders and 40,000 files per side; run explicitly"]
fn native_subfolder_timing() {
    let root = fixture();
    let service = midnight_platform::LocalFileSystem::new(None);
    for content in [false, true] {
        let options = CompareOptions {
            by_content: content,
            recursive: true,
            ..CompareOptions::default()
        };
        let started = Instant::now();
        let outcome = bench_run(&service, request(&root, options));
        eprintln!("content {content}: {:?}; {outcome}", started.elapsed());
    }
}
