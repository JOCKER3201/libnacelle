//! "Wszystkie elementy listy mają wyglądać jak kotwica" — every element
//! of the list is to look like the anchor.
//!
//! The anchor a drop-down hangs from is a BUTTON, drawn by
//! [`nacelle::object::button`]. So the claim has an exact form, and this
//! file states it in the strongest one there is: the commands an element
//! puts in the register are the commands [`button::dress`] puts in the
//! register for the same rectangle on the same rung — the same plate,
//! the same state wash, the same ring, the same corner, command for
//! command and colour for colour. Not "the same tokens, read again
//! here": the same call. A second reading would drift the first time a
//! theme moved one of the three, and that drift is what
//! [`nacelle::object::elev`] was pulled out of `panel.rs` to end.
//!
//! What the list still owns is the LABEL, and the file states that too:
//! a row's label is set in the role its list binds (`[list].label_role`)
//! while a cap is set in `[button].role`. The dress is shared, the type
//! ladder is not — and the owner's one instruction about type was that
//! the label's px must not move.
//!
//! Every claim is measured out of a recording [`DrawList`], and every
//! claim has its negative control: the rung that must NOT match, the
//! token that must NOT reach, the fixture that has to move the number.
//!
//! One test in a binary of its own: the resolved theme is process-wide
//! (§7.1 hands every draw path the same `&'static ResolvedTheme`), so a
//! test that swaps themes must not run beside one that reads them.

use nacelle::draw::{DrawCmd, DrawList};
use nacelle::font::FontSystem;
use nacelle::object::button::{self, ButtonState};
use nacelle::object::dropdown::{self, AccordionStyle};
use nacelle::theme;
use nacelle::view::ScrollView;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const ITEM_H: f32 = 30.0;
const ANCHOR: Rect = Rect { x: 200.0, y: 300.0, w: 400.0, h: 36.0 };
const NAMES: [&str; 3] = ["ALPHA", "BETA", "GAMMA"];
const AWAY: (f32, f32) = (-1.0, -1.0);

/// The size the master sets a drop-down element's label in, in device px
/// at this viewport. The owner named the number, so the number is
/// written down: `[list].label_role = body`, `type.body.size` in `u`,
/// and a 1080-px viewport at scale 1.
///
/// A sentinel and not a source of truth — every other stage here asks
/// the theme. Its job is to fail if a change moves the size the owner
/// said was to stay where it is.
const LABEL_PX: f32 = 13.338;

fn names() -> Vec<String> {
    NAMES.iter().map(|s| s.to_string()).collect()
}

fn skin(body: &str) {
    let path = std::env::temp_dir().join(format!("nacelle-element-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        format!("[meta]\nschema = 1\nname = \"Fixture\"\nbase = \"default\"\n\n{body}"),
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(theme::LoadRequest { path: Some(path.clone()), ..Default::default() });
    let _ = std::fs::remove_file(&path);
    theme::set_viewport(H, 1.0);
}

fn master() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
}

fn px_of(name: &str) -> f32 {
    let t = theme::resolved();
    t.px(theme::id(name).unwrap_or_else(|| panic!("the master declares {name}")))
}

fn ctx<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem, mouse: (f32, f32)) -> Ctx<'a> {
    Ctx {
        access: None,
        dl,
        fonts,
        w: W,
        h: H,
        t: 0.0,
        mouse: nacelle::pointer::Pointer::new(mouse.0, mouse.1),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    }
}

/// The open list, recorded, with the rectangles it handed back.
fn shoot(
    fonts: &mut FontSystem,
    current: Option<usize>,
    mouse: (f32, f32),
    p: f32,
) -> (DrawList, Vec<(Rect, bool)>) {
    let names = names();
    let mut dl = DrawList::recording();
    let rects = {
        let mut c = ctx(&mut dl, fonts, mouse);
        // A fresh, untouched offset: three elements fit their frame, so
        // the scroll is a passenger in every stage of this file.
        dropdown::accordion(
            &mut c,
            ANCHOR,
            ITEM_H,
            &names,
            p,
            &AccordionStyle { current, ..AccordionStyle::default() },
            &mut ScrollView::new(),
        )
    };
    (dl, rects)
}

/// What [`button::dress`] alone writes for one rectangle on one rung —
/// the anchor's own dress, in isolation.
fn anchor_dress(fonts: &mut FontSystem, r: Rect, st: ButtonState) -> Vec<DrawCmd> {
    let mut dl = DrawList::recording();
    {
        let mut c = ctx(&mut dl, fonts, AWAY);
        button::dress(&mut c, r, st);
    }
    dl.cmds().to_vec()
}

/// Element `i`'s own commands, taken out of the list's register: the run
/// of shape commands between its label and the previous one. Clips and
/// text are dropped, so what is left is the DRESS and only the dress.
fn element_dress(dl: &DrawList, i: usize) -> Vec<DrawCmd> {
    let mut runs: Vec<Vec<DrawCmd>> = Vec::new();
    let mut run: Vec<DrawCmd> = Vec::new();
    for c in dl.cmds() {
        match c {
            DrawCmd::ClipPush { .. } | DrawCmd::ClipPop => {}
            // A label closes its element.
            DrawCmd::Text { .. } => {
                runs.push(std::mem::take(&mut run));
            }
            other => run.push(other.clone()),
        }
    }
    assert!(run.is_empty(), "the last element drew shapes after its label");
    assert_eq!(runs.len(), NAMES.len(), "one label per element");
    runs[i].clone()
}

/// One label as the register kept it: where it sits, in which font
/// slot, at what px, stepped by what figure box, in what ink.
#[derive(Clone, PartialEq, Debug)]
struct Label {
    at: [f32; 2],
    font: u8,
    px: f32,
    /// The figure advance the run was stepped by; 0.0 for a
    /// proportional one.
    fig: f32,
    ink: [f32; 4],
    text: String,
}

fn labels(dl: &DrawList) -> Vec<Label> {
    dl.cmds()
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { at, font, px, tabular, color, text, .. } => Some(Label {
                at: *at,
                font: *font,
                px: *px,
                fig: *tabular,
                ink: [color.r, color.g, color.b, color.a],
                text: text.clone(),
            }),
            _ => None,
        })
        .collect()
}

/// The rectangle the list drew element `i` on, read off its first shape.
fn slat(dl: &DrawList, i: usize) -> Rect {
    match element_dress(dl, i)[0] {
        DrawCmd::RingFill { r, .. } => Rect::new(r[0], r[1], r[2], r[3]),
        ref other => panic!("element {i} opens with {other:?}, not with a plate"),
    }
}

// =====================================================================

/// One test in the binary, and every stage inside it: `skin` swaps the
/// process-wide resolved theme, so two stages running in parallel
/// threads would each be measuring the other's fixture.
#[test]
fn an_element_of_the_list_is_the_anchor_drawn_again() {
    master();
    let mut fonts = FontSystem::new();
    the_dress_is_the_anchors_own_call(&mut fonts);
    the_ladder_is_the_anchors_ladder(&mut fonts);
    the_shape_is_the_buttons_and_not_the_lists(&mut fonts);
    the_label_is_the_lists_own(&mut fonts);
    an_element_hangs_off_the_anchors_bottom_edge(&mut fonts);
    master();
}

// --------------------------------------------------------------------

/// Command for command, colour for colour: an element's dress IS
/// [`button::dress`].
fn the_dress_is_the_anchors_own_call(fonts: &mut FontSystem) {
    let (open, _) = shoot(fonts, None, AWAY, 1.0);
    for i in 0..NAMES.len() {
        let r = slat(&open, i);
        let mine = element_dress(&open, i);
        assert!(!mine.is_empty(), "element {i} drew no dress at all");
        assert_eq!(
            mine,
            anchor_dress(fonts, r, ButtonState::default()),
            "element {i} is not dressed by the anchor's own code — it is a second \
             assembly of the same idea, and the two will drift"
        );
    }

    // NEGATIVE CONTROL. `dress` on a DIFFERENT rung writes a different
    // register, so the equality above is a claim with a wrong answer
    // available and not an identity that holds for anything.
    let r = slat(&open, 0);
    assert_ne!(
        element_dress(&open, 0),
        anchor_dress(fonts, r, ButtonState { selected: true, ..ButtonState::default() }),
        "the button ladder's idle and selected rungs write the same commands — this \
         machine cannot tell one dress from another and the stage above proves nothing"
    );
    // ...and on a different RECTANGLE likewise: the comparison is
    // pinned to the geometry the list actually drew.
    assert_ne!(
        element_dress(&open, 0),
        anchor_dress(fonts, Rect::new(r.x, r.y + 1.0, r.w, r.h), ButtonState::default()),
        "the dress does not depend on the rectangle it is given"
    );
}

/// Idle, hover, selected, selected_hover — the anchor's four rungs, worn
/// by the element the same conditions put on them.
fn the_ladder_is_the_anchors_ladder(fonts: &mut FontSystem) {
    // The pointer inside element 1, wherever the blind puts it.
    let (open, rects) = shoot(fonts, None, AWAY, 1.0);
    let mid = rects[1].0;
    let on_mid = (mid.x + 10.0, mid.y + mid.h / 2.0);
    let r = slat(&open, 1);

    for (name, current, mouse, st) in [
        ("idle", None, AWAY, ButtonState::default()),
        ("hover", None, on_mid, ButtonState { hover: true, ..ButtonState::default() }),
        ("selected", Some(1), AWAY, ButtonState { selected: true, ..ButtonState::default() }),
        (
            "selected_hover",
            Some(1),
            on_mid,
            ButtonState { hover: true, selected: true, ..ButtonState::default() },
        ),
    ] {
        let (dl, _) = shoot(fonts, current, mouse, 1.0);
        assert_eq!(
            element_dress(&dl, 1),
            anchor_dress(fonts, r, st),
            "an element that is {name} is not dressed as a {name} button"
        );
    }

    // And the four really are four: a ladder whose rungs were equal
    // would satisfy every line above and show the user one flat list.
    let mut seen: Vec<Vec<DrawCmd>> = Vec::new();
    for st in [
        ButtonState::default(),
        ButtonState { hover: true, ..ButtonState::default() },
        ButtonState { selected: true, ..ButtonState::default() },
        ButtonState { hover: true, selected: true, ..ButtonState::default() },
    ] {
        let d = anchor_dress(fonts, r, st);
        assert!(!seen.contains(&d), "two rungs of the button ladder draw the same picture");
        seen.push(d);
    }

    // The mark travels with the INDEX and not with a position: whichever
    // element is named is the one that changes, and its neighbours do not.
    let (last, _) = shoot(fonts, Some(2), AWAY, 1.0);
    let (none, _) = shoot(fonts, None, AWAY, 1.0);
    assert_eq!(element_dress(&last, 0), element_dress(&none, 0), "an untouched element moved");
    assert_eq!(element_dress(&last, 1), element_dress(&none, 1));
    assert_ne!(
        element_dress(&last, 2),
        element_dress(&none, 2),
        "the element in force is drawn exactly like the ones that are not — which is \
         the window with no current theme visible anywhere in it"
    );
    // The label follows the rung too: the ink is the one the dress chose.
    assert_ne!(
        labels(&last)[2].ink,
        labels(&none)[2].ink,
        "the element in force kept the resting ink"
    );
}

/// The corner is `[button]`'s, and `[list]`'s corner does not reach it.
fn the_shape_is_the_buttons_and_not_the_lists(fonts: &mut FontSystem) {
    let corner_of = |dl: &DrawList| match element_dress(dl, 0)[0] {
        DrawCmd::RingFill { corners, .. } => corners[0],
        ref other => panic!("an element opens with {other:?}"),
    };

    master();
    let (open, _) = shoot(fonts, None, AWAY, 1.0);
    let c = corner_of(&open);
    assert!(
        (c.size - px_of("button.corner")).abs() < 0.01,
        "an element is cut at {} where [button].corner says {}",
        c.size,
        px_of("button.corner")
    );

    // The token moves it...
    skin("[button]\ncorner = 0u\n");
    let (flat, _) = shoot(fonts, None, AWAY, 1.0);
    assert_eq!(corner_of(&flat).size, 0.0, "[button].corner does not reach an element");
    skin("[button]\ncorner_style = square\n");
    let (square, _) = shoot(fonts, None, AWAY, 1.0);
    assert_eq!(
        corner_of(&square).style,
        nacelle::draw::CornerStyle::Square,
        "[button].corner_style does not reach an element"
    );

    // ...and `[list]`'s does NOT. This is the negative control that
    // separates "the element is a button" from "the element is a list
    // row that happens to be rounded": the old design cut its plate to
    // `[list].corner`, and an element that still answered it would be
    // wearing two shapes at once.
    skin("[list]\ncorner = 0u\ncorner_style = square\n");
    let (under_list, _) = shoot(fonts, None, AWAY, 1.0);
    let c = corner_of(&under_list);
    assert!(
        (c.size - px_of("button.corner")).abs() < 0.01
            && c.style != nacelle::draw::CornerStyle::Square,
        "[list].corner still cuts an element — the element is not the anchor, it is \
         a list row in disguise"
    );
    master();
}

/// The dress is shared; the type ladder is not.
fn the_label_is_the_lists_own(fonts: &mut FontSystem) {
    master();
    let (open, _) = shoot(fonts, None, AWAY, 1.0);
    let ls = labels(&open);
    assert_eq!(ls.len(), NAMES.len());

    // The size the owner said must not move.
    for l in &ls {
        assert_eq!(l.px, px_of("type.body.size"), "an element's label left type.body's size");
        assert!(
            (l.px - LABEL_PX).abs() < 0.001,
            "an element's label is set at {} px and the owner's number is {LABEL_PX}",
            l.px
        );
    }
    // …centred on the element's own edge.
    for (i, l) in ls.iter().enumerate() {
        assert_eq!(l.text, NAMES[i]);
        let r = slat(&open, i);
        assert!((l.at[0] - r.cx()).abs() < 0.01, "element {i}'s label is not centred on it");
    }

    // `[list].label_role` binds it...
    skin("[list]\nlabel_role = caption\n");
    let (dl, _) = shoot(fonts, None, AWAY, 1.0);
    let caption_px = labels(&dl)[0].px;
    assert_ne!(caption_px, LABEL_PX, "[list].label_role does not reach an element's label");

    // ...and `[button].role` — the binding that sets the ANCHOR's cap —
    // does not. The negative control for "the dress is shared, the type
    // ladder is not": if the element took the button's role as well as
    // the button's plate, this fixture would move its label.
    skin("[button]\nrole = caption\n");
    let (dl, _) = shoot(fonts, None, AWAY, 1.0);
    assert_eq!(
        labels(&dl)[0].px,
        LABEL_PX,
        "an element's label answers [button].role — the element took the anchor's \
         type ladder as well as its dress"
    );
    // Same fixture, and it really does move the ANCHOR's own cap: proof
    // the token baked and the line above is a refusal, not a no-op.
    let cap = {
        let mut dl = DrawList::recording();
        {
            let mut c = ctx(&mut dl, fonts, AWAY);
            button::draw(&mut c, ANCHOR, "ALPHA", ButtonState::default());
        }
        labels(&dl)[0].px
    };
    assert_eq!(cap, caption_px, "[button].role did not bake — the refusal above is untested");

    // The face is the ROLE's, not a slot named in this object.
    skin("[type]\nbody.face = ui\n");
    let (dl, _) = shoot(fonts, None, AWAY, 1.0);
    let ui = labels(&dl)[0].font;
    skin("[type]\nbody.face = mono\n");
    let (dl, _) = shoot(fonts, None, AWAY, 1.0);
    let mono = labels(&dl)[0].font;
    assert_ne!(ui, mono, "type.body.face moves no element: the slot is written into the object");

    // The figure box is the role's too.
    skin("[type]\nbody.tabular = false\n");
    let (dl, _) = shoot(fonts, None, AWAY, 1.0);
    assert!(labels(&dl).iter().all(|l| l.fig == 0.0), "an element was boxed under tabular = false");
    skin("[type]\nbody.tabular = true\n");
    let (dl, _) = shoot(fonts, None, AWAY, 1.0);
    assert!(labels(&dl).iter().all(|l| l.fig > 0.0), "type.body.tabular does not reach an element");

    // And an element still coming out drops its label below the share
    // `[list].unfold_text_threshold` names.
    skin("[list]\nunfold_text_threshold = 0.7\n");
    let shy = {
        let (dl, _) = shoot(fonts, None, AWAY, 0.2);
        labels(&dl).len()
    };
    skin("[list]\nunfold_text_threshold = 0.0\n");
    let eager = {
        let (dl, _) = shoot(fonts, None, AWAY, 0.2);
        labels(&dl).len()
    };
    assert!(
        eager > shy,
        "[list].unfold_text_threshold does not decide when an emerging element takes \
         its label ({eager} labels against {shy})"
    );
    master();
}

/// The elements start at the anchor's left edge and run to the end of
/// the anchor's BOTTOM edge — shear and width floor included.
fn an_element_hangs_off_the_anchors_bottom_edge(fonts: &mut FontSystem) {
    master();
    let (open, _) = shoot(fonts, None, AWAY, 1.0);
    let r = slat(&open, 0);
    let skew = px_of("button.skew");
    assert_eq!(r.x, ANCHOR.x, "an element left the anchor's left edge");
    assert_eq!(r.w, ANCHOR.w - skew, "an element is not as wide as the anchor's bottom edge");

    // A theme that shears its buttons shortens that edge, and the
    // elements follow it. The master shears nothing, so this fixture is
    // the only thing that can tell "follows the anchor" from "is as wide
    // as the anchor".
    skin("[button]\nskew = 3u\n");
    let sheared = px_of("button.skew");
    assert!(sheared > 0.0, "the fixture's own shear did not bake");
    let (dl, _) = shoot(fonts, None, AWAY, 1.0);
    assert_eq!(
        slat(&dl, 0).w,
        ANCHOR.w - sheared,
        "an element kept the anchor's full width under a shear that shortened the \
         edge it hangs from"
    );
    assert!(
        (labels(&dl)[0].at[0] - (ANCHOR.x + (ANCHOR.w - sheared) / 2.0)).abs() < 0.01,
        "the label centred on the anchor's box instead of on the edge the element \
         actually occupies"
    );

    // `[menu].anchor_width` decides whether the anchor's edge is the
    // whole story. A narrow anchor under `min_w` gets the floor; under
    // `anchor` it keeps its own width, however unreadable.
    let narrow = Rect::new(100.0, 400.0, 40.0, 36.0);
    let width_at = |fonts: &mut FontSystem| {
        let names = names();
        let mut dl = DrawList::recording();
        {
            let mut c = ctx(&mut dl, fonts, AWAY);
            dropdown::accordion(
                &mut c,
                narrow,
                ITEM_H,
                &names,
                1.0,
                &AccordionStyle::default(),
                &mut ScrollView::new(),
            );
        }
        slat(&dl, 0).w
    };
    skin("[menu]\nanchor_width = anchor\n");
    assert_eq!(
        width_at(fonts),
        narrow.w - px_of("button.skew"),
        "the list left its anchor's width"
    );
    skin("[menu]\nanchor_width = min_w\n");
    let floor = px_of("menu.min_w");
    assert!(floor > narrow.w, "the fixture's floor is under the anchor — nothing to see");
    assert_eq!(
        width_at(fonts),
        floor,
        "menu.anchor_width = min_w did not widen the list to its floor"
    );
    master();
}
