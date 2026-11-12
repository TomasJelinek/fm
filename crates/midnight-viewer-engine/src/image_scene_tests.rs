use super::*;

fn fill(scene: &mut Scene) -> usize {
    let mut count = 0;
    while let Some((key, charge)) = scene.next().unwrap() {
        let pixels = ViewerPixels::new(vec![255; 256 * 256 * 4], charge).unwrap();
        scene.complete(key, Arc::new(pixels)).unwrap();
        count += 1;
    }
    count
}

#[test]
fn old_zoom_resize_page_and_file_cameras_cannot_mutate_a_replacement_surface() {
    let ids = Arc::new(AtomicU64::new(0));
    let mut scene = Scene::new(ids.clone());
    let mut geometry = ImageGeometry::new(4000, 3000, [900, 600]);
    geometry.actual();
    scene.update(0, &geometry).unwrap();
    fill(&mut scene);
    let old = scene.surface;
    assert!(scene.accept_pan(&mut geometry, old, 10, [1400.25, 1300.75]));
    assert!(!scene.accept_pan(&mut geometry, old, 9, [0.0; 2]));
    assert!(!scene.accept_pan(&mut geometry, old, 11, [f64::NAN; 2]));
    for change in 0..3 {
        match change {
            0 => geometry.zoom(1.25, geometry.center()),
            1 => geometry.resize([1200, 800]),
            _ => {}
        }
        scene
            .update(if change == 2 { 1 } else { 0 }, &geometry)
            .unwrap();
        assert_ne!(scene.surface, old);
        assert!(!scene.accept_pan(&mut geometry, old, 100, [0.0; 2]));
    }
    let mut next_file = Scene::new(ids);
    next_file.update(0, &geometry).unwrap();
    assert_ne!(next_file.surface, scene.surface);
}

#[test]
fn pan_repositions_retained_tiles_and_stale_inflight_sampling_scale_never_replaces_them() {
    let mut scene = Scene::new(Arc::new(AtomicU64::new(0)));
    let mut geometry = ImageGeometry::new(4000, 3000, [900, 600]);
    geometry.actual();
    scene.update(0, &geometry).unwrap();
    fill(&mut scene);
    let before = scene.tiles().unwrap();
    geometry.scroll([5.25, 7.75]);
    scene.update(0, &geometry).unwrap();
    assert_eq!(fill(&mut scene), 0);
    let after = scene.tiles().unwrap();
    assert_eq!(before[0].id, after[0].id);
    assert!((before[0].position[0] - after[0].position[0] - 5.25).abs() < 0.001);
    let stale = scene.plan.as_ref().unwrap().visible[0];
    let charge = scene.cache.reserve().unwrap();
    geometry.zoom(0.8, geometry.center());
    scene.update(0, &geometry).unwrap();
    scene
        .complete(
            stale,
            Arc::new(ViewerPixels::new(vec![255; 256 * 256 * 4], charge).unwrap()),
        )
        .unwrap();
    assert!(scene.tiles().is_none());
}

#[test]
fn blank_tile_optimization_retains_grid_coverage_and_only_renders_image_intersections() {
    let mut geometry = ImageGeometry::new(607, 204, [2250, 1400]);
    for (basis, expected) in [(5000, 2), (10000, 3), (12500, 3), (15625, 3)] {
        let mut scene = Scene::new(Arc::new(AtomicU64::new(0)));
        geometry.restore_zoom(midnight_core::ViewerZoomPreference::ScaleBasisPoints(basis));
        scene.update(0, &geometry).unwrap();
        assert_eq!(
            fill(&mut scene),
            expected,
            "only tiles intersecting the source need decoder work"
        );
        let tiles = scene.tiles().unwrap();
        assert_eq!(tiles.len(), scene.plan.as_ref().unwrap().keys().count());
        assert_eq!(
            tiles
                .iter()
                .filter(|tile| !tile.pixels.is_known_transparent())
                .count(),
            expected
        );
        assert!(tiles.iter().all(|tile| tile.pixels.is_known_transparent()
            == tile.pixels.rgba().iter().all(|byte| *byte == 0)));
        assert_eq!(
            tiles
                .iter()
                .filter(|tile| tile.pixels.rgba().iter().any(|byte| *byte != 0))
                .count(),
            expected
        );
        assert_eq!(fill(&mut scene), 0);
    }
}

#[test]
fn blank_tile_optimization_preserves_fractional_edges_and_all_transparent_scenes() {
    for (dimension, expected) in [(256, 1), (257, 4)] {
        let mut scene = Scene::new(Arc::new(AtomicU64::new(0)));
        let mut geometry = ImageGeometry::new(dimension, dimension, [800, 600]);
        geometry.actual();
        scene.update(0, &geometry).unwrap();
        assert_eq!(
            fill(&mut scene),
            expected,
            "boundary pixels require decoder sampling"
        );
        geometry.origin = [4096.0; 2];
        scene.update(0, &geometry).unwrap();
        assert_eq!(fill(&mut scene), 0);
        let tiles = scene.tiles().unwrap();
        assert!(!tiles.is_empty());
        assert!(
            tiles
                .iter()
                .all(|tile| tile.pixels.rgba().iter().all(|byte| *byte == 0))
        );
    }
}

#[test]
fn magnification_reuses_exact_source_pixels_and_retires_previous_camera_surfaces() {
    let mut scene = Scene::new(Arc::new(AtomicU64::new(0)));
    let mut geometry = ImageGeometry::new(607, 204, [2250, 1400]);
    scene.update(0, &geometry).unwrap();
    assert_eq!(fill(&mut scene), 3);
    let originals = scene.tiles().unwrap();
    for basis in [12500, 15625, 38147, 116414, 10000] {
        let old_surface = scene.surface;
        let anchor = geometry.center();
        let source =
            std::array::from_fn(|axis| geometry.origin[axis] + anchor[axis] / geometry.scale);
        assert!(geometry.zoom_from(
            midnight_core::ViewerZoomPreference::ScaleBasisPoints(basis),
            source,
            anchor.map(|value| value as f32),
        ));
        scene.update(0, &geometry).unwrap();
        assert_ne!(scene.surface, old_surface);
        assert!(!scene.accept_pan(&mut geometry, old_surface, 1, [0.0; 2]));
        let plan = scene.plan.as_ref().unwrap();
        assert_eq!(plan.scale.to_bits(), 1.0f64.to_bits());
        assert!(plan.keys().all(|key| key.scale == 1.0f64.to_bits()));
        for axis in 0..2 {
            let source_extent = f64::from(geometry.viewport[axis]) / geometry.scale;
            assert!(f64::from(plan.raster_size[axis]) <= source_extent.max(1.0));
            assert!(f64::from(plan.raster_size[axis]) + 1.0 >= source_extent);
        }
        assert_eq!(
            fill(&mut scene),
            0,
            "magnification must reuse source pixels"
        );
        for tile in scene.tiles().unwrap() {
            if !tile.pixels.is_known_transparent() {
                let original = originals.iter().find(|old| old.id == tile.id).unwrap();
                assert!(Arc::ptr_eq(&tile.pixels, &original.pixels));
            }
        }
    }
}
#[test]
fn documents_and_downsampling_preserve_the_original_bounded_raster_geometry() {
    for viewport in [[1200, 800], [3840, 2160]] {
        for basis in [6250, 12500, 116414] {
            let mut geometry = ImageGeometry::document(607, 204, viewport);
            geometry.restore_zoom(midnight_core::ViewerZoomPreference::ScaleBasisPoints(basis));
            let (size, density) = geometry.raster_geometry();
            let mut document = Scene::new(Arc::new(AtomicU64::new(0)));
            document.update_document(0, &geometry).unwrap();
            let plan = document.plan.as_ref().unwrap();
            assert_eq!(plan.scale, geometry.scale * density);
            assert_eq!(plan.raster_size, size);
            if geometry.scale * density <= 1.0 {
                let mut image = Scene::new(Arc::new(AtomicU64::new(0)));
                image.update(0, &geometry).unwrap();
                let image_plan = image.plan.as_ref().unwrap();
                assert_eq!(image_plan.scale, plan.scale);
                assert_eq!(image_plan.raster_size, size);
            }
        }
    }
}
