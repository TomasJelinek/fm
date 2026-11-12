use super::*;

#[test]
fn unequal_byte_density_refill_preserves_thumb_position_and_extent() {
    // The supplied build.bat has short commands beside long comments. A bounded
    // replacement changes page-average bytes/height without moving visible rows.
    let source: String = (0..120)
        .map(|row| {
            let suffix = if row < 30 {
                "x".repeat(160)
            } else {
                "x".repeat(12)
            };
            format!("rem row {row:03} {suffix}\n")
        })
        .collect();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    let (mut worker, output) = worker();
    worker.length = source.len() as u64;
    worker.input(ViewerInput::RetainedRaster(true), &output);
    assert!(worker.set_page(fragment(&source, 0, starts[60]), &output));
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    let target = [
        0.0,
        f64::from(worker.shaped.as_ref().unwrap().height())
            - f64::from(worker.geometry.height)
            - 32.375,
    ];
    pan(&mut worker, &output, info, 1, target, false);
    assert!(worker.raster.continuing.is_some());
    let before = paint(&mut worker, &output);
    assert!(worker.set_page(fragment(&source, starts[30], starts[90]), &output));
    worker.apply_raster_target(&output);
    let after = paint(&mut worker, &output);
    let old = before.raster.as_ref().unwrap();
    let new = after.raster.as_ref().unwrap();
    assert_eq!(old.surface, new.surface);
    assert!((old.origin[1] - new.origin[1]).abs() < 1e-4);
    assert!(
        (before.position - after.position).abs() < 1e-7,
        "same visible rows changed thumb position: {} -> {}",
        before.position,
        after.position
    );
    assert!(
        (before.vertical_extent - after.vertical_extent).abs() < 1e-7,
        "same visible rows changed thumb extent: {} -> {}",
        before.vertical_extent,
        after.vertical_extent
    );
}

#[test]
fn continuation_that_discovers_the_whole_file_keeps_source_coordinates() {
    let source: String = (0..100)
        .map(|row| {
            format!(
                "row {row}: {}\n",
                "x".repeat(if row < 30 { 100 } else { 10 })
            )
        })
        .collect();
    let starts: Vec<_> = std::iter::once(0)
        .chain(source.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    let (mut worker, output) = worker();
    worker.length = source.len() as u64;
    worker.input(ViewerInput::RetainedRaster(true), &output);
    assert!(worker.set_page(fragment(&source, 0, starts[60]), &output));
    let first = paint(&mut worker, &output);
    let info = first.raster.as_ref().unwrap();
    let target = [
        0.0,
        f64::from(worker.shaped.as_ref().unwrap().height())
            - f64::from(worker.geometry.height)
            - 32.375,
    ];
    pan(&mut worker, &output, info, 1, target, false);
    let before = paint(&mut worker, &output);
    assert!(worker.set_page(fragment(&source, 0, source.len()), &output));
    worker.apply_raster_target(&output);
    let after = paint(&mut worker, &output);
    assert_eq!(after.raster.as_ref().unwrap().surface, info.surface);
    assert!((before.position - after.position).abs() < 1e-7);
    assert!((before.vertical_extent - after.vertical_extent).abs() < 1e-7);
}
