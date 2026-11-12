//! Opt-in installed native/archive/thumbnail-worker probe. All fixtures and cache are private.
//! Usage: archive_thumbnails <protected-runtime-dir> <fixture-root> [qualified-video.avi]
use midnight_core::{ArchiveMount, ArchiveNode, Cancellation};
use midnight_filesystem_host::{Catalog, Mount};
use midnight_native_filesystem_host::InstalledProvider;
use midnight_thumbnail_host::{
    archive_source::ArchiveThumbnailSource,
    decode::Source,
    service::{Capture, Ready, Service, Target, View},
};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct CountedSource {
    source: Arc<dyn Source>,
    reads: Arc<AtomicU64>,
}
impl Source for CountedSource {
    fn length(&self) -> u64 {
        self.source.length()
    }
    fn version(&self) -> Vec<u8> {
        self.source.version()
    }
    fn validate(&self) -> io::Result<()> {
        self.source.validate().inspect_err(|error| {
            eprintln!("probe source validation failed: {error}");
        })
    }
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        self.read_at_cooperative(offset, length, &|| false)
    }
    fn read_at_cooperative(
        &self,
        offset: u64,
        length: u32,
        cancelled: &dyn Fn() -> bool,
    ) -> io::Result<Vec<u8>> {
        self.reads.fetch_add(1, Ordering::Relaxed);
        self.source
            .read_at_cooperative(offset, length, cancelled)
            .inspect_err(|error| {
                eprintln!("probe member read offset={offset} length={length} failed: {error}");
            })
    }
}

fn counted(capture: Capture, reads: Arc<AtomicU64>) -> Capture {
    Arc::new(move |cancel| {
        Ok(Arc::new(CountedSource {
            source: capture(cancel).inspect_err(|error| {
                eprintln!("probe admitted capture failed: {error}");
            })?,
            reads: reads.clone(),
        }))
    })
}

fn require(condition: bool, message: &str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message))
    }
}

fn trace_native_revision(runtime: &Path, path: &Path) {
    let native = InstalledProvider::from_installation(runtime.to_owned(), None);
    match native.capture(path, &|| false) {
        Ok(current) => eprintln!(
            "probe current native source length={} revision={:?}",
            current.length(),
            current.stable_version()
        ),
        Err(failure) => eprintln!("probe current native source capture failed: {failure}"),
    }
}

/// Fixture preparation only: managed endpoint tools can update ChangeTime/ADS after a write.
/// Wait for a stable installed-provider revision and check every authored byte before testing
/// immutable captures. This never repairs/retries a production preview or ignores a revision.
fn settle_fixture(runtime: &Path, path: &Path, expected: &[u8]) -> io::Result<()> {
    let native = InstalledProvider::from_installation(runtime.to_owned(), None);
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut candidate: Option<midnight_plugin_host::CapturedFile> = None;
    let mut stable_since = Instant::now();
    loop {
        require(
            Instant::now() < deadline,
            "Owned fixture revision did not settle within ten seconds",
        )?;
        if candidate.is_none() {
            match native.capture(path, &|| false) {
                Ok(source) => {
                    candidate = Some(source);
                    stable_since = Instant::now();
                }
                Err(error) => eprintln!("probe fixture capture is not settled: {error}"),
            }
        }
        thread::sleep(Duration::from_millis(100));
        let Some(source) = &candidate else { continue };
        if let Err(error) = source.revalidate() {
            eprintln!("probe fixture revision changed before qualification: {error}");
            candidate = None;
            continue;
        }
        if stable_since.elapsed() < Duration::from_secs(2) {
            continue;
        }
        require(
            source.length() == expected.len() as u64,
            "Owned fixture length changed",
        )?;
        for (index, chunk) in expected.chunks(256 * 1024).enumerate() {
            let bytes = source
                .read_at((index * 256 * 1024) as u64, chunk.len() as u32)
                .map_err(io::Error::other)?;
            require(
                bytes == chunk,
                "Owned fixture content changed before qualification",
            )?;
        }
        eprintln!("probe owned fixture revision stable for two seconds; authored bytes verified");
        return Ok(());
    }
}

fn open_archive(runtime: &Path, zip: &Path) -> io::Result<Arc<Mount>> {
    let native = InstalledProvider::from_installation(runtime.to_owned(), None);
    let source = native.capture(zip, &|| false).map_err(io::Error::other)?;
    eprintln!("probe captured native source length={}", source.length());
    let catalog = Catalog::discover(runtime)?;
    let (package, format) = catalog.choose(zip).ok_or_else(|| {
        io::Error::other(format!(
            "No installed ZIP provider: {:?}",
            catalog.diagnostics
        ))
    })?;
    // Service shutdown retires source owners asynchronously on its backend thread.
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        match Mount::open_captured(
            package,
            format,
            zip,
            source.clone(),
            &Cancellation::default(),
        ) {
            Ok(mount) => {
                eprintln!("probe mounted archive id={}", mount.id());
                return Ok(mount);
            }
            Err(error)
                if error.to_string().contains("Two archive mounts")
                    && Instant::now() < deadline =>
            {
                thread::sleep(Duration::from_millis(10));
            }
            Err(error) => {
                eprintln!("probe archive opening failed: {error}");
                match native.capture(zip, &|| false) {
                    Ok(current) => eprintln!(
                        "probe current native source length={} revision={:?}",
                        current.length(),
                        current.stable_version()
                    ),
                    Err(failure) => {
                        eprintln!("probe current native source capture failed: {failure}")
                    }
                }
                return Err(io::Error::other(error));
            }
        }
    }
}

fn member(mount: &Arc<Mount>, name: &[u8]) -> io::Result<ArchiveNode> {
    mount
        .list(1, &Cancellation::default())
        .map_err(io::Error::other)?
        .into_iter()
        .find(|node| node.readable && node.raw_name.as_ref() == name)
        .ok_or_else(|| io::Error::other("Fixture member was not listed readably"))
}

fn capture(mount: &Arc<Mount>, node: ArchiveNode) -> Capture {
    ArchiveThumbnailSource {
        mount: mount.clone(),
        node,
        source_version: mount.address().source_version,
    }
    .capture()
}

fn target(capture: Capture, extension: &str, selected: bool) -> Target {
    Target {
        token: 17,
        edge: 96,
        identity: b"archive-preview-v1".to_vec(),
        extension: extension.into(),
        selected,
        hovered: false,
        capture,
    }
}

fn show(service: &Service, generation: u64, target: Target) -> io::Result<()> {
    require(
        service.set_view(View {
            generation,
            targets: vec![target],
        }),
        "Thumbnail service refused the fixture view",
    )
}

fn ready(service: &Service, generation: u64) -> io::Result<Ready> {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(ready) = service.poll()
            && ready.generation == generation
        {
            require(ready.token == 17, "Unexpected thumbnail identity")?;
            require(
                ready.frame.width > 0
                    && ready.frame.height > 0
                    && ready.frame.rgba.len()
                        == (ready.frame.width * ready.frame.height * 4) as usize,
                "Invalid decoded frame",
            )?;
            return Ok(ready);
        }
        if let Some(notice) = service.take_notice() {
            return Err(io::Error::other(notice));
        }
        if Instant::now() >= deadline {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "No current-generation frame from installed thumbnail worker",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn cache_records(root: &Path) -> io::Result<BTreeSet<PathBuf>> {
    let mut records = BTreeSet::new();
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            for item in fs::read_dir(entry.path())? {
                let item = item?;
                if item.file_type()?.is_file()
                    && item.path().extension().is_some_and(|value| value == "mtc")
                {
                    records.insert(
                        item.path()
                            .strip_prefix(root)
                            .map_err(io::Error::other)?
                            .into(),
                    );
                }
            }
        } else if entry.path().extension().is_some_and(|value| value == "mtc") {
            records.insert(
                entry
                    .path()
                    .strip_prefix(root)
                    .map_err(io::Error::other)?
                    .into(),
            );
        }
    }
    Ok(records)
}

fn check_bmp(ready: &Ready, color: [u8; 4]) -> io::Result<()> {
    require(
        ready.frame.width == 2
            && ready.frame.height == 2
            && ready
                .frame
                .rgba
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| *pixel == color),
        "Installed image decoder did not return the authored BMP pixels",
    )
}

fn write_ppm(path: &Path, ready: &Ready) -> io::Result<()> {
    let mut file = fs::File::create_new(path)?;
    write!(
        file,
        "P6\n{} {}\n255\n",
        ready.frame.width, ready.frame.height
    )?;
    for pixel in ready.frame.rgba.as_chunks::<4>().0 {
        file.write_all(&pixel[..3])?;
    }
    file.sync_all()
}

fn main() -> io::Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    require(
        (2..=3).contains(&args.len()),
        "Usage: archive_thumbnails <protected-runtime-dir> <fixture-root> [qualified-video.avi]",
    )?;
    let runtime = PathBuf::from(&args[0]).canonicalize()?;
    let fixture_root = PathBuf::from(&args[1]);
    require(
        fixture_root.is_absolute()
            && fixture_root
                .components()
                .any(|part| part.as_os_str() == "_build"),
        "Fixture root must be absolute and below _build",
    )?;
    fs::create_dir_all(&fixture_root)?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(io::Error::other)?
        .as_nanos();
    let run = fixture_root.join(format!("run-{}-{stamp}", std::process::id()));
    fs::create_dir(&run)?;
    let zip = run.join("fixture 文档@.zip");
    let cache = run.join("cache");
    let executable = runtime.join(if cfg!(windows) { "me.exe" } else { "me" });
    require(
        executable.is_file(),
        "Installed application executable is missing",
    )?;
    let mut members = vec![("picture 文档@.bmp".as_bytes().to_vec(), bmp([20, 80, 160]))];
    if let Some(video) = args.get(2) {
        let bytes = fs::read(video)?;
        require(
            bytes.len() <= 16 * 1024 * 1024,
            "Video fixture exceeds probe budget",
        )?;
        members.push((b"video.avi".to_vec(), bytes));
    }
    let archive_bytes = stored_zip(&members)?;
    fs::write(&zip, &archive_bytes)?;
    eprintln!("probe fixture created: {}", zip.display());
    settle_fixture(&runtime, &zip, &archive_bytes)?;

    let mount = open_archive(&runtime, &zip)?;
    let first_mount_id = mount.id();
    let image = member(&mount, &members[0].0)?;
    eprintln!(
        "probe image listed id={:?} length={:?}",
        image.id, image.size
    );
    let image_capture = capture(&mount, image);
    require(
        matches!(image_capture(&AtomicBool::new(true)), Err(error) if error.kind() == io::ErrorKind::Interrupted),
        "Cancelled archive preview capture was accepted",
    )?;
    let source = image_capture(&AtomicBool::new(false))?;
    eprintln!("probe initial image source captured");
    let version = source.version();
    if std::env::var_os("MIDNIGHT_ARCHIVE_PROBE_DIRECT_DECODE").is_some() {
        let helper = runtime.join("thumbnailers").join(if cfg!(windows) {
            "midnight-thumbnail-image.exe"
        } else {
            "midnight-thumbnail-image"
        });
        eprintln!("probe direct decoder diagnostic: {}", helper.display());
        let diagnostic = midnight_thumbnail_host::decode::decode(
            &helper,
            &*source,
            midnight_thumbnail_api::Request {
                id: 1,
                source_length: source.length(),
                width: 96,
                height: 96,
                frame: 0,
            },
            &AtomicBool::new(false),
            Duration::from_secs(10),
        );
        match &diagnostic {
            Ok(frame) => eprintln!(
                "probe direct decoder returned {}x{} pixels",
                frame.width, frame.height
            ),
            Err(error) => {
                eprintln!("probe direct decoder failed: {error}");
                trace_native_revision(&runtime, &zip);
            }
        }
        diagnostic?;
    }
    let reads = Arc::new(AtomicU64::new(0));
    let measured = counted(image_capture.clone(), reads.clone());
    let service = Service::start(executable.clone(), cache.clone());
    show(&service, 1, target(measured.clone(), "bmp", false))?;
    eprintln!("probe awaiting first image decode");
    let first = ready(&service, 1).inspect_err(|_| trace_native_revision(&runtime, &zip))?;
    check_bmp(&first, [160, 80, 20, 255])?;
    write_ppm(&run.join("decoded.ppm"), &first)?;
    require(
        reads.load(Ordering::Relaxed) > 0,
        "First decode never read the archive member",
    )?;
    let records = cache_records(&cache)?;
    require(
        records.len() == 1,
        "First decode did not persist exactly one image record",
    )?;

    require(
        service.set_view(View {
            generation: 2,
            targets: Vec::new(),
        }),
        "Empty view rejected",
    )?;
    thread::sleep(Duration::from_millis(150));
    let before = reads.load(Ordering::Relaxed);
    show(&service, 3, target(measured, "bmp", false))?;
    let restored = ready(&service, 3)?;
    check_bmp(&restored, [160, 80, 20, 255])?;
    require(
        first.frame.rgba == restored.frame.rgba,
        "Retained image pixels changed",
    )?;
    require(
        reads.load(Ordering::Relaxed) == before,
        "Restored visible target reread member",
    )?;
    service.stop();
    drop(service);
    drop(image_capture);
    drop(source);
    drop(mount);

    let mount = open_archive(&runtime, &zip)?;
    require(
        mount.id() != first_mount_id,
        "Reopening did not allocate a new mount session",
    )?;
    let reopened_id = mount.id();
    let reopened = capture(&mount, member(&mount, &members[0].0)?);
    let stale_source = reopened(&AtomicBool::new(false))?;
    require(
        stale_source.version() == version,
        "Persistent identity includes transient mount ID",
    )?;
    let reads = Arc::new(AtomicU64::new(0));
    let service = Service::start(executable.clone(), cache.clone());
    show(
        &service,
        4,
        target(counted(reopened.clone(), reads.clone()), "bmp", false),
    )?;
    check_bmp(&ready(&service, 4)?, [160, 80, 20, 255])?;
    require(
        reads.load(Ordering::Relaxed) == 0,
        "Fresh service missed persistent archive cache",
    )?;
    require(
        cache_records(&cache)? == records,
        "Reopened mount changed the disk cache key",
    )?;

    let mut video_frames = BTreeSet::new();
    let mut video_pixels = BTreeSet::new();
    if members.len() == 2 {
        show(
            &service,
            5,
            target(capture(&mount, member(&mount, b"video.avi")?), "avi", true),
        )?;
        let deadline = Instant::now() + Duration::from_secs(90);
        while video_frames.len() < 6 {
            require(
                Instant::now() < deadline,
                "Selected-video frame sequence did not finish",
            )?;
            let preview = ready(&service, 5)?;
            video_frames.insert(preview.frame.frame);
            video_pixels.insert(preview.frame.rgba.clone());
        }
        require(
            video_pixels.len() > 1,
            "Video preview repeated one static image",
        )?;
    }
    let before_change_records = cache_records(&cache)?;
    service.stop();
    drop(service);
    members[0].1 = bmp([80, 20, 160]);
    let changed_bytes = stored_zip(&members)?;
    fs::write(&zip, &changed_bytes)?;
    require(
        stale_source.validate().is_err(),
        "Changed source still validates cached pixels",
    )?;
    require(
        stale_source.read_at(0, 1).is_err(),
        "Changed source still returns member bytes",
    )?;
    require(
        reopened(&AtomicBool::new(false)).is_err(),
        "Changed source can be recaptured",
    )?;
    drop(stale_source);
    drop(reopened);
    drop(mount);

    settle_fixture(&runtime, &zip, &changed_bytes)?;

    let mount = open_archive(&runtime, &zip)?;
    let changed = capture(&mount, member(&mount, &members[0].0)?);
    require(
        changed(&AtomicBool::new(false))?.version() != version,
        "New source revision reused old identity",
    )?;
    let service = Service::start(executable, cache.clone());
    show(&service, 6, target(changed, "bmp", false))?;
    check_bmp(&ready(&service, 6)?, [160, 20, 80, 255])?;
    let new_records = cache_records(&cache)?;
    require(
        new_records
            .difference(&before_change_records)
            .next()
            .is_some(),
        "Changed source produced no new cache record",
    )?;
    service.stop();
    drop(service);
    let report = format!(
        "status=passed\nfirst_mount={first_mount_id}\nreopened_mount={reopened_id}\nimage_decode=true\nrestore_without_member_read=true\npersistent_reopen_without_member_read=true\nchanged_source_rejected=true\nchanged_pixels_decoded=true\nvideo_frames={video_frames:?}\nvideo_distinct_pixels={}\nvideo_qualification={}\ncache_records={records:?}\n",
        video_pixels.len(),
        if video_frames.len() == 6 {
            "passed"
        } else {
            "pending"
        }
    );
    fs::write(run.join("report.txt"), &report)?;
    println!("{report}artifacts={}", run.display());
    Ok(())
}

fn bmp(color: [u8; 3]) -> Vec<u8> {
    let mut bytes = vec![0; 70];
    bytes[..2].copy_from_slice(b"BM");
    bytes[2..6].copy_from_slice(&70u32.to_le_bytes());
    bytes[10..14].copy_from_slice(&54u32.to_le_bytes());
    bytes[14..18].copy_from_slice(&40u32.to_le_bytes());
    bytes[18..22].copy_from_slice(&2u32.to_le_bytes());
    bytes[22..26].copy_from_slice(&2u32.to_le_bytes());
    bytes[26..28].copy_from_slice(&1u16.to_le_bytes());
    bytes[28..30].copy_from_slice(&24u16.to_le_bytes());
    bytes[34..38].copy_from_slice(&16u32.to_le_bytes());
    for row in 0..2 {
        for column in 0..2 {
            let start = 54 + row * 8 + column * 3;
            bytes[start..start + 3].copy_from_slice(&color);
        }
    }
    bytes
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320u32 & 0u32.wrapping_sub(crc & 1));
        }
    }
    !crc
}

fn u16le(bytes: &mut Vec<u8>, value: u16) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn u32le(bytes: &mut Vec<u8>, value: u32) {
    bytes.extend_from_slice(&value.to_le_bytes());
}
fn stored_zip(members: &[(Vec<u8>, Vec<u8>)]) -> io::Result<Vec<u8>> {
    let mut zip = Vec::new();
    let mut central = Vec::new();
    for (name, data) in members {
        let length = u32::try_from(data.len()).map_err(io::Error::other)?;
        let name_length = u16::try_from(name.len()).map_err(io::Error::other)?;
        let offset = u32::try_from(zip.len()).map_err(io::Error::other)?;
        let crc = crc32(data);
        u32le(&mut zip, 0x0403_4b50);
        for field in [20, 0x0800, 0, 0, 33] {
            u16le(&mut zip, field);
        }
        for field in [crc, length, length] {
            u32le(&mut zip, field);
        }
        u16le(&mut zip, name_length);
        u16le(&mut zip, 0);
        zip.extend_from_slice(name);
        zip.extend_from_slice(data);
        u32le(&mut central, 0x0201_4b50);
        for field in [20, 20, 0x0800, 0, 0, 33] {
            u16le(&mut central, field);
        }
        for field in [crc, length, length] {
            u32le(&mut central, field);
        }
        for field in [name_length, 0, 0, 0, 0] {
            u16le(&mut central, field);
        }
        u32le(&mut central, 0);
        u32le(&mut central, offset);
        central.extend_from_slice(name);
    }
    let offset = u32::try_from(zip.len()).map_err(io::Error::other)?;
    let length = u32::try_from(central.len()).map_err(io::Error::other)?;
    zip.extend_from_slice(&central);
    u32le(&mut zip, 0x0605_4b50);
    for field in [0, 0, members.len() as u16, members.len() as u16] {
        u16le(&mut zip, field);
    }
    u32le(&mut zip, length);
    u32le(&mut zip, offset);
    u16le(&mut zip, 0);
    Ok(zip)
}
