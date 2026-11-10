//! Opt-in tests of actual app composition against independently installed providers.
use super::*;
use midnight_core::{FileCompareLaunch, FileCompareSource, ViewerAppearance, ViewerGeometry};
use std::{
    collections::HashSet,
    sync::atomic::AtomicBool,
    time::{Duration, Instant},
};

fn installation() -> PathBuf {
    std::env::var_os("MIDNIGHT_NATIVE_TEST_INSTALLATION")
        .unwrap()
        .into()
}
fn fixtures() -> PathBuf {
    std::env::var_os("MIDNIGHT_NATIVE_MEDIA_TEST_FILES")
        .unwrap()
        .into()
}
fn provider() -> Arc<InstalledProvider> {
    Arc::new(InstalledProvider::from_installation(installation(), None))
}
fn missing() -> Arc<InstalledProvider> {
    Arc::new(InstalledProvider::from_installation(
        installation().join("no-native-provider"),
        None,
    ))
}

#[test]
#[ignore = "requires protected native installation and owned MIDNIGHT_NATIVE_MEDIA_TEST_FILES"]
fn thumbnails_capture_many_unicode_sources_without_exhausting_readers_or_retargeting() {
    let capture = factory(provider());
    let cancel = AtomicBool::new(false);
    let mut sources = Vec::new();
    // These pre-created owned fixtures exceed the 16-reader cache and preserve exact bytes.
    for index in 0..40 {
        let path = fixtures().join(format!("capture-{index}-文档.bin"));
        let source = thumbnail(capture.clone(), path, false)(&cancel).unwrap();
        assert!(!source.confirmed_local_ssd());
        sources.push(source);
    }
    for _ in 0..2 {
        for (index, source) in sources.iter().enumerate() {
            assert_eq!(source.read_at(0, 4).unwrap(), (index as u32).to_le_bytes());
        }
    }
    std::fs::write(fixtures().join("capture-0-文档.bin"), b"changed source").unwrap();
    assert!(sources[0].validate().is_err());
    assert!(sources[0].read_at(0, 4).is_err());
    assert_eq!(sources[1].read_at(0, 4).unwrap(), 1u32.to_le_bytes());
}

#[test]
#[ignore = "requires protected native installation and owned media fixtures"]
fn thumbnail_capture_has_no_native_fallback_and_honors_cancel() {
    let path = fixtures().join("picture 文档.bmp");
    assert!(path.is_file());
    let absent = thumbnail(factory(missing()), path.clone(), false);
    let error = match absent(&AtomicBool::new(false)) {
        Err(error) => error,
        Ok(_) => panic!("missing provider silently opened native content"),
    };
    assert!(error.to_string().contains("Native filesystem"));
    let cancelled = thumbnail(factory(provider()), path, false);
    assert!(matches!(cancelled(&AtomicBool::new(true)),
        Err(error) if error.kind() == io::ErrorKind::Interrupted));
}

fn open_compare(
    service: &midnight_compare_engine::Service,
    files: [&str; 2],
) -> midnight_compare_engine::Session {
    service
        .open(
            FileCompareLaunch {
                sources: files
                    .map(|name| FileCompareSource::Focused(fixtures().join(name)))
                    .into(),
                disabled_plugins: Arc::from([]),
            },
            ViewerAppearance {
                family: "monospace".into(),
                font_size: 14.0,
                foreground: [240; 4],
                background: [24, 24, 24, 255],
                selection: [80, 100, 180, 255],
                accent: [120; 4],
            },
            ViewerGeometry {
                width: 400,
                height: 250,
                scale: 1.0,
            },
        )
        .unwrap()
}

#[test]
#[ignore = "run test executable inside protected runtime with native and image viewer packages"]
fn actual_text_and_image_comparison_use_installed_native_content() {
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let service = crate::comparator::service(runtime.handle(), provider());
    for (files, image) in [
        (["left 文档.txt", "right.txt"], false),
        (["picture 文档.bmp", "picture 文档.bmp"], true),
    ] {
        let session = open_compare(&service, files);
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut status = String::new();
        loop {
            match session.mailbox.take() {
                Some(midnight_compare_engine::Output::Frame { frame, .. }) => {
                    assert_eq!(frame.image, image);
                    if image {
                        assert!(frame.layers.is_some());
                    } else {
                        assert!(frame.accessible_text.contains("native"));
                    }
                    break;
                }
                Some(midnight_compare_engine::Output::ChooseFiles(choice)) => {
                    panic!("Provider comparison failed: {}", choice.message)
                }
                Some(midnight_compare_engine::Output::Status(text)) => status = text,
                None => {}
            }
            assert!(Instant::now() < deadline, "No comparison frame: {status}");
            std::thread::sleep(Duration::from_millis(5));
        }
        session.close();
    }
    let service = crate::comparator::service(runtime.handle(), missing());
    let session = open_compare(&service, ["left 文档.txt", "right.txt"]);
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        match session.mailbox.take() {
            Some(midnight_compare_engine::Output::Frame { .. }) => {
                panic!("Missing native provider emitted compared content")
            }
            Some(midnight_compare_engine::Output::ChooseFiles(choice)) if choice.is_error => {
                assert!(choice.message.contains("Native filesystem"));
                break;
            }
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "Missing unavailable-provider error"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    session.close();
}

#[test]
#[ignore = "requires protected native and thumbnail packages and owned BMP/AVI fixtures"]
fn actual_thumbnail_service_decodes_image_and_video_from_native_plugin() {
    let cache = fixtures().join(format!("cache-{}", std::process::id()));
    let service = service::Service::start(installation().join("me.exe"), cache);
    let capture = factory(provider());
    let targets = [(1, "picture 文档.bmp", "bmp"), (2, "video.avi", "avi")]
        .into_iter()
        .map(|(token, name, extension)| {
            let path = fixtures().join(name);
            service::Target {
                token,
                edge: 96,
                identity: path.as_os_str().as_encoded_bytes().to_vec(),
                extension: extension.into(),
                selected: token == 2,
                hovered: false,
                capture: thumbnail(capture.clone(), path, false),
            }
        })
        .collect();
    assert!(service.set_view(service::View {
        generation: 1,
        targets
    }));
    let deadline = Instant::now() + Duration::from_secs(30);
    let mut images = false;
    let mut video_frames = HashSet::new();
    while !images || video_frames.len() < 2 {
        if let Some(notice) = service.take_notice() {
            panic!("{notice}");
        }
        if let Some(ready) = service.poll() {
            assert_eq!(ready.generation, 1);
            assert!(!ready.frame.rgba.is_empty());
            if ready.token == 1 {
                images = true;
            } else {
                video_frames.insert(ready.frame.frame);
            }
        }
        assert!(
            Instant::now() < deadline,
            "Missing native preview: image={images}, video={video_frames:?}"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    service.stop();
}
