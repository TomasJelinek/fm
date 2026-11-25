//! Red cases of the UI polish gate (scenario `ui_polish_checks`): every check runs on a
//! deliberately broken fixture window and must flag it, and on the corrected twin of
//! that window and must not flag anything. A check that cannot fail here is broken.
use crate::{
    Context, Scenario,
    app::{App, Config},
    scenarios::{
        ui_align as align,
        ui_gate::{self as gate, Frame, Issue, Target},
    },
};
use slint::{ComponentHandle, platform::software_renderer::MinimalSoftwareWindow};
use std::{rc::Rc, time::Duration};

slint::slint! {
    // One window that shows one deliberately broken (or corrected) case at a time.
    export component RedFixture inherits Window {
        in property <int> case;
        in property <bool> broken: true;
        in property <bool> framed: true;
        callback closed();
        no-frame: !root.framed;
        title: "Red case";
        background: #ffffff;
        default-font-size: 14px;
        preferred-width: 520px;
        preferred-height: 360px;

        if !(root.case == 14 && root.broken): Text {
            x: 12px; y: 10px; text: "Red case " + root.case; color: #000000; font-size: 16px;
        }

        // A close button of its own in a framed window (the corrected twin is frameless).
        if root.case == 1: Rectangle {
            x: parent.width - 40px; y: 6px; width: 30px; height: 30px; background: #e0e0e0;
            accessible-role: button; accessible-label: "Close dialog";
            Rectangle { x: 9px; y: 9px; width: 12px; height: 12px; background: #202020; }
            TouchArea { clicked => { root.closed(); } }
            Tooltip { text: @markdown("Close"); }
        }

        // Overflowing notes without a scroll bar whose wheel is swallowed; the twin
        // shows a thumb beside the list.
        if root.case == 2: Rectangle {
            x: 12px; y: 40px; width: 220px; height: 100px;
            Flickable {
                x: 0px; y: 0px; width: 200px; height: 100px;
                content-width: 200px; content-height: 300px;
                for i in 10: Text { x: 0px; y: i * 28px; text: "Outcome note " + (i + 1); color: #000000; }
            }
            if root.broken: TouchArea {
                x: 0px; y: 0px; width: 220px; height: 100px;
                scroll-event(event) => { accept }
            }
            if !root.broken: Rectangle { x: 194px; y: 4px; width: 4px; height: 30px; background: #707070; }
        }

        // Notes cut off by a clipping container that does not scroll.
        if root.case == 3: Rectangle {
            x: 12px; y: 40px; width: 220px; height: root.broken ? 40px : 120px; clip: true;
            for i in 4: Text { x: 0px; y: i * 26px; text: "Note " + (i + 1); color: #000000; }
        }

        // A caption clipped by its own element without an ellipsis (in a box with no
        // room beside it: the twin's "…" is needed).
        if root.case == 4: Rectangle {
            x: 12px; y: 40px; width: 106px; height: 20px;
            Text {
                x: 0px; y: 0px; width: 90px; height: 20px; color: #000000;
                text: "A caption that does not fit here";
                overflow: root.broken ? TextOverflow.clip : TextOverflow.elide;
            }
        }

        // An elided caption whose full text nothing provides (the twin has a tooltip).
        if root.case == 5: Rectangle {
            x: 12px; y: 40px; width: 90px; height: 20px;
            Text {
                x: 0px; y: 0px; width: 90px; height: 20px; color: #000000;
                text: "Another long caption here"; overflow: elide; accessible-role: none;
            }
            if !root.broken: Tooltip { text: @markdown("Another long caption here"); }
        }

        // Wrapped text taller than its element.
        if root.case == 6: Text {
            x: 12px; y: 40px; width: 110px; height: root.broken ? 20px : 140px; color: #000000;
            wrap: word-wrap; text: "Several words that wrap onto more lines than fit";
        }

        // A button without an accessible name.
        if root.case == 7: Rectangle {
            x: 12px; y: 40px; width: 90px; height: 28px; background: #d0d0d0;
            accessible-role: button; accessible-label: root.broken ? "" : "Apply";
            Text {
                x: 0px; y: 0px; width: parent.width; height: parent.height;
                horizontal-alignment: center; vertical-alignment: center;
                text: "Apply"; color: #000000; accessible-role: none;
            }
            TouchArea {}
        }

        // An icon-only button without a tooltip.
        if root.case == 8: Rectangle {
            x: 12px; y: 40px; width: 30px; height: 30px; background: #d0d0d0;
            accessible-role: button; accessible-label: "Refresh";
            Rectangle { x: 9px; y: 9px; width: 12px; height: 12px; background: #202020; }
            TouchArea {}
            if !root.broken: Tooltip { text: @markdown("Refresh"); }
        }

        // Two texts drawn over each other.
        if root.case == 9: Rectangle {
            x: 12px; y: 40px; width: 300px; height: 60px;
            Text { x: 0px; y: 0px; text: "Copied 12 files"; color: #000000; }
            Text {
                x: root.broken ? 20px : 160px; y: root.broken ? 4px : 0px;
                text: "Remaining 3"; color: #000000;
            }
        }

        // A text that runs out of the window.
        if root.case == 10: Text {
            x: root.broken ? parent.width - 40px : 12px; y: 40px; width: 150px;
            text: "Outside the window"; color: #000000;
        }

        // A footer row: uneven heights and gaps, Cancel first.
        if root.case == 11: Rectangle {
            x: 12px; y: 280px; width: 480px; height: 40px;
            Rectangle {
                x: 0px; y: root.broken ? 2px : 5px; width: 80px; height: root.broken ? 36px : 30px;
                background: #d0d0d0;
                accessible-role: button; accessible-label: root.broken ? "Cancel" : "Apply";
                Text {
                    x: 0px; y: 0px; width: parent.width; height: parent.height;
                    horizontal-alignment: center; vertical-alignment: center;
                    text: root.broken ? "Cancel" : "Apply"; color: #000000; accessible-role: none;
                }
            }
            Rectangle {
                x: 88px; y: 5px; width: 80px; height: 30px; background: #d0d0d0;
                accessible-role: button; accessible-label: "OK";
                Text {
                    x: 0px; y: 0px; width: parent.width; height: parent.height;
                    horizontal-alignment: center; vertical-alignment: center;
                    text: "OK"; color: #000000; accessible-role: none;
                }
            }
            Rectangle {
                x: root.broken ? 184px : 176px; y: 5px; width: 80px; height: 30px;
                background: #d0d0d0;
                accessible-role: button; accessible-label: root.broken ? "Apply" : "Cancel";
                Text {
                    x: 0px; y: 0px; width: parent.width; height: parent.height;
                    horizontal-alignment: center; vertical-alignment: center;
                    text: root.broken ? "Apply" : "Cancel"; color: #000000; accessible-role: none;
                }
            }
        }

        // Grey text on a grey surface.
        if root.case == 12: Rectangle {
            x: 12px; y: 40px; width: 240px; height: 40px; background: #808080;
            Text {
                x: 8px; y: 10px; text: "Low contrast note";
                color: root.broken ? #9a9a9a : #000000;
            }
        }

        // Selected text whose selection colours barely differ.
        if root.case == 13: Rectangle {
            x: 12px; y: 40px; width: 260px; height: 32px; background: #ffffff;
            TextInput {
                x: 0px; y: 4px; width: 250px; height: 24px;
                text: "Selected words in a viewer"; color: #000000; read-only: true;
                selection-foreground-color: root.broken ? #b8b8b8 : #ffffff;
                selection-background-color: root.broken ? #c8c8c8 : #1a5fb4;
                accessible-label: "Viewer text";
                init => { self.select-all(); }
            }
        }

        // 14: a blank window (the twin shows its caption).

        // A control half outside the window.
        if root.case == 15: Rectangle {
            x: root.broken ? parent.width - 30px : 12px; y: 40px; width: 90px; height: 28px;
            background: #d0d0d0;
            accessible-role: button; accessible-label: "Apply";
            Text {
                x: 0px; y: 0px; width: parent.width; height: parent.height;
                horizontal-alignment: center; vertical-alignment: center;
                text: "Apply"; color: #000000; accessible-role: none;
            }
        }

        // Two buttons on top of each other.
        if root.case == 16: Rectangle {
            x: 12px; y: 40px; width: 300px; height: 40px;
            Rectangle {
                x: 0px; y: 0px; width: 90px; height: 28px; background: #d0d0d0;
                accessible-role: button; accessible-label: "Apply";
                Text {
                    x: 0px; y: 0px; width: parent.width; height: parent.height;
                    horizontal-alignment: center; vertical-alignment: center;
                    text: "Apply"; color: #000000; accessible-role: none;
                }
            }
            Rectangle {
                x: root.broken ? 60px : 100px; y: 0px; width: 90px; height: 28px;
                background: #d0d0d0;
                accessible-role: button; accessible-label: "Reset";
                Text {
                    x: 0px; y: 0px; width: parent.width; height: parent.height;
                    horizontal-alignment: center; vertical-alignment: center;
                    text: "Reset"; color: #000000; accessible-role: none;
                }
            }
        }

        // A label of no width.
        if root.case == 17: Text {
            x: 12px; y: 40px; width: root.broken ? 0px : 140px; height: 20px;
            text: "Hidden caption"; color: #000000;
        }

        // A label elided to nothing.
        if root.case == 18: Text {
            x: 12px; y: 40px; width: root.broken ? 6px : 100px; height: 20px;
            text: "Caption"; overflow: elide; color: #000000;
        }

        // A button caption wider than its button.
        if root.case == 19: Rectangle {
            x: 12px; y: 40px; width: root.broken ? 50px : 140px; height: 28px; background: #d0d0d0;
            accessible-role: button; accessible-label: "Long caption";
            Text {
                x: 0px; y: 0px; width: parent.width; height: parent.height;
                horizontal-alignment: center; vertical-alignment: center;
                text: "Long caption"; overflow: elide; color: #000000; accessible-role: none;
            }
        }

        // Selected text that meets WCAG AA (white on #1a5fb4, 6.3:1) but not a contrast
        // palette's 7:1 or a high-contrast palette's 12:1; the twin's selection is
        // #001a40 (17:1).
        if root.case == 21: Rectangle {
            x: 12px; y: 40px; width: 260px; height: 32px; background: #ffffff;
            TextInput {
                x: 0px; y: 4px; width: 250px; height: 24px;
                text: "Selected words in a field"; color: #000000; read-only: true;
                selection-foreground-color: #ffffff;
                selection-background-color: root.broken ? #1a5fb4 : #001a40;
                accessible-label: "Field text";
                init => { self.select-all(); }
            }
        }

        // A field and its "…" button as the owner saw them in Checksums › Verify: the
        // button taller than the field and its dots drawn low; the twin is square, as
        // tall as the field, its glyph centred.
        if root.case == 22: Rectangle {
            x: 12px; y: 40px; width: 400px; height: 40px;
            Rectangle {
                x: 0px; y: 0px; width: 300px; height: 24px; background: #ffffff;
                border-width: 1px; border-color: #606060;
                accessible-role: text-input; accessible-label: "Checksum file";
                Text {
                    x: 6px; y: 0px; width: 288px; height: 24px; vertical-alignment: center;
                    text: "/data/checksums/list.sha256"; color: #000000; accessible-role: none;
                }
            }
            Rectangle {
                x: 308px; y: 0px; width: 24px; height: root.broken ? 32px : 24px;
                background: #d0d0d0;
                accessible-role: button; accessible-label: "Browse checksum file";
                if root.broken: Text {
                    x: 0px; y: 0px; width: parent.width; height: parent.height;
                    horizontal-alignment: center; vertical-alignment: center;
                    text: "…"; font-size: 20px; color: #000000; accessible-role: none;
                }
                if !root.broken: Rectangle { x: 7px; y: 7px; width: 10px; height: 10px; background: #202020; }
                TouchArea {}
                Tooltip { text: @markdown("Browse"); }
            }
        }

        // Two fields of one column whose left and right edges miss each other by 3 px.
        if root.case == 23: Rectangle {
            x: 12px; y: 40px; width: 400px; height: 80px;
            for i in 2: Rectangle {
                x: root.broken && i == 1 ? 3px : 0px; y: i * 32px; width: 300px; height: 24px;
                background: #ffffff; border-width: 1px; border-color: #606060;
                accessible-role: text-input; accessible-label: "Field " + (i + 1);
            }
        }

        // A column of options with one gap seven times the others (the last option, a
        // dialog's footer row, may sit lower).
        if root.case == 24: Rectangle {
            x: 12px; y: 40px; width: 400px; height: 300px;
            for i in 5: Rectangle {
                x: 0px; y: i * 26px + (root.broken && i >= 2 ? 40px : 0px); width: 200px; height: 20px;
                accessible-role: checkbox; accessible-label: "Option " + (i + 1);
                Rectangle { x: 2px; y: 3px; width: 14px; height: 14px; border-width: 1px; border-color: #404040; }
                Text { x: 24px; y: 0px; height: 20px; vertical-alignment: center;
                    text: "Option " + (i + 1); color: #000000; accessible-role: none; }
            }
        }

        // Two buttons 1 px apart (the twin keeps 8 px).
        if root.case == 25: Rectangle {
            x: 12px; y: 40px; width: 400px; height: 40px;
            for i in 2: Rectangle {
                x: i * (root.broken ? 81px : 88px); y: 0px; width: 80px; height: 28px;
                background: #d0d0d0;
                accessible-role: button; accessible-label: i == 0 ? "Apply" : "Reset";
                Text {
                    x: 0px; y: 0px; width: parent.width; height: parent.height;
                    horizontal-alignment: center; vertical-alignment: center;
                    text: i == 0 ? "Apply" : "Reset"; color: #000000; accessible-role: none;
                }
            }
        }

        // A label shortened to 80 px with 300 px free beside it.
        if root.case == 26: Rectangle {
            x: 12px; y: 40px; width: 400px; height: 24px;
            Text {
                x: 0px; y: 0px; width: root.broken ? 80px : 300px; height: 20px;
                text: "Current folder: projects"; overflow: elide; color: #000000;
            }
        }

        // A button whose caption's line sits 5 px below its middle.
        if root.case == 27: Rectangle {
            x: 12px; y: 40px; width: 90px; height: 30px; background: #d0d0d0;
            accessible-role: button; accessible-label: "Apply";
            Text {
                x: 0px; y: root.broken ? 10px : 5px; width: parent.width; height: 20px;
                horizontal-alignment: center; vertical-alignment: center;
                text: "Apply"; color: #000000; accessible-role: none;
            }
            TouchArea {}
        }

        // Options whose last gap is 8 px wider than the others (U-K39).
        if root.case == 28: Rectangle {
            x: 12px; y: 40px; width: 400px; height: 200px;
            for i in 4: Rectangle {
                x: 0px; y: i * 26px + (root.broken && i == 3 ? 8px : 0px); width: 200px; height: 20px;
                accessible-role: checkbox; accessible-label: "Choice " + (i + 1);
                Rectangle { x: 2px; y: 3px; width: 14px; height: 14px; border-width: 1px; border-color: #404040; }
                Text { x: 24px; y: 0px; height: 20px; vertical-alignment: center;
                    text: "Choice " + (i + 1); color: #000000; accessible-role: none; }
            }
        }

        // Options in a scrolled page whose last row is taller: its description wraps and
        // runs past the page's bottom edge, its check box centred beside it (Options ›
        // Archives on Linux). The twin keeps U-K39's exemption for a row that a wrapped
        // text makes taller; the broken window has no description and an 8 px wider gap.
        if root.case == 29: Rectangle {
            x: 12px; y: 40px; width: 400px; height: 112px;
            Flickable {
                x: 0px; y: 0px; width: 400px; height: 112px;
                content-width: 400px; content-height: 200px;
                for i in 4: Rectangle {
                    x: 0px; width: 120px; height: 20px;
                    y: i < 3 ? i * 26px : root.broken ? 86px : 88px;
                    accessible-role: checkbox; accessible-label: "Type " + (i + 1);
                    Rectangle { x: 2px; y: 3px; width: 14px; height: 14px; border-width: 1px; border-color: #404040; }
                    Text { x: 24px; y: 0px; height: 20px; vertical-alignment: center;
                        text: "Type " + (i + 1); color: #000000; accessible-role: none; }
                }
                if !root.broken: Text {
                    x: 130px; y: 78px; width: 220px; height: 40px; vertical-alignment: center;
                    wrap: word-wrap; text: "Android development\npackage"; color: #000000;
                }
            }
        }

        // Overflow without a scroll bar that does scroll with the wheel (the twin
        // passes the check because it scrolls; the broken one swallows the wheel).
        if root.case == 20: Rectangle {
            x: 12px; y: 40px; width: 220px; height: 100px;
            Flickable {
                x: 0px; y: 0px; width: 200px; height: 100px;
                content-width: 200px; content-height: 300px;
                for i in 10: Text { x: 0px; y: i * 28px; text: "Row " + (i + 1); color: #000000; }
            }
            if root.broken: TouchArea {
                x: 0px; y: 0px; width: 220px; height: 100px;
                scroll-event(event) => { accept }
            }
        }
    }
}

/// One static red case: the fixture case, the check it must trigger, and whether the
/// corrected twin is frameless.
struct Case {
    id: i32,
    kind: &'static str,
    twin_frameless: bool,
}

const CASES: [Case; 28] = [
    Case {
        id: 1,
        kind: gate::FRAMED_CLOSE,
        twin_frameless: true,
    },
    Case {
        id: 2,
        kind: gate::OVERFLOW,
        twin_frameless: false,
    },
    Case {
        id: 3,
        kind: gate::CUT_OFF,
        twin_frameless: false,
    },
    Case {
        id: 4,
        kind: gate::TEXT_CLIPPED,
        twin_frameless: false,
    },
    Case {
        id: 5,
        kind: gate::ELIDED_HIDDEN,
        twin_frameless: false,
    },
    Case {
        id: 6,
        kind: gate::WRAP_CUT,
        twin_frameless: false,
    },
    Case {
        id: 7,
        kind: gate::NO_NAME,
        twin_frameless: false,
    },
    Case {
        id: 8,
        kind: gate::NO_TOOLTIP,
        twin_frameless: false,
    },
    Case {
        id: 9,
        kind: gate::TEXT_OVERLAP,
        twin_frameless: false,
    },
    Case {
        id: 10,
        kind: gate::TEXT_OUTSIDE,
        twin_frameless: false,
    },
    Case {
        id: 11,
        kind: gate::BUTTON_HEIGHTS,
        twin_frameless: false,
    },
    Case {
        id: 12,
        kind: gate::CONTRAST,
        twin_frameless: false,
    },
    Case {
        id: 13,
        kind: gate::CONTRAST,
        twin_frameless: false,
    },
    Case {
        id: 14,
        kind: gate::BLANK,
        twin_frameless: false,
    },
    Case {
        id: 15,
        kind: "control clipped by window",
        twin_frameless: false,
    },
    Case {
        id: 16,
        kind: "overlapping controls",
        twin_frameless: false,
    },
    Case {
        id: 17,
        kind: "zero-size label",
        twin_frameless: false,
    },
    Case {
        id: 18,
        kind: "label elided to nothing",
        twin_frameless: false,
    },
    Case {
        id: 19,
        kind: "text does not fit its button",
        twin_frameless: false,
    },
    Case {
        id: 20,
        kind: gate::OVERFLOW,
        twin_frameless: false,
    },
    Case {
        id: 22,
        kind: align::ROW_ALIGNMENT,
        twin_frameless: false,
    },
    Case {
        id: 23,
        kind: align::EDGE_ALIGNMENT,
        twin_frameless: false,
    },
    Case {
        id: 24,
        kind: align::ROW_GAPS,
        twin_frameless: false,
    },
    Case {
        id: 25,
        kind: align::TOUCHING,
        twin_frameless: false,
    },
    Case {
        id: 26,
        kind: align::ELIDED_WITH_ROOM,
        twin_frameless: false,
    },
    Case {
        id: 27,
        kind: align::CAPTION_CENTRE,
        twin_frameless: false,
    },
    Case {
        id: 28,
        kind: align::OPTION_ROWS,
        twin_frameless: false,
    },
    Case {
        id: 29,
        kind: align::OPTION_ROWS,
        twin_frameless: false,
    },
];

/// Case 11 must also trigger these.
const ROW_KINDS: [&str; 2] = [gate::BUTTON_SPACING, gate::DISMISS_ORDER];

/// Case 22, the owner's "…" button, must also trigger these: not square, and its dots
/// drawn below its middle.
const COMPANION_KINDS: [&str; 2] = [align::COMPANION_SIZE, align::CAPTION_CENTRE];

/// The fixture's software window: the newest visible tool window once it is shown.
fn fixture_window(app: &App) -> Result<Rc<MinimalSoftwareWindow>, String> {
    app.tool_windows()
        .pop()
        .ok_or_else(|| "the red fixture window did not open".to_owned())
}

/// Every check the gate runs on a capture, wheel probe included.
pub fn issues_of(app: &mut App, window: &Rc<MinimalSoftwareWindow>) -> Vec<Issue> {
    let target = Target::Tool(window.clone());
    gate::settled_issues(app, &target)
}

fn summary(issues: &[Issue]) -> String {
    issues
        .iter()
        .map(|issue| format!("{}: {}", issue.kind, issue.detail))
        .collect::<Vec<_>>()
        .join("; ")
}

pub fn checks(context: &mut Context) -> Result<(), String> {
    let folder = context.fixture.path("docs");
    let mut app = App::start(Config::new(&folder, &folder));
    let fixture = RedFixture::new().map_err(|error| format!("red fixture: {error}"))?;
    fixture
        .show()
        .map_err(|error| format!("show the red fixture: {error}"))?;
    let window = fixture_window(&app)?;
    gate::size_tool(&window, (520.0, 360.0), 1.0);
    app.settle();
    let mut failures = Vec::new();
    for case in &CASES {
        fixture.set_case(case.id);
        fixture.set_framed(true);
        fixture.set_broken(true);
        app.settle();
        let broken = issues_of(&mut app, &window);
        let frame = Frame::of(&window);
        let name = format!("red-{:02}-broken", case.id);
        let caption = format!("Red case {}: {} (broken)", case.id, case.kind);
        gate::save(context, &frame, &name, &caption);
        let mut wanted = vec![case.kind];
        if case.id == 11 {
            wanted.extend(ROW_KINDS);
        }
        if case.id == 22 {
            wanted.extend(COMPANION_KINDS);
        }
        for kind in wanted {
            if !broken.iter().any(|issue| issue.kind == kind) {
                failures.push(format!(
                    "case {} ({kind}): the broken window was not flagged; found: {}",
                    case.id,
                    summary(&broken)
                ));
            }
        }
        fixture.set_broken(false);
        fixture.set_framed(!case.twin_frameless);
        app.settle();
        let fixed = issues_of(&mut app, &window);
        if !fixed.is_empty() {
            failures.push(format!(
                "case {} ({}): the corrected twin was flagged: {}",
                case.id,
                case.kind,
                summary(&fixed)
            ));
        }
        context.note(format!(
            "red case {} {}: broken → {}; corrected twin → {} flag(s)",
            case.id,
            case.kind,
            summary(&broken),
            fixed.len()
        ));
    }
    // Case 21: selected text judged by the palette (G2). The broken selection passes
    // WCAG AA (the control: no contrast flag at 4.5:1), and must be flagged at the
    // contrast palettes' 7:1 and the high-contrast palettes' 12:1; its twin passes both.
    fixture.set_case(21);
    fixture.set_framed(true);
    fixture.set_broken(true);
    app.settle();
    let target = Target::Tool(window.clone());
    let frame = Frame::of(&window);
    gate::save(
        context,
        &frame,
        "red-21-broken",
        "Red case 21: selected text below a contrast palette's minimum (broken)",
    );
    let at_aa = gate::settled_issues_at(&mut app, &target, 4.5);
    if at_aa.iter().any(|issue| issue.kind == gate::CONTRAST) {
        failures.push(format!(
            "case 21 (control): selected text at 6.3:1 was flagged at WCAG AA: {}",
            summary(&at_aa)
        ));
    }
    let mut noted = Vec::new();
    for minimum in [7.0, 12.0] {
        fixture.set_broken(true);
        app.settle();
        let broken = gate::settled_issues_at(&mut app, &target, minimum);
        if !broken
            .iter()
            .any(|issue| issue.kind == gate::CONTRAST && issue.detail.contains("selected text"))
        {
            failures.push(format!(
                "case 21 ({}, selected text at {minimum}:1): the broken window was not flagged; found: {}",
                gate::CONTRAST,
                summary(&broken)
            ));
        }
        fixture.set_broken(false);
        app.settle();
        let fixed = gate::settled_issues_at(&mut app, &target, minimum);
        if !fixed.is_empty() {
            failures.push(format!(
                "case 21 ({}, selected text at {minimum}:1): the corrected twin was flagged: {}",
                gate::CONTRAST,
                summary(&fixed)
            ));
        }
        noted.push(format!(
            "at {minimum}:1 broken → {}; corrected twin → {} flag(s)",
            summary(&broken),
            fixed.len()
        ));
    }
    context.note(format!(
        "red case 21 {} (selected text by palette): at 4.5:1 broken → {} flag(s); {}",
        gate::CONTRAST,
        at_aa.len(),
        noted.join("; ")
    ));
    let _ = fixture.hide();
    app.settle();
    if let Err(error) = super::ui_interact::red_cases(&mut app, context) {
        failures.push(error);
    }
    app.shutdown()?;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "{} red case(s) not proven:\n{}",
            failures.len(),
            failures.join("\n")
        ))
    }
}

/// The registration of `ui_polish_checks` (`scenarios::all`).
pub fn scenario() -> Scenario {
    Scenario {
        name: "ui_polish_checks",
        isolation: crate::Isolation::Parallel,
        area: "ui",
        summary: "Every UI polish check flags a deliberately broken window and passes its corrected twin (red cases).",
        run: checks,
        timeout: Duration::from_secs(10 * 60),
    }
}
