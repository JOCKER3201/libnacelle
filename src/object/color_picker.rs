//! Colour picker: a two-dimensional field of hue by value, a saturation
//! bar beside it, the chosen colour as a patch, that same colour written
//! out in one of six notations, and two grids of ready-made colours.
//! (The field and the bar traded axes 2026-08-23: the field answered
//! for saturation and the bar for value until then. The names below
//! are written for the swap already in force.)
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
//! THE FIELD IS EXACT, AND THAT IS WHY IT IS TWO CALLS AND NOT A GRID OF
//! CELLS. HSV's own definition is affine in value:
//!
//! ```text
//! rgb(h, s, v) = v · rgb(h, s, 1)
//! ```
//!
//! — black mixed with the fully brightened hue by `v`. So the field is
//! a horizontal hue ramp drawn at the current saturation, with ONE
//! two-stop vertical overlay of black whose alpha runs 0 at the top
//! (fully bright) to 1 at the bottom. The compositor's straight alpha
//! over encoded values reproduces the line above exactly, which is
//! what [`field_colour`] and its test assert. Dicing the field into
//! cells would have been the obvious way and would have banded, cost a
//! quad per cell, and put a number of cells in Rust that no theme could
//! have argued with.
//!
//! THE SATURATION BAR IS EXACT FOR THE SAME REASON: `rgb(h, s, v) = s ·
//! rgb(h, 1, v) + (1 − s) · grey(v)`, so it is two stops, the colour at
//! full saturation and grey at the field's own value — never black,
//! which this axis cannot reach.
//!
//! HSV AND NOT OKLCh FOR THE FIELD, and the owner ruled on this on
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
//! what the field's arithmetic above is true of. OKLCh is defined over
//! **LINEAR LIGHT**. Every crossing therefore decodes on the way in
//! ([`Color::to_linear`]) and encodes on the way back ([`Color::to_srgb`]),
//! and neither step is optional. The one time this program mixed the two
//! it did not merely mis-report: the editor seeded itself from what it
//! had just written, so the accent's lightness climbed 0.8200 → 0.8904 →
//! 0.9413 → 0.9715 over successive visits with every slider at rest.
//! `the_notation_survives_twenty_round_trips` is that measurement turned
//! into a test.

use super::focus_ring;
use crate::access::{AccessInfo, Role};
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

/// The colour the field shows at `(fx, fy)`, both 0..1 from its top-left,
/// for a bar standing at saturation `s`.
///
/// The one statement of what the field MEANS. The drawing does not call
/// it — two gradient calls do — and that is the point: this is the
/// definition the drawing is tested against.
pub fn field_colour(fx: f32, fy: f32, s: f32) -> (f32, f32, f32) {
    hsv_to_rgb(fx.clamp(0.0, 1.0) * 360.0, s.clamp(0.0, 1.0), 1.0 - fy.clamp(0.0, 1.0))
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

    /// The field's handle, 0..1 from the field's top-left. Hue across,
    /// VALUE down (2026-08-23: was saturation — the bar answers for that
    /// now, so the two controls do not both move the same axis).
    pub fn field_at(&self) -> (f32, f32) {
        (self.hsv[0].rem_euclid(360.0) / 360.0, 1.0 - self.hsv[2])
    }

    /// The bar's handle, 0..1 from its top (saturated) to its bottom
    /// (grey). Was VALUE (bright to black) until 2026-08-23; the name
    /// stayed — it names the CONTROL, the bar, not the channel it moves.
    pub fn value_at(&self) -> f32 {
        1.0 - self.hsv[1]
    }

    /// A press or a drag inside the field: hue from x, VALUE from y.
    pub fn pick_field(&mut self, fx: f32, fy: f32) {
        self.hsv[0] = fx.clamp(0.0, 1.0) * 360.0;
        self.hsv[2] = 1.0 - fy.clamp(0.0, 1.0);
    }

    /// A press or a drag along the bar: SATURATION from y.
    pub fn pick_value(&mut self, fy: f32) {
        self.hsv[1] = 1.0 - fy.clamp(0.0, 1.0);
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
/// colours of its own.
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
    let field_w = (left_w - m.gap - value_w).max(0.0);
    let field = Rect::new(area.x, area.y, field_w, m.field_h);
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

/// Draws the whole control. `custom` are the caller's own colours; the
/// picker keeps none of its own, because a swatch a person banked
/// outlives the frame and the control does not.
pub fn draw(ctx: &mut Ctx, l: &Layout, p: &Picker, custom: &[Color]) {
    static HUE_STOPS: OnceLock<TokenId> = OnceLock::new();
    static HANDLE_R: OnceLock<TokenId> = OnceLock::new();
    static ROLE: OnceLock<TokenId> = OnceLock::new();
    static TEXT_INK: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    // Read straight off the field: `value_at()` names the BAR now, not
    // this channel (2026-08-23's swap), and using it here would read the
    // bar's own axis (saturation) as if it were value.
    let v = p.hsv[2];

    // ---- the field: one hue ramp at the bar's saturation, one black
    // overlay for value. `picker.hue_stops` is how finely the circle is
    // sampled, and it is the theme's number because it is a trade between
    // a smooth ramp and the bands `rect_grad` cuts between stops.
    let n = (t.px(tok(&HUE_STOPS, "picker.hue_stops")).round() as usize).clamp(2, 64);
    let stops: Vec<(f32, Color)> = (0..=n)
        .map(|i| {
            let f = i as f32 / n as f32;
            let (r, g, b) = hsv_to_rgb(f * 360.0, p.hsv[1], 1.0);
            (f, Color { r, g, b, a: 1.0 })
        })
        .collect();
    ctx.dl.push_clip(l.field.x, l.field.y, l.field.w, l.field.h);
    ctx.dl.rect_grad(l.field, &stops, 0.0);
    // Uniformly scaling RGB toward black is exactly HSV's V shrinking with
    // H and S held fixed, so a black overlay whose alpha runs 0 at the top
    // to 1 at the bottom is value, drawn without a second pass over hue.
    let black0 = Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
    ctx.dl.rect_grad(
        l.field,
        &[(0.0, black0), (1.0, Color { a: 1.0, ..black0 })],
        std::f32::consts::FRAC_PI_2,
    );
    ctx.dl.pop_clip();
    frame(ctx, l.field);

    // ---- the bar: the chosen hue at full saturation and the field's
    // value, down to grey at that same value — never black, which was
    // the value bar's picture and would draw a colour this axis cannot
    // reach (saturation 0 is grey, not the absence of light).
    let (hr, hg, hb) = hsv_to_rgb(p.hsv[0], 1.0, v);
    ctx.dl.rect_grad(
        l.value,
        &[
            (0.0, Color { r: hr, g: hg, b: hb, a: 1.0 }),
            (1.0, Color { r: v, g: v, b: v, a: 1.0 }),
        ],
        std::f32::consts::FRAC_PI_2,
    );
    frame(ctx, l.value);

    // ---- the two handles.
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

/// What a screen reader should say for one part: a name a PERSON would
/// call it, the role it actually plays, and the reading it holds right
/// now — never the Debug-derived `"Field"`/`"Custom(2)"` the foundation
/// pass placeholdered here, which names the enum variant and not the
/// control.
///
/// THE FIELD AND THE BAR ARE NAMED FOR WHAT THEY MOVE, NOT FOR THE
/// VARIANT: `Part::Value` is the bar, and the bar answers for SATURATION
/// since the 2026-08-23 axis swap ([`Picker::value_at`]'s own note) — a
/// bridge that read the variant's name would tell a screen reader the
/// one thing that stopped being true. `Part::Field` still carries two
/// channels at once (hue across, value down), which is why it gets both
/// in its value rather than a single number standing in for two.
///
/// THE FORMAT PLATE AND THE SWATCHES ARE BUTTONS, NOT SLIDERS: a press
/// steps the notation on one, banks or picks a colour on the others —
/// none of them read back a dragged position, which is what
/// [`Role::Slider`] promises a bridge. [`Role::TextInput`] stays on
/// `Part::Text` alone, the one part a person types into.
///
/// EVERY VALUE IS THE COLOUR ITSELF, NOT ITS COORDINATES, on the base
/// and custom cells and the bank button — `write`'s RGBA form, because a
/// swatch can carry alpha (`swatch`'s own chequerboard says so) and a
/// notation that dropped the byte would misreport a transparent cell as
/// opaque. `bases` and `custom` are handed in rather than re-read so a
/// part's value can never disagree with the swatch [`draw`] paints for
/// the same index.
fn part_access(part: Part, p: &Picker, bases: &[Color], custom: &[Color]) -> AccessInfo {
    match part {
        Part::Field => {
            let hue = p.hsv[0].rem_euclid(360.0);
            let value = p.hsv[2] * 100.0;
            AccessInfo::new(Role::Slider, "Hue and value")
                .with_value(format!("hue {hue:.0}°, value {value:.0}%"))
        }
        Part::Value => AccessInfo::new(Role::Slider, "Saturation")
            .with_value(format!("{:.0}%", p.hsv[1] * 100.0)),
        Part::Format => {
            AccessInfo::new(Role::Button, "Colour notation").with_value(p.format.word())
        }
        Part::Text => AccessInfo::new(Role::TextInput, format!("{} value", p.format.word()))
            .with_value(p.text()),
        Part::Base(i) => {
            let mut info = AccessInfo::new(Role::Button, "Preset colour");
            if let Some(c) = bases.get(i) {
                info = info
                    .with_value(write(*c, Format::Rgba))
                    .with_index(i as u32 + 1, bases.len() as u32);
            }
            info
        }
        Part::Custom(i) => {
            let mut info = AccessInfo::new(Role::Button, "Custom colour");
            if let Some(c) = custom.get(i) {
                info = info
                    .with_value(write(*c, Format::Rgba))
                    .with_index(i as u32 + 1, custom.len() as u32);
            }
            info
        }
        Part::Add => AccessInfo::new(Role::Button, "Save current colour")
            .with_value(write(p.colour(), Format::Rgba)),
    }
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
    id_of: impl Fn(Part) -> FocusId,
) {
    // Read once, for every base cell alike: the same rule `layout_with`
    // itself follows (`Metrics` read once and passed around), so a
    // re-bake mid-loop cannot leave one cell's report disagreeing with
    // the swatch [`draw`] paints beside it.
    let bases = base_colours();
    let rings: Vec<(Rect, bool)> = parts(l)
        .into_iter()
        .map(|(part, r)| {
            let access = part_access(part, p, &bases, custom);
            let f = ctx.focus.as_deref_mut().map(|fc| fc.register(id_of(part), r, Caps::NONE, access));
            (r, f.map_or(false, |f| f.ring))
        })
        .collect();
    draw(ctx, l, p, custom);
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
    use crate::focus::FocusCtl;
    use crate::font::FontSystem;
    use crate::pointer::Pointer;

    fn approx(a: f32, b: f32, eps: f32, what: &str) {
        assert!((a - b).abs() <= eps, "{what}: {a} vs {b} (eps {eps})");
    }

    /// A window to draw into. No GPU and no surface: the questions below
    /// are about what was ASKED for, which is all a draw list holds.
    fn probe<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem) -> Ctx<'a> {
        Ctx {
            access: None,
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
    fn the_field_is_what_the_two_gradients_draw() {
        // The value line the drawing stands on: black laid over the hue
        // at full brightness by alpha 1-v IS hsv(h, s, v).
        for &v in &[0.25f32, 0.6, 1.0] {
            for &s in &[0.0f32, 0.35, 1.0] {
                for &h in &[0.0f32, 95.0, 210.0, 359.0] {
                    let (r, g, b) = hsv_to_rgb(h, s, v);
                    let (fr, fg, fb) = field_colour(h / 360.0, 1.0 - v, s);
                    approx(fr, r, 1e-5, "field red");
                    approx(fg, g, 1e-5, "field green");
                    approx(fb, b, 1e-5, "field blue");
                    // What the compositor computes: base·v + black·(1−v).
                    let (br, _, _) = hsv_to_rgb(h, s, 1.0);
                    approx(br * v, r, 1e-5, "overlay red");
                }
            }
        }
    }

    #[test]
    fn a_drag_onto_the_grey_axis_keeps_the_hue_it_came_from() {
        // The field no longer answers for saturation (2026-08-23) — the
        // bar does — so the drag that can zero saturation out and land
        // on the true grey axis is now on the bar, not the field.
        let mut p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        let fx = p.field_at().0;
        p.pick_value(1.0); // saturation to nothing
        assert_eq!(p.colour().r, p.colour().b, "the grey axis is grey");
        let fx2 = p.field_at().0;
        approx(fx2, fx, 1e-6, "the hue handle stayed where the hand left it");
        // AND IT SURVIVES A RE-SEED, which is the road this actually
        // happens on: the editor reads the theme back into its controls
        // on every visit, and a grey read back in has no hue to give.
        let grey = p.colour();
        p.set_colour(grey);
        let fx3 = p.field_at().0;
        approx(fx3, fx, 1e-6, "a re-seed off a grey kept the hue");
        // And coming back off the axis returns the same hue.
        p.pick_value(0.0);
        approx(p.oklch().h, Picker::of(Color::rgb8(0x00, 0xFF, 0xB0)).oklch().h, 40.0, "hue");
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
        draw(&mut probe(&mut dl, &mut fonts), &l, &p, &[Color::WHITE, Color::BLACK]);
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
    fn the_field_the_two_gradients_emit_is_the_field_the_reference_states() {
        //! `the_field_is_what_the_two_gradients_draw` above checks the
        //! ARITHMETIC of the value line; this checks the CALLS. The
        //! drawing does not use `field_colour`, so nothing tied the two
        //! together: a hue ramp emitted at the wrong saturation, or an
        //! overlay whose alpha ran the wrong way, would have left every
        //! test above green.
        //!
        //! WHAT THIS STILL DOES NOT REACH, said plainly. Compositing the
        //! two stops here is arithmetic on the values that were HANDED
        //! OUT; that the compositor blends straight alpha over ENCODED
        //! values — and not in linear light — is the renderer's promise,
        //! it lives in another repository, and no test in this one can
        //! stand in for it. If that promise breaks, the field is wrong on
        //! screen with every assertion in this file passing.
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::recording();
        let l = layout(Rect::new(30.0, 40.0, 520.0, 0.0), 0);
        let p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        let s = 1.0 - p.value_at(); // `value_at` names the bar now: saturation.
        draw(&mut probe(&mut dl, &mut fonts), &l, &p, &[]);
        let grads: Vec<(Vec<(f32, Color)>, f32)> = dl
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::RectGrad { r, stops, angle }
                    if (r[0] - l.field.x).abs() < 1e-3 && (r[1] - l.field.y).abs() < 1e-3 =>
                {
                    Some((stops.clone(), *angle))
                }
                _ => None,
            })
            .collect();
        assert_eq!(grads.len(), 2, "the field is a hue ramp and one overlay");
        let (hue, hue_angle) = &grads[0];
        let (over, over_angle) = &grads[1];
        approx(*hue_angle, 0.0, 1e-6, "the hue runs across");
        approx(*over_angle, std::f32::consts::FRAC_PI_2, 1e-6, "the overlay runs down");
        assert_eq!(over.len(), 2, "the overlay is two stops and not a dice of cells");
        approx(over[0].1.a, 0.0, 1e-6, "the top of the field is fully bright");
        approx(over[1].1.a, 1.0, 1e-6, "the bottom of it is black");
        // At a stop the ramp IS its stop, so the composite can be put
        // against the reference with no interpolation in between.
        for (fx, base) in hue.iter() {
            for &fy in &[0.0f32, 0.25, 0.5, 0.75, 1.0] {
                let grey = Color {
                    r: over[0].1.r + (over[1].1.r - over[0].1.r) * fy,
                    g: over[0].1.g + (over[1].1.g - over[0].1.g) * fy,
                    b: over[0].1.b + (over[1].1.b - over[0].1.b) * fy,
                    a: over[0].1.a + (over[1].1.a - over[0].1.a) * fy,
                };
                let (wr, wg, wb) = field_colour(*fx, fy, s);
                for (got, want, ch) in [
                    (base.r * (1.0 - grey.a) + grey.r * grey.a, wr, 'r'),
                    (base.g * (1.0 - grey.a) + grey.g * grey.a, wg, 'g'),
                    (base.b * (1.0 - grey.a) + grey.b * grey.a, wb, 'b'),
                ] {
                    approx(got, want, 1e-5, &format!("the field at ({fx}, {fy}) channel {ch}"));
                }
            }
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

    // ---- accessible reporting -------------------------------------

    /// Runs [`draw_focusable`] into a live [`FocusCtl`] and hands back
    /// what every part reported, keyed by [`Part`] — the register /
    /// `begin_frame` / read-`prev` dance every test below needs, written
    /// once rather than three times.
    fn registered_access(l: &Layout, p: &Picker, custom: &[Color]) -> Vec<(Part, AccessInfo)> {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::new();
        let mut fc = FocusCtl::new();
        {
            let mut ctx = probe(&mut dl, &mut fonts);
            ctx.focus = Some(&mut fc);
            draw_focusable(&mut ctx, l, p, custom, |part| FocusId::of(&format!("{part:?}")));
        }
        fc.begin_frame();
        parts(l)
            .into_iter()
            .map(|(part, _)| {
                let id = FocusId::of(&format!("{part:?}"));
                let info = fc
                    .entries()
                    .find(|(eid, _, _)| *eid == id)
                    .map(|(_, _, info)| info.clone())
                    .unwrap_or_else(|| panic!("{part:?} never registered"));
                (part, info)
            })
            .collect()
    }

    #[test]
    fn every_part_reports_a_human_name_and_not_its_debug_variant() {
        //! The foundation pass placeholdered `AccessInfo::new(role,
        //! format!("{part:?}"))` at every registration — a screen reader
        //! would have read out `"Base(2)"` and `"Custom(1)"`. This is the
        //! fix, checked both ways: NO part answers with its own Debug
        //! spelling, and the parts a person actually reaches for answer
        //! with the word this control's own doc comments already use for
        //! them.
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), 2);
        let p = Picker::of(Color::rgba8(0x3F, 0xE3, 0xAE, 0xCC));
        let custom = [Color::WHITE, Color::BLACK];
        let by_part = registered_access(&l, &p, &custom);
        for (part, info) in &by_part {
            assert_ne!(
                info.name,
                format!("{part:?}"),
                "{part:?} still reports its Debug-derived identity, not a name"
            );
        }
        let name_of =
            |want: Part| by_part.iter().find(|(part, _)| *part == want).unwrap().1.name.clone();
        assert_eq!(name_of(Part::Field), "Hue and value");
        // `Part::Value` is the BAR, and the bar answers for saturation
        // since the axis swap (`Picker::value_at`'s own note) — its name
        // must say what it moves today, not what the variant is called.
        assert_eq!(name_of(Part::Value), "Saturation");
        assert_eq!(name_of(Part::Format), "Colour notation");
        assert_eq!(name_of(Part::Text), format!("{} value", p.format.word()));
        assert_eq!(name_of(Part::Base(0)), "Preset colour");
        assert_eq!(name_of(Part::Custom(1)), "Custom colour");
        assert_eq!(name_of(Part::Add), "Save current colour");
    }

    #[test]
    fn each_part_reports_the_role_it_actually_plays() {
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), 2);
        let p = Picker::of(Color::rgba8(0x3F, 0xE3, 0xAE, 0xCC));
        let custom = [Color::WHITE, Color::BLACK];
        let by_part = registered_access(&l, &p, &custom);
        for (part, info) in &by_part {
            let want = match part {
                // Dragged, which is what a bridge means by `Role::Slider`.
                Part::Field | Part::Value => Role::Slider,
                // The one part a person types into.
                Part::Text => Role::TextInput,
                // Everything else answers a PRESS — a step, a pick or a
                // bank — never a dragged position.
                Part::Format | Part::Base(_) | Part::Custom(_) | Part::Add => Role::Button,
            };
            assert_eq!(info.role, want, "{part:?} has the wrong role");
        }
    }

    #[test]
    fn each_part_announces_its_current_reading_and_not_just_its_identity() {
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), 2);
        let mut p = Picker::of(Color::rgba8(0x3F, 0xE3, 0xAE, 0xCC));
        // Off the construction defaults, so a test that read back a
        // stale placeholder value would not pass by accident.
        p.pick_field(0.7, 0.2);
        p.pick_value(0.35);
        let custom = [Color::WHITE, Color::BLACK];
        let by_part = registered_access(&l, &p, &custom);
        let value_of = |want: Part| {
            by_part
                .iter()
                .find(|(part, _)| *part == want)
                .unwrap()
                .1
                .value
                .clone()
                .unwrap_or_else(|| panic!("{want:?} has no reading"))
        };
        let index_of = |want: Part| by_part.iter().find(|(part, _)| *part == want).unwrap().1.index;

        assert_eq!(
            value_of(Part::Field),
            format!("hue {:.0}°, value {:.0}%", p.hsv[0], p.hsv[2] * 100.0)
        );
        assert_eq!(value_of(Part::Value), format!("{:.0}%", p.hsv[1] * 100.0));
        assert_eq!(value_of(Part::Format), p.format.word());
        assert_eq!(value_of(Part::Text), p.text());
        // The swatches' readings are the colour itself, RGBA and not
        // RGB: a swatch can carry alpha (`swatch`'s own chequerboard is
        // why) and a notation that dropped the byte would misreport a
        // transparent cell as opaque.
        assert_eq!(value_of(Part::Base(0)), write(base_colours()[0], Format::Rgba));
        assert_eq!(value_of(Part::Custom(1)), write(custom[1], Format::Rgba));
        assert_eq!(value_of(Part::Add), write(p.colour(), Format::Rgba));
        // And the grid cells carry their place in the set
        // (`AccessInfo::index`'s own doc: a tab's `(2, 5)` among five).
        assert_eq!(index_of(Part::Base(0)), Some((1, base_colours().len() as u32)));
        assert_eq!(index_of(Part::Custom(1)), Some((2, custom.len() as u32)));
    }
}
