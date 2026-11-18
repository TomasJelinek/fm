//! Native Source acceptance through the packaged plugin and installed capture authority.
//! Windows test child of tests.rs; Build owns compilation and qualified native execution.
#[path = "source_native_retirement.rs"]
mod native_retirement;

use super::*;
use midnight_plugin_host::api::viewer::{MODE_SOURCE, MODE_TEXT};
use native_retirement::SourceRuntime;
use std::{cell::Cell, fmt::Write as _, io::Write, path::PathBuf};

mod native_setup {
    //! Explicit catalog and capture setup for the qualified product installation.
    //! Invoked by tests::source_native_acceptance, never by product UI callbacks.
    //!
    //! Merely authoring this file performs no discovery/load/capture. Invoking its
    //! functions requires Root's later native lease and a NEW qualified C product
    //! runtime. Build must verify exact Source DLL hashes/SDK-0.2 provenance outside
    //! Rust before invocation; package version 0.1.0 does not identify the SDK.
    use super::SourceRuntime;
    use crate::{NativeCapture, Service};
    use midnight_native_filesystem_host::{
        Catalog as FilesystemCatalog, InstalledProvider, Package,
    };
    use midnight_plugin_host::{PluginInfo, Runtime, api::viewer::MODE_SOURCE};
    use std::{
        collections::HashMap,
        path::{Path, PathBuf},
        sync::{Arc, atomic::Ordering},
    };

    const SOURCE_ID: &str = "midnight.viewer.source";
    const SOURCE_PACKAGE_VERSION: &str = "0.1.0";

    #[derive(Clone, Debug)]
    pub(super) struct Provenance {
        pub(super) application: PathBuf,
        pub(super) installation: PathBuf,
        pub(super) source: PluginInfo,
        pub(super) source_manifest: PathBuf,
        pub(super) native_package: Package,
        /// Required BUILD provenance, not a version discovered from the DLL or ABI.
        pub(super) required_source_sdk: &'static str,
    }

    pub(super) struct QualifiedService {
        pub(super) service: Service,
        pub(super) runtime: Arc<Runtime>,
        pub(super) provenance: Provenance,
        /// Keep this same authority for every capture; final destruction belongs on
        /// a backend worker after all window/capture owners have been released.
        pub(super) filesystem: Arc<InstalledProvider>,
    }

    /// Backend-test setup only. All discovery roots derive from ONE explicit
    /// genuine product application. No installed-executable registration is changed.
    /// Do not call prepare_catalogs/open before this function has seeded every cache.
    pub(super) fn qualified_service(
        executor: &SourceRuntime,
        application: &Path,
    ) -> Result<QualifiedService, String> {
        if !application.is_absolute() {
            return Err("Source acceptance needs an absolute qualified application path".into());
        }
        let installation = application
            .parent()
            .ok_or("Application has no parent")?
            .to_path_buf();
        // Use Build's actual canonical/verbatim application spelling. These normal
        // hosts retain all existing trust/package/target checks; no path is fabricated.
        let runtime =
            Arc::new(Runtime::discover(&installation).map_err(|error| error.to_string())?);
        let images: Arc<[_]> = midnight_image_viewer_host::discover(application)
            .map_err(|error| error.to_string())?
            .into();
        let web: Arc<[_]> = midnight_web_viewer_host::discover_with_diagnostics(application)
            .map_err(|error| error.to_string())?
            .providers
            .into();
        let source = runtime
            .catalog()
            .iter()
            .find(|plugin| plugin.id == SOURCE_ID)
            .ok_or("Qualified runtime has no compatible native Source plugin")?
            .clone();
        let source_ids = runtime
            .catalog()
            .iter()
            .filter(|plugin| plugin.id == SOURCE_ID)
            .count()
            + images
                .iter()
                .filter(|plugin| plugin.id == SOURCE_ID)
                .count()
            + web.iter().filter(|plugin| plugin.id == SOURCE_ID).count();
        if source_ids != 1 {
            return Err("Source ID is absent or duplicated across viewer transports".into());
        }
        if source.version != SOURCE_PACKAGE_VERSION
            || !source.modes.contains(&MODE_SOURCE)
            || !source.models.iter().any(|model| model == "text-page.v1")
            || ["cpp", "lua", "yaml"].iter().any(|extension| {
                !source
                    .extensions
                    .iter()
                    .any(|found| found.as_str() == *extension)
            })
            || source.package_dir.parent() != Some(installation.join("viewers").as_path())
            || source.entry_library.parent() != Some(source.package_dir.as_path())
        {
            return Err("Source metadata/path differs from the approved package contract".into());
        }
        if runtime.is_loaded(SOURCE_ID) {
            return Err(
                "Source already loaded; start a fresh process for this candidate runtime".into(),
            );
        }
        let native_catalog =
            FilesystemCatalog::discover(&installation).map_err(|error| error.to_string())?;
        let native_package = native_catalog
            .native_package()
            .map_err(|error| error.to_string())?
            .clone();
        if native_package.id != "native.windows" {
            return Err(
                "Source Windows acceptance needs the qualified native.windows package".into(),
            );
        }
        let filesystem = Arc::new(InstalledProvider::from_installation(
            installation.clone(),
            None,
        ));
        let authority = filesystem.clone();
        let capture: NativeCapture = Arc::new(move |path, closed| {
            authority
                .capture(path, &|| closed.load(Ordering::Acquire))
                .map_err(|failure| format!("Native filesystem: {failure}"))
        });
        let service =
            Service::with_native_capture(executor.handle().clone(), executor.spawner(), capture);
        executor.track_spare(&service);
        let expected = (runtime.clone(), images, web);
        cache_explicit_catalogs(&service, &expected)?;
        Ok(QualifiedService {
            service,
            runtime,
            filesystem,
            provenance: Provenance {
                application: application.to_path_buf(),
                installation,
                source_manifest: source.package_dir.join("plugin.toml"),
                source,
                native_package,
                required_source_sdk: "=0.2.0 (verify from immutable Build/Cargo receipt)",
            },
        })
    }

    fn cache_explicit_catalogs(
        service: &Service,
        expected: &crate::Catalogs,
    ) -> Result<(), String> {
        *service
            .runtime
            .lock()
            .map_err(|_| "Runtime cache poisoned")? = Some(expected.0.clone());
        *service.images.lock().map_err(|_| "Image cache poisoned")? = Some(expected.1.clone());
        *service.web.lock().map_err(|_| "Web cache poisoned")? = Some(expected.2.clone());
        // All THREE Some values are required. A missing one would invoke the process-
        // cached installed_executable resolver and could select an unrelated old app.
        let actual = service.catalogs()?;
        if !Arc::ptr_eq(&actual.0, &expected.0)
            || !Arc::ptr_eq(&actual.1, &expected.1)
            || !Arc::ptr_eq(&actual.2, &expected.2)
        {
            return Err(
                "Viewer Service did not retain the three explicit catalog identities".into(),
            );
        }
        Ok(())
    }

    #[derive(Clone, Debug)]
    pub(super) struct RouteAudit {
        pub(super) original_path: PathBuf,
        pub(super) selected_id: String,
        pub(super) selected_mode: u16,
        pub(super) selected_library: PathBuf,
    }

    /// Metadata-only route check through the SAME combined chooser as Worker.
    /// ViewerLaunch must likewise use basic=false, empty associations
    /// and no disabled Source ID. This function does not capture or read that file.
    pub(super) fn require_source_route(
        qualified: &QualifiedService,
        path: &Path,
    ) -> Result<RouteAudit, String> {
        let (runtime, images, web) = qualified.service.catalogs()?;
        if !Arc::ptr_eq(&runtime, &qualified.runtime) {
            return Err("Source route audit observed a different runtime cache".into());
        }
        // Worker applies the same cross-transport duplicate-ID suppression before
        // choosing. Do not audit only native Runtime::choose and miss a process winner.
        let mut identities = HashMap::new();
        for id in runtime
            .catalog()
            .iter()
            .map(|provider| &provider.id)
            .chain(images.iter().map(|provider| &provider.id))
            .chain(web.iter().map(|provider| &provider.id))
        {
            *identities.entry(id.clone()).or_insert(0_usize) += 1;
        }
        let disabled: Vec<_> = identities
            .into_iter()
            .filter_map(|(id, count)| (count > 1).then_some(id))
            .collect();
        let office_renderer = web
            .iter()
            .position(|provider| {
                provider.id == crate::routing::HTML_ID && !disabled.contains(&provider.id)
            })
            .map(|index| index + images.len());
        let route = crate::routing::choose(
            runtime.catalog(),
            images
                .iter()
                .map(|provider| crate::routing::ProcessCandidate {
                    id: &provider.id,
                    extensions: &provider.extensions,
                })
                .chain(web.iter().map(|provider| crate::routing::ProcessCandidate {
                    id: &provider.id,
                    extensions: &provider.extensions,
                })),
            path,
            false,
            &disabled,
            &midnight_core::ViewerAssociations::default(),
            office_renderer,
        )
        .ok_or("No default combined viewer route selected for acceptance path")?;
        let crate::routing::Route::Native { index, mode, .. } = route else {
            return Err("A process/Office viewer won the automatic Source association".into());
        };
        let plugin = &runtime.catalog()[index];
        if plugin.id != SOURCE_ID
            || mode != MODE_SOURCE
            || plugin.entry_library != qualified.provenance.source.entry_library
        {
            return Err(
                "Acceptance path does not automatically select the proven Source DLL".into(),
            );
        }
        Ok(RouteAudit {
            original_path: path.to_path_buf(),
            selected_id: plugin.id.clone(),
            selected_mode: mode,
            selected_library: plugin.entry_library.clone(),
        })
    }
}

const REPRO: &str = "D:/dev/ov-physics/ovclash/source/suite/ovclash_suite_api.cpp";
const WAIT: Duration = Duration::from_secs(45);

#[test]
#[ignore = "SERIAL: requires NEW qualified legitimate C runtime/Source DLL and Build provenance preflight"]
fn source_native_rapid_cpp_lua_yaml_positions_eventually_paint() {
    let executor = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let executor = SourceRuntime::new(executor).unwrap();
    // This helper seeds ALL three explicit catalogs and uses the SAME installed
    // native provider. It must not fall back to the harness installation.
    let application = PathBuf::from(
        std::env::var_os("MIDNIGHT_SOURCE_VIEWER_TEST_APPLICATION")
            .expect("Build must supply the new qualified product application"),
    );
    let qualified = native_setup::qualified_service(&executor, &application).unwrap();
    let service = &qualified.service;
    let work = owned_d_work_root();
    // Copy ONLY our authored Lua/YAML inputs, never the C++ repro.
    let subjects = [
        PathBuf::from(REPRO),
        authored_fixture(&work, "cold-context.lua", lua_fixture().as_bytes()),
        authored_fixture(&work, "cold-context.yaml", yaml_fixture().as_bytes()),
    ];
    let mut report = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(work.join("source-positions.tsv"))
        .unwrap();
    writeln!(report, "application\t{}\ninstallation\t{}\nsource_manifest\t{}\nrequired_sdk\t{}\nnative_package\t{:?}\nnative_authority_refs\t{}",
        qualified.provenance.application.display(),
        qualified.provenance.installation.display(),
        qualified.provenance.source_manifest.display(),
        qualified.provenance.required_source_sdk,
        qualified.provenance.native_package,
        Arc::strong_count(&qualified.filesystem)).unwrap();
    for path in subjects {
        // Existing private route metadata must confirm AUTOMATIC Source choice,
        // not an ID override; initial frame mode is checked again below.
        let route = native_setup::require_source_route(&qualified, &path).unwrap();
        writeln!(
            report,
            "route\t{}\t{}\t{}\t{}",
            route.original_path.file_name().unwrap().to_string_lossy(),
            route.selected_id,
            route.selected_mode,
            route.selected_library.display()
        )
        .unwrap();
        let targets = if path.as_path() == Path::new(REPRO) {
            [0.82, 0.43, 0.94]
        } else {
            [0.43, 0.82, 0.999]
        };
        let mut destinations: Vec<Destination> = Vec::new();
        for (trial, target) in targets.into_iter().enumerate() {
            let observed = check_destination(service, &path, trial, target, &mut report);
            for previous in &destinations {
                assert!(
                    observed.line != previous.line,
                    "distributed targets did not reach distinct visible Source lines"
                );
                assert!(
                    observed.pixel_fingerprint != previous.pixel_fingerprint,
                    "distributed targets repeated the same visible Source pixels"
                );
            }
            destinations.push(observed);
        }
        check_close_during_burst(service, &path);
        if path.as_path() != Path::new(REPRO) {
            wait_owned_native_handles_released(&path);
        }
    }
    drop(qualified);
    executor.shutdown_timeout(WAIT).unwrap();
}

fn open_auto(service: &Service, path: &Path) -> (WindowSession, ViewerFrame) {
    let name = path.file_name().unwrap().to_str().unwrap();
    let mut request = launch(path.parent().unwrap(), &[name], false);
    request.preferences = Arc::new(ViewerPreferences {
        line_numbers: false,
        wrap: false,
        ..Default::default()
    });
    let session = service
        .open(
            request,
            appearance(),
            ViewerGeometry {
                width: 900,
                // Seven-ish visible rows let the authored final code/dedent
                // region occupy the viewport instead of only a loaded fragment.
                height: 120,
                scale: 1.0,
            },
        )
        .unwrap();
    let initial = frame(&session, |_| true);
    assert_eq!(
        initial.mode, MODE_SOURCE,
        "automatic association did not choose Source"
    );
    assert!(
        initial.raster.is_none(),
        "CPU viewport acceptance deliberately excludes retained-raster coverage"
    );
    (session, initial)
}

fn check_destination(
    service: &Service,
    path: &Path,
    trial: usize,
    target: f64,
    report: &mut impl Write,
) -> Destination {
    let trace = FailurePhase::new(path, trial, target);
    trace.at("control", "initial", 0);
    let (slow, mut control) = open_auto(service, path);
    let slow_initial_pixels = control.pixels.clone();
    let slow_initial_line = Destination::parse(&control).map(|value| value.line);
    let mut sequence = control.position_sequence[0];
    // Warm only THIS session using small changes, mirroring the reported working
    // path. The other fresh Source session below must not reuse its checkpoints.
    for step in 1..=32 {
        sequence += 1;
        position(&slow, &control, sequence, target * f64::from(step) / 32.0);
        control = final_ack(
            &slow,
            &control,
            sequence,
            Instant::now() + WAIT,
            trace.at("control", "ack", step as u64),
        );
    }
    trace.at("control", "syntax", 32);
    control = wait_colored_same_destination(&slow, &control);
    assert!(
        visible_paint(&control) && chroma(&control) >= 8,
        "slow control has no syntax at this destination"
    );
    let destination = require_destination(path, trial, &control);
    assert!(
        control.pixels != slow_initial_pixels,
        "slow control ACKs did not move visible pixels from the initial page"
    );
    assert!(
        destination.line > 1 && Some(destination.line) != slow_initial_line,
        "slow control ACKs did not leave the initial visible Source line"
    );

    trace.at("rapid", "initial", 0);
    let (fast, initial) = open_auto(service, path);
    let mut last = initial.position_sequence[0];
    let began = Instant::now();
    let deadline = began + WAIT;
    trace.at("rapid", "enqueue", 0);
    for fraction in [0.12, 0.91, 0.27, 0.76, 0.38, 0.98, 0.04, target] {
        last += 1;
        position(&fast, &initial, last, fraction);
    }
    let ack = final_ack(&fast, &initial, last, deadline, trace.at("rapid", "ack", 8));
    let ack_ms = began.elapsed().as_millis();
    // Colored paint can precede asynchronous Locate/Ln status. Wait for the
    // full reference conjunction instead of comparing a transient status.
    let painted = wait_reference_match(
        &fast,
        &ack,
        &control,
        deadline,
        trace.at("rapid", "reference_match", 8),
    );
    let settled_match_ms = began.elapsed().as_millis();
    let observed = require_destination(path, trial, &painted);
    assert!(
        painted.pixels != initial.pixels,
        "rapid ACKs did not move visible pixels from the initial page"
    );
    assert!(
        observed.line > 1
            && Some(observed.line) != Destination::parse(&initial).map(|value| value.line),
        "rapid ACKs did not leave the initial visible Source line"
    );
    // Final ACK alone proves only content preparation. Equal visible metadata,
    // actual CPU pixel content and chroma distinguish syntax from stale pixels.
    assert_eq!(painted.mode, MODE_SOURCE);
    assert_eq!(painted.width, control.width);
    assert_eq!(painted.height, control.height);
    assert_eq!(painted.status, control.status);
    assert_eq!(painted.position, control.position);
    assert_eq!(painted.vertical_extent, control.vertical_extent);
    // Never dump the supplied C++ text or raw pixels into a failing test log.
    assert!(
        painted.accessible_text == control.accessible_text,
        "visible control text differs"
    );
    assert!(
        painted.pixels == control.pixels,
        "visible control pixels differ"
    );

    // Mode switches reopen at initial_anchor. Explicitly reposition the Text
    // calibration; never assume it preserves the Source destination.
    trace.at("text", "initial", 0);
    assert!(fast.send(ViewerInput::Mode(MODE_TEXT)));
    let plain_initial = frame(&fast, |value| value.mode == MODE_TEXT);
    let plain_sequence = plain_initial.position_sequence[0] + 1;
    position(&fast, &plain_initial, plain_sequence, target);
    let plain = final_ack(
        &fast,
        &plain_initial,
        plain_sequence,
        Instant::now() + WAIT,
        trace.at("text", "ack", 1),
    );
    assert!(
        visible_paint(&plain),
        "plain calibration lacks visible ink/background"
    );
    assert_eq!(chroma(&plain), 0, "plain control contains non-syntax color");
    // This is only the same-file grayscale calibration. Source destination
    // proof above uses the full warm Source viewport, not this fraction alone.
    assert_eq!(plain.position, painted.position);

    writeln!(
        report,
        "{}\t{target}\t{}\t{}\t{last}\t{ack_ms}\t{}\t{}\t{:016x}\t{:016x}\t{}\t{:?}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        path.file_name().unwrap().to_string_lossy(),
        painted.generation,
        painted.revision,
        settled_match_ms,
        chroma(&painted),
        fingerprint(painted.accessible_text.as_bytes()),
        observed.pixel_fingerprint,
        observed.line, observed.column, observed.byte_start, observed.byte_end,
        observed.length, painted.position, painted.vertical_extent,
        painted.horizontal_position, painted.horizontal_extent, painted.width, painted.height
    )
    .unwrap();
    // No source text or C++ image is saved. Digests are non-security fingerprints.
    trace.at("retirement", "windows", 0);
    slow.close();
    fast.close();
    wait_service_idle(service);
    observed
}

// Numeric status fields are current existing observables, not an invented
// exact-byte camera API. Never include the user's loaded C++ text in failures.
struct Destination {
    line: u64,
    column: Option<u64>,
    byte_start: u64,
    byte_end: u64,
    length: u64,
    pixel_fingerprint: u64,
}

impl Destination {
    fn parse(frame: &ViewerFrame) -> Option<Self> {
        let (line, tail) = frame.status.strip_prefix("Ln ")?.split_once(", Col ")?;
        let column = tail.split_whitespace().next()?;
        let mut bytes = frame.status.split_once("Bytes ")?.1.split_whitespace();
        let (start, end) = bytes.next()?.split_once('–')?;
        if bytes.next()? != "/" {
            return None;
        }
        Some(Self {
            line: line.parse().ok()?,
            column: if column == "unavailable" {
                None
            } else {
                Some(column.parse().ok()?)
            },
            byte_start: start.parse().ok()?,
            byte_end: end.parse().ok()?,
            length: bytes.next()?.parse().ok()?,
            pixel_fingerprint: fingerprint(&frame.pixels),
        })
    }
}

fn require_destination(path: &Path, trial: usize, frame: &ViewerFrame) -> Destination {
    let observed =
        Destination::parse(frame).expect("Source destination lacks settled numeric coordinates");
    assert!(
        observed.byte_start <= observed.byte_end && observed.byte_end <= observed.length,
        "Source reported an invalid loaded byte range"
    );
    let fixture = match path.extension().and_then(|value| value.to_str()) {
        Some("lua") => Some((lua_fixture(), "LUA_")),
        Some("yaml") => Some((yaml_fixture(), "YAML_")),
        _ => None,
    };
    if let Some((text, family)) = fixture {
        let lines: Vec<_> = text.lines().collect();
        let line_of = |prefix: &str| -> u64 {
            lines
                .iter()
                .position(|line| line.starts_with(prefix))
                .expect("authored fixture context marker missing") as u64
                + 1
        };
        let (first, last, marker) = if family == "LUA_" {
            match trial {
                0 => (
                    line_of("LUA_COMMENT_0001"),
                    line_of("LUA_COMMENT_0384"),
                    "LUA_COMMENT_",
                ),
                1 => (
                    line_of("LUA_STRING_0001"),
                    line_of("LUA_STRING_0384"),
                    "LUA_STRING_",
                ),
                _ => (
                    line_of("local function accumulate"),
                    lines.len() as u64,
                    "LUA_FINAL_RETURN_TOKEN",
                ),
            }
        } else {
            match trial {
                0 | 1 => (
                    line_of("  YAML_BLOCK_0001"),
                    line_of("  YAML_BLOCK_0512"),
                    "YAML_BLOCK_",
                ),
                _ => (
                    line_of("after_block:"),
                    lines.len() as u64,
                    "YAML_FINAL_STRING_TOKEN",
                ),
            }
        };
        assert!(
            (first..=last).contains(&observed.line),
            "owned fixture landed outside required context: trial={trial}; line={}; expected={first}..={last}",
            observed.line
        );
        assert!(
            frame.accessible_text.contains(marker),
            "owned fixture loaded fragment lacks required context marker"
        );
    }
    observed
}

fn position(session: &WindowSession, context: &ViewerFrame, sequence: u64, fraction: f64) {
    assert!(session.send(ViewerInput::Position {
        generation: context.generation,
        sequence,
        horizontal: false,
        position: fraction,
    }));
}

fn final_ack(
    session: &WindowSession,
    old: &ViewerFrame,
    sequence: u64,
    deadline: Instant,
    phase: WaitPhase,
) -> ViewerFrame {
    let observation = FailedWait::new(phase, deadline, old, sequence, None);
    frame_until(session, deadline, observation, |value| {
        value.generation == old.generation
            && value.revision > old.revision
            && value.position_sequence[0] == sequence
            && !value.accessible_text.is_empty()
            && visible_paint(value)
    })
}

fn wait_colored_same_destination(session: &WindowSession, ack: &ViewerFrame) -> ViewerFrame {
    if visible_paint(ack) && chroma(ack) >= 8 && ack.status.starts_with("Ln ") {
        return ack.clone();
    }
    frame(session, |value| {
        value.generation == ack.generation
            && value.mode == MODE_SOURCE
            && value.revision > ack.revision
            && value.position_sequence == ack.position_sequence
            && value.accessible_text == ack.accessible_text
            && value.width == ack.width
            && value.height == ack.height
            && value.position == ack.position
            && value.status.starts_with("Ln ")
            && visible_paint(value)
            && chroma(value) >= 8
    })
}

fn wait_reference_match(
    session: &WindowSession,
    ack: &ViewerFrame,
    reference: &ViewerFrame,
    deadline: Instant,
    phase: WaitPhase,
) -> ViewerFrame {
    let observation = FailedWait::new(
        phase,
        deadline,
        ack,
        ack.position_sequence[0],
        Some(reference),
    );
    let matches = |value: &ViewerFrame| {
        value.generation == ack.generation
            && value.position_sequence == ack.position_sequence
            && value.revision >= ack.revision
            && value.mode == MODE_SOURCE
            && same_visible_destination(value, ack)
            && same_visible_destination(value, reference)
            && value.status == reference.status
            && value.pixels == reference.pixels
            && visible_paint(value)
            && chroma(value) >= 8
    };
    assert!(
        Instant::now() < deadline,
        "rapid burst exhausted its absolute deadline"
    );
    if matches(ack) {
        assert!(
            Instant::now() < deadline,
            "rapid Source verdict missed absolute deadline"
        );
        ack.clone()
    } else {
        frame_until(session, deadline, observation, matches)
    }
}

fn same_visible_destination(value: &ViewerFrame, expected: &ViewerFrame) -> bool {
    value.mode == expected.mode
        && value.width == expected.width
        && value.height == expected.height
        && value.position == expected.position
        && value.vertical_extent == expected.vertical_extent
        && value.horizontal_position == expected.horizontal_position
        && value.horizontal_extent == expected.horizontal_extent
        && value.wrap_enabled == expected.wrap_enabled
        && value.wrap_available == expected.wrap_available
        && value.accessible_text == expected.accessible_text
        && value.image.is_none()
        && expected.image.is_none()
        && value.raster.is_none()
        && expected.raster.is_none()
        && value.tiles.is_none()
        && expected.tiles.is_none()
}

// A uniform colored surface has chroma but is not readable source. Require both
// opaque body background and contrasting glyph ink in the actual CPU viewport.
fn visible_paint(frame: &ViewerFrame) -> bool {
    let Some(bytes) = (frame.width as usize)
        .checked_mul(frame.height as usize)
        .and_then(|pixels| pixels.checked_mul(4))
    else {
        return false;
    };
    if frame.width == 0 || frame.height == 0 || frame.pixels.len() != bytes {
        return false;
    }
    let background = appearance().background;
    let mut background_pixels = 0;
    let mut ink_pixels = 0;
    for rgba in frame.pixels.as_chunks::<4>().0 {
        if *rgba == background {
            background_pixels += 1;
        } else if rgba[3] >= 200
            && (0..3).any(|channel| rgba[channel].abs_diff(background[channel]) >= 32)
        {
            ink_pixels += 1;
        }
    }
    background_pixels >= (bytes / 4 / 8).max(8) && ink_pixels >= 8
}

#[derive(Clone, Copy)]
struct WaitPhase {
    subject: &'static str,
    trial: usize,
    target: f64,
    lane: &'static str,
    stage: &'static str,
    step: u64,
}

impl std::fmt::Debug for WaitPhase {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("WaitPhase")
            .field("subject", &self.subject)
            .field("trial", &self.trial)
            .field("target", &self.target)
            .field("lane", &self.lane)
            .field("stage", &self.stage)
            .field("step", &self.step)
            .finish()
    }
}

struct FailurePhase {
    current: Cell<WaitPhase>,
    began: Instant,
}

impl FailurePhase {
    fn new(path: &Path, trial: usize, target: f64) -> Self {
        let subject = match path.extension().and_then(|value| value.to_str()) {
            Some("cpp") => "cpp",
            Some("lua") => "lua",
            Some("yaml") => "yaml",
            _ => "other",
        };
        Self {
            current: Cell::new(WaitPhase {
                subject,
                trial,
                target,
                lane: "setup",
                stage: "destination",
                step: 0,
            }),
            began: Instant::now(),
        }
    }

    fn at(&self, lane: &'static str, stage: &'static str, step: u64) -> WaitPhase {
        let phase = WaitPhase {
            lane,
            stage,
            step,
            ..self.current.get()
        };
        self.current.set(phase);
        phase
    }
}

impl Drop for FailurePhase {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ = writeln!(
                std::io::stderr().lock(),
                "Source failure phase: {:?}; destination_elapsed_ms={}",
                self.current.get(),
                self.began.elapsed().as_millis()
            );
        }
    }
}

#[derive(Clone, Copy)]
struct StatusObservation {
    category: &'static str,
    bytes: usize,
}

impl std::fmt::Debug for StatusObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StatusObservation")
            .field("category", &self.category)
            .field("bytes", &self.bytes)
            .finish()
    }
}

impl StatusObservation {
    fn new(status: &str) -> Self {
        let category = if status.is_empty() {
            "empty"
        } else if status.starts_with("Ln ") {
            "source_coordinates"
        } else if status.starts_with("Bytes ") {
            "byte_coordinates"
        } else if status.starts_with("Opening") {
            "opening"
        } else if status.starts_with("Preparing") {
            "preparing"
        } else if status.starts_with("Locating") {
            "locating"
        } else if status.starts_with("Viewer closed") {
            "closed"
        } else {
            "other_redacted"
        };
        Self {
            category,
            bytes: status.len(),
        }
    }
}

#[derive(Clone, Copy)]
struct FrameObservation {
    generation: u64,
    revision: u64,
    sequence: [u64; 2],
    mode: u16,
    dimensions: [u32; 2],
    pixel_bytes: usize,
    accessible_bytes: usize,
    position: f64,
    vertical_extent: f64,
    horizontal_position: f64,
    horizontal_extent: f64,
    wrap_enabled: bool,
    wrap_available: bool,
    image: bool,
    raster: bool,
    tiles: bool,
    status: StatusObservation,
}

impl std::fmt::Debug for FrameObservation {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FrameObservation")
            .field("generation", &self.generation)
            .field("revision", &self.revision)
            .field("sequence", &self.sequence)
            .field("mode", &self.mode)
            .field("dimensions", &self.dimensions)
            .field("pixel_bytes", &self.pixel_bytes)
            .field("accessible_bytes", &self.accessible_bytes)
            .field("position", &self.position)
            .field("vertical_extent", &self.vertical_extent)
            .field("horizontal_position", &self.horizontal_position)
            .field("horizontal_extent", &self.horizontal_extent)
            .field("wrap_enabled", &self.wrap_enabled)
            .field("wrap_available", &self.wrap_available)
            .field("image", &self.image)
            .field("raster", &self.raster)
            .field("tiles", &self.tiles)
            .field("status", &self.status)
            .finish()
    }
}

impl FrameObservation {
    fn new(frame: &ViewerFrame) -> Self {
        // Copy metadata only. Do not retain a frame, clone source strings/pixels,
        // compute image digests or run extra paint scans in the observed path.
        Self {
            generation: frame.generation,
            revision: frame.revision,
            sequence: frame.position_sequence,
            mode: frame.mode,
            dimensions: [frame.width, frame.height],
            pixel_bytes: frame.pixels.len(),
            accessible_bytes: frame.accessible_text.len(),
            position: frame.position,
            vertical_extent: frame.vertical_extent,
            horizontal_position: frame.horizontal_position,
            horizontal_extent: frame.horizontal_extent,
            wrap_enabled: frame.wrap_enabled,
            wrap_available: frame.wrap_available,
            image: frame.image.is_some(),
            raster: frame.raster.is_some(),
            tiles: frame.tiles.is_some(),
            status: StatusObservation::new(&frame.status),
        }
    }
}

struct FailedWait {
    phase: WaitPhase,
    began: Instant,
    deadline: Instant,
    entry_remaining: Duration,
    wanted_sequence: u64,
    expected: FrameObservation,
    reference: Option<FrameObservation>,
    outputs: u64,
    frames: u64,
    opening: Option<Result<(), StatusObservation>>,
    last_status: Option<StatusObservation>,
    last_frame: Option<FrameObservation>,
}

impl FailedWait {
    fn new(
        phase: WaitPhase,
        deadline: Instant,
        expected: &ViewerFrame,
        wanted_sequence: u64,
        reference: Option<&ViewerFrame>,
    ) -> Self {
        let began = Instant::now();
        Self {
            phase,
            began,
            deadline,
            entry_remaining: deadline.saturating_duration_since(began),
            wanted_sequence,
            expected: FrameObservation::new(expected),
            reference: reference.map(FrameObservation::new),
            outputs: 0,
            frames: 0,
            opening: None,
            last_status: None,
            last_frame: None,
        }
    }
}

impl Drop for FailedWait {
    fn drop(&mut self) {
        if std::thread::panicking() {
            let _ = writeln!(
                std::io::stderr().lock(),
                "Source failed wait: phase={:?}; elapsed_ms={}; entry_remaining_ms={}; remaining_ms={}; wanted_sequence={}; outputs={}; frames={}; opening={:?}; last_status={:?}; expected={:?}; reference={:?}; last_frame={:?}",
                self.phase,
                self.began.elapsed().as_millis(),
                self.entry_remaining.as_millis(),
                self.deadline
                    .saturating_duration_since(Instant::now())
                    .as_millis(),
                self.wanted_sequence,
                self.outputs,
                self.frames,
                self.opening,
                self.last_status,
                self.expected,
                self.reference,
                self.last_frame
            );
        }
    }
}

fn frame_until(
    session: &WindowSession,
    deadline: Instant,
    mut observation: FailedWait,
    predicate: impl Fn(&ViewerFrame) -> bool,
) -> ViewerFrame {
    while Instant::now() < deadline {
        match session.mailbox.take_open_result() {
            Some(Err(error)) => {
                observation.opening = Some(Err(StatusObservation::new(&error)));
                panic!("Viewer open failed before the required Source frame");
            }
            Some(Ok(())) => observation.opening = Some(Ok(())),
            None => {}
        }
        for output in session.mailbox.take() {
            observation.outputs = observation.outputs.saturating_add(1);
            match output {
                ViewerOutput::Frame(frame) => {
                    observation.frames = observation.frames.saturating_add(1);
                    observation.last_frame = Some(FrameObservation::new(&frame));
                    if predicate(&frame) {
                        assert!(
                            Instant::now() < deadline,
                            "rapid frame verdict missed absolute deadline"
                        );
                        return *frame;
                    }
                }
                ViewerOutput::Status(status) => {
                    observation.last_status = Some(StatusObservation::new(&status));
                }
                ViewerOutput::Closed => panic!("Viewer closed before the required frame"),
                ViewerOutput::ImageUnavailable { .. } => {
                    panic!("Source input unexpectedly used image failure path")
                }
                _ => {}
            }
        }
        // This is a finite native acceptance wait, not a fresh timeout per stage.
        let remaining = deadline.saturating_duration_since(Instant::now());
        std::thread::sleep(remaining.min(Duration::from_millis(5)));
    }
    panic!("required Source frame did not arrive before the shared absolute deadline")
}

fn chroma(frame: &ViewerFrame) -> usize {
    frame
        .pixels
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|rgba| {
            let maximum = rgba[0].max(rgba[1]).max(rgba[2]);
            let minimum = rgba[0].min(rgba[1]).min(rgba[2]);
            rgba[3] > 32 && maximum - minimum >= 24
        })
        .count()
}

fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x100_0000_01b3)
    })
}

fn check_close_during_burst(service: &Service, path: &Path) {
    let (session, initial) = open_auto(service, path);
    for step in 1..=32 {
        position(
            &session,
            &initial,
            initial.position_sequence[0] + step,
            if step % 2 == 0 { 0.99 } else { 0.05 },
        );
    }
    session.close();
    assert!(!session.send(ViewerInput::Seek(0.5)));
    let deadline = Instant::now() + WAIT;
    loop {
        if session
            .mailbox
            .take()
            .iter()
            .any(|event| matches!(event, ViewerOutput::Closed))
        {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "closed Viewer did not publish Closed"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    wait_service_idle(service);
}

fn wait_service_idle(service: &Service) {
    let deadline = Instant::now() + WAIT;
    while service.windows.load(Ordering::Acquire) != 0 {
        assert!(
            Instant::now() < deadline,
            "closed Viewer worker did not retire within observation bound"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn owned_d_work_root() -> PathBuf {
    let work = PathBuf::from(
        std::env::var_os("MIDNIGHT_SOURCE_VIEWER_TEST_WORK")
            .expect("Build must supply a new owned D _build work directory"),
    );
    let workspace = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let build = workspace.parent().unwrap().parent().unwrap().join("_build");
    assert!(work.is_absolute() && work.starts_with(&build));
    assert!(!work.exists(), "native evidence must use a fresh run root");
    std::fs::create_dir_all(&work).unwrap();
    work
}

fn authored_fixture(work: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = work.join(name);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    file.write_all(bytes).unwrap();
    file.sync_all().unwrap();
    drop(file);
    images::settle_generated_fixture(&path);
    path
}

#[cfg(windows)]
fn wait_owned_native_handles_released(path: &Path) {
    use std::os::windows::fs::OpenOptionsExt;
    let deadline = Instant::now() + WAIT;
    loop {
        match std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(path)
        {
            Ok(file) => {
                drop(file);
                return;
            }
            Err(error) => {
                assert!(
                    Instant::now() < deadline,
                    "owned fixture still has a conflicting native reader: {error}"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    }
}

fn lua_fixture() -> String {
    let mut body = String::from(
        r###"-- Authored native acceptance fixture; no project/customer content.
local enabled = true
local threshold = 0x2A + 3.5e1
local title = 'LUA_SHORT_STRING_TOKEN'
--[==[
"###,
    );
    for index in 1..=384 {
        writeln!(body, r###"LUA_COMMENT_{index:04} local fake = "not code"; return false -- comment context crosses cold checkpoint gap; [=[ decoy ]=]"###).unwrap();
    }
    body.push_str(
        r###"]==]
local after_comment = 'LUA_AFTER_COMMENT_TOKEN'
local payload = [=[
"###,
    );
    for index in 1..=384 {
        writeln!(body, r###"LUA_STRING_{index:04} --[==[ decoy ]==] local fake = 123; return nil; long string context stays open across cold viewport jumps."###).unwrap();
    }
    body.push_str(
        r###"]=]
local function accumulate(values)
    local total = 0
    for index, value in ipairs(values) do
        if enabled and value >= threshold then
            total = total + value + index
        end
    end
    return total, 'LUA_FINAL_RETURN_TOKEN', after_comment, payload
end
print(title, accumulate({ 1, 42, 99 })) -- LUA_FINAL_COMMENT_TOKEN
"###,
    );
    body
}

fn yaml_fixture() -> String {
    let mut body = String::from(
        r###"%YAML 1.2
---
# Authored native acceptance fixture; no project/customer content.
defaults: &defaults
  enabled: true
  retries: 3
  ratio: 1.25
  absent: null
title: 'YAML_SHORT_STRING_TOKEN'
quoted: "YAML_DOUBLE_STRING_TOKEN"
flow: [false, 0x2A, {kind: worker, name: YAML_FLOW_TOKEN}]
description: |+
"###,
    );
    for index in 1..=512 {
        writeln!(body, r###"  YAML_BLOCK_{index:04} fake_key: false # literal text, not a mapping or comment; &decoy [one, two] keeps multiline lexical context."###).unwrap();
    }
    body.push_str(
        r###"after_block: YAML_AFTER_BLOCK_TOKEN
folded: >-
  YAML_FOLDED_FIRST_TOKEN
  YAML_FOLDED_SECOND_TOKEN
services:
  - name: YAML_SERVICE_0001
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0002
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0003
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0004
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0005
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0006
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0007
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0008
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0009
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0010
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0011
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0012
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0013
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0014
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0015
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0016
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0017
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0018
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0019
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0020
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0021
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0022
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0023
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0024
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0025
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0026
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0027
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0028
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0029
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0030
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0031
    <<: *defaults
    labels: {role: worker, ready: true}
  - name: YAML_SERVICE_0032
    <<: *defaults
    labels: {role: worker, ready: true}
final: "YAML_FINAL_STRING_TOKEN" # YAML_FINAL_COMMENT_TOKEN
...
"###,
    );
    body
}

// Native execution requires Build qualification of the new product runtime and Source DLL.
