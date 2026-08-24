//! Two keys of `[gauge]` were set against each other, and the one that
//! lost had no reader at all.
//!
//! `gauge.h` declares "the gauge body's height" (`@size.sm` = 4u).
//! `gauge.min_h_for_value` declares the height below which the numeric
//! readout is dropped, as a ratio of the READING (1.4x `@type.value.size`
//! = 4.55u). Nothing held the two in step, and once the type ladder was
//! unified the threshold climbed past the body it guards: a gauge drawn
//! at the height this file declares for it would throw its number away.
//!
//! `gauge.h` was the arbiter and nobody read it — not one line of the
//! program — so the contradiction could not even be observed from the
//! code. `ui::gauge_grid` now takes the smaller of the two, which gives
//! `gauge.h` its reader and makes the two keys unable to contradict each
//! other whatever a theme writes.
//!
//! Both keys are asked for here, in both directions: a theme that raises
//! the threshold past the body height cannot silence a gauge drawn at
//! that height, and a theme that lowers it below is obeyed. The second
//! direction is the one that keeps `min_h_for_value` alive — everything
//! above `gauge.h` is satisfied by `gauge.h` alone, so without it a
//! reader could drop the threshold entirely and this file would still
//! pass.
//!
//! What this file does NOT decide is how big the reading should be. That
//! is `gauge.value_role` (3.25u today, 1.77u before the ladder was
//! unified), it is the owner's to settle from two renderings rather than
//! from a rule, and nothing here moves a drawn glyph by a pixel.

use nacelle::draw::{DrawCmd, DrawList};
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::theme::{self, LoadRequest};
use nacelle::ui::{self, GaugeKind, GaugeLabels, GaugeStyle, GaugeValueFmt};
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;

/// Runs one question on a thread of its own — the reason
/// `tests/gauge_role_bindings.rs` gives, in full: the toolkit memoises a
/// resolved role per thread, and a reload renumbers the word set a
/// binding lives in.
fn fresh<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| s.spawn(f).join().expect("the drawing thread panicked"))
}

/// The readings a single-cell gauge block draws in a box `h` tall.
/// Empty means the readout was dropped.
fn readings(h: f32) -> Vec<String> {
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
                value_fmt: GaugeValueFmt::Percent,
                shrink: 1.0,
            };
            // One value in one column: `gauge_grid`'s row arithmetic
            // hands the whole box to the single cell, so the cell is
            // exactly `h` tall and the question is asked at the height
            // this test names.
            ui::gauge_grid(&mut c, Rect::new(40.0, 40.0, 400.0, h), &[42.0], &st);
        }
        dl.cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.to_string()),
                _ => None,
            })
            .collect()
    })
}

/// Whether a reading was drawn at all.
///
/// NOT a count of runs. A reading is a NUMBER and, where the format asks
/// for one, a UNIT beside it — two runs, because the master gives the unit
/// its own size, tracking and baseline (`num.unit.*`) and none of that can
/// be said by appending characters to a string. Counting runs here once
/// asserted `== 1` and broke the day the unit got the run the master had
/// always described for it. What this test is about is whether the
/// threshold DROPPED the readout, so it asks exactly that.
fn wrote_a_number(runs: &[String]) -> bool {
    runs.iter().any(|s| s.chars().any(|c| c.is_ascii_digit()))
}

fn px(name: &str) -> f32 {
    let owned = name.to_string();
    fresh(move || {
        theme::resolved().px(theme::id(&owned).expect("the master declares this key"))
    })
}

const HEAD: &str =
    "[meta]\nschema = 1\nname = \"Gauge threshold fixture\"\nbase = \"default\"\n\n";

/// Loads the master, or a theme based on it that rewrites a few keys.
fn apply(fixture: Option<&str>) {
    match fixture {
        None => {
            let _ = theme::load();
        }
        Some(text) => {
            let path = std::env::temp_dir()
                .join(format!("nacelle-gauge-min-h-{}.theme", std::process::id()));
            std::fs::write(&path, text).expect("the fixture theme must be writable");
            let _ = theme::load_with(LoadRequest { path: Some(path), ..Default::default() });
        }
    }
}

#[test]
fn a_gauge_at_its_declared_height_keeps_its_reading() {
    apply(None);
    let body_h = px("gauge.h");
    assert!(body_h > 0.0, "the master declares a gauge body height");

    // The claim, at the master's own numbers: a gauge drawn at exactly
    // the height the master declares for it draws its number.
    let at_home = readings(body_h);
    assert!(
        wrote_a_number(&at_home),
        "a gauge drawn at `gauge.h` ({body_h} px) dropped its reading — the threshold \
         guarding the readout stands above the body it guards, and `gauge.h` says \
         which of the two is the ceiling. Runs drawn: {at_home:?}"
    );

    // And the threshold still works BELOW that height: this is a cap, not
    // a way of turning the guard off. A sliver of a gauge has no room for
    // a number and must not draw one.
    let sliver = readings(body_h / 4.0);
    assert!(
        sliver.is_empty(),
        "a gauge a quarter of its declared height still wrote a number in it: \
         {sliver:?}"
    );

    // Stated once more without leaning on the master's current numbers: a
    // theme that raises the threshold far past the body height cannot
    // make a gauge at its own declared height throw its reading away.
    // (This is the stage that keeps passing the day the owner settles the
    // reading's SIZE and the master's two numbers stop contradicting each
    // other on their own.)
    apply(Some(&format!(
        "{HEAD}[gauge]\nmin_h_for_value = 10x @type.value.size\n"
    )));
    let body_h = px("gauge.h");
    let greedy = readings(body_h);
    assert!(
        wrote_a_number(&greedy),
        "a theme set `min_h_for_value` above `gauge.h` and the reading vanished from a \
         gauge drawn at `gauge.h`: the two keys can still be set against each other. \
         Runs drawn: {greedy:?}"
    );

    // And the other way round, which is what keeps `min_h_for_value`
    // itself alive. Every stage above is satisfied by `gauge.h` ALONE —
    // a cap that is never reached from below looks exactly like no
    // threshold at all — so a reader could drop the key this program has
    // always read and nothing here would notice. Below `gauge.h` the two
    // answers part: a theme that lowers the threshold is obeyed, and a
    // gauge shorter than its declared body still writes its number.
    apply(Some(&format!(
        "{HEAD}[gauge]\nmin_h_for_value = 0.4x @type.value.size\n"
    )));
    let body_h = px("gauge.h");
    let low = px("gauge.min_h_for_value");
    assert!(
        low > 0.0 && low * 1.5 < body_h,
        "the fixture must part the lowered threshold from the body height, or this \
         stage asks nothing: threshold {low} px against a body {body_h} px tall"
    );
    let short = readings(low * 1.5);
    assert!(
        wrote_a_number(&short),
        "a theme lowered `min_h_for_value` to {low} px and a gauge {} px tall — taller \
         than that threshold, shorter than `gauge.h` — dropped its reading anyway: \
         the threshold is not being read, only the body height. Runs drawn: {short:?}",
        low * 1.5
    );

    // The lowered threshold is still a threshold, not an amnesty.
    let too_short = readings(low * 0.5);
    assert!(
        too_short.is_empty(),
        "a gauge under the theme's own lowered threshold wrote a number in it: \
         {too_short:?}"
    );

    apply(None);
}
