//! Shell artwork at the edge it is drawn at, from the system image lists.
//!
//! The Shell keeps every icon it has looked up in four process-wide lists whose
//! edges follow the system DPI (16, 32, 48 and 256 px at 96 DPI; 36, 72, 108 and
//! 256 px at 216 DPI). An icon is drawn from the smallest list at least as large as
//! the requested edge and reduced once with a premultiplied-alpha filter
//! (`icon_scale::fit`), so it is never enlarged while larger artwork exists.
use super::{Dib, OwnedIcon, straight_rgba};
use midnight_platform_common::icon_scale;
use windows::Win32::UI::{
    Controls::IImageList,
    Shell::{SHGetImageList, SHIL_EXTRALARGE, SHIL_JUMBO, SHIL_LARGE, SHIL_SMALL},
};

/// `ILD_TRANSPARENT`: the list's own transparency, without its background color.
const DRAW_TRANSPARENT: u32 = 0x0001;
/// The four overlay bits of an image list drawing request (`ILD_OVERLAYMASK`).
const OVERLAY_MASK: u32 = 0x0F00;

/// An entry of the system image lists, with the Shell overlay drawn over it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct SystemImage {
    pub index: i32,
    /// One-based Shell overlay index (`SHGetIconOverlayIndexW`), zero for none.
    pub overlay: u32,
}

struct List {
    list: IImageList,
    edge: u16,
    jumbo: bool,
}

/// The available system image lists, smallest first. A list whose edge is
/// unknown or outside 1..=256 is left out.
fn lists() -> Vec<List> {
    let mut lists = Vec::with_capacity(4);
    for which in [SHIL_SMALL, SHIL_LARGE, SHIL_EXTRALARGE, SHIL_JUMBO] {
        // SAFETY: a documented list identifier and the interface identifier of
        // the requested type; the returned reference is owned and released on drop.
        let Ok(list) = (unsafe { SHGetImageList::<IImageList>(which as i32) }) else {
            continue;
        };
        let (mut width, mut height) = (0, 0);
        // SAFETY: both outputs point to live integers for the duration of the call.
        if unsafe { list.GetIconSize(&mut width, &mut height) }.is_err() || width != height {
            continue;
        }
        let Some(edge) = u16::try_from(width)
            .ok()
            .filter(|edge| (1..=midnight_core::MAX_ICON_EDGE).contains(edge))
        else {
            continue;
        };
        if lists.iter().any(|known: &List| known.edge == edge) {
            continue;
        }
        lists.push(List {
            list,
            edge,
            jumbo: which == SHIL_JUMBO,
        });
    }
    lists.sort_by_key(|list| list.edge);
    lists
}

/// `image` as straight-alpha RGBA of `edge` by `edge` pixels, drawn from the
/// smallest list at least as large as `edge`, else from the largest.
pub(super) fn render(image: SystemImage, edge: u16) -> Option<Vec<u8>> {
    let lists = lists();
    let at = lists
        .iter()
        .position(|list| list.edge >= edge)
        .or_else(|| lists.len().checked_sub(1))?;
    let mut source = &lists[at];
    let mut rgba = draw(source, image)?;
    if source.jumbo {
        // Artwork without a 256 px image stands in the jumbo list as the next
        // smaller list's bitmap in its top-left corner, on a transparent canvas.
        // That list's own bitmap is then the largest true one.
        let smaller = at.checked_sub(1).map(|smaller| &lists[smaller]);
        let corner = smaller.map_or(48, |list| list.edge);
        if corner_art(&rgba, source.edge, corner)
            && let Some(smaller) = smaller
            && let Some(drawn) = draw(smaller, image)
        {
            source = smaller;
            rgba = drawn;
        }
    }
    let source_edge = u32::from(source.edge);
    icon_scale::fit(&rgba, source_edge, source_edge, u32::from(edge))
}

fn draw(source: &List, image: SystemImage) -> Option<Vec<u8>> {
    let flags = DRAW_TRANSPARENT | ((image.overlay << 8) & OVERLAY_MASK);
    // SAFETY: the list is live; an index outside it fails without drawing. The
    // returned icon is a new handle owned, and destroyed once, by `OwnedIcon`.
    let icon = OwnedIcon(unsafe { source.list.GetIcon(image.index, flags) }.ok()?.0);
    if icon.0.is_null() {
        return None;
    }
    let mut surface = Dib::new(source.edge)?;
    let black = surface.draw(icon.0, 0)?;
    let white = surface.draw(icon.0, 255)?;
    Some(straight_rgba(&black, &white))
}

/// Whether every visible pixel of the `edge` bitmap lies in its top-left
/// `corner` square: a smaller bitmap placed in a larger list, not artwork that
/// fills it. An empty bitmap counts as one as well.
fn corner_art(rgba: &[u8], edge: u16, corner: u16) -> bool {
    let (edge, corner) = (usize::from(edge), usize::from(corner));
    corner < edge
        && rgba
            .as_chunks::<4>()
            .0
            .iter()
            .enumerate()
            .all(|(at, pixel)| pixel[3] == 0 || (at % edge < corner && at / edge < corner))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_bitmap_confined_to_the_top_left_square_is_a_smaller_image() {
        let edge = 8u16;
        let mut rgba = vec![0u8; 8 * 8 * 4];
        assert!(corner_art(&rgba, edge, 3), "an empty bitmap");
        let set = |rgba: &mut [u8], x: usize, y: usize| rgba[(y * 8 + x) * 4 + 3] = 255;
        set(&mut rgba, 0, 0);
        set(&mut rgba, 2, 2);
        assert!(corner_art(&rgba, edge, 3));
        set(&mut rgba, 3, 0);
        assert!(!corner_art(&rgba, edge, 3), "a pixel right of the square");
        let mut below = vec![0u8; 8 * 8 * 4];
        below[(3 * 8) * 4 + 3] = 1;
        assert!(
            !corner_art(&below, edge, 3),
            "a faint pixel below the square"
        );
        assert!(
            !corner_art(&below, edge, 8),
            "the square is the whole bitmap"
        );
    }
}
