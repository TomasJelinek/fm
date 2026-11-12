//! Join existing shaped row coordinates to the provider's validated source anchors.
use super::*;

pub(super) fn build(
    page: &Page,
    shaped: &TextPage,
    length: u64,
    complete_context: bool,
    viewport: u32,
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
    // Tiny byte-limited fragments cannot describe a complete viewport. They
    // keep the existing explicit fallback path rather than inventing rows.
    if shaped.height() < viewport as f32 {
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
    ViewerScrollMap::new(anchors)
}
