//! The three places a line still moved, driven through the REAL elements.
//!
//! `tests/tabular_figures.rs` measures the figure box itself. This file
//! measures the elements a widget actually calls, because a mechanism that
//! works and is not reached is a mechanism nobody has.
//!
//! Three claims, each with the control that makes it non-vacuous:
//!
//! * **The clock stands still.** `clock.rhai` draws the time through
//!   `ui::runs` and nothing else. `runs` measured and drew `FONT_UI`
//!   proportionally whatever role its items named, so `11:11:11` and
//!   `88:88:88` came to different widths and a centred line jumped
//!   sideways on the tick — the very symptom the owner reported.
//! * **The `columns` strip stands still.** SYSINFO's readings are a
//!   `columns` cell, `script.columns_value_role` is `value`, and `value`
//!   is tabular; the strip drew proportionally regardless.
//! * **Prose is not spaced out.** `script.rows_value_role` is ONE binding
//!   for every widget, so HARDWARE's machine names are set in the same
//!   tabular `value` role as NETWORK's addresses. `num.tabular_punct`
//!   pins a SPACE into the figure box, and a figure box is half again as
//!   wide as a space — so unifying the ladder stretched every gap in
//!   `THINKPAD X1 CARBON GEN 9`. A mark is boxed where it is part of a
//!   NUMBER, and a space between two words is not.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::theme::TokenId;
use nacelle::ui::{
    self, Align, ColumnCell, ColumnsStyle, LabelWidth, Role, RowItem, RowsStyle, Run,
};
use nacelle::{Ctx, Rect};
use std::sync::OnceLock;

const W: f32 = 1920.0;
const H: f32 = 1080.0;

/// A pair of readings of the same shape: narrow figures against wide ones.
/// Any face whose digits are uniform would make every claim below vacuous,
/// which is why each one asserts its proportional control first.
const NARROW: &str = "11:11:11";
const WIDE: &str = "88:88:88";

/// The x of the left edge of every glyph quad, in draw order.
fn pen_stops(dl: &DrawList) -> Vec<f32> {
    dl.verts.chunks(6).map(|q| q[0].pos[0]).collect()
}

/// Draws through `f` with a frame's worth of context at the reference
/// viewport, and answers the glyph stops plus whatever `f` returned.
fn drawn<T>(f: impl FnOnce(&mut Ctx) -> T) -> (Vec<f32>, T) {
    let mut fonts = FontSystem::new();
    let mut dl = DrawList::new();
    let out = {
        let mut c = Ctx {
            access: None,
            dl: &mut dl,
            fonts: &mut fonts,
            w: W,
            h: H,
            t: 0.0,
            mouse: Pointer::new(-1.0, -1.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        };
        f(&mut c)
    };
    (pen_stops(&dl), out)
}

/// What the run WOULD measure with no figure box: the control every claim
/// here needs, taken in the role's own face and at the role's own size so
/// that it is the same question minus the one thing under test.
fn proportional(role: Role, text: &str) -> f32 {
    let mut fonts = FontSystem::new();
    let mut dl = DrawList::new();
    let c = Ctx {
        access: None,
        dl: &mut dl,
        fonts: &mut fonts,
        w: W,
        h: H,
        t: 0.0,
        mouse: Pointer::new(-1.0, -1.0),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    };
    let px = role.px(&c, 1.0);
    let track = role.tracking_px(px);
    let face = role.font();
    c.fonts.measure(face, px, text, track)
}

fn run_of(text: &str, role: Role) -> Run {
    Run { text: text.to_string(), role, sev: None, blink: None, end: false }
}

// ------------------------------------------------------------ the clock

#[test]
fn the_desktop_clock_does_not_jump_when_a_one_appears() {
    let role = ui::role("display.clock");
    assert!(role.tabular(), "type.display.clock.tabular — the master's own word");

    // The control. If the face's digits were uniform this test would prove
    // nothing, so it is an assertion and not a comment.
    let (loose_narrow, loose_wide) = (proportional(role, NARROW), proportional(role, WIDE));
    assert_ne!(
        loose_narrow, loose_wide,
        "proportional figures must differ, or the clock could not have jumped"
    );

    let line = Rect::new(0.0, 40.0, 480.0, 80.0);
    let width = |text: &'static str| {
        drawn(|c| ui::runs(c, line, &[run_of(text, role)], Align::Center, 1.0)).1
    };
    assert_eq!(
        width(NARROW),
        width(WIDE),
        "ui::runs draws a clock of two widths, so the line still moves"
    );

    // Width is not placement: a centred line is placed FROM its width, so
    // where the glyphs landed is the claim, not how wide they came to.
    //
    // The FIGURES themselves are deliberately not compared: a '1' is
    // centred in its box and an '8' fills it, so their ink starts at
    // different x by design — that is the box working. A SENTINEL after
    // the reading is what shows a run walking, and it is compared by
    // position in the list rather than by index: a face that fell back to
    // Regular for a >=600 weight draws every glyph twice, so a fixed index
    // into the quads is a claim about this machine's fonts.
    let last = |text: &str| {
        let owned = format!("{text}|");
        *drawn(move |c| ui::runs(c, line, &[run_of(&owned, role)], Align::Center, 1.0))
            .0
            .last()
            .expect("the clock drew nothing")
    };
    assert_eq!(
        last(NARROW),
        last(WIDE),
        "the glyph after a centred clock moved, so the line still walks"
    );
}

/// A line of runs is not only the clock: the `end` cluster is placed from
/// the right edge and the start cluster in the room it leaves, so a run
/// whose width moves drags its neighbour with it.
#[test]
fn a_reading_beside_a_label_does_not_drag_the_line() {
    let label = ui::role("caption");
    let reading = ui::role("value");
    let line = Rect::new(0.0, 40.0, 480.0, 40.0);
    // The reading FIRST and the word after it: the word is the neighbour
    // a run of changing width used to drag along, and a word is not a
    // figure, so every one of its glyphs is shared ground.
    let stops = |text: &'static str| {
        drawn(|c| {
            ui::runs(c, line, &[run_of(text, reading), run_of("LOAD", label)], Align::Left, 1.0)
        })
        .0
    };
    let (a, b) = (stops("1111"), stops("8888"));
    assert_eq!(a.len(), b.len());
    assert_eq!(
        a.last(),
        b.last(),
        "a reading of different digits moved the run beside it"
    );
}

// ----------------------------------------------------------- the strip

#[test]
fn a_columns_strip_does_not_creep() {
    static LABEL: OnceLock<TokenId> = OnceLock::new();
    static VALUE: OnceLock<TokenId> = OnceLock::new();
    let label_role = ui::bound_role(&LABEL, "script.columns_label_role");
    let value_role = ui::bound_role(&VALUE, "script.columns_value_role");
    assert!(value_role.tabular(), "script.columns_value_role must land on a tabular role");
    assert_ne!(
        proportional(value_role, NARROW),
        proportional(value_role, WIDE),
        "the control must differ or nothing is proved"
    );

    let strip = Rect::new(0.0, 0.0, 600.0, 60.0);
    let stops = |text: &'static str| {
        drawn(|c| {
            let st = ColumnsStyle {
                label_role,
                value_role,
                align: Some(Align::Left),
                dividers: false,
                shrink: 1.0,
            };
            let cells = [
                ColumnCell { label: "TIME".into(), value: text.into(), sev: None },
                ColumnCell { label: "MODE".into(), value: "AC".into(), sev: None },
            ];
            ui::columns(c, strip, &cells, &st);
        })
        .0
    };
    let (a, b) = (stops(NARROW), stops(WIDE));
    assert_eq!(a.len(), b.len(), "the two strips drew different glyph counts");
    // The strip is sized from its CONTENT, so a value that measures wider
    // pushes every cell after it. The last glyph of the SECOND cell — a
    // letter, in a cell holding no figure at all — is where that shows.
    assert_eq!(
        a.last(),
        b.last(),
        "a `columns` cell of different digits moved the cell beside it"
    );
}

// ------------------------------------------------------------ the prose

/// The regression unifying the ladder introduced, and the reason the
/// figure box reads a character's NEIGHBOURS.
///
/// HARDWARE's values are machine names. They are set in `value` — the one
/// role every `rows` value is set in — and `value` is tabular, so before
/// this rule every space in them was widened to a figure box.
#[test]
fn a_machine_name_is_not_spaced_out_by_a_figure_box() {
    static VALUE: OnceLock<TokenId> = OnceLock::new();
    let role = ui::bound_role(&VALUE, "script.rows_value_role");
    assert!(role.tabular(), "the case only exists because the value role IS tabular");
    let boxed = |text: &str| {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::new();
        let c = Ctx {
            access: None,
            dl: &mut dl,
            fonts: &mut fonts,
            w: W,
            h: H,
            t: 0.0,
            mouse: Pointer::new(-1.0, -1.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        };
        let px = role.px(&c, 1.0);
        let track = role.tracking_px(px);
        let face = role.font();
        let fig = role.figures(c.fonts, face, px);
        c.fonts.measure_fig(face, px, text, track, &fig)
    };

    // The regression is about the GAPS, so the gaps are what is measured:
    // a name's width less the same name with its spaces taken out is the
    // spaces, and it must be the face's space and not the figure box.
    //
    // Stated this way rather than as "the whole string is untouched",
    // because the whole string is NOT untouched and should not be: a
    // figure inside a name is still a figure and still gets its box, so
    // `AMD RYZEN 7` and `AMD RYZEN 9` come to the same width. That is the
    // box working. What went wrong was the spaces.
    let space = {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::new();
        let c = Ctx {
            access: None,
            dl: &mut dl,
            fonts: &mut fonts,
            w: W,
            h: H,
            t: 0.0,
            mouse: Pointer::new(-1.0, -1.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        };
        let px = role.px(&c, 1.0);
        let track = role.tracking_px(px);
        c.fonts.measure(role.font(), px, " ", track)
    };
    assert!(space > 0.0, "the face must have a space, or nothing below is measured");
    for name in [
        "AMD RYZEN 7",
        "THINKPAD X1 CARBON GEN 9",
        "LENOVO",
        "INTEL CORE I7",
        "12 GB DDR4",
    ] {
        let gaps = name.chars().filter(|c| *c == ' ').count() as f32;
        let tight: String = name.chars().filter(|c| *c != ' ').collect();
        let widened = boxed(name) - boxed(&tight);
        assert!(
            (widened - gaps * space).abs() < 0.01,
            "\"{name}\": its {gaps} word gaps measure {widened} px where the \
             face's space is {space} px — the figure box is still holding them"
        );
    }
    // And a name that differs only in a figure is one width, which is the
    // half of the rule the gaps must not have taken away.
    assert_eq!(boxed("AMD RYZEN 7"), boxed("AMD RYZEN 1"));

    // And the box is still on: the same role, given figures, still holds
    // them to one width. Without this the test above would pass on a role
    // whose box was simply switched off.
    assert_eq!(boxed("192.168.1.1"), boxed("888.888.8.8"));
    assert_ne!(
        proportional(role, "192.168.1.1"),
        proportional(role, "888.888.8.8"),
        "the control must differ or the address proves nothing"
    );
    // A space INSIDE a number is grouping and keeps its box, which is the
    // half of the rule the prose case must not have taken away.
    assert_eq!(boxed("1 234 567"), boxed("8 888 888"));
    assert_ne!(proportional(role, "1 234 567"), proportional(role, "8 888 888"));
}

/// The same claim through the element rather than through the box: a
/// `rows` line of prose draws the same glyphs in the same places as the
/// proportional run it used to be.
#[test]
fn a_rows_line_of_prose_draws_where_it_always_did() {
    static LABEL: OnceLock<TokenId> = OnceLock::new();
    static VALUE: OnceLock<TokenId> = OnceLock::new();
    let label_role = ui::bound_role(&LABEL, "script.rows_label_role");
    let value_role = ui::bound_role(&VALUE, "script.rows_value_role");
    let block = Rect::new(0.0, 0.0, 640.0, 120.0);
    let stops = |value: &str| {
        let owned = value.to_string();
        drawn(move |c| {
            let st = RowsStyle {
                label_role,
                value_role,
                columns: 1,
                label_width: LabelWidth::Max,
                row_h: 24.0,
                shrink: 1.0,
            };
            let rows = [RowItem { label: "CPU".into(), value: owned, sev: None }];
            ui::rows_label_value(c, block, &rows, &st);
        })
        .0
    };
    // Two machine names of the same length in letters but different in
    // FIGURES: under a box that swallowed spaces these two came out at
    // different widths, which is a value column that shifts with the
    // hardware it is describing. The SENTINEL after them is the claim —
    // the figure itself is centred in its box and is meant to sit
    // differently, and a fixed index into the quads would be a claim about
    // whether this machine's fonts made the toolkit fake a weight.
    let a = stops("AMD RYZEN 7|");
    let b = stops("AMD RYZEN 1|");
    assert_eq!(a.len(), b.len());
    assert_eq!(a.last(), b.last(), "the words moved with the digit");
}
