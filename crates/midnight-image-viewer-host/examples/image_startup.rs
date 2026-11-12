//! Read-only cold/hot image startup attribution against an explicit trusted installation.
use midnight_image_viewer_host::{ImageSession, Source, Viewport};
use midnight_plugin_host::CapturedFile;
use std::{
    io,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Default)]
struct Stats {
    first_validation: Option<Duration>,
    first_read: Option<Duration>,
    last_read: Option<Duration>,
    reads: usize,
    bytes: u64,
    validation: Duration,
    reading: Duration,
}
struct Measured {
    file: CapturedFile,
    began: Instant,
    stats: Mutex<Stats>,
}
impl Source for Measured {
    fn length(&self) -> u64 {
        self.file.length()
    }
    fn version(&self) -> Vec<u8> {
        self.file.stable_version()
    }
    fn validate(&self) -> io::Result<()> {
        let began = Instant::now();
        self.stats
            .lock()
            .unwrap()
            .first_validation
            .get_or_insert(self.began.elapsed());
        let result = self.file.revalidate().map_err(io::Error::other);
        self.stats.lock().unwrap().validation += began.elapsed();
        result
    }
    fn read_at(&self, offset: u64, length: u32) -> io::Result<Vec<u8>> {
        let began = Instant::now();
        let result = self.file.read_at(offset, length).map_err(io::Error::other);
        let mut stats = self.stats.lock().unwrap();
        stats
            .first_read
            .get_or_insert(began.duration_since(self.began));
        stats.last_read = Some(self.began.elapsed());
        stats.reading += began.elapsed();
        stats.reads += 1;
        stats.bytes += u64::from(length);
        result
    }
}
fn ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let application = PathBuf::from(args.next().ok_or("application path required")?);
    let image = PathBuf::from(args.next().ok_or("read-only image path required")?);
    for pass in 0..3 {
        let began = Instant::now();
        let providers = midnight_image_viewer_host::discover(&application)?;
        println!("pass={pass} discovery_ms={:.3}", ms(began.elapsed()));
        let provider = providers
            .iter()
            .find(|provider| provider.id == "midnight.viewer.image")
            .ok_or("image provider missing")?;
        let began = Instant::now();
        let file = CapturedFile::capture(&image)?;
        println!("pass={pass} capture_ms={:.3}", ms(began.elapsed()));
        let source = Arc::new(Measured {
            file,
            began: Instant::now(),
            stats: Mutex::default(),
        });
        let cancel = AtomicBool::new(false);
        let mut session =
            ImageSession::open(provider, source.clone(), &cancel, Duration::from_secs(30))?;
        let elapsed = source.began.elapsed();
        let metadata = session.metadata();
        {
            let stats = source.stats.lock().unwrap();
            println!(
                "pass={pass} open_ms={:.3} provider_validation_ms={:.3} first_read_ms={:.3} \
                 last_read_ms={:.3} decode_tail_ms={:.3} reads={} bytes={} validation_ms={:.3} \
                 reading_ms={:.3} width={} height={}",
                ms(elapsed),
                ms(stats.first_validation.unwrap_or_default()),
                ms(stats.first_read.unwrap_or_default()),
                ms(stats.last_read.unwrap_or_default()),
                ms(elapsed.saturating_sub(stats.last_read.unwrap_or_default())),
                stats.reads,
                stats.bytes,
                ms(stats.validation),
                ms(stats.reading),
                metadata.width,
                metadata.height,
            );
        }
        for scale in [1.0, 0.5, 0.25, 1.0] {
            let began = Instant::now();
            let frame = session.render(
                Viewport {
                    width: 256,
                    height: 256,
                    scale,
                    origin_x: 0.0,
                    origin_y: 0.0,
                },
                &cancel,
                Duration::from_secs(15),
            )?;
            println!(
                "pass={pass} tile_scale={scale} tile_ms={:.3} bytes={}",
                ms(began.elapsed()),
                frame.rgba.len()
            );
        }
        cancel.store(true, Ordering::Release);
        session.close();
    }
    Ok(())
}
