use super::*;

fn plan(x: f64, y: f64) -> Plan {
    Plan::new(0, 1.0, [x, y], [900, 600], [1.0, 1.0])
}
fn fill(cache: &mut Tiles, plan: &Plan) -> usize {
    let mut renders = 0;
    for key in plan.keys() {
        if cache.get(key).is_none() {
            let pixels =
                ViewerPixels::new(vec![255; TILE_BYTES], cache.reserve().unwrap()).unwrap();
            cache.insert(key, Arc::new(pixels), plan).unwrap();
            renders += 1;
        }
    }
    renders
}

#[test]
fn repeated_pan_reuses_pixels_and_guards_only_request_the_new_frontier() {
    let mut cache = Tiles::new(Arc::new(AtomicU64::new(0)));
    let original = plan(0.0, 0.0);
    assert!(fill(&mut cache, &original) > 0);
    let identity = cache.get(original.visible[0]).unwrap();
    assert_eq!(fill(&mut cache, &plan(12.25, 10.75)), 0);
    let retained = cache.get(original.visible[0]).unwrap();
    assert_eq!(identity.0, retained.0);
    assert!(Arc::ptr_eq(&identity.1, &retained.1));
    assert_eq!(fill(&mut cache, &plan(256.0, 0.0)), 4);
    assert_eq!(fill(&mut cache, &original), 0);
}

#[test]
fn every_bounded_aspect_and_fractional_origin_fits_visible_plus_one_frontier() {
    for width in [1u32, 128, 512, 1024, 2048, 4096] {
        let height = (4_194_304 / width).min(4096);
        let plan = Plan::new(1, 0.25, [-19.5, 1023.5], [width, height], [-1.0, 1.0]);
        assert!(plan.keys().count() <= CACHE_ENTRIES, "{width}x{height}");
        assert!(plan.keys().count() * TILE_BYTES <= CACHE_BYTES);
    }
}

#[test]
fn pixel_lease_counts_retained_frames_eviction_and_inflight_until_last_drop() {
    let ids = Arc::new(AtomicU64::new(0));
    let mut cache = Tiles::new(ids.clone());
    let original = plan(0.0, 0.0);
    fill(&mut cache, &original);
    let retained = cache.get(original.visible[0]).unwrap();
    let amount = cache.bytes.clone();
    let pending = cache.reserve().unwrap();
    let allocated = amount.load(Ordering::Acquire);
    assert!(allocated > TILE_BYTES);
    drop(cache);
    assert_eq!(amount.load(Ordering::Acquire), 2 * TILE_BYTES);
    drop(pending);
    assert_eq!(amount.load(Ordering::Acquire), TILE_BYTES);
    drop(retained);
    assert_eq!(amount.load(Ordering::Acquire), 0);
    let replacement = Tiles::new(ids);
    let mut charges = Vec::new();
    while let Some(charge) = replacement.reserve() {
        charges.push(charge);
    }
    assert_eq!(charges.len() * TILE_BYTES, TOTAL_BYTES);
    assert!(replacement.reserve().is_none());
    charges.pop();
    assert!(replacement.reserve().is_some());
}

#[test]
fn scale_page_and_nonreused_ids_prevent_stale_texture_mixing() {
    let ids = Arc::new(AtomicU64::new(0));
    let mut cache = Tiles::new(ids.clone());
    let original = plan(0.0, 0.0);
    fill(&mut cache, &original);
    let first = cache.get(original.visible[0]).unwrap().0;
    for changed in [
        Plan::new(1, 1.0, [0.0; 2], [900, 600], [0.0; 2]),
        Plan::new(0, 1.25, [0.0; 2], [900, 600], [0.0; 2]),
    ] {
        assert!(!cache.contains(changed.visible[0]));
    }
    drop(cache);
    let mut next = Tiles::new(ids);
    fill(&mut next, &original);
    assert_ne!(next.get(original.visible[0]).unwrap().0, first);
}

#[test]
fn scanning_many_viewports_evicts_old_tiles_without_losing_the_current_visible_set() {
    let mut cache = Tiles::new(Arc::new(AtomicU64::new(0)));
    let first = plan(0.0, 0.0);
    fill(&mut cache, &first);
    let old_id = cache.get(first.visible[0]).unwrap().0;
    for step in 1..200 {
        let current = plan(f64::from(step * EDGE), 0.0);
        fill(&mut cache, &current);
        assert!(current.visible.iter().all(|key| cache.contains(*key)));
        assert!(cache.entries.len() <= CACHE_ENTRIES);
        assert!(cache.bytes.load(Ordering::Acquire) <= CACHE_BYTES);
    }
    assert!(!cache.contains(first.visible[0]));
    fill(&mut cache, &first);
    assert_ne!(cache.get(first.visible[0]).unwrap().0, old_id);
}
