//! Join existing shaped row coordinates to the provider's validated source anchors.
use super::*;

pub(super) fn build(
    page: &Page,
    shaped: &TextPage,
    length: u64,
    complete_context: bool,
    viewport: u32,
    rows_go_on: bool,
) -> Option<ViewerScrollMap> {
    // A complete already-loaded layout has an exact pixel range. Keep its
    // existing linear seek behavior; this never requests more source bytes.
    if complete_context && !page.before && !page.after {
        return ViewerScrollMap::new(vec![
            ViewerScrollAnchor {
                y: 0.0,
                fraction: 0.0,
            },
            ViewerScrollAnchor {
                y: f64::from(shaped.height().max(1.0)),
                fraction: 1.0,
            },
        ]);
    }
    // Rows too long for a page to hold the window go on below it at its own density while
    // a refill grows the page with them (`rows_go_on`, `Worker::rows_go_on`), also below a
    // page shorter than the window. Other tiny byte-limited fragments cannot describe a
    // complete viewport. They keep the existing explicit fallback path rather than
    // inventing rows.
    let short = shaped.height() < viewport as f32;
    if short && !rows_go_on {
        return None;
    }
    let source_fraction = |index: usize| {
        let index = page
            .anchors
            .partition_point(|a| (a.text_offset as usize) < index);
        page.anchors
            .get(index)
            .or_else(|| page.anchors.last())
            .map(|anchor| anchor.source_offset as f64 / length.max(1) as f64)
    };
    let mut anchors: Vec<ViewerScrollAnchor> = Vec::new();
    for (index, y) in shaped.scroll_anchors() {
        let fraction = source_fraction(index)?;
        let y = f64::from(y);
        if let Some(previous) = anchors.last_mut() {
            if y < previous.y || fraction < previous.fraction {
                return None;
            }
            if y == previous.y {
                previous.fraction = fraction;
                continue;
            }
        }
        anchors.push(ViewerScrollAnchor { y, fraction });
    }
    if page.before && anchors.first().is_some_and(|a| a.y > 0.0) {
        anchors.insert(
            0,
            ViewerScrollAnchor {
                y: 0.0,
                fraction: page.start as f64 / length.max(1) as f64,
            },
        );
    }
    if !page.before {
        if anchors.first().is_some_and(|a| a.y == 0.0) {
            anchors[0].fraction = 0.0;
        } else {
            anchors.insert(
                0,
                ViewerScrollAnchor {
                    y: 0.0,
                    fraction: 0.0,
                },
            );
        }
    }
    if !page.after
        && let Some(last) = anchors.last_mut()
    {
        last.fraction = 1.0;
    }
    if rows_go_on && !extend(&mut anchors, f64::from(viewport)) && short {
        return None;
    }
    ViewerScrollMap::new(anchors)
}

/// Continue the `anchors` of a page of rows a window below its last line's start, at the
/// density of its whole rows: from its second line's start (its first row may be cut) to its
/// last's. The page's end at its bottom gives way to them, as its last row may be cut. A camera
/// over the page's last rows, which rows too long for a page to hold the window allow
/// (`viewport::camera_span`), then shows the thumb it will show once a refill has grown the
/// page with the rows the file goes on with, and a scrollbar position lands where it stays.
/// Nothing is added when the page holds no whole row after its first one, or when the rows
/// would reach the file's end.
fn extend(anchors: &mut Vec<ViewerScrollAnchor>, viewport: f64) -> bool {
    let rows = &anchors[..anchors.len().saturating_sub(1)];
    let (Some(second), Some(last)) = (rows.get(1).copied(), rows.last().copied()) else {
        return false;
    };
    if rows.len() < 3 || last.y <= second.y || last.fraction <= second.fraction {
        return false;
    }
    let fraction =
        last.fraction + (last.fraction - second.fraction) / (last.y - second.y) * viewport;
    if fraction >= 1.0 {
        return false;
    }
    anchors.pop();
    anchors.push(ViewerScrollAnchor {
        y: last.y + viewport,
        fraction,
    });
    true
}
