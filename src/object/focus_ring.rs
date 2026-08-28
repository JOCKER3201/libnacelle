//! The keyboard focus ring — the one overlay every control draws
//! identically, so two objects can never disagree about what focus
//! looks like.
//!
//! Focus is not a state-ladder rung (§5.21): the ring sits AROUND the
//! control, outside its own edge, drawn only while the chain answers
//! `ring = true` — keyboard navigation has happened and no pointer
//! press has hidden it since. At boot neither has happened, so the boot
//! frame keeps its pixels.
//!
//! Because it is not a rung, it cannot ride the class ladder's own
//! crossfade — so it rides a GATE instead ([`crate::motion::gate`] under
//! `motion.focus`, the catalogue entry that had no reader until it),
//! and [`draw_faded`] is what a control calls: every frame, ring or no
//! ring, because a band drawn only while focused has nothing left on
//! screen to fade away. [`draw`] is the unfaded band, for a caller with
//! no clock of its own.
//!
//! Every token is read per frame: `focus.ring.enabled` is
//! a11y-protected and the hc variant may thicken `focus.ring.width`
//! mid-run, so only `TokenId`s are cached (the `OnceLock` idiom the
//! rest of the objects use), never resolved pixels.

use crate::draw::{Corner, CornerStyle};
use crate::font::FontSystem;
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

/// The resolved ring treatment, read fresh each call.
struct Ring {
    w: f32,
    off: f32,
    color: Color,
    /// `focus.ring.style = dashed` resolved to its two lengths, or None
    /// for a solid band.
    dash: Option<(f32, f32)>,
    /// The ring's own corner. A square band around a rounded field is two
    /// shapes claiming one control, and the ring is the overlay a user
    /// sees on the roundest thing on screen.
    cut: CornerStyle,
    radius: f32,
}

/// The treatment, or None while `focus.ring.enabled` is off or the width
/// degrades to nothing.
fn treatment() -> Option<Ring> {
    static ENABLED: OnceLock<TokenId> = OnceLock::new();
    static WIDTH: OnceLock<TokenId> = OnceLock::new();
    static OFFSET: OnceLock<TokenId> = OnceLock::new();
    static COLOR: OnceLock<TokenId> = OnceLock::new();
    static STYLE: OnceLock<TokenId> = OnceLock::new();
    static DASHED: OnceLock<Option<u16>> = OnceLock::new();
    static DASH: OnceLock<TokenId> = OnceLock::new();
    static GAP: OnceLock<TokenId> = OnceLock::new();
    static CORNER: OnceLock<TokenId> = OnceLock::new();
    static CUT: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    if !t.flag(tok(&ENABLED, "focus.ring.enabled")) {
        return None;
    }
    let w = t.px(tok(&WIDTH, "focus.ring.width")).max(0.0);
    if w <= 0.0 {
        return None;
    }
    let off = t.px(tok(&OFFSET, "focus.ring.offset")).max(0.0);
    // Only the word slot is remembered — the enum's own index moves with
    // the theme, so it is read every frame like every other token here.
    let style = tok(&STYLE, "focus.ring.style");
    let dash = (*DASHED.get_or_init(|| theme::enum_index(style, "dashed")) == Some(t.enum_of(style)))
        // The ring's OWN rhythm. It used to borrow border.edge.dash /
        // .gap, whose declaration reserves them for the `segmented` style
        // of a container's outline: a different object at a different
        // scale, which no theme could move one of without moving both.
        .then(|| (t.px(tok(&DASH, "focus.ring.dash")), t.px(tok(&GAP, "focus.ring.gap"))));
    // The cut is a WORD, compared as one: an enum's indices intern in load
    // order, so a remembered index means nothing after a theme swap. The
    // comparison itself is `corner::cut`'s — this file used to hold its
    // own copy of the same three arms, and the ring is exactly the
    // overlay that must not disagree with the control it surrounds.
    // Borrowed, not cloned: the word is read every frame here.
    let cut = crate::ui::with_theme_word(tok(&CUT, "focus.ring.corner_style"), crate::corner::cut);
    Some(Ring {
        w,
        off,
        color: col(t.color(tok(&COLOR, "focus.ring.color"))),
        dash,
        cut,
        radius: t.px(tok(&CORNER, "focus.ring.corner")),
    })
}

/// Draws the keyboard focus ring AROUND `r`, outside the control's own
/// edge: a gap of `focus.ring.offset` between the control and the
/// band's inner face, `focus.ring.width` thick, in `focus.ring.color`,
/// plus the `glow.focus_ring` halo when a theme enables that class.
/// No-op when `focus.ring.enabled` is false.
pub fn draw(ctx: &mut Ctx, r: Rect) {
    ring(ctx, r, 1.0);
}

/// [`draw`], but the ring ARRIVES and LEAVES over `motion.focus` rather
/// than appearing and vanishing between two frames.
///
/// `on` is what the focus chain answers — call this EVERY frame, with
/// false as readily as with true, because a ring that is only drawn while
/// focused has nothing left on screen to fade out. At rest with `on`
/// false the gate is exactly 0 and nothing is drawn at all, which is the
/// pixel this file has always produced; at rest with `on` true the gate
/// is exactly 1 and every colour is the theme's own, untouched.
///
/// Focus is NOT a rung of the state ladder (§5.21) — that is why it needs
/// a gate of its own rather than a rung of `class_state`, and why
/// `motion.focus` sat in the closed catalogue with no reader until here.
pub fn draw_faded(ctx: &mut Ctx, r: Rect, on: bool) {
    let g = crate::motion::gate("focus.ring", r, on, "focus", ctx.t);
    if g > 0.0 {
        ring(ctx, r, g);
    }
}

/// The band itself, at `alpha` of its themed presence.
fn ring(ctx: &mut Ctx, r: Rect, alpha: f32) {
    let Some(mut t) = treatment() else {
        return;
    };
    if alpha < 1.0 {
        // Clamped: an overshooting `custom` curve is meaningful for a
        // position and meaningless for a coverage.
        t.color = t.color.alpha((t.color.a * alpha).clamp(0.0, 1.0));
    }
    // The ring strokes INSIDE its rect, so the rect grows by offset +
    // width on every side and the band lands wholly outside the control:
    // [offset, offset + width] past its edge.
    let d = t.off + t.w;
    let ring = Rect::new(r.x - d, r.y - d, r.w + 2.0 * d, r.h + 2.0 * d);
    // The ring stands `d` outside the control, so its radius grows by the
    // same distance the boundary moved: a concentric arc, which is what
    // keeps the band an even width all the way round the corner.
    let outer = Corner::sized(t.cut, t.radius, r).inset(-d);
    let corners = [outer; 4];
    let seg = crate::draw::ring_segments(outer.size, 0.25, segments_ceiling());
    match t.dash {
        None => ctx.dl.ring(ring, &corners, seg, t.w, t.color),
        // A dash is a stroke centred on its path, so the path is the
        // band's own centreline — half a width inside the ring rect,
        // which is where the solid band's middle lands.
        Some((dash, gap)) => {
            let h = t.w * 0.5;
            let c = Rect::new(ring.x + h, ring.y + h, ring.w - t.w, ring.h - t.w);
            let mut path = Vec::new();
            crate::draw::ring_points(c, &[outer.inset(h); 4], seg, &mut path);
            dashes(ctx, &path, t.w, dash, gap, t.color);
        }
    }
    glow(ctx, ring, outer, t.color);
}

/// The theme's arc-tessellation ceiling, asked per ring like every other
/// token in this file.
fn segments_ceiling() -> u8 {
    static SEGMENTS: OnceLock<TokenId> = OnceLock::new();
    theme::resolved().px(tok(&SEGMENTS, "corner.segments")) as u8
}

/// The parallelogram variant — a button's slanted quad. Same treatment,
/// stroked as a closed polyline centred on the outward-offset outline.
pub fn draw_quad(ctx: &mut Ctx, q: [[f32; 2]; 4]) {
    ring_quad(ctx, q, 1.0);
}

/// [`draw_quad`] on [`draw_faded`]'s clock — the parallelogram's half of
/// the same contract. The key is the quad's bounding box, which is what
/// the shared registry can hold.
pub fn draw_quad_faded(ctx: &mut Ctx, q: [[f32; 2]; 4], on: bool) {
    let (mut x0, mut y0) = (f32::MAX, f32::MAX);
    let (mut x1, mut y1) = (f32::MIN, f32::MIN);
    for p in q {
        x0 = x0.min(p[0]);
        y0 = y0.min(p[1]);
        x1 = x1.max(p[0]);
        y1 = y1.max(p[1]);
    }
    let box_ = Rect::new(x0, y0, x1 - x0, y1 - y0);
    let g = crate::motion::gate("focus.ring", box_, on, "focus", ctx.t);
    if g > 0.0 {
        ring_quad(ctx, q, g);
    }
}

fn ring_quad(ctx: &mut Ctx, q: [[f32; 2]; 4], alpha: f32) {
    let Some(mut t) = treatment() else {
        return;
    };
    if alpha < 1.0 {
        // Clamped: an overshooting `custom` curve is meaningful for a
        // position and meaningless for a coverage.
        t.color = t.color.alpha((t.color.a * alpha).clamp(0.0, 1.0));
    }
    // polyline centres its stroke on the path, so the path runs through
    // the band's middle: offset + width/2 out from the control's edge.
    let outer = offset_convex_quad(q, t.off + t.w * 0.5);
    match t.dash {
        None => ctx.dl.polyline(&outer, t.w, t.color, true),
        Some((dash, gap)) => dashes(ctx, &outer, t.w, dash, gap, t.color),
    }
    // No halo here yet: glow_ring speaks rects only. Default ships the
    // glow class disabled; a theme that enables it halos the
    // rectangular controls, and the parallelograms join when the glow
    // primitives grow a quad form.
}

/// Strokes a CLOSED path as `dash`-long marks separated by `gap`, each
/// mark `w` thick and centred on the path.
///
/// The cycle carries across corners rather than restarting at each one:
/// a ring whose every corner begins a fresh dash reads as four separate
/// strokes. A cycle of no length draws nothing — that is what a theme
/// asking for zero-length dashes asked for, and it is also what keeps
/// the walk finite.
fn dashes(ctx: &mut Ctx, path: &[[f32; 2]], w: f32, dash: f32, gap: f32, color: Color) {
    let step = dash + gap;
    if dash <= 0.0 || step <= 0.0 {
        return;
    }
    // How far into the current cycle the walk already is.
    let mut phase = 0.0f32;
    for i in 0..path.len() {
        let a = path[i];
        let b = path[(i + 1) % path.len()];
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len <= 0.0 {
            continue;
        }
        let (ux, uy) = (dx / len, dy / len);
        let mut s = 0.0f32;
        while s < len {
            if phase < dash {
                let end = (s + (dash - phase)).min(len);
                ctx.dl.line(
                    a[0] + ux * s,
                    a[1] + uy * s,
                    a[0] + ux * end,
                    a[1] + uy * end,
                    w,
                    color,
                );
            }
            let next = (s + (step - phase)).min(len);
            let advance = next - s;
            if advance <= 0.0 {
                break;
            }
            phase = (phase + advance) % step;
            s = next;
        }
    }
}

/// The `glow.focus_ring` halo around the ring band — the `element` tint
/// rule: the halo wears the ring's own resolved colour, at the class's
/// alpha scaled by the one global knob.
fn glow(ctx: &mut Ctx, ring: Rect, corner: Corner, tint: Color) {
    static ON: OnceLock<TokenId> = OnceLock::new();
    static RADIUS: OnceLock<TokenId> = OnceLock::new();
    static ALPHA: OnceLock<TokenId> = OnceLock::new();
    static SCALE: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    if !t.flag(tok(&ON, "glow.focus_ring.enabled")) {
        return;
    }
    let radius = t.px(tok(&RADIUS, "glow.focus_ring.radius")).max(0.0);
    let alpha = (t.px(tok(&ALPHA, "glow.focus_ring.alpha"))
        * t.px(tok(&SCALE, "glow.alpha_scale")))
    .clamp(0.0, 1.0);
    if radius <= 0.0 || alpha <= 0.0 {
        return;
    }
    // The halo wears the band's own corner: a square glow around a
    // rounded ring is the two-shapes bug with the volume turned down.
    let c = [corner; 4];
    let seg = crate::draw::ring_segments(corner.size, 0.25, segments_ceiling());
    ctx.dl.glow_ring(ring, &c, seg, radius, tint.alpha(alpha), FontSystem::mask_soft_uv());
}

/// Offsets a convex quad outward by `d`: each edge's line moves `d`
/// along its outward normal (away from the centroid, whatever the
/// winding) and neighbouring lines re-intersect — the mitre join, exact
/// for a convex quad. Near-parallel neighbours (a degenerate quad) fall
/// back to pushing the vertex along the edge normal.
fn offset_convex_quad(q: [[f32; 2]; 4], d: f32) -> [[f32; 2]; 4] {
    if d <= 0.0 {
        return q;
    }
    let cx = (q[0][0] + q[1][0] + q[2][0] + q[3][0]) * 0.25;
    let cy = (q[0][1] + q[1][1] + q[2][1] + q[3][1]) * 0.25;
    // Each edge as its offset line n·p = c, |n| = 1, n pointing outward.
    let mut lines = [[0.0f32; 3]; 4];
    for i in 0..4 {
        let p = q[i];
        let r = q[(i + 1) % 4];
        let (dx, dy) = (r[0] - p[0], r[1] - p[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-4);
        let (mut nx, mut ny) = (dy / len, -dx / len);
        let (mx, my) = ((p[0] + r[0]) * 0.5 - cx, (p[1] + r[1]) * 0.5 - cy);
        if nx * mx + ny * my < 0.0 {
            nx = -nx;
            ny = -ny;
        }
        lines[i] = [nx, ny, nx * p[0] + ny * p[1] + d];
    }
    let mut out = q;
    for i in 0..4 {
        // Vertex (i+1) joins edge i and edge i+1.
        let a = lines[i];
        let b = lines[(i + 1) % 4];
        let v = (i + 1) % 4;
        let det = a[0] * b[1] - a[1] * b[0];
        if det.abs() < 1e-6 {
            out[v] = [q[v][0] + a[0] * d, q[v][1] + a[1] * d];
        } else {
            out[v] = [
                (a[2] * b[1] - a[1] * b[2]) / det,
                (a[0] * b[2] - a[2] * b[0]) / det,
            ];
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::offset_convex_quad;

    fn close(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn rect_quad_grows_by_d_on_every_side() {
        let q = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let o = offset_convex_quad(q, 2.0);
        assert!(close(o[0], [-2.0, -2.0]), "{:?}", o[0]);
        assert!(close(o[1], [12.0, -2.0]), "{:?}", o[1]);
        assert!(close(o[2], [12.0, 12.0]), "{:?}", o[2]);
        assert!(close(o[3], [-2.0, 12.0]), "{:?}", o[3]);
    }

    #[test]
    fn winding_does_not_matter() {
        // The same square, wound the other way round.
        let q = [[0.0, 10.0], [10.0, 10.0], [10.0, 0.0], [0.0, 0.0]];
        let o = offset_convex_quad(q, 2.0);
        assert!(close(o[0], [-2.0, 12.0]), "{:?}", o[0]);
        assert!(close(o[2], [12.0, -2.0]), "{:?}", o[2]);
    }

    #[test]
    fn parallelogram_edges_stay_parallel() {
        // A button quad: skew 3 on a 20x10 rect.
        let q = [[3.0, 0.0], [20.0, 0.0], [17.0, 10.0], [0.0, 10.0]];
        let o = offset_convex_quad(q, 1.5);
        // Top and bottom edges stay horizontal, moved out by d.
        assert!((o[0][1] - -1.5).abs() < 1e-3 && (o[1][1] - -1.5).abs() < 1e-3);
        assert!((o[2][1] - 11.5).abs() < 1e-3 && (o[3][1] - 11.5).abs() < 1e-3);
        // The slanted sides keep their direction.
        let s0 = (q[3][0] - q[0][0], q[3][1] - q[0][1]);
        let s1 = (o[3][0] - o[0][0], o[3][1] - o[0][1]);
        let cross = s0.0 * s1.1 - s0.1 * s1.0;
        assert!(cross.abs() < 1e-2, "slant changed: {cross}");
    }

    #[test]
    fn zero_offset_is_identity() {
        let q = [[3.0, 0.0], [20.0, 0.0], [17.0, 10.0], [0.0, 10.0]];
        assert_eq!(offset_convex_quad(q, 0.0), q);
    }
}
