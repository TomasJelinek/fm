//! Red cases of the UI polish gate (scenario `ui_polish_checks`): every check runs on a
//! deliberately broken fixture window and must flag it, and on the corrected twin of
//! that window and must not flag anything. A check that cannot fail here is broken.
use crate::{
    Context, Scenario,
    app::{App, Config},
    scenarios::ui_gate::{self as gate, Frame, Issue, Target},
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
                for i in 10: Text { x: 4px; y: i * 28px; text: "Outcome note " + (i + 1); color: #000000; }
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
            for i in 4: Text { x: 4px; y: i * 26px; text: "Note " + (i + 1); color: #000000; }
        }

        // A caption clipped by its own element without an ellipsis.
        if root.case == 4: Text {
            x: 12px; y: 40px; width: 90px; height: 20px; color: #000000;
            text: "A caption that does not fit here";
            overflow: root.broken ? TextOverflow.clip : TextOverflow.elide;
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
                x: 4px; y: 4px; width: 250px; height: 24px;
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

        // Overflow without a scroll bar that does scroll with the wheel (the twin
        // passes the check because it scrolls; the broken one swallows the wheel).
        if root.case == 20: Rectangle {
            x: 12px; y: 40px; width: 220px; height: 100px;
            Flickable {
                x: 0px; y: 0px; width: 200px; height: 100px;
                content-width: 200px; content-height: 300px;
                for i in 10: Text { x: 4px; y: i * 28px; text: "Row " + (i + 1); color: #000000; }
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

const CASES: [Case; 20] = [
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
];

/// Case 11 must also trigger these.
const ROW_KINDS: [&str; 2] = [gate::BUTTON_SPACING, gate::DISMISS_ORDER];

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
        area: "ui",
        summary: "Every UI polish check flags a deliberately broken window and passes its corrected twin (red cases).",
        run: checks,
        timeout: Duration::from_secs(10 * 60),
    }
}
