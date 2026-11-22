//! Opt-in headless measurement: a Windows touchpad fling through the real panel
//! input path, frame by frame in software rendering. Run with
//! `cargo test --release -p midnight-ui --lib fling_measure -- --ignored --nocapture`.
use super::*;
use slint::{ModelRc, ModelTracker};
use std::time::Instant;

/// Counts row reads; the presenter's refresh then skips its own row
/// notifications, which an unchanged presenter does not send anyway.
struct CountedRows {
    real: ModelRc<crate::FileRow>,
    reads: Cell<usize>,
}

impl Model for CountedRows {
    type Data = crate::FileRow;

    fn row_count(&self) -> usize {
        self.real.row_count()
    }

    fn row_data(&self, row: usize) -> Option<crate::FileRow> {
        self.reads.set(self.reads.get() + 1);
        self.real.row_data(row)
    }

    fn model_tracker(&self) -> &dyn ModelTracker {
        self.real.model_tracker()
    }
}

fn percentile(values: &mut [f64], share: f64) -> f64 {
    values.sort_by(f64::total_cmp);
    values[((values.len() as f64 * share) as usize).min(values.len() - 1)]
}

fn fling(mode: i32) {
    let h = harness_entries(true, false, || {
        (0..50_000u64)
            .map(|id| {
                Entry::new(
                    EntryId(id),
                    PathBuf::from(format!("entry-{id:06}-文档.png")),
                    EntryKind::File,
                )
            })
            .collect()
    });
    h.window.invoke_view_mode(0, mode);
    h.advance(2);
    let real = h.window.get_left_rows();
    let counter = Rc::new(CountedRows {
        real: real.clone(),
        reads: Cell::new(0),
    });
    h.window.set_left_rows(counter.clone().into());
    let at_x = h.window.get_left_list_left() + 100.0;
    let at_y = h.window.get_left_list_top() + 100.0;
    h.point(at_x, at_y);
    h.advance(4);
    counter.reads.set(0);
    let start = h.offset(0);
    // 12 finger reports of 0.8 line, then 80 momentum reports decaying by 5 %, 8 ms
    // apart: the probe's `--fling` schedule. A frame follows every other report.
    let reports: Vec<f32> = std::iter::repeat_n(-0.8f32, 12)
        .chain((0..80).map(|k| -((96.0 * 0.95f64.powi(k)).round() / 120.0) as f32))
        .filter(|lines| *lines != 0.0)
        .collect();
    let mut frame_ms = Vec::new();
    let mut reads = Vec::new();
    let mut range = (
        h.window.get_left_thumbnail_row_start(),
        h.window.get_left_thumbnail_row_end(),
    );
    let (mut changes, mut largest) = (0usize, range.1 - range.0);
    let mut frame = |h: &Harness| {
        let began = Instant::now();
        h.time.set(h.time.get() + Duration::from_millis(8));
        slint::platform::update_timers_and_animations();
        h.input(WindowEvent::RedrawRequested);
        h.draw();
        frame_ms.push(began.elapsed().as_secs_f64() * 1000.0);
        reads.push(counter.reads.replace(0));
        let now = (
            h.window.get_left_thumbnail_row_start(),
            h.window.get_left_thumbnail_row_end(),
        );
        if now != range {
            changes += 1;
            range = now;
            largest = largest.max(now.1 - now.0);
        }
    };
    let input_began = Instant::now();
    for (index, lines) in reports.iter().enumerate() {
        h.time.set(h.time.get() + Duration::from_millis(8));
        assert!(h.wheel(*lines));
        if index % 2 == 1 {
            frame(&h);
        }
    }
    // The glide's tail and, after the list rests, the row alignment (Details).
    let mut quiet = 0;
    let mut tail = 0;
    let mut previous = h.offset(0);
    while quiet < 30 {
        frame(&h);
        let current = h.offset(0);
        quiet = if current == previous && !h.timer.running() {
            quiet + 1
        } else {
            0
        };
        previous = current;
        tail += 1;
        assert!(tail < 2000, "the fling ends");
    }
    let total_ms = input_began.elapsed().as_secs_f64() * 1000.0;
    let moving = frame_ms.len() - 30;
    let mut active: Vec<f64> = frame_ms[..moving].to_vec();
    let busy: f64 = active.iter().sum();
    println!(
        "panel-fling-measure {}",
        serde_json::json!({
            "view_mode": mode, "entries": 50_000, "window": "1340x820 software",
            "reports": reports.len(), "frames_moving": moving,
            "travel_px": start - h.offset(0), "row_height": h.window.get_left_view_row_height(),
            "frame_ms_p50": percentile(&mut active, 0.5),
            "frame_ms_p95": percentile(&mut active, 0.95),
            "frame_ms_max": percentile(&mut active, 1.0),
            "frame_ms_sum": busy, "wall_ms": total_ms,
            "row_reads": reads.iter().sum::<usize>(),
            "row_reads_max_frame": reads.iter().max(),
            "thumbnail_range_changes": changes, "largest_thumbnail_range": largest,
        })
    );
    h.window.set_left_rows(real);
}

#[test]
#[ignore = "measurement; run explicitly in release"]
fn details_fling_measure() {
    fling(0);
}

#[test]
#[ignore = "measurement; run explicitly in release"]
fn thumbnails_fling_measure() {
    fling(2);
}
