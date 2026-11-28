//! Measurement, not a check: Find by name and by content over a tree of many small
//! files, on the native filesystem as the application searches it (the platform's Find
//! service and locale). Run in a release build, serially, on an otherwise idle machine:
//! `MIDNIGHT_HOT_PATH_ROOT=<folder> cargo test --release -p midnight-engine --lib
//! hot_path_measure -- --ignored --nocapture --test-threads 1`.
//!
//! The tree is created once below `MIDNIGHT_HOT_PATH_ROOT` (else the temporary folder):
//! `find-tree`, 20 × 10 folders of 100 files of 1–8 KiB (20,000 files, about 90 MiB),
//! one file in a hundred holding the searched word. `MIDNIGHT_MEASURE_RUNS` (default 5)
//! searches of each kind run one after another; each prints its time, then the median.
use crate::find_worker::{CurrentDirectory, search_with_location};
use midnight_core::*;
use midnight_platform::LocalFileSystem;
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Instant, SystemTime},
};

const FOLDERS: usize = 20;
const SUBFOLDERS: usize = 10;
const FILES: usize = 100;
const EXTENSIONS: [&str; 5] = ["rs", "txt", "md", "json", "log"];

fn root() -> PathBuf {
    std::env::var_os("MIDNIGHT_HOT_PATH_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("midnight-hot-path-measure"))
}

fn runs() -> usize {
    std::env::var("MIDNIGHT_MEASURE_RUNS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(5)
}

/// Text of file `index`: words of a working tree, `needle` in one file of a hundred.
fn contents(index: usize) -> Vec<u8> {
    let length = 1024 * (1 + index % 8);
    let mut text = String::with_capacity(length + 64);
    let words = ["alpha ", "beta ", "gamma\n", "delta ", "epsilon ", "zeta\n"];
    let mut word = index;
    while text.len() < length {
        text.push_str(words[word % words.len()]);
        word = word.wrapping_mul(31).wrapping_add(7);
    }
    if index % 100 == 42 {
        text.insert_str(text.len() / 2, " needle ");
    }
    text.into_bytes()
}

fn tree() -> PathBuf {
    let tree = root().join("find-tree");
    let marker = tree.join("complete");
    if marker.exists() {
        return tree;
    }
    let mut index = 0;
    for folder in 0..FOLDERS {
        for subfolder in 0..SUBFOLDERS {
            let directory = tree
                .join(format!("area {folder:02}"))
                .join(format!("module {subfolder:02}"));
            fs::create_dir_all(&directory).unwrap();
            for file in 0..FILES {
                let extension = EXTENSIONS[index % EXTENSIONS.len()];
                fs::write(
                    directory.join(format!("source {file:03}.{extension}")),
                    contents(index),
                )
                .unwrap();
                index += 1;
            }
        }
    }
    fs::write(&marker, b"").unwrap();
    tree
}

fn search(tree: &Path, masks: &str, content: Option<&str>) -> (f64, usize, FindProgress) {
    let service = LocalFileSystem::new(None);
    let mut request = FindRequest::new(FindSessionId(1), FindGeneration(1), vec![tree.into()]);
    masks.clone_into(&mut request.masks);
    request.content = content.map(|text| FindContentQuery {
        text: text.into(),
        case_sensitive: false,
        whole_words: false,
        regex: false,
    });
    let mut results = 0;
    let started = Instant::now();
    let summary = search_with_location(
        &service,
        Some(&service),
        &request,
        &Cancellation::default(),
        &mut |update| {
            if let FindUpdate::Batch(batch) = update {
                results += batch.len();
            }
            true
        },
        &CurrentDirectory::default(),
        SystemTime::now(),
    );
    let seconds = started.elapsed().as_secs_f64();
    assert!(matches!(summary.completion, FindCompletion::Completed));
    (seconds, results, summary.progress)
}

fn median(mut values: Vec<f64>) -> f64 {
    values.sort_by(f64::total_cmp);
    values[values.len() / 2]
}

#[test]
#[ignore = "measurement; run explicitly in a release build"]
fn find_by_name_and_content() {
    let tree = tree();
    let files = FOLDERS * SUBFOLDERS * FILES;
    for (label, masks, content, expected) in [
        ("name *.rs", "*.rs", None, Some(files / EXTENSIONS.len())),
        ("name *.*", "*.*", None, None),
        ("content needle", "*.*", Some("needle"), Some(files / 100)),
    ] {
        let mut times = Vec::new();
        for run in 0..runs() {
            let (seconds, results, progress) = search(&tree, masks, content);
            if let Some(expected) = expected {
                assert_eq!(results, expected, "{label}");
            }
            eprintln!(
                "find {label} run {}: {:.3} ms, {results} results, {} examined, {:.0} entries/s",
                run + 1,
                seconds * 1000.0,
                progress.examined,
                progress.examined as f64 / seconds
            );
            times.push(seconds);
        }
        let median = median(times);
        eprintln!("find {label} median: {:.3} ms", median * 1000.0);
    }
}
