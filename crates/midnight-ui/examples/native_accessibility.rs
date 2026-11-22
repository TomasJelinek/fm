//! Explicit QA-only native UIA/virtualization fixture; never loads user settings.
use serde_json::{Value, json};
use slint::{ComponentHandle, Model};
use std::{
    cell::RefCell,
    collections::HashSet,
    io::{self, Write},
    path::PathBuf,
    rc::Rc,
    time::{Duration, Instant},
};

#[cfg(windows)]
#[path = "native_accessibility/windows.rs"]
mod windows;

const ROWS: usize = 100_000;
const POSITIONS: [i32; 4] = [0, 50_000, 99_980, 0];
const PLAIN: &str = "AX-PLAIN-POSITIVE-7319";
const PASSWORD: &str = "\u{e100}\u{e101}\u{e102}\u{e103}\u{e104}\u{e105}\u{e106}\u{e107}";

slint::slint! {
    import { LineEdit, VerticalBox } from "std-widgets.slint";
    import { PanelList } from "ui/panel-list.slint";
    export component AccessibilityProbe inherits Window {
        title: "ME AX Canary and Virtual Panel QA";
        width: 680px; height: 620px;
        in property <[int]> rows;
        in property <string> phase;
        in property <string> plain-canary;
        in property <string> password-canary;
        out property <length> panel-height <=> list.visible-height;
        callback advance();
        public function reveal(row: int) {
            list.stop-scroll-motion();
            list.content-y = -row * 24px;
        }
        init => { plain.focus(); }
        FocusScope {
            capture-key-pressed(event) => {
                if event.text == Key.F8 { root.advance(); return accept; }
                reject
            }
            VerticalBox {
                Text {
                    text: "AX phase " + root.phase;
                    accessible-role: text;
                    accessible-label: self.text;
                }
                plain := LineEdit {
                    accessible-label: "AX Plain";
                    text: root.plain-canary;
                }
                LineEdit {
                    accessible-label: "AX Password";
                    input-type: password;
                    text: root.password-canary;
                }
                Rectangle {
                    min-width: 80px; max-width: 80px;
                    min-height: 60px; max-height: 60px;
                    background: #115395;
                    accessible-role: text;
                    accessible-label: "Capture canary";
                }
                list := PanelList {
                    vertical-stretch: 1; min-height: 300px; row-height: 24px;
                    for row[index] in root.rows: Rectangle {
                        height: 24px;
                        Rectangle {
                            width: parent.width; height: parent.height;
                            visible: list.band-visible(index);
                            background: mod(row, 2) == 0 ? #eeeeee : #dddddd;
                            accessible-role: list-item;
                            accessible-label: "AX row " + row;
                            Text { text: "AX row " + row; color: #202020; }
                        }
                    }
                }
            }
        }
    }
}

#[derive(Default)]
struct Reads {
    calls: usize,
    unique: HashSet<usize>,
}

impl Reads {
    fn report_and_reset(&mut self) -> Value {
        let report = json!({"rowDataCalls": self.calls, "uniqueRows": self.unique.len(),
            "minRow": self.unique.iter().min().copied(), "maxRow": self.unique.iter().max().copied()});
        self.calls = 0;
        self.unique.clear();
        report
    }
}

struct CountedRows {
    rows: Rc<slint::VecModel<i32>>,
    reads: Rc<RefCell<Reads>>,
}

impl Model for CountedRows {
    type Data = i32;

    fn row_count(&self) -> usize {
        self.rows.row_count()
    }

    fn row_data(&self, row: usize) -> Option<i32> {
        let mut reads = self.reads.borrow_mut();
        reads.calls += 1;
        reads.unique.insert(row);
        self.rows.row_data(row)
    }

    fn model_tracker(&self) -> &dyn slint::ModelTracker {
        self.rows.model_tracker()
    }
}

#[cfg(windows)]
fn cpu_ms() -> Option<f64> {
    windows::thread_cpu_ms()
}

#[cfg(not(windows))]
fn cpu_ms() -> Option<f64> {
    None
}

struct State {
    phase: usize,
    next: usize,
    running: bool,
    started: Instant,
    cpu: Option<f64>,
    reports: Vec<Value>,
    activation: Vec<Value>,
    completed: bool,
    timed_out: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let output = PathBuf::from(
        std::env::args_os()
            .nth(1)
            .ok_or("Expected owned QA JSON output")?,
    );
    let qa = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../_build/qa/accessibility-feature-20261119")
        .canonicalize()?;
    let parent = output
        .parent()
        .ok_or("Output needs parent")?
        .canonicalize()?;
    if !output.is_absolute() || !parent.starts_with(qa) || output.exists() {
        return Err("Output must be a new file under the owned D accessibility QA root".into());
    }
    let output = parent.join(output.file_name().ok_or("Output needs filename")?);
    slint::BackendSelector::new()
        .backend_name("winit".into())
        .renderer_name("software".into())
        .select()?;
    let window = AccessibilityProbe::new()?;
    let reads = Rc::new(RefCell::new(Reads::default()));
    window.set_rows(
        Rc::new(CountedRows {
            rows: Rc::new(slint::VecModel::from((0..ROWS as i32).collect::<Vec<_>>())),
            reads: reads.clone(),
        })
        .into(),
    );
    window.set_plain_canary(PLAIN.into());
    window.set_password_canary(PASSWORD.into());
    window.set_phase("never_queried_running".into());
    let state = Rc::new(RefCell::new(State {
        phase: 0,
        next: 0,
        running: true,
        started: Instant::now(),
        cpu: cpu_ms(),
        reports: Vec::new(),
        activation: Vec::new(),
        completed: false,
        timed_out: false,
    }));
    window.on_advance({
        let state = state.clone();
        let reads = reads.clone();
        let weak = window.as_weak();
        move || {
            let mut state = state.borrow_mut();
            if state.running || state.phase >= 2 {
                return;
            }
            let Some(window) = weak.upgrade() else { return };
            state.activation.push(reads.borrow_mut().report_and_reset());
            state.phase += 1;
            state.next = 0;
            state.running = true;
            state.started = Instant::now();
            state.cpu = cpu_ms();
            window.set_phase(
                if state.phase == 1 {
                    "active_running"
                } else {
                    "retired_running"
                }
                .into(),
            );
        }
    });
    let timer = slint::Timer::default();
    timer.start(slint::TimerMode::Repeated, Duration::from_millis(150), {
        let state = state.clone();
        let reads = reads.clone();
        let weak = window.as_weak();
        move || {
            let Some(window) = weak.upgrade() else { return };
            let mut state = state.borrow_mut();
            if !state.running {
                return;
            }
            if let Some(&row) = POSITIONS.get(state.next) {
                window.invoke_reveal(row);
                state.next += 1;
                return;
            }
            let count = reads.borrow_mut().report_and_reset();
            let visible = (window.get_panel_height() / 24.0).ceil().max(1.0) as usize;
            // Conservative QA envelope for four jumps plus ListView's retained
            // boundary/measurement cells; not an application row-count limit.
            let bound = POSITIONS.len() * (visible + 8) * 2 + 16;
            let qualified = count["uniqueRows"].as_u64().is_some_and(|n| n > 0 && n <= bound as u64)
                && count["minRow"] == 0 && count["maxRow"].as_u64().is_some_and(|n| n >= 99_980 && n < ROWS as u64);
            let phase = state.phase;
            let report = json!({"phase": phase, "counts": count, "visibleRows": visible,
                "uniqueRowEnvelope": bound, "virtualizationQualified": qualified,
                "coldWindowShowAndInitialFontsIncluded": phase == 0,
                "elapsedMs": state.started.elapsed().as_secs_f64()*1000.0,
                "uiThreadCpuMs": state.cpu.zip(cpu_ms()).map(|(start,end)| end-start),
                "measurement": "Finite scripted viewport positions; CPU/work counts, not presentation latency"});
            state.reports.push(report);
            state.running = false;
            match phase {
                0 => { window.set_phase("inactive_done".into()); println!("AX_BARRIER inactive_done"); }
                1 => { window.set_phase("active_done".into()); println!("AX_BARRIER active_done"); }
                _ => {
                    state.completed = true;
                    window.set_phase("complete".into());
                    let _ = window.hide();
                    let _ = slint::quit_event_loop();
                }
            }
        }
    });
    let deadline = slint::Timer::default();
    deadline.start(slint::TimerMode::SingleShot, Duration::from_secs(75), {
        let state = state.clone();
        let weak = window.as_weak();
        move || {
            state.borrow_mut().timed_out = true;
            if let Some(window) = weak.upgrade() {
                let _ = window.hide();
            }
            let _ = slint::quit_event_loop();
        }
    });
    window.show()?;
    slint::run_event_loop()?;
    timer.stop();
    deadline.stop();
    let state = state.borrow();
    let visible_rows = state
        .reports
        .iter()
        .filter_map(|r| r["visibleRows"].as_u64())
        .max()
        .unwrap_or(1);
    let activation_bound = (visible_rows + 8) * 2 + 16;
    let activation_ok = state.activation.iter().all(|n| {
        n["uniqueRows"]
            .as_u64()
            .is_some_and(|n| n <= activation_bound)
    });
    let qualified = state.completed
        && !state.timed_out
        && state.reports.len() == 3
        && activation_ok
        && state
            .reports
            .iter()
            .all(|r| r["virtualizationQualified"] == true);
    let report = json!({"qualified": qualified, "renderer": "winit/software", "rows": ROWS,
        "phases": state.reports, "betweenPhaseRowReads": state.activation,
        "activationUniqueRowEnvelope": activation_bound, "activationVirtualizationQualified": activation_ok,
        "timedOut": state.timed_out, "persistence": "No settings/backend/filesystem service installed",
        "limits": "QA component/native provider only. Phase0 includes cold show/initial fonts; raw phase CPU ratios do not isolate accessibility cost. No whole-app/F3/default-WGPU/performance-improvement qualification"});
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)?;
    serde_json::to_writer_pretty(&mut file, &report)?;
    writeln!(file)?;
    if !qualified {
        return Err(
            io::Error::other("Accessibility QA phases/virtualization did not qualify").into(),
        );
    }
    Ok(())
}
