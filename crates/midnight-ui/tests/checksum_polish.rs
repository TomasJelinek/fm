#[path = "support/modeless.rs"]
mod modeless;
use i_slint_core::{
    accessibility::{AccessibilityAction, AccessibleStringProperty},
    items::AccessibleRole,
};
use midnight_ui::{ChecksumDraft, ChecksumWindow};
use modeless::{Windows, accessible_item, click, draw};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent},
};
use std::{cell::RefCell, rc::Rc};

#[test]
fn verification_glyphs_are_green_and_red_in_light_and_dark_schemes() {
    use midnight_ui::ChecksumDisplayRow;
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.set_verifying(true);
    window.set_rows(
        Rc::new(slint::VecModel::from(vec![
            ChecksumDisplayRow {
                name: "matching.txt".into(),
                status: "Verified".into(),
                verified: true,
                ..Default::default()
            },
            ChecksumDisplayRow {
                name: "changed.txt".into(),
                status: "Mismatch".into(),
                error: true,
                ..Default::default()
            },
        ]))
        .into(),
    );
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for theme in [1, 2] {
        window.invoke_apply_preferences(theme, 0, "".into(), 13, 15);
        draw(&native, 860, 620, &format!("verification-colors-{theme}"));
        let mut pixels = vec![slint::Rgb8Pixel::default(); 860 * 620];
        native.window().request_redraw();
        native.draw_if_needed(|renderer| {
            renderer.render(&mut pixels, 860);
        });
        for (label, passed) in [
            ("Passed: matching.txt", true),
            ("Failed: changed.txt", false),
        ] {
            let icon = accessible_item(&native, AccessibleRole::Image, label).unwrap();
            let rect = icon.geometry();
            let position = icon.map_to_native_window(rect.origin);
            let mut colored = 0;
            for y in position.y.ceil() as usize..(position.y + rect.size.height).floor() as usize {
                for x in position.x.ceil() as usize..(position.x + rect.size.width).floor() as usize
                {
                    let pixel = pixels[y * 860 + x];
                    let [r, g, b] = [pixel.r, pixel.g, pixel.b].map(i32::from);
                    if if passed {
                        g > r + 20 && g > b + 20
                    } else {
                        r > g + 20 && r > b + 20
                    } {
                        colored += 1;
                    }
                }
            }
            assert!(
                colored >= 8,
                "{label} must visibly render in its result color, theme={theme}"
            );
        }
    }
    window.hide().unwrap();
}

#[test]
fn results_receive_the_window_instead_of_always_visible_configuration() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.invoke_apply_preferences(1, 0, "".into(), 13, 15);
    window.set_draft(ChecksumDraft {
        sha256: true,
        ..Default::default()
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "results-space");
    let list = accessible_item(&native, AccessibleRole::List, "Checksum results").unwrap();
    let rect = list.geometry();
    let position = list.map_to_native_window(rect.origin);
    assert!(position.y < 110.0, "Results begin too low: {position:?}");
    assert!(rect.size.height >= 440.0, "Results are squeezed: {rect:?}");
    window.hide().unwrap();
}

#[test]
fn save_manifest_opens_the_native_picker_before_showing_read_only_confirmation() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.invoke_apply_preferences(1, 0, "".into(), 13, 15);
    window.set_has_output(true);
    window.set_partial(true);
    window.set_output_available(Rc::new(slint::VecModel::from(vec![true; 5])).into());
    window.set_output_algorithm(4);
    window.set_destination("D:\\Žluťoučký 日本語\\checksums.sfv".into());
    let choices = Rc::new(RefCell::new(Vec::new()));
    window.on_browse_manifest({
        let choices = choices.clone();
        move |verify, algorithm| choices.borrow_mut().push((verify, algorithm))
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "save-native-picker");
    click(&native, "Save manifest…");
    assert_eq!(&*choices.borrow(), &[(false, 4)]);
    assert_eq!(
        window.get_page(),
        0,
        "Opening or cancelling the picker keeps Results"
    );
    window.set_picker_busy(true);
    draw(&native, 860, 620, "save-picker-busy");
    click(&native, "Save manifest…");
    assert_eq!(choices.borrow().len(), 1);
    window.set_picker_busy(false);
    window.set_saving(true);
    draw(&native, 860, 620, "save-write-busy");
    click(&native, "Save manifest…");
    assert_eq!(choices.borrow().len(), 1);
    window.set_saving(false);
    window.set_page(2);
    draw(&native, 860, 620, "save-read-only-destination");
    assert!(accessible_item(&native, AccessibleRole::TextInput, "Manifest output path").is_none());
    let destination =
        accessible_item(&native, AccessibleRole::Text, "Manifest output path").unwrap();
    assert_eq!(
        destination
            .accessible_string_property(AccessibleStringProperty::Value)
            .unwrap(),
        window.get_destination(),
    );
    click(&native, "Choose file…");
    assert_eq!(&*choices.borrow(), &[(false, 4), (false, 4)]);
    window.hide().unwrap();
}

#[test]
fn partial_export_requires_consent_before_dispatching_a_save() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.invoke_apply_preferences(1, 0, "".into(), 13, 15);
    window.set_page(2);
    window.set_generation_token("1".into());
    window.set_partial(true);
    window.set_has_output(true);
    window.set_output_available(
        Rc::new(slint::VecModel::from(vec![true, false, false, false])).into(),
    );
    window.set_destination("checksums.sha256".into());
    let saves = Rc::new(RefCell::new(Vec::new()));
    window.on_save_manifest({
        let saves = saves.clone();
        move |token, algorithm, destination, acknowledge| {
            saves
                .borrow_mut()
                .push((token, algorithm, destination, acknowledge));
        }
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "partial-export");
    click(&native, "Save");
    assert!(
        saves.borrow().is_empty(),
        "Unacknowledged partial export was dispatched"
    );
    accessible_item(&native, AccessibleRole::Checkbox, "Save partial results")
        .unwrap()
        .accessible_action(&AccessibilityAction::Default);
    draw(&native, 860, 620, "partial-acknowledged");
    click(&native, "Save");
    assert_eq!(
        &*saves.borrow(),
        &[("1".into(), 0, "checksums.sha256".into(), true)]
    );
    window.set_page(1);
    draw(&native, 860, 620, "options-retained");
    window.set_page(2);
    draw(&native, 860, 620, "export-retained");
    click(&native, "Save");
    assert_eq!(
        saves.borrow().len(),
        2,
        "Page changes must keep acknowledgement"
    );
    window.set_generation_token("2".into());
    slint::platform::update_timers_and_animations();
    draw(&native, 860, 620, "new-export-generation");
    click(&native, "Save");
    assert_eq!(saves.borrow().len(), 2, "A new run needs new consent");
    window.set_partial(false);
    window.set_destination("".into());
    draw(&native, 860, 620, "empty-export-destination");
    click(&native, "Save");
    assert_eq!(
        saves.borrow().len(),
        2,
        "An empty destination cannot be saved"
    );
    window.set_destination("new.sha256".into());
    window.set_saving(true);
    let stops = Rc::new(std::cell::Cell::new(0));
    window.on_stop_save({
        let stops = stops.clone();
        move || stops.set(stops.get() + 1)
    });
    draw(&native, 860, 620, "export-running");
    click(&native, "Stop save");
    assert_eq!(stops.get(), 1);
    window.set_save_stopping(true);
    draw(&native, 860, 620, "export-stopping");
    click(&native, "Stopping…");
    assert_eq!(stops.get(), 1);
    window.hide().unwrap();
}

#[test]
fn algorithm_selection_copy_keys_and_running_options_keep_their_meaning() {
    use midnight_ui::ChecksumDisplayRow;
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.invoke_apply_preferences(0, 0, "".into(), 13, 15);
    window.set_draft(ChecksumDraft {
        sha512: true,
        sha1: true,
        ..Default::default()
    });
    window.set_output_values(Rc::new(slint::VecModel::from(vec![1, 3])).into());
    window.set_output_labels(
        Rc::new(slint::VecModel::from(vec![
            "SHA-512".into(),
            "SHA-1".into(),
        ]))
        .into(),
    );
    window.set_output_available(
        Rc::new(slint::VecModel::from(vec![false, true, false, true])).into(),
    );
    window.set_output_algorithm(1);
    window.set_rows(
        Rc::new(slint::VecModel::from(vec![ChecksumDisplayRow {
            token: "1:9".into(),
            name: "Žluťoučký 日本語.bin".into(),
            calculated: true,
            sha512: "42".repeat(64).into(),
            sha1: "43".repeat(20).into(),
            status: "Calculated".into(),
            bytes: "3 B".into(),
            ..Default::default()
        }]))
        .into(),
    );
    window.set_selected_row(0);
    let copies = Rc::new(RefCell::new(Vec::new()));
    window.on_copy_result({
        let copies = copies.clone();
        move |token, algorithm, full| copies.borrow_mut().push((token, algorithm, full))
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    draw(&native, 860, 620, "algorithm-choice");
    accessible_item(&native, AccessibleRole::Combobox, "Output algorithm")
        .unwrap()
        .accessible_action(&AccessibilityAction::Expand);
    draw(&native, 860, 620, "algorithm-popup");
    native.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::DownArrow.into(),
    });
    native.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::DownArrow.into(),
    });
    native.window().dispatch_event(WindowEvent::KeyPressed {
        text: Key::Return.into(),
    });
    native.window().dispatch_event(WindowEvent::KeyReleased {
        text: Key::Return.into(),
    });
    draw(&native, 860, 620, "algorithm-selected");
    assert_eq!(
        window.get_output_algorithm(),
        3,
        "Visible choice maps to SHA-1, not array ordinal 1"
    );
    assert!(accessible_item(&native, AccessibleRole::Text, &"43".repeat(20)).is_some());
    assert!(
        accessible_item(&native, AccessibleRole::Text, &"42".repeat(64)).is_none(),
        "The results table must show the chosen SHA-1 digest, not a different algorithm"
    );
    click(&native, "Copy SHA-1");
    window.invoke_begin();
    for key in [Key::Control.into(), "c".into()] {
        native
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: key });
    }
    for key in ["c".into(), Key::Control.into()] {
        native
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: key });
    }
    assert_eq!(copies.borrow().len(), 2);
    // Dispatch the actual modifier sequence for Ctrl+Shift+C.
    for key in [Key::Control.into(), Key::Shift.into(), "C".into()] {
        native
            .window()
            .dispatch_event(WindowEvent::KeyPressed { text: key });
    }
    for key in ["C".into(), Key::Shift.into(), Key::Control.into()] {
        native
            .window()
            .dispatch_event(WindowEvent::KeyReleased { text: key });
    }
    assert_eq!(
        &*copies.borrow(),
        &[
            ("1:9".into(), 3, false),
            ("1:9".into(), 3, false),
            ("1:9".into(), 3, true)
        ]
    );
    window.set_running(true);
    window.set_page(1);
    draw(&native, 860, 620, "locked-options");
    let before = window.get_draft();
    // Actual pointer input must not change disabled controls.
    let checkbox = accessible_item(&native, AccessibleRole::Checkbox, "SHA-512").unwrap();
    let rect = checkbox.geometry();
    let origin = checkbox.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(origin.x + 10.0, origin.y + rect.size.height / 2.0);
    native.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: slint::platform::PointerEventButton::Left,
    });
    native
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
    assert_eq!(window.get_draft(), before);
    window.hide().unwrap();
}

#[test]
fn large_fonts_keep_page_navigation_close_and_manifest_confirmation_usable() {
    use midnight_ui::ChecksumDisplayRow;
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.set_destination("D:\\Žluťoučký 日本語\\checksums.sha256".into());
    window.set_details(
        "Captured folder\nD:\\Žluťoučký 日本語\n\n1 discovered · 0 skipped · 0 errors".into(),
    );
    window.set_export_summary("1 included · 2 excluded\nIncomplete discovery: additional files may not have been examined.".into());
    window.set_partial(true);
    window.set_rows(
        Rc::new(slint::VecModel::from(vec![ChecksumDisplayRow {
            name: "Visible result.txt".into(),
            status: "Calculated".into(),
            calculated: true,
            ..Default::default()
        }]))
        .into(),
    );
    window.set_progress("Completed with exclusions · 1 calculated · 1 024 B read".into());
    let closes = Rc::new(std::cell::Cell::new(0));
    window.on_close_session({
        let closes = closes.clone();
        move || closes.set(closes.get() + 1)
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for theme in [0, 1, 2] {
        for density in 0..2 {
            window.invoke_apply_preferences(theme, density, "".into(), 22, 22);
            for (width, height) in [(320, 200), (460, 720), (1000, 720)] {
                for page in 0..4 {
                    window.set_page(page);
                    draw(
                        &native,
                        width,
                        height,
                        &format!("large-{theme}-{density}-{page}"),
                    );
                    let before = closes.get();
                    click(&native, "Close");
                    assert_eq!(closes.get(), before + 1);
                    if page == 0 {
                        let list =
                            accessible_item(&native, AccessibleRole::List, "Checksum results")
                                .unwrap();
                        assert!(
                            list.geometry().size.height >= 38.0,
                            "No room for a result row: {:?}",
                            list.geometry()
                        );
                    }
                    if page == 2 {
                        assert!(
                            accessible_item(
                                &native,
                                AccessibleRole::TextInput,
                                "Manifest output path",
                            )
                            .is_none()
                        );
                        assert!(
                            accessible_item(&native, AccessibleRole::Text, "Manifest output path",)
                                .is_some()
                        );
                    }
                }
            }
        }
    }
    window.invoke_apply_preferences(0, 0, "".into(), 13, 15);
    window.set_page(0);
    window.set_attention(true);
    window.set_error("Access denied\nD:\\private\\資料.txt".into());
    window.invoke_apply_preferences(0, 1, "".into(), 22, 22);
    draw(&native, 320, 200, "large-warning");
    let list = accessible_item(&native, AccessibleRole::List, "Checksum results").unwrap();
    assert!(
        list.geometry().size.height >= 38.0,
        "Warning squeezed out results"
    );
    let before = closes.get();
    click(&native, "Close");
    assert_eq!(closes.get(), before + 1);
    window.invoke_apply_preferences(0, 0, "".into(), 13, 15);
    draw(&native, 860, 620, "warning");
    click(&native, "Dismiss checksum warning");
    draw(&native, 860, 620, "warning-dismissed");
    window.set_page(3);
    let copies = Rc::new(std::cell::Cell::new(0));
    window.on_copy_details({
        let copies = copies.clone();
        move || copies.set(copies.get() + 1)
    });
    draw(&native, 860, 620, "diagnostics");
    click(&native, "Copy details");
    assert_eq!(
        copies.get(),
        1,
        "Dismissing a notice must retain diagnostics"
    );
    window.set_page(1);
    let calculations = Rc::new(std::cell::Cell::new(0));
    window.on_calculate({
        let calculations = calculations.clone();
        move |_| calculations.set(calculations.get() + 1)
    });
    draw(&native, 860, 620, "no-algorithm");
    click(&native, "Calculate");
    assert_eq!(calculations.get(), 0);
    window.set_draft(ChecksumDraft {
        sha256: true,
        ..Default::default()
    });
    draw(&native, 860, 620, "configured");
    click(&native, "Calculate");
    assert_eq!(calculations.get(), 1);
    assert_eq!(window.get_page(), 0);
    window.hide().unwrap();
}

#[test]
fn result_and_export_algorithm_labels_stay_in_sync_after_repeated_choices() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(Windows(windows.clone()))).unwrap();
    let window = ChecksumWindow::new().unwrap();
    window.invoke_apply_preferences(0, 0, "".into(), 13, 15);
    window.set_output_values(Rc::new(slint::VecModel::from(vec![1, 3])).into());
    window.set_output_labels(
        Rc::new(slint::VecModel::from(vec![
            "SHA-512".into(),
            "SHA-1".into(),
        ]))
        .into(),
    );
    window.set_output_available(
        Rc::new(slint::VecModel::from(vec![false, true, false, true])).into(),
    );
    window.set_format_labels(
        Rc::new(slint::VecModel::from(vec![
            "SHA-512 (*.sha512)".into(),
            "SHA-1 (*.sha1)".into(),
        ]))
        .into(),
    );
    window.set_output_algorithm(1);
    window.set_has_output(true);
    window.set_destination("checksums.txt".into());
    let saves = Rc::new(RefCell::new(Vec::new()));
    window.on_save_manifest({
        let saves = saves.clone();
        move |_, algorithm, _, _| saves.borrow_mut().push(algorithm)
    });
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for (page, key, expected, label) in [
        (2, Key::DownArrow, 3, "SHA-1"),
        (0, Key::UpArrow, 1, "SHA-512"),
        (2, Key::DownArrow, 3, "SHA-1"),
    ] {
        window.set_page(page);
        draw(&native, 860, 620, "picker-before");
        accessible_item(
            &native,
            AccessibleRole::Combobox,
            if page == 2 {
                "Checksum file format"
            } else {
                "Output algorithm"
            },
        )
        .unwrap()
        .accessible_action(&AccessibilityAction::Expand);
        draw(&native, 860, 620, "picker-open");
        for text in [slint::SharedString::from(key), Key::Return.into()] {
            native
                .window()
                .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
            native
                .window()
                .dispatch_event(WindowEvent::KeyReleased { text });
        }
        slint::platform::update_timers_and_animations();
        assert_eq!(window.get_output_algorithm(), expected);
        for destination_page in [0, 2] {
            window.set_page(destination_page);
            slint::platform::update_timers_and_animations();
            draw(&native, 860, 620, "picker-shared");
            let picker = accessible_item(
                &native,
                AccessibleRole::Combobox,
                if destination_page == 2 {
                    "Checksum file format"
                } else {
                    "Output algorithm"
                },
            )
            .unwrap();
            let expected_label = if destination_page == 2 {
                format!(
                    "{label} (*.{})",
                    if expected == 3 { "sha1" } else { "sha512" }
                )
            } else {
                label.to_owned()
            };
            assert_eq!(
                picker
                    .accessible_string_property(AccessibleStringProperty::Value)
                    .unwrap(),
                expected_label.as_str()
            );
        }
        click(&native, "Save");
        assert_eq!(saves.borrow().last(), Some(&expected));
    }
    window.set_output_values(Rc::new(slint::VecModel::from(vec![0])).into());
    window.set_output_labels(Rc::new(slint::VecModel::from(vec!["SHA-256".into()])).into());
    window.set_format_labels(
        Rc::new(slint::VecModel::from(vec!["SHA-256 (*.sha256)".into()])).into(),
    );
    window.set_output_algorithm(0);
    window.set_generation_token("2".into());
    for page in [0, 2] {
        window.set_page(page);
        slint::platform::update_timers_and_animations();
        draw(&native, 860, 620, "picker-new-generation");
        let picker = accessible_item(
            &native,
            AccessibleRole::Combobox,
            if page == 2 {
                "Checksum file format"
            } else {
                "Output algorithm"
            },
        )
        .unwrap();
        assert_eq!(
            picker
                .accessible_string_property(AccessibleStringProperty::Value)
                .unwrap(),
            if page == 2 {
                "SHA-256 (*.sha256)"
            } else {
                "SHA-256"
            }
        );
    }
    window.hide().unwrap();
}
