//! A cold and a warm pass of the Windows icon provider over a System32-like listing,
//! as detailed rows and as thumbnail tiles. Measurement only, so ignored by default:
//!
//! `cargo test --release -p midnight-platform-windows --lib icons_measure -- --ignored --nocapture`
//!
//! The listing is the first 5,000 files of `%SystemRoot%\System32` by name, read in
//! place. Edges are those of a 225 % display: rows 45 px, tiles 216 px. The figures
//! cover the provider calls the engine's icon actor makes, one at a time as it does,
//! not the capture around per-file artwork or the upload of bitmaps to the renderer.
use super::*;
use std::{
    collections::HashSet,
    path::PathBuf,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::{FILETIME, HANDLE},
    System::Threading::{GetCurrentProcess, GetProcessTimes},
};

#[repr(C)]
#[derive(Default)]
struct MemoryCounters {
    cb: u32,
    page_faults: u32,
    peak_working_set: usize,
    working_set: usize,
    quota_peak_paged_pool: usize,
    quota_paged_pool: usize,
    quota_peak_non_paged_pool: usize,
    quota_non_paged_pool: usize,
    pagefile: usize,
    peak_pagefile: usize,
}

#[link(name = "kernel32")]
unsafe extern "system" {
    fn K32GetProcessMemoryInfo(process: HANDLE, counters: *mut MemoryCounters, size: u32) -> i32;
}

/// Peak working set and private bytes of this process, in MiB.
fn memory() -> (f64, f64) {
    let mut counters = MemoryCounters {
        cb: size_of::<MemoryCounters>() as u32,
        ..MemoryCounters::default()
    };
    // SAFETY: a pseudo handle of this process and a sized, writable structure.
    unsafe { K32GetProcessMemoryInfo(GetCurrentProcess(), &mut counters, counters.cb) };
    let mib = |bytes: usize| bytes as f64 / 1024.0 / 1024.0;
    (mib(counters.peak_working_set), mib(counters.pagefile))
}

/// User and kernel time this process has used.
fn cpu() -> Duration {
    let mut times = [FILETIME::default(); 4];
    let [creation, exit, kernel, user] = &mut times;
    // SAFETY: a pseudo handle of this process and four writable times.
    unsafe { GetProcessTimes(GetCurrentProcess(), creation, exit, kernel, user) };
    let ticks =
        |time: &FILETIME| (u64::from(time.dwHighDateTime) << 32) | u64::from(time.dwLowDateTime);
    Duration::from_nanos((ticks(kernel) + ticks(user)) * 100)
}

/// Time to all sharp icons, time to the first screen of them, CPU and loads.
struct Pass {
    name: &'static str,
    total: Duration,
    first_screen: Duration,
    cpu: Duration,
    loads: usize,
    artwork: usize,
    per_file: Vec<Duration>,
}

impl Pass {
    fn report(&self) {
        let mut per_file = self.per_file.clone();
        per_file.sort();
        let percentile = |share: f64| {
            per_file
                .get(
                    ((per_file.len() as f64 * share) as usize)
                        .min(per_file.len().saturating_sub(1)),
                )
                .copied()
                .unwrap_or_default()
        };
        let (peak, private) = memory();
        println!(
            "{:<24} all {:>8.1} ms | first screen {:>6.1} ms | CPU {:>8.1} ms | {:>5} loads, {:>5} with artwork | per program median {:>5.2} ms, p95 {:>5.2} ms | peak working set {:>6.1} MiB, private {:>6.1} MiB",
            self.name,
            self.total.as_secs_f64() * 1e3,
            self.first_screen.as_secs_f64() * 1e3,
            self.cpu.as_secs_f64() * 1e3,
            self.loads,
            self.artwork,
            percentile(0.5).as_secs_f64() * 1e3,
            percentile(0.95).as_secs_f64() * 1e3,
            peak,
            private,
        );
    }
}

fn listing() -> Vec<PathBuf> {
    let system = Path::new(&std::env::var_os("SystemRoot").unwrap()).join("System32");
    let mut files: Vec<PathBuf> = std::fs::read_dir(system)
        .unwrap()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.path())
        .collect();
    files.sort_by_key(|path| path.file_name().unwrap().to_string_lossy().to_lowercase());
    files.truncate(5000);
    files
}

fn type_key(path: &Path) -> IconKey {
    IconKey {
        extension: path.extension().map(Into::into),
        ..IconKey::file()
    }
}

/// Rows: one load per type, in listing order, as the engine asks for each new type.
fn rows(services: &WindowsFileSystem, files: &[PathBuf], edge: u16, name: &'static str) -> Pass {
    const SCREEN: usize = 40;
    let (started, cpu_before) = (Instant::now(), cpu());
    let mut seen = HashSet::new();
    let (mut loads, mut artwork, mut first_screen) = (0, 0, Duration::ZERO);
    for (row, path) in files.iter().enumerate() {
        let key = type_key(path).sized(edge);
        if seen.insert(key.clone()) {
            loads += 1;
            artwork += usize::from(services.load_icon(&key).unwrap().is_some());
        }
        if row + 1 == SCREEN {
            first_screen = started.elapsed();
        }
    }
    Pass {
        name,
        total: started.elapsed(),
        first_screen,
        cpu: cpu() - cpu_before,
        loads,
        artwork,
        per_file: Vec::new(),
    }
}

/// Tiles: each program's own artwork, the type icon of every other file once.
fn tiles(services: &WindowsFileSystem, files: &[PathBuf], edge: u16, name: &'static str) -> Pass {
    const SCREEN: usize = 15;
    let cancel = midnight_core::Cancellation::default();
    let (started, cpu_before) = (Instant::now(), cpu());
    let mut seen = HashSet::new();
    let (mut loads, mut artwork, mut first_screen) = (0, 0, Duration::ZERO);
    let mut per_file = Vec::new();
    for (tile, path) in files.iter().enumerate() {
        let program = path
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"));
        if program {
            let at = Instant::now();
            let own = services.load_file_icon(path, edge, &cancel).unwrap();
            per_file.push(at.elapsed());
            loads += 1;
            artwork += usize::from(own.is_some());
        } else {
            let key = type_key(path).sized(edge);
            if seen.insert(key.clone()) {
                loads += 1;
                artwork += usize::from(services.load_icon(&key).unwrap().is_some());
            }
        }
        if tile + 1 == SCREEN {
            first_screen = started.elapsed();
        }
    }
    Pass {
        name,
        total: started.elapsed(),
        first_screen,
        cpu: cpu() - cpu_before,
        loads,
        artwork,
        per_file,
    }
}

#[test]
#[ignore = "measurement over System32; run in release with --ignored --nocapture"]
fn icons_measure_cold_and_warm_passes_over_a_system32_listing() {
    let files = listing();
    let programs = files
        .iter()
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("exe"))
        })
        .count();
    let services = WindowsFileSystem::new(None);
    let (peak, private) = memory();
    println!(
        "{} files, {} programs, {} types; before: peak working set {peak:.1} MiB, private {private:.1} MiB",
        files.len(),
        programs,
        files
            .iter()
            .map(|path| type_key(path))
            .collect::<HashSet<_>>()
            .len()
    );
    rows(&services, &files, 45, "details, cold").report();
    rows(&services, &files, 45, "details, warm").report();
    tiles(&services, &files, 216, "thumbnails, cold").report();
    tiles(&services, &files, 216, "thumbnails, warm").report();
    let retained = programs * 216 * 216 * 4;
    println!(
        "every program tile at 216 px: {:.1} MiB of bitmaps; the UI's thumbnail cache keeps at most 54 MiB",
        retained as f64 / 1024.0 / 1024.0
    );
}
