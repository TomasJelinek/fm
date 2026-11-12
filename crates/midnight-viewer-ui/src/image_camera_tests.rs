use super::*;

fn info() -> ViewerImageInfo {
    ViewerImageInfo {
        surface: ViewerSurfaceId(3),
        origin: [1024.0; 2],
        visible_source: [900.0, 600.0],
        raster_scale: 1.0,
        zoom: 1.0,
        width: 6000,
        height: 4000,
        fitted: false,
        page: 0,
        page_count: 1,
        pan_sequence: 0,
    }
}
fn grid(info: &ViewerImageInfo, origin: [f64; 2]) -> HashSet<[i32; 2]> {
    let (low, high) = grid_bounds(origin, info);
    ((low[1] - 1)..=high[1])
        .flat_map(|y| ((low[0] - 1)..=high[0]).map(move |x| [x, y]))
        .collect()
}

#[test]
fn retained_pixels_move_before_any_backend_reply_and_late_ack_never_snaps_them_back() {
    let mut camera = Camera::default();
    let info = info();
    camera.receive(&info, 1.0, grid(&info, info.origin));
    camera.push(12.25, 18.75, Duration::ZERO, true);
    assert!(camera.tick(Duration::ZERO));
    assert_eq!(camera.origin, [1036.25, 1042.75]);
    let (_, sequence, _) = camera.request().unwrap();
    camera.push(5.0, 7.0, Duration::from_millis(1), true);
    camera.tick(Duration::from_millis(1));
    let mut old_ack = info.clone();
    old_ack.pan_sequence = sequence;
    old_ack.origin = [1036.25, 1042.75];
    camera.receive(&old_ack, 1.0, grid(&info, info.origin));
    assert_eq!(camera.origin, [1041.25, 1049.75]);
}

#[test]
fn wheel_has_fractional_coast_immediate_reversal_and_waits_for_missing_tiles() {
    let mut camera = Camera::default();
    let info = info();
    camera.receive(&info, 1.0, grid(&info, info.origin));
    camera.push(0.0, 120.0, Duration::ZERO, false);
    let mut positions = Vec::new();
    for frame in 1..=25 {
        camera.tick(Duration::from_millis(frame * 8));
        positions.push(camera.origin[1]);
    }
    assert!(
        positions
            .windows(2)
            .filter(|pair| pair[1] > pair[0])
            .count()
            >= 20
    );
    assert!((camera.origin[1] - 1144.0).abs() < 0.001);
    camera.push(0.0, 120.0, Duration::from_millis(201), false);
    camera.tick(Duration::from_millis(209));
    let before = camera.origin[1];
    camera.push(0.0, -60.0, Duration::from_millis(210), false);
    camera.tick(Duration::from_millis(218));
    assert!(camera.origin[1] < before);
    camera.push(0.0, 1200.0, Duration::from_millis(220), true);
    camera.tick(Duration::from_millis(220));
    assert!(camera.active());
    let paused = camera.origin;
    camera.tick(Duration::from_millis(300));
    assert_eq!(camera.origin, paused);
    let (_, _, requested) = camera.request().unwrap();
    assert!(requested[1] - paused[1] <= 256.0);
    let mut more = grid(&info, info.origin);
    more.extend(grid(&info, requested));
    camera.receive(&info, 1.0, more);
    assert!(camera.tick(Duration::from_millis(308)));
}

#[test]
fn scrollbar_keeps_latest_absolute_thumb_and_cached_seek_is_immediate() {
    let mut camera = Camera::default();
    let info = info();
    camera.receive(&info, 1.0, grid(&info, info.origin));
    camera.seek(1, 0.75);
    let first = camera.request().unwrap();
    camera.seek(1, 0.9);
    let latest = camera.request().unwrap();
    assert!(latest.1 > first.1);
    assert_eq!(camera.position(1), 0.9);
    let mut stale = info.clone();
    stale.origin = first.2;
    stale.pan_sequence = first.1;
    let mut retained = grid(&info, info.origin);
    retained.extend(grid(&info, first.2));
    camera.receive(&stale, 1.0, retained);
    assert_eq!(camera.position(1), 0.9);
    assert_eq!(camera.origin, info.origin);
    let mut final_info = info.clone();
    final_info.origin = latest.2;
    final_info.pan_sequence = latest.1;
    camera.receive(&final_info, 1.0, grid(&info, latest.2));
    assert_eq!(camera.origin[1], 3060.0);
    camera.seek(1, 0.89);
    assert_eq!(camera.origin[1], 3026.0);
}

#[test]
fn boundaries_resize_and_file_switch_cancel_stale_motion_without_reusing_sequences() {
    let mut camera = Camera::default();
    let mut info = info();
    info.origin = [0.0; 2];
    camera.receive(&info, 1.0, grid(&info, info.origin));
    camera.push(-120.0, -120.0, Duration::ZERO, false);
    camera.tick(Duration::from_millis(16));
    assert!(!camera.active());
    assert_eq!(camera.origin, [0.0; 2]);
    let first = camera.request().unwrap();
    camera.suspend();
    assert!(!camera.push(0.0, 20.0, Duration::ZERO, true));
    info.surface = ViewerSurfaceId(40);
    camera.receive(&info, 2.0, grid(&info, info.origin));
    camera.push(0.0, 10.0, Duration::ZERO, true);
    camera.tick(Duration::ZERO);
    assert_eq!(camera.origin[1], 20.0);
    assert!(camera.request().unwrap().1 > first.1);
}
