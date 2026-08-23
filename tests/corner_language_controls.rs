//! The corner SHAPE of the three controls that still froze it in Rust.
//!
//! The token audit of 2026-08-17 (Z13) counted the hole: of the master's
//! twenty-nine radii, thirteen carried a `*_corner_style` sibling and
//! SIXTEEN carried none (`corner::tests::
//! every_radius_in_the_master_states_the_shape_of_its_own_cut` counts
//! them the same way, and now holds the file to the answer) — so
//! `object/slider.rs`, `object/checkbox.rs` and
//! `object/winframe.rs` wrote `CornerStyle::Round` into Rust and cited
//! `[corner]`'s own header as their authority. A theme writing
//! `corner.mode = chamfer` got a chamfered panel, a chamfered window and
//! a chamfered scrollbar beside a ROUNDED slider, a rounded checkbox and
//! rounded window-frame buttons, on one screen.
//!
//! What is asked here is geometry, not a token read. The three controls
//! are drawn into their own lists and the OUTLINE POINTS the frame put
//! into one corner are counted — the top-left corner square, whose side
//! is the cut plus the stroke that rides it, which is exactly the patch
//! of boundary a cut rewrites and nothing else reaches:
//!
//! * `square` leaves ONE point per boundary, the corner itself;
//! * `chamfer` leaves TWO, the ends of the 45° face;
//! * `round` leaves `segments + 1`, the tessellated arc — and
//!   `segments` is `ring_segments`' own answer under the theme's
//!   ceiling, computed here rather than assumed.
//!
//! A count that came out of the token would pass with the cut still
//! nailed down in Rust. These cannot: with the cut frozen at Round all
//! three fixtures draw the same arc, and the three counts are equal.
//!
//! ONE test in a binary of its own, because the resolved theme is
//! process-wide (§7.1 hands every draw path the same `&'static
//! ResolvedTheme`): a test that swaps it must not run beside one that
//! reads it.

use nacelle::draw::{ring_segments, DrawCmd, DrawList, Vertex};
use nacelle::font::FontSystem;
use nacelle::object::{checkbox, slider, winframe};
use nacelle::pointer::Pointer;
use nacelle::theme::{self, LoadRequest};
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
/// Positions agree to a thousandth of a pixel — the grain the command
/// register already prints at.
const GRAIN: f32 = 1e-3;

fn ctx<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem) -> Ctx<'a> {
    Ctx {
        dl,
        fonts,
        w: W,
        h: H,
        t: 0.0,
        // Parked in the far corner: a hovered plate is a different rung
        // and a different colour, and nothing here is about colour.
        mouse: Pointer::new(4.0, 4.0),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    }
}

/// The master with one cut word and the radii that make a cut visible.
///
/// Two of the three controls ship at `@corner.none`, and a zero radius is
/// a square corner under every word there is — so a fixture that only
/// said `chamfer` would prove nothing about them. The radii below are
/// the fixture's, identical in all three loads; the WORD is the only
/// thing that moves. `winframe.icon.inset` pushes the glyph clear of the
/// plate's corner so the count is the plate's boundary alone.
fn skin(mode: &str) {
    let path = std::env::temp_dir()
        .join(format!("nacelle-cuts-{mode}-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        format!(
            "[meta]\nschema = 1\nname = \"Cuts\"\nbase = \"default\"\n\n\
             [corner]\nmode = {mode}\n\n\
             [checkbox]\ncorner = 1u\n\n\
             [winframe]\nbutton.corner = 1u\n\
             icon.inset = 0.45x @winframe.button.size\n"
        ),
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(LoadRequest { path: Some(path.clone()), ..Default::default() });
    let _ = std::fs::remove_file(&path);
    theme::set_viewport(H, 1.0);
}

/// How many DISTINCT points the frame put inside the top-left corner
/// square of `bx` — the corner's own patch of the boundary.
///
/// `side` has to hold the whole cut and the stroke riding it: an inner
/// chamfer sits `(√2 − 1)·stroke` further out along the edge than the
/// outer one does (`Corner::inset`'s derivation), so a square measured
/// at the cut alone would drop it and count a chamfered ring as a
/// square one.
fn corner_points(verts: &[Vertex], bx: [f32; 4], side: f32) -> usize {
    let (x0, y0) = (bx[0], bx[1]);
    let mut pts: Vec<[f32; 2]> = Vec::new();
    for v in verts {
        let p = v.pos;
        let inside = p[0] >= x0 - GRAIN
            && p[0] <= x0 + side + GRAIN
            && p[1] >= y0 - GRAIN
            && p[1] <= y0 + side + GRAIN;
        if inside
            && !pts
                .iter()
                .any(|q| (q[0] - p[0]).abs() < GRAIN && (q[1] - p[1]).abs() < GRAIN)
        {
            pts.push(p);
        }
    }
    pts.len()
}

/// One drawn control: the box a named command claimed, the cut length it
/// carried, the stroke it was drawn with, and the points that landed in
/// its top-left corner.
struct Probe {
    points: usize,
    /// `ring_segments`' answer for this corner under the theme's
    /// ceiling — what a ROUND corner is entitled to spend.
    arc: u8,
}

/// Takes the vertex slice rather than a whole `DrawList` so a caller whose
/// object shares screen space with something else's boundary — `plate`,
/// since 2026-08-23, whose frame draws its own lit edge close enough to a
/// button plate's corner to land points in it — can filter that other
/// boundary's verts out first. `groove` and `box_of_checkbox` each draw
/// into an otherwise-empty list, so they pass `&dl.verts` whole.
fn probe(verts: &[Vertex], bx: [f32; 4], size: f32, stroke: f32, ceiling: u8) -> Probe {
    assert!(
        size > 0.0,
        "a zero radius is a square corner under every word — the fixture states no radius here"
    );
    Probe {
        points: corner_points(verts, bx, size + stroke),
        arc: ring_segments(size, 0.25, ceiling),
    }
}

fn ceiling() -> u8 {
    let id = theme::id("corner.segments").expect("the master declares corner.segments");
    theme::resolved().px(id) as u8
}

/// The slider's groove: `ring_fill` alone, ONE boundary, and the first
/// fill the object asks for. At `t = 1` the filled part covers exactly
/// the same box with exactly the same corners, so its points fall on the
/// groove's and add none; the knob is away at the far end.
fn groove(fonts: &mut FontSystem) -> Probe {
    let mut dl = DrawList::recording();
    {
        let mut c = ctx(&mut dl, fonts);
        slider::track(&mut c, Rect::new(200.0, 400.0, 300.0, 40.0), 1.0);
    }
    let (r, corners) = dl
        .cmds()
        .iter()
        .find_map(|c| match c {
            DrawCmd::RingFill { r, corners, .. } => Some((*r, *corners)),
            _ => None,
        })
        .expect("the slider drew no groove");
    probe(&dl.verts, r, corners[0].size, 0.0, ceiling())
}

/// The checkbox's box: `ring` alone, so TWO boundaries — the outer face
/// flush with the box and the one inset by `checkbox.border`. Drawn
/// UNCHECKED so the tick, which is inset far enough to fall inside the
/// corner square, is not there to be counted.
fn box_of_checkbox(fonts: &mut FontSystem) -> Probe {
    let mut dl = DrawList::recording();
    {
        let mut c = ctx(&mut dl, fonts);
        checkbox::draw(&mut c, Rect::new(200.0, 500.0, 300.0, 30.0), "SHOW GRID", false, false);
    }
    let (r, corners, stroke) = dl
        .cmds()
        .iter()
        .find_map(|c| match c {
            DrawCmd::Ring { r, corners, stroke, .. } => Some((*r, *corners, *stroke)),
            _ => None,
        })
        .expect("the checkbox drew no box");
    probe(&dl.verts, r, corners[0].size, stroke, ceiling())
}

/// A window control's hit plate: `ring` alone again, two boundaries, and
/// the SMALLEST ring in a frame that also draws its own outline — the
/// plate is a fraction of the title bar and the frame is the window.
///
/// Since 2026-08-23 the frame's own edge sits under a lit `panel_edge`
/// (the neon-by-default change), and its glow burns a second ring at the
/// frame's OWN corner — close enough, in the title bar's top-left, to
/// land points inside THAT button's corner square too, which is
/// `corner_points`' whole risk (any vertex in the box counts, whoever
/// drew it). The title bar carries more than one button of the smallest
/// size, so rather than clean a button's own points out of a box it
/// shares with the frame's — indistinguishable once both are inside it —
/// the search SKIPS any candidate whose own corner box reaches the
/// frame's, and measures one that does not.
fn plate(fonts: &mut FontSystem) -> Probe {
    let mut dl = DrawList::recording();
    {
        let mut c = ctx(&mut dl, fonts);
        let m = winframe::Metrics::new(H);
        let f = winframe::Frame::new();
        f.draw(&mut c, &m, Rect::new(300.0, 200.0, 800.0, 500.0), "TERMINAL", true);
    }
    let all: Vec<_> = dl
        .cmds()
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Ring { r, corners, stroke, .. } => Some((*r, *corners, *stroke)),
            _ => None,
        })
        .collect();
    let frame_area = all
        .iter()
        .map(|(r, ..)| r[2] * r[3])
        .fold(0.0f32, f32::max);
    let frame_rings: Vec<_> =
        all.iter().copied().filter(|(r, ..)| r[2] * r[3] == frame_area).collect();
    let clear_of_the_frame = |r: [f32; 4], side: f32| {
        !frame_rings.iter().any(|(fr, fc, fs)| {
            corner_boxes(*fr, fc[0].size + fs)
                .into_iter()
                .any(|f_origin| boxes_overlap((r[0], r[1]), side, f_origin, fc[0].size + fs))
        })
    };
    let (r, corners, stroke) = all
        .iter()
        .copied()
        .filter(|(r, ..)| r[2] * r[3] < frame_area)
        .filter(|(r, c, s)| clear_of_the_frame(*r, c[0].size + s))
        .min_by(|a, b| (a.0[2] * a.0[3]).total_cmp(&(b.0[2] * b.0[3])))
        .expect("every button plate the frame drew sits inside its own corner's reach");
    probe(&dl.verts, r, corners[0].size, stroke, ceiling())
}

/// The origins of `r`'s four corner squares, each `side` wide.
fn corner_boxes(r: [f32; 4], side: f32) -> [(f32, f32); 4] {
    [
        (r[0], r[1]),
        (r[0] + r[2] - side, r[1]),
        (r[0], r[1] + r[3] - side),
        (r[0] + r[2] - side, r[1] + r[3] - side),
    ]
}

/// Whether the GRAIN-padded `side`-wide squares at `a` and `b` share any
/// ground — the same padding `corner_points` itself measures with, so a
/// box is judged clear of another exactly as generously as a vertex
/// would be judged inside either of them.
fn boxes_overlap(a: (f32, f32), a_side: f32, b: (f32, f32), b_side: f32) -> bool {
    a.0 - GRAIN <= b.0 + b_side + GRAIN
        && b.0 - GRAIN <= a.0 + a_side + GRAIN
        && a.1 - GRAIN <= b.1 + b_side + GRAIN
        && b.1 - GRAIN <= a.1 + a_side + GRAIN
}

#[test]
fn one_line_of_theme_cuts_the_slider_the_checkbox_and_the_frame_buttons() {
    let mut fonts = FontSystem::new();

    // Boundaries per control, from what each object actually asks the
    // draw list for: the groove is a fill (one), the checkbox box and
    // the plate are strokes (an outer face and an inner one).
    const GROOVE_EDGES: usize = 1;
    const RING_EDGES: usize = 2;

    // ---- square: one point per boundary, the corner itself ------------
    skin("square");
    let (g, c, p) = (groove(&mut fonts), box_of_checkbox(&mut fonts), plate(&mut fonts));
    assert_eq!(g.points, GROOVE_EDGES, "the groove did not come out square");
    assert_eq!(c.points, RING_EDGES, "the checkbox box did not come out square");
    let plate_square = p.points;
    assert!(plate_square >= RING_EDGES, "the plate drew no corner at all");

    // ---- chamfer: two, the ends of the 45° face -----------------------
    skin("chamfer");

    // Every sibling the audit found missing, checked where it now
    // stands: one line said `chamfer` and each of these has to say it
    // back. A name misspelled here is a token nobody declares and
    // `theme::id` says so; a chain pointed at the wrong parent stays
    // `round` and the word says that.
    for name in [
        "avatar.corner_style",
        "chart.bar.corner_style",
        "checkbox.corner_style",
        "dialog.corner_mode",
        "dock.corner_mode",
        "feed.corner_style",
        "filetile.corner_style",
        "iconbtn.corner_style",
        "keyboard.key_corner_style",
        "panel.button.corner_style",
        "slider.knob_corner_style",
        "slider.track_corner_style",
        // Not a `_style` sibling but the key that cuts `tile.corner`:
        // a whole SHAPE, whose vocabulary keeps `hex` on top of the
        // three cuts. It was the last radius in the master answering a
        // literal, which is the same defect one level up — a rounded
        // launcher tile beside a chamfered panel.
        "tile.shape",
        "toast.corner_mode",
        "winframe.button.corner_style",
    ] {
        let id = theme::id(name).unwrap_or_else(|| panic!("the master declares no {name}"));
        assert_eq!(
            theme::enum_word_of(id).as_deref(),
            Some("chamfer"),
            "{name} does not follow @corner.mode"
        );
    }

    let (g, c, p) = (groove(&mut fonts), box_of_checkbox(&mut fonts), plate(&mut fonts));
    assert_eq!(g.points, 2 * GROOVE_EDGES, "the groove kept its old cut");
    assert_eq!(c.points, 2 * RING_EDGES, "the checkbox box kept its old cut");
    let plate_chamfer = p.points;

    // ---- round: the arc the tessellation rule pays for ----------------
    skin("round");
    let (g, c, p) = (groove(&mut fonts), box_of_checkbox(&mut fonts), plate(&mut fonts));
    assert_eq!(
        g.points,
        (g.arc as usize + 1) * GROOVE_EDGES,
        "the groove's arc is not the one ring_segments asked for"
    );
    assert_eq!(
        c.points,
        (c.arc as usize + 1) * RING_EDGES,
        "the checkbox box's arc is not the one ring_segments asked for"
    );
    let plate_round = p.points;

    // The plate shares its corner square with the glyph inside it, so
    // its count carries a constant the fixture cannot remove — the same
    // constant under all three words, because nothing but the cut moved.
    // The ORDER is therefore the claim, and it is the whole claim: three
    // words, three shapes, strictly more boundary as the corner softens.
    // With the cut nailed to Round in Rust these three are equal.
    assert!(
        plate_square < plate_chamfer && plate_chamfer < plate_round,
        "the window-frame plate did not follow the theme's cut: \
         square {plate_square}, chamfer {plate_chamfer}, round {plate_round}"
    );
    assert_eq!(
        plate_chamfer - plate_square,
        RING_EDGES,
        "a chamfer adds exactly one point per boundary over a square corner"
    );
    assert_eq!(
        plate_round - plate_square,
        p.arc as usize * RING_EDGES,
        "a round corner adds exactly `segments` points per boundary over a square one"
    );
}
