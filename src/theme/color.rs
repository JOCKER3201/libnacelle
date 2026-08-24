//! Colour, in the four spaces this engine actually uses.
//!
//! * **sRGB, encoded** — what an author writes (`#3FE3AE`, `rgb(63 227 174)`)
//!   and what the GPU blends on the `FormatKind::Unorm` path.
//! * **linear light** — what the engine stores between parse and bake, and the
//!   space `mix()` and `over()` model physics in (§6).
//! * **OKLab / OKLCh** — perception. `shade`, `tint`, `lum*`, `sat`, `hue`,
//!   `ramp` and `ensure` all live here (§6).
//!
//! Two rules from the specification are load-bearing and are enforced by this
//! module rather than by its callers:
//!
//! 1. **Alpha is straight, never premultiplied** ([CONFLICT 20], §6.3). The blend
//!    state is `SRC_ALPHA / ONE_MINUS_SRC_ALPHA`, so a premultiplying draw-list
//!    builder would double-apply it. Nothing here multiplies rgb by a.
//! 2. **Every OKLCh -> sRGB conversion gamut-maps by chroma reduction** (§6.2),
//!    automatically, with 22 bisection steps at fixed L and hue. Per-channel
//!    clamping is forbidden: it collapsed two of pure-green's eight data series
//!    onto the same colour. [`Color::from_oklch`] is the only public entry point
//!    and it always maps; [`Color::from_oklch_unmapped`] exists solely for the
//!    extended-range (scRGB/PQ) path, where the clamp belongs to the output.
//!
//! ### Relationship to `theme::Color`
//!
//! `nacelle::theme::Color` IS this type: with the old engine deleted,
//! `pub use color::Color` replaced the legacy seven-field engine's colour and
//! no call site changed. The five methods the program was built on (`rgb8`,
//! `from_hex`, `alpha`, `dim`, `to_array`) keep their names and semantics.
//!
//! Not in this stage: `encode.rs`. The sRGB-encode / leave-linear decision keyed
//! on the live swapchain format (§6.3) is a swapchain-format dependency, so
//! [`Color::to_srgb`] is applied by `bake.rs` for the `Unorm` path today and
//! moves to `encode.rs` when that lands.

/// A colour: four `f32` channels, **straight** (non-premultiplied) alpha.
///
/// Which space `r`/`g`/`b` are in is a property of the *value*, not the type,
/// and the pipeline stage says which: parse decodes to linear, derivation works
/// in linear and OKLab, bake encodes to sRGB for the `Unorm` swapchain.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// OKLab: perceptual lightness plus two opponent axes.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklab {
    pub l: f32,
    pub a: f32,
    pub b: f32,
    pub alpha: f32,
}

/// OKLCh: OKLab in polar form. `h` is degrees, `c` is chroma.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Oklch {
    pub l: f32,
    pub c: f32,
    pub h: f32,
    pub alpha: f32,
}

impl Color {
    pub const TRANSPARENT: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    pub const BLACK: Color = Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
    pub const WHITE: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    /// The engine's per-kind raw ink (§governing principle): what a colour
    /// token answers when no theme anywhere declares it. Deliberately dull.
    pub const GREY: Color = Color { r: 0.5, g: 0.5, b: 0.5, a: 1.0 };

    // ---------------------------------------------------------------- ctors

    pub const fn new(r: f32, g: f32, b: f32, a: f32) -> Self {
        Color { r, g, b, a }
    }

    /// Eight-bit **sRGB-encoded** components, as an author writes them.
    /// Same semantics as the legacy `Color::rgb8`, which is why it keeps the name.
    pub fn rgb8(r: u8, g: u8, b: u8) -> Self {
        Color { r: r as f32 / 255.0, g: g as f32 / 255.0, b: b as f32 / 255.0, a: 1.0 }
    }

    pub fn rgba8(r: u8, g: u8, b: u8, a: u8) -> Self {
        Color { a: a as f32 / 255.0, ..Self::rgb8(r, g, b) }
    }

    /// `#RGB` `#RGBA` `#RRGGBB` `#RRGGBBAA`, case-insensitive, short forms
    /// expanded by digit doubling (§3.2). The result is **sRGB-encoded**;
    /// the parser calls [`Color::to_linear`] on it.
    ///
    /// Rejects non-ASCII before slicing, so a six-*byte* two-character value
    /// cannot panic mid-character.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let h = hex.trim().trim_start_matches('#');
        if !h.is_ascii() || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        let d = |i: usize| -> u8 { u8::from_str_radix(&h[i..i + 1], 16).unwrap() * 17 };
        let p = |i: usize| -> u8 { u8::from_str_radix(&h[i..i + 2], 16).unwrap() };
        match h.len() {
            3 => Some(Self::rgb8(d(0), d(1), d(2))),
            4 => Some(Self::rgba8(d(0), d(1), d(2), d(3))),
            6 => Some(Self::rgb8(p(0), p(2), p(4))),
            8 => Some(Self::rgba8(p(0), p(2), p(4), p(6))),
            _ => None,
        }
    }

    /// `#RRGGBB` of an sRGB-encoded colour — for diagnostics, which quote hex.
    pub fn to_hex(self) -> String {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
        format!("#{:02X}{:02X}{:02X}", q(self.r), q(self.g), q(self.b))
    }

    // ------------------------------------------------------- transfer curve

    /// sRGB-encoded -> linear light. Alpha is untouched: it is already linear.
    pub fn to_linear(self) -> Self {
        Color { r: srgb_to_linear(self.r), g: srgb_to_linear(self.g), b: srgb_to_linear(self.b), a: self.a }
    }

    /// Linear light -> sRGB-encoded. This is `encode.rs`'s `Unorm` path (§6.3),
    /// applied by `bake.rs` until that stage exists.
    pub fn to_srgb(self) -> Self {
        Color { r: linear_to_srgb(self.r), g: linear_to_srgb(self.g), b: linear_to_srgb(self.b), a: self.a }
    }

    // ------------------------------------------------------------- channels

    /// **Sets** alpha (§6 `alpha`). Kept at this name because the whole program
    /// already calls `Color::alpha`.
    pub fn alpha(self, a: f32) -> Self {
        Color { a: a.clamp(0.0, 1.0), ..self }
    }

    /// **Multiplies** alpha (§6 `fade`) — the honest name for GTK's `alpha()`.
    pub fn fade(self, f: f32) -> Self {
        Color { a: (self.a * f.max(0.0)).clamp(0.0, 1.0), ..self }
    }

    /// Per-channel multiply. **Cut from the derivation functions** (§6.1): in
    /// sRGB it makes red vanish while green survives. Retained only because
    /// the program still calls it; authors get `lum()`, which is the same
    /// intent done in OKLCh.
    pub fn dim(self, f: f32) -> Self {
        Color { r: self.r * f, g: self.g * f, b: self.b * f, a: self.a }
    }

    pub fn to_array(self) -> [f32; 4] {
        [self.r, self.g, self.b, self.a]
    }

    pub fn is_finite(self) -> bool {
        self.r.is_finite() && self.g.is_finite() && self.b.is_finite() && self.a.is_finite()
    }

    /// Clamp into the unit cube. Used only at the very end of the pipeline, and
    /// never as a substitute for gamut mapping (§6.2).
    pub fn clamped(self) -> Self {
        Color {
            r: self.r.clamp(0.0, 1.0),
            g: self.g.clamp(0.0, 1.0),
            b: self.b.clamp(0.0, 1.0),
            a: self.a.clamp(0.0, 1.0),
        }
    }

    // --------------------------------------------------------------- OKLab

    /// Linear-light sRGB -> OKLab (Ottosson's matrices).
    pub fn to_oklab(self) -> Oklab {
        let (r, g, b) = (self.r, self.g, self.b);
        let l = 0.412_221_47 * r + 0.536_332_54 * g + 0.051_445_995 * b;
        let m = 0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b;
        let s = 0.088_302_46 * r + 0.281_718_84 * g + 0.629_978_7 * b;
        let (l_, m_, s_) = (cbrt(l), cbrt(m), cbrt(s));
        Oklab {
            l: 0.210_454_26 * l_ + 0.793_617_8 * m_ - 0.004_072_047 * s_,
            a: 1.977_998_5 * l_ - 2.428_592_2 * m_ + 0.450_593_7 * s_,
            b: 0.025_904_037 * l_ + 0.782_771_77 * m_ - 0.808_675_77 * s_,
            alpha: self.a,
        }
    }

    /// OKLab -> linear-light sRGB, **without** gamut mapping. Callers that will
    /// display the result must go through [`Color::from_oklch`].
    pub fn from_oklab_unmapped(v: Oklab) -> Self {
        let l_ = v.l + 0.396_337_78 * v.a + 0.215_803_76 * v.b;
        let m_ = v.l - 0.105_561_346 * v.a - 0.063_854_17 * v.b;
        let s_ = v.l - 0.089_484_18 * v.a - 1.291_485_5 * v.b;
        let (l, m, s) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
        Color {
            r: 4.076_741_7 * l - 3.307_711_6 * m + 0.230_969_94 * s,
            g: -1.268_438 * l + 2.609_757_4 * m - 0.341_319_38 * s,
            b: -0.004_196_086_3 * l - 0.703_418_6 * m + 1.707_614_7 * s,
            a: v.alpha,
        }
    }

    /// OKLab -> sRGB with the mandatory chroma-reduction gamut map (§6.2).
    pub fn from_oklab(v: Oklab) -> Self {
        Self::from_oklch(v.to_oklch())
    }

    pub fn to_oklch(self) -> Oklch {
        self.to_oklab().to_oklch()
    }

    /// OKLCh -> linear-light sRGB, **gamut-mapped**: L and hue are held exactly
    /// and chroma is bisected down (22 iterations) until every channel is inside
    /// [0,1]. §6.2, mandatory and automatic.
    ///
    /// The body is [`gamut_map`]'s, called with the fixed-matrix sRGB
    /// candidate — the SAME closure this function always evaluated, so a
    /// caller here sees bit-for-bit the colour it always did; only
    /// [`Color::from_oklch_in`] and [`Color::max_chroma_in`] below are new,
    /// and neither is on this call's path.
    pub fn from_oklch(v: Oklch) -> Self {
        gamut_map(v.l, v.c, v.h, v.alpha, Self::from_oklab_unmapped).0
    }

    /// [`Color::from_oklch`], generalised to an arbitrary target gamut's own
    /// primaries — the picker's gamut-boundary curve (`object::color_picker`)
    /// is what asks for this, one spoke at a time, so it can compare a wide
    /// gamut's own chroma ceiling against sRGB's at the SAME lightness and
    /// hue. `in_gamut` and the 22-step bisection are [`gamut_map`]'s, shared
    /// with [`Color::from_oklch`] and untouched; only the candidate colour —
    /// OKLab routed through [`Primaries::xyz_to_linear_rgb`] instead of the
    /// fixed sRGB matrices — differs.
    pub fn from_oklch_in(v: Oklch, p: &Primaries) -> Self {
        gamut_map(v.l, v.c, v.h, v.alpha, |ok| Self::from_oklab_unmapped_in(ok, p)).0
    }

    /// The widest chroma a gamut can hold at a given lightness and hue —
    /// [`gamut_map`]'s OWN bisection answer, which [`Color::from_oklch`] has
    /// always computed and thrown away. `CHROMA_CEILING` stands in for "as
    /// far out as the caller could possibly have asked", safely past any
    /// named space's real maximum (sRGB's is under 0.32, BT.2020's under
    /// 0.46 at their respective widest hues), so the bisection's own answer
    /// IS the gamut's boundary at this `l`/`h` and not merely an echo of
    /// whatever chroma was asked for.
    pub fn max_chroma_in(l: f32, h: f32, p: &Primaries) -> f32 {
        const CHROMA_CEILING: f32 = 0.5;
        gamut_map(l, CHROMA_CEILING, h, 1.0, |ok| Self::from_oklab_unmapped_in(ok, p)).1
    }

    /// The extended-range escape hatch (§6.2, last paragraph): where the
    /// downstream scRGB/PQ pipeline can carry out-of-[0,1] values, the clamp
    /// belongs to the output stage and not to the derivation.
    pub fn from_oklch_unmapped(v: Oklch) -> Self {
        Self::from_oklab_unmapped(v.to_oklab())
    }

    /// [`Color::from_oklab_unmapped`], routed through an arbitrary target
    /// gamut's own primaries instead of the fixed sRGB matrices — the only
    /// thing that differs between the two, per [`Primaries`]'s own doc.
    /// **Not** gamut-mapped, same promise `from_oklab_unmapped` makes: a
    /// caller that will show the result on screen goes through
    /// [`Color::from_oklch_in`] instead.
    pub fn from_oklab_unmapped_in(v: Oklab, p: &Primaries) -> Self {
        let xyz = oklab_to_xyz(v);
        let rgb = mul_mat_vec(p.xyz_to_linear_rgb(), xyz);
        Color { r: rgb[0], g: rgb[1], b: rgb[2], a: v.alpha }
    }

    // ------------------------------------------------------------ contrast

    /// WCAG 2.x relative luminance. **Input must be linear light.**
    pub fn luminance(self) -> f32 {
        0.2126 * self.r + 0.7152 * self.g + 0.0722 * self.b
    }

    /// The blend the renderer really performs: `SRC_ALPHA / ONE_MINUS_SRC_ALPHA`,
    /// straight alpha, applied to the values in whatever encoding they are in.
    ///
    /// §2.2 calls this `composite_as_rendered` and is emphatic that enforcement
    /// must measure *this* and not the linear [`Color::over`], because the GPU
    /// composites in the swapchain's own encoding. It is an internal engine
    /// routine, **not** a fifteenth derivation function: it is not authorable and
    /// does not appear in `fn-name` (§6).
    pub fn composite_as_rendered(fg: Color, bg: Color) -> Color {
        let ia = 1.0 - fg.a;
        Color {
            r: fg.r * fg.a + bg.r * ia,
            g: fg.g * fg.a + bg.g * ia,
            b: fg.b * fg.a + bg.b * ia,
            a: fg.a + bg.a * ia,
        }
    }

    /// The **authoring** composite (§6 `over`): translucent `fg` onto opaque
    /// `bg`, in linear light, returning an opaque colour. This models physics;
    /// `composite_as_rendered` models the hardware. Two questions, two answers.
    pub fn over(fg: Color, bg: Color) -> Color {
        let a = fg.a + bg.a * (1.0 - fg.a);
        if a <= 0.0 {
            return Color::TRANSPARENT;
        }
        let f = |x: f32, y: f32| (x * fg.a + y * bg.a * (1.0 - fg.a)) / a;
        Color { r: f(fg.r, bg.r), g: f(fg.g, bg.g), b: f(fg.b, bg.b), a: 1.0 }
    }

    /// WCAG 2.x contrast ratio, 1.0 ..= 21.0. **Both inputs must be linear.**
    pub fn wcag_contrast(a: Color, b: Color) -> f32 {
        let (x, y) = (a.luminance(), b.luminance());
        let (hi, lo) = if x >= y { (x, y) } else { (y, x) };
        (hi + 0.05) / (lo + 0.05)
    }

    /// APCA Lc (SAPC/APCA-W3 0.1.9 G-4g), **advisory only** (§4.4 pass D 4,
    /// [CONFLICT 6]). Computed for every text pair, reported, never enforced.
    ///
    /// **Both inputs must be sRGB-encoded** — APCA's own transfer exponent is
    /// 2.4 applied to the encoded value, which is not the sRGB decode curve.
    /// Sign carries polarity: positive is dark-on-light, negative light-on-dark.
    pub fn apca_lc(text_srgb: Color, bg_srgb: Color) -> f32 {
        const TRC: f32 = 2.4;
        const BLK_THRS: f32 = 0.022;
        const BLK_CLMP: f32 = 1.414;
        const DELTA_Y_MIN: f32 = 0.0005;
        const LO_CLIP: f32 = 0.1;
        let y = |c: Color| {
            let f = |v: f32| v.clamp(0.0, 1.0).powf(TRC);
            let y = 0.212_672_9 * f(c.r) + 0.715_152_2 * f(c.g) + 0.072_175 * f(c.b);
            if y < BLK_THRS { y + (BLK_THRS - y).powf(BLK_CLMP) } else { y }
        };
        let (yt, yb) = (y(text_srgb), y(bg_srgb));
        if (yb - yt).abs() < DELTA_Y_MIN {
            return 0.0;
        }
        let sapc = if yb > yt {
            (yb.powf(0.56) - yt.powf(0.57)) * 1.14
        } else {
            (yb.powf(0.65) - yt.powf(0.62)) * 1.14
        };
        if sapc.abs() < LO_CLIP {
            0.0
        } else if sapc > 0.0 {
            (sapc - 0.027) * 100.0
        } else {
            (sapc + 0.027) * 100.0
        }
    }

    /// OKLab ΔE — plain Euclidean distance in OKLab, which is what §4.4's
    /// separation floors (0.09 .. 0.115) are calibrated against.
    pub fn delta_e_ok(a: Color, b: Color) -> f32 {
        let (x, y) = (a.to_oklab(), b.to_oklab());
        ((x.l - y.l).powi(2) + (x.a - y.a).powi(2) + (x.b - y.b).powi(2)).sqrt()
    }
}

impl Oklab {
    pub fn to_oklch(self) -> Oklch {
        let c = (self.a * self.a + self.b * self.b).sqrt();
        let h = if c < 1e-7 { 0.0 } else { self.b.atan2(self.a).to_degrees().rem_euclid(360.0) };
        Oklch { l: self.l, c, h, alpha: self.alpha }
    }
}

impl Oklch {
    pub fn to_oklab(self) -> Oklab {
        let r = self.h.to_radians();
        Oklab { l: self.l, a: self.c * r.cos(), b: self.c * r.sin(), alpha: self.alpha }
    }
}

// ------------------------------------------------------------------ helpers

fn in_gamut(c: Color) -> bool {
    const E: f32 = 1e-4;
    (-E..=1.0 + E).contains(&c.r) && (-E..=1.0 + E).contains(&c.g) && (-E..=1.0 + E).contains(&c.b)
}

/// The chroma-reduction bisection §6.2 states, factored out so
/// [`Color::from_oklch`] and [`Color::from_oklch_in`] run the SAME 22
/// steps against the SAME `in_gamut` test — a candidate colour is the
/// only thing that differs between an sRGB picker and one asking after a
/// wider gamut, so `to_linear` is a closure and every line below is
/// §6.2's original body, unchanged since before this function had a name.
///
/// Returns the mapped, clamped colour AND the chroma the bisection landed
/// on. [`Color::from_oklch`] wants only the first half of that pair — it
/// is what it has always returned — and [`Color::max_chroma_in`] wants
/// only the second, which is why both ride home together instead of one
/// being computed twice under two names.
fn gamut_map(l: f32, c0: f32, h: f32, alpha: f32, to_linear: impl Fn(Oklab) -> Color) -> (Color, f32) {
    let l = l.clamp(0.0, 1.0);
    let c0 = c0.max(0.0);
    let at = |c: f32| to_linear(Oklch { l, c, h, alpha }.to_oklab());
    let top = at(c0);
    if in_gamut(top) {
        return (Color { a: alpha.clamp(0.0, 1.0), ..top.clamped() }, c0);
    }
    // C = 0 is always in gamut for L in [0,1]: the achromatic axis is the
    // SAME OKLab point (a = b = 0) whichever gamut's own primaries
    // `to_linear` routes it through, and every named gamut here shares the
    // D65 white point that axis walks toward — so the bisection is total
    // for every space this module builds a [`Primaries`] for, not only sRGB.
    let (mut lo, mut hi) = (0.0f32, c0);
    for _ in 0..22 {
        let mid = 0.5 * (lo + hi);
        if in_gamut(at(mid)) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (Color { a: alpha.clamp(0.0, 1.0), ..at(lo).clamped() }, lo)
}

// ------------------------------------------------------------ wide gamuts

/// A target RGB space's own chromaticity primaries (CIE 1931 xy) and white
/// point — what [`Color::from_oklch`]'s fixed sRGB matrices generalise
/// into once the picker has to draw a boundary for a gamut that is not
/// sRGB. All four named spaces below share the D65 white point, which is
/// what lets [`gamut_map`]'s "chroma zero is always in gamut" argument
/// hold for every one of them without restating it per space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Primaries {
    pub r: (f32, f32),
    pub g: (f32, f32),
    pub b: (f32, f32),
    pub white: (f32, f32),
}

impl Primaries {
    pub const SRGB: Primaries =
        Primaries { r: (0.6400, 0.3300), g: (0.3000, 0.6000), b: (0.1500, 0.0600), white: (0.3127, 0.3290) };
    pub const DISPLAY_P3: Primaries =
        Primaries { r: (0.6800, 0.3200), g: (0.2650, 0.6900), b: (0.1500, 0.0600), white: (0.3127, 0.3290) };
    pub const ADOBE_RGB: Primaries =
        Primaries { r: (0.6400, 0.3300), g: (0.2100, 0.7100), b: (0.1500, 0.0600), white: (0.3127, 0.3290) };
    pub const BT2020: Primaries =
        Primaries { r: (0.7080, 0.2920), g: (0.1700, 0.7970), b: (0.1310, 0.0460), white: (0.3127, 0.3290) };

    /// This space's own CIE XYZ (D65) -> linear-light RGB matrix, built
    /// from its primaries and white point the standard way (Bruce
    /// Lindbloom's derivation, "RGB/XYZ Matrices"): each primary's own XYZ
    /// is its chromaticity's `(x/y, 1, (1-x-y)/y)`, the three are scaled so
    /// together they reproduce the white point exactly, and the RGB -> XYZ
    /// matrix that gives is inverted.
    ///
    /// Computed on every call rather than cached: a 3x3 invert is a dozen
    /// multiplies, this is asked for once per spoke of the picker's curve
    /// (tens of times a frame, not thousands), and a cache keyed on which
    /// of four `const` values was asked for would be more code than the
    /// arithmetic it is saving.
    fn xyz_to_linear_rgb(&self) -> [[f32; 3]; 3] {
        let col = |xy: (f32, f32)| -> [f32; 3] {
            let (x, y) = xy;
            [x / y, 1.0, (1.0 - x - y) / y]
        };
        let (cr, cg, cb) = (col(self.r), col(self.g), col(self.b));
        // Columns are the three primaries' own XYZ, unscaled.
        let unscaled = [[cr[0], cg[0], cb[0]], [cr[1], cg[1], cb[1]], [cr[2], cg[2], cb[2]]];
        let s = mul_mat_vec(invert3(unscaled), col(self.white));
        let rgb_to_xyz = [
            [unscaled[0][0] * s[0], unscaled[0][1] * s[1], unscaled[0][2] * s[2]],
            [unscaled[1][0] * s[0], unscaled[1][1] * s[1], unscaled[1][2] * s[2]],
            [unscaled[2][0] * s[0], unscaled[2][1] * s[1], unscaled[2][2] * s[2]],
        ];
        invert3(rgb_to_xyz)
    }

    /// A chromaticity `xy` — any space's own primary, not necessarily
    /// `self`'s — decomposed as a WEIGHT TRIPLE in **sRGB's own** r/g/b
    /// basis: `[x/y, 1, (1-x-y)/y]` (that chromaticity's own XYZ at Y = 1)
    /// run through [`Primaries::SRGB`]'s [`xyz_to_linear_rgb`], unclamped —
    /// a component going negative is exactly `xy` lying outside sRGB's own
    /// triangle, which is the question this function exists to answer.
    ///
    /// [`xyz_to_linear_rgb`]: Primaries::xyz_to_linear_rgb
    ///
    /// WHAT THIS IS FOR: the colour picker's gamut-boundary triangle
    /// (`object::color_picker::draw`) places a wide gamut's own primaries
    /// on a wheel whose hue and saturation ARE `rgb_to_hsv` of an
    /// sRGB-encoded colour — hue 0/120/240° are sRGB's own red/green/blue
    /// by that function's construction, and saturation 1 is sRGB's own
    /// edge (the module header there, "HSV AND NOT OKLCh", is the ruling
    /// this leans on). Running a target primary's `xy` through THIS matrix
    /// and then through THAT SAME `rgb_to_hsv` asks one coherent question —
    /// "what hue and saturation would this chromaticity read as, in the
    /// terms the wheel already speaks?" — with the wheel's own law, not a
    /// second one invented for the curve alone.
    ///
    /// TWO ANCHORS FALL OUT OF THE SAME ARITHMETIC, NOT A SPECIAL CASE FOR
    /// EITHER — READ IN `rgb_to_hsv` TERMS, WHICH IS THE ONLY TERM THAT
    /// MATTERS HERE, since hue and saturation are what a caller does with
    /// this triple next and both are invariant to a positive uniform
    /// rescale. Feed sRGB's own red back in: this function passes the
    /// chromaticity through at a FIXED `Y = 1`, not through the same
    /// per-primary scale [`xyz_to_linear_rgb`]'s own matrix was built with
    /// (the `s` in that method's body), so the weight triple back is
    /// `[k, 0, 0]` for some `k > 0` and not literally `[1, 0, 0]` — but
    /// `rgb_to_hsv`'s hue ignores an overall scale entirely and its
    /// saturation is `(max - min) / max`, which is exactly 1 whenever the
    /// other two components are exactly 0 regardless of what `k` is. So
    /// the READING is hue 0°, saturation 1 — the wheel's own rim — even
    /// though the raw triple is not literally `[1, 0, 0]`.
    /// Feed in any space's white point (every [`Primaries`] constant here
    /// shares sRGB's D65) and the weights come back EXACTLY `[1, 1, 1]`,
    /// no scale ambiguity at all: [`xyz_to_linear_rgb`] is built (its own
    /// doc, above) so that unit RGB reproduces the white point's OWN
    /// `Y = 1` chromaticity exactly — the definition of "white" in a
    /// coherent additive space, and the one case where this function's own
    /// `Y = 1` convention matches the matrix's internal scale by
    /// construction. `rgb_to_hsv` of an equal triple is saturation 0, the
    /// wheel's own centre. Neither anchor is asserted by this function;
    /// both are the one decomposition landing where a coherent RGB
    /// system's own definitions put it, one exactly and one up to a scale
    /// that the reading downstream never sees.
    pub fn in_srgb_basis(xy: (f32, f32)) -> [f32; 3] {
        let (x, y) = xy;
        let y = y.max(1e-6);
        let xyz = [x / y, 1.0, (1.0 - x - y) / y];
        mul_mat_vec(Primaries::SRGB.xyz_to_linear_rgb(), xyz)
    }
}

/// The sRGB (D65) linear-light -> CIE XYZ matrix (IEC 61966-2-1 / Bruce
/// Lindbloom's published constants) — the one place this module names CIE
/// XYZ with a hand-typed constant. It is used for exactly one thing,
/// [`lms_to_xyz`], composed there with the SAME sRGB<->LMS matrix
/// [`Color::to_oklab`] and [`Color::from_oklab_unmapped`] already use and
/// are already tested against (`oklab_anchors`), so a digit mistyped here
/// cannot silently agree with the rest of this module: it would move
/// `from_oklch_in`'s answer for `Primaries::SRGB` away from
/// `from_oklch`'s, which `an_srgb_target_matches_the_fixed_sRGB_path`
/// below checks directly, over many lightnesses, hues and chromas at once.
const SRGB_TO_XYZ: [[f32; 3]; 3] = [
    [0.4124564, 0.3575761, 0.1804375],
    [0.2126729, 0.7151522, 0.0721750],
    [0.0193339, 0.1191920, 0.9503041],
];

/// The same sRGB -> LMS matrix [`Color::to_oklab`] opens with, named here
/// so [`lms_to_xyz`] can be built by composing it with [`SRGB_TO_XYZ`]
/// instead of introducing a second, independently sourced constant for
/// Ottosson's own XYZ<->LMS matrix — one fewer place for this module to
/// misquote a published number.
const SRGB_TO_LMS: [[f32; 3]; 3] = [
    [0.412_221_47, 0.536_332_54, 0.051_445_995],
    [0.211_903_5, 0.680_699_5, 0.107_396_96],
    [0.088_302_46, 0.281_718_84, 0.629_978_7],
];

/// LMS -> CIE XYZ: `SRGB_TO_XYZ * SRGB_TO_LMS^-1`, i.e. XYZ -> sRGB_lin ->
/// LMS run backwards. [`oklab_to_xyz`]'s last step.
fn lms_to_xyz() -> [[f32; 3]; 3] {
    mul_mat_mat(SRGB_TO_XYZ, invert3(SRGB_TO_LMS))
}

/// OKLab -> CIE XYZ (D65): the fixed half of Ottosson's construction,
/// space-agnostic because CIE XYZ is the space every RGB gamut is defined
/// relative TO. OKLab -> LMS' -> LMS is the same three lines
/// [`Color::from_oklab_unmapped`] opens with (copied rather than shared,
/// since that function's own contract — direct to linear sRGB, no XYZ
/// stop in between — is untouched by this addition); LMS -> XYZ is
/// [`lms_to_xyz`]. Every target gamut's own [`Primaries::xyz_to_linear_rgb`]
/// takes it from here.
fn oklab_to_xyz(v: Oklab) -> [f32; 3] {
    let l_ = v.l + 0.396_337_78 * v.a + 0.215_803_76 * v.b;
    let m_ = v.l - 0.105_561_346 * v.a - 0.063_854_17 * v.b;
    let s_ = v.l - 0.089_484_18 * v.a - 1.291_485_5 * v.b;
    let lms = [l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_];
    mul_mat_vec(lms_to_xyz(), lms)
}

fn mul_mat_vec(m: [[f32; 3]; 3], v: [f32; 3]) -> [f32; 3] {
    [
        m[0][0] * v[0] + m[0][1] * v[1] + m[0][2] * v[2],
        m[1][0] * v[0] + m[1][1] * v[1] + m[1][2] * v[2],
        m[2][0] * v[0] + m[2][1] * v[1] + m[2][2] * v[2],
    ]
}

fn mul_mat_mat(a: [[f32; 3]; 3], b: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let mut out = [[0.0f32; 3]; 3];
    for i in 0..3 {
        for j in 0..3 {
            out[i][j] = a[i][0] * b[0][j] + a[i][1] * b[1][j] + a[i][2] * b[2][j];
        }
    }
    out
}

fn invert3(m: [[f32; 3]; 3]) -> [[f32; 3]; 3] {
    let det = m[0][0] * (m[1][1] * m[2][2] - m[1][2] * m[2][1])
        - m[0][1] * (m[1][0] * m[2][2] - m[1][2] * m[2][0])
        + m[0][2] * (m[1][0] * m[2][1] - m[1][1] * m[2][0]);
    let d = 1.0 / det;
    [
        [
            (m[1][1] * m[2][2] - m[1][2] * m[2][1]) * d,
            (m[0][2] * m[2][1] - m[0][1] * m[2][2]) * d,
            (m[0][1] * m[1][2] - m[0][2] * m[1][1]) * d,
        ],
        [
            (m[1][2] * m[2][0] - m[1][0] * m[2][2]) * d,
            (m[0][0] * m[2][2] - m[0][2] * m[2][0]) * d,
            (m[0][2] * m[1][0] - m[0][0] * m[1][2]) * d,
        ],
        [
            (m[1][0] * m[2][1] - m[1][1] * m[2][0]) * d,
            (m[0][1] * m[2][0] - m[0][0] * m[2][1]) * d,
            (m[0][0] * m[1][1] - m[0][1] * m[1][0]) * d,
        ],
    ]
}

fn cbrt(v: f32) -> f32 {
    if v < 0.0 { -(-v).powf(1.0 / 3.0) } else { v.powf(1.0 / 3.0) }
}

pub fn srgb_to_linear(v: f32) -> f32 {
    if v <= 0.040_45 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
}

pub fn linear_to_srgb(v: f32) -> f32 {
    if v <= 0.003_130_8 { v * 12.92 } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f32, b: f32, eps: f32) -> bool {
        (a - b).abs() <= eps
    }

    /// Within `n` 8-bit steps on every channel.
    fn near_hex(c: Color, hex: &str, n: i32) -> bool {
        let want = Color::from_hex(hex).unwrap();
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
        (q(c.r) - q(want.r)).abs() <= n
            && (q(c.g) - q(want.g)).abs() <= n
            && (q(c.b) - q(want.b)).abs() <= n
    }

    #[test]
    fn hex_forms_and_the_multibyte_trap() {
        assert_eq!(Color::from_hex("#3FE3AE"), Some(Color::rgb8(0x3F, 0xE3, 0xAE)));
        assert_eq!(Color::from_hex("3fe3ae"), Some(Color::rgb8(0x3F, 0xE3, 0xAE)));
        // digit doubling
        assert_eq!(Color::from_hex("#abc"), Color::from_hex("#aabbcc"));
        assert_eq!(Color::from_hex("#abcd"), Color::from_hex("#aabbccdd"));
        // straight alpha: #RRGGBBAA does not scale rgb
        let c = Color::from_hex("#3FE3AE80").unwrap();
        assert!(approx(c.r, 0x3F as f32 / 255.0, 1e-6));
        assert!(approx(c.a, 128.0 / 255.0, 1e-6));
        // six BYTES, two chars: must not panic on a mid-character slice
        assert!(Color::from_hex("#\u{20ac}\u{20ac}").is_none());
        assert!(Color::from_hex("#zzzzzz").is_none());
        assert!(Color::from_hex("#fffff").is_none());
    }

    #[test]
    fn transfer_curve_round_trips() {
        for i in 0..=255u32 {
            let v = i as f32 / 255.0;
            assert!(approx(linear_to_srgb(srgb_to_linear(v)), v, 1e-5), "at {v}");
        }
    }

    #[test]
    fn oklab_anchors() {
        let w = Color::WHITE.to_oklab();
        assert!(approx(w.l, 1.0, 1e-3), "white L = {}", w.l);
        assert!(approx(w.a, 0.0, 1e-3) && approx(w.b, 0.0, 1e-3));
        let k = Color::BLACK.to_oklab();
        assert!(approx(k.l, 0.0, 1e-4));
        // round trip through OKLCh for an in-gamut colour
        let src = Color::from_hex("#3FE3AE").unwrap().to_linear();
        let back = Color::from_oklch(src.to_oklch());
        assert!(Color::delta_e_ok(src, back) < 1e-3, "ΔE {}", Color::delta_e_ok(src, back));
    }

    #[test]
    fn gamut_map_reduces_chroma_and_never_clamps_per_channel() {
        // Wildly out-of-gamut: L held, hue held, chroma bisected down (§6.2).
        let want = Oklch { l: 0.75, c: 0.40, h: 150.0, alpha: 1.0 };
        let got = Color::from_oklch(want);
        let back = got.to_oklch();
        assert!(approx(back.l, want.l, 2e-3), "L moved: {} -> {}", want.l, back.l);
        let dh = (back.h - want.h).abs();
        assert!(dh < 1.0, "hue moved {dh} deg");
        assert!(back.c < want.c, "chroma not reduced: {} -> {}", want.c, back.c);
        assert!(back.c > 0.05, "chroma over-reduced to {}", back.c);
        // A per-channel clamp would have moved the hue; this is the exact
        // failure §6.2 forbids.
        let naive = Color::from_oklch_unmapped(want).clamped();
        assert!((naive.to_oklch().h - want.h).abs() > dh);
    }

    #[test]
    fn wcag_extremes_and_a_known_pair() {
        assert!(approx(Color::wcag_contrast(Color::WHITE, Color::BLACK), 21.0, 1e-3));
        assert!(approx(Color::wcag_contrast(Color::WHITE, Color::WHITE), 1.0, 1e-6));
        // the azure #29B6F6 chip: WCAG says dark text (§6 contrast_on).
        let chip = Color::from_hex("#29B6F6").unwrap().to_linear();
        let vs_black = Color::wcag_contrast(chip, Color::BLACK);
        let vs_white = Color::wcag_contrast(chip, Color::WHITE);
        assert!(vs_black > vs_white, "black {vs_black} white {vs_white}");
    }

    #[test]
    fn apca_polarity_and_direction() {
        let black = Color::BLACK;
        let white = Color::WHITE;
        // light text on dark bg is the reverse polarity: negative Lc.
        assert!(Color::apca_lc(white, black) < -90.0);
        // dark on light is positive.
        assert!(Color::apca_lc(black, white) > 90.0);
        // equal colours produce no signal at all.
        assert_eq!(Color::apca_lc(white, white), 0.0);
    }

    #[test]
    fn composite_as_rendered_differs_from_linear_over() {
        // §4.4: `#15201B / 0.82` over `#0B1310` is #131E1A as rendered and
        // #141F1A when composited in linear light. The two must not agree, or
        // enforcing on the wrong one would be harmless and the spec pointless.
        let fg_s = Color::from_hex("#15201B").unwrap().alpha(0.82);
        let bg_s = Color::from_hex("#0B1310").unwrap();
        let rendered = Color::composite_as_rendered(fg_s, bg_s);
        let authored = Color::over(fg_s.to_linear(), bg_s.to_linear()).to_srgb();
        assert_ne!(rendered.to_hex(), authored.to_hex());
        // The spec quotes #131E1A and #141F1A; both land within one 8-bit step
        // of that, the difference being how it rounded 0.82. What matters — and
        // what §4.4 is built on — is that the two answers are NOT the same.
        assert!(near_hex(rendered, "#131E1A", 1), "as-rendered {}", rendered.to_hex());
        assert!(near_hex(authored, "#141F1A", 1), "authored {}", authored.to_hex());
    }

    #[test]
    fn over_returns_opaque_and_transparent_is_absorbing() {
        let fg = Color::new(1.0, 0.0, 0.0, 0.0);
        let bg = Color::new(0.0, 0.0, 1.0, 1.0);
        let out = Color::over(fg, bg);
        assert_eq!(out.a, 1.0);
        assert!(approx(out.b, 1.0, 1e-6) && approx(out.r, 0.0, 1e-6));
    }

    #[test]
    fn an_srgb_target_matches_the_fixed_srgb_path() {
        //! `from_oklch_in`/`max_chroma_in` reach linear-light RGB through
        //! CIE XYZ and a primaries-built matrix; `from_oklch` never leaves
        //! the fixed sRGB matrices [`Color::to_oklab`] and
        //! [`Color::from_oklab_unmapped`] already use. Asking the XYZ road
        //! for `Primaries::SRGB` is the same question two different ways —
        //! so if [`SRGB_TO_XYZ`] or the LMS<->XYZ composition it feeds is
        //! wrong, this is where the two roads' answers come apart, over a
        //! spread of lightnesses, hues and chromas at once.
        for l in [0.15f32, 0.35, 0.55, 0.75, 0.92] {
            for h in [0.0f32, 47.0, 130.0, 210.0, 300.0] {
                for c in [0.02f32, 0.08, 0.15, 0.30] {
                    let want = Color::from_oklch(Oklch { l, c, h, alpha: 1.0 });
                    let got = Color::from_oklch_in(Oklch { l, c, h, alpha: 1.0 }, &Primaries::SRGB);
                    for (a, b, ch) in [(got.r, want.r, 'r'), (got.g, want.g, 'g'), (got.b, want.b, 'b')] {
                        assert!(
                            approx(a, b, 3e-3),
                            "l={l} h={h} c={c} channel {ch}: {a} vs {b}"
                        );
                    }
                }
            }
        }
        // The achromatic axis lands on the same grey either road, at both
        // ends of L — the `gamut_map` argument that chroma zero is always
        // in gamut leans on this.
        for l in [0.0f32, 0.5, 1.0] {
            let want = Color::from_oklch(Oklch { l, c: 0.0, h: 0.0, alpha: 1.0 });
            let got = Color::from_oklch_in(Oklch { l, c: 0.0, h: 0.0, alpha: 1.0 }, &Primaries::SRGB);
            assert!(approx(got.r, want.r, 1e-3) && approx(got.g, want.g, 1e-3) && approx(got.b, want.b, 1e-3));
        }
    }

    #[test]
    fn wider_gamuts_hold_more_chroma_at_a_green_that_shows_it() {
        //! P3's own primaries are famously wider than sRGB's at saturated
        //! green — the textbook example every gamut-comparison chart
        //! reaches for — so [`Color::max_chroma_in`] must answer a BIGGER
        //! number for [`Primaries::DISPLAY_P3`] than for [`Primaries::SRGB`]
        //! at the same lightness and hue, or the picker's boundary curve
        //! would draw a wide gamut's own rim INSIDE sRGB's, which is
        //! backwards.
        let (l, h) = (0.87, 142.0); // a bright, saturated green
        let srgb = Color::max_chroma_in(l, h, &Primaries::SRGB);
        let p3 = Color::max_chroma_in(l, h, &Primaries::DISPLAY_P3);
        assert!(p3 > srgb + 0.02, "P3 chroma {p3} is not wider than sRGB's {srgb} at l={l} h={h}");
        // And BT.2020, wider again at the same green.
        let bt2020 = Color::max_chroma_in(l, h, &Primaries::BT2020);
        assert!(bt2020 > p3, "BT.2020 chroma {bt2020} is not wider than P3's {p3}");
    }

    #[test]
    fn max_chroma_in_is_what_the_bisection_lands_on() {
        //! Not a duplicate of `gamut_map_reduces_chroma_and_never_clamps_per_channel`:
        //! that test reads the MAPPED COLOUR back through `to_oklch`, which
        //! is a round trip through OKLab's own cbrt/cube pair and picks up
        //! its own rounding. `max_chroma_in` is the bisection's raw answer,
        //! asked of directly and checked against `from_oklch_in` reporting
        //! a colour AT that exact chroma as in-gamut and just past it as
        //! not — the two, together, are what a picker deriving a curve's
        //! RADIUS from this number needs to be true.
        let (l, h) = (0.7, 30.0);
        let c = Color::max_chroma_in(l, h, &Primaries::SRGB);
        assert!(c > 0.0 && c < 0.5, "chroma {c} outside a plausible sRGB range");
        let at_c = Color::from_oklch_in(Oklch { l, c, h, alpha: 1.0 }, &Primaries::SRGB);
        assert!(in_gamut(at_c), "the bisection's own answer must itself be in gamut");
        let just_over = Color::from_oklab_unmapped_in(
            Oklch { l, c: c + 0.01, h, alpha: 1.0 }.to_oklab(),
            &Primaries::SRGB,
        );
        assert!(!in_gamut(just_over), "a chroma just past the bisection's answer should not be");
    }

    #[test]
    fn in_srgb_basis_anchors_a_primary_at_the_rim_and_white_at_the_centre() {
        //! [`Primaries::in_srgb_basis`]'s own doc claims two anchors fall
        //! out of ONE decomposition rather than being asserted specially:
        //! sRGB's own primaries come back PURELY ALONG THEIR OWN AXIS
        //! (some positive `k` in their own slot, the other two exactly 0
        //! — not literally `[1, 0, 0]`, since this function's fixed
        //! `Y = 1` is not the matrix's own per-primary scale, and the doc
        //! is explicit about that), and any shared white point is unit RGB
        //! (`[1, 1, 1]`, exactly, no scale ambiguity) by construction of
        //! `xyz_to_linear_rgb`. Checked here directly, in weight-triple
        //! terms, before the picker's own test asks the same question one
        //! step further along in hue/saturation terms — where an overall
        //! scale on the primaries' own axis stops mattering at all.
        let red = Primaries::in_srgb_basis(Primaries::SRGB.r);
        assert!(red[0] > 0.5, "{red:?}");
        assert!(approx(red[1] / red[0], 0.0, 1e-4) && approx(red[2] / red[0], 0.0, 1e-4), "{red:?}");
        let green = Primaries::in_srgb_basis(Primaries::SRGB.g);
        assert!(green[1] > 0.5, "{green:?}");
        assert!(approx(green[0] / green[1], 0.0, 1e-4) && approx(green[2] / green[1], 0.0, 1e-4), "{green:?}");
        let blue = Primaries::in_srgb_basis(Primaries::SRGB.b);
        assert!(blue[2] > 0.5, "{blue:?}");
        assert!(approx(blue[0] / blue[2], 0.0, 1e-4) && approx(blue[1] / blue[2], 0.0, 1e-4), "{blue:?}");
        // Display P3 shares sRGB's own blue primary exactly — one more
        // check that a DIFFERENT space's primary, when it happens to
        // coincide with sRGB's, decomposes along the same axis rather
        // than picking up noise from going the long way round.
        let p3_blue = Primaries::in_srgb_basis(Primaries::DISPLAY_P3.b);
        assert!(p3_blue[2] > 0.5, "{p3_blue:?}");
        assert!(approx(p3_blue[0] / p3_blue[2], 0.0, 1e-4) && approx(p3_blue[1] / p3_blue[2], 0.0, 1e-4), "{p3_blue:?}");
        // Every named space here shares sRGB's own D65, so every one of
        // their white points decomposes to unit RGB, not only sRGB's own.
        for p in [Primaries::SRGB, Primaries::DISPLAY_P3, Primaries::ADOBE_RGB, Primaries::BT2020] {
            let w = Primaries::in_srgb_basis(p.white);
            assert!(
                approx(w[0], 1.0, 1e-3) && approx(w[1], 1.0, 1e-3) && approx(w[2], 1.0, 1e-3),
                "{p:?}'s white point: {w:?}"
            );
        }
        // A primary genuinely OUTSIDE sRGB's own triangle carries a
        // negative weight on the component it left behind — Display P3's
        // red reaches further than sRGB's own, so decomposing it in
        // sRGB's basis must borrow negatively from green or blue.
        let p3_red = Primaries::in_srgb_basis(Primaries::DISPLAY_P3.r);
        assert!(
            p3_red[1] < 0.0 || p3_red[2] < 0.0,
            "Display P3's red reads as fully inside sRGB's triangle: {p3_red:?}"
        );
    }

    #[test]
    fn alpha_sets_fade_multiplies() {
        let c = Color::WHITE.alpha(0.5);
        assert_eq!(c.alpha(0.25).a, 0.25);
        assert_eq!(c.fade(0.5).a, 0.25);
        assert_eq!(c.fade(4.0).a, 1.0); // clamped
        // and neither touches rgb — straight alpha, never premultiplied
        assert_eq!(c.fade(0.5).r, 1.0);
    }
}
