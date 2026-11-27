//! Dismissing presentation never clears the host's failure or grants capabilities.
use crate::support::modeless as ui;
use i_slint_core::{
    accessibility::AccessibilityAction,
    item_tree::ItemRc,
    items::{AccessibleRole, ImageItem},
};
use slint::{
    ComponentHandle,
    platform::{Key, WindowAdapter, WindowEvent, software_renderer::MinimalSoftwareWindow},
};
use std::{cell::RefCell, rc::Rc};

slint::slint! {
    // Exported so that the scheme is not a compile-time constant, which would copy the
    // whole palette into every color binding of the fixture.
    import { Theme } from "../ui/theme.slint";
    export { Theme }
    import { ErrorNotice } from "../ui/error-notice.slint";
    export component NoticeFixture inherits Window {
        preferred-width: 320px; preferred-height: 200px;
        in-out property <string> error;
        in-out property <string> occurrence;
        in-out property <bool> compact;
        in-out property <bool> can-authorize;
        out property <int> authorization-count;
        out property <bool> showing: notice.showing;
        out property <length> notice-height: notice.height;
        out property <int> dismiss-count;
        in-out property <string> copied;
        VerticalLayout {
            padding: 6px; spacing: 0px;
            Rectangle { vertical-stretch: 1; background: #252630; }
            notice := ErrorNotice {
                text: root.error; context-key: root.occurrence;
                copy-enabled: true;
                compact: root.compact;
                retry-enabled: root.can-authorize;
                retry-authorized => { root.authorization-count += 1; }
                minimum-notice-height: root.compact ? 12px : 60px;
                preferred-notice-height: root.compact ? 12px : 100px;
                maximum-notice-height: root.compact ? 12px : 160px;
                copy-details(text) => { root.copied = text; }
                dismissed => { root.dismiss-count += 1; }
            }
        }
    }
    import { NoticeDialog } from "../ui/notice-dialog.slint";
    // The folder notice's dialog alone, as wide as its window.
    export component NoticeDialogFixture inherits Window {
        preferred-width: 560px; preferred-height: 420px;
        in-out property <bool> has-details;
        in-out property <bool> expanded;
        in-out property <bool> can-authorize;
        in-out property <string> body: "Access to this folder was denied.";
        background: #252630;
        NoticeDialog {
            x: 0px; y: 0px; width: root.width; height: root.height;
            title: "Cannot open folder"; error: true;
            body: root.body;
            folder-path: "C:\\Users\\Example";
            details: root.has-details ? "Observed issues: 1\nStored details: 1" : "";
            can-authorize: root.can-authorize;
            details-expanded <=> root.expanded;
        }
    }
}

fn image_item(item: ItemRc) -> Option<ItemRc> {
    if item.downcast::<ImageItem>().is_some() {
        return Some(item);
    }
    let mut child = item.first_child();
    while let Some(item) = child {
        child = item.next_sibling();
        if let Some(image) = image_item(item) {
            return Some(image);
        }
    }
    None
}

#[test]
fn wide_notice_uses_its_width_and_keeps_actions_at_opposite_corners() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = NoticeFixture::new().unwrap();
    let message = "Cannot open folder. Access denied while reading the requested directory.\nPath\nD:\\資料\\protected\\folder";
    window.set_error(message.into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    for (width, compact) in [(1550, false), (320, false), (320, true)] {
        window.set_compact(compact);
        ui::draw(
            &native,
            width,
            200,
            if compact {
                "error-notice-compact"
            } else {
                "error-notice-full-width"
            },
        );
        let text = ui::accessible_item(&native, AccessibleRole::Text, message).unwrap();
        assert!(
            text.geometry().width() >= width as f32 - 140.0,
            "message should fill the banner between its small action columns: {:?}",
            text.geometry()
        );
        let close =
            ui::accessible_item(&native, AccessibleRole::Button, "Dismiss message").unwrap();
        let copy =
            ui::accessible_item(&native, AccessibleRole::Button, "Copy error details").unwrap();
        let close_at = close.map_to_native_window(close.geometry().origin);
        let copy_at = copy.map_to_native_window(copy.geometry().origin);
        let glyph = image_item(copy.clone()).expect("Copy has an image");
        let glyph_at = glyph.map_to_native_window(glyph.geometry().origin);
        assert!(glyph_at.x >= copy_at.x && glyph_at.y >= copy_at.y);
        assert!(glyph_at.x + glyph.geometry().width() <= copy_at.x + copy.geometry().width());
        assert!(glyph_at.y + glyph.geometry().height() <= copy_at.y + copy.geometry().height());
        assert!(
            (glyph_at.x + glyph.geometry().width() / 2.0
                - copy_at.x
                - copy.geometry().width() / 2.0)
                .abs()
                < 0.1
        );
        assert!(
            (glyph_at.y + glyph.geometry().height() / 2.0
                - copy_at.y
                - copy.geometry().height() / 2.0)
                .abs()
                < 0.1
        );
        assert!(close_at.x + close.geometry().width() >= width as f32 - 22.0);
        assert!(
            copy_at.x < 45.0,
            "Copy belongs at the bottom-left: {copy_at:?}"
        );
        if compact {
            assert!(window.get_notice_height() <= 12.0);
        } else {
            assert!(
                window.get_notice_height() <= 100.1,
                "a short notice must not stretch beyond its preferred height: {}",
                window.get_notice_height()
            );
            assert!(copy_at.y > close_at.y + close.geometry().height());
        }
        ui::click(&native, "Copy error details");
        assert_eq!(window.get_copied(), message);
        for key in [Key::Return, Key::Space] {
            window.set_copied("".into());
            native
                .window()
                .dispatch_event(WindowEvent::KeyPressed { text: key.into() });
            native
                .window()
                .dispatch_event(WindowEvent::KeyReleased { text: key.into() });
            assert_eq!(window.get_copied(), message);
        }
        window.set_copied("".into());
        copy.accessible_action(&AccessibilityAction::Default);
        assert_eq!(window.get_copied(), message);
        assert!(window.get_showing());
    }
}

#[test]
fn long_notice_scrolls_while_copy_retains_the_complete_error() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = NoticeFixture::new().unwrap();
    let message = format!(
        "Access denied\n{}\nLast diagnostic line",
        "D:\\資料\\protected\\folder\n".repeat(30)
    );
    window.set_error(message.as_str().into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 320, 200, "error-notice-scroll-before");
    let text = ui::accessible_item(&native, AccessibleRole::Text, &message).unwrap();
    let before = text.map_to_native_window(text.geometry().origin).y;
    native
        .window()
        .dispatch_event(slint::platform::WindowEvent::PointerScrolled {
            position: slint::LogicalPosition::new(160.0, 160.0),
            delta_x: 0.0,
            delta_y: -10_000.0,
        });
    ui::draw(&native, 320, 200, "error-notice-scroll-after");
    assert!(text.map_to_native_window(text.geometry().origin).y < before - 20.0);
    ui::click(&native, "Copy error details");
    assert_eq!(window.get_copied(), message);
    assert!(window.get_showing());
}

#[test]
fn close_collapses_notice_but_new_occurrences_remain_visible() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = NoticeFixture::new().unwrap();
    let message = "Archive plugin unavailable\nD:\\資料\\example.zip";
    window.set_error(message.into());
    window.set_occurrence("request-1".into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    slint::platform::update_timers_and_animations();
    ui::draw(&native, 320, 200, "error-close-before");
    assert!(window.get_showing());
    assert!(window.get_notice_height() > 0.0);
    ui::click(&native, "Copy error details");
    assert_eq!(window.get_copied(), message);
    assert!(window.get_showing(), "copying must not dismiss the error");
    ui::click(&native, "Dismiss message");
    ui::draw(&native, 320, 200, "error-close-after");
    assert!(!window.get_showing());
    assert_eq!(window.get_notice_height(), 0.0);
    assert_eq!(window.get_error(), message);
    assert_eq!(window.get_dismiss_count(), 1);

    window.set_occurrence("request-2".into());
    slint::platform::update_timers_and_animations();
    ui::draw(&native, 320, 200, "error-close-new-occurrence");
    assert!(
        window.get_showing(),
        "identical diagnostics from a new request must reappear"
    );
    ui::click(&native, "Dismiss message");
    window.set_error("A different error".into());
    slint::platform::update_timers_and_animations();
    ui::draw(&native, 320, 200, "error-close-new-text");
    assert!(window.get_showing());
    window.set_error("".into());
    slint::platform::update_timers_and_animations();
    ui::draw(&native, 320, 200, "error-close-empty");
    assert!(!window.get_showing());
    assert_eq!(window.get_notice_height(), 0.0);
}

#[test]
fn authorization_retry_requires_an_explicit_current_error_gesture() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = NoticeFixture::new().unwrap();
    window.set_error("Access denied\nPath\nC:/private".into());
    window.set_occurrence("request-1".into());
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    ui::draw(&native, 320, 230, "authorization-unavailable");
    assert!(
        ui::accessible_item(&native, AccessibleRole::Button, "Retry with authorization").is_none()
    );
    window.set_can_authorize(true);
    ui::draw(&native, 320, 230, "authorization-available");
    assert_eq!(
        window.get_authorization_count(),
        0,
        "showing an error must never authorize"
    );
    ui::click(&native, "Retry with authorization");
    assert_eq!(window.get_authorization_count(), 1);
    let item =
        ui::accessible_item(&native, AccessibleRole::Button, "Retry with authorization").unwrap();
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    let position = slint::LogicalPosition::new(
        origin.x + rect.size.width / 2.0,
        origin.y + rect.size.height / 2.0,
    );
    native.window().dispatch_event(WindowEvent::PointerPressed {
        position,
        button: slint::platform::PointerEventButton::Left,
    });
    window.set_occurrence("request-2".into());
    native
        .window()
        .dispatch_event(WindowEvent::PointerReleased {
            position,
            button: slint::platform::PointerEventButton::Left,
        });
    assert_eq!(
        window.get_authorization_count(),
        1,
        "rebound error must reject old press"
    );
    window.set_compact(true);
    ui::draw(&native, 320, 230, "authorization-compact");
    ui::click(&native, "Retry with authorization");
    assert_eq!(window.get_authorization_count(), 2);
}

const DIALOG_HEIGHT: u32 = 420;

/// The visible button with this accessible label.
fn notice_button(native: &MinimalSoftwareWindow, label: &str) -> ItemRc {
    ui::accessible_items(native, AccessibleRole::Button, label)
        .into_iter()
        .find(ItemRc::is_visible)
        .unwrap_or_else(|| panic!("the notice shows no {label}"))
}

/// Whether the folder notice shows its actions beside Close with their captions, and
/// whether the authorization offer shares their row. Every button on Close's row is
/// whole, the actions are 8 px apart, and Close ends the row inside the dialog's
/// padding; collapsed Details leaves the room of Hide details before Close.
fn notice_actions(
    native: &MinimalSoftwareWindow,
    width: u32,
    icon: f32,
    authorize: bool,
    case: &str,
) -> (bool, bool) {
    ui::draw(native, width, DIALOG_HEIGHT, case);
    let buttons = ui::footer_row(native, "Close");
    for (label, button) in &buttons {
        let (whole, shown) = (ui::edges(button), ui::shown_edges(native, button));
        assert!(
            (whole.0 - shown.0).abs() < 0.5 && (whole.2 - shown.2).abs() < 0.5,
            "{case}: {label} is cut: {whole:?} shows {shown:?}"
        );
    }
    for (index, pair) in buttons.windows(2).enumerate() {
        let gap = ui::edges(&pair[1].1).0 - ui::edges(&pair[0].1).2;
        let before_close = index + 2 == buttons.len();
        assert!(
            (gap - 8.0).abs() < 0.75 || before_close && gap > 8.0,
            "{case}: {} | {} are {gap}px apart",
            pair[0].0,
            pair[1].0
        );
    }
    let row: Vec<String> = buttons.into_iter().map(|(label, _)| label).collect();
    assert_eq!(
        row.last().map(String::as_str),
        Some("Close"),
        "{case}: {row:?}"
    );
    let close = notice_button(native, "Close");
    let (_, _, right, _) = ui::edges(&close);
    assert!(
        right <= width as f32 - 20.0 + 0.5,
        "{case}: Close ends at {right}"
    );
    // A glyph-only button is its glyph inside 12 px of padding on each side.
    let captioned = |label: &str| notice_button(native, label).geometry().width() > icon + 25.0;
    let shown = captioned("Copy details");
    for label in &row {
        assert_eq!(
            captioned(label),
            shown || label == "Close",
            "{case}: {label} differs from its row {row:?}"
        );
    }
    let in_row = row.iter().any(|label| label == "Retry with authorization");
    if authorize && !in_row {
        assert!(
            captioned("Retry with authorization"),
            "{case}: the offer's own row keeps its caption"
        );
    }
    (shown, in_row)
}

/// The actions beside Close keep their captions down to the exact width where the
/// captioned buttons, 8 px apart, no longer fit beside Close inside the dialog's 20 px
/// padding; a pixel narrower they show their glyphs only. The authorization offer keeps
/// its place on that row down to its own such width, and then takes a row of its own
/// with its caption before the other actions give up theirs. Details counts as Hide
/// details at either state, so that showing the details moves nothing. The thresholds
/// come from the buttons' own widths at every density and font size.
#[test]
fn folder_notice_actions_keep_their_captions_while_they_fit_beside_close() {
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = NoticeDialogFixture::new().unwrap();
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    let theme = window.global::<Theme>();
    for (density, font) in [(0, 12), (0, 8), (0, 24), (1, 15), (1, 32)] {
        theme.set_density(density);
        if density == 1 {
            theme.set_touch_font_size(font);
        } else {
            theme.set_desktop_font_size(font);
        }
        let icon = if density == 1 { 20.0 } else { 14.0 };
        for (details, authorize) in [(true, true), (true, false), (false, true), (false, false)] {
            let case = format!("notice-actions-{density}-{font}-{details}-{authorize}");
            window.set_has_details(details);
            window.set_can_authorize(authorize);
            // The captioned widths, with Details showing Hide details, on a wide row.
            window.set_expanded(details);
            ui::layout(&native, 1600, DIALOG_HEIGHT);
            let width = |label: &str| notice_button(&native, label).geometry().width();
            let close = width("Close");
            let mut actions = width("Copy details");
            if details {
                actions += width("Hide details") + 8.0;
            }
            // The narrowest dialog whose row holds these actions and Close.
            let fits = |row: f32| (2.0 * 20.0 + row + 8.0 + close).ceil() as u32;
            let glyphs_below = fits(actions);
            let offer_below =
                authorize.then(|| fits(actions + 8.0 + width("Retry with authorization")));
            for expanded in [false, true] {
                if expanded && !details {
                    continue;
                }
                window.set_expanded(expanded);
                let check = |width: u32| {
                    notice_actions(
                        &native,
                        width,
                        icon,
                        authorize,
                        &format!("{case}-{expanded}-{width}"),
                    )
                };
                assert_eq!(
                    check(glyphs_below),
                    (true, false),
                    "{case}-{expanded}: captions fit at {glyphs_below}"
                );
                if let Some(offer) = offer_below {
                    assert_eq!(
                        check(offer),
                        (true, true),
                        "{case}-{expanded}: the offer fits in the row at {offer}"
                    );
                    assert_eq!(
                        check(offer - 1),
                        (true, false),
                        "{case}-{expanded}: the offer takes its own row below {offer}"
                    );
                }
                assert_eq!(
                    check(glyphs_below - 1),
                    (false, false),
                    "{case}-{expanded}: glyphs below {glyphs_below}"
                );
            }
        }
    }
}

/// An available authorization promises the operating system's prompt and offers the retry;
/// an unavailable one says why in the message and offers neither (the presenter then reports
/// `can-authorize` false and completes the message with the reason).
#[test]
fn folder_notice_promises_a_prompt_only_when_authorization_is_available() {
    const HINT: &str = "Your operating system will ask for permission to read this folder.";
    let windows = Rc::new(RefCell::new(Vec::new()));
    slint::platform::set_platform(Box::new(ui::Windows(windows.clone()))).unwrap();
    let window = NoticeDialogFixture::new().unwrap();
    window.show().unwrap();
    let native = windows.borrow()[0].clone();
    let shown = |role, label: &str| {
        ui::accessible_items(&native, role, label)
            .into_iter()
            .any(|item| item.is_visible())
    };
    window.set_can_authorize(true);
    ui::draw(&native, 560, DIALOG_HEIGHT, "authorization-offer-available");
    assert!(shown(AccessibleRole::Text, HINT));
    assert!(shown(AccessibleRole::Button, "Retry with authorization"));

    window.set_can_authorize(false);
    window.set_body(
        "Access to this folder was denied. Opening it needs administrator permission, which \
         this installation cannot ask for: its authorization helper \
         (midnight-elevated-listing.exe) is missing. Reinstall Midnight Explorer to restore it."
            .into(),
    );
    ui::draw(
        &native,
        560,
        DIALOG_HEIGHT,
        "authorization-offer-unavailable",
    );
    assert!(!shown(AccessibleRole::Text, HINT));
    assert!(!shown(AccessibleRole::Button, "Retry with authorization"));
    assert!(shown(AccessibleRole::Button, "Close"));
}
