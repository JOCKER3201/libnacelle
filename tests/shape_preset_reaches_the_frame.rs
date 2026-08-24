//! **f3 K6's acceptance condition, executed.**
//!
//! The step's own words: *"a theme setting `shape.panel.corners_tl =
//! [chamfer, 2u]` changes one corner and nothing else"*. A reader that
//! nobody calls does not satisfy that — the token is still dead where it
//! counts, and the picture on the glass is the only place it counts. So
//! this file writes exactly that line into a theme, draws the panel
//! frame the shipped code draws, and looks at what came out.
//!
//! Two halves, and both are needed. The command register says what the
//! frame ASKED FOR — four corners, one of them different — and the
//! vertices say what the ring generator DID with it, corner region by
//! corner region: the three the theme did not name have to come out
//! point for point identical, or "and nothing else" is a wish.
//!
//! It is a test binary of its own, for the reason `control_shape_tokens`
//! is: the resolved theme is process-wide, so a test that swaps it must
//! not run beside a test that reads it.

use nacelle::draw::{Corner, CornerStyle, DrawCmd, DrawList};
use nacelle::font::FontSystem;
use nacelle::object::window;
use nacelle::pointer::Pointer;
use nacelle::theme;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
/// Big enough that no corner's disc below can reach another's, and that
/// the fan centroid sits far outside every one of them.
const PANEL: Rect = Rect { x: 200.0, y: 150.0, w: 400.0, h: 260.0 };

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

/// Loads a fixture theme whose base is the master, so every token but
/// the ones in `body` is the master's own.
fn skin(body: &str) {
    let path =
        std::env::temp_dir().join(format!("nacelle-preset-fixture-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        format!("[meta]\nschema = 1\nname = \"Fixture\"\nbase = \"default\"\n\n{body}"),
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(theme::LoadRequest { path: Some(path.clone()), ..Default::default() });
    let _ = std::fs::remove_file(&path);
    theme::set_viewport(H, 1.0);
}

/// The frame, drawn twice through whatever theme is loaded — the second
/// list is the answer, so a glyph rasterised on the way in cannot move
/// the atlas under the comparison.
fn shot(fonts: &mut FontSystem) -> (Vec<[f32; 2]>, Vec<DrawCmd>) {
    let mut warm = DrawList::recording();
    window::frame(&mut ctx(&mut warm, fonts), PANEL);
    let mut dl = DrawList::recording();
    window::frame(&mut ctx(&mut dl, fonts), PANEL);
    (dl.verts.iter().map(|v| v.pos).collect(), dl.cmds().to_vec())
}

/// The four corners the frame asked its ring generator for, in
/// `ring_points`' order — tl, tr, br, bl.
fn asked(cmds: &[DrawCmd]) -> [Corner; 4] {
    for c in cmds {
        if let DrawCmd::RingFill { corners, .. } = c {
            return *corners;
        }
    }
    panic!("the frame drew no ring fill at all");
}

/// Every drawn point within `reach` of one corner of the panel, sorted
/// so two runs compare as sets and not as orderings: a chamfer and an
/// arc put out different NUMBERS of points, which renumbers everything
/// downstream of them in the same list.
fn near(pts: &[[f32; 2]], corner: [f32; 2], reach: f32) -> Vec<[i64; 2]> {
    let mut out: Vec<[i64; 2]> = pts
        .iter()
        .filter(|p| (p[0] - corner[0]).hypot(p[1] - corner[1]) <= reach)
        // Quantised to a thousandth of a pixel: the comparison is about
        // WHERE the generator put a point, not about which order the
        // additions inside it happened to run in.
        .map(|p| [(p[0] as f64 * 1000.0).round() as i64, (p[1] as f64 * 1000.0).round() as i64])
        .collect();
    out.sort_unstable();
    out
}

#[test]
fn one_corner_of_the_panel_and_nothing_else() {
    let mut fonts = FontSystem::new();
    let corners = [
        ("tl", [PANEL.x, PANEL.y]),
        ("tr", [PANEL.right(), PANEL.y]),
        ("br", [PANEL.right(), PANEL.bottom()]),
        ("bl", [PANEL.x, PANEL.bottom()]),
    ];
    // A quarter of the shorter side: wide enough to hold any corner the
    // master or the fixture cuts, narrow enough that the discs are
    // disjoint and that none of them reaches the fan's own centroid.
    let reach = PANEL.w.min(PANEL.h) * 0.25;

    // The harness first: two loads of the same fixture must draw the
    // same picture, or a difference below means nothing.
    skin("");
    let (base_pts, base_cmds) = shot(&mut fonts);
    skin("");
    let (again, _) = shot(&mut fonts);
    assert_eq!(base_pts, again, "reloading one theme changed the picture");

    // The master ships every slot inheriting, so the frame's four
    // corners are one answer four times — the picture K6 must not have
    // moved.
    let before = asked(&base_cmds);
    assert!(
        before.iter().all(|c| *c == before[0]),
        "the master's own panel came out asymmetric: {before:?}"
    );
    assert!(before[0].size > 0.0, "the panel's radius arrived as nothing: {:?}", before[0]);

    // And now the line from the step, verbatim.
    skin("[shape.panel]\ncorners_tl = [ chamfer, 2u ]\n");
    let (cut_pts, cut_cmds) = shot(&mut fonts);
    let after = asked(&cut_cmds);
    assert_eq!(after[0].style, CornerStyle::Chamfer, "the top-left kept the preset's cut");
    assert_ne!(after[0].size, before[0].size, "the stated length never arrived");
    assert_eq!(after[1..], before[1..], "a corner the theme did not name moved");

    // The picture: the named corner is a different shape, and the three
    // others are the same points they were.
    assert_ne!(
        near(&cut_pts, corners[0].1, reach),
        near(&base_pts, corners[0].1, reach),
        "the top-left drew the same shape it drew before the theme said otherwise"
    );
    for (name, at) in &corners[1..] {
        assert_eq!(
            near(&cut_pts, *at, reach),
            near(&base_pts, *at, reach),
            "the {name} corner moved, and the theme named only the top-left"
        );
    }

    // The pair is TWO independent slots, which is the whole reason the
    // master spells these keys as pairs: a theme may keep the preset's
    // cut and only shorten it.
    skin("[shape.panel]\ncorners_bl = [ same_as_parent, 0.6u ]\n");
    let (short_pts, short_cmds) = shot(&mut fonts);
    let short = asked(&short_cmds);
    assert_eq!(short[3].style, before[3].style, "the inherited slot lost the preset's style");
    assert!(short[3].size < before[3].size, "the stated slot never arrived");
    assert_eq!(short[..3], before[..3], "shortening one corner moved another");
    for (name, at) in &corners[..3] {
        assert_eq!(
            near(&short_pts, *at, reach),
            near(&base_pts, *at, reach),
            "the {name} corner moved, and the theme named only the bottom-left"
        );
    }

    // The ONE thing "and nothing else" cannot mean, said out loud rather
    // than dodged. A ring carries a single segment count for all four
    // corners (`ring_points`), so a theme that makes one corner ROUNDER
    // than the rest raises the count the other three are drawn at: their
    // arcs are the same arcs, sampled at more places. That is
    // `ring_points`' contract and not this reader's — the alternative is
    // to under-tessellate the corner the theme just enlarged, which is a
    // visible defect where this is not. A chamfer cannot do it, because
    // a straight cut has no arc to pay for.
    skin("[shape.panel]\ncorners_bl = [ round, 6u ]\n");
    let (rounder_pts, _) = shot(&mut fonts);
    assert_ne!(
        near(&rounder_pts, corners[1].1, reach),
        near(&base_pts, corners[1].1, reach),
        "a rounder corner left the ring's shared segment count alone — check `round_reach`"
    );
    // And that is the ONLY coupling: the chamfer of the first case is a
    // bigger number than the master's radius and moved nothing.
    assert!(after[0].size > before[0].size, "the case above stopped being the harder one");
}
