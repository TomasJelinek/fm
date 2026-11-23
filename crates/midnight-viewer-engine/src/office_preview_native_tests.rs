//! Installed-CEF regression, ignored pending the native lease and first-content-on-open hook.
//! All rendering, Find, selection and navigation use the real installed CEF worker.
use midnight_core::Cancellation;
use midnight_office_preview::OfficeFormat;
use midnight_plugin_host::CapturedFile;
use midnight_web_viewer_host::{
    PolicyCeiling, Provider, ResourceCancellation, ResourcePathPolicy, Resources, Session, api,
};
use std::{
    collections::VecDeque,
    fmt,
    fs::{self, File, OpenOptions},
    io::Write,
    path::{Component, Path, PathBuf, Prefix},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

// Future canonical destination is src/office_native_fixture/fixture.rs with its
// sibling XLSX assets. Do not import private HTML/plugin sources or rewrite HTML.
#[path = "office_native_fixture/fixture.rs"]
mod fixture;
#[path = "office_native_fixture/read_gate.rs"]
mod read_gate;
use read_gate::ReadGate;
#[path = "office_native_fixture/online_http.rs"]
mod online_http;

const ALPHA: &str = "NATIVE_ALPHA_A1_V1";
const BETA: &str = "NATIVE_BETA_A1_V1";
const CATALOG: &str = "/office/v1/catalog.html";
const ALPHA_FIRST: &str = "/office/v1/sheet/0/first.html";
const BETA_FIRST: &str = "/office/v1/sheet/1/first.html";
const ALPHA_ROWS: &str = "/office/v1/sheet/0/row/129/col/1.html";
const ALPHA_COLUMNS: &str = "/office/v1/sheet/0/row/129/col/33.html";
const ALPHA_FAR: &str = "/office/v1/sheet/0/row/1000000/col/16384.html";
const BETA_ROWS: &str = "/office/v1/sheet/1/row/513/col/1.html";
const BETA_COLUMNS: &str = "/office/v1/sheet/1/row/513/col/33.html";
const STEP_TIMEOUT: Duration = Duration::from_secs(20);
const SOURCE_CHANGED: &str = "Source changed; showing previous contents. Reopen to refresh.";

#[test]
#[ignore = "requires Root native lease, trusted CEF installation and fresh D profile/report env"]
fn html_native_cef_returns_from_local_network_to_the_same_captured_root() {
    let result = (|| {
        let config = Config::read("html-online-captured-root-return")?;
        let provider = config.provider()?;
        let mut server = online_http::Server::start()?;
        let scenario = (|| {
            let path = config.report.join("authored-captured-root.html");
            let (main, reads) = online_http::capture_main(&path, &server.url())?;
            let mut native = Native::open_html(provider, main, &config, "html")?;
            native.phase_observation = Some(PhaseObservation::default());
            let scenario = (|| {
                native.observation_milestone("html-initial-load");
                native.wait_loaded()?;
                let initial = native.last_header.ok_or("missing initial root header")?;
                native.expected_target = format!("https://mewv-{}.invalid/", initial.document);
                native.observation_milestone("html-offline-proof");
                native.prove("01-offline-captured-root", online_http::ROOT_TOKEN)?;
                let offline = native.last_header.ok_or("missing offline root header")?;
                ensure(
                    native.session()?.policy_change_allowed(),
                    "generic HTML lost policy controls",
                )?;
                native.observation_milestone("html-policy-reopen");
                native.send(api::CommandKind::Policy(api::Policy {
                    network: true,
                    scripts: false,
                }))?;
                native.frame = None;
                native.loaded = 0;
                native.loading = 0;
                native.observation_milestone("html-online-load");
                native.wait_loaded()?;
                // A receipt label only; the return capability binding below is
                // derived again after real content/Copy/fresh-paint proof.
                native.expected_target = format!(
                    "https://mewv-{}.invalid/",
                    native
                        .last_header
                        .ok_or("missing new online header")?
                        .document
                );
                native.observation_milestone("html-online-proof");
                native.prove("02-online-captured-root", online_http::ROOT_TOKEN)?;
                // State.url is the provider's fixed policy URL. Actual content and
                // its fresh paint establish this root, then its real Header names it.
                let online = native
                    .last_header
                    .ok_or("missing proved online root header")?;
                ensure(
                    online.session == offline.session && online.document > offline.document,
                    "online policy did not establish a fresh document generation",
                )?;
                let root = format!("https://mewv-{}.invalid/", online.document);
                native.expected_target.clone_from(&root);
                server.set_return_root(&root)?;

                native.observation_milestone("html-network-link");
                native.activate_link(1)?;
                native.observation_milestone("html-http-response");
                native.wait_for(|_| Ok(server.responses() > 0))?;
                native.observation_milestone("html-network-content");
                native.wait_content(online_http::NETWORK_TOKEN)?;
                native.expected_target = server.url();
                native.observation_milestone("html-network-proof");
                native.prove("03-actual-local-network-page", online_http::NETWORK_TOKEN)?;
                native.observation_milestone("html-network-root-absent");
                native.find(online_http::ROOT_TOKEN, 0)?;
                ensure(
                    native.last_header.is_some_and(|header| {
                        header.session == online.session && header.document == online.document
                    }),
                    "network navigation replaced the online host document",
                )?;
                let before = reads.completed_reads.load(Ordering::Acquire);
                let bytes_before = reads.returned_bytes.load(Ordering::Acquire);
                let starts_before = reads.first_reads.load(Ordering::Acquire);
                ensure(
                    before > 0,
                    "captured root never delivered its actual main source",
                )?;

                native.observation_milestone("html-return-link");
                native.activate_link(1)?;
                native.observation_milestone("html-return-main-read");
                native.wait_for(|_| {
                    Ok(reads.completed_reads.load(Ordering::Acquire) > before
                        && reads.first_reads.load(Ordering::Acquire) > starts_before
                        && reads
                            .returned_bytes
                            .load(Ordering::Acquire)
                            .saturating_sub(bytes_before)
                            >= reads.length)
                })?;
                native.observation_milestone("html-return-content");
                native.wait_content(online_http::ROOT_TOKEN)?;
                native.expected_target.clone_from(&root);
                native.observation_milestone("html-return-proof");
                native.prove("04-real-captured-root-return", online_http::ROOT_TOKEN)?;
                native.observation_milestone("html-return-network-absent");
                native.find(online_http::NETWORK_TOKEN, 0)?;
                ensure(
                    native.last_header.is_some_and(|header| {
                        header.session == online.session && header.document == online.document
                    }),
                    "captured return reopened a different host document",
                )?;
                ensure(
                    !native.session()?.finished(),
                    "captured-root return ended the live browser",
                )?;
                server.assert_only_network_page_requests()?;
                let after = reads.completed_reads.load(Ordering::Acquire);
                ensure(
                    after > before,
                    "return did not complete a genuine delegated main-source read",
                )?;
                fs::write(config.report.join("online-return.txt"), format!(
                    "session={}\ndocument={}\nprivate_root={root}\nnetwork_url={}\nmain_completed_reads_before={before}\nmain_completed_reads_after={after}\nmain_returned_bytes_before={bytes_before}\nmain_returned_bytes_after={}\nmain_length={}\nmain_offset_zero_reads_before={starts_before}\nmain_offset_zero_reads_after={}\nnetwork_responses={}\nrequested_network=true\nrequested_scripts=false\nroot_find=1\nnetwork_find_after_return=0\n",
                    online.session, online.document, server.url(),
                    reads.returned_bytes.load(Ordering::Acquire), reads.length,
                    reads.first_reads.load(Ordering::Acquire), server.responses(),
                )).map_err(text)
            })();
            if scenario.is_err() {
                native.observe_phase(|observation| {
                    observation.html_counters = Some((
                        server.responses(),
                        reads.completed_reads.load(Ordering::Acquire),
                        reads.first_reads.load(Ordering::Acquire),
                        reads.returned_bytes.load(Ordering::Acquire),
                        reads.length,
                    ));
                });
            }
            let cleanup = native.finish_case(scenario);
            combine(cleanup, reads.verify_released(&path))
        })();
        // A successful process result requires the actual listening worker to
        // stop/join and relinquish its socket, even after a browser/source failure.
        combine(scenario, server.finish())
    })();
    assert!(
        result.is_ok(),
        "native online captured-root roundtrip failed: {result:?}"
    );
}

#[test]
#[ignore = "requires Root native lease, trusted CEF installation and fresh D profile/report env"]
fn office_xlsx_native_cef_navigates_sheet_and_cell_windows_offline() {
    let result = (|| {
        let config = Config::read("navigation")?;
        let provider = config.provider()?;
        let mut native = Native::open_file(provider, fixture::WORKBOOK_V1, &config, "v1")?;
        native.phase_observation = Some(PhaseObservation::default());
        let scenario = (|| {
            native.observation_milestone("xlsx-01-wait-loaded");
            native.wait_loaded()?;
            // Root acceptance: do not navigate first. Catalog-only prepare is a
            // real regression and must fail here rather than qualify startup.
            native.observation_milestone("xlsx-02-prove");
            native.prove("01-first-content-on-open", ALPHA)?;
            native.observation_milestone("xlsx-03-find");
            native.find(BETA, 0)?;
            native.observation_milestone("xlsx-04-assert-fixed-policy");
            native.assert_fixed_policy()?;
            native.observation_milestone("xlsx-05-prove");
            native.prove("02-policy-rejected-page-intact", ALPHA)?;

            native.observation_milestone("xlsx-06-navigate");
            native.navigate(CATALOG, 1)?; // All sheets
            native.observation_milestone("xlsx-07-prove");
            native.prove("03-catalog-via-all-sheets", "Workbook sheets")?;
            native.observation_milestone("xlsx-08-find");
            native.find("NativeAlpha", 1)?;
            native.observation_milestone("xlsx-09-find");
            native.find("NativeBeta", 1)?;

            native.observation_milestone("xlsx-10-navigate");
            native.navigate(BETA_FIRST, 2)?;
            native.observation_milestone("xlsx-11-prove");
            native.prove("04-second-sheet", BETA)?;
            native.observation_milestone("xlsx-12-prove");
            native.prove(
                "04-beta-escaped-literal",
                "NATIVE_BETA_ESCAPED_<office>&_V1",
            )?;
            native.observation_milestone("xlsx-13-find");
            native.find(ALPHA, 0)?;
            native.observation_milestone("xlsx-14-navigate");
            native.navigate(BETA_ROWS, 6)?; // Next populated region, skipping empty rows
            native.observation_milestone("xlsx-15-prove");
            native.prove("05-beta-sparse-row-window", "NATIVE_BETA_A513_V1")?;
            native.observation_milestone("xlsx-16-navigate");
            native.navigate(BETA_COLUMNS, 6)?; // Next columns
            native.observation_milestone("xlsx-17-prove");
            native.prove("06-beta-column-window", "NATIVE_BETA_AG513_V1")?;

            native.observation_milestone("xlsx-18-navigate");
            native.navigate(CATALOG, 1)?;
            native.observation_milestone("xlsx-19-navigate");
            native.navigate(ALPHA_FIRST, 1)?;
            native.observation_milestone("xlsx-20-prove");
            native.prove("07-first-sheet-revisit", ALPHA)?;
            native.observation_milestone("xlsx-21-navigate");
            native.navigate(ALPHA_ROWS, 4)?; // Next rows
            native.observation_milestone("xlsx-22-prove");
            native.prove("08-alpha-row-window", "NATIVE_ALPHA_A129_V1")?;
            native.observation_milestone("xlsx-23-find");
            native.find(ALPHA, 0)?;
            native.observation_milestone("xlsx-24-navigate");
            native.navigate(ALPHA_COLUMNS, 6)?; // Next columns
            native.observation_milestone("xlsx-25-prove");
            native.prove("09-alpha-column-window", "NATIVE_ALPHA_AG129_V1")?;
            native.observation_milestone("xlsx-26-navigate");
            native.navigate(ALPHA_FAR, 8)?; // Next populated region
            native.observation_milestone("xlsx-27-prove");
            native.prove("10-alpha-far-region", "NATIVE_ALPHA_XFD1000000_V1")?;
            native.observation_milestone("xlsx-28-assert-expected-resources");
            native.assert_expected_resources()?;
            Ok(())
        })();
        native.finish_case(scenario)
    })();
    assert!(
        result.is_ok(),
        "native Office navigation failed: {result:?}"
    );
}

#[test]
#[ignore = "requires Root native lease, trusted CEF installation and fresh D profile/report env"]
fn office_xlsx_native_cef_rejects_changed_parent_and_keeps_rendered_frame() {
    let result = (|| {
        let config = Config::read("parent-change")?;
        let provider = config.provider()?;
        let mut old = Native::open(provider.clone(), fixture::WORKBOOK_V1, &config, "v1")?;
        let scenario = (|| {
            old.wait_loaded()?;
            old.prove("01-before-parent-change", ALPHA)?;
            old.arm_source_refusal(ALPHA_ROWS)?;
            let grants = old.trace.granted.lock().map_err(text)?.len();
            old.source
                .as_ref()
                .ok_or("controlled captured-source observer missing")?
                .revoke();
            old.activate_link(4)?; // Real CEF request for Next rows
            old.wait_source_refusal()?;
            ensure(
                old.trace.granted.lock().map_err(text)?.len() == grants,
                "changed parent returned a new capture from the Office scope",
            )?;
            // A denied generation must not replace the readable committed page.
            let retained = old.prove("02-denied-change-keeps-readable-page", ALPHA);
            if retained.is_err() {
                let diagnostic = old.diagnose_failed_content_proof(ALPHA);
                eprintln!("Office post-failure live-content diagnostic: {diagnostic:?}");
            }
            // Diagnostics cannot turn the original failed Find/Copy/repaint into a pass.
            retained?;
            ensure(
                !old.session()?.finished(),
                "source refusal ended the committed interactive document",
            )?;
            ensure(
                old.trace.granted.lock().map_err(text)?.len() == grants,
                "post-refusal local interaction returned a new Office scope capture",
            )?;
            ensure(
                !old.trace
                    .granted
                    .lock()
                    .map_err(text)?
                    .iter()
                    .any(|path| path == ALPHA_ROWS),
                "stale parent granted a new generated page",
            )?;
            Ok(())
        })();
        old.finish_case(scenario)?;

        let mut replacement = Native::open(provider, fixture::WORKBOOK_V2, &config, "v2")?;
        let scenario = (|| {
            replacement.wait_loaded()?;
            replacement.prove("03-new-parent-revision", "NATIVE_ALPHA_A1_V2")?;
            replacement.find(ALPHA, 0)?;
            replacement.navigate(CATALOG, 1)?;
            replacement.navigate(BETA_FIRST, 2)?;
            replacement.prove("04-new-parent-second-sheet", "NATIVE_BETA_A1_V2")?;
            replacement.find(BETA, 0)?;
            Ok(())
        })();
        replacement.finish_case(scenario)
    })();
    assert!(
        result.is_ok(),
        "native Office parent replacement failed: {result:?}"
    );
}

#[test]
#[ignore = "requires Root native lease, trusted CEF installation and fresh D profile/report env"]
fn office_xlsx_native_cef_rejects_parent_change_after_resolve_before_data() {
    let result = (|| {
        let config = Config::read("parent-change-after-resolve")?;
        let provider = config.provider()?;
        let mut old = Native::open(provider.clone(), fixture::WORKBOOK_V1, &config, "v1")?;
        let gate = Arc::new(ReadGate::new(old.window_closed.clone()));
        let scenario = (|| {
            old.wait_loaded()?;
            old.prove("01-before-resolved-target-revocation", ALPHA)?;
            *old.trace.read_gate.lock().map_err(text)? = Some((ALPHA_ROWS.into(), gate.clone()));
            old.arm_read_refusal(ALPHA_ROWS, gate.clone())?;
            old.activate_link(4)?;
            // The wrapper is reached by the actual host resource Read only after
            // the provider obtained its successful Resolved capability.
            old.wait_for(|_| Ok(gate.entered.load(Ordering::Acquire)))?;
            ensure(
                old.trace
                    .granted
                    .lock()
                    .map_err(text)?
                    .iter()
                    .any(|path| path == ALPHA_ROWS),
                "target read gate entered without a genuine generated capture",
            )?;
            ensure(
                !gate.timed_out.load(Ordering::Acquire),
                "real Read gate expired",
            )?;
            old.source
                .as_ref()
                .ok_or("controlled parent missing")?
                .revoke();
            gate.resume.store(true, Ordering::Release);
            old.wait_source_refusal()?;
            ensure(
                gate.delegated.load(Ordering::Acquire)
                    && gate.delegated_denial.load(Ordering::Acquire),
                "target refusal did not come from its real checked generated Read",
            )?;
            let reads = old
                .source
                .as_ref()
                .ok_or("controlled parent missing")?
                .reads
                .load(Ordering::Acquire);
            old.prove("02-read-denial-keeps-interactive-alpha", ALPHA)?;
            ensure(
                old.source
                    .as_ref()
                    .ok_or("controlled parent missing")?
                    .reads
                    .load(Ordering::Acquire)
                    == reads,
                "local retained-DOM proof reread the revoked original source",
            )?;
            ensure(!old.session()?.finished(), "Read refusal ended the browser")?;
            Ok(())
        })();
        old.finish_case(scenario)?;
        ensure(
            gate.releases.load(Ordering::Acquire) == 1,
            "checked cleanup did not release the wrapped real target exactly once",
        )?;

        let mut replacement = Native::open(provider, fixture::WORKBOOK_V2, &config, "v2")?;
        let scenario = (|| {
            replacement.wait_loaded()?;
            replacement.prove("03-new-parent-after-read-refusal", "NATIVE_ALPHA_A1_V2")?;
            replacement.find(ALPHA, 0)?;
            Ok(())
        })();
        replacement.finish_case(scenario)
    })();
    assert!(
        result.is_ok(),
        "native Office after-Resolved refusal failed: {result:?}"
    );
}

#[test]
#[ignore = "requires Root native lease, trusted CEF installation and fresh D profile/report env"]
fn office_xlsx_native_cef_close_cancels_pending_resource_and_checks_cleanup() {
    let result = (|| {
        let config = Config::read("close-cancel")?;
        let mut native = Native::open(config.provider()?, fixture::WORKBOOK_V1, &config, "v1")?;
        let scenario = (|| {
            native.wait_loaded()?;
            native.prove("01-readable-before-pending-target", ALPHA)?;
            *native.trace.gate_target.lock().map_err(text)? = Some(ALPHA_ROWS.into());
            let grants = native.trace.granted.lock().map_err(text)?.len();
            native.activate_link(4)?;
            native.wait_for(|native| Ok(native.trace.gate_entered.load(Ordering::Acquire)))?;

            native.resize_viewport([932, 624])?;
            // Find can wait for load completion. Real Ctrl+A/Copy instead proves
            // readable old content while the new resource worker turn is blocked.
            let (content_sequence, selection) = native.copy_all(ALPHA)?;
            native.save_proof(
                "02-copy-and-resize-during-pending-target",
                ALPHA,
                content_sequence,
                &selection,
            )?;
            ensure(
                native.trace.granted.lock().map_err(text)?.len() == grants,
                "pending target unexpectedly published while gated",
            )?;

            native.expect_closed = true;
            native.session()?.close();
            native.wait_for(|native| {
                Ok(native.trace.cancellation_seen.load(Ordering::Acquire)
                    && native.trace.gate_finished.load(Ordering::Acquire))
            })?;
            ensure(
                native.trace.gate_returned_denial.load(Ordering::Acquire),
                "real generated scope returned a capture for the cancelled token",
            )?;
            ensure(
                native.trace.late_captures.load(Ordering::Acquire)
                    == native.trace.late_releases.load(Ordering::Acquire),
                "late generated capture did not finish checked release",
            )?;
            ensure(
                native.trace.late_captures.load(Ordering::Acquire) == 0,
                "real generated scope returned late success after cancellation",
            )?;
            ensure(
                native.trace.granted.lock().map_err(text)?.len() == grants,
                "cancelled native navigation published a generated resource",
            )?;
            ensure(
                native
                    .trace
                    .denied
                    .lock()
                    .map_err(text)?
                    .last()
                    .is_some_and(|path| path == ALPHA_ROWS),
                "real generated scope did not deny the cancelled target",
            )?;
            ensure(
                !native.window_closed.load(Ordering::Acquire),
                "test cancelled through its own parent flag instead of host resource cancellation",
            )?;
            ensure(
                !native.session()?.send(api::CommandKind::Copy),
                "closed session accepted input",
            )?;
            Ok(())
        })();
        native.finish_case(scenario)
    })();
    assert!(
        result.is_ok(),
        "native Office close/cancel failed: {result:?}"
    );
}

struct Config {
    application: PathBuf,
    report: PathBuf,
}

impl Config {
    fn read(case: &str) -> Result<Self, String> {
        let application = environment_path("MIDNIGHT_OFFICE_VIEWER_TEST_APPLICATION")?;
        ensure(
            application.is_absolute(),
            "explicit trusted application must be absolute",
        )?;
        let report = environment_path("MIDNIGHT_OFFICE_NATIVE_TEST_REPORT")?;
        let local = environment_path("LOCALAPPDATA")?;
        let build = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .ok_or("workspace root unavailable")?
            .join("_build");
        for (label, path) in [("report", &report), ("LOCALAPPDATA", &local)] {
            ensure_d_build_path(label, path, &build)?;
        }
        for label in ["TEMP", "TMP", "TMPDIR"] {
            ensure_d_build_path(label, &environment_path(label)?, &build)?;
        }
        // No in-process env mutation (unsafe in edition 2024), trust changes,
        // fake midnight-explorer.exe, registration, C staging or inherited private leaf DACL.
        ensure(
            !local.join("MidnightExplorer/web-viewer-sessions").exists(),
            "LOCALAPPDATA must be fresh; leave browser store/session leaves absent",
        )?;
        fs::create_dir_all(&report).map_err(text)?;
        let report = report.join(case);
        fs::create_dir(&report).map_err(text)?; // Refuse accidental artifact reuse.
        Ok(Self {
            application,
            report,
        })
    }

    fn provider(&self) -> Result<Provider, String> {
        // Discovery alone validates the same real protected C installation used
        // by PDF qualification. The argument is never executed as an application.
        let discovery =
            midnight_web_viewer_host::discover_with_diagnostics(&self.application).map_err(text)?;
        ensure(
            discovery.diagnostics.is_empty(),
            &format!(
                "trusted HTML discovery diagnostics: {:?}",
                discovery.diagnostics
            ),
        )?;
        let mut providers = discovery
            .providers
            .into_iter()
            .filter(|provider| provider.id == "midnight.viewer.html");
        let provider = providers
            .next()
            .ok_or("trusted installed HTML provider unavailable")?;
        ensure(
            providers.next().is_none(),
            "ambiguous HTML provider identity",
        )?;
        Ok(provider)
    }
}

fn environment_path(name: &str) -> Result<PathBuf, String> {
    std::env::var_os(name)
        .map(PathBuf::from)
        .ok_or_else(|| format!("missing {name}"))
}

fn ensure_d_build_path(label: &str, path: &Path, build: &Path) -> Result<(), String> {
    let d_drive = matches!(path.components().next(), Some(Component::Prefix(prefix))
        if matches!(prefix.kind(), Prefix::Disk(drive) | Prefix::VerbatimDisk(drive)
            if drive.eq_ignore_ascii_case(&b'D')));
    ensure(
        path.is_absolute()
            && d_drive
            && path.starts_with(build)
            && !path
                .components()
                .any(|part| matches!(part, Component::ParentDir)),
        &format!("{label} must be an absolute owned D workspace _build path"),
    )
}

#[derive(Default)]
struct Trace {
    requested: Mutex<Vec<String>>,
    granted: Mutex<Vec<String>>,
    denied: Mutex<Vec<String>>,
    gate_target: Mutex<Option<String>>,
    gate_entered: AtomicBool,
    gate_finished: AtomicBool,
    gate_returned_denial: AtomicBool,
    cancellation_seen: AtomicBool,
    late_captures: AtomicUsize,
    late_releases: AtomicUsize,
    scope_releases: AtomicUsize,
    read_gate: Mutex<Option<(String, Arc<ReadGate>)>>,
}

struct ObservedResources {
    inner: Option<Arc<dyn Resources>>,
    trace: Arc<Trace>,
}

impl fmt::Debug for ObservedResources {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ObservedRealOfficeResources")
    }
}

impl Resources for ObservedResources {
    fn capture(&self, segments: &[String]) -> Result<CapturedFile, String> {
        self.capture_with_cancellation(segments, &ResourceCancellation::default())
    }

    fn capture_with_cancellation(
        &self,
        segments: &[String],
        cancellation: &ResourceCancellation,
    ) -> Result<CapturedFile, String> {
        let path = format!("/{}", segments.join("/"));
        push_trace(&self.trace.requested, &path)?;
        let gated = self.trace.gate_target.lock().map_err(text)?.as_ref() == Some(&path);
        if gated {
            self.trace.gate_entered.store(true, Ordering::Release);
            let deadline = Instant::now() + STEP_TIMEOUT;
            while !cancellation.is_cancelled() {
                if Instant::now() >= deadline {
                    return Err("native test's resource cancellation gate timed out".into());
                }
                thread::sleep(Duration::from_millis(5));
            }
        }
        // This is the REAL canonical generated-resource implementation. No HTML,
        // renderer, parser or resource success is substituted by the observer.
        let cancelled_at_delegation = cancellation.is_cancelled();
        let delegated = self
            .inner
            .as_ref()
            .ok_or("generated scope already released")?
            .capture_with_cancellation(segments, cancellation);
        eprintln!(
            "Office resource scope result: path={path:?}; cancelled_at_delegation={cancelled_at_delegation}; cancelled_on_return={}; capture={:?}",
            cancellation.is_cancelled(),
            delegated
                .as_ref()
                .map(CapturedFile::length)
                .map_err(String::as_str),
        );
        let returned_denial = delegated.is_err();
        let result = match delegated {
            Ok(file) => {
                let late = cancellation.is_cancelled();
                if late {
                    self.trace.late_captures.fetch_add(1, Ordering::AcqRel);
                }
                let failure = push_trace(&self.trace.granted, &path).err().or_else(|| {
                    late.then(|| "real generated scope returned a late capture".into())
                });
                if let Some(error) = failure {
                    let cleanup = file.release_on_worker().map_err(text);
                    if late && cleanup.is_ok() {
                        self.trace.late_releases.fetch_add(1, Ordering::AcqRel);
                    }
                    Err(match cleanup {
                        Ok(()) => error,
                        Err(cleanup) => format!("{error}; {cleanup}"),
                    })
                } else {
                    let mut file = super::Retained::new(file);
                    let read_gate = self
                        .trace
                        .read_gate
                        .lock()
                        .map_err(text)?
                        .as_ref()
                        .filter(|(target, _)| target == &path)
                        .map(|(_, gate)| gate.clone());
                    match read_gate {
                        Some(gate) => read_gate::wrap(file.take(), gate),
                        None => Ok(file.take()),
                    }
                }
            }
            Err(error) => match push_trace(&self.trace.denied, &path) {
                Ok(()) => Err(error),
                Err(trace) => Err(format!("{error}; {trace}")),
            },
        };
        if gated {
            // Publish acknowledgement LAST: the real scope returned and any late capture
            // completed its checked release, while the independent parent flag stays false.
            self.trace
                .gate_returned_denial
                .store(returned_denial, Ordering::Release);
            self.trace.cancellation_seen.store(
                cancelled_at_delegation && cancellation.is_cancelled(),
                Ordering::Release,
            );
            self.trace.gate_finished.store(true, Ordering::Release);
        }
        result
    }

    fn path_policy(&self) -> ResourcePathPolicy {
        self.inner
            .as_ref()
            .expect("live observer retains its real generated scope")
            .path_policy()
    }

    fn release_on_worker(self: Arc<Self>) -> Result<(), String> {
        match Arc::try_unwrap(self) {
            Ok(scope) => {
                let mut scope = super::Retained::new(scope);
                let inner = scope
                    .get_mut()
                    .inner
                    .take()
                    .ok_or("generated scope release consumed")?;
                inner.release_on_worker()?;
                scope
                    .get()
                    .trace
                    .scope_releases
                    .fetch_add(1, Ordering::AcqRel);
                drop(scope.take());
                Ok(())
            }
            Err(shared) => {
                drop(shared);
                Ok(())
            }
        }
    }
}

fn push_trace(values: &Mutex<Vec<String>>, value: &str) -> Result<(), String> {
    let mut values = values.lock().map_err(text)?;
    ensure(values.len() < 64, "unexpected native resource fan-out")?;
    values.push(value.into());
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HostRefusal {
    header: api::Header,
    request: u64,
    base: u64,
}

/// The host echoes this actual origin only after sending its matching Denied reply.
/// Its sequence is cause metadata, never an input acknowledgement or paint fence.
struct SourceRefusal {
    armed_after: api::Header,
    after_sequence: u64,
    target: &'static str,
    pending: Option<HostRefusal>,
    observed: Option<HostRefusal>,
    read_gate: Option<Arc<ReadGate>>,
}

impl SourceRefusal {
    fn new(armed_after: api::Header, after_sequence: u64, target: &'static str) -> Self {
        Self {
            armed_after,
            after_sequence,
            target,
            pending: None,
            observed: None,
            read_gate: None,
        }
    }

    fn observe(&mut self, event: &api::Event) -> Result<bool, String> {
        match &event.kind {
            api::EventKind::ResourceResolve { request, base, url } => {
                ensure(
                    self.read_gate.is_none(),
                    "Read-refusal case received a Resolve refusal",
                )?;
                ensure(
                    self.pending.is_none() && self.observed.is_none(),
                    "duplicate host source-refusal origin",
                )?;
                ensure(
                    *request != 0
                        && *base != 0
                        && event.header.session == self.armed_after.session
                        && event.header.document == self.armed_after.document
                        && event.header.sequence > self.after_sequence
                        && url.strip_prefix('/').unwrap_or(url)
                            == self.target.strip_prefix('/').unwrap_or(self.target),
                    "host source-refusal origin did not match the armed canonical request",
                )?;
                self.pending = Some(HostRefusal {
                    header: event.header,
                    request: *request,
                    base: *base,
                });
                Ok(true)
            }
            api::EventKind::Error {
                code: api::ErrorCode::ResourceChanged,
                message,
            } => {
                ensure(
                    self.observed.is_none(),
                    "duplicate source-change transition",
                )?;
                let origin = self
                    .pending
                    .take()
                    .ok_or("source-change diagnostic lacked its actual refusal origin")?;
                ensure(
                    event.header == origin.header && message == SOURCE_CHANGED,
                    "source-change diagnostic did not match its actual refusal origin",
                )?;
                self.observed = Some(origin);
                Ok(true)
            }
            api::EventKind::ResourceRead {
                request,
                resource,
                offset,
                length,
            } => {
                let gate = self
                    .read_gate
                    .as_ref()
                    .ok_or("parent-change case expected an actual canonical Resolve refusal")?;
                ensure(
                    self.pending.is_none() && self.observed.is_none(),
                    "duplicate host Read-refusal origin",
                )?;
                let target_length = gate.length.load(Ordering::Acquire);
                ensure(
                    gate.entered.load(Ordering::Acquire)
                        && gate.delegated.load(Ordering::Acquire)
                        && gate.delegated_denial.load(Ordering::Acquire)
                        && !gate.timed_out.load(Ordering::Acquire)
                        && target_length > 0
                        && *request != 0
                        && *resource != 0
                        && *offset == 0
                        && *length == target_length.min(api::MAX_READ_BYTES as u64) as u32
                        && event.header.session == self.armed_after.session
                        && event.header.document == self.armed_after.document
                        && event.header.sequence > self.after_sequence,
                    "host Read-refusal origin did not match the real gated target Read",
                )?;
                self.pending = Some(HostRefusal {
                    header: event.header,
                    request: *request,
                    base: *resource,
                });
                Ok(true)
            }
            _ => {
                ensure(
                    self.pending.is_none(),
                    "host refusal origin and source-change diagnostic were not adjacent",
                )?;
                Ok(false)
            }
        }
    }
}

// Fixed-size test metadata only; never retains document content or input text.
#[derive(Debug, Default)]
struct PhaseObservation {
    milestone: &'static str,
    operation: &'static str,
    proof_step: &'static str,
    // Harness-local observed Find/Copy/Resize count, never a host command sequence.
    local_action_ordinal: u64,
    find_submissions: u64,
    find_after_event: u64,
    find_empty: bool,
    find_expected: Option<u32>,
    find_admitted: bool,
    find_responses: u64,
    // event sequence (if known), actual request sequence, matches, active, complete.
    last_find: Option<(Option<u64>, u64, u32, u32, bool)>,
    consumed_find: Option<(Option<u64>, u64, u32, u32, bool)>,
    copy_submissions: u64,
    last_selection: Option<(u64, usize)>,
    resize_after_event: Option<u64>,
    desired_viewport: Option<(u64, u32, u32)>,
    // Actual received event sequence, viewport revision, width and height only.
    last_frame: Option<(u64, u64, u32, u32)>,
    navigation_after_event: Option<u64>,
    navigation_grants_before: Option<usize>,
    navigation_link_ordinal: Option<usize>,
    navigation_target_granted: Option<bool>,
    navigation_helper_returned: bool,
    // Responses, completed reads, offset-zero reads, returned bytes, main length.
    html_counters: Option<(usize, usize, usize, u64, u64)>,
}

fn observed_find(
    sequence: Option<u64>,
    result: &api::FindResult,
) -> (Option<u64>, u64, u32, u32, bool) {
    (
        sequence,
        result.request_sequence,
        result.matches,
        result.active,
        result.complete,
    )
}

struct Native {
    session: Option<Session>,
    source: Option<Arc<fixture::SourceState>>,
    trace: Arc<Trace>,
    window_closed: Arc<AtomicBool>,
    report: PathBuf,
    phase_observation: Option<PhaseObservation>,
    phase: &'static str,
    sequence: u64,
    last_header: Option<api::Header>,
    source_refusal: Option<SourceRefusal>,
    diagnostic_events: usize,
    diagnostic_deadline: Option<Instant>,
    viewport: api::Viewport,
    expected_target: String,
    loaded: u64,
    loading: u64,
    frame: Option<(u64, api::Frame)>,
    finds: VecDeque<(u64, api::FindResult)>,
    selection: Option<(u64, String)>,
    expect_closed: bool,
    expected_scope_releases: usize,
}

impl Native {
    fn open_html(
        provider: Provider,
        main: CapturedFile,
        config: &Config,
        phase: &'static str,
    ) -> Result<Self, String> {
        let viewport = api::Viewport {
            revision: 1,
            width: 900,
            height: 600,
            scale: 1.0,
        };
        let session = Session::open(provider, main, None, viewport, Arc::new(|| true))?;
        let mut native = Self::attached(
            session,
            None,
            Arc::new(Trace::default()),
            Arc::new(AtomicBool::new(false)),
            config,
            phase,
            viewport,
        );
        // This generic main has no generated-resource scope. Its real native
        // main-source release is checked by the dedicated delegating observer.
        native.expected_scope_releases = 0;
        Ok(native)
    }

    fn open(
        provider: Provider,
        bytes: &'static [u8],
        config: &Config,
        phase: &'static str,
    ) -> Result<Self, String> {
        let (parent, source) = fixture::captured(bytes);
        Self::open_captured(provider, parent, Some(source), config, phase)
    }

    fn open_file(
        provider: Provider,
        bytes: &'static [u8],
        config: &Config,
        phase: &'static str,
    ) -> Result<Self, String> {
        let path = config
            .report
            .join(format!("{phase}-authored-two-sheet.xlsx"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(text)?;
        file.write_all(bytes).map_err(text)?;
        file.sync_all().map_err(text)?;
        drop(file);
        // MAIN vertical proof uses actual authored XLSX on D and the existing
        // native CapturedFile opener, never an in-memory substitute for I/O.
        let parent = CapturedFile::capture(&path).map_err(text)?;
        Self::open_captured(provider, parent, None, config, phase)
    }

    fn open_captured(
        provider: Provider,
        parent: CapturedFile,
        source: Option<Arc<fixture::SourceState>>,
        config: &Config,
        phase: &'static str,
    ) -> Result<Self, String> {
        let window_closed = Arc::new(AtomicBool::new(false));
        let mut prepared = super::Retained::new(super::prepare(
            parent,
            OfficeFormat::Xlsx,
            Cancellation::observing(window_closed.clone()),
        )?);
        let admission = (|| {
            ensure(
                source
                    .as_ref()
                    .is_none_or(|source| source.reads.load(Ordering::Acquire) > 0),
                "real Office parser did not read captured XLSX bytes",
            )?;
            ensure(
                prepared.get().main.resources().is_none(),
                "main inherited an original directory scope",
            )?;
            ensure(
                prepared.get().resources.path_policy() == ResourcePathPolicy::CanonicalRoot,
                "real generated scope did not select CanonicalRoot",
            )
        })();
        if let Err(error) = admission {
            let prepared = prepared.take();
            let cleanup = combine(
                prepared.main.release_on_worker().map_err(text),
                prepared.resources.release_on_worker(),
            );
            return Err(match cleanup {
                Ok(()) => error,
                Err(cleanup) => format!("{error}; {cleanup}"),
            });
        }
        let prepared = prepared.take();
        let trace = Arc::new(Trace::default());
        let resources = Arc::new(ObservedResources {
            inner: Some(prepared.resources),
            trace: trace.clone(),
        });
        let viewport = api::Viewport {
            revision: 1,
            width: 900,
            height: 600,
            scale: 1.0,
        };
        let session = Session::open_with_policy_ceiling(
            provider,
            prepared.main,
            Some(resources),
            viewport,
            Arc::new(|| true),
            PolicyCeiling::OfflineOnly,
        )?;
        Ok(Self::attached(
            session,
            source,
            trace,
            window_closed,
            config,
            phase,
            viewport,
        ))
    }

    fn attached(
        session: Session,
        source: Option<Arc<fixture::SourceState>>,
        trace: Arc<Trace>,
        window_closed: Arc<AtomicBool>,
        config: &Config,
        phase: &'static str,
        viewport: api::Viewport,
    ) -> Self {
        Self {
            session: Some(session),
            source,
            trace,
            window_closed,
            report: config.report.clone(),
            phase_observation: None,
            phase,
            sequence: 0,
            last_header: None,
            source_refusal: None,
            diagnostic_events: 0,
            diagnostic_deadline: None,
            viewport,
            expected_target: ALPHA_FIRST.into(),
            loaded: 0,
            loading: 0,
            frame: None,
            finds: VecDeque::new(),
            selection: None,
            expect_closed: false,
            expected_scope_releases: 1,
        }
    }

    fn observation_milestone(&mut self, milestone: &'static str) {
        if let Some(observation) = self.phase_observation.as_mut() {
            observation.milestone = milestone;
            observation.operation = "between-helpers";
        }
    }

    fn observe_phase(&mut self, apply: impl FnOnce(&mut PhaseObservation)) {
        if let Some(observation) = self.phase_observation.as_mut() {
            apply(observation);
        }
    }

    fn observe_find_submission(&mut self, empty: bool, expected: Option<u32>) {
        let after = self.sequence;
        self.observe_phase(|observation| {
            observation.local_action_ordinal = observation.local_action_ordinal.saturating_add(1);
            observation.find_submissions = observation.find_submissions.saturating_add(1);
            observation.find_after_event = after;
            observation.find_empty = empty;
            observation.find_expected = expected;
            observation.find_admitted = false;
        });
    }

    fn observe_phase_event(&mut self, event: &api::Event) {
        self.observe_phase(|observation| match &event.kind {
            api::EventKind::FindResult(result) => {
                observation.find_responses = observation.find_responses.saturating_add(1);
                observation.last_find = Some(observed_find(Some(event.header.sequence), result));
            }
            api::EventKind::Selection(selection) => {
                observation.last_selection = Some((event.header.sequence, selection.len()));
            }
            api::EventKind::Frame(frame) => {
                observation.last_frame = Some((
                    event.header.sequence,
                    frame.viewport_revision,
                    frame.width,
                    frame.height,
                ));
            }
            _ => {}
        });
    }

    fn print_failure_observation(&self) {
        let Some(observation) = &self.phase_observation else {
            return;
        };
        // Failure-only, nonblocking counts. No paths are copied or emitted; None
        // means a trace lock was unavailable, never an acceptance/cleanup error.
        let scope_counts = [
            self.trace
                .requested
                .try_lock()
                .ok()
                .map(|paths| paths.len()),
            self.trace.granted.try_lock().ok().map(|paths| paths.len()),
            self.trace.denied.try_lock().ok().map(|paths| paths.len()),
        ];
        let header = self
            .last_header
            .map(|header| (header.session, header.document, header.sequence));
        let frame = self.frame.as_ref().map(|(sequence, frame)| {
            (
                *sequence,
                frame.viewport_revision,
                frame.width,
                frame.height,
            )
        });
        // Ignored diagnostic I/O cannot replace the original scenario failure.
        let _ = writeln!(
            std::io::stderr().lock(),
            "Office native failure observation: {observation:?}; header={header:?}; latest_event={}; loading={}; loaded={}; viewport={:?}; retained_frame={frame:?}; scope_counts_requested_granted_denied={scope_counts:?}",
            self.sequence,
            self.loading,
            self.loaded,
            (
                self.viewport.revision,
                self.viewport.width,
                self.viewport.height
            ),
        );
    }

    fn session(&self) -> Result<&Session, String> {
        self.session
            .as_ref()
            .ok_or_else(|| "native session already consumed".into())
    }

    fn send(&self, command: api::CommandKind) -> Result<(), String> {
        ensure(
            self.session()?.send(command),
            "native command was not admitted",
        )
    }

    fn poll(&mut self) -> Result<(), String> {
        let events = self.session()?.drain(64);
        self.observe_live_batch(events, |native| Ok(native.session()?.finished()))
    }

    fn observe_live_batch(
        &mut self,
        events: Vec<api::Event>,
        finished: impl FnOnce(&Self) -> Result<bool, String>,
    ) -> Result<(), String> {
        self.observe_events(events)?;
        if !self.expect_closed {
            ensure(
                !finished(self)?,
                "native browser exited before requested evidence",
            )?;
        }
        Ok(())
    }

    fn observe_events(&mut self, events: Vec<api::Event>) -> Result<(), String> {
        for event in events {
            self.sequence = self.sequence.max(event.header.sequence);
            self.last_header = Some(event.header);
            self.observe_phase_event(&event);
            self.trace_find_event(&event);
            if let Some(expectation) = self.source_refusal.as_mut()
                && expectation.observe(&event)?
            {
                continue;
            }
            if action_refusal(&event.kind) {
                if let api::EventKind::Error { message, .. } = &event.kind {
                    eprintln!("Office recoverable navigation notice: {message}");
                }
                continue;
            }
            match event.kind {
                api::EventKind::Error { code, message } => {
                    return Err(format!("native browser {code:?}: {message}"));
                }
                api::EventKind::Closed if self.expect_closed => {}
                api::EventKind::Closed => {
                    return Err("native browser closed before requested evidence".into());
                }
                api::EventKind::Frame(frame) => self.frame = Some((event.header.sequence, frame)),
                api::EventKind::State(state) if state.loading => {
                    self.loading = event.header.sequence
                }
                api::EventKind::State(_) => self.loaded = event.header.sequence,
                api::EventKind::FindResult(result) => {
                    ensure(
                        self.finds.len() < 8,
                        "unexpected native find-result fan-out",
                    )?;
                    self.finds.push_back((event.header.sequence, result));
                }
                api::EventKind::Selection(selection) => {
                    self.selection = Some((event.header.sequence, selection));
                }
                api::EventKind::HostShortcut(shortcut) => {
                    return Err(format!("test key escaped browser focus: {shortcut:?}"));
                }
                api::EventKind::ResourceResolve { .. } | api::EventKind::ResourceRead { .. } => {
                    return Err("unarmed host source-refusal origin".into());
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn trace_find_event(&mut self, event: &api::Event) {
        if self.source_refusal.is_none() || self.diagnostic_events >= 64 {
            return;
        }
        match &event.kind {
            api::EventKind::FindResult(result) => eprintln!(
                "Office Find metadata: session={} document={} event={} request={} matches={} active={} complete={} last_loading={} last_loaded={}",
                event.header.session,
                event.header.document,
                event.header.sequence,
                result.request_sequence,
                result.matches,
                result.active,
                result.complete,
                self.loading,
                self.loaded,
            ),
            api::EventKind::State(state) => eprintln!(
                "Office load metadata: session={} document={} event={} loading={}",
                event.header.session, event.header.document, event.header.sequence, state.loading,
            ),
            _ => return,
        }
        self.diagnostic_events += 1;
    }

    fn diagnose_failed_content_proof(&mut self, token: &str) -> Result<(), String> {
        // Preserve the actual frame at the original failure before any diagnostic input.
        if let Some((_, frame)) = &self.frame {
            fixture::save_bmp(
                &self
                    .report
                    .join(format!("{}-original-content-failure.bmp", self.phase)),
                frame,
            )
            .map_err(text)?;
        }
        self.diagnostic_deadline = Some(Instant::now() + STEP_TIMEOUT);
        let diagnostic = (|| {
            let reads_before = self
                .source
                .as_ref()
                .map(|source| source.reads.load(Ordering::Acquire));
            let copied = self.copy_all(token);
            let copy_sequence = copied.as_ref().ok().map(|(sequence, _)| *sequence);
            let copy_bytes = copied.as_ref().ok().map(|(_, selection)| selection.len());
            if let Ok((_, selection)) = &copied {
                fs::write(
                    self.report
                        .join(format!("{}-diagnostic-copy.selection.txt", self.phase)),
                    selection,
                )
                .map_err(text)?;
            }
            let after = self.sequence;
            let resized = self.resize_viewport([
                self.viewport
                    .width
                    .checked_add(1)
                    .ok_or("diagnostic width exhausted")?,
                self.viewport
                    .height
                    .checked_add(1)
                    .ok_or("diagnostic height exhausted")?,
            ]);
            let frame_metadata = self.frame.as_ref().map(|(sequence, frame)| {
                (
                    *sequence,
                    frame.viewport_revision,
                    frame.width,
                    frame.height,
                )
            });
            let fresh = resized.is_ok()
                && self.frame.as_ref().is_some_and(|(sequence, frame)| {
                    correlated_frame(*sequence, frame, &self.viewport, after)
                });
            let reads_after = self
                .source
                .as_ref()
                .map(|source| source.reads.load(Ordering::Acquire));
            if let Some((_, frame)) = &self.frame {
                fixture::save_bmp(
                    &self
                        .report
                        .join(format!("{}-diagnostic-post-find-resize.bmp", self.phase)),
                    frame,
                )
                .map_err(text)?;
            }
            let mut report = File::create(
                self.report
                    .join(format!("{}-post-find-diagnostic.txt", self.phase)),
            )
            .map_err(text)?;
            writeln!(
                report,
                "original_case_remains_failed=true\ncopy_ok={}\ncopy_sequence={copy_sequence:?}\ncopy_bytes={copy_bytes:?}\nresize_ok={}\nfresh_contrasted_frame={fresh}\nframe_metadata={frame_metadata:?}\nlast_loading={}\nlast_loaded={}\nsession_finished={}\noriginal_source_reads_before={reads_before:?}\noriginal_source_reads_after={reads_after:?}",
                copied.is_ok(), resized.is_ok(), self.loading, self.loaded,
                self.session()?.finished(),
            ).map_err(text)?;
            eprintln!(
                "Office post-Find metadata: copy_ok={} copy_sequence={copy_sequence:?} copy_bytes={copy_bytes:?} resize_ok={} fresh_contrasted_frame={fresh} frame={frame_metadata:?} last_loading={} last_loaded={} session_finished={}",
                copied.is_ok(),
                resized.is_ok(),
                self.loading,
                self.loaded,
                self.session()?.finished(),
            );
            ensure(
                reads_before == reads_after,
                "diagnostic local interaction reread the source",
            )?;
            combine(copied.map(|_| ()), resized.map(|_| ()))
        })();
        self.diagnostic_deadline = None;
        diagnostic
    }

    fn arm_source_refusal(&mut self, target: &'static str) -> Result<(), String> {
        self.poll()?;
        ensure(
            self.source_refusal.is_none(),
            "source refusal already armed",
        )?;
        let after = self
            .last_header
            .ok_or("missing real native origin identity")?;
        self.source_refusal = Some(SourceRefusal::new(after, self.sequence, target));
        Ok(())
    }

    fn arm_read_refusal(
        &mut self,
        target: &'static str,
        gate: Arc<ReadGate>,
    ) -> Result<(), String> {
        self.arm_source_refusal(target)?;
        self.source_refusal
            .as_mut()
            .ok_or("Read refusal was not armed")?
            .read_gate = Some(gate);
        Ok(())
    }

    fn wait_source_refusal(&mut self) -> Result<(), String> {
        self.wait_for(|native| {
            Ok(native
                .source_refusal
                .as_ref()
                .is_some_and(|expected| expected.observed.is_some()))
        })?;
        ensure(
            !self.session()?.finished(),
            "source refusal ended the committed interactive document",
        )?;
        let expected = self
            .source_refusal
            .as_ref()
            .ok_or("source refusal was not armed")?;
        let refusal = expected.observed.ok_or("host refusal was not observed")?;
        let mut receipt = File::create(
            self.report
                .join(format!("{}-source-refusal.tsv", self.phase)),
        )
        .map_err(text)?;
        let origin = if expected.read_gate.is_some() {
            "sent-denial-read-origin"
        } else {
            "sent-denial-origin"
        };
        writeln!(
            receipt,
            "{origin}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            refusal.header.session,
            refusal.header.document,
            refusal.header.sequence,
            refusal.request,
            refusal.base,
            expected.target,
            SOURCE_CHANGED,
        )
        .map_err(text)
    }

    fn wait_for(
        &mut self,
        mut condition: impl FnMut(&mut Self) -> Result<bool, String>,
    ) -> Result<(), String> {
        let deadline = self
            .diagnostic_deadline
            .unwrap_or_else(|| Instant::now() + STEP_TIMEOUT);
        loop {
            self.poll()?;
            if condition(self)? {
                return Ok(());
            }
            ensure(Instant::now() < deadline, "native Office stage timed out")?;
            ensure(
                !self.session()?.finished(),
                "native browser exited before requested evidence",
            )?;
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn wait_loaded(&mut self) -> Result<(), String> {
        self.observe_phase(|observation| observation.operation = "wait-loaded");
        self.wait_for(|native| {
            Ok(native.loaded > 0
                && native
                    .frame
                    .as_ref()
                    .is_some_and(|(_, frame)| nonblank(frame)))
        })
    }

    /// Navigation has no public commit/URL ACK. Repeated real Find observations
    /// wait for the distinct new content; Closed/Runtime still fail in poll().
    fn wait_content(&mut self, query: &str) -> Result<(), String> {
        self.observe_phase(|observation| observation.operation = "wait-content");
        self.poll()?;
        self.finds.clear();
        let deadline = Instant::now() + STEP_TIMEOUT;
        let mut outstanding = false;
        let mut next = Instant::now();
        loop {
            self.poll()?; // Processes the entire batch: a later Closed still fails.
            while let Some((_, result)) = self.finds.pop_front() {
                self.observe_phase(|observation| {
                    observation.consumed_find = Some(observed_find(None, &result));
                });
                if result.complete {
                    match result.matches {
                        1 => return Ok(()),
                        0 => {
                            outstanding = false;
                            next = Instant::now() + Duration::from_millis(25);
                        }
                        _ => return Err("ambiguous native transition content".into()),
                    }
                }
            }
            ensure(
                Instant::now() < deadline,
                "new native navigation content never appeared",
            )?;
            ensure(
                !self.session()?.finished(),
                "native browser stopped before transition content",
            )?;
            if !outstanding && Instant::now() >= next {
                self.observe_find_submission(query.is_empty(), Some(1));
                self.send(api::CommandKind::Find(api::Find {
                    query: query.into(),
                    forward: true,
                    match_case: true,
                    find_next: false,
                }))?;
                self.observe_phase(|observation| observation.find_admitted = true);
                outstanding = true;
            }
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn find(&mut self, query: &str, expected: u32) -> Result<u64, String> {
        self.observe_phase(|observation| observation.operation = "find");
        self.poll()?;
        self.finds.clear();
        if self.source_refusal.is_some() && self.diagnostic_events < 64 {
            self.diagnostic_events += 1;
            eprintln!(
                "Office Find submission: query_empty={} expected={expected} after_event={} last_loading={} last_loaded={}",
                query.is_empty(),
                self.sequence,
                self.loading,
                self.loaded,
            );
        }
        self.observe_find_submission(query.is_empty(), Some(expected));
        self.send(api::CommandKind::Find(api::Find {
            query: query.into(),
            forward: true,
            match_case: true,
            find_next: false,
        }))?;
        self.observe_phase(|observation| observation.find_admitted = true);
        let mut observed = None;
        self.wait_for(|native| {
            while let Some((sequence, result)) = native.finds.pop_front() {
                native.observe_phase(|observation| {
                    observation.consumed_find = Some(observed_find(Some(sequence), &result));
                });
                if result.complete {
                    ensure(
                        result.matches == expected,
                        &format!(
                            "real CEF Find({query:?}) expected {expected}, got {}",
                            result.matches
                        ),
                    )?;
                    observed = Some(sequence);
                    return Ok(true);
                }
            }
            Ok(false)
        })?;
        observed.ok_or_else(|| "native Find completed without an observation sequence".into())
    }

    fn key(&self, key: &str, code: &str, modifiers: api::Modifiers) -> Result<(), String> {
        for pressed in [true, false] {
            self.send(api::CommandKind::Key(api::Key {
                key: key.into(),
                code: code.into(),
                pressed,
                repeat: false,
                modifiers,
            }))?;
        }
        Ok(())
    }

    fn activate_link(&mut self, ordinal: usize) -> Result<(), String> {
        ensure((1..=8).contains(&ordinal), "invalid fixture link ordinal")?;
        self.observe_phase(|observation| observation.operation = "link-clear-find");
        self.find("", 0)?; // Stop prior Find before ordinary DOM navigation.
        self.observe_phase(|observation| observation.operation = "link-input");
        self.send(api::CommandKind::Focus(true))?;
        self.key(
            "Home",
            "Home",
            api::Modifiers {
                control: true,
                ..Default::default()
            },
        )?;
        for kind in [api::PointerKind::Down, api::PointerKind::Up] {
            self.send(api::CommandKind::Pointer(api::Pointer {
                kind,
                button: api::PointerButton::Left,
                x: 2.0,
                y: 2.0,
                modifiers: Default::default(),
                click_count: 1,
            }))?;
        }
        for _ in 0..ordinal {
            self.key("Tab", "Tab", Default::default())?;
        }
        self.send(api::CommandKind::Key(api::Key {
            key: "Enter".into(),
            code: "Enter".into(),
            pressed: true,
            repeat: false,
            modifiers: Default::default(),
        }))?;
        self.send(api::CommandKind::Text("\r".into()))?;
        self.send(api::CommandKind::Key(api::Key {
            key: "Enter".into(),
            code: "Enter".into(),
            pressed: false,
            repeat: false,
            modifiers: Default::default(),
        }))
    }

    fn navigate(&mut self, target: &str, ordinal: usize) -> Result<(), String> {
        self.poll()?;
        let sequence = self.sequence;
        let grants = self.trace.granted.lock().map_err(text)?.len();
        self.observe_phase(|observation| {
            observation.navigation_after_event = Some(sequence);
            observation.navigation_grants_before = Some(grants);
            observation.navigation_link_ordinal = Some(ordinal);
            observation.navigation_target_granted = None;
            observation.navigation_helper_returned = false;
        });
        self.activate_link(ordinal)?;
        self.observe_phase(|observation| observation.operation = "navigate-resource-load-frame");
        self.wait_for(|native| {
            let requested = native
                .trace
                .granted
                .lock()
                .map_err(text)?
                .get(grants..)
                .is_some_and(|paths| paths.iter().any(|path| path == target));
            native.observe_phase(|observation| {
                observation.navigation_target_granted = Some(requested);
            });
            Ok(requested
                && native.loaded > sequence
                && native.loaded > native.loading
                && native
                    .frame
                    .as_ref()
                    .is_some_and(|(painted, frame)| *painted > sequence && nonblank(frame)))
        })?;
        self.observe_phase(|observation| observation.navigation_helper_returned = true);
        self.expected_target = target.into();
        Ok(())
    }

    fn copy_all(&mut self, expected: &str) -> Result<(u64, String), String> {
        self.observe_phase(|observation| observation.operation = "copy");
        self.poll()?;
        self.selection = None;
        self.send(api::CommandKind::Focus(true))?;
        for kind in [api::PointerKind::Down, api::PointerKind::Up] {
            self.send(api::CommandKind::Pointer(api::Pointer {
                kind,
                button: api::PointerButton::Left,
                x: 2.0,
                y: 2.0,
                modifiers: Default::default(),
                click_count: 1,
            }))?;
        }
        self.key(
            "a",
            "KeyA",
            api::Modifiers {
                control: true,
                ..Default::default()
            },
        )?;
        let deadline = self.diagnostic_deadline.map_or_else(
            || Instant::now() + Duration::from_secs(5),
            |diagnostic| diagnostic.min(Instant::now() + Duration::from_secs(5)),
        );
        let mut request = Instant::now();
        loop {
            if Instant::now() >= request {
                self.observe_phase(|observation| {
                    observation.local_action_ordinal =
                        observation.local_action_ordinal.saturating_add(1);
                    observation.copy_submissions = observation.copy_submissions.saturating_add(1);
                });
                self.send(api::CommandKind::Copy)?;
                request = Instant::now() + Duration::from_millis(100);
            }
            self.poll()?;
            if let Some((sequence, selection)) = self.selection.take()
                && selection.contains(expected)
            {
                return Ok((sequence, selection));
            }
            ensure(
                Instant::now() < deadline,
                "native Ctrl+A/Copy did not return expected rendered cell",
            )?;
            thread::sleep(Duration::from_millis(5));
        }
    }

    fn prove(&mut self, stage: &str, token: &str) -> Result<(), String> {
        self.observe_phase(|observation| observation.proof_step = "find");
        let found = self.find(token, 1)?;
        self.observe_phase(|observation| observation.proof_step = "copy");
        let (selected, selection) = self.copy_all(token)?;
        self.observe_phase(|observation| observation.proof_step = "save-proof");
        self.save_proof(stage, token, found.max(selected), &selection)
    }

    fn resize_viewport(&mut self, size: [u32; 2]) -> Result<u64, String> {
        self.observe_phase(|observation| observation.operation = "resize");
        self.poll()?;
        let after = self.sequence;
        let viewport = api::Viewport {
            revision: self
                .viewport
                .revision
                .checked_add(1)
                .ok_or("native proof viewport revision exhausted")?,
            width: size[0],
            height: size[1],
            scale: self.viewport.scale,
        };
        // Reject retained/queued old-size paints. A real dimension change forces a new
        // CEF surface; acknowledged_input is zero and cannot establish pixel causality.
        ensure(
            size != [self.viewport.width, self.viewport.height],
            "proof resize must change dimensions",
        )?;
        self.frame = None;
        self.observe_phase(|observation| {
            observation.local_action_ordinal = observation.local_action_ordinal.saturating_add(1);
            observation.resize_after_event = Some(after);
            observation.desired_viewport =
                Some((viewport.revision, viewport.width, viewport.height));
        });
        self.send(api::CommandKind::Resize(viewport))?;
        self.viewport = viewport;
        self.wait_for(|native| {
            Ok(native.frame.as_ref().is_some_and(|(sequence, frame)| {
                correlated_frame(*sequence, frame, &viewport, after)
            }))
        })?;
        Ok(viewport.revision)
    }

    fn save_proof(
        &mut self,
        stage: &str,
        token: &str,
        content_sequence: u64,
        selection: &str,
    ) -> Result<(), String> {
        ensure(
            selection.contains(token),
            "native selection omitted expected cell",
        )?;
        ensure(
            content_sequence != 0 && content_sequence <= self.sequence,
            "missing actual native content observation",
        )?;
        self.resize_viewport([
            self.viewport
                .width
                .checked_add(1)
                .ok_or("proof width exhausted")?,
            self.viewport
                .height
                .checked_add(1)
                .ok_or("proof height exhausted")?,
        ])?;
        let (frame_sequence, frame) = self.frame.as_ref().ok_or("no native frame available")?;
        ensure(
            correlated_frame(*frame_sequence, frame, &self.viewport, content_sequence),
            "native browser proof was not repainted after observed content",
        )?;
        let stem = format!("{}-{stage}", self.phase);
        fixture::save_bmp(&self.report.join(format!("{stem}.bmp")), frame).map_err(text)?;
        fs::write(self.report.join(format!("{stem}.selection.txt")), selection).map_err(text)?;
        let mut proof = OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.report.join("native-proof.tsv"))
            .map_err(text)?;
        writeln!(
            proof,
            "{stem}\t{}\t{token}\t{content_sequence}\t{frame_sequence}\t{}\t{}\t{}\t{}",
            self.expected_target,
            frame.viewport_revision,
            frame.width,
            frame.height,
            dark_pixels(frame)
        )
        .map_err(text)?;
        eprintln!(
            "Office native proof: {stem}; token={token}; dark_pixels={}",
            dark_pixels(frame)
        );
        self.observe_phase(|observation| observation.proof_step = "saved");
        Ok(())
    }

    fn assert_fixed_policy(&mut self) -> Result<(), String> {
        ensure(
            !self.session()?.policy_change_allowed(),
            "Office allowed policy controls",
        )?;
        for policy in [
            api::Policy::default(),
            api::Policy {
                network: true,
                scripts: false,
            },
            api::Policy {
                network: true,
                scripts: true,
            },
            api::Policy {
                network: false,
                scripts: true,
            },
        ] {
            ensure(
                !self.session()?.send(api::CommandKind::Policy(policy)),
                "Office admitted a policy override",
            )?;
        }
        ensure(
            !self.session()?.finished(),
            "rejected policy unexpectedly ended native document",
        )
    }

    fn assert_expected_resources(&self) -> Result<(), String> {
        let allowed = [
            CATALOG,
            ALPHA_FIRST,
            BETA_FIRST,
            ALPHA_ROWS,
            ALPHA_COLUMNS,
            ALPHA_FAR,
            BETA_ROWS,
            BETA_COLUMNS,
        ];
        let requested = self.trace.requested.lock().map_err(text)?;
        let granted = self.trace.granted.lock().map_err(text)?.clone();
        let denied_snapshot = self.trace.denied.lock().map_err(text)?.clone();
        let unexpected: Vec<_> = requested
            .iter()
            .filter(|path| !allowed.contains(&path.as_str()))
            .collect();
        // Observer success precedes Host post-validation/publication. Keep the
        // original requested guard and both qualification conditions unchanged.
        eprintln!(
            "Office resource snapshot: requested={requested:?}; scope_granted={granted:?}; scope_denied_snapshot={denied_snapshot:?}; unexpected={unexpected:?}"
        );
        ensure(
            requested
                .iter()
                .all(|path| allowed.contains(&path.as_str())),
            &format!(
                "CEF requested a foreign/native/original-directory resource; unexpected={unexpected:?}; requested={requested:?}; scope_granted={granted:?}; scope_denied_snapshot={denied_snapshot:?}"
            ),
        )?;
        let denied = self.trace.denied.lock().map_err(text)?;
        ensure(
            denied.is_empty(),
            &format!(
                "normal native navigation was denied; scope_denied={denied:?}; requested={requested:?}; scope_granted={granted:?}"
            ),
        )
    }

    fn finish_case(&mut self, scenario: Result<(), String>) -> Result<(), String> {
        if let Err(error) = &scenario {
            self.print_failure_observation();
            let _ = fs::write(
                self.report.join(format!("{}-failure.txt", self.phase)),
                error,
            );
            if let Some((_, frame)) = &self.frame {
                let _ = fixture::save_bmp(
                    &self.report.join(format!("{}-failure.bmp", self.phase)),
                    frame,
                );
            }
        }
        self.window_closed.store(true, Ordering::Release);
        let began = Instant::now();
        let cleanup = self
            .session
            .take()
            .ok_or("native session already released")?
            .release_on_worker();
        let ownership = ensure(
            self.source.as_ref().is_none_or(|source| {
                source.releases.load(Ordering::Acquire) == 1
                    && source.drops.load(Ordering::Acquire) == 1
            }) && self.trace.scope_releases.load(Ordering::Acquire) == self.expected_scope_releases,
            "checked shutdown left an original/generated resource owner or skipped explicit release",
        );
        let mut receipt =
            File::create(self.report.join(format!("{}-cleanup.txt", self.phase))).map_err(text)?;
        let counters = self.source.as_ref().map(|source| {
            (
                source.releases.load(Ordering::Acquire),
                source.drops.load(Ordering::Acquire),
                source.reads.load(Ordering::Acquire),
            )
        });
        writeln!(receipt, "session_cleanup={cleanup:?}\ncontrolled_source_counters={counters:?}\nscope_releases={}\nclose_ms={}",
            self.trace.scope_releases.load(Ordering::Acquire),
            began.elapsed().as_millis()).map_err(text)?;
        // A Session receipt does not substitute for Build's independent observation
        // that this harness's owned CEF child/process tree actually terminated.
        combine(scenario, combine(cleanup, ownership))
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            self.window_closed.store(true, Ordering::Release);
            if let Err(error) = session.release_on_worker() {
                eprintln!("native Office fallback cleanup unconfirmed: {error}");
            }
        }
    }
}

fn dark_pixels(frame: &api::Frame) -> usize {
    frame
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .filter(|pixel| pixel[3] > 200 && pixel[0] < 170 && pixel[1] < 170 && pixel[2] < 170)
        .count()
}

fn nonblank(frame: &api::Frame) -> bool {
    frame.width > 0
        && frame.height > 0
        && frame.rgba.len() as u64 == u64::from(frame.width) * u64::from(frame.height) * 4
        && dark_pixels(frame) > 150
        && frame
            .rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|pixel| pixel[3] > 200 && pixel[0] > 235 && pixel[1] > 235 && pixel[2] > 235)
            .count()
            > 150
}

fn correlated_frame(
    sequence: u64,
    frame: &api::Frame,
    viewport: &api::Viewport,
    observed_after: u64,
) -> bool {
    sequence > observed_after
        && frame.viewport_revision == viewport.revision
        && frame.width == viewport.width
        && frame.height == viewport.height
        && nonblank(frame)
}

#[test]
fn office_native_source_refusal_rejects_unmatched_origin_and_status_pairs() {
    let armed = api::Header {
        session: 1,
        document: 2,
        sequence: 10,
    };
    let header = api::Header {
        sequence: 11,
        ..armed
    };
    let origin = |header, request, base, url: &str| api::Event {
        header,
        kind: api::EventKind::ResourceResolve {
            request,
            base,
            url: url.into(),
        },
    };
    let diagnostic = |header, message: &str| api::Event {
        header,
        kind: api::EventKind::Error {
            code: api::ErrorCode::ResourceChanged,
            message: message.into(),
        },
    };
    for url in [ALPHA_ROWS, ALPHA_ROWS.strip_prefix('/').unwrap()] {
        let mut expected = SourceRefusal::new(armed, 10, ALPHA_ROWS);
        assert!(expected.observe(&origin(header, 42, 1, url)).unwrap());
        assert!(
            expected.observed.is_none(),
            "an origin alone is not a refusal"
        );
        assert!(
            expected
                .observe(&diagnostic(header, SOURCE_CHANGED))
                .unwrap()
        );
        assert_eq!(
            expected.observed,
            Some(HostRefusal {
                header,
                request: 42,
                base: 1
            })
        );
        assert!(
            expected
                .observe(&diagnostic(header, SOURCE_CHANGED))
                .is_err()
        );
        assert!(expected.observe(&origin(header, 42, 1, url)).is_err());
    }
    for event in [
        origin(header, 0, 1, ALPHA_ROWS),
        origin(header, 42, 0, ALPHA_ROWS),
        origin(
            api::Header {
                session: 3,
                ..header
            },
            42,
            1,
            ALPHA_ROWS,
        ),
        origin(
            api::Header {
                document: 3,
                ..header
            },
            42,
            1,
            ALPHA_ROWS,
        ),
        origin(armed, 42, 1, ALPHA_ROWS),
        origin(header, 42, 1, ALPHA_COLUMNS),
        origin(header, 42, 1, "//office/v1/sheet/0/row/129/col/1.html"),
        origin(header, 42, 1, "/office/v1/sheet/0/row/129/col/1.html?x=1"),
    ] {
        let mut expected = SourceRefusal::new(armed, 10, ALPHA_ROWS);
        assert!(expected.observe(&event).is_err());
        assert!(expected.observed.is_none());
    }
    let mut expected = SourceRefusal::new(armed, 10, ALPHA_ROWS);
    assert!(
        expected
            .observe(&diagnostic(header, SOURCE_CHANGED))
            .is_err()
    );
    for event in [
        diagnostic(
            api::Header {
                sequence: 12,
                ..header
            },
            SOURCE_CHANGED,
        ),
        diagnostic(
            api::Header {
                document: 3,
                ..header
            },
            SOURCE_CHANGED,
        ),
        diagnostic(
            api::Header {
                session: 3,
                ..header
            },
            SOURCE_CHANGED,
        ),
        diagnostic(header, "unrecognized source-change payload"),
        api::Event {
            header,
            kind: api::EventKind::Ready,
        },
    ] {
        let mut expected = SourceRefusal::new(armed, 10, ALPHA_ROWS);
        expected
            .observe(&origin(header, 42, 1, ALPHA_ROWS))
            .unwrap();
        assert!(expected.observe(&event).is_err());
        assert!(expected.observed.is_none());
    }
    let mut expected = SourceRefusal::new(armed, 10, ALPHA_ROWS);
    assert!(
        !expected
            .observe(&api::Event {
                header,
                kind: api::EventKind::Error {
                    code: api::ErrorCode::Runtime,
                    message: "fatal errors remain for Native::poll".into(),
                },
            })
            .unwrap()
    );
}

#[test]
fn office_native_action_refusal_accepts_only_bounded_denied_not_terminal_errors() {
    assert!(action_refusal(&api::EventKind::Error {
        code: api::ErrorCode::Denied,
        message: "Navigation could not be prepared.".into(),
    }));
    for (code, message) in [
        (api::ErrorCode::Runtime, "runtime".into()),
        (api::ErrorCode::ResourceChanged, SOURCE_CHANGED.into()),
        (api::ErrorCode::Denied, String::new()),
        (api::ErrorCode::Denied, "x".repeat(513)),
        (api::ErrorCode::Denied, "not\na bounded action".into()),
    ] {
        assert!(!action_refusal(&api::EventKind::Error { code, message }));
    }
    assert!(!action_refusal(&api::EventKind::Closed));
}

#[test]
fn office_native_closed_after_success_events_fails_the_live_batch_consumer() {
    let viewport = api::Viewport {
        revision: 3,
        width: 20,
        height: 20,
        scale: 1.0,
    };
    let mut native = Native {
        session: None,
        source: None,
        trace: Arc::new(Trace::default()),
        window_closed: Arc::new(AtomicBool::new(false)),
        report: PathBuf::new(),
        phase_observation: None,
        phase: "unit",
        sequence: 0,
        last_header: None,
        source_refusal: None,
        diagnostic_events: 0,
        diagnostic_deadline: None,
        viewport,
        expected_target: ALPHA_FIRST.into(),
        loaded: 0,
        loading: 0,
        frame: None,
        finds: VecDeque::new(),
        selection: None,
        expect_closed: false,
        expected_scope_releases: 1,
    };
    let event = |sequence, kind| api::Event {
        header: api::Header {
            session: 1,
            document: 1,
            sequence,
        },
        kind,
    };
    let mut rgba = vec![255; 20 * 20 * 4];
    for pixel in rgba.as_chunks_mut::<4>().0.iter_mut().take(200) {
        *pixel = [0, 0, 0, 255];
    }
    let result = native.observe_events(vec![
        event(
            1,
            api::EventKind::FindResult(api::FindResult {
                request_sequence: 7,
                matches: 1,
                active: 1,
                complete: true,
            }),
        ),
        event(2, api::EventKind::Selection(ALPHA.into())),
        event(
            3,
            api::EventKind::Frame(api::Frame {
                viewport_revision: 3,
                acknowledged_input: 0,
                width: 20,
                height: 20,
                rgba,
            }),
        ),
        event(4, api::EventKind::Closed),
    ]);
    assert_eq!(
        result,
        Err("native browser closed before requested evidence".into())
    );
    assert!(
        native
            .finds
            .iter()
            .any(|(_, result)| result.complete && result.matches == 1)
    );
    assert!(
        native
            .selection
            .as_ref()
            .is_some_and(|(_, selection)| selection == ALPHA)
    );
    assert!(
        native
            .frame
            .as_ref()
            .is_some_and(|(sequence, frame)| correlated_frame(*sequence, frame, &viewport, 2))
    );

    // Only the explicitly requested close/cancellation-ack phase may consume
    // Closed. A later Runtime in that very same batch must still be fatal.
    native.expect_closed = true;
    assert!(
        native
            .observe_events(vec![event(5, api::EventKind::Closed)])
            .is_ok()
    );
    assert!(
        native
            .observe_events(vec![
                event(6, api::EventKind::Closed),
                event(
                    7,
                    api::EventKind::Error {
                        code: api::ErrorCode::Runtime,
                        message: "late runtime failure".into()
                    }
                ),
            ])
            .is_err()
    );
}

#[test]
fn office_native_finished_after_drain_rejects_success_before_live_wait_returns() {
    let mut native = Native {
        session: None,
        source: None,
        trace: Arc::new(Trace::default()),
        window_closed: Arc::new(AtomicBool::new(false)),
        report: PathBuf::new(),
        phase_observation: None,
        phase: "unit",
        sequence: 0,
        last_header: None,
        source_refusal: None,
        diagnostic_events: 0,
        diagnostic_deadline: None,
        viewport: api::Viewport {
            revision: 1,
            width: 20,
            height: 20,
            scale: 1.0,
        },
        expected_target: ALPHA_FIRST.into(),
        loaded: 0,
        loading: 0,
        frame: None,
        finds: VecDeque::new(),
        selection: None,
        expect_closed: false,
        expected_scope_releases: 1,
    };
    let event = |sequence, kind| api::Event {
        header: api::Header {
            session: 1,
            document: 1,
            sequence,
        },
        kind,
    };
    let sampled = std::cell::Cell::new(false);
    let result = native.observe_live_batch(
        vec![
            event(
                1,
                api::EventKind::FindResult(api::FindResult {
                    request_sequence: 7,
                    matches: 1,
                    active: 1,
                    complete: true,
                }),
            ),
            event(2, api::EventKind::Selection(ALPHA.into())),
        ],
        |observed| {
            // The real poll uses Session::finished here. Delaying this injected
            // read until after both events models completion after the drain.
            assert_eq!(observed.sequence, 2);
            assert!(
                observed
                    .finds
                    .front()
                    .is_some_and(|(_, result)| { result.complete && result.matches == 1 })
            );
            assert!(
                observed
                    .selection
                    .as_ref()
                    .is_some_and(|(_, text)| text == ALPHA)
            );
            sampled.set(true);
            Ok(true)
        },
    );
    assert!(sampled.get());
    assert_eq!(
        result,
        Err("native browser exited before requested evidence".into())
    );
    assert!(native.observe_live_batch(vec![], |_| Ok(false)).is_ok());
    assert!(
        native
            .observe_live_batch(vec![event(3, api::EventKind::Closed)], |_| {
                panic!("unexpected Closed must fail before the liveness probe")
            })
            .is_err()
    );

    native.expect_closed = true;
    assert!(
        native
            .observe_live_batch(vec![event(4, api::EventKind::Closed)], |_| {
                panic!("the explicitly requested close phase needs no live-session probe")
            })
            .is_ok()
    );
    assert!(
        native
            .observe_live_batch(
                vec![
                    event(5, api::EventKind::Closed),
                    event(
                        6,
                        api::EventKind::Error {
                            code: api::ErrorCode::Runtime,
                            message: "late runtime failure".into(),
                        },
                    ),
                ],
                |_| panic!("Runtime must fail the whole batch even during requested close"),
            )
            .is_err()
    );
}

#[test]
fn office_native_read_refusal_requires_the_real_gated_read_tuple_and_adjacent_diagnostic() {
    let armed = api::Header {
        session: 1,
        document: 2,
        sequence: 10,
    };
    let header = api::Header {
        sequence: 11,
        ..armed
    };
    let gate = Arc::new(ReadGate::new(Arc::new(AtomicBool::new(false))));
    gate.length.store(13, Ordering::Release);
    gate.entered.store(true, Ordering::Release);
    gate.delegated.store(true, Ordering::Release);
    gate.delegated_denial.store(true, Ordering::Release);
    let expected = || {
        let mut source = SourceRefusal::new(armed, 10, ALPHA_ROWS);
        source.read_gate = Some(gate.clone());
        source
    };
    let read = |request, resource, offset, length| api::Event {
        header,
        kind: api::EventKind::ResourceRead {
            request,
            resource,
            offset,
            length,
        },
    };
    let diagnostic = api::Event {
        header,
        kind: api::EventKind::Error {
            code: api::ErrorCode::ResourceChanged,
            message: SOURCE_CHANGED.into(),
        },
    };
    let mut source = expected();
    assert!(source.observe(&read(42, 7, 0, 13)).unwrap());
    assert!(source.observe(&diagnostic).unwrap());
    assert_eq!(
        source.observed,
        Some(HostRefusal {
            header,
            request: 42,
            base: 7
        })
    );
    for origin in [
        read(0, 7, 0, 13),
        read(42, 0, 0, 13),
        read(42, 7, 1, 13),
        read(42, 7, 0, 12),
    ] {
        assert!(expected().observe(&origin).is_err());
    }
    assert!(expected().observe(&diagnostic).is_err());
    let mut interrupted = expected();
    assert!(interrupted.observe(&read(42, 7, 0, 13)).unwrap());
    assert!(
        interrupted
            .observe(&api::Event {
                header,
                kind: api::EventKind::Ready
            })
            .is_err()
    );
    gate.delegated_denial.store(false, Ordering::Release);
    assert!(expected().observe(&read(42, 7, 0, 13)).is_err());
}

#[test]
fn office_native_frame_evidence_rejects_uniform_and_transparent_surfaces() {
    let frame = |pixel: [u8; 4]| api::Frame {
        viewport_revision: 1,
        acknowledged_input: 1,
        width: 20,
        height: 20,
        rgba: pixel.repeat(400),
    };
    for pixel in [
        [0, 0, 0, 255],
        [128, 128, 128, 255],
        [255, 255, 255, 255],
        [0, 0, 0, 0],
        [255, 255, 255, 0],
    ] {
        assert!(
            !nonblank(&frame(pixel)),
            "uniform surface passed: {pixel:?}"
        );
    }
    let mut contrasted = frame([255, 255, 255, 255]);
    for pixel in contrasted.rgba[..200 * 4].as_chunks_mut::<4>().0 {
        *pixel = [32, 37, 43, 255];
    }
    assert!(nonblank(&contrasted));
    let viewport = api::Viewport {
        revision: 2,
        width: 20,
        height: 20,
        scale: 1.0,
    };
    assert!(
        !correlated_frame(11, &contrasted, &viewport, 10),
        "fresh sequence incorrectly qualified an old viewport"
    );
    contrasted.viewport_revision = 2;
    assert!(
        !correlated_frame(10, &contrasted, &viewport, 10),
        "frame predating content observation passed"
    );
    assert!(correlated_frame(11, &contrasted, &viewport, 10));
    assert!(
        !correlated_frame(
            11,
            &contrasted,
            &api::Viewport {
                width: 21,
                ..viewport
            },
            10,
        ),
        "wrong native output size passed"
    );
    for pixel in contrasted.rgba.as_chunks_mut::<4>().0 {
        pixel[3] = 0;
    }
    assert!(!nonblank(&contrasted), "transparent contrast passed");
    contrasted.rgba.pop();
    assert!(!nonblank(&contrasted), "malformed frame length passed");
}

fn ensure(condition: bool, message: &str) -> Result<(), String> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

fn action_refusal(kind: &api::EventKind) -> bool {
    matches!(kind, api::EventKind::Error { code: api::ErrorCode::Denied, message }
        if !message.is_empty() && message.len() <= 512 && !message.chars().any(char::is_control))
}

fn text(error: impl fmt::Display) -> String {
    error.to_string()
}

fn combine(first: Result<(), String>, second: Result<(), String>) -> Result<(), String> {
    match (first, second) {
        (Ok(()), Ok(())) => Ok(()),
        (Err(error), Ok(())) | (Ok(()), Err(error)) => Err(error),
        (Err(first), Err(second)) => Err(format!("{first}; {second}")),
    }
}
