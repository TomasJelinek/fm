use super::*;

fn fill(scene: &mut Scene) -> usize {
    let mut count = 0;
    while let Some((key, charge)) = scene.next() {
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
fn pan_repositions_retained_tiles_and_stale_inflight_zoom_never_replaces_them() {
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
    geometry.zoom(1.25, geometry.center());
    scene.update(0, &geometry).unwrap();
    scene
        .complete(
            stale,
            Arc::new(ViewerPixels::new(vec![255; 256 * 256 * 4], charge).unwrap()),
        )
        .unwrap();
    assert!(scene.tiles().is_none());
}
