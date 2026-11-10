//! Product rows retain colored Unicode prefixes, compact compression marks and normal input.
use crate::ui;
use i_slint_core::items::AccessibleRole;
use midnight_ui::{FileRow, MainWindow, Theme};
use slint::{
    ComponentHandle, Image, Model, ModelRc, Rgb8Pixel, SharedPixelBuffer, VecModel,
    platform::{
        Platform, PlatformError, PointerEventButton, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::{cell::RefCell, rc::Rc};

const WIDTH: usize = 1340;
const HEIGHT: usize = 820;
const NATIVE: [u8; 3] = [68, 113, 192];

fn name_end(window: &MainWindow) -> usize {
    let size = ui::root_item(window, "Sort by Size", AccessibleRole::Button);
    let origin = size.map_to_native_window(size.geometry().origin);
    (origin.x - 8.0).max(0.0) as usize
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn close(pixel: Rgb8Pixel, color: slint::Color) -> bool {
    pixel.r.abs_diff(color.red()) <= 18
        && pixel.g.abs_diff(color.green()) <= 18
        && pixel.b.abs_diff(color.blue()) <= 18
}

// Small glyphs may have no fully opaque pixels. Distinguish their antialiased
// match-color ink from ordinary text, rather than requiring a solid-color core.
fn prefix_ink(
    pixel: Rgb8Pixel,
    matched: slint::Color,
    normal: slint::Color,
    bg: slint::Color,
) -> bool {
    let channels = |color: slint::Color| [color.red(), color.green(), color.blue()].map(f32::from);
    let observed = [pixel.r, pixel.g, pixel.b].map(f32::from);
    let background = channels(bg);
    let distance = |foreground: slint::Color| {
        let delta = std::array::from_fn::<_, 3, _>(|i| channels(foreground)[i] - background[i]);
        let magnitude: f32 = delta.iter().map(|channel| channel * channel).sum();
        let alpha = (0..3)
            .map(|i| (observed[i] - background[i]) * delta[i])
            .sum::<f32>()
            / magnitude.max(1.0);
        let distance = (0..3)
            .map(|i| (observed[i] - background[i] - alpha.clamp(0.0, 1.0) * delta[i]).powi(2))
            .sum::<f32>();
        (alpha, distance)
    };
    let (coverage, matched_distance) = distance(matched);
    let (_, normal_distance) = distance(normal);
    (0.25..=1.01).contains(&coverage)
        && matched_distance <= 108.0
        && normal_distance > matched_distance + 36.0
}

#[test]
fn product_prefix_color_and_compression_badge_survive_themes_density_fonts_and_row_input() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    midnight_ui::install_file_keys(&window);
    let bytes: Vec<_> = (0..16 * 16).flat_map(|_| NATIVE).collect();
    let icon = Image::from_rgb8(SharedPixelBuffer::<Rgb8Pixel>::clone_from_slice(
        &bytes, 16, 16,
    ));
    let names = [
        ("office Straße", 3, true),
        (
            "שלום-文档-office-long-file-name-with-enough-letters-to-exceed-the-entire-file-column-without-any-manual-string-splitting.txt",
            "שלום".len(),
            false,
        ),
        (
            "e\u{301}-👩🏽‍💻-long-name-that-must-not-wrap-across-rows.txt",
            3,
            false,
        ),
        ("<u>*&[name](https://example.com)", 3, false),
    ];
    window.set_left_rows(ModelRc::new(VecModel::from(
        names
            .into_iter()
            .enumerate()
            .map(|(row, (name, end, compressed))| FileRow {
                identity: row.to_string().into(),
                name: name.into(),
                prefix_end: end as i32,
                compressed,
                icon: icon.clone(),
                cursor: row == 0,
                selected: row == 1,
                ..Default::default()
            })
            .collect::<Vec<_>>(),
    )));
    window.set_left_path("/prefix-render 文档".into());
    let clicked = Rc::new(RefCell::new(Vec::new()));
    let captured = clicked.clone();
    window.on_row_click(move |pane, row, _, _| captured.borrow_mut().push((pane, row)));
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32),
    });
    let theme = window.global::<Theme>();
    let mut pixels = vec![Rgb8Pixel::default(); WIDTH * HEIGHT];
    for scheme in 0..3 {
        for (density, font) in [(0, 11), (1, 15), (0, 32), (1, 32)] {
            window.invoke_apply_fonts("".into(), font, font);
            window.invoke_apply_preferences(scheme, density);
            native.draw_if_needed(|renderer| {
                renderer.render(&mut pixels, WIDTH);
            });
            let top = window.get_left_list_top();
            let start = (theme.get_window_padding() + 1.0 + 12.0) as usize;
            let text_start = (start as f32 + theme.get_icon_size() + 6.0) as usize;
            for row in 0..4 {
                let y1 = (top + row as f32 * theme.get_row_height()) as usize;
                let y2 = (top + (row + 1) as f32 * theme.get_row_height()) as usize;
                let background = match row {
                    0 => theme.get_cursor_background(),
                    1 => theme.get_selected_background(),
                    2 => theme.get_pane_background(),
                    _ => theme.get_alternate_row(),
                };
                let normal = if row == 1 {
                    theme.get_selected_text()
                } else {
                    theme.get_text()
                };
                let highlighted = (y1..y2)
                    .flat_map(|y| (text_start..name_end(&window)).map(move |x| y * WIDTH + x))
                    .filter(|index| {
                        prefix_ink(pixels[*index], theme.get_search_match(), normal, background)
                    })
                    .count();
                assert!(
                    highlighted > 2,
                    "actual prefix glyphs need their own color: scheme={scheme},density={density},font={font},row={row},pixels={highlighted}"
                );
                let icon_end = start + theme.get_icon_size() as usize + 2;
                let marks = (y1..y2)
                    .flat_map(|y| (start..icon_end).map(move |x| y * WIDTH + x))
                    .filter(|index| close(pixels[*index], theme.get_compression_mark()))
                    .count();
                if row == 0 {
                    assert!(marks > 0, "compressed metadata must have a visible mark");
                    let host = (y1..y2)
                        .flat_map(|y| (start..icon_end).map(move |x| y * WIDTH + x))
                        .filter(|index| {
                            let p = pixels[*index];
                            [p.r, p.g, p.b] == NATIVE
                        })
                        .count();
                    assert!(
                        host > 15,
                        "the mark must preserve the remaining host-icon RGB"
                    );
                } else {
                    assert_eq!(
                        marks, 0,
                        "ordinary/unknown metadata must not invent a compression mark"
                    );
                }
            }
            let row = 1;
            let position = slint::LogicalPosition::new(
                text_start as f32 + 10.0,
                top + (row as f32 + 0.5) * theme.get_row_height(),
            );
            let count = clicked.borrow().len();
            window
                .window()
                .dispatch_event(WindowEvent::PointerMoved { position });
            for event in [
                WindowEvent::PointerPressed {
                    position,
                    button: PointerEventButton::Left,
                },
                WindowEvent::PointerReleased {
                    position,
                    button: PointerEventButton::Left,
                },
            ] {
                window.window().dispatch_event(event);
            }
            assert_eq!(
                clicked.borrow().get(count),
                Some(&(0, row)),
                "colored glyphs must pass file-row clicks through"
            );
            assert_eq!(window.get_focused_pane(), 0);
        }
    }
    // Changing the layout must retain shaped prefix feedback. The long bidi
    // filename must wrap inside a thumbnail label instead of becoming one
    // unbounded line or being split into independently shaped substrings.
    for mode in [1, 2] {
        window.set_left_view_mode(mode);
        for scheme in 0..3 {
            for (density, font) in [(0, 11), (1, 15), (0, 32)] {
                window.invoke_apply_fonts("".into(), font, font);
                window.invoke_apply_preferences(scheme, density);
                native.draw_if_needed(|renderer| {
                    renderer.render(&mut pixels, WIDTH);
                });
                for (row, (name, _, compressed)) in names.iter().enumerate() {
                    let label = format!(
                        "{name}{}",
                        if *compressed {
                            " · Filesystem compressed"
                        } else {
                            ""
                        }
                    );
                    let item = ui::root_item(&window, &label, AccessibleRole::ListItem);
                    let origin = item.map_to_native_window(item.geometry().origin);
                    let rect = item.geometry();
                    let x1 = (origin.x
                        + if mode == 2 {
                            10.0
                        } else {
                            18.0 + theme.get_icon_size()
                        }) as usize;
                    let x2 = (origin.x + rect.width() - 12.0) as usize;
                    let y1 = (origin.y + if mode == 2 { 90.0 } else { 0.0 }) as usize;
                    let y2 =
                        (origin.y + rect.height() - if mode == 2 { 8.0 } else { 0.0 }) as usize;
                    let background = match row {
                        0 => theme.get_cursor_background(),
                        1 => theme.get_selected_background(),
                        2 => theme.get_pane_background(),
                        _ => theme.get_alternate_row(),
                    };
                    let normal = if row == 1 {
                        theme.get_selected_text()
                    } else {
                        theme.get_text()
                    };
                    let highlighted = (y1..y2)
                        .flat_map(|y| (x1..x2).map(move |x| y * WIDTH + x))
                        .filter(|index| {
                            prefix_ink(pixels[*index], theme.get_search_match(), normal, background)
                        })
                        .count();
                    assert!(
                        highlighted > 2,
                        "mode={mode}, scheme={scheme}, density={density}, font={font}, row={row}: lost shaped prefix pixels"
                    );
                    if mode == 2 && row == 1 {
                        let ink_rows: Vec<_> = (y1..y2)
                            .filter(|y| {
                                (x1..x2).any(|x| {
                                    let pixel = pixels[y * WIDTH + x];
                                    u32::from(pixel.r.abs_diff(background.red()))
                                        + u32::from(pixel.g.abs_diff(background.green()))
                                        + u32::from(pixel.b.abs_diff(background.blue()))
                                        > 60
                                })
                            })
                            .collect();
                        assert!(
                            ink_rows.last().unwrap() - ink_rows[0] > font as usize * 3 / 2,
                            "thumbnail text must render multiple wrapped lines"
                        );
                    }
                }
            }
        }
    }
    window.set_left_view_mode(0);
    let rows = window.get_left_rows();
    let mut row = rows.row_data(0).unwrap();
    row.prefix_end = 0;
    rows.set_row_data(0, row);
    native.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, WIDTH);
    });
    let top = window.get_left_list_top();
    let start = (theme.get_window_padding() + 1.0 + 12.0 + theme.get_icon_size() + 6.0) as usize;
    let ordinary_prefix = (top as usize..(top + theme.get_row_height()) as usize)
        .flat_map(|y| (start..name_end(&window)).map(move |x| y * WIDTH + x))
        .filter(|index| {
            prefix_ink(
                pixels[*index],
                theme.get_search_match(),
                theme.get_text(),
                theme.get_cursor_background(),
            )
        })
        .count();
    assert_eq!(
        ordinary_prefix, 0,
        "ordinary glyphs must not satisfy the prefix-color criterion"
    );
    window.hide().unwrap();
}
