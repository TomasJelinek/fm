use super::*;
use midnight_core::{ViewerPixelOwner, ViewerPixels};
use std::sync::Arc;

#[derive(Debug)]
struct Charge;
impl ViewerPixelOwner for Charge {}
fn info(origin: [f64; 2]) -> ViewerImageInfo {
    ViewerImageInfo {
        surface: ViewerSurfaceId(1),
        origin,
        visible_source: [128.0; 2],
        raster_scale: 1.0,
        zoom: 1.0,
        width: 4000,
        height: 3000,
        fitted: false,
        page: 0,
        page_count: 1,
        pan_sequence: 0,
    }
}
fn tile(position: [f32; 2]) -> ViewerTile {
    ViewerTile {
        id: ViewerTileId(1),
        position,
        width: 256,
        height: 256,
        pixels: Arc::new(ViewerPixels::new(vec![255; 256 * 256 * 4], Arc::new(Charge)).unwrap()),
    }
}

#[test]
fn camera_ack_keeps_the_same_repeater_model_and_texture_without_rebuilding_rows() {
    let mut textures = Textures::default();
    let first = textures
        .receive(&[tile([0.0; 2])], &info([0.0; 2]), [0.0; 2], true)
        .unwrap();
    let second = textures
        .receive(
            &[tile([-1.25, -2.5])],
            &info([1.25, 2.5]),
            [1.25, 2.5],
            false,
        )
        .unwrap();
    assert_eq!(
        first, second,
        "pan acknowledgement must preserve repeater identity"
    );
    assert_eq!(textures.uploads, 1);
}

#[test]
fn wide_diagonal_transition_evicts_optional_guards_and_preserves_both_visible_rectangles() {
    let pixels = Arc::new(ViewerPixels::new(vec![255; 256 * 256 * 4], Arc::new(Charge)).unwrap());
    let info = |origin| ViewerImageInfo {
        visible_source: [4096.0, 1024.0],
        width: 32768,
        height: 32768,
        ..info(origin)
    };
    let tiles = |origin: [f64; 2], low: i32| {
        (low..low + 6)
            .flat_map(|y| (low..low + 18).map(move |x| (x, y)))
            .map(|(x, y)| ViewerTile {
                id: ViewerTileId((1 + x + y * 100) as u64),
                position: [
                    (f64::from(x) * 256.0 - origin[0]) as f32,
                    (f64::from(y) * 256.0 - origin[1]) as f32,
                ],
                width: 256,
                height: 256,
                pixels: pixels.clone(),
            })
            .collect::<Vec<_>>()
    };
    let mut textures = Textures::default();
    textures
        .receive(&tiles([0.5; 2], 0), &info([0.5; 2]), [0.5; 2], true)
        .unwrap();
    textures
        .receive(&tiles([256.5; 2], 1), &info([256.5; 2]), [0.5; 2], false)
        .unwrap();
    for origin in [[0.5; 2], [256.5; 2]] {
        let (low, high) = grid_bounds(origin, &info(origin));
        assert!(
            (low[1]..high[1]).all(|y| (low[0]..high[0]).all(|x| textures.grid().contains(&[x, y])))
        );
    }
    assert!(textures.textures.len() <= MAX_ENTRIES);
    assert!(textures.bytes <= MAX_BYTES);
}
