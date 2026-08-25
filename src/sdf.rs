//! The CPU referee of the vector core (f3 §6, level E): the distance
//! formulas `fs_shape` computes in WGSL (§2.2), written once in Rust so
//! the mathematics is provable without a GPU. The shader is the
//! implementation, this file is the specification — the two must read
//! line for line alike, and a change to one without the other is wrong
//! by definition.
//!
//! `p` is the fragment's position in local pixels relative to the
//! shape's centre — exactly what a shape vertex carries in its `uv`
//! slot — and `b` the half sizes; screen convention throughout, y grows
//! downward.
//!
//! K6 ended the scope of "kind = Box alone". Bits 8-11 of the record
//! now select a field, and [`d_record`] is the whole of that selection:
//! one function, the mirror of the shader's own, so the contract at the
//! top of this file has exactly one place to be checked. What was added
//! is what the Box family could not spell — the truncated arc, the
//! hexagon, the chevron — and nothing that it could.
//!
//! K4 adds the ORIENTED frame ([`Frame`]) and not one field: a diagonal
//! stroke is the same box read along its own axes, and a joint disc is
//! the same box with round corners as big as itself. Both are here
//! because both had to be PROVED before they could be drawn, and the
//! proof is the same kind as K3's — rasterise, measure, state the
//! number.
//!
//! K3 also makes this file the place where the two lanes are COMPARED.
//! The tessellated generator ([`crate::draw::ring_points`]) and the
//! field here describe the same silhouettes by different means, and the
//! only honest way to arm `render.vector` is to measure the difference
//! rather than to look at it: the tests below rasterise both against the
//! polygon's own supersampled area and state, as thresholds, how far
//! each lane lands from it.

use crate::draw::{Corner, CornerStyle};

/// cos 45°, the chamfer plane's normalisation — WGSL's SQRT1_2.
pub const SQRT1_2: f32 = std::f32::consts::FRAC_1_SQRT_2;

/// Exact signed distance to the axis-aligned box of half sizes `b`.
pub fn d_box(p: [f32; 2], b: [f32; 2]) -> f32 {
    let q = [p[0].abs() - b[0], p[1].abs() - b[1]];
    let o = [q[0].max(0.0), q[1].max(0.0)];
    q[0].max(q[1]).min(0.0) + (o[0] * o[0] + o[1] * o[1]).sqrt()
}

/// The rounded corner of radius `k` — the exact rounded-box distance.
pub fn d_round(p: [f32; 2], b: [f32; 2], k: f32) -> f32 {
    let q = [p[0].abs() - b[0] + k, p[1].abs() - b[1] + k];
    let o = [q[0].max(0.0), q[1].max(0.0)];
    q[0].max(q[1]).min(0.0) + (o[0] * o[0] + o[1] * o[1]).sqrt() - k
}

/// The chamfered corner of cut `k`: the box intersected with the 45°
/// half-plane `|x| + |y| = b.x + b.y − k`. The `max` of two fields is
/// the exact distance only near the boundary — and only there does
/// coverage read it; inside, the underestimate saturates anyway
/// (§2.2's honesty note).
pub fn d_chamfer(p: [f32; 2], b: [f32; 2], k: f32) -> f32 {
    let cut = (p[0].abs() + p[1].abs() - (b[0] + b[1] - k)) * SQRT1_2;
    d_box(p, b).max(cut)
}

/// Which corner rules `p`: 0 tl, 1 tr, 2 br, 3 bl — `ring_points`'
/// order, y down. The quadrant boundary sits mid-edge, where |d| is at
/// least `min(b) − max(k)` and the treatment switch cannot reach the
/// coverage ramp (§2.2).
pub fn corner_index(p: [f32; 2]) -> usize {
    match (p[1] >= 0.0, p[0] >= 0.0) {
        (false, false) => 0,
        (false, true) => 1,
        (true, true) => 2,
        (true, false) => 3,
    }
}

/// The Box-family distance under four per-corner treatments — what one
/// `fs_shape` fragment computes for a Box record.
pub fn d_shape(p: [f32; 2], b: [f32; 2], corners: &[Corner; 4]) -> f32 {
    let c = corners[corner_index(p)];
    match c.style {
        CornerStyle::Square => d_box(p, b),
        CornerStyle::Round => d_round(p, b, c.size),
        CornerStyle::Chamfer => d_chamfer(p, b, c.size),
    }
}

// ---- The kinds past Box (K6) -----------------------------------------
//
// Three fields, three silhouettes the Box family cannot spell, and one
// rule they were admitted under: a kind is added only when the family
// CANNOT already draw it. The disc and the closed annulus were turned
// away by that rule and are Box records to this day (see [`d_disc`]);
// what got in was the truncated arc, the hexagon and the chevron.
//
// All three are read the same way as Box — signed, in local px, with
// `|∇d| = 1` near the boundary so [`coverage`] and [`band_coverage`]
// work on them unchanged.

/// The regular hexagon of apothem `r`, FLAT-TOPPED: horizontal edges at
/// `y = ±r`, vertices at `x = ±2r/√3`. A turn is the CALLER's, applied
/// to `p` before this is read ([`turned`]), because the field is
/// cheaper to rotate than the shape.
///
/// The fold is the standard one for a six-fold lattice: reflect into one
/// sixth by the mirror whose normal is `(−cos 30°, sin 30°)`, then read
/// the distance to that sixth's single edge, the segment `x ∈ [−r/√3,
/// r/√3]` at `y = r`. Exact everywhere, inside and out, which is what
/// lets the band and the AA ramp read it without a second thought.
pub fn d_hex(p0: [f32; 2], r: f32) -> f32 {
    // (−cos 30°, sin 30°, 1/√3): the mirror's normal and the half length
    // of one edge in units of the apothem.
    const HK: [f32; 3] = [-0.866_025_4, 0.5, 0.577_350_3];
    let mut p = [p0[0].abs(), p0[1].abs()];
    let fold = 2.0 * (HK[0] * p[0] + HK[1] * p[1]).min(0.0);
    p = [p[0] - fold * HK[0], p[1] - fold * HK[1]];
    p = [p[0] - p[0].clamp(-HK[2] * r, HK[2] * r), p[1] - r];
    let len = (p[0] * p[0] + p[1] * p[1]).sqrt();
    // The sign by comparison and not by `signum`: WGSL's `sign(0.0)` is
    // zero where Rust's `signum(0.0)` is one, and the two files may not
    // disagree on the boundary itself.
    if p[1] >= 0.0 {
        len
    } else {
        -len
    }
}

/// `p` rotated by `a` — the one rotation in this file, so a field
/// written about a single axis can be read about any.
///
/// A silhouette turned by `t` is the base field read at `turned(p, −t)`:
/// rotating the QUESTION the other way is the same as rotating the
/// answer, and it costs two multiplies instead of a shape.
///
/// The sign is the SCREEN's. y grows downward here, so the matrix that
/// a mathematician reads as anticlockwise turns clockwise on the glass —
/// and clockwise is what an angle means to everything else in this
/// project, from `donut.start_deg` to a knob's travel.
pub fn turned(p: [f32; 2], a: f32) -> [f32; 2] {
    let (s, c) = a.sin_cos();
    [p[0] * c - p[1] * s, p[0] * s + p[1] * c]
}

/// The annular ARC: a band of half width `rb` about the circle of
/// radius `ra`, swept `2·half_sweep` about local +y, with round caps.
///
/// This is the one member of the family the Box records cannot spell.
/// A closed ring — `half_sweep ≥ π` — they can: `d_round(p, [R, R], R)`
/// with the inward stroke is the same annulus, and the branch below
/// answers the same number for it (the test says so). The branch earns
/// its instructions only where the sweep is cut short, because a cut
/// arc has CAPS, and a cap is a distance to a point rather than to a
/// circle.
///
/// Outside the swept wedge the nearest boundary is the nearer cap's
/// centre-line end; inside it, the circle. The two agree exactly on the
/// ray that divides them — both read `|distance to the cap centre| − rb`
/// there — so the seam carries no gradient step and the AA ramp crosses
/// it without a mark.
pub fn d_arc(p0: [f32; 2], half_sweep: f32, ra: f32, rb: f32) -> f32 {
    let (sn, cs) = half_sweep.sin_cos();
    // A sweep of half a turn or more is CLOSED, and the wedge test has
    // to say so without an `if`. `sin` is what carries that: it falls to
    // zero at π and turns negative past it, so clamping it at zero
    // leaves `cos·|x| > 0`, which a non-positive left side can never
    // satisfy — every fragment takes the circle. Without the clamp a
    // full ring drawn as `half_sweep = PI` fell to the CAP branch on
    // half the rays, because `sin(PI)` in f32 is a hair BELOW zero and
    // that hair decided the comparison.
    let sn = sn.max(0.0);
    let p = [p0[0].abs(), p0[1]];
    if cs * p[0] > sn * p[1] {
        let (dx, dy) = (p[0] - sn * ra, p[1] - cs * ra);
        (dx * dx + dy * dy).sqrt() - rb
    } else {
        ((p[0] * p[0] + p[1] * p[1]).sqrt() - ra).abs() - rb
    }
}

/// The chevron: the box of half sizes `b` with its left and/or right
/// end collapsed to a point at mid-height, `left` and `right` px deep.
///
/// Each collapsed end is a pair of mirror-image half-planes through the
/// tip `(∓b.x, 0)` and the end of the cut `(∓b.x ± depth, ±b.y)`, which
/// `|p.y|` folds into one. A depth of zero gives back the end's own
/// vertical edge exactly — `(0·|p.y| − b.y·along)/b.y = −along` — so an
/// end that is not collapsed costs the field nothing and changes it not
/// at all, and `chevron_dir = left` needs no second formula.
///
/// `max` of half-planes is the exact distance near every edge and an
/// underestimate in the acute wedge behind a tip, where the true
/// distance is to the point. Coverage reads the boundary, the boundary
/// is where the estimate is exact, and §2.2 accepted the same trade for
/// the chamfer.
pub fn d_chevron(p: [f32; 2], b: [f32; 2], left: f32, right: f32) -> f32 {
    // `along` grows inward from the end's own edge; the normalisation is
    // the length of the slanted edge, so the quotient is a true distance
    // and not merely a sign.
    let end = |depth: f32, along: f32| {
        let l = (b[1] * b[1] + depth * depth).sqrt().max(1e-6);
        (depth * p[1].abs() - b[1] * along) / l
    };
    d_box(p, b)
        .max(end(left, p[0] + b[0]))
        .max(end(right, b[0] - p[0]))
}

/// The distance ONE record computes — the whole of `fs_shape`'s branch
/// on bits 8-11, in the reference's own terms.
///
/// This is the function the shader mirrors line for line. It reads the
/// record and nothing else, exactly as the fragment does: the kind out
/// of the flag word, the lengths out of `corner`, the angles out of
/// `arc_half` / `arc_dir`, all per the table on
/// [`crate::draw::ShapeKind`].
pub fn d_record(s: &crate::draw::Shape, p: [f32; 2]) -> f32 {
    use crate::draw::ShapeKind;
    let b = s.half;
    match ShapeKind::of_code((s.flags >> crate::draw::Shape::KIND_SHIFT) & 0xF) {
        ShapeKind::Ring { .. } => {
            let rb = s.corner[0];
            // The band's OUTER edge meets the shorter side of the rect,
            // so the axis radius sits one half thickness inside it.
            let ra = (b[0].min(b[1]) - rb).max(0.0);
            d_arc(turned(p, -s.arc_dir), s.arc_half, ra, rb)
        }
        ShapeKind::Hex { .. } => d_hex(turned(p, -s.arc_dir), s.corner[0]),
        ShapeKind::Chevron { .. } => d_chevron(p, b, s.corner[0], s.corner[1]),
        // Box, and Capsule until something emits one: the four corner
        // treatments over the box.
        _ => d_shape(p, b, &record_corners(s)),
    }
}

/// The four corner treatments back out of a record's flag word — what
/// the fragment shader reads, read the same way.
pub fn record_corners(s: &crate::draw::Shape) -> [Corner; 4] {
    [0usize, 1, 2, 3].map(|i| Corner {
        style: match (s.flags >> (2 * i as u32)) & 3 {
            1 => CornerStyle::Round,
            2 => CornerStyle::Chamfer,
            _ => CornerStyle::Square,
        },
        size: s.corner[i],
    })
}

/// Box-filter coverage of the half-plane at signed distance `d` under
/// AA width `w` (§2.3): exact for a straight edge, first-order correct
/// for curvature well above the pixel. In the shader `w` is
/// `length(vec2(dpdx(d), dpdy(d)))` — never `fwidth`, which over-reads
/// √2 on a 45° slope; the reference takes it as a parameter.
pub fn coverage(d: f32, w: f32) -> f32 {
    (0.5 - d / w.max(1e-6)).clamp(0.0, 1.0)
}

/// Coverage of the INWARD stroke band as an AREA: the interior minus the
/// interior inset by `stroke` — one coverage ramp less the other, never
/// their product.
///
/// K2 read the band off a folded field, `clamp(0.5 − max(d, −d−stroke)/w)`,
/// and multiplied it by the silhouette's own coverage. That is the
/// intersection of two half-planes weighted by a third, and it is wrong
/// in two ways that show on screen. On the silhouette both factors read
/// a half, so a stroked edge landed at **0.25 where a hard edge covers
/// 0.5** — a border that antialiasing made half as present as the one it
/// replaced. And a band THINNER than the AA width kept reading a half at
/// its centre however thin it grew, so a 0.2 px hairline painted itself
/// 0.5 px wide.
///
/// The difference of the two ramps is the exact swept area between the
/// boundaries for a straight edge, and it is what §2.8 asks of a
/// hairline **without a floor token**: its cross-section integrates to
/// `stroke` at every width, so a sub-pixel stroke keeps its mass by
/// dimming instead of by fattening or by vanishing. A 0.3 px border
/// draws as 1 px at alpha 0.3 because that is its area, not because a
/// rule was written to make it so — which is why `render.hairline_floor`
/// and its push constant were never added: the arithmetic that made
/// them necessary is the arithmetic this replaces.
///
/// `stroke` is the band's width in the field's own units — inward is
/// the project's convention, [`crate::draw::DrawList::ring`]'s own.
///
/// EXACT ON A STRAIGHT EDGE UNDER EITHER CORNER RULE. Since 2026-08-25
/// a Box's band reads its inner boundary off a SECOND silhouette
/// ([`band_inner_d`]) rather than this same-field offset, but the two
/// agree everywhere a corner is not: the inner box's field along a
/// straight edge IS `d + stroke`. Every 1-D claim below — the mass
/// rule, the hairline, the shared edge — is a straight-edge claim, and
/// this stays their reference; what the second silhouette changes is
/// only WHERE the inner boundary bends, which is [`band_inner_d`]'s to
/// say.
pub fn band_coverage(d: f32, stroke: f32, w: f32) -> f32 {
    // Non-increasing in its argument, so the difference is already
    // non-negative and never exceeds the silhouette's coverage; the
    // clamp is against the last bit of the subtraction, not against the
    // mathematics.
    (coverage(d, w) - coverage(d + stroke, w)).max(0.0)
}

/// The signed distance to a record's band's own INNER contour — the
/// mirror of the shader's `band_inner_d`, argument for argument
/// (2026-08-25, the equal-rounding rule; the owner's word: "wewnątrz
/// róg ma być tak samo zaokrąglony jak na zewnątrz").
///
/// A Box's band used to end on the same field's `d + stroke` isoline —
/// the concentric answer, an inner corner of `R − stroke` that turns
/// SQUARE the moment the band outgrows the radius. Now it ends on a
/// SECOND silhouette: the rect inset by the stroke, wearing the same
/// corner radii, each clamped to the inner rect's own cap exactly as
/// [`crate::draw::ring_points`] clamps them on the tessellated lane —
/// so an inner corner is as round as the outer one at every width, and
/// the two lanes bend in the same place. Every other kind keeps the
/// offset: its payload describes the outer curve, and shrinking the
/// half-size alone would not shrink it.
pub fn band_inner_d(s: &crate::draw::Shape, p: [f32; 2], d: f32) -> f32 {
    use crate::draw::ShapeKind;
    let code = (s.flags >> crate::draw::Shape::KIND_SHIFT) & 0xF;
    if !matches!(ShapeKind::of_code(code), ShapeKind::Box | ShapeKind::Capsule) {
        return d + s.stroke;
    }
    let half_in = [(s.half[0] - s.stroke).max(0.0), (s.half[1] - s.stroke).max(0.0)];
    let cap_in = half_in[0].min(half_in[1]);
    let mut corners = record_corners(s);
    for c in &mut corners {
        c.size = c.size.min(cap_in);
    }
    d_shape(p, half_in, &corners)
}

/// How many standard deviations of the gaussian fit inside the reach
/// (§2.6) — the profile's whole shape, in one number.
///
/// **Why this is not a theme token, in a project whose hardest rule is
/// that appearance lives in the theme.** It is not a value chosen for a
/// look; it is the DEFINITION of the profile the atlas already bakes.
/// `FontSystem::bake_masks` writes `exp(−d²/2σ²)` with `σ = r/3` and a
/// hard zero at `r` into the soft-disk sprite (`font.rs:471-484`), and
/// every glow and shadow drawn off the tessellated lane samples it.
/// Two lanes draw the same glow, so the two profiles have to be the
/// same function; a token here would let a theme make them differ and
/// there is no picture in which that is what anybody wanted. What the
/// theme DOES own is the reach — `glow.<class>.radius`, `shadow.radius`
/// — which is the only number of the profile a design has an opinion
/// about.
pub const GAUSS_SIGMAS: f32 = 3.0;

/// §2.6's softness profile: `FontSystem::bake_masks`' own gaussian, so
/// a glow moved from the sprite to the field keeps its CHARACTER and
/// loses only the nine-slice's stretched middle.
///
/// `d` is the signed distance, `feather` the reach. Inside the
/// silhouette (`d ≤ 0`) it is a flat 1 — the plateau a shadow lays
/// under a panel — and it falls to a hard zero at `feather`, where the
/// sprite's own texel is zero too.
///
/// **A soft shape has ONE coverage, and this is it.** §2.6 says so in
/// as many words, and the trap it warns about is multiplying the
/// profile by the crisp ramp: that would dim the boundary twice and
/// leave a dark seam a pixel wide all round. What [`outside_mask`] does
/// is a different thing, and the note there says why.
/// A reach of zero is the degenerate case, and the two sides answer it
/// identically ON PURPOSE: the profile collapses to the HARD
/// silhouette — 1 within, 0 without — rather than to nothing. It is
/// unreachable through the toolkit (`shape_verts` drops a `Soft` whose
/// reach is not positive, so `GAUSS` is never set beside a zero
/// feather), which is exactly why the two files could have drifted here
/// unnoticed. `the_soft_profile_answers_what_the_reference_answers`
/// sweeps it for that reason.
pub fn soft_profile(d: f32, feather: f32) -> f32 {
    if d >= feather {
        return 0.0;
    }
    let x = d.max(0.0);
    let sg = feather.max(1e-6) / GAUSS_SIGMAS;
    (-(x * x) / (2.0 * sg * sg)).exp()
}

/// [`crate::draw::Shape::OUTSIDE_ONLY`]'s factor: the area of the pixel
/// the silhouette does NOT cover — `coverage`'s own complement.
///
/// A glow lights what is around a shape, and today's tessellated glow
/// says so by emitting no geometry inside its path: the mask is exact,
/// and exactly aliased, because a polygon edge is where it falls. Here
/// it is an area, so the boundary pixel gets the fraction it is owed —
/// and the panel standing on that same boundary takes the rest through
/// its own coverage, which is what makes the two add up to one instead
/// of leaving a seam.
///
/// This is not the double attenuation §2.6 forbids. That warning is
/// about weighting a soft profile by the softness of the same edge; the
/// factor here is geometry — it is exactly 1 as soon as the fragment is
/// half a pixel clear of the boundary, so nothing in the body of the
/// glow is touched by it at all.
pub fn outside_mask(d: f32, w: f32) -> f32 {
    (0.5 + d / w.max(1e-6)).clamp(0.0, 1.0)
}

/// The coverage ONE record puts on a fragment at signed distance `d`
/// under AA width `w`: the crisp ramp, or the soft profile when
/// [`crate::draw::Shape::GAUSS`] says so, masked to the outside when
/// [`crate::draw::Shape::OUTSIDE_ONLY`] does.
///
/// The whole of the fragment's branch on the soft bits, in one place on
/// each side of the seam — the shader's twin is `shape_alpha`, which
/// takes the same four numbers for the same reason `shape_field` takes
/// the record's: a function with no derivatives in it can be RUN
/// against this one without a GPU.
pub fn shape_alpha(d: f32, w: f32, feather: f32, flags: u32) -> f32 {
    use crate::draw::Shape;
    let cov = if flags & Shape::GAUSS != 0 {
        soft_profile(d, feather)
    } else {
        coverage(d, w)
    };
    if flags & Shape::OUTSIDE_ONLY != 0 {
        cov * outside_mask(d, w)
    } else {
        cov
    }
}

/// §2.10's one composition: bed and edge live in ONE record, so their
/// shared outer silhouette blends ONCE. Straight-alpha RGBA out, the
/// form the fragment shader returns.
///
/// The model is areas, not mixes. Of the pixel, `a_band` is the part the
/// stroke covers, `cov − a_band` the part only the fill covers, and
/// `1 − cov` is empty. The stroke lies OVER the fill — `ring_fill` draws
/// on the original rect and the border stands on top of it — so the band's
/// own colour is the stroke composited over the fill, and the two parts
/// are then averaged by area:
///
/// ```text
/// alpha   = cov·fill_a + s_a·(1 − fill_a)          , s_a = a_band·stroke_a
/// rgb·α   = s_a·stroke_rgb + fill_a·(cov − s_a)·fill_rgb
/// ```
///
/// Two properties are worth naming because they are what makes this
/// change safe. **Inside**, past the band, it returns the fill exactly
/// as the split pair did — same alpha, same colour, no arithmetic. **In**
/// the band it returns `stroke over fill`, again exactly what two draws
/// produced. Only on the shared edge do the two differ, and there the
/// pair was wrong: `1 − (1 − a)²` instead of `a`, the dark rim on a
/// translucent panel over glass.
///
/// A caller with no fill passes `fill_a = 0`; with no band, `a_band = 0`.
pub fn compose(fill: [f32; 4], stroke_c: [f32; 4], cov: f32, a_band: f32) -> [f32; 4] {
    let s_a = a_band * stroke_c[3];
    let f_a = fill[3];
    let alpha = cov * f_a + s_a * (1.0 - f_a);
    if s_a <= 0.0 {
        // No band under this fragment: the fill's own colour, carried
        // through untouched. Dividing the premultiplied sum by `alpha`
        // would return the same colour to within an ulp, and an ulp is
        // not worth spending where the answer is already exact.
        return [fill[0], fill[1], fill[2], alpha];
    }
    let k = f_a * (cov - s_a);
    let inv = 1.0 / alpha.max(1e-5);
    [
        (s_a * stroke_c[0] + k * fill[0]) * inv,
        (s_a * stroke_c[1] + k * fill[1]) * inv,
        (s_a * stroke_c[2] + k * fill[2]) * inv,
        alpha,
    ]
}

/// `top` over `bottom`, straight alpha in and out — the one form of
/// Porter-Duff OVER this project computes, wherever it computes it.
///
/// The hardware blends straight alpha: `d' = c.rgb·c.a + d·(1 − c.a)`.
/// Laying `bottom` and then `top` on the same unknown destination
/// leaves `a = t_a + b_a·(1 − t_a)` and `rgb·a = top·t_a + (1 −
/// t_a)·bottom·b_a`; matching both sides against a single fill gives
/// exactly the lines below. It is an identity for EVERY destination,
/// not a match on one — which is what lets two draws become one
/// fragment at all.
///
/// Two readers, and they are the same arithmetic for the same reason:
/// [`crate::draw::DrawList`]'s weld folds a wash into the bed it stands
/// on (§2.10), and `fs_shape_glass` folds that wash over the tinted
/// blur beneath it (§3.3). One is done once on the CPU because the
/// colours are known there; the other every fragment because one of the
/// two colours is a texture sample.
///
/// The composite is done on the numbers the vertices carry — the
/// swapchain's own encoding, not linear light — because those are the
/// numbers the blender works on.
pub fn over(top: [f32; 4], bottom: [f32; 4]) -> [f32; 4] {
    let (ta, ba) = (top[3], bottom[3]);
    let a = ta + ba * (1.0 - ta);
    if a <= 0.0 {
        // Nothing was laid at all; the colour is unobservable, and the
        // top's is as good a nothing as any.
        return top;
    }
    let ch = |t: f32, b: f32| (t * ta + b * ba * (1.0 - ta)) / a;
    [ch(top[0], bottom[0]), ch(top[1], bottom[1]), ch(top[2], bottom[2]), a]
}

/// What one `fs_shape_glass` fragment stands on (f3 §3.3): the surface
/// UNDER the band, as one straight-alpha colour, so the band's single
/// coverage can be applied to it once.
///
/// A frosted surface is three layers deep, and until K3b the middle two
/// were three draws: the blurred scene multiplied by the tint (`fs_blur`
/// over the pyramid), the wash laid over that, and the border over
/// both. Each blended with its own coverage, and on the shared
/// silhouette that is the R4 doubling by another name — at half
/// coverage the pair leaves `c·b + c·a·(1 − c·b)` where the surface
/// covers `c·(b + a·(1 − b))`, an excess of `c·(1 − c)·a·b` that reads
/// as a heavier rim exactly where the eye looks. Folding them here and
/// letting [`compose`] apply `cov` once removes it by construction.
///
/// `blur` is the pyramid sample at this fragment (straight alpha, the
/// scene's own), `tint` the record's — it MULTIPLIES, which is why it
/// can only darken — and `wash` the vertex's, which lies over with
/// alpha and is the only one of the three that can brighten. The
/// master's ladder says exactly this at `elev.*.glass`; this function
/// is where those words become arithmetic.
///
/// `display` IS THE SEAM, and it is a parameter because getting it
/// wrong is invisible until somebody loads a colour LUT. The renderer
/// ends every fragment with `grade()`, which is the identity until one
/// is loaded and an arbitrary curve after that. A frosted surface is
/// drawn in two pieces — a core of two ordinary quads the hardware
/// blends, and this band — so the two pieces agree only if what the
/// band folds is what the hardware would have blended. OVER is
/// associative, so a transform applied to EACH LAYER survives the fold:
/// `over(f(w), f(k))` composited on any destination is exactly `f(w)`
/// over `f(k)` over that destination. A transform applied to the FOLD
/// does not survive it — `f(over(w, k))` is a different colour, and the
/// difference draws as a rectangle inside the panel, on the line where
/// the core's cut happens to fall. Pass the identity where there is no
/// display transform to apply.
pub fn glass_base(
    blur: [f32; 4],
    tint: [f32; 4],
    wash: [f32; 4],
    display: impl Fn([f32; 4]) -> [f32; 4],
) -> [f32; 4] {
    let frost = [
        blur[0] * tint[0],
        blur[1] * tint[1],
        blur[2] * tint[2],
        blur[3] * tint[3],
    ];
    over(display(wash), display(frost))
}

// ---- The oriented lane (f3 §3.1, §K4) --------------------------------

/// The local frame a silhouette is read in when its axes are not the
/// screen's — a chart stroke, a tick, a chevron, the arms of a cross.
///
/// **The whole of the oriented lane is one observation.** `fs_shape`
/// takes the fragment's local position out of `uv`, and `uv` is
/// interpolated linearly across the quad. Put the four vertices at
/// `centre + lx·ux + ly·uy` and give each the `[lx, ly]` it was built
/// from, and every fragment reads exactly its own local coordinate —
/// for ANY invertible pair of axes. The rasteriser inverts the map, per
/// vertex, for free; the shader needs no rotation, no matrix, and not
/// one new instruction. K4 adds nothing to the GPU at all.
///
/// **The antialiasing follows from the same fact.** Coverage is
/// `0.5 − d/w` with `w = |∇d|` in SCREEN space (§2.3), and near a
/// straight edge `d` is linear, so `d/|∇d|` is the true signed distance
/// in device pixels whatever the frame did to the field's units. A
/// rotation leaves `|∇d| = 1`; a shear would make it something else and
/// the quotient would absorb it. This is the property §2.3 bought when
/// it insisted the width come from the field's own derivatives rather
/// than from a constant — and the oriented lane is where the purchase
/// pays.
///
/// Everything this file's callers build is ORTHONORMAL: `ux` and `uy`
/// are perpendicular unit vectors, so one local unit is one screen
/// pixel and the padding a quad needs is stated in pixels either way.
/// The arithmetic below never assumes it; the emitter does, and says so.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frame {
    /// The silhouette's centre, screen px.
    pub centre: [f32; 2],
    /// Local +x, in screen px per local unit.
    pub ux: [f32; 2],
    /// Local +y, likewise.
    pub uy: [f32; 2],
}

impl Frame {
    /// The screen's own axes at `centre` — a disc, a dot, and every
    /// shape K3 already draws.
    pub const fn upright(centre: [f32; 2]) -> Frame {
        Frame { centre, ux: [1.0, 0.0], uy: [0.0, 1.0] }
    }

    /// The frame of the straight segment `a → b`: local x runs ALONG
    /// the path and local y across it, so the silhouette is the box
    /// `[±len/2, ±t/2]` — the very quad [`crate::draw::DrawList::line`]
    /// has always drawn, now with a field on it. `None` where the two
    /// ends coincide and there is no direction to speak of.
    ///
    /// The normal is `(-dy, dx)`: y grows downward here, so this is the
    /// same handedness the rest of the toolkit draws in, and the same
    /// one `line_verts` picked.
    pub fn along(a: [f32; 2], b: [f32; 2]) -> Option<(Frame, f32)> {
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if !(len > 0.0) {
            return None;
        }
        let (ux, uy) = (dx / len, dy / len);
        Some((
            Frame {
                centre: [(a[0] + b[0]) * 0.5, (a[1] + b[1]) * 0.5],
                ux: [ux, uy],
                uy: [-uy, ux],
            },
            len,
        ))
    }

    /// The screen point of a local one — what the VERTEX stage is spared
    /// because the CPU does it once per corner.
    pub fn to_screen(&self, l: [f32; 2]) -> [f32; 2] {
        [
            self.centre[0] + l[0] * self.ux[0] + l[1] * self.uy[0],
            self.centre[1] + l[0] * self.ux[1] + l[1] * self.uy[1],
        ]
    }

    /// The local point of a screen one — the map the RASTERISER performs
    /// on the GPU by interpolating `uv`, written out here because the
    /// reference has no rasteriser. Degenerate axes give back the
    /// centre's own local coordinate rather than an infinity; nothing
    /// emits them.
    pub fn to_local(&self, p: [f32; 2]) -> [f32; 2] {
        let det = self.ux[0] * self.uy[1] - self.ux[1] * self.uy[0];
        if det.abs() <= 1e-12 {
            return [0.0, 0.0];
        }
        let (qx, qy) = (p[0] - self.centre[0], p[1] - self.centre[1]);
        [
            (qx * self.uy[1] - qy * self.uy[0]) / det,
            (qy * self.ux[0] - qx * self.ux[1]) / det,
        ]
    }
}

/// The AA width `fs_shape` computes, in the reference's own terms:
/// `length(vec2(dpdx(d), dpdy(d)))` where the field is read as a
/// function of the SCREEN point.
///
/// The hardware takes finite differences across the 2×2 quad rather
/// than derivatives. For the affine fields of this lane the two agree
/// exactly — `d` is linear in a neighbourhood of every edge — so a
/// central difference over one pixel is the honest stand-in, and it is
/// the one place the reference has to model the GPU rather than the
/// mathematics.
pub fn screen_width(d: impl Fn([f32; 2]) -> f32, p: [f32; 2]) -> f32 {
    let gx = d([p[0] + 0.5, p[1]]) - d([p[0] - 0.5, p[1]]);
    let gy = d([p[0], p[1] + 0.5]) - d([p[0], p[1] - 0.5]);
    (gx * gx + gy * gy).sqrt().max(1e-6)
}

/// The disc of radius `r` — and the point is that this function is NOT
/// a new field. `d_round(p, [r, r], r)` is `|p| − r` identically (the
/// test below proves it term by term), so a joint disc, a dot in a
/// matrix (§3.4) and `glow.node_dot` are all Box records with round
/// corners as big as their own half size. Nothing new reaches the
/// shader; `ShapeKind::Ring` stays reserved for the ARC, which is the
/// one thing the Box family cannot spell.
pub fn d_disc(p: [f32; 2], r: f32) -> f32 {
    (p[0] * p[0] + p[1] * p[1]).sqrt() - r
}

/// The width a band of the oriented lane is drawn at, and the factor
/// its colour is dimmed by — §2.8's energy rule, in the one domain
/// where it applies.
///
/// K3's snap already lifts every sub-pixel band on the AXIS-ALIGNED
/// lane: `round().max(1.0)` runs before the record is written, so no
/// hairline reaches the field there. A diagonal has no grid to round
/// to, and the single coverage ramp `fs_shape` computes for a fill is a
/// HALF-PLANE's — exact for an edge, and half again too generous for a
/// slab thinner than the filter. A 0.5 px stroke read that way paints
/// 0.75 of the pixel it runs through: fifty per cent heavier than the
/// line asked for, and heavier still as it thins.
///
/// So the rule of §2.8, stated once: a band under a pixel is drawn ONE
/// pixel wide and dimmed by what it lost. Its integral across the
/// section is `t` either way — it dims instead of fattening — and above
/// a pixel nothing happens at all.
pub fn thin_band(t: f32) -> (f32, f32) {
    if t < 1.0 {
        (1.0, t.max(0.0))
    } else {
        (t, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const B: [f32; 2] = [40.0, 25.0];

    fn mixed() -> [Corner; 4] {
        [
            Corner::round(8.0),
            Corner::chamfer(10.0),
            Corner::SQUARE,
            Corner::round(3.0),
        ]
    }

    /// Deep inside the coverage saturates to one, deep outside to zero
    /// — across all three corner treatments at once.
    #[test]
    fn coverage_saturates_inside_and_vanishes_outside() {
        let c = mixed();
        assert_eq!(coverage(d_shape([0.0, 0.0], B, &c), 1.0), 1.0);
        assert_eq!(
            coverage(d_shape([B[0] - 2.0, 0.0], B, &c), 1.0),
            1.0,
            "two px inside is still fully covered"
        );
        for p in [[80.0, 0.0], [0.0, -60.0], [70.0, 55.0], [-90.0, -70.0]] {
            assert_eq!(coverage(d_shape(p, B, &c), 1.0), 0.0, "{p:?}");
        }
    }

    /// 64 directions from the centre: where the sign of d flips, the
    /// coverage reads one half within the stated tolerance — including
    /// the rays that cross a quadrant seam or a treatment switch.
    #[test]
    fn the_boundary_reads_half_in_64_directions() {
        let c = mixed();
        for i in 0..64 {
            let a = i as f32 / 64.0 * std::f32::consts::TAU;
            let (s, co) = a.sin_cos();
            // Bisect d = 0 along the ray: the centre is inside, 200 px
            // out is outside for this box, and the shape is convex, so
            // the sign flips exactly once.
            let (mut lo, mut hi) = (0.0f32, 200.0f32);
            for _ in 0..48 {
                let mid = 0.5 * (lo + hi);
                if d_shape([co * mid, s * mid], B, &c) < 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let t = 0.5 * (lo + hi);
            let cov = coverage(d_shape([co * t, s * t], B, &c), 1.0);
            assert!((cov - 0.5).abs() <= 0.02, "direction {i}: coverage {cov}");
        }
    }

    /// The round corner runs on its own arc: d = 0 at both tangent
    /// points and at the arc's 45° point, and the square corner it
    /// replaced lies outside by exactly k(√2 − 1).
    #[test]
    fn a_round_corner_runs_on_its_arc() {
        let k = 8.0f32;
        let e = 1e-3;
        assert!(d_round([B[0], B[1] - k], B, k).abs() <= e);
        assert!(d_round([B[0] - k, B[1]], B, k).abs() <= e);
        let c45 = [B[0] - k + k * SQRT1_2, B[1] - k + k * SQRT1_2];
        assert!(d_round(c45, B, k).abs() <= e);
        let d = d_round(B, B, k);
        assert!((d - k * (std::f32::consts::SQRT_2 - 1.0)).abs() <= e, "{d}");
    }

    /// A field is only a field if its gradient has unit norm ON THE
    /// COVERAGE BAND — that is what makes `0.5 − d/w` a coverage and not
    /// a guess, and it is the only place the shader ever reads it.
    ///
    /// Sampled over a grid, keeping `|d| ≤ 1.5` — a pixel and a half
    /// either side of the silhouette, wider than any AA ramp — and
    /// dropping a 2 px disc around each `corner` the caller names.
    ///
    /// EVERY polygon corner starts a medial axis, and inside a corner
    /// the nearest boundary point is not one point but two: there is no
    /// gradient there to have a norm, in the geometry and not in the
    /// arithmetic. It does not show, and cannot: on that ridge `|d|`
    /// already exceeds what the ramp resolves, so coverage is saturated
    /// at 1 whichever of the two edges the field answered — which is the
    /// same argument §2.2 made when it accepted `max` for the chamfer.
    fn gradient_is_unit(d: impl Fn([f32; 2]) -> f32, span: [f32; 2], corners: &[[f32; 2]]) {
        let h = 0.05f32;
        let (mut worst, mut seen) = (0.0f32, 0usize);
        for iy in -60..=60 {
            for ix in -60..=60 {
                let p = [ix as f32 / 60.0 * span[0], iy as f32 / 60.0 * span[1]];
                if d(p).abs() > 1.5 {
                    continue;
                }
                if corners.iter().any(|c| {
                    let (dx, dy) = (p[0] - c[0], p[1] - c[1]);
                    dx * dx + dy * dy < 4.0
                }) {
                    continue;
                }
                seen += 1;
                let gx = (d([p[0] + h, p[1]]) - d([p[0] - h, p[1]])) / (2.0 * h);
                let gy = (d([p[0], p[1] + h]) - d([p[0], p[1] - h])) / (2.0 * h);
                worst = worst.max(((gx * gx + gy * gy).sqrt() - 1.0).abs());
            }
        }
        // Fail closed: a band nobody sampled proves nothing.
        assert!(seen >= 100, "only {seen} samples landed on the band");
        assert!(worst <= 0.02, "gradient norm strayed by {worst}");
    }

    /// The bisection the box family is proved by, run on any field: 64
    /// rays out of the centre, and where the sign flips the coverage
    /// must read a half. `reach` is a radius known to be outside.
    fn boundary_reads_half(d: impl Fn([f32; 2]) -> f32, reach: f32, what: &str) {
        for i in 0..64 {
            let a = i as f32 / 64.0 * std::f32::consts::TAU;
            let (s, co) = a.sin_cos();
            let (mut lo, mut hi) = (0.0f32, reach);
            for _ in 0..48 {
                let mid = 0.5 * (lo + hi);
                if d([co * mid, s * mid]) < 0.0 {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let t = 0.5 * (lo + hi);
            let cov = coverage(d([co * t, s * t]), 1.0);
            assert!((cov - 0.5).abs() <= 0.02, "{what}, direction {i}: coverage {cov}");
        }
    }

    /// The hexagon stands where its own geometry says: a flat edge at
    /// `y = ±r`, a vertex at `x = ±2r/√3`, the boundary reading a half
    /// all the way round, and the field a true distance.
    #[test]
    fn a_hexagon_stands_on_its_apothem_and_its_circumradius() {
        let r = 30.0f32;
        let circum = 2.0 * r / 3.0f32.sqrt();
        let e = 1e-3;
        for p in [[0.0, r], [0.0, -r], [r * 0.4, r], [circum, 0.0], [-circum, 0.0]] {
            assert!(d_hex(p, r).abs() <= e, "{p:?} reads {}", d_hex(p, r));
        }
        assert!(d_hex([0.0, 0.0], r) < -r + e, "the centre is an apothem deep");
        // A vertex of the FLAT-topped hexagon sits on +x; turned by 30°
        // it sits on +y, which is what `pointy` means.
        let turn = std::f32::consts::FRAC_PI_6;
        let pointy = |p: [f32; 2]| d_hex(turned(p, -turn), r);
        assert!(pointy([0.0, circum]).abs() <= e, "pointy has its vertex on +y");
        assert!(pointy([r, 0.0]).abs() <= e, "and a flat edge on +x");
        boundary_reads_half(|p| d_hex(p, r), 200.0, "hexagon");
        // The six vertices, where a hexagon's own medial axis reaches
        // the boundary.
        let verts: Vec<[f32; 2]> = (0..6)
            .map(|k| {
                let a = k as f32 * std::f32::consts::FRAC_PI_3;
                [a.cos() * circum, a.sin() * circum]
            })
            .collect();
        gradient_is_unit(|p| d_hex(p, r), [60.0, 60.0], &verts);
    }

    /// The chevron collapses the end the caller named and leaves the
    /// other one alone: the tip lands at mid-height on the rect's own
    /// edge, the cut lands `depth` inside it, and an end of depth zero
    /// gives back the box exactly — the same numbers, not merely the
    /// same picture.
    #[test]
    fn a_chevron_collapses_the_end_it_was_given_and_no_other() {
        let b = [50.0f32, 20.0];
        let (e, depth) = (1e-3, 16.0f32);
        let one = |p| d_chevron(p, b, 0.0, depth);
        assert!(one([b[0], 0.0]).abs() <= e, "the right tip is on the edge");
        assert!(one([b[0] - depth, b[1]]).abs() <= e, "the cut meets the bottom");
        assert!(one([b[0] - depth, -b[1]]).abs() <= e, "and the top");
        // The MIDDLE of each slant, which is the only place the two of
        // them can be told apart. The two ends above sit on the rect's
        // own boundary, where the box distance is zero anyway, so a
        // chevron that collapsed one side only would satisfy them both.
        for half in [-1.0f32, 1.0] {
            let mid = [b[0] - depth * 0.5, half * b[1] * 0.5];
            assert!(one(mid).abs() <= e, "the slant at {mid:?} reads {}", one(mid));
        }
        assert!(one([b[0] - 1.0, 0.0]) < 0.0, "mid-height is still inside");
        assert!(one([b[0] - 1.0, b[1] - 1.0]) > 0.0, "the cut corner is gone");
        // The uncollapsed end is the box's, to the bit: a depth of zero
        // is not a special case in the formula, it is the identity.
        for p in [[-b[0], 0.0], [-b[0] + 3.0, 5.0], [-b[0] - 4.0, 0.0]] {
            assert_eq!(one(p), d_box(p, b), "the left end moved: {p:?}");
        }
        boundary_reads_half(one, 400.0, "chevron");
        // Everywhere but the tip the collapse leaves, the field is a
        // true distance — the tip is the acute wedge `max` cannot see
        // round.
        gradient_is_unit(one, [70.0, 40.0], &[[b[0], 0.0]]);
    }

    /// The arc is the ONE thing the Box family cannot spell — and the
    /// test says both halves of that sentence. Closed, it reproduces the
    /// annulus a Box record draws through its own round corner, to
    /// within the float noise of two different routes to one circle.
    /// Cut short, it grows caps the box has no way to state.
    #[test]
    fn a_closed_arc_is_the_box_s_own_annulus_and_a_cut_one_is_not() {
        let (ra, rb) = (40.0f32, 6.0);
        let closed = std::f32::consts::PI;
        for i in 0..64 {
            let a = i as f32 / 64.0 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            for t in [ra - rb, ra - 2.0, ra, ra + 2.0, ra + rb] {
                let p = [c * t, s * t];
                // The annulus as a Box record reads it: the silhouette
                // is the disc of radius ra + rb, and the inward band of
                // width 2·rb is the ring itself.
                let outer = ra + rb;
                let disc = d_round(p, [outer, outer], outer);
                let band = disc.max(-disc - 2.0 * rb);
                let arc = d_arc(p, closed, ra, rb);
                assert!((band - arc).abs() <= 1e-3, "at {t} on ray {i}: {band} vs {arc}");
            }
        }
        // Cut to a quarter turn: the far side is empty and the cap is a
        // half circle of radius rb about the end of the axis.
        let quarter = std::f32::consts::FRAC_PI_4;
        let far = d_arc([0.0, -ra], quarter, ra, rb);
        assert!(far > 0.0, "the unswept side is outside, not inside: {far}");
        let (s, c) = quarter.sin_cos();
        let cap = [s * ra, c * ra];
        let d_cap = d_arc(cap, quarter, ra, rb);
        assert!((d_cap + rb).abs() <= 1e-3, "the cap's centre is rb deep, not {d_cap}");
        // The cap is ROUND: a half width further along the tangent — off
        // the end of the axis, where a butt cap would have stopped — the
        // field still reads the boundary.
        let tip = [cap[0] + rb * c, cap[1] - rb * s];
        assert!(d_arc(tip, quarter, ra, rb).abs() <= 1e-3, "the cap is not round");
        // A ring's centre is OUTSIDE it, so the bisection every other
        // silhouette is proved by has no inside to start from: both of
        // its boundaries are read straight instead.
        for i in 0..64 {
            let a = i as f32 / 64.0 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            for t in [ra - rb, ra + rb] {
                let cov = coverage(d_arc([c * t, s * t], closed, ra, rb), 1.0);
                assert!((cov - 0.5).abs() <= 0.02, "ring at {t}, direction {i}: {cov}");
            }
        }
        // Inside the band, and outside it only as far as coverage ever
        // looks: past that lies the exterior medial axis every
        // non-convex silhouette has, the hole's own centre included.
        gradient_is_unit(|p| d_arc(p, quarter, ra, rb), [60.0, 60.0], &[]);
    }

    /// The record is the only thing the shader gets, so the reference
    /// has to read it the same way: the kind out of bits 8-11, the
    /// lengths out of `corner`, the angles out of `arc_*`. What
    /// [`crate::draw::DrawList::shape`] wrote is what `d_record`
    /// answers.
    #[test]
    fn the_record_carries_each_kind_s_own_numbers() {
        use crate::draw::{DrawList, ShapeKind, ShapeSpec};
        use crate::theme::Color;
        use crate::Rect;
        let r = Rect::new(0.0, 0.0, 120.0, 60.0);
        let emit = |kind| {
            let mut dl = DrawList::new();
            dl.shape(&ShapeSpec {
                rect: r,
                corners: [Corner::round(9.0); 4],
                kind,
                fill: Some(Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }),
                stroke: None,
                glass: None,
                soft: None,
            });
            dl.shapes()[0]
        };
        let turn = std::f32::consts::FRAC_PI_6;
        let hex = emit(ShapeKind::Hex { turn });
        assert_eq!(hex.flags >> crate::draw::Shape::KIND_SHIFT & 0xF, 2);
        assert_eq!(hex.arc_dir, turn);
        // The corner treatments the caller happened to pass do NOT
        // reach a record that has no corners: bits 0-7 are Box's alone.
        assert_eq!(hex.flags & 0xFF, 0, "a hexagon carried Box's corner bits");
        assert_eq!(hex.corner[1..], [0.0; 3], "only the apothem is spent");
        // A pointy hexagon in a 120x60 rect is limited by the HEIGHT:
        // its circumradius runs along y, so apothem = 30·(√3/2).
        let want = 30.0 * 3.0f32.sqrt() * 0.5;
        assert!((hex.corner[0] - want).abs() <= 1e-3, "apothem {}", hex.corner[0]);
        assert!(d_record(&hex, [0.0, 30.0]).abs() <= 1e-2, "the vertex is on the rect");
        assert!(d_record(&hex, [59.0, 0.0]) > 0.0, "and the rect's own corner is not");

        let chev = emit(ShapeKind::Chevron { left: 0.0, right: 18.0 });
        assert_eq!(chev.flags >> crate::draw::Shape::KIND_SHIFT & 0xF, 3);
        assert_eq!([chev.corner[0], chev.corner[1]], [0.0, 18.0]);
        assert_eq!((chev.arc_half, chev.arc_dir), (0.0, 0.0), "a chevron has no angle");
        assert!(d_record(&chev, [60.0, 0.0]).abs() <= 1e-3, "the tip is on the edge");
        assert!(d_record(&chev, [-59.0, 29.0]) < 0.0, "the untouched end is square");

        let ring = emit(ShapeKind::Ring { width: 8.0, half_sweep: 0.6, dir: 0.3 });
        assert_eq!(ring.flags >> crate::draw::Shape::KIND_SHIFT & 0xF, 1);
        assert_eq!(ring.corner[0], 4.0, "half the thickness");
        assert_eq!((ring.arc_half, ring.arc_dir), (0.6, 0.3));
        // The band's outer edge meets the shorter side, turned by dir.
        // Where that is, is stated as a POSITION ON THE GLASS and not by
        // calling `turned` — a probe built out of the rotation it is
        // checking cancels the rotation's sign and passes either way,
        // which is how three sign mutations once survived this file
        // whole. `the_turn_runs_clockwise_on_the_glass` owns the
        // convention; this line only spends it.
        let out = at(30.0, std::f32::consts::FRAC_PI_2 + 0.3);
        assert!(d_record(&ring, out).abs() <= 1e-3, "outer edge reads {}", d_record(&ring, out));
    }

    /// A point `radius` from the centre at `angle` MEASURED ON THE
    /// GLASS: clockwise from +x, because y grows downward here, so a
    /// growing angle runs 3 o'clock → 6 o'clock → 9 o'clock.
    ///
    /// Written out rather than taken from [`turned`] on purpose: this is
    /// the fixed frame the tests below state their expectations in, and
    /// it has to be independent of the function whose sign they pin.
    fn at(radius: f32, angle: f32) -> [f32; 2] {
        [radius * angle.cos(), radius * angle.sin()]
    }

    /// **The one convention two crates share, stated as geometry.**
    ///
    /// `turned` and the `−arc_dir` in [`d_record`] are the whole of how
    /// an angle reaches a silhouette, and `fs_shape` has to reproduce
    /// both from a comment. Until this test there was nothing to
    /// reproduce: flipping the sign in `turned`, or dropping the minus
    /// in either of `d_record`'s two turning kinds, left every test in
    /// this file passing. The old probes could not see it — one was
    /// built by calling `turned` and then read back through `turned`,
    /// which cancels whatever sign it has, and the other turned a
    /// hexagon by 30°, an angle at which the lattice is its own mirror.
    ///
    /// So the expectations here are clock positions, and both turns are
    /// angles that are NOT symmetries: 15° on a six-fold lattice, and an
    /// arc whose swept wedge and its mirror image do not overlap.
    #[test]
    fn the_turn_runs_clockwise_on_the_glass() {
        use crate::draw::{DrawList, ShapeKind, ShapeSpec};
        use crate::theme::Color;
        use crate::Rect;
        let quarter = std::f32::consts::FRAC_PI_2;

        // A quarter turn takes 6 o'clock to 9 o'clock and 3 o'clock to 6
        // — clockwise, the sense every other angle in this project has.
        let down = turned([0.0, 1.0], quarter);
        assert!((down[0] + 1.0).abs() <= 1e-6 && down[1].abs() <= 1e-6, "+y went to {down:?}");
        let right = turned([1.0, 0.0], quarter);
        assert!((right[1] - 1.0).abs() <= 1e-6 && right[0].abs() <= 1e-6, "+x went to {right:?}");

        let r = Rect::new(0.0, 0.0, 120.0, 120.0);
        let emit = |kind| {
            let mut dl = DrawList::new();
            dl.shape(&ShapeSpec {
                rect: r,
                corners: [Corner::SQUARE; 4],
                kind,
                fill: Some(Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 }),
                stroke: None,
                glass: None,
                soft: None,
            });
            dl.shapes()[0]
        };

        // The hexagon. At turn 0 a flat edge sits at the top, so its
        // midpoint is straight up — 12 o'clock, `−quarter` on the glass
        // — one apothem from the centre. Turn the LATTICE by 15° and
        // that midpoint has to move 15° clockwise with it.
        let turn = std::f32::consts::PI / 12.0;
        let hex = emit(ShapeKind::Hex { turn });
        let apothem = hex.corner[0];
        assert!(apothem > 40.0, "the fitted apothem collapsed: {apothem}");
        let on = at(apothem, -quarter + turn);
        assert!(d_record(&hex, on).abs() <= 1e-2, "the flat edge is not at 12 + 15°: {on:?}");
        // 15° is the whole point: at that angle the MIRROR of the edge
        // midpoint lands 30° off the nearest edge normal, which is as
        // deep inside a hexagon as a point at the apothem's radius ever
        // gets. A lattice turned anticlockwise would put the boundary
        // there and leave the assertion above outside.
        let mirrored = at(apothem, -quarter - turn);
        let inside = d_record(&hex, mirrored);
        assert!(inside < -1.0, "the lattice turned the wrong way: {inside} at {mirrored:?}");

        // The arc. Its middle starts at 6 o'clock (`+quarter`) and the
        // direction turns it clockwise from there. A sweep of ±0.3 rad
        // about 0.9 rad cannot reach −0.9 rad, so the mirror of the
        // middle is not merely off-centre in the band — it is off the
        // band altogether, by more than the band is wide.
        let (dir, half_sweep) = (0.9f32, 0.3f32);
        let ring = emit(ShapeKind::Ring { width: 8.0, half_sweep, dir });
        let (rb, ra) = (ring.corner[0], 60.0 - ring.corner[0]);
        let middle = d_record(&ring, at(ra, quarter + dir));
        assert!((middle + rb).abs() <= 1e-2, "the axis of the arc is not at 6 + dir: {middle}");
        let opposite = d_record(&ring, at(ra, quarter - dir));
        assert!(opposite > rb, "the arc swept the wrong way: {opposite} at the mirror");
        // And the sweep is symmetric ABOUT that middle: both ends of the
        // wedge sit on the axis, at the depth of a cap's own centre.
        for end in [dir - half_sweep, dir + half_sweep] {
            let d = d_record(&ring, at(ra, quarter + end));
            assert!((d + rb).abs() <= 1e-2, "the cap centre at {end} rad reads {d}");
        }
    }

    /// The chamfer passes through (b.x − c, b.y) and (b.x, b.y − c) —
    /// the same condition `chamfer_frame_stroke_never_leaves_the_rect`
    /// pins on the tessellated path — its midpoint lies ON the cut, and
    /// the old square corner sits outside it by k·cos 45°.
    #[test]
    fn a_chamfer_runs_on_its_cut() {
        let k = 10.0f32;
        let e = 1e-3;
        assert!(d_chamfer([B[0] - k, B[1]], B, k).abs() <= e);
        assert!(d_chamfer([B[0], B[1] - k], B, k).abs() <= e);
        assert!(d_chamfer([B[0] - k * 0.5, B[1] - k * 0.5], B, k).abs() <= e);
        let d = d_chamfer(B, B, k);
        assert!((d - k * SQRT1_2).abs() <= e, "{d}");
    }

    /// The band runs inward from the boundary: nothing outside, half the
    /// pixel on the silhouette, all of it in the middle, half again at
    /// depth `stroke`, and nothing past that.
    #[test]
    fn the_band_runs_inward_from_the_boundary() {
        let t = 4.0f32;
        assert_eq!(band_coverage(2.0, t, 1.0), 0.0, "outside");
        assert_eq!(band_coverage(0.0, t, 1.0), 0.5, "on the silhouette");
        assert_eq!(band_coverage(-t * 0.5, t, 1.0), 1.0, "the band's heart");
        assert_eq!(band_coverage(-t, t, 1.0), 0.5, "the inner edge");
        assert_eq!(band_coverage(-t - 2.0, t, 1.0), 0.0, "past the band");
    }

    /// K2's defect, named: the band read as a product of two ramps put
    /// **0.25** on the silhouette where a hard edge covers 0.5 — the
    /// stroke's own outer edge and the silhouette are ONE edge, and
    /// multiplying a thing by itself is not compositing it once.
    #[test]
    fn the_shared_edge_covers_a_half_and_not_a_quarter() {
        let (t, w) = (4.0f32, 1.0f32);
        // What K2 shipped: clamp(0.5 − max(d, −d−stroke)/w) times the
        // silhouette's own coverage.
        let folded = |d: f32| coverage(d.max(-d - t), w) * coverage(d, w);
        assert_eq!(folded(0.0), 0.25, "the quadratic undercoverage");
        assert_eq!(band_coverage(0.0, t, w), 0.5);
        // And on the way out it stays the truth of the geometry: a
        // quarter of the pixel inside means a quarter of the band.
        assert!((band_coverage(0.25, t, w) - 0.25).abs() <= 1e-6);
    }

    /// §2.8 without a floor token: a band thinner than the AA width
    /// keeps its cross-sectional mass — it dims instead of fattening or
    /// vanishing — because the difference of two ramps integrates to the
    /// stroke's width whatever that width is. K2's folded form kept
    /// reading a half at the centre of a 0.2 px hairline, painting it
    /// two and a half times as present as it is.
    #[test]
    fn a_hairline_band_keeps_its_mass() {
        let w = 1.0f32;
        for t in [0.2f32, 0.5, 1.0, 4.0] {
            let step = 0.001;
            let mut mass = 0.0f32;
            let mut d = -t - 4.0;
            while d < 4.0 {
                mass += band_coverage(d, t, w) * step;
                d += step;
            }
            assert!((mass - t).abs() <= 2e-3, "stroke {t}: mass {mass}");
        }
        assert!((band_coverage(0.0, 0.2, w) - 0.2).abs() <= 1e-6);
        let folded = coverage(0.0f32.max(-0.0 - 0.2), w);
        assert_eq!(folded, 0.5, "what K2 painted a 0.2 px hairline as");
    }

    /// §2.10: on the shared silhouette the composed alpha is the
    /// STROKE's own, not 1 − (1 − a)² — the double blend a split
    /// fill+ring pair produces there — and everywhere else the pair's
    /// own answer, bit for bit.
    #[test]
    fn fill_and_stroke_share_one_edge_not_two() {
        let fill = [0.2, 0.4, 0.6, 1.0];
        let stroke = [1.0, 1.0, 1.0, 1.0];
        let t = 4.0f32;
        // On the silhouette: half the pixel, and the band owns all of
        // that half, so the edge blends exactly once.
        let px = compose(fill, stroke, coverage(0.0, 1.0), band_coverage(0.0, t, 1.0));
        assert_eq!(px[3], 0.5);
        let double = 1.0 - (1.0 - 0.5f32) * (1.0 - 0.5);
        assert_ne!(px[3], double, "the split-record double blend");
        // Inside, past the band: the fill alone, bit for bit.
        assert_eq!(compose(fill, stroke, 1.0, 0.0), fill);
        // In the band's heart: the stroke's own colour at full alpha.
        assert_eq!(compose(fill, stroke, 1.0, 1.0), stroke);
        // Stroke alone, no bed: the band IS the alpha, and on the
        // silhouette that is a half — the number K2 halved again.
        let none = [0.0; 4];
        let bare = compose(none, stroke, coverage(0.0, 1.0), band_coverage(0.0, t, 1.0));
        assert_eq!(bare[3], 0.5);
    }

    /// A translucent bed under a translucent border composes exactly as
    /// the two draws did — stroke over fill — everywhere the two draws
    /// were right. This is the property that lets one record replace the
    /// pair without repainting the interface: only the shared edge moves.
    #[test]
    fn the_band_still_reads_as_the_stroke_over_the_fill() {
        let fill = [0.0, 0.0, 0.0, 0.5];
        let stroke = [1.0, 1.0, 1.0, 0.5];
        // Deep in the band, fully covered: what src-over of the pair
        // gives — 0.5 + 0.5·0.5.
        let px = compose(fill, stroke, 1.0, 1.0);
        assert!((px[3] - 0.75).abs() <= 1e-6, "{px:?}");
        // …and its colour is the same weighted sum the blender made:
        // 0.5·white over 0.25 of black.
        assert!((px[0] - 0.5 / 0.75).abs() <= 1e-5, "{px:?}");
    }

    /// **The soft profile is the SPRITE's profile, texel for texel.**
    ///
    /// §2.6 asks that moving a glow from the atlas to the field keeps
    /// its character and changes nothing but the nine-slice's stretched
    /// middle. "Same character" is a judgement on a screen — but "same
    /// function" is not, and this is that: the mask
    /// `FontSystem::bake_masks` writes into the reserved band is read
    /// back and compared against [`soft_profile`] at every one of its
    /// 4096 texels, quantised the same way the baker quantises.
    ///
    /// The mapping is `glow_ring`'s own: the sprite is addressed from
    /// the disk's peak on the path outward to its zero at the rim, so
    /// a texel `t` px from the disk's centre is the profile at distance
    /// `t` from the silhouette, and the reach is the disk's radius.
    ///
    /// Equality is exact rather than approximate because both sides
    /// compute one formula in f32 and truncate the same way; a
    /// tolerance here would hide exactly the drift it is meant to
    /// catch.
    #[test]
    fn the_soft_profile_is_the_sprite_s_own_gauss() {
        let fs = crate::font::FontSystem::new();
        let (mx, my, mw, mh) = crate::font::MASK_SOFT;
        let (cx, cy) = (mw as f32 / 2.0 - 0.5, mh as f32 / 2.0 - 0.5);
        let reach = mw as f32 / 2.0;
        let mut compared = 0usize;
        let mut lit = 0usize;
        for y in 0..mh {
            for x in 0..mw {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let baked = fs.atlas[(my + y) * crate::font::ATLAS_W + (mx + x)];
                let field = (soft_profile(d, reach) * 255.0) as u8;
                assert_eq!(
                    field, baked,
                    "at {d} px from the path the sprite says {baked} and the \
                     field says {field}: the two lanes would draw different glows"
                );
                compared += 1;
                lit += usize::from(baked > 0);
            }
        }
        // Fail closed: a comparison of 4096 zeroes proves nothing.
        assert_eq!(compared, mw * mh);
        assert!(lit > mw * mh / 4, "only {lit} texels carried any light");
    }

    /// [`outside_mask`] is [`coverage`]'s exact complement, and that is
    /// the whole reason a glow may be masked by it without a seam.
    ///
    /// On the boundary pixel of a panel the glow takes the fraction the
    /// panel does not, so the two sum to one pixel's worth of paint. A
    /// `step` at `d = 0` — which is what §2.5's table asks for in words
    /// — would give the glow either all of that pixel or none of it,
    /// and against an antialiased panel that reads as a bright or a
    /// dark hairline all the way round.
    #[test]
    fn the_outside_mask_is_the_coverage_s_complement() {
        for &w in &[0.5f32, 1.0, 2.7, 4.0] {
            for i in -60..=60 {
                let d = i as f32 * 0.1;
                let s = coverage(d, w) + outside_mask(d, w);
                assert!(
                    (s - 1.0).abs() <= 1e-6,
                    "at d={d}, w={w} the pixel adds up to {s}, not 1"
                );
            }
        }
        // And it is inert where it must be: a full pixel outside, none
        // of it in.
        assert_eq!(outside_mask(3.0, 1.0), 1.0);
        assert_eq!(outside_mask(-3.0, 1.0), 0.0);
        assert_eq!(outside_mask(0.0, 1.0), 0.5);
    }

    /// [`shape_alpha`]'s four answers, one per corner of the two-bit
    /// space — the branch the fragment takes, stated where it can be
    /// read without a device.
    ///
    /// The one worth naming is the third: a GAUSS record with no
    /// `OUTSIDE_ONLY` is a shadow, and inside its silhouette it is a
    /// flat 1 — the plateau. That is what lets a shadow keep the core
    /// split a glow has to refuse, and what makes a shadow under a
    /// translucent panel look like a shadow instead of a ring.
    #[test]
    fn the_soft_bits_pick_the_four_answers() {
        use crate::draw::Shape;
        let (w, f) = (1.0f32, 12.0f32);
        // Crisp: the ramp, and the feather is not read at all.
        assert_eq!(shape_alpha(-5.0, w, f, 0), 1.0);
        assert_eq!(shape_alpha(5.0, w, f, 0), 0.0);
        // Shadow: plateau inside, gauss outside, zero past the reach.
        assert_eq!(shape_alpha(-5.0, w, f, Shape::GAUSS), 1.0);
        assert_eq!(shape_alpha(0.0, w, f, Shape::GAUSS), 1.0);
        assert!(shape_alpha(f * 0.5, w, f, Shape::GAUSS) > 0.0);
        assert_eq!(shape_alpha(f, w, f, Shape::GAUSS), 0.0);
        // Glow: nothing inside, half on the boundary, the plain profile
        // once the fragment is clear of it.
        let glow = Shape::GAUSS | Shape::OUTSIDE_ONLY;
        assert_eq!(shape_alpha(-5.0, w, f, glow), 0.0);
        assert!((shape_alpha(0.0, w, f, glow) - 0.5).abs() <= 1e-6);
        assert_eq!(shape_alpha(3.0, w, f, glow), soft_profile(3.0, f));
        // OUTSIDE_ONLY without GAUSS is not a thing the toolkit emits,
        // and the fragment still answers something defensible: the
        // crisp silhouette minus its own interior, which is the empty
        // shape everywhere but the boundary pixel.
        assert_eq!(shape_alpha(-5.0, w, f, Shape::OUTSIDE_ONLY), 0.0);
        assert_eq!(shape_alpha(5.0, w, f, Shape::OUTSIDE_ONLY), 0.0);
    }

    /// d_box is the true Euclidean distance wherever that is checkable
    /// by hand: past an edge the offset, past a corner the diagonal,
    /// at the centre minus the short half side.
    #[test]
    fn the_box_distance_is_euclidean() {
        assert_eq!(d_box([B[0] + 3.0, 0.0], B), 3.0);
        assert_eq!(d_box([0.0, -(B[1] + 7.0)], B), 7.0);
        let d = d_box([B[0] + 3.0, B[1] + 4.0], B);
        assert!((d - 5.0).abs() <= 1e-5, "{d}");
        assert_eq!(d_box([0.0, 0.0], B), -B[1]);
    }

    // ---- The two lanes, measured against each other (f3 §6, level E) --
    //
    // `render.vector` is not a switch to arm on an opinion. What follows
    // rasterises the SAME silhouette twice — once through the tessellated
    // generator the program ships, once through the field above — and
    // grades both against the polygon's own supersampled area. The
    // referee is the geometry; neither lane is the standard. Every
    // threshold below is a measurement, not a hope.

    use crate::base::Rect;
    use crate::draw::ring_points;

    /// The tessellated lane's answer at a point: in or out, 1 or 0 and
    /// nothing between — nothing in this pipeline is antialiased except
    /// text. Crossing count; `ring_points` builds a closed simple
    /// polygon by construction.
    fn inside(poly: &[[f32; 2]], p: [f32; 2]) -> bool {
        let mut w = false;
        for i in 0..poly.len() {
            let a = poly[i];
            let b = poly[(i + 1) % poly.len()];
            if (a[1] > p[1]) != (b[1] > p[1]) {
                let t = (p[1] - a[1]) / (b[1] - a[1]);
                if p[0] < a[0] + t * (b[0] - a[0]) {
                    w = !w;
                }
            }
        }
        w
    }

    /// The polygon's area inside the pixel centred on `p`, to 1/256 —
    /// what a perfect rasteriser would put there, and the number both
    /// lanes are graded against.
    fn pixel_area(poly: &[[f32; 2]], p: [f32; 2], hole: Option<&[[f32; 2]]>) -> f32 {
        const N: usize = 16;
        let mut hit = 0u32;
        for j in 0..N {
            for i in 0..N {
                let q = [
                    p[0] - 0.5 + (i as f32 + 0.5) / N as f32,
                    p[1] - 0.5 + (j as f32 + 0.5) / N as f32,
                ];
                if inside(poly, q) && !hole.is_some_and(|h| inside(h, q)) {
                    hit += 1;
                }
            }
        }
        hit as f32 / (N * N) as f32
    }

    /// What one silhouette measures over its padded bounds, pixel by
    /// pixel. `stroke` picks the band lane — the annulus `ring` draws
    /// between the boundary and the boundary inset by that width — and
    /// `None` the fill lane.
    struct Lanes {
        /// Σ coverage: the area each lane actually paints, and the area
        /// the polygon actually encloses.
        sdf: f64,
        tess: f64,
        area: f64,
        /// The worst single pixel each lane puts wrong.
        e_sdf: f32,
        e_tess: f32,
        /// The worst disagreement between the lanes anywhere, and the
        /// worst more than one pixel from every boundary.
        gap: f32,
        gap_far: f32,
        /// Σ of the band under K2's folded product — the mass the old
        /// form shed. Zero on the fill lane.
        folded: f64,
    }

    fn walk(r: Rect, c: &[Corner; 4], segments: u8, stroke: Option<f32>) -> Lanes {
        let mut outer = Vec::new();
        ring_points(r, c, segments, &mut outer);
        let inner = stroke.map(|t| {
            let ir = Rect::new(r.x + t, r.y + t, r.w - 2.0 * t, r.h - 2.0 * t);
            let ic = [c[0].inset(t), c[1].inset(t), c[2].inset(t), c[3].inset(t)];
            let mut v = Vec::new();
            ring_points(ir, &ic, segments, &mut v);
            v
        });
        let b = [r.w * 0.5, r.h * 0.5];
        let centre = [r.x + b[0], r.y + b[1]];
        let mut m = Lanes {
            sdf: 0.0,
            tess: 0.0,
            area: 0.0,
            e_sdf: 0.0,
            e_tess: 0.0,
            gap: 0.0,
            gap_far: 0.0,
            folded: 0.0,
        };
        let x0 = (r.x - 3.0).floor() as i32;
        let x1 = (r.x + r.w + 3.0).ceil() as i32;
        let y0 = (r.y - 3.0).floor() as i32;
        let y1 = (r.y + r.h + 3.0).ceil() as i32;
        for py in y0..y1 {
            for px in x0..x1 {
                let p = [px as f32 + 0.5, py as f32 + 0.5];
                let d = d_shape([p[0] - centre[0], p[1] - centre[1]], b, c);
                // The field's gradient is unit here: a still screen maps
                // one local px onto one device px, so w is one (§2.3).
                let sdf = match stroke {
                    Some(t) => band_coverage(d, t, 1.0),
                    None => coverage(d, 1.0),
                };
                let tess = match &inner {
                    Some(h) => f32::from(inside(&outer, p) && !inside(h, p)),
                    None => f32::from(inside(&outer, p)),
                };
                let area = pixel_area(&outer, p, inner.as_deref());
                m.sdf += sdf as f64;
                m.tess += tess as f64;
                m.area += area as f64;
                m.e_sdf = m.e_sdf.max((sdf - area).abs());
                m.e_tess = m.e_tess.max((tess - area).abs());
                m.gap = m.gap.max((sdf - tess).abs());
                let far = match stroke {
                    Some(t) => d.abs() > 1.0 && (d + t).abs() > 1.0,
                    None => d.abs() > 1.0,
                };
                if far {
                    m.gap_far = m.gap_far.max((sdf - tess).abs());
                }
                if let Some(t) = stroke {
                    m.folded += (coverage(d.max(-d - t), 1.0) * coverage(d, 1.0)) as f64;
                }
            }
        }
        m
    }

    /// Deliberately off the grid: an integer rect is the one case where
    /// a hard raster is already exact, and it would flatter both lanes.
    fn wide() -> Rect {
        Rect::new(12.3, 20.7, 141.0, 83.0)
    }

    fn mixed_corners() -> [Corner; 4] {
        [
            Corner::round(16.0),
            Corner::chamfer(12.0),
            Corner::SQUARE,
            Corner::round(6.0),
        ]
    }

    /// §2.7's hard invariant for K3, and the reason `DrawList::shape`
    /// snaps outer edges on the CPU: an axis-aligned rect on the pixel
    /// grid with square corners comes out of the field PIXEL FOR PIXEL
    /// what the tessellated quad drew. Antialiasing is allowed to soften
    /// a curve; it is not allowed to smear the interface's own edges
    /// across two half-lit pixels, and the snap is what keeps the ramp
    /// landing exactly on the grid where a border is straight.
    #[test]
    fn the_snapped_axis_aligned_rect_is_pixel_for_pixel_the_old_one() {
        let r = Rect::new(12.0, 20.0, 141.0, 83.0);
        let sq = [Corner::SQUARE; 4];
        let fill = walk(r, &sq, 6, None);
        assert_eq!(fill.gap, 0.0, "the fill lane moved a pixel");
        assert_eq!(fill.sdf, fill.tess);
        for t in [1.0f32, 2.0, 4.0] {
            let band = walk(r, &sq, 6, Some(t));
            assert_eq!(band.gap, 0.0, "the {t} px border moved a pixel");
            assert_eq!(band.sdf, band.tess);
        }
    }

    /// The proof the switch waits on, part one: the two lanes enclose
    /// the SAME AREA, and against a silhouette whose area is known in
    /// closed form the field is the one that gets it right.
    ///
    /// A rounded rect encloses `w·h − (4 − π)·r²` exactly. On a 141×83
    /// panel with 16 px corners that is 11483.25 px². The field paints
    /// 11483.62 — four tenths of a square pixel out, over a 440 px
    /// boundary. The tessellated lane at the ladder the theme actually
    /// ships (`corner.segments` ceiling 6) paints 11470: **13 px²
    /// short**, because a chord is shorter than its arc and a hard
    /// raster then rounds every boundary pixel to nothing or to one.
    /// Antialiasing here does not blur the silhouette — it stops
    /// quantising it.
    #[test]
    fn the_two_lanes_enclose_the_same_area() {
        let r = wide();
        let k = 16.0f64;
        let truth = r.w as f64 * r.h as f64 - (4.0 - std::f64::consts::PI) * k * k;
        let c = [Corner::round(k as f32); 4];
        let fine = walk(r, &c, 16, None);
        let shipped = walk(r, &c, 6, None);
        assert!((fine.sdf - truth).abs() <= 1.0, "field {} truth {truth}", fine.sdf);
        assert!(
            (fine.sdf - truth).abs() * 10.0 < (shipped.tess - truth).abs(),
            "field {} raster {} truth {truth}",
            fine.sdf,
            shipped.tess
        );
        // And on a silhouette with every treatment on it, the two lanes
        // still enclose the same area to under two square pixels — the
        // residue is the first-order coverage's outward bias on convex
        // curvature, three thousandths of a pixel along the boundary,
        // and it is the whole of what §2.3 gives up.
        let m = walk(r, &mixed_corners(), 16, None);
        assert!((m.sdf - m.tess).abs() <= 2.0, "sdf {} tess {}", m.sdf, m.tess);
        assert!((m.sdf - m.area).abs() <= 2.0, "sdf {} area {}", m.sdf, m.area);
    }

    /// Part two: the field reads the polygon's own area per pixel to
    /// within a tenth, where the raster lane is off by half a pixel —
    /// that half-pixel IS the staircase, stated as a number rather than
    /// looked at. Away from the boundary the lanes are identical, so
    /// what the vector lane changes is the edge and nothing else.
    #[test]
    fn the_field_reads_the_area_where_tessellation_reads_a_bit() {
        let m = walk(wide(), &mixed_corners(), 16, None);
        assert!(m.e_sdf <= 0.10, "the field's worst pixel: {}", m.e_sdf);
        assert!(m.e_tess >= 0.4, "the raster's worst pixel: {}", m.e_tess);
        assert_eq!(m.gap_far, 0.0, "the lanes differ away from the boundary");
    }

    /// Part three, the band — the number that decides whether a border
    /// keeps its weight when it stops being triangles.
    ///
    /// A 4 px border survives either way. A ONE px border is where the
    /// raster shows what it costs: hard edges drop about 6 px² of the
    /// 428 the annulus holds, unevenly — the dotted look of a hairline
    /// on a rect whose coordinates are not integers. The field lands
    /// within 1.5 px² of the true annulus at both widths.
    ///
    /// And K2's folded band sheds a quarter of a pixel of rim ALL THE
    /// WAY ROUND — 88 px² here whatever the width, which on a 4 px
    /// border is five per cent and on a 1 px border is **a fifth of the
    /// whole thing**. That is §2.10's double ramp, the reason the switch
    /// shipped false, and what one FILL|STROKE record removes.
    #[test]
    fn the_band_lane_carries_the_generator_s_own_annulus() {
        let c = mixed_corners();
        for t in [4.0f32, 1.0] {
            let m = walk(wide(), &c, 16, Some(t));
            assert!((m.sdf - m.area).abs() <= 1.5, "{t} px: sdf {} area {}", m.sdf, m.area);
            assert!(m.e_sdf <= 0.10, "{t} px: worst band pixel {}", m.e_sdf);
            assert_eq!(m.gap_far, 0.0, "{t} px: the lanes differ off the boundary");
            // K2's product form, on this very annulus.
            assert!(m.sdf - m.folded >= 60.0, "{t} px: folded {} of {}", m.folded, m.sdf);
        }
        // The hairline the raster cannot hold.
        let thin = walk(wide(), &c, 16, Some(1.0));
        assert!(
            thin.area - thin.tess >= 5.0,
            "the raster kept {} of {}",
            thin.tess,
            thin.area
        );
    }

    // ---- The ring of quads, proved pixel by pixel (f3 §7b, remedy 1) -
    //
    // The split cuts the interior out of a shape's quad and draws it as
    // a plain fill, because there the field's answer is known before it
    // is computed. That is a claim about the PICTURE, and a claim about
    // the picture is settled by rasterising both and comparing, not by
    // reasoning about where the ramps land. What follows shades every
    // pixel of both variants out of the functions above — the same
    // functions `fs_shape` implements — and asserts the fragments are
    // equal TO THE BIT.

    use crate::draw::{DrawList, Shape, NO_SHAPE};
    use crate::theme::Color;

    /// The corner treatments back out of a record's flag word — what
    /// the fragment shader reads, read the same way.
    ///
    /// The catch-all is part of the mirror, not an oversight left over
    /// from the four this crate collapsed into `corner.rs`. A cut here
    /// is TWO BITS wide by contract with `fs_shape`, whose own decode
    /// has three arms and a fallback; a reading that walked
    /// `corner::WORDS` instead would be a truer statement about the
    /// toolkit and a FALSE one about the shader, which is the only thing
    /// this file is a specification of.
    fn record_corners(s: &Shape) -> [Corner; 4] {
        [0usize, 1, 2, 3].map(|i| Corner {
            style: match (s.flags >> (2 * i as u32)) & 3 {
                1 => CornerStyle::Round,
                2 => CornerStyle::Chamfer,
                _ => CornerStyle::Square,
            },
            size: s.corner[i],
        })
    }

    /// One `fs_shape` fragment, spelled out of this file's own
    /// functions: the shader is the implementation and these are the
    /// specification, so a proof written here is a proof about the
    /// shader (the note at the top of this file).
    ///
    /// `w`, the AA width, is 1: on a still screen one local pixel is one
    /// device pixel. It would not matter if it were not — both variants
    /// evaluate the SAME field at the SAME points, and the screen
    /// derivatives of `d` are taken over framebuffer-aligned 2×2 blocks
    /// that neither variant can move.
    fn fs_shape(rec: &Shape, local: [f32; 2], colour: [f32; 4]) -> [f32; 4] {
        let d = d_record(rec, local);
        let has = |bit: u32| f32::from(rec.flags & bit != 0);
        let fill = [colour[0], colour[1], colour[2], colour[3] * has(Shape::FILL)];
        compose(
            fill,
            rec.stroke_c,
            coverage(d, 1.0),
            band_coverage(d, rec.stroke, 1.0) * has(Shape::STROKE),
        )
    }

    /// Every fragment a draw list puts on the pixel centred on `p`, in
    /// emission order — a rasteriser small enough to read whole.
    ///
    /// Every quad on this lane is an AXIS-ALIGNED rectangle laid out as
    /// `v0 v1 v2 v0 v2 v3`, so containment is a half-open box test: the
    /// partition a top-left fill rule gives, and the reason the shared
    /// edge between the core and a strip is covered exactly once — no
    /// gap, no double blend.
    ///
    /// A quad outside every record (`NO_SHAPE`) is the ORDINARY FILL
    /// PATH, and it returns the vertex colour: `fs_main` samples the
    /// atlas's white pixel — which is 1, at a texel centre, so filtering
    /// does not touch it — raises it to the text gamma, which leaves 1,
    /// and multiplies the alpha by it.
    fn frags(dl: &DrawList, p: [f32; 2]) -> Vec<[f32; 4]> {
        let mut out = Vec::new();
        for q in dl.verts.chunks_exact(6) {
            let (a, b) = (q[0].pos, q[2].pos);
            let inside = |i: usize| p[i] >= a[i].min(b[i]) && p[i] < a[i].max(b[i]);
            if !inside(0) || !inside(1) {
                continue;
            }
            out.push(if q[0].shape == NO_SHAPE {
                q[0].color
            } else {
                let rec = &dl.shapes()[q[0].shape as usize];
                // The uv contract: a shape vertex carries pos − centre.
                let c = [q[0].pos[0] - q[0].uv[0], q[0].pos[1] - q[0].uv[1]];
                fs_shape(rec, [p[0] - c[0], p[1] - c[1]], q[0].color)
            });
        }
        out
    }

    /// Straight alpha over an opaque destination, as the blender does
    /// it. A fragment with alpha 0 leaves the destination untouched to
    /// the bit — which is how "the interior of a bare border is empty"
    /// and "the interior of a bare border is a transparent fragment"
    /// come out the same picture.
    fn blend(dl: &DrawList, p: [f32; 2], dst: [f32; 3]) -> [f32; 3] {
        let mut d = dst;
        for f in frags(dl, p) {
            for k in 0..3 {
                d[k] = f[k] * f[3] + d[k] * (1.0 - f[3]);
            }
        }
        d
    }

    fn bed() -> Color {
        Color::rgba8(20, 30, 40, 190)
    }

    fn edge() -> Color {
        Color::rgba8(230, 210, 120, 220)
    }

    /// One framed surface, drawn the way the whole toolkit spells one.
    /// `warp` is the control: at 2 the split stays out of the way — a
    /// ride's screen gradient is not one — and the shape rasterises
    /// through whole quads over the same padded bounds, which is the
    /// geometry this remedy replaces. The rects are on the integer grid
    /// and the strokes are whole pixels so that the snap is a no-op and
    /// the two variants write the SAME RECORD; the assertion below
    /// checks that rather than trusting it.
    fn surface(r: Rect, c: &[Corner; 4], fill: bool, stroke: Option<f32>, warp: u8) -> DrawList {
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.set_warp(warp);
        if fill {
            dl.ring_fill(r, c, 16, bed());
        }
        if let Some(t) = stroke {
            dl.ring(r, c, 16, t, edge());
        }
        dl
    }

    /// **The proof the split rests on.** For every pixel of every case,
    /// the frame of five quads and the single quad it replaces leave
    /// the destination bit for bit the same.
    ///
    /// The cases are the ones that can break it: a bare bed, a bed with
    /// a border welded on (where the band deepens AFTER the geometry
    /// was laid out and the core has to be re-cut), a bare border with
    /// no bed at all (where the interior is not drawn at all — §7b's
    /// risk 1, the window frame that would otherwise cost its area),
    /// a hairline, square corners (reach 0, the tightest margin the
    /// core boundary ever gets) and a deep chamfer (the treatment that
    /// eats furthest in).
    #[test]
    fn the_frame_paints_what_the_whole_quad_painted() {
        /// name, rect, corners, has a bed, the border's width.
        type Case<'a> = (&'a str, Rect, &'a [Corner; 4], bool, Option<f32>);
        let deep = [Corner::chamfer(20.0); 4];
        let mix = &mixed_corners();
        let cases: [Case; 6] = [
            ("a bare bed", Rect::new(12.0, 20.0, 200.0, 100.0), mix, true, None),
            ("bed and border", Rect::new(12.0, 20.0, 200.0, 100.0), mix, true, Some(2.0)),
            ("a bare border", Rect::new(12.0, 20.0, 200.0, 100.0), mix, false, Some(3.0)),
            ("a hairline", Rect::new(12.0, 20.0, 200.0, 100.0), mix, true, Some(1.0)),
            ("square corners", Rect::new(0.0, 0.0, 90.0, 60.0), &[Corner::SQUARE; 4], true, Some(1.0)),
            ("a deep chamfer", Rect::new(5.0, 7.0, 150.0, 90.0), &deep, true, Some(2.0)),
        ];
        for (name, r, c, fill, stroke) in cases {
            let split = surface(r, c, fill, stroke, 1);
            let whole = surface(r, c, fill, stroke, 2);
            assert_eq!(split.shapes(), whole.shapes(), "{name}: not the same record");
            assert_eq!(split.shape_len(), 1, "{name}: not one record");
            assert!(
                split.verts.len() == if fill { 30 } else { 24 },
                "{name}: {} vertices — the split did not happen",
                split.verts.len()
            );
            let mut lit = 0u32;
            for py in (r.y as i32 - 3)..(r.y + r.h) as i32 + 3 {
                for px in (r.x as i32 - 3)..(r.x + r.w) as i32 + 3 {
                    let p = [px as f32 + 0.5, py as f32 + 0.5];
                    for dst in [[0.0; 3], [1.0, 0.5, 0.25], [1.0; 3]] {
                        assert_eq!(
                            blend(&split, p, dst),
                            blend(&whole, p, dst),
                            "{name}: pixel {p:?} over {dst:?}"
                        );
                    }
                    // …and every pixel the frame touches, it touches
                    // ONCE. A shared edge covered twice would blend the
                    // fill onto itself; a gap would show the wall.
                    let n = frags(&split, p).len();
                    assert!(n <= 1, "{name}: pixel {p:?} covered {n} times");
                    lit += n as u32;
                }
            }
            assert!(lit > 0, "{name}: nothing was drawn at all");
            // §7b's risk 1, settled: a border with no bed under it
            // rasterises its PERIMETER and not its area. The middle of
            // the window frame is not a transparent fragment — it is
            // not a fragment.
            let middle = [r.x + r.w * 0.5, r.y + r.h * 0.5];
            if !fill {
                assert!(frags(&split, middle).is_empty(), "{name}: the middle was drawn");
                assert_eq!(frags(&whole, middle).len(), 1, "{name}: the control");
                assert!(
                    (lit as f32) < (r.w + 2.0) * (r.h + 2.0),
                    "{name}: {lit} pixels, the whole quad"
                );
            } else {
                assert_eq!(frags(&split, middle).len(), 1, "{name}: the bed has a hole");
            }
        }
    }

    /// What the remedy buys, on the document's own panel: 315×175 with
    /// a 6.5 px corner and a 1 px border. §7b measured the interior at
    /// ~101 instructions a pixel against ~5 on the ordinary fill path,
    /// over the whole 55 kpx of the padded quad. After the cut the
    /// field sees a 10.5 px band round the perimeter — under a fifth of
    /// the pixels, and the other four fifths pay the fill's price.
    ///
    /// The band is `corner + stroke + AA_PAD + CORE_PAD` deep and the
    /// last two are the margin the proof above needs; a tighter margin
    /// would buy a few per cent more and would have to be argued for
    /// against multisampling, which shades at the pixel centre.
    #[test]
    fn the_field_stops_paying_for_the_interior() {
        let r = Rect::new(0.0, 0.0, 315.0, 175.0);
        let dl = surface(r, &[Corner::round(6.5); 4], true, Some(1.0), 1);
        let area = |q: &[crate::draw::Vertex]| {
            ((q[2].pos[0] - q[0].pos[0]) * (q[2].pos[1] - q[0].pos[1])).abs()
        };
        let mut field = 0.0f32;
        let mut plain = 0.0f32;
        for q in dl.verts.chunks_exact(6) {
            *if q[0].shape == NO_SHAPE { &mut plain } else { &mut field } += area(q);
        }
        let padded = (r.w + 2.0) * (r.h + 2.0);
        assert!((field + plain - padded).abs() <= 0.01, "the frame is not the quad");
        assert!(
            field * 5.0 <= padded,
            "the field still pays for {field} px of {padded}"
        );
        assert!(plain > 0.0);

        // §7b's RISK 1 by name: `winframe.rs:453` draws a border over
        // the whole window and no fill under it, so the analysis that
        // counted vertices said "cheap" where the fragment count said
        // the area of the screen. Cut, it costs its perimeter: a
        // 1200×800 frame with a 6 px corner and a 1 px border asks the
        // field for 43 604 px of the 964 004 it covered — a 10 px band
        // round the edge, twenty-two times less.
        let w = Rect::new(0.0, 0.0, 1200.0, 800.0);
        let dl = surface(w, &[Corner::round(6.0); 4], false, Some(1.0), 1);
        let field: f32 = dl.verts.chunks_exact(6).map(area).sum();
        let padded = (w.w + 2.0) * (w.h + 2.0);
        assert!(dl.verts.iter().all(|v| v.shape == 0), "a bed appeared");
        assert!(
            field * 20.0 <= padded,
            "the frame still pays for {field} px of {padded}"
        );
    }

    // ---- The oriented lane, measured (f3 §3.1, §K4) ------------------
    //
    // Everything below grades the DIAGONAL against the same referee K3
    // used for the rect: the silhouette's own supersampled area. The
    // tessellated lane is not the standard — it is the control, and the
    // staircase it draws is stated as a number rather than looked at.

    /// The identity the joint disc and the dot matrix stand on, checked
    /// where it could fail: the corner arc's own quadrant, the axes,
    /// the centre and well outside. `d_round` with `k` equal to both
    /// half sizes has no straight edge left to be the box's — every
    /// point is in the corner's quadrant — and reduces, term for term,
    /// to `|p| − r`.
    #[test]
    fn a_disc_is_the_box_family_s_own_round_corner() {
        for r in [0.5f32, 1.0, 3.0, 12.5] {
            for i in 0..37 {
                let a = i as f32 / 37.0 * std::f32::consts::TAU;
                let (s, c) = a.sin_cos();
                for m in [0.0f32, 0.3, 1.0, 1.7, 4.0] {
                    let p = [c * r * m, s * r * m];
                    let round = d_round(p, [r, r], r);
                    assert!(
                        (round - d_disc(p, r)).abs() <= 1e-4,
                        "r {r} at {p:?}: box family {round}, circle {}",
                        d_disc(p, r)
                    );
                }
            }
        }
    }

    /// The frame is a rigid motion, and that is what makes the shader's
    /// arithmetic survive it untouched: the map is its own inverse's
    /// inverse, it preserves distance, and the field's SCREEN gradient
    /// stays exactly one — so `w` is the pixel it always was and the
    /// coverage ramp has the width §2.3 sized it to.
    #[test]
    fn an_oriented_frame_moves_the_shape_and_not_the_field() {
        let half = [30.0f32, 2.0];
        for i in 0..24 {
            let a = i as f32 / 24.0 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            let (f, len) = Frame::along([100.0, 60.0], [100.0 + c * 80.0, 60.0 + s * 80.0]).unwrap();
            assert!((len - 80.0).abs() <= 1e-3);
            // Round trip, and distance preserved: an isometry.
            for l in [[0.0, 0.0], [half[0], half[1]], [-7.5, 3.25]] {
                let back = f.to_local(f.to_screen(l));
                assert!((back[0] - l[0]).abs() <= 1e-3 && (back[1] - l[1]).abs() <= 1e-3);
            }
            let (p, q) = (f.to_screen([-9.0, 1.0]), f.to_screen([4.0, -3.0]));
            let d = ((p[0] - q[0]).powi(2) + (p[1] - q[1]).powi(2)).sqrt();
            assert!((d - (13.0f32 * 13.0 + 4.0 * 4.0).sqrt()).abs() <= 1e-3, "{d}");
            // …and the width the shader would take from its own
            // derivatives is one, everywhere the coverage ramp reads
            // it: on the long edge, on the end cap, a pixel outside
            // each, and inside on the way to them. (Not at the CENTRE:
            // the box distance has a crease along the axis of a thin
            // shape, where a central difference cancels and the
            // gradient does not exist. It is also `cov = 1` there, four
            // ramp widths from anything.)
            let field = |p: [f32; 2]| d_box(f.to_local(p), half);
            for l in [
                [0.0, half[1]],
                [half[0], 0.0],
                [0.0, half[1] + 1.0],
                [half[0] + 1.0, 0.0],
                [0.0, half[1] * 0.5],
            ] {
                let w = screen_width(field, f.to_screen(l));
                assert!((w - 1.0).abs() <= 1e-3, "angle {i} at {l:?}: w {w}");
            }
        }
    }

    /// What one oriented record puts on the pixel centred on `p`, and
    /// what the quad the toolkit draws today puts there — the two lanes
    /// of a segment, side by side, plus the truth both are graded
    /// against.
    ///
    /// The quad is the silhouette's own four corners, so `inside` and
    /// `pixel_area` from K3's harness serve unchanged; only the field
    /// has to learn the frame.
    struct Seg {
        /// Σ coverage, the area each lane paints, and the true one.
        sdf: f64,
        tess: f64,
        area: f64,
        /// The worst single pixel each lane puts wrong…
        e_sdf: f32,
        e_tess: f32,
        /// …and the worst the field puts wrong along the SIDES, clear
        /// of the two end caps, where a right angle folds two edges
        /// into one pixel and one ramp cannot describe them.
        e_side: f32,
        /// The worst disagreement more than a pixel from the boundary.
        gap_far: f32,
    }

    fn segment_lanes(a: [f32; 2], b: [f32; 2], t: f32) -> Seg {
        let (f, len) = Frame::along(a, b).unwrap();
        let half = [len * 0.5, t * 0.5];
        let poly: Vec<[f32; 2]> = [[-1.0, -1.0], [1.0, -1.0], [1.0, 1.0], [-1.0, 1.0]]
            .iter()
            .map(|s| f.to_screen([s[0] * half[0], s[1] * half[1]]))
            .collect();
        let field = |p: [f32; 2]| d_box(f.to_local(p), half);
        let mut m = Seg {
            sdf: 0.0,
            tess: 0.0,
            area: 0.0,
            e_sdf: 0.0,
            e_tess: 0.0,
            e_side: 0.0,
            gap_far: 0.0,
        };
        let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for p in &poly {
            x0 = x0.min(p[0] - 3.0);
            y0 = y0.min(p[1] - 3.0);
            x1 = x1.max(p[0] + 3.0);
            y1 = y1.max(p[1] + 3.0);
        }
        for py in y0.floor() as i32..y1.ceil() as i32 {
            for px in x0.floor() as i32..x1.ceil() as i32 {
                let p = [px as f32 + 0.5, py as f32 + 0.5];
                let d = field(p);
                let sdf = coverage(d, screen_width(field, p));
                let tess = f32::from(inside(&poly, p));
                let area = pixel_area(&poly, p, None);
                m.sdf += sdf as f64;
                m.tess += tess as f64;
                m.area += area as f64;
                m.e_sdf = m.e_sdf.max((sdf - area).abs());
                m.e_tess = m.e_tess.max((tess - area).abs());
                if f.to_local(p)[0].abs() <= half[0] - 2.0 {
                    m.e_side = m.e_side.max((sdf - area).abs());
                }
                if d.abs() > 1.0 {
                    m.gap_far = m.gap_far.max((sdf - tess).abs());
                }
            }
        }
        m
    }

    /// **The proof the diagonal lane rests on.** At two dozen angles, a
    /// stroke four pixels wide: along its sides the field reads the
    /// silhouette's own area to within a tenth of a pixel, where the
    /// hard raster is off by half — that half-pixel IS the staircase —
    /// and the two agree exactly wherever they are more than a pixel
    /// from the boundary. The areas enclosed agree to under two square
    /// pixels over a 98 px perimeter.
    ///
    /// **What the field does NOT get right, named rather than hidden:**
    /// the pixel a right-angle CORNER falls in. Two edges meet inside
    /// it, one signed distance cannot say where both are, and the ramp
    /// can be a quarter of a pixel out — 0.19 at the worst angle of
    /// these two dozen. It is the same residue K3 measured on the
    /// square corner of a rect, it is bounded by the corner's own
    /// quarter, and it costs two pixels at the end of a stroke against
    /// a staircase down the whole of it.
    ///
    /// The angles are deliberately not multiples of anything: 24 turns
    /// of the circle put edges at every relation to the pixel grid,
    /// except the two the raster flatters — the axis-aligned ones,
    /// which the emitter refuses to send here at all (§2.7).
    #[test]
    fn the_oriented_segment_reads_the_area_where_the_raster_reads_a_bit() {
        let mut worst_side = 0.0f32;
        let mut worst_corner = 0.0f32;
        let mut worst_raster = 0.0f32;
        for i in 0..24 {
            let a = (i as f32 + 0.37) / 24.0 * std::f32::consts::TAU;
            let (s, c) = a.sin_cos();
            let from = [60.3, 40.7];
            let to = [from[0] + c * 45.0, from[1] + s * 45.0];
            let m = segment_lanes(from, to, 4.0);
            assert!(m.e_side <= 0.10, "angle {i}: the field's worst side pixel {}", m.e_side);
            assert!(m.e_sdf <= 0.26, "angle {i}: the field's worst corner {}", m.e_sdf);
            assert!(m.e_tess >= 0.4, "angle {i}: the raster's worst pixel {}", m.e_tess);
            assert_eq!(m.gap_far, 0.0, "angle {i}: the lanes differ off the boundary");
            assert!(
                (m.sdf - m.area).abs() <= 2.0,
                "angle {i}: field {} area {}",
                m.sdf,
                m.area
            );
            worst_side = worst_side.max(m.e_side);
            worst_corner = worst_corner.max(m.e_sdf);
            worst_raster = worst_raster.max(m.e_tess);
        }
        assert!(worst_raster > 4.0 * worst_side, "{worst_raster} vs {worst_side}");
        assert!(worst_corner > worst_side, "the corner is the residue, and it is real");
    }

    /// §2.8 on the lane that needs it. A single coverage ramp is a
    /// HALF-PLANE's: run it on a slab thinner than the filter and the
    /// slab reads far heavier than it is — 0.75 of a pixel for a 0.5 px
    /// stroke, and 0.65 for a 0.3 px one, which is more than twice its
    /// mass. [`thin_band`] draws it a pixel wide and dims it by what it
    /// lost, and the mass comes back exactly at every width and every
    /// offset from the grid.
    #[test]
    fn a_sub_pixel_stroke_dims_instead_of_fattening() {
        // The mass of one cross-section: Σ over the pixel column the
        // slab runs through, for a slab whose centre line sits at `c`.
        // Pixel centres are at k + ½, so c = ½ is a slab through the
        // middle of a pixel and c = 1 one straddling two.
        let mass = |t: f32, c: f32, floored: bool| {
            let (w, dim) = if floored { thin_band(t) } else { (t, 1.0) };
            let half = w * 0.5;
            (-6i32..6)
                .map(|k| coverage((k as f32 + 0.5 - c).abs() - half, 1.0) * dim)
                .sum::<f32>()
        };
        for t in [0.2f32, 0.3, 0.5, 0.8, 1.0, 2.5] {
            for c in [0.5f32, 0.75, 1.0, 1.25] {
                let got = mass(t, c, true);
                assert!((got - t).abs() <= 1e-3, "{t} px at {c}: mass {got}");
            }
        }
        // What it would have painted without the rule, at the two
        // widths the sentence above names.
        assert!((mass(0.5, 0.5, false) - 0.75).abs() <= 1e-6);
        assert!((mass(0.3, 0.5, false) - 0.65).abs() <= 1e-6);
        // …and above a pixel the rule is not there at all.
        assert_eq!(thin_band(1.0), (1.0, 1.0));
        assert_eq!(thin_band(2.5), (2.5, 1.0));
    }

    /// The joint, measured — §3.1's ruling put to the referee.
    ///
    /// The truth of a stroked polyline is the set of points within
    /// `t/2` of the path: a round join, which is what a disc at the
    /// corner builds. Two butt-capped segments alone leave the outer
    /// wedge EMPTY, and antialiasing does not hide it — it draws it
    /// accurately. The disc fills it.
    ///
    /// What the disc costs is stated here too, because §3.1's claim
    /// that "nothing overlaps" is not true of this decomposition and
    /// the honest thing is to say by how much: the disc lies over the
    /// half of itself that is inside each segment, so a TRANSLUCENT
    /// stroke blends twice there. The overlap is confined to the disc —
    /// never more than `π(t/2)²` — and for an opaque stroke it is
    /// invisible, since `a = 1` composites the same either way. Both
    /// numbers below are measured, not argued.
    #[test]
    fn a_joint_disc_closes_the_notch_the_two_segments_leave() {
        let pts = [[20.0f32, 70.0], [60.0, 20.0], [100.0, 62.0]];
        let t = 7.0f32;
        // What the lane draws: two oriented boxes, and the disc at the
        // corner between them.
        let boxes: Vec<(Frame, [f32; 2])> = pts
            .windows(2)
            .map(|w| {
                let (f, len) = Frame::along(w[0], w[1]).unwrap();
                (f, [len * 0.5, t * 0.5])
            })
            .collect();
        let disc = (Frame::upright(pts[1]), [t * 0.5, t * 0.5]);
        // The truth, supersampled 16×16: the SILHOUETTE, as a set —
        // the two rectangles, and the disc when the joint is drawn.
        let sample = |p: [f32; 2], hit: &dyn Fn([f32; 2]) -> bool| {
            const N: usize = 16;
            let mut n = 0u32;
            for j in 0..N {
                for i in 0..N {
                    let q = [
                        p[0] - 0.5 + (i as f32 + 0.5) / N as f32,
                        p[1] - 0.5 + (j as f32 + 0.5) / N as f32,
                    ];
                    if hit(q) {
                        n += 1;
                    }
                }
            }
            n as f32 / (N * N) as f32
        };
        let in_arms = |q: [f32; 2]| {
            boxes.iter().any(|(f, half)| {
                let l = f.to_local(q);
                l[0].abs() <= half[0] && l[1].abs() <= half[1]
            })
        };
        let in_disc = |q: [f32; 2]| {
            ((q[0] - pts[1][0]).powi(2) + (q[1] - pts[1][1]).powi(2)).sqrt() <= t * 0.5
        };
        let truth = |p: [f32; 2]| sample(p, &|q| in_arms(q) || in_disc(q));
        // THE DISC IS THE RIGHT SHAPE, not merely a shape: near the
        // corner, the union above is exactly the set of points within
        // t/2 of the path — the round join a stroked path is defined to
        // have. Checked where it is checkable, which is everywhere the
        // butt-capped ENDS are out of reach.
        let to_seg = |p: [f32; 2], a: [f32; 2], b: [f32; 2]| {
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l2 = dx * dx + dy * dy;
            let u = (((p[0] - a[0]) * dx + (p[1] - a[1]) * dy) / l2).clamp(0.0, 1.0);
            ((a[0] + dx * u - p[0]).powi(2) + (a[1] + dy * u - p[1]).powi(2)).sqrt()
        };
        let round_join = |p: [f32; 2]| {
            sample(p, &|q| {
                to_seg(q, pts[0], pts[1]).min(to_seg(q, pts[1], pts[2])) <= t * 0.5
            })
        };
        for py in 8..36 {
            for px in 44..78 {
                let p = [px as f32 + 0.5, py as f32 + 0.5];
                assert_eq!(truth(p), round_join(p), "the joint is not a round join at {p:?}");
            }
        }
        let cov = |f: &Frame, half: [f32; 2], round: bool, p: [f32; 2]| {
            let c = if round {
                [Corner::round(half[1]); 4]
            } else {
                [Corner::SQUARE; 4]
            };
            let field = |q: [f32; 2]| d_shape(f.to_local(q), half, &c);
            coverage(field(p), screen_width(field, p))
        };
        // Straight alpha, in emission order, over an empty destination
        // — the alpha the blender ends up with is all this compares.
        let alpha = |p: [f32; 2], joint: bool, a: f32| {
            let mut acc = 0.0f32;
            for (f, half) in &boxes {
                acc += cov(f, *half, false, p) * a * (1.0 - acc);
            }
            if joint {
                acc += cov(&disc.0, disc.1, true, p) * a * (1.0 - acc);
            }
            acc
        };
        let bare_truth = |p: [f32; 2]| sample(p, &in_arms);
        let (mut bare, mut with, mut over) = (0.0f64, 0.0f64, 0.0f64);
        let (mut worst_bare, mut worst_with, mut worst_end) = (0.0f32, 0.0f32, 0.0f32);
        for py in 10..80 {
            for px in 10..110 {
                let p = [px as f32 + 0.5, py as f32 + 0.5];
                let want = truth(p);
                let miss = (alpha(p, true, 1.0) - want).abs();
                // The two BUTT ENDS carry the right-angle corner the
                // segment test already measured and named; the joint is
                // what this test is about, so the two are counted apart.
                let end = [pts[0], pts[2]]
                    .iter()
                    .any(|e| ((p[0] - e[0]).powi(2) + (p[1] - e[1]).powi(2)).sqrt() <= t);
                if end {
                    worst_end = worst_end.max(miss);
                } else {
                    worst_with = worst_with.max(miss);
                }
                worst_bare = worst_bare.max(want - alpha(p, false, 1.0));
                bare += (alpha(p, false, 1.0) - want).abs() as f64;
                with += miss as f64;
                // The double blend, on a half-translucent stroke —
                // measured as what the DISC adds, because the two arms
                // already overlap each other in the wedge at every
                // joint and have since the toolkit was written. Each
                // side is the ink laid past what one blend of its own
                // silhouette would have laid.
                let half_a = 0.5f32;
                let now = (alpha(p, true, half_a) - want * half_a).max(0.0);
                let before = (alpha(p, false, half_a) - bare_truth(p) * half_a).max(0.0);
                over += (now - before) as f64;
            }
        }
        // The notch: without the disc a WHOLE PIXEL goes missing at the
        // outer corner, and the shortfall over the joint adds up to 18
        // square pixels of a stroke seven wide.
        assert!(worst_bare >= 0.9, "the notch was only {worst_bare} deep");
        assert!(bare >= 15.0, "the notch cost only {bare} px");
        // With it, the picture is the round join, and what is left is
        // the residue of drawing a union as a sequence of blends: where
        // the disc's own ramp crosses an arm's, two partial coverages
        // are composited as if they were independent, and they are not.
        // A quarter of a pixel at the worst of them, half the total
        // error of the bare pair, and both numbers measured here.
        assert!(worst_with <= 0.25, "worst joint pixel {worst_with}");
        assert!(worst_end <= 0.25, "worst end-cap pixel {worst_end}");
        assert!(with * 1.9 <= bare, "with {with} bare {bare}");
        // And the price, named: on a HALF-TRANSLUCENT stroke the disc
        // lays about six square pixels of extra ink — a sixth of its
        // own area, all of it inside the silhouette, none outside it.
        // On an opaque stroke it costs nothing at all, because `a = 1`
        // composites the same either way. The arms' own overlap in the
        // wedge is not counted: it is older than this lane and the disc
        // does not add to it.
        assert!(over <= 7.0, "the double blend cost {over} px");
        assert!(over >= 1.0, "the overlap vanished; the measurement is wrong");
        // **The alternative, measured rather than asserted.** Shorten
        // each arm by t/2 so nothing overlaps and the disc is tangent
        // to both caps — and the crescent between a flat cap and the
        // circle it touches at ONE POINT is left empty, at every angle,
        // turn or no turn. A hole is worse than a hot spot.
        let clipped: Vec<(Frame, [f32; 2])> = [(pts[0], pts[1]), (pts[1], pts[2])]
            .iter()
            .enumerate()
            .map(|(i, (a, b))| {
                let (f, len) = Frame::along(*a, *b).unwrap();
                let cut = t * 0.5;
                let mid = if i == 0 { -cut * 0.5 } else { cut * 0.5 };
                (
                    Frame { centre: f.to_screen([mid, 0.0]), ..f },
                    [(len - cut) * 0.5, t * 0.5],
                )
            })
            .collect();
        let mut worst_gap = 0.0f32;
        for py in 10..80 {
            for px in 10..110 {
                let p = [px as f32 + 0.5, py as f32 + 0.5];
                let mut acc = 0.0f32;
                for (f, half) in clipped.iter().chain(std::iter::once(&disc)) {
                    let round = std::ptr::eq(f, &disc.0);
                    acc += cov(f, *half, round, p) * (1.0 - acc);
                }
                worst_gap = worst_gap.max(truth(p) - acc);
            }
        }
        assert!(worst_gap >= 0.3, "the clipped arms left only {worst_gap}");
    }

    /// The three brakes on the split, each for its own reason (§7b):
    /// a ride, because the field's screen gradient is no longer one; a
    /// kind past Box, because the shader draws every record as its box
    /// distance TODAY and will not tomorrow; and a core too small to
    /// pay for the four strips around it.
    #[test]
    fn a_ride_a_foreign_kind_and_a_small_core_keep_the_whole_quad() {
        use crate::draw::{ShapeKind, ShapeSpec};
        let r = Rect::new(0.0, 0.0, 200.0, 100.0);
        let c = [Corner::round(8.0); 4];
        let spec = |kind, rect| ShapeSpec {
            rect,
            corners: c,
            kind,
            fill: Some(bed()),
            stroke: None,
            glass: None,
            soft: None,
        };
        let emit = |warp: u8, kind, rect| {
            let mut dl = DrawList::new();
            dl.set_warp(warp);
            dl.shape(&spec(kind, rect));
            dl.verts.len()
        };
        assert_eq!(emit(1, ShapeKind::Box, r), 30, "the frame");
        assert_eq!(emit(3, ShapeKind::Box, r), 54, "a ride keeps whole quads");
        // A foreign silhouette, and one whose OWN numbers would let the
        // cut through if the kind did not stop it. A hexagon's apothem
        // is large enough that `CORE_MIN` refuses the split anyway, so
        // it proves nothing on its own: a chevron 20 px deep leaves a
        // core of 77×27 px, far past the minimum, and its interior is
        // NOT the rect's — the plain-fill path would paint the corners
        // the collapse cut away, at full alpha. That is the picture the
        // guard exists to stop, and this is the case that shows it.
        assert_eq!(emit(1, ShapeKind::Hex { turn: 0.0 }, r), 6, "a foreign silhouette");
        assert_eq!(
            emit(1, ShapeKind::Chevron { left: 20.0, right: 20.0 }, r),
            6,
            "a chevron split its core: the rect's corners will fill solid"
        );
        // A core of 4×24 px is 96 px² — under the 256 the four strips
        // have to earn — so this one stays one quad.
        assert_eq!(emit(1, ShapeKind::Box, Rect::new(0.0, 0.0, 44.0, 30.0)), 6, "too small");
    }

    /// **K3c, measurement 2: what the split buys, silhouette by
    /// silhouette — and where it stops buying anything.**
    ///
    /// `the_field_stops_paying_for_the_interior` above proves the cut
    /// happens and bounds it loosely; this one states the exact fraction
    /// on the four shapes the interface is actually made of, because the
    /// bound and the fraction lead to opposite conclusions and only the
    /// fraction can be held up against the instruction counts in
    /// `nacelle-renderer/src/spirvstat.rs`.
    ///
    /// **The finding, and it is not the one §7b expected.** The saving
    /// is a property of the PERIMETER-TO-AREA ratio, so it collapses on
    /// small controls — and small controls are most of a screen. A
    /// 315x175 panel keeps 19 % of its pixels on the field; a 120x34
    /// button keeps 71 % and a 132x9 list row keeps 74 %, because the
    /// band is `corner + stroke + AA_PAD + CORE_PAD` deep whatever the
    /// control — 10.5 px on the panel, 3 px on the bare row — and a
    /// nine-pixel row has one and a half pixels of interior to give
    /// back. The window frame is the other extreme and the only
    /// unambiguous win: with no bed under it the interior is not
    /// rasterised AT ALL.
    ///
    /// Every case is also checked pixel for pixel against the whole
    /// quad it replaces, on three destinations. The cases here are
    /// SMALLER than the ones above on purpose: a short control is where
    /// `core_half` has least room and where an off-by-one in the band
    /// would first show.
    #[test]
    fn the_split_buys_less_the_smaller_the_control_is() {
        /// name, rect, corner radius (0 = square), has a bed, border,
        /// the padded area, the area still on the field.
        type Case<'a> = (&'a str, Rect, f32, bool, Option<f32>, f32, f32);
        let cases: [Case; 4] = [
            // §1.1's reference panel.
            ("a panel", Rect::new(0.0, 0.0, 315.0, 175.0), 6.5, true, Some(1.0), 56_109.0, 10_833.0),
            // §7b risk 1: a border over a whole window, no bed.
            ("a window frame", Rect::new(0.0, 0.0, 1200.0, 800.0), 6.0, false, Some(1.0), 964_004.0, 43_604.0),
            // The case that decides K3d, and the one nobody measured.
            ("a button", Rect::new(0.0, 0.0, 120.0, 34.0), 6.5, true, Some(1.0), 4_392.0, 3_105.0),
            // A list row: square, no border, and there are hundreds.
            ("a list row", Rect::new(0.0, 0.0, 132.0, 9.0), 0.0, true, None, 1_474.0, 1_096.0),
        ];
        let area = |q: &[crate::draw::Vertex]| {
            ((q[2].pos[0] - q[0].pos[0]) * (q[2].pos[1] - q[0].pos[1])).abs()
        };
        for (name, r, k, fill, stroke, padded, want_field) in cases {
            let c = if k == 0.0 { [Corner::SQUARE; 4] } else { [Corner::round(k); 4] };
            let split = surface(r, &c, fill, stroke, 1);
            let whole = surface(r, &c, fill, stroke, 2);
            let mut field = 0.0f32;
            let mut plain = 0.0f32;
            for q in split.verts.chunks_exact(6) {
                *if q[0].shape == NO_SHAPE { &mut plain } else { &mut field } += area(q);
            }
            assert_eq!(
                (r.w + 2.0) * (r.h + 2.0),
                padded,
                "{name}: the padded quad is not the area this case was measured at"
            );
            assert!(
                (field - want_field).abs() <= 0.5,
                "{name}: the field pays for {field} px, measured {want_field}"
            );
            if fill {
                assert!(
                    (field + plain - padded).abs() <= 0.5,
                    "{name}: the frame and the core do not partition the quad"
                );
            } else {
                // The one case where the two do not add up, and the
                // reason it is the biggest win on the list: with no bed
                // the interior is not a transparent fragment, it is not
                // a fragment.
                assert_eq!(plain, 0.0, "{name}: a bed appeared under a bare border");
            }
            // The picture is the same picture, or the fraction above is
            // a saving on the wrong image.
            for py in (r.y as i32 - 2)..(r.y + r.h) as i32 + 2 {
                for px in (r.x as i32 - 2)..(r.x + r.w) as i32 + 2 {
                    let p = [px as f32 + 0.5, py as f32 + 0.5];
                    for dst in [[0.0; 3], [1.0, 0.5, 0.25], [1.0; 3]] {
                        assert_eq!(
                            blend(&split, p, dst),
                            blend(&whole, p, dst),
                            "{name}: pixel {p:?} over {dst:?}"
                        );
                    }
                }
            }
        }
    }

    /// **K3c, measurement 3, the toolkit's half: what the split does to
    /// the RUN count.**
    ///
    /// §7b priced the cut at "two runs more per shape, MEASURED"; this
    /// is that measurement carried to a screenful, where it stops being
    /// a per-shape overhead and becomes the dominant fact about the
    /// lane. A run is a pipeline bind and a draw call, and the cut
    /// breaks the merge a row of plain shapes used to get for free.
    ///
    /// The scene is twelve panels of sixteen list rows each — 204
    /// silhouettes, the shape of a real desktop board. The tessellated
    /// lane draws it in ONE run. The vector lane with the split draws it
    /// in 408: two per shape, forever, because the core samples the
    /// atlas and the strips read the shape buffer, so no two adjacent
    /// shapes can merge. The same lane with the split suppressed draws
    /// it in one again — which is what identifies the runs as the CUT's
    /// price and not the lane's.
    ///
    /// §7b's own remedy 3 (merging shape runs host-side) is the answer
    /// and is not written. Until it is, this number is the strongest
    /// argument against K3d.
    #[test]
    fn the_cut_turns_one_run_into_two_per_shape() {
        let board = |vector: bool, warp: u8| {
            let mut dl = DrawList::new();
            dl.set_vector(vector);
            dl.set_warp(warp);
            let c = [Corner::round(6.5); 4];
            for i in 0..12 {
                let r = Rect::new(10.0 + 150.0 * i as f32, 40.0, 140.0, 175.0);
                dl.ring_fill(r, &c, 16, bed());
                dl.ring(r, &c, 16, 1.0, edge());
                for j in 0..16 {
                    let row = Rect::new(r.x + 4.0, r.y + 6.0 + 10.0 * j as f32, 132.0, 9.0);
                    dl.ring_fill(row, &[Corner::SQUARE; 4], 4, bed());
                }
            }
            dl
        };

        let old = board(false, 1);
        assert_eq!(old.shape_len(), 0, "the tessellated lane writes no records");
        assert_eq!(old.verts.len(), 8_496);
        assert_eq!(old.runs.len(), 1, "one texture, one scissor, one draw call");

        let cut = board(true, 1);
        assert_eq!(cut.shape_len(), 204);
        assert_eq!(cut.verts.len(), 6_120, "the vector lane spends 1.39x fewer vertices");
        assert_eq!(
            cut.runs.len(),
            408,
            "the cut costs two runs a shape and there is no host-side merge yet"
        );

        // The control: the same lane, the same records, the split held
        // back by the ride brake. One run again — so the 408 above is
        // the price of the CUT, not of the field.
        let uncut = board(true, 2);
        assert_eq!(uncut.shape_len(), 204, "the same silhouettes");
        assert_eq!(uncut.runs.len(), 1);
        // What the lane costs in vertices when the cut is not taken at
        // all: 204 shapes, one quad each. The ride above quadruples
        // that by its own grid, so the number is stated where the grid
        // is one — it is the figure the vertex half of the trade is
        // argued from.
        assert_eq!(board(true, 2).verts.len(), 4_896, "the ride's 2x2 grid");
    }

    // ---- Glass on the band (f3 §3.3, K3b) ----------------------------
    //
    // A frosted surface is three layers over one silhouette, and before
    // K3b they were three draws: the blurred scene times the tint, the
    // wash over it, the border over both. What follows shades both
    // variants of the new one — the frame of quads and the whole quad
    // it replaces — out of this file's own functions, and states two
    // things a picture cannot state about itself: that the split did
    // not move the picture, and that folding the layers into one
    // fragment removed exactly the excess the old order left on the
    // shared edge.

    /// The blurred scene, as a fragment sees it: a function of SCREEN
    /// position and nothing else — which is the whole contract of the
    /// glass lane (`shaders.rs`: `pos.xy / pc.screen`), and the reason
    /// a frosted quad may ride any animation without the frost sliding
    /// under it. Anything smooth and non-constant does; this one varies
    /// in both axes so a fragment reading the wrong position would show.
    fn blurred(p: [f32; 2]) -> [f32; 4] {
        let f = |x: f32, k: f32| (x * k).sin() * 0.5 + 0.5;
        [f(p[0], 0.11), f(p[1], 0.07), f(p[0] + p[1], 0.05), 1.0]
    }

    /// The DISPLAY TRANSFORM every fragment of the renderer ends with:
    /// `shaders.rs: grade()`, a colour LUT the user may load (the
    /// desktop's `ColorLut` setting), applied to rgb and never to
    /// alpha. Nothing below depends on WHICH curve it is — smoothstep
    /// is here because it is monotone, bounded and unmistakably
    /// non-linear — and non-linearity is the whole of what matters: it
    /// is what makes the PLACE a transform is applied observable in the
    /// picture. Identity is the shipped default and the case in which
    /// the question cannot be asked, which is why the proofs run both.
    fn graded(c: [f32; 4], lut: bool) -> [f32; 4] {
        if !lut {
            return c;
        }
        let f = |x: f32| {
            let x = x.clamp(0.0, 1.0);
            x * x * (3.0 - 2.0 * x)
        };
        [f(c[0]), f(c[1]), f(c[2]), c[3]]
    }

    /// One `fs_shape_glass` fragment, out of this file's functions — the
    /// twin of [`fs_shape`] with `glass_base` where its fill was.
    ///
    /// The grade goes on the three LAYERS — the frost and the wash
    /// inside `glass_base`, the stroke on its way into [`compose`] —
    /// and not on the composite, because the core of the same surface
    /// is two ordinary draws the hardware blends after grading each.
    /// `fs_shape` may and does grade its composite: its fill is ONE
    /// layer, so there is nothing there for a fold to reassociate.
    fn fs_shape_glass(
        rec: &Shape,
        at: [f32; 2],
        local: [f32; 2],
        colour: [f32; 4],
        lut: bool,
    ) -> [f32; 4] {
        let d = d_shape(local, rec.half, &record_corners(rec));
        let has = |bit: u32| f32::from(rec.flags & bit != 0);
        let wash = [colour[0], colour[1], colour[2], colour[3] * has(Shape::FILL)];
        compose(
            glass_base(blurred(at), rec.tint, wash, |c| graded(c, lut)),
            graded(rec.stroke_c, lut),
            coverage(d, 1.0),
            band_coverage(d, rec.stroke, 1.0) * has(Shape::STROKE),
        )
    }

    /// [`frags`] with the lanes told apart, which a frosted surface
    /// needs and an ordinary one never did: a quad's RUN says which
    /// fragment shades it. `GLASS_RANK_*` is `fs_blur` — the blurred
    /// sample times the vertex colour, by screen position, uv ignored —
    /// `SHAPE_GLASS_*` is the fragment above, and everything else is
    /// what [`frags`] already knew.
    ///
    /// `lut` arms the display transform in all four of them, each where
    /// its own fragment applies it: `fs_blur` and `fs_main` grade what
    /// they return, `fs_shape` grades its composite, and the frosted
    /// fragment grades its layers.
    fn frags_glass(dl: &DrawList, p: [f32; 2], lut: bool) -> Vec<[f32; 4]> {
        let mut out = Vec::new();
        let mut start = 0usize;
        for run in &dl.runs {
            let end = run.end as usize;
            for q in dl.verts[start..end].chunks_exact(6) {
                let (a, b) = (q[0].pos, q[2].pos);
                let inside = |k: usize| p[k] >= a[k].min(b[k]) && p[k] < a[k].max(b[k]);
                if !inside(0) || !inside(1) {
                    continue;
                }
                out.push(match run.image {
                    Some(img) if is_glass_rank(img) => {
                        let g = blurred(p);
                        let c = q[0].color;
                        graded([g[0] * c[0], g[1] * c[1], g[2] * c[2], g[3] * c[3]], lut)
                    }
                    _ if q[0].shape == crate::draw::NO_SHAPE => graded(q[0].color, lut),
                    Some(img) if is_shape_glass(img) => {
                        let rec = &dl.shapes()[q[0].shape as usize];
                        let c = [q[0].pos[0] - q[0].uv[0], q[0].pos[1] - q[0].uv[1]];
                        fs_shape_glass(rec, p, [p[0] - c[0], p[1] - c[1]], q[0].color, lut)
                    }
                    _ => {
                        let rec = &dl.shapes()[q[0].shape as usize];
                        let c = [q[0].pos[0] - q[0].uv[0], q[0].pos[1] - q[0].uv[1]];
                        graded(fs_shape(rec, [p[0] - c[0], p[1] - c[1]], q[0].color), lut)
                    }
                });
            }
            start = end;
        }
        out
    }

    fn is_glass_rank(img: crate::draw::ImageId) -> bool {
        use crate::draw::{GLASS_RANK_1, GLASS_RANK_2, GLASS_RANK_3};
        img == GLASS_RANK_1 || img == GLASS_RANK_2 || img == GLASS_RANK_3
    }

    fn is_shape_glass(img: crate::draw::ImageId) -> bool {
        use crate::draw::{SHAPE_GLASS_1, SHAPE_GLASS_2, SHAPE_GLASS_3};
        img == SHAPE_GLASS_1 || img == SHAPE_GLASS_2 || img == SHAPE_GLASS_3
    }

    fn blend_glass(dl: &DrawList, p: [f32; 2], dst: [f32; 3], lut: bool) -> [f32; 3] {
        let mut d = dst;
        for f in frags_glass(dl, p, lut) {
            for k in 0..3 {
                d[k] = f[k] * f[3] + d[k] * (1.0 - f[3]);
            }
        }
        d
    }

    fn tint() -> Color {
        Color::rgba8(120, 150, 200, 150)
    }

    /// A frosted surface, spelled the way `window::frame` and
    /// `elev::Level::draw` spell one. `warp` is the control, exactly as
    /// in [`surface`]: at 2 the split stays out of the way and the
    /// whole silhouette rasterises through the field, which is the
    /// geometry the frame of quads replaces.
    fn frosted(r: Rect, c: &[Corner; 4], stroke: Option<f32>, warp: u8) -> DrawList {
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.set_warp(warp);
        dl.glass_fill(r, c, 16, 2.0, tint());
        dl.ring_fill(r, c, 16, bed());
        if let Some(t) = stroke {
            dl.ring(r, c, 16, t, edge());
        }
        dl
    }

    /// **The proof K3b rests on.** The frame of quads — the frost's
    /// core on the tessellated glass lane, the wash's core over it, and
    /// four analytic strips around both — paints what the single
    /// frosted quad paints, over every pixel of the shape and its
    /// margin, over three destinations.
    ///
    /// The tolerance is not slack: the two variants compute the SAME
    /// composite by different associations. Inside the core the split
    /// blends the frost and then the wash, the whole quad blends
    /// `over(wash, frost)` once, and `over` divides by an alpha it then
    /// multiplies back — which is exact in arithmetic and one ulp off
    /// in binary. 1e-6 is a thousand times under a step of an 8-bit
    /// channel.
    ///
    /// It runs with the display transform OFF and ON, and the second
    /// pass is the one that states something the first cannot. Off, the
    /// question "where is the grade applied" has no observable answer;
    /// on, the core is two graded draws the hardware blends and the
    /// band is one fragment, so the band agrees with its own interior
    /// only if it grades the layers rather than their fold. That
    /// difference is a RECTANGLE inside a frosted panel, on the line
    /// where the core's cut falls, and it is the seam K3b would have
    /// introduced (§3.3, `glass_base`'s `display`).
    #[test]
    fn the_frosted_frame_paints_what_the_whole_frosted_quad_painted() {
        let mix = mixed_corners();
        let cases: [(&str, Rect, &[Corner; 4], Option<f32>); 4] = [
            ("a bare frost", Rect::new(12.0, 20.0, 200.0, 100.0), &mix, None),
            ("frost and border", Rect::new(12.0, 20.0, 200.0, 100.0), &mix, Some(2.0)),
            ("square corners", Rect::new(0.0, 0.0, 90.0, 60.0), &[Corner::SQUARE; 4], Some(1.0)),
            ("a deep chamfer", Rect::new(5.0, 7.0, 150.0, 90.0), &[Corner::chamfer(20.0); 4], Some(2.0)),
        ];
        for (name, r, c, stroke) in cases {
            let split = frosted(r, c, stroke, 1);
            let whole = frosted(r, c, stroke, 2);
            assert_eq!(split.shapes(), whole.shapes(), "{name}: not the same record");
            assert_eq!(split.shape_len(), 1, "{name}: not one record");
            // The frost's core, the wash's core, four strips.
            assert_eq!(split.verts.len(), 36, "{name}: the split did not happen");
            let mut lit = 0u32;
            for py in (r.y as i32 - 3)..(r.y + r.h) as i32 + 3 {
                for px in (r.x as i32 - 3)..(r.x + r.w) as i32 + 3 {
                    let p = [px as f32 + 0.5, py as f32 + 0.5];
                    for dst in [[0.0; 3], [1.0, 0.5, 0.25], [1.0; 3]] {
                        for lut in [false, true] {
                            let a = blend_glass(&split, p, dst, lut);
                            let b = blend_glass(&whole, p, dst, lut);
                            for k in 0..3 {
                                assert!(
                                    (a[k] - b[k]).abs() <= 1e-6,
                                    "{name}: pixel {p:?} over {dst:?}, lut {lut}: {a:?} vs {b:?}"
                                );
                            }
                        }
                    }
                    // Two layers over the core, one fragment over the
                    // band, none outside: the frost and the wash are the
                    // only pair on this lane that may cover a pixel
                    // twice, and they are two DIFFERENT layers.
                    let n = frags_glass(&split, p, false).len();
                    assert!(n <= 2, "{name}: pixel {p:?} covered {n} times");
                    lit += n as u32;
                }
            }
            assert!(lit > 0, "{name}: nothing was drawn at all");
        }
    }

    /// **What the fold bought, as a number.** On the shared silhouette
    /// the old order — frost blended, then wash, then border — leaves
    /// more alpha than the surface covers, and the excess is exactly
    /// `c·(1 − c)·a·b`: R4 by another name, and the reason §3.3 asked
    /// for one fragment. The composed lane leaves the ideal instead.
    ///
    /// The ideal is not an opinion: a pixel `c` covered by a surface
    /// whose colour is `S` over the destination `d` is `c·S + (1 − c)·d`
    /// — one lerp, the definition of coverage.
    #[test]
    fn three_layers_blend_once_and_the_old_order_left_a_rim() {
        let (a, b) = (tint().a, bed().a);
        let dst = [0.15f32, 0.2, 0.25];
        let mut worst = 0.0f32;
        for i in 0..=20 {
            let cov = i as f32 / 20.0;
            let g = blurred([7.5, 11.5]);
            let frost = [g[0] * tint().r, g[1] * tint().g, g[2] * tint().b, g[3] * a];
            let surface = over(bed().to_array(), frost);
            // The lane: one coverage over the composed surface.
            let one = compose(surface, [0.0; 4], cov, 0.0);
            // The old order: two draws, each with its own coverage.
            let mut old = dst;
            for f in [
                [frost[0], frost[1], frost[2], frost[3] * cov],
                [bed().r, bed().g, bed().b, b * cov],
            ] {
                for k in 0..3 {
                    old[k] = f[k] * f[3] + old[k] * (1.0 - f[3]);
                }
            }
            // THE IDEAL, derived from neither of them: `cov` of the
            // pixel is wash-over-frost and the rest is the wall, with
            // both layers written out premultiplied rather than taken
            // from [`over`] — a referee that shares a function with a
            // player is no referee.
            let sa = b + (1.0 - b) * frost[3];
            let ideal: Vec<f32> = (0..3)
                .map(|k| {
                    let s = bed().to_array()[k] * b + (1.0 - b) * frost[k] * frost[3];
                    cov * s + (1.0 - cov * sa) * dst[k]
                })
                .collect();
            for k in 0..3 {
                let lane = one[k] * one[3] + dst[k] * (1.0 - one[3]);
                assert!((lane - ideal[k]).abs() <= 1e-6, "cov {cov}: the lane missed the ideal");
                worst = worst.max((old[k] - ideal[k]).abs());
            }
            // The excess alpha the pair leaves, stated in closed form.
            let pair_a = cov * b + (1.0 - cov * b) * (cov * a);
            assert!(
                (pair_a - (cov * surface[3] + cov * (1.0 - cov) * a * b)).abs() <= 1e-6,
                "cov {cov}: the excess is not c(1-c)ab"
            );
        }
        // …and it is not a rounding error: at these alphas the old
        // order is off by more than a step of an 8-bit channel.
        assert!(worst > 1.0 / 255.0, "the pair was already right, off by {worst}");
    }

    /// **Where the display transform goes, as a number.** The fold of
    /// §3.3 buys one coverage for three layers, and it costs one thing:
    /// the band composes on the CPU what the core still composes in the
    /// blender, so the two agree only while every per-fragment
    /// transform is applied to the same operands on both sides.
    ///
    /// `grade()` is that transform, it is the last thing every fragment
    /// of the renderer does, and it is the identity until a user loads
    /// a colour LUT — so this is a defect that ships looking correct
    /// and appears the day somebody uses a feature that has nothing to
    /// do with glass. OVER is associative, so grading each LAYER
    /// survives the fold exactly; grading the FOLD does not, and the
    /// gap is the seam. Both are asserted here: the first to 1e-6, the
    /// second as a difference no eye needs help to find.
    #[test]
    fn the_display_transform_goes_on_the_layers_and_not_on_their_fold() {
        let wash = bed().to_array();
        let t = tint().to_array();
        let dst = [0.15f32, 0.2, 0.25];
        let mut worst = 0.0f32;
        for i in 0..=20 {
            let blur = blurred([i as f32 * 7.0, i as f32 * 3.0]);
            let frost = [blur[0] * t[0], blur[1] * t[1], blur[2] * t[2], blur[3] * t[3]];
            // The CORE of a frosted surface, and the ground truth: two
            // ordinary draws, each graded on its way out of its own
            // fragment, blended by the hardware.
            let mut core = dst;
            for f in [graded(frost, true), graded(wash, true)] {
                for k in 0..3 {
                    core[k] = f[k] * f[3] + core[k] * (1.0 - f[3]);
                }
            }
            // The BAND, both ways round.
            let layers = glass_base(blur, t, wash, |c| graded(c, true));
            let fold = graded(glass_base(blur, t, wash, |c| c), true);
            for k in 0..3 {
                let a = layers[k] * layers[3] + dst[k] * (1.0 - layers[3]);
                let b = fold[k] * fold[3] + dst[k] * (1.0 - fold[3]);
                assert!((a - core[k]).abs() <= 1e-6, "the band left its own core: {a} vs {core:?}");
                worst = worst.max((b - core[k]).abs());
            }
        }
        assert!(worst > 1.0 / 255.0, "the grade commuted after all, off by {worst}");
    }
}
