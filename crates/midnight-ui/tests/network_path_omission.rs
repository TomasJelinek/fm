//! Polish batch 3: the path bar's omission marker names what it stands for. In Network,
//! whose path editor holds only the location's own caption, its tooltip is the chain
//! of captions from the outermost location through the last one it omits (the omitted
//! ancestors with the chain that leads to them), not that caption. On a native or
//! archive path the editor holds the whole path, and the tooltip stays that path.
use crate::support::ui;

use i_slint_core::{
    accessibility::AccessibleStringProperty,
    item_tree::ItemRc,
    items::{AccessibleRole, ComplexText, SimpleText},
    window::{WindowInner, WindowKind},
};
use midnight_core::*;
use midnight_ui::{PathPartRow, Presenter, install, refresh};
use slint::{ComponentHandle, Model, ModelRc, VecModel, platform::WindowEvent};
use std::{cell::Cell, cell::RefCell, rc::Rc, sync::Arc, time::Duration};

const MARKER: &str = "Edit full folder path";

/// A server below a workgroup, a provider and the root (0), with captions long enough
/// that a narrow panel shortens the path.
fn chain_address(level: u8) -> NetworkAddress {
    let captions = [
        "Network",
        "Microsoft Windows Network",
        "A rather long workgroup name 文档",
        "SERVER-0042",
    ];
    NetworkAddress {
        provider: "network.test".into(),
        opaque: Arc::from([7, level]),
        display: captions[usize::from(level)].into(),
    }
}

fn walk(item: &ItemRc, found: &mut dyn FnMut(&ItemRc)) {
    found(item);
    let mut child = item.first_child();
    while let Some(current) = child {
        child = current.next_sibling();
        walk(&current, found);
    }
}

fn label(item: &ItemRc) -> String {
    item.accessible_string_property(AccessibleStringProperty::Label)
        .map(|label| label.to_string())
        .unwrap_or_default()
}

/// The left path bar's shown buttons whose label passes `keep`, left to right: the
/// bar lies above the left list and inside its columns.
fn left_bar(test: &ui::TestUi, keep: &dyn Fn(&str) -> bool) -> Vec<(String, ItemRc)> {
    let mut found = Vec::new();
    walk(
        &ItemRc::new_root(WindowInner::from_pub(test.window.window()).component()),
        &mut |item| {
            if item.is_visible()
                && item.accessible_role() == AccessibleRole::Button
                && keep(&label(item))
            {
                found.push((label(item), item.clone()));
            }
        },
    );
    let left = test.window.get_left_list_left();
    let right = left + test.window.get_left_list_width();
    let top = test.window.get_left_list_top();
    let origin = |item: &ItemRc| item.map_to_native_window(item.geometry().origin);
    found.retain(|(_, item)| {
        let at = origin(item);
        at.x >= left - 0.5 && at.x < right && at.y < top
    });
    found.sort_by(|(_, a), (_, b)| origin(a).x.total_cmp(&origin(b).x));
    found
}

/// The texts of the tooltip that resting the pointer on `item` shows.
fn tooltip_of(test: &mut ui::TestUi, clock: &Cell<Duration>, item: &ItemRc) -> Vec<String> {
    let window = test.window.window();
    window.dispatch_event(WindowEvent::PointerExited);
    ui::advance(clock, Duration::from_millis(200));
    test.draw();
    let rect = item.geometry();
    let origin = item.map_to_native_window(rect.origin);
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerMoved {
            position: slint::LogicalPosition::new(
                origin.x + rect.width() / 2.0,
                origin.y + rect.height() / 2.0,
            ),
        });
    ui::advance(clock, Duration::from_millis(1200));
    test.draw();
    let mut texts = Vec::new();
    for popup in WindowInner::from_pub(test.window.window())
        .active_popups()
        .iter()
    {
        if matches!(popup.window_kind, WindowKind::ToolTip) {
            walk(&ItemRc::new_root(popup.component.clone()), &mut |item| {
                if let Some(text) = item.downcast::<ComplexText>() {
                    texts.push(text.as_pin_ref().text().to_string());
                } else if let Some(text) = item.downcast::<SimpleText>() {
                    texts.push(text.as_pin_ref().text().to_string());
                }
            });
        }
    }
    test.window
        .window()
        .dispatch_event(WindowEvent::PointerExited);
    ui::advance(clock, Duration::from_millis(200));
    test.draw();
    texts.retain(|text| !text.trim().is_empty());
    texts
}

#[test]
fn the_network_omission_marker_names_the_omitted_ancestors() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let presenter = Rc::new(RefCell::new(Presenter::new(root.clone(), root)));
    presenter
        .borrow_mut()
        .apply(Event::PreferencesLoaded(Ok(Box::default())));
    let calls = Rc::new(RefCell::new(Vec::new()));
    let captured = calls.clone();
    install(
        &test.window,
        presenter.clone(),
        Rc::new(move |command| {
            captured.borrow_mut().push(command);
            true
        }),
    );
    presenter
        .borrow_mut()
        .apply(Event::NetworkAvailability(true));
    refresh(&test.window, &presenter.borrow());
    let network = test
        .window
        .get_navigation_choices()
        .iter()
        .find(|row| row.kind == midnight_ui::LocationRole::Network)
        .unwrap();
    test.window.invoke_navigation_choice(0, network.identity);
    let navigation = calls
        .borrow()
        .iter()
        .find_map(|command| match command {
            Command::NetworkNavigate { navigation, .. } => Some(*navigation),
            _ => None,
        })
        .expect("the menu opens Network");
    presenter.borrow_mut().apply(Event::NetworkUpdated {
        pane: PaneId::Left,
        navigation,
        update: Box::new(NetworkUpdate::Listing {
            listing: NetworkListing {
                address: chain_address(3),
                ancestors: (0..3).map(chain_address).collect(),
                entries: Arc::default(),
                index: Arc::default(),
            },
            done: true,
            error: None,
        }),
    });
    // The bar joins the captions; each part opens its chain (`Open <chain>`).
    let chain: Vec<String> = (0..4)
        .map(|level| {
            (0..=level)
                .map(|level| chain_address(level).display)
                .collect::<Vec<_>>()
                .join(" › ")
        })
        .collect();
    // The editor holds only the server's caption.
    test.window.set_panel_ratio(0.0);
    refresh(&test.window, &presenter.borrow());
    test.draw();
    assert_eq!(test.window.get_left_path(), "SERVER-0042");
    let shown: Vec<String> = left_bar(&test, &|label| label.starts_with("Open "))
        .into_iter()
        .map(|(label, _)| label["Open ".len()..].to_owned())
        .collect();
    let omitted: Vec<usize> = (0..chain.len())
        .filter(|&level| !shown.contains(&chain[level]))
        .collect();
    assert!(
        !omitted.is_empty() && omitted.windows(2).all(|pair| pair[1] == pair[0] + 1),
        "a narrow panel omits one run of ancestors: shown {shown:?}"
    );
    let markers = left_bar(&test, &|label| label == MARKER);
    assert_eq!(markers.len(), 1, "one marker stands for the omitted run");
    let tip = tooltip_of(&mut test, &clock, &markers[0].1);
    let last = *omitted.last().unwrap();
    assert!(
        tip.contains(&chain[last]),
        "the marker's tooltip is the chain through the last omitted ancestor {:?}: {tip:?}",
        chain[last]
    );
    for level in omitted {
        let caption = chain_address(level as u8).display;
        assert!(
            tip.iter().any(|text| text.contains(&caption)),
            "the tooltip names the omitted {caption:?}: {tip:?}"
        );
    }
}

/// A shortened native path, and an archive path whose archive (the anchor) stands
/// between two markers because not even the suffix from it fits: the editor holds the
/// whole path, and every marker's tooltip is that path, as before.
#[test]
fn native_and_archive_omission_markers_still_show_the_whole_path() {
    let (mut test, clock) = ui::TestUi::with_virtual_clock();
    let rows = |labels: &[&str]| {
        let mut prefix = String::new();
        let parts: Vec<PathPartRow> = labels
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
            .collect();
        (prefix, parts)
    };
    for (case, labels, anchor, expected_markers) in [
        (
            "native",
            vec![
                "C:\\",
                "Users\\",
                "Owner\\",
                "Documents\\",
                "a-rather-long-project-directory\\",
                "日本語-sources-of-the-project\\",
                "november-polish-gate-review\\",
                "screenshots-and-findings\\",
                "main-window-states",
            ],
            -1,
            1..=2,
        ),
        (
            "archive",
            vec![
                "/",
                "home",
                "/owner",
                "/文档.zip",
                "/the-first-folder-inside-the-archive-with-a-long-name",
                "/the-second-folder-inside-the-archive-with-a-long-name",
                "/x",
            ],
            3,
            2..=2,
        ),
    ] {
        let (path, parts) = rows(&labels);
        test.window.set_left_path(path.as_str().into());
        test.window
            .set_left_path_parts(ModelRc::new(VecModel::from(parts)));
        test.window.set_left_path_anchor(anchor);
        test.window.set_panel_ratio(0.0);
        test.draw();
        let markers = left_bar(&test, &|label| label == MARKER);
        assert!(
            expected_markers.contains(&markers.len()),
            "{case}: the shortened path shows {} markers",
            markers.len()
        );
        for (_, marker) in markers {
            let tip = tooltip_of(&mut test, &clock, &marker);
            assert!(
                tip.contains(&path),
                "{case}: the marker's tooltip is the whole path {path:?}: {tip:?}"
            );
        }
    }
}
