//! Path omissions are explicit while native targets and complete values survive.
use crate::support::modeless as ui;
use i_slint_core::{
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
};
use slint::{
    ComponentHandle, Model, ModelRc, VecModel,
    platform::{PointerEventButton, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

slint::slint! {
    // Exported so that the scheme is not a compile-time constant, which would copy the
    // whole palette into every color binding of the fixture.
    import { Theme } from "../ui/theme.slint";
    export { Theme }
    import { PathNavigation } from "../ui/path-navigation.slint";
    import { PathPartRow } from "../ui/navigation-models.slint";
    import { PathText } from "../ui/path-text.slint";
    import { CompactPicker, TipBody, TooltipPlacement } from "../ui/controls.slint";
    export { PathPartRow, TooltipPlacement }
    export component TipFixture inherits Window {
        preferred-width: 700px; preferred-height: 400px;
        in property <string> text;
        in property <length> limit: 560px;
        out property <length> body-width: body.width;
        out property <length> body-height: body.height;
        body := TipBody { text: root.text; width-limit: root.limit; }
    }
    export component PathFixture inherits Window {
        preferred-width: 320px; preferred-height: 80px;
        in property <string> path;
        in property <[PathPartRow]> parts;
        out property <string> selected;
        out property <string> copied;
        out property <string> edit-text: navigation.text;
        // The path editor owns keyboard focus exactly while the path is being edited.
        out property <bool> editing: navigation.has-focus;
        navigation := PathNavigation {
            x: 12px; y: 12px; width: parent.width - 24px; height: 32px;
            path: root.path; parts: root.parts;
            selected-part(identity) => { root.selected = identity; }
            copy-part(identity) => { root.copied = identity; }
        }
    }
    import { NameText } from "../ui/path-text.slint";
    export component NameDirectionFixture inherits Window {
        preferred-width: 400px; preferred-height: 60px;
        background: white;
        in property <string> name;
        in property <bool> isolated;
        Text {
            x: 10px; y: 10px; font-size: 24px; color: black;
            text: root.isolated ? NameText.ltr(root.name) : root.name;
        }
    }
    export component PathLabelFixture inherits Window {
        preferred-width: 320px; preferred-height: 80px;
        in property <string> path;
        in property <string> tail;
        in property <length> text-size: 16px;
        label := PathText {
            x: 12px; y: 12px; width: parent.width - 24px; height: 48px;
            text: root.path; tail: root.tail; font-size: root.text-size;
        }
    }
    export component PathPickerFixture inherits Window {
        preferred-width: 320px; preferred-height: 200px;
        in property <[string]> paths;
        out property <int> selected: -1;
        public function open() { picker.open(); }
        picker := CompactPicker {
            x: 12px; y: 12px; width: 100px;
            path-labels: true; model: root.paths; label: root.paths[0];
            accessible-name: "Saved paths";
            popup-width: 280px;
            selected(index) => { root.selected = index; }
        }
    }
}

#[test]
fn shortened_path_keeps_the_root_clickable_and_edits_the_unshortened_path() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = PathFixture::new().unwrap();
    let labels = [
        "C:\\",
        "Users\\",
        "Owner\\",
        "Documents\\",
        "long-project-directory\\",
        "日本語\\",
        "Sources",
    ];
    let mut prefix = String::new();
    let rows = labels
        .iter()
        .enumerate()
        .map(|(index, label)| {
            prefix.push_str(label);
            PathPartRow {
                identity: format!("native-{index}").into(),
                label: (*label).into(),
                prefix: prefix.as_str().into(),
            }
        })
        .collect::<Vec<_>>();
    window.set_path(prefix.as_str().into());
    window.set_parts(ModelRc::new(VecModel::from(rows)));
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 320, 80, "path-middle-elision");
    let marker = ui::accessible_item(&native, AccessibleRole::Button, "Edit full folder path")
        .expect("the shortened breadcrumb exposes its omission marker");
    let dots = visible_text(marker, "...")
        .expect("the omitted path must be marked by three visible literal dots");
    let bounds = dots.geometry();
    let origin = dots.map_to_native_window(bounds.origin);
    assert!(origin.x >= 12.0 && origin.x + bounds.size.width <= 308.0);
    assert_eq!(painted_dot_count(&native, origin, bounds.size), 3);
    fn visible_root(item: i_slint_core::item_tree::ItemRc) -> Option<slint::LogicalPosition> {
        if let Some(label) = item.accessible_string_property(
            i_slint_core::accessibility::AccessibleStringProperty::Label,
        ) && label == "Open C:\\"
        {
            let geometry = item.geometry();
            let origin = item.map_to_native_window(geometry.origin);
            if origin.x >= 12.0 {
                return Some(slint::LogicalPosition::new(
                    origin.x + geometry.size.width / 2.0,
                    origin.y + geometry.size.height / 2.0,
                ));
            }
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            if let Some(point) = visible_root(item) {
                return Some(point);
            }
        }
        None
    }
    let point = visible_root(i_slint_core::item_tree::ItemRc::new_root(
        i_slint_core::window::WindowInner::from_pub(window.window()).component(),
    ))
    .expect("visible leading root");
    for button in [PointerEventButton::Left, PointerEventButton::Right] {
        for event in [
            WindowEvent::PointerMoved { position: point },
            WindowEvent::PointerPressed {
                position: point,
                button,
            },
            WindowEvent::PointerReleased {
                position: point,
                button,
            },
        ] {
            window.window().dispatch_event(event);
        }
    }
    assert_eq!(window.get_selected(), "native-0");
    assert_eq!(window.get_copied(), "native-0");
    ui::click(&native, "Edit full folder path");
    assert!(window.get_editing());
    assert_eq!(window.get_edit_text(), prefix);
}

/// Window rectangle of an item: x, y, width, height.
fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    (origin.x, origin.y, rect.size.width, rect.size.height)
}

/// A visible breadcrumb button: label, window bounds, the width its caption needs and
/// the width the caption has.
type Crumb = (String, (f32, f32, f32, f32), f32, f32);

/// Visible breadcrumb buttons in tree order.
fn breadcrumb_buttons(window: &PathFixture, adapter: &Rc<dyn WindowAdapter>) -> Vec<Crumb> {
    use i_slint_core::items::Orientation;
    fn collect(item: ItemRc, found: &mut Vec<ItemRc>) {
        if !item.is_visible() {
            return;
        }
        if item.accessible_role() == AccessibleRole::Button {
            found.push(item.clone());
        }
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            collect(item, found);
        }
    }
    let mut found = Vec::new();
    collect(
        ItemRc::new_root(i_slint_core::window::WindowInner::from_pub(window.window()).component()),
        &mut found,
    );
    found
        .into_iter()
        .map(|item| {
            let label = item
                .accessible_string_property(
                    i_slint_core::accessibility::AccessibleStringProperty::Label,
                )
                .unwrap_or_default()
                .to_string();
            let mut caption = (0.0, f32::MAX);
            let mut child = item.first_child();
            while let Some(text) = child {
                child = text.next_sibling();
                if text.is_visible()
                    && (text.downcast::<SimpleText>().is_some()
                        || text.downcast::<ComplexText>().is_some())
                {
                    let needed = text
                        .borrow()
                        .as_ref()
                        .layout_info(Orientation::Horizontal, -1.0, adapter, &text)
                        .preferred;
                    caption = (needed, text.geometry().size.width);
                }
            }
            (label, bounds(&item), caption.0, caption.1)
        })
        .collect()
}

#[test]
fn collapsed_paths_show_whole_components_that_never_overlap() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = PathFixture::new().unwrap();
    let parts = |labels: &[&str]| {
        let mut prefix = String::new();
        labels
            .iter()
            .enumerate()
            .map(|(index, label)| {
                prefix.push_str(label);
                PathPartRow {
                    identity: format!("part-{index}").into(),
                    label: (*label).into(),
                    prefix: prefix.as_str().into(),
                }
            })
            .collect::<Vec<_>>()
    };
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    let adapter: Rc<dyn WindowAdapter> = native.clone();
    for (case, labels) in [
        (
            "deep",
            vec![
                "/",
                "workspace",
                "/_build",
                "/acceptance-ui-polish",
                "/20261120-011003",
                "/work",
                "/ui_review",
                "/review",
            ],
        ),
        (
            "long root",
            vec![
                "/a-very-long-mount-point-name-that-takes-most-of-the-bar",
                "/x",
                "/y",
                "/leaf",
            ],
        ),
        (
            "right-to-left",
            vec![
                "/",
                "srv",
                "/עברית",
                "/מסמכים",
                "/תיקייה-ארוכה",
                "/עברית.txt",
            ],
        ),
    ] {
        let rows = parts(&labels);
        let path = rows.last().unwrap().prefix.to_string();
        window.set_path(path.as_str().into());
        window.set_parts(ModelRc::new(VecModel::from(rows)));
        for width in [200, 260, 320] {
            ui::draw(&native, width, 80, "path-collapse");
            let case = format!("{case} at {width}px");
            let buttons = breadcrumb_buttons(&window, &adapter);
            let shown = buttons
                .iter()
                .filter(|(label, ..)| label.starts_with("Open "))
                .count();
            let marker = buttons
                .iter()
                .any(|(label, ..)| label == "Edit full folder path");
            assert_eq!(
                marker,
                shown < labels.len(),
                "{case}: the marker stands exactly for collapsed components ({shown} of {} shown)",
                labels.len()
            );
            if (case.starts_with("deep") || case.starts_with("long root")) && width <= 260 {
                assert!(marker, "{case}: this path cannot fit whole");
            }
            let last = format!("Open {path}");
            assert!(
                buttons.iter().any(|(label, ..)| *label == last),
                "{case}: the current folder must stay visible"
            );
            let bar = (12.0, 12.0 + width as f32 - 24.0);
            for (index, (label, (x, _, w, _), needs, has)) in buttons.iter().enumerate() {
                assert!(
                    *x >= bar.0 - 0.5 && x + w <= bar.1 + 0.5,
                    "{case}: {label} leaves the bar: {x}..{}",
                    x + w
                );
                if label.starts_with("Open ") && *label != last {
                    assert!(
                        needs <= &(has + 0.5),
                        "{case}: {label} is cut: needs {needs}px, has {has}px"
                    );
                }
                for (other, (ox, _, ow, _), ..) in &buttons[index + 1..] {
                    let overlap = (x + w).min(ox + ow) - x.max(*ox);
                    assert!(
                        overlap <= 0.5,
                        "{case}: {label} and {other} overlap by {overlap}px"
                    );
                }
            }
            // Leading components, the marker, then the trailing ones, left to right.
            let xs: Vec<f32> = buttons
                .iter()
                .filter(|(label, ..)| {
                    label.starts_with("Open ") || *label == "Edit full folder path"
                })
                .map(|(_, (x, ..), ..)| *x)
                .collect();
            assert!(
                xs.windows(2).all(|pair| pair[0] < pair[1]),
                "{case}: order {xs:?}"
            );
        }
    }
    // A path that fits shows every component and no marker.
    let rows = parts(&["/", "srv", "/data"]);
    window.set_path("/srv/data".into());
    window.set_parts(ModelRc::new(VecModel::from(rows)));
    ui::draw(&native, 320, 80, "path-uncollapsed");
    let buttons = breadcrumb_buttons(&window, &adapter);
    assert!(
        !buttons
            .iter()
            .any(|(label, ..)| label == "Edit full folder path")
    );
    for label in ["Open /", "Open /srv", "Open /srv/data"] {
        assert!(buttons.iter().any(|(shown, ..)| shown == label), "{label}");
    }
}

#[test]
fn right_to_left_names_keep_their_extension_at_the_end() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = NameDirectionFixture::new().unwrap();
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    let render = |name: &str, isolated: bool| {
        window.set_name(name.into());
        window.set_isolated(isolated);
        let mut pixels = vec![slint::Rgb8Pixel::default(); 400 * 60];
        native.set_size(slint::PhysicalSize::new(400, 60));
        window.window().request_redraw();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 400);
        });
        pixels
    };
    let columns = |pixels: &[slint::Rgb8Pixel]| -> Vec<Vec<(u8, u8, u8)>> {
        (0..400)
            .map(|x| {
                (0..60)
                    .map(|y| pixels[y * 400 + x])
                    .map(|p| (p.r, p.g, p.b))
                    .collect()
            })
            .collect()
    };
    let ink = |pixels: &[slint::Rgb8Pixel]| {
        let background = pixels[0];
        let columns = columns(pixels);
        let inked: Vec<usize> = (0..400)
            .filter(|&x| {
                columns[x]
                    .iter()
                    .any(|p| *p != (background.r, background.g, background.b))
            })
            .collect();
        (*inked.first().unwrap(), *inked.last().unwrap())
    };
    // The isolation marks themselves draw nothing.
    assert_eq!(render("plain.txt", true), render("plain.txt", false));
    let word = render("עברית", true);
    let (start, end) = ink(&word);
    let named = render("עברית.txt", true);
    let (named_start, named_end) = ink(&named);
    assert_eq!(named_start, start, "the name starts where the word does");
    assert!(named_end > end + 10, "the extension follows the word");
    let (word_columns, named_columns) = (columns(&word), columns(&named));
    assert_eq!(
        word_columns[start..end.saturating_sub(1)],
        named_columns[start..end.saturating_sub(1)],
        "the word is drawn first, the extension after it"
    );
    // Without the marks the paragraph would be right-to-left: "txt." comes first.
    let bare = columns(&render("עברית.txt", false));
    assert_ne!(
        word_columns[start..end.saturating_sub(1)],
        bare[start..end.saturating_sub(1)]
    );
}

fn painted_dot_count(
    window: &slint::platform::software_renderer::MinimalSoftwareWindow,
    origin: i_slint_core::lengths::LogicalPoint,
    size: i_slint_core::lengths::LogicalSize,
) -> usize {
    let mut pixels = vec![slint::Rgb8Pixel::default(); 320 * 80];
    window.window().request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut pixels, 320);
    });
    let x0 = origin.x.ceil() as usize;
    let y0 = origin.y.ceil() as usize;
    let x1 = (origin.x + size.width).floor() as usize;
    let y1 = (origin.y + size.height).floor() as usize;
    let background = pixels[y0 * 320 + x0];
    let mut previous = false;
    let mut count = 0;
    for x in x0..x1 {
        let ink = (y0..y1).any(|y| {
            let pixel = pixels[y * 320 + x];
            pixel.r.abs_diff(background.r) > 40
                || pixel.g.abs_diff(background.g) > 40
                || pixel.b.abs_diff(background.b) > 40
        });
        if ink && !previous {
            count += 1;
        }
        previous = ink;
    }
    count
}

#[test]
fn find_progress_and_result_paths_mark_omissions_without_changing_their_values() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = midnight_ui::FindWindow::new().unwrap();
    let directory = format!("C:\\{}\\文档", "long-directory\\".repeat(30));
    let path = format!("{directory}\\report.txt");
    window.set_current_directory(directory.as_str().into());
    window.set_rows(ModelRc::new(VecModel::from(vec![midnight_ui::FindRow {
        token: "captured-result".into(),
        name: "report.txt".into(),
        extension: "txt".into(),
        path: path.as_str().into(),
        size: "1 B".into(),
        modified: "2026-11-08".into(),
        directory: false,
        ..Default::default()
    }])));
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 860, 620, "find-path-elision");
    let progress = ui::accessible_item(&native, AccessibleRole::Text, &directory)
        .expect("the complete progress path remains accessible");
    assert!(visible_text(progress, "...").is_some());
    let row = ui::accessible_item(
        &native,
        AccessibleRole::ListItem,
        &format!("report.txt · txt · {path} · 1 B · 2026-11-08"),
    )
    .expect("the captured result retains its complete path");
    assert!(visible_text(row, "...").is_some());
    assert_eq!(window.get_current_directory(), directory);
    assert_eq!(window.get_rows().row_data(0).unwrap().path, path);

    window.set_current_directory("C:\\short".into());
    ui::draw(&native, 860, 620, "find-path-unshortened");
    let progress = ui::accessible_item(&native, AccessibleRole::Text, "C:\\short").unwrap();
    assert!(visible_text(progress, "...").is_none());
}

#[test]
fn path_markers_survive_narrow_widths_and_font_changes_and_disappear_when_unneeded() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = PathLabelFixture::new().unwrap();
    let path = format!("C:\\{}\\👩🏽‍💻.txt", "文档 e\u{301}\\".repeat(20));
    window.set_path(path.as_str().into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for font in [16.0, 30.0] {
        window.set_text_size(font);
        for width in [64, 120, 320] {
            ui::draw(&native, width, 80, "path-marker-narrow");
            let label = ui::accessible_item(&native, AccessibleRole::Text, &path).unwrap();
            let dots = visible_text(label, "...").expect("narrow paths retain the entire marker");
            let bounds = dots.geometry();
            let origin = dots.map_to_native_window(bounds.origin);
            assert!(origin.x >= 12.0 && origin.x + bounds.size.width <= width as f32 - 12.0);
            assert_eq!(window.get_path(), path);
        }
    }
    window.set_path("C:\\短".into());
    ui::draw(&native, 320, 80, "path-marker-unneeded");
    let label = ui::accessible_item(&native, AccessibleRole::Text, "C:\\短").unwrap();
    assert!(visible_text(label, "...").is_none());
}

#[test]
fn path_labels_with_a_tail_omit_the_middle_and_keep_the_last_component() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = PathLabelFixture::new().unwrap();
    let parent = format!("C:\\{}", "shared parent\\".repeat(12));
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for leaf in ["alpha 文档", "beta 文档"] {
        let path = format!("{parent}{leaf}");
        let tail = format!("\\{leaf}");
        window.set_path(path.as_str().into());
        window.set_tail(tail.as_str().into());
        ui::draw(&native, 320, 80, "path-marker-middle");
        let label = ui::accessible_item(&native, AccessibleRole::Text, &path).unwrap();
        let marker = visible_texts(label.clone(), "...");
        let shown = visible_texts(label.clone(), &tail);
        assert_eq!(
            (marker.len(), shown.len()),
            (1, 1),
            "{leaf}: one marker, whole tail"
        );
        let (x, _, width, _) = bounds(&label);
        let (marker_x, _, marker_width, _) = bounds(&marker[0]);
        let (tail_x, _, tail_width, _) = bounds(&shown[0]);
        assert!(
            x < marker_x && marker_x + marker_width <= tail_x + 0.5,
            "the marker stands for the middle, between the start and {leaf}"
        );
        assert!(
            tail_x + tail_width <= x + width + 0.5,
            "{leaf} ends inside the label"
        );
        assert_eq!(window.get_path(), path);
    }

    // A tail wider than the label is cut at its end too, after a leading marker.
    let path = format!("{parent}{}", "long-leaf".repeat(12));
    let tail = format!("\\{}", "long-leaf".repeat(12));
    window.set_path(path.as_str().into());
    window.set_tail(tail.as_str().into());
    ui::draw(&native, 160, 80, "path-marker-cut-tail");
    let label = ui::accessible_item(&native, AccessibleRole::Text, &path).unwrap();
    let markers = visible_texts(label.clone(), "...");
    assert_eq!(markers.len(), 2);
    let shown = visible_texts(label.clone(), &tail);
    assert_eq!(shown.len(), 1);
    let (x, _, width, _) = bounds(&label);
    let (tail_x, _, tail_width, _) = bounds(&shown[0]);
    assert!(bounds(&markers[0]).0 < tail_x && tail_x + tail_width <= bounds(&markers[1]).0 + 0.5);
    for marker in &markers {
        let (marker_x, _, marker_width, _) = bounds(marker);
        assert!(marker_x >= x - 0.5 && marker_x + marker_width <= x + width + 0.5);
    }

    // A label that fits shows no marker; a tail that is not the end is ignored.
    window.set_path("C:\\短\\alpha".into());
    window.set_tail("\\alpha".into());
    ui::draw(&native, 320, 80, "path-marker-middle-unneeded");
    let label = ui::accessible_item(&native, AccessibleRole::Text, "C:\\短\\alpha").unwrap();
    assert!(visible_texts(label.clone(), "...").is_empty());
    assert!(visible_texts(label, "\\alpha").is_empty());
    window.set_path(path.as_str().into());
    window.set_tail("\\elsewhere".into());
    ui::draw(&native, 320, 80, "path-marker-foreign-tail");
    let label = ui::accessible_item(&native, AccessibleRole::Text, &path).unwrap();
    let marker = visible_texts(label.clone(), "...");
    assert_eq!(marker.len(), 1);
    let (x, _, width, _) = bounds(&label);
    let (marker_x, _, marker_width, _) = bounds(&marker[0]);
    assert!(
        (marker_x + marker_width - (x + width)).abs() <= 0.5,
        "the end is omitted"
    );
}

#[test]
fn tooltips_wrap_between_words_and_break_only_tokens_wider_than_a_line() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = TipFixture::new().unwrap();
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    // The tooltip frame's width and height for `text` under a width limit.
    let measure = |text: &str, limit: f32| {
        window.set_text(text.into());
        window.set_limit(limit);
        ui::draw(&native, 700, 400, "tooltip-wrap");
        (window.get_body_width(), window.get_body_height())
    };
    let lines = |height: f32, line: f32| ((height - 16.0) / line).round();
    let word = "tooltipword";
    let line = measure(word, 560.0).1 - 16.0;
    let two_words = measure(&format!("{word} {word}"), 560.0).0 - 20.0;
    assert!(
        two_words + 20.0 > 120.0,
        "two words are measured, not the minimum frame"
    );
    // A line slightly narrower than two words: word wrapping gives each word a line of
    // its own, where character wrapping would fill two lines and split the second word.
    let (_, height) = measure(&format!("{word} {word} {word}"), two_words - 2.0 + 20.0);
    assert_eq!(lines(height, line), 3.0, "no word is split across lines");

    // A path wider than the line breaks where it must instead of running out of the frame.
    let path = format!("/srv/{}", "long-folder-name/".repeat(20));
    let (width, height) = measure(&path, 200.0);
    assert!(width <= 200.0 + 1e-3);
    assert!(lines(height, line) >= 5.0, "the long path wraps: {height}");

    // A frame rounded up to whole device pixels still holds a one-line hint.
    window.global::<TooltipPlacement>().set_scale(1.25);
    for text in [
        "Open the folder in a new tab",
        "Copy the full path",
        "Rename the selected file",
    ] {
        let (width, height) = measure(text, 560.0);
        let device = width * 1.25;
        assert!((device - device.round()).abs() < 1e-3, "{text}: {width}");
        assert_eq!(lines(height, line), 1.0, "{text} stays on one line");
    }
}

#[test]
fn path_picker_marks_omissions_and_selects_the_original_full_value() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = PathPickerFixture::new().unwrap();
    let path = format!("C:\\{}\\文档", "long-directory\\".repeat(20));
    window.set_paths(ModelRc::new(VecModel::from(vec![path.as_str().into()])));
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 320, 200, "path-picker-closed");
    let picker = ui::accessible_item(&native, AccessibleRole::Combobox, "Saved paths").unwrap();
    assert!(visible_text(picker, "...").is_some());
    window.invoke_open();
    ui::draw(&native, 320, 200, "path-picker-open");
    let choice = ui::accessible_popup_item(&native, AccessibleRole::Button, &path).unwrap();
    assert!(visible_text(choice, "...").is_some());
    ui::click(&native, &path);
    assert_eq!(window.get_selected(), 0);
    assert_eq!(window.get_paths().row_data(0).unwrap(), path);
}

#[test]
fn find_root_picker_marks_omissions_and_removes_the_selected_original_root() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = midnight_ui::FindWindow::new().unwrap();
    let paths = [
        format!("C:\\{}\\first", "文档-long\\".repeat(30)),
        format!("D:\\{}\\second", "other-long\\".repeat(30)),
    ];
    let roots = ModelRc::new(VecModel::from(
        paths
            .iter()
            .map(|path| midnight_ui::FindRootRow {
                label: path.as_str().into(),
                searched: true,
            })
            .collect::<Vec<_>>(),
    ));
    window.set_roots(roots);
    let removed = Rc::new(RefCell::new(Vec::new()));
    let captured = removed.clone();
    window.on_remove_root(move |index| captured.borrow_mut().push(index));
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 860, 620, "find-root-picker-closed");
    let picker = ui::accessible_item(&native, AccessibleRole::Combobox, "Search folders").unwrap();
    assert!(visible_text(picker.clone(), "...").is_some());
    let bounds = picker.geometry();
    let origin = picker.map_to_native_window(bounds.origin);
    let position = slint::LogicalPosition::new(
        origin.x + bounds.size.width / 2.0,
        origin.y + bounds.size.height / 2.0,
    );
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
        native.window().dispatch_event(event);
    }
    ui::draw(&native, 860, 620, "find-root-picker-open");
    let choice = ui::accessible_popup_item(&native, AccessibleRole::Button, &paths[1]).unwrap();
    assert!(visible_text(choice, "...").is_some());
    ui::click(&native, &paths[1]);
    ui::click(&native, "Remove");
    assert_eq!(removed.borrow().as_slice(), [1]);
    assert_eq!(window.get_roots().row_data(1).unwrap().label, paths[1]);

    window.set_roots(ModelRc::new(VecModel::from(vec![
        midnight_ui::FindRootRow {
            label: paths[0].as_str().into(),
            searched: true,
        },
    ])));
    ui::draw(&native, 860, 620, "find-root-picker-shrunk");
    ui::click(&native, "Remove");
    assert_eq!(removed.borrow().as_slice(), [1, 0]);
    window.set_running(true);
    ui::draw(&native, 860, 620, "find-root-picker-running");
    ui::click(&native, "Remove");
    assert_eq!(removed.borrow().as_slice(), [1, 0]);
}

/// Every visible text item below `item` that shows exactly `expected`, in tree order.
fn visible_texts(item: ItemRc, expected: &str) -> Vec<ItemRc> {
    let mut found = Vec::new();
    let mut pending = vec![item];
    while let Some(item) = pending.pop() {
        if !item.is_visible() {
            continue;
        }
        if item
            .downcast::<SimpleText>()
            .is_some_and(|text| text.as_pin_ref().text() == expected)
            || item
                .downcast::<ComplexText>()
                .is_some_and(|text| text.as_pin_ref().text() == expected)
        {
            found.push(item.clone());
        }
        let mut children = Vec::new();
        let mut child = item.first_child();
        while let Some(item) = child {
            child = item.next_sibling();
            children.push(item);
        }
        pending.extend(children.into_iter().rev());
    }
    found
}

fn visible_text(item: ItemRc, expected: &str) -> Option<ItemRc> {
    if item.is_visible()
        && (item
            .downcast::<SimpleText>()
            .is_some_and(|text| text.as_pin_ref().text() == expected)
            || item
                .downcast::<ComplexText>()
                .is_some_and(|text| text.as_pin_ref().text() == expected))
    {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(found) = visible_text(item, expected) {
            return Some(found);
        }
    }
    None
}
