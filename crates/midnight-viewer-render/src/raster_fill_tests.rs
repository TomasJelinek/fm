use super::*;

// Preserve the original scalar coverage/compositing as a pixel oracle. The fast
// path must match every byte, including clipped arcs and translucent edge pixels.
fn scalar_reference(raster: &mut Raster, fill: &Fill, origin: [f32; 2]) {
    let x0 = stable_pixel(fill.rect.x0 - f64::from(origin[0]));
    let x1 = stable_pixel(fill.rect.x1 - f64::from(origin[0]));
    let y0 = stable_pixel(fill.rect.y0 - f64::from(origin[1]));
    let y1 = stable_pixel(fill.rect.y1 - f64::from(origin[1]));
    let radius = f64::from(fill.radius)
        .min((x1 - x0) / 2.0)
        .min((y1 - y0) / 2.0);
    let color = premultiply(fill.color);
    for y in y0.floor().max(0.0) as usize..y1.ceil().min(f64::from(raster.height)).max(0.0) as usize
    {
        for x in
            x0.floor().max(0.0) as usize..x1.ceil().min(f64::from(raster.width)).max(0.0) as usize
        {
            let coverage = if fill.radius <= 0.0 {
                1.0
            } else {
                let px = x as f64 + 0.5;
                let py = y as f64 + 0.5;
                let dx = (x0 + radius - px).max(0.0).max(px - x1 + radius);
                let dy = (y0 + radius - py).max(0.0).max(py - y1 + radius);
                (radius + 0.5 - dx.hypot(dy)).clamp(0.0, 1.0)
            };
            let color = color.map(|channel| (f64::from(channel) * coverage).round() as u8);
            let at = (y * raster.width as usize + x) * 4;
            for (target, source) in raster.rgba[at..at + 4].iter_mut().zip(color) {
                *target = (u16::from(source)
                    + (u16::from(*target) * (255 - u16::from(color[3])) + 127) / 255)
                    .min(255) as u8;
            }
        }
    }
}

#[test]
fn batched_backgrounds_are_pixel_identical_at_fractional_and_clipped_edges() {
    for rect in [
        Rect {
            x0: -4.25,
            y0: -1.75,
            x1: 31.5,
            y1: 25.25,
        },
        Rect {
            x0: 2.125,
            y0: 3.375,
            x1: 27.875,
            y1: 20.625,
        },
        Rect {
            x0: 10.0,
            y0: 9.0,
            x1: 10.25,
            y1: 9.75,
        },
        Rect {
            x0: -40.0,
            y0: -40.0,
            x1: -20.0,
            y1: -20.0,
        },
    ] {
        for radius in [0.0, 0.01, 2.5, 8.0] {
            for origin in [[0.0, 0.0], [0.25, 0.125], [-0.75, 1.625]] {
                for alpha in [0, 1, 77, 128, 254, 255] {
                    let fill = Fill {
                        rect,
                        radius,
                        color: [32, 89, 170, alpha],
                    };
                    let mut actual = Raster {
                        width: 32,
                        height: 24,
                        rgba: [11, 23, 47, 101].repeat(32 * 24),
                    };
                    let mut expected = Raster {
                        width: 32,
                        height: 24,
                        rgba: actual.rgba.clone(),
                    };
                    actual.fill_rounded(&fill, origin);
                    scalar_reference(&mut expected, &fill, origin);
                    assert_eq!(
                        actual.rgba, expected.rgba,
                        "radius={radius} origin={origin:?} alpha={alpha}"
                    );
                }
            }
        }
    }
}
