//! Slider object: a horizontal track with a filled part and a knob.
//! The caller draws its own label/value text and hit-tests the
//! returned track rectangle.

use super::focus_ring;
use crate::access::{AccessInfo, Role};
use crate::corner::Cuts;
use crate::draw::{Corner, CornerStyle};
use crate::focus::{Caps, FocusId};
use crate::theme::{self, Color, TokenId};
use crate::{Ctx, Rect};
use std::sync::OnceLock;

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// The engine's colour, in the draw list's clothes.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// The four corners a radius token asks for on a `w` x `h` box, with the
/// arc tessellation that radius earns.
///
/// A negative radius is a §5.0 sentinel, and translating one is
/// [`Corner::sized`]'s job — the one place that knows `pill` means half
/// the shorter side, which on the groove is the end cap the master's
/// `@corner.pill` promises.
///
/// HOW the radius is cut is not this file's decision either, and it used
/// to be: `CornerStyle::Round` stood here in Rust, citing [corner]'s old
/// rule that a radius without a sibling is cut round. The siblings exist
/// now — `slider.track_corner_style` and `slider.knob_corner_style` —
/// and `cut` is what the caller read out of them, so a theme writing
/// `corner.mode = chamfer` gets a bevelled groove instead of the one
/// round control left over in a chamfered interface. Zero is spelled
/// Square whatever the theme says, because a zero-radius arc IS a square
/// corner and the square path draws it in one quad.
fn shape(
    t: &theme::ResolvedTheme,
    cut: CornerStyle,
    radius: f32,
    w: f32,
    h: f32,
) -> ([Corner; 4], u8) {
    static SEGMENTS: OnceLock<TokenId> = OnceLock::new();
    let c = Corner::sized(cut, radius, Rect::new(0.0, 0.0, w, h));
    let c = if c.size > 0.0 { c } else { Corner::SQUARE };
    ([c; 4], super::window::corner_segments(t, &SEGMENTS, c.size))
}

/// Draws the track with the knob at position `t` (0..1).
pub fn track(ctx: &mut Ctx, track: Rect, t: f32) {
    static TRACK_COLOR: OnceLock<TokenId> = OnceLock::new();
    static FILL_COLOR: OnceLock<TokenId> = OnceLock::new();
    static KNOB_COLOR: OnceLock<TokenId> = OnceLock::new();
    static TRACK_H: OnceLock<TokenId> = OnceLock::new();
    static FILL_H: OnceLock<TokenId> = OnceLock::new();
    static KNOB_W: OnceLock<TokenId> = OnceLock::new();
    static KNOB_H: OnceLock<TokenId> = OnceLock::new();
    static TRACK_CORNER: OnceLock<TokenId> = OnceLock::new();
    static KNOB_CORNER: OnceLock<TokenId> = OnceLock::new();
    static TRACK_CUT: OnceLock<TokenId> = OnceLock::new();
    static TRACK_CUT_IDX: OnceLock<Cuts> = OnceLock::new();
    static KNOB_CUT: OnceLock<TokenId> = OnceLock::new();
    static KNOB_CUT_IDX: OnceLock<Cuts> = OnceLock::new();
    let th = theme::resolved();
    let cy = track.y + track.h / 2.0;
    let track_h = th.px(tok(&TRACK_H, "slider.track_h"));
    let track_corner = th.px(tok(&TRACK_CORNER, "slider.track_corner"));
    // The pair, both halves from the theme: the length above and the cut
    // here. The master points both slider styles at the button's, so a
    // theme states its corner language once and the groove answers.
    let track_cut =
        crate::corner::style(th, tok(&TRACK_CUT, "slider.track_corner_style"), &TRACK_CUT_IDX);
    // A groove is a box with end caps, not a stroke: `line` is a quad cut
    // square at both ends, and `slider.track_corner` is what says how the
    // ends are shaped.
    let groove = Rect::new(track.x, cy - track_h / 2.0, track.w, track_h);
    let (gc, gseg) = shape(th, track_cut, track_corner, groove.w, groove.h);
    ctx.dl.ring_fill(
        groove,
        &gc,
        gseg,
        col(th.color(tok(&TRACK_COLOR, "slider.track_color"))),
    );
    let t = t.clamp(0.0, 1.0);
    let knob_x = track.x + t * track.w;
    // same_as_parent bakes to a negative sentinel: the fill inherits the
    // track's thickness.
    let mut fill_h = th.px(tok(&FILL_H, "slider.fill_h"));
    if fill_h < 0.0 {
        fill_h = track_h;
    }
    // The filled part lies INSIDE the groove and wears the groove's cap:
    // a square-ended fill in a capsule groove hangs out past the cap.
    let fill = Rect::new(track.x, cy - fill_h / 2.0, knob_x - track.x, fill_h);
    let (fc, fseg) = shape(th, track_cut, track_corner, fill.w, fill.h);
    ctx.dl.ring_fill(
        fill,
        &fc,
        fseg,
        col(th.color(tok(&FILL_COLOR, "slider.fill_color"))),
    );
    // The knob is its own length now, not a cut of the row height.
    let kw = th.px(tok(&KNOB_W, "slider.knob_w"));
    let kh = th.px(tok(&KNOB_H, "slider.knob_h"));
    let knob = Rect::new(knob_x - kw / 2.0, cy - kh / 2.0, kw, kh);
    // The knob's own sibling, which the master derives from the track's:
    // a knob cut one way in a groove cut another is two controls.
    let knob_cut =
        crate::corner::style(th, tok(&KNOB_CUT, "slider.knob_corner_style"), &KNOB_CUT_IDX);
    let (kc, kseg) =
        shape(th, knob_cut, th.px(tok(&KNOB_CORNER, "slider.knob_corner")), kw, kh);
    ctx.dl.ring_fill(
        knob,
        &kc,
        kseg,
        col(th.color(tok(&KNOB_COLOR, "slider.knob_color"))),
    );
}

/// [`track`], joined to the world's focus chain. A slider EATS the
/// arrows (`GREEDY_ARROWS`): while it owns focus, Left/Right adjust the
/// value instead of navigating — the router dispatches them to the
/// caller's value logic. Tab still leaves. The ring wraps the track
/// rect the caller already hit-tests.
pub fn track_focusable(ctx: &mut Ctx, r: Rect, t: f32, id: FocusId) {
    let f = ctx.focus.as_deref_mut().map(|fc| {
        fc.register(id, r, Caps::GREEDY_ARROWS, AccessInfo::new(Role::Slider, ""))
    });
    track(ctx, r, t);
    focus_ring::draw_faded(ctx, r, f.map_or(false, |f| f.ring));
}
