//! Colour picker: a hue/saturation WHEEL, a value bar beside it, the
//! chosen colour as a patch, that same colour written out in one of six
//! notations, and two grids of ready-made colours.
//! (The field has moved twice in one day. It was a rectangle from
//! 2026-08-18: hue across, value down, the bar answering for saturation.
//! 2026-08-23 first traded those two axes — the field took value, the
//! bar took saturation — and then, later the same day, gave the field up
//! entirely for a WHEEL: hue swept round the rim, saturation out from
//! the centre, and the bar handed BACK to value, because saturation now
//! has a home of its own and a straight bar was never it. The names
//! below are written for the wheel already in force.)
//!
//! THE FIELD IS A SQUARE, since 2026-08-23: `layout_with`'s box was
//! already a square (the wheel note directly below explains why it is a
//! DISC drawn inside one), so the frame the field wears no longer forces
//! a circle — it is the ordinary square/chamfered corner every other part
//! of this control wears (`shape`, `frame`, `handle`), and the wheel's
//! own triangulation is widened by one more band per wedge, from the rim
//! it used to stop at out to the square's own edge, so nothing between
//! the inscribed circle and the field's own corners is left unpainted.
//! [`square_reach`] is the closed form for how far that band reaches at a
//! given angle, and it is flat-coloured at the rim's own colour, because
//! `wheel_pick`'s overshoot rule already established that saturation
//! pins past r = 1 — there is nothing to interpolate.
//!
//! A SQUARE AND NOT A CIRCLE IS ALSO WHY A WIDE GAMUT HAS SOMEWHERE TO
//! DRAW ITSELF. When `draw()` is handed a [`GamutSpace`], it draws that
//! space's own gamut boundary as what a real RGB gamut's boundary
//! actually is: THREE STRAIGHT EDGES between its three primaries, fixed
//! in shape and answering to no lightness at all — never a per-hue
//! sampled curve (an earlier version of this drew exactly that, one
//! spoke per hue wedge, each spoke's radius a chroma ratio taken at that
//! spoke's OWN OKLCh lightness — which wobbled with `v` and was never a
//! gamut's real shape to begin with, since chromaticity does not depend
//! on how bright the colour built from it is). `theme::color::Primaries::
//! in_srgb_basis` decomposes each of the caller's three primaries' own
//! CIE xy into the SAME sRGB-relative terms the wheel's own hue and
//! saturation already are (its own doc has the argument in full: sRGB's
//! own primaries decompose purely along their own axis, which `rgb_to_hsv`
//! reads as hue 0/120/240° at saturation exactly 1 — the wheel's own rim —
//! regardless of the scale that decomposition lands on, and any shared
//! white point decomposes to unit RGB exactly, which is saturation 0, the
//! wheel's own centre — both by construction, not by a special case in
//! this file); `rgb_to_hsv` turns
//! each decomposition into an (hue, radius) pair the SAME way it always
//! has, `wheel_point_unclamped` places it, and `DrawList::polyline` draws
//! the closed three-point triangle — past the rim, toward the square's
//! own corners, wherever a primary reaches further than sRGB's own.
//!
//! THIS IS A STYLISED TRIANGLE, NOT A LITERAL CIE 1931 xy DIAGRAM
//! TRACED ONTO THE FIELD — the same trade the wheel itself already makes
//! (hue as angle, saturation as radius, next paragraph below) rather than
//! plotting sRGB-encoded colour in its own literal x/y plane. An edge
//! between two mapped primaries is a straight line ON SCREEN, anchored
//! exactly at both ends; it is not claimed to retrace every intermediate
//! chromaticity of the real gamut edge along the way, because the
//! wheel's own polar coordinates are not a straightness-preserving map of
//! CIE xy and nothing short of drawing the field itself in literal xy
//! (a different, much larger change to what the field's axes mean) could
//! make that claim honestly. What this triangle DOES guarantee, and what
//! the earlier curve did not: exactly three straight edges, matching a
//! real RGB gamut's own edge count, and a shape that cannot move when
//! `v` does, because no lightness enters the computation anywhere.
//!
//! WHY AN OBJECT AND NOT A PAGE OF SLIDERS. Until 2026-08-18 a colour in
//! the theme editor was three sliders — brightness, saturation, hue —
//! and thirteen colours were thirty-nine rows in which the one thing you
//! could not see was the colour. Three numbers are the SHAPE of the
//! value; they are not a way of ANSWERING the question "what colour is
//! this". The owner looked at a picker and asked for one, "dopasowane do
//! projektu": so the behaviour is the behaviour every picker has had
//! since the eighties, and the geometry, the colours, the corner
//! language and the grid of ready-made colours are this theme's, read
//! from `[picker]` like everything else in this toolkit.
//!
//! WHY A WHEEL AND NOT A SQUARE. A rectangular hue-by-saturation field
//! puts the one axis that reads as "how much colour is this" along a
//! single edge — full saturation is a sliver a pixel wide at the top,
//! and a handle spends most of its travel in the muddy two-thirds
//! nearest the bottom. A disk puts saturation on the RADIUS instead: the
//! whole rim is "as saturated as this hue gets", at every hue at once,
//! and the grey axis every hue's spoke actually passes through is one
//! POINT at the centre rather than a line along an edge — which is also
//! why a drag can lose its way onto grey without losing which hue it
//! came from: the centre is a point and not a line, and [`wheel_pick`]'s
//! dead zone is where that is written down.
//!
//! THE WHEEL IS EXACT FOR THE SAME REASON THE OLD FIELD WAS, IN TWO
//! DIMENSIONS INSTEAD OF ONE. HSV's own definition is affine in
//! saturation at fixed hue and value:
//!
//! ```text
//! rgb(h, s, v) = rgb(h, 1, v) · s + grey(v) · (1 − s)
//! ```
//!
//! — so along any one spoke out from the centre, the colour is an exact
//! linear function of radius, and [`crate::draw::DrawList::quad_c`]'s Gouraud
//! interpolation reproduces a linear function exactly on any
//! triangulation (its own doc says so). The one approximation left is
//! ACROSS a wedge: `hsv_to_rgb` is only piecewise-affine in hue, with
//! kinks at the six 60° sector boundaries, so the wheel rounds
//! `picker.hue_stops` up to a multiple of six and lands a wedge boundary
//! on every one of them — [`wheel_tessellation`] states the rule and its
//! test measures it. It is the same trade the old field made between a
//! smooth ramp and `rect_grad`'s banding, read in two dimensions instead
//! of one, and it is drawn with [`crate::draw::DrawList::fan_c`] and
//! [`crate::draw::DrawList::quad_c`], never [`crate::draw::DrawList::rect_grad`]: the wheel's own
//! triangulation IS its silhouette, so there is nothing to clip a circle
//! out of a rectangle for.
//!
//! THE VALUE BAR IS EXACT FOR THE SAME REASON THE OLD SATURATION ONE
//! WAS, read at the OTHER axis: `rgb(h, s, v) = v · rgb(h, s, 1)`, black
//! mixed with the wheel's own current hue-and-saturation by `v` — so it
//! is two stops, that colour at the top and black at the bottom, and
//! never the grey saturation's zero would have drawn: value's own zero
//! is black, and that is the point of the axis.
//!
//! HSV AND NOT OKLCh FOR THE WHEEL, and the owner ruled on this on
//! 2026-08-16 about the sliders this replaces: brightness at 100 % must
//! be the FULL BRIGHTNESS OF THE HUE — red lands on #FF0000 — and never
//! white. OKLCh's lightness at 1.0 is white by definition, which reads
//! as a broken control. OKLCh is on the list of NOTATIONS, where it
//! belongs and where it is mandatory: the theme file writes `oklch(...)`,
//! so an author who cannot type one cannot move a value between the
//! editor and their own file. "Mandatory" is a claim about the SCREEN as
//! well as about the list, and it was false for a day: the readout sat
//! in the column beside the field, which leaves it 215 px on a
//! 1080-line screen, and the notation needs 224 — so the one notation
//! this file calls compulsory was the one it cut off, at every size.
//! [`Layout`] carries the measurements and where the readout went.
//!
//! THE TRAP THIS FILE IS WRITTEN AROUND. The colour a picker holds is
//! **sRGB-ENCODED** — that is what a bake hands back, what hex spells and
//! what the wheel's and the bar's arithmetic above are true of. OKLCh is
//! defined over **LINEAR LIGHT**. Every crossing therefore decodes on the
//! way in ([`Color::to_linear`]) and encodes on the way back
//! ([`Color::to_srgb`]), and neither step is optional. The one time this
//! program mixed the two it did not merely mis-report: the editor seeded
//! itself from what it had just written, so the accent's lightness
//! climbed 0.8200 → 0.8904 → 0.9413 → 0.9715 over successive visits with
//! every slider at rest. `the_notation_survives_twenty_round_trips` is
//! that measurement turned into a test.

use super::focus_ring;
use crate::corner::Cuts;
use crate::draw::Corner;
use crate::focus::{Caps, FocusId};
use crate::theme::color::Oklch;
use crate::theme::{self, Color, TokenId};
use crate::{ui, Ctx, Rect};
use std::sync::OnceLock;

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// The engine's colour, in the draw list's clothes.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

// ------------------------------------------------------------- notation

/// How the chosen colour is written out, and read back in.
///
/// SIX, AND EVERY ONE OF THEM EARNS ITS PLACE:
///
/// * [`Format::Argb`] — `#AARRGGBB`, the owner's default (2026-08-18).
///   Eight digits, alpha first, so **the alpha lives in the format** and
///   there is no separate opacity knob anywhere near this control.
/// * [`Format::Rgba`] — `#RRGGBBAA`, what CSS and every drawing program
///   spells. The same eight digits in a different order, which is
///   precisely why both are offered: a colour carried between two
///   programs that disagree about where alpha goes is the commonest way
///   to arrive at a transparent red instead of a dark one.
/// * [`Format::Oklch`] — `oklch(L, C, H / A)`, **mandatory**: it is what
///   a `.theme` file is full of, so it is the only notation in which a
///   value typed here and a value read out of the author's own file are
///   the same text.
/// * [`Format::Hsv`] — the field's own coordinates. The three numbers
///   under the field ARE where the two handles stand, so this is the
///   notation in which the control explains itself.
/// * [`Format::Hsl`] — what web tooling means by "hue, saturation,
///   lightness", and it is NOT [`Format::Hsv`]: at 100 % HSL is white
///   and HSV is the full hue. Offering one and calling it the other is
///   how a picker lies to half its users.
/// * [`Format::Dec`] — four plain numbers 0..255, which is what
///   screenshot tools, eyedroppers and image editors report.
///
/// NO CMYK: the owner withdrew it on 2026-08-18. It describes ink on
/// paper and there is no press at the end of this pipeline.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Argb,
    /// Six hex digits, no alpha byte at all — not even a dropped one. The
    /// three byte notations otherwise carry alpha unconditionally
    /// (`write`'s own head note), which is right for a colour a slider
    /// can fade and wrong for one that never had a fade to begin with: a
    /// control whose transparency lives entirely in a SEPARATE knob (a
    /// picker fed by `Settings::tone_bed`, say, with an OPACITY slider of
    /// its own) has nothing honest to put in an alpha byte, and RGB is
    /// the notation that says so by construction rather than by writing
    /// `FF` and hoping nobody reads it as a promise.
    Rgb,
    Rgba,
    Oklch,
    Hsv,
    Hsl,
    Dec,
}

impl Format {
    /// The offer, in the order the control steps through it. ARGB stands
    /// first because it is the default and a cycler that starts anywhere
    /// else would make the default the hardest one to get back to.
    pub const ALL: [Format; 7] = [
        Format::Argb,
        Format::Rgb,
        Format::Rgba,
        Format::Oklch,
        Format::Hsv,
        Format::Hsl,
        Format::Dec,
    ];

    /// The word on the button. Upper case like every other word this
    /// window puts on a plate; the CASE is the type role's business
    /// (`type.<role>.case`), and this is the word itself.
    pub fn word(self) -> &'static str {
        match self {
            Format::Argb => "ARGB",
            Format::Rgb => "RGB",
            Format::Rgba => "RGBA",
            Format::Oklch => "OKLCH",
            Format::Hsv => "HSV",
            Format::Hsl => "HSL",
            Format::Dec => "DEC",
        }
    }

    /// The next notation round the ring.
    pub fn next(self) -> Format {
        let i = Format::ALL.iter().position(|f| *f == self).unwrap_or(0);
        Format::ALL[(i + 1) % Format::ALL.len()]
    }
}

/// HSV -> sRGB-encoded RGB. `h` in degrees, `s` and `v` in 0..1.
///
/// The value line at the head of this file is this function read
/// sideways, and the field's two draw calls stand on it.
pub fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (f32, f32, f32) {
    let h = h.rem_euclid(360.0) / 60.0;
    let s = s.clamp(0.0, 1.0);
    let v = v.clamp(0.0, 1.0);
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let m = v - c;
    (r + m, g + m, b + m)
}

/// sRGB-encoded RGB -> HSV. Hue is 0 on the grey axis, where hue does not
/// exist; the picker never asks this of a colour it is already holding,
/// exactly so that a drag onto the axis does not forget which hue it came
/// from ([`Picker::hsv`]).
pub fn rgb_to_hsv(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let d = max - min;
    let h = if d == 0.0 {
        0.0
    } else if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    let s = if max == 0.0 { 0.0 } else { d / max };
    (h, s, max)
}

/// The colour the bar shows at fraction `fy` down from its top, for a
/// wheel standing at hue `h` and saturation `s`.
///
/// The one statement of what the BAR means. Its predecessor `field_colour`
/// was tested against the black overlay a rectangular field composited
/// over a hue ramp; this is that same law moved onto value, which is
/// what the bar draws now that the field is a wheel and saturation is
/// its radius. The drawing does not call it — [`crate::draw::DrawList::rect_grad`]
/// draws the bar from its own two stops — and that is the point: this
/// is the definition the drawing is tested against.
pub fn bar_colour(h: f32, s: f32, fy: f32) -> (f32, f32, f32) {
    hsv_to_rgb(h, s, 1.0 - fy.clamp(0.0, 1.0))
}

/// Where the wheel's handle sits for (hue°, sat), as a 0..1 fraction of
/// the wheel's own bounding square, top-left origin — the same contract
/// [`Picker::field_at`] has always had, so `draw()`'s handle-placement
/// arithmetic did not have to change when the field did.
///
/// Hue 0° sits at 3 o'clock (`+x`) and increasing hue sweeps toward `+y`
/// — which reads as CLOCKWISE on screen, since screen y is down. That is
/// an arbitrary choice and now a fixed one; what matters is only that it
/// is the exact inverse of [`wheel_pick`] outside the dead zone at the
/// centre, which is what round-tripping through the two needs and all
/// they promise.
pub fn wheel_point(hue_deg: f32, sat: f32) -> (f32, f32) {
    wheel_point_unclamped(hue_deg, sat.clamp(0.0, 1.0))
}

/// [`wheel_point`] without the saturation clamp — for the gamut-boundary
/// triangle's three vertices, each a primary's own chromaticity decomposed
/// into sRGB-relative hue and saturation
/// ([`theme::color::Primaries::in_srgb_basis`]), which legitimately
/// exceeds 1.0 wherever that primary reaches past the wheel's inscribed
/// rim toward the square field's own corners. Every OTHER caller wants the
/// clamp — a handle's own position is never past the wheel it stands on —
/// which is why this is a second function and not `wheel_point` with an
/// argument nobody but the triangle would ever pass differently.
pub fn wheel_point_unclamped(hue_deg: f32, sat: f32) -> (f32, f32) {
    let r = hue_deg.to_radians();
    (0.5 + 0.5 * sat * r.cos(), 0.5 + 0.5 * sat * r.sin())
}

/// How far a ray at angle `theta` (radians, the wheel's own hue angle)
/// travels from the centre of a square before it leaves the square,
/// expressed as a multiple of the inscribed circle's own radius: 1.0 at
/// an edge's midpoint, `√2` at a corner. The closed form for a unit
/// square centred on the origin is `1 / max(|cos θ|, |sin θ|)` — whichever
/// axis the ray is closer to end-on is the one whose edge it reaches
/// first — and it is exact, not an approximation the way the wheel's own
/// wedges are: a square has four straight edges and this is their
/// equation, not a tessellation of them.
pub fn square_reach(theta: f32) -> f32 {
    let (c, s) = (theta.cos().abs(), theta.sin().abs());
    1.0 / c.max(s).max(1e-6)
}

/// The inverse of [`wheel_point`]: a press or a drag at local-fractional
/// `(fx, fy)` becomes `(hue°, sat)`. `keep_hue` is the caller's own
/// current hue, read only inside the dead zone at the centre.
///
/// CLAMPED, NEVER REJECTED. `fx`/`fy` are not pre-clamped to 0..1 by any
/// caller in this file — a drag can wander arbitrarily far outside the
/// wheel's own bounding square — so the radius `r_norm` this computes can
/// exceed 1.0 for an overshoot. `atan2` is exact for any nonzero
/// `(dx, dy)` however large, so hue keeps tracking a drag that has left
/// the wheel altogether; saturation is simply pinned at its rim with
/// `.min(1.0)`. That is the standard picker feel — a drag that overshoots
/// keeps steering by angle and stops climbing in vividness — and it costs
/// no branch beyond the one `.min` already visible below.
///
/// THE DEAD ZONE IS THE WHEEL'S OWN VERSION OF "A DRAG ONTO THE GREY AXIS
/// KEEPS THE HUE IT CAME FROM". Below `r_norm = 1e-4`, `atan2(0, 0)` has
/// no angle to give, so hue is left exactly where the caller already had
/// it rather than asked of a coordinate pair with nothing left to answer
/// with — the same POLICY the old rectangular field's bottom edge used to
/// carry, now living at one point instead of along one edge.
pub fn wheel_pick(fx: f32, fy: f32, keep_hue: f32) -> (f32, f32) {
    let (dx, dy) = (fx - 0.5, fy - 0.5);
    let r_norm = (dx * dx + dy * dy).sqrt() / 0.5;
    let sat = r_norm.min(1.0);
    let hue = if r_norm > 1e-4 {
        dy.atan2(dx).to_degrees().rem_euclid(360.0)
    } else {
        keep_hue
    };
    (hue, sat)
}

/// How many wedges and how many rings the wheel is cut into, from
/// `picker.hue_stops` (`n`, already clamped to 2..64 by the caller).
///
/// WEDGES ARE `n` ROUNDED UP TO A MULTIPLE OF SIX, FLOORED AT SIX, so a
/// wedge boundary always lands on one of `hsv_to_rgb`'s six 60° sector
/// kinks — the one place the cross term between hue and saturation
/// inside a wedge is exactly zero, which removes a whole source of
/// tessellation error for the cost of rounding a number that was already
/// approximate. RINGS ARE A THIRD OF THE WEDGE COUNT, CLAMPED 4..16:
/// radial interpolation inside a ring is exact regardless of how many
/// there are (HSV is affine in saturation at fixed hue), so the ring
/// count only has to be enough that the one approximation left — the
/// cross term across a wedge's own angular width — shrinks with both the
/// radial step and the angular one, without spending vertices nobody
/// asked for on an axis that was already exact.
fn wheel_tessellation(n: usize) -> (usize, usize) {
    let wedges = n.max(1).div_ceil(6) * 6;
    let rings = (wedges / 3).clamp(4, 16);
    (wedges, rings)
}

fn q8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// The chosen colour as text.
///
/// ALPHA IS SPELLED THE WAY THE NOTATION SPELLS NUMBERS, which is one
/// rule and not six: the hex notations carry it as a BYTE because bytes
/// are what they are made of, `DEC` likewise, and the three functional
/// notations carry it as a FRACTION after a slash, because that is what
/// `oklch(... / a)` means in the theme language this program already
/// parses. A picker that wrote `0.50` in one place and `128` in another
/// for the same channel would be teaching two dialects.
///
/// TWO DECIMALS ON THE ANGLES AND THE PERCENTAGES, AND THAT IS A
/// CORRECTNESS DECISION AND NOT A TASTE ONE. Whole numbers cost more
/// than they look: one degree of hue moves a channel by up to 255/60 ≈
/// 4.3, and one percent of value moves it by 2.55, so `hsv(166, 78, 89)`
/// read back is a VISIBLY different colour to the one it was written
/// from — measured at 0.042 in OKLCh lightness, against 0.000 for the
/// hex notations. Two decimals put the same trip under 1e-4, which is
/// past what eight-bit output can hold. The three hex-and-byte notations
/// need no such choice: a byte IS their resolution.
pub fn write(c: Color, f: Format) -> String {
    let (r, g, b, a) = (q8(c.r), q8(c.g), q8(c.b), q8(c.a));
    match f {
        Format::Argb => format!("#{a:02X}{r:02X}{g:02X}{b:02X}"),
        // No `a` in the format string at all — the byte is computed above
        // for every other arm's sake and simply unused here, which is the
        // whole difference from RGBA.
        Format::Rgb => format!("#{r:02X}{g:02X}{b:02X}"),
        Format::Rgba => format!("#{r:02X}{g:02X}{b:02X}{a:02X}"),
        // The theme's own spelling, called and not copied: one program,
        // one way of writing a colour into a file.
        Format::Oklch => theme::edit::oklch_literal(c.to_linear().to_oklch()),
        Format::Hsv => {
            let (h, s, v) = rgb_to_hsv(c.r, c.g, c.b);
            with_alpha(format!("hsv({:.2}, {:.2}, {:.2}", h, s * 100.0, v * 100.0), c.a)
        }
        Format::Hsl => {
            let (h, s, l) = rgb_to_hsl(c.r, c.g, c.b);
            with_alpha(format!("hsl({:.2}, {:.2}, {:.2}", h, s * 100.0, l * 100.0), c.a)
        }
        Format::Dec => format!("{r}, {g}, {b}, {a}"),
    }
}

fn with_alpha(mut head: String, a: f32) -> String {
    if a < 1.0 {
        head.push_str(&format!(" / {:.3}", a));
    }
    head.push(')');
    head
}

/// sRGB-encoded RGB -> HSL, the web's triple. Separate from
/// [`rgb_to_hsv`] because they are different quantities that share two
/// names — see [`Format::Hsl`].
pub fn rgb_to_hsl(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    let s = if d == 0.0 { 0.0 } else { d / (1.0 - (2.0 * l - 1.0).abs()).max(1e-6) };
    let (h, _, _) = rgb_to_hsv(r, g, b);
    (h, s.clamp(0.0, 1.0), l)
}

/// HSL -> sRGB-encoded RGB, the inverse of [`rgb_to_hsl`].
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let s = s.clamp(0.0, 1.0);
    let l = l.clamp(0.0, 1.0);
    // HSL and HSV meet through v = l + s·min(l, 1−l): the same cone read
    // from its middle instead of its tip.
    let v = l + s * l.min(1.0 - l);
    let sv = if v <= 0.0 { 0.0 } else { 2.0 * (1.0 - l / v) };
    hsv_to_rgb(h, sv, v)
}

/// Text back to a colour, or `None` when the text is not that notation.
///
/// FORGIVING ABOUT PUNCTUATION, STRICT ABOUT MEANING. The name in front
/// (`oklch(`, `hsv(`), the `#`, the parentheses, the commas and the
/// spaces are all optional, because a person pasting a value from a file
/// or a screenshot should not have to tidy it first; what is NOT
/// optional is the count and the order of the numbers, because those are
/// the notation. Six hex digits with no alpha mean an OPAQUE colour —
/// the reading every tool in the world agrees on.
pub fn parse(text: &str, f: Format) -> Option<Color> {
    let t = text.trim();
    match f {
        Format::Argb | Format::Rgb | Format::Rgba => {
            let h: String = t
                .trim_start_matches('#')
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            if !h.is_ascii() || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                return None;
            }
            let p = |i: usize| u8::from_str_radix(&h[i..i + 2], 16).ok();
            match (h.len(), f) {
                // Six digits is opaque under every hex notation, RGB's
                // own included — the reading every tool in the world
                // agrees on (this function's own head note).
                (6, _) => Some(Color::rgb8(p(0)?, p(2)?, p(4)?)),
                (8, Format::Argb) => Some(Color::rgba8(p(2)?, p(4)?, p(6)?, p(0)?)),
                // Eight digits pasted into an RGB field is a value from a
                // notation that carries alpha; reading it RGBA-style —
                // the same fallback the catch-all below gives Argb — is
                // forgiving about which one, not silent about the byte.
                (8, _) => Some(Color::rgba8(p(0)?, p(2)?, p(4)?, p(6)?)),
                _ => None,
            }
        }
        Format::Oklch => {
            let (n, a) = numbers(t, "oklch")?;
            if n.len() != 3 {
                return None;
            }
            // The decode-free direction: OKLCh IS linear light, so the
            // encode happens on the way OUT and nowhere else.
            Some(
                Color::from_oklch(Oklch { l: n[0], c: n[1], h: n[2], alpha: a.unwrap_or(1.0) })
                    .to_srgb(),
            )
        }
        Format::Hsv | Format::Hsl => {
            let (n, a) = numbers(t, if f == Format::Hsv { "hsv" } else { "hsl" })?;
            if n.len() != 3 {
                return None;
            }
            let (r, g, b) = if f == Format::Hsv {
                hsv_to_rgb(n[0], n[1] / 100.0, n[2] / 100.0)
            } else {
                hsl_to_rgb(n[0], n[1] / 100.0, n[2] / 100.0)
            };
            Some(Color { r, g, b, a: a.unwrap_or(1.0).clamp(0.0, 1.0) })
        }
        Format::Dec => {
            let (n, _) = numbers(t, "")?;
            let b8 = |v: f32| (v / 255.0).clamp(0.0, 1.0);
            match n.len() {
                3 => Some(Color { r: b8(n[0]), g: b8(n[1]), b: b8(n[2]), a: 1.0 }),
                4 => Some(Color { r: b8(n[0]), g: b8(n[1]), b: b8(n[2]), a: b8(n[3]) }),
                _ => None,
            }
        }
    }
}

/// The numbers of a functional notation, and the alpha behind the slash
/// if one was written. The leading name is accepted and ignored — it says
/// which notation, and the CALLER already knows which notation it asked
/// for; refusing `hsv(...)` typed into an HSL field would be refusing to
/// read three numbers that are right there.
fn numbers(t: &str, name: &str) -> Option<(Vec<f32>, Option<f32>)> {
    let body = match t.find('(') {
        Some(i) if !name.is_empty() => t[i + 1..].trim_end_matches(')'),
        _ => t.trim_end_matches(')'),
    };
    let (head, tail) = match body.split_once('/') {
        Some((h, a)) => (h, a.trim().parse::<f32>().ok()),
        None => (body, None),
    };
    let n: Vec<f32> = head
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|s| !s.is_empty())
        .map(|s| s.trim_end_matches('%').parse::<f32>())
        .collect::<Result<_, _>>()
        .ok()?;
    if n.is_empty() {
        return None;
    }
    Some((n, tail))
}

// --------------------------------------------------------------- the space

/// The active output space's own boundary — a plain value the CALLER
/// supplies, the same way `custom: &[Color]` and `id_of: impl Fn(Part) ->
/// FocusId` already reach this control. This crate has no dependency on a
/// host application's own notion of a colour space (`SpaceRange`,
/// `ColorConf` and the like, in `nacelle-desktop`'s case); whatever those
/// types resolve to becomes a struct before it crosses into this one.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GamutSpace {
    /// The space's own primaries — what the gamut-boundary curve is
    /// measured against (`theme::color::Primaries`).
    pub primaries: theme::color::Primaries,
}

// --------------------------------------------------------------- the model

/// What the control holds between frames.
///
/// THE HUE IS KEPT, NOT DERIVED. A colour on the grey axis has no hue —
/// `rgb_to_hsv` answers 0 there, and it has to — so a picker that
/// recomputed its coordinates from the colour every frame would swing the
/// field's handle to red the moment a drag reached the bottom edge, and
/// leave it there when the drag came back. The field's two numbers and
/// the bar's one are therefore the state, and the COLOUR is what they
/// answer.
#[derive(Clone, Debug, PartialEq)]
pub struct Picker {
    /// Hue in degrees, saturation and value 0..1 — the field's own
    /// coordinates.
    hsv: [f32; 3],
    /// The alpha channel, which is part of the colour and not a knob
    /// beside it (the owner's decision of 2026-08-18).
    alpha: f32,
    /// Which notation the text side is written in.
    pub format: Format,
}

impl Picker {
    /// A picker opened on a colour.
    pub fn of(c: Color) -> Picker {
        let mut p = Picker { hsv: [0.0, 0.0, 0.0], alpha: 1.0, format: Format::Argb };
        p.set_colour(c);
        p
    }

    /// A picker opened on nothing in particular — `component.picker.rest`,
    /// the colour a theme says its picker holds before anything seeds it.
    ///
    /// A CONSTRUCTOR AND NOT A CONSTANT, and the difference is the whole
    /// rule this toolkit is built on. A picker is built long before its
    /// owner knows what colour to point it at, and whatever it holds in
    /// the meantime is ON THE SCREEN for as long as it takes somebody to
    /// reach the control. That is a look, and a look is a token. The
    /// settings window opened its picker on a grey written into Rust
    /// until 2026-08-18, defended as neutrality — but a neutral is a
    /// choice too, and the master already had a name for the one it
    /// wanted (`@palette.neutral`).
    pub fn at_rest() -> Picker {
        static REST: OnceLock<TokenId> = OnceLock::new();
        Picker::of(col(theme::resolved().color(tok(&REST, "component.picker.rest"))))
    }

    /// The chosen colour, sRGB-encoded, alpha included.
    pub fn colour(&self) -> Color {
        let (r, g, b) = hsv_to_rgb(self.hsv[0], self.hsv[1], self.hsv[2]);
        Color { r, g, b, a: self.alpha }
    }

    /// Moves the picker onto a colour. The hue is taken from the colour
    /// EXCEPT on the grey axis, where the colour has none to give and the
    /// handle keeps the hue it was already standing on.
    pub fn set_colour(&mut self, c: Color) {
        let (h, s, v) = rgb_to_hsv(c.r.clamp(0.0, 1.0), c.g.clamp(0.0, 1.0), c.b.clamp(0.0, 1.0));
        if s > 0.0 {
            self.hsv[0] = h;
        }
        self.hsv[1] = s;
        self.hsv[2] = v;
        self.alpha = c.a.clamp(0.0, 1.0);
    }

    /// The chosen colour in the space the theme file writes.
    ///
    /// The decode is part of the trip and never an optimisation to skip;
    /// the head of this file records what skipping it cost.
    pub fn oklch(&self) -> Oklch {
        self.colour().to_linear().to_oklch()
    }

    /// The way back in, with the same discipline.
    pub fn set_oklch(&mut self, v: Oklch) {
        self.set_colour(Color::from_oklch(v).to_srgb());
    }

    /// The hue the field is standing on, on its own — what
    /// `a_drag_onto_the_grey_axis_keeps_the_hue_it_came_from` asks the
    /// wheel about directly rather than reverse-engineering it out of a
    /// 2-D point. THE HUE IS KEPT, NOT DERIVED, is this module's header's
    /// own claim, and a struct that makes good on that claim ought to
    /// say so through an accessor and not only through [`field_at`]'s
    /// arithmetic.
    ///
    /// [`field_at`]: Picker::field_at
    pub fn hue(&self) -> f32 {
        self.hsv[0]
    }

    /// The wheel's handle, 0..1 from the wheel's own top-left — exactly
    /// [`wheel_point`], which is this method's whole body. Kept as a
    /// method and not inlined at the one call site because `draw()` and
    /// this file's tests both need the same two numbers from the same
    /// two of `hsv`'s three, and a caller reading `field_at()` should not
    /// have to know which two those are.
    pub fn field_at(&self) -> (f32, f32) {
        wheel_point(self.hsv[0], self.hsv[1])
    }

    /// The bar's handle, 0..1 from its top (bright) to its bottom
    /// (black) — VALUE, the name's original meaning, restored the same
    /// day it left: saturation now lives in the wheel's own radius, so
    /// the bar has exactly one axis left to answer for and this is it.
    pub fn value_at(&self) -> f32 {
        1.0 - self.hsv[2]
    }

    /// A press or a drag inside the wheel: hue from angle, saturation
    /// from radius, exactly [`wheel_pick`] — the field's own hue is what
    /// the dead zone at the centre falls back to, so a hand that lands
    /// exactly on grey does not swing the handle to whatever `atan2`
    /// makes of `(0, 0)`.
    pub fn pick_field(&mut self, fx: f32, fy: f32) {
        let (h, s) = wheel_pick(fx, fy, self.hsv[0]);
        self.hsv[0] = h;
        self.hsv[1] = s;
    }

    /// A press or a drag along the bar: VALUE from y.
    pub fn pick_value(&mut self, fy: f32) {
        self.hsv[2] = 1.0 - fy.clamp(0.0, 1.0);
    }

    /// The colour as text, in the notation in force.
    pub fn text(&self) -> String {
        write(self.colour(), self.format)
    }

    /// Text typed by a person. `false` means it was not read and NOTHING
    /// moved — a picker that fell back to black on a typo would destroy
    /// the value it was showing.
    pub fn set_text(&mut self, s: &str) -> bool {
        match parse(s, self.format) {
            Some(c) => {
                self.set_colour(c);
                true
            }
            None => false,
        }
    }

    /// Steps to the next notation. THE COLOUR DOES NOT MOVE: this changes
    /// how the value is spelled and nothing else, which is what
    /// `changing_the_notation_changes_the_spelling_and_not_the_colour`
    /// pins down.
    pub fn cycle_format(&mut self) {
        self.format = self.format.next();
    }
}

// -------------------------------------------------------------- geometry

/// Where every part of the control stands, in the caller's coordinates.
///
/// THE READOUT IS A STRIP UNDER THE WHOLE CONTROL, and that is arithmetic
/// rather than taste. The longest thing this control ever writes is
/// `oklch(0.8200, 0.1531, 166.22 / 0.502)`, which measures 224 px at
/// `type.data`. It used to sit in the column beside the field, and that
/// column is narrower than the value at every size the settings window
/// has: the picker's band measures 730 px on a 1080-line screen, 596 on
/// a 900 and 497 on a 768, which left the readout 215, 158 and 117 px.
/// So the ONE notation this file calls mandatory — the one a `.theme`
/// file is written in — was cut off on every screen there is. Across the
/// full band the same three sizes give it 662, 528 and 429 px.
///
/// It costs no height: the strip is exactly the row the right-hand
/// column no longer carries, and the control is as tall as it was.
#[derive(Clone, Debug)]
pub struct Layout {
    /// Hue across, value down.
    pub field: Rect,
    /// Saturation, vivid at the top, grey at the bottom.
    pub value: Rect,
    /// The chosen colour over the transparency checker.
    pub patch: Rect,
    /// The plate that names the notation and steps to the next, at the
    /// head of the readout strip.
    pub format: Rect,
    /// The colour written out, across the rest of that strip.
    pub text: Rect,
    /// The theme's own ready-made colours.
    pub base: Vec<Rect>,
    /// The caller's own, and the cell that banks the current colour.
    pub custom: Vec<Rect>,
    pub add: Rect,
}

/// What one part of the control answers to.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Part {
    Field,
    Value,
    Format,
    Text,
    Base(usize),
    Custom(usize),
    Add,
}

/// The numbers `[picker]` states, read once per call and passed around
/// rather than re-read: a layout and the drawing that follows it must not
/// be able to disagree because the theme was re-baked between them.
struct Metrics {
    gap: f32,
    pad_x: f32,
    field_h: f32,
    value_w: f32,
    field_w_frac: f32,
    patch_h: f32,
    row_h: f32,
    format_w: f32,
    swatch: f32,
    swatch_gap: f32,
    cols: usize,
    base_count: usize,
}

impl Metrics {
    fn read() -> Metrics {
        static GAP: OnceLock<TokenId> = OnceLock::new();
        static FIELD_H: OnceLock<TokenId> = OnceLock::new();
        static VALUE_W: OnceLock<TokenId> = OnceLock::new();
        static FRAC: OnceLock<TokenId> = OnceLock::new();
        static PATCH_H: OnceLock<TokenId> = OnceLock::new();
        static ROW_H: OnceLock<TokenId> = OnceLock::new();
        static FORMAT_W: OnceLock<TokenId> = OnceLock::new();
        static SWATCH: OnceLock<TokenId> = OnceLock::new();
        static SWATCH_GAP: OnceLock<TokenId> = OnceLock::new();
        static COLS: OnceLock<TokenId> = OnceLock::new();
        static BASE_N: OnceLock<TokenId> = OnceLock::new();
        static PAD_X: OnceLock<TokenId> = OnceLock::new();
        let t = theme::resolved();
        Metrics {
            gap: t.px(tok(&GAP, "picker.gap")),
            pad_x: t.px(tok(&PAD_X, "picker.pad_x")),
            field_h: t.px(tok(&FIELD_H, "picker.field_h")),
            value_w: t.px(tok(&VALUE_W, "picker.value_w")),
            field_w_frac: t.px(tok(&FRAC, "picker.field_w_frac")).clamp(0.1, 0.9),
            patch_h: t.px(tok(&PATCH_H, "picker.patch_h")),
            row_h: t.px(tok(&ROW_H, "picker.row_h")),
            format_w: t.px(tok(&FORMAT_W, "picker.format_w")),
            swatch: t.px(tok(&SWATCH, "picker.swatch")),
            swatch_gap: t.px(tok(&SWATCH_GAP, "picker.swatch_gap")),
            // Counts, floored at one: a grid nought cells wide is a
            // division by zero, and a theme is a file a person edits.
            cols: (t.px(tok(&COLS, "picker.swatch_cols")).round() as usize).max(1),
            base_count: offered(t.px(tok(&BASE_N, "picker.base_count"))),
        }
    }
}

/// How far the reader LOOKS for `picker.base.N`, and the reason it is a
/// search bound and not an answer: the tokens are a numbered series, and
/// a numbered series is a promise about numbering that only a reader can
/// keep (`[glow]`'s own warning). What the grid offers is [`base_ids`] —
/// however many of these the build actually declares — and never this.
const BASE_SEARCH: usize = 24;

/// The ids of the ready-made colours, in numbering order, stopping at the
/// first number this build does not declare.
///
/// STOPPING AND NOT SKIPPING. Skipping a hole would renumber every cell
/// behind it, so `base.5` would answer to a press aimed at `base.4` and
/// the grid a theme wrote would not be the grid it saw.
fn base_ids() -> &'static [TokenId] {
    static IDS: OnceLock<Vec<TokenId>> = OnceLock::new();
    IDS.get_or_init(|| {
        (1..=BASE_SEARCH).map_while(|i| theme::id(&format!("picker.base.{i}"))).collect()
    })
}

/// How many ready-made cells the grid really offers: what the theme
/// asked for, floored by what this build has colours for.
///
/// THE FLOOR IS THE FIX FOR A GHOST CELL. `base_count` used to be clamped
/// to [`BASE_SEARCH`] and the colours were gathered with a `filter_map`
/// over the same range, so a theme writing `base_count = 24` against a
/// master declaring sixteen laid TWENTY-FOUR rectangles and produced
/// SIXTEEN colours. The eight over the end went into [`parts`], and so
/// into [`hit`] and into the focus chain, and were never drawn: eight
/// cells you could Tab to and press, which looked like nothing at all and
/// answered nothing at all. [`parts`]'s own rule — "a part that is drawn
/// is a part that can be reached" — has to hold in the other direction
/// too, and this is where it does.
fn offered(wish: f32) -> usize {
    (wish.round().max(0.0) as usize).min(base_ids().len())
}

/// How tall the control stands in a band `w` wide, offering `custom`
/// colours of the caller's own.
///
/// Asked BEFORE the row is laid out, and answered from the same numbers
/// AND the same count the layout uses — a height that disagreed with the
/// layout would leave the swatches drawn over the row below, and it would
/// start disagreeing on the day somebody banked a ninth colour.
pub fn height(w: f32, custom: usize) -> f32 {
    let m = Metrics::read();
    layout_with(&m, Rect::new(0.0, 0.0, w, 0.0), custom).1
}

/// Where everything stands inside `area`, for a picker offering `custom`
/// colours of its own. The active output space plays no part in the
/// geometry any more — it did only while an HDR ceiling could reserve a
/// second bar, and that bar is gone (2026-08-23); [`draw`] still takes a
/// space, for the gamut-boundary triangle, which affects PAINT, not layout.
pub fn layout(area: Rect, custom: usize) -> Layout {
    let m = Metrics::read();
    layout_with(&m, area, custom).0
}

fn layout_with(m: &Metrics, area: Rect, custom: usize) -> (Layout, f32) {
    // NOTHING MAY LEAVE THE BAND, AND THE BAND IS THE ONLY NUMBER HERE
    // THAT IS NOT THE THEME'S. Every width below is the theme's wish
    // clamped by the room there is, in that order, because the two are
    // answers to different questions: a theme says how wide a value bar
    // ought to be, and only the caller knows how wide the row it stands
    // in turned out. Where they disagree the room wins — a part laid past
    // the band is drawn and PRESSED over whatever is beside it, which is
    // not a look but a fault. Measured before this clamping existed: at
    // a 200 px band the readout began 7.8 px past the right edge, and at
    // 30 px the first ready-made cell stood 29.4 px outside.
    let band = area.w.max(0.0);
    let value_w = m.value_w.min(band);
    let left_w = (band * m.field_w_frac).max(value_w + m.gap).min(band);
    // THE WHEEL IS A CIRCLE INSCRIBED IN THE SAME BOX THE OLD RECTANGULAR
    // FIELD FILLED, centred rather than stretched to it: a disk stretched
    // to a box that is not square would draw an ellipse, and an ellipse
    // is not a hue wheel, it is a hue wheel that has been sat on. The box
    // itself is unchanged — same width, same `m.field_h` — so nothing
    // downstream that reasons about the BOX (`right_h`, `strip_y`) has to
    // change; only what is drawn inside it shrank to a square.
    let box_w = (left_w - m.gap - value_w).max(0.0);
    let box_h = m.field_h;
    let diameter = box_w.min(box_h).max(0.0);
    let field = Rect::new(
        area.x + (box_w - diameter) / 2.0,
        area.y + (box_h - diameter) / 2.0,
        diameter,
        diameter,
    );
    // The bar is hung from the RIGHT of the left column rather than from
    // the field's edge. With room the two are the same point to the last
    // bit; without it, this one is still inside the band.
    let value = Rect::new(area.x + left_w - value_w, area.y, value_w, m.field_h);
    let rw = (band - left_w - m.gap).max(0.0);
    let rx = (area.x + left_w + m.gap).min(area.x + band);
    let patch = Rect::new(rx, area.y, rw, m.patch_h);
    let mut y = patch.bottom() + m.gap;
    // HOW MANY CELLS THE THEME ASKS FOR, AND HOW MANY THERE IS ROOM FOR.
    // `picker.swatch_cols` is the theme's wish and this is the band's
    // answer: a grid wider than the column it stands in would lay cells
    // past the window's own edge. Which of the two wins is not a look —
    // it is arithmetic about a width nobody knew when the theme was
    // written — so the cells wrap sooner, and in a column too narrow for
    // even one they are squeezed rather than allowed out.
    let swatch = m.swatch.min(rw);
    let pitch = swatch + m.swatch_gap;
    let fits = ((rw + m.swatch_gap) / pitch.max(f32::MIN_POSITIVE)).floor();
    let cols = m.cols.min((fits.max(1.0)) as usize).max(1);
    let cell = |i: usize, y0: f32| {
        let (c, r) = (i % cols, i / cols);
        Rect::new(rx + c as f32 * pitch, y0 + r as f32 * pitch, swatch, swatch)
    };
    let base: Vec<Rect> = (0..m.base_count).map(|i| cell(i, y)).collect();
    let base_rows = m.base_count.div_ceil(cols).max(1);
    y += base_rows as f32 * pitch - m.swatch_gap + m.gap;
    // The caller's own colours, and the cell that banks the current one
    // AFTER them: a grid that put the bank first would move every custom
    // colour one place along the moment a new one was added.
    let custom_rects: Vec<Rect> = (0..custom).map(|i| cell(i, y)).collect();
    let add = cell(custom, y);
    let right_h = add.bottom() - area.y;
    // ---- the readout strip, under both columns and across the band.
    // Why it is here and not beside the patch is [`Layout`]'s own note:
    // the mandatory notation does not fit in that column and never could.
    let strip_y = area.y + m.field_h.max(right_h) + m.gap;
    let fmt_w = m.format_w.min(band);
    let format = Rect::new(area.x, strip_y, fmt_w, m.row_h);
    let text = Rect::new(
        (area.x + fmt_w + m.gap).min(area.x + band),
        strip_y,
        (band - fmt_w - m.gap).max(0.0),
        m.row_h,
    );
    (
        Layout { field, value, patch, format, text, base, custom: custom_rects, add },
        text.bottom() - area.y,
    )
}

/// Every part of the control and where it stands, in ONE order.
///
/// The hit test, the focus chain and whatever the application hangs off
/// each part all read this, so a part that is drawn is a part that can be
/// reached — the fault that list exists to prevent is a control with a
/// rect and no place in the Tab order, which is invisible until somebody
/// tries to use the window without a mouse.
///
/// The order is the reading order — the two areas a hand lands in first,
/// then the cells beside them, then the readout strip that runs under
/// both. It FOLLOWS the geometry rather than leading it: the strip moved
/// to the bottom of the control and its two plates moved to the end of
/// this list on the same day, because a Tab order that disagreed with
/// where things stand is a second layout nobody drew. Nothing overlaps,
/// so it is a statement about reading and not about precedence.
pub fn parts(l: &Layout) -> Vec<(Part, Rect)> {
    let mut out = vec![(Part::Field, l.field), (Part::Value, l.value)];
    out.extend(l.base.iter().enumerate().map(|(i, r)| (Part::Base(i), *r)));
    out.extend(l.custom.iter().enumerate().map(|(i, r)| (Part::Custom(i), *r)));
    out.push((Part::Add, l.add));
    out.push((Part::Format, l.format));
    out.push((Part::Text, l.text));
    out
}

/// Which part of the control a point is over.
pub fn hit(l: &Layout, x: f32, y: f32) -> Option<Part> {
    parts(l)
        .into_iter()
        .find(|(_, r)| x >= r.x && x < r.right() && y >= r.y && y < r.bottom())
        .map(|(p, _)| p)
}

/// The theme's ready-made colours, in the order the grid shows them.
///
/// They are TOKENS and not a table in Rust, which is the whole rule this
/// program is built on: the colours a picker offers first are a look, and
/// a look lives in the theme. The master points them at its own palette
/// and severity roles, so the grid of a theme is that theme's own
/// vocabulary rather than a wheel of primaries nobody chose.
///
/// EXACTLY AS MANY AS THE GRID LAYS. The count is [`Metrics`]'s, which is
/// [`offered`]'s, which is floored by [`base_ids`] — so this can never be
/// shorter than the row of rectangles [`layout`] made, and the zip in
/// [`draw`] can never run out.
pub fn base_colours() -> Vec<Color> {
    let n = Metrics::read().base_count;
    let t = theme::resolved();
    base_ids().iter().take(n).map(|i| col(t.color(*i))).collect()
}

// -------------------------------------------------------------- the drawing

fn shape(t: &theme::ResolvedTheme, r: Rect) -> ([Corner; 4], u8) {
    static CORNER: OnceLock<TokenId> = OnceLock::new();
    static CUT: OnceLock<TokenId> = OnceLock::new();
    static CUT_IDX: OnceLock<Cuts> = OnceLock::new();
    static SEGMENTS: OnceLock<TokenId> = OnceLock::new();
    let cut = crate::corner::style(t, tok(&CUT, "picker.corner_style"), &CUT_IDX);
    let c = Corner::sized(cut, t.px(tok(&CORNER, "picker.corner")), r);
    let c = if c.size > 0.0 { c } else { Corner::SQUARE };
    ([c; 4], super::window::corner_segments(t, &SEGMENTS, c.size))
}

/// The chequerboard a colour with alpha is shown against — otherwise a
/// transparent colour and a colour the same shade as the page are the
/// same picture, and the one control that owns alpha could not show it.
fn checker(ctx: &mut Ctx, r: Rect) {
    static SIZE: OnceLock<TokenId> = OnceLock::new();
    static A: OnceLock<TokenId> = OnceLock::new();
    static B: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let s = t.px(tok(&SIZE, "picker.checker")).max(1.0);
    let (a, b) = (col(t.color(tok(&A, "component.picker.checker_a"))), col(t.color(tok(&B, "component.picker.checker_b"))));
    ctx.dl.push_clip(r.x, r.y, r.w, r.h);
    let (nx, ny) = ((r.w / s).ceil() as usize, (r.h / s).ceil() as usize);
    for iy in 0..ny {
        for ix in 0..nx {
            let c = if (ix + iy) % 2 == 0 { a } else { b };
            ctx.dl.rect(r.x + ix as f32 * s, r.y + iy as f32 * s, s, s, c);
        }
    }
    ctx.dl.pop_clip();
}

/// The frame every part of this control wears: one ring, one corner
/// language, both the theme's.
fn frame(ctx: &mut Ctx, r: Rect) {
    static BORDER: OnceLock<TokenId> = OnceLock::new();
    static EDGE: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let (c, seg) = shape(t, r);
    ctx.dl.ring(
        r,
        &c,
        seg,
        t.px(tok(&BORDER, "picker.border")),
        col(t.color(tok(&EDGE, "component.picker.edge"))),
    );
}

/// The handle that marks a chosen point: a ring, because a filled mark
/// would hide the very colour it is pointing at.
fn handle(ctx: &mut Ctx, at: Rect) {
    static STROKE: OnceLock<TokenId> = OnceLock::new();
    static INK: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let (c, seg) = shape(t, at);
    ctx.dl.ring(
        at,
        &c,
        seg,
        t.px(tok(&STROKE, "picker.handle_stroke")),
        col(t.color(tok(&INK, "component.picker.handle"))),
    );
}

/// Draws the whole control in the active output `space` (`None` for no
/// colour management at all). `custom` are the caller's own colours; the
/// picker keeps none of its own, because a swatch a person banked
/// outlives the frame and the control does not.
pub fn draw(ctx: &mut Ctx, l: &Layout, p: &Picker, custom: &[Color], space: Option<GamutSpace>) {
    static HUE_STOPS: OnceLock<TokenId> = OnceLock::new();
    static HANDLE_R: OnceLock<TokenId> = OnceLock::new();
    static ROLE: OnceLock<TokenId> = OnceLock::new();
    static TEXT_INK: OnceLock<TokenId> = OnceLock::new();
    static EDGE: OnceLock<TokenId> = OnceLock::new();
    static BORDER: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    // Read straight off the model: value is `hsv`'s own third coordinate,
    // and the wheel below bakes it into every ring it draws — there is no
    // second control to misread it off any more, the way the bar briefly
    // was between the two swaps this file's header records.
    let v = p.hsv[2];

    // ---- the wheel: hue swept round the rim, saturation out from the
    // centre, at the bar's own value — one fan for the inner cap, one
    // quad per ring-and-wedge cell for the annulus, one more flat quad
    // per wedge for the square's own corners, NEVER `rect_grad`: the
    // wheel's own triangulation is exactly the field's silhouette, so
    // there is nothing to clip and no `push_clip`/`pop_clip` pair to
    // bracket it in. `picker.hue_stops` is how finely the circle is
    // sampled, no longer a metaphor now that the field IS one;
    // [`wheel_tessellation`] is where it becomes a wedge count and a ring
    // count, and the module header is where the kink-alignment that buys
    // is argued.
    let n = (t.px(tok(&HUE_STOPS, "picker.hue_stops")).round() as usize).clamp(2, 64);
    let (wedges, rings) = wheel_tessellation(n);
    let cx = l.field.x + l.field.w / 2.0;
    let cy = l.field.y + l.field.h / 2.0;
    let radius = l.field.w.min(l.field.h) / 2.0;
    // `point_r` takes an explicit radius FRACTION rather than a ring
    // index, so the outer band below — whose outer edge is
    // [`square_reach`]'s and not a ring boundary at all — can share it
    // with the annulus loop; `point(ring, wedge)` is `point_r` at that
    // ring's own fraction, kept as a thin wrapper so the annulus loop
    // below reads exactly as it always has.
    let point_r = |rho: f32, wedge: usize| {
        let theta = (wedge as f32 * 360.0 / wedges as f32).to_radians();
        [cx + radius * rho * theta.cos(), cy + radius * rho * theta.sin()]
    };
    let point = |ring: usize, wedge: usize| point_r(ring as f32 / rings as f32, wedge);
    let colour = |ring: usize, wedge: usize| {
        let rho = ring as f32 / rings as f32;
        let theta = wedge as f32 * 360.0 / wedges as f32;
        let (r, g, b) = hsv_to_rgb(theta, rho, v);
        Color { r, g, b, a: 1.0 }
    };
    // The cap: ring 0 collapsed to the centre point itself, grey at the
    // field's value because saturation 0 has no hue to disagree about —
    // `fan_c`'s own contract closes the rim (wedge `wedges-1` joins
    // wedge 0), which is exactly a full 360° sweep with no seam to name.
    let rim: Vec<[f32; 2]> = (0..wedges).map(|w| point(1, w)).collect();
    let rim_c: Vec<Color> = (0..wedges).map(|w| colour(1, w)).collect();
    ctx.dl.fan_c([cx, cy], &rim, Color { r: v, g: v, b: v, a: 1.0 }, &rim_c);
    // The annulus: one quad per cell between ring `k` and ring `k+1`,
    // exact along the radius by construction (HSV is affine in saturation
    // at fixed hue) and exact across a wedge wherever that wedge's own
    // two edges sit on one of `hsv_to_rgb`'s six 60° kinks, which
    // `wedges` being a multiple of six guarantees for every one of them.
    for ring in 1..rings {
        for w in 0..wedges {
            let w2 = (w + 1) % wedges;
            ctx.dl.quad_c(
                [point(ring, w), point(ring, w2), point(ring + 1, w2), point(ring + 1, w)],
                [colour(ring, w), colour(ring, w2), colour(ring + 1, w2), colour(ring + 1, w)],
            );
        }
    }
    // The square's own corners: one flat-coloured quad per wedge, from
    // the wheel's own rim (ring `rings`, saturation exactly 1) out to
    // [`square_reach`]'s answer at each of the wedge's two edges. FLAT
    // and not another Gouraud band, because `wheel_pick`'s own overshoot
    // rule already established that saturation pins at the rim past
    // r = 1 — every point out here is the SAME colour as the rim point it
    // grows from, so there is nothing to interpolate toward.
    for w in 0..wedges {
        let w2 = (w + 1) % wedges;
        let theta1 = (w as f32 * 360.0 / wedges as f32).to_radians();
        let theta2 = (w2 as f32 * 360.0 / wedges as f32).to_radians();
        let (c1, c2) = (colour(rings, w), colour(rings, w2));
        ctx.dl.quad_c(
            [
                point_r(1.0, w),
                point_r(1.0, w2),
                point_r(square_reach(theta2), w2),
                point_r(square_reach(theta1), w),
            ],
            [c1, c2, c2, c1],
        );
    }
    frame(ctx, l.field);

    // ---- the gamut-boundary triangle, iff the caller named a space:
    // THREE STRAIGHT EDGES between the space's own three primaries, never
    // a per-hue sampled curve and never touching `v` — a real RGB gamut's
    // chromaticity boundary is exactly this shape and it does not move
    // when the picker's own lightness does (module header, "A SQUARE AND
    // NOT A CIRCLE"). Each primary's CIE xy goes through
    // `Primaries::in_srgb_basis` into the wheel's own sRGB-relative hue
    // and saturation terms, `wheel_point_unclamped` places it — past the
    // rim, toward the square's own corners, wherever that primary reaches
    // further than sRGB's own — and `DrawList::polyline` closes the
    // triangle.
    if let Some(sp) = space {
        let vertex = |xy: (f32, f32)| {
            let [pr, pg, pb] = theme::color::Primaries::in_srgb_basis(xy);
            let (h, s, _) = rgb_to_hsv(pr, pg, pb);
            let (fx, fy) = wheel_point_unclamped(h, s);
            [l.field.x + fx * l.field.w, l.field.y + fy * l.field.h]
        };
        let curve = [vertex(sp.primaries.r), vertex(sp.primaries.g), vertex(sp.primaries.b)];
        ctx.dl.polyline(
            &curve,
            t.px(tok(&BORDER, "picker.border")),
            col(t.color(tok(&EDGE, "component.picker.edge"))),
            true,
        );
    }

    // ---- the bar: value, down to black — restored the same day it left,
    // now that saturation is the wheel's own radius and the bar has
    // exactly one axis left. `p.hsv[1]` and not `1.0` in the top stop:
    // the bar shows THIS colour's own saturation fading to black, not
    // every hue's own maximum, which is what the wheel is for.
    let (hr, hg, hb) = hsv_to_rgb(p.hsv[0], p.hsv[1], 1.0);
    ctx.dl.rect_grad(
        l.value,
        &[
            (0.0, Color { r: hr, g: hg, b: hb, a: 1.0 }),
            (1.0, Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }),
        ],
        std::f32::consts::FRAC_PI_2,
    );
    frame(ctx, l.value);

    // ---- the two handles: the field's own themed shape, the same as
    // every other part of this control since the field went back to
    // being an ordinary square (`frame`'s own note, module header).
    let hr_px = t.px(tok(&HANDLE_R, "picker.handle"));
    let (fx, fy) = p.field_at();
    let hx = l.field.x + fx * l.field.w;
    let hy = l.field.y + fy * l.field.h;
    handle(ctx, Rect::new(hx - hr_px, hy - hr_px, hr_px * 2.0, hr_px * 2.0));
    let vy = l.value.y + p.value_at() * l.value.h;
    handle(
        ctx,
        Rect::new(l.value.x, vy - hr_px, l.value.w, hr_px * 2.0),
    );

    // ---- the patch, over the chequerboard so alpha is visible.
    checker(ctx, l.patch);
    let (c, seg) = shape(t, l.patch);
    ctx.dl.ring_fill(l.patch, &c, seg, p.colour());
    frame(ctx, l.patch);

    // ---- the notation's name, and the value written in it.
    let role = ui::bound_role(&ROLE, "picker.role");
    let px = role.px(ctx, 1.0);
    let font = role.font();
    let track = role.tracking_px(px);
    let fig = role.figures(ctx.fonts, font, px);
    let ink = col(t.color(tok(&TEXT_INK, "component.picker.text")));
    let baseline = |r: &Rect| r.y + (r.h - px * role.leading()) / 2.0;
    // BOTH PLATES INSET THEIR OWN INK BY `picker.pad_x` AND CLIP IT
    // THERE. The inset is the theme's, like every other control's
    // (`[field] pad_x`, `[cell] pad_x`, `[button] pad_x`): text laid on
    // the coordinate the ring is drawn at touches the ring, and "touches"
    // is a look — the look of nought padding, chosen in Rust, which is
    // the one thing this toolkit does not do.
    //
    // The clip stays even though the strip is now wide enough for the
    // longest notation ([`Layout`]'s note): the width comes from the
    // caller's band and a caller may hand this control any band at all,
    // and ink cut at a plate's edge is a readout that is hard to read,
    // while ink running out of it is a readout drawn over the row below.
    // Asked of [`Metrics`] and not of a reader of this function's own:
    // one statement of what `[picker]` says, so the drawing and the
    // layout cannot come apart over a re-bake between them.
    let pad = Metrics::read().pad_x;
    for (r, s) in [(l.format, role.cased(p.format.word())), (l.text, p.text().into())] {
        frame(ctx, r);
        let inner = (r.w - pad * 2.0).max(0.0);
        ctx.dl.push_clip(r.x + pad, r.y, inner, r.h);
        ctx.dl.text_fig(ctx.fonts, font, px, r.x + pad, baseline(&r), &s, ink, track, &fig);
        ctx.dl.pop_clip();
    }

    // ---- the two grids.
    for (r, c) in l.base.iter().zip(base_colours()) {
        swatch(ctx, *r, c);
    }
    for (r, c) in l.custom.iter().zip(custom.iter()) {
        swatch(ctx, *r, *c);
    }
    // The bank cell wears the colour it would bank, so it is a preview
    // and a button at once.
    swatch(ctx, l.add, p.colour());
}

fn swatch(ctx: &mut Ctx, r: Rect, c: Color) {
    let t = theme::resolved();
    if c.a < 1.0 {
        checker(ctx, r);
    }
    let (sh, seg) = shape(t, r);
    ctx.dl.ring_fill(r, &sh, seg, c);
    frame(ctx, r);
}

/// [`draw`], joined to the world's focus chain.
///
/// EVERY PART REGISTERS, not just the field: a swatch the pointer can
/// press and the keyboard cannot reach is a control that exists for half
/// its users. The caller says what each part's identity is (`id_of`),
/// because an id is a PATH in the application's own tree and this
/// library has no idea where in that tree its picker is standing.
///
/// NO PART CLAIMS THE ARROWS. A field could take them — arrows nudging
/// the handle is what a picker on a desktop does — and until something
/// answers them, claiming them would mean four keys that do nothing on
/// the one control that has swallowed them from the chain. So the arrows
/// go on walking the chain, and moving the handle by keyboard is the
/// next stage's work; the swatches make the control usable without a
/// mouse in the meantime.
pub fn draw_focusable(
    ctx: &mut Ctx,
    l: &Layout,
    p: &Picker,
    custom: &[Color],
    space: Option<GamutSpace>,
    id_of: impl Fn(Part) -> FocusId,
) {
    let rings: Vec<(Rect, bool)> = parts(l)
        .into_iter()
        .map(|(part, r)| {
            let f = ctx
                .focus
                .as_deref_mut()
                .map(|fc| fc.register(id_of(part), r, Caps::NONE));
            (r, f.map_or(false, |f| f.ring))
        })
        .collect();
    draw(ctx, l, p, custom, space);
    // The rings go on TOP of the whole control, not each beside its own
    // part: a ring drawn before the patch beside it would be painted over
    // by it.
    for (r, on) in rings {
        focus_ring::draw_faded(ctx, r, on);
    }
}

#[cfg(test)]
mod tests {
    //! The model's four promises, and one of them is a scar.
    //!
    //! Nothing here needs a window: the notations, the coordinates and
    //! the round trips are arithmetic. The layout tests read the theme,
    //! which every test in this crate may do — the master is compiled in.

    use super::*;
    use crate::draw::{DrawCmd, DrawList};
    use crate::font::FontSystem;
    use crate::pointer::Pointer;

    fn approx(a: f32, b: f32, eps: f32, what: &str) {
        assert!((a - b).abs() <= eps, "{what}: {a} vs {b} (eps {eps})");
    }

    /// A window to draw into. No GPU and no surface: the questions below
    /// are about what was ASKED for, which is all a draw list holds.
    fn probe<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem) -> Ctx<'a> {
        Ctx {
            dl,
            fonts,
            w: 1920.0,
            h: 1080.0,
            t: 0.0,
            mouse: Pointer::new(-1.0, -1.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        }
    }

    /// How wide a string is on the readout, in the role and at the size
    /// the control actually draws it — figure box included, because the
    /// data role steps its digits and a measurement without that is a
    /// measurement of a different string.
    fn readout_px(fonts: &mut FontSystem, s: &str) -> f32 {
        static ROLE: OnceLock<TokenId> = OnceLock::new();
        let role = ui::bound_role(&ROLE, "picker.role");
        let mut dl = DrawList::new();
        let px = {
            let ctx = probe(&mut dl, fonts);
            role.px(&ctx, 1.0)
        };
        let font = role.font();
        let track = role.tracking_px(px);
        let fig = role.figures(fonts, font, px);
        fonts.measure_fig(font, px, s, track, &fig)
    }

    #[test]
    fn a_colour_typed_in_hex_comes_back_the_same_colour() {
        // ARGB: alpha FIRST, which is the whole reason this notation is
        // the default — the alpha is inside the value.
        let c = parse("#80112233", Format::Argb).expect("eight digits are a colour");
        assert_eq!(write(c, Format::Argb), "#80112233");
        assert_eq!((q8(c.r), q8(c.g), q8(c.b), q8(c.a)), (0x11, 0x22, 0x33, 0x80));
        // The same digits read as RGBA are a DIFFERENT colour, and that
        // is the confusion both notations exist to make visible.
        let d = parse("#80112233", Format::Rgba).expect("eight digits are a colour");
        assert_eq!((q8(d.r), q8(d.g), q8(d.b), q8(d.a)), (0x80, 0x11, 0x22, 0x33));
        assert_eq!(write(d, Format::Rgba), "#80112233");
        // Six digits are opaque in both, and come back as eight with the
        // alpha where that notation keeps it — which is the whole of the
        // difference between them.
        for (f, want) in [(Format::Argb, "#FF3FE3AE"), (Format::Rgba, "#3FE3AEFF")] {
            let s = parse("#3FE3AE", f).expect("six digits are a colour");
            assert_eq!(q8(s.a), 255);
            assert_eq!(write(s, f), want);
        }
        // Through the control itself: what the picker shows is what a
        // person typed.
        let mut p = Picker::of(Color::BLACK);
        assert!(p.set_text("#80112233"));
        assert_eq!(p.text(), "#80112233");
    }

    #[test]
    fn changing_the_notation_changes_the_spelling_and_not_the_colour() {
        let mut p = Picker::of(Color::rgba8(0x3F, 0xE3, 0xAE, 0xCC));
        let before = p.colour();
        let mut seen = Vec::new();
        for _ in 0..Format::ALL.len() {
            seen.push(p.text());
            p.cycle_format();
            // The colour is bit-for-bit what it was: a notation is a way
            // of writing, not a way of rounding.
            assert_eq!(p.colour(), before, "the notation moved the colour");
        }
        // A full ring comes back to where it started.
        assert_eq!(p.format, Format::Argb);
        // Six notations, six different strings: a format that spelled the
        // same as its neighbour would be a choice that does nothing.
        let mut sorted = seen.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), Format::ALL.len());
        // AND EVERY STRING SAYS WHICH NOTATION IT IS. Three of the six are
        // functional, and a functional notation that announced itself as
        // another one would be a value nobody could paste anywhere: the
        // numbers of `hsl` written under the word `hsv` are a different
        // colour to every reader in the world, this file's forgiving
        // parser included.
        for (f, s) in Format::ALL.iter().zip(seen.iter()) {
            if let Format::Oklch | Format::Hsv | Format::Hsl = f {
                assert!(
                    s.starts_with(&f.word().to_lowercase()),
                    "{f:?} must announce itself: {s}"
                );
            }
        }
        // And every one of them READS BACK as the colour it wrote, TO ITS
        // OWN RESOLUTION AND NOT TO A TOLERANCE THAT COVERS FOR IT. The
        // three byte notations quantise by definition, so half a step of
        // 1/255 is exactly what they may lose; the three functional ones
        // write decimals and have no such excuse. A blanket 0.02 used to
        // stand here, and it hid `{:.0}` degrees and whole percents in
        // HSV and HSL — a colour shown in those notations and read back
        // was 0.042 away in OKLCh lightness, which is visible.
        for f in Format::ALL {
            let eps = match f {
                Format::Argb | Format::Rgb | Format::Rgba | Format::Dec => 0.5 / 255.0,
                Format::Oklch | Format::Hsv | Format::Hsl => 1e-3,
            };
            let s = write(before, f);
            let back = parse(&s, f).unwrap_or_else(|| panic!("{f:?} cannot read {s}"));
            // RGB IS THE ONE EXCEPTION TO ITS OWN RESOLUTION, and on
            // purpose: it has no alpha byte to lose a HALF a step of — it
            // has none at all, by the same construction that lets a
            // control fed by it skip an alpha channel it never meant to
            // carry (`Format::Rgb`'s own doc). So its round trip is
            // checked against 1.0, not against `before.a`, everywhere
            // else in this loop unchanged.
            let want_a = if f == Format::Rgb { 1.0 } else { before.a };
            for (a, b, ch) in
                [(back.r, before.r, 'r'), (back.g, before.g, 'g'), (back.b, before.b, 'b'), (back.a, want_a, 'a')]
            {
                approx(a, b, eps, &format!("{f:?} channel {ch}"));
            }
        }
    }

    #[test]
    fn the_alpha_of_an_argb_value_reaches_the_theme() {
        // The picker is handed a half-transparent colour as eight hex
        // digits and asked for what the FILE would receive.
        let mut p = Picker::of(Color::WHITE);
        assert!(p.set_text("#803FE3AE"));
        let lit = theme::edit::oklch_literal(p.oklch());
        assert!(
            lit.contains(" / 0.502"),
            "the alpha must cross into the theme's own spelling: {lit}"
        );
        // And it is the ALPHA that crossed, not a darkened colour: the
        // opaque twin writes no slash at all.
        let mut q = Picker::of(Color::WHITE);
        assert!(q.set_text("#FF3FE3AE"));
        let opaque = theme::edit::oklch_literal(q.oklch());
        assert!(!opaque.contains('/'), "an opaque colour writes no alpha: {opaque}");
        approx(p.oklch().l, q.oklch().l, 1e-4, "alpha must not move lightness");
    }

    #[test]
    fn the_notation_survives_twenty_round_trips() {
        //! THE SCAR. `.gap-program/obalone-naprawy.md` and the head of
        //! this file record what happened when a crossing to OKLCh
        //! skipped the decode: the editor seeded itself from what it had
        //! written, so the accent's lightness climbed 0.8200 -> 0.8904 ->
        //! 0.9413 -> 0.9715 with nobody touching a control. Twenty trips
        //! is far past where that was already obvious.
        let mut p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        let first = p.oklch();
        for i in 0..20 {
            let there = p.oklch();
            p.set_oklch(there);
            let back = p.oklch();
            approx(back.l, first.l, 2e-3, &format!("lightness after trip {i}"));
            approx(back.c, first.c, 2e-3, &format!("chroma after trip {i}"));
            approx(back.h, first.h, 0.5, &format!("hue after trip {i}"));
        }
        // The same walk through the TEXT, which is the road a person
        // takes: write it out, read it back, twenty times.
        let mut q = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        q.format = Format::Oklch;
        for i in 0..20 {
            let s = q.text();
            assert!(q.set_text(&s), "trip {i} wrote a value it cannot read: {s}");
        }
        approx(q.oklch().l, first.l, 3e-3, "lightness after twenty written trips");
    }

    #[test]
    fn the_bar_is_what_its_gradient_draws() {
        // The value line the module header states, `rgb(h, s, v) = v ·
        // rgb(h, s, 1)`, read as `bar_colour` reads it — value shrinking
        // toward BLACK at fixed hue and saturation, which is what moved
        // onto the bar the day saturation moved onto the wheel.
        for &s in &[0.0f32, 0.35, 1.0] {
            for &h in &[0.0f32, 95.0, 210.0, 359.0] {
                for &fy in &[0.0f32, 0.25, 0.5, 0.75, 1.0] {
                    let (r, g, b) = hsv_to_rgb(h, s, 1.0 - fy);
                    let (br, bg, bb) = bar_colour(h, s, fy);
                    approx(br, r, 1e-5, "bar red");
                    approx(bg, g, 1e-5, "bar green");
                    approx(bb, b, 1e-5, "bar blue");
                }
            }
        }
        // THE BOTTOM IS BLACK AND NOT GREY, NAMED. Saturation's own zero
        // used to be this axis's bottom and that is grey, not the
        // absence of light — the very confusion the module header's
        // "VALUE BAR IS EXACT" paragraph turns on. Value's zero is black,
        // whatever the saturation, and that is checked here directly
        // rather than left to fall out of the loop above.
        for &s in &[0.0f32, 0.6, 1.0] {
            let (r, g, b) = bar_colour(140.0, s, 1.0);
            approx(r, 0.0, 1e-6, "bar bottom is black");
            approx(g, 0.0, 1e-6, "bar bottom is black");
            approx(b, 0.0, 1e-6, "bar bottom is black");
        }
    }

    #[test]
    fn wheel_point_and_wheel_pick_are_exact_inverses_off_the_centre() {
        //! [`wheel_point`] places the handle, [`wheel_pick`] reads a
        //! press back into hue and saturation — a picker whose handle
        //! drifted from the point a press landed on would be lying about
        //! where it is standing, one drag at a time.
        for &s in &[0.02f32, 0.2, 0.5, 0.9, 1.0] {
            for &h in &[0.0f32, 12.0, 90.0, 179.9, 271.0, 359.9] {
                let (fx, fy) = wheel_point(h, s);
                // A hue the dead zone would never itself answer with, so
                // a wrong fallback into it cannot pass by accident.
                let (h2, s2) = wheel_pick(fx, fy, -1.0);
                approx(h2, h, 1e-2, &format!("hue round-trip at h={h} s={s}"));
                approx(s2, s, 1e-5, &format!("sat round-trip at h={h} s={s}"));
            }
        }
    }

    #[test]
    fn wheel_pick_clamps_an_overshoot_and_keeps_hue_in_the_dead_zone() {
        //! CLAMPED, NEVER REJECTED: a drag that leaves the wheel's own
        //! bounding square still answers, pinned to the rim rather than
        //! refused — the standard picker feel, and [`wheel_pick`]'s own
        //! doc states it as the reason `fx`/`fy` are not pre-clamped
        //! before this call.
        let (h, s) = wheel_pick(0.5 + 4.0, 0.5, 999.0); // four diameters out, due east
        approx(h, 0.0, 1e-3, "an overshoot keeps steering by angle");
        approx(s, 1.0, 1e-6, "an overshoot pins saturation at the rim");
        // The dead zone at the exact centre answers with the hue it was
        // handed rather than inventing one out of `atan2(0, 0)`, which
        // has no angle to give — the wheel's own version of the rule
        // `a_drag_onto_the_grey_axis_keeps_the_hue_it_came_from` states
        // for the whole control below.
        let (h0, s0) = wheel_pick(0.5, 0.5, 123.0);
        approx(h0, 123.0, 1e-6, "the dead zone keeps the hue it was given");
        approx(s0, 0.0, 1e-6, "the centre is zero saturation");
    }

    #[test]
    fn a_drag_onto_the_grey_axis_keeps_the_hue_it_came_from() {
        // The wheel's own dead zone (2026-08-23, `wheel_pick`) is where a
        // hand can land exactly on grey — its centre — without the
        // field forgetting which hue it stood on, so this drives the
        // invariant through the wheel and [`Picker::hue`] rather than
        // through the bar's `pick_value`, which is what used to zero
        // saturation out while the bar still answered for that axis.
        let mut p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        let hue = p.hue();
        p.pick_field(0.5, 0.5); // the wheel's own centre: saturation to nothing
        assert_eq!(p.colour().r, p.colour().b, "the centre is grey");
        approx(p.hue(), hue, 1e-6, "the hue handle stayed where the hand left it");
        // AND IT SURVIVES A RE-SEED, which is the road this actually
        // happens on: the editor reads the theme back into its controls
        // on every visit, and a grey read back in has no hue to give.
        let grey = p.colour();
        p.set_colour(grey);
        approx(p.hue(), hue, 1e-6, "a re-seed off a grey kept the hue");
        // And coming back off the centre, at the SAME hue and full
        // saturation, returns the same hue.
        let (fx, fy) = wheel_point(hue, 1.0);
        p.pick_field(fx, fy);
        approx(p.oklch().h, Picker::of(Color::rgb8(0x00, 0xFF, 0xB0)).oklch().h, 40.0, "hue");
    }

    #[test]
    fn the_field_is_a_square_centred_in_the_box_the_old_rectangle_filled() {
        //! `layout_with`'s own note: the BOX is unchanged from the old
        //! rectangular field's — same width, same `picker.field_h` — and
        //! only what is drawn inside it shrank to a circle's own square,
        //! centred rather than stretched to the box, so a wheel never
        //! reads as an ellipse. Recomputed from [`Metrics`] independently
        //! of `layout_with`'s own arithmetic, the way this file's other
        //! layout tests already check a public reader against a value
        //! worked out from the theme rather than only against itself.
        let m = Metrics::read();
        for w in [520.0f32, 400.0, 300.0, 150.0] {
            let area = Rect::new(30.0, 40.0, w, 0.0);
            let l = layout(area, 0);
            let band = area.w.max(0.0);
            let value_w = m.value_w.min(band);
            let left_w = (band * m.field_w_frac).max(value_w + m.gap).min(band);
            let box_w = (left_w - m.gap - value_w).max(0.0);
            let box_h = m.field_h;
            let diameter = box_w.min(box_h).max(0.0);
            approx(l.field.w, diameter, 1e-4, &format!("field width at band {w}"));
            approx(l.field.h, diameter, 1e-4, &format!("field height at band {w}"));
            approx(
                l.field.x,
                area.x + (box_w - diameter) / 2.0,
                1e-3,
                &format!("field is centred horizontally in its box at band {w}"),
            );
            approx(
                l.field.y,
                area.y + (box_h - diameter) / 2.0,
                1e-3,
                &format!("field is centred vertically in its box at band {w}"),
            );
        }
    }

    #[test]
    fn the_layout_reserves_exactly_the_height_it_reports() {
        let area = Rect::new(30.0, 40.0, 520.0, 0.0);
        // Past a full row of banked colours the grid grows a row, which
        // is the case a height that ignored the count would get wrong.
        for custom in [0usize, 1, 7, 8, 17] {
            let l = layout(area, custom);
            let h = height(area.w, custom);
            let low = l
                .base
                .iter()
                .chain(l.custom.iter())
                .chain([l.field, l.value, l.patch, l.format, l.text, l.add].iter())
                .fold(area.y, |acc, r| acc.max(r.bottom()));
            approx(h, low - area.y, 0.51, "the reported height covers every part");
        }
    }

    #[test]
    fn nothing_is_laid_outside_the_band_it_was_given() {
        //! THE WIDTHS GO DOWN TO ABSURD ON PURPOSE. The old sweep asked
        //! at 520 and 260 and the comment above `layout_with` promised
        //! "none of them leaves the band" for every width there is — and
        //! at 200 the readout began 7.8 px past the right edge, at 150
        //! and 100 it was 8.1, and at 30 the first ready-made cell stood
        //! 29.4 px outside. Two widths on the safe side of a threshold do
        //! not measure a threshold. A part outside the band is not merely
        //! ugly: `parts` hands it to `hit` and to the focus chain, so it
        //! is pressed and Tabbed to over whatever it is lying on.
        for custom in [0usize, 1, 7, 8, 17] {
            for w in [520.0f32, 400.0, 300.0, 260.0, 200.0, 150.0, 100.0, 60.0, 30.0, 20.0, 0.0] {
                let area = Rect::new(30.0, 40.0, w, 0.0);
                let l = layout(area, custom);
                for (part, r) in parts(&l) {
                    assert!(
                        r.x >= area.x - 0.01 && r.right() <= area.x + area.w + 0.01,
                        "{part:?} runs past the band at width {w}: \
                         {} .. {} against {} .. {}",
                        r.x,
                        r.right(),
                        area.x,
                        area.x + area.w
                    );
                    assert!(r.w >= 0.0 && r.h >= 0.0, "{part:?} has a negative side at {w}");
                }
                // The patch is drawn and not pressed, so it is not in
                // `parts` — and it is inside the band all the same.
                assert!(
                    l.patch.x >= area.x - 0.01 && l.patch.right() <= area.x + area.w + 0.01,
                    "the patch runs past the band at width {w}"
                );
                approx(
                    height(w, custom),
                    parts(&l).iter().fold(area.y, |a, (_, r)| a.max(r.bottom())) - area.y,
                    0.51,
                    &format!("the reported height covers every part at width {w}"),
                );
            }
        }
    }

    #[test]
    fn the_readout_holds_the_notation_this_file_calls_mandatory() {
        //! OKLCh is not one notation of six here: it is the one a
        //! `.theme` file is written in, and the head of this module calls
        //! typing it the only way a value moves between the editor and an
        //! author's own file. That claim is about the SCREEN, and it was
        //! false while the readout lived in the column beside the field —
        //! `oklch(0.8200, 0.1531, 166.22 / 0.502)` measures 224 px at
        //! `type.data` and that column is 210 px wide in the window this
        //! control was built for, so the mandatory notation was the one
        //! that got cut off. Measured here rather than reasoned about,
        //! because the answer is a font's and not arithmetic's.
        let mut fonts = FontSystem::new();
        let longest: Vec<String> = Format::ALL
            .iter()
            .map(|f| {
                // The widest a notation ever gets: every digit at its
                // fattest, and an alpha, which adds the slash clause.
                write(Color { r: 0.7333, g: 0.2667, b: 0.9333, a: 0.502 }, *f)
            })
            .collect();
        let need = longest
            .iter()
            .map(|s| readout_px(&mut fonts, s))
            .fold(0.0f32, f32::max);
        let pad = Metrics::read().pad_x;
        // The band the settings window gives it, and comfortably below.
        for w in [520.0f32, 460.0, 400.0] {
            let l = layout(Rect::new(0.0, 0.0, w, 0.0), 3);
            assert!(
                l.text.w - pad * 2.0 >= need,
                "the readout is {} px wide inside its padding at band {w}, \
                 and the longest value this control writes is {need} px: {longest:?}",
                l.text.w - pad * 2.0
            );
        }
        // And the plate that names the notation holds the longest word.
        let word = Format::ALL
            .iter()
            .map(|f| readout_px(&mut fonts, f.word()))
            .fold(0.0f32, f32::max);
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), 3);
        assert!(
            l.format.w - pad * 2.0 >= word,
            "the notation's plate is {} px inside its padding, the longest word {word} px",
            l.format.w - pad * 2.0
        );
    }

    #[test]
    fn the_plates_hold_their_ink_off_their_own_ring() {
        //! An inset is a look and every comparable object in this toolkit
        //! takes one from the theme (`[field] pad_x`, `[cell] pad_x`,
        //! `[button] pad_x`). This control drew both its plates' text at
        //! the plate's own x — the coordinate `frame` puts the ring on —
        //! which is nought padding, and nought is a value like any other.
        let pad = Metrics::read().pad_x;
        assert!(pad > 0.0, "the master gives the plates an inset");
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::recording();
        let l = layout(Rect::new(30.0, 40.0, 520.0, 0.0), 2);
        let p = Picker::of(Color::rgba8(0x3F, 0xE3, 0xAE, 0xCC));
        draw(&mut probe(&mut dl, &mut fonts), &l, &p, &[Color::WHITE, Color::BLACK], None);
        let runs: Vec<[f32; 2]> = dl
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { at, .. } => Some(*at),
                _ => None,
            })
            .collect();
        assert_eq!(runs.len(), 2, "the two plates write one run each");
        for (at, plate) in runs.iter().zip([l.format, l.text]) {
            approx(at[0], plate.x + pad, 1e-4, "the ink starts a padding in");
        }
    }

    #[test]
    fn the_grid_lays_no_cell_it_has_no_colour_for() {
        //! `picker.base_count` is a wish and `picker.base.N` is what the
        //! build can honour. They were clamped to different numbers — the
        //! count to a round 24 in Rust, the colours to whichever ids
        //! resolved — so a theme writing `base_count = 24` against a
        //! master declaring sixteen got twenty-four rectangles and
        //! sixteen colours. The eight over the end were in `parts`, so in
        //! `hit` and in the focus chain, and were never drawn: cells you
        //! could Tab to and press that showed nothing and did nothing.
        let asked = offered(BASE_SEARCH as f32);
        assert_eq!(asked, base_ids().len(), "the wish is floored by what exists");
        assert!(asked > 0, "the master declares a grid");
        let mut m = Metrics::read();
        m.base_count = asked;
        let (l, _) = layout_with(&m, Rect::new(0.0, 0.0, 520.0, 0.0), 0);
        assert_eq!(
            l.base.len(),
            base_colours().len(),
            "the grid lays exactly as many cells as it has colours"
        );
        // The reader stops at a hole rather than closing it up: a skipped
        // number would renumber every cell behind it, and `Base(i)` is
        // the number a press is answered by.
        for (i, id) in base_ids().iter().enumerate() {
            assert_eq!(
                Some(*id),
                theme::id(&format!("picker.base.{}", i + 1)),
                "cell {i} is base.{}",
                i + 1
            );
        }
    }

    #[test]
    fn the_wheel_the_drawing_emits_is_the_wheel_the_reference_states() {
        //! `the_bar_is_what_its_gradient_draws` above checks the
        //! ARITHMETIC of the value line; this checks the CALLS the wheel
        //! makes. There is no `wheel_colour` reference function the way
        //! there was a `field_colour` — the wheel's law is `hsv_to_rgb`
        //! directly, at the ring's own radius fraction and the wedge's
        //! own angle — so this recomputes exactly that and checks every
        //! vertex the fan and the quads actually emitted against it: a
        //! wedge read at the wrong angle, a ring at the wrong radius, or
        //! `v` picked up from the wrong place would have left every
        //! arithmetic-only test above green.
        //!
        //! WHAT THIS STILL DOES NOT REACH, said plainly, same as its
        //! predecessor: these are the vertices HANDED OUT: that Gouraud
        //! interpolation reproduces a linear function exactly between
        //! them is the renderer's promise (`quad_c`'s own doc), it lives
        //! in another repository, and no test in this one can stand in
        //! for it. If that promise breaks, the wheel is wrong on screen
        //! with every assertion in this file passing.
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::recording();
        let l = layout(Rect::new(30.0, 40.0, 520.0, 0.0), 0);
        let p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        let hue_stops = theme::resolved().px(theme::id("picker.hue_stops").expect("declared"));
        let (wedges, rings) = wheel_tessellation((hue_stops.round() as usize).clamp(2, 64));
        draw(&mut probe(&mut dl, &mut fonts), &l, &p, &[], None);
        let cx = l.field.x + l.field.w / 2.0;
        let cy = l.field.y + l.field.h / 2.0;
        let radius = l.field.w.min(l.field.h) / 2.0;
        let point_at = |rho: f32, wedge: usize| -> [f32; 2] {
            let theta = (wedge as f32 * 360.0 / wedges as f32).to_radians();
            [cx + radius * rho * theta.cos(), cy + radius * rho * theta.sin()]
        };
        let want = |ring: usize, wedge: usize| -> ([f32; 2], Color) {
            let rho = ring as f32 / rings as f32;
            let theta_deg = wedge as f32 * 360.0 / wedges as f32;
            let (r, g, b) = hsv_to_rgb(theta_deg, rho, p.hsv[2]);
            (point_at(rho, wedge), Color { r, g, b, a: 1.0 })
        };
        // ---- the cap: one fan, one point per wedge, closed by fan_c's
        // own contract — checked against the reference at ring 1, which
        // is the rim the cap's own wedge triangles reach out to.
        let fans: Vec<([f32; 2], Color, Vec<([f32; 2], Color)>)> = dl
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::FanC { centre, c_centre, rim } => Some((*centre, *c_centre, rim.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(fans.len(), 1, "the cap is one fan and not a dice of triangles");
        let (centre, c_centre, rim) = &fans[0];
        approx(centre[0], cx, 1e-2, "the fan is centred on the field");
        approx(centre[1], cy, 1e-2, "the fan is centred on the field");
        approx(c_centre.r, p.hsv[2], 1e-5, "the centre is grey at the field's value");
        approx(c_centre.g, p.hsv[2], 1e-5, "the centre is grey at the field's value");
        approx(c_centre.b, p.hsv[2], 1e-5, "the centre is grey at the field's value");
        assert_eq!(rim.len(), wedges, "the cap's rim has one point per wedge");
        for (i, (pos, col)) in rim.iter().enumerate() {
            let (want_pos, want_col) = want(1, i);
            approx(pos[0], want_pos[0], 1e-2, &format!("cap rim {i} x"));
            approx(pos[1], want_pos[1], 1e-2, &format!("cap rim {i} y"));
            approx(col.r, want_col.r, 1e-5, &format!("cap rim {i} red"));
            approx(col.g, want_col.g, 1e-5, &format!("cap rim {i} green"));
            approx(col.b, want_col.b, 1e-5, &format!("cap rim {i} blue"));
        }
        // ---- the annular bands: one quad per ring-and-wedge cell.
        let quads: Vec<([[f32; 2]; 4], [Color; 4])> = dl
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::QuadC { p, c } => Some((*p, *c)),
                _ => None,
            })
            .collect();
        // ONE QUAD PER RING-AND-WEDGE CELL PAST THE CAP, PLUS ONE MORE
        // PER WEDGE FOR THE SQUARE'S OWN CORNERS (2026-08-23): the
        // annulus (`rings - 1` bands) and the outer band (always exactly
        // one band, out to `square_reach`) are both `quad_c`, so the
        // total grew from `(rings - 1) * wedges` to `rings * wedges`.
        assert_eq!(
            quads.len(),
            rings * wedges,
            "one quad per ring-and-wedge cell past the cap, plus one outer-band quad per wedge"
        );
        // The first cell of every annulus ring: the fan above already
        // proves the wedge law holds all the way ROUND one ring, so this
        // proves it holds all the way OUT, and the two together cover the
        // grid the ring/wedge loop in `draw` actually walks.
        for ring in 1..rings {
            let (verts, cols) = quads[(ring - 1) * wedges];
            let corners = [want(ring, 0), want(ring, 1), want(ring + 1, 1), want(ring + 1, 0)];
            for i in 0..4 {
                approx(verts[i][0], corners[i].0[0], 1e-2, &format!("ring {ring} corner {i} x"));
                approx(verts[i][1], corners[i].0[1], 1e-2, &format!("ring {ring} corner {i} y"));
                approx(cols[i].r, corners[i].1.r, 1e-5, &format!("ring {ring} corner {i} red"));
                approx(cols[i].g, corners[i].1.g, 1e-5, &format!("ring {ring} corner {i} green"));
                approx(cols[i].b, corners[i].1.b, 1e-5, &format!("ring {ring} corner {i} blue"));
            }
        }
        // ---- the outer band: one flat-coloured quad per wedge, from the
        // rim (ring `rings`, saturation 1) out to `square_reach`'s own
        // answer at each of the wedge's two edges — every wedge's outer
        // band, this time, since [`square_reach`]'s own value is what is
        // under test and it is NOT constant across a wedge's width the
        // way the annulus's radius fraction is.
        let outer_base = (rings - 1) * wedges;
        for w in 0..wedges {
            let w2 = (w + 1) % wedges;
            let (verts, cols) = quads[outer_base + w];
            let theta1 = (w as f32 * 360.0 / wedges as f32).to_radians();
            let theta2 = (w2 as f32 * 360.0 / wedges as f32).to_radians();
            let (rim_w, c_w) = want(rings, w);
            let (rim_w2, c_w2) = want(rings, w2);
            let corners = [
                rim_w,
                rim_w2,
                point_at(square_reach(theta2), w2),
                point_at(square_reach(theta1), w),
            ];
            let want_cols = [c_w, c_w2, c_w2, c_w];
            for i in 0..4 {
                approx(verts[i][0], corners[i][0], 1e-2, &format!("outer band {w} corner {i} x"));
                approx(verts[i][1], corners[i][1], 1e-2, &format!("outer band {w} corner {i} y"));
                approx(cols[i].r, want_cols[i].r, 1e-5, &format!("outer band {w} corner {i} red"));
                approx(cols[i].g, want_cols[i].g, 1e-5, &format!("outer band {w} corner {i} green"));
                approx(cols[i].b, want_cols[i].b, 1e-5, &format!("outer band {w} corner {i} blue"));
            }
            // FLAT, NOT A GRADIENT: both outer corners carry the SAME
            // colour as the inner corner nearest them — there is nothing
            // to interpolate toward, per the module header's argument.
            approx(cols[2].r, cols[1].r, 1e-6, "outer edge is flat, not a second gradient");
            approx(cols[3].r, cols[0].r, 1e-6, "outer edge is flat, not a second gradient");
        }
    }

    #[test]
    fn every_part_of_the_control_answers_for_itself() {
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), 3);
        let mid = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        let (x, y) = mid(l.field);
        assert_eq!(hit(&l, x, y), Some(Part::Field));
        let (x, y) = mid(l.value);
        assert_eq!(hit(&l, x, y), Some(Part::Value));
        let (x, y) = mid(l.format);
        assert_eq!(hit(&l, x, y), Some(Part::Format));
        let (x, y) = mid(l.text);
        assert_eq!(hit(&l, x, y), Some(Part::Text));
        let (x, y) = mid(l.base[0]);
        assert_eq!(hit(&l, x, y), Some(Part::Base(0)));
        let (x, y) = mid(l.custom[2]);
        assert_eq!(hit(&l, x, y), Some(Part::Custom(2)));
        let (x, y) = mid(l.add);
        assert_eq!(hit(&l, x, y), Some(Part::Add));
        // A point in none of them is nobody's.
        assert_eq!(hit(&l, -5.0, -5.0), None);
    }

    #[test]
    fn the_ready_made_colours_come_from_the_theme() {
        let base = base_colours();
        assert!(!base.is_empty(), "the master declares a grid");
        // They are the THEME's: the first cell is the accent the rest of
        // the interface is derived from, so a theme's picker opens on the
        // theme's own vocabulary.
        let accent = theme::id("palette.accent")
            .map(|i| theme::resolved().color(i))
            .expect("the accent is a token");
        approx(base[0].r, accent.r, 1e-5, "the first cell is the accent");
        approx(base[0].g, accent.g, 1e-5, "the first cell is the accent");
        approx(base[0].b, accent.b, 1e-5, "the first cell is the accent");
    }

    #[test]
    fn square_reach_is_one_at_an_edge_and_root_two_at_a_corner() {
        //! [`square_reach`]'s own doc states the closed form; this checks
        //! it against the two points on a square's own boundary a person
        //! can name without doing trigonometry — an edge's midpoint and a
        //! corner — and then against the direct definition (the farther
        //! of `|cos θ|`/`|sin θ|`) at angles that are neither.
        for deg in [0.0f32, 90.0, 180.0, 270.0] {
            approx(square_reach(deg.to_radians()), 1.0, 1e-5, &format!("edge midpoint at {deg}°"));
        }
        for deg in [45.0f32, 135.0, 225.0, 315.0] {
            approx(square_reach(deg.to_radians()), std::f32::consts::SQRT_2, 1e-5, &format!("corner at {deg}°"));
        }
        for deg in [10.0f32, 62.0, 200.0, 340.0] {
            let theta = deg.to_radians();
            let want = 1.0 / theta.cos().abs().max(theta.sin().abs());
            approx(square_reach(theta), want, 1e-5, &format!("direct definition at {deg}°"));
        }
    }

    #[test]
    fn a_ray_at_square_reach_lands_exactly_on_the_squares_boundary() {
        //! Not a restatement of the closed form: this checks the GEOMETRIC
        //! claim [`square_reach`]'s doc makes — that `point_r`'s own
        //! construction, `centre + radius * rho * (cos θ, sin θ)`, lands
        //! exactly on the edge of the square whose half-width is `radius`,
        //! for every angle the outer band actually asks about. If the two
        //! were out of step the outer band drawn in [`draw`] would either
        //! leave a gap at the field's own corners or paint past them.
        for wedges in [6usize, 12, 24, 48] {
            for w in 0..wedges {
                let theta = w as f32 * 360.0 / wedges as f32;
                let reach = square_reach(theta.to_radians());
                let (dx, dy) = (reach * theta.to_radians().cos(), reach * theta.to_radians().sin());
                // The point half a unit square's own side (1.0) from the
                // centre along whichever axis it is closer to end-on.
                approx(dx.abs().max(dy.abs()), 1.0, 1e-4, &format!("wedges={wedges} w={w}"));
            }
        }
    }

    #[test]
    fn the_gamut_triangle_has_three_straight_edges_and_does_not_move_with_value() {
        //! The triangle's own law (module header, "A SQUARE AND NOT A
        //! CIRCLE"): three vertices, one per primary, each placed by
        //! [`theme::color::Primaries::in_srgb_basis`] — sRGB's own
        //! primaries land exactly on the wheel's rim at its own three
        //! primary hues, a wider gamut's own primaries land past it, and
        //! NONE of this reads `Picker::hsv`'s value at all, which is the
        //! bug the earlier per-spoke OKLCh curve had and this replaces.
        let srgb = theme::color::Primaries::SRGB;
        let vertex_hue_sat = |xy: (f32, f32)| {
            let [r, g, b] = theme::color::Primaries::in_srgb_basis(xy);
            let (h, s, _) = rgb_to_hsv(r, g, b);
            (h, s)
        };
        // Circular distance: `rgb_to_hsv`'s own `rem_euclid(360)` can land
        // a hue that is really 0° at 360° minus a rounding hair, and 0°
        // and 360° are the same direction on the wheel.
        let hue_diff = |a: f32, b: f32| {
            let d = (a - b).rem_euclid(360.0);
            d.min(360.0 - d)
        };
        // sRGB targeting itself: the three vertices are exactly the
        // wheel's own three primary hues, at the rim.
        for (xy, want_hue) in [(srgb.r, 0.0), (srgb.g, 120.0), (srgb.b, 240.0)] {
            let (h, s) = vertex_hue_sat(xy);
            assert!(hue_diff(h, want_hue) < 1e-2, "sRGB primary at {xy:?}: hue {h} vs {want_hue}");
            approx(s, 1.0, 1e-3, &format!("sRGB primary at {xy:?} sits on the rim"));
        }
        // A wider gamut's red primary reaches past the rim — the whole
        // reason the field is a square and not a circle — at a hue close
        // to sRGB's own red, not some unrelated direction.
        let p3 = theme::color::Primaries::DISPLAY_P3;
        let (h, s) = vertex_hue_sat(p3.r);
        assert!(s > 1.0, "Display P3's red did not reach past the wheel's own rim: s={s}");
        assert!(hue_diff(h, 0.0) < 15.0, "P3 red strayed to hue {h}");
        // Display P3 shares sRGB's own blue primary exactly, so its
        // vertex must land on the SAME point sRGB's own blue does.
        let (h_b, s_b) = vertex_hue_sat(p3.b);
        assert!(hue_diff(h_b, 240.0) < 1e-2, "P3's shared blue primary: hue {h_b}");
        approx(s_b, 1.0, 1e-3, "P3's shared blue primary sits on the rim");
        // NOTHING here reads `Picker::hsv` — the whole point is that no
        // picker state, and in particular no lightness/value, enters this
        // computation, so two pickers at different values agree on the
        // same triangle. `draw`'s own vertex closure takes no `v` either;
        // this is a structural guarantee, not a numeric probe of it.
        let a = Picker::of(Color::rgb8(0xB0, 0x30, 0x30));
        let b = Picker::of(Color::rgb8(0x10, 0x10, 0x10));
        assert_ne!(a.hsv[2], b.hsv[2], "the two pickers must actually differ in value for this to test anything");
    }

}
