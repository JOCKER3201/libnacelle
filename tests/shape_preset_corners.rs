//! The road from a `shape.*` preset to a draw record, walked on the
//! REAL master (f3 K6).
//!
//! Sixteen presets have carried `corners_tl/tr/br/bl` since the theme
//! engine was written, each saying in its own comment that it overrides
//! one corner, and until K6 not one line of Rust read them. That is two
//! failures at once, and the unit tests only catch the second:
//!
//!   * the master declared each key as a bare `same_as_parent` WORD,
//!     which can hold a style or a length but never the pair the
//!     comment describes — so a theme that wrote `corners_tl =
//!     [ chamfer, 2u ]` had its whole line thrown away as an unknown
//!     key, and heard about it only in a diagnostic;
//!   * nothing read them anyway.
//!
//! This drives the shipped `default.theme` through the shipped reader
//! and the shipped emitter, which is the only place both halves are on
//! trial at once.

use nacelle::draw::{Corner, CornerStyle, DrawList, ShapeKind, ShapeSpec};
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::theme::{self, Color};
use nacelle::view::{paint, CtxSurface};
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const INK: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

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

/// Every per-corner key in the master is the two-slot PAIR its own
/// comment describes, and the one-slot form it used to be is gone.
///
/// A theme cannot pin a slot the schema does not declare — the cascade
/// looks the family up by name and warns the whole line away — so this
/// is the assertion that the door is open at all.
#[test]
fn every_preset_declares_its_corners_as_a_pair() {
    let _ = theme::resolved();
    let presets = [
        "panel", "card", "window", "button", "button_alt", "icon_tile", "badge", "chip",
        "field", "tab", "taskbar", "hex", "tile", "key", "modal", "spare",
    ];
    let same = theme::expr::sentinel("same_as_parent").expect("§5.0's own sentinel");
    for p in presets {
        for slot in ["tl", "tr", "br", "bl"] {
            let key = format!("shape.{p}.corners_{slot}");
            assert!(
                theme::id(&format!("{key}[0]")).is_some() && theme::id(&format!("{key}[1]")).is_some(),
                "{key} is not a pair: a theme cannot state a style AND a length for one corner"
            );
            assert!(
                theme::id(&key).is_none(),
                "{key} is still a token of its own — two spellings of one key is one too many"
            );
        }
        // And the preset's own `corners` is the parent the four inherit.
        assert!(theme::id(&format!("shape.{p}.corners[0]")).is_some());
    }
    // The master leaves every slot inheriting but the two `button_alt`
    // pins square on purpose — the one asymmetric preset it ships, and
    // the proof that the pair is reachable rather than merely declared.
    let t = theme::resolved();
    let px = |n: &str| t.px(theme::id(n).expect(n));
    assert_eq!(px("shape.panel.corners_tl[0]"), same, "a panel inherits every corner");
    assert_eq!(px("shape.panel.corners_tl[1]"), same);
    assert_eq!(px("shape.button_alt.corners_tr[1]"), 0.0, "the pinned square has no radius");
    assert_ne!(px("shape.button_alt.corners_tr[0]"), same, "and states its own cut");
}

/// The reader, on the master: `shape.button_alt` is the one preset that
/// asks for two different corners, and it gets them.
#[test]
fn the_asymmetric_preset_comes_out_asymmetric() {
    let mut dl = DrawList::new();
    let mut fonts = FontSystem::new();
    let mut c = ctx(&mut dl, &mut fonts);
    let mut sf = CtxSurface::new(&mut c);
    let r = Rect::new(0.0, 0.0, 120.0, 40.0);
    let p = paint::preset(&mut sf, "shape.button_alt", r);

    assert_eq!(p.kind, ShapeKind::Box);
    // tl and br take the preset's own chamfer; tr and bl are pinned
    // square. Four corners, two answers — which is the whole of what
    // these tokens were written for and none of what they could do.
    assert_eq!(p.corners[0].style, CornerStyle::Chamfer, "tl lost the preset's cut");
    assert_eq!(p.corners[2].style, CornerStyle::Chamfer, "br lost the preset's cut");
    assert_eq!(p.corners[1], Corner::SQUARE, "tr was not the square the master pinned");
    assert_eq!(p.corners[3], Corner::SQUARE, "bl was not the square the master pinned");
    assert!(p.corners[0].size > 0.0, "the inherited cut arrived with no length");
    assert_eq!(p.corners[0].size, p.corners[2].size, "one parent, two readings");

    // `shape.tab` is the master's OTHER asymmetric preset, and its own
    // comment says why: "the bottom corners stay square so the tab meets
    // its strip". Two presets, two different asymmetries, both of them
    // written down years before anything could read them.
    let mut c = ctx(&mut dl, &mut fonts);
    let mut sf = CtxSurface::new(&mut c);
    let tab = paint::preset(&mut sf, "shape.tab", r);
    assert_eq!(tab.corners[0].style, CornerStyle::Chamfer, "the tab's top-left");
    assert_eq!(tab.corners[1].style, CornerStyle::Chamfer, "the tab's top-right");
    assert_eq!(tab.corners[2], Corner::SQUARE, "the tab stopped meeting its strip");
    assert_eq!(tab.corners[3], Corner::SQUARE, "the tab stopped meeting its strip");
}

/// The two shape words, on the master, all the way into the record:
/// `shape.hex` is a hexagon and `shape.taskbar` a chevron, and the
/// record's bits 8-11 say so.
#[test]
fn the_shape_words_reach_the_record_as_kinds() {
    let mut dl = DrawList::new();
    let mut fonts = FontSystem::new();
    let mut c = ctx(&mut dl, &mut fonts);
    let mut sf = CtxSurface::new(&mut c);
    let r = Rect::new(0.0, 0.0, 60.0, 60.0);
    let hex = paint::preset(&mut sf, "shape.hex", r);
    let bar = paint::preset(&mut sf, "shape.taskbar", Rect::new(0.0, 0.0, 200.0, 40.0));
    drop(sf);

    // `shape.hex.orientation = pointy` in the master: a vertex at the
    // top, which is a thirty-degree turn on the lattice.
    assert_eq!(hex.kind, ShapeKind::Hex { turn: std::f32::consts::FRAC_PI_6 });
    // `chevron_depth = 50%` of the HEIGHT, `chevron_dir = both`.
    assert_eq!(bar.kind, ShapeKind::Chevron { left: 20.0, right: 20.0 });

    let mut dl = DrawList::new();
    dl.shape(&ShapeSpec {
        rect: r,
        corners: hex.corners,
        kind: hex.kind,
        fill: Some(INK),
        stroke: None,
        glass: None,
        soft: None,
    });
    let rec = dl.shapes()[0];
    assert_eq!((rec.flags >> 8) & 0xF, 2, "the record did not come out a hexagon");
    assert_eq!(rec.arc_dir, std::f32::consts::FRAC_PI_6);
    // A square rect: the pointy hexagon is limited by its own height.
    assert!((rec.corner[0] - 30.0 * 3.0f32.sqrt() * 0.5).abs() <= 1e-3, "{}", rec.corner[0]);
    // And the field the shader will read agrees with the record.
    assert!(nacelle::sdf::d_record(&rec, [0.0, 30.0]).abs() <= 1e-2, "the vertex missed the rect");
}
