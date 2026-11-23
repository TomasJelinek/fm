//! Make File List: one content column whose fields, pickers and preview end where the
//! header's close button and the footer's Cancel end, at every font, density, theme
//! and size; the line template is the dialogs' common edit (inline clear, recent
//! templates from the line itself) and Output is the common dropdown.
#[path = "support/embedded_form.rs"]
mod form;
use crate::support::modeless;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ClippedImage, ImageItem},
};
use midnight_ui::FileListPresentation;
use slint::{
    ComponentHandle, ModelRc, SharedString, VecModel,
    platform::{Key, WindowAdapter},
};
use std::{cell::RefCell, rc::Rc};

fn strings(values: &[&str]) -> ModelRc<SharedString> {
    ModelRc::from(Rc::new(VecModel::from(
        values
            .iter()
            .map(|value| SharedString::from(*value))
            .collect::<Vec<_>>(),
    )))
}

/// The embedded form with a file destination, so that every control shows.
fn file_list_host() -> form::Host {
    let host = form::Host::new();
    let list = host.main.global::<FileListPresentation>();
    let weak = host.main.as_weak();
    list.on_cancel(move |_| {
        if let Some(main) = weak.upgrade() {
            main.global::<FileListPresentation>().set_active(false);
        }
    });
    list.set_template_text("$(Drive)$(Path)$(FileName)$(CRLF)".into());
    list.set_template_history(strings(&["$(FileName)$(LF)", "$(FileSize)$(LF)"]));
    list.set_variable_labels(strings(&["File name", "Full path", "Size", "Tab", "CRLF"]));
    list.set_destination(2);
    list.set_destination_path("C:\\out\\list 文档.txt".into());
    list.set_preview_text("C:\\data\\文档.txt\r\nC:\\data\\notes.txt\r\n".into());
    list.set_summary("8 items".into());
    list.set_token("file-list".into());
    list.invoke_begin();
    host.settle();
    host
}

/// Window-space left, top, right and bottom of `item`.
fn bounds(item: &ItemRc) -> (f32, f32, f32, f32) {
    let at = item.map_to_native_window(item.geometry().origin);
    let size = item.geometry().size;
    (at.x, at.y, at.x + size.width, at.y + size.height)
}

fn visible_images(item: ItemRc, found: &mut Vec<ItemRc>) {
    if item.is_visible()
        && (item.downcast::<ImageItem>().is_some() || item.downcast::<ClippedImage>().is_some())
    {
        found.push(item.clone());
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        visible_images(item, found);
    }
}

const TEMPLATE: &str = "File list line template";
const PRESETS: &str = "File list template preset";
const VARIABLES: &str = "Insert file list variable";
const OUTPUT: &str = "File list output destination";
const PATH: &str = "File list destination path";
const BROWSE: &str = "Browse file list destination";
const PREVIEW: &str = "File list preview";

/// Every alignment and size rule of one rendered state, as failures.
fn check(host: &form::Host, case: &str, failures: &mut Vec<String>) {
    let mut fail = |message: String| failures.push(format!("{case}: {message}"));
    let cancel = host.item(AccessibleRole::Button, "Cancel");
    let (body_at, body) = form::body(&cancel);
    let (body_left, body_right) = (body_at.x, body_at.x + body.width);
    let right = bounds(&cancel).2;
    let close = bounds(&host.item(AccessibleRole::Button, "Cancel file list"));
    if (close.2 - right).abs() > 0.5 {
        fail(format!(
            "the header close ends at {} but Cancel at {right}",
            close.2
        ));
    }
    let template = bounds(&host.item(AccessibleRole::TextInput, TEMPLATE));
    let left = template.0;
    if ((left - body_left) - (body_right - right)).abs() > 0.5 {
        fail(format!(
            "left margin {} differs from right margin {}",
            left - body_left,
            body_right - right
        ));
    }
    let presets = bounds(&host.item(AccessibleRole::Combobox, PRESETS));
    let variables = bounds(&host.item(AccessibleRole::Combobox, VARIABLES));
    let output = bounds(&host.item(AccessibleRole::Combobox, OUTPUT));
    let path = bounds(&host.item(AccessibleRole::TextInput, PATH));
    let browse = bounds(&host.item(AccessibleRole::Button, BROWSE));
    let preview = bounds(&host.item(AccessibleRole::TextInput, PREVIEW));
    let stacked = (presets.1 - variables.1).abs() > 0.5;
    let mut rightmost = vec![
        (TEMPLATE, template),
        (VARIABLES, variables),
        (OUTPUT, output),
        (BROWSE, browse),
        (PREVIEW, preview),
    ];
    if stacked {
        rightmost.push((PRESETS, presets));
    }
    for (label, edges) in rightmost {
        if (edges.2 - right).abs() > 0.5 {
            fail(format!("{label} ends at {} but Cancel at {right}", edges.2));
        }
    }
    let mut leftmost = vec![
        (PRESETS, presets),
        (OUTPUT, output),
        (PATH, path),
        (PREVIEW, preview),
    ];
    if stacked {
        leftmost.push((VARIABLES, variables));
    }
    for (label, edges) in leftmost {
        if (edges.0 - left).abs() > 0.5 {
            fail(format!(
                "{label} starts at {} but the template at {left}",
                edges.0
            ));
        }
    }
    let height = template.3 - template.1;
    for (label, edges) in [
        (PRESETS, presets),
        (VARIABLES, variables),
        (OUTPUT, output),
        (PATH, path),
        (BROWSE, browse),
    ] {
        if ((edges.3 - edges.1) - height).abs() > 0.5 {
            fail(format!(
                "{label} is {} tall, the template {height}",
                edges.3 - edges.1
            ));
        }
    }
    // Pickers side by side are each wide enough for their caption.
    for (label, edges) in [(PRESETS, presets), (VARIABLES, variables)] {
        if edges.2 - edges.0 < 40.0 {
            fail(format!("{label} is squeezed to {}", edges.2 - edges.0));
        }
    }
    for label in ["Create file list", "Cancel", "Cancel file list"] {
        let edges = bounds(&host.item(AccessibleRole::Button, label));
        if edges.0 < body_left - 0.5
            || edges.2 > body_right + 0.5
            || edges.1 < body_at.y - 0.5
            || edges.3 > body_at.y + body.height + 0.5
        {
            fail(format!("{label} is cut by the dialog body: {edges:?}"));
        }
    }
    if host
        .try_item(AccessibleRole::Combobox, "File list template history")
        .is_some()
    {
        fail("the template history is a picker beside the line, not the line's own list".into());
    }
}

/// The owner's report: the fields ended about 50 px left of the header's close button
/// and Cancel, the template was not the common edit and Output not the common dropdown.
#[test]
fn make_file_list_fields_share_the_content_edge_of_close_and_cancel() {
    let host = file_list_host();
    let mut failures = Vec::new();
    for (scheme, theme) in [(1, "dark"), (2, "light")] {
        for (density, kind) in [(0, "desktop"), (1, "touch")] {
            for font in [8, 12, 32] {
                for (width, height, size) in [(1340, 900, "wide"), (400, 560, "narrow")] {
                    host.apply_preferences(scheme, density, "", font, font);
                    let case = format!("file-list-{theme}-{kind}-{font}px-{size}");
                    host.draw(width, height, &case);
                    check(&host, &case, &mut failures);
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// At the smallest window and the largest fonts every control keeps its size and the
/// footer stays whole; the form scrolls instead of squeezing.
#[test]
fn make_file_list_controls_are_not_squeezed_at_the_smallest_size() {
    let host = file_list_host();
    for (density, font) in [(0, 8), (0, 32), (1, 12), (1, 32)] {
        host.apply_preferences(1, density, "", font, font);
        host.draw(328, 208, &format!("file-list-minimum-{density}-{font}px"));
        for label in ["Create file list", "Cancel"] {
            form::assert_inside(&host.item(AccessibleRole::Button, label), label);
        }
        let template = bounds(&host.item(AccessibleRole::TextInput, TEMPLATE));
        let minimum = 24f32.max(font as f32 + 10.0);
        assert!(
            template.3 - template.1 >= minimum - 0.5,
            "density {density}, {font} px: the template is {} tall",
            template.3 - template.1
        );
        let cancel = bounds(&host.item(AccessibleRole::Button, "Cancel"));
        assert!(
            (template.2 - cancel.2).abs() <= 0.5,
            "density {density}, {font} px: the template ends at {} but Cancel at {}",
            template.2,
            cancel.2
        );
    }
}

/// The line template is the common edit: while it has the keyboard and text its clear ×
/// is its only icon, at its right end; the × empties it as an edit; Down lists the
/// recent templates from the line itself and choosing one restores it.
#[test]
fn the_line_template_is_the_common_edit_with_its_recent_templates() {
    let host = file_list_host();
    let list = host.main.global::<FileListPresentation>();
    let edits = Rc::new(RefCell::new(Vec::new()));
    let captured = edits.clone();
    list.on_template_edited(move |_, text| captured.borrow_mut().push(text.to_string()));
    let chosen = Rc::new(RefCell::new(Vec::new()));
    let captured = chosen.clone();
    let weak = host.main.as_weak();
    list.on_select_history(move |_, index| {
        captured.borrow_mut().push(index);
        if let Some(main) = weak.upgrade() {
            main.global::<FileListPresentation>()
                .set_template_text(["$(FileName)$(LF)", "$(FileSize)$(LF)"][index as usize].into());
        }
    });
    host.apply_preferences(1, 0, "", 12, 12);
    host.draw(1340, 900, "file-list-common-edit");
    let line = host.item(AccessibleRole::TextInput, TEMPLATE);
    let mut images = Vec::new();
    visible_images(line.clone(), &mut images);
    assert_eq!(
        images.len(),
        1,
        "the focused template shows its inline clear"
    );
    let (_, _, right, _) = bounds(&line);
    let clear = bounds(&images[0]);
    assert!(
        clear.2 <= right + 0.5 && right - clear.2 <= 14.0,
        "the clear sits inside the line at its right end: {clear:?}, line ends at {right}"
    );
    // The history opens from the line: Down lists the recent templates.
    host.key(Key::DownArrow);
    host.draw(1340, 900, "file-list-history-open");
    let row = host.item(AccessibleRole::Button, "$(FileSize)$(LF)");
    assert!(row.is_visible(), "the recent templates are listed");
    host.key(Key::DownArrow);
    host.key(Key::Return);
    assert_eq!(*chosen.borrow(), [1], "Return chooses the browsed template");
    assert_eq!(list.get_template_text(), "$(FileSize)$(LF)");
    // The inline clear empties the line as an edit; the line keeps the keyboard.
    host.layout(1340, 900);
    let line = host.item(AccessibleRole::TextInput, TEMPLATE);
    let mut images = Vec::new();
    visible_images(line.clone(), &mut images);
    let clear = images.pop().expect("the inline clear is shown");
    let (x, y, r, b) = bounds(&clear);
    let position = slint::LogicalPosition::new((x + r) / 2.0, (y + b) / 2.0);
    for event in [
        slint::platform::WindowEvent::PointerPressed {
            position,
            button: slint::platform::PointerEventButton::Left,
        },
        slint::platform::WindowEvent::PointerReleased {
            position,
            button: slint::platform::PointerEventButton::Left,
        },
    ] {
        host.native.window().dispatch_event(event);
    }
    host.settle();
    assert_eq!(list.get_template_text(), "");
    assert_eq!(edits.borrow().last().map(String::as_str), Some(""));
    host.key("x");
    assert_eq!(list.get_template_text(), "x", "the line keeps the keyboard");
    let output = host.item(AccessibleRole::Combobox, OUTPUT);
    assert_eq!(
        output
            .accessible_string_property(AccessibleStringProperty::Value)
            .as_deref(),
        Some("File"),
        "Output shows its choice"
    );
}
