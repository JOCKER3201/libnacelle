//! The keys the audit found declared and unread — proved read, by
//! drawing.
//!
//! Every case here is the same experiment: draw one control through a
//! theme, draw it again through a theme that differs in exactly one
//! token, and look at what came out. Both sides are fixtures over the
//! master, so the only thing that can explain a difference is the token
//! under test — and a token that explains no difference is a comment,
//! not a binding.
//!
//! It is ONE test in a binary of its own, for the reason `mood_engine`
//! is: the resolved theme is process-wide (§7.1 hands every draw path
//! the same `&'static ResolvedTheme`), so a test that swaps it must not
//! run beside a test that reads it.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::object::{checkbox, dropdown, focus_ring, slider};
use nacelle::pointer::Pointer;
use nacelle::theme;
use nacelle::view::ScrollView;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;

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
        std::env::temp_dir().join(format!("nacelle-shape-fixture-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        format!("[meta]\nschema = 1\nname = \"Fixture\"\nbase = \"default\"\n\n{body}"),
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(theme::LoadRequest { path: Some(path.clone()), ..Default::default() });
    let _ = std::fs::remove_file(&path);
    theme::set_viewport(H, 1.0);
}

/// The master, untouched.
fn master() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
}

/// Vertex positions only: a shape change moves them, and they are what
/// reaches the screen.
fn pts(dl: &DrawList) -> Vec<[f32; 2]> {
    dl.verts.iter().map(|v| v.pos).collect()
}

/// Draws `f` twice through the theme now loaded and answers the second
/// list, so a glyph rasterised on the way in cannot move the atlas under
/// the comparison.
fn shot(fonts: &mut FontSystem, f: impl Fn(&mut Ctx)) -> Vec<[f32; 2]> {
    let mut warm = DrawList::new();
    f(&mut ctx(&mut warm, fonts));
    let mut dl = DrawList::new();
    f(&mut ctx(&mut dl, fonts));
    pts(&dl)
}

/// The same drawing under two fixtures that differ in one token.
fn pair(
    fonts: &mut FontSystem,
    a: &str,
    b: &str,
    f: impl Fn(&mut Ctx),
) -> (Vec<[f32; 2]>, Vec<[f32; 2]>) {
    skin(a);
    let left = shot(fonts, &f);
    skin(b);
    let right = shot(fonts, &f);
    (left, right)
}

const TRACK: Rect = Rect { x: 100.0, y: 200.0, w: 300.0, h: 40.0 };
const ROW: Rect = Rect { x: 100.0, y: 400.0, w: 300.0, h: 36.0 };

#[test]
fn every_shape_token_the_audit_found_unread_now_moves_the_picture() {
    let mut fonts = FontSystem::new();
    let slider = |c: &mut Ctx| slider::track(c, TRACK, 0.6);
    let checked = |c: &mut Ctx| checkbox::draw(c, ROW, "LABEL", true, false);
    let unchecked = |c: &mut Ctx| checkbox::draw(c, ROW, "LABEL", false, false);

    // ---- the harness itself ------------------------------------------
    // Two identical fixtures draw identically: whatever a difference
    // below means, it does not mean "a theme file was reloaded".
    let (one, two) = pair(&mut fonts, "[slider]\nknob_w = 1u\n", "[slider]\nknob_w = 1u\n", slider);
    assert_eq!(one, two, "reloading the same theme changed the picture");

    // ---- slider.track_corner -----------------------------------------
    let (pill, square) = pair(
        &mut fonts,
        "[slider]\ntrack_corner = @corner.pill\n",
        "[slider]\ntrack_corner = 0u\n",
        slider,
    );
    assert_ne!(pill, square, "slider.track_corner does not reach the groove");
    // And the master's own value is the capsule it says it is: a capsule
    // has no corner point, a squared groove does.
    master();
    let shipped = shot(&mut fonts, slider);
    let cy = TRACK.y + TRACK.h / 2.0;
    let th = theme::resolved().px(theme::id("slider.track_h").expect("[slider] declares track_h"));
    let corner = [TRACK.x, cy - th / 2.0];
    assert!(
        square.contains(&corner),
        "the squared groove is missing its own corner point — the probe is wrong"
    );
    assert!(
        !shipped.contains(&corner),
        "the master's @corner.pill groove still has a square corner at {corner:?}"
    );

    // ---- slider.knob_corner ------------------------------------------
    let (sharp, round) = pair(
        &mut fonts,
        "[slider]\nknob_corner = @corner.none\n",
        "[slider]\nknob_corner = 1u\n",
        slider,
    );
    assert_ne!(sharp, round, "slider.knob_corner does not reach the knob");

    // ---- checkbox.corner ---------------------------------------------
    let (sharp_box, round_box) = pair(
        &mut fonts,
        "[checkbox]\ncorner = @corner.none\n",
        "[checkbox]\ncorner = 1.5u\n",
        checked,
    );
    assert_ne!(sharp_box, round_box, "checkbox.corner does not reach the box");

    // `pill` is the trap here, not the round radius: §5.0 bakes the word
    // to a NEGATIVE number, so a box that clamped the token at zero drew
    // the very square the theme wrote `pill` to avoid — and said nothing.
    let (square_box, pill_box) = pair(
        &mut fonts,
        "[checkbox]\ncorner = @corner.none\n",
        "[checkbox]\ncorner = @corner.pill\n",
        checked,
    );
    assert_ne!(square_box, pill_box, "checkbox.corner = pill still draws the square");
    // And a capsule on a square box is a circle: the box's own corner
    // point survives `none` and cannot survive `pill`.
    let side =
        theme::resolved().px(theme::id("checkbox.size").expect("[checkbox] declares size"));
    let corner_pt = [ROW.x, ROW.y + (ROW.h - side) / 2.0];
    assert!(
        square_box.contains(&corner_pt),
        "the squared box is missing its own corner point — the probe is wrong"
    );
    assert!(
        !pill_box.contains(&corner_pt),
        "checkbox.corner = pill left a square corner at {corner_pt:?}"
    );

    // ---- checkbox.tick_shape -----------------------------------------
    // Three words, three marks, no two alike.
    let (mark_square, mark_check) = pair(
        &mut fonts,
        "[checkbox]\ntick_shape = square\n",
        "[checkbox]\ntick_shape = check\n",
        checked,
    );
    let (_, mark_cross) = pair(
        &mut fonts,
        "[checkbox]\ntick_shape = square\n",
        "[checkbox]\ntick_shape = cross\n",
        checked,
    );
    assert_ne!(mark_square, mark_check, "tick_shape = check draws the square");
    assert_ne!(mark_square, mark_cross, "tick_shape = cross draws the square");
    assert_ne!(mark_check, mark_cross, "check and cross draw the same mark");

    // With nothing to mark, the same two themes must agree pixel for
    // pixel: what moved was the MARK and nothing else.
    let (off_check, off_cross) = pair(
        &mut fonts,
        "[checkbox]\ntick_shape = check\n",
        "[checkbox]\ntick_shape = cross\n",
        unchecked,
    );
    assert_eq!(off_check, off_cross, "tick_shape moved something other than the mark");

    // ---- checkbox.role -----------------------------------------------
    // The label is set in the role the binding names, not in `body`
    // because the code says so.
    let (body, caption) = pair(
        &mut fonts,
        "[checkbox]\nrole = body\n",
        "[checkbox]\nrole = caption\n",
        unchecked,
    );
    assert_ne!(body, caption, "checkbox.role does not reach the label");

    // ---- focus.ring.style --------------------------------------------
    // Solid is one band; dashed is marks with gaps between them.
    let ring = |c: &mut Ctx| focus_ring::draw(c, ROW);
    let (solid, dashed) = pair(
        &mut fonts,
        "[focus]\nring.style = solid\n",
        "[focus]\nring.style = dashed\n",
        ring,
    );
    assert!(!solid.is_empty(), "the solid ring drew nothing — the probe is wrong");
    assert!(
        dashed.len() > solid.len(),
        "focus.ring.style = dashed drew {} points against solid's {}",
        dashed.len(),
        solid.len()
    );

    // ---- list.label_role, in the drop-down ---------------------------
    // The binding a drop-down ELEMENT's label takes. The element wears
    // the anchor's dress and the list's type ladder, so the size comes
    // from here and not from `[button].role`; `dropdown_element_is_the_
    // anchor` is where that split is stated in full.
    let names = vec!["ALPHA".to_string(), "BETA".to_string()];
    let list = |c: &mut Ctx| {
        dropdown::accordion(
            c,
            ROW,
            30.0,
            &names,
            1.0,
            &dropdown::AccordionStyle::default(),
            &mut ScrollView::new(),
        );
    };
    let (rows_body, rows_caption) = pair(
        &mut fonts,
        "[list]\nlabel_role = body\n",
        "[list]\nlabel_role = caption\n",
        list,
    );
    assert_ne!(rows_body, rows_caption, "list.label_role does not reach a drop-down row");

    // ---- menu.anchor_width -------------------------------------------
    // A narrow anchor under `min_w` gets the declared floor; under
    // `anchor` it keeps the anchor's own width, as it always did.
    //
    // What `anchor_width` decides is the ELEMENT's width outright. There
    // is no box around the list any more and so no `[menu].pad` between
    // the two: an element hangs off the anchor's bottom EDGE, which is
    // the anchor's width less whatever `[button].skew` takes off it.
    let narrow = Rect::new(100.0, 400.0, 40.0, 36.0);
    let rects = |fonts: &mut FontSystem| {
        let mut dl = DrawList::new();
        let mut c = ctx(&mut dl, fonts);
        dropdown::accordion(
            &mut c,
            narrow,
            30.0,
            &names,
            1.0,
            &dropdown::AccordionStyle::default(),
            &mut ScrollView::new(),
        )
    };
    skin("[menu]\nanchor_width = anchor\n");
    let skew = theme::resolved().px(theme::id("button.skew").expect("[button] declares skew"));
    assert_eq!(rects(&mut fonts)[0].0.w, narrow.w - skew, "the list left its anchor's width");
    skin("[menu]\nanchor_width = min_w\n");
    let floor = theme::resolved().px(theme::id("menu.min_w").expect("[menu] declares min_w"));
    assert_eq!(
        rects(&mut fonts)[0].0.w,
        floor,
        "menu.anchor_width = min_w did not widen the list to its floor"
    );

    // ---- menu.anchor_gap ---------------------------------------------
    // The one number the blind is spaced by: the air under the anchor
    // and the air between any two elements. `dropdown_blind` proves the
    // two seams are the SAME number; this audit's job is the token's own
    // reach — a theme moves it and the picture follows.
    let tight = {
        skin("[menu]\nanchor_gap = @space.0\n");
        rects(&mut fonts)[1].0.y
    };
    let airy = {
        skin("[menu]\nanchor_gap = 4u\n");
        rects(&mut fonts)[1].0.y
    };
    assert!(
        airy > tight,
        "menu.anchor_gap does not open the seam between two elements ({airy} against \
         {tight})"
    );
}
