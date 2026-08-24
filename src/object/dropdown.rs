//! Venetian-blind drop-down list: an anchor, and N SEPARATE elements
//! that slide out from under it.
//!
//! THERE IS NO BOX. The list used to occupy a surface level of its own
//! — `[elev.popover]`, one bed, one ring, the rows kept inside it by
//! `menu.pad` — and the owner looked at that and asked for the opposite:
//! the frame around the WHOLE is gone, and what is left is the anchor
//! plus a column of elements each of which is a complete object. A frame
//! around a group says "these belong to one body"; the owner's picture
//! says "these are N things you may pick", and N things are N frames.
//!
//! AN ELEMENT IS THE ANCHOR. The anchor is a button, so an element is
//! drawn by [`super::button::dress`] — the very code that draws the
//! anchor — and therefore wears the anchor's plate (`shape.button.fill`),
//! the anchor's corner (`button.corner`, `button.corner_style`), the
//! anchor's ring, and the anchor's DICTIONARY (the `button` class ladder:
//! idle, hover, selected, selected_hover). Not "the same tokens read
//! again here": the same call. What this file still owns is the LABEL,
//! because a row's label is set in the role its list binds
//! (`list.label_role` → `body`) and a cap is set in `button.role` — the
//! dress is shared, the type ladder is not.
//!
//! ONE GAP. `menu.anchor_gap` stands between the anchor and the first
//! element AND between every pair of elements. One number, not two: the
//! owner asked for every row of the list to be spaced like the first,
//! and a list that took its inner gap from `[list].gap` and its outer
//! one from somewhere else could not answer that with a single token.
//! `[list].gap` and `[list].rule` are the furniture of a list drawn as
//! one body and this one is not drawn as one body, so it reads neither.
//!
//! THE BLIND. At `p = 0` every element is stowed UNDER the anchor — one
//! stack, out of sight. At `p = 1` element `i` stands at
//! `anchor.bottom() + gap + i·(item_h + gap)`. The distance element `i`
//! ends up travelling is therefore `d_i = item_h + gap + i·(item_h +
//! gap)`, which grows LINEARLY with `i`: the last element goes furthest,
//! and while the stack is still stowed it is the one on top of it —
//! which it is, because the elements are drawn in index order and the
//! painter's algorithm puts the last one over its neighbours. Pull the
//! cord and the slat that was on top of the pile ends up at the bottom
//! of the blind. The order of the NAMES never changes: `DEFAULT` is the
//! first element at `p = 0` and the first element at `p = 1`. The blind
//! is how they arrive, not what they say.
//!
//! TWO PHASES, ONE CORD (the owner's ask, 2026-08-16: the stack comes
//! OUT from under the anchor first, and only then unfolds — the old law
//! spread every slat from frame zero, so there was nothing between the
//! click and the spread). The cord is pulled at ONE speed: with
//! `D = d_(n-1)` the whole run, element `i` stands at
//! `stowed + min(p·D, d_i)` —
//!
//! * PHASE A, `p·D < d_0`: no element has reached its own distance, so
//!   every `min` answers `p·D` and the stack slides out from under the
//!   anchor as ONE PILE;
//! * PHASE B, after: elements whose `d_i` the cord has passed have
//!   LANDED and stand still; the rest are still the pile, sliding on.
//!   The first element lands first — the blind fills from the top.
//!
//! Where the phases meet is `d_0 / D`, a ratio of travel distances: the
//! split is GEOMETRY, a fact of the layout the theme has no token for
//! (§5.22 — the geometry of motion is a layout fact), so
//! `motion.menu_unfold.duration_ms` covers the whole pull, slide-out and
//! unfold together. At `p = 1` every `min` answers `d_i` and the resting
//! picture is exactly what it always was.
//!
//! FROM UNDER, NOT OVER. The application draws the anchor and this
//! library draws the list AFTERWARDS, so without a clip the elements
//! would slide across the anchor's face on their way down. The list
//! pushes a clip whose top edge is `anchor.bottom()` — everything above
//! the anchor's bottom edge belongs to the anchor — and the elements
//! appear out of it. The clip is the draw list's, and the draw list's
//! clip is a RECTANGLE (`cmd_set_scissor`), so this is exact along a
//! straight bottom edge and only approximate where the anchor's own
//! bottom corners are cut: an emerging element is a full-width sliver at
//! a height where the anchor itself is already narrowing into its
//! rounding, so for the first pixels of the unfold the element's top
//! corners stand slightly proud of the anchor's silhouette. Clipping to
//! the rounding would need a shaped clip, which this draw list does not
//! have. Stated here rather than papered over.
//!
//! THE FRAME. A blind is as tall as its slats until the slats outgrow
//! the window: a list of forty themes or four hundred font families
//! used to hang past the desktop's edge, pressable where it could not
//! be seen. The body now stops at `menu.max_h_frac` of the viewport's
//! height (floored by `menu.max_h_min_px`, the 3.2 companion) and what
//! does not fit SCROLLS inside that frame. The offset, its clamp, its
//! physics and its bar are the toolkit's ([`crate::view::scroll`]): the
//! caller owns the [`ScrollView`] — the list is stateless, an offset is
//! not — and hands the wheel over with the toolkit's sign (positive
//! notches toward the end of the content, so the CALLER NEGATES the
//! platform's delta, exactly as the settings window already does for its
//! pages). The bar is the toolkit's scrollbar in the master's inset
//! lane, carved from the slats' width only while the list scrolls — a
//! short list keeps the anchor's width to the pixel. An element outside
//! the frame is reported with no area and registers nothing: the frame
//! cuts the hits exactly as it cuts the picture, the same rule the
//! foreign-clip fix states one paragraph down.
//!
//! THE CORD AND THE FRAME ARE TWO LAWS, AND THEY COMPOSE IN THIS ORDER:
//! `y_i = stowed + min(p·D, d_i) − offset`. The `min` is the unfold and
//! it is written in the BODY's coordinates — where a slat has got to on
//! its way out from under the anchor — while the offset is the FRAME
//! sliding over that finished body. Capping the sum instead (`min(p·D,
//! d_i − offset)`) would make the cord shorter for every slat the user
//! has scrolled past, and the list would jam short of its end. The
//! frame's own arithmetic (`content`, the cap, the bar) is the RESTING
//! body's, `pitch · n`, and never `p`'s: a list must not shrink its own
//! scrollbar while it is still opening.

use super::button::ButtonState;
use super::focus_ring;
use crate::access::{AccessInfo, Role};
use crate::focus::{Caps, FocusId};
use crate::theme::{self, Color, TokenId};
use crate::view::paint;
use crate::view::scroll::{self, ScrollPhysics, ScrollView, ScrollbarEdge, ScrollbarLook, Snap};
use crate::view::surface::CtxSurface;
use crate::{ui, Ctx, Rect};
use std::sync::OnceLock;

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// The engine's colour, in the draw list's clothes.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// How a list is dressed for this frame — the two things about it that
/// are not its geometry or its contents.
///
/// A struct and not two more free functions: `focus` and `current` are
/// independent, so entry points would multiply as their product, and
/// the pair that draws a focusable list WITH a chosen row is exactly the
/// one the settings window needs. Written the way [`InputStyle`] is
/// written, for the same reason.
///
/// [`InputStyle`]: crate::object::text_input::InputStyle
#[derive(Clone, Copy, Debug, Default)]
pub struct AccordionStyle {
    /// The focus chain root, when the list joins the chain. Every
    /// element of a list AT REST registers as `base.item(i)` (an
    /// element's order is its content's order, so the index is legal),
    /// letting arrows walk the open list and Enter pick — the router
    /// compares the chain's focused id against the same derived ids.
    /// A blind still running registers nothing: its elements are moving,
    /// and a ring on a moving rect is the board-ride pitfall in
    /// miniature.
    pub focus: Option<FocusId>,
    /// The element that is ALREADY in force — the theme now applied, the
    /// layout now loaded — drawn on the button ladder's `selected` rung,
    /// which is the rung the anchor itself wears while its list is open.
    ///
    /// Not a fashion: with the anchor wearing the list's own name, a
    /// list that cannot mark its current element leaves the standing
    /// choice unstated everywhere in the window. `None` says the set has
    /// no member in force, which is not the same as "the first one".
    pub current: Option<usize>,
}

/// Draws the blind at unfold progress `p` (0..1, eased by the caller or
/// pass 1.0 for fully open). Returns the element rectangles in order —
/// AS DRAWN, which for an element still half under the anchor is the
/// half that is out, which for a body longer than its frame is what the
/// frame leaves in view at the current offset, and which for a list
/// unfolding inside somebody else's clip is what that clip leaves of
/// it: the caller hit-tests these, so an element is clickable where it
/// can be seen and nowhere else. An element the frame or the enclosing
/// clip took whole is reported at its place with NO AREA — the entry
/// stays, because the caller maps the index to an act, but there is
/// nothing left to press. The `bool` says whether the whole of it is
/// out AND uncut.
///
/// `scroll` is the body's offset, owned by the caller because the list
/// is drawn fresh every frame and an offset is not: reset it when the
/// list opens, feed it wheel notches with [`ScrollView::wheel`] — the
/// toolkit's sign, positive toward the end, so the caller negates the
/// platform's delta — and this function ticks, clamps and draws the bar.
/// A list shorter than its frame never moves and never shows one, so a
/// caller with a three-element list loses nothing by carrying the state.
///
/// A host that wants the OPENING ANIMATION should not run a clock of
/// its own to make this `p`: [`accordion_at`] takes the moment the list
/// opened and asks `motion.menu_unfold` itself, so the duration, the
/// curve, `motion.scale` and the enabled flag are all the theme's — a
/// private `Instant` with a hard-coded ease honours none of them. This
/// entry stays for the caller that already HAS a progress: the tests,
/// and a list drawn at rest with `1.0`.
///
/// [`AccordionStyle`] carries the rest: whether the elements join the
/// focus chain, and which of them is the one already in force. A list
/// that wants neither passes `&AccordionStyle::default()`.
pub fn accordion(
    ctx: &mut Ctx,
    anchor: Rect,
    item_h: f32,
    names: &[String],
    p: f32,
    style: &AccordionStyle,
    scroll: &mut ScrollView,
) -> Vec<(Rect, bool)> {
    static GAP: OnceLock<TokenId> = OnceLock::new();
    static SKEW: OnceLock<TokenId> = OnceLock::new();
    static THRESHOLD: OnceLock<TokenId> = OnceLock::new();
    static ROLE: OnceLock<TokenId> = OnceLock::new();
    static ANCHOR_W: OnceLock<TokenId> = OnceLock::new();
    static ANCHOR_W_IDX: OnceLock<Option<u16>> = OnceLock::new();
    static MIN_W: OnceLock<TokenId> = OnceLock::new();
    static MAX_H_FRAC: OnceLock<TokenId> = OnceLock::new();
    static MAX_H_MIN_PX: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let role = ui::bound_role(&ROLE, "list.label_role");
    // No `ui_font_scale`: the viewport carries the user's scale into u,
    // and the role's size is written in u — applying it here too squares it.
    let px = role.px(ctx, 1.0);
    // The role's own face, asked of the role. A slot named here would
    // pin every theme's list to the interface family whatever
    // `type.<role>.face` says, which is a design decision taken in Rust.
    let font = role.font();
    let leading = role.leading();
    let tracking = role.tracking_px(px);
    // …and the role's figure box with it, so a list of versions or
    // addresses steps its digits the way the boxed label beside it does.
    // Read ONCE, outside the loop — the box costs a theme read and ten
    // glyph lookups.
    let fig = role.figures(ctx.fonts, font, px);
    // The BOTTOM edge is what the list hangs from, and `button.skew` is
    // what shortens it: under a theme that shears its buttons the
    // anchor's underside is `skew` narrower than its box, so the
    // elements are too. The master leaves the token at zero — a button
    // now wears the same corners as the frames around it and
    // [`super::button::dress`] fills a rectangle, so the shear survives
    // only in [`super::button::quad`], which the focus ring is drawn on.
    // Reading it here keeps the two in step for the theme that brings
    // the parallelogram back.
    let skew = t.px(tok(&SKEW, "button.skew"));
    // Below this SHARE of an element's full height an element that is
    // still coming out draws no label. A fraction and not a length,
    // because a blind's element height is the one its anchor hands it
    // and not `@list.row_h`.
    let text_threshold = item_h * t.px(tok(&THRESHOLD, "list.unfold_text_threshold"));
    // `menu.anchor_width` says whether the anchor's edge is the whole
    // story: under `min_w` the elements still start at that edge, but
    // `menu.min_w` is a floor under their width, so a narrow anchor no
    // longer makes an unreadable list.
    let aw = tok(&ANCHOR_W, "menu.anchor_width");
    let floored = *ANCHOR_W_IDX.get_or_init(|| theme::enum_index(aw, "min_w")) == Some(t.enum_of(aw));
    let mut row_w = anchor.w - skew;
    if floored {
        row_w = row_w.max(t.px(tok(&MIN_W, "menu.min_w")));
    }
    // THE gap — the one below the anchor and the one between any two
    // elements are the same number, read once.
    let gap = t.px(tok(&GAP, "menu.anchor_gap")).max(0.0);
    let p = p.clamp(0.0, 1.0);
    let mut out = Vec::new();
    if names.is_empty() || p <= 0.0 || item_h <= 0.0 || row_w <= 0.0 {
        // A closed blind is not a stack of zero-height elements, it is
        // nothing drawn at all — and a list of nothing is nothing either.
        return out;
    }
    // Where the anchor ends and the world below it begins. Everything
    // above this line belongs to the anchor.
    let horizon = anchor.bottom();
    // THE CLIP THAT WAS ALREADY THERE. The elements are DRAWN under the
    // caller's clip stack — `push_clip` intersects, so a list unfolding
    // inside a scrolled body is cut by that body's box — but the rects
    // handed back used to know only the horizon. A caller aiming at
    // them would then press an element the scissor had already taken:
    // invisible, and clickable, OVER whatever really stood there. So
    // every rect is cut by the stack's top too, which is the
    // intersection of everything pushed — exactly what the picture was
    // cut by. Read BEFORE this function's own frame clip goes on: the
    // horizon and the frame's bottom live in the `top`/`seen`
    // arithmetic below, and the own clip's side edges are the window's,
    // which the rects must NOT be cut to.
    let outer = ctx.dl.clip();
    // Stowed: the whole stack tucked under the anchor, every element at
    // the same place, none of it showing.
    let stowed = horizon - item_h;
    let pitch = item_h + gap;
    // The whole cord: the last element's travel. `p` runs the cord at
    // one speed and every element rides it until its own distance is
    // paid out — the two phases of the header, in one `min`. Safe on
    // `len() - 1`: the empty list returned above.
    let total = item_h + gap + pitch * (names.len() - 1) as f32;
    // THE FRAME. The finished body is one pitch per element — the seam
    // under the anchor plus a slat plus a seam, `names.len()` times over
    // — and it may stand no taller than `menu.max_h_frac` of the
    // viewport, floored by `menu.max_h_min_px` so a tiny window still
    // shows a few elements. What does not fit scrolls: the offset is the
    // caller's [`ScrollView`], ticked here the way the settings window
    // ticks its pages — `Snap::None`, because the clip is real and half
    // an element in the frame is half an element on the screen.
    let content = pitch * names.len() as f32;
    let cap = (ctx.h * t.px(tok(&MAX_H_FRAC, "menu.max_h_frac")))
        .max(t.px(tok(&MAX_H_MIN_PX, "menu.max_h_min_px")))
        .max(0.0);
    let body_h = content.min(cap);
    let frame_bottom = horizon + body_h;
    let scrolls = content > body_h + 0.5;
    scroll.tick(ctx.t, body_h, content, Snap::None, &ScrollPhysics::from_theme());
    let offset = scroll.offset();
    // THE LANE. The master's bar is inset — it stands BESIDE the content
    // (the owner's ask: a bar over the controls read as a defect) — so a
    // body that scrolls carves the bar's lane out of the slats' width.
    // Only then: a list that fits keeps the anchor's width to the pixel,
    // which is every list the toolkit drew before it learnt to scroll.
    let look = ScrollbarLook::from_theme();
    let bar_box_w = row_w;
    let row_w = if scrolls { (row_w - scroll::inset_w(&look)).max(0.0) } else { row_w };
    ctx.dl.push_clip(0.0, horizon, ctx.w, body_h);
    // A blind that has stopped moving is a list; a blind still moving is
    // an animation. Only the first joins the focus chain.
    let at_rest = p >= 1.0;
    let mut ring: Option<Rect> = None;
    for (i, name) in names.iter().enumerate() {
        // Element `i`'s DESTINATION distance: `item_h` to clear the
        // anchor, the gap below it, and one pitch for every element that
        // stands above it — linear in `i`, so the last one goes
        // furthest. What it has travelled so far is the cord's payout
        // capped at that distance: one pile while the cord is short of
        // `d_0` (phase A), landed and still once the cord passes `d_i`
        // (phase B).
        //
        // …and the whole column then stands `offset` higher than that,
        // which is what scrolling a fixed frame over a longer body IS.
        // The two laws compose in this order and only this order: the
        // `min` is the UNFOLD, written in the body's own coordinates,
        // and the offset is the FRAME sliding over the finished body.
        // Subtracting inside the `min` would cap the scroll instead of
        // the travel — an element scrolled up would stop at its landing
        // place and the list would jam a pitch short of its end.
        let d_i = item_h + gap + pitch * i as f32;
        let y = stowed + (p * total).min(d_i) - offset;
        let slat = Rect::new(anchor.x, y, row_w, item_h);
        // What of it is inside the frame. The scissor's own arithmetic,
        // repeated here because the rect handed back has to BE the rect
        // that was drawn: a caller aiming at where an element will
        // eventually be would be aiming at the anchor — or, past the
        // frame's bottom, at whatever the desktop drew under the list.
        let top = y.max(horizon);
        let seen = ((y + item_h).min(frame_bottom) - top).max(0.0);
        let shown = Rect::new(slat.x, top, slat.w, seen);
        // …and what of THAT the enclosing clip leaves standing. The
        // same intersection `push_clip` performed on the way in, so the
        // reported rect, the cover and the hover all agree with the
        // scissor the plate was actually drawn under. With no foreign
        // clip this is `shown`, bit for bit.
        let shown = match outer {
            Some([ox, oy, ow, oh]) => {
                let x0 = shown.x.max(ox);
                let y0 = shown.y.max(oy);
                let x1 = (shown.x + shown.w).min(ox + ow);
                let y1 = (shown.y + shown.h).min(oy + oh);
                Rect::new(x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0))
            }
            None => shown,
        };
        // All out from under the anchor AND uncut by the world around:
        // only such an element joins the focus chain, because a ring on
        // a sliver says "this is the whole object" about a part.
        let full = shown.h >= item_h - 0.5 && shown.w >= row_w - 0.5;
        if at_rest && full {
            if let (Some(base), Some(fc)) = (style.focus, ctx.focus.as_deref_mut()) {
                let access = AccessInfo::new(Role::ComboBox, name.as_str());
                if fc.register(base.item(i), shown, Caps::NONE, access).ring {
                    ring = Some(shown);
                }
            }
        }
        // The pointer is over what it can SEE, which is the same rect
        // the caller was handed — and only if nothing already drawn this
        // frame stands over it.
        // The claim comes FIRST, the question second — the order is the
        // fix, not a preference. `Pointer::begin` reveals the pointer only
        // once as many covers have been recorded as the depth at which it
        // was claimed LAST frame, and last frame this element's own cover
        // was what claimed it. Asking before covering left the count one
        // short, so every element of every unfolded list was occluded by
        // ITSELF and hover never fired anywhere in the toolkit. `cover`'s
        // own doc states the intended shape: claim the box, then draw the
        // controls into it.
        ctx.mouse.cover(shown);
        let hover = ctx.mouse.over(shown);
        // And this element is itself something to stand over. The box that
        // used to claim this ground is gone, so each element claims its
        // own: an element of an open list covers whatever the list was
        // opened on top of.

        // The anchor's own dress, drawn by the anchor's own code. The
        // element in force wears `selected` — the rung the anchor wears
        // while its list is open — and keeps it under the pointer as
        // `selected_hover`, so a hovered current element does not lose
        // its mark exactly while the user decides whether to replace it.
        let ink = super::button::dress(
            ctx,
            slat,
            ButtonState { hover, flash: false, selected: style.current == Some(i) },
        );
        if seen >= text_threshold {
            ctx.dl.text_center_fig(
                ctx.fonts,
                font,
                px,
                slat.cx(),
                slat.y + (item_h - px * leading) / 2.0,
                name,
                col(ink.text),
                tracking,
                &fig,
            );
        }
        out.push((shown, full));
    }
    ctx.dl.pop_clip();
    // The focus ring is drawn OUTSIDE the clip: it is an overlay around
    // an element at rest, it reaches past that element's own edges by
    // whatever `[focus]` states, and a ring cut off at the horizon would
    // report the first element as a different object from the rest.
    if let Some(r) = ring {
        focus_ring::draw(ctx, r);
    }
    // THE BAR. Geometry from the toolkit ([`scroll::scrollbar`]), paint
    // from the toolkit ([`paint::scrollbar`]) — the same two calls the
    // settings window makes for its pages, over the same state, so the
    // list's bar and the page's bar cannot drift apart. It stands in the
    // lane carved above, against the slats' ORIGINAL right edge, and
    // under the master it auto-hides: a list at rest shows no bar until
    // the wheel moves it, exactly like a page.
    if scrolls {
        let area = Rect::new(anchor.x, horizon, bar_box_w, body_h);
        // The band the bar could occupy at its WIDEST: a bar that grows
        // under the pointer must not shrink out from under it.
        let reach = look.w_hover.max(look.w) + look.margin;
        let band = match look.edge {
            ScrollbarEdge::Left => Rect::new(area.x, area.y, reach, area.h),
            ScrollbarEdge::Right => Rect::new(area.right() - reach, area.y, reach, area.h),
        };
        let hovered = ctx.mouse.over(band);
        if let Some(geom) = scroll::scrollbar(area, &look, offset, body_h, content, hovered) {
            let alpha = if hovered {
                1.0
            } else {
                scroll.fade_alpha(ctx.t, look.auto_hide, look.fade_ms)
            };
            paint::scrollbar(&mut CtxSurface::new(ctx), &geom, alpha, hovered, scroll.dragging());
        }
    }
    out
}

/// [`accordion`] with the toolkit holding the cord: `opened_t` is the
/// moment (on `Ctx.t`'s clock) the list was opened, and the unfold
/// progress is `motion.menu_unfold`'s — duration, easing, `motion.scale`
/// and `enabled` all honoured by [`crate::motion::Effect::one_shot`],
/// which freezes AT FULLY OPEN when any of them says "no animation".
///
/// This is the entry a host should call every frame while its list is
/// open (today's hosts keep an `Instant` and a hard-coded ease around
/// `accordion(p)` — the settings window's `draw_dropdown` is the one to
/// migrate). Time comes in as a parameter, so nothing here reads a
/// clock of its own.
///
/// `scroll` is [`accordion`]'s, passed straight through: the toolkit
/// holding the cord does not make the toolkit hold the OFFSET as well.
/// The offset outlives the frame and the unfold does not — a host that
/// let this function own the scroll state would find its list jumping
/// back to the top on every redraw.
pub fn accordion_at(
    ctx: &mut Ctx,
    anchor: Rect,
    item_h: f32,
    names: &[String],
    opened_t: f64,
    style: &AccordionStyle,
    scroll: &mut ScrollView,
) -> Vec<(Rect, bool)> {
    let p = crate::motion::Effect::of("menu_unfold").one_shot(opened_t, ctx.t);
    accordion(ctx, anchor, item_h, names, p, style, scroll)
}
