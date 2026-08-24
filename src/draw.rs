//! Draw list — everything as triangles. Most of them sample the glyph
//! atlas (text by its glyphs, solid shapes by the atlas's white
//! pixel); a run may instead sample an application-registered image.
//!
//! Beside the triangles the list can keep a REGISTER of what it was
//! asked to draw — [`DrawCmd`], one entry per public call, armed by
//! `NACELLE_DRAW_CMDS` and off in every other run. Triangles answer
//! "did the geometry change"; the register answers "did the scene
//! change", and a change to the drawing pipeline is only provable with
//! both: one of the two is what the commit is allowed to move.

use crate::base::Rect;
use crate::font::{Figures, FontSystem, Glyph};
use crate::sdf::{thin_band, Frame};
use crate::theme::Color;
use std::fmt;
use std::sync::atomic::{AtomicU8, Ordering};

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Vertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    /// Index into the frame's [`Shape`] records, or [`NO_SHAPE`] for
    /// every vertex that belongs to no shape — glyphs, images, glass,
    /// the whole tessellated path (f3 D3). For a shape run the `uv`
    /// slot carries the LOCAL position in px from the record's centre;
    /// nothing else changes hands, which is why the vertex grows by
    /// exactly these four bytes (32 → 36) and not by the record.
    pub shape: u32,
}

/// The `shape` a vertex outside any shape carries. All ones rather than
/// zero so that a record index of 0 stays a real index and a forgotten
/// field reads as an out-of-range one, never as "the first shape".
pub const NO_SHAPE: u32 = u32::MAX;

/// A handle to pixels the RENDERER owns. The list only records which
/// image a run samples; registering the pixels, uploading them and
/// mapping the handle to a texture is the renderer's job — the same
/// split as with everything else here: the toolkit describes, the
/// application draws.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct ImageId(pub u32);

/// The reserved image handle for frosted glass: a run tagged with it
/// samples a BLURRED copy of everything drawn before the first such
/// run this frame, by screen position. The renderer owns the blurring;
/// the list only marks where the glass lies.
pub const BLUR_IMAGE: ImageId = ImageId(u32::MAX);

/// The reserved handle band (r1). Everything at or above RESERVED_IMAGE_MIN
/// is a renderer instruction, not a texture: the glass ranks, the additive
/// pipeline, and BLUR_IMAGE itself (which aliases rank 2 — exactly what the
/// renderer drew for it before ranks existed). `create_texture` must never
/// hand these out.
pub const RESERVED_IMAGE_MIN: ImageId = ImageId(u32::MAX - 15);
/// Glass at pyramid rank 1..3: lightest to deepest blur. A rank the frame's
/// blur depth did not write resolves to the deepest one that exists.
pub const GLASS_RANK_1: ImageId = ImageId(u32::MAX - 1);
pub const GLASS_RANK_2: ImageId = ImageId(u32::MAX - 2);
pub const GLASS_RANK_3: ImageId = ImageId(u32::MAX - 3);
/// Additive blending over the glyph atlas: the run renders through fs_main
/// with SRC_ALPHA/ONE colour, ZERO/ONE alpha — glow and bloom compose with
/// light instead of milk (Appendix B, R1).
pub const ADD_ATLAS: ImageId = ImageId(u32::MAX - 8);
/// The vector core's lane (f3 §2.9): a run tagged SHAPE draws through
/// the renderer's `fs_shape` — every vertex carries the index of one
/// [`Shape`] record and its `uv` is the local position in px from that
/// record's centre. Normal blend, and nothing sampled: the record IS
/// the picture. Its additive twin is [`SHAPE_ADD`].
pub const SHAPE: ImageId = ImageId(u32::MAX - 4);
/// The vector core's ADDITIVE lane (f3 §2.6): the same `fs_shape` over
/// the same records, under the blend [`ADD_ATLAS`] already carries —
/// SRC_ALPHA/ONE colour, ZERO/ONE alpha.
///
/// It is a second HANDLE and not a second flag because the difference
/// is blend state, and blend state is fixed for a whole pipeline before
/// the first fragment of a run is shaded: no bit a fragment could read
/// can decide whether its answer is added to the target or laid over
/// it. The record says which silhouette and how soft; the run says
/// whether that is light or cover. A glow takes this lane, a shadow the
/// plain one, and the fragment is the same code in both.
pub const SHAPE_ADD: ImageId = ImageId(u32::MAX - 9);
/// The vector core's GLASS lanes (f3 §3.3, K3b): a run tagged with one
/// of these draws through `fs_shape_glass` — the same record, the same
/// analytic silhouette, and one sample of the pyramid rank the HANDLE
/// names, composed with the record's tint and the vertex's wash in a
/// single fragment under a single coverage.
///
/// The rank rides in the handle rather than in the record because the
/// blurred target is a DESCRIPTOR: the renderer binds one per run, so
/// the rank is already a property of the run and a field would only
/// have said it twice (the question left open at [`DrawList::glow_ring`],
/// answered here). Three handles, three rungs of the pyramid, in step
/// with [`GLASS_RANK_1`]`..3` — a frosted surface's core still rides
/// those, and only its perimeter band comes here.
pub const SHAPE_GLASS_1: ImageId = ImageId(u32::MAX - 5);
pub const SHAPE_GLASS_2: ImageId = ImageId(u32::MAX - 6);
pub const SHAPE_GLASS_3: ImageId = ImageId(u32::MAX - 7);

/// The shape-lane handle that serves pyramid rank `rank` (1..=3), the
/// twin of [`glass_rank_handle`]. Out-of-range ranks resolve to the
/// deepest, as the renderer's own clamp does.
pub fn shape_glass_handle(rank: u8) -> ImageId {
    match rank {
        1 => SHAPE_GLASS_1,
        2 => SHAPE_GLASS_2,
        _ => SHAPE_GLASS_3,
    }
}

/// The tessellated glass handle for rank `rank` (1..=3) — the core of a
/// frosted surface, and every frosted surface drawn off the vector lane.
pub fn glass_rank_handle(rank: u8) -> ImageId {
    match rank {
        1 => GLASS_RANK_1,
        2 => GLASS_RANK_2,
        _ => GLASS_RANK_3,
    }
}

/// Whether a run's handle is one of the three `SHAPE_GLASS_*` — the
/// band's own lane, and the question [`DrawList::ring_grad`] asks of an
/// open weld before it will land a gradient on it (K3b's second bounded
/// edge case). `Frost` claims no flag on the record itself (§3.3's own
/// note at [`Shape::KIND_SHIFT`]), so the RUN is the only place this can
/// be read.
pub(crate) fn is_shape_glass(img: ImageId) -> bool {
    img == SHAPE_GLASS_1 || img == SHAPE_GLASS_2 || img == SHAPE_GLASS_3
}

/// Whether a handle is one of the reserved instructions rather than a
/// registered texture.
pub fn is_reserved(id: ImageId) -> bool {
    id.0 >= RESERVED_IMAGE_MIN.0
}

/// What the fragment shader reads to compute one shape (f3 §2.5).
/// std430, 80 B; the index into the frame's array rides in
/// [`Vertex::shape`]. The fill colour is NOT here: it stays on the
/// vertex, like every fill before it, so a dot matrix of one geometry
/// is one record however many colours it wears (f3 §3.4).
///
/// **Why it is 80 and not the 64 §2.5 pinned.** K3b (§3.3) put frosted
/// glass on this lane, and a frosted band composes THREE colours in one
/// fragment: the tint that multiplies the blurred scene, the wash the
/// vertex carries, and the border in `stroke_c`. Two of the three were
/// already here; the third had nowhere to sit. The record has three
/// spare floats — `arc_half`, `arc_dir`, `_pad` — and a fourth in
/// `feather`, and a glass Box uses none of them, so the tint COULD have
/// been read out of that hole. It is not, for one reason: the arc pair
/// belongs to `Ring` and `feather` to the soft profiles, both under way
/// on other branches, and a union of two live features in one memory
/// area is a defect the compiler cannot see and a merge cannot report.
/// A named field costs 16 B on records that never read it and buys a
/// meaning that survives being merged with work nobody has written yet.
/// The array stride stays a multiple of 16, which is all std430 asks.
#[repr(C)]
#[derive(Clone, Copy, Default, Debug, PartialEq)]
pub struct Shape {
    /// Half sizes, local px.
    pub half: [f32; 2],
    /// Stroke band width, INWARD from the boundary (the project's
    /// convention — see [`DrawList::ring`]); 0 = no band.
    pub stroke: f32,
    /// Softness: where the gaussian profile reaches zero, px past the
    /// boundary; 0 = crisp. Read only with [`Shape::GAUSS`], and it is
    /// the ONE number the band and the quad have to be told about
    /// before they are cut — see [`Soft`] and `shape_verts`' envelope.
    pub feather: f32,
    /// The kind's LENGTHS, local px, sentinels already resolved on the
    /// CPU (R9): never negative here. Box spends all four on its corner
    /// sizes tl, tr, br, bl ([`ring_points`]' order); the kinds K6 added
    /// spend the first one or two on their own — see the table on
    /// [`ShapeKind`]. A slot no kind reads is zero, so two records of
    /// one silhouette compare equal whatever the caller passed.
    pub corner: [f32; 4],
    /// The stroke band's colour; the fill colour is the vertex's.
    pub stroke_c: [f32; 4],
    /// Bits 0-7: [`CornerStyle`] × 4 (tl, tr, br, bl), 2 bits each —
    /// Box's, and zero under every other kind. Bits 8-11:
    /// [`ShapeKind::code`]. Bit 12 [`Shape::FILL`], bit 13
    /// [`Shape::STROKE`], bit 14 [`Shape::OUTSIDE_ONLY`], bit 15
    /// [`Shape::GAUSS`]. Bits 16-31 are unclaimed and MUST be zero; 16
    /// is spoken for by §2.5's table (the kind modifier).
    ///
    /// **There is no GLASS bit, and for one day there was.** A frost is
    /// told apart by the two things that actually decide the picture:
    /// the RUN's handle — `SHAPE_GLASS_*`, which is what binds a
    /// blurred target and picks the fragment — and `tint`, which is
    /// zero on every other record and is the frost's own identity in
    /// `over` when it is. A bit that no shader reads and no branch
    /// turns is a bit spent, and this record's bits have claimants
    /// waiting.
    pub flags: u32,
    /// The kind's first ANGLE, radians: half the arc's sweep for a Ring
    /// (>= PI = a closed ring), unread by every other kind. Lengths ride
    /// [`Shape::corner`]; the split is what keeps one field from meaning
    /// a px on one record and a radian on the next.
    pub arc_half: f32,
    /// The kind's second angle, radians: the direction of a Ring's
    /// middle, the lattice turn of a Hex. Both are measured from local
    /// +y — DOWNWARD on screen — turning toward **−x**, which with y
    /// down is CLOCKWISE on the glass: 6 o'clock toward 9 o'clock. That
    /// is the one sense an angle has anywhere in this project, from
    /// `donut.start_deg` to a knob's travel, and it is the sense
    /// [`crate::sdf::turned`] carries — a positive `arc_dir` sends the
    /// silhouette's own +y point to `(−R·sin dir, R·cos dir)`.
    /// `sdf::the_turn_runs_clockwise_on_the_glass` is the assertion.
    pub arc_dir: f32,
    pub _pad: f32,
    /// What the blurred scene behind a FROSTED band is multiplied by
    /// before the wash lies over it (§3.3) — the same product `fs_blur`
    /// draws for the surface's core, so band and core frost alike.
    ///
    /// ZERO on every record that is not a frost, and that is arithmetic
    /// rather than convention: `over(wash, blur × 0)` IS the wash, so a
    /// plain record read by the frosted fragment draws exactly what the
    /// plain fragment would have drawn. That identity is why the lane
    /// needs no flag to gate it.
    pub tint: [f32; 4],
}
const _: () = assert!(std::mem::size_of::<Shape>() == 80);
const _: () = assert!(std::mem::align_of::<Shape>() == 4);

impl Shape {
    /// Bit 12: draw the interior with the vertex colour.
    pub const FILL: u32 = 1 << 12;
    /// Bit 13: draw the inward stroke band with `stroke_c`.
    pub const STROKE: u32 = 1 << 13;
    /// Bit 14: the coverage is zero INSIDE the silhouette — a glow
    /// lights what is around a shape and must not tint a translucent
    /// fill through it (the rule the tessellated glow got by emitting
    /// no geometry inside its path).
    ///
    /// It is not the crisp cut §2.5 wrote. What multiplies the profile
    /// is the area of the pixel the silhouette does NOT cover,
    /// `clamp(0.5 + d/w)` — geometry, evaluated once, and exactly 1 as
    /// soon as the fragment is half a pixel clear of the boundary. A
    /// step function there would put a stair on the one edge this whole
    /// lane exists to smooth, and would leave a seam against the panel
    /// standing on it: the panel's own AA gives that boundary pixel a
    /// half, and the glow under it has to give the other half.
    pub const OUTSIDE_ONLY: u32 = 1 << 14;
    /// Bit 15: the softness profile of §2.6 instead of the crisp
    /// coverage ramp — [`crate::sdf::soft_profile`], which is
    /// `FontSystem::bake_masks`' own gaussian to the letter, so a glow
    /// moved onto this lane keeps its CHARACTER and only loses the
    /// nine-slice's stretched middle.
    pub const GAUSS: u32 = 1 << 15;
    /// Bits 8-11 carry the [`ShapeKind`].
    pub const KIND_SHIFT: u32 = 8;
    /// Bits 14-15: the soft profile's own pair. A record carrying
    /// either draws a different FUNCTION of the distance, not another
    /// part of one silhouette, which is why the weld refuses it (§2.10)
    /// and why the core split refuses it too.
    pub const SOFT: u32 = Shape::OUTSIDE_ONLY | Shape::GAUSS;
    /// Bits 0-11: everything that describes the SILHOUETTE — the four
    /// corner treatments and the kind — as against the parts drawn on
    /// it. Two records agreeing here and on the numbers above outline
    /// the same curve, which is what makes welding them legal (§2.10).
    pub const SILHOUETTE: u32 = (1 << 12) - 1;
}

/// The closed family of silhouettes the vector core draws (f3 §2.1),
/// each carrying the numbers ITS OWN field reads and no others.
///
/// **A kind is worth adding only when the family cannot already spell
/// it**, and the rule has teeth: a disc is `d_round(p, [r, r], r)`,
/// which is `|p| − r` identically, so a joint disc, a dot in a matrix
/// and every circular knob are Box records with round corners as big as
/// their own half size — one record, one quad, and the shader learns
/// nothing (`crate::sdf::d_disc`). A closed annulus is the same Box
/// wearing its inward stroke. What the Box family CANNOT spell is the
/// angularly truncated arc, and that is the whole of [`ShapeKind::Ring`].
///
/// How the payloads reach the 64-byte record ([`Shape`]):
///
/// | kind | `corner[0..3]` | `arc_half` | `arc_dir` |
/// |---|---|---|---|
/// | `Box` (0) | the four corner sizes | — | — |
/// | `Ring` (1) | \[0\] half the band's radial thickness | half the sweep | the middle's direction |
/// | `Hex` (2) | \[0\] the apothem, fitted to the rect here | — | the lattice turn |
/// | `Chevron` (3) | \[0\] the left end's depth, \[1\] the right's | — | — |
/// | `Capsule` (4) | reserved: `line()` still tessellates | — | — |
///
/// Lengths in `corner`, angles in `arc_*`: one field never means a
/// pixel on one record and a radian on the next, and a reader of either
/// file can say which it is holding without knowing the kind.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ShapeKind {
    Box,
    /// The annular arc — a band of `width` px about a circle, swept
    /// `2 · half_sweep` radians about `dir`, with round caps. The
    /// circle's outer edge meets the shorter side of the rect.
    /// `half_sweep >= PI` closes it into a plain ring, which is the one
    /// case Box could have drawn too.
    Ring { width: f32, half_sweep: f32, dir: f32 },
    /// The regular hexagon, as large as fits the rect under its own
    /// turn. `turn` is the lattice angle: 0 puts a flat edge at the top
    /// (`shape.hex.orientation = flat`), 30° puts a vertex there
    /// (`pointy`). Any angle between is legal and nothing forbids it.
    Hex { turn: f32 },
    /// The rect with one or both vertical ends collapsed to a point at
    /// mid-height — `shape.taskbar`'s silhouette, and at full depth on
    /// one end the solid paging arrow the master describes. The depths
    /// are px, measured inward from each end.
    Chevron { left: f32, right: f32 },
    /// Reserved. A segment of given width with flat or round caps —
    /// what `line()` and `polyline()` would become; K4 left them
    /// tessellated and K6 does not move them.
    Capsule,
}

impl ShapeKind {
    /// The number bits 8-11 carry. Written out rather than derived from
    /// a discriminant: the payloads make this enum an ordinary Rust one,
    /// and the wire numbers are a promise to the shader (and, at K7, to
    /// every compiled plugin) that no reordering here may quietly break.
    pub fn code(self) -> u32 {
        match self {
            ShapeKind::Box => 0,
            ShapeKind::Ring { .. } => 1,
            ShapeKind::Hex { .. } => 2,
            ShapeKind::Chevron { .. } => 3,
            ShapeKind::Capsule => 4,
        }
    }

    /// The kind of a record read back out of its flag word, WITHOUT the
    /// payloads — which live in the record's own numbers, not in the
    /// bits. The reference field ([`crate::sdf::d_record`]) takes them
    /// from there, exactly as the shader does.
    pub fn of_code(code: u32) -> ShapeKind {
        match code {
            1 => ShapeKind::Ring { width: 0.0, half_sweep: 0.0, dir: 0.0 },
            2 => ShapeKind::Hex { turn: 0.0 },
            3 => ShapeKind::Chevron { left: 0.0, right: 0.0 },
            4 => ShapeKind::Capsule,
            _ => ShapeKind::Box,
        }
    }
}

/// The two soft parts of the vector family (f3 §2.6, and §4.6/§4.7 of
/// the scope decision), which are ONE mechanism wearing two settings.
///
/// A glow and a shadow differ in exactly two things, and both of them
/// are here: whether the interior draws ([`Shape::OUTSIDE_ONLY`]) and
/// whether the run adds light or covers ([`SHAPE_ADD`] against
/// [`SHAPE`]). The profile, the field, the fragment and the record are
/// the same in both. Spelling them as one enum rather than two booleans
/// is what stops a caller from asking for the two combinations nobody
/// means — an inside-only glow that lights nothing, or a shadow that
/// brightens what it falls on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SoftKind {
    /// Light around the silhouette, nothing within it: `glow.*`.
    Glow,
    /// A plateau under the whole silhouette, falling off outside it:
    /// `shadow.*`. The offset is the CALLER's — a shadow is its own
    /// record, so a shifted shadow is a shifted rect and the record
    /// needs no field for it.
    Shadow,
}

/// The soft profile a shape wears, or [`None`] for a crisp one.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Soft {
    /// Where the profile reaches zero, px past the boundary: the
    /// theme's `glow.<class>.radius` or `shadow.radius`. σ is a third
    /// of it — [`crate::sdf::soft_profile`] carries the derivation and
    /// the reason it is not a token.
    pub reach: f32,
    pub kind: SoftKind,
}

/// One shape as the caller means it (f3 §2.11: bed, edge, glow and
/// shadow). Bed and edge SHARE the record
/// when both are present, because they share a silhouette — as two
/// records their antialiased outer edges would blend twice on the same
/// pixels and a translucent panel over glass would grow a dark rim
/// (§2.10).
#[derive(Clone, Copy, Debug)]
pub struct ShapeSpec {
    pub rect: Rect,
    /// tl, tr, br, bl — [`ring_points`]' order. Read by
    /// [`ShapeKind::Box`] ALONE: every other kind states its own
    /// geometry in its payload, and passing corners with one of them is
    /// not an error, it is simply not read. The record's corner slots
    /// are then zeroed, so two hexagons of one size weld together
    /// however each caller happened to fill this field.
    pub corners: [Corner; 4],
    pub kind: ShapeKind,
    /// Interior colour; rides the vertices, like every fill before it.
    pub fill: Option<Color>,
    /// Width and colour of the inward band; rides the record.
    pub stroke: Option<(f32, Color)>,
    /// The frosted layer UNDER the fill, or none for an ordinary
    /// surface (f3 §3.3). It is part of the spec rather than a call of
    /// its own because it is part of the same silhouette: one record,
    /// one edge, one coverage — the whole reason K3b exists.
    pub glass: Option<Frost>,
    /// The softness profile (§2.6), or none for a crisp shape.
    ///
    /// Unlike the frost this is NOT another layer of one surface: the
    /// glow around a panel and the shadow under it are separate records
    /// from the panel, because they are separate functions of the same
    /// distance and the blender must see them one after another. What
    /// they share with the panel is the silhouette, and that is why
    /// they belong in this type at all — a glow that did not read the
    /// same corners would be the square-bloom bug in a new place.
    ///
    /// A soft record carries no `stroke`: the band's coverage is the
    /// difference of two CRISP ramps and means nothing under a
    /// gaussian. `shape_verts` asserts it rather than silently dropping
    /// one of the two.
    pub soft: Option<Soft>,
}

/// The blurred scene behind a surface, as a shape carries it (f3 §3.3).
///
/// `rank` is the pyramid rung, 1..=3, lightest to deepest blur — the
/// renderer clamps it against the depth the frame actually wrote, so a
/// theme asking for more blur than the frame built gets the deepest
/// there is rather than an unwritten image. `tint` MULTIPLIES what the
/// rank samples, which is why the master's ladder says it can only
/// darken; the wash that can brighten is the surface's own fill.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Frost {
    pub rank: u8,
    pub tint: Color,
    /// What the RECORD's BAND carries, when it must differ from `tint`
    /// (K3c). `None` means "the same colour" — every caller but one, and
    /// the record ends up with `tint` exactly as it always did.
    ///
    /// The one caller is [`DrawList::glass_fill`]'s fractional path.
    /// `tint` there still governs the CORE — the two rungs' plain quads,
    /// composed straight (the core is artifact-free: `cov` is 1
    /// throughout it, so two sequential blends of a constant alpha are
    /// exact, not a double count). The BAND is the opposite case: it is
    /// the silhouette's own outer edge, and a SECOND record blending a
    /// SECOND coverage ramp there is the defect `glass_fill`'s own doc
    /// names — `c·(1 − c)·a·b` of excess alpha, repeated because two
    /// runs cannot fold into one fragment (closing THAT needs a
    /// fragment that reads two pyramid targets, a renderer change on
    /// the far side of the repository boundary).
    ///
    /// This field answers the part that does not need the renderer to
    /// change: a record's BAND coverage is evaluated once regardless of
    /// how many records carry a frost, so let exactly one of the two
    /// carry it. The lower rung's band is silenced (alpha 0 — its own
    /// core still shows, unaffected); the upper rung's band is set to
    /// the two rungs' COMBINED alpha `a1 + a2 − a1·a2`, the standard
    /// over-composite of two fully-covering layers, which is exactly
    /// what the one coverage evaluation that survives is supposed to
    /// fold in.
    pub band_tint: Option<Color>,
}

/// A bed still open to the parts that belong to it (f3 §2.10).
///
/// `ring_fill` and `ring` are two calls because the tessellated lane
/// needs two: a fan and a strip. On the vector lane they are two
/// DESCRIPTIONS OF ONE SILHOUETTE, and drawing that silhouette twice
/// means blending its antialiased outer edge twice — `1 − (1 − a)²`
/// where the geometry says `a`, the dark rim R4 names. So the bed's
/// record stays open: the very next call, if it draws another part of
/// the same outline and nothing has happened in between, joins that
/// record instead of writing a second one. No new quad, no second
/// edge, no change at any call site.
///
/// **Two parts join, not one.** A border (STROKE) takes the record's
/// band bits. A SECOND FILL — the state wash every button and every
/// field lays over its plate — is composited into the colour of the
/// quad that is already there ([`fill_over`]), and the offer stays
/// open behind it, so plate + wash + border is one record.
///
/// The wash was the half of R4 that survived the first round, and it
/// is the half that matters most: the census of the twelve shape sites
/// (2026-08-17, probe on the live theme, records read back from
/// `DrawList::shapes`) found the fill+fill pair in exactly two of them
/// — `button::dress` (`button.rs:109` + `:114`) and `text_input::draw`
/// (`text_input.rs:953` + `:967`) — and those two are the button, every
/// row of every drop-down through `dropdown.rs`, and every field you
/// type into. Its depth is the wash's alpha, which is the THEME's
/// number and not the code's, so nothing in this file can bound it.
/// Welded, the pair is one bed whose colour is the wash over the plate,
/// carrying one AA edge, which is what the geometry always meant.
///
/// "Nothing in between" is checked, not assumed. Every drawing call
/// moves `verts`; every clip push, pop, restore and texture change
/// pushes a `run`; the record must still be the last one written. The
/// geometry compared is what the RECORD holds — post-snap, post-sentinel
/// — because two specs that resolve to the same record are the same
/// silhouette whatever the caller wrote.
///
/// **The alternative, and why not.** The other way to spell this is an
/// explicit entry — `framed(rect, corners, fill, stroke)` — and it is
/// the honest one in the abstract: the caller says what it means and
/// nothing has to be inferred. It was rejected because the meaning is
/// already there. TWELVE call sites across the toolkit already write
/// the pair in the same order for the same reason — `button`, `window`,
/// `menu`, `tooltip`, `text_input`, `winframe`, `elev`, `segmented`,
/// `tabs`, `surface`, and `paint`'s pill and scrollbar thumb — every one
/// of them a fill and then a ring with nothing drawn between.
/// [`ShapeSpec`] can already carry both parts, and `shape()` is the door
/// for a caller who wants to say it outright. A new entry would ask every object to be rewritten to say
/// what it already says, would leave the old pair
/// working-but-wrong for anything not rewritten (a plugin, a script
/// table, a widget written later), and would make the defect's absence
/// depend on remembering. This way the defect cannot come back through
/// the door it came in by.
#[derive(Clone, Copy)]
struct Weld {
    idx: usize,
    /// The bed's own quads, `from..verts` of the vertex buffer — the
    /// geometry a deepening band re-cuts. `verts` doubles as the
    /// "nothing drawn since" mark.
    from: usize,
    /// Where the vertices a welding fill RECOLOURS begin. The same as
    /// `from` for an ordinary bed, and one quad later on a frosted one:
    /// a frost's core rides the tessellated glass lane and carries the
    /// TINT, which a wash must lie over and must never overwrite.
    paint: usize,
    verts: usize,
    runs: usize,
    centre: [f32; 2],
    half: [f32; 2],
    corner: [f32; 4],
    /// [`Shape::SILHOUETTE`] of the record: corner treatments and kind.
    bits: u32,
    /// What those vertices carry NOW: every fill welded so far,
    /// composited. The bed the next one composites onto.
    fill: [f32; 4],
    /// The band the bed's frame was cut at (f3 §7b), or `None` where the
    /// bed rasterises through one quad. A border welding in deepens the
    /// band by its own width, so the core has to shrink to match — the
    /// geometry was laid out for a bed with no border at all, and the
    /// strips must still cover every pixel the field can bend.
    frame: Option<f32>,
}

/// `top` over `bottom` when both are about to be blended onto the same
/// unknown destination — the arithmetic that lets two fills of one
/// silhouette become one.
///
/// The hardware blends straight alpha: `d' = c.rgb·c.a + d·(1 − c.a)`.
/// Two fills in a row leave
///
/// ```text
/// d₂ = B.rgb·B.a + (1 − B.a)·A.rgb·A.a + d·(1 − A.a)(1 − B.a)
/// ```
///
/// and one fill `C` leaves `C.rgb·C.a + d·(1 − C.a)`. Matching the
/// coefficient of `d` gives `C.a = A.a + B.a − A.a·B.a`, and what is
/// left gives `C.rgb·C.a = B.rgb·B.a + (1 − B.a)·A.rgb·A.a`. Both sides
/// are the same function of the destination, so this is an identity for
/// EVERY background, not a match on one.
///
/// Two consequences worth naming:
///
/// * **No transfer curve here.** The composite is done on the numbers
///   the vertices carry, because those are the numbers the blender
///   works on — the swapchain's own encoding, not linear light. This is
///   [`Color::composite_as_rendered`]'s question, not [`Color::over`]'s.
/// * **A ride commutes with it** (§2.9, R8). The board ride mixes every
///   vertex colour toward `surface.void`, `x ↦ v + (x − v)·s`, which is
///   affine, and the composite's rgb weights sum to exactly `C.a`; so
///   dimming the composite equals compositing the dimmed pair. The
///   welded bed dims like the two it replaced, to the last bit of the
///   arithmetic.
///
/// Like the band's own composition, this happens BEFORE the fragment's
/// `grade()` — one fragment per silhouette is the lane's whole premise.
///
/// The arithmetic itself lives in [`crate::sdf::over`], with the rest of
/// the lane's specification, because the fragment shader needs the same
/// identity for a wash over frosted glass (§3.3) and two copies of one
/// composite is how a lane grows two answers.
fn fill_over(top: [f32; 4], bottom: [f32; 4]) -> [f32; 4] {
    crate::sdf::over(top, bottom)
}

/// How far past the silhouette a shape's quad reaches, so the coverage
/// ramp has somewhere to land. A feather would join this margin.
const AA_PAD: f32 = 1.0;

/// The slack between the deepest the field can still differ from the
/// plain fill and where the CORE quad begins (f3 §7b, remedy 1).
///
/// The core is shaded by the ordinary fill path, so the split is only
/// legal where `fs_shape` would have returned the fill colour and
/// nothing else — `cov = 1`, `a_band = 0`. That needs
/// `d ≤ −(stroke + w/2)` with `w` the field's screen gradient, one on a
/// still screen. [`corner_reach`] + the stroke buys `d ≤ −(reach +
/// stroke + AA_PAD + CORE_PAD)` inside the core, so the margin is
/// `AA_PAD + CORE_PAD − w/2` — 2.5 px at `w = 1`, and still positive at
/// `w = 4`.
///
/// The margin is not decoration. Under multisampling the fragment is
/// shaded at the PIXEL CENTRE while coverage is decided per sample, so
/// a core pixel can be shaded up to half a pixel diagonal (0.71 px)
/// outside the core it belongs to; the unsplit quad would have shaded
/// that same fragment through the field. Two px of slack covers that
/// with more than a pixel to spare, and costs a two-pixel-deeper band
/// on a saving measured in hundreds.
const CORE_PAD: f32 = 2.0;

/// The smallest core worth cutting out, px². Below it the four strips
/// of the frame cost more vertices than the fragments they save — the
/// split is an optimisation and has to earn its place on every shape,
/// not on the average one. 256 px² is a 16×16 core.
const CORE_MIN: f32 = 256.0;

/// How deep from the boundary a corner treatment of size `size` can
/// still bend the field — the reason a shape needs an analytic band at
/// all, and the number that sizes it (f3 §7b, remedy 1).
///
/// Take the rect inset by `m` on all four sides. Its box distance is at
/// most `−m` everywhere for free; what can spoil that is the corner.
///
/// * **Square** cannot: the field IS the box distance. Reach 0.
/// * **Round** of radius `k`: the inset rect's own corner reads
///   `√2·(k − m) − k` while `m < k` and exactly `−m` once `m ≥ k`, so
///   reach `k`.
/// * **Chamfer** of cut `k`: the 45° plane reads `(k − 2m)/√2`, and
///   `(k − 2m)/√2 ≤ −m` exactly when `m ≥ k/(2 − √2) ≈ 1.707·k`. A
///   chamfer eats deeper than a round corner of the same size because
///   it cuts straight across where the arc bulges out.
fn corner_reach(style: CornerStyle, size: f32) -> f32 {
    match style {
        CornerStyle::Square => 0.0,
        CornerStyle::Round => size,
        // 1/(2 − √2) = (2 + √2)/2, written so the constant is derivable
        // by eye rather than looked up.
        CornerStyle::Chamfer => size * (2.0 + std::f32::consts::SQRT_2) * 0.5,
    }
}

/// The record's four length slots for a kind past Box (the table on
/// [`ShapeKind`]), everything the kind does not read left at zero.
///
/// Every number is CLAMPED to what the rect can hold, here and not in
/// the shader: the field has no rect to ask, and a payload that reached
/// it unclamped would draw a silhouette outside the quad that carries
/// it — a shape cut off at a straight edge nobody wrote.
fn kind_lengths(kind: ShapeKind, half: [f32; 2]) -> [f32; 4] {
    let short = half[0].min(half[1]).max(0.0);
    match kind {
        // Half the band's thickness, which is also its radius of
        // curvature at the caps: past half the short side the band would
        // swallow its own hole, so that is where it stops.
        ShapeKind::Ring { width, .. } => [(width * 0.5).clamp(0.0, short), 0.0, 0.0, 0.0],
        ShapeKind::Hex { turn } => [hex_apothem(half, turn), 0.0, 0.0, 0.0],
        // Two ends may each eat the whole width; both at once meet in
        // the middle and the shape closes to a pair of touching points,
        // which is what "collapsed to a point" says at 100 % twice.
        ShapeKind::Chevron { left, right } => {
            let w = (half[0] * 2.0).max(0.0);
            [left.clamp(0.0, w), right.clamp(0.0, w), 0.0, 0.0]
        }
        ShapeKind::Box | ShapeKind::Capsule => [0.0; 4],
    }
}

/// The record's two angle slots, radians — likewise zero for a kind
/// that reads none.
fn kind_angles(kind: ShapeKind) -> (f32, f32) {
    match kind {
        ShapeKind::Ring { half_sweep, dir, .. } => (half_sweep.max(0.0), dir),
        ShapeKind::Hex { turn } => (0.0, turn),
        _ => (0.0, 0.0),
    }
}

/// The largest apothem a regular hexagon turned by `turn` can wear
/// inside the rect of half sizes `half`.
///
/// A hexagon of apothem `r` has circumradius `2r/√3`, and at `turn = 0`
/// — the flat-topped one [`crate::sdf::d_hex`] is written about — its
/// six vertices lie at `k·60°`, the first of them on +x. Its half
/// extent along an axis is the largest projection of those vertices on
/// that axis; solve each axis for `r` and take the tighter, so `flat`
/// fills a wide rect and `pointy` a tall one, and neither ever pokes
/// out of the quad the emitter padded for it.
///
/// The 30° is not a look value: it is what the words `flat` and `pointy`
/// MEAN on a six-fold lattice, the same way `SQRT1_2` is what a 45° cut
/// means. The choice between them is the theme's
/// (`shape.hex.orientation`), and only the choice.
fn hex_apothem(half: [f32; 2], turn: f32) -> f32 {
    let circum = 2.0 / 3.0f32.sqrt();
    let (mut cx, mut cy) = (0.0f32, 0.0f32);
    // Three vertices span every direction the other three do, mirrored.
    for k in 0..3 {
        let a = turn + k as f32 * std::f32::consts::FRAC_PI_3;
        cx = cx.max(a.cos().abs());
        cy = cy.max(a.sin().abs());
    }
    let fit = |extent: f32, reach: f32| {
        if reach > 1e-6 {
            extent / (reach * circum)
        } else {
            f32::INFINITY
        }
    };
    fit(half[0].max(0.0), cx).min(fit(half[1].max(0.0), cy))
}

/// Half sizes of the core a band `band` deep leaves behind — clamped at
/// zero, where the frame closes over the whole shape and the core is
/// empty.
fn core_half(half: [f32; 2], band: f32) -> [f32; 2] {
    [(half[0] - band).max(0.0), (half[1] - band).max(0.0)]
}

/// The five rectangles a split shape rasterises through — the core
/// first, then the four strips around it — as `[x0, y0, x1, y1]`.
///
/// Their union is EXACTLY the padded bounds of the single quad they
/// replace and their interiors are disjoint, for every core the clamp
/// above can produce: at a core of zero the two horizontal strips meet
/// on the centre line and the other three collapse to nothing, which is
/// the unsplit quad again in four pieces. That identity is what lets
/// the picture be argued rather than looked at, and it is why the
/// decomposition is a frame and not a nine-patch.
fn frame_rects(centre: [f32; 2], ext: [f32; 2], core: [f32; 2]) -> [[f32; 4]; 5] {
    let (x0, x1) = (centre[0] - ext[0], centre[0] + ext[0]);
    let (y0, y1) = (centre[1] - ext[1], centre[1] + ext[1]);
    let (ix0, ix1) = (centre[0] - core[0], centre[0] + core[0]);
    let (iy0, iy1) = (centre[1] - core[1], centre[1] + core[1]);
    [
        [ix0, iy0, ix1, iy1],
        [x0, y0, x1, iy0],
        [x0, iy1, x1, y1],
        [x0, iy0, ix0, iy1],
        [ix1, iy0, x1, iy1],
    ]
}

/// Where a split shape's CORE quad goes: the core rect while it has a
/// colour to carry, and nothing at all — a rectangle of no area, at the
/// centre — while it has not.
///
/// The quad stays in the LAYOUT either way (the cores then four strips,
/// six vertices each, the arithmetic [`DrawList::respan_frame`] walks
/// by), because a weld recolours quads and never adds one. What it does
/// not do is cost fragments: an interior the width of a panel, blended
/// at alpha zero, is a read-modify-write per pixel for a picture that
/// does not change. The case that made this necessary is a frosted
/// surface (§3.3), whose bed is laid empty and filled by a wash that
/// arrives one call later — or never.
fn bed_rect(core: [f32; 4], centre: [f32; 2], colour: [f32; 4]) -> [f32; 4] {
    if colour[3] > 0.0 {
        core
    } else {
        [centre[0], centre[1], centre[0], centre[1]]
    }
}

/// One axis-aligned rectangle as six vertices, in `push_quad4`'s own
/// winding. `local` carries the record's centre on the shape lane —
/// the uv contract, `pos − centre` — and `None` puts the atlas's white
/// pixel there, which is what every solid fill in this file already
/// samples.
fn quad6(r: [f32; 4], local: Option<[f32; 2]>, colour: [f32; 4], shape: u32) -> [Vertex; 6] {
    let p = [[r[0], r[1]], [r[2], r[1]], [r[2], r[3]], [r[0], r[3]]];
    let white = FontSystem::white_uv();
    let v = |i: usize| Vertex {
        pos: p[i],
        uv: match local {
            Some(c) => [p[i][0] - c[0], p[i][1] - c[1]],
            None => [white.0, white.1],
        },
        color: colour,
        shape,
    };
    [v(0), v(1), v(2), v(0), v(2), v(3)]
}

/// Treatment of one rect corner — the vocabulary of the one tessellated
/// ring generator (r1 §3). There is no arc primitive and no mask-based
/// corner: nothing in this pipeline is antialiased except text, and a
/// smooth corner alone would be the only soft silhouette on screen.
#[repr(u8)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CornerStyle {
    Square = 0,
    Round = 1,
    Chamfer = 2,
}

/// One corner: the style plus its size — the cut length for Chamfer, the
/// radius for Round, ignored by Square. The size is a design value and
/// therefore always arrives as a parameter from a token; nothing here
/// defaults it.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Corner {
    pub style: CornerStyle,
    pub size: f32,
}

impl Corner {
    pub const SQUARE: Corner = Corner { style: CornerStyle::Square, size: 0.0 };

    pub const fn round(r: f32) -> Corner {
        Corner { style: CornerStyle::Round, size: r }
    }

    pub const fn chamfer(len: f32) -> Corner {
        Corner { style: CornerStyle::Chamfer, size: len }
    }

    /// The corner a THEME asks for, on the box it is about to cut: the
    /// style from the token's `*_corner_style` sibling, the radius from
    /// the `*.corner` token, and the box because §5.0's `pill` is not a
    /// length until there is one.
    ///
    /// This is what a capsule is made of. `pill` says "as round as this
    /// box can be" and bakes to a negative sentinel, so every consumer
    /// that compares it against zero has so far drawn a rectangle where
    /// the theme wrote a capsule; half the short side is the largest
    /// radius `ring_points` honours, so a pill is exactly that.
    pub fn sized(style: CornerStyle, radius: f32, r: Rect) -> Corner {
        Corner { style, size: crate::theme::corner_radius(radius, r.w, r.h) }
    }

    /// The same corner on a boundary moved inward by `d` (outward when `d`
    /// is negative), keeping the moved face parallel to the original.
    /// Round offsets to a concentric arc: exactly `r − d`. Chamfer: moving
    /// the 45° face `x + y = k` inward by `d` shifts its constant by `d·√2`
    /// while the corner it is measured from shifts by `2d`, so the cut
    /// shrinks by `(2 − √2)·d` — the derivation behind `chamfer_frame`'s
    /// 0.293·t (there `d = t/2`). Square has nothing to move.
    pub fn inset(self, d: f32) -> Corner {
        let size = match self.style {
            CornerStyle::Square => 0.0,
            CornerStyle::Round => (self.size - d).max(0.0),
            CornerStyle::Chamfer => {
                (self.size - (2.0 - std::f32::consts::SQRT_2) * d).max(0.0)
            }
        };
        Corner { style: self.style, size }
    }
}

/// HOW a glow spends its light across its reach — the shape, never the
/// amount. Reach is `radius`, amount is the caller's alpha; this is the
/// only thing left to decide once those two are known.
///
/// Five numbers because the owner's brief for a neon tube is three
/// statements about light and one of them (the burned-white core) is not
/// light at all but the EDGE, drawn by the caller; the fourth is how
/// finely the first three are laid down, and the fifth is how much of
/// `radius` the light is let to spend at all. What is here:
///
/// * `decay` — the distance re-map. 1.0 is the soft disk laid flat
///   across the reach, which is what every glow in this toolkit drew
///   before there was a choice. Above 1.0 the profile is pulled in
///   toward the path: what the disk showed at a third of the reach a
///   `decay` of 3 shows at a twenty-seventh of it. Light that STOPS is
///   the difference between a lit tube and a blurred copy of a line.
/// * `aura` — a multiplier on the alpha at the path, ramped back to 1.0
///   over `aura_reach`. The band of colour a photographed sign keeps
///   right against the glass, and the reason a tube reads as coloured at
///   all once its core has been driven white.
/// * `aura_reach` — how far that band goes, as a fraction of `radius *
///   cutoff` — the reach the light actually gets, not the reach the
///   theme merely declared (see `cutoff`).
/// * `bands` — how many rings the reach is cut into, which is how closely
///   the emitted geometry follows the re-map. IT LOOKED LIKE A QUALITY
///   NUMBER AND IT IS NOT, which is why it is a token like the rest: the
///   rasteriser interpolates between the rings it is given, so a coarse
///   cut is not a rougher drawing of the same curve but a DIFFERENT
///   curve, and at one band the decay disappears entirely. A rule in Rust
///   that grew the count with the radius also meant one theme drew two
///   different tubes — a steeper one around a button than around a panel
///   — with nothing in the file saying so.
/// * `cutoff` — the fraction of `radius` the geometry actually reaches
///   out to, `0.0 .. 1.0`. `decay` alone can pull an exponential
///   arbitrarily close to the mask's own zero texel, but "close" is not
///   "identical" at any point that is not the true rim — the curve is
///   continuous, so a decay steep enough to look flat by some small
///   fraction of a LARGE radius still carries a measurable, monotonic
///   residual the rest of the way out, because that is what a continuous
///   re-map of one texture sample can and cannot do (2026-08-24's own
///   measurement: `decay = 50` on a 4u reach still moved the background
///   channel by four steps over the outer 90% of it, not zero of them).
///   A cutoff under 1.0 answers a DIFFERENT question than decay does: at
///   `radius * cutoff` the geometry simply STOPS being extruded — there
///   is no quad out there for any alpha to be wrong on, additive or
///   otherwise, which is the one way to be exactly the background and
///   not only close to it past a chosen point. 1.0 spends the whole of
///   `radius`, which is every picture this struct drew before this field
///   existed.
///
/// Every one of them arrives from a theme token. Nothing in this file
/// chooses a value; [`GlowProfile::HALO`] is not a design default but the
/// identity element — the profile at which this whole struct disappears
/// and the emitter draws exactly what it drew before it existed.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GlowProfile {
    pub decay: f32,
    pub aura: f32,
    pub aura_reach: f32,
    pub bands: u32,
    pub cutoff: f32,
}

impl GlowProfile {
    /// The soft disk, unshaped: the picture every caller got before a
    /// profile could be asked for, and the one `glow_ring` still asks
    /// for. Not a default anybody chose — the numbers at which every
    /// arithmetic step in [`DrawList::glow_ring_with`] is an identity.
    pub const HALO: Self = Self { decay: 1.0, aura: 1.0, aura_reach: 0.0, bands: 1, cutoff: 1.0 };

    /// The most rings a reach is ever cut into. A GUARD ON A USER FILE,
    /// in the same family as the `3..=16` on [`ring_points`]'s segments:
    /// the token's declared range said in code, so a theme that writes a
    /// million cannot ask this process for a million ring strokes. It
    /// picks no picture — every count the master declares is inside it.
    pub const MAX_BANDS: u32 = 16;

    /// Whether this profile asks for nothing the halo did not already do.
    /// A theme is free to name a shaped profile and then flatten every
    /// knob of it; the answer here is about the NUMBERS, so such a theme
    /// gets the halo's own vertices rather than a subdivided copy of them.
    ///
    /// `bands` is not among them ON PURPOSE. It says how finely a shape
    /// is followed, and there is no shape here to follow: subdividing the
    /// identity re-map lands every extra ring on the line the single band
    /// already drew, so honouring a count would spend vertices to emit
    /// the same picture and break the bit-for-bit rename proof.
    pub fn is_halo(&self) -> bool {
        self.decay == 1.0 && !self.lifts()
    }

    fn lifts(&self) -> bool {
        self.aura > 1.0 && self.aura_reach > 0.0
    }

    /// The distance fractions this profile lays a ring of vertices at, in
    /// rising order, written into `out`; the count is the return.
    ///
    /// The theme's even cut of the reach, PLUS ONE BOUNDARY THE THEME
    /// ASKS FOR BY NAME: `aura_reach`. Without it the aura can only let
    /// go where the even grid happens to have a ring, and the rasteriser
    /// interpolates across the gap — a reach of 0.25 on a five-band grid
    /// still lifts every pixel out to 0.4, and a reach under `1/bands`
    /// cannot be drawn at all. The theme states a distance; this is what
    /// makes that distance the one the picture shows. It costs one ring
    /// and only on a profile that lifts.
    ///
    /// The halo takes exactly one stop whatever it asked for — see
    /// [`GlowProfile::is_halo`].
    fn stops(&self, out: &mut [f32; Self::MAX_BANDS as usize + 1]) -> usize {
        if self.is_halo() {
            out[0] = 1.0;
            return 1;
        }
        let n = self.bands.clamp(1, Self::MAX_BANDS);
        let aura = if self.lifts() { Some(self.aura_reach) } else { None };
        let mut m = 0usize;
        for k in 1..=n {
            let f = k as f32 / n as f32;
            // The reach joins the cut wherever it falls strictly between
            // two of the cut's own stops. `a > prev` is what makes that
            // happen exactly once and is the whole condition: after the
            // insert every later stop is above the reach, so no second
            // one can pass, and a reach the cut already lands on never
            // passes at all — a ring emitted there would be a band of
            // zero width, a quad that draws nothing. A reach at or past
            // the rim satisfies no `a < f` and so never arrives, which is
            // right; there is no light out there for it to let go over.
            if let Some(a) = aura {
                let prev = if m == 0 { 0.0 } else { out[m - 1] };
                if a < f && a > prev {
                    out[m] = a;
                    m += 1;
                }
            }
            out[m] = f;
            m += 1;
        }
        m
    }

    /// The mask's v at distance fraction `f` of the reach, between the
    /// disk's peak `vi` on the path and its zero `v0` at the rim.
    ///
    /// The two ENDS ARE EXACT and not the formula's answer at 0 and 1,
    /// which is what lets a halo's band land on the same texel the
    /// unshaped emitter used to hand it: `vi + (v0 - vi) * 1.0` is not
    /// `v0` in binary floating point, and one texel of drift is a
    /// different picture for a proof that compares vertices.
    fn v_at(&self, f: f32, vi: f32, v0: f32) -> f32 {
        if f <= 0.0 {
            return vi;
        }
        if f >= 1.0 {
            return v0;
        }
        let s = if self.decay == 1.0 { f } else { f.powf(1.0 / self.decay) };
        vi + (v0 - vi) * s
    }

    /// The alpha at distance fraction `f`, given the caller's own.
    ///
    /// EXACTLY 0.0 AT THE RIM, BY THE VERTEX AND NOT BY THE TEXTURE. The
    /// mask sprite's own edge texel is baked to 0.0 (`bake_masks`'s own
    /// `d >= r` branch), and the ring at `f = 1.0` samples exactly that
    /// texel (`v_at`'s own `f >= 1.0 => v0` branch) — so in principle the
    /// additive blend already lands on nothing past the reach. In
    /// practice the GPU's own bilinear filter reads a NEIGHBOURHOOD of
    /// texels around the sample point, not one, and this sprite shares
    /// an atlas with everything else the font system packs — a rim
    /// sampled a half-texel short of the edge, or packed with no margin
    /// past it, blends in whatever sits next to it in the shelf. Before
    /// this the fade past the reach was carried ENTIRELY by that
    /// sampling being exact; now the vertex alpha the caller multiplies
    /// against is *itself* zero at the rim, so even a mis-sampled
    /// texture is multiplied against nothing and still lands on nothing.
    /// The two ARE the same picture inside the reach — this changes
    /// nothing `f < 1.0` was already answering — and are no longer only
    /// hopefully the same picture at it.
    fn alpha_at(&self, f: f32, alpha: f32) -> f32 {
        if f >= 1.0 {
            return 0.0;
        }
        if !self.lifts() {
            return alpha;
        }
        let t = (f / self.aura_reach).clamp(0.0, 1.0);
        (alpha * (1.0 + (self.aura - 1.0) * (1.0 - t))).min(1.0)
    }
}

/// The smallest segment count whose chord error stays under `tol` px at
/// radius `r`, capped by `ceiling` — the sagitta of a quarter-arc chord is
/// `r·(1 − cos(45°/S))`. The caller passes the theme's `corner.segments`
/// as the ceiling and the tolerance it can live with; at 0.25 px the
/// shipped corner ladder answers 3/3/4 where a flat 6 would spend 40 %
/// more vertices for error already below the pixel grid (r1 §3.4).
pub fn ring_segments(r: f32, tol: f32, ceiling: u8) -> u8 {
    let ceiling = ceiling.clamp(3, 16);
    let r = r.max(0.0);
    for s in 3..=ceiling {
        let half = std::f32::consts::FRAC_PI_4 / s as f32;
        if r * (1.0 - half.cos()) <= tol {
            return s;
        }
    }
    ceiling
}

/// Boundary of `r` under the four corner treatments, tl → tr → br → bl,
/// clockwise in screen coordinates (y down). Square contributes 1 point,
/// Chamfer 2, Round `segments + 1` — the counts depend only on style and
/// segments, never on size, so two parallel rings always correspond
/// index-to-index and a stroke between them is watertight. Sizes are
/// clamped to half the short side, segments to 3..=16 (geometry clamps,
/// not design defaults). One `sin_cos` per call; the arc itself is adds
/// and multiplies via incremental rotation, endpoints pinned exactly onto
/// the edges so a flush test can compare bitwise-close.
///
/// `pub(crate)` because a stroke is not always a fill: the focus ring
/// walks this boundary to lay dashes along it, and a second generator for
/// the dashed case would be a second answer to what shape a control is.
pub(crate) fn ring_points(r: Rect, c: &[Corner; 4], segments: u8, out: &mut Vec<[f32; 2]>) {
    out.clear();
    let seg = segments.clamp(3, 16) as u32;
    let cap = (r.w.min(r.h) * 0.5).max(0.0);
    let (sin_t, cos_t) = (std::f32::consts::FRAC_PI_2 / seg as f32).sin_cos();
    // Corner point plus the unit directions back along the two edges it
    // joins: the ring enters at `p + sz·e_in` and leaves at `p + sz·e_out`.
    let corners: [([f32; 2], [f32; 2], [f32; 2]); 4] = [
        ([r.x, r.y], [0.0, 1.0], [1.0, 0.0]),
        ([r.x + r.w, r.y], [-1.0, 0.0], [0.0, 1.0]),
        ([r.x + r.w, r.y + r.h], [0.0, -1.0], [-1.0, 0.0]),
        ([r.x, r.y + r.h], [1.0, 0.0], [0.0, -1.0]),
    ];
    for (i, &(p, ein, eout)) in corners.iter().enumerate() {
        let sz = c[i].size.clamp(0.0, cap);
        match c[i].style {
            CornerStyle::Square => out.push(p),
            CornerStyle::Chamfer => {
                out.push([p[0] + sz * ein[0], p[1] + sz * ein[1]]);
                out.push([p[0] + sz * eout[0], p[1] + sz * eout[1]]);
            }
            CornerStyle::Round => {
                let cx = p[0] + sz * (ein[0] + eout[0]);
                let cy = p[1] + sz * (ein[1] + eout[1]);
                out.push([p[0] + sz * ein[0], p[1] + sz * ein[1]]);
                let (mut vx, mut vy) = (-sz * eout[0], -sz * eout[1]);
                for _ in 1..seg {
                    let (nx, ny) = (vx * cos_t - vy * sin_t, vx * sin_t + vy * cos_t);
                    vx = nx;
                    vy = ny;
                    out.push([cx + vx, cy + vy]);
                }
                out.push([p[0] + sz * eout[0], p[1] + sz * eout[1]]);
            }
        }
    }
}

/// Two colours mixed in OUTPUT space — the space the rasteriser
/// interpolates in, which is what makes the two-stop gradient exact
/// (r1 §6.1). The `a·(1−u) + b·u` form returns the stops bit-for-bit at
/// u = 0 and u = 1; the endpoint-exactness test relies on that.
fn lerp(a: Color, b: Color, u: f32) -> Color {
    let k = 1.0 - u;
    Color {
        r: a.r * k + b.r * u,
        g: a.g * k + b.g * u,
        b: a.b * k + b.b * u,
        a: a.a * k + b.a * u,
    }
}

/// Sutherland–Hodgman against one gradient-space bound. `t` is affine in
/// position, so interpolating the crossing by `t` is exact: both bands
/// sharing the boundary compute the identical point and the seam cannot
/// crack. Returns the vertex count written into `out`.
fn clip_t(
    input: &[([f32; 2], f32)],
    bound: f32,
    keep_ge: bool,
    out: &mut [([f32; 2], f32); 8],
) -> usize {
    let inside = |t: f32| if keep_ge { t >= bound } else { t <= bound };
    let mut m = 0;
    let n = input.len();
    for i in 0..n {
        let (p0, t0) = input[i];
        let (p1, t1) = input[(i + 1) % n];
        let (in0, in1) = (inside(t0), inside(t1));
        if in0 {
            out[m] = (p0, t0);
            m += 1;
        }
        if in0 != in1 {
            let u = (bound - t0) / (t1 - t0);
            out[m] = (
                [p0[0] + (p1[0] - p0[0]) * u, p0[1] + (p1[1] - p0[1]) * u],
                bound,
            );
            m += 1;
        }
    }
    m
}

/// One contiguous run of vertices sampling one texture: the glyph
/// atlas (`None`) or a registered image. Runs partition the vertex
/// list in emission order, which is what keeps images correctly
/// layered between the things drawn before and after them.
#[derive(Clone, Copy)]
pub struct DrawRun {
    pub image: Option<ImageId>,
    /// One past the run's last vertex; the run starts where the
    /// previous one ended.
    pub end: u32,
    /// Scissor for this run, in device px, already intersected down the
    /// clip stack (r1's R2). None = the whole target. The renderer maps it
    /// to `cmd_set_scissor`, which is already dynamic state — clipping a
    /// ribbon, a scrolling list or the terminal costs nothing per frame.
    pub clip: Option<[f32; 4]>,
}

// ---------------------------------------------------------------------
// The command register: what the caller ASKED FOR, kept beside what the
// tessellator made of it.

/// Where a text command's anchor point sits: [`DrawList::text`] pins the
/// left edge of the box, [`DrawList::text_center`] its middle,
/// [`DrawList::text_right`] its right edge. Three calls, one intent with
/// three anchors — the x they finally hand the glyph loop differs
/// because the measured width differs, what the caller asked for does
/// not.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TextAnchor {
    Left,
    Centre,
    Right,
}

/// One drawing call as the caller MEANT it: the kind, the box, the
/// colour, the corner treatment, the string — and deliberately not the
/// vertices it became.
///
/// The vertex list already proves that two builds tessellate alike. That
/// is the wrong question for a change that is ALLOWED to tessellate
/// differently: an SDF core draws a rounded panel as one quad where the
/// ring generator drew twenty-eight, and a hash of vertices then reports
/// "different frame" for a picture that is identical. The register
/// answers the other question — did the SCENE change — so a commit can
/// state which of the two it is permitted to move: hydraulics under the
/// picture (D0's matrix) moves neither, a tessellation core moves the
/// vertices and not the register, and anything that moves the register
/// moved what the program meant to draw.
///
/// So nothing a tessellator may legitimately choose belongs in here.
/// `segments` is absent from [`DrawCmd::Ring`] because it IS the
/// tessellation knob; the mask band is absent from [`DrawCmd::GlowRing`]
/// and [`DrawCmd::MaskQuad`] because it names texels in an atlas an SDF
/// core has no use for. What a corner is — round, 4 px — is intent; how
/// many chords it takes to draw it is not.
///
/// Rects arrive here as `[x, y, w, h]` whatever the call spelled them,
/// so a rect and a ring over the same box print the same box.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawCmd {
    /// [`DrawList::push_clip`] — the rect the caller ASKED for, not the
    /// intersection the stack made of it. The intersection is a function
    /// of the pushes already in the register, and printing it as well
    /// would report one decision twice.
    ClipPush { r: [f32; 4] },
    ClipPop,
    /// [`DrawList::restore_clips`] — the host putting a foreign drawer's
    /// stack back. Recorded even when it restores what was already
    /// there, because "the host insisted" is the fact worth pinning.
    ClipRestore { stack: Vec<[f32; 4]> },
    Rect { r: [f32; 4], color: Color },
    RectOutline { r: [f32; 4], stroke: f32, color: Color },
    Quad { p: [[f32; 2]; 4], color: Color },
    QuadC { p: [[f32; 2]; 4], c: [Color; 4] },
    Line { from: [f32; 2], to: [f32; 2], stroke: f32, color: Color },
    Polyline { pts: Vec<[f32; 2]>, stroke: f32, color: Color, closed: bool },
    ChamferFrame { r: [f32; 4], cut: f32, stroke: f32, color: Color },
    ChamferFill { r: [f32; 4], cut: f32, color: Color },
    Ring { r: [f32; 4], corners: [Corner; 4], stroke: f32, color: Color },
    /// [`DrawList::ring_grad`] — the same ring under a two-stop gradient.
    /// `dir` is recorded as given, unnormalised: the direction a theme
    /// wrote is what the frame is answerable for, and the normalisation is
    /// a function of it.
    RingGrad {
        r: [f32; 4],
        corners: [Corner; 4],
        stroke: f32,
        near: Color,
        far: Color,
        dir: [f32; 2],
    },
    RingFill { r: [f32; 4], corners: [Corner; 4], color: Color },
    /// [`DrawList::shape`] — the vector family's own entry. `ring` and
    /// `ring_fill` keep their names in the register even when the
    /// vector lane draws them: which lane tessellated is exactly the
    /// knob the register must not see.
    Shape {
        r: [f32; 4],
        corners: [Corner; 4],
        kind: ShapeKind,
        fill: Option<Color>,
        stroke: Option<(f32, Color)>,
        /// The soft profile, when the caller asked for one. `glow_ring`
        /// and `shadow` keep their OWN names in the register whichever
        /// lane draws them, exactly as `ring` does — this slot is for a
        /// caller who spelled the softness through [`DrawList::shape`].
        soft: Option<Soft>,
    },
    GlassFill { r: [f32; 4], corners: [Corner; 4], depth: f32, tint: Color },
    RectGrad { r: [f32; 4], stops: Vec<(f32, Color)>, angle: f32 },
    FanC { centre: [f32; 2], c_centre: Color, rim: Vec<([f32; 2], Color)> },
    Image { r: [f32; 4], id: ImageId, tint: Color },
    ImageUv { r: [f32; 4], uv: [[f32; 2]; 4], id: ImageId, tint: Color },
    Blur { r: [f32; 4], tint: Color },
    MaskQuad { p: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: Color, additive: bool },
    /// [`DrawList::icon_quad`] — an SVG icon's own atlas rect (K8),
    /// resolved by [`crate::font::FontSystem::icon`] and sampled here
    /// exactly as a glyph run is. `icon` prints the id the caller asked
    /// for, not the atlas texels it happened to land at: a re-run of the
    /// same frame that shelf-packed differently must still print the
    /// same line.
    IconQuad { p: [[f32; 2]; 4], icon: u32, color: Color },
    /// [`DrawList::glow_ring_with`] — `profile` is INTENT (a theme named
    /// a shape for the light) and belongs here whole, band count
    /// included: the count is a token like the other three, and the
    /// picture is a different curve at a different one. What stays out is
    /// `segments`, which is tessellation of the PATH and answers to
    /// [`DrawCmd::Ring`]'s own rule.
    GlowRing {
        r: [f32; 4],
        corners: [Corner; 4],
        radius: f32,
        color: Color,
        profile: GlowProfile,
    },
    SoftBox { r: [f32; 4], radius: f32, color: Color },
    Shadow {
        r: [f32; 4],
        corners: [Corner; 4],
        offset: [f32; 2],
        radius: f32,
        color: Color,
    },
    Text {
        at: [f32; 2],
        anchor: TextAnchor,
        font: u8,
        px: f32,
        tracking: f32,
        /// The figure box this run was set under, 0.0 for a proportional
        /// one. It belongs in the register because it is GEOMETRY: two
        /// runs of the same string at the same px occupy different widths
        /// depending on it, and a register that cannot tell them apart
        /// cannot witness the feature it is here to witness.
        tabular: f32,
        color: Color,
        text: String,
    },
    ModuleTitle {
        at: [f32; 2],
        w: f32,
        px: f32,
        color: Color,
        underline: bool,
        left: String,
        right: String,
    },
}

/// Decimals for a length in pixels: a thousandth, the grain the frame
/// hash already rounds to — fine enough that nothing an eye or a pixel
/// grid can hold is lost, coarse enough that a compiler reassociating a
/// multiply cannot make two identical scenes disagree.
const PX: usize = 3;
/// Decimals for a colour channel. A ten-thousandth is finer than the
/// 8-bit output can carry (1/255 ≈ 0.0039), so every difference that
/// can reach a pixel survives and the float noise under it does not.
const CH: usize = 4;
/// Decimals for the unit-interval and angular quantities — texture
/// coordinates, gradient stop positions, radians. A millionth of a
/// radian moves a point a five-hundredth of a pixel across a 2000 px
/// window: just under the pixel grain, which is where this grain
/// belongs.
const FINE: usize = 6;

/// One number at a FIXED number of decimals.
///
/// Fixed precision is the whole point: `{}` on an f32 prints the
/// shortest text that round-trips, so 0.1 and 0.1 + 1e-9 print
/// differently and two runs of the same scene could disagree over a bit
/// no pixel can show. Quantising first and printing a fixed width makes
/// the text a FUNCTION of the picture instead of the float.
fn num(f: &mut fmt::Formatter<'_>, v: f32, places: usize) -> fmt::Result {
    if !v.is_finite() {
        // The three ways a frame goes wrong here stay distinguishable
        // instead of all arriving as some rounded number.
        return f.write_str(if v.is_nan() {
            "nan"
        } else if v > 0.0 {
            "inf"
        } else {
            "-inf"
        });
    }
    let scale = 10f64.powi(places as i32);
    let q = (v as f64 * scale).round() / scale;
    // Negative zero and a value that rounded down to zero must print
    // alike: -0.0 + 0.0 is +0.0 under round-to-nearest, and two runs
    // that differ only in a sign bit no eye can see are one frame.
    write!(f, "{:.*}", places, q + 0.0)
}

fn nums(f: &mut fmt::Formatter<'_>, vs: &[f32], places: usize) -> fmt::Result {
    for v in vs {
        f.write_str(" ")?;
        num(f, *v, places)?;
    }
    Ok(())
}

/// One named number, ` name value` — the shape every scalar field on a
/// command line takes, so a reader (and a `grep`) can find one by name
/// instead of by counting columns.
fn field(f: &mut fmt::Formatter<'_>, name: &str, v: f32, places: usize) -> fmt::Result {
    write!(f, " {name} ")?;
    num(f, v, places)
}

fn rgba(f: &mut fmt::Formatter<'_>, c: Color) -> fmt::Result {
    f.write_str(" rgba")?;
    nums(f, &c.to_array(), CH)
}

fn points(f: &mut fmt::Formatter<'_>, p: &[[f32; 2]]) -> fmt::Result {
    for q in p {
        nums(f, q, PX)?;
    }
    Ok(())
}

fn uvs(f: &mut fmt::Formatter<'_>, uv: &[[f32; 2]; 4]) -> fmt::Result {
    f.write_str(" uv")?;
    for q in uv {
        nums(f, q, FINE)?;
    }
    Ok(())
}

/// One corner as `style:size`, except that a Square corner prints its
/// style alone — `ring_points` ignores the size of a Square, so a stray
/// size there draws nothing, and two commands that draw the same picture
/// must print the same line.
fn corner(f: &mut fmt::Formatter<'_>, c: Corner) -> fmt::Result {
    match c.style {
        CornerStyle::Square => f.write_str(" square"),
        CornerStyle::Round => {
            f.write_str(" round:")?;
            num(f, c.size, PX)
        }
        CornerStyle::Chamfer => {
            f.write_str(" chamfer:")?;
            num(f, c.size, PX)
        }
    }
}

fn corners(f: &mut fmt::Formatter<'_>, c: &[Corner; 4]) -> fmt::Result {
    f.write_str(" corners")?;
    for k in c {
        corner(f, *k)?;
    }
    Ok(())
}

/// A string as ONE token: quoted, and escaped so that no content can
/// smuggle a line break, a quote or a control character into a dump that
/// is compared line by line.
fn quoted(f: &mut fmt::Formatter<'_>, s: &str) -> fmt::Result {
    f.write_str("\"")?;
    for ch in s.chars() {
        match ch {
            '"' => f.write_str("\\\"")?,
            '\\' => f.write_str("\\\\")?,
            '\n' => f.write_str("\\n")?,
            '\r' => f.write_str("\\r")?,
            '\t' => f.write_str("\\t")?,
            c if (c as u32) < 0x20 || c as u32 == 0x7f => write!(f, "\\u{{{:x}}}", c as u32)?,
            c => write!(f, "{c}")?,
        }
    }
    f.write_str("\"")
}

/// One command as one line, no trailing newline — the register's
/// canonical form. The consumer numbers the lines; two dumps of the same
/// scene are byte-for-byte equal, so the text itself is what a guard
/// compares or hashes, and no second rounding rule is needed anywhere
/// downstream.
impl fmt::Display for DrawCmd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DrawCmd::ClipPush { r } => {
                f.write_str("clip push")?;
                nums(f, r, PX)
            }
            DrawCmd::ClipPop => f.write_str("clip pop"),
            DrawCmd::ClipRestore { stack } => {
                write!(f, "clip restore {}", stack.len())?;
                for r in stack {
                    nums(f, r, PX)?;
                }
                Ok(())
            }
            DrawCmd::Rect { r, color } => {
                f.write_str("rect at")?;
                nums(f, r, PX)?;
                rgba(f, *color)
            }
            DrawCmd::RectOutline { r, stroke, color } => {
                f.write_str("rect_outline at")?;
                nums(f, r, PX)?;
                field(f, "stroke", *stroke, PX)?;
                rgba(f, *color)
            }
            DrawCmd::Quad { p, color } => {
                f.write_str("quad p")?;
                points(f, p)?;
                rgba(f, *color)
            }
            DrawCmd::QuadC { p, c } => {
                f.write_str("quad_c p")?;
                points(f, p)?;
                for k in c {
                    rgba(f, *k)?;
                }
                Ok(())
            }
            DrawCmd::Line { from, to, stroke, color } => {
                f.write_str("line from")?;
                nums(f, from, PX)?;
                f.write_str(" to")?;
                nums(f, to, PX)?;
                field(f, "stroke", *stroke, PX)?;
                rgba(f, *color)
            }
            DrawCmd::Polyline { pts, stroke, color, closed } => {
                write!(f, "polyline {}", pts.len())?;
                points(f, pts)?;
                field(f, "stroke", *stroke, PX)?;
                rgba(f, *color)?;
                f.write_str(if *closed { " closed" } else { " open" })
            }
            DrawCmd::ChamferFrame { r, cut, stroke, color } => {
                f.write_str("chamfer_frame at")?;
                nums(f, r, PX)?;
                field(f, "cut", *cut, PX)?;
                field(f, "stroke", *stroke, PX)?;
                rgba(f, *color)
            }
            DrawCmd::ChamferFill { r, cut, color } => {
                f.write_str("chamfer_fill at")?;
                nums(f, r, PX)?;
                field(f, "cut", *cut, PX)?;
                rgba(f, *color)
            }
            DrawCmd::Ring { r, corners: c, stroke, color } => {
                f.write_str("ring at")?;
                nums(f, r, PX)?;
                corners(f, c)?;
                field(f, "stroke", *stroke, PX)?;
                rgba(f, *color)
            }
            DrawCmd::RingGrad { r, corners: c, stroke, near, far, dir } => {
                f.write_str("ring_grad at")?;
                nums(f, r, PX)?;
                corners(f, c)?;
                field(f, "stroke", *stroke, PX)?;
                f.write_str(" near")?;
                rgba(f, *near)?;
                f.write_str(" far")?;
                rgba(f, *far)?;
                f.write_str(" dir")?;
                nums(f, dir, FINE)
            }
            DrawCmd::RingFill { r, corners: c, color } => {
                f.write_str("ring_fill at")?;
                nums(f, r, PX)?;
                corners(f, c)?;
                rgba(f, *color)
            }
            DrawCmd::Shape { r, corners: c, kind, fill, stroke, soft } => {
                f.write_str("shape at")?;
                nums(f, r, PX)?;
                corners(f, c)?;
                // The payload prints with the kind: the register holds
                // INTENT (level A), and a hexagon's turn is as much of
                // the caller's intent as a corner's radius is. Box says
                // exactly what it said before K6, because Box carries
                // nothing.
                match kind {
                    ShapeKind::Box => f.write_str(" kind box")?,
                    ShapeKind::Ring { width, half_sweep, dir } => {
                        f.write_str(" kind ring")?;
                        field(f, "width", *width, PX)?;
                        field(f, "half_sweep", *half_sweep, FINE)?;
                        field(f, "dir", *dir, FINE)?;
                    }
                    ShapeKind::Hex { turn } => {
                        f.write_str(" kind hex")?;
                        field(f, "turn", *turn, FINE)?;
                    }
                    ShapeKind::Chevron { left, right } => {
                        f.write_str(" kind chevron")?;
                        field(f, "left", *left, PX)?;
                        field(f, "right", *right, PX)?;
                    }
                    ShapeKind::Capsule => f.write_str(" kind capsule")?,
                }
                if let Some(col) = fill {
                    f.write_str(" fill")?;
                    rgba(f, *col)?;
                }
                if let Some((w, col)) = stroke {
                    field(f, "stroke", *w, PX)?;
                    rgba(f, *col)?;
                }
                if let Some(soft) = soft {
                    f.write_str(match soft.kind {
                        SoftKind::Glow => " glow",
                        SoftKind::Shadow => " shadow",
                    })?;
                    field(f, "reach", soft.reach, PX)?;
                }
                Ok(())
            }
            DrawCmd::RectGrad { r, stops, angle } => {
                f.write_str("rect_grad at")?;
                nums(f, r, PX)?;
                field(f, "angle", *angle, FINE)?;
                write!(f, " stops {}", stops.len())?;
                for (t, c) in stops {
                    f.write_str(" ")?;
                    num(f, *t, FINE)?;
                    rgba(f, *c)?;
                }
                Ok(())
            }
            DrawCmd::FanC { centre, c_centre, rim } => {
                f.write_str("fan_c centre")?;
                nums(f, centre, PX)?;
                rgba(f, *c_centre)?;
                write!(f, " rim {}", rim.len())?;
                for (p, c) in rim {
                    nums(f, p, PX)?;
                    rgba(f, *c)?;
                }
                Ok(())
            }
            DrawCmd::Image { r, id, tint } => {
                f.write_str("image at")?;
                nums(f, r, PX)?;
                write!(f, " id {}", id.0)?;
                rgba(f, *tint)
            }
            DrawCmd::ImageUv { r, uv, id, tint } => {
                f.write_str("image_uv at")?;
                nums(f, r, PX)?;
                uvs(f, uv)?;
                write!(f, " id {}", id.0)?;
                rgba(f, *tint)
            }
            DrawCmd::Blur { r, tint } => {
                f.write_str("blur at")?;
                nums(f, r, PX)?;
                rgba(f, *tint)
            }
            DrawCmd::GlassFill { r, corners: c, depth, tint } => {
                f.write_str("glass_fill at")?;
                nums(f, r, PX)?;
                corners(f, c)?;
                field(f, "depth", *depth, FINE)?;
                rgba(f, *tint)
            }
            DrawCmd::MaskQuad { p, uv, color, additive } => {
                f.write_str("mask_quad p")?;
                points(f, p)?;
                uvs(f, uv)?;
                rgba(f, *color)?;
                f.write_str(if *additive { " add" } else { " cover" })
            }
            DrawCmd::IconQuad { p, icon, color } => {
                f.write_str("icon_quad p")?;
                points(f, p)?;
                write!(f, " icon {icon}")?;
                rgba(f, *color)
            }
            DrawCmd::GlowRing { r, corners: c, radius, color, profile } => {
                f.write_str("glow_ring at")?;
                nums(f, r, PX)?;
                corners(f, c)?;
                field(f, "radius", *radius, PX)?;
                rgba(f, *color)?;
                // The unshaped disk prints nothing, so every line this
                // register has ever written for a glow still reads the
                // same. A profile only appears once a theme has asked
                // for one, which is also the only time it is news.
                if profile.is_halo() {
                    return Ok(());
                }
                field(f, "decay", profile.decay, FINE)?;
                field(f, "aura", profile.aura, FINE)?;
                field(f, "aura_reach", profile.aura_reach, FINE)
            }
            DrawCmd::SoftBox { r, radius, color } => {
                f.write_str("soft_box at")?;
                nums(f, r, PX)?;
                field(f, "radius", *radius, PX)?;
                rgba(f, *color)
            }
            DrawCmd::Shadow { r, corners: c, offset, radius, color } => {
                f.write_str("shadow at")?;
                nums(f, r, PX)?;
                corners(f, c)?;
                f.write_str(" offset")?;
                nums(f, offset, PX)?;
                field(f, "radius", *radius, PX)?;
                rgba(f, *color)
            }
            DrawCmd::Text { at, anchor, font, px, tracking, tabular, color, text } => {
                f.write_str("text at")?;
                nums(f, at, PX)?;
                f.write_str(match anchor {
                    TextAnchor::Left => " anchor left",
                    TextAnchor::Centre => " anchor centre",
                    TextAnchor::Right => " anchor right",
                })?;
                write!(f, " font {font}")?;
                field(f, "px", *px, PX)?;
                field(f, "track", *tracking, PX)?;
                // Written only when there IS a box, so a proportional run
                // dumps the line it has always dumped and the corpus of
                // recorded images stays comparable across this change.
                if *tabular > 0.0 {
                    field(f, "figure", *tabular, PX)?;
                }
                rgba(f, *color)?;
                f.write_str(" ")?;
                quoted(f, text)
            }
            DrawCmd::ModuleTitle { at, w, px, color, underline, left, right } => {
                f.write_str("module_title at")?;
                nums(f, at, PX)?;
                field(f, "w", *w, PX)?;
                field(f, "px", *px, PX)?;
                rgba(f, *color)?;
                f.write_str(if *underline { " rule" } else { " no_rule" })?;
                f.write_str(" left ")?;
                quoted(f, left)?;
                f.write_str(" right ")?;
                quoted(f, right)
            }
        }
    }
}

/// The register's switch, resolved once: 0 unread, 1 off, 2 on.
static CMD_REGISTER: AtomicU8 = AtomicU8::new(0);

/// What a value of `NACELLE_DRAW_CMDS` means. Pure, so the parsing is
/// testable — the reader below can only be exercised once per process.
fn armed_by(v: Option<&str>) -> bool {
    matches!(v, Some(v) if !v.is_empty() && v != "0")
}

/// Whether lists made from here on record their commands.
/// `NACELLE_DRAW_CMDS` arms the register and nothing else does; unarmed
/// is the shipping case and costs a relaxed load per list, per frame.
pub fn cmds_armed() -> bool {
    match CMD_REGISTER.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = armed_by(std::env::var("NACELLE_DRAW_CMDS").ok().as_deref());
            CMD_REGISTER.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Arms the register whatever the environment says — for an application
/// that has a switch of its own and wants the register to follow it. One
/// way on purpose: an armed run is a measurement, not a session, and
/// half a measured frame is worth nothing.
pub fn arm_cmds() {
    CMD_REGISTER.store(2, Ordering::Relaxed);
}

pub struct DrawList {
    pub verts: Vec<Vertex>,
    pub runs: Vec<DrawRun>,
    /// The frame's shape records (f3 §2.5): what runs tagged [`SHAPE`]
    /// index into through [`Vertex::shape`]. Uploaded beside the
    /// vertices, cleared with them.
    shapes: Vec<Shape>,
    /// Whether ring/ring_fill take the vector lane. The application
    /// sets it from the theme's `render.vector`; a fresh list starts
    /// `false` — the tessellated path bit for bit — until something arms
    /// it, and `true` is the shipping theme's own default as of K3d
    /// (2026-08-23).
    vector: bool,
    /// §2.3's ride subdivision: shape quads emit as warp×warp grids
    /// while a post-emission transform is in flight. 1 = one quad, and
    /// only there does §2.7's edge snap apply.
    warp: u8,
    /// The bed the vector lane last wrote, still open for the border
    /// that belongs to it (§2.10).
    weld: Option<Weld>,
    /// The clip stack: pushes intersect, pops restore. The TOP is stamped
    /// onto every run the moment it is opened.
    clips: Vec<[f32; 4]>,
    /// Reused ring-point buffers (r1 §5.3): the generators borrow them via
    /// mem::take so a ring costs no allocation after the first frame.
    scratch_a: Vec<[f32; 2]>,
    scratch_b: Vec<[f32; 2]>,
    /// The command register, absent unless armed. `None` is a null
    /// pointer's worth of state and no allocation at all: an unarmed
    /// frame pays one branch per drawing call and never builds a
    /// command, which is why the strings and point lists in [`DrawCmd`]
    /// cost a shipping run nothing.
    cmds: Option<Vec<DrawCmd>>,
}

impl DrawList {
    pub fn new() -> Self {
        DrawList {
            verts: Vec::with_capacity(1 << 16),
            runs: Vec::new(),
            shapes: Vec::new(),
            vector: false,
            warp: 1,
            weld: None,
            clips: Vec::new(),
            scratch_a: Vec::new(),
            scratch_b: Vec::new(),
            cmds: cmds_armed().then(Vec::new),
        }
    }

    /// A list that records its commands whatever the environment says —
    /// the door the guard's own tests come in by, and an application
    /// that arms one list without arming the process.
    pub fn recording() -> Self {
        DrawList { cmds: Some(Vec::new()), ..DrawList::new() }
    }

    pub fn clear(&mut self) {
        self.verts.clear();
        self.runs.clear();
        self.shapes.clear();
        // The warp is FRAME state — a ride that forgot to lower it must
        // not thicken every later frame — while `vector` is a mode, set
        // once from the theme, and survives like the register does.
        self.warp = 1;
        // The records it indexed are gone; an open weld across a frame
        // boundary would name a record that no longer exists.
        self.weld = None;
        self.clips.clear();
        match &mut self.cmds {
            Some(cmds) => cmds.clear(),
            // A list built before the register was armed picks it up at
            // the frame boundary, so an application is free to read its
            // own switch after it has made its list. Never the other
            // way: arming is one-way, so a list that records keeps
            // recording.
            none => *none = cmds_armed().then(Vec::new),
        }
    }

    /// The commands this frame asked for, in call order — empty when the
    /// register is off. One line each through [`DrawCmd`]'s `Display`.
    pub fn cmds(&self) -> &[DrawCmd] {
        self.cmds.as_deref().unwrap_or(&[])
    }

    /// Whether this list records commands at all. `cmds().is_empty()`
    /// cannot answer that — an armed frame that drew nothing looks the
    /// same.
    pub fn is_recording(&self) -> bool {
        self.cmds.is_some()
    }

    /// Records one command, if this list records at all.
    ///
    /// The closure is what makes the unarmed case free: a text call
    /// would otherwise copy its string sixty times a second for nobody,
    /// and a polyline its points. Off, this is one branch on a pointer.
    #[inline]
    fn cmd(&mut self, f: impl FnOnce() -> DrawCmd) {
        if let Some(cmds) = &mut self.cmds {
            cmds.push(f());
        }
    }

    /// Clip everything drawn until the matching pop to this rect,
    /// intersected with whatever is already clipping. Unbalanced pushes are
    /// forgiven at clear() — a widget that early-returns must not wedge the
    /// whole frame.
    pub fn push_clip(&mut self, x: f32, y: f32, w: f32, h: f32) {
        self.cmd(|| DrawCmd::ClipPush { r: [x, y, w, h] });
        let new = match self.clips.last() {
            Some(&[cx, cy, cw, ch]) => {
                let x0 = x.max(cx);
                let y0 = y.max(cy);
                let x1 = (x + w).min(cx + cw);
                let y1 = (y + h).min(cy + ch);
                [x0, y0, (x1 - x0).max(0.0), (y1 - y0).max(0.0)]
            }
            None => [x, y, w.max(0.0), h.max(0.0)],
        };
        self.clips.push(new);
        // A clip change is a run boundary even for the same texture.
        self.runs.push(DrawRun {
            image: self.runs.last().and_then(|r| r.image),
            end: self.verts.len() as u32,
            clip: Some(new),
        });
    }

    pub fn pop_clip(&mut self) {
        self.cmd(|| DrawCmd::ClipPop);
        self.clips.pop();
        let clip = self.clips.last().copied();
        self.runs.push(DrawRun {
            image: self.runs.last().and_then(|r| r.image),
            end: self.verts.len() as u32,
            clip,
        });
    }

    /// The clip in force right now: the TOP of the stack, which is
    /// already the intersection of everything pushed. `None` = nothing
    /// clips.
    ///
    /// For an object that hands rectangles back to its caller to
    /// hit-test: what the object DRAWS is cut by this scissor, so what
    /// it REPORTS has to be cut by the same one, or an element the
    /// scissor took would still answer the mouse.
    /// [`crate::object::dropdown::accordion`] is the reader.
    pub fn clip(&self) -> Option<[f32; 4]> {
        self.clips.last().copied()
    }

    /// The clip stack as it stands. The host takes one of these before
    /// handing the list to a foreign drawer (a plugin across the ABI)
    /// and puts it back with [`DrawList::restore_clips`] afterwards: a
    /// plugin that pushes without popping — or pops what it never
    /// pushed — must not decide what its NEIGHBOURS are clipped to.
    /// Costs no allocation in the ordinary case, where the stack is
    /// empty.
    pub fn clip_stack(&self) -> Vec<[f32; 4]> {
        self.clips.clone()
    }

    /// How many runs the list has recorded — the renderer's draw calls,
    /// and the cheapest measure of "did that change the state the runs
    /// carry?".
    pub fn run_count(&self) -> usize {
        self.runs.len()
    }

    /// Forces the clip stack back to `saved`. The rectangles are already
    /// intersected — this list produced them — so nothing is intersected
    /// again. A caller that left the stack as it found it costs one
    /// comparison and stamps no run.
    pub fn restore_clips(&mut self, saved: &[[f32; 4]]) {
        self.cmd(|| DrawCmd::ClipRestore { stack: saved.to_vec() });
        if self.clips == saved {
            return;
        }
        self.clips.clear();
        self.clips.extend_from_slice(saved);
        let clip = self.clips.last().copied();
        self.runs.push(DrawRun {
            image: self.runs.last().and_then(|r| r.image),
            end: self.verts.len() as u32,
            clip,
        });
    }

    /// Makes sure the vertices about to be pushed extend a run that
    /// samples `image`, starting a new run when the texture changes.
    fn run_for(&mut self, image: Option<ImageId>) {
        let clip = self.clips.last().copied();
        match self.runs.last_mut() {
            Some(run) if run.image == image && run.clip == clip => {}
            _ => self.runs.push(DrawRun { image, end: self.verts.len() as u32, clip }),
        }
    }

    fn seal(&mut self) {
        if let Some(run) = self.runs.last_mut() {
            run.end = self.verts.len() as u32;
        }
    }

    /// One quad, any binding, a colour per vertex — every shape above the
    /// glyph level funnels into here. `Vertex.color` was always interpolated
    /// by the rasteriser; the list simply never exposed it (r1 §0, fact 1).
    fn push_quad4(
        &mut self,
        image: Option<ImageId>,
        p: [[f32; 2]; 4],
        uv: [[f32; 2]; 4],
        c: [[f32; 4]; 4],
    ) {
        self.run_for(image);
        let v = |i: usize| Vertex { pos: p[i], uv: uv[i], color: c[i], shape: NO_SHAPE };
        self.verts.extend_from_slice(&[v(0), v(1), v(2), v(0), v(2), v(3)]);
        self.seal();
    }

    /// One triangle over the atlas white pixel, a colour per vertex — the
    /// fan primitives' unit.
    fn push_tri_c(&mut self, image: Option<ImageId>, p: [[f32; 2]; 3], c: [[f32; 4]; 3]) {
        self.run_for(image);
        let (u, v) = FontSystem::white_uv();
        let vx = |i: usize| Vertex { pos: p[i], uv: [u, v], color: c[i], shape: NO_SHAPE };
        self.verts.extend_from_slice(&[vx(0), vx(1), vx(2)]);
        self.seal();
    }

    fn push_quad(&mut self, p: [[f32; 2]; 4], uv: [[f32; 2]; 4], color: Color) {
        let c = color.to_array();
        self.push_quad4(None, p, uv, [c; 4]);
    }

    /// A rectangle filled with a registered image, whole. The color
    /// multiplies the image — white leaves it as it is, the alpha
    /// fades it.
    pub fn image(&mut self, x: f32, y: f32, w: f32, h: f32, id: ImageId, tint: Color) {
        self.cmd(|| DrawCmd::Image { r: [x, y, w, h], id, tint });
        self.run_for(Some(id));
        let c = tint.to_array();
        let p = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        let uv = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        let v = |i: usize| Vertex { pos: p[i], uv: uv[i], color: c, shape: NO_SHAPE };
        self.verts.extend_from_slice(&[v(0), v(1), v(2), v(0), v(2), v(3)]);
        self.seal();
    }

    /// Frosted glass over the given rectangle: what was drawn before
    /// the first glass quad this frame shows through it blurred,
    /// tinted by `tint` (white leaves the blur as it is). The renderer
    /// samples by SCREEN position, so these vertices may be translated
    /// afterwards — an animation can carry the glass around and the
    /// frost stays put on the picture beneath.
    pub fn blur(&mut self, x: f32, y: f32, w: f32, h: f32, tint: Color) {
        self.cmd(|| DrawCmd::Blur { r: [x, y, w, h], tint });
        self.run_for(Some(BLUR_IMAGE));
        let c = tint.to_array();
        let (u, v) = FontSystem::white_uv();
        let p = [[x, y], [x + w, y], [x + w, y + h], [x, y + h]];
        let vx = |i: usize| Vertex { pos: p[i], uv: [u, v], color: c, shape: NO_SHAPE };
        self.verts
            .extend_from_slice(&[vx(0), vx(1), vx(2), vx(0), vx(2), vx(3)]);
        self.seal();
    }

    /// Arbitrary quadrilateral (vertices along the perimeter).
    ///
    /// **K4 leaves this hard, and the census is the argument** (f3 §3.2,
    /// counted across libnacelle, nacelle-desktop and nacelle-addons on
    /// 2026-08-17). A filled polygon is not in the vector family: its
    /// silhouette is a LIST OF POINTS, not a formula, so the field
    /// cannot draw it and §3.2's answer is a different mechanism
    /// altogether — a one-pixel `fringe()` band along the true boundary,
    /// alpha 1 inside and 0 out, Gouraud-interpolated by the rasteriser.
    /// That mechanism was not built, and these are the reasons:
    ///
    /// * **The live callers are TRAPEZOIDS**, which no affine frame can
    ///   reach: `winframe.rs:415` (the title band's 45° shoulders) and
    ///   `:432` (the body below it). A parallelogram would have been
    ///   free — an affine image of a box is still a box read in oblique
    ///   axes, and [`crate::sdf::Frame`] already carries one — but a
    ///   trapezoid's two ends are not parallel and there is no such map.
    /// * **The parallelogram callers are switched OFF in the shipped
    ///   theme**: `tabs.rs:351` and `focus_ring.rs:139` draw one only
    ///   when `tab.skew` or `button.skew` is a length, and the master
    ///   sets both to `0u` (`default.theme:4401`, `:4602`) — a slanted
    ///   control is a theme's choice and nobody's default.
    /// * **The remaining caller is the ABI tunnel** (`plugin.rs:99`),
    ///   where the toolkit cannot know what the polygon means.
    ///
    /// So the shape that would pay for a fringe is drawn by nobody, and
    /// the shape that is drawn cannot use one. The mechanism waits for
    /// a caller that needs it — and when it comes, the parallelogram is
    /// the cheap half and the trapezoid the one that needs the band.
    /// Meanwhile the OUTLINE of a slanted control is already smooth:
    /// `polyline` strokes it, and every diagonal arm of it is a shape.
    pub fn quad(&mut self, p: [[f32; 2]; 4], color: Color) {
        self.cmd(|| DrawCmd::Quad { p, color });
        self.quad_verts(p, color);
    }

    /// The vertices of [`DrawList::quad`] without the command.
    ///
    /// This is the shape of every shape here that is built out of
    /// another one: the PUBLIC name records the caller's intent and then
    /// calls a `_verts` twin, and the shapes above it call the twin.
    /// Otherwise a rect outline would enter the register as an outline
    /// AND four rects, and the day a tessellation core stops cutting it
    /// into four the register would report a scene change where the
    /// scene never moved. What this file decomposes a shape into is
    /// exactly what the register must not see.
    fn quad_verts(&mut self, p: [[f32; 2]; 4], color: Color) {
        let (u, v) = FontSystem::white_uv();
        self.push_quad(p, [[u, v]; 4], color);
    }

    /// An axis-aligned rectangle, hard-edged — and it stays that way on
    /// every lane (f3 §3.3, and §2.7 before it).
    ///
    /// This is the TERMINAL's own primitive: the shell plugin draws
    /// every cell background, the cursor and the selection through
    /// `HostApi::rect`, and none of them may ever reach the field.
    /// Three reasons, none of them about cost alone. Cell backgrounds
    /// touch side by side, so an antialiased edge is a SEAM — two
    /// neighbours at half coverage draw a lattice across the whole
    /// screen. Cells sit on integer coordinates by construction, so
    /// there is nothing to smooth. And an 80×24 grid is 1 920 records a
    /// frame, 115 000 a second, for a picture that was already exact.
    pub fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.cmd(|| DrawCmd::Rect { r: [x, y, w, h], color });
        self.rect_verts(x, y, w, h, color);
    }

    fn rect_verts(&mut self, x: f32, y: f32, w: f32, h: f32, color: Color) {
        self.quad_verts([[x, y], [x + w, y], [x + w, y + h], [x, y + h]], color);
    }

    pub fn rect_outline(&mut self, x: f32, y: f32, w: f32, h: f32, t: f32, color: Color) {
        self.cmd(|| DrawCmd::RectOutline { r: [x, y, w, h], stroke: t, color });
        self.rect_verts(x, y, w, t, color);
        self.rect_verts(x, y + h - t, w, t, color);
        self.rect_verts(x, y + t, t, h - 2.0 * t, color);
        self.rect_verts(x + w - t, y + t, t, h - 2.0 * t, color);
    }

    /// One straight segment `t` px wide, ends cut square across the
    /// path — and no end treatment, on purpose.
    ///
    /// A rounded end was asked for here because `slider.track_corner =
    /// @corner.pill` drew a square-ended bar, but the track is not a line
    /// with caps: it is a BOX with corners, and the theme says so in the
    /// word it uses. `*.corner` is a rect-corner token with a
    /// `*_corner_style` sibling deciding the cut; there is no `*_cap` key
    /// in the whole master, so capping a segment would answer a corner
    /// token with a vocabulary the theme cannot spell. The capsule
    /// therefore belongs to [`DrawList::ring_fill`], which already takes
    /// four [`Corner`]s and already tessellates them — see
    /// [`Corner::sized`], which is the piece that was actually missing.
    ///
    /// A cap here would also cost what this file protects: `polyline`
    /// builds every chart stroke out of `line_verts`, so a cap on the
    /// segment would double-draw at each joint, which is exactly what
    /// `ring`'s watertight band exists to avoid and what additive runs
    /// show as a bright pip. The corner of a PATH is a separate disc,
    /// laid once by [`DrawList::joints`] where the path actually turns
    /// (f3 §3.1) — never by the segment, which cannot know whether
    /// anything follows it.
    pub fn line(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, t: f32, color: Color) {
        self.cmd(|| DrawCmd::Line {
            from: [x0, y0],
            to: [x1, y1],
            stroke: t,
            color,
        });
        self.line_verts(x0, y0, x1, y1, t, color);
    }

    fn line_verts(&mut self, x0: f32, y0: f32, x1: f32, y1: f32, t: f32, color: Color) {
        // f3 §K4 — THE DIAGONAL GOES TO THE FIELD, THE AXIS DOES NOT.
        //
        // An axis-aligned segment is a rectangle on the screen's own
        // grid, and §2.7 has already ruled on those: a rule, an
        // underline, a table guide and a grid line are the interface's
        // own edges, the raster puts them exactly where they belong,
        // and a coverage ramp could only smear them across two half-lit
        // pixels. They stay quads, at four vertices and no record, bit
        // for bit what they are today. That is most of this method's
        // callers by count (`ui.rs`, `list.rs`, `tabs.rs`, `panel.rs`,
        // the editor's grid) and all of its cheap ones.
        //
        // A DIAGONAL is the case the raster cannot hold: it has no
        // representation on the grid at all, and what it draws is the
        // staircase MSAA was covering for. Those go to the field — the
        // tick and the cross of every checkbox, the menu's chevron, the
        // sort arrow and the disclosure triangle of every list, the
        // dashes of a focus ring on a slanted control, every icon and
        // every chart a plugin draws through the ABI's `polyline`.
        if self.vector && x0 != x1 && y0 != y1 && self.segment_verts([x0, y0], [x1, y1], t, color) {
            return;
        }
        let dx = x1 - x0;
        let dy = y1 - y0;
        let len = (dx * dx + dy * dy).sqrt().max(0.0001);
        let nx = -dy / len * t * 0.5;
        let ny = dx / len * t * 0.5;
        self.quad_verts(
            [
                [x0 + nx, y0 + ny],
                [x1 + nx, y1 + ny],
                [x1 - nx, y1 - ny],
                [x0 - nx, y0 - ny],
            ],
            color,
        );
    }

    pub fn polyline(&mut self, pts: &[[f32; 2]], t: f32, color: Color, closed: bool) {
        self.cmd(|| DrawCmd::Polyline {
            pts: pts.to_vec(),
            stroke: t,
            color,
            closed,
        });
        if pts.len() < 2 {
            return;
        }
        for w in pts.windows(2) {
            self.line_verts(w[0][0], w[0][1], w[1][0], w[1][1], t, color);
        }
        if closed {
            let a = pts[pts.len() - 1];
            let b = pts[0];
            self.line_verts(a[0], a[1], b[0], b[1], t, color);
        }
        if self.vector {
            self.joints(pts, t, color, closed);
        }
    }

    /// The joints of a stroked path on the vector lane (f3 §3.1): one
    /// disc of radius `t/2` at every corner where the field draws BOTH
    /// sides of the turn.
    ///
    /// Two butt-capped segments meeting at an angle leave the outer
    /// wedge empty. The hard raster left it empty too — it is the notch
    /// on every triangle and every tick the toolkit draws — and
    /// antialiasing does not hide it, it draws it accurately. The disc
    /// closes it, and the union becomes the round join a stroked path
    /// is supposed to have. Measured against the true stroke — the set
    /// of points within `t/2` of the path — the bare pair is a whole
    /// pixel short at the corner and the disc lands within a tenth
    /// (`sdf::tests::a_joint_disc_closes_the_notch_the_two_segments_leave`).
    ///
    /// **Both sides, and the reason.** Where one arm is axis-aligned
    /// its edge is hard and exactly on the grid; a disc would lay a
    /// soft half-pixel halo along it and move a picture §2.7 promised
    /// not to move. Those joints stay exactly as they are today. So do
    /// straight ones — a disc on a path that does not turn is ink for
    /// nothing.
    ///
    /// The disc DOES lie over the two segments it joins, and §3.1's
    /// claim that nothing overlaps is not true of this decomposition.
    /// For an opaque stroke that is invisible; for a translucent one it
    /// is a bounded double blend — six square pixels of extra ink on a
    /// seven-pixel stroke at half alpha, a sixth of the disc's own
    /// area, all of it inside the silhouette. The alternative —
    /// shortening each arm by `t/2` so nothing overlaps — leaves the
    /// crescent between a flat cap and the circle tangent to it EMPTY,
    /// at every angle, turn or no turn. The same test measures that
    /// too: a hole is worse than a hot spot.
    fn joints(&mut self, pts: &[[f32; 2]], t: f32, color: Color, closed: bool) {
        let n = pts.len();
        let diagonal = |a: [f32; 2], b: [f32; 2]| a[0] != b[0] && a[1] != b[1];
        let mut corner = |a: [f32; 2], v: [f32; 2], b: [f32; 2]| {
            if !diagonal(a, v) || !diagonal(v, b) {
                return;
            }
            let (e0, e1) = ([v[0] - a[0], v[1] - a[1]], [b[0] - v[0], b[1] - v[1]]);
            // A path that runs straight through has nothing to join.
            if e0[0] * e1[1] - e0[1] * e1[0] == 0.0 {
                return;
            }
            self.joint_verts(v, t, color);
        };
        for w in pts.windows(3) {
            corner(w[0], w[1], w[2]);
        }
        if closed && n >= 3 {
            corner(pts[n - 2], pts[n - 1], pts[0]);
            corner(pts[n - 1], pts[0], pts[1]);
        }
    }

    /// Frame with clipped corners in the augmented-ui style (eDEX panels).
    ///
    /// The stroke is drawn INSIDE the rect. The rect comes from layout and the
    /// width from a theme token, and only this convention keeps the theme out
    /// of panel geometry: a heavier border under one theme must never grow the
    /// thing it borders (r1's ruling on the centred-vs-inside split —
    /// `rect_outline` was already inside, this path was centred, and the two
    /// disagreed by half a stroke).
    ///
    /// The polyline is centred on its own path, so the path is inset by t/2 —
    /// and the 45° face needs more than that: offsetting the line x + y = k
    /// inward by d moves its CONSTANT by d·√2, so the cut length measured
    /// along the axes changes by d·(√2−1) each side, i.e. the effective cut
    /// shrinks by (2−√2)·t/2 ≈ 0.293·t. The earlier t/2 guess left the face
    /// 0.44 px outside the rect at stroke.regular (r1's derivation).
    pub fn chamfer_frame(&mut self, x: f32, y: f32, w: f32, h: f32, cut: f32, t: f32, color: Color) {
        self.cmd(|| DrawCmd::ChamferFrame {
            r: [x, y, w, h],
            cut,
            stroke: t,
            color,
        });
        // A wrapper over the one ring generator (r1 §3.3): identical band —
        // outer face on the rect, inner face `t` further in, the 45° face
        // shortened per Corner::inset — at the same 48 vertices, but
        // watertight where the polyline overlapped and notched at joints.
        self.ring_verts(Rect::new(x, y, w, h), &[Corner::chamfer(cut); 4], 3, t, color);
    }

    /// The filled counterpart of `chamfer_frame`: the very octagon the
    /// frame outlines, as three quads. A background drawn with this
    /// stays inside the border instead of poking past the cut corners.
    pub fn chamfer_fill(&mut self, x: f32, y: f32, w: f32, h: f32, cut: f32, color: Color) {
        self.cmd(|| DrawCmd::ChamferFill { r: [x, y, w, h], cut, color });
        self.chamfer_fill_verts(x, y, w, h, cut, color);
    }

    fn chamfer_fill_verts(&mut self, x: f32, y: f32, w: f32, h: f32, cut: f32, color: Color) {
        let cut = cut.min(w * 0.5).min(h * 0.5).max(0.0);
        self.quad_verts(
            [[x + cut, y], [x + w - cut, y], [x + w - cut, y + h], [x + cut, y + h]],
            color,
        );
        self.quad_verts(
            [[x, y + cut], [x + cut, y], [x + cut, y + h], [x, y + h - cut]],
            color,
        );
        self.quad_verts(
            [[x + w - cut, y], [x + w, y + cut], [x + w, y + h - cut], [x + w - cut, y + h]],
            color,
        );
    }

    /// Stroked ring over `r` — the one tessellated generator (r1 §3): a
    /// Square, Chamfer or Round treatment per corner, `stroke` px wide,
    /// drawn INSIDE the rect. The rect is layout's and the width is the
    /// theme's, and only this alignment keeps the theme's knob out of the
    /// layout's geometry: a heavier border must never grow the thing it
    /// borders. Emitted as one quad per boundary segment between the rect's
    /// own boundary and the boundary inset by `stroke` (corners via
    /// Corner::inset, which carries chamfer_frame's 0.293·t derivation at
    /// full width), so the outer face is exactly flush with `r`, nothing
    /// leaks, and nothing overlaps — which additive runs care about.
    /// Cost: all-square 24 verts (rect_outline's price), all-chamfer 48
    /// (chamfer_frame's), round 6·(S+1) per corner; `segments` from
    /// ring_segments() with the theme's ceiling.
    pub fn ring(&mut self, r: Rect, c: &[Corner; 4], segments: u8, stroke: f32, color: Color) {
        self.cmd(|| DrawCmd::Ring {
            r: [r.x, r.y, r.w, r.h],
            corners: *c,
            stroke,
            color,
        });
        if self.vector {
            // The vector lane: STROKE alone, the band inward as ever —
            // and if the bed under it was written by the `ring_fill`
            // immediately before, this WELDS onto that record instead
            // of writing a second one ([`Weld`], f3 §2.10). Nothing
            // here decides that; `shape_verts` compares the resolved
            // silhouettes and the caller never learns which happened.
            self.shape_verts(&ShapeSpec {
                rect: r,
                corners: *c,
                kind: ShapeKind::Box,
                fill: None,
                stroke: Some((stroke, color)),
                glass: None,
                soft: None,
            });
            return;
        }
        self.ring_verts(r, c, segments, stroke, color);
    }

    fn ring_verts(&mut self, r: Rect, c: &[Corner; 4], segments: u8, stroke: f32, color: Color) {
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let t = stroke.max(0.0).min(r.w.min(r.h) * 0.5);
        if t <= 0.0 {
            return;
        }
        let inner_r = Rect::new(
            r.x + t,
            r.y + t,
            (r.w - 2.0 * t).max(0.0),
            (r.h - 2.0 * t).max(0.0),
        );
        let ci = [c[0].inset(t), c[1].inset(t), c[2].inset(t), c[3].inset(t)];
        let mut outer = std::mem::take(&mut self.scratch_a);
        let mut inner = std::mem::take(&mut self.scratch_b);
        ring_points(r, c, segments, &mut outer);
        ring_points(inner_r, &ci, segments, &mut inner);
        debug_assert_eq!(outer.len(), inner.len());
        let (u, v) = FontSystem::white_uv();
        let col = color.to_array();
        let n = outer.len();
        for i in 0..n {
            let j = (i + 1) % n;
            self.push_quad4(
                None,
                [outer[i], outer[j], inner[j], inner[i]],
                [[u, v]; 4],
                [col; 4],
            );
        }
        self.scratch_a = outer;
        self.scratch_b = inner;
    }

    /// The snapped rect, resolved corner radii, half-extent and centre a
    /// Box-kind [`ShapeSpec`] over `(r, corners)` would carry into its
    /// record — `shape_verts`' own snap (§2.7) and corner resolution
    /// (R9), copied rather than shared: `shape_verts` takes a whole
    /// `ShapeSpec` and does six other things with it besides, and a
    /// read-only preflight has no business behind that door. Kept a
    /// LITERAL copy of that one branch so the two cannot silently drift:
    /// a divergence here only ever makes [`Self::open_glass_weld`]
    /// answer "no" where `shape_verts` would have welded, never the
    /// other way, which is the safe direction to be wrong in.
    fn box_silhouette(
        &self,
        mut r: Rect,
        corners: &[Corner; 4],
    ) -> Option<(Rect, [f32; 4], [f32; 2], [f32; 2])> {
        if r.w <= 0.0 || r.h <= 0.0 {
            return None;
        }
        if self.warp <= 1 {
            let x1 = (r.x + r.w).round();
            let y1 = (r.y + r.h).round();
            r.x = r.x.round();
            r.y = r.y.round();
            r.w = x1 - r.x;
            r.h = y1 - r.y;
            if r.w <= 0.0 || r.h <= 0.0 {
                return None;
            }
        }
        let cap = (r.w.min(r.h) * 0.5).max(0.0);
        let same = crate::theme::expr::sentinel("same_as_parent").unwrap_or(-3.0);
        let base = crate::theme::corner_radius(corners[0].size, r.w, r.h);
        let resolve = |c: &Corner| -> f32 {
            let sz = if c.size == same {
                base
            } else {
                crate::theme::corner_radius(c.size, r.w, r.h)
            };
            sz.min(cap)
        };
        let corner = [
            resolve(&corners[0]),
            resolve(&corners[1]),
            resolve(&corners[2]),
            resolve(&corners[3]),
        ];
        let half = [r.w * 0.5, r.h * 0.5];
        let centre = [r.x + half[0], r.y + half[1]];
        Some((r, corner, half, centre))
    }

    /// Whether an open weld (§2.10) is both this call's OWN silhouette —
    /// same rect, same corners, same bits a plain [`ring`](Self::ring)
    /// would weld onto — and bound to a GLASS run, so
    /// [`ring_grad`](Self::ring_grad) may join the same one-record
    /// silhouette a solid ring already does (K3b's second bounded edge
    /// case).
    ///
    /// `Frost` claims no flag of its own on the record (§3.3: "a frost
    /// claims no bit here"), so the only place that still says "this bed
    /// is glass" is the RUN it rides — and that is only readable while
    /// `bed.runs == self.runs.len()` still guarantees the last run is
    /// the one the weld was opened on, the same guarantee `shape_verts`'
    /// own `fits` check makes before it looks at anything else.
    ///
    /// Returns the snapped rect and resolved corner radii on a match, so
    /// the caller does not resolve them twice.
    fn open_glass_weld(&self, r: Rect, c: &[Corner; 4]) -> Option<(Rect, [f32; 4])> {
        let bed = self.weld.as_ref()?;
        if bed.verts != self.verts.len() || bed.runs != self.runs.len() {
            return None;
        }
        let (r, corner, half, centre) = self.box_silhouette(r, c)?;
        let mut bits = 0u32;
        for (i, corner) in c.iter().enumerate() {
            bits |= (corner.style as u32) << (2 * i as u32);
        }
        bits |= ShapeKind::Box.code() << Shape::KIND_SHIFT;
        if bed.centre != centre || bed.half != half || bed.corner != corner || bed.bits != bits {
            return None;
        }
        let on_glass = matches!(
            self.runs.last().and_then(|run| run.image),
            Some(img) if is_shape_glass(img)
        );
        on_glass.then_some((r, corner))
    }

    /// [`ring`](Self::ring)'s ring, coloured by a two-stop gradient
    /// projected along `dir`.
    ///
    /// The same tessellation and the same vertex count: `Vertex` already
    /// carries a colour per corner and the rasteriser already interpolates
    /// it, so a gradient ring costs exactly what a flat one does — which is
    /// the master's own claim at `[grad]` ("a gradient border is continuous
    /// around a frame at the same 24 verts a solid border costs").
    ///
    /// `dir` need not be normalised and its sign is the direction of
    /// travel; `t` is normalised against the RECT's own projected extent,
    /// so the near stop lands on the least-projected corner and the far
    /// stop on the most-projected one whatever the box's aspect. A `dir`
    /// of zero length degrades to the near colour, which is the flat ring
    /// the caller would have drawn without it.
    ///
    /// Kept apart from `ring_verts` rather than folded into it: `lerp` at
    /// `near == far` is not bit-for-bit `near`, so routing the flat ring
    /// through this one would move every existing frame's vertices by an
    /// ulp for no picture.
    ///
    /// # This one has no vector lane of its own, and that is stated, not
    /// forgotten
    ///
    /// [`ring`](Self::ring) branches on `self.vector` and hands the stroke
    /// to `shape_verts`; this does not, and cannot yet. A shape record
    /// carries `stroke: Option<(f32, Color)>` — ONE colour on the band —
    /// so a two-stop edge is not a thing the lane can be asked for, and
    /// asking it anyway would flatten the gradient to its near stop with
    /// no word about it. Tessellation is the only path that can draw what
    /// the theme wrote, so a gradient ring takes it whichever way
    /// `render.vector` is set.
    ///
    /// Widening the record to a stop pair is K4's business, not a merge's:
    /// it is a change to the shape record itself and to `fs_shape`, on
    /// the far side of a repository boundary, and it wants deciding
    /// together with the NAMED `edge.gradient` slot the theme engine still
    /// bakes no stops for. **That stays bounded and unfixed here** for a
    /// gradient ring over an ordinary (non-glass) bed: its fill-only
    /// record and this ring's own strip are still two coverages on one
    /// outer edge, and closing that in general needs the record change
    /// above.
    ///
    /// **A GLASS rung is narrower, and does not need that change (K3b).**
    /// A frosted bed already asks `ring`/`ring_fill` to WELD onto it
    /// (§2.10, §3.3), and the reason a solid border can join that one
    /// record — one uniform `stroke_c` — is exactly the reason a
    /// two-stop gradient cannot. But the record's OUTER antialiased edge
    /// (`coverage(d, 1.0)` in `fs_shape`/`fs_shape_glass`) is a pure
    /// function of the record's own `half`/`corner`, computed at every
    /// fragment regardless of which vertices happen to cover it — it
    /// does not care whether the pixels over the band are one flat
    /// colour or ten. So when there IS an open glass weld with this
    /// call's own silhouette (checked by [`DrawList::open_glass_weld`]),
    /// this welds a SOLID anchor (the two stops' own midpoint) into that
    /// SAME record — one edge, the ordinary weld every solid ring gets —
    /// and then paints the true two-stop gradient back in as a second,
    /// tessellated pass, INSET by [`AA_PAD`] from both the record's outer
    /// edge and its inner (stroke/fill) edge so the second pass never
    /// overlaps either coverage ramp: away from those two ~1px fringes
    /// the record's own coverage is already 1, so painting over it is an
    /// ordinary composite, not a second blend of the same partial pixel.
    /// The cost is that the outermost and innermost pixel of the border
    /// reads the anchor colour rather than the true gradient value there
    /// — bounded, and the same order of approximation `glass_fill`'s own
    /// fractional depth already accepts. A border too thin to leave any
    /// interior past both insets (`stroke <= 2 · AA_PAD`) skips the
    /// second pass entirely and stays the flat anchor, which is still
    /// welded and still rim-free.
    pub fn ring_grad(
        &mut self,
        r: Rect,
        c: &[Corner; 4],
        segments: u8,
        stroke: f32,
        near: Color,
        far: Color,
        dir: [f32; 2],
    ) {
        self.cmd(|| DrawCmd::RingGrad {
            r: [r.x, r.y, r.w, r.h],
            corners: *c,
            stroke,
            near,
            far,
            dir,
        });
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let t = stroke.max(0.0).min(r.w.min(r.h) * 0.5);
        if t <= 0.0 {
            return;
        }
        // K3b's second bounded edge case, closed for the glass rung: see
        // the doc above `open_glass_weld` welds an anchor colour into
        // the SAME record an open glass bed offers, so the true gradient
        // (painted back in below, inset from both its edges) never
        // shares an antialiased boundary with a second record.
        let glass = self.vector.then(|| self.open_glass_weld(r, c)).flatten();
        let (outer_r, outer_c, inner_r, inner_c) = if let Some((gr, gc)) = glass {
            let anchor = lerp(near, far, 0.5);
            let before = self.shapes.len();
            self.shape_verts(&ShapeSpec {
                rect: r,
                corners: *c,
                kind: ShapeKind::Box,
                fill: None,
                stroke: Some((t, anchor)),
                glass: None,
                soft: None,
            });
            debug_assert_eq!(
                self.shapes.len(),
                before,
                "ring_grad's own weld pre-check disagreed with shape_verts' fits: \
                 the anchor wrote a second record instead of joining the bed"
            );
            let cap = (gr.w.min(gr.h) * 0.5).max(0.0);
            let stroke_w = if self.warp <= 1 { t.round().max(1.0) } else { t }.min(cap);
            let pad = (stroke_w - 2.0 * AA_PAD).max(0.0);
            if pad <= 0.0 {
                // No interior survives both insets: the anchor already
                // welded above stands for the whole band, rim-free.
                return;
            }
            let gcorner = |i: usize| Corner { style: c[i].style, size: gc[i] };
            let outer_r = Rect::new(
                gr.x + AA_PAD,
                gr.y + AA_PAD,
                (gr.w - 2.0 * AA_PAD).max(0.0),
                (gr.h - 2.0 * AA_PAD).max(0.0),
            );
            let outer_c = [
                gcorner(0).inset(AA_PAD),
                gcorner(1).inset(AA_PAD),
                gcorner(2).inset(AA_PAD),
                gcorner(3).inset(AA_PAD),
            ];
            let inset_in = stroke_w - AA_PAD;
            let inner_r = Rect::new(
                gr.x + inset_in,
                gr.y + inset_in,
                (gr.w - 2.0 * inset_in).max(0.0),
                (gr.h - 2.0 * inset_in).max(0.0),
            );
            let inner_c = [
                gcorner(0).inset(inset_in),
                gcorner(1).inset(inset_in),
                gcorner(2).inset(inset_in),
                gcorner(3).inset(inset_in),
            ];
            (outer_r, outer_c, inner_r, inner_c)
        } else {
            let inner_r = Rect::new(
                r.x + t,
                r.y + t,
                (r.w - 2.0 * t).max(0.0),
                (r.h - 2.0 * t).max(0.0),
            );
            let ci = [c[0].inset(t), c[1].inset(t), c[2].inset(t), c[3].inset(t)];
            (r, *c, inner_r, ci)
        };
        let mut outer = std::mem::take(&mut self.scratch_a);
        let mut inner = std::mem::take(&mut self.scratch_b);
        ring_points(outer_r, &outer_c, segments, &mut outer);
        ring_points(inner_r, &inner_c, segments, &mut inner);
        debug_assert_eq!(outer.len(), inner.len());
        // The rect's own projected extent, from its four corners: the two
        // extremes of an axis-aligned box under a linear projection are two
        // of its corners, so `min`/`max` over them is exact and needs no
        // sampling of the boundary the ring was tessellated into.
        let (dx, dy) = (dir[0], dir[1]);
        let lo = r.x * dx + r.y * dy + r.w * dx.min(0.0) + r.h * dy.min(0.0);
        let hi = r.x * dx + r.y * dy + r.w * dx.max(0.0) + r.h * dy.max(0.0);
        let span = hi - lo;
        let at = |p: [f32; 2]| {
            if span.abs() <= 1e-6 {
                return near;
            }
            lerp(near, far, ((p[0] * dx + p[1] * dy - lo) / span).clamp(0.0, 1.0))
        };
        let (u, v) = FontSystem::white_uv();
        let n = outer.len();
        for i in 0..n {
            let j = (i + 1) % n;
            self.push_quad4(
                None,
                [outer[i], outer[j], inner[j], inner[i]],
                [[u, v]; 4],
                [
                    at(outer[i]).to_array(),
                    at(outer[j]).to_array(),
                    at(inner[j]).to_array(),
                    at(inner[i]).to_array(),
                ],
            );
        }
        self.scratch_a = outer;
        self.scratch_b = inner;
    }

    /// Filled interior of the same ring, the fill counterpart of ring().
    /// Fast paths keep the shapes the program already draws at their old
    /// price: all-Square is one quad (6 verts — exactly rect()), all-Chamfer
    /// at one cut is chamfer_fill's three quads (18). Everything else fans
    /// from the centroid at 3 verts per boundary point. Drawn on the
    /// ORIGINAL rect: the fill must reach the rect edge under the border,
    /// and the z-order puts the ring above it.
    pub fn ring_fill(&mut self, r: Rect, c: &[Corner; 4], segments: u8, color: Color) {
        self.cmd(|| DrawCmd::RingFill {
            r: [r.x, r.y, r.w, r.h],
            corners: *c,
            color,
        });
        if self.vector {
            // The vector lane: FILL alone, the colour on the vertices
            // as every fill before it, still on the ORIGINAL rect —
            // the fill must reach the rect edge under the border. A
            // wash laid straight over another fill of the same outline
            // — the button's and the field's idiom — composites into
            // that bed instead of writing a second record ([`Weld`]).
            self.shape_verts(&ShapeSpec {
                rect: r,
                corners: *c,
                kind: ShapeKind::Box,
                fill: Some(color),
                stroke: None,
                glass: None,
                soft: None,
            });
            return;
        }
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        if c.iter().all(|k| k.style == CornerStyle::Square) {
            self.rect_verts(r.x, r.y, r.w, r.h, color);
            return;
        }
        if c.iter().all(|k| k.style == CornerStyle::Chamfer && k.size == c[0].size) {
            self.chamfer_fill_verts(r.x, r.y, r.w, r.h, c[0].size, color);
            return;
        }
        let mut pts = std::mem::take(&mut self.scratch_a);
        ring_points(r, c, segments, &mut pts);
        let n = pts.len();
        if n >= 3 {
            let (mut cx, mut cy) = (0.0f32, 0.0f32);
            for p in &pts {
                cx += p[0];
                cy += p[1];
            }
            let inv = 1.0 / n as f32;
            let (cx, cy) = (cx * inv, cy * inv);
            let col = color.to_array();
            for i in 0..n {
                let j = (i + 1) % n;
                self.push_tri_c(None, [[cx, cy], pts[i], pts[j]], [col; 3]);
            }
        }
        self.scratch_a = pts;
    }

    /// One shape of the vector family (f3 §2.11): bed and/or edge over
    /// one silhouette, one record and one quad (6 vertices) whatever
    /// the corners — where the ring generator spends up to 168.
    ///
    /// K6 gave bits 8-11 a reader, so Ring, Hex and Chevron now draw as
    /// themselves rather than as their bounding box; `Capsule` is still
    /// recorded and still drawn as a box, because nothing emits one and
    /// `line()` tessellates. The `feather` slot has a writer since the
    /// soft profiles landed: see [`Soft`].
    pub fn shape(&mut self, s: &ShapeSpec) {
        self.cmd(|| DrawCmd::Shape {
            r: [s.rect.x, s.rect.y, s.rect.w, s.rect.h],
            corners: s.corners,
            kind: s.kind,
            fill: s.fill,
            stroke: s.stroke,
            soft: s.soft,
        });
        self.shape_verts(s);
    }

    /// The record and quads of [`DrawList::shape`] without the command —
    /// ring/ring_fill's vector lane comes in here having recorded its
    /// own name, because the register holds intent and their intent is
    /// Ring/RingFill whatever tessellates them.
    fn shape_verts(&mut self, s: &ShapeSpec) {
        let mut r = s.rect;
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let stroke = s.stroke.filter(|&(w, _)| w > 0.0);
        let fill = s.fill;
        // A FROST is a part in its own right: a record carrying nothing
        // but glass still draws the blurred scene through its own
        // silhouette. Without this clause the guard would throw away a
        // frosted layer that has no wash of its own — which is every
        // lower rung of a fractional depth.
        if fill.is_none() && stroke.is_none() && s.glass.is_none() {
            return;
        }
        // The softness, resolved once: a reach that is not positive is
        // no softness at all, and a crisp record is what the caller
        // then gets — the same degeneracy `glow_ring` has always had at
        // radius 0.
        let soft = s.soft.filter(|f| f.reach > 0.0);
        debug_assert!(
            !(soft.is_some() && stroke.is_some()),
            "a soft record has no band: the stroke's coverage is the \
             difference of two crisp ramps and means nothing under a gaussian"
        );
        debug_assert!(
            !(soft.is_some() && s.glass.is_some()),
            "a soft record reads no pyramid: the frosted fragment composes \
             a crisp band over a blurred sample, which a profile is not"
        );
        let snap = self.warp <= 1;
        if snap {
            // §2.7: an axis-aligned shape snaps its OUTER edges to the
            // device pixel grid on the CPU, before the record is
            // written — the vector lane must never smear a hard
            // interface edge across two half-lit pixels. Off during a
            // warp ride, where the screen grid does not correspond to
            // the shape's anyway.
            let x1 = (r.x + r.w).round();
            let y1 = (r.y + r.h).round();
            r.x = r.x.round();
            r.y = r.y.round();
            r.w = x1 - r.x;
            r.h = y1 - r.y;
            if r.w <= 0.0 || r.h <= 0.0 {
                return;
            }
        }
        // R9: sentinels resolve HERE — the buffer never sees a negative
        // corner. `pill` is half the short side (corner_radius's one
        // rule, sized like ring_points' own cap); `same_as_parent`
        // takes the FIRST corner as its parent — the base every
        // `[Corner { .. }; 4]` builder repeats, and the parent
        // `view::paint::preset` hands down when a per-corner token says
        // to inherit.
        let cap = (r.w.min(r.h) * 0.5).max(0.0);
        let same = crate::theme::expr::sentinel("same_as_parent").unwrap_or(-3.0);
        let base = crate::theme::corner_radius(s.corners[0].size, r.w, r.h);
        let resolve = |c: &Corner| -> f32 {
            let sz = if c.size == same {
                base
            } else {
                crate::theme::corner_radius(c.size, r.w, r.h)
            };
            debug_assert!(sz >= 0.0, "a sentinel reached the record: {sz}");
            sz.min(cap)
        };
        // Corner radii are NOT snapped (§2.7): the curve does not lie
        // on the grid to begin with.
        //
        // The kinds past Box do not have corners at all, so they do not
        // come through here: their lengths are the payload's own, in px
        // already, and `cap` — which exists to stop two corner arcs
        // meeting — would be the wrong ceiling for every one of them (a
        // chevron collapsing a whole end reaches the middle of the rect
        // by DESIGN). Their slots are filled by [`kind_lengths`] and the
        // rest zeroed, so the weld compares silhouettes and not leftovers.
        let half = [r.w * 0.5, r.h * 0.5];
        let corner = match s.kind {
            ShapeKind::Box => [
                resolve(&s.corners[0]),
                resolve(&s.corners[1]),
                resolve(&s.corners[2]),
                resolve(&s.corners[3]),
            ],
            kind => kind_lengths(kind, half),
        };
        let (stroke_w, stroke_c) = match stroke {
            // The baker's own hairline rule — `stroke.*` bakes as
            // max(1, round(x·u)) — applied at the same moment as the
            // edge snap, and skipped with it. `cap` is `ring_verts`'
            // own ceiling: a band deeper than half the short side would
            // meet itself, and the two lanes must agree on that too.
            //
            // §2.8, THE HAIRLINE, AND WHERE IT APPLIES. This rounding
            // is also the answer to a question §2.8 asks of the shader:
            // a band thinner than a pixel must not vanish or shimmer,
            // so the plan spreads it to one pixel and dims it by
            // `stroke / (floor·w)` to keep its integral. While the snap
            // is on — every still frame, `warp <= 1` — no such band
            // ever reaches the record: `round().max(1.0)` has already
            // lifted 0.3 px to 1 px, at full strength, and the theme's
            // baker lifted it once before that. The energy rule
            // therefore governs the UNSNAPPED lane alone — a ride, and
            // whatever K4 brings that is not axis-aligned — where a
            // real 0.3 px band can exist because there is no pixel grid
            // to round to. Two rules, two domains, and they cannot both
            // fire on the same band.
            Some((w, c)) => {
                (if snap { w.round().max(1.0) } else { w }.min(cap), c.to_array())
            }
            None => (0.0, [0.0; 4]),
        };
        let mut flags = 0u32;
        // Bits 0-7 belong to Box's four corner treatments and to nothing
        // else. Under another kind they stay zero rather than carrying
        // whatever the caller left in `corners`: the shader does not read
        // them there, and a silhouette bit that varies without changing
        // the picture would break the weld's own premise (§2.10).
        if s.kind == ShapeKind::Box {
            for (i, c) in s.corners.iter().enumerate() {
                flags |= (c.style as u32) << (2 * i as u32);
            }
        }
        flags |= s.kind.code() << Shape::KIND_SHIFT;
        if fill.is_some() {
            flags |= Shape::FILL;
        }
        if stroke.is_some() {
            flags |= Shape::STROKE;
        }
        // §2.6's pair. GAUSS says WHAT function of the distance the
        // fragment computes; OUTSIDE_ONLY says where it is allowed to
        // be non-zero. The third difference between a glow and a
        // shadow — light against cover — is the run's, below.
        let feather = soft.map_or(0.0, |f| f.reach);
        if let Some(f) = soft {
            flags |= Shape::GAUSS;
            if f.kind == SoftKind::Glow {
                flags |= Shape::OUTSIDE_ONLY;
            }
        }
        // A frost claims no bit here: `tint` below says it, the run's
        // handle binds it, and the fragment needs neither told twice.
        let half = [r.w * 0.5, r.h * 0.5];
        let centre = [r.x + half[0], r.y + half[1]];
        // §2.10: a part drawn over the bed that was just written is not
        // a second shape. It is the same silhouette wearing another
        // part, and the record has a bit — and a bed colour — for
        // exactly that.
        if let Some(bed) = self.weld.take() {
            let fits = bed.verts == self.verts.len()
                && bed.runs == self.runs.len()
                && bed.idx + 1 == self.shapes.len()
                && bed.centre == centre
                && bed.half == half
                && bed.corner == corner
                && bed.bits == flags & Shape::SILHOUETTE
                // A FROST never welds onto anything. Two frosted layers
                // of one outline are the two rungs a fractional depth
                // mixes between (`glass_fill`), and a run binds ONE
                // pyramid target — so a fragment cannot read both and
                // the second layer needs its own record and its own
                // run. The wash and the border that follow have no such
                // trouble: they read no target at all.
                && s.glass.is_none()
                // NOR DOES A SOFT RECORD, and this one is load-bearing:
                // [`Shape::SILHOUETTE`] is bits 0-11, so a glow around
                // a panel matches that panel's bits EXACTLY — same
                // corners, same kind — and without this clause the
                // glow's colour would be composited into the panel's
                // quad and its profile lost. The soft bits are
                // deliberately outside SILHOUETTE (they do not change
                // which curve is drawn, only what is drawn about it),
                // so the refusal has to be said here rather than fall
                // out of the mask.
                && soft.is_none();
            if fits {
                // A second fill goes into the quad that is already
                // there. The record keeps its FILL bit — one bed, one
                // colour, one edge.
                let fill = match fill {
                    Some(c) => {
                        let mixed = fill_over(c.to_array(), bed.fill);
                        for v in &mut self.verts[bed.paint..bed.verts] {
                            v.color = mixed;
                        }
                        // An EMPTY bed carries no area ([`bed_rect`]) —
                        // a frosted surface lays one, because its wash
                        // comes a call later than the geometry that
                        // holds it. This is that call: the frame is cut
                        // again at the band it already has, which opens
                        // the bed and is the identity on every other
                        // quad.
                        if let (Some(band), true) = (bed.frame, bed.fill[3] <= 0.0) {
                            if mixed[3] > 0.0 {
                                self.respan_frame(&bed, band);
                            }
                        }
                        mixed
                    }
                    None => bed.fill,
                };
                if stroke.is_some() {
                    let rec = &mut self.shapes[bed.idx];
                    rec.flags |= Shape::STROKE;
                    rec.stroke = stroke_w;
                    rec.stroke_c = stroke_c;
                    // The band the bed's frame was cut for was the band
                    // of a bed with no border; this one runs `stroke_w`
                    // deeper, so the core has to give that back or the
                    // fill path would shade pixels the field bends.
                    // The layout is fixed, so this is a rewrite in
                    // place — no vertex is added and none is dropped.
                    if let Some(band) = bed.frame {
                        self.respan_frame(&bed, band + stroke_w);
                    }
                    // The offer is NOT renewed: a fill arriving after a
                    // band would have to sink under it here and stands
                    // over it in the tessellated lane. It gets its own
                    // record, and the picture stays the one the caller
                    // asked for.
                } else {
                    self.weld = Some(Weld { fill, ..bed });
                }
                return;
            }
        }
        let idx = self.shapes.len() as u32;
        let (arc_half, arc_dir) = kind_angles(s.kind);
        self.shapes.push(Shape {
            half,
            stroke: stroke_w,
            feather,
            corner,
            stroke_c,
            flags,
            arc_half,
            arc_dir,
            _pad: 0.0,
            // The RECORD carries the BAND's tint, not necessarily the
            // layer's own — see `Frost::band_tint`. Every caller but
            // `glass_fill`'s fractional path leaves it unset, and the
            // record ends up with `g.tint` exactly as before K3c. The
            // CORE quad above reads `g.tint` directly and never this
            // field: it is artifact-free by construction (§K3c) and
            // has nothing to substitute.
            tint: s.glass.map_or([0.0; 4], |g| g.band_tint.unwrap_or(g.tint).to_array()),
        });
        // The vertex colour is the fill's — or the stroke's when there
        // is no fill, so §2.10's mix starts from the band's own colour
        // and the band's inner AA edge cannot pick up a foreign tint.
        let colour = fill
            .or(stroke.map(|(_, c)| c))
            .unwrap_or(Color::WHITE)
            .to_array();
        // THE ENVELOPE MUST KNOW THE REACH OF THE EFFECT (§3.1, and the
        // first thing the scope decision asks of this step).
        //
        // The quad reaches one pixel past the silhouette so the crisp
        // AA ramp has somewhere to land; a soft profile lands `feather`
        // px further out, and a quad cut at `AA_PAD` would slice the
        // glow off on a straight line and draw it as a FRAME. The
        // margin was always documented as "a feather would join this",
        // which was correct while nothing wrote one — this is the day
        // something does.
        //
        // What grows is the whole envelope and not one side of it: the
        // profile is a function of the DISTANCE, so it reaches equally
        // far past every edge and every corner, and a rounded corner
        // needs the extra room most (its own arc is furthest inside the
        // rect's corner). §2.7's snap has already fixed where the
        // silhouette's edges are; `feather` is added after it, because
        // the effect's extent is not an edge of the interface and
        // rounding it would move the profile's zero off its own curve.
        let pad = AA_PAD + feather;
        let ext = [half[0] + pad, half[1] + pad];
        let n = self.warp.max(1) as u32;
        // f3 §7b, REMEDY 1 — THE RING OF QUADS. One quad over the whole
        // shape asks the field for every pixel of the interior too,
        // where the answer is known before it is computed: `cov` is 1,
        // the band is 0, and `fs_shape` returns the vertex colour after
        // spending a hundred instructions to find that out — against
        // five on the ordinary fill path. So the interior is CUT OUT
        // and drawn as what it is: a plain fill of the same colour, on
        // the same vertices, through the path every solid rect in this
        // file already takes. Only a band `reach + stroke + AA_PAD +
        // CORE_PAD` deep along the perimeter still goes to the field.
        //
        // The picture does not move, and that is provable rather than
        // hoped: `sdf::tests` rasterises both variants pixel by pixel
        // and asserts the fragments are equal to the bit. See
        // [`CORE_PAD`] for why the boundary sits where it does.
        //
        // THREE THINGS HOLD THE SPLIT BACK, each for its own reason:
        //
        // * **A ride** (`warp > 1`). The field's screen gradient is 1
        //   only while one local px is one device px; under a
        //   perspective ride it is not, the ramp widens in local units,
        //   and the core's boundary could land inside it. Same
        //   condition as the edge snap, and for the same reason.
        // * **A kind past Box.** THE DAY THIS GUARD WAS WRITTEN FOR HAS
        //   COME. Until K6 the shader drew every record as its box
        //   distance, so a Hex record would have split correctly by
        //   accident; now bits 8-11 are read and its interior is the
        //   hexagon's, which is strictly smaller than the rect's. Cut a
        //   core out of it and the plain-fill path would paint the four
        //   triangles the hexagon does not cover — the corners of the
        //   rect, at full alpha. The guard is what stops that, it is
        //   load-bearing rather than cautious, and the test
        //   `a_ride_a_foreign_kind_and_a_small_core_keep_the_whole_quad`
        //   is what keeps it. The cut is worth nothing on a shape whose
        //   interior is not the box's anyway: `core_half` measures the
        //   rect, and the rect is not the silhouette.
        // * **A core too small to pay for itself** ([`CORE_MIN`]).
        //
        // The cost of the cut is TWO runs more per shape, not one —
        // MEASURED, after an adversary called the first figure out.
        // The core samples the atlas and the strips read the shape
        // buffer, so the pair is two pipelines; but the cut also BREAKS
        // the merge of the runs on either side, which a row of plain
        // shapes used to get for free. A band of twelve panels goes
        // from 1 run to 24. Remedy 3 of the same section — merging
        // shape runs on the host's side — is where that is answered,
        // and it stops being optional; it is not answered here.
        // * **A GLOW**, and for the reason the split's own premise
        //   states: the cut is legal only where the field would have
        //   returned the fill and nothing else. Under `OUTSIDE_ONLY`
        //   the field returns NOTHING there — the interior is what the
        //   glow deliberately leaves empty — so a plain-fill core would
        //   paint the panel's whole face in the glow's colour at full
        //   alpha. No band depth fixes that; only refusing does.
        //   A shadow's interior IS its plateau and could be cut, and is
        //   not, for the reason on the band below.
        let split = (snap && s.kind == ShapeKind::Box && flags & Shape::OUTSIDE_ONLY == 0)
            .then(|| {
                let reach = s
                    .corners
                    .iter()
                    .zip(corner)
                    .fold(0.0f32, |m, (c, k)| m.max(corner_reach(c.style, k)));
                // THE BAND MUST KNOW THE REACH TOO (§3.1). It is the
                // depth at which the field's answer is certainly the
                // plain fill; a soft profile is a function OF the
                // distance on both sides of the boundary, so the depth
                // at which it has certainly settled is `feather`
                // further in. §2.6's own profile flattens at exactly
                // `d = 0` and would not need it — but the band is the
                // guarantee, not the profile, and a guarantee that
                // holds only for the profile written today is what a
                // later §2.6 would quietly break. It costs nothing
                // measurable: the only soft record the toolkit emits is
                // a glow, and a glow does not reach here at all.
                let band = reach + stroke_w + feather + AA_PAD + CORE_PAD;
                let core = core_half(half, band);
                (core[0] * core[1] * 4.0 >= CORE_MIN).then_some((band, core))
            })
            .flatten();
        // Which lane the strips ride: the plain shape pipeline, the one
        // that samples the pyramid rung this frost asked for (§3.3), or
        // the ADDITIVE twin a glow needs ([`SHAPE_ADD`]). One record in
        // every case — the rank of a frost and the blend of a glow are
        // alike properties of the RUN, because a descriptor and a blend
        // state are both bound before the first fragment is shaded.
        let lane = match (s.glass, soft.map(|f| f.kind)) {
            (Some(g), _) => shape_glass_handle(g.rank),
            (None, Some(SoftKind::Glow)) => SHAPE_ADD,
            (None, _) => SHAPE,
        };
        let from;
        let paint;
        match split {
            Some((_, core)) => {
                let frame = frame_rects(centre, ext, core);
                from = self.verts.len();
                // THE FROST'S OWN CORE, and it goes first because it is
                // the bottom layer of the three. It rides the
                // tessellated glass lane exactly as the whole surface
                // did before K3b — same handle, same tint, same
                // fragment — because the interior of a silhouette has
                // no edge to be smooth about, and the picture there is
                // the one that shipped, to the bit. Only the perimeter
                // band, where the stair-steps were, moves to the field.
                if let Some(g) = s.glass {
                    let tint = g.tint.to_array();
                    self.run_for(Some(glass_rank_handle(g.rank)));
                    self.verts.extend_from_slice(&quad6(
                        bed_rect(frame[0], centre, tint),
                        None,
                        tint,
                        NO_SHAPE,
                    ));
                    self.seal();
                }
                // Then the bed: it joins whatever ordinary run precedes
                // it, and `paint` has to be its first vertex for a wash
                // to recolour every quad that carries one at once —
                // every quad from here on, and not the frost below.
                paint = self.verts.len();
                if fill.is_some() {
                    self.run_for(None);
                    self.verts.extend_from_slice(&quad6(
                        bed_rect(frame[0], centre, colour),
                        None,
                        colour,
                        NO_SHAPE,
                    ));
                    self.seal();
                }
                // A band with no bed leaves the interior EMPTY — not
                // filled, not shaded, not rasterised at all. This is
                // what makes a window frame cost its perimeter instead
                // of its area (f3 §7b, risk 1: `winframe.rs:453` draws
                // a border over the whole window and nothing else).
                self.run_for(Some(lane));
                for r in &frame[1..] {
                    self.verts
                        .extend_from_slice(&quad6(*r, Some(centre), colour, idx));
                }
                self.seal();
            }
            None => {
                self.run_for(Some(lane));
                from = self.verts.len();
                paint = from;
                let step = [ext[0] * 2.0 / n as f32, ext[1] * 2.0 / n as f32];
                let (x0, y0) = (centre[0] - ext[0], centre[1] - ext[1]);
                for j in 0..n {
                    for i in 0..n {
                        let xa = x0 + step[0] * i as f32;
                        let xb = x0 + step[0] * (i + 1) as f32;
                        let ya = y0 + step[1] * j as f32;
                        let yb = y0 + step[1] * (j + 1) as f32;
                        let v = |x: f32, y: f32| Vertex {
                            pos: [x, y],
                            uv: [x - centre[0], y - centre[1]],
                            color: colour,
                            shape: idx,
                        };
                        self.verts.extend_from_slice(&[
                            v(xa, ya),
                            v(xb, ya),
                            v(xb, yb),
                            v(xa, ya),
                            v(xb, yb),
                            v(xa, yb),
                        ]);
                    }
                }
                self.seal();
            }
        }
        // A BARE BED is left open — a fill with no band of its own.
        // `ring_fill` (then another `ring_fill`) then `ring` is how
        // every panel, field, menu and button in the toolkit spells a
        // framed surface, and it is what §2.10 exists to weld. Anything
        // else drawn first closes the offer, because the guard above
        // will find `verts` or `runs` moved. A record that already
        // carries a band offers nothing: see the take.
        //
        // WHAT THE OFFER HAS TO COVER, measured rather than assumed —
        // the census of the twelve shape-drawing sites of the toolkit
        // (2026-08-17; the probe armed this lane, called the real entry
        // points on the live theme and read `shapes()` back):
        //
        // * TWO sites lay a fill over a fill: `button::dress`
        //   (`button.rs:109` plate, `:114` state wash, `:116` border)
        //   and `text_input::draw` (`text_input.rs:953`, `:967`,
        //   `:977`). Three calls each; before the fill weld they wrote
        //   two records and the R4 rim was real and visible on every
        //   button, every drop-down row (through `dropdown.rs:337`) and
        //   every field. Now: one record, three calls, one edge.
        // * NINE sites are the plain bed+border pair and were already
        //   one record with the border weld alone: `window::frame`
        //   (`window.rs:187`/`:189`), `menu.rs:445`/`:448`,
        //   `tooltip.rs:266`/`:269`, `elev::Level::draw`
        //   (`elev.rs:121`/`:127`), `view::surface` (`surface.rs:418`/
        //   `:423`), `paint`'s pill (`paint.rs:559`/`:561`) and
        //   scrollbar thumb (`:695`/`:700`), and the cells of
        //   `segmented.rs:149`/`:153` and `tabs.rs:353`/`:362`.
        // * ONE site draws borders and no fill at all: `winframe`
        //   (`winframe.rs:453` the frame, `:495` the four button
        //   plates) — five records, five silhouettes, nothing to weld.
        //   2 + 9 + 1 = the twelve.
        //
        // Three further sites were measured as CONTROLS, outside that
        // census, because they draw fills that are NOT one silhouette
        // and must never weld: the meter's track and its bar on a
        // shorter rect (`paint.rs:483`/`:491`), the slider's three
        // (`slider.rs:60`/`:78`/`:89`), and a list's row plates
        // (`list.rs:354`) — one per row. The geometry check refuses
        // them by itself; nothing here has to know their names.
        // * The GLASS pair (`window.rs:181`/`:184`, `elev.rs:113`/
        //   `:116`) is the thirteenth site and the one K3b was for: it
        //   is one record now, and for the right reason. The frost
        //   opens the offer with an EMPTY bed, the wash welds into it,
        //   the border welds after — three calls, one silhouette, one
        //   coverage over all three layers (§3.3).
        //
        // A SOFT record offers nothing either, and it would otherwise:
        // a glow is FILL without STROKE, which is the offer's own
        // shape. The border of the panel the glow wraps would then weld
        // its band onto the GLOW's record and the panel would lose it.
        self.weld = (flags & (Shape::FILL | Shape::STROKE | Shape::SOFT) == Shape::FILL).then_some(Weld {
            idx: idx as usize,
            from,
            paint,
            verts: self.verts.len(),
            runs: self.runs.len(),
            centre,
            half,
            corner,
            bits: flags & Shape::SILHOUETTE,
            fill: colour,
            frame: split.map(|(band, _)| band),
        });
    }

    /// One FILLED shape of the vector family read in an oriented frame
    /// (f3 §K4) — the entry the whole diagonal lane is built out of.
    ///
    /// The record is the Box family's, untouched: the shader takes the
    /// fragment's local position out of `uv` and knows nothing about
    /// any frame. What makes this a diagonal is where the four vertices
    /// are put and what they carry — position through the frame, uv the
    /// local corner they were built from — and the rasteriser inverts
    /// the map for free. **The renderer needed no change for K4 at
    /// all**; see [`crate::sdf::Frame`] for why, and for why the
    /// antialiasing survives the rotation.
    ///
    /// Three of `shape_verts`' mechanisms are deliberately absent:
    ///
    /// * **No snap** (§2.7). There is no pixel grid to snap a diagonal
    ///   to; rounding its ends would move the path the caller drew.
    /// * **No split** (§7b). The frame of five rectangles is
    ///   axis-aligned, and a stroke has no interior to save — its core
    ///   is a pixel wide and [`CORE_MIN`] would refuse it anyway.
    /// * **No weld** (§2.10). The offer compares centre, half sizes and
    ///   corners, which the two arms of a CROSS share exactly
    ///   (`checkbox.rs:70-71` draws that very pair) — and they are not
    ///   the same silhouette. An oriented record neither joins one nor
    ///   offers itself, and it closes any offer standing, which is what
    ///   drawing anything at all does.
    ///
    /// The frame must be ORTHONORMAL: the padding below is stated in
    /// local units and spent as screen pixels, which is the same number
    /// only when a local unit is a pixel and the axes are square to
    /// each other. Everything [`Frame`] builds is; a sheared frame
    /// would have to divide the pad by the sine of its own angle.
    fn oriented_fill(&mut self, f: Frame, half: [f32; 2], corners: [Corner; 4], colour: Color) {
        debug_assert!(
            (f.ux[0] * f.ux[0] + f.ux[1] * f.ux[1] - 1.0).abs() < 1e-3
                && (f.ux[0] * f.uy[0] + f.ux[1] * f.uy[1]).abs() < 1e-3,
            "an oriented frame must be orthonormal"
        );
        if !(half[0] > 0.0) || !(half[1] > 0.0) {
            return;
        }
        // The same ceiling `ring_verts` and `shape_verts` keep: a
        // corner deeper than the short half side would meet itself.
        let cap = half[0].min(half[1]);
        let mut flags = (ShapeKind::Box.code() << Shape::KIND_SHIFT) | Shape::FILL;
        let mut corner = [0.0f32; 4];
        for (i, c) in corners.iter().enumerate() {
            flags |= (c.style as u32) << (2 * i as u32);
            corner[i] = c.size.clamp(0.0, cap);
        }
        let idx = self.shapes.len() as u32;
        self.shapes.push(Shape {
            half,
            stroke: 0.0,
            feather: 0.0,
            corner,
            stroke_c: [0.0; 4],
            flags,
            arc_half: 0.0,
            arc_dir: 0.0,
            _pad: 0.0,
            // The oblique lane draws ticks, arms and joint discs, and
            // none of them is a surface: no frost has an orientation.
            tint: [0.0; 4],
        });
        let ext = [half[0] + AA_PAD, half[1] + AA_PAD];
        let l = [
            [-ext[0], -ext[1]],
            [ext[0], -ext[1]],
            [ext[0], ext[1]],
            [-ext[0], ext[1]],
        ];
        let col = colour.to_array();
        self.run_for(Some(SHAPE));
        let v = |i: usize| Vertex {
            pos: f.to_screen(l[i]),
            uv: l[i],
            color: col,
            shape: idx,
        };
        self.verts
            .extend_from_slice(&[v(0), v(1), v(2), v(0), v(2), v(3)]);
        self.seal();
        self.weld = None;
    }

    /// The vector lane's straight segment (f3 §3.1): the oriented box
    /// [`DrawList::line`] has always drawn, given the field that makes
    /// its two long edges analytic instead of quantised.
    ///
    /// Ends stay CUT SQUARE across the path — `line`'s own contract,
    /// and the reason a polyline can be built out of these at all. The
    /// cap that a stroke needs at a corner is a separate disc, laid by
    /// [`DrawList::polyline`], because a cap on the segment would be
    /// drawn twice at every joint.
    ///
    /// `false` where there is no segment to speak of, so the caller can
    /// fall back to the geometry it would have drawn.
    fn segment_verts(&mut self, a: [f32; 2], b: [f32; 2], t: f32, color: Color) -> bool {
        if !(t > 0.0) {
            return false;
        }
        let Some((f, len)) = Frame::along(a, b) else {
            return false;
        };
        // §2.8's energy rule, in the one domain where it fires: the
        // snapped lane has already rounded every sub-pixel band away,
        // and a diagonal has no grid to round to. See [`thin_band`].
        let (w, dim) = thin_band(t);
        self.oriented_fill(
            f,
            [len * 0.5, w * 0.5],
            [Corner::SQUARE; 4],
            color.fade(dim),
        );
        true
    }

    /// The disc that closes one joint of a polyline (f3 §3.1): radius
    /// `t/2` at the corner, tangent to both segments' long edges, so
    /// the union is the round join a stroked path is supposed to have.
    ///
    /// It is a Box record with round corners as big as its own half
    /// size — `d_round(p, [r, r], r)` IS `|p| − r`, proved in
    /// [`crate::sdf`] — so a joint costs one record and one quad and
    /// the shader learns nothing. It does NOT go through `shape_verts`,
    /// which would snap it to the pixel grid: a corner sits where the
    /// path put it, and moving it half a pixel would bend the line.
    ///
    /// It takes the same [`thin_band`] treatment as the arms it joins,
    /// and for the arms' reason rather than its own: a disc's mass goes
    /// as the square of its radius, not as its width, so this dims the
    /// joint slightly further than its area alone would ask. Matching
    /// the strokes it belongs to is what a joint is for, and both
    /// numbers are under a pixel wherever the rule fires at all.
    fn joint_verts(&mut self, at: [f32; 2], t: f32, color: Color) {
        let (w, dim) = thin_band(t);
        let r = w * 0.5;
        self.oriented_fill(
            Frame::upright(at),
            [r, r],
            [Corner::round(r); 4],
            color.fade(dim),
        );
    }

    /// Re-cuts a welded bed's frame at a deeper band (f3 §7b): the core
    /// shrinks by exactly the border that just joined the record, and
    /// the four strips grow to meet it.
    ///
    /// A rewrite rather than an emission, because the layout is fixed —
    /// the cores, then top, bottom, left, right, six vertices each —
    /// and every vertex keeps the colour and the record index it
    /// already had. Only positions and the uv that trails them move.
    /// Where the new band swallows the shape whole the core collapses
    /// to nothing and the two horizontal strips meet on the centre
    /// line, which is the unsplit quad again: correct, and five quads
    /// where one would have done. That costs four degenerate triangles
    /// on a shape whose border is half its size, and it buys the
    /// invariant that a bed never has to guess how deep a border it has
    /// not seen yet will be.
    ///
    /// A frosted bed has TWO cores over the one rectangle — the frost
    /// on the glass lane and the wash over it (§3.3) — so the count is
    /// read off the geometry rather than assumed. Both take the core
    /// rect; only the four strips carry a local origin, because only
    /// they read the field.
    fn respan_frame(&mut self, bed: &Weld, band: f32) {
        let quads = (bed.verts - bed.from) / 6;
        debug_assert!(quads >= 5, "a frame is four strips and at least one core");
        debug_assert_eq!((bed.verts - bed.from) % 6, 0, "a frame is whole quads");
        let cores = quads - 4;
        let ext = [bed.half[0] + AA_PAD, bed.half[1] + AA_PAD];
        let frame = frame_rects(bed.centre, ext, core_half(bed.half, band));
        for i in 0..quads {
            let at = bed.from + i * 6;
            let (colour, shape) = (self.verts[at].color, self.verts[at].shape);
            // The cores share the first rect; the strips follow it in
            // order.
            let r = if i < cores {
                bed_rect(frame[0], bed.centre, colour)
            } else {
                frame[i - cores + 1]
            };
            let local = (i >= cores).then_some(bed.centre);
            self.verts[at..at + 6].copy_from_slice(&quad6(r, local, colour, shape));
        }
    }

    /// How many shape records the list has written — the host's marker
    /// for a post-emission transform, the twin of `verts.len()`
    /// (f3 §2.9).
    pub fn shape_len(&self) -> usize {
        self.shapes.len()
    }

    /// The frame's shape records, in emission order — what the renderer
    /// uploads beside the vertices.
    pub fn shapes(&self) -> &[Shape] {
        &self.shapes
    }

    /// The records from `range` on, mutably: a board ride dims
    /// `stroke_c` here exactly as it dims vertex colours, or the two
    /// fall out of step mid-flight (f3 §2.9, R8).
    pub fn shapes_mut(&mut self, range: std::ops::RangeFrom<usize>) -> &mut [Shape] {
        &mut self.shapes[range]
    }

    /// Splits every following BOX shape quad into an n×n grid for the
    /// duration of a post-emission transform (f3 §2.3): the affine
    /// interpolation error of a perspective ride falls as 1/n². 1 = one
    /// quad, and §2.7's edge snap applies only there. Nothing outside
    /// shapes is affected. Reset to 1 by [`DrawList::clear`].
    ///
    /// NOT every shape, and the word BOX above is the whole of it: the
    /// oblique lane (capsule, disc, polygon) emits its own geometry
    /// from a local frame and does not read this at all. On a ride, a
    /// dash of a focus ring or a plugin's diagonal wire therefore
    /// carries the affine error this call exists to remove. Measured
    /// and left standing on purpose — the ride dims and moves whole
    /// panels, and a hairline inside one is below the error of the
    /// panel's own corner — but it is a limit, not a property.
    pub fn set_warp(&mut self, n: u8) {
        self.warp = n.max(1);
        // The quads of an open bed were emitted under the old grid; a
        // border welded onto it after the change would ride them. The
        // geometry check would catch it — the snap differs — but not
        // for a rect already on the grid, so the offer is withdrawn
        // here rather than left to a coincidence.
        self.weld = None;
    }

    /// Arms the vector lane: ring/ring_fill emit one SDF record and one
    /// quad instead of tessellating. The application sets this from the
    /// theme's `render.vector`; the list itself reads no tokens.
    ///
    /// **The reader is the host, and it exists** — since f3 K3a it is
    /// `nacelle-desktop/src/vector.rs`, which arms every screen's list
    /// at the frame boundary and re-arms it whenever the answer moves.
    /// It could not be here: a reader has to be someone who owns a list
    /// and knows the frame boundary, and in this library nobody does.
    /// Every production `DrawList` belongs to the host, which builds
    /// one per screen, clears it per frame and hands `shapes()` to the
    /// renderer. Reading the token in `clear()` would put the answer in
    /// the right file and the wrong place — it would make a MODE into
    /// frame state on behalf of an object that reads no tokens at all
    /// by design.
    ///
    /// `clear()` does not touch the flag, and that is load-bearing
    /// rather than incidental: it is what lets the host set the mode
    /// once per theme instead of once per frame
    /// (`the_record_carries_the_flags_and_clear_resets_the_frame`).
    pub fn set_vector(&mut self, on: bool) {
        self.vector = on;
        self.weld = None;
    }

    /// The blurred scene behind a SHAPE — [`DrawList::blur`]'s corner-aware
    /// counterpart, and the first emitter the `GLASS_RANK_*` handles ever
    /// had. `blur()` emits a rectangle, and the renderer's scissor is a
    /// rectangle too, so a frosted panel with rounded corners would poke
    /// past its own silhouette at every arc. The fragment shader samples by
    /// SCREEN POSITION and ignores uv (`shaders.rs: pos.xy / pc.screen`),
    /// so the geometry is free — this fans the same boundary `ring_fill`
    /// draws, and the frost ends exactly where the surface does.
    ///
    /// `rank` picks the pyramid depth the renderer serves (clamped there to
    /// what the frame actually wrote); 0 is not a rank — a surface with no
    /// glass simply does not call this.
    ///
    /// **K3b, and what it moved** (§3.3). Off the vector lane this is
    /// what it always was: a fan of the silhouette on `GLASS_RANK_*`,
    /// hard-edged like everything else on the screen, and the picture
    /// is bit for bit the shipped one. On the lane it becomes the same
    /// three layers the plan asked for:
    ///
    /// * the **core** — an axis-aligned quad inside the band, on
    ///   `GLASS_RANK_*`, the same handle and the same tint. The
    ///   interior of a silhouette has no edge, so there is nothing
    ///   there to antialias and nothing there to change;
    /// * the **band** — four strips on [`SHAPE_GLASS_1`]`..3`, carrying
    ///   the record this call opens: tint on the record, wash on the
    ///   vertex, the pyramid sample taken by screen position, and all
    ///   of it composed in ONE fragment under ONE coverage.
    ///
    /// The wash and the border are not drawn here and never were: they
    /// arrive as the `ring_fill`/`ring` pair every framed surface in
    /// the toolkit writes (`window.rs`, `elev.rs`), and they WELD into
    /// the record this call leaves open (§2.10). That is the whole
    /// point — three draws, one silhouette, one antialiased edge. Drawn
    /// as three coverages they would leave `c·(1 − c)·a·b` of excess
    /// alpha on the shared boundary, which is R4 by another name and
    /// reads as a heavy rim exactly where the eye goes.
    ///
    /// The fan could not be welded and was never going to be: it reads
    /// a blurred target through a different pipeline, so it cannot join
    /// a record that reads none. What K3b did was give the record a
    /// pipeline that reads one.
    pub fn glass_fill(&mut self, r: Rect, c: &[Corner; 4], segments: u8, depth: f32, tint: Color) {
        let depth = depth.clamp(1.0, 3.0);
        self.cmd(|| DrawCmd::GlassFill {
            r: [r.x, r.y, r.w, r.h],
            corners: *c,
            depth,
            tint,
        });
        if r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        // A FRACTIONAL depth is two layers: the lower rank in full, the higher
        // one at the fraction's alpha, and the blend between them IS the
        // interpolation — the pyramid has three rungs and the renderer binds
        // one target per run, so the mixing that would otherwise need a
        // two-sampler pipeline happens in the blender instead. Exact at full
        // effect opacity; at partial opacity the base leaks in slightly more
        // than a true three-way mix would allow, which is invisible next to
        // the blur it buys. One layer when the depth lands on a rung.
        //
        // ON THE VECTOR LANE THIS USED TO BE WHERE A SECOND EDGE SURVIVED
        // A FROST (K3c). `ring_grad` is still the other survivor on the
        // lane, for a reason of its own, and says so at its own door —
        // that one needs a wider record and a renderer change, and stays
        // down. THIS one did not: two rungs are still two runs and two
        // records (one pyramid target per run, §K3b), but a record's
        // OUTER EDGE is the BAND, not the core, and the band's tint is
        // `Frost::band_tint` now — a field the core never reads. So the
        // core keeps blending both rungs (exact, since its coverage is 1
        // throughout and two sequential blends of a constant alpha are
        // not a double count), and the band is given to ONE rung only:
        // the lower rung's band is silenced (alpha 0, its core still
        // shows), the upper rung's band carries the two rungs' COMBINED
        // alpha `a1 + a2 − a1·a2` — the standard over-composite of two
        // fully-covering layers, which is exactly what one coverage
        // evaluation is supposed to fold in. Closing the CORE the same
        // way — one fragment sampling two pyramid targets — is still the
        // renderer's, and still not this repository's; see
        // `Frost::band_tint`.
        let lo = depth.floor().clamp(1.0, 3.0);
        let frac = depth - lo;
        let two = frac > 0.01 && lo < 3.0;
        if two {
            let a1 = tint.a;
            let a2 = a1 * frac;
            let mut lower_band = tint;
            lower_band.a = 0.0;
            let mut t2 = tint;
            t2.a = a2;
            let mut upper_band = tint;
            upper_band.a = a1 + a2 - a1 * a2;
            self.glass_layer(r, c, segments, lo as u8, tint, false, Some(lower_band));
            self.glass_layer(r, c, segments, lo as u8 + 1, t2, true, Some(upper_band));
        } else {
            self.glass_layer(r, c, segments, lo as u8, tint, true, None);
        }
    }

    /// One frosted layer of one rank, down whichever lane is armed —
    /// the field's core-and-band on the vector lane, the tessellated
    /// fan off it.
    ///
    /// `bed` says whether this layer is the one the wash will land on,
    /// and only the TOP layer ever is: a wash lies over all the frost
    /// there is, so an empty bed under the upper rung would be six
    /// vertices and a run that can never be given a colour.
    ///
    /// `band_tint` is [`Frost::band_tint`] passed straight through —
    /// `None` off a whole-rung depth, where the record's band is still
    /// exactly this layer's `tint`. The tessellated fan reads neither:
    /// it has no record to carry a second colour on and no double-edge
    /// defect to answer (§K3c on `glass_fill`), so it stays the shipped
    /// picture regardless.
    fn glass_layer(
        &mut self,
        r: Rect,
        c: &[Corner; 4],
        segments: u8,
        rank: u8,
        tint: Color,
        bed: bool,
        band_tint: Option<Color>,
    ) {
        if self.vector {
            self.shape_verts(&ShapeSpec {
                rect: r,
                corners: *c,
                kind: ShapeKind::Box,
                // An EMPTY bed, laid now because a weld recolours quads
                // and never adds one: the wash arrives in a later call
                // (`ring_fill`) and needs a quad already in place, cut
                // to a geometry only this call knows.
                fill: bed.then_some(Color::TRANSPARENT),
                stroke: None,
                glass: Some(Frost { rank, tint, band_tint }),
                soft: None,
            });
        } else {
            self.glass_fan(r, c, segments, rank, tint);
        }
    }

    /// One fan of one rank — the tessellated lane's frosted layer, and
    /// the whole of it before K3b.
    fn glass_fan(&mut self, r: Rect, c: &[Corner; 4], segments: u8, rank: u8, tint: Color) {
        let img = glass_rank_handle(rank);
        let mut pts = std::mem::take(&mut self.scratch_a);
        ring_points(r, c, segments, &mut pts);
        let n = pts.len();
        if n >= 3 {
            let (mut cx, mut cy) = (0.0f32, 0.0f32);
            for p in &pts {
                cx += p[0];
                cy += p[1];
            }
            let inv = 1.0 / n as f32;
            let (cx, cy) = (cx * inv, cy * inv);
            let col = tint.to_array();
            for i in 0..n {
                let j = (i + 1) % n;
                self.push_tri_c(Some(img), [[cx, cy], pts[i], pts[j]], [col; 3]);
            }
        }
        self.scratch_a = pts;
    }

    /// Quadrilateral with a colour per vertex — the entry point every
    /// gradient is built on (r1 §6). A two-stop gradient interpolated in
    /// output space is affine in (x, y), and Gouraud reproduces an affine
    /// function exactly on any triangulation: no diagonal seam, no bands,
    /// 6 verts at any angle.
    pub fn quad_c(&mut self, p: [[f32; 2]; 4], c: [Color; 4]) {
        self.cmd(|| DrawCmd::QuadC { p, c });
        let (u, v) = FontSystem::white_uv();
        self.push_quad4(
            None,
            p,
            [[u, v]; 4],
            [c[0].to_array(), c[1].to_array(), c[2].to_array(), c[3].to_array()],
        );
    }

    /// Rect under a linear gradient, banded only where it must be. `stops`
    /// are (position 0..1 along the axis, colour), positions clamped and
    /// forced non-decreasing; `angle` is radians, 0 = left→right,
    /// π/2 = top→bottom (y down), t = 0 at the least-projected corner.
    /// Two stops spanning 0..1 are EXACTLY free at any angle — one quad,
    /// corner colours evaluated in output space (see quad_c). Anything
    /// else is piecewise affine, so it becomes one band per stop interval:
    /// the rect clipped to the slab between two stops, each band exact,
    /// seams sharing bitwise-identical vertices — 8 stops = 7 bands =
    /// 42 verts on an axis-aligned angle. Multi-stop or OKLab-space
    /// gradients arrive here already sampled: the resolver did that, the
    /// list never reads tokens.
    pub fn rect_grad(&mut self, r: Rect, stops: &[(f32, Color)], angle: f32) {
        self.cmd(|| DrawCmd::RectGrad {
            r: [r.x, r.y, r.w, r.h],
            stops: stops.to_vec(),
            angle,
        });
        if r.w <= 0.0 || r.h <= 0.0 || stops.is_empty() {
            return;
        }
        if stops.len() == 1 {
            self.rect_verts(r.x, r.y, r.w, r.h, stops[0].1);
            return;
        }
        // Corners tl,tr,br,bl with their normalised projection onto the
        // axis. Normalising by the observed min/max makes the extreme
        // corners land on t = 0 and t = 1 exactly, at any angle.
        let (sin_a, cos_a) = angle.sin_cos();
        let p = [
            [r.x, r.y],
            [r.x + r.w, r.y],
            [r.x + r.w, r.y + r.h],
            [r.x, r.y + r.h],
        ];
        let proj = [
            p[0][0] * cos_a + p[0][1] * sin_a,
            p[1][0] * cos_a + p[1][1] * sin_a,
            p[2][0] * cos_a + p[2][1] * sin_a,
            p[3][0] * cos_a + p[3][1] * sin_a,
        ];
        let (mut lo, mut hi) = (proj[0], proj[0]);
        for &v in &proj[1..] {
            lo = lo.min(v);
            hi = hi.max(v);
        }
        let span = (hi - lo).max(1e-6);
        let s0 = stops[0].0.clamp(0.0, 1.0);
        let s_last = stops[stops.len() - 1].0.clamp(0.0, 1.0);
        if stops.len() == 2 && s0 == 0.0 && s_last == 1.0 {
            let t = |i: usize| (proj[i] - lo) / span;
            let c = |i: usize| lerp(stops[0].1, stops[1].1, t(i)).to_array();
            let (u, v) = FontSystem::white_uv();
            self.push_quad4(None, p, [[u, v]; 4], [c(0), c(1), c(2), c(3)]);
            return;
        }
        // Band edges: 0, every stop, 1 — the flat caps fall out as bands
        // between two equal colours, zero-width bands are skipped. Within a
        // band the stop function is affine by construction.
        let corners: [([f32; 2], f32); 4] = [
            (p[0], (proj[0] - lo) / span),
            (p[1], (proj[1] - lo) / span),
            (p[2], (proj[2] - lo) / span),
            (p[3], (proj[3] - lo) / span),
        ];
        let band = |a: (f32, Color), b: (f32, Color), list: &mut Self| {
            if b.0 - a.0 <= 1e-6 {
                return;
            }
            let mut buf1 = [([0.0f32; 2], 0.0f32); 8];
            let mut buf2 = [([0.0f32; 2], 0.0f32); 8];
            let n1 = clip_t(&corners, a.0, true, &mut buf1);
            let n2 = clip_t(&buf1[..n1], b.0, false, &mut buf2);
            if n2 < 3 {
                return;
            }
            let colour = |t: f32| lerp(a.1, b.1, (t - a.0) / (b.0 - a.0)).to_array();
            let (v0, t0) = buf2[0];
            for i in 1..n2 - 1 {
                let (v1, t1) = buf2[i];
                let (v2, t2) = buf2[i + 1];
                list.push_tri_c(None, [v0, v1, v2], [colour(t0), colour(t1), colour(t2)]);
            }
        };
        let mut prev = (0.0f32, stops[0].1);
        let mut running = 0.0f32;
        for &(pos, col) in stops {
            running = running.max(pos.clamp(0.0, 1.0));
            band(prev, (running, col), self);
            prev = (running, col);
        }
        band(prev, (1.0, prev.1), self);
    }

    /// Triangle fan around `centre`, one colour per rim point, the rim
    /// CLOSED — rim[k−1] joins rim[0]: hexagons, reticles, radial washes
    /// (r1 §6.3, 3·k verts). An open wedge is quad_c's job; closing is what
    /// makes a k-gon k triangles. Fewer than 3 rim points draw nothing.
    ///
    /// **Nothing calls this.** f3 §3.2 planned a `fringe()` for the fan,
    /// `quad_c` and `rect_grad`; K4's census (2026-08-17, grep across
    /// libnacelle, nacelle-desktop, nacelle-addons and nacelle-ai) found
    /// ZERO production callers of all three — the only mentions outside
    /// this file's own tests are the command register's formatting and
    /// this documentation. A silhouette nobody draws cannot be graded
    /// against anything, so K4 leaves the three of them exactly as they
    /// are rather than antialiasing a picture that is not on screen.
    /// The hexagon a reticle would want has a home already: it is
    /// `ShapeKind::Hex`, waiting on K6.
    pub fn fan_c(&mut self, centre: [f32; 2], rim: &[[f32; 2]], c_centre: Color, c_rim: &[Color]) {
        self.cmd(|| DrawCmd::FanC {
            centre,
            c_centre,
            // Paired, and cut to the shorter of the two: the fan draws
            // that many wedges and the register may not claim more.
            rim: rim.iter().copied().zip(c_rim.iter().copied()).collect(),
        });
        let n = rim.len().min(c_rim.len());
        if n < 3 {
            return;
        }
        let cc = c_centre.to_array();
        for i in 0..n {
            let j = (i + 1) % n;
            self.push_tri_c(
                None,
                [centre, rim[i], rim[j]],
                [cc, c_rim[i].to_array(), c_rim[j].to_array()],
            );
        }
    }

    /// An image quad with explicit texture coordinates, corner order
    /// tl,tr,br,bl: sub-rect sprites, tiled decoration under a REPEAT
    /// sampler, the scanline plate's drifting window (r1 §6.3). The tint
    /// multiplies as in image(); the UVs are the caller's business — a
    /// reserved handle here is deliberate, not policed, because the sprite
    /// glow endgame is exactly ADD_ATLAS with explicit UVs.
    pub fn image_uv(&mut self, r: Rect, uv: [[f32; 2]; 4], id: ImageId, tint: Color) {
        self.cmd(|| DrawCmd::ImageUv {
            r: [r.x, r.y, r.w, r.h],
            uv,
            id,
            tint,
        });
        let p = [
            [r.x, r.y],
            [r.x + r.w, r.y],
            [r.x + r.w, r.y + r.h],
            [r.x, r.y + r.h],
        ];
        self.push_quad4(Some(id), p, uv, [tint.to_array(); 4]);
    }

    /// One quad over the soft-mask sprite, adding or covering — the
    /// "ADD_ATLAS with explicit UVs" endgame [`DrawList::image_uv`]'s
    /// comment names, but with the coordinates in the SPRITE's own 0..1
    /// space rather than the atlas's. Each uv is clamped to the unit
    /// square and mapped into `band` (`FontSystem::mask_soft_uv()`,
    /// passed by the caller — the list keeps no font-system state), so a
    /// caller can address the disk's profile and nothing else: glyph
    /// texels stay unreachable whatever numbers arrive, which is what
    /// lets the plugin ABI expose this without policing its input. An
    /// EMPTY band (u1 ≤ u0 or v1 ≤ v0) is the maskless degenerate
    /// case and falls back to the atlas's white pixel — a solid quad,
    /// raw but present, the same discipline as `soft_box`. `additive`
    /// picks light (the ADD_ATLAS run — glow) over cover (the normal
    /// atlas run — shadow).
    pub fn mask_quad(
        &mut self,
        p: [[f32; 2]; 4],
        uv: [[f32; 2]; 4],
        band: (f32, f32, f32, f32),
        color: Color,
        additive: bool,
    ) {
        self.cmd(|| DrawCmd::MaskQuad { p, uv, color, additive });
        if color.a <= 0.0 {
            return;
        }
        let (u0, v0, u1, v1) = band;
        let m: [[f32; 2]; 4] = if u1 <= u0 || v1 <= v0 {
            let (u, v) = FontSystem::white_uv();
            [[u, v]; 4]
        } else {
            std::array::from_fn(|i| {
                [
                    u0 + (u1 - u0) * uv[i][0].clamp(0.0, 1.0),
                    v0 + (v1 - v0) * uv[i][1].clamp(0.0, 1.0),
                ]
            })
        };
        let image = if additive { Some(ADD_ATLAS) } else { None };
        self.push_quad4(image, p, m, [color.to_array(); 4]);
    }

    /// One quad over an SVG icon's own placement in the shared atlas
    /// (K8) — [`FontSystem::icon`]'s uv rect, sampled corner-for-corner
    /// and tinted by `color`, exactly the way a glyph run already is.
    ///
    /// `id` is the icon's id ([`FontSystem::register_icon`] or the
    /// interned [`FontSystem::icon_id`]) and `px` the RASTER size to
    /// resolve it at — not necessarily the size `p` draws it into: a
    /// caller free to stretch a 32px rasterization across a 40px box
    /// pays a resample, the same tolerance a glyph already has between
    /// how it was rasterized and how big it is drawn. Unlike
    /// [`DrawList::mask_quad`] the uv rect is not a caller-addressed
    /// sprite space mapped through a `band`: it comes straight from
    /// `fonts.icon`, which is also the reason this takes `fonts`
    /// explicitly rather than reading atlas state of its own — the
    /// list draws, [`FontSystem`] alone knows where anything landed.
    ///
    /// An unregistered `id`, `px == 0`, or an atlas with no room this
    /// frame all answer the same way: [`FontSystem::icon`] returns
    /// `None`, the command is still RECORDED (a caller reading the
    /// register back sees the icon that was asked for even on the frame
    /// it could not be drawn), and nothing is pushed to the vertex
    /// buffer — "try again next frame", not an error, exactly like an
    /// atlas-full glyph.
    pub fn icon_quad(
        &mut self,
        fonts: &mut FontSystem,
        id: u32,
        px: u32,
        p: [[f32; 2]; 4],
        color: Color,
    ) {
        self.cmd(|| DrawCmd::IconQuad { p, icon: id, color });
        if color.a <= 0.0 {
            return;
        }
        let Some(g) = fonts.icon(id, px) else { return };
        let uv = [[g.u0, g.v0], [g.u1, g.v0], [g.u1, g.v1], [g.u0, g.v1]];
        self.push_quad4(None, p, uv, [color.to_array(); 4]);
    }

    /// Glow OUTSIDE the silhouette, in an ADDITIVE run — the pipeline
    /// adds light instead of filming milk over a lit backdrop.
    ///
    /// # The vector lane (f3 §2.6, and §4.6 of the scope decision)
    ///
    /// With `render.vector` on this is ONE record and one quad: the
    /// caller's own silhouette, `radius` in [`Shape::feather`], bits
    /// [`Shape::GAUSS`] and [`Shape::OUTSIDE_ONLY`] set, on
    /// [`SHAPE_ADD`]. The record's quad grows by `radius` so the
    /// profile has room to reach zero — that growth is the FIRST thing
    /// this step had to get right, and `shape_verts` says why at the
    /// envelope.
    ///
    /// Three things the sprite could not do and the field does. The
    /// profile is the same gauss everywhere, corners included, instead
    /// of a 2-texel strip laid perpendicular to a polyline. The inner
    /// boundary is antialiased against the panel standing on it —
    /// `OUTSIDE_ONLY` masks by AREA, not by a step — where the sprite's
    /// was the tessellation's own staircase. And the cost stops
    /// depending on the corner: 6 vertices whatever the radius, against
    /// 24 square, 48 chamfered and 168 at round S=6.
    ///
    /// # The tessellated lane, unchanged
    ///
    /// The technique (r1 §4.1/§4.3): the soft-disk mask from the R8
    /// band, laid along the ring's OWN path — the outline, every corner
    /// in its declared style, extruded outward by `radius`, with the
    /// disk's 2-texel cardinal strip across the extrusion. A rounded
    /// corner glows on its own arc grown by the glow, a chamfered
    /// corner along its diagonal, a square corner mitres. Nothing is
    /// emitted inside the path, so the glow never tints a translucent
    /// fill. `mask_uv` is `FontSystem::mask_soft_uv()`, passed by the
    /// caller (Ctx has it; the draw list stays free of the font
    /// system).
    ///
    /// An EMPTY `mask_uv` (u1 ≤ u0 or v1 ≤ v0) is the maskless
    /// degenerate case, and its answer is now the FIELD: the analytic
    /// glow reads no atlas at all, so a run with no baked sprite has
    /// something better to fall back to than an approximation of one.
    /// This is what retired `glow_shell` — r1 §4.1's concentric shells,
    /// 3 to 5 ring strokes deep, up to 840 vertices around a rounded
    /// panel, with a quadratic stand-in for the tail and a Square
    /// corner that stayed square however far the glow grew. It existed
    /// because "a themeless run must still draw something raw", and
    /// the raw thing to draw is no longer an approximation.
    ///
    /// **Both lanes are one record's worth of intent**: the register
    /// records `glow_ring` either way, because which lane tessellated
    /// is exactly the knob it must not see.
    pub fn glow_ring(
        &mut self,
        r: Rect,
        c: &[Corner; 4],
        segments: u8,
        radius: f32,
        color: Color,
        mask_uv: (f32, f32, f32, f32),
    ) {
        self.glow_ring_with(r, c, segments, radius, color, mask_uv, GlowProfile::HALO);
    }

    /// [`DrawList::glow_ring`] with the light SHAPED — the one emitter,
    /// asked for a profile instead of assuming the soft disk's own.
    ///
    /// The halo above is this call at [`GlowProfile::HALO`], and the
    /// equality is exact rather than approximate: at one band, a decay of
    /// 1 and no aura every number below reduces to the statement the
    /// halo used to make, vertex for vertex. That is deliberate and it is
    /// the reason there is no second emitter — a neon tube is not
    /// another way of drawing light, it is the same soft disk with its
    /// DISTANCE re-mapped and its innermost band lifted.
    ///
    /// THE RE-MAP. One quad ring lays the mask's profile linearly across
    /// the whole reach: at half the radius the fragment samples the disk
    /// at half its own radius. Cut the reach into bands and the sample
    /// point at each BOUNDARY is ours to choose, so band `k` of `n` puts
    /// its outer rim at the disk's `(k/n)^(1/decay)` instead of its
    /// `k/n`. A decay above 1 therefore spends the disk's profile in the
    /// first fraction of the reach and leaves the rest of it in the tail
    /// — light that stops instead of fading, which is what a lit tube
    /// does and a Gaussian blur cannot. Inside a band the mask is still
    /// sampled continuously, so the picture is smooth however few bands
    /// there are; the count decides how closely the re-map is followed,
    /// and the theme owns it ([`GlowProfile`]).
    ///
    /// THE AURA is a per-vertex alpha and nothing else: the boundaries
    /// inside `aura_reach` carry `aura` times the caller's alpha, ramped
    /// to one at the reach. WHICH IS WHY THE REACH IS ITS OWN BOUNDARY
    /// ([`GlowProfile::stops`]): between two rings the rasteriser
    /// interpolates, so an aura whose ramp ends between them goes on
    /// lifting pixels out to the next one, and the picture stops where
    /// the grid says instead of where the theme does. Clipped at 1.0
    /// whatever the product, because a blend factor outside 0..1 is not a
    /// brighter pixel — it is the undefined output the master's own note
    /// on negative alpha warns about.
    ///
    /// Bands never overlap (each one's inner rim is the last one's
    /// outer), so additive blending cannot double-brighten a seam —
    /// [`DrawList::glow_shell`]'s invariant, relied on for the same
    /// reason.
    ///
    /// WITHOUT A MASK BAND there is no soft disk to re-map and this falls
    /// to [`DrawList::glow_shell`], which draws the halo's own shape:
    /// the profile is DROPPED, and a tube asked for that way comes back
    /// an unshaped glow with no aura and no decay. It is unreachable from
    /// [`crate::object::window::panel_edge_glow`] — `FontSystem`'s soft
    /// band is a compile-time rectangle — but this is `pub`, the recipe
    /// for the next consumer sends callers here, and a silent degradation
    /// nobody wrote down is one somebody rediscovers. Pinned by
    /// `a_maskless_tube_falls_back_to_the_unshaped_shell`.
    pub fn glow_ring_with(
        &mut self,
        r: Rect,
        c: &[Corner; 4],
        segments: u8,
        radius: f32,
        color: Color,
        mask_uv: (f32, f32, f32, f32),
        profile: GlowProfile,
    ) {
        self.cmd(|| DrawCmd::GlowRing {
            r: [r.x, r.y, r.w, r.h],
            corners: *c,
            radius,
            color,
            profile,
        });
        if !(radius > 0.0) || color.a <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let (u0, v0, u1, v1) = mask_uv;
        // The vector lane takes a soft record and needs no sprite, and
        // NEITHER DOES THE SPRITE'S ABSENCE: `glow_shell` — the concentric
        // shells this used to fall back to — was retired by the record, so
        // the record answers both. One arm, because the two questions have
        // one answer now.
        //
        // The tube's own profile (aura, decay, bands) does NOT ride this
        // record: `Soft` carries a reach and a kind, not a ramp. K3d
        // (2026-08-23) paid down the debt this comment used to describe by
        // gating on the profile instead of on the lane alone: a HALO has
        // nothing a flat record could lose, so it takes the record on
        // either lane, but a profile that SHAPES the reach (`is_halo() ==
        // false` — a tube's `decay = 3.0`, or any aura) still takes the
        // strip below even with the switch raised, because that is the
        // only path that draws the shape the theme asked for. Carrying a
        // ramp into the record is real follow-on work, not a `self.vector`
        // check away.
        if (self.vector && profile.is_halo()) || u1 <= u0 || v1 <= v0 {
            self.shape_verts(&ShapeSpec {
                rect: r,
                corners: *c,
                kind: ShapeKind::Box,
                // The glow's colour rides the vertices, like every
                // other fill: [`Shape::FILL`] means "the interior wears
                // the vertex colour", and under `OUTSIDE_ONLY` the
                // profile is what decides how much of it lands where.
                fill: Some(color),
                stroke: None,
                glass: None,
                soft: Some(Soft { reach: radius * profile.cutoff, kind: SoftKind::Glow }),
            });
            return;
        }
        // The soft strip laid perpendicular to the path and OUTWARD from
        // it — the halo's one-way bleed and the tube's outer face alike.
        // A tube's INNER face, the light it throws onto the frame it edges,
        // is the same strip mirrored: [`glow_ring_inward_with`].
        self.glow_strip(r, c, segments, radius, color, mask_uv, profile, false);
    }

    /// One face of a soft ring's light: the profile laid perpendicular to
    /// the path, extruded `radius` px to one side of it.
    ///
    /// `inward = false` grows the strip AWAY from the rect — the halo and a
    /// tube's outer face, the picture this toolkit has always drawn.
    /// `inward = true` mirrors it INTO the rect, peak on the path and
    /// falling toward the middle, and is the tube's inner face
    /// ([`glow_ring_inward_with`]). The two differ in exactly the sign of
    /// the extrusion and, on the inner side, a clamp: the reach cannot pass
    /// half the shorter side or the ring's two facing runs would cross and
    /// fold. Everything else — the band re-map, the aura, the additive
    /// atlas, the mask sample — is the SAME arithmetic, which is why it is
    /// one function and not two.
    #[allow(clippy::too_many_arguments)]
    fn glow_strip(
        &mut self,
        r: Rect,
        c: &[Corner; 4],
        segments: u8,
        radius: f32,
        color: Color,
        mask_uv: (f32, f32, f32, f32),
        profile: GlowProfile,
        inward: bool,
    ) {
        let (u0, v0, u1, v1) = mask_uv;
        // u pinned to the centre of the stretchable band, v running from
        // the disk's peak on the path to the sprite's zero at the reach's
        // rim — the profile the nine-slice edges carried, now perpendicular
        // to the path everywhere, corners included. Point counts agree
        // because counts depend only on corner STYLE, which inset()
        // preserves.
        let su = u0 + (u1 - u0) * (32.0 / 64.0);
        let vi = v0 + (v1 - v0) * (31.0 / 64.0);
        // The inner face cannot reach past the panel's own middle: at that
        // depth the two runs coming off opposite edges meet, and beyond it
        // they would swap sides and the ring would turn itself inside out.
        // The outer face has no such ceiling and keeps the caller's radius.
        // `cutoff` is applied LAST, after that fold clamp — it shrinks
        // whichever reach the direction already settled on, so the
        // profile never draws further than either constraint alone
        // would allow.
        let reach =
            (if inward { radius.min(r.w.min(r.h) * 0.5) } else { radius }) * profile.cutoff;
        if !(reach > 0.0) {
            return;
        }
        let mut stops = [0.0f32; GlowProfile::MAX_BANDS as usize + 1];
        let n_stops = profile.stops(&mut stops);
        let mut inner = std::mem::take(&mut self.scratch_a);
        let mut outer = std::mem::take(&mut self.scratch_b);
        ring_points(r, c, segments, &mut inner);
        // The path itself, asked of the profile like every other boundary
        // rather than written out here: a distance of nothing is a
        // distance, and a second spelling of the peak is a second place for
        // it to drift.
        let (mut v_in, mut a_in) =
            (profile.v_at(0.0, vi, v0), profile.alpha_at(0.0, color.a));
        for &f in &stops[..n_stops] {
            let d = reach * f;
            // `g` is how far the boundary moves OUTWARD: `+d` for the outer
            // face, `-d` for the inner. The rect's origin shifts by `-g` and
            // its size by `+2g` (out grows it, in shrinks it), and the
            // corners move by `inset(-g)` — `Corner::inset` takes a positive
            // inset and a negative outset, so one sign carries the rect and
            // its arcs together. At `g = +d` this is byte for byte the
            // outward strip this emitter has always drawn.
            let g = if inward { -d } else { d };
            let grown = Rect::new(r.x - g, r.y - g, r.w + 2.0 * g, r.h + 2.0 * g);
            let ck = [c[0].inset(-g), c[1].inset(-g), c[2].inset(-g), c[3].inset(-g)];
            ring_points(grown, &ck, segments, &mut outer);
            let v_out = profile.v_at(f, vi, v0);
            let a_out = profile.alpha_at(f, color.a);
            let ci = Color { a: a_in, ..color }.to_array();
            let co = Color { a: a_out, ..color }.to_array();
            let n = inner.len().min(outer.len());
            for i in 0..n {
                let j = (i + 1) % n;
                self.push_quad4(
                    Some(ADD_ATLAS),
                    [inner[i], inner[j], outer[j], outer[i]],
                    [[su, v_in], [su, v_in], [su, v_out], [su, v_out]],
                    [ci, ci, co, co],
                );
            }
            std::mem::swap(&mut inner, &mut outer);
            v_in = v_out;
            a_in = a_out;
        }
        self.scratch_a = inner;
        self.scratch_b = outer;
    }

    /// A tube's INNER face: [`glow_ring_with`]'s strip thrown into the
    /// frame instead of off it.
    ///
    /// A neon tube set on the rim of a frame lights the surface it frames,
    /// not only the dark outside its outer edge — so NEON is drawn twice,
    /// the outer bloom by [`glow_ring_with`] and this inner one over the
    /// body. A HALO is a one-way bleed and never asks for this; the caller
    /// gates on the profile ([`GlowProfile::is_halo`]), which is the theme's
    /// own `falloff = tube` reaching the screen.
    ///
    /// It writes NO command to the register: the outer call already stands
    /// for "a tube is here", and the inner face is that one intent
    /// rendered, not a second glow to hash. Drawn AFTER the body fill by
    /// its one caller ([`crate::object::window::panel_edge_glow`], itself
    /// the rung's last act), so the additive light lands ON the frame and
    /// is not buried under an opaque fill.
    #[allow(clippy::too_many_arguments)]
    pub fn glow_ring_inward_with(
        &mut self,
        r: Rect,
        c: &[Corner; 4],
        segments: u8,
        radius: f32,
        color: Color,
        mask_uv: (f32, f32, f32, f32),
        profile: GlowProfile,
    ) {
        if !(radius > 0.0) || color.a <= 0.0 || r.w <= 0.0 || r.h <= 0.0 {
            return;
        }
        let (u0, v0, u1, v1) = mask_uv;
        // The vector lane cannot yet express an inside-only feather — its
        // soft glow is OUTSIDE_ONLY by construction — and a shaped profile
        // has nowhere to ride on it (the same gate as `glow_ring_with`,
        // K3d, 2026-08-23). The caller already only reaches this function
        // when `!profile.is_halo()` (`panel_edge_glow`'s own gate — a HALO
        // never asks for an inner face at all), so `is_halo()` is checked
        // here too rather than assumed, for the caller that has not been
        // written yet. Dropped on a maskless caller either way.
        if (self.vector && profile.is_halo()) || u1 <= u0 || v1 <= v0 {
            return;
        }
        self.glow_strip(r, c, segments, radius, color, mask_uv, profile, true);
    }

    /// The nine-slice core every soft sprite shape shares (r1 §4.3): `r`
    /// cut at `cell` px from each side, the mask's corner cells pinned to
    /// the corners, its 2-texel middle band stretched along the edges and
    /// across the centre. `centre = false` drops the middle quad (8 quads,
    /// 48 verts — the glows); `true` keeps it (9 quads, 54 — the fills).
    /// The 31/64 · 33/64 split is the mask-band CONTRACT's geometry (a
    /// 64-texel sprite whose stretchable middle is texels 31..33, r1 §4.2
    /// via font::MASK_SOFT), not a design value.
    fn nine_slice(
        &mut self,
        image: Option<ImageId>,
        r: Rect,
        cell: f32,
        uv: (f32, f32, f32, f32),
        color: Color,
        centre: bool,
    ) {
        let (u0, v0, u1, v1) = uv;
        let cell = cell.clamp(0.0, r.w.min(r.h) * 0.5);
        let xs = [r.x, r.x + cell, r.x + r.w - cell, r.x + r.w];
        let ys = [r.y, r.y + cell, r.y + r.h - cell, r.y + r.h];
        let (su, sv) = (u1 - u0, v1 - v0);
        let us = [u0, u0 + su * (31.0 / 64.0), u0 + su * (33.0 / 64.0), u1];
        let vs = [v0, v0 + sv * (31.0 / 64.0), v0 + sv * (33.0 / 64.0), v1];
        let col = color.to_array();
        for j in 0..3 {
            for i in 0..3 {
                if i == 1 && j == 1 && !centre {
                    continue;
                }
                self.push_quad4(
                    image,
                    [
                        [xs[i], ys[j]],
                        [xs[i + 1], ys[j]],
                        [xs[i + 1], ys[j + 1]],
                        [xs[i], ys[j + 1]],
                    ],
                    [
                        [us[i], vs[j]],
                        [us[i + 1], vs[j]],
                        [us[i + 1], vs[j + 1]],
                        [us[i], vs[j + 1]],
                    ],
                    [col; 4],
                );
            }
        }
    }

    /// FILLED soft rectangle: the same nine-slice with the centre kept, in
    /// a normal-blend atlas run — the shadow bed under a panel, not light.
    /// `radius` is the feather: alpha is zero exactly on the rect boundary
    /// and ramps to the disk's peak over `radius` px, so the whole soft
    /// shape stays INSIDE `r` (the caller inflates when it wants the blur
    /// to reach past an edge — shadow() below does exactly that). 54 verts.
    /// An empty `mask_uv` degrades raw to a plain filled rect.
    pub fn soft_box(&mut self, r: Rect, radius: f32, color: Color, mask_uv: (f32, f32, f32, f32)) {
        self.cmd(|| DrawCmd::SoftBox { r: [r.x, r.y, r.w, r.h], radius, color });
        self.soft_box_verts(r, radius, color, mask_uv);
    }

    fn soft_box_verts(&mut self, r: Rect, radius: f32, color: Color, mask_uv: (f32, f32, f32, f32)) {
        if r.w <= 0.0 || r.h <= 0.0 || color.a <= 0.0 {
            return;
        }
        let (u0, v0, u1, v1) = mask_uv;
        if u1 <= u0 || v1 <= v0 {
            self.rect_verts(r.x, r.y, r.w, r.h, color);
            return;
        }
        self.nine_slice(None, r, radius.max(0.0), mask_uv, color, true);
    }

    /// Drop shadow under a panel — normal blend, because a shadow
    /// subtracts by covering and is not light. Offset, radius and
    /// colour are the caller's tokens (`shadow.dx/dy`, `shadow.radius`,
    /// `shadow.color`); nothing here defaults them, and the master
    /// ships `shadow.color = none` on all nine rungs of `elev.*`.
    ///
    /// # The vector lane (f3 §2.6, and §4.7 of the scope decision)
    ///
    /// One record on the PLAIN shape lane: the caller's silhouette
    /// shifted by `offset`, `radius` in [`Shape::feather`], bit
    /// [`Shape::GAUSS`] and no `OUTSIDE_ONLY` — the interior is the
    /// profile's own plateau, which is what a shadow under a
    /// translucent panel has to be. THE OFFSET COSTS THE RECORD
    /// NOTHING: a shadow is its own record, so a shifted shadow is a
    /// shifted rect, resolved on the CPU before the record is written.
    ///
    /// Two defects of the sprite path go with it, and both are
    /// correctness rather than taste. The nine-slice STRETCHES the
    /// mask's middle band (see `nine_slice`), so today's shadow has one
    /// profile in the corners and a smeared one along a long side; the
    /// field has the same gauss everywhere. And the sprite is a
    /// RECTANGLE, so a rounded panel casts a square shadow; the record
    /// carries the panel's own corners.
    ///
    /// `c` is read by the vector lane alone — the sprite cannot follow
    /// a silhouette at all, which is the second defect stated as a
    /// signature.
    ///
    /// # The tessellated lane
    ///
    /// `soft_box` over `r` translated by `offset` and INFLATED by
    /// `radius`, because the sprite's feather runs inward: the plateau
    /// then still covers the panel's own footprint and the falloff
    /// reaches `radius` px past every edge of the shifted rect. The
    /// vector lane does not inflate, and must not — its feather runs
    /// outward from the silhouette the caller passed.
    pub fn shadow(
        &mut self,
        r: Rect,
        c: &[Corner; 4],
        offset: [f32; 2],
        radius: f32,
        color: Color,
        mask_uv: (f32, f32, f32, f32),
    ) {
        self.cmd(|| DrawCmd::Shadow {
            r: [r.x, r.y, r.w, r.h],
            corners: *c,
            offset,
            radius,
            color,
        });
        let radius = radius.max(0.0);
        if self.vector {
            self.shape_verts(&ShapeSpec {
                rect: Rect::new(r.x + offset[0], r.y + offset[1], r.w, r.h),
                corners: *c,
                kind: ShapeKind::Box,
                fill: Some(color),
                stroke: None,
                glass: None,
                soft: Some(Soft { reach: radius, kind: SoftKind::Shadow }),
            });
            return;
        }
        self.soft_box_verts(
            Rect::new(
                r.x + offset[0] - radius,
                r.y + offset[1] - radius,
                r.w + 2.0 * radius,
                r.h + 2.0 * radius,
            ),
            radius,
            color,
            mask_uv,
        );
    }

    fn glyph_quad(&mut self, g: &Glyph, pen_x: f32, baseline: f32, color: Color) {
        if g.w <= 0.0 {
            return;
        }
        let x0 = (pen_x + g.xmin).round();
        let y1 = (baseline - g.ymin).round(); // bitmap bottom
        let y0 = y1 - g.h;
        let x1 = x0 + g.w;
        self.push_quad(
            [[x0, y0], [x1, y0], [x1, y1], [x0, y1]],
            [
                [g.u0, g.v0],
                [g.u1, g.v0],
                [g.u1, g.v1],
                [g.u0, g.v1],
            ],
            color,
        );
    }

    /// Draws text; (x, y) is the top-left corner of the text box.
    #[allow(clippy::too_many_arguments)]
    pub fn text(
        &mut self,
        fs: &mut FontSystem,
        font: u8,
        px: f32,
        x: f32,
        y: f32,
        text: &str,
        color: Color,
        letter_spacing: f32,
    ) {
        self.text_fig(fs, font, px, x, y, text, color, letter_spacing, &Figures::NONE);
    }

    /// [`DrawList::text`] under a figure box (§5.17): every character the
    /// box holds is stepped by it and centred in it, so the run keeps its
    /// width when its digits change. `&Figures::NONE` is the proportional
    /// run every caller drew before the box existed.
    #[allow(clippy::too_many_arguments)]
    pub fn text_fig(
        &mut self,
        fs: &mut FontSystem,
        font: u8,
        px: f32,
        x: f32,
        y: f32,
        text: &str,
        color: Color,
        letter_spacing: f32,
        fig: &Figures,
    ) {
        self.cmd(|| DrawCmd::Text {
            at: [x, y],
            anchor: TextAnchor::Left,
            font,
            px,
            tracking: letter_spacing,
            tabular: fig.advance(),
            color,
            text: text.to_string(),
        });
        self.text_verts(fs, font, px, x, y, text, color, letter_spacing, fig);
    }

    /// The glyphs of [`DrawList::text`] without the command. Which
    /// glyphs a string becomes — and how many quads each one is worth —
    /// is the atlas's business; the register holds the STRING, so a
    /// change of rasteriser, of hinting or of the atlas's packing moves
    /// the vertex dump and leaves the register alone.
    #[allow(clippy::too_many_arguments)]
    fn text_verts(
        &mut self,
        fs: &mut FontSystem,
        font: u8,
        px: f32,
        x: f32,
        y: f32,
        text: &str,
        color: Color,
        letter_spacing: f32,
        fig: &Figures,
    ) {
        let (ascent, _) = fs.line_metrics(font, px);
        let baseline = y + ascent;
        // §5.16's fake bold: a slot that asked for >=600 and found only a
        // Regular file draws every glyph twice, the second pass offset by
        // `face.<id>.synthetic_bold` em. The ADVANCE is untouched — the
        // fake thickens ink and never widens a step — so a tabular column
        // measures the same whether its face is real or faked, and
        // `measure` needs to know nothing about this at all.
        let fake = fs.synthetic_bold(font) * px;
        let mut pen = x;
        for (prev, ch, next) in crate::font::with_neighbours(text) {
            let boxed = fig.advance_of(prev, ch, next);
            match fs.glyph(font, px, ch) {
                Some(g) => {
                    // Centred in its box rather than left-aligned in it: a
                    // narrow '1' beside a wide '8' has to keep the column's
                    // optical rhythm, which is the whole point of paying
                    // for the box in the first place.
                    let off = boxed.map_or(0.0, |a| Figures::centre_in(a, g.advance));
                    self.glyph_quad(&g, pen + off, baseline, color);
                    if fake > 0.0 {
                        self.glyph_quad(&g, pen + off + fake, baseline, color);
                    }
                    pen += boxed.unwrap_or(g.advance) + letter_spacing;
                }
                // The atlas filled up mid-frame and this glyph waits a
                // frame. A boxed character still steps its box, because a
                // tabular column that closes the gap around a missing
                // figure reflows the very thing the box exists to hold
                // still; an unboxed one behaves as it always has.
                None => {
                    if let Some(a) = boxed {
                        pen += a + letter_spacing;
                    }
                }
            }
        }
    }

    /// Text horizontally centered on cx.
    #[allow(clippy::too_many_arguments)]
    pub fn text_center(
        &mut self,
        fs: &mut FontSystem,
        font: u8,
        px: f32,
        cx: f32,
        y: f32,
        text: &str,
        color: Color,
        letter_spacing: f32,
    ) {
        self.text_center_fig(fs, font, px, cx, y, text, color, letter_spacing, &Figures::NONE);
    }

    /// [`DrawList::text_center`] under a figure box.
    #[allow(clippy::too_many_arguments)]
    pub fn text_center_fig(
        &mut self,
        fs: &mut FontSystem,
        font: u8,
        px: f32,
        cx: f32,
        y: f32,
        text: &str,
        color: Color,
        letter_spacing: f32,
        fig: &Figures,
    ) {
        self.cmd(|| DrawCmd::Text {
            at: [cx, y],
            anchor: TextAnchor::Centre,
            font,
            px,
            tracking: letter_spacing,
            tabular: fig.advance(),
            color,
            text: text.to_string(),
        });
        let w = fs.measure_fig(font, px, text, letter_spacing, fig);
        self.text_verts(fs, font, px, cx - w / 2.0, y, text, color, letter_spacing, fig);
    }

    /// Text right-aligned to the rx edge.
    #[allow(clippy::too_many_arguments)]
    pub fn text_right(
        &mut self,
        fs: &mut FontSystem,
        font: u8,
        px: f32,
        rx: f32,
        y: f32,
        text: &str,
        color: Color,
        letter_spacing: f32,
    ) {
        self.text_right_fig(fs, font, px, rx, y, text, color, letter_spacing, &Figures::NONE);
    }

    /// [`DrawList::text_right`] under a figure box. This is the alignment
    /// the box was asked for: a right-aligned numeric column under a box
    /// has a left edge that does not move when the number does.
    #[allow(clippy::too_many_arguments)]
    pub fn text_right_fig(
        &mut self,
        fs: &mut FontSystem,
        font: u8,
        px: f32,
        rx: f32,
        y: f32,
        text: &str,
        color: Color,
        letter_spacing: f32,
        fig: &Figures,
    ) {
        self.cmd(|| DrawCmd::Text {
            at: [rx, y],
            anchor: TextAnchor::Right,
            font,
            px,
            tracking: letter_spacing,
            tabular: fig.advance(),
            color,
            text: text.to_string(),
        });
        self.text_right_verts(fs, font, px, rx, y, text, color, letter_spacing, fig);
    }

    /// The glyphs of [`DrawList::text_right`] without the command — the
    /// right-hand half of a module title records itself as part of the
    /// title, not as a text run of its own.
    #[allow(clippy::too_many_arguments)]
    fn text_right_verts(
        &mut self,
        fs: &mut FontSystem,
        font: u8,
        px: f32,
        rx: f32,
        y: f32,
        text: &str,
        color: Color,
        letter_spacing: f32,
        fig: &Figures,
    ) {
        let w = fs.measure_fig(font, px, text, letter_spacing, fig);
        self.text_verts(fs, font, px, rx - w, y, text, color, letter_spacing, fig);
    }

    /// Module header: text on the left, optionally on the right, and an
    /// optional plain underline. Any part can be left out — empty text
    /// with the underline on gives just the line.
    #[allow(clippy::too_many_arguments)]
    pub fn module_title(
        &mut self,
        fs: &mut FontSystem,
        x: f32,
        y: f32,
        w: f32,
        px: f32,
        left: &str,
        right: &str,
        color: Color,
        underline: bool,
    ) {
        self.cmd(|| DrawCmd::ModuleTitle {
            at: [x, y],
            w,
            px,
            color,
            underline,
            left: left.to_string(),
            right: right.to_string(),
        });
        // The five constants that survived the first wave, tokened: this is
        // the one text path with no Ctx, so it reads the resolved theme
        // directly. em tokens bake to bare multipliers of the caller's px.
        use crate::theme::{self, TokenId};
        use std::sync::OnceLock;
        fn tokc(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
            *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
        }
        static LEAD: OnceLock<TokenId> = OnceLock::new();
        static TRACK: OnceLock<TokenId> = OnceLock::new();
        static PAD: OnceLock<TokenId> = OnceLock::new();
        static GAP: OnceLock<TokenId> = OnceLock::new();
        static RULE: OnceLock<TokenId> = OnceLock::new();
        static RULE_COL: OnceLock<TokenId> = OnceLock::new();
        let t = theme::resolved();
        let h = px * t.px(tokc(&LEAD, "component.module.leading")).max(0.0);
        let font = crate::font::FONT_UI;
        let spacing = px * t.px(tokc(&TRACK, "component.module.tracking")).max(0.0);
        let pad = px * t.px(tokc(&PAD, "component.module.pad")).max(0.0);
        let gap = px * t.px(tokc(&GAP, "component.module.gap")).max(0.0);
        // A module title is not a numeric run: it takes the proportional
        // box, as it always has.
        self.text_verts(fs, font, px, x + pad, y, left, color, spacing, &Figures::NONE);
        if !right.is_empty() {
            // The right-hand text is trimmed to whatever the left one
            // leaves. Without this the two simply overlapped in a narrow
            // panel — the CPU header wrote its model name straight
            // through its own title.
            let used = fs.measure(font, px, left, spacing) + gap;
            let room = (w - used).max(0.0);
            let shown = fit_tail(fs, font, px, right, spacing, room);
            if !shown.is_empty() {
                self.text_right_verts(
                    fs, font, px, x + w - pad, y, &shown, color, spacing, &Figures::NONE,
                );
            }
        }
        if underline {
            let rw = t.px(tokc(&RULE, "component.module.rule")).max(0.0);
            let rc = t.color(tokc(&RULE_COL, "component.module.rule_color"));
            self.line_verts(
                x,
                y + h,
                x + w,
                y + h,
                rw,
                Color { r: rc.r, g: rc.g, b: rc.b, a: rc.a },
            );
        }
    }
}

/// Shortens `text` with an ellipsis until it fits `max_w`; empty when
/// there is no room even for the ellipsis. The `ui` module has the same
/// thing built on `Ctx`; this one needs only the font system, because a
/// draw list has no context.
pub(crate) fn fit_tail(
    fs: &mut FontSystem,
    font: u8,
    px: f32,
    text: &str,
    spacing: f32,
    max_w: f32,
) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    if fs.measure(font, px, text, spacing) <= max_w {
        return text.to_string();
    }
    // `type.ellipsis`, read once the run is known not to fit — the same
    // key the other three trimmers now end on, so a console theme asking
    // for `>` gets it everywhere or nowhere.
    let cut = crate::ui::ellipsis();
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len().saturating_sub(1);
    while n > 0 {
        let cand: String = chars[..n].iter().collect::<String>() + cut.as_ref();
        if fs.measure(font, px, &cand, spacing) <= max_w {
            return cand;
        }
        n -= 1;
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write as _;

    /// Vertices a split shape rasterises through (f3 §7b): the core the
    /// ordinary fill path draws, then the four strips of the frame the
    /// field draws — six vertices each, and every one of them cheaper
    /// than the fragments it saves.
    const FRAME: usize = 30;

    /// The two shipped track corners, drawn. `slider.track_corner` says
    /// `@corner.pill` and `slider.knob_corner` says `@corner.none`, and
    /// until `Corner::sized` existed both reached the vertex list as the
    /// same square-ended bar — the theme said capsule and the program drew
    /// a slab, which is the one case where even the SHIPPED look was not
    /// what the master wrote.
    #[test]
    fn a_corner_token_decides_the_shape_and_the_two_shipped_words_differ() {
        let t = crate::theme::resolved();
        let pill = t.px(crate::theme::id("slider.track_corner").unwrap());
        let none = t.px(crate::theme::id("slider.knob_corner").unwrap());
        // A pill is a sentinel, not a length: every consumer testing
        // `radius > 0.0` has read it as "no corner at all".
        assert!(pill < 0.0, "{pill}");
        let r = Rect::new(0.0, 0.0, 200.0, 8.0);
        assert_eq!(Corner::sized(CornerStyle::Round, pill, r).size, 4.0);
        assert_eq!(Corner::sized(CornerStyle::Round, none, r).size, 0.0);

        // And the difference reaches the picture: a capsule has no vertex
        // at the box's own corner, a square-cornered fill has four.
        let fill = |radius: f32| {
            let mut dl = DrawList::new();
            dl.ring_fill(r, &[Corner::sized(CornerStyle::Round, radius, r); 4], 6, Color::WHITE);
            dl.verts.iter().filter(|v| v.pos == [r.x, r.y]).count()
        };
        assert!(fill(none) > 0);
        assert_eq!(fill(pill), 0);
    }

    /// The frame's stroke stays INSIDE the rect it frames. The rect is
    /// layout's and the width is the theme's, so a theme thickening a border
    /// must never move a panel edge — which the old centred stroke did, by
    /// half the width on every side.
    #[test]
    fn chamfer_frame_stroke_never_leaves_the_rect() {
        let (x, y, w, h) = (10.0, 20.0, 200.0, 100.0);
        for t in [1.0f32, 2.0, 4.0, 8.0] {
            let mut dl = DrawList::new();
            dl.chamfer_frame(x, y, w, h, 16.0, t, Color::rgb8(255, 255, 255));
            let e = 0.01;
            for v in &dl.verts {
                let [px, py] = v.pos;
                assert!(
                    px >= x - e && px <= x + w + e && py >= y - e && py <= y + h + e,
                    "stroke t={t} leaks: ({px},{py}) outside ({x},{y},{w},{h})"
                );
            }
        }
    }

    /// The generator's counts are the contract every cost estimate in r1
    /// stands on: Square 1 point, Chamfer 2, Round S+1; the stroke is one
    /// quad per boundary segment, the fill 3 verts per point past the
    /// fast paths.
    #[test]
    fn ring_vertex_counts_per_corner_mode() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let cases: [(&[Corner; 4], usize, usize); 4] = [
            // all square: rect_outline's price / rect's price
            (&[Corner::SQUARE; 4], 24, 6),
            // all chamfer: chamfer_frame's / chamfer_fill's price
            (&[Corner::chamfer(12.0); 4], 48, 18),
            // all round S=6: 28 points -> 168 stroke, 84 fan
            (&[Corner::round(8.0); 4], 168, 84),
            // mixed chamfer tl+br, square tr+bl: 6 points
            (
                &[
                    Corner::chamfer(12.0),
                    Corner::SQUARE,
                    Corner::chamfer(12.0),
                    Corner::SQUARE,
                ],
                36,
                18,
            ),
        ];
        for (c, stroke_verts, fill_verts) in cases {
            let mut dl = DrawList::new();
            dl.ring(r, c, 6, 2.0, Color::rgb8(255, 255, 255));
            assert_eq!(dl.verts.len(), stroke_verts, "stroke {c:?}");
            let mut dl = DrawList::new();
            dl.ring_fill(r, c, 6, Color::rgb8(255, 255, 255));
            assert_eq!(dl.verts.len(), fill_verts, "fill {c:?}");
        }
    }

    /// The adaptive segment rule at the shipped corner ladder (r1 §3.4):
    /// 3/3/4 at a 0.25 px chord tolerance, and the theme's ceiling always
    /// wins on large radii.
    #[test]
    fn ring_segments_matches_the_shipped_ladder() {
        assert_eq!(ring_segments(4.3, 0.25, 6), 3);
        assert_eq!(ring_segments(6.5, 0.25, 6), 3);
        assert_eq!(ring_segments(11.9, 0.25, 6), 4);
        assert_eq!(ring_segments(1000.0, 0.25, 6), 6);
    }

    /// Every corner mix, every width: the stroke never leaves the rect —
    /// the rect is layout's, the width the theme's — AND it stays flush,
    /// touching all four edges. Inside but shrunken would be a different
    /// bug with the same containment signature.
    #[test]
    fn ring_stroke_stays_inside_and_flush() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let cases: [[Corner; 4]; 4] = [
            [Corner::SQUARE; 4],
            [Corner::chamfer(16.0); 4],
            [Corner::round(16.0); 4],
            [
                Corner::chamfer(40.0),
                Corner::SQUARE,
                Corner::round(12.0),
                Corner::chamfer(4.0),
            ],
        ];
        for c in &cases {
            for t in [1.0f32, 2.0, 4.0, 8.0] {
                let mut dl = DrawList::new();
                dl.ring(r, c, 6, t, Color::rgb8(255, 255, 255));
                let e = 1e-3;
                let (mut lo_x, mut hi_x) = (f32::MAX, f32::MIN);
                let (mut lo_y, mut hi_y) = (f32::MAX, f32::MIN);
                for v in &dl.verts {
                    let [px, py] = v.pos;
                    assert!(
                        px >= r.x - e
                            && px <= r.x + r.w + e
                            && py >= r.y - e
                            && py <= r.y + r.h + e,
                        "stroke t={t} leaks: ({px},{py}) outside {c:?}"
                    );
                    lo_x = lo_x.min(px);
                    hi_x = hi_x.max(px);
                    lo_y = lo_y.min(py);
                    hi_y = hi_y.max(py);
                }
                assert!((lo_x - r.x).abs() <= e, "left edge not flush, t={t} {c:?}");
                assert!((hi_x - r.right()).abs() <= e, "right edge not flush, t={t} {c:?}");
                assert!((lo_y - r.y).abs() <= e, "top edge not flush, t={t} {c:?}");
                assert!((hi_y - r.bottom()).abs() <= e, "bottom edge not flush, t={t} {c:?}");
            }
        }
    }

    /// The fill through the generator honours the same boundary the stroke
    /// does — the retargeted successor of the chamfer_fill test below.
    #[test]
    fn ring_fill_stays_inside_the_rect() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [
            Corner::round(16.0),
            Corner::chamfer(24.0),
            Corner::SQUARE,
            Corner::round(8.0),
        ];
        let mut dl = DrawList::new();
        dl.ring_fill(r, &c, 6, Color::rgb8(255, 255, 255));
        assert!(!dl.verts.is_empty());
        let e = 1e-3;
        for v in &dl.verts {
            let [px, py] = v.pos;
            assert!(px >= r.x - e && px <= r.x + r.w + e && py >= r.y - e && py <= r.y + r.h + e);
        }
    }

    /// The gradient ring is the SAME ring: same tessellation, same vertex
    /// count, same flush boundary — only the colour per vertex differs.
    /// That equality is the master's own argument for declaring a gradient
    /// border on all nine rungs ("the same 24 verts a solid border
    /// costs"), so it is worth a test rather than a comment.
    #[test]
    fn a_gradient_ring_is_a_ring_that_costs_the_same() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [
            Corner::round(16.0),
            Corner::chamfer(24.0),
            Corner::SQUARE,
            Corner::round(8.0),
        ];
        let a = Color::rgb8(10, 200, 30).alpha(0.7);
        let b = Color::rgb8(250, 40, 90);
        let mut flat = DrawList::new();
        flat.ring(r, &c, 6, 3.0, a);
        let mut grad = DrawList::new();
        grad.ring_grad(r, &c, 6, 3.0, a, b, [1.0, 0.0]);
        assert_eq!(flat.verts.len(), grad.verts.len());
        for (f, g) in flat.verts.iter().zip(&grad.verts) {
            assert_eq!(f.pos, g.pos, "the gradient moved a vertex");
        }
        // The two ends land ON the box: t is normalised against the rect's
        // own projected extent, so the extremes are exact and nothing in
        // between leaves the pair.
        let e = 1e-6;
        let mut saw_near = false;
        let mut saw_far = false;
        for v in &grad.verts {
            if (v.pos[0] - r.x).abs() < 1e-3 {
                assert!((v.color[0] - a.r).abs() < e, "near end drifted: {:?}", v.color);
                saw_near = true;
            }
            if (v.pos[0] - r.right()).abs() < 1e-3 {
                assert!((v.color[0] - b.r).abs() < e, "far end drifted: {:?}", v.color);
                saw_far = true;
            }
        }
        assert!(saw_near && saw_far);
    }

    /// A direction of zero length is not a direction. The ring draws the
    /// NEAR colour flat rather than dividing by the span it does not have
    /// — the raw degradation, not a fallback design.
    #[test]
    fn a_gradient_with_no_direction_is_the_flat_ring() {
        let r = Rect::new(0.0, 0.0, 40.0, 40.0);
        let c = [Corner::SQUARE; 4];
        let a = Color::rgb8(10, 200, 30);
        let mut dl = DrawList::new();
        dl.ring_grad(r, &c, 4, 2.0, a, Color::rgb8(250, 40, 90), [0.0, 0.0]);
        assert!(!dl.verts.is_empty());
        for v in &dl.verts {
            assert_eq!(v.color, a.to_array());
        }
    }

    /// The register witnesses a gradient ring as its own command, with
    /// both ends and the direction the theme wrote — a frame that says
    /// "ring" where a gradient was asked for cannot be told from one that
    /// drew the bug this command exists to fix.
    #[test]
    fn the_register_tells_a_gradient_ring_from_a_flat_one() {
        let r = Rect::new(0.0, 0.0, 40.0, 40.0);
        let c = [Corner::SQUARE; 4];
        let mut dl = DrawList::recording();
        dl.ring(r, &c, 4, 2.0, ink());
        dl.ring_grad(r, &c, 4, 2.0, ink(), wash(), [1.0, -1.0]);
        let lines: Vec<String> = dl.cmds().iter().map(|c| c.to_string()).collect();
        assert!(lines[0].starts_with("ring at"), "{}", lines[0]);
        assert!(lines[1].starts_with("ring_grad at"), "{}", lines[1]);
        assert!(lines[1].contains(" near rgba"), "{}", lines[1]);
        assert!(lines[1].contains(" far rgba"), "{}", lines[1]);
        assert!(lines[1].ends_with(" dir 1.000000 -1.000000"), "{}", lines[1]);
    }

    /// The two-stop fast path is one quad whose extreme corners carry the
    /// stop colours BIT-FOR-BIT — the a·(1−u) + b·u lerp guarantees it —
    /// and it stays one quad at any angle, because an output-space two-stop
    /// gradient is affine and Gouraud needs no bands for affine.
    #[test]
    fn gradient_two_stop_endpoints_exact_and_free() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let a = Color::rgb8(10, 200, 30).alpha(0.7);
        let b = Color::rgb8(250, 40, 90);
        let mut dl = DrawList::new();
        dl.rect_grad(r, &[(0.0, a), (1.0, b)], 0.0);
        assert_eq!(dl.verts.len(), 6, "two stops must not band");
        for v in &dl.verts {
            if v.pos[0] == r.x {
                assert_eq!(v.color, a.to_array(), "left endpoint drifted");
            }
            if v.pos[0] == r.x + r.w {
                assert_eq!(v.color, b.to_array(), "right endpoint drifted");
            }
        }
        // Any angle: still exactly one quad.
        let mut dl = DrawList::new();
        dl.rect_grad(r, &[(0.0, a), (1.0, b)], 0.6);
        assert_eq!(dl.verts.len(), 6, "the fast path is angle-independent");
    }

    /// Eight stops = seven bands = 42 verts on an axis-aligned angle
    /// (r1 §6.2): one band per stop interval, the caps zero-width, every
    /// band a single quad.
    #[test]
    fn gradient_eight_stops_band_count() {
        let r = Rect::new(0.0, 0.0, 700.0, 100.0);
        let stops: Vec<(f32, Color)> = (0..8)
            .map(|i| (i as f32 / 7.0, Color::rgb8(i as u8 * 30, 100, 200)))
            .collect();
        let mut dl = DrawList::new();
        dl.rect_grad(r, &stops, 0.0);
        assert_eq!(dl.verts.len(), 42, "7 bands of one quad each");
    }

    /// Pushed clips intersect down the stack and stamp the runs; popping
    /// restores the enclosing clip.
    #[test]
    fn clip_intersection_stamps_runs() {
        let mut dl = DrawList::new();
        let col = Color::rgb8(255, 255, 255);
        dl.push_clip(10.0, 10.0, 100.0, 100.0);
        dl.push_clip(50.0, 50.0, 100.0, 100.0);
        dl.rect(0.0, 0.0, 500.0, 500.0, col);
        assert_eq!(
            dl.runs.last().unwrap().clip,
            Some([50.0, 50.0, 60.0, 60.0]),
            "inner clip must be the intersection"
        );
        dl.pop_clip();
        dl.rect(0.0, 0.0, 500.0, 500.0, col);
        assert_eq!(dl.runs.last().unwrap().clip, Some([10.0, 10.0, 100.0, 100.0]));
        dl.pop_clip();
        dl.rect(0.0, 0.0, 500.0, 500.0, col);
        assert_eq!(dl.runs.last().unwrap().clip, None);
    }

    /// The SPRITE glow lives OUTSIDE the rect, inside rect+radius, and
    /// in an ADD_ATLAS run — light, never milk. The vector lane's own
    /// containment is a different claim about a different picture (the
    /// quad covers the rect and the FIELD empties it), and
    /// `the_glow_lane_adds_light_and_leaves_the_interior_alone` makes
    /// it.
    #[test]
    fn glow_ring_additive_and_outside() {
        let r = Rect::new(50.0, 60.0, 200.0, 100.0);
        let radius = 8.0;
        let uv = FontSystem::mask_soft_uv();
        let mut dl = DrawList::new();
        dl.glow_ring(r, &[Corner::chamfer(16.0); 4], 6, radius, Color::rgb8(0, 255, 200), uv);
        assert!(!dl.verts.is_empty());
        assert!(
            dl.runs.iter().any(|run| run.image == Some(ADD_ATLAS)),
            "glow must be an additive run"
        );
        let e = 1e-3;
        for v in &dl.verts {
            let [px, py] = v.pos;
            let inside =
                px > r.x + e && px < r.right() - e && py > r.y + e && py < r.bottom() - e;
            assert!(!inside, "glow leaked into the rect: ({px},{py})");
            assert!(
                px >= r.x - radius - e
                    && px <= r.right() + radius + e
                    && py >= r.y - radius - e
                    && py <= r.bottom() + radius + e,
                "glow past its own radius: ({px},{py})"
            );
        }
    }

    /// The emitter's OWN transcript, from before it could be asked for a
    /// profile: one quad ring, the mask laid flat across the reach, one
    /// colour on all four corners of every quad.
    ///
    /// Transcribed statement for statement rather than shared with the
    /// emitter, which is the whole point — a no-move proof written
    /// against the code it is proving has not moved proves nothing.
    fn the_single_band_emitter(
        dl: &mut DrawList,
        r: Rect,
        c: &[Corner; 4],
        segments: u8,
        radius: f32,
        color: Color,
        mask_uv: (f32, f32, f32, f32),
    ) {
        let (u0, v0, u1, v1) = mask_uv;
        let mut inner = Vec::new();
        let mut outer = Vec::new();
        ring_points(r, c, segments, &mut inner);
        let grown =
            Rect::new(r.x - radius, r.y - radius, r.w + 2.0 * radius, r.h + 2.0 * radius);
        let ck = [
            c[0].inset(-radius),
            c[1].inset(-radius),
            c[2].inset(-radius),
            c[3].inset(-radius),
        ];
        ring_points(grown, &ck, segments, &mut outer);
        let su = u0 + (u1 - u0) * (32.0 / 64.0);
        let vi = v0 + (v1 - v0) * (31.0 / 64.0);
        let col = color.to_array();
        // The rim's own two corners carry ZERO alpha, not `color`'s —
        // `alpha_at`'s own `f >= 1.0` branch, so this transcript still
        // matches the emitter it is proving has not otherwise moved.
        let rim = [color.r, color.g, color.b, 0.0];
        let n = inner.len().min(outer.len());
        for i in 0..n {
            let j = (i + 1) % n;
            dl.push_quad4(
                Some(ADD_ATLAS),
                [inner[i], inner[j], outer[j], outer[i]],
                [[su, vi], [su, vi], [su, v0], [su, v0]],
                [col, col, rim, rim],
            );
        }
    }

    /// THE RENAME'S FOUNDATION: an emitter that can shape its light still
    /// draws the unshaped halo bit for bit.
    ///
    /// The theme editor's old NEON became GLOW on 2026-08-18 and the
    /// owner's condition on the rename was that the picture not move. It
    /// cannot move at the token level if it does not move here, because
    /// every glow in this toolkit — panel edge, focus ring — reaches the
    /// screen through this one call. Compared over the corner styles
    /// separately: the profile's arithmetic runs per band boundary, and a
    /// square corner, a chamfer and an arc take different numbers of them.
    ///
    /// [`the_single_band_emitter`]'s rim carries zero alpha now
    /// (2026-08-24), the one deliberate exception to "bit for bit": the
    /// owner's condition guards against an ACCIDENTAL move, not against
    /// this one, named one — every glow class, halo included, must reach
    /// exactly the background past its own reach, and that guarantee has
    /// to live in the vertex the emitter hands the rasteriser, not only
    /// in the mask texture's own zero texel (`alpha_at`'s own doc says
    /// why). The transcript moved on purpose, together with the code it
    /// is proving has not moved any OTHER way.
    #[test]
    fn the_shaped_emitter_still_draws_the_unshaped_halo() {
        let uv = FontSystem::mask_soft_uv();
        let col = Color { r: 0.2, g: 0.8, b: 0.9, a: 0.34 };
        for c in [
            [Corner { style: CornerStyle::Square, size: 0.0 }; 4],
            [Corner::chamfer(8.0); 4],
            [Corner::round(9.0); 4],
        ] {
            for radius in [1.0f32, 8.0, 27.0] {
                for r in [Rect::new(50.0, 60.0, 200.0, 100.0), Rect::new(0.0, 0.0, 24.0, 24.0)] {
                    let mut was = DrawList::new();
                    the_single_band_emitter(&mut was, r, &c, 6, radius, col, uv);
                    let mut now = DrawList::new();
                    now.glow_ring(r, &c, 6, radius, col, uv);
                    assert!(!was.verts.is_empty(), "the transcript drew nothing to compare");
                    let dump = |dl: &DrawList| {
                        dl.verts.iter().map(|v| (v.pos, v.uv, v.color)).collect::<Vec<_>>()
                    };
                    assert_eq!(
                        dump(&was),
                        dump(&now),
                        "the halo moved at radius {radius} on {c:?}"
                    );
                }
            }
        }
    }

    /// A profile whose knobs are all at rest is the halo, not a
    /// subdivided imitation of it — the identity [`GlowProfile::HALO`]
    /// claims, asked of a profile a THEME could write by naming `tube`
    /// and then flattening every number of it.
    #[test]
    fn a_flattened_profile_is_the_halo_itself() {
        let uv = FontSystem::mask_soft_uv();
        let col = Color { r: 0.9, g: 0.2, b: 0.7, a: 0.5 };
        let r = Rect::new(12.0, 20.0, 180.0, 90.0);
        let c = [Corner::round(9.0); 4];
        let mut halo = DrawList::new();
        halo.glow_ring(r, &c, 6, 20.0, col, uv);
        for flat in [
            GlowProfile { decay: 1.0, aura: 1.0, aura_reach: 0.25, bands: 5, cutoff: 1.0 },
            GlowProfile { decay: 1.0, aura: 2.0, aura_reach: 0.0, bands: 5, cutoff: 1.0 },
            // A band count on its own shapes nothing: there is no
            // re-map for the extra rings to follow, so they would land
            // on the line the single band already draws.
            GlowProfile { decay: 1.0, aura: 1.0, aura_reach: 0.0, bands: 16, cutoff: 1.0 },
        ] {
            let mut dl = DrawList::new();
            dl.glow_ring_with(r, &c, 6, 20.0, col, uv, flat);
            let dump = |dl: &DrawList| {
                dl.verts.iter().map(|v| (v.pos, v.uv, v.color)).collect::<Vec<_>>()
            };
            assert_eq!(dump(&halo), dump(&dl), "{flat:?} was not the halo");
        }
    }

    /// K3d's own regression (2026-08-23): raising `render.vector` must
    /// not silently flatten a tube. A HALO has nothing a shape record
    /// could lose, so it rides the vector lane's record on either side of
    /// the switch; a SHAPED profile — `decay != 1.0`, or an aura — has no
    /// ramp a record can carry yet, so `glow_ring_with` must still spend
    /// the tessellated strip even with the lane on, and its inner face
    /// must still draw rather than vanish. Read off `shape_len` and
    /// `verts`, the same instruments `vector.rs`'s own lane test in
    /// nacelle-desktop uses, so a future record format that DOES carry a
    /// ramp is exactly the change that would need to touch this test too.
    #[test]
    fn a_shaped_glow_keeps_its_shape_when_the_vector_lane_is_on() {
        let uv = FontSystem::mask_soft_uv();
        let col = Color { r: 0.9, g: 0.2, b: 0.7, a: 0.5 };
        let r = Rect::new(12.0, 20.0, 180.0, 90.0);
        let c = [Corner::round(9.0); 4];
        let tube = GlowProfile { decay: 3.0, aura: 1.6, aura_reach: 0.3, bands: 5, cutoff: 1.0 };
        assert!(!tube.is_halo(), "the fixture must be a shaped profile");

        // The tessellated answer, off the vector lane — the picture the
        // tube was written for and the one this test holds the lane to.
        let mut tess = DrawList::new();
        tess.glow_ring_with(r, &c, 6, 20.0, col, uv, tube);
        tess.glow_ring_inward_with(r, &c, 6, 20.0, col, uv, tube);
        assert_eq!(tess.shape_len(), 0, "the tessellated lane writes no shape record");
        assert!(!tess.verts.is_empty(), "the tube's strips must have drawn something");

        // The same calls with the lane armed. A record would replace the
        // strips with one flat glow and drop the inner face outright —
        // exactly the two failures this test exists to catch.
        let mut vec_on = DrawList::new();
        vec_on.set_vector(true);
        vec_on.glow_ring_with(r, &c, 6, 20.0, col, uv, tube);
        vec_on.glow_ring_inward_with(r, &c, 6, 20.0, col, uv, tube);
        assert_eq!(
            vec_on.shape_len(),
            0,
            "a shaped profile must still take the strip, not a shape record, \
             even with render.vector on"
        );
        let dump = |dl: &DrawList| {
            dl.verts.iter().map(|v| (v.pos, v.uv, v.color)).collect::<Vec<_>>()
        };
        assert_eq!(
            dump(&tess),
            dump(&vec_on),
            "the vector lane must draw the tube identically to the tessellated lane \
             until a shape record can carry a ramp"
        );

        // And the control: a HALO on the same lane DOES take the record,
        // so the gate is on the profile and not a lane that has stopped
        // taking records at all.
        let mut halo_on = DrawList::new();
        halo_on.set_vector(true);
        halo_on.glow_ring_with(r, &c, 6, 20.0, col, uv, GlowProfile::HALO);
        assert_eq!(
            halo_on.shape_len(),
            1,
            "an unshaped halo must still ride the vector lane's record"
        );
    }

    /// THE TUBE'S LIGHT STOPS WHERE THE HALO'S IS STILL FADING.
    ///
    /// Read off the emitted geometry rather than off a formula, and as a
    /// RELATION rather than against a number: for every distance the tube
    /// puts a band boundary at, the mask is sampled CLOSER TO ITS ZERO
    /// than the halo samples it at that same distance. The mask's own
    /// profile falls monotonically from `vi` to `v0`, so a smaller v at
    /// the same place is less light at that place, whatever the disk is
    /// shaped like — which is what makes this a claim about the tube and
    /// not about the sprite it borrows.
    ///
    /// Both are drawn at the same radius on purpose: a glow that merely
    /// reached less far would be a smaller halo, and the owner asked for
    /// light that falls off abruptly, not for a shorter one.
    #[test]
    fn the_tube_spends_its_light_sooner_than_the_halo() {
        let uv = FontSystem::mask_soft_uv();
        let (u0, v0, _u1, v1) = uv;
        let vi = v0 + (v1 - v0) * (31.0 / 64.0);
        let col = Color { r: 0.6, g: 0.2, b: 0.95, a: 0.4 };
        let r = Rect::new(40.0, 40.0, 200.0, 120.0);
        let c = [Corner { style: CornerStyle::Square, size: 0.0 }; 4];
        let radius = 24.0;
        let tube = GlowProfile { decay: 3.0, aura: 1.0, aura_reach: 0.0, bands: 5, cutoff: 1.0 };
        let mut dl = DrawList::new();
        dl.glow_ring_with(r, &c, 6, radius, col, uv, tube);
        // The distance a vertex stands at, off the geometry: the glow is
        // emitted strictly outside `r`, so the gap to the nearest edge IS
        // the distance along the extrusion.
        let dist = |p: [f32; 2]| {
            let dx = (r.x - p[0]).max(p[0] - r.right()).max(0.0);
            let dy = (r.y - p[1]).max(p[1] - r.bottom()).max(0.0);
            dx.max(dy)
        };
        let mut seen_interior = false;
        for v in &dl.verts {
            let d = dist(v.pos);
            let f = (d / radius).clamp(0.0, 1.0);
            // The halo's own sample at this distance: the flat lay.
            let halo_v = vi + (v0 - vi) * f;
            let got = v.uv[1];
            if f > 0.001 && f < 0.999 {
                seen_interior = true;
                assert!(
                    got < halo_v - 1e-6,
                    "at {:.0}% of the reach the tube sampled {got} where the halo \
                     samples {halo_v}; the tube is not falling faster",
                    f * 100.0
                );
            }
            assert!(
                got >= v0 - 1e-6 && got <= vi + 1e-6,
                "the tube sampled {got}, outside the disk's own band {v0}..{vi}"
            );
        }
        assert!(
            seen_interior,
            "every band landed on an end of the reach, so nothing was compared"
        );
        // The ends are still the ends: light at the glass, none at the rim.
        let uv1: Vec<f32> = dl.verts.iter().map(|v| v.uv[1]).collect();
        assert!(uv1.iter().any(|&v| (v - vi).abs() < 1e-6), "no band starts at the glass");
        assert!(uv1.iter().any(|&v| (v - v0).abs() < 1e-6), "no band ends at the rim");
        let _ = u0;
    }

    /// A tube throws light INTO the frame too: the inner face lands
    /// strictly on the body, peak on the border and gone within the reach
    /// — the mirror of the outer bloom the test above measures on the far
    /// side. And it costs the register nothing: the outer call already
    /// recorded the tube, and a second record would double-count the border
    /// in the frame hash.
    #[test]
    fn the_tube_lights_the_frame_interior() {
        let uv = FontSystem::mask_soft_uv();
        let (_, v0, _, v1) = uv;
        let vi = v0 + (v1 - v0) * (31.0 / 64.0);
        let col = Color { r: 0.6, g: 0.2, b: 0.95, a: 0.4 };
        let r = Rect::new(40.0, 40.0, 200.0, 120.0);
        let c = [Corner { style: CornerStyle::Square, size: 0.0 }; 4];
        let radius = 24.0;
        let tube = GlowProfile { decay: 3.0, aura: 1.0, aura_reach: 0.0, bands: 5, cutoff: 1.0 };

        // No register line: the outer call stands for the tube; this is
        // that one intent's second face, not a second glow to hash.
        let mut rec = DrawList::recording();
        rec.glow_ring_inward_with(r, &c, 6, radius, col, uv, tube);
        assert!(
            !rec.cmds().iter().any(|cmd| matches!(cmd, DrawCmd::GlowRing { .. })),
            "the inner face recorded a second glow_ring"
        );

        let mut dl = DrawList::new();
        dl.glow_ring_inward_with(r, &c, 6, radius, col, uv, tube);
        assert!(!dl.verts.is_empty(), "the inner face drew nothing");
        assert!(
            dl.runs.iter().any(|run| run.image == Some(ADD_ATLAS)),
            "the inner face must be additive, like the outer bloom"
        );
        let e = 1e-3;
        let (mut saw_peak, mut saw_rim) = (false, false);
        for v in &dl.verts {
            let [px, py] = v.pos;
            // Strictly inside the frame — never past its outer edge, which
            // is the outer bloom's ground.
            assert!(
                px >= r.x - e && px <= r.right() + e && py >= r.y - e && py <= r.bottom() + e,
                "the inner face leaked outside the frame: ({px},{py})"
            );
            // And no deeper than the reach: the light is gone before the
            // panel's middle.
            let depth = (px - r.x).min(r.right() - px).min(py - r.y).min(r.bottom() - py);
            assert!(depth <= radius + e, "the inner face reached past its radius: depth {depth}");
            saw_peak |= (v.uv[1] - vi).abs() < 1e-6;
            saw_rim |= (v.uv[1] - v0).abs() < 1e-6;
        }
        assert!(saw_peak, "no band sits at the glass — the peak is not on the border");
        assert!(saw_rim, "no band fades to nothing — the light never lets go");

        // The two faces stand on opposite sides of the path: the outer
        // bloom is entirely outside the same rect the inner face is
        // entirely inside.
        let mut out = DrawList::new();
        out.glow_ring_with(r, &c, 6, radius, col, uv, tube);
        assert!(
            out.verts.iter().any(|v| {
                v.pos[0] < r.x - e
                    || v.pos[0] > r.right() + e
                    || v.pos[1] < r.y - e
                    || v.pos[1] > r.bottom() + e
            }),
            "the outer bloom is not outside the frame — the faces do not oppose"
        );
    }

    /// The alpha a PIXEL is drawn with at distance fraction `f` of the
    /// reach, reconstructed from an emitted glow the way the rasteriser
    /// reconstructs it: the rings carry alpha at their own distances and
    /// everything between two rings is the straight line joining them.
    ///
    /// Reading the vertices alone is not the same claim and the
    /// difference is not academic — it is the shape of the bug the first
    /// version of `the_aura_lifts_...` failed to see. A ring at 0.2 and a
    /// ring at 0.4 hide a lit strip between them: every vertex "past the
    /// reach" of 0.25 that the test could see was the one at 0.4, where
    /// the ramp is over by construction, so the assertion passed on a
    /// picture that went on lifting pixels 60 % beyond the theme's own
    /// number.
    ///
    /// Distance comes off the geometry: the glow is emitted strictly
    /// outside `r`, so the gap to the nearest edge IS the distance along
    /// the extrusion.
    fn alpha_ramp(dl: &DrawList, r: Rect, radius: f32) -> impl Fn(f32) -> f32 {
        let mut rings: Vec<(f32, f32)> = Vec::new();
        for v in &dl.verts {
            let dx = (r.x - v.pos[0]).max(v.pos[0] - r.right()).max(0.0);
            let dy = (r.y - v.pos[1]).max(v.pos[1] - r.bottom()).max(0.0);
            let f = (dx.max(dy) / radius).clamp(0.0, 1.0);
            match rings.iter().find(|(g, _)| (g - f).abs() < 1e-4) {
                Some((_, a)) => assert!(
                    (a - v.color[3]).abs() < 1e-6,
                    "two alphas at the same distance {f}: {a} and {}",
                    v.color[3]
                ),
                None => rings.push((f, v.color[3])),
            }
        }
        rings.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
        assert!(rings.len() >= 2, "one ring cannot describe a ramp");
        move |f: f32| {
            let i = rings.partition_point(|(g, _)| *g <= f);
            if i == 0 {
                return rings[0].1;
            }
            if i == rings.len() {
                return rings[rings.len() - 1].1;
            }
            let ((f0, a0), (f1, a1)) = (rings[i - 1], rings[i]);
            a0 + (a1 - a0) * ((f - f0) / (f1 - f0))
        }
    }

    /// THE AURA IS BRIGHTER THAN THE LIGHT IT SITS IN, IT LETS GO WHERE
    /// THE THEME SAYS, AND IT REACHES EXACTLY 0 AT THE RIM — measured on
    /// the picture, not on the rings.
    ///
    /// A relation, as everywhere here: the alpha at the glass is above
    /// the alpha the caller asked for, the alpha anywhere past the reach
    /// and short of the final band is exactly it, the final band ramps
    /// that down to exactly 0 at the rim (`alpha_at`'s own `f >= 1.0`
    /// branch — the vertex is zero there BY CONSTRUCTION, not by the
    /// texture sample landing on the mask's own zero texel; see that
    /// function's doc for why the vertex has to carry this and not only
    /// the texture), and nothing anywhere leaves 0..1 — a blend factor
    /// outside that range is the undefined output the master warns about
    /// at `glow.panel_edge.color`.
    ///
    /// What makes it a claim about the REACH and not about the rings is
    /// [`alpha_ramp`]: it walks fractions the emitter never put a vertex
    /// at and asks what a fragment there would be blended with. Run at
    /// three band counts and three reaches, INCLUDING a reach finer than
    /// one band, because a reach under `1/bands` is the case an even cut
    /// cannot express at all; and at two amounts, the second of which the
    /// aura drives past 1 so the clip is exercised rather than assumed.
    #[test]
    fn the_aura_lifts_the_light_at_the_glass_and_lets_go_at_its_reach() {
        let uv = FontSystem::mask_soft_uv();
        let r = Rect::new(40.0, 40.0, 200.0, 120.0);
        let c = [Corner { style: CornerStyle::Square, size: 0.0 }; 4];
        let radius = 24.0;
        // 0.4 doubled is still a legal blend factor; 0.8 doubled is not,
        // and what the theme gets for asking is the clip, never 1.6.
        for amount in [0.4f32, 0.8] {
            for bands in [3u32, 5, 8] {
                for reach in [0.25f32, 0.5, 0.05] {
                    let col = Color { r: 0.6, g: 0.2, b: 0.95, a: amount };
                    let p = GlowProfile { decay: 3.0, aura: 2.0, aura_reach: reach, bands, cutoff: 1.0 };
                    let mut dl = DrawList::new();
                    dl.glow_ring_with(r, &c, 6, radius, col, uv, p);
                    for v in &dl.verts {
                        let a = v.color[3];
                        assert!((0.0..=1.0).contains(&a), "alpha {a} left 0..1 at {p:?}");
                    }
                    let at = alpha_ramp(&dl, r, radius);
                    assert!(
                        at(0.0) > col.a,
                        "the aura at the glass is {}, no stronger than the {} asked for, \
                         at {p:?}",
                        at(0.0),
                        col.a
                    );
                    // The final band's own start: none of `reach`'s three
                    // values here ever falls past it, so `stops()` never
                    // inserts an extra ring inside this last cut and the
                    // even grid's own last boundary is exactly it.
                    let last_band_start = (bands - 1) as f32 / bands as f32;
                    assert!(reach < last_band_start - 1e-3, "test assumption: {p:?}");
                    // 401 samples across the whole reach, so the strip
                    // between any two rings is walked whatever the cut.
                    let mut last = at(0.0);
                    for i in 0..=400 {
                        let f = i as f32 / 400.0;
                        let a = at(f);
                        assert!((0.0..=1.0).contains(&a), "alpha {a} left 0..1 at f={f}, {p:?}");
                        // "Ramps from tube_aura at the glass to 1.0 here
                        // and stays there, then to 0 at the rim" — a ramp
                        // that ever turns back up is a second, brighter
                        // band nobody named.
                        assert!(
                            a <= last + 1e-6,
                            "the aura brightened again at {:.1}% of the radius: {a} after \
                             {last}, at {p:?}",
                            f * 100.0
                        );
                        last = a;
                        if f > reach + 1e-3 && f < last_band_start - 1e-3 {
                            assert!(
                                (a - col.a).abs() < 1e-6,
                                "the aura was still lifting at {:.1}% of the radius, {:.1}% \
                                 past its own reach: {a} vs {}, at {p:?}",
                                f * 100.0,
                                (f - reach) * 100.0,
                                col.a
                            );
                        } else if f < reach - 1e-3 {
                            assert!(
                                a > col.a + 1e-6,
                                "the aura had already let go at {:.1}% of the radius, inside \
                                 its own reach of {:.0}%: {a} vs {}, at {p:?}",
                                f * 100.0,
                                reach * 100.0,
                                col.a
                            );
                        } else if f > last_band_start + 1e-3 {
                            // The final band's own straight line from
                            // `col.a` at its start to exactly 0 at the
                            // rim — `alpha_ramp` linearly interpolates
                            // between the two real vertices that bound
                            // it, so the expected value is that same
                            // line, not a second measurement of it.
                            let want =
                                col.a * (1.0 - (f - last_band_start) / (1.0 - last_band_start));
                            assert!(
                                (a - want).abs() < 1e-5,
                                "the final band did not ramp straight to 0 at the rim: {a} vs \
                                 {want} at {:.1}% of the radius, {p:?}",
                                f * 100.0
                            );
                        }
                    }
                    assert!(
                        at(1.0).abs() < 1e-6,
                        "the rim must be exactly 0, not merely small: {} at {p:?}",
                        at(1.0)
                    );
                }
            }
        }
    }

    /// HOW FINELY THE LIGHT IS CUT IS THE THEME'S, NOT THIS FILE'S.
    ///
    /// It read like tessellation and it is not: the re-map is applied at
    /// the ring boundaries and the picture is a straight line between
    /// them, so the count IS the curve. Two claims, both about a number
    /// that used to be `(radius * 0.5).ceil().clamp(3.0, 5.0)` in Rust:
    ///
    /// * the rings land exactly where the theme's cut says, `k/n` of the
    ///   reach, with the aura's own boundary added and no others —
    ///   nothing here rounds the count to a grid of its own;
    /// * ONE radius, two counts, two different pictures. That is what
    ///   makes it a design number, and it is also what the old rule got
    ///   wrong from the other side: it grew the count with the radius, so
    ///   one theme drew a steeper tube around a button than around a
    ///   panel with nothing in the file saying so.
    #[test]
    fn the_theme_owns_how_finely_the_light_is_cut() {
        let uv = FontSystem::mask_soft_uv();
        let col = Color { r: 0.6, g: 0.2, b: 0.95, a: 0.4 };
        let r = Rect::new(40.0, 40.0, 200.0, 120.0);
        let c = [Corner { style: CornerStyle::Square, size: 0.0 }; 4];
        let radius = 24.0;
        // The distinct distances a profile lays vertices at, AND how many
        // vertices it spent: a ring emitted twice at the same distance is
        // a zero-width band that draws nothing and costs a quad, and the
        // distances alone cannot tell it from a ring emitted once.
        let rings = |p: GlowProfile| -> (Vec<f32>, usize) {
            let mut dl = DrawList::new();
            dl.glow_ring_with(r, &c, 6, radius, col, uv, p);
            let mut out: Vec<f32> = Vec::new();
            for v in &dl.verts {
                let dx = (r.x - v.pos[0]).max(v.pos[0] - r.right()).max(0.0);
                let dy = (r.y - v.pos[1]).max(v.pos[1] - r.bottom()).max(0.0);
                let f = (dx.max(dy) / radius).clamp(0.0, 1.0);
                if !out.iter().any(|g| (g - f).abs() < 1e-4) {
                    out.push(f);
                }
            }
            out.sort_by(|a, b| a.partial_cmp(b).unwrap());
            (out, dl.verts.len())
        };
        // Four points per band on a square corner, twice over: a quad is
        // six vertices for four corners.
        let per_band = 24;
        for bands in [1u32, 2, 3, 5, 8, 16] {
            // No aura, so the cut is the theme's and nothing else's.
            let (got, verts) =
                rings(GlowProfile { decay: 3.0, aura: 1.0, aura_reach: 0.0, bands, cutoff: 1.0 });
            let mut want: Vec<f32> = vec![0.0];
            want.extend((1..=bands).map(|k| k as f32 / bands as f32));
            assert_eq!(got.len(), want.len(), "asked for {bands} bands, got rings at {got:?}");
            for (g, w) in got.iter().zip(&want) {
                assert!((g - w).abs() < 1e-4, "at {bands} bands the rings are {got:?}, not {want:?}");
            }
            assert_eq!(
                verts,
                bands as usize * per_band,
                "{bands} bands cost {verts} vertices, not {}",
                bands as usize * per_band
            );
        }
        // The aura's own boundary is the ONE extra, and only when it
        // falls between two of the cut's.
        let (got, verts) = rings(GlowProfile { decay: 3.0, aura: 2.0, aura_reach: 0.25, bands: 5, cutoff: 1.0 });
        assert_eq!(got.len(), 7, "the aura's boundary did not join the cut once: {got:?}");
        assert!(got.iter().any(|f| (f - 0.25).abs() < 1e-4), "no ring at the reach: {got:?}");
        assert_eq!(verts, 6 * per_band, "the reach's own boundary cost more than one band");
        let (got, verts) = rings(GlowProfile { decay: 3.0, aura: 2.0, aura_reach: 0.2, bands: 5, cutoff: 1.0 });
        assert_eq!(got.len(), 6, "a reach the cut already lands on was cut twice: {got:?}");
        assert_eq!(
            verts,
            5 * per_band,
            "a reach the cut already lands on was emitted twice — {verts} vertices for five \
             bands, so one of them is a ring of zero width"
        );
        // Same radius, same everything else: a different count is a
        // different picture, which is what a design number means.
        let coarse = rings(GlowProfile { decay: 3.0, aura: 1.0, aura_reach: 0.0, bands: 3, cutoff: 1.0 });
        let fine = rings(GlowProfile { decay: 3.0, aura: 1.0, aura_reach: 0.0, bands: 8, cutoff: 1.0 });
        assert_ne!(coarse, fine, "the count reached nothing at radius {radius}");
        // The emitter's own guard, not the reader's: this is `pub`, so a
        // count nobody clamped on the way in still has to land inside the
        // stop buffer rather than off the end of it.
        let (got, _) =
            rings(GlowProfile { decay: 3.0, aura: 2.0, aura_reach: 0.25, bands: u32::MAX, cutoff: 1.0 });
        assert!(
            got.len() <= GlowProfile::MAX_BANDS as usize + 2,
            "an unclamped count emitted {} rings",
            got.len()
        );
    }

    /// A TUBE WITH NO MASK BAND COMES BACK AN UNSHAPED GLOW, and that is
    /// written down rather than discovered.
    ///
    /// `glow_ring_with` re-maps the soft disk's own profile, so with no
    /// disk to sample it falls to `glow_shell` and the profile is
    /// dropped whole — no aura, no decay. Unreachable from
    /// `panel_edge_glow` today (`FontSystem::mask_soft_uv()` is a
    /// compile-time rectangle), but the call is `pub` and the recipe on
    /// `TubeKeys` sends the next consumer here, so the degradation is
    /// pinned: the shaped call and the unshaped one must agree vertex for
    /// vertex, which is what "the profile is dropped" MEANS.
    #[test]
    fn a_maskless_tube_falls_back_to_the_unshaped_shell() {
        let col = Color { r: 0.6, g: 0.2, b: 0.95, a: 0.4 };
        let r = Rect::new(40.0, 40.0, 200.0, 120.0);
        let c = [Corner::round(9.0); 4];
        let none = (0.0, 0.0, 0.0, 0.0);
        let tube = GlowProfile { decay: 3.0, aura: 2.0, aura_reach: 0.25, bands: 5, cutoff: 1.0 };
        let mut shaped = DrawList::new();
        shaped.glow_ring_with(r, &c, 6, 24.0, col, none, tube);
        let mut plain = DrawList::new();
        plain.glow_ring(r, &c, 6, 24.0, col, none);
        assert!(!shaped.verts.is_empty(), "the fallback drew nothing to compare");
        let dump = |dl: &DrawList| {
            dl.verts.iter().map(|v| (v.pos, v.uv, v.color)).collect::<Vec<_>>()
        };
        assert_eq!(
            dump(&shaped),
            dump(&plain),
            "the maskless fallback shaped its light, so the note on glow_ring_with \
             that says it does not is now the wrong warning"
        );
    }

    /// The sprite costs r1 §4.4 stands on: the glow is 8 quads = 48 verts
    /// with the centre dropped, soft_box keeps the centre at 9 quads = 54 —
    /// at ANY radius and panel size, which is the whole point over shells.
    #[test]
    fn nine_slice_vertex_counts() {
        let uv = FontSystem::mask_soft_uv();
        let col = Color::rgb8(0, 255, 200);
        for radius in [2.0f32, 8.0, 40.0] {
            for r in [Rect::new(50.0, 60.0, 200.0, 100.0), Rect::new(0.0, 0.0, 24.0, 24.0)] {
                // One quad per outline segment, at ANY radius and size:
                // 4 points square, 8 chamfered, 4·(S+1) at round S=6.
                let mut dl = DrawList::new();
                dl.glow_ring(r, &[Corner { style: CornerStyle::Square, size: 0.0 }; 4], 6, radius, col, uv);
                assert_eq!(dl.verts.len(), 24, "square glow r={radius} rect={:?}", (r.w, r.h));
                let mut dl = DrawList::new();
                dl.glow_ring(r, &[Corner::chamfer(8.0); 4], 6, radius, col, uv);
                assert_eq!(dl.verts.len(), 48, "chamfer glow r={radius} rect={:?}", (r.w, r.h));
                let mut dl = DrawList::new();
                dl.glow_ring(r, &[Corner::round(8.0); 4], 6, radius, col, uv);
                assert_eq!(dl.verts.len(), 168, "round glow r={radius} rect={:?}", (r.w, r.h));
                let mut dl = DrawList::new();
                dl.soft_box(r, radius, col, uv);
                assert_eq!(dl.verts.len(), 54, "soft_box r={radius} rect={:?}", (r.w, r.h));
            }
        }
    }

    /// The plugin-facing mask quad: sprite-space uv is clamped into the
    /// band, so whatever numbers cross the ABI the quad can sample the
    /// soft disk and nothing else; `additive` picks the ADD_ATLAS run
    /// over the normal one; an empty band degrades to the white pixel —
    /// solid, raw, still present.
    #[test]
    fn mask_quad_stays_inside_the_band() {
        let band = FontSystem::mask_soft_uv();
        let (u0, v0, u1, v1) = band;
        let p = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let wild = [[-3.0, 0.5], [7.0, -2.0], [0.5, 9.0], [0.25, 0.75]];
        let col = Color::rgb8(0, 255, 200);
        let e = 1e-6;

        let mut dl = DrawList::new();
        dl.mask_quad(p, wild, band, col, true);
        assert_eq!(dl.verts.len(), 6, "one quad, additive");
        assert!(dl.runs.iter().any(|r| r.image == Some(ADD_ATLAS)));
        for v in &dl.verts {
            assert!(v.uv[0] >= u0 - e && v.uv[0] <= u1 + e, "u escaped the band: {}", v.uv[0]);
            assert!(v.uv[1] >= v0 - e && v.uv[1] <= v1 + e, "v escaped the band: {}", v.uv[1]);
        }

        // Cover blend: the same geometry lands in the plain atlas run.
        let mut dl = DrawList::new();
        dl.mask_quad(p, wild, band, col, false);
        assert_eq!(dl.verts.len(), 6);
        assert!(dl.runs.iter().all(|r| r.image.is_none()));

        // The maskless degenerate case: every vertex on the white pixel.
        let mut dl = DrawList::new();
        dl.mask_quad(p, wild, (0.0, 0.0, 0.0, 0.0), col, true);
        let w = FontSystem::white_uv();
        assert!(dl.verts.iter().all(|v| v.uv == [w.0, w.1]));

        // A fully transparent colour draws nothing at all.
        let mut dl = DrawList::new();
        dl.mask_quad(p, wild, band, col.alpha(0.0), true);
        assert!(dl.verts.is_empty());
    }

    /// A registered icon draws through [`DrawList::icon_quad`] at the uv
    /// rect [`FontSystem::icon`] answers — and the register keeps the id,
    /// not the atlas floats, so a caller reading it back learns WHICH
    /// icon was drawn rather than where the shelf packer happened to
    /// put it this run.
    #[test]
    fn icon_quad_samples_the_icons_own_uv_rect() {
        const CIRCLE_SVG: &[u8] = br#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
  <circle cx="12" cy="12" r="10"/>
</svg>"#;
        let mut fonts = FontSystem::new();
        fonts.register_icon(7, CIRCLE_SVG).unwrap();
        let g = fonts.icon(7, 16).expect("registered id at a nonzero size");

        let mut dl = DrawList::new();
        let p = [[0.0, 0.0], [40.0, 0.0], [40.0, 40.0], [0.0, 40.0]];
        let col = Color::rgb8(255, 255, 255);
        dl.icon_quad(&mut fonts, 7, 16, p, col);

        assert_eq!(dl.verts.len(), 6, "one quad, six vertices");
        assert!(dl.runs.iter().all(|r| r.image.is_none()), "an icon samples the plain atlas run, not ADD_ATLAS");
        let uvs: Vec<[f32; 2]> = dl.verts.iter().map(|v| v.uv).collect();
        for want in [[g.u0, g.v0], [g.u1, g.v0], [g.u1, g.v1], [g.u0, g.v1]] {
            assert!(uvs.contains(&want), "missing corner {want:?} in {uvs:?}");
        }
        assert!(dl.verts.iter().all(|v| v.color == col.to_array()));
    }

    #[test]
    fn icon_quad_records_the_id_and_draws_nothing_for_an_unregistered_one() {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::recording();
        let p = [[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        dl.icon_quad(&mut fonts, 404, 16, p, Color::rgb8(1, 2, 3));
        assert!(dl.verts.is_empty(), "an id nobody registered draws nothing");
        assert!(
            matches!(dl.cmds.as_ref().unwrap().last(), Some(DrawCmd::IconQuad { icon: 404, .. })),
            "the attempt is still recorded, so a register reader sees what was ASKED for"
        );
    }

    /// The glow follows the corner it wraps: around a Round corner every
    /// sprite vertex keeps the corner's own radius from the arc centre
    /// (the glow is an arc grown by the glow, not a square bloom), and
    /// some of them live INSIDE the bounding rect — in the notch beside
    /// the arc, which is outside the panel's rounded fill.
    #[test]
    fn glow_ring_follows_a_round_corner() {
        let r = Rect::new(100.0, 100.0, 200.0, 160.0);
        let (s, radius) = (24.0f32, 10.0);
        let mut dl = DrawList::new();
        dl.glow_ring(r, &[Corner::round(s); 4], 8, radius, Color::rgb8(0, 255, 200), FontSystem::mask_soft_uv());
        let centre = [r.x + s, r.y + s];
        let e = 1e-3;
        let mut in_notch = 0;
        for v in &dl.verts {
            let [px, py] = v.pos;
            if px < centre[0] && py < centre[1] {
                let d = ((px - centre[0]).powi(2) + (py - centre[1]).powi(2)).sqrt();
                assert!(d >= s - e, "glow entered the rounded fill: ({px},{py}) d={d}");
                if px > r.x + e && py > r.y + e {
                    in_notch += 1;
                }
            }
        }
        assert!(in_notch > 0, "no vertex hugs the arc — the glow is not corner-true");
    }

    /// Every nine-slice vertex samples INSIDE the mask sprite's uv rect —
    /// never a glyph, never the white pixel — and its interior slice
    /// edges sit on the sprite's 31/64 · 33/64 middle band, so the edges
    /// stretch only the 2-texel cardinal strips.
    #[test]
    fn nine_slice_samples_only_the_mask() {
        let uv = FontSystem::mask_soft_uv();
        let (u0, v0, u1, v1) = uv;
        let mut dl = DrawList::new();
        dl.glow_ring(
            Rect::new(50.0, 60.0, 200.0, 100.0),
            &[Corner::SQUARE; 4],
            3,
            12.0,
            Color::rgb8(255, 0, 255),
            uv,
        );
        dl.soft_box(Rect::new(10.0, 10.0, 80.0, 40.0), 6.0, Color::rgb8(0, 0, 0), uv);
        assert!(!dl.verts.is_empty());
        let e = 1e-6;
        for v in &dl.verts {
            let [u, w] = v.uv;
            assert!(
                u >= u0 - e && u <= u1 + e && w >= v0 - e && w <= v1 + e,
                "uv ({u},{w}) escapes the mask sprite"
            );
        }
    }

    /// soft_box stays inside its rect (the feather is inward); shadow is
    /// soft_box on the rect shifted by the offset and inflated by the
    /// radius, and stays inside THAT envelope. Both run normal-blend:
    /// no ADD_ATLAS run may appear — a shadow is not light.
    #[test]
    fn soft_box_and_shadow_containment() {
        let uv = FontSystem::mask_soft_uv();
        let r = Rect::new(30.0, 40.0, 120.0, 60.0);
        let e = 1e-3;
        let mut dl = DrawList::new();
        dl.soft_box(r, 10.0, Color::rgb8(0, 0, 0), uv);
        assert!(!dl.verts.is_empty());
        for v in &dl.verts {
            let [px, py] = v.pos;
            assert!(
                px >= r.x - e && px <= r.right() + e && py >= r.y - e && py <= r.bottom() + e,
                "soft_box leaks: ({px},{py})"
            );
        }
        let (dx, dy, radius) = (4.0, 6.0, 10.0);
        let mut dl = DrawList::new();
        dl.shadow(r, &[Corner::SQUARE; 4], [dx, dy], radius, Color::rgb8(0, 0, 0), uv);
        assert!(!dl.verts.is_empty());
        assert!(
            dl.runs.iter().all(|run| run.image != Some(ADD_ATLAS)),
            "a shadow must not be additive"
        );
        let (x0, y0) = (r.x + dx - radius, r.y + dy - radius);
        let (x1, y1) = (r.right() + dx + radius, r.bottom() + dy + radius);
        for v in &dl.verts {
            let [px, py] = v.pos;
            assert!(
                px >= x0 - e && px <= x1 + e && py >= y0 - e && py <= y1 + e,
                "shadow past its envelope: ({px},{py})"
            );
        }
    }

    /// The raw degenerate cases the governing principle demands: an empty
    /// mask uv must still draw — never nothing, never a sample of
    /// unrelated atlas texels. `soft_box` falls back to a plain rect;
    /// `glow_ring` falls back to the FIELD, which reads no atlas at all
    /// and is what retired the concentric-shell approximation.
    #[test]
    fn empty_mask_uv_degrades_raw() {
        let empty = (0.0, 0.0, 0.0, 0.0);
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        let mut dl = DrawList::new();
        dl.glow_ring(r, &[Corner::SQUARE; 4], 3, 8.0, Color::rgb8(255, 255, 255), empty);
        assert_eq!(dl.verts.len(), 6, "maskless glow is one analytic quad");
        assert_eq!(dl.shape_len(), 1, "maskless glow is one record");
        assert!(dl.runs.iter().any(|run| run.image == Some(SHAPE_ADD)));
        let mut dl = DrawList::new();
        dl.soft_box(r, 8.0, Color::rgb8(255, 255, 255), empty);
        assert_eq!(dl.verts.len(), 6, "maskless soft_box is a plain rect");
    }

    // ------------------------------------------------------ soft profiles
    //
    // f3 §2.6 and §4.6/§4.7 of the scope decision: the glow and the
    // shadow as SHAPE RECORDS. The tests below are grouped by the four
    // claims that step makes, in the order the plan binds them.

    /// **Claim 1, and nothing may go before it: the quad and the band
    /// must know how far the effect reaches** (§3.1).
    ///
    /// The envelope was `half + AA_PAD` on every side, which is exactly
    /// right for a coverage ramp that dies within a pixel of the
    /// boundary and exactly wrong for a profile that lives `feather` px
    /// past it: a glow drawn through the old quad would be sliced off
    /// on four straight lines and would read as a FRAME. The band is
    /// the same claim from the inside — it is the depth at which the
    /// field's answer is certainly the plain fill, and under a soft
    /// profile that depth moves in by the reach.
    ///
    /// Both numbers are asserted against the crisp record of the same
    /// silhouette, so the assertion is the DIFFERENCE and not a
    /// restatement of the formula.
    #[test]
    fn a_soft_record_grows_its_quad_and_its_band_by_the_reach() {
        let r = Rect::new(40.0, 30.0, 300.0, 180.0);
        let c = [Corner::round(12.0); 4];
        let reach = 9.0f32;
        let spec = |soft| ShapeSpec {
            rect: r,
            corners: c,
            kind: ShapeKind::Box,
            fill: Some(Color::rgb8(0, 255, 200)),
            stroke: None,
            glass: None,
            soft,
        };
        let span = |dl: &DrawList| {
            dl.verts.iter().fold(
                [f32::MAX, f32::MAX, f32::MIN, f32::MIN],
                |[x0, y0, x1, y1], v| {
                    [x0.min(v.pos[0]), y0.min(v.pos[1]), x1.max(v.pos[0]), y1.max(v.pos[1])]
                },
            )
        };

        // THE QUAD. A glow refuses the core split, so its geometry is
        // one quad and the span IS the envelope.
        let mut soft = DrawList::new();
        soft.shape(&spec(Some(Soft { reach, kind: SoftKind::Glow })));
        let [sx0, sy0, sx1, sy1] = span(&soft);
        let e = 1e-4;
        for (got, want, what) in [
            (sx0, r.x - AA_PAD - reach, "left"),
            (sy0, r.y - AA_PAD - reach, "top"),
            (sx1, r.right() + AA_PAD + reach, "right"),
            (sy1, r.bottom() + AA_PAD + reach, "bottom"),
        ] {
            assert!(
                (got - want).abs() < e,
                "the glow's quad stops at {got} on the {what}, and the profile \
                 reaches {want} — the effect would be cut off on its own aura"
            );
        }

        // THE BAND. A shadow keeps the split (its interior is the
        // profile's plateau, which the fill path draws exactly), so the
        // frame's hole is where the band ends — and it must sit `reach`
        // px deeper than the crisp record's.
        let mut crisp = DrawList::new();
        crisp.shape(&spec(None));
        let mut shade = DrawList::new();
        shade.shape(&spec(Some(Soft { reach, kind: SoftKind::Shadow })));
        // The interior quad of a split shape is the one quad carrying
        // NO_SHAPE: the plain-fill core.
        let core_span = |dl: &DrawList| {
            let core: Vec<_> = dl.verts.iter().filter(|v| v.shape == NO_SHAPE).collect();
            assert!(!core.is_empty(), "the split did not happen — nothing to measure");
            core.iter().fold([f32::MAX, f32::MIN], |[a, b], v| {
                [a.min(v.pos[0]), b.max(v.pos[0])]
            })
        };
        let [cx0, cx1] = core_span(&crisp);
        let [gx0, gx1] = core_span(&shade);
        assert!(
            (gx0 - cx0 - reach).abs() < e && (cx1 - gx1 - reach).abs() < e,
            "the band deepened by {} px on the left and {} px on the right, \
             and the profile reaches {reach}",
            gx0 - cx0,
            cx1 - gx1
        );
    }

    /// **Claim 2: which lane, and therefore which blend.** A glow adds
    /// light ([`SHAPE_ADD`]), a shadow covers ([`SHAPE`]) — the one
    /// difference between them that no bit in the record could have
    /// carried, because blend state is fixed for a whole pipeline
    /// before the first fragment is shaded.
    #[test]
    fn the_glow_adds_light_and_the_shadow_covers() {
        let r = Rect::new(10.0, 10.0, 120.0, 60.0);
        let c = [Corner::round(8.0); 4];
        let col = Color::rgb8(0, 255, 200);
        let uv = FontSystem::mask_soft_uv();

        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.glow_ring(r, &c, 6, 7.0, col, uv);
        assert!(
            dl.runs.iter().any(|run| run.image == Some(SHAPE_ADD)),
            "the vector glow must ride the additive shape lane"
        );
        assert!(dl.runs.iter().all(|run| run.image != Some(SHAPE)));

        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.shadow(r, &c, [3.0, 4.0], 7.0, col, uv);
        assert!(
            dl.runs.iter().all(|run| run.image != Some(SHAPE_ADD)),
            "a shadow is not light"
        );
        assert!(dl.runs.iter().any(|run| run.image == Some(SHAPE)));
    }

    /// **Claim 3: the bits, and the numbers beside them.** A glow sets
    /// [`Shape::GAUSS`] and [`Shape::OUTSIDE_ONLY`]; a shadow sets
    /// GAUSS alone; a crisp shape sets neither and leaves `feather` at
    /// zero, which is the invariant every record shipped before this
    /// step relied on.
    ///
    /// The shadow's OFFSET is asserted here too, because it is the
    /// whole reason the record needed no new field: a shifted shadow is
    /// a shifted rect, resolved on the CPU, and the silhouette that
    /// arrives is the panel's own — corners included, where the sprite
    /// could only ever cast a rectangle.
    #[test]
    fn the_soft_bits_and_the_feather_ride_the_record() {
        let r = Rect::new(20.0, 30.0, 160.0, 90.0);
        let c = [Corner::round(11.0); 4];
        let uv = FontSystem::mask_soft_uv();
        let col = Color::rgb8(0, 255, 200);

        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.glow_ring(r, &c, 6, 6.0, col, uv);
        let g = dl.shapes()[0];
        assert_eq!(g.flags & Shape::SOFT, Shape::GAUSS | Shape::OUTSIDE_ONLY);
        assert_eq!(g.feather, 6.0);
        assert_eq!(g.corner, [11.0; 4], "the glow wears the silhouette it wraps");

        let (dx, dy) = (5.0, 7.0);
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.shadow(r, &c, [dx, dy], 12.0, col, uv);
        let s = dl.shapes()[0];
        assert_eq!(s.flags & Shape::SOFT, Shape::GAUSS, "a shadow has an inside");
        assert_eq!(s.feather, 12.0);
        assert_eq!(s.corner, [11.0; 4], "a rounded panel casts a rounded shadow");
        assert_eq!(
            s.half,
            [r.w * 0.5, r.h * 0.5],
            "the shadow is SHIFTED, never inflated — the sprite inflated \
             because its feather ran inward, and this one's runs out"
        );
        let mid = dl.verts.iter().fold([0.0f32; 2], |a, v| {
            [a[0] + v.pos[0] / dl.verts.len() as f32, a[1] + v.pos[1] / dl.verts.len() as f32]
        });
        assert!(
            (mid[0] - (r.x + r.w * 0.5 + dx)).abs() < 1e-3
                && (mid[1] - (r.y + r.h * 0.5 + dy)).abs() < 1e-3,
            "the shadow did not move with its offset: {mid:?}"
        );

        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.ring_fill(r, &c, 6, col);
        let plain = dl.shapes()[0];
        assert_eq!(plain.flags & Shape::SOFT, 0, "a crisp record claims no soft bit");
        assert_eq!(plain.feather, 0.0);
    }

    /// **Claim 4, the two refusals a soft record has to make**, both of
    /// which the SILHOUETTE mask cannot make for it: the soft bits sit
    /// outside [`Shape::SILHOUETTE`] on purpose (they do not change
    /// which curve is drawn), so a glow's bits compare EQUAL to those
    /// of the panel it wraps.
    ///
    /// * It must not weld. The bed of a panel and the glow around it
    ///   share centre, half sizes, corners and kind — everything the
    ///   weld compares — so without the refusal the glow's colour would
    ///   be composited into the panel's quad and its profile lost
    ///   entirely.
    /// * It must not offer a weld. A glow is FILL without STROKE, which
    ///   is exactly the offer's own shape, so a border drawn after one
    ///   would sink its band into the GLOW's record and the panel would
    ///   never get it.
    #[test]
    fn a_soft_record_neither_welds_nor_offers_a_weld() {
        let r = Rect::new(15.0, 25.0, 140.0, 80.0);
        let c = [Corner::round(9.0); 4];
        let bed = Color::rgb8(20, 30, 40);
        let halo = Color::rgb8(0, 255, 200);
        let edge = Color::rgb8(255, 0, 128);
        let uv = FontSystem::mask_soft_uv();

        // Bed, then glow: two records, and the bed keeps its own colour.
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.ring_fill(r, &c, 6, bed);
        dl.glow_ring(r, &c, 6, 5.0, halo, uv);
        assert_eq!(dl.shape_len(), 2, "the glow welded into the bed");
        assert_eq!(dl.shapes()[0].flags & Shape::SOFT, 0);
        assert_eq!(dl.shapes()[1].flags & Shape::GAUSS, Shape::GAUSS);

        // Glow, then border: the border may not join the glow.
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.glow_ring(r, &c, 6, 5.0, halo, uv);
        dl.ring(r, &c, 6, 2.0, edge);
        assert_eq!(dl.shape_len(), 2, "the border welded onto the glow");
        assert_eq!(
            dl.shapes()[0].flags & Shape::STROKE,
            0,
            "the glow took the border's band"
        );
        assert_eq!(dl.shapes()[1].flags & Shape::STROKE, Shape::STROKE);
    }

    /// A GLOW refuses the core split, and this is the one refusal that
    /// is about the picture rather than about cost. The split's premise
    /// is that the field would have returned the fill and nothing else
    /// inside the core; under [`Shape::OUTSIDE_ONLY`] the field returns
    /// NOTHING there. Cut a core out and the plain-fill path paints the
    /// whole face of the panel in the glow's colour at full alpha — the
    /// exact opposite of what a glow is.
    ///
    /// A shadow, whose interior IS its plateau, keeps the split; that
    /// half is asserted in `a_soft_record_grows_its_quad_and_its_band`.
    #[test]
    fn a_glow_keeps_its_whole_quad_and_paints_no_core() {
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.glow_ring(
            // Big enough that the split would certainly have fired: the
            // same rect splits in the crisp case below.
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &[Corner::round(10.0); 4],
            6,
            8.0,
            Color::rgb8(0, 255, 200),
            FontSystem::mask_soft_uv(),
        );
        assert_eq!(dl.verts.len(), 6, "the glow was cut into a frame");
        assert!(
            dl.verts.iter().all(|v| v.shape != NO_SHAPE),
            "a plain-fill quad appeared under the glow — that is the \
             panel's whole face at the glow's own alpha"
        );

        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.ring_fill(
            Rect::new(0.0, 0.0, 400.0, 300.0),
            &[Corner::round(10.0); 4],
            6,
            Color::rgb8(0, 255, 200),
        );
        assert!(
            dl.verts.iter().any(|v| v.shape == NO_SHAPE),
            "the control did not split, so the test above proves nothing"
        );
    }

    /// Every vertex of the fill must satisfy the eight half-planes of
    /// the chamfered octagon — a fill poking past a cut corner is the
    /// bug this shape exists to prevent.
    #[test]
    fn chamfer_fill_stays_inside_the_frame() {
        let (x, y, w, h, cut) = (10.0, 20.0, 200.0, 100.0, 16.0);
        let mut dl = DrawList::new();
        dl.chamfer_fill(x, y, w, h, cut, Color::rgb8(255, 255, 255));
        assert!(!dl.verts.is_empty());
        let e = 0.001;
        for v in &dl.verts {
            let [px, py] = v.pos;
            assert!(px >= x - e && px <= x + w + e && py >= y - e && py <= y + h + e);
            // The four corner diagonals, as x+y style half-planes.
            assert!(px + py >= x + y + cut - e, "top-left corner leaks");
            assert!((x + w - px) + py >= cut - e + y, "top-right corner leaks");
            assert!(px + (y + h - py) >= x + cut - e, "bottom-left corner leaks");
            assert!((x + w - px) + (y + h - py) >= cut - e, "bottom-right corner leaks");
        }
    }

    // -----------------------------------------------------------------
    // The vector lane (f3 §2).

    /// Level A survives the switch: the register holds intent, and the
    /// intent of ring/ring_fill does not move when their tessellation
    /// becomes one record and one quad each. The vertices move, the
    /// dump does not — and the SHAPE run exists only on the new lane.
    #[test]
    fn the_vector_switch_moves_the_vertices_and_not_the_register() {
        let scene = |vector: bool| {
            let mut dl = DrawList::recording();
            dl.set_vector(vector);
            let r = Rect::new(10.0, 20.0, 200.0, 100.0);
            dl.ring(r, &[Corner::round(8.0); 4], 6, 2.0, ink());
            dl.ring_fill(r, &[Corner::round(8.0); 4], 6, wash());
            dl
        };
        let (old, new) = (scene(false), scene(true));
        assert_eq!(dump(&old), dump(&new));
        assert_ne!(old.verts.len(), new.verts.len());
        assert_eq!(old.shape_len(), 0);
        assert_eq!(new.shape_len(), 2);
        assert!(old.runs.iter().all(|r| r.image != Some(SHAPE)));
        assert!(new.runs.iter().any(|r| r.image == Some(SHAPE)));
        // And the lane is opt-in: a fresh list tessellates.
        let mut dl = DrawList::new();
        dl.ring_fill(Rect::new(0.0, 0.0, 10.0, 10.0), &[Corner::SQUARE; 4], 6, ink());
        assert_eq!(dl.shape_len(), 0);
    }

    /// R3's control: a frame at warp 1, a 4×4 grid of WHOLE quads at
    /// warp 4 — 30 and 96 vertices — every shape-lane vertex carrying
    /// the same record index, so a ride's transform bends the grid and
    /// the record stays one.
    ///
    /// The ride is the case §7b's split stays out of (the field's
    /// screen gradient is no longer one), so warp 4 is also the control
    /// that shows what the unsplit geometry still looks like.
    #[test]
    fn a_shape_quad_splits_into_the_warp_grid() {
        let spec = ShapeSpec {
            rect: Rect::new(0.0, 0.0, 100.0, 50.0),
            corners: [Corner::round(6.0); 4],
            kind: ShapeKind::Box,
            fill: Some(ink()),
            stroke: None,
            glass: None,
            soft: None,
        };
        let mut dl = DrawList::new();
        dl.shape(&spec);
        assert_eq!(dl.verts.len(), FRAME, "core plus four strips");
        assert_eq!(dl.shape_len(), 1);
        let mut dl = DrawList::new();
        dl.set_warp(4);
        dl.shape(&spec);
        assert_eq!(dl.verts.len(), 96, "a ride does not split the interior out");
        assert_eq!(dl.shape_len(), 1, "a grid is one shape, not sixteen");
        assert!(dl.verts.iter().all(|v| v.shape == 0));
    }

    /// Level D (§2.7): an axis-aligned INTEGER rect with
    /// a 1 px stroke passes the snap untouched — same edges, same
    /// stroke — and a fractional one lands on the grid: edges rounded,
    /// the stroke on the baker's own max(1, round) rule, corner radii
    /// never rounded. The centre is recoverable from any vertex as
    /// pos − uv, which is also the uv contract itself.
    #[test]
    fn an_integer_rect_survives_the_snap_and_a_fractional_one_lands_on_it() {
        let shape_of = |r: Rect, w: f32| {
            let mut dl = DrawList::new();
            dl.set_vector(true);
            dl.ring(r, &[Corner::round(4.3); 4], 6, w, ink());
            (dl.shapes()[0], dl.verts.clone())
        };
        let (s, verts) = shape_of(Rect::new(10.0, 20.0, 200.0, 100.0), 1.0);
        assert_eq!(s.half, [100.0, 50.0]);
        assert_eq!(s.stroke, 1.0);
        assert_eq!(s.corner, [4.3; 4], "radii are never snapped");
        for v in &verts {
            assert_eq!([v.pos[0] - v.uv[0], v.pos[1] - v.uv[1]], [110.0, 70.0]);
        }
        let (s, verts) = shape_of(Rect::new(10.4, 19.6, 200.2, 100.1), 0.4);
        assert_eq!(s.half, [100.5, 50.0], "10..211 by 20..120");
        assert_eq!(s.stroke, 1.0, "a hairline never drops under one device px");
        for v in &verts {
            assert_eq!([v.pos[0] - v.uv[0], v.pos[1] - v.uv[1]], [110.5, 70.0]);
        }
    }

    /// R9: sentinels resolve on the CPU and the record never holds a
    /// negative corner — pill is half the short side, same_as_parent
    /// the first corner's own resolved size, and any other absence is
    /// zero, exactly corner_radius's rule.
    #[test]
    fn sentinels_resolve_before_the_record() {
        let pill = crate::theme::expr::sentinel("pill").unwrap();
        let same = crate::theme::expr::sentinel("same_as_parent").unwrap();
        let auto = crate::theme::expr::sentinel("auto").unwrap();
        let mut dl = DrawList::new();
        dl.shape(&ShapeSpec {
            rect: Rect::new(0.0, 0.0, 200.0, 8.0),
            corners: [
                Corner::round(pill),
                Corner::round(same),
                Corner::round(3.0),
                Corner::round(auto),
            ],
            kind: ShapeKind::Box,
            fill: Some(ink()),
            stroke: None,
            glass: None,
            soft: None,
        });
        assert_eq!(dl.shapes()[0].corner, [4.0, 4.0, 3.0, 0.0]);
    }

    /// The record carries what the call meant: ring is STROKE alone
    /// with its colour in stroke_c, ring_fill FILL alone with the
    /// colour on the vertex; the corner styles pack two bits each in
    /// ring_points' order; Box contributes no kind bits. clear() drops
    /// the records and rests the warp back to one — while the vector
    /// switch, a mode rather than frame state, survives it.
    #[test]
    fn the_record_carries_the_flags_and_clear_resets_the_frame() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        let c = [
            Corner::SQUARE,
            Corner::round(4.0),
            Corner::chamfer(6.0),
            Corner::round(2.0),
        ];
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.set_warp(4);
        dl.ring(r, &c, 6, 2.0, ink());
        dl.ring_fill(r, &c, 6, wash());
        let (ring, fill) = (dl.shapes()[0], dl.shapes()[1]);
        let styles = (1u32 << 2) | (2 << 4) | (1 << 6);
        assert_eq!(ring.flags, styles | Shape::STROKE);
        assert_eq!(ring.stroke_c, ink().to_array());
        assert_eq!(ring.stroke, 2.0);
        assert_eq!(fill.flags, styles | Shape::FILL);
        assert_eq!(fill.stroke, 0.0);
        dl.clear();
        assert_eq!(dl.shape_len(), 0);
        dl.ring_fill(r, &c, 6, wash());
        assert_eq!(dl.verts.len(), FRAME, "the warp did not reset to one");
        assert_eq!(dl.shape_len(), 1, "the vector switch must survive clear()");
    }

    /// §2.10 / R4, the reason the switch stayed false: a bed and the
    /// border over it are ONE silhouette, so on the vector lane they are
    /// one record — one quad, one coverage, one blend of the shared
    /// outer edge. Two records would compose `1 − (1 − a)²` there and
    /// grow the dark rim on a translucent panel over glass.
    ///
    /// Nothing at the call site changes: this is `ring_fill` then
    /// `ring`, which is how every framed surface in the toolkit is
    /// spelled, and the register still reports both.
    #[test]
    fn a_bed_and_the_border_over_it_are_one_record() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(8.0), Corner::chamfer(6.0), Corner::SQUARE, Corner::round(2.0)];
        let mut dl = DrawList::recording();
        dl.set_vector(true);
        dl.ring_fill(r, &c, 6, wash());
        dl.ring(r, &c, 6, 2.0, ink());
        assert_eq!(dl.shape_len(), 1, "the pair wrote two records");
        assert_eq!(dl.verts.len(), FRAME, "the border drew a second frame");
        let s = dl.shapes()[0];
        assert_eq!(s.flags & (Shape::FILL | Shape::STROKE), Shape::FILL | Shape::STROKE);
        assert_eq!(s.stroke, 2.0);
        assert_eq!(s.stroke_c, ink().to_array());
        // The quads' colour is the BED's — §2.10's mix starts from the
        // fill and the band's own colour rides the record — and that
        // holds for the core cut out of the middle as much as for the
        // strips: it is the same fill, drawn where the field had
        // nothing left to say (§7b).
        assert!(dl.verts.iter().all(|v| v.color == wash().to_array()));
        assert!(dl.verts[..6].iter().all(|v| v.shape == NO_SHAPE), "the core");
        assert!(dl.verts[6..].iter().all(|v| v.shape == 0), "the strips");
        // Level A: the register is untouched. What the frame MEANT is
        // still a fill and a ring; only the triangles moved.
        let mut tess = DrawList::recording();
        tess.ring_fill(r, &c, 6, wash());
        tess.ring(r, &c, 6, 2.0, ink());
        assert_eq!(dump(&dl), dump(&tess));
    }

    /// …and only that pair. The weld is offered by the bed and taken by
    /// the very next call, on the resolved silhouette, with nothing
    /// between: a different rect, a different corner, a clip in the way,
    /// anything drawn in between, or a second border, and the border
    /// gets a record of its own. Each case here would be a wrong picture
    /// if it welded.
    #[test]
    fn only_the_border_that_shares_the_bed_s_silhouette_welds() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(8.0); 4];
        let armed = || {
            let mut dl = DrawList::new();
            dl.set_vector(true);
            dl.ring_fill(r, &c, 6, wash());
            dl
        };
        // A border on another rect.
        let mut dl = armed();
        dl.ring(Rect::new(10.0, 20.0, 200.0, 99.0), &c, 6, 2.0, ink());
        assert_eq!(dl.shape_len(), 2, "a different rect welded");
        // A border on another corner treatment.
        let mut dl = armed();
        dl.ring(r, &[Corner::chamfer(8.0); 4], 6, 2.0, ink());
        assert_eq!(dl.shape_len(), 2, "a different corner welded");
        // A clip pushed in between: the border belongs to a different
        // scissor and cannot ride the bed's own quad.
        let mut dl = armed();
        dl.push_clip(0.0, 0.0, 50.0, 50.0);
        dl.ring(r, &c, 6, 2.0, ink());
        assert_eq!(dl.shape_len(), 2, "a clip change welded");
        // Anything drawn in between: the border would sink under it.
        let mut dl = armed();
        dl.rect(0.0, 0.0, 4.0, 4.0, ink());
        dl.ring(r, &c, 6, 2.0, ink());
        assert_eq!(dl.shape_len(), 2, "a draw in between welded");
        // A second border on the same outline — two bands, two records.
        let mut dl = armed();
        dl.ring(r, &c, 6, 2.0, ink());
        dl.ring(r, &c, 6, 4.0, wash());
        assert_eq!(dl.shape_len(), 2, "the second band welded onto the first");
        assert_eq!(dl.shapes()[0].stroke, 2.0);
        assert_eq!(dl.shapes()[1].stroke, 4.0);
        // The other order — a bed laid over a border — is two shapes and
        // stays two: the bed would bury the band it welded to.
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.ring(r, &c, 6, 2.0, ink());
        dl.ring_fill(r, &c, 6, wash());
        assert_eq!(dl.shape_len(), 2);
        // And a ride withdraws the offer: the bed's quads were laid out
        // on the old grid.
        let mut dl = armed();
        dl.set_warp(4);
        dl.ring(r, &c, 6, 2.0, ink());
        assert_eq!(dl.shape_len(), 2, "the warp change welded");
    }

    /// The half of R4 that the border weld alone left standing, and the
    /// commonest shape in the whole interface: a plate with a state
    /// wash over it and a border over both — `button::dress`
    /// (`button.rs:109`/`:114`/`:116`), `text_input::draw`
    /// (`text_input.rs:953`/`:967`/`:977`), and through `dropdown.rs`
    /// every row of every drop-down. Three calls, ONE silhouette, so
    /// one record, one quad and one antialiased edge; the wash rides
    /// the bed's own vertices, composited.
    #[test]
    fn a_wash_over_the_bed_joins_it_and_the_border_still_welds() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(8.0); 4];
        let plate = Color::rgb8(20, 30, 40);
        let over = Color::rgba8(200, 60, 90, 102);
        let mut dl = DrawList::recording();
        dl.set_vector(true);
        dl.ring_fill(r, &c, 6, plate);
        dl.ring_fill(r, &c, 6, over);
        dl.ring(r, &c, 6, 2.0, ink());
        assert_eq!(dl.shape_len(), 1, "the wash wrote a second record");
        assert_eq!(dl.verts.len(), FRAME, "the wash drew a second frame");
        let s = dl.shapes()[0];
        assert_eq!(s.flags & (Shape::FILL | Shape::STROKE), Shape::FILL | Shape::STROKE);
        assert_eq!(s.stroke, 2.0);
        assert_eq!(s.stroke_c, ink().to_array());
        // The bed's colour is the wash over the plate — worked out here
        // from the blend equation, not read back from `fill_over`.
        let a = over.a;
        let want = [
            over.r * a + plate.r * (1.0 - a),
            over.g * a + plate.g * (1.0 - a),
            over.b * a + plate.b * (1.0 - a),
            1.0,
        ];
        for v in &dl.verts {
            for k in 0..4 {
                assert!(
                    (v.color[k] - want[k]).abs() < 1e-6,
                    "channel {k}: {} vs {}",
                    v.color[k],
                    want[k]
                );
            }
        }
        // Level A: the register still reports three calls, in order.
        let mut tess = DrawList::recording();
        tess.ring_fill(r, &c, 6, plate);
        tess.ring_fill(r, &c, 6, over);
        tess.ring(r, &c, 6, 2.0, ink());
        assert_eq!(dump(&dl), dump(&tess));
    }

    /// …and the composite is not merely plausible: for every background
    /// it lands where the two blends it replaces would have landed.
    /// This is the identity the weld rests on — one source over an
    /// unknown destination, matched coefficient by coefficient — so it
    /// is checked against a simulation of the hardware rather than
    /// against the formula that produced it.
    #[test]
    fn the_welded_bed_blends_where_the_two_fills_would_have() {
        let r = Rect::new(0.0, 0.0, 40.0, 20.0);
        let c = [Corner::SQUARE; 4];
        let cases = [
            (Color::rgba8(200, 60, 90, 255), Color::rgba8(20, 200, 40, 64)),
            (Color::rgba8(200, 60, 90, 128), Color::rgba8(20, 200, 40, 128)),
            (Color::rgba8(10, 10, 10, 0), Color::rgba8(240, 240, 240, 200)),
            (Color::rgba8(240, 240, 240, 200), Color::rgba8(10, 10, 10, 0)),
            (Color::rgba8(90, 90, 90, 30), Color::rgba8(30, 30, 30, 20)),
        ];
        for (bed, top) in cases {
            let mut dl = DrawList::new();
            dl.set_vector(true);
            dl.ring_fill(r, &c, 6, bed);
            dl.ring_fill(r, &c, 6, top);
            assert_eq!(dl.shape_len(), 1);
            let got = dl.verts[0].color;
            for dst in [0.0f32, 0.37, 1.0] {
                for k in 0..3 {
                    let ch = |c: Color| [c.r, c.g, c.b][k];
                    // Two blends, as the tessellated lane would do them.
                    let d1 = ch(bed) * bed.a + dst * (1.0 - bed.a);
                    let d2 = ch(top) * top.a + d1 * (1.0 - top.a);
                    // One blend, as the welded record does it.
                    let one = got[k] * got[3] + dst * (1.0 - got[3]);
                    assert!(
                        (d2 - one).abs() < 1e-6,
                        "bed {bed:?} top {top:?} dst {dst} channel {k}: {d2} vs {one}"
                    );
                }
            }
        }
    }

    /// The fill weld obeys the same fence as the border weld, and one
    /// more of its own: a fill arriving after a BAND would have to sink
    /// underneath it, so it gets a record of its own. Each case here
    /// would be a wrong picture if it welded.
    #[test]
    fn only_the_wash_that_shares_the_bed_s_silhouette_welds() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(8.0); 4];
        let armed = || {
            let mut dl = DrawList::new();
            dl.set_vector(true);
            dl.ring_fill(r, &c, 6, ink());
            dl
        };
        // A wash on another rect.
        let mut dl = armed();
        dl.ring_fill(Rect::new(10.0, 20.0, 200.0, 99.0), &c, 6, wash());
        assert_eq!(dl.shape_len(), 2, "a different rect welded");
        // A wash on another corner treatment.
        let mut dl = armed();
        dl.ring_fill(r, &[Corner::chamfer(8.0); 4], 6, wash());
        assert_eq!(dl.shape_len(), 2, "a different corner welded");
        // A clip in between: a different scissor cannot ride the bed.
        let mut dl = armed();
        dl.push_clip(0.0, 0.0, 50.0, 50.0);
        dl.ring_fill(r, &c, 6, wash());
        assert_eq!(dl.shape_len(), 2, "a clip change welded");
        // Anything drawn in between: the wash would sink under it.
        let mut dl = armed();
        dl.rect(0.0, 0.0, 4.0, 4.0, ink());
        dl.ring_fill(r, &c, 6, wash());
        assert_eq!(dl.shape_len(), 2, "a draw in between welded");
        // A wash after the border: the band is over the bed, and this
        // fill is over the band. Its own record, and the picture keeps
        // the order the caller wrote.
        let mut dl = armed();
        dl.ring(r, &c, 6, 2.0, ink());
        dl.ring_fill(r, &c, 6, wash());
        assert_eq!(dl.shape_len(), 2, "a fill welded onto a banded record");
        assert_eq!(dl.shapes()[0].flags & Shape::STROKE, Shape::STROKE);
        assert_eq!(dl.shapes()[1].flags & Shape::STROKE, 0);
        // Three fills are still one bed: the offer survives its own
        // weld, and `elev`'s glass wash over a plate over a fill is
        // spelled exactly this way.
        let mut dl = armed();
        dl.ring_fill(r, &c, 6, Color::rgba8(0, 0, 255, 100));
        dl.ring_fill(r, &c, 6, Color::rgba8(255, 0, 0, 100));
        dl.ring(r, &c, 6, 1.0, wash());
        assert_eq!(dl.shape_len(), 1, "the third fill wrote a record");
        assert_eq!(dl.verts.len(), FRAME);
        assert_eq!(dl.shapes()[0].stroke, 1.0);
    }

    /// f3 §7b, remedy 1: the core of a split shape is not "like" an
    /// ordinary fill — it IS one, vertex for vertex. Read the core's
    /// bounds back off the frame, draw `rect()` over them, and the six
    /// vertices agree in every field: position, the atlas's white
    /// pixel, colour, and `NO_SHAPE`.
    ///
    /// That is the whole safety argument stated as code. The interior
    /// is where `fs_shape` computes `cov = 1`, `a_band = 0` and returns
    /// the vertex colour it was handed; `fs_main` over the white pixel
    /// returns the same colour by a shorter road. `sdf::tests` proves
    /// the two roads meet at every pixel.
    #[test]
    fn the_core_of_a_frame_is_the_quad_a_rect_would_have_drawn() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.ring_fill(r, &[Corner::round(8.0); 4], 6, wash());
        assert_eq!(dl.verts.len(), FRAME);
        let core = &dl.verts[..6];
        let (x0, y0) = (core[0].pos[0], core[0].pos[1]);
        let (x1, y1) = (core[2].pos[0], core[2].pos[1]);
        // The band is corner + AA_PAD + CORE_PAD deep, with no border
        // on the record yet.
        assert_eq!([x0, y0, x1, y1], [21.0, 31.0, 199.0, 109.0]);
        let mut plain = DrawList::new();
        plain.rect(x0, y0, x1 - x0, y1 - y0, wash());
        assert_eq!(plain.verts.len(), 6);
        for (a, b) in core.iter().zip(&plain.verts) {
            assert_eq!(a.pos, b.pos);
            assert_eq!(a.uv, b.uv);
            assert_eq!(a.color, b.color);
            assert_eq!(a.shape, b.shape);
            assert_eq!(a.shape, NO_SHAPE);
        }
        // The core is on an ORDINARY run — the one every solid rect
        // draws through — and the strips on the shape lane. Two runs,
        // which is what the cut costs (§7b, remedy 3 is where that is
        // answered).
        assert_eq!(dl.runs.len(), 2);
        assert_eq!(dl.runs[0].image, None);
        assert_eq!(dl.runs[0].end, 6);
        assert_eq!(dl.runs[1].image, Some(SHAPE));
        assert_eq!(dl.runs[1].end, FRAME as u32);
    }

    /// …and when a border welds onto that bed, the core gives back
    /// exactly the border's width on every side. The geometry was laid
    /// out for a bed with no border — nothing could have known one was
    /// coming — so the frame is re-cut in place: same vertex count,
    /// same colours, same record indices, deeper strips.
    ///
    /// Without this the border's own band would run under the core, and
    /// the fill path would paint over the part of it nearest the
    /// inside: a border that thins by a pixel or two wherever the core
    /// begins.
    #[test]
    fn a_border_welding_on_re_cuts_the_core_it_landed_in() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(8.0); 4];
        let core_of = |t: Option<f32>| {
            let mut dl = DrawList::new();
            dl.set_vector(true);
            dl.ring_fill(r, &c, 6, wash());
            if let Some(t) = t {
                dl.ring(r, &c, 6, t, ink());
            }
            assert_eq!(dl.verts.len(), FRAME);
            assert_eq!(dl.shape_len(), 1);
            let v = &dl.verts[..6];
            ([v[0].pos, v[2].pos], dl.verts[6..].to_vec())
        };
        let (bare, _) = core_of(None);
        for t in [1.0f32, 4.0] {
            let (cut, strips) = core_of(Some(t));
            assert_eq!(cut[0], [bare[0][0] + t, bare[0][1] + t], "{t} px");
            assert_eq!(cut[1], [bare[1][0] - t, bare[1][1] - t], "{t} px");
            // The strips still reach the padded bounds outward and
            // still carry the record and the bed's colour.
            assert!(strips.iter().all(|v| v.shape == 0));
            assert!(strips.iter().all(|v| v.color == wash().to_array()));
            let left = strips.iter().fold(f32::MAX, |m, v| m.min(v.pos[0]));
            assert_eq!(left, r.x - 1.0, "the AA margin moved");
            // The uv contract survives the re-cut: pos − uv is the
            // record's centre for every strip vertex.
            for v in &strips {
                assert_eq!([v.pos[0] - v.uv[0], v.pos[1] - v.uv[1]], [110.0, 70.0]);
            }
            // And the re-cut lands where a caller who said both parts
            // AT ONCE would have landed. Two roads to one record, and
            // to one frame around it.
            let mut once = DrawList::new();
            once.shape(&ShapeSpec {
                rect: r,
                corners: c,
                kind: ShapeKind::Box,
                fill: Some(wash()),
                stroke: Some((t, ink())),
                glass: None,
                soft: None,
            });
            let mut pair = DrawList::new();
            pair.set_vector(true);
            pair.ring_fill(r, &c, 6, wash());
            pair.ring(r, &c, 6, t, ink());
            assert_eq!(once.shapes(), pair.shapes(), "{t} px");
            for (a, b) in once.verts.iter().zip(&pair.verts) {
                assert_eq!((a.pos, a.uv, a.color, a.shape), (b.pos, b.uv, b.color, b.shape));
            }
            assert_eq!(once.verts.len(), pair.verts.len());
        }
    }

    // -----------------------------------------------------------------
    // K3b — glass on the band (f3 §3.3).

    fn frost() -> Color {
        Color::rgba8(30, 60, 90, 160)
    }

    /// How `window::frame` and `elev::Level::draw` spell a frosted
    /// surface, and the only spelling either of them has: the frost,
    /// the wash over it, the border over both. Three calls, and after
    /// K3b one record with one edge.
    fn frosted(depth: f32, vector: bool, wash_a: bool, border: bool) -> DrawList {
        let mut dl = DrawList::new();
        dl.set_vector(vector);
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(8.0); 4];
        dl.glass_fill(r, &c, 6, depth, frost());
        if wash_a {
            dl.ring_fill(r, &c, 6, wash());
        }
        if border {
            dl.ring(r, &c, 6, 2.0, ink());
        }
        dl
    }

    /// **The whole of K3b in one assertion.** A frosted surface is ONE
    /// record — frost, wash and border — so its silhouette is
    /// antialiased once; its perimeter rides a `SHAPE_GLASS_*` run so
    /// the frost's own corners are analytic; and its interior still
    /// rides `GLASS_RANK_*`, carrying the tint, because there is no
    /// edge in there to be smooth about.
    #[test]
    fn a_frosted_surface_is_one_record_and_its_core_is_still_a_glass_run() {
        let dl = frosted(2.0, true, true, true);
        assert_eq!(dl.shape_len(), 1, "the wash or the border wrote its own record");
        let rec = dl.shapes()[0];
        assert_eq!(rec.flags & Shape::FILL, Shape::FILL);
        assert_eq!(rec.flags & Shape::STROKE, Shape::STROKE);
        // And a frost spends no FLAG on saying so. The tint below and
        // the lane further down are the two readers there are; a third
        // statement of the same fact would be a bit of this record
        // spent on nothing, with §2.5's table still waiting for 14.
        assert_eq!(
            rec.flags & !(Shape::SILHOUETTE | Shape::FILL | Shape::STROKE),
            0,
            "a record grew a flag past the two parts and the silhouette"
        );
        assert_eq!(rec.tint, frost().to_array(), "the tint did not reach the record");
        assert_eq!(rec.stroke_c, ink().to_array());
        // Three runs, in the order the layers stack.
        let lanes: Vec<Option<ImageId>> = dl.runs.iter().map(|r| r.image).collect();
        assert_eq!(lanes, vec![Some(GLASS_RANK_2), None, Some(SHAPE_GLASS_2)]);
        // The frost's core keeps the TINT; every quad above it carries
        // the wash. A weld that recoloured the core would have washed
        // the frost away — the whole reason the bed's paint mark is not
        // its first vertex.
        assert!(dl.verts[..6].iter().all(|v| v.color == frost().to_array()));
        assert!(dl.verts[6..].iter().all(|v| v.color == wash().to_array()));
        assert!(dl.verts[..12].iter().all(|v| v.shape == NO_SHAPE), "the cores");
        assert!(dl.verts[12..].iter().all(|v| v.shape == 0), "the strips");
        // Two cores over the one core rect, then the four strips.
        assert_eq!(dl.verts.len(), FRAME + 6);
        assert_eq!(dl.verts[0].pos, dl.verts[6].pos);
        assert_eq!(dl.verts[2].pos, dl.verts[8].pos);
    }

    /// The rank rides the HANDLE, on both lanes and at every rung — the
    /// renderer binds one blurred target per run, so a record could
    /// only have said it twice.
    #[test]
    fn every_rung_reaches_its_own_lane() {
        for (depth, tess, field) in [
            (1.0, GLASS_RANK_1, SHAPE_GLASS_1),
            (2.0, GLASS_RANK_2, SHAPE_GLASS_2),
            (3.0, GLASS_RANK_3, SHAPE_GLASS_3),
        ] {
            let dl = frosted(depth, true, true, true);
            assert!(dl.runs.iter().any(|r| r.image == Some(tess)), "{depth}: no core");
            assert!(dl.runs.iter().any(|r| r.image == Some(field)), "{depth}: no band");
            assert!(
                dl.runs.iter().all(|r| r.image != Some(SHAPE)),
                "{depth}: a frosted band on the plain shape lane reads no target"
            );
        }
    }

    /// Off the lane nothing moved: the fans, their handles and their
    /// vertex count are the shipped picture, and no record is written
    /// at all. The switch is still down in the master, and this is what
    /// says the picture under it did not change.
    #[test]
    fn the_tessellated_frost_is_untouched_by_the_lane_it_did_not_take() {
        let dl = frosted(2.0, false, true, true);
        assert_eq!(dl.shape_len(), 0, "a record on the tessellated lane");
        assert!(dl.runs.iter().all(|r| r.image != Some(SHAPE_GLASS_2)));
        assert_eq!(dl.runs[0].image, Some(GLASS_RANK_2));
        // The fan is three vertices per boundary point, as it always
        // was — the count the ring generator produces for six segments
        // over four round corners.
        let mut pts = Vec::new();
        ring_points(
            Rect::new(10.0, 20.0, 200.0, 100.0),
            &[Corner::round(8.0); 4],
            6,
            &mut pts,
        );
        assert_eq!(dl.runs[0].end as usize, pts.len() * 3);
    }

    /// A FRACTIONAL depth is two rungs, and the second one may not weld
    /// onto the first: one run binds one blurred target, so a fragment
    /// that read both would be a fragment that cannot exist. The wash
    /// and the border join the UPPER layer, which is the one that lies
    /// over the other.
    #[test]
    fn a_fractional_depth_is_two_frosts_and_the_wash_joins_the_upper() {
        let dl = frosted(1.5, true, true, true);
        assert_eq!(dl.shape_len(), 2, "the frosts merged or multiplied");
        let (lo, hi) = (dl.shapes()[0], dl.shapes()[1]);
        // The lower layer carries no wash and no border: they welded
        // into the upper one, which is the one drawn last.
        assert_eq!(lo.flags & Shape::STROKE, 0);
        assert_eq!(hi.flags & Shape::STROKE, Shape::STROKE);
        let lanes: Vec<Option<ImageId>> = dl.runs.iter().map(|r| r.image).collect();
        assert_eq!(
            lanes,
            vec![
                Some(GLASS_RANK_1),
                Some(SHAPE_GLASS_1),
                Some(GLASS_RANK_2),
                None,
                Some(SHAPE_GLASS_2),
            ],
            "the two rungs did not stack in order"
        );
    }

    /// **K3c.** Two rungs, two records — the renderer still binds one
    /// pyramid target per run, so that half of the split stays — but
    /// only ONE of the two records may carry a BAND, or the silhouette's
    /// shared outer edge blends twice (`c·(1 − c)·a·b`, R4 by another
    /// name). The lower rung's record is silenced at the band (its own
    /// core, drawn straight with no coverage ramp, is unaffected and
    /// still carries the interpolation); the upper rung's band carries
    /// the two rungs' COMBINED alpha, the standard over-composite of two
    /// fully-covering layers, so the one coverage evaluation that
    /// survives folds in what both rungs would have contributed.
    #[test]
    fn a_fractional_depth_blends_its_band_once() {
        let dl = frosted(1.5, true, true, true);
        let (lo, hi) = (dl.shapes()[0], dl.shapes()[1]);
        let a1 = frost().a;
        let a2 = a1 * 0.5;
        let combined = a1 + a2 - a1 * a2;
        assert_eq!(lo.tint[3], 0.0, "the lower rung still wrote a band");
        assert_eq!(lo.tint[..3], frost().to_array()[..3], "the silenced band changed hue");
        assert!(
            (hi.tint[3] - combined).abs() < 1e-6,
            "the upper band is not the two rungs combined: {} vs {combined}",
            hi.tint[3]
        );
        // The CORE is untouched by any of this: it is not a coverage
        // ramp (`cov` is 1 throughout it), so two sequential blends of a
        // constant alpha are exact and the interpolation between rungs
        // still lives there, at the fraction alone. Vertex 0 is the
        // lower rung's own core (unsplit from its band, §K3b); vertex 30
        // is the upper rung's, right after the lower rung's core (6) and
        // band strips (`FRAME - 6`, no paint quad — the lower rung's bed
        // is `false`).
        assert!(dl.verts[..6].iter().all(|v| v.color == frost().to_array()));
        assert!((dl.verts[30].color[3] - a2).abs() < 1e-6, "the upper core lost the fraction");
    }

    /// A frosted bed with no wash over it costs nothing: the quad is in
    /// the layout, because a weld recolours quads and never adds one,
    /// but it has no area until a wash gives it something to carry.
    /// A theme writing `glass.wash = none` is a theme, not a mistake.
    #[test]
    fn an_unwashed_frost_lays_a_bed_with_no_area_in_it() {
        let dry = frosted(2.0, true, false, true);
        assert_eq!(dry.shape_len(), 1);
        assert_eq!(dry.verts.len(), FRAME + 6, "the layout lost a quad");
        let bed = &dry.verts[6..12];
        assert!(bed.iter().all(|v| v.pos == bed[0].pos), "the bed has area");
        assert!(bed.iter().all(|v| v.color[3] == 0.0));
        // …and the frost's core is the one that keeps the interior.
        assert_ne!(dry.verts[0].pos, dry.verts[2].pos);
        // The wash opens it, at the band the record already has — onto
        // the very rectangle the frost's core covers.
        let wet = frosted(2.0, true, true, true);
        for i in 0..6 {
            assert_eq!(wet.verts[i].pos, wet.verts[6 + i].pos, "vertex {i}");
        }
    }

    /// The border deepens the band, so BOTH cores give the same ground
    /// back — the frost's and the wash's. A frost left at the old cut
    /// would be drawn twice under the strips that grew over it, and a
    /// tint that multiplies twice is a stain.
    #[test]
    fn a_border_welding_onto_a_frost_re_cuts_both_of_its_cores() {
        let bare = frosted(2.0, true, true, false);
        let cut = frosted(2.0, true, true, true);
        for q in [0usize, 1] {
            let (b, c) = (&bare.verts[q * 6..], &cut.verts[q * 6..]);
            assert_eq!(c[0].pos, [b[0].pos[0] + 2.0, b[0].pos[1] + 2.0], "quad {q}");
            assert_eq!(c[2].pos, [b[2].pos[0] - 2.0, b[2].pos[1] - 2.0], "quad {q}");
        }
        // The two cores stay on the same rectangle, which is what makes
        // "the wash lies over the frost" true pixel by pixel.
        assert_eq!(cut.verts[0].pos, cut.verts[6].pos);
        assert_eq!(cut.verts[2].pos, cut.verts[8].pos);
    }

    /// **A re-cut quad keeps the lane it was cut for.** `uv` means two
    /// different things on the two lanes: on a strip it is the local
    /// point the field is read at (`pos − centre`), on a core it is the
    /// atlas's white pixel, because a core is an ordinary fill and
    /// samples coverage like every other solid rect in this file.
    /// `respan_frame` rewrites every quad of a frame each time a wash
    /// or a border deepens the band, so it is the one place that has to
    /// know which is which — and it counts the cores off the geometry
    /// rather than assuming one, because a frosted surface has TWO.
    ///
    /// A rule written for a single core would hand the second one a
    /// local origin, and a local origin is a texture coordinate a
    /// hundred pixels outside the atlas: the interior of every frosted
    /// panel with a border would sample the glyph sheet, wrapped, and
    /// the wash would come out of the letters. It would not crash, and
    /// every position in this file's other tests would still be right.
    #[test]
    fn a_re_cut_core_reads_the_white_pixel_and_a_strip_reads_the_field() {
        let (u, v) = FontSystem::white_uv();
        // The centre of the rect `frosted` draws on: whole numbers, so
        // §2.7's snap is the identity and this is exact.
        let centre = [10.0 + 200.0 * 0.5, 20.0 + 100.0 * 0.5];
        for (name, dl) in [
            ("frost, wash and border", frosted(2.0, true, true, true)),
            ("frost and wash", frosted(2.0, true, true, false)),
            ("an unwashed frost", frosted(2.0, true, false, true)),
            ("a fractional depth", frosted(1.5, true, true, true)),
        ] {
            for (i, vx) in dl.verts.iter().enumerate() {
                if vx.shape == NO_SHAPE {
                    assert_eq!(vx.uv, [u, v], "{name}: core vertex {i} left the atlas");
                } else {
                    assert_eq!(
                        vx.uv,
                        [vx.pos[0] - centre[0], vx.pos[1] - centre[1]],
                        "{name}: strip vertex {i} lost its local origin"
                    );
                }
            }
        }
    }

    /// A FROST never joins a bed — not another frost's, and not a
    /// plain one's. The plain case is the one that could pass unnoticed:
    /// a caller filling a rect and then frosting the same rect offers a
    /// bed of exactly the right silhouette, and a frost that took it
    /// would be composited in as an ordinary fill — no tint, no lane,
    /// no blurred sample, and the frost simply gone from a picture that
    /// still draws.
    #[test]
    fn a_frost_never_joins_a_bed_that_is_not_its_own() {
        let mut dl = DrawList::new();
        dl.set_vector(true);
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(8.0); 4];
        dl.ring_fill(r, &c, 6, wash());
        dl.glass_fill(r, &c, 6, 2.0, frost());
        assert_eq!(dl.shape_len(), 2, "the frost welded into the plate under it");
        assert_eq!(dl.shapes()[0].tint, [0.0; 4], "the plate turned to glass");
        assert_eq!(dl.shapes()[1].tint, frost().to_array());
        // …and the lanes agree with the tints: the plate reads no
        // target, the frost reads its rung's.
        let lanes: Vec<Option<ImageId>> = dl.runs.iter().map(|r| r.image).collect();
        assert!(lanes.contains(&Some(SHAPE)), "{lanes:?}");
        assert!(lanes.contains(&Some(SHAPE_GLASS_2)), "{lanes:?}");
    }

    /// The register holds INTENT, and the intent of a frosted surface
    /// is the same three calls whichever lane draws them (level A).
    #[test]
    fn the_glass_switch_moves_the_vertices_and_not_the_register() {
        let scene = |vector: bool| {
            let mut dl = DrawList::recording();
            dl.set_vector(vector);
            let r = Rect::new(10.0, 20.0, 200.0, 100.0);
            let c = [Corner::round(8.0); 4];
            dl.glass_fill(r, &c, 6, 1.5, frost());
            dl.ring_fill(r, &c, 6, wash());
            dl
        };
        let (old, new) = (scene(false), scene(true));
        assert_eq!(dump(&old), dump(&new));
        assert_ne!(old.verts.len(), new.verts.len());
    }

    /// **K3b's second bounded edge case, closed for the glass rung.** A
    /// gradient border landing on an open glass bed welds into the SAME
    /// record a solid border would (§2.10) — one silhouette, one
    /// antialiased outer edge — instead of opening a second one the way
    /// `ring_grad` used to unconditionally. The record's own band can
    /// only be ONE colour (`stroke_c`), so it takes the two stops'
    /// midpoint; the true gradient still shows, painted back in by a
    /// second pass.
    #[test]
    fn a_gradient_border_welds_onto_an_open_glass_bed() {
        let mut dl = DrawList::new();
        dl.set_vector(true);
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(8.0); 4];
        let near = Color::rgb8(10, 200, 30);
        let far = Color::rgb8(250, 40, 90);
        dl.glass_fill(r, &c, 6, 2.0, frost());
        dl.ring_fill(r, &c, 6, wash());
        let before = dl.verts.len();
        dl.ring_grad(r, &c, 6, 6.0, near, far, [1.0, 0.0]);
        assert_eq!(dl.shape_len(), 1, "the gradient opened a second record");
        let rec = dl.shapes()[0];
        assert_eq!(rec.flags & Shape::STROKE, Shape::STROKE);
        assert_eq!(rec.flags & Shape::FILL, Shape::FILL, "the wash's weld was lost");
        let anchor = lerp(near, far, 0.5).to_array();
        assert_eq!(rec.stroke_c, anchor, "the welded band is not the two stops' midpoint");
        // The true gradient still shows: everything `ring_grad` itself
        // added — the frost's core and the wash's own quads are already
        // in `before` and excluded — spans close to BOTH stops. It is
        // inset by `AA_PAD` from the rect's true extreme corners (see
        // `the_gradient_overlay_stays_inset_...` below), so it never
        // reaches `near`/`far` bit for bit, but it spans far enough past
        // the anchor's own midpoint (0.51 on the red channel here) that
        // a flattened, anchor-only band could not have produced either
        // reading.
        let overlay = &dl.verts[before..];
        assert!(!overlay.is_empty(), "no overlay pass at all");
        let reds = overlay.iter().map(|v| v.color[0]);
        let lo = reds.clone().fold(f32::INFINITY, f32::min);
        let hi = reds.fold(f32::NEG_INFINITY, f32::max);
        assert!(lo < 0.1, "nothing reads close to the near stop: min red {lo}");
        assert!(hi > 0.9, "nothing reads close to the far stop: max red {hi}");
    }

    /// The overlay's own hard edges never sit where the welded record's
    /// analytic coverage ramp does: every vertex the second pass writes
    /// stays at least `AA_PAD` in from the TRUE outer silhouette — the
    /// margin that keeps `c·(1 − c)·a·b` from ever having two partial
    /// coverages to multiply at the same pixel.
    #[test]
    fn the_gradient_overlay_stays_inset_from_the_welded_record_s_outer_edge() {
        let mut dl = DrawList::new();
        dl.set_vector(true);
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::SQUARE; 4];
        dl.glass_fill(r, &c, 6, 2.0, frost());
        dl.ring_fill(r, &c, 6, wash());
        let before = dl.verts.len();
        dl.ring_grad(r, &c, 6, 6.0, Color::rgb8(0, 0, 0), Color::rgb8(255, 255, 255), [1.0, 0.0]);
        let overlay: Vec<[f32; 2]> = dl.verts[before..].iter().map(|v| v.pos).collect();
        assert!(!overlay.is_empty(), "no overlay pass at all");
        for p in &overlay {
            assert!(p[0] >= r.x + AA_PAD - 1e-3, "{p:?} reaches the left edge");
            assert!(p[0] <= r.right() - AA_PAD + 1e-3, "{p:?} reaches the right edge");
            assert!(p[1] >= r.y + AA_PAD - 1e-3, "{p:?} reaches the top edge");
            assert!(p[1] <= r.bottom() - AA_PAD + 1e-3, "{p:?} reaches the bottom edge");
        }
    }

    /// A border too thin to leave any interior once BOTH insets are
    /// taken out (`stroke <= 2 · AA_PAD`) skips the second pass and
    /// stays the welded anchor for its whole width — rim-free either
    /// way, since nothing draws a second boundary at all.
    #[test]
    fn a_hairline_gradient_border_still_welds_with_no_room_for_the_overlay() {
        let mut dl = DrawList::new();
        dl.set_vector(true);
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(8.0); 4];
        dl.glass_fill(r, &c, 6, 2.0, frost());
        dl.ring_fill(r, &c, 6, wash());
        let before = dl.verts.len();
        dl.ring_grad(r, &c, 6, 1.0, Color::rgb8(10, 200, 30), Color::rgb8(250, 40, 90), [1.0, 0.0]);
        assert_eq!(dl.shape_len(), 1, "the hairline border opened a second record");
        assert_eq!(before, dl.verts.len(), "a hairline overlay drew geometry it has no room for");
    }

    /// Off a glass bed — no wash just drawn, no bed at all — a gradient
    /// ring is UNCHANGED: still tessellated, still no record, exactly
    /// the bounded cost `ring_grad`'s own doc already named for the
    /// general case. Only a glass rung closes it.
    #[test]
    fn a_gradient_ring_with_no_open_glass_bed_is_untouched() {
        let mut dl = DrawList::new();
        dl.set_vector(true);
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(8.0); 4];
        dl.ring_grad(r, &c, 6, 6.0, Color::rgb8(10, 200, 30), Color::rgb8(250, 40, 90), [1.0, 0.0]);
        assert_eq!(dl.shape_len(), 0, "a bare gradient ring wrote a record");
    }

    /// An ORDINARY bed (a plain wash, no frost under it) stays exactly
    /// the known bounded cost too: this fix is scoped to the glass
    /// rung, not to every welded surface, so a non-glass weld must not
    /// start absorbing a gradient border it was never asked to.
    #[test]
    fn a_gradient_ring_over_an_ordinary_bed_stays_the_known_bounded_cost() {
        let mut dl = DrawList::new();
        dl.set_vector(true);
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(8.0); 4];
        dl.ring_fill(r, &c, 6, wash());
        let before = dl.shape_len();
        assert_eq!(before, 1, "the ordinary bed did not even write its own record");
        dl.ring_grad(r, &c, 6, 6.0, Color::rgb8(10, 200, 30), Color::rgb8(250, 40, 90), [1.0, 0.0]);
        assert_eq!(dl.shape_len(), before, "a non-glass bed welded — out of this fix's scope");
    }

    // -----------------------------------------------------------------
    // K4 — the diagonal lane (f3 §3.1).

    /// The half of §2.7 that K4 must not break: an axis-aligned segment
    /// is drawn by the same four vertices on both lanes, in the same
    /// run, with no record at all. Rules, underlines, table guides, tab
    /// underlines and the editor's grid all come through here, and the
    /// vector switch must be invisible to every one of them.
    #[test]
    fn an_axis_aligned_rule_is_the_same_quad_on_both_lanes() {
        let draw = |vector: bool| {
            let mut dl = DrawList::recording();
            dl.set_vector(vector);
            dl.line(10.0, 20.5, 90.0, 20.5, 2.0, ink());
            dl.line(10.25, 4.0, 10.25, 40.0, 1.0, wash());
            // A closed path on the grid: four axis-aligned arms and
            // four right-angle joints, and not one disc among them.
            dl.polyline(
                &[[0.0, 0.0], [30.0, 0.0], [30.0, 20.0], [0.0, 20.0]],
                1.0,
                ink(),
                true,
            );
            dl
        };
        let (old, new) = (draw(false), draw(true));
        assert_eq!(dump(&old), dump(&new), "the register moved");
        assert_eq!(new.shape_len(), 0, "an axis-aligned segment took the field");
        assert_eq!(old.verts.len(), new.verts.len());
        for (a, b) in old.verts.iter().zip(&new.verts) {
            assert_eq!((a.pos, a.uv, a.color, a.shape), (b.pos, b.uv, b.color, b.shape));
        }
        assert!(new.runs.iter().all(|r| r.image != Some(SHAPE)));
    }

    /// One diagonal, one record, one quad — and the uv contract that
    /// makes the shader's job free: every vertex carries the LOCAL
    /// coordinate of the corner it sits on, so `to_screen(uv)` returns
    /// the position it was built from and the rasteriser hands each
    /// fragment its own local point without a matrix.
    #[test]
    fn a_diagonal_is_one_record_read_along_its_own_axes() {
        let (a, b) = ([12.0f32, 30.0], [60.0, 66.0]);
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.line(a[0], a[1], b[0], b[1], 3.0, ink());
        assert_eq!(dl.shape_len(), 1);
        assert_eq!(dl.verts.len(), 6, "one quad");
        assert_eq!(dl.runs.last().unwrap().image, Some(SHAPE));
        let rec = dl.shapes()[0];
        // 48 × 36 → a 60 px path; the band is the stroke, across it.
        assert_eq!(rec.half, [30.0, 1.5]);
        assert_eq!(rec.corner, [0.0; 4]);
        assert_eq!(rec.flags, Shape::FILL, "square corners, kind Box, fill only");
        assert_eq!(rec.stroke, 0.0);
        let (f, len) = Frame::along(a, b).unwrap();
        assert_eq!(len, 60.0);
        for v in &dl.verts {
            assert_eq!(v.shape, 0);
            // The padded local corner, and the screen point it maps to.
            assert_eq!(v.uv.map(f32::abs), [31.0, 2.5]);
            let want = f.to_screen(v.uv);
            assert!(
                (v.pos[0] - want[0]).abs() <= 1e-3 && (v.pos[1] - want[1]).abs() <= 1e-3,
                "{:?} is not to_screen({:?}) = {want:?}",
                v.pos,
                v.uv
            );
        }
    }

    /// **The weld trap, and why an oriented record never offers itself.**
    /// The two arms of a cross — `checkbox.rs:70-71`, drawn on every
    /// unchecked box in the "cross" tick style — share their centre,
    /// their half sizes, their corners and their flags exactly. §2.10's
    /// offer compares those four things, so a bed left open would have
    /// swallowed the second arm and drawn a single diagonal.
    #[test]
    fn the_two_arms_of_a_cross_stay_two_shapes() {
        let m = Rect::new(20.0, 40.0, 16.0, 16.0);
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.polyline(&[[m.x, m.y], [m.right(), m.bottom()]], 2.0, ink(), false);
        dl.polyline(&[[m.right(), m.y], [m.x, m.bottom()]], 2.0, ink(), false);
        assert_eq!(dl.shape_len(), 2, "the second arm welded onto the first");
        assert_eq!(dl.verts.len(), 12);
        assert_eq!(dl.shapes()[0], dl.shapes()[1], "the trap: identical records");
        // Different frames, same record: the two quads do not overlap
        // in the uv they carry, which is the whole difference.
        assert_ne!(dl.verts[0].pos, dl.verts[6].pos);
        // …and a shape drawn after a diagonal cannot weld onto it
        // either: the offer is closed, not merely unmatched.
        let r = Rect::new(0.0, 0.0, 40.0, 40.0);
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.ring_fill(r, &[Corner::SQUARE; 4], 6, wash());
        dl.line(0.0, 0.0, 10.0, 7.0, 1.0, ink());
        dl.ring(r, &[Corner::SQUARE; 4], 6, 1.0, ink());
        assert_eq!(dl.shape_len(), 3, "the border welded across a diagonal");
    }

    /// A disc lands where the path TURNS and both arms are on the field
    /// — and nowhere else. The tick of a checkbox turns once, the
    /// disclosure triangle three times, a straight run not at all, and
    /// a corner with an axis-aligned arm keeps the picture §2.7
    /// promised it.
    #[test]
    fn a_path_gets_a_disc_where_it_turns_and_nowhere_else() {
        let count = |pts: &[[f32; 2]], closed: bool| {
            let mut dl = DrawList::new();
            dl.set_vector(true);
            dl.polyline(pts, 2.0, ink(), closed);
            (dl.shape_len(), dl.verts.len())
        };
        // The tick: two diagonal arms, one corner. Three records.
        let tick = [[10.0f32, 25.0], [18.0, 33.0], [34.0, 12.0]];
        assert_eq!(count(&tick, false), (3, 18));
        // A closed triangle with no arm on the grid: three arms, three
        // corners, six records.
        let tri = [[10.0f32, 10.0], [26.0, 18.0], [12.0, 26.0]];
        assert_eq!(count(&tri, true), (6, 36));
        // The disclosure triangle the toolkit actually draws
        // (`paint.rs:657`) closes on a VERTICAL edge: two diagonal arms
        // and one quad, and the two corners that edge touches keep
        // today's picture — 2 records for the arms, 1 for the single
        // joint between them, and the vertical arm still a plain quad.
        let disclosure = [[10.0f32, 10.0], [26.0, 18.0], [10.0, 26.0]];
        assert_eq!(count(&disclosure, true), (3, 24));
        // A path that runs straight through its middle point: two
        // segments, no turn, no disc.
        let straight = [[0.0f32, 0.0], [10.0, 5.0], [20.0, 10.0]];
        assert_eq!(count(&straight, false), (2, 12));
        // One arm on the grid: that corner keeps today's picture.
        let bent = [[0.0f32, 0.0], [20.0, 0.0], [30.0, 14.0]];
        assert_eq!(count(&bent, false), (1, 12), "a mixed joint grew a disc");
        // The disc itself: a Box record with round corners as big as
        // its own half size, which `sdf` proves is the circle exactly.
        let mut dl = DrawList::new();
        dl.set_vector(true);
        dl.polyline(&tick, 2.0, ink(), false);
        let joint = dl.shapes()[2];
        assert_eq!(joint.half, [1.0, 1.0]);
        assert_eq!(joint.corner, [1.0; 4]);
        assert_eq!(joint.flags & Shape::SILHOUETTE, 0b01_01_01_01);
        // …centred on the corner the path turns at, to the pixel it was
        // given — no snap: a snapped joint would bend the line.
        let centre = [
            dl.verts[12].pos[0] - dl.verts[12].uv[0],
            dl.verts[12].pos[1] - dl.verts[12].uv[1],
        ];
        assert_eq!(centre, tick[1]);
    }

    /// §2.8 where it actually fires. The snap lifts every sub-pixel
    /// band on the axis-aligned lane before the record is written; a
    /// diagonal has no grid to round to, and a single coverage ramp
    /// over-reads a slab thinner than the filter. So a 0.4 px stroke is
    /// drawn one pixel wide at 0.4 of its alpha — same mass, no
    /// shimmer, no fattening — and a stroke at or above a pixel is
    /// untouched.
    #[test]
    fn a_sub_pixel_diagonal_dims_instead_of_fattening() {
        let record = |t: f32| {
            let mut dl = DrawList::new();
            dl.set_vector(true);
            dl.line(0.0, 0.0, 30.0, 40.0, t, Color::rgba8(255, 255, 255, 128));
            (dl.shapes()[0], dl.verts[0].color[3])
        };
        let (thin, alpha) = record(0.4);
        assert_eq!(thin.half, [25.0, 0.5], "the band did not reach a pixel");
        let full = Color::rgba8(255, 255, 255, 128).a;
        assert!((alpha - full * 0.4).abs() <= 1e-6, "alpha {alpha} of {full}");
        let (fat, alpha) = record(3.0);
        assert_eq!(fat.half, [25.0, 1.5]);
        assert_eq!(alpha, full, "a stroke above a pixel was dimmed");
    }

    /// The register holds INTENT, and a lane is not one: a polyline is
    /// a polyline whichever geometry carries it. The twin of
    /// `the_vector_switch_moves_the_vertices_and_not_the_register`, for
    /// the primitives K4 moves.
    #[test]
    fn the_diagonal_lane_moves_the_vertices_and_not_the_register() {
        let draw = |vector: bool| {
            let mut dl = DrawList::recording();
            dl.set_vector(vector);
            dl.line(4.0, 5.0, 44.0, 35.0, 2.0, ink());
            dl.polyline(&[[0.0, 0.0], [12.0, 9.0], [30.0, 2.0]], 1.5, wash(), false);
            dl
        };
        let (old, new) = (draw(false), draw(true));
        assert_eq!(dump(&old), dump(&new));
        assert_eq!(old.shape_len(), 0);
        assert_eq!(new.shape_len(), 4, "two segments, one joint, and the line");
        assert_ne!(old.verts.len(), new.verts.len());
    }

    // -----------------------------------------------------------------
    // The command register.

    /// Counts the heap allocations THIS THREAD makes, which is what lets
    /// "the unarmed register allocates nothing" be measured instead of
    /// asserted. The counter is thread-local because the test harness
    /// runs tests in parallel threads and a process-wide number would
    /// only measure the neighbours; it is const-initialised and reached
    /// through `try_with` because an allocator that allocates, or that
    /// panics while a thread is being torn down, is a hang.
    mod meter {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::cell::Cell;

        thread_local! {
            static N: Cell<u64> = const { Cell::new(0) };
        }

        pub struct Counting;

        unsafe impl GlobalAlloc for Counting {
            unsafe fn alloc(&self, l: Layout) -> *mut u8 {
                let _ = N.try_with(|n| n.set(n.get() + 1));
                System.alloc(l)
            }
            unsafe fn alloc_zeroed(&self, l: Layout) -> *mut u8 {
                let _ = N.try_with(|n| n.set(n.get() + 1));
                System.alloc_zeroed(l)
            }
            unsafe fn realloc(&self, p: *mut u8, l: Layout, new: usize) -> *mut u8 {
                let _ = N.try_with(|n| n.set(n.get() + 1));
                System.realloc(p, l, new)
            }
            unsafe fn dealloc(&self, p: *mut u8, l: Layout) {
                System.dealloc(p, l)
            }
        }

        #[global_allocator]
        static A: Counting = Counting;

        pub fn allocations(f: impl FnOnce()) -> u64 {
            let before = N.with(|n| n.get());
            f();
            N.with(|n| n.get()) - before
        }
    }

    fn ink() -> Color {
        Color::rgb8(10, 200, 30)
    }

    fn wash() -> Color {
        Color::rgb8(250, 40, 90)
    }

    /// A scene wide enough to reach every kind of buffer the list keeps:
    /// the clip stack, the ring scratch, the run list and the vertices.
    fn scene(dl: &mut DrawList, tint: Color) {
        let r = Rect::new(0.0, 0.0, 80.0, 40.0);
        dl.push_clip(0.0, 0.0, 300.0, 200.0);
        dl.rect(1.0, 2.0, 30.0, 40.0, tint);
        dl.rect_outline(5.0, 5.0, 50.0, 20.0, 2.0, wash());
        dl.ring(r, &[Corner::round(6.0); 4], 6, 2.0, tint);
        dl.ring_fill(r, &[Corner::chamfer(4.0); 4], 6, wash());
        dl.polyline(&[[0.0, 0.0], [10.0, 10.0], [20.0, 0.0]], 1.5, tint, true);
        dl.rect_grad(r, &[(0.0, tint), (0.5, wash()), (1.0, tint)], 0.6);
        dl.glow_ring(r, &[Corner::round(6.0); 4], 6, 8.0, wash(), FontSystem::mask_soft_uv());
        dl.shadow(r, &[Corner::SQUARE; 4], [2.0, 3.0], 4.0, wash(), FontSystem::mask_soft_uv());
        dl.pop_clip();
    }

    fn dump(dl: &DrawList) -> String {
        let mut s = String::new();
        for (i, c) in dl.cmds().iter().enumerate() {
            let _ = writeln!(s, "cmd {i} {c}");
        }
        s
    }

    #[test]
    fn the_word_arms_the_register_and_anything_else_leaves_it_off() {
        assert!(!armed_by(None));
        assert!(!armed_by(Some("")));
        assert!(!armed_by(Some("0")));
        assert!(armed_by(Some("1")));
        assert!(armed_by(Some("yes")));
    }

    /// The price of carrying the register in the shipping build, MEASURED:
    /// a warmed list that is not recording allocates nothing at all while
    /// it draws. The armed list beside it allocates on the same pass —
    /// without that half the test would pass with a broken meter.
    #[test]
    fn an_unarmed_frame_allocates_nothing_and_an_armed_one_does() {
        let mut off = DrawList::new();
        // The first pass buys the capacity every later one reuses — the
        // list is a per-process object drawn into sixty times a second,
        // so the steady state is the thing worth measuring.
        scene(&mut off, ink());
        off.clear();
        // Only now, and after the clear that would have re-read it:
        // this test is about the default, and an armed shell must not be
        // able to turn it either way.
        off.cmds = None;
        let n = meter::allocations(|| scene(&mut off, ink()));
        assert_eq!(n, 0, "an unarmed frame allocated {n} times");

        let mut on = DrawList::recording();
        scene(&mut on, ink());
        on.clear();
        let n = meter::allocations(|| scene(&mut on, ink()));
        assert!(n > 0, "the meter reads zero even for a recording list");
    }

    /// The claim the whole register rests on: the same scene twice is the
    /// same text, byte for byte — nothing in a command reads an address,
    /// an allocation or a clock, and the fixed-precision numbers leave no
    /// room for a shortest-round-trip printer to disagree with itself.
    #[test]
    fn the_same_scene_dumps_byte_for_byte() {
        let (mut a, mut b) = (DrawList::recording(), DrawList::recording());
        scene(&mut a, ink());
        scene(&mut b, ink());
        assert!(!dump(&a).is_empty());
        assert_eq!(dump(&a), dump(&b));
        assert_eq!(dump(&DrawList::recording()), "");
    }

    /// And the other half: a register that never moves proves nothing.
    #[test]
    fn a_recoloured_scene_is_a_different_dump() {
        let (mut a, mut b) = (DrawList::recording(), DrawList::recording());
        scene(&mut a, ink());
        scene(&mut b, Color { a: 0.99, ..ink() });
        assert_ne!(dump(&a), dump(&b));
        assert_eq!(a.cmds().len(), b.cmds().len(), "only the colour moved");
    }

    /// THE test this register exists for. The same commands tessellated
    /// two different ways — the segment count is the tessellation knob —
    /// give different vertex lists and the SAME dump. An SDF core that
    /// draws a rounded corner as one quad instead of twenty-eight is a
    /// bigger version of exactly this, and it must be able to prove the
    /// scene did not move while the geometry did.
    #[test]
    fn the_register_holds_the_intent_and_not_the_tessellation() {
        let r = Rect::new(10.0, 20.0, 200.0, 100.0);
        let c = [Corner::round(12.0); 4];
        let uv = FontSystem::mask_soft_uv();
        let two = |segments: u8| {
            let mut dl = DrawList::recording();
            dl.ring(r, &c, segments, 2.0, ink());
            dl.ring_fill(r, &c, segments, wash());
            dl.glow_ring(r, &c, segments, 6.0, ink(), uv);
            dl
        };
        let (coarse, fine) = (two(3), two(12));
        assert!(
            fine.verts.len() > coarse.verts.len(),
            "the two tessellations must actually differ, or the test proves nothing"
        );
        assert_eq!(dump(&coarse), dump(&fine));
    }

    /// A shape built out of other shapes enters the register ONCE, as
    /// itself. Otherwise a rounded fill would be logged as a fill AND the
    /// rect it takes a shortcut through, and the day the shortcut goes
    /// the register would report a scene change that never happened.
    #[test]
    fn a_shape_records_itself_and_not_its_parts() {
        let r = Rect::new(0.0, 0.0, 100.0, 50.0);
        let uv = FontSystem::mask_soft_uv();
        let one = |f: &dyn Fn(&mut DrawList)| {
            let mut dl = DrawList::recording();
            f(&mut dl);
            dl.cmds().len()
        };
        // rect_outline is four rects, chamfer_frame a ring, ring_fill a
        // rect or a chamfer_fill by fast path, polyline a run of lines,
        // shadow a soft_box, rect_grad a rect when it has one stop.
        assert_eq!(one(&|dl| dl.rect_outline(0.0, 0.0, 100.0, 50.0, 2.0, ink())), 1);
        assert_eq!(one(&|dl| dl.chamfer_frame(0.0, 0.0, 100.0, 50.0, 8.0, 2.0, ink())), 1);
        assert_eq!(one(&|dl| dl.ring_fill(r, &[Corner::SQUARE; 4], 6, ink())), 1);
        assert_eq!(one(&|dl| dl.ring_fill(r, &[Corner::chamfer(8.0); 4], 6, ink())), 1);
        assert_eq!(one(&|dl| dl.polyline(&[[0.0, 0.0], [1.0, 1.0], [2.0, 0.0]], 1.0, ink(), true)), 1);
        assert_eq!(one(&|dl| dl.shadow(r, &[Corner::SQUARE; 4], [1.0, 2.0], 3.0, ink(), uv)), 1);
        assert_eq!(one(&|dl| dl.rect_grad(r, &[(0.0, ink())], 0.0)), 1);
        assert_eq!(one(&|dl| dl.soft_box(r, 4.0, ink(), (0.0, 0.0, 0.0, 0.0))), 1);
        // And the suppression is not a latch: the next call still lands.
        let mut dl = DrawList::recording();
        dl.rect_outline(0.0, 0.0, 100.0, 50.0, 2.0, ink());
        dl.rect(0.0, 0.0, 1.0, 1.0, ink());
        assert_eq!(dl.cmds().len(), 2);
    }

    /// The number grain, both ways: a difference under it is deliberately
    /// invisible — that tolerance is what makes the register survive a
    /// compiler reassociating a multiply — and a difference over it must
    /// show. And the two spellings of zero print alike, because a sign
    /// bit no pixel can carry is not a scene change.
    #[test]
    fn the_grain_is_a_thousandth_of_a_pixel_and_zero_has_one_spelling() {
        let line = |x: f32| DrawCmd::Rect { r: [x, 0.0, 1.0, 1.0], color: ink() }.to_string();
        assert_eq!(line(10.0), line(10.0004));
        assert_ne!(line(10.0), line(10.001));
        assert_eq!(line(0.0), line(-0.0));
        assert_eq!(line(0.0), line(-0.0001));
        assert!(line(0.0).starts_with("rect at 0.000 0.000 1.000 1.000 rgba "));
        // A colour channel is finer, because 8-bit output is: a step of
        // 1/255 must never round away.
        let shade = |v: f32| {
            DrawCmd::Rect { r: [0.0; 4], color: Color { r: v, g: 0.0, b: 0.0, a: 1.0 } }
                .to_string()
        };
        assert_ne!(shade(0.5), shade(0.5 + 1.0 / 255.0));
        // What cannot be drawn must still be greppable rather than
        // silently rounded into a plausible number.
        assert!(line(f32::NAN).contains("nan"));
        assert!(line(f32::INFINITY).contains(" inf "));
    }

    /// One command is one line, whatever the payload: a string that
    /// carries a newline, a quote or a control character may not break a
    /// dump that is compared line by line.
    #[test]
    fn a_text_command_stays_on_one_line() {
        let c = DrawCmd::Text {
            at: [12.0, 30.0],
            anchor: TextAnchor::Centre,
            font: 1,
            px: 14.0,
            tracking: 0.5,
            tabular: 0.0,
            color: ink(),
            text: "a\"b\\c\nd\te\u{7f}".to_string(),
        };
        let s = c.to_string();
        assert!(!s.contains('\n'), "{s}");
        assert!(s.ends_with(r#""a\"b\\c\nd\te\u{7f}""#), "{s}");
        assert!(s.starts_with("text at 12.000 30.000 anchor centre font 1 px 14.000 track 0.500"));
    }

    /// A Square corner prints its style alone: `ring_points` ignores the
    /// size of a Square, so a stray size there draws nothing, and two
    /// commands that draw the same picture must print the same line.
    #[test]
    fn a_corner_prints_what_it_draws() {
        let ring = |c: Corner| {
            DrawCmd::Ring {
                r: [0.0, 0.0, 10.0, 10.0],
                corners: [c; 4],
                stroke: 1.0,
                color: ink(),
            }
            .to_string()
        };
        assert_eq!(ring(Corner::SQUARE), ring(Corner { style: CornerStyle::Square, size: 7.0 }));
        assert!(ring(Corner::SQUARE).contains(" corners square square square square "));
        assert!(ring(Corner::round(4.0)).contains(" corners round:4.000"));
        assert!(ring(Corner::chamfer(4.0)).contains(" corners chamfer:4.000"));
        assert_ne!(ring(Corner::round(4.0)), ring(Corner::chamfer(4.0)));
    }

    /// The register follows the clip stack, and records the rect the
    /// caller ASKED for rather than the intersection — the intersection
    /// is a function of the pushes already in the register.
    #[test]
    fn the_clip_stack_is_part_of_the_scene() {
        let mut dl = DrawList::recording();
        dl.push_clip(10.0, 10.0, 100.0, 100.0);
        dl.push_clip(50.0, 50.0, 100.0, 100.0);
        dl.pop_clip();
        dl.restore_clips(&[[1.0, 2.0, 3.0, 4.0]]);
        assert_eq!(
            dump(&dl),
            "cmd 0 clip push 10.000 10.000 100.000 100.000\n\
             cmd 1 clip push 50.000 50.000 100.000 100.000\n\
             cmd 2 clip pop\n\
             cmd 3 clip restore 1 1.000 2.000 3.000 4.000\n"
        );
    }
}

