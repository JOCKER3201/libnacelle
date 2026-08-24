//! A segmented cell's corner belongs to the theme END TO END — the cut,
//! the radius, AND the arc density.
//!
//! The cell used to be drawn through `chamfer_fill`; moving it onto the
//! [`Surface`](nacelle::view::surface::Surface) ring primitive gave it the
//! theme's cut, but routed its tessellation through a ceiling written in
//! Rust as `16` while every other object spent the master's own
//! `corner.segments`. A theme lowering that key moved five objects and
//! left the sixth alone, which is the kind of half-obedience nobody sees
//! until they measure it.
//!
//! Two questions, one drawing:
//!   * does `@corner.pill` reach a cell, or is the sentinel eaten on the
//!     way and answered with the square the theme wrote `pill` to avoid?
//!   * does `corner.segments` reach a cell's arcs?
//!
//! ONE test in a binary of its own, for the reason `control_shape_tokens`
//! is: the resolved theme is process-wide (§7.1 hands every draw path the
//! same `&'static ResolvedTheme`), so a test that swaps it must not run
//! beside a test that reads it.

use nacelle::draw::{ring_segments, DrawList};
use nacelle::font::FontSystem;
use nacelle::object::segmented::{self, StripState};
use nacelle::pointer::Pointer;
use nacelle::theme;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const AREA: Rect = Rect { x: 400.0, y: 500.0, w: 400.0, h: 60.0 };
const LABELS: [&str; 2] = ["ONE", "TWO"];

fn ctx<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem) -> Ctx<'a> {
    Ctx {
        access: None,
        dl,
        fonts,
        w: W,
        h: H,
        t: 0.0,
        mouse: Pointer::new(-1.0, -1.0),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    }
}

/// Loads a fixture whose base is the master, so every token but the ones
/// in `body` is the master's own.
fn skin(body: &str) {
    let path =
        std::env::temp_dir().join(format!("nacelle-segarc-fixture-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        format!("[meta]\nschema = 1\nname = \"Fixture\"\nbase = \"default\"\n\n{body}"),
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(theme::LoadRequest { path: Some(path.clone()), ..Default::default() });
    let _ = std::fs::remove_file(&path);
    theme::set_viewport(H, 1.0);
}

/// Draws the control twice and answers the second list's vertices and the
/// cells it laid out. Twice, so a glyph rasterised on the way in cannot
/// move the atlas under the comparison.
fn shot(fonts: &mut FontSystem) -> (Vec<[f32; 2]>, Vec<Rect>) {
    let st = StripState::new(0);
    let mut warm = DrawList::new();
    segmented::draw(&mut ctx(&mut warm, fonts), AREA, &LABELS, &st);
    let mut dl = DrawList::new();
    let cells = segmented::draw(&mut ctx(&mut dl, fonts), AREA, &LABELS, &st);
    (dl.verts.iter().map(|v| v.pos).collect(), cells)
}

#[test]
fn a_cell_takes_the_capsule_and_the_arc_count_the_theme_states() {
    let mut fonts = FontSystem::new();

    // ---- the sentinel reaches the cell -------------------------------
    // A capsule has no corner point; a squared cell has four, and the
    // first of them is the cell's own origin.
    skin("[segmented]\ncorner = 0u\n");
    let (square, cells) = shot(&mut fonts);
    let cell = cells[0];
    let corner_pt = [cell.x, cell.y];
    assert!(
        square.contains(&corner_pt),
        "the squared cell is missing its own corner point — the probe is wrong"
    );

    skin("[segmented]\ncorner = @corner.pill\n");
    let (pill, pill_cells) = shot(&mut fonts);
    let moved = pill_cells[0];
    assert_eq!(
        (moved.x, moved.y, moved.w, moved.h),
        (cell.x, cell.y, cell.w, cell.h),
        "the corner token moved the layout — it must not"
    );
    assert!(
        !pill.contains(&corner_pt),
        "segmented.corner = @corner.pill left a square corner at {corner_pt:?}"
    );
    assert_ne!(square, pill, "segmented.corner = pill still draws the square");

    // ---- and so does the arc count -----------------------------------
    // `pill` pins the radius to half the cell's shorter side, so the only
    // thing left to move between these two fixtures is the ceiling.
    let radius = cell.w.min(cell.h) / 2.0;
    let low = "[corner]\nsegments = 3\n\n[segmented]\ncorner = @corner.pill\n";
    let high = "[corner]\nsegments = 6\n\n[segmented]\ncorner = @corner.pill\n";
    skin(low);
    let (coarse, _) = shot(&mut fonts);
    skin(high);
    let (fine, _) = shot(&mut fonts);
    assert_eq!(
        ring_segments(radius, 0.25, 3),
        3,
        "the low fixture does not bind — pick a ceiling the quarter-pixel rule cannot reach"
    );
    assert!(
        ring_segments(radius, 0.25, 6) > ring_segments(radius, 0.25, 3),
        "the two fixtures ask for the same arc — the measurement proves nothing"
    );
    // The witness for the ceiling that USED to be written here: 16 and 6
    // buy the same arc at this radius, so a cell still spending a hard 16
    // would answer both fixtures identically. It does not.
    assert_eq!(
        ring_segments(radius, 0.25, 16),
        ring_segments(radius, 0.25, 6),
        "16 and 6 differ at this radius — the old ceiling would show up as a third answer"
    );
    assert!(
        coarse.len() < fine.len(),
        "corner.segments = 3 drew {} vertices against 6's {} — the ceiling is not the theme's",
        coarse.len(),
        fine.len()
    );
}
