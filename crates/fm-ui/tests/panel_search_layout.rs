//! Native committed text keeps its caret and match count visible in the compact path slot.
use fm_ui::MainWindow;
use i_slint_core::{
    accessibility::AccessibleStringProperty,
    input::{InternalKeyEvent, KeyEvent, KeyEventType},
    item_tree::ItemRc,
    items::{Orientation, TextCursorAffinity, TextInput},
    window::WindowInner,
};
use slint::{
    ComponentHandle,
    platform::{
        Key, Platform, PlatformError, WindowAdapter, WindowEvent,
        software_renderer::{MinimalSoftwareWindow, RepaintBufferType},
    },
};
use std::rc::Rc;

const WIDTH: usize = 1040;
const HEIGHT: usize = 900;

struct TestPlatform(Rc<MinimalSoftwareWindow>);
impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, PlatformError> {
        Ok(self.0.clone())
    }
}

fn find(item: ItemRc, label: &str) -> Vec<ItemRc> {
    let mut result = Vec::new();
    if item
        .accessible_string_property(AccessibleStringProperty::Label)
        .is_some_and(|value| value.as_str() == label)
    {
        result.push(item.clone());
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        result.extend(find(item, label));
    }
    result
}

fn root(window: &MainWindow) -> ItemRc {
    ItemRc::new_root(WindowInner::from_pub(window.window()).component())
}

fn key(window: &MainWindow, key: Key) {
    let text: slint::SharedString = key.into();
    window
        .window()
        .dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window
        .window()
        .dispatch_event(WindowEvent::KeyReleased { text });
}

fn composition(window: &MainWindow, text: &str, preedit: &str) {
    let mut key_event = KeyEvent::default();
    key_event.text = text.into();
    window
        .window()
        .dispatch_event(WindowEvent::internal(InternalKeyEvent {
            key_event,
            event_type: if preedit.is_empty() {
                KeyEventType::CommitComposition
            } else {
                KeyEventType::UpdateComposition
            },
            preedit_text: preedit.into(),
            ..Default::default()
        }));
}

fn draw(native: &MinimalSoftwareWindow, pixels: &mut [slint::Rgb8Pixel]) {
    slint::platform::update_timers_and_animations();
    native.draw_if_needed(|renderer| {
        renderer.render(pixels, WIDTH);
    });
    // Layout can instantiate native inputs/change handlers lazily. Process the
    // next normal event-loop turn before inspecting their rendered caret geometry.
    slint::platform::update_timers_and_animations();
    native.draw_if_needed(|renderer| {
        renderer.render(pixels, WIDTH);
    });
}

#[test]
fn long_native_queries_pan_the_caret_without_covering_full_match_count_or_clear_button() {
    let native = MinimalSoftwareWindow::new(RepaintBufferType::ReusedBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(native.clone()))).unwrap();
    let window = MainWindow::new().unwrap();
    fm_ui::install_file_keys(&window);
    window.show().unwrap();
    native.set_size(slint::PhysicalSize::new(WIDTH as u32, HEIGHT as u32));
    window.window().dispatch_event(WindowEvent::Resized {
        size: slint::LogicalSize::new(WIDTH as f32, HEIGHT as f32),
    });
    let adapter: Rc<dyn WindowAdapter> = native.clone();
    let mut pixels = vec![slint::Rgb8Pixel::default(); WIDTH * HEIGHT];
    let text = format!("{}文档👩🏽‍💻tail", "wide-prefix-".repeat(16));
    let mut clipped = Vec::new();
    for (density, font) in [(0, 11), (1, 15), (0, 32), (1, 32)] {
        window.invoke_apply_fonts("".into(), font, font);
        window.invoke_apply_preferences(0, density);
        window.invoke_focus_files();
        key(&window, Key::Escape);
        composition(&window, &text, "");
        window.set_left_search_count(1_234_567);
        draw(&native, &mut pixels);
        assert_eq!(window.get_left_search_query().as_str(), text);
        let input = find(root(&window), "Filename prefix search")
            .into_iter()
            .find(|item| {
                item.downcast::<TextInput>()
                    .is_some_and(|input| input.as_pin_ref().has_focus())
            })
            .unwrap();
        let count = find(root(&window), "1234567 matches")
            .into_iter()
            .next()
            .unwrap();
        let count_origin = count.map_to_native_window(count.geometry().origin);
        let clear = find(root(&window), "Clear filename search")
            .into_iter()
            .next()
            .unwrap();
        let clear_origin = clear.map_to_native_window(clear.geometry().origin);
        let preferred = count.borrow().as_ref().layout_info(
            Orientation::Horizontal,
            count.geometry().size.height,
            &adapter,
            &count,
        );
        if count.geometry().size.width + 1.0 < preferred.preferred {
            clipped.push(format!(
                "count clipped: density={density},font={font},width={},needed={}",
                count.geometry().size.width,
                preferred.preferred
            ));
        }
        assert!(count_origin.x + count.geometry().size.width <= clear_origin.x + 1.0);
        let check_caret = || {
            let input_ref = input.downcast::<TextInput>().unwrap();
            let input_ref = input_ref.as_pin_ref();
            let caret = native.renderer().text_input_cursor_rect_for_byte_offset(
                input_ref,
                &input,
                input_ref.cursor_position_byte_offset() as usize,
                TextCursorAffinity::NextCharacter,
            );
            let origin =
                input.map_to_native_window(input.geometry().origin + caret.origin.to_vector());
            let left = window.global::<fm_ui::Theme>().get_window_padding() + 30.0;
            (!(origin.x >= left && origin.x + caret.size.width <= count_origin.x - 2.0))
                .then(|| format!(
                    "caret clipped: density={density},font={font},caret={origin:?},count={count_origin:?}"
                ))
        };
        clipped.extend(check_caret());
        for _ in 0..text.chars().count() + 2 {
            key(&window, Key::LeftArrow);
        }
        draw(&native, &mut pixels);
        clipped.extend(check_caret());
        assert_eq!(window.get_left_search_query().as_str(), text);
        composition(&window, "", "未提交");
        assert_eq!(window.get_left_search_preedit().as_str(), "未提交");
        assert_eq!(window.get_left_search_query().as_str(), text);
        composition(&window, "字", "");
        assert_eq!(window.get_left_search_query().as_str(), format!("字{text}"));
    }
    assert!(clipped.is_empty(), "{}", clipped.join("\n"));
    window.hide().unwrap();
}
