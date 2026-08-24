//! `[rhythm]`'s baseline grid and its two column alignments — the keys
//! §5.25 declared and nothing read.
//!
//! Z24 counted eight of the block's sixteen keys dead. Five of the eight
//! are this file's: three are the grid (`baseline`, `snap_baseline`,
//! `snap_origin`) and two are where a run sits in the column reserved for
//! it (`label_align`, `value_align`). The other three — `label_min`,
//! `label_max`, `value_col` — are the settings window's, because that is
//! where a settings row's columns are measured; `nacelle-desktop`'s
//! `widgets/settings.rs` reads all three today.
//!
//! The grid ships OFF (see the master's TODO at `snap_baseline`), so the
//! first claim below is that the switch really is a switch: under the
//! shipped theme a line lands exactly where the centring arithmetic put
//! it, to the last bit. Every other claim turns it on in a fixture.

use nacelle::draw::{DrawCmd, DrawList};
use nacelle::font::{FontSystem, FONT_UI};
use nacelle::pointer::Pointer;
use nacelle::theme::{self, LoadRequest};
use nacelle::ui::{self, GaugeKind, GaugeLabels, GaugeStyle, GaugeValueFmt};
use nacelle::view::paint;
use nacelle::view::surface::CtxSurface;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
/// A line well clear of `type.min_px`, in a box with room to move it.
const PX: f32 = 16.0;
const LEADING: f32 = 1.2;
const BOX_H: f32 = 40.0;

fn fresh<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| s.spawn(f).join().expect("the drawing thread panicked"))
}

fn apply(fixture: Option<&str>) {
    match fixture {
        None => {
            let _ = theme::load();
        }
        Some(text) => {
            let path = std::env::temp_dir()
                .join(format!("nacelle-rhythm-grid-{}.theme", std::process::id()));
            std::fs::write(&path, text).expect("the fixture theme must be writable");
            let _ = theme::load_with(LoadRequest { path: Some(path), ..Default::default() });
        }
    }
}

const HEAD: &str = "[meta]\nschema = 1\nname = \"Rhythm fixture\"\nbase = \"default\"\n\n";

/// The unsnapped top, the snapped top and the ascent between them, for a
/// line centred in a box beginning at `y`.
fn line(y: f32, origin: f32) -> (f32, f32, f32) {
    fresh(move || {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::new();
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
        paint::set_grid_origin(origin);
        let ascent = c.fonts.line_metrics(FONT_UI, PX).0;
        let mut sf = CtxSurface::new(&mut c);
        (
            paint::center_line_y(&mut sf, y, BOX_H, PX, LEADING),
            paint::center_line_y_in(&mut sf, FONT_UI, y, BOX_H, PX, LEADING),
            ascent,
        )
    })
}

/// Where every label and every reading of a gauge ROW block was drawn,
/// as (x, anchor-is-right) pairs in draw order.
fn row_runs(labels: &[&str]) -> Vec<(String, f32)> {
    let labels: Vec<String> = labels.iter().map(|s| (*s).to_string()).collect();
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
                kind: GaugeKind::Row,
                labels: GaugeLabels::Text(labels.clone()),
                value_fmt: GaugeValueFmt::Raw,
                shrink: 1.0,
            };
            // Readings of unequal width, so a value column that is wider
            // than its content has something to align INSIDE it.
            let values = [7.0f32, 100.0];
            ui::gauge_grid(&mut c, Rect::new(40.0, 40.0, 400.0, 200.0), &values, &st);
        }
        dl.cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { text, at, .. } => Some((text.clone(), at[0])),
                _ => None,
            })
            .collect()
    })
}

#[test]
fn the_theme_owns_the_vertical_grid_and_the_column_alignments() {
    the_switch_is_a_switch();
    a_snapped_baseline_lands_on_the_grid();
    the_origin_is_where_snap_origin_says();
    the_two_alignments_move_their_runs();
    apply(None);
}

/// The shipped master snaps nothing, so the two entry points answer the
/// same float — not "close", the same.
fn the_switch_is_a_switch() {
    apply(None);
    let (plain, snapped, _) = line(40.0, 0.0);
    assert_eq!(
        plain, snapped,
        "the master ships rhythm.snap_baseline = false and a line still moved"
    );
}

/// With the grid on, the BASELINE — the top plus the face's ascent — is a
/// whole number of steps from the origin.
fn a_snapped_baseline_lands_on_the_grid() {
    // A step no centring arithmetic would land on by accident, and a
    // long way from the master's own 1u, so a stale token would show.
    apply(Some(&format!(
        "{HEAD}[rhythm]\nsnap_baseline = true\nbaseline = 7u\nsnap_origin = screen_top\n"
    )));
    let step = fresh(|| {
        let t = theme::resolved();
        t.px(theme::id("rhythm.baseline").expect("the master declares it"))
    });
    assert!(step > 0.0, "the fixture must bake a positive step");

    for y in [40.0f32, 41.0, 42.7, 103.5] {
        let (plain, snapped, ascent) = line(y, 0.0);
        let landed = snapped + ascent;
        let k = (landed / step).round();
        assert!(
            (landed - k * step).abs() < 0.01,
            "y={y}: the baseline landed at {landed}, which is not a multiple of {step}"
        );
        // And it is the NEAREST line, so the snap never moves a run
        // further than half a step from where it was centred.
        assert!(
            (snapped - plain).abs() <= step / 2.0 + 0.01,
            "y={y}: the grid moved the line {} px, more than half a step",
            (snapped - plain).abs()
        );
    }

    // A grid of no width is no grid: the switch may be on and the line
    // still must not be divided by zero into somewhere.
    apply(Some(&format!("{HEAD}[rhythm]\nsnap_baseline = true\nbaseline = 0u\n")));
    let (plain, snapped, _) = line(40.0, 0.0);
    assert_eq!(plain, snapped, "a baseline of zero moved a line");
}

/// `snap_origin` says WHERE the grid is measured from, and the panel
/// object publishes the content top it is measured from.
fn the_origin_is_where_snap_origin_says() {
    const ORIGIN: f32 = 13.0;
    apply(Some(&format!(
        "{HEAD}[rhythm]\nsnap_baseline = true\nbaseline = 7u\nsnap_origin = screen_top\n"
    )));
    let (_, from_screen, ascent) = line(40.0, ORIGIN);
    apply(Some(&format!(
        "{HEAD}[rhythm]\nsnap_baseline = true\nbaseline = 7u\nsnap_origin = panel_content_top\n"
    )));
    let (_, from_panel, _) = line(40.0, ORIGIN);
    assert_ne!(
        from_screen, from_panel,
        "both origins put the line in one place, so snap_origin has no reader"
    );

    let step = fresh(|| {
        let t = theme::resolved();
        t.px(theme::id("rhythm.baseline").expect("the master declares it"))
    });
    let off = (from_panel + ascent - ORIGIN) / step;
    assert!(
        (off - off.round()).abs() < 0.01,
        "the panel-relative grid is not measured from the content top: {from_panel}"
    );
}

/// `label_align` and `value_align` — where the two halves of an
/// instrument row sit in the columns measured for them.
fn the_two_alignments_move_their_runs() {
    // Labels of unequal width, so the shared label column is wider than
    // the narrow one and an alignment has room to mean something.
    const LABELS: [&str; 2] = ["A", "LONGEST"];

    apply(None);
    let left = row_runs(&LABELS);
    let narrow_left = left
        .iter()
        .find(|(t, _)| t == "A")
        .expect("the short label is drawn")
        .1;
    let wide = left
        .iter()
        .find(|(t, _)| t == "LONGEST")
        .expect("the long label is drawn")
        .1;
    assert_eq!(
        narrow_left, wide,
        "the master aligns labels left, so both start at one x"
    );

    apply(Some(&format!("{HEAD}[rhythm]\nlabel_align = right\n")));
    let narrow_right = row_runs(&LABELS)
        .iter()
        .find(|(t, _)| t == "A")
        .expect("the short label is drawn")
        .1;
    assert!(
        narrow_right > narrow_left,
        "rhythm.label_align = right did not move the short label into its column: \
         {narrow_right} vs {narrow_left}"
    );

    // The reading: the master aligns it right, so the narrow one ENDS
    // where the wide one does. Aligned left it starts where the wide one
    // starts instead, which moves its right edge inwards.
    let right_edge = |runs: &[(String, f32)]| -> f32 {
        runs.iter()
            .find(|(t, _)| t == "7")
            .expect("the narrow reading is drawn")
            .1
    };
    apply(None);
    let aligned_right = right_edge(&row_runs(&LABELS));
    apply(Some(&format!("{HEAD}[rhythm]\nvalue_align = left\n")));
    let aligned_left = right_edge(&row_runs(&LABELS));
    assert!(
        aligned_left < aligned_right,
        "rhythm.value_align = left did not move the narrow reading: \
         {aligned_left} vs {aligned_right}"
    );
}
