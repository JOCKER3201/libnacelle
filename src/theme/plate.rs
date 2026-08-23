//! The decoration plates — the CPU rasteriser of DECISION M10 / r1 §8.
//!
//! All STATIC decoration is baked once, on the CPU, into TWO screen-sized
//! RGBA images. Per frame each costs one image quad (6 verts); the bake
//! itself runs only when the theme or the surface size changes, never per
//! frame. The application registers the pixels through the renderer's
//! ordinary image path (`create_texture` / `update_texture`) and draws
//! them with `DrawList::image`:
//!
//! * the BACKDROP plate — z 0, before anything else and therefore inside
//!   the glass snapshot. Layers, in the master's stated order:
//!   `decor.traces.*` (PCB traces: a seeded random walk on a cell grid),
//!   `decor.grid.*` (the measuring grid, minor and major lines),
//!   `decor.starfield.*` (image 6's seeded stars), and the vignette when
//!   `decor.vignette.layer = backdrop`.
//! * the OVERLAY plate (v2) — z 70, one quad after everything themed:
//!   `decor.scanlines.*`, `decor.noise.*` (film grain), and the vignette
//!   when `decor.vignette.layer = overlay` — the master's own word, the
//!   one that darkens the panels too, as image 4 shows.
//!
//! Every `seed` token pins its layer's pattern; `0` derives it from the
//! theme's name, so two silent themes still differ without either
//! authoring a number.
//!
//! NOT a plate, deliberately: `decor.ribbons.*` are the ONLY animated
//! decoration — real geometry every frame, drawn by the host inside its
//! panel, never baked. `decor.scanlines.drift` is per-frame UV motion of
//! the overlay quad and is the HOST's accumulator, not the bake's; this
//! bake is the pattern at rest (see theme-engine-notes.md).
//!
//! Every colour and length below comes from a `decor.*` token; a token
//! the master does not declare degrades through the engine's per-kind
//! fallback (grey ink, zero, false) exactly like every other draw site —
//! there is no design constant in this file. With every layer off — which
//! is `default.theme`'s shipped state — [`bake_backdrop`] and
//! [`bake_overlay`] return `None` and the program draws no plate at all:
//! the governing principle's raw run grows no decoration.
//!
//! Measured cost (Ryzen 7 9800X3D, release): a 2560x1440 bake with the
//! former aurora theme's traces on lands in ~5 ms; every layer stays inside the
//! tens-of-ms budget r1 §8 states per layer (the starfield is the
//! cheapest of all — a few hundred tiny discs). The application runs the
//! bakes on a worker thread besides, so even a slow bake never blocks a
//! frame.

use super::bake::ResolvedTheme;
use super::color::Color;
use super::TokenId;

/// One baked plate: tightly packed straight-alpha RGBA, `w * h * 4`
/// bytes — exactly what `Gfx::update_texture` takes.
pub struct Plate {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
    /// How long the bake took, for the log line and for the budget.
    pub bake_ms: f32,
}

/// Bake the backdrop plate for the CURRENT resolved theme at the given
/// surface size. `None` when nothing is enabled — the caller then draws
/// no quad and owns no texture. Reads the theme once at entry, so a
/// theme swap mid-bake cannot mix two designs.
pub fn bake_backdrop(w: u32, h: u32) -> Option<Plate> {
    let t = super::resolved();
    let p = gather(t);
    if p.is_empty() || w == 0 || h == 0 {
        return None;
    }
    Some(bake_params(&p, w, h))
}

/// Bake the overlay plate — z 70, the quad OVER everything themed:
/// scanlines, grain, the top vignette. Same contract as
/// [`bake_backdrop`]: `None` when nothing is enabled, one theme read at
/// entry.
pub fn bake_overlay(w: u32, h: u32) -> Option<Plate> {
    let t = super::resolved();
    let p = gather_overlay(t);
    if p.is_empty() || w == 0 || h == 0 {
        return None;
    }
    Some(bake_overlay_params(&p, w, h))
}

// ------------------------------------------------------------ parameters

#[derive(Clone, Copy)]
enum Falloff {
    Cos2,
    Linear,
    Quad,
}

#[derive(Clone, Copy)]
struct TracesP {
    cell: f32,
    density: f32,
    width: f32,
    color: Color,
    alpha: f32,
    via_radius: f32,
    via_alpha: f32,
    seed: u64,
    /// The walk's own shape: how long one run of trace is, and how often
    /// it bends. Texture, like the cell pitch and the density beside it,
    /// and it used to be four numbers written into this file.
    run_min: i64,
    run_max: i64,
    turn_chance: f32,
    turn_bias: f32,
}

#[derive(Clone, Copy)]
struct GridP {
    spacing: f32,
    width: f32,
    alpha: f32,
    major_every: u32,
    major_alpha: f32,
    color: Color,
}

#[derive(Clone, Copy)]
struct StarfieldP {
    count: u32,
    size_min: f32,
    size_max: f32,
    alpha_min: f32,
    alpha_max: f32,
    color: Color,
    seed: u64,
}

#[derive(Clone, Copy)]
struct VignetteP {
    strength: f32,
    radius: f32,
    color: Color,
    shape: Falloff,
}

#[derive(Clone, Copy)]
struct ScanlinesP {
    period: f32,
    duty: f32,
    alpha: f32,
    color: Color,
}

#[derive(Clone, Copy)]
struct NoiseP {
    alpha: f32,
    grain: f32,
    chroma: f32,
    seed: u64,
}

/// The backdrop plate's layers, in the master's stated z-order.
#[derive(Default)]
struct Params {
    traces: Option<TracesP>,
    grid: Option<GridP>,
    starfield: Option<StarfieldP>,
    vignette: Option<VignetteP>,
}

impl Params {
    fn is_empty(&self) -> bool {
        self.traces.is_none()
            && self.grid.is_none()
            && self.starfield.is_none()
            && self.vignette.is_none()
    }
}

/// The overlay plate's layers, in the master's stated z-order.
#[derive(Default)]
struct OverlayParams {
    scanlines: Option<ScanlinesP>,
    noise: Option<NoiseP>,
    vignette: Option<VignetteP>,
}

impl OverlayParams {
    fn is_empty(&self) -> bool {
        self.scanlines.is_none() && self.noise.is_none() && self.vignette.is_none()
    }
}

/// The master switch, and the user's ceiling over the theme:
/// `performance.decor = none` means no plates at all.
fn decor_off(t: &ResolvedTheme) -> bool {
    let id = |name: &str| super::id(name).unwrap_or(TokenId::MISSING);
    if !t.flag(id("decor.enabled")) {
        return true;
    }
    if let Some(perf) = super::id("performance.decor") {
        if super::enum_index(perf, "none") == Some(t.enum_of(perf)) {
            return true;
        }
    }
    false
}

/// A layer's `seed` token: `0` derives from the theme's name, as the
/// token's own comment specifies — two silent themes still differ.
fn seed_or_theme_name(seed: f32) -> u64 {
    if seed != 0.0 {
        seed as u64
    } else {
        fnv(super::diagnostics().localised_name("").as_bytes())
    }
}

/// The vignette, IF the theme enables it and `decor.vignette.layer`
/// names the plate being gathered. The unmatched arm of the layer word
/// is `overlay` — index 0 of the enum's word list, the master's own
/// declared word — so a themeless run darkens over the panels, exactly
/// as image 4 does.
fn gather_vignette(t: &ResolvedTheme, overlay: bool) -> Option<VignetteP> {
    let id = |name: &str| super::id(name).unwrap_or(TokenId::MISSING);
    if !t.flag(id("decor.vignette.enabled")) {
        return None;
    }
    let layer = super::id("decor.vignette.layer");
    let le = layer.map(|l| t.enum_of(l));
    let on_backdrop =
        le.is_some() && le == layer.and_then(|l| super::enum_index(l, "backdrop"));
    if on_backdrop == overlay {
        return None;
    }
    let shape = super::id("decor.vignette.shape");
    let e = shape.map(|s| t.enum_of(s));
    let word = |w: &str| shape.and_then(|s| super::enum_index(s, w));
    Some(VignetteP {
        strength: t.px(id("decor.vignette.strength")),
        radius: t.px(id("decor.vignette.radius")),
        color: t.color(id("decor.vignette.color")),
        // Index 0 of an enum's word list is the master's own declared
        // word (`cos2`), so the unmatched arm IS the kind fallback.
        shape: if e.is_some() && e == word("linear") {
            Falloff::Linear
        } else if e.is_some() && e == word("quad") {
            Falloff::Quad
        } else {
            Falloff::Cos2
        },
    })
}

/// The cold-path token reads for the BACKDROP plate. Runs on a rebake
/// only, so the by-name lookups are fine here — this is exactly the
/// "resolve at init, not in the draw loop" split, with the bake standing
/// in for init.
fn gather(t: &ResolvedTheme) -> Params {
    let id = |name: &str| super::id(name).unwrap_or(TokenId::MISSING);
    let mut out = Params::default();
    if decor_off(t) {
        return out;
    }

    if t.flag(id("decor.traces.enabled")) {
        out.traces = Some(TracesP {
            cell: t.px(id("decor.traces.cell")),
            density: t.px(id("decor.traces.density")),
            width: t.px(id("decor.traces.width")),
            color: t.color(id("decor.traces.color")),
            alpha: t.px(id("decor.traces.alpha")),
            via_radius: t.px(id("decor.traces.via_radius")),
            via_alpha: t.px(id("decor.traces.via_alpha")),
            seed: seed_or_theme_name(t.px(id("decor.traces.seed"))),
            run_min: t.px(id("decor.traces.run_min")) as i64,
            run_max: t.px(id("decor.traces.run_max")) as i64,
            turn_chance: t.px(id("decor.traces.turn_chance")),
            turn_bias: t.px(id("decor.traces.turn_bias")),
        });
    }

    if t.flag(id("decor.grid.enabled")) {
        out.grid = Some(GridP {
            spacing: t.px(id("decor.grid.spacing")),
            width: t.px(id("decor.grid.width")),
            alpha: t.px(id("decor.grid.alpha")),
            major_every: t.px(id("decor.grid.major_every")).round().max(0.0) as u32,
            major_alpha: t.px(id("decor.grid.major_alpha")),
            color: t.color(id("decor.grid.color")),
        });
    }

    if t.flag(id("decor.starfield.enabled")) {
        out.starfield = Some(StarfieldP {
            count: t.px(id("decor.starfield.count")).round().max(0.0) as u32,
            size_min: t.px(id("decor.starfield.size_min")),
            size_max: t.px(id("decor.starfield.size_max")),
            alpha_min: t.px(id("decor.starfield.alpha_min")),
            alpha_max: t.px(id("decor.starfield.alpha_max")),
            color: t.color(id("decor.starfield.color")),
            seed: seed_or_theme_name(t.px(id("decor.starfield.seed"))),
        });
    }

    out.vignette = gather_vignette(t, false);
    out
}

/// The cold-path token reads for the OVERLAY plate.
fn gather_overlay(t: &ResolvedTheme) -> OverlayParams {
    let id = |name: &str| super::id(name).unwrap_or(TokenId::MISSING);
    let mut out = OverlayParams::default();
    if decor_off(t) {
        return out;
    }

    if t.flag(id("decor.scanlines.enabled")) {
        out.scanlines = Some(ScanlinesP {
            period: t.px(id("decor.scanlines.period")),
            duty: t.px(id("decor.scanlines.duty")),
            alpha: t.px(id("decor.scanlines.alpha")),
            color: t.color(id("decor.scanlines.color")),
            // `decor.scanlines.drift` is the HOST's per-frame UV
            // accumulator over this quad, not a bake input: the plate is
            // the pattern at rest (theme-engine-notes.md).
        });
    }

    if t.flag(id("decor.noise.enabled")) {
        out.noise = Some(NoiseP {
            alpha: t.px(id("decor.noise.alpha")),
            grain: t.px(id("decor.noise.grain")),
            chroma: t.px(id("decor.noise.chroma")),
            seed: seed_or_theme_name(t.px(id("decor.noise.seed"))),
        });
    }

    out.vignette = gather_vignette(t, true);
    out
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

// ------------------------------------------------------------ the bake

fn bake_params(p: &Params, w: u32, h: u32) -> Plate {
    let t0 = std::time::Instant::now();
    let (wi, hi) = (w as usize, h as usize);
    let mut rgba = vec![0u8; wi * hi * 4];

    // Each single-colour layer rasterises into an R8 coverage first and
    // composites ONCE: overlapping stamps within a layer meet as max(),
    // so a trace crossing itself (or a grid crossing) cannot darken
    // beyond the alpha its token states. The buffer is reused.
    let mut cov = vec![0u8; wi * hi];

    if let Some(tr) = &p.traces {
        cov.fill(0);
        rasterise_traces(&mut cov, wi, hi, tr);
        composite_coverage(&mut rgba, &cov, tr.color);
    }
    if let Some(g) = &p.grid {
        cov.fill(0);
        rasterise_grid(&mut cov, wi, hi, g);
        composite_coverage(&mut rgba, &cov, g.color);
    }
    if let Some(s) = &p.starfield {
        cov.fill(0);
        rasterise_starfield(&mut cov, wi, hi, s);
        composite_coverage(&mut rgba, &cov, s.color);
    }
    if let Some(v) = &p.vignette {
        rasterise_vignette(&mut rgba, wi, hi, v);
    }

    Plate {
        w,
        h,
        rgba,
        bake_ms: t0.elapsed().as_secs_f32() * 1000.0,
    }
}

fn bake_overlay_params(p: &OverlayParams, w: u32, h: u32) -> Plate {
    let t0 = std::time::Instant::now();
    let (wi, hi) = (w as usize, h as usize);
    let mut rgba = vec![0u8; wi * hi * 4];

    // Same z-order the master states for the overlay: scanlines, noise,
    // top vignette. Scanlines share the coverage-then-composite scheme;
    // noise is per-pixel colour and blends directly, like the vignette.
    if let Some(s) = &p.scanlines {
        let mut cov = vec![0u8; wi * hi];
        rasterise_scanlines(&mut cov, wi, hi, s);
        composite_coverage(&mut rgba, &cov, s.color);
    }
    if let Some(n) = &p.noise {
        rasterise_noise(&mut rgba, wi, hi, n);
    }
    if let Some(v) = &p.vignette {
        rasterise_vignette(&mut rgba, wi, hi, v);
    }

    Plate {
        w,
        h,
        rgba,
        bake_ms: t0.elapsed().as_secs_f32() * 1000.0,
    }
}

/// Straight-alpha OVER of `color` scaled by the coverage, per pixel.
fn composite_coverage(rgba: &mut [u8], cov: &[u8], color: Color) {
    for (i, &c) in cov.iter().enumerate() {
        if c == 0 {
            continue;
        }
        let a = color.a * (c as f32 / 255.0);
        blend_px(rgba, i, color.r, color.g, color.b, a);
    }
}

/// src OVER dst in straight alpha, one pixel.
///
/// `pub(super)` rather than private: `backdrop.rs`'s `treat.tint` wash is
/// the same operation on the same byte layout — a flat colour composited
/// straight-alpha over a screen-sized RGBA plate — and a second copy of
/// eight lines of blend math is exactly the kind of drift this crate's
/// `elev::Level` header warns about (one reader, not one copy per file).
#[inline]
pub(super) fn blend_px(rgba: &mut [u8], i: usize, r: f32, g: f32, b: f32, a: f32) {
    if a <= 0.0 {
        return;
    }
    let o = i * 4;
    let da = rgba[o + 3] as f32 / 255.0;
    let oa = a + da * (1.0 - a);
    if oa <= 0.0 {
        return;
    }
    let mix = |s: f32, d: u8| {
        let d = d as f32 / 255.0;
        ((s * a + d * da * (1.0 - a)) / oa * 255.0).round().clamp(0.0, 255.0) as u8
    };
    rgba[o] = mix(r, rgba[o]);
    rgba[o + 1] = mix(g, rgba[o + 1]);
    rgba[o + 2] = mix(b, rgba[o + 2]);
    rgba[o + 3] = (oa * 255.0).round().clamp(0.0, 255.0) as u8;
}

// ----------------------------------------------------------- coverage ops

#[inline]
fn stamp_max(cov: &mut [u8], w: usize, h: usize, x: i64, y: i64, v: u8) {
    if x >= 0 && y >= 0 && (x as usize) < w && (y as usize) < h {
        let i = y as usize * w + x as usize;
        if cov[i] < v {
            cov[i] = v;
        }
    }
}

fn fill_box(cov: &mut [u8], w: usize, h: usize, x0: i64, y0: i64, x1: i64, y1: i64, v: u8) {
    let x0 = x0.max(0);
    let y0 = y0.max(0);
    let x1 = x1.min(w as i64);
    let y1 = y1.min(h as i64);
    for y in y0..y1 {
        for x in x0..x1 {
            let i = y as usize * w + x as usize;
            if cov[i] < v {
                cov[i] = v;
            }
        }
    }
}

fn fill_disc(cov: &mut [u8], w: usize, h: usize, cx: f32, cy: f32, r: f32, v: u8) {
    if r <= 0.0 {
        return;
    }
    let r2 = r * r;
    let x0 = (cx - r).floor() as i64;
    let x1 = (cx + r).ceil() as i64 + 1;
    let y0 = (cy - r).floor() as i64;
    let y1 = (cy + r).ceil() as i64 + 1;
    for y in y0..y1 {
        for x in x0..x1 {
            let dx = x as f32 + 0.5 - cx;
            let dy = y as f32 + 0.5 - cy;
            if dx * dx + dy * dy <= r2 {
                stamp_max(cov, w, h, x, y, v);
            }
        }
    }
}

/// A hard-edged thick segment: square stamps along the line, like every
/// other silhouette in this pipeline — nothing here is antialiased.
fn stamp_segment(
    cov: &mut [u8],
    w: usize,
    h: usize,
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    width: f32,
    v: u8,
) {
    let half = (width * 0.5).max(0.5); // raster floor, not a design value
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt();
    if len < 0.5 {
        fill_box(
            cov, w, h,
            (x0 - half).round() as i64,
            (y0 - half).round() as i64,
            (x0 + half).round() as i64,
            (y0 + half).round() as i64,
            v,
        );
        return;
    }
    let steps = (len / half.min(1.0)).ceil() as usize;
    for s in 0..=steps {
        let t = s as f32 / steps as f32;
        let px = x0 + dx * t;
        let py = y0 + dy * t;
        let bx0 = (px - half).round() as i64;
        let by0 = (py - half).round() as i64;
        let bx1 = ((px + half).round() as i64).max(bx0 + 1);
        let by1 = ((py + half).round() as i64).max(by0 + 1);
        fill_box(cov, w, h, bx0, by0, bx1, by1, v);
    }
}

// -------------------------------------------------------------- layers

/// The eight PCB walk directions: the four axes and the four diagonals.
const DIRS: [(i64, i64); 8] = [
    (1, 0), (1, 1), (0, 1), (-1, 1), (-1, 0), (-1, -1), (0, -1), (1, -1),
];

/// splitmix64 — tiny, seedable, deterministic: the `seed` token IS the
/// pattern, and a re-bake at the same size reproduces it bit for bit.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
    fn frac(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
}

fn rasterise_traces(cov: &mut [u8], w: usize, h: usize, p: &TracesP) {
    let cell = p.cell.max(1.0);
    let cols = ((w as f32 / cell).floor() as i64).max(1);
    let rows = ((h as f32 / cell).floor() as i64).max(1);
    let line_v = (p.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    let via_v = (p.via_alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    if line_v == 0 && via_v == 0 {
        return;
    }
    // `density` is the fraction of cells that carry a trace; the walk
    // runs until it has stepped through that many cells (or gives up —
    // the guard keeps a degenerate density from spinning).
    let budget = ((cols * rows) as f32 * p.density.clamp(0.0, 1.0)) as i64;
    let mut rng = Rng(p.seed);
    let centre = |c: i64, r: i64| (c as f32 * cell + cell * 0.5, r as f32 * cell + cell * 0.5);
    let mut covered: i64 = 0;
    let mut guard = cols * rows * 4;
    while covered < budget && guard > 0 {
        guard -= 1;
        let mut cx = rng.below(cols as u64) as i64;
        let mut cy = rng.below(rows as u64) as i64;
        let mut dir = rng.below(8) as usize;
        // A run between the theme's two lengths, inclusive at both ends.
        // Ordered here rather than trusted: a theme that writes the pair
        // the wrong way round asks for a range, not for a panic.
        let (lo, hi) = (p.run_min.min(p.run_max).max(0), p.run_min.max(p.run_max).max(0));
        let len = lo + rng.below((hi - lo + 1) as u64) as i64;
        let (sx, sy) = centre(cx, cy);
        fill_disc(cov, w, h, sx, sy, p.via_radius, via_v);
        for _ in 0..len {
            // PCB bends: an occasional 45-degree turn, never a U-turn —
            // the walk turns by one eighth at a time, so it cannot double
            // back. `turn_bias` splits the bends that happen between the
            // two hands; at 0.5 the walk wanders, away from it it spirals.
            let turn = rng.frac();
            let cw = p.turn_chance * p.turn_bias.clamp(0.0, 1.0);
            if turn < cw {
                dir = (dir + 1) % 8;
            } else if turn < p.turn_chance {
                dir = (dir + 7) % 8;
            }
            let (dx, dy) = DIRS[dir];
            let (nx, ny) = (cx + dx, cy + dy);
            if nx < 0 || ny < 0 || nx >= cols || ny >= rows {
                break;
            }
            let (ax, ay) = centre(cx, cy);
            let (bx, by) = centre(nx, ny);
            stamp_segment(cov, w, h, ax, ay, bx, by, p.width, line_v);
            cx = nx;
            cy = ny;
            covered += 1;
        }
        let (ex, ey) = centre(cx, cy);
        fill_disc(cov, w, h, ex, ey, p.via_radius, via_v);
    }
}

fn rasterise_grid(cov: &mut [u8], w: usize, h: usize, p: &GridP) {
    if p.spacing < 1.0 {
        return; // sub-pixel spacing would be a solid fill, not a grid
    }
    let minor_v = (p.alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    let major_v = (p.major_alpha.clamp(0.0, 1.0) * 255.0).round() as u8;
    let lw = p.width.max(0.0);
    if lw <= 0.0 || (minor_v == 0 && major_v == 0) {
        return;
    }
    let value = |k: i64| {
        if p.major_every >= 2 && k % p.major_every as i64 == 0 {
            major_v
        } else {
            minor_v
        }
    };
    let mut k: i64 = 0;
    loop {
        let x = (k as f32 * p.spacing).round() as i64;
        if x >= w as i64 {
            break;
        }
        fill_box(cov, w, h, x, 0, x + lw.round().max(1.0) as i64, h as i64, value(k));
        k += 1;
    }
    let mut k: i64 = 0;
    loop {
        let y = (k as f32 * p.spacing).round() as i64;
        if y >= h as i64 {
            break;
        }
        fill_box(cov, w, h, 0, y, w as i64, y + lw.round().max(1.0) as i64, value(k));
        k += 1;
    }
}

fn rasterise_starfield(cov: &mut [u8], w: usize, h: usize, p: &StarfieldP) {
    let a_lo = p.alpha_min.clamp(0.0, 1.0);
    let a_hi = p.alpha_max.clamp(0.0, 1.0);
    if p.count == 0 || a_lo.max(a_hi) <= 0.0 {
        return;
    }
    let mut rng = Rng(p.seed);
    // A fixed draw order per star — x, y, size, alpha — so the field is
    // a pure function of the seed, and raising `count` adds stars to the
    // same sky instead of reshuffling it.
    for _ in 0..p.count {
        let x = rng.frac() * w as f32;
        let y = rng.frac() * h as f32;
        let d = p.size_min + (p.size_max - p.size_min) * rng.frac();
        let a = a_lo + (a_hi - a_lo) * rng.frac();
        if d >= 1.0 {
            fill_disc(cov, w, h, x, y, d * 0.5, (a * 255.0).round() as u8);
        } else {
            // Below a device pixel the size becomes alpha alone, as the
            // token's own comment states: one texel at the star's alpha
            // scaled by its squared diameter — the disc's area ratio.
            let v = (a * (d * d).clamp(0.0, 1.0) * 255.0).round() as u8;
            stamp_max(cov, w, h, x.floor() as i64, y.floor() as i64, v);
        }
    }
}

fn rasterise_scanlines(cov: &mut [u8], w: usize, h: usize, p: &ScanlinesP) {
    let period = p.period;
    let alpha = p.alpha.clamp(0.0, 1.0);
    if !(period > 0.0) || alpha <= 0.0 {
        return;
    }
    let dark = p.duty.clamp(0.0, 1.0) * period;
    // The pattern is hard-edged, but its period (~2.3 px at 1080p) does
    // not land on texel rows, so each ROW takes the exact length of dark
    // band inside it — the resample the fixed texel grid forces, not an
    // antialiasing choice. `dark_below(t)` is the total dark length in
    // [0, t); a row's coverage is the difference across it.
    let dark_below =
        |t: f32| (t / period).floor() * dark + (t - (t / period).floor() * period).min(dark);
    for y in 0..h {
        let f = dark_below(y as f32 + 1.0) - dark_below(y as f32);
        let v = (alpha * f.clamp(0.0, 1.0) * 255.0).round() as u8;
        if v > 0 {
            fill_box(cov, w, h, 0, y as i64, w as i64, y as i64 + 1, v);
        }
    }
}

fn rasterise_noise(rgba: &mut [u8], w: usize, h: usize, p: &NoiseP) {
    let alpha = p.alpha.clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    // Sub-pixel grain (0.14u is under a device pixel at 1080p) is
    // per-pixel grain: a cell can never be smaller than a texel.
    let g = p.grain.max(1.0);
    let cols = ((w as f32 / g).ceil() as usize).max(1);
    let rows = ((h as f32 / g).ceil() as usize).max(1);
    let chroma = p.chroma.clamp(0.0, 1.0);
    let mut rng = Rng(p.seed);
    for cy in 0..rows {
        let y0 = (cy as f32 * g).round() as i64;
        let y1 = (((cy + 1) as f32 * g).round() as i64).min(h as i64);
        for cx in 0..cols {
            // Four draws per cell whatever `chroma` says, so sliding the
            // token between 0 and 1 fades one grain field between
            // monochrome and per-channel instead of reshuffling it.
            let mono = rng.frac();
            let (cr, cg, cb) = (rng.frac(), rng.frac(), rng.frac());
            let mix = |c: f32| mono + (c - mono) * chroma;
            let x0 = (cx as f32 * g).round() as i64;
            let x1 = (((cx + 1) as f32 * g).round() as i64).min(w as i64);
            for y in y0..y1 {
                for x in x0..x1 {
                    let i = y as usize * w + x as usize;
                    blend_px(rgba, i, mix(cr), mix(cg), mix(cb), alpha);
                }
            }
        }
    }
}

fn rasterise_vignette(rgba: &mut [u8], w: usize, h: usize, p: &VignetteP) {
    let strength = p.strength.clamp(0.0, 1.0);
    if strength <= 0.0 {
        return;
    }
    let cx = w as f32 * 0.5;
    let cy = h as f32 * 0.5;
    let half_diag = (cx * cx + cy * cy).sqrt().max(1.0);
    let r0 = p.radius.clamp(0.0, 1.0);
    let inner2 = (r0 * half_diag) * (r0 * half_diag);
    let denom = (1.0 - r0).max(1e-3);
    for y in 0..h {
        let dy = y as f32 + 0.5 - cy;
        let dy2 = dy * dy;
        for x in 0..w {
            let dx = x as f32 + 0.5 - cx;
            let d2 = dx * dx + dy2;
            if d2 <= inner2 {
                continue; // inside the untouched radius
            }
            let t = (((d2.sqrt() / half_diag) - r0) / denom).clamp(0.0, 1.0);
            let f = match p.shape {
                Falloff::Linear => t,
                Falloff::Quad => t * t,
                // The photographic falloff the master documents: 0 at the
                // radius, 1 at the corner, cosine-squared in between.
                Falloff::Cos2 => {
                    let c = (t * std::f32::consts::FRAC_PI_2).sin();
                    c * c
                }
            };
            blend_px(rgba, y * w + x, p.color.r, p.color.g, p.color.b, strength * f);
        }
    }
}

// --------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    fn grey(a: f32) -> Color {
        Color { r: 0.5, g: 0.5, b: 0.5, a }
    }

    /// The seed IS the pattern: two bakes with the same parameters are
    /// identical bytes, and a different seed is a different field.
    #[test]
    fn the_trace_walk_is_reproducible_from_its_seed() {
        let p = |seed| Params {
            traces: Some(TracesP {
                cell: 12.0,
                density: 0.3,
                width: 1.0,
                color: grey(1.0),
                alpha: 0.4,
                via_radius: 1.5,
                via_alpha: 0.6,
                seed,
                run_min: 3,
                run_max: 14,
                turn_chance: 0.36,
                turn_bias: 0.5,
            }),
            ..Default::default()
        };
        let a = bake_params(&p(11172124), 320, 180);
        let b = bake_params(&p(11172124), 320, 180);
        let c = bake_params(&p(7), 320, 180);
        assert_eq!(a.rgba, b.rgba);
        assert_ne!(a.rgba, c.rgba, "two seeds baked one pattern");
        assert!(a.rgba.iter().any(|&v| v != 0), "the walk drew nothing");
    }

    /// Coverage compositing is max(), so a self-crossing trace can never
    /// exceed the alpha its token states.
    #[test]
    fn a_layer_never_exceeds_its_own_alpha() {
        let p = Params {
            traces: Some(TracesP {
                cell: 6.0,
                density: 1.0,
                width: 3.0,
                color: grey(1.0),
                alpha: 0.25,
                via_radius: 0.0,
                via_alpha: 0.0,
                seed: 3,
                run_min: 3,
                run_max: 14,
                turn_chance: 0.36,
                turn_bias: 0.5,
            }),
            ..Default::default()
        };
        let plate = bake_params(&p, 128, 128);
        let max_a = plate.rgba.chunks(4).map(|px| px[3]).max().unwrap();
        assert!(max_a <= (0.25f32 * 255.0).round() as u8 + 1, "alpha {max_a}");
    }

    /// The starfield is a pure function of its seed: same seed, same
    /// sky, byte for byte; a different seed is a different sky.
    #[test]
    fn the_starfield_is_reproducible_from_its_seed() {
        let p = |seed| Params {
            starfield: Some(StarfieldP {
                count: 120,
                size_min: 0.8,
                size_max: 1.6,
                alpha_min: 0.2,
                alpha_max: 0.8,
                color: grey(1.0),
                seed,
            }),
            ..Default::default()
        };
        let a = bake_params(&p(6), 320, 180);
        let b = bake_params(&p(6), 320, 180);
        let c = bake_params(&p(9), 320, 180);
        assert_eq!(a.rgba, b.rgba);
        assert_ne!(a.rgba, c.rgba, "two seeds baked one sky");
        assert!(a.rgba.iter().any(|&v| v != 0), "the sky is starless");
    }

    /// No star exceeds `alpha_max`, and a sub-pixel star dims by its
    /// squared diameter — the size token's "below a device pixel it
    /// becomes alpha alone".
    #[test]
    fn a_star_never_exceeds_alpha_max_and_shrinks_into_alpha() {
        let p = |size: f32| Params {
            starfield: Some(StarfieldP {
                count: 200,
                size_min: size,
                size_max: size,
                alpha_min: 0.6,
                alpha_max: 0.6,
                color: grey(1.0),
                seed: 42,
            }),
            ..Default::default()
        };
        let full = bake_params(&p(1.0), 160, 90);
        let max_a = full.rgba.chunks(4).map(|px| px[3]).max().unwrap();
        assert!(max_a <= (0.6f32 * 255.0).round() as u8 + 1, "alpha {max_a}");
        let tiny = bake_params(&p(0.5), 160, 90);
        let tiny_a = tiny.rgba.chunks(4).map(|px| px[3]).max().unwrap();
        let want = (0.6f32 * 0.25 * 255.0).round() as u8;
        assert!(tiny_a <= want + 1, "sub-pixel star too bright: {tiny_a}");
        assert!(tiny_a > 0, "sub-pixel star vanished instead of dimming");
    }

    /// Scanlines: every texel row of one period carries exactly the dark
    /// length that falls inside it, so a column sums to `duty` of the
    /// pattern whatever the sub-pixel phase — and never exceeds `alpha`.
    #[test]
    fn scanlines_conserve_their_duty_across_texel_rows() {
        let p = OverlayParams {
            scanlines: Some(ScanlinesP {
                period: 2.3,
                duty: 0.34,
                alpha: 0.8,
                color: grey(1.0),
            }),
            ..Default::default()
        };
        // 230 rows = 100 whole periods, so the total dark length is exact.
        let plate = bake_overlay_params(&p, 8, 230);
        let col: f32 = (0..230)
            .map(|y| plate.rgba[(y * 8) * 4 + 3] as f32 / 255.0)
            .sum();
        let want = 0.8 * 0.34 * 230.0;
        assert!((col - want).abs() < 2.0, "column sum {col}, want ~{want}");
        let max_a = plate.rgba.chunks(4).map(|px| px[3]).max().unwrap();
        assert!(max_a <= (0.8f32 * 255.0).round() as u8 + 1, "alpha {max_a}");
    }

    /// Grain is seeded like every other layer — reproducible, never over
    /// its own alpha — and `chroma = 0` is strictly monochrome.
    #[test]
    fn noise_is_seeded_and_monochrome_at_zero_chroma() {
        let p = |seed, chroma| OverlayParams {
            noise: Some(NoiseP {
                alpha: 0.5,
                grain: 1.0,
                chroma,
                seed,
            }),
            ..Default::default()
        };
        let a = bake_overlay_params(&p(3, 0.0), 64, 64);
        let b = bake_overlay_params(&p(3, 0.0), 64, 64);
        let c = bake_overlay_params(&p(4, 0.0), 64, 64);
        assert_eq!(a.rgba, b.rgba);
        assert_ne!(a.rgba, c.rgba, "two seeds baked one grain field");
        for px in a.rgba.chunks(4) {
            assert!(px[0] == px[1] && px[1] == px[2], "chroma leaked: {px:?}");
            assert!(px[3] <= (0.5f32 * 255.0).round() as u8 + 1, "alpha {:?}", px[3]);
        }
        let d = bake_overlay_params(&p(3, 1.0), 64, 64);
        assert!(
            d.rgba.chunks(4).any(|px| px[0] != px[1] || px[1] != px[2]),
            "chroma = 1 still monochrome"
        );
    }

    /// The vignette is zero inside its radius and rises to `strength`
    /// at the corner, monotonically, for every declared falloff word.
    #[test]
    fn the_vignette_rises_from_radius_to_corner() {
        for shape in [Falloff::Cos2, Falloff::Linear, Falloff::Quad] {
            let p = Params {
                vignette: Some(VignetteP {
                    strength: 0.55,
                    radius: 0.5,
                    color: grey(1.0),
                    shape,
                }),
                ..Default::default()
            };
            let plate = bake_params(&p, 200, 200);
            let a = |x: usize, y: usize| plate.rgba[(y * 200 + x) * 4 + 3];
            assert_eq!(a(100, 100), 0, "centre must stay untouched");
            let corner = a(0, 0);
            let mid = a(25, 25);
            assert!(corner as f32 >= 0.5 * 255.0 * 0.9, "corner {corner}");
            assert!(corner >= mid, "not monotone: corner {corner} < mid {mid}");
        }
    }

    /// The governing principle's own check: the embedded master ships
    /// every decor layer OFF, so the raw run grows no decoration — a
    /// plate is a thing a theme turns on, never a default. Built from
    /// the embedded text directly, so no environment variable and no
    /// user overlay on the machine running the tests can vote.
    #[test]
    fn the_default_master_ships_every_decor_layer_off() {
        use super::super::{bake, cascade::Schema, parse, resolve, BakeInput};
        let mut out = Vec::new();
        let mut src = parse::Sources::new();
        let f = src.add("default.theme", super::super::DEFAULT_THEME);
        let doc = parse::parse(&mut src, f, None, &mut out);
        let mut schema = Schema::from_default(&doc, &mut out);
        let r = resolve::resolve_default(&schema, &mut out);
        schema.adopt_kinds(&r.values);
        let rr = resolve::resolve(&schema, &schema.base_spec(), &mut out);
        let t = bake::bake(&schema, &rr, &BakeInput::default(), &mut out);
        for name in [
            "decor.enabled",
            "decor.traces.enabled",
            "decor.grid.enabled",
            "decor.starfield.enabled",
            "decor.vignette.enabled",
            "decor.scanlines.enabled",
            "decor.noise.enabled",
            "decor.ribbons.enabled",
        ] {
            let id = schema.id(name).unwrap_or_else(|| panic!("{name} not declared"));
            assert!(!t.flag(id), "{name} must ship OFF in the master");
        }
    }
}
