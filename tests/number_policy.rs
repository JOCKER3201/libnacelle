//! `[num]` decides how a reading is written down — measured on the
//! instrument the master names.
//!
//! §5.17 opens with a sentence about itself: **"THE THEME DECIDES, not a
//! locale guess"**. Until 2026-08-17 two of its sixteen keys had a reader
//! and both were about the figure BOX; the reading itself came out of
//! `format!("{v:.0}%")` in `ui.rs`, so a theme could not move the decimal
//! mark, the thousands separator, the number of places or the letters of
//! the unit. This file asks the gauge — the one instrument the master's
//! own comments point at (`decimals_compact`: "temperatures, gauge
//! readouts") — what it draws under themes that differ in one key each.
//!
//! Every stage names ONE key and requires the drawing to follow it. The
//! master's own picture is measured first, so that a stage which changes
//! nothing fails instead of passing quietly.
//!
//! Half of §5.17 is typography that changes no CHARACTER of the reading
//! — the unit's size, spacing, baseline, ink and the gap before it — so
//! half of this file asks the draw register how the run was SET and not
//! merely what it says. A file that reads the strings alone cannot tell
//! a unit set from `[num]` from a unit set from five numbers written into
//! `ui.rs`, and its first cut could not.
//!
//! ONE test function, on purpose: the resolved theme is process-wide, so
//! a test that switches it must not run beside a test that reads it — the
//! same ruling `tests/gauge_role_bindings.rs` makes. The five chains
//! below are functions of that one test and not tests of their own.

use nacelle::draw::{DrawCmd, DrawList};
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::theme::{self, Color, LoadRequest};
use nacelle::ui::{self, GaugeKind, GaugeLabels, GaugeStyle, GaugeValueFmt};
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;

/// Runs one question on a thread of its own: the toolkit memoises text
/// tokens, resolved roles and enum words per THREAD and per epoch, and a
/// reload renumbers the open word sets a binding lives in.
fn fresh<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| s.spawn(f).join().expect("the drawing thread panicked"))
}

/// Loads the master, or a theme based on it that rewrites a few keys.
fn apply(fixture: Option<&str>) {
    match fixture {
        None => {
            let _ = theme::load();
        }
        Some(text) => {
            let path = std::env::temp_dir()
                .join(format!("nacelle-number-policy-{}.theme", std::process::id()));
            std::fs::write(&path, text).expect("the fixture theme must be writable");
            let _ = theme::load_with(LoadRequest { path: Some(path), ..Default::default() });
        }
    }
}

const HEAD: &str = "[meta]\nschema = 1\nname = \"Number policy fixture\"\nbase = \"default\"\n\n";

/// One drawn run and everything the register says about HOW it is set.
///
/// The strings alone are enough for the digits, and they are enough for
/// nothing else: `unit.scale`, `unit.tracking`, `unit.baseline_shift`,
/// `unit.color` and `unit.gap` are five keys that change no character of
/// the reading. A test reading only `text` cannot tell a unit set from
/// `[num]` from a unit set from five numbers written into `ui.rs`, which
/// is exactly the hole the first cut of this file left open.
#[derive(Clone, Debug)]
struct Run {
    text: String,
    at: [f32; 2],
    px: f32,
    tracking: f32,
    color: Color,
}

/// A length or scalar the loaded theme bakes `name` to.
fn scalar(name: &str) -> f32 {
    let owned = name.to_string();
    fresh(move || {
        theme::resolved().px(theme::id(&owned).unwrap_or_else(|| panic!("{owned} is not declared")))
    })
}

/// The ink the loaded theme bakes `name` to.
fn ink(name: &str) -> Color {
    let owned = name.to_string();
    fresh(move || {
        theme::resolved()
            .color(theme::id(&owned).unwrap_or_else(|| panic!("{owned} is not declared")))
    })
}

/// Every run a gauge block draws, in the order they were laid down. A
/// cell gauge draws its unit first and its number second — the run is
/// laid out from its right edge, because the unit hangs off the number's
/// end.
fn drawn(values: &[f32], fmt: GaugeValueFmt) -> Vec<Run> {
    let values = values.to_vec();
    fresh(move || {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::recording();
        {
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
            let st = GaugeStyle {
                cols: 1,
                kind: GaugeKind::Cell,
                labels: GaugeLabels::None,
                value_fmt: fmt,
                shrink: 1.0,
            };
            ui::gauge_grid(&mut c, Rect::new(40.0, 40.0, 400.0, 320.0), &values, &st);
        }
        dl.cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, at, px, tracking, color, .. } => Some(Run {
                    text: text.clone(),
                    at: *at,
                    px: *px,
                    tracking: *tracking,
                    color: *color,
                }),
                _ => None,
            })
            .collect()
    })
}

/// Every string a gauge block draws, for the claims that are about the
/// characters alone.
fn runs(values: &[f32], fmt: GaugeValueFmt) -> Vec<String> {
    drawn(values, fmt).into_iter().map(|r| r.text).collect()
}

/// The reading of a single-gauge block: the number run, without its unit.
fn number(values: &[f32], fmt: GaugeValueFmt) -> String {
    let all = runs(values, fmt);
    all.iter()
        .find(|s| !s.contains('%'))
        .cloned()
        .unwrap_or_else(|| panic!("no number run in {all:?}"))
}

/// The two runs of one percent gauge's reading — the number, then the
/// unit that hangs off its end.
fn pair(v: f32) -> (Run, Run) {
    let all = drawn(&[v], GaugeValueFmt::Percent);
    assert_eq!(all.len(), 2, "a percent reading is two runs: {all:?}");
    let unit = all
        .iter()
        .find(|r| r.text == "%")
        .unwrap_or_else(|| panic!("no unit run in {all:?}"))
        .clone();
    let num = all
        .iter()
        .find(|r| r.text != "%")
        .unwrap_or_else(|| panic!("no number run in {all:?}"))
        .clone();
    (num, unit)
}

// ------------------------------------------------------- the decimal mark

/// The owner's first question of `[num]`: a theme that writes `12,50`
/// gets `12,50`.
///
/// Three stages, because two would not separate the two keys involved.
/// The master writes its gauge readouts whole (`decimals_compact = 0`),
/// so a mark cannot show until a theme asks for a fraction — which is
/// itself the second key of this block with no reader before today.
#[test]
fn the_theme_decides_how_a_reading_is_written_down() {
    the_decimal_mark_and_how_many_places();
    where_the_thousands_open_up();
    the_unit_is_a_run_and_not_an_appended_character();
    the_unit_run_is_set_by_its_own_four_keys();
    the_joint_between_the_two_runs();
}

fn the_decimal_mark_and_how_many_places() {
    // ---- the master: a whole number and no mark ----------------------
    apply(None);
    let plain = number(&[12.5], GaugeValueFmt::Percent);
    assert_eq!(plain, "12", "the master's gauge readout is whole (decimals_compact = 0)");

    // ---- `decimals_compact` alone: the fraction appears ---------------
    apply(Some(&format!("{HEAD}[num]\ndecimals_compact = 2\n")));
    let with_places = number(&[12.5], GaugeValueFmt::Percent);
    assert_eq!(
        with_places, "12.50",
        "num.decimals_compact moved and the gauge readout did not follow it"
    );

    // ---- and the mark itself -----------------------------------------
    apply(Some(&format!("{HEAD}[num]\ndecimals_compact = 2\ndecimal_sep = \",\"\n")));
    let comma = number(&[12.5], GaugeValueFmt::Percent);
    assert_eq!(
        comma, "12,50",
        "num.decimal_sep = ',' did not reach the gauge readout — the mark is \
         still the one `format!` puts there"
    );
    assert_ne!(comma, with_places, "the two fixtures must differ or nothing is proved");

    apply(None);
}

// ----------------------------------------------------------- thousands

/// `num.group_min` is the length at which an integer starts being
/// grouped, and `num.group_sep` is what it is grouped with.
///
/// The master ships a minimum of five and a thin space, so `1234` stands
/// bare and `12345` becomes `12 345` — the exact pair the key's own
/// comment gives as its example.
fn where_the_thousands_open_up() {
    apply(None);
    let sep = theme::diagnostics().text("num.group_sep").unwrap_or_default().to_string();
    assert!(!sep.is_empty(), "the master ships a thousands separator");

    // Raw, because a percentage never reaches four figures — and the
    // block is about integers, not about the unit.
    let short = number(&[1234.0], GaugeValueFmt::Raw);
    assert_eq!(short, "1234", "four digits are under the master's group_min");
    let long = number(&[12345.0], GaugeValueFmt::Raw);
    assert_eq!(long, format!("12{sep}345"), "five digits are grouped");

    // ---- moved up: the same reading closes back up --------------------
    apply(Some(&format!("{HEAD}[num]\ngroup_min = 9\n")));
    assert_eq!(
        number(&[12345.0], GaugeValueFmt::Raw),
        "12345",
        "num.group_min moved and the grouping did not follow it"
    );

    // ---- and moved down: four digits open up --------------------------
    apply(Some(&format!("{HEAD}[num]\ngroup_min = 3\n")));
    assert_eq!(
        number(&[1234.0], GaugeValueFmt::Raw),
        format!("1{sep}234"),
        "num.group_min = 3 did not open up a four-digit reading"
    );

    // ---- the separator is the theme's too -----------------------------
    apply(Some(&format!("{HEAD}[num]\ngroup_sep = \"'\"\n")));
    assert_eq!(
        number(&[12345.0], GaugeValueFmt::Raw),
        "12'345",
        "num.group_sep did not reach the reading"
    );

    apply(None);
}

// ---------------------------------------------------------------- the unit

/// The unit is a RUN of its own — its own size, its own letters, its own
/// place — which is the half of §5.17 that a string could never carry.
fn the_unit_is_a_run_and_not_an_appended_character() {
    apply(None);

    // `unit.case` moves the LETTERS of a unit, and the percent sign has
    // none, so the claim is made through the byte formatter, whose units
    // are letters. The master ships `none`: a unit symbol is not a label,
    // and the small `i` of GiB is what makes it the IEC binary prefix.
    let shipped = fresh(|| nacelle::telemetry::fmt_bytes(2 * 1024 * 1024 * 1024));
    assert_eq!(shipped, "2.00 GiB", "the master's unit.case = none still cased the unit");

    apply(Some(&format!("{HEAD}[num]\nunit.case = upper\n")));
    let upper = fresh(|| nacelle::telemetry::fmt_bytes(2 * 1024 * 1024 * 1024));
    assert_eq!(upper, "2.00 GIB", "num.unit.case = upper did not reach the unit");
    assert_ne!(upper, shipped, "the two fixtures must differ or nothing is proved");

    // The joint between the two, where they are one string: a text token,
    // because a string carries no ems.
    apply(Some(&format!("{HEAD}[num]\nunit.text_gap = \"\"\n")));
    assert_eq!(
        fresh(|| nacelle::telemetry::fmt_bytes(2 * 1024 * 1024 * 1024)),
        "2.00GiB",
        "num.unit.text_gap did not close the joint"
    );

    apply(None);
    let both = runs(&[12.0], GaugeValueFmt::Percent);
    assert_eq!(both.len(), 2, "a percent reading is a number run and a unit run: {both:?}");
    assert!(both.iter().any(|s| s == "%"), "the unit is its own run: {both:?}");

    apply(None);
}

/// The four keys that change no character of the reading: `unit.scale`,
/// `unit.tracking`, `unit.baseline_shift`, `unit.color`.
///
/// Each is asked of the DRAWN run and against the token's own baked
/// value, then moved in a fixture and asked again. Against the shipped
/// master alone every one of them can be written into `ui.rs` as the
/// number the master happens to ship and nothing notices — which is what
/// the first cut of this file allowed. A stage that changes nothing is a
/// stage that proves nothing, so each fixture also has to differ from the
/// master's picture.
fn the_unit_run_is_set_by_its_own_four_keys() {
    // ---- the master's picture ----------------------------------------
    apply(None);
    let (n0, u0) = pair(12.0);

    let scale0 = scalar("num.unit.scale");
    assert_eq!(u0.px, n0.px * scale0, "the unit is not set at num.unit.scale of the number");
    assert_eq!(
        u0.tracking,
        u0.px * scalar("num.unit.tracking"),
        "num.unit.tracking is an em of the UNIT's own px, not of the number's"
    );
    assert_eq!(
        scalar("num.unit.baseline_shift"),
        0.0,
        "the master says units sit on the baseline, NEVER superscript"
    );
    assert_eq!(u0.at[1], n0.at[1], "a shift of nothing must leave the two runs on one line");
    assert_eq!(u0.color, ink("num.unit.color"), "the unit is not drawn in num.unit.color");
    assert_ne!(
        u0.color, n0.color,
        "the master steps the unit's ink back from the number's; if the two bake to one \
         colour this file cannot tell a unit reading its key from one that is not"
    );

    // ---- the size ------------------------------------------------------
    apply(Some(&format!("{HEAD}[num]\nunit.scale = 0.45\n")));
    let (n1, u1) = pair(12.0);
    assert_eq!(n1.px, n0.px, "the number's own size must not move between fixtures");
    assert_eq!(
        u1.px,
        n1.px * scalar("num.unit.scale"),
        "num.unit.scale moved and the unit run did not follow it"
    );
    assert_ne!(u1.px, u0.px, "the fixture must move the size or nothing is proved");

    // ---- the letter spacing --------------------------------------------
    apply(Some(&format!("{HEAD}[num]\nunit.tracking = 0.5em\n")));
    let (_, u2) = pair(12.0);
    assert_eq!(
        u2.tracking,
        u2.px * scalar("num.unit.tracking"),
        "num.unit.tracking moved and the unit run did not follow it"
    );
    assert_ne!(u2.tracking, u0.tracking, "the fixture must move the tracking");

    // ---- the baseline ---------------------------------------------------
    apply(Some(&format!("{HEAD}[num]\nunit.baseline_shift = 0.25em\n")));
    let (n3, u3) = pair(12.0);
    let shift = scalar("num.unit.baseline_shift");
    assert!(shift > 0.0, "the fixture must ask for a visible shift");
    // Not to the bit, and for the reason given at the joint below: the
    // two y's are one line minus another and the difference carries their
    // ulp. Half a thousandth of a pixel is well inside "the unit did not
    // move at all", which is what the assertion is for.
    let moved = u3.at[1] - n3.at[1];
    assert!(
        (moved - u3.px * shift).abs() < 0.001,
        "num.unit.baseline_shift names {} px and the unit moved {moved}",
        u3.px * shift
    );

    // ---- the ink --------------------------------------------------------
    apply(Some(&format!("{HEAD}[num]\nunit.color = #FF00AA\n")));
    let (_, u4) = pair(12.0);
    assert_eq!(u4.color, ink("num.unit.color"), "num.unit.color moved and the unit did not");
    assert_ne!(u4.color, u0.color, "the fixture must move the ink");

    apply(None);
}

/// `num.unit.gap` and `num.unit.percent_attached` — the joint on the
/// DRAWN side, where it is a distance and not a character.
///
/// The unit hangs off the number's end, so the unit's right edge stands
/// still and the gap pushes the NUMBER left. Three fixtures, because the
/// two keys can only be told apart by holding one still: a gap of nothing
/// and an attached percent must close the joint to the same place, and a
/// gap that is something must open it by exactly what it names.
fn the_joint_between_the_two_runs() {
    const WIDE: &str = "unit.gap = 0.5em\n";

    apply(Some(&format!("{HEAD}[num]\nunit.gap = 0em\nunit.percent_attached = false\n")));
    let (closed, u_closed) = pair(12.0);

    apply(Some(&format!("{HEAD}[num]\n{WIDE}unit.percent_attached = true\n")));
    let (attached, u_attached) = pair(12.0);

    apply(Some(&format!("{HEAD}[num]\n{WIDE}unit.percent_attached = false\n")));
    let (free, u_free) = pair(12.0);

    assert_eq!(
        [u_closed.at[0], u_attached.at[0]],
        [u_free.at[0], u_free.at[0]],
        "the unit run hangs off the reading's right edge and that edge does not move"
    );
    assert_eq!(
        attached.at[0], closed.at[0],
        "num.unit.percent_attached = true must close the joint as tightly as a gap of \
         nothing does — the gap of 0.5em was not suppressed before the percent sign"
    );
    let gap = scalar("num.unit.gap");
    assert!(gap > 0.0, "the fixture must ask for a visible gap");
    // Inexact for one reason: the two x's are a large edge minus a small
    // distance, so their difference is a subtraction of neighbours and
    // carries their ulp. Every claim that CAN be made to the bit — which
    // is every other one in this file — is.
    let opened = closed.at[0] - free.at[0];
    assert!(
        (opened - free.px * gap).abs() < 0.001,
        "num.unit.gap = 0.5em opened the joint by {opened} px and the key names {} px",
        free.px * gap
    );
}
