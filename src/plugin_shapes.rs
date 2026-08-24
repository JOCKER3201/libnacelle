//! Client-side shape drawing for a plugin — the geometry nine `.so`
//! files each carried a copy of before this module existed, in the
//! nine-plugin split [`HostApi::ring_fill`] and its kin still date from
//! (K6). Every function here is compiled INTO the plugin: `libnacelle`
//! is a source dependency of an addon, not a shared object, so calling
//! one costs no boundary crossing and needs no `has_*` gate OF ITS OWN —
//! the gate each one owes is on the [`HostApi`] entry it forwards to.
//!
//! [`ring_fill`], [`ring`] and [`ring_glow`] are the whole of what a
//! plugin should call: pass the [`crate::runtime::CORNER_SQUARE`] family
//! code and a resolved length exactly as [`HostApi::ring_fill`] itself
//! takes them, and a host new enough draws through the ABI's own
//! tessellation — round properly arced, chamfer on its fast path, the
//! two ring lanes this crate has drawn every OTHER ring through since
//! ABI 6. A host that predates the entry being asked for gets the same
//! octagon every one of those nine files drew by hand: [`chamfer_fill`],
//! [`chamfer_frame`] and [`chamfer_glow`] are that degrade, kept here
//! ONCE rather than in every addon that might need it.
//!
//! `cut` throughout is a RESOLVED length, in device px, not a theme
//! sentinel: a caller that reads `@corner.pill` off the master already
//! ran [`crate::theme::corner_radius`] once, on its own box, before it
//! had a plain rect to hand this module — the same rule
//! [`HostApi::ring_fill`]'s own doc states for the ABI entry these
//! functions wrap.

use crate::runtime::{ColorC, HostApi, RectC, CORNER_SQUARE, MASK_QUAD_ADD};
use std::ffi::c_void;

/// The eight points of a rect's chamfered outline, flat, clockwise from
/// the top-left cut. `cut = 0` collapses each corner's pair to one
/// point, which is a square drawn as a very short octagon rather than a
/// special case — [`chamfer_fill`] and [`chamfer_frame`] both rely on
/// that collapse instead of testing for it.
pub fn octagon(r: RectC, cut: f32) -> [f32; 16] {
    let cut = cut.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    [
        x + cut, y,
        x + w - cut, y,
        x + w, y + cut,
        x + w, y + h - cut,
        x + w - cut, y + h,
        x + cut, y + h,
        x, y + h - cut,
        x, y + cut,
    ]
}

/// A filled rectangle with its corners cut off, as three quads: the
/// middle band and the two trapezoids the cut leaves — the shape every
/// non-square corner degraded to before [`HostApi::ring_fill`] existed,
/// and what a host still without it gets today, chamfer asked for
/// outright or round approximated by it.
pub fn chamfer_fill(api: &HostApi, ctx: *mut c_void, r: RectC, cut: f32, c: ColorC) {
    let cut = cut.min(r.w / 2.0).min(r.h / 2.0).max(0.0);
    let (x, y, w, h) = (r.x, r.y, r.w, r.h);
    (api.rect)(ctx, RectC { x, y: y + cut, w, h: h - 2.0 * cut }, c);
    let top: [f32; 8] = [x + cut, y, x + w - cut, y, x + w, y + cut, x, y + cut];
    (api.quad)(ctx, top.as_ptr(), c);
    let bottom: [f32; 8] =
        [x, y + h - cut, x + w, y + h - cut, x + w - cut, y + h, x + cut, y + h];
    (api.quad)(ctx, bottom.as_ptr(), c);
}

/// The ring of the same shape — a closed polyline through eight points.
pub fn chamfer_frame(api: &HostApi, ctx: *mut c_void, r: RectC, cut: f32, t: f32, c: ColorC) {
    let pts = octagon(r, cut);
    (api.polyline)(ctx, pts.as_ptr(), 8, t, c, true);
}

/// Glow OUTSIDE the ring — the outline extruded outward by `radius`, one
/// additive quad per segment, the soft disk's cardinal strip laid across
/// the extrusion. Nothing is emitted inside the path, so the glow never
/// tints the fill. Requires [`HostApi::has_mask_quad`]; a caller without
/// it draws nothing, which is the same silence every other glow in this
/// toolkit answers for a host that cannot sample the sprite.
pub fn chamfer_glow(api: &HostApi, ctx: *mut c_void, r: RectC, cut: f32, radius: f32, c: ColorC) {
    if !api.has_mask_quad() || !radius.is_finite() || radius <= 0.0 || c.a <= 0.0 {
        return;
    }
    let inner = octagon(r, cut);
    let grown = RectC {
        x: r.x - radius,
        y: r.y - radius,
        w: r.w + 2.0 * radius,
        h: r.h + 2.0 * radius,
    };
    let outer = octagon(grown, cut + radius);
    // The strip's profile in the SPRITE's own space: the mask-band
    // contract's 31..33 stretchable middle.
    const SU: f32 = 32.0 / 64.0;
    const VI: f32 = 31.0 / 64.0;
    let uv: [f32; 8] = [SU, VI, SU, VI, SU, 0.0, SU, 0.0];
    for i in 0..8 {
        let j = (i + 1) % 8;
        let pts: [f32; 8] = [
            inner[2 * i], inner[2 * i + 1],
            inner[2 * j], inner[2 * j + 1],
            outer[2 * j], outer[2 * j + 1],
            outer[2 * i], outer[2 * i + 1],
        ];
        (api.mask_quad)(ctx, pts.as_ptr(), uv.as_ptr(), c, MASK_QUAD_ADD);
    }
}

/// A filled rectangle wearing the family's corners — [`HostApi::ring_fill`]
/// on a host that carries the ring pair, [`chamfer_fill`] on one that
/// does not, a plain [`HostApi::rect`] for a square corner on either.
/// `style` is whatever [`crate::corner::code`] gives; a host too old to
/// draw it properly draws it plainer, never a different shape from a
/// different rule.
pub fn ring_fill(api: &HostApi, ctx: *mut c_void, r: RectC, style: u32, cut: f32, c: ColorC) {
    if c.a <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    if api.has_ring() {
        (api.ring_fill)(ctx, r, style, cut, c);
    } else if cut > 0.0 && style != CORNER_SQUARE {
        chamfer_fill(api, ctx, r, cut, c);
    } else {
        (api.rect)(ctx, r, c);
    }
}

/// The stroke of the same shape — [`HostApi::ring`], [`chamfer_frame`] or
/// [`HostApi::rect_outline`], by the same rule [`ring_fill`] applies to
/// the fill.
pub fn ring(
    api: &HostApi,
    ctx: *mut c_void,
    r: RectC,
    style: u32,
    cut: f32,
    t: f32,
    c: ColorC,
) {
    if t <= 0.0 || c.a <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    if api.has_ring() {
        (api.ring)(ctx, r, style, cut, t, c);
    } else if cut > 0.0 && style != CORNER_SQUARE {
        chamfer_frame(api, ctx, r, cut, t, c);
    } else {
        (api.rect_outline)(ctx, r, t, c);
    }
}

/// The glow of the same shape — [`HostApi::ring_glow`] on a host new
/// enough to own it, [`chamfer_glow`]'s hand-extruded octagon on one that
/// carries the ring pair but not its glow, and nothing on one that
/// carries neither. Three rungs, not two, because a host between ABI 6's
/// ring pair and this entry can already draw the RING itself correctly
/// (round arced, chamfer on its fast path) and only the halo around it
/// was ever the approximation.
pub fn ring_glow(
    api: &HostApi,
    ctx: *mut c_void,
    r: RectC,
    style: u32,
    cut: f32,
    radius: f32,
    c: ColorC,
) {
    if !radius.is_finite() || radius <= 0.0 || c.a <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    if api.has_ring_glow() {
        (api.ring_glow)(ctx, r, style, cut, radius, c);
    } else {
        chamfer_glow(api, ctx, r, cut, radius, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::{CORNER_CHAMFER, CORNER_ROUND};

    fn r() -> RectC {
        RectC { x: 10.0, y: 20.0, w: 100.0, h: 60.0 }
    }

    fn c() -> ColorC {
        ColorC { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }
    }

    /// `cut = 0` collapses every pair of the octagon's points to one —
    /// the shape a square corner draws when it is asked for through the
    /// same path as a chamfer, rather than through a separate one.
    #[test]
    fn a_zero_cut_collapses_the_octagon_to_a_rect() {
        let pts = octagon(r(), 0.0);
        assert_eq!([pts[0], pts[1]], [pts[14], pts[15]], "top-left pair");
        assert_eq!([pts[2], pts[3]], [pts[4], pts[5]], "top-right pair");
        assert_eq!([pts[6], pts[7]], [pts[8], pts[9]], "bottom-right pair");
        assert_eq!([pts[10], pts[11]], [pts[12], pts[13]], "bottom-left pair");
    }

    /// A cut past half the short side is clamped rather than folding the
    /// outline on itself — the same ceiling [`crate::corner`]'s own
    /// callers hold `ring_radius` to.
    #[test]
    fn the_cut_is_clamped_to_half_the_short_side() {
        let pts = octagon(r(), 1000.0);
        let short = r().h.min(r().w) / 2.0;
        // The top edge's two points sit exactly `short` apart from the
        // corners at a clamped cut, collapsing the top edge to nothing.
        assert!((pts[0] - r().x - short).abs() < 1e-4);
        assert!((pts[2] - (r().x + r().w) + short).abs() < 1e-4);
    }

    /// A host that answers `has_ring` never falls to the hand-rolled
    /// octagon — [`ring_fill`]'s whole reason to prefer the ABI entry.
    #[test]
    fn ring_fill_prefers_the_abi_entry_when_the_host_has_it() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static CALLS: AtomicU32 = AtomicU32::new(0);
        extern "C" fn counting_ring_fill(
            _ctx: *mut c_void,
            _r: RectC,
            _style: u32,
            _radius: f32,
            _c: ColorC,
        ) {
            CALLS.fetch_add(1, Ordering::SeqCst);
        }
        extern "C" fn unreachable_rect(_ctx: *mut c_void, _r: RectC, _c: ColorC) {
            panic!("ring_fill must not fall back once the host has the ring pair");
        }
        let api = HostApi { ring_fill: counting_ring_fill, rect: unreachable_rect, ..t_api() };
        ring_fill(&api, std::ptr::null_mut(), r(), CORNER_CHAMFER, 8.0, c());
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }

    /// A host too old for the ring pair still draws a chamfer for a
    /// ROUND request — visibly plainer, never a square where a rounded
    /// or chamfered corner was asked for.
    #[test]
    fn an_old_host_bevels_round_the_same_as_chamfer() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static QUADS: AtomicU32 = AtomicU32::new(0);
        extern "C" fn counting_quad(_ctx: *mut c_void, _pts: *const f32, _c: ColorC) {
            QUADS.fetch_add(1, Ordering::SeqCst);
        }
        extern "C" fn unreachable_ring_fill(
            _ctx: *mut c_void,
            _r: RectC,
            _style: u32,
            _radius: f32,
            _c: ColorC,
        ) {
            panic!("an old host's table must not be called through");
        }
        let api = HostApi {
            api_size: crate::runtime::HOST_API_HAS_CLIP as u32,
            ring_fill: unreachable_ring_fill,
            quad: counting_quad,
            ..t_api()
        };
        assert!(!api.has_ring());
        ring_fill(&api, std::ptr::null_mut(), r(), CORNER_ROUND, 8.0, c());
        // chamfer_fill draws one rect and two quads; only the quads are
        // instrumented here, and there are exactly two of them.
        assert_eq!(QUADS.load(Ordering::SeqCst), 2);
    }

    /// [`ring_glow`] drops to [`chamfer_glow`] when the host has the ring
    /// pair but not its glow — the middle rung, neither the ABI entry
    /// nor total silence.
    #[test]
    fn ring_glow_falls_to_the_octagon_extrusion_between_the_two_abi_rungs() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static MASK_QUADS: AtomicU32 = AtomicU32::new(0);
        extern "C" fn counting_mask_quad(
            _ctx: *mut c_void,
            _pts: *const f32,
            _uv: *const f32,
            _c: ColorC,
            _flags: u32,
        ) {
            MASK_QUADS.fetch_add(1, Ordering::SeqCst);
        }
        extern "C" fn unreachable_ring_glow(
            _ctx: *mut c_void,
            _r: RectC,
            _style: u32,
            _radius: f32,
            _glow_radius: f32,
            _c: ColorC,
        ) {
            panic!("a host without ring_glow must not be called through it");
        }
        let api = HostApi {
            api_size: crate::runtime::HOST_API_HAS_RING as u32,
            mask_quad: counting_mask_quad,
            ring_glow: unreachable_ring_glow,
            ..t_api()
        };
        assert!(api.has_ring());
        assert!(!api.has_ring_glow());
        ring_glow(&api, std::ptr::null_mut(), r(), CORNER_CHAMFER, 8.0, 4.0, c());
        // One additive quad per side of the octagon.
        assert_eq!(MASK_QUADS.load(Ordering::SeqCst), 8);
    }

    /// A host that answers `has_ring_glow` is called through it, and
    /// never through the octagon extrusion.
    #[test]
    fn ring_glow_prefers_the_abi_entry_when_the_host_has_it() {
        use std::sync::atomic::{AtomicU32, Ordering};
        static CALLS: AtomicU32 = AtomicU32::new(0);
        extern "C" fn counting_ring_glow(
            _ctx: *mut c_void,
            _r: RectC,
            _style: u32,
            _radius: f32,
            _glow_radius: f32,
            _c: ColorC,
        ) {
            CALLS.fetch_add(1, Ordering::SeqCst);
        }
        extern "C" fn unreachable_mask_quad(
            _ctx: *mut c_void,
            _pts: *const f32,
            _uv: *const f32,
            _c: ColorC,
            _flags: u32,
        ) {
            panic!("ring_glow must not fall back once the host has the entry");
        }
        let api =
            HostApi { ring_glow: counting_ring_glow, mask_quad: unreachable_mask_quad, ..t_api() };
        assert!(api.has_ring_glow());
        ring_glow(&api, std::ptr::null_mut(), r(), CORNER_ROUND, 8.0, 4.0, c());
        assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    }

    /// A complete, current-version table to derive test fixtures from —
    /// the real [`crate::plugin::host_api`], copied (it is `Copy`) so a
    /// test can override one field with `..t_api()` and stay otherwise
    /// indistinguishable from the host every shipped plugin actually
    /// runs against.
    fn t_api() -> HostApi {
        *crate::plugin::host_api()
    }
}
