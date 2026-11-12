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
fn transparent_tile(id: u64, position: [f32; 2]) -> ViewerTile {
    ViewerTile {
        id: ViewerTileId(id),
        position,
        width: 256,
        height: 256,
        pixels: Arc::new(ViewerPixels::transparent(256 * 256 * 4, Arc::new(Charge)).unwrap()),
    }
}

#[test]
fn transparent_tiles_cover_camera_motion_without_uploads_or_image_rows() {
    let mut textures = Textures::default();
    let info = info([0.0; 2]);
    let model = textures
        .receive(&[transparent_tile(1, [0.0; 2])], &info, info.origin, true)
        .unwrap();
    assert_eq!(model.row_count(), 0);
    assert_eq!(textures.uploads, 0);
    assert_eq!(textures.bytes, 0);
    let mut camera = crate::image_camera::Camera::default();
    camera.receive(&info, 1.0, textures.grid());
    assert!(camera.push(10.0, 12.0, std::time::Duration::ZERO, true));
    assert!(camera.tick(std::time::Duration::ZERO));
    assert_eq!(camera.origin, [10.0, 12.0]);
}

#[test]
fn replacing_visible_tiles_with_transparent_coverage_removes_rows_and_reuses_cached_images() {
    let mut textures = Textures::default();
    let visible = tile([0.0; 2]);
    let info = info([0.0; 2]);
    let model = textures
        .receive(std::slice::from_ref(&visible), &info, info.origin, true)
        .unwrap();
    assert_eq!(model.row_count(), 1);
    assert_eq!(textures.uploads, 1);
    let blank = textures
        .receive(&[transparent_tile(2, [0.0; 2])], &info, info.origin, false)
        .unwrap();
    assert_eq!(model, blank);
    assert_eq!(blank.row_count(), 0);
    assert_eq!(textures.uploads, 1);
    assert_eq!(textures.grid(), HashSet::from([[0, 0]]));
    let restored = textures
        .receive(&[visible], &info, info.origin, false)
        .unwrap();
    assert_eq!(model, restored);
    assert_eq!(restored.row_count(), 1);
    assert_eq!(textures.uploads, 1);
}

#[test]
fn transparent_metadata_is_bounded_and_eviction_removes_old_camera_coverage() {
    let mut textures = Textures::default();
    let pixels = Arc::new(ViewerPixels::transparent(256 * 256 * 4, Arc::new(Charge)).unwrap());
    for index in 0..=MAX_ENTRIES {
        let info = ViewerImageInfo {
            width: 65536,
            ..info([index as f64 * 256.0, 0.0])
        };
        let tile = ViewerTile {
            id: ViewerTileId(index as u64 + 1),
            position: [0.0; 2],
            width: 256,
            height: 256,
            pixels: pixels.clone(),
        };
        let model = textures
            .receive(&[tile], &info, info.origin, false)
            .unwrap();
        assert_eq!(model.row_count(), 0);
        assert!(textures.textures.len() <= MAX_ENTRIES);
    }
    assert_eq!(textures.textures.len(), MAX_ENTRIES);
    assert_eq!(textures.grid().len(), MAX_ENTRIES);
    assert!(!textures.grid().contains(&[0, 0]));
    assert!(textures.grid().contains(&[MAX_ENTRIES as i32, 0]));
    assert_eq!(textures.bytes, 0);
    assert_eq!(textures.uploads, 0);
}

#[test]
fn reused_tile_identity_cannot_change_transparency() {
    for transparent_first in [false, true] {
        let mut textures = Textures::default();
        let visible = tile([0.0; 2]);
        let transparent = transparent_tile(visible.id.0, [0.0; 2]);
        let (first, replacement) = if transparent_first {
            (transparent, visible)
        } else {
            (visible, transparent)
        };
        let info = info([0.0; 2]);
        textures
            .receive(&[first], &info, info.origin, true)
            .unwrap();
        assert!(
            textures
                .receive(&[replacement], &info, info.origin, false)
                .is_err()
        );
    }
}
