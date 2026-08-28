//! Colour picker: a bank of sliders, the chosen colour as a patch, that
//! same colour written out as `hsl(...)` — typed into directly, in place
//! — and two grids of ready-made colours.
//!
//! HSL IS THE ONLY NOTATION (the owner's decision, 2026-08-28). Five
//! others stood here through this file's own `Format` enum — HSV, ARGB,
//! RGB, OKLCh, DEC — each with its own doc paragraph arguing for its
//! place; the owner withdrew the argument rather than lost it, the same
//! way CMYK and a separate `Rgba` notation were withdrawn before this
//! file's 2026-08-24 rewrite ever ran. What is built below is what is
//! left once a picker answers exactly one question ("what is this
//! colour, in the one spelling everybody now reads it in") rather than
//! six.
//!
//! WHY AN OBJECT AND NOT A DUMB SLICE OF ROWS. A slider bank could have
//! been three separate `object::slider` rows wired up by hand in
//! `nacelle-desktop`, the way BASIC's own triple was before the
//! 2026-08-18 rewrite that first gave this control its own object — the
//! round trip, the hue kept through the grey axis, the ready-made grid,
//! ALL of it is behaviour this control owns so fourteen pickers on one
//! page cannot each get it a little differently.
//!
//! CANONICAL STATE, AND WHY IT IS TWO KEPT NUMBERS AND NOT ONE: `hsv:
//! [f32; 3]` plus `alpha` was the sole thing a picker remembered between
//! frames while six notations shared the ring — see
//! `the_notation_survives_twenty_round_trips` ("THE SCAR", below), still
//! true of `hsv`/`alpha` themselves. HSL being the only one left
//! (2026-08-28) surfaced a second hole of the SAME SHAPE hue's own kept
//! field ([`Picker::hue`]) already existed to close: `hsv_from_hsl`
//! answers saturation 0 at EITHER lightness extreme — black and white
//! have none to give, by construction, the identical reason a colour on
//! the grey axis has no hue to give — so a picker that re-derived HSL
//! saturation from the colour every frame lost it the instant Lightness
//! reached either end and never gave it back on the way out (found live,
//! after this file already shipped: dragging Lightness to 0 % or 100 %
//! snapped the Saturation slider to 0 %). [`Picker::hsl_sat`] is that
//! second kept field, read and written the same way hue is. Every OTHER
//! HSL slider still reads and writes through `rgb_to_hsl`/`hsl_to_rgb`;
//! the one piece of hygiene beyond that is [`hsv_from_hsl`], factored
//! out of `hsl_to_rgb`'s own body so an HSL slider WRITE lands on
//! `hsv`/`alpha` directly instead of taking an extra lossy hop through
//! full sRGB and back.
//!
//! `Picker::oklch`/`set_oklch` STAY, WITH NO `Format::Oklch` LEFT TO CALL
//! THEM: a `.theme` file is written in OKLCh regardless of what notation
//! a picker's OWN plate shows — `Settings::set_tone_from_picker` and
//! every other crossing from a picked colour into the theme file reads
//! or writes through these two, and losing THAT crossing was never part
//! of the owner's instruction. "THE TRAP THIS FILE IS WRITTEN AROUND",
//! below, is about that crossing and is unchanged by the notations lost
//! around it.
//!
//! INLINE CLICK-TO-EDIT. The value plate is a target with real behaviour
//! now: a press opens [`Picker::editing`], an
//! [`super::text_input::InputModel`] seeded from what the plate shows,
//! drawn IN PLACE by [`text_input::draw`] the moment it is `Some`. Enter
//! commits through the same parser a press already trusts
//! ([`Picker::set_text`]) and STAYS OPEN on a bad parse — nothing typed
//! is thrown away for a typo mid-word; Escape discards and reverts; a
//! BLUR — the one event the SAVE AS prompt never has to answer, since it
//! covers the whole window and a click cannot land anywhere else —
//! COMMITS, because "never destroy a good value over a bad edit" argues
//! for TRYING the typed text over silently dropping what was just
//! finished, and falls back to the last-good colour on a bad parse since
//! focus has already left and there is no more affordance to fix it.
//! `nacelle-desktop`'s own `Settings::editing_picker` is the "one at a
//! time" bookkeeping, mirroring `naming`'s; the model itself lives ON the
//! picker, not in a window-level slot, because fourteen pickers can each
//! be mid-edit in principle and "which one, and what's in the box" is a
//! fact about the picker being typed into.
//!
//! THE GAMUT-BOUNDARY TRIANGLE AND `GamutSpace` ARE GONE. The triangle
//! was an overlay ON THE FIELD — three vertices in the wheel's own polar
//! coordinates — and there is no honest translation of a 2-D boundary
//! shape onto a bank of independent 1-D tracks; forcing it onto a slider
//! is a different feature, not a smaller version of this one. The one
//! place gamut-awareness could honestly reappear — a tick on the OKLCh
//! `C` slider at [`theme::color::Color::max_chroma_in`]'s own answer for
//! the active output space — is real, is general, and is NOT built here:
//! it is a materially smaller feature nobody asked for yet, named so the
//! question is answered and not merely dropped.
//! [`theme::color::Primaries::in_srgb_basis`] stays in `theme/color.rs`
//! regardless — it is cheap, general, and tested on its own terms there;
//! its only caller was this triangle, and losing a caller is not a
//! reason to delete tested general-purpose code (`max_chroma_in`'s own
//! doc already makes this argument about itself).
//!
//! THE TRAP THIS FILE IS WRITTEN AROUND. The colour a picker holds is
//! **sRGB-ENCODED** — what a bake hands back, what hex spells, what every
//! byte slider's own arithmetic below is true of. OKLCh is defined over
//! **LINEAR LIGHT**. Every crossing therefore decodes on the way in
//! ([`Color::to_linear`]) and encodes on the way back ([`Color::to_srgb`]),
//! and neither step is optional. The one time this program mixed the two
//! it did not merely mis-report: the editor seeded itself from what it
//! had just written, so the accent's lightness climbed 0.8200 -> 0.8904 ->
//! 0.9413 -> 0.9715 over successive visits with every slider at rest.
//! `the_notation_survives_twenty_round_trips` is that measurement turned
//! into a test.

use super::focus_ring;
use super::text_input::{self, InputModel, InputStyle};
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

/// The sliders HSL offers — three, alpha not among them (the owner's
/// decision of 2026-08-28: a slider that ends every colour it touches at
/// the same place a Save discards, since a theme's own colour tokens
/// carry no alpha, was a control promising a move it could never keep).
const MAX_SLIDERS: usize = 3;

/// How the chosen colour is written out, read back in, and split into
/// sliders — HSL, and only HSL (the owner's decision of 2026-08-28; see
/// the module header for what stood here before and why it does not
/// anymore). Kept as a one-variant type, rather than pulled out
/// altogether, because [`Picker::format`] and [`write`]/[`parse`] still
/// need one word to name what they read and write, and a future notation
/// re-added here is a variant, not a second control.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Hsl,
}

impl Format {
    /// The one notation on offer.
    pub const ALL: [Format; 1] = [Format::Hsl];

    /// The word on the plate. Upper case like every other word this
    /// window puts on one; the CASE is the type role's business
    /// (`type.<role>.case`), and this is the word itself.
    pub fn word(self) -> &'static str {
        match self {
            Format::Hsl => "HSL",
        }
    }

    /// The next notation round the ring — itself, there being only the
    /// one; [`Picker::cycle_format`] still calls this rather than a
    /// caller assuming what it would find, so a notation re-added here
    /// is reachable again with no change on that side.
    pub fn next(self) -> Format {
        self
    }

    /// How many sliders HSL's own bank offers.
    pub fn slider_count(self) -> usize {
        MAX_SLIDERS
    }

    /// The channel letter over slider `i`, in the order [`write`] spells
    /// HSL in.
    ///
    /// PANICS past `slider_count()`, on purpose: every caller in this
    /// file walks `0..l.sliders.len()`, which IS `slider_count()` by
    /// construction (`layout_with`'s own note), so an out-of-range index
    /// here is a caller that stopped trusting that count rather than a
    /// colour this control was ever asked to show.
    pub fn slider_label(self, i: usize) -> &'static str {
        match self {
            Format::Hsl => ["H", "S", "L"][i],
        }
    }
}

/// HSV -> sRGB-encoded RGB. `h` in degrees, `s` and `v` in 0..1.
///
/// Every slider's gradient that touches hue or value is this function
/// read sideways ([`slider_stops`]).
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
/// from ([`Picker::hue`]).
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
/// past what eight-bit output can hold. The two hex-and-byte notations
/// need no such choice: a byte IS their resolution.
pub fn write(c: Color, f: Format) -> String {
    match f {
        Format::Hsl => {
            let (h, s, l) = rgb_to_hsl(c.r, c.g, c.b);
            write_hsl(h, s, l, c.a)
        }
    }
}

/// The `hsl(...)` spelling of one explicit set of coordinates — the one
/// place [`write`] (a plain colour, nothing kept) and
/// [`Picker::text`](Picker) (the picker's own KEPT hue and saturation,
/// module header "CANONICAL STATE") both land, so the two can never drift
/// apart on how a number becomes text.
fn write_hsl(h: f32, s: f32, l: f32, a: f32) -> String {
    with_alpha(format!("hsl({:.2}, {:.2}, {:.2}", h, s * 100.0, l * 100.0), a)
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

/// HSL's saturation and lightness -> HSV's saturation and value: the
/// pair of lines `hsl_to_rgb` needs on its way to sRGB, factored out so
/// an HSL slider WRITE can land directly on [`Picker::hsv`] without an
/// extra hop through full sRGB and back through `rgb_to_hsv` — the same
/// shape of detour this file's own header warns against for OKLCh, at
/// far lower stakes here, and cheap to avoid since the algebra was
/// already sitting in `hsl_to_rgb`'s own body.
fn hsv_from_hsl(s_l: f32, l: f32) -> (f32, f32) {
    let l = l.clamp(0.0, 1.0);
    let s_l = s_l.clamp(0.0, 1.0);
    // HSL and HSV meet through v = l + s·min(l, 1−l): the same cone read
    // from its middle instead of its tip.
    let v = l + s_l * l.min(1.0 - l);
    let sv = if v <= 0.0 { 0.0 } else { 2.0 * (1.0 - l / v) };
    (sv, v)
}

/// HSL -> sRGB-encoded RGB, the inverse of [`rgb_to_hsl`].
pub fn hsl_to_rgb(h: f32, s: f32, l: f32) -> (f32, f32, f32) {
    let (sv, v) = hsv_from_hsl(s, l);
    hsv_to_rgb(h, sv, v)
}

/// A degree of hue as the 0..1 a hue SLIDER stands on — `h.rem_euclid`,
/// EXCEPT at 360° itself.
///
/// `rem_euclid` is right for the COLOUR (hue 0° and hue 360° are the
/// same point on the wheel) and wrong for the HANDLE: [`Picker::pick_slider`]
/// writes `frac * 360.0` verbatim, so a drag to the track's own right
/// edge (`frac == 1.0`) stores exactly `360.0`, and reading that back
/// through a plain `rem_euclid(360.0)` answers `0.0` — the handle
/// teleports to the track's LEFT edge the instant a press reaches its
/// right one (2026-08-28's fix). Every OTHER hue this control ever
/// holds comes back out of [`rgb_to_hsv`]/[`rgb_to_hsl`], which never
/// themselves answer exactly `360.0`, so this only ever changes the one
/// value a slider's own extreme can produce.
fn hue_frac(h: f32) -> f32 {
    if h > 0.0 && h % 360.0 == 0.0 { 1.0 } else { h.rem_euclid(360.0) / 360.0 }
}

/// Text back to a colour, or `None` when the text is not that notation.
///
/// FORGIVING ABOUT PUNCTUATION, STRICT ABOUT MEANING. The name in front
/// (`hsl(`), the parentheses, the commas and the spaces are all
/// optional, because a person pasting a value from a file should not
/// have to tidy it first; what is NOT optional is the count and the
/// order of the numbers, because those are the notation.
pub fn parse(text: &str, f: Format) -> Option<Color> {
    let t = text.trim();
    match f {
        Format::Hsl => {
            let (n, a) = numbers(t, "hsl")?;
            if n.len() != 3 {
                return None;
            }
            let (r, g, b) = hsl_to_rgb(n[0], n[1] / 100.0, n[2] / 100.0);
            Some(Color { r, g, b, a: a.unwrap_or(1.0).clamp(0.0, 1.0) })
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
/// recomputed its coordinates from the colour every frame would swing
/// every hue-bearing slider to red the moment a drag reached the grey
/// axis, and leave it there when the drag came back. `hsv[0]` is
/// therefore the state, and the COLOUR is what it answers, together with
/// `hsv[1]`, `hsv[2]` and `alpha`.
///
/// How far short of 0.0/1.0 lightness still counts as "at the extreme"
/// for [`Picker::set_colour`]'s own kept-saturation guard — see that
/// function's own doc for why an exact `> 0.0 && < 1.0` boundary is not
/// enough.
const LIGHTNESS_EXTREME_MARGIN: f32 = 1e-4;

pub struct Picker {
    /// Hue in degrees, saturation and value 0..1 — HSV's own coordinates
    /// and this control's OWN canonical state (module header,
    /// "CANONICAL STATE"). `hsv[1]`/`hsv[2]` are the colour's HSV
    /// saturation and value, not HSL's own saturation — see `hsl_sat`
    /// for that.
    hsv: [f32; 3],
    /// The alpha channel, which is part of the colour and not a knob
    /// beside it (the owner's decision of 2026-08-18).
    alpha: f32,
    /// The HSL saturation the picker is standing on — kept, not
    /// re-derived, the same reason and the same way [`hue`](Self::hue)
    /// is (module header, "CANONICAL STATE"). `hsv_from_hsl` answers 0
    /// at either lightness extreme regardless of what is asked for, so
    /// reading this back out of the colour instead would zero the
    /// Saturation slider's own reading the instant Lightness reached
    /// either end and never give it back on the way out.
    hsl_sat: f32,
    /// Which notation the text side is written in.
    pub format: Format,
    /// The inline editor over the value plate, while one stands open —
    /// module header, "INLINE CLICK-TO-EDIT". `None` the rest of the
    /// time, which is most of the time: a picker spends its life being
    /// dragged and pressed, and only occasionally typed into.
    editing: Option<InputModel>,
}

impl Picker {
    /// A picker opened on a colour.
    pub fn of(c: Color) -> Picker {
        let mut p = Picker {
            hsv: [0.0, 0.0, 0.0],
            alpha: 1.0,
            hsl_sat: 0.0,
            format: Format::Hsl,
            editing: None,
        };
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
    /// reach the control. That is a look, and a look is a token.
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
    /// state keeps the hue it was already standing on — and, the same
    /// way, the HSL saturation is taken from the colour except at either
    /// lightness extreme, which has none to give either.
    ///
    /// THE LIGHTNESS-EXTREME GUARD HAS A MARGIN, NOT A KNIFE EDGE
    /// (2026-08-28, found live a second time): `Picker::oklch`/`set_oklch`
    /// — the exact crossing every theme save and reload goes through — do
    /// not round-trip white back to bit-exact `(1, 1, 1)`; the OKLab
    /// matrix and cube-root arithmetic leaves roughly 1e-7 of chroma
    /// behind, which decodes to something like `(0.99999994, ..., ...)`,
    /// three EQUAL channels a hair short of 1.0. `l < 1.0` on that number
    /// is true, so an exact-boundary guard reads it as "real information"
    /// and overwrites the kept saturation with the 0 three equal channels
    /// always give — reproducing the exact bug this field exists to fix,
    /// through a save/reload instead of a drag. `LIGHTNESS_EXTREME_MARGIN`
    /// is comfortably past that noise (which measures roughly 1e-7) and
    /// nowhere near a lightness a slider or a typed value can mean on
    /// purpose (the plate itself prints lightness to two decimal PERCENT
    /// places — 1e-4 of span).
    pub fn set_colour(&mut self, c: Color) {
        let (r, g, b) = (c.r.clamp(0.0, 1.0), c.g.clamp(0.0, 1.0), c.b.clamp(0.0, 1.0));
        let (h, s, v) = rgb_to_hsv(r, g, b);
        if s > 0.0 {
            self.hsv[0] = h;
        }
        self.hsv[1] = s;
        self.hsv[2] = v;
        self.alpha = c.a.clamp(0.0, 1.0);
        let (_, s_l, l) = rgb_to_hsl(r, g, b);
        if l > LIGHTNESS_EXTREME_MARGIN && l < 1.0 - LIGHTNESS_EXTREME_MARGIN {
            self.hsl_sat = s_l;
        }
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

    /// The hue the picker is standing on, on its own — kept rather than
    /// re-derived from `colour()`, which is this module's header's own
    /// claim made through an accessor.
    pub fn hue(&self) -> f32 {
        self.hsv[0]
    }

    /// The HSL saturation the picker is standing on, kept the same way
    /// [`hue`](Self::hue) is (module header, "CANONICAL STATE").
    pub fn hsl_sat(&self) -> f32 {
        self.hsl_sat
    }

    /// How many sliders `format` offers.
    pub fn slider_count(&self) -> usize {
        self.format.slider_count()
    }

    /// The channel letter over slider `i`.
    pub fn slider_label(&self, i: usize) -> &'static str {
        self.format.slider_label(i)
    }

    /// Channel `i`'s handle position, 0..1 of ITS OWN range — hue 180° is
    /// 0.5 regardless of hue's range being 0..360. Hue and saturation
    /// read the two KEPT fields directly ([`hue`](Self::hue),
    /// [`hsl_sat`](Self::hsl_sat)); lightness is a pure function of
    /// `colour()`, which is always well-defined.
    pub fn slider_at(&self, i: usize) -> f32 {
        match self.format {
            Format::Hsl => {
                let c = self.colour();
                let (_, _, l) = rgb_to_hsl(c.r, c.g, c.b);
                match i {
                    0 => hue_frac(self.hue()),
                    1 => self.hsl_sat,
                    _ => l,
                }
            }
        }
    }

    /// A press or a drag on slider `i`: `frac` 0..1 becomes that
    /// channel's value; every OTHER channel is left exactly as it reads
    /// today. CLAMPED, NEVER REJECTED — `frac` is clamped here once, so a
    /// caller may hand this an overshoot the way a drag off a track's own
    /// edge always can (`wheel_pick`'s rule, carried over from the
    /// wheel).
    pub fn pick_slider(&mut self, i: usize, frac: f32) {
        let frac = frac.clamp(0.0, 1.0);
        match self.format {
            // S and L land on `hsv[1..3]` through `hsv_from_hsl` directly
            // (module header) — hue untouched, and no round trip through
            // full sRGB for a write that never needed one. Saturation
            // ALSO always writes `hsl_sat` outright, the same as hue's
            // own arm does for `hsv[0]` — the hand on THIS slider always
            // wins regardless of where Lightness currently sits.
            // Lightness reads `hsl_sat` back rather than re-deriving it
            // from the colour, which is exactly what breaks at either
            // lightness extreme (module header, "CANONICAL STATE").
            Format::Hsl => match i {
                0 => self.hsv[0] = frac * 360.0,
                1 => {
                    self.hsl_sat = frac;
                    let c = self.colour();
                    let (_, _, l) = rgb_to_hsl(c.r, c.g, c.b);
                    let (sv, v) = hsv_from_hsl(frac, l);
                    self.hsv[1] = sv;
                    self.hsv[2] = v;
                }
                _ => {
                    let (sv, v) = hsv_from_hsl(self.hsl_sat, frac);
                    self.hsv[1] = sv;
                    self.hsv[2] = v;
                }
            },
        }
    }

    /// The colour as text, in the notation in force.
    ///
    /// HUE AND SATURATION ARE THE KEPT ONES, NOT [`write`]'s OWN FRESH
    /// READ (found live, after the fix that keeps them for the SLIDERS
    /// shipped, 2026-08-28): [`write`] takes a plain `Color` and has no
    /// kept state to read, so its own `rgb_to_hsl` answers 0 for either
    /// one at the axis/extreme they have none to give at — correct for a
    /// swatch, which has no hand on it to remember, and WRONG for this
    /// picker's own plate, which would then print "0% saturation" a
    /// breath after the Saturation slider beside it — reading the exact
    /// same field — printed 60%. Lightness alone is `write`'s own fresh
    /// read: it is never degenerate on its own, only H/S's OWN business
    /// is what a Lightness extreme empties out.
    pub fn text(&self) -> String {
        match self.format {
            Format::Hsl => {
                let c = self.colour();
                let (_, _, l) = rgb_to_hsl(c.r, c.g, c.b);
                write_hsl(self.hue().rem_euclid(360.0), self.hsl_sat, l, c.a)
            }
        }
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

    /// Opens the inline editor on the value plate, seeded with what the
    /// plate already shows — the same "seed from the current value"
    /// contract [`InputModel::set_value`] gives any field opened on an
    /// existing one.
    pub fn begin_edit(&mut self) {
        let mut m = InputModel::new();
        m.set_value(&self.text());
        self.editing = Some(m);
    }

    pub fn is_editing(&self) -> bool {
        self.editing.is_some()
    }

    pub fn editing_mut(&mut self) -> Option<&mut InputModel> {
        self.editing.as_mut()
    }

    /// Enter, or a blur: tries to read the typed text through the same
    /// parser a press already trusts ([`Picker::set_text`]), and closes
    /// the editor ONLY ON SUCCESS. On a bad parse the colour is untouched
    /// — `set_text`'s own contract — and the editor is left exactly as
    /// this call found it: OPEN, so Enter's own failure case can leave
    /// the typed text in place for another try, the same way the SAVE AS
    /// prompt's `if name.is_empty() { return KeyOut::Consumed }` leaves
    /// ITS field open rather than silently discarding what was typed. A
    /// caller for whom "no more chances to fix it" is the right rule — a
    /// blur, where focus has already left — forces the close itself with
    /// [`Picker::cancel_edit`] the moment this answers `false`.
    pub fn commit_edit(&mut self) -> bool {
        let Some(text) = self.editing.as_ref().map(|m| m.value().to_string()) else {
            return false;
        };
        if self.set_text(&text) {
            self.editing = None;
            true
        } else {
            false
        }
    }

    /// Escape: discards the typed text and reverts to the colour the
    /// picker already had — nothing here ever touched it.
    pub fn cancel_edit(&mut self) {
        self.editing = None;
    }
}

// -------------------------------------------------------------- geometry

/// Where every part of the control stands, in the caller's coordinates.
#[derive(Clone, Debug)]
pub struct Layout {
    /// One row per channel this notation offers — [`MAX_SLIDERS`], the
    /// same for all six ([`Format::slider_count`]) — vertically centred
    /// in a band that is always that many slots tall (`layout_with`'s
    /// own note), so the control's total height never depends on which
    /// format is showing.
    pub sliders: Vec<Rect>,
    /// The chosen colour over the transparency checker.
    pub patch: Rect,
    /// The plate that names the notation and steps to the next.
    pub format: Rect,
    /// The colour written out — a read-only plate, or an inline text
    /// field while [`Picker::is_editing`].
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
    Slider(usize),
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
    bank_w_frac: f32,
    slider_h: f32,
    slider_gap: f32,
    slider_label_w: f32,
    slider_min_w: f32,
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
        static PAD_X: OnceLock<TokenId> = OnceLock::new();
        static FRAC: OnceLock<TokenId> = OnceLock::new();
        static SLIDER_H: OnceLock<TokenId> = OnceLock::new();
        static SLIDER_GAP: OnceLock<TokenId> = OnceLock::new();
        static SLIDER_LABEL_W: OnceLock<TokenId> = OnceLock::new();
        static SLIDER_MIN_W: OnceLock<TokenId> = OnceLock::new();
        static PATCH_H: OnceLock<TokenId> = OnceLock::new();
        static ROW_H: OnceLock<TokenId> = OnceLock::new();
        static FORMAT_W: OnceLock<TokenId> = OnceLock::new();
        static SWATCH: OnceLock<TokenId> = OnceLock::new();
        static SWATCH_GAP: OnceLock<TokenId> = OnceLock::new();
        static COLS: OnceLock<TokenId> = OnceLock::new();
        static BASE_N: OnceLock<TokenId> = OnceLock::new();
        let t = theme::resolved();
        Metrics {
            gap: t.px(tok(&GAP, "picker.gap")),
            pad_x: t.px(tok(&PAD_X, "picker.pad_x")),
            bank_w_frac: t.px(tok(&FRAC, "picker.bank_w_frac")).clamp(0.1, 0.9),
            slider_h: t.px(tok(&SLIDER_H, "picker.slider_h")),
            slider_gap: t.px(tok(&SLIDER_GAP, "picker.slider_gap")),
            slider_label_w: t.px(tok(&SLIDER_LABEL_W, "picker.slider_label_w")),
            slider_min_w: t.px(tok(&SLIDER_MIN_W, "picker.slider_min_w")),
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
/// too, and this is where it does. The same rule is why RGB's unfilled
/// fourth slot registers nothing at all rather than an empty one
/// (`layout_with`'s own note).
fn offered(wish: f32) -> usize {
    (wish.round().max(0.0) as usize).min(base_ids().len())
}

/// How tall the control stands in a band `w` wide, offering `custom`
/// colours of the caller's own.
///
/// NO SLIDER COUNT HERE, AND THAT IS NOT AN OVERSIGHT: the slider bank's
/// own band is always [`MAX_SLIDERS`] slots tall, whichever format is
/// showing (`layout_with`'s own note), so the control's total height
/// never depends on it. Asked BEFORE the row is laid out and before
/// anything is known about which picker's format is live — this is
/// exactly the structural guarantee that makes that possible.
pub fn height(w: f32, custom: usize) -> f32 {
    let m = Metrics::read();
    layout_with(&m, Rect::new(0.0, 0.0, w, 0.0), MAX_SLIDERS, custom).1
}

/// Where everything stands inside `area`, for a picker offering
/// `slider_count` sliders (`Format::slider_count`) and `custom` colours
/// of its own.
pub fn layout(area: Rect, slider_count: usize, custom: usize) -> Layout {
    let m = Metrics::read();
    layout_with(&m, area, slider_count, custom).0
}

fn layout_with(m: &Metrics, area: Rect, slider_count: usize, custom: usize) -> (Layout, f32) {
    // NOTHING MAY LEAVE THE BAND, AND THE BAND IS THE ONLY NUMBER HERE
    // THAT IS NOT THE THEME'S. Every width below is the theme's wish
    // clamped by the room there is, in that order: a theme says how wide
    // the slider bank ought to be, and only the caller knows how wide the
    // row it stands in turned out. Where they disagree the room wins — a
    // part laid past the band is drawn and PRESSED over whatever is
    // beside it, which is not a look but a fault.
    let band = area.w.max(0.0);
    let left_w = (band * m.bank_w_frac).max(m.slider_min_w).min(band);
    // EVERY SLIDER SPANS THE WHOLE LEFT COLUMN'S OWN WIDTH — there is no
    // inscribed-square collapse mode the way the wheel's box had: a
    // slider that narrows just gets thinner, it never disappears.
    let n = slider_count.min(MAX_SLIDERS);
    // THE RESERVED BAND IS ALWAYS MAX_SLIDERS SLOTS TALL, populated or
    // not — `bank_h` reads no count at all — and the POPULATED rows are
    // centred inside it. That is the whole answer to "the control must
    // not jump around when the format cycles": it is not a policy kept
    // by hand, it is `bank_h` never being a function of `n`.
    let bank_h = MAX_SLIDERS as f32 * m.slider_h + (MAX_SLIDERS as f32 - 1.0) * m.slider_gap;
    let used_h = n as f32 * m.slider_h + n.saturating_sub(1) as f32 * m.slider_gap;
    let y0 = area.y + (bank_h - used_h) / 2.0;
    // The label lane is clamped to `left_w` itself so the track can never
    // go negative-width and the two together can never run past the
    // column they share.
    let label_w = m.slider_label_w.min(left_w);
    let track_x = area.x + label_w;
    let track_w = (left_w - label_w).max(0.0);
    let sliders: Vec<Rect> = (0..n)
        .map(|i| {
            Rect::new(track_x, y0 + i as f32 * (m.slider_h + m.slider_gap), track_w, m.slider_h)
        })
        .collect();
    let rw = (band - left_w - m.gap).max(0.0);
    let rx = (area.x + left_w + m.gap).min(area.x + band);
    let patch = Rect::new(rx, area.y, rw, m.patch_h);
    let mut y = patch.bottom() + m.gap;
    // HOW MANY CELLS THE THEME ASKS FOR, AND HOW MANY THERE IS ROOM FOR.
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
    let strip_y = area.y + bank_h.max(right_h) + m.gap;
    let fmt_w = m.format_w.min(band);
    let format = Rect::new(area.x, strip_y, fmt_w, m.row_h);
    let text = Rect::new(
        (area.x + fmt_w + m.gap).min(area.x + band),
        strip_y,
        (band - fmt_w - m.gap).max(0.0),
        m.row_h,
    );
    (
        Layout { sliders, patch, format, text, base, custom: custom_rects, add },
        text.bottom() - area.y,
    )
}

/// Every part of the control and where it stands, in ONE order.
///
/// The hit test, the focus chain and whatever the application hangs off
/// each part all read this, so a part that is drawn is a part that can be
/// reached. The order is the reading order — the sliders first, then the
/// cells beside them, then the readout strip that runs under both.
pub fn parts(l: &Layout) -> Vec<(Part, Rect)> {
    let mut out: Vec<(Part, Rect)> =
        l.sliders.iter().enumerate().map(|(i, r)| (Part::Slider(i), *r)).collect();
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
/// Clipped to the SAME rounded boundary `r`'s own fill and frame use
/// (2026-08-28) — a plain rectangular clip left the tiles' own square
/// corners showing past the rounded patch drawn over them.
fn checker(ctx: &mut Ctx, r: Rect) {
    static SIZE: OnceLock<TokenId> = OnceLock::new();
    static A: OnceLock<TokenId> = OnceLock::new();
    static B: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let s = t.px(tok(&SIZE, "picker.checker")).max(1.0);
    let (a, b) = (col(t.color(tok(&A, "component.picker.checker_a"))), col(t.color(tok(&B, "component.picker.checker_b"))));
    let (c, seg) = shape(t, r);
    ctx.dl.checker_shape(r, &c, seg, s, a, b);
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

fn two_stops(a: Color, b: Color) -> Vec<(f32, Color)> {
    vec![(0.0, a), (1.0, b)]
}

/// Seven stops at every 60° kink `hsv_to_rgb` has — exact across the
/// whole hue sweep for the SAME reason the old wheel's wedges were
/// (module header, "THE WHEEL IS EXACT" in its own retired form): the
/// function is piecewise affine in hue with a kink at each sector
/// boundary, and `rect_grad` reproduces an affine function exactly
/// inside one band (its own doc).
fn hue_stops(f: impl Fn(f32) -> Color) -> Vec<(f32, Color)> {
    (0..=6).map(|k| ((k as f32 * 60.0 / 360.0).min(1.0), f(k as f32 * 60.0))).collect()
}

/// The full-range gradient slider `i` of `p.format` paints, at every
/// OTHER channel held fixed at what the picker currently reads — so the
/// track keeps answering "what colour is this" while a hand drags it
/// (module header, "THE ONE REAL CONFLICT"). [`draw`] calls this and
/// nothing else decides what a track looks like.
fn slider_stops(p: &Picker, i: usize) -> Vec<(f32, Color)> {
    match p.format {
        Format::Hsl => {
            let c = p.colour();
            let (_, _, l) = rgb_to_hsl(c.r, c.g, c.b);
            // `p.hsl_sat()`, the KEPT saturation, not a fresh
            // `rgb_to_hsl` read — a track painted from the derived one
            // would show a flat, desaturated preview the instant
            // Lightness reached either extreme, the same failure
            // `pick_slider`'s own fix is about.
            match i {
                0 => hue_stops(|h| {
                    let (r, g, b) = hsl_to_rgb(h, p.hsl_sat(), l);
                    Color { r, g, b, a: 1.0 }
                }),
                // Affine at fixed lightness and hue — the same shape as
                // HSV's own saturation axis, read through `hsl_to_rgb`.
                1 => {
                    let (r, g, b) = hsl_to_rgb(p.hue(), 0.0, l);
                    let lo = Color { r, g, b, a: 1.0 };
                    let (r, g, b) = hsl_to_rgb(p.hue(), 1.0, l);
                    two_stops(lo, Color { r, g, b, a: 1.0 })
                }
                // Lightness: `hsl_to_rgb`'s own `c = s·(1 − |2l−1|)` is
                // piecewise-affine in `l` with its one kink at l = 0.5
                // (the cone's own middle), so three stops — not two —
                // are exact.
                _ => [0.0f32, 0.5, 1.0]
                    .into_iter()
                    .map(|l2| {
                        let (r, g, b) = hsl_to_rgb(p.hue(), p.hsl_sat(), l2);
                        (l2, Color { r, g, b, a: 1.0 })
                    })
                    .collect(),
            }
        }
    }
}

/// Draws the whole control. `custom` are the caller's own colours; the
/// picker keeps none of its own, because a swatch a person banked
/// outlives the frame and the control does not. `text_id` is the focus
/// id the value plate registers under, EITHER as a plain plate (the
/// blanket loop in [`draw_focusable`]) OR, while
/// [`Picker::is_editing`], as [`text_input::draw`]'s own registration —
/// see that function's own note on why the two must never both run.
///
/// `p` IS `&mut` even though this is "only drawing": `text_input::draw`
/// mutates its `InputModel` while drawing it (blink phase, horizontal
/// scroll, its own measure cache), and the model lives on the picker.

/// Half the handle's own width — the radius a slider's travel is inset
/// by ([`slider_travel`]), so the round marker [`draw`] paints never
/// reaches past the track it stands in.
fn handle_radius() -> f32 {
    static HANDLE_R: OnceLock<TokenId> = OnceLock::new();
    theme::resolved().px(tok(&HANDLE_R, "picker.handle"))
}

/// A slider's own track `r`, inset by the handle's radius on each side
/// — the range the handle's CENTRE travels across, so its drawn edges
/// land exactly on `r`'s own edges at either extreme instead of
/// overshooting past them (2026-08-28: they used to, which silently
/// swallowed a press on the outer half of the knob at either end — see
/// [`slider_frac`]). The one place this control decides where a
/// slider's ends really are; [`draw`] paints the handle across it and
/// [`slider_frac`] reads a press back through the same inset, so the
/// two stay exact inverses of one another.
fn slider_travel(r: Rect) -> Rect {
    let inset = handle_radius().min(r.w / 2.0);
    Rect::new(r.x + inset, r.y, (r.w - 2.0 * inset).max(0.0), r.h)
}

/// The fraction along slider `i`'s own track a point answers to — the
/// exact inverse of where [`draw`] paints the handle, through the same
/// [`slider_travel`] inset, so a press anywhere on the handle's own
/// drawn face reads back the frac that put it there, all the way to
/// both ends of the track, instead of landing short at either one.
/// Clamped past either end, matching [`Picker::pick_slider`]'s own
/// promise for a drag that outruns the track.
pub fn slider_frac(track: Rect, x: f32) -> f32 {
    let travel = slider_travel(track);
    ((x - travel.x) / travel.w.max(1.0)).clamp(0.0, 1.0)
}

pub fn draw(ctx: &mut Ctx, l: &Layout, p: &mut Picker, custom: &[Color], text_id: FocusId) {
    static ROLE: OnceLock<TokenId> = OnceLock::new();
    static TEXT_INK: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let m = Metrics::read();
    let role = ui::bound_role(&ROLE, "picker.role");
    let px = role.px(ctx, 1.0);
    let font = role.font();
    let track = role.tracking_px(px);
    let fig = role.figures(ctx.fonts, font, px);
    let ink = col(t.color(tok(&TEXT_INK, "component.picker.text")));
    let baseline = |r: &Rect| r.y + (r.h - px * role.leading()) / 2.0;

    // ---- the slider bank: track, frame, knob, letter, one row at a
    // time; [`slider_stops`] is the one statement of what a track means.
    let hr_px = handle_radius();
    for (i, r) in l.sliders.iter().enumerate() {
        let stops = slider_stops(p, i);
        let (c, seg) = shape(t, *r);
        ctx.dl.rect_grad_shape(*r, &stops, 0.0, &c, seg);
        frame(ctx, *r);
        let travel = slider_travel(*r);
        let kx = travel.x + p.slider_at(i).clamp(0.0, 1.0) * travel.w;
        handle(ctx, Rect::new(kx - hr_px, r.y, hr_px * 2.0, r.h));
        let label = Rect::new(r.x - m.slider_label_w, r.y, m.slider_label_w, r.h);
        ctx.dl.text_center(
            ctx.fonts,
            font,
            px,
            label.x + label.w / 2.0,
            baseline(&label),
            p.slider_label(i),
            ink,
            track,
        );
    }

    // ---- the patch, over the chequerboard so alpha is visible.
    checker(ctx, l.patch);
    let (c, seg) = shape(t, l.patch);
    ctx.dl.ring_fill(l.patch, &c, seg, p.colour());
    frame(ctx, l.patch);

    // ---- the notation's name: always the read-only plate.
    let pad = m.pad_x;
    frame(ctx, l.format);
    {
        let inner = (l.format.w - pad * 2.0).max(0.0);
        ctx.dl.push_clip(l.format.x + pad, l.format.y, inner, l.format.h);
        ctx.dl.text_fig(
            ctx.fonts,
            font,
            px,
            l.format.x + pad,
            baseline(&l.format),
            &role.cased(p.format.word()),
            ink,
            track,
            &fig,
        );
        ctx.dl.pop_clip();
    }

    // ---- the value: an INLINE TEXT FIELD while `p.editing` stands,
    // the read-only plate otherwise (module header, "INLINE
    // CLICK-TO-EDIT").
    match p.editing.as_mut() {
        Some(model) => {
            let hover = ctx.mouse.over(l.text);
            text_input::draw(
                ctx,
                l.text,
                model,
                text_id,
                &InputStyle { placeholder: "", hover, disabled: false, focused_fallback: true },
            );
        }
        None => {
            frame(ctx, l.text);
            let inner = (l.text.w - pad * 2.0).max(0.0);
            ctx.dl.push_clip(l.text.x + pad, l.text.y, inner, l.text.h);
            ctx.dl.text_fig(
                ctx.fonts,
                font,
                px,
                l.text.x + pad,
                baseline(&l.text),
                &p.text(),
                ink,
                track,
                &fig,
            );
            ctx.dl.pop_clip();
        }
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
/// and custom cells and the bank button — `write`'s HSL form, which
/// carries alpha too (`swatch`'s own chequerboard says a swatch can hold
/// one). `bases` and `custom` are handed in rather than re-read so a
/// part's value can never disagree with the swatch [`draw`] paints for
/// the same index.
fn part_access(part: Part, p: &Picker, bases: &[Color], custom: &[Color]) -> AccessInfo {
    match part {
        Part::Slider(i) => {
            let (key, fallback) = match p.slider_label(i) {
                "H" => ("catalog.color_picker.channel_hue", "Hue"),
                "S" => ("catalog.color_picker.channel_saturation", "Saturation"),
                "V" => ("catalog.color_picker.channel_value", "Value"),
                "L" => ("catalog.color_picker.channel_lightness", "Lightness"),
                "C" => ("catalog.color_picker.channel_chroma", "Chroma"),
                "R" => ("catalog.color_picker.channel_red", "Red"),
                "G" => ("catalog.color_picker.channel_green", "Green"),
                "B" => ("catalog.color_picker.channel_blue", "Blue"),
                // Unreachable while `Format::slider_label` only ever
                // spells the nine letters above, kept here as the same
                // never-Debug fallback the rest of this file answers a
                // gap with rather than a panic on a user-facing path.
                _ => ("catalog.color_picker.channel_unknown", "Colour channel"),
            };
            AccessInfo::new(Role::Slider, ui::theme_catalog_named(key, fallback).to_string())
                .with_value(format!("{:.0}%", p.slider_at(i) * 100.0))
        }
        Part::Format => AccessInfo::new(
            Role::Button,
            ui::theme_catalog_named("catalog.color_picker.notation", "Colour notation").to_string(),
        )
        .with_value(p.format.word()),
        Part::Text => AccessInfo::new(
            Role::TextInput,
            format!(
                "{} {}",
                p.format.word(),
                ui::theme_catalog_named("catalog.color_picker.value_suffix", "value")
            ),
        )
        .with_value(p.text()),
        Part::Base(i) => {
            let mut info = AccessInfo::new(
                Role::Button,
                ui::theme_catalog_named("catalog.color_picker.preset", "Preset colour").to_string(),
            );
            if let Some(c) = bases.get(i) {
                info = info
                    .with_value(write(*c, Format::Hsl))
                    .with_index(i as u32 + 1, bases.len() as u32);
            }
            info
        }
        Part::Custom(i) => {
            let mut info = AccessInfo::new(
                Role::Button,
                ui::theme_catalog_named("catalog.color_picker.custom", "Custom colour").to_string(),
            );
            if let Some(c) = custom.get(i) {
                info = info
                    .with_value(write(*c, Format::Hsl))
                    .with_index(i as u32 + 1, custom.len() as u32);
            }
            info
        }
        Part::Add => AccessInfo::new(
            Role::Button,
            ui::theme_catalog_named("catalog.color_picker.save", "Save current colour").to_string(),
        )
        .with_value(write(p.colour(), Format::Hsl)),
    }
}

/// [`draw`], joined to the world's focus chain.
///
/// EVERY PART REGISTERS, not just the sliders: a swatch the pointer can
/// press and the keyboard cannot reach is a control that exists for half
/// its users. The caller says what each part's identity is (`id_of`),
/// because an id is a PATH in the application's own tree and this
/// library has no idea where in that tree its picker is standing.
///
/// `Part::Text` IS THE ONE EXCEPTION, and only while [`Picker::is_editing`]:
/// [`text_input::draw`] registers that same focus slot itself, with
/// `Caps::TEXT | GREEDY_ARROWS` rather than this loop's own `Caps::NONE`
/// — two registrations under the SAME id would double the Tab stop, the
/// exact "ghost cell" bug class [`offered`] already fixed once for the
/// ready-made grid (`base_count`/`base_ids`). So this loop skips the text
/// part while editing and [`draw`] registers it once, correctly, itself.
pub fn draw_focusable(
    ctx: &mut Ctx,
    l: &Layout,
    p: &mut Picker,
    custom: &[Color],
    id_of: impl Fn(Part) -> FocusId,
) {
    // Read once, for every base cell alike: the same rule `layout_with`
    // itself follows (`Metrics` read once and passed around), so a
    // re-bake mid-loop cannot leave one cell's report disagreeing with
    // the swatch [`draw`] paints beside it.
    let bases = base_colours();
    let editing = p.is_editing();
    let rings: Vec<(Rect, bool)> = parts(l)
        .into_iter()
        .filter(|(part, _)| !(editing && *part == Part::Text))
        .map(|(part, r)| {
            let access = part_access(part, p, &bases, custom);
            let f = ctx.focus.as_deref_mut().map(|fc| fc.register(id_of(part), r, Caps::NONE, access));
            (r, f.map_or(false, |f| f.ring))
        })
        .collect();
    draw(ctx, l, p, custom, id_of(Part::Text));
    // The rings go on TOP of the whole control, not each beside its own
    // part: a ring drawn before the patch beside it would be painted over
    // by it.
    for (r, on) in rings {
        focus_ring::draw_faded(ctx, r, on);
    }
}

#[cfg(test)]
mod tests {
    //! The model's promises, and one of them is still a scar.

    use super::*;
    use crate::draw::{DrawCmd, DrawList};
    use crate::focus::FocusCtl;
    use crate::font::FontSystem;
    use crate::pointer::Pointer;

    fn approx(a: f32, b: f32, eps: f32, what: &str) {
        assert!((a - b).abs() <= eps, "{what}: {a} vs {b} (eps {eps})");
    }

    fn approx_color(a: Color, b: Color, eps: f32, what: &str) {
        approx(a.r, b.r, eps, &format!("{what} red"));
        approx(a.g, b.g, eps, &format!("{what} green"));
        approx(a.b, b.b, eps, &format!("{what} blue"));
        approx(a.a, b.a, eps, &format!("{what} alpha"));
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
    fn changing_the_notation_changes_the_spelling_and_not_the_colour() {
        //! Only HSL stands now, so "changes" is a claim about a full trip
        //! round the ring being a no-op — the same invariant this test
        //! pinned when six notations shared the ring, kept because the
        //! canonical state's own promise (module header, "ONE CHANGE OF
        //! CANONICAL STATE") did not change with their removal.
        let mut p = Picker::of(Color::rgba8(0x3F, 0xE3, 0xAE, 0xCC));
        let before = p.colour();
        p.cycle_format();
        assert_eq!(p.colour(), before, "the notation moved the colour");
        assert_eq!(p.format, Format::Hsl, "the one notation stayed itself");
        let s = write(before, Format::Hsl);
        assert!(s.starts_with("hsl"), "HSL must announce itself: {s}");
        let back = parse(&s, Format::Hsl).unwrap_or_else(|| panic!("HSL cannot read {s}"));
        for (a, b, ch) in
            [(back.r, before.r, 'r'), (back.g, before.g, 'g'), (back.b, before.b, 'b'), (back.a, before.a, 'a')]
        {
            approx(a, b, 1e-3, &format!("channel {ch}"));
        }
    }

    #[test]
    fn the_alpha_of_a_typed_hsl_value_reaches_the_theme() {
        // hsl(160.61, 74.55%, 56.86%) is rgb8(0x3F, 0xE3, 0xAE); 0.502 is
        // 0x80/255 — the same fixture and alpha byte this test used
        // while ARGB still existed, spelled in the notation left.
        let mut p = Picker::of(Color::WHITE);
        assert!(p.set_text("hsl(160.61, 74.55%, 56.86% / 0.502)"));
        let lit = theme::edit::oklch_literal(p.oklch());
        assert!(
            lit.contains(" / 0.502"),
            "the alpha must cross into the theme's own spelling: {lit}"
        );
        let mut q = Picker::of(Color::WHITE);
        assert!(q.set_text("hsl(160.61, 74.55%, 56.86%)"));
        let opaque = theme::edit::oklch_literal(q.oklch());
        assert!(!opaque.contains('/'), "an opaque colour writes no alpha: {opaque}");
        approx(p.oklch().l, q.oklch().l, 1e-4, "alpha must not move lightness");
    }

    #[test]
    fn the_notation_survives_twenty_round_trips() {
        //! THE SCAR. `.gap-program/obalone-naprawy.md` and the head of
        //! this file record what happened when a crossing to OKLCh
        //! skipped the decode: the editor seeded itself from what it had
        //! just written, so the accent's lightness climbed 0.8200 ->
        //! 0.8904 -> 0.9413 -> 0.9715 with nobody touching a control.
        //! Twenty trips is far past where that was already obvious.
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
        // The same claim through the picker's own TYPED road — HSL now,
        // since OKLCh no longer has a notation of its own to type into.
        let mut q = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        let first_l = q.oklch().l;
        for i in 0..20 {
            let s = q.text();
            assert!(q.set_text(&s), "trip {i} wrote a value it cannot read: {s}");
        }
        approx(q.oklch().l, first_l, 3e-3, "lightness after twenty written trips");
    }

    #[test]
    fn format_all_starts_and_stays_on_hsl() {
        assert_eq!(Format::ALL[0], Format::Hsl, "HSL is the one notation there is");
        assert_eq!(Picker::of(Color::BLACK).format, Format::Hsl, "Picker::of seeds HSL");
        assert_eq!(Picker::at_rest().format, Format::Hsl, "at_rest inherits Picker::of's seed");
        for f in Format::ALL {
            assert_eq!(f.slider_count(), MAX_SLIDERS, "{f:?} offers the wrong slider count");
        }
    }

    #[test]
    fn hsv_from_hsl_matches_the_algebra_hsl_to_rgb_used_to_inline() {
        for &s in &[0.0f32, 0.3, 0.8, 1.0] {
            for &l in &[0.0f32, 0.25, 0.5, 0.75, 1.0] {
                let (sv, v) = hsv_from_hsl(s, l);
                let want_v = l + s * l.min(1.0 - l);
                let want_sv = if want_v <= 0.0 { 0.0 } else { 2.0 * (1.0 - l / want_v) };
                approx(v, want_v, 1e-6, "v");
                approx(sv, want_sv, 1e-6, "sv");
                let (r1, g1, b1) = hsv_to_rgb(123.0, sv, v);
                let (r2, g2, b2) = hsl_to_rgb(123.0, s, l);
                approx(r1, r2, 1e-6, "r");
                approx(g1, g2, 1e-6, "g");
                approx(b1, b2, 1e-6, "b");
            }
        }
    }

    #[test]
    fn a_drag_onto_the_grey_axis_keeps_the_hue_it_came_from() {
        // The dead centre of the old wheel is the saturation SLIDER'S
        // OWN ZERO now — the same point on a different control, driven
        // through `pick_slider` and `Picker::hue` rather than through
        // `wheel_pick`, which is gone with the wheel.
        let mut p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        p.format = Format::Hsl;
        let hue = p.hue();
        p.pick_slider(1, 0.0); // saturation to nothing
        assert_eq!(p.colour().r, p.colour().b, "zero saturation is grey");
        approx(p.hue(), hue, 1e-6, "the hue stayed where the hand left it");
        // AND IT SURVIVES A RE-SEED, which is the road this actually
        // happens on: the editor reads the theme back into its controls
        // on every visit, and a grey read back in has no hue to give.
        let grey = p.colour();
        p.set_colour(grey);
        approx(p.hue(), hue, 1e-6, "a re-seed off a grey kept the hue");
        // Coming back off zero, at the SAME hue and full saturation,
        // returns the same hue.
        p.pick_slider(1, 1.0);
        approx(p.hue(), hue, 1e-3, "hue returned off the grey axis");
    }

    #[test]
    fn a_drag_onto_either_lightness_extreme_keeps_the_saturation_it_came_from() {
        //! Found LIVE, after this file already shipped: dragging
        //! Lightness to 0 % or 100 % snapped the Saturation slider to
        //! 0 % — `hsv_from_hsl` answers saturation 0 at EITHER extreme
        //! regardless of what is asked for (black and white have none
        //! to give, by construction), and a picker that re-derived HSL
        //! saturation from the colour every frame had nowhere else to
        //! read it from. The exact hue-axis shape of the test above,
        //! over the OTHER kept field ([`Picker::hsl_sat`]).
        let mut p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        let sat = p.hsl_sat();
        assert!(sat > 0.1, "the fixture must start with real saturation: {sat}");

        p.pick_slider(2, 0.0); // lightness to black
        assert_eq!(p.colour(), Color::BLACK, "zero lightness is black");
        approx(p.hsl_sat(), sat, 1e-6, "saturation stayed where the hand left it, at black");
        approx(p.slider_at(1), sat, 1e-6, "the Saturation slider itself did not snap to 0%");

        // AND IT SURVIVES A RE-SEED, the same road the hue test's own
        // note names: the editor reads the theme back into its controls
        // on every visit, and black read back in has no saturation to
        // give.
        let black = p.colour();
        p.set_colour(black);
        approx(p.hsl_sat(), sat, 1e-6, "a re-seed off black kept the saturation");

        // Coming back off black, at the SAME saturation, returns the
        // same colour the picker started from.
        p.pick_slider(2, 0.5);
        approx(p.hsl_sat(), sat, 1e-3, "saturation returned off the lightness extreme");
        let (r, g, b) = hsl_to_rgb(p.hue(), sat, 0.5);
        approx_color(p.colour(), Color { r, g, b, a: 1.0 }, 2e-3, "the original hue and saturation came back");

        // The other extreme, white, the identical claim.
        p.pick_slider(2, 1.0);
        assert_eq!(p.colour(), Color::WHITE, "full lightness is white");
        approx(p.hsl_sat(), sat, 1e-6, "saturation stayed where the hand left it, at white");
        approx(p.slider_at(1), sat, 1e-6, "the Saturation slider itself did not snap to 0%, at white");
    }

    #[test]
    fn saturation_survives_a_theme_reseed_at_white_despite_oklch_round_trip_noise() {
        //! Found LIVE a second time: `Picker::oklch`/`set_oklch` — the
        //! exact crossing every theme save and reload goes through — do
        //! not round-trip white back to bit-exact `(1, 1, 1)`; the OKLab
        //! matrix and cube-root arithmetic leaves roughly 1e-7 of chroma
        //! behind. An exact `l < 1.0` boundary on the resulting
        //! `0.99999994` reads that residue as real information and wipes
        //! the kept saturation the direct-drag test above already proves
        //! survives — reproducing the SAME bug through a save/reload
        //! instead of a drag. `LIGHTNESS_EXTREME_MARGIN` is the fix.
        for base in [
            Color::rgb8(0x80, 0x20, 0x20),
            Color::rgb8(0x20, 0x80, 0x20),
            Color::rgb8(0x20, 0x20, 0x80),
            Color::rgb8(0xC0, 0x90, 0x30),
        ] {
            let mut p = Picker::of(base);
            let sat = p.hsl_sat();
            assert!(sat > 0.1, "the fixture must start with real saturation: {sat}");

            p.pick_slider(2, 1.0); // white, through the real slider path
            assert_eq!(p.colour(), Color::WHITE);
            approx(p.hsl_sat(), sat, 1e-6, "the direct drag alone kept it");

            // The exact crossing `Settings::seed_editor_from_theme` takes
            // on every visit (module header): read the theme's own
            // number back into the control.
            let reseeded = p.oklch();
            p.set_oklch(reseeded);
            approx(p.hsl_sat(), sat, 1e-4, "a theme reseed at white must not wipe the saturation");

            // And the colour it returns to off white is the ORIGINAL
            // one, not a grey the noise quietly substituted for it.
            p.pick_slider(2, 0.5);
            let (r, g, b) = hsl_to_rgb(p.hue(), sat, 0.5);
            approx_color(p.colour(), Color { r, g, b, a: 1.0 }, 5e-3, "the reseed did not turn the colour grey");
        }
    }

    #[test]
    fn the_value_plate_names_the_same_saturation_the_slider_shows_at_either_extreme() {
        //! Found alongside the reseed bug above: [`write`] takes a plain
        //! `Color` and has no kept state to read, so before this fix
        //! `Picker::text` printed 0 % saturation at either lightness
        //! extreme — the Saturation slider's own reading (`slider_at`)
        //! disagreeing with the very value plate beside it, and with
        //! `Part::Text`'s accessible value, which a screen reader reads
        //! aloud, and with `begin_edit`'s own seed for the inline editor.
        let mut p = Picker::of(Color::rgb8(0x80, 0x20, 0x20));
        let sat_pct = format!("{:.2}", p.hsl_sat() * 100.0);
        for l in [0.0f32, 1.0] {
            p.pick_slider(2, l);
            assert!(
                p.text().contains(&sat_pct),
                "the plate ({}) must name the same saturation the slider shows ({sat_pct}%) at l={l}",
                p.text()
            );
        }
    }

    #[test]
    fn a_hue_dragged_to_the_tracks_own_end_reads_back_as_the_tracks_own_end() {
        //! `pick_slider(0, 1.0)` writes `360.0` verbatim (the module's
        //! own doc on [`Picker::pick_slider`]: "CLAMPED, NEVER
        //! REJECTED"). Reading it back with a plain `rem_euclid(360.0)`
        //! answers `0.0` — correct for the COLOUR, since hue 0 and hue
        //! 360 are the same point, but wrong for the HANDLE, which
        //! teleported across the whole track the instant a press
        //! reached its own right edge (2026-08-28's fix, `hue_frac`).
        let mut p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        p.pick_slider(0, 1.0);
        approx(p.hue(), 360.0, 1e-6, "the stored hue is the literal 360");
        approx(p.slider_at(0), 1.0, 1e-6, "the handle reads back at the track's own end");
    }

    #[test]
    fn slider_at_and_pick_slider_are_inverses_within_a_format() {
        //! `pick_slider` places a value, `slider_at` reads it back — a
        //! knob that drifted from the point a press landed on would be
        //! lying about where it is standing, one drag at a time.
        //!
        //! SLIDER 1 IS CHECKED TWICE, AND FOR A REASON: since `hsl_sat`
        //! was added, `pick_slider(1, frac)` writes it directly and
        //! `slider_at(1)` reads it directly, so comparing the two alone
        //! is true by construction and would no longer notice a broken
        //! `hsv_from_hsl` saturation axis (found in the fix's own
        //! adversarial review, 2026-08-28). The second assertion re-reads
        //! saturation independently, straight off the COLOUR the drag
        //! actually produced, the same road every other slider here is
        //! already checked by.
        let base = Color::rgb8(0x3F, 0xE3, 0xAE);
        let fracs = [0.0f32, 0.1, 0.33, 0.5, 0.75, 0.999];
        for i in 0..Format::Hsl.slider_count() {
            let mut p = Picker::of(base);
            for &frac in &fracs {
                p.pick_slider(i, frac);
                approx(p.slider_at(i), frac, 1e-3, &format!("HSL slider {i} at frac {frac}"));
                if i == 1 {
                    let c = p.colour();
                    let (_, real_s, _) = rgb_to_hsl(c.r, c.g, c.b);
                    approx(real_s, frac, 1e-3, &format!("HSL slider 1's own colour at frac {frac}"));
                }
            }
        }
    }

    #[test]
    fn the_hue_slider_is_exact_at_every_sixty_degree_kink() {
        // Four saturated, non-grey fixtures, spanning different levels of
        // HSL saturation and lightness: `rgb_to_hsl` on each hands
        // `slider_stops` a distinct (s, l) to hold fixed across the sweep.
        for base in [
            Color::rgb8(0x80, 0x40, 0x40),
            Color::rgb8(0xE0, 0x60, 0x20),
            Color::rgb8(0x30, 0x90, 0x30),
            Color::rgb8(0xC0, 0xC0, 0x80),
        ] {
            let p = Picker::of(base);
            let c = p.colour();
            let (_, s_l, l) = rgb_to_hsl(c.r, c.g, c.b);
            let stops = slider_stops(&p, 0);
            assert_eq!(stops.len(), 7, "six kinks, seven stops");
            for (k, (t, col)) in stops.iter().enumerate() {
                let h = k as f32 * 60.0;
                approx(*t, h / 360.0, 1e-6, "stop position");
                let (r, g, b) = hsl_to_rgb(h, s_l, l);
                approx_color(*col, Color { r, g, b, a: 1.0 }, 1e-5, &format!("kink {k}"));
            }
        }
    }

    #[test]
    fn the_saturation_and_lightness_sliders_are_the_lines_the_header_states() {
        //! The state is reached through `pick_slider` itself — hue, then
        //! lightness, then saturation, in that order — rather than by
        //! poking `hsv` directly (the old HSV version of this test's own
        //! trick): HSL is not the identity mapping onto the canonical
        //! state the way HSV is, so a raw `hsv` write would set HSV
        //! coordinates, not the HSL ones this test means to ask for.
        for &h in &[0.0f32, 95.0, 210.0, 359.0] {
            for &s in &[0.0f32, 0.35, 1.0] {
                for &l in &[0.1f32, 0.5, 0.9] {
                    let mut p = Picker::of(Color::BLACK);
                    p.pick_slider(0, h / 360.0);
                    p.pick_slider(2, l);
                    p.pick_slider(1, s);
                    let sat = slider_stops(&p, 1);
                    let (r, g, b) = hsl_to_rgb(h, 0.0, l);
                    approx_color(sat[0].1, Color { r, g, b, a: 1.0 }, 1e-3, "s=0");
                    let (r, g, b) = hsl_to_rgb(h, 1.0, l);
                    approx_color(sat[1].1, Color { r, g, b, a: 1.0 }, 1e-3, "s=1");
                    let light = slider_stops(&p, 2);
                    assert_eq!(light.len(), 3, "lightness has a kink at 0.5, three stops");
                    approx_color(light[0].1, Color::BLACK, 1e-6, "l=0 is black");
                    approx_color(light[2].1, Color::WHITE, 1e-6, "l=1 is white");
                    let (r, g, b) = hsl_to_rgb(h, s, 0.5);
                    approx_color(light[1].1, Color { r, g, b, a: 1.0 }, 1e-3, "l=0.5 is the kink");
                }
            }
        }
    }

    #[test]
    fn each_sliders_track_is_the_gradient_slider_stops_states() {
        let mut fonts = FontSystem::new();
        for fmt in Format::ALL {
            let mut dl = DrawList::recording();
            let mut p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
            p.format = fmt;
            let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), p.slider_count(), 0);
            draw(&mut probe(&mut dl, &mut fonts), &l, &mut p, &[], FocusId::of("test"));
            let grads: Vec<Vec<(f32, Color)>> = dl
                .cmds()
                .iter()
                .filter_map(|c| match c {
                    DrawCmd::RectGradShape { stops, .. } => Some(stops.clone()),
                    _ => None,
                })
                .collect();
            assert_eq!(grads.len(), p.slider_count(), "{fmt:?}: one gradient per slider");
            for (i, got) in grads.iter().enumerate() {
                let want = slider_stops(&p, i);
                assert_eq!(got.len(), want.len(), "{fmt:?} slider {i} stop count");
                for (a, b) in got.iter().zip(want.iter()) {
                    approx(a.0, b.0, 1e-5, &format!("{fmt:?} slider {i} stop t"));
                    approx_color(a.1, b.1, 1e-5, &format!("{fmt:?} slider {i} stop colour"));
                }
            }
        }
    }

    #[test]
    fn the_controls_total_height_does_not_depend_on_how_many_sliders_are_populated() {
        //! The structural claim `layout_with`'s own note makes: `bank_h`
        //! reads no slider count, so `height` never needs one either.
        let m = Metrics::read();
        for w in [520.0f32, 260.0, 100.0] {
            for custom in [0usize, 5] {
                let a = layout_with(&m, Rect::new(0.0, 0.0, w, 0.0), 2, custom).1;
                let b = layout_with(&m, Rect::new(0.0, 0.0, w, 0.0), MAX_SLIDERS, custom).1;
                approx(a, b, 1e-4, &format!("height at w={w} custom={custom}"));
            }
        }
    }

    #[test]
    fn the_layout_reserves_exactly_the_height_it_reports() {
        let area = Rect::new(30.0, 40.0, 520.0, 0.0);
        for custom in [0usize, 1, 7, 8, 17] {
            let l = layout(area, MAX_SLIDERS, custom);
            let h = height(area.w, custom);
            let low = l
                .base
                .iter()
                .chain(l.custom.iter())
                .chain(l.sliders.iter())
                .chain([l.patch, l.format, l.text, l.add].iter())
                .fold(area.y, |acc, r| acc.max(r.bottom()));
            approx(h, low - area.y, 0.51, "the reported height covers every part");
        }
    }

    #[test]
    fn nothing_is_laid_outside_the_band_it_was_given() {
        for custom in [0usize, 1, 7, 8, 17] {
            for slider_count in [3usize, 4] {
                for w in [520.0f32, 400.0, 300.0, 260.0, 200.0, 150.0, 100.0, 60.0, 30.0, 20.0, 0.0] {
                    let area = Rect::new(30.0, 40.0, w, 0.0);
                    let l = layout(area, slider_count, custom);
                    for (part, r) in parts(&l) {
                        assert!(
                            r.x >= area.x - 0.01 && r.right() <= area.x + area.w + 0.01,
                            "{part:?} runs past the band at width {w}: {} .. {} against {} .. {}",
                            r.x,
                            r.right(),
                            area.x,
                            area.x + area.w
                        );
                        assert!(r.w >= 0.0 && r.h >= 0.0, "{part:?} has a negative side at {w}");
                    }
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
    }

    #[test]
    fn a_press_anywhere_on_the_drawn_handle_reaches_the_extreme_it_stands_on() {
        //! `draw`'s handle used to be painted `handle_radius()` past
        //! `l.sliders[i]`'s own edges at either extreme — the exact
        //! rect [`hit`]/[`parts`] register — so a press on the outer
        //! half of the visible knob at 0% or 100% landed on nothing
        //! (2026-08-28's fix). `slider_frac` is the inverse of the SAME
        //! inset `draw` now paints the handle through, so a press
        //! anywhere inside the registered rect — including its own bare
        //! edges, which is where the knob's outer half now actually
        //! sits — must clamp to the extreme the handle is drawn at, not
        //! fall short of it.
        let r = Rect::new(100.0, 0.0, 300.0, 20.0);
        approx(slider_frac(r, r.x), 0.0, 1e-6, "the track's own left edge is 0%");
        approx(slider_frac(r, r.right()), 1.0, 1e-6, "the track's own right edge is 100%");
        approx(slider_frac(r, r.x - 50.0), 0.0, 1e-6, "past the left edge still clamps to 0%");
        approx(slider_frac(r, r.right() + 50.0), 1.0, 1e-6, "past the right edge still clamps to 100%");
        approx(slider_frac(r, r.x + r.w / 2.0), 0.5, 1e-6, "the midpoint is untouched by the inset");
    }

    #[test]
    fn the_handle_never_draws_past_the_track_that_is_hit_tested_for_it() {
        //! The structural claim behind the fix above, checked against
        //! the real paint: at every extreme, the handle's own outer
        //! edge — `kx ± handle_radius()` — must land ON `l.sliders[i]`'s
        //! edge, never beyond it, since that rect is exactly what
        //! `parts`/`hit` register as pressable.
        let mut fonts = FontSystem::new();
        let mut p = Picker::of(Color::rgb8(0xFF, 0x00, 0x00)); // full-scale hue and lightness-adjacent
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), p.slider_count(), 0);
        let hr = handle_radius();
        for i in 0..l.sliders.len() {
            let r = l.sliders[i];
            for frac in [0.0f32, 1.0] {
                p.pick_slider(i, frac);
                let mut dl = DrawList::recording();
                draw(&mut probe(&mut dl, &mut fonts), &l, &mut p, &[], FocusId::of("test"));
                let handles: Vec<[f32; 4]> = dl
                    .cmds()
                    .iter()
                    .filter_map(|c| match c {
                        DrawCmd::Ring { r: hr_rect, .. }
                            if (hr_rect[1] - r.y).abs() < 1e-3
                                && (hr_rect[3] - r.h).abs() < 1e-3
                                && hr_rect[2] < r.w
                                && hr_rect[0] >= r.x - hr - 1.0
                                && hr_rect[0] + hr_rect[2] <= r.right() + hr + 1.0 =>
                        {
                            Some(*hr_rect)
                        }
                        _ => None,
                    })
                    .collect();
                let (hx, hw) = handles
                    .last()
                    .map(|h| (h[0], h[2]))
                    .unwrap_or_else(|| panic!("slider {i} painted no handle at frac {frac}"));
                approx(hw, hr * 2.0, 0.5, &format!("slider {i} handle width at frac {frac}"));
                if frac == 0.0 {
                    approx(hx, r.x, 0.5, &format!("slider {i} handle's left edge at frac 0"));
                } else {
                    approx(hx + hw, r.right(), 0.5, &format!("slider {i} handle's right edge at frac 1"));
                }
            }
        }
    }

    #[test]
    fn the_readout_holds_the_notation_this_file_calls_mandatory() {
        let mut fonts = FontSystem::new();
        let longest: Vec<String> = Format::ALL
            .iter()
            .map(|f| write(Color { r: 0.7333, g: 0.2667, b: 0.9333, a: 0.502 }, *f))
            .collect();
        let need = longest.iter().map(|s| readout_px(&mut fonts, s)).fold(0.0f32, f32::max);
        let pad = Metrics::read().pad_x;
        for w in [520.0f32, 460.0, 400.0] {
            let l = layout(Rect::new(0.0, 0.0, w, 0.0), MAX_SLIDERS, 3);
            assert!(
                l.text.w - pad * 2.0 >= need,
                "the readout is {} px wide inside its padding at band {w}, \
                 and the longest value this control writes is {need} px: {longest:?}",
                l.text.w - pad * 2.0
            );
        }
        let word = Format::ALL.iter().map(|f| readout_px(&mut fonts, f.word())).fold(0.0f32, f32::max);
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), MAX_SLIDERS, 3);
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
        //! `[button] pad_x`). Checked against `DrawCmd::Text`'s own
        //! LEFT-anchored runs only — the slider letters are
        //! `text_center` runs (a different anchor, checked nowhere here)
        //! and would otherwise be indistinguishable rects in the same
        //! command stream.
        let pad = Metrics::read().pad_x;
        assert!(pad > 0.0, "the master gives the plates an inset");
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::recording();
        let mut p = Picker::of(Color::rgba8(0x3F, 0xE3, 0xAE, 0xCC));
        let l = layout(Rect::new(30.0, 40.0, 520.0, 0.0), p.slider_count(), 2);
        draw(&mut probe(&mut dl, &mut fonts), &l, &mut p, &[Color::WHITE, Color::BLACK], FocusId::of("test"));
        let left_runs: Vec<[f32; 2]> = dl
            .cmds()
            .iter()
            .filter_map(|c| match c {
                DrawCmd::Text { at, anchor: crate::draw::TextAnchor::Left, .. } => Some(*at),
                _ => None,
            })
            .collect();
        assert_eq!(left_runs.len(), 2, "the two plates write one LEFT-anchored run each");
        for (at, plate) in left_runs.iter().zip([l.format, l.text]) {
            approx(at[0], plate.x + pad, 1e-4, "the ink starts a padding in");
        }
    }

    #[test]
    fn the_grid_lays_no_cell_it_has_no_colour_for() {
        let asked = offered(BASE_SEARCH as f32);
        assert_eq!(asked, base_ids().len(), "the wish is floored by what exists");
        assert!(asked > 0, "the master declares a grid");
        let mut m = Metrics::read();
        m.base_count = asked;
        let (l, _) = layout_with(&m, Rect::new(0.0, 0.0, 520.0, 0.0), MAX_SLIDERS, 0);
        assert_eq!(l.base.len(), base_colours().len(), "the grid lays exactly as many cells as it has colours");
        for (i, id) in base_ids().iter().enumerate() {
            assert_eq!(Some(*id), theme::id(&format!("picker.base.{}", i + 1)), "cell {i} is base.{}", i + 1);
        }
    }

    #[test]
    fn every_part_of_the_control_answers_for_itself() {
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), MAX_SLIDERS, 3);
        let mid = |r: Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
        for i in 0..l.sliders.len() {
            let (x, y) = mid(l.sliders[i]);
            assert_eq!(hit(&l, x, y), Some(Part::Slider(i)));
        }
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
        assert_eq!(hit(&l, -5.0, -5.0), None);
    }

    #[test]
    fn the_ready_made_colours_come_from_the_theme() {
        let base = base_colours();
        assert!(!base.is_empty(), "the master declares a grid");
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
    fn registered_access(l: &Layout, p: &mut Picker, custom: &[Color]) -> Vec<(Part, AccessInfo)> {
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
    fn begin_edit_seeds_from_the_plate_and_commit_reads_it_back() {
        let mut p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        assert!(!p.is_editing());
        p.begin_edit();
        assert!(p.is_editing());
        let seeded = p.text();
        assert_eq!(p.editing_mut().unwrap().value(), seeded);
        // Type a new, valid value over the seeded one — pure green.
        p.editing_mut().unwrap().set_value("hsl(120, 100%, 50%)");
        assert!(p.commit_edit(), "a good value must commit");
        assert!(!p.is_editing(), "commit closes the editor on success");
        assert_eq!((q8(p.colour().r), q8(p.colour().g), q8(p.colour().b)), (0x00, 0xFF, 0x00));
    }

    #[test]
    fn commit_edit_on_a_bad_parse_stays_open_and_keeps_the_colour() {
        let mut p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        let before = p.colour();
        p.begin_edit();
        p.editing_mut().unwrap().set_value("not a colour");
        assert!(!p.commit_edit(), "a bad parse must not commit");
        assert!(p.is_editing(), "a bad parse leaves the editor OPEN");
        assert_eq!(p.editing_mut().unwrap().value(), "not a colour", "the typed text is untouched");
        assert_eq!(p.colour(), before, "a bad parse never destroys the good value");
    }

    #[test]
    fn cancel_edit_discards_the_typed_text() {
        let mut p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        let before = p.colour();
        p.begin_edit();
        p.editing_mut().unwrap().set_value("hsl(0, 0%, 0%)");
        p.cancel_edit();
        assert!(!p.is_editing());
        assert_eq!(p.colour(), before, "cancel never touched the colour");
    }

    #[test]
    fn draw_focusable_does_not_double_register_the_text_plate_while_editing() {
        //! The mechanical hazard `draw_focusable`'s own doc names:
        //! `text_input::draw` registers `Part::Text`'s id itself while
        //! editing, so the blanket loop must not register it a second
        //! time under the same id — that would double the Tab stop, the
        //! "ghost cell" bug class again.
        use crate::focus::FocusCtl;
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::recording();
        let mut p = Picker::of(Color::rgb8(0x3F, 0xE3, 0xAE));
        p.begin_edit();
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), p.slider_count(), 0);
        let mut fc = FocusCtl::new();
        {
            let mut ctx = probe(&mut dl, &mut fonts);
            ctx.focus = Some(&mut fc);
            draw_focusable(&mut ctx, &l, &mut p, &[], |part| FocusId::of("t").item(match part {
                Part::Slider(i) => i,
                Part::Format => 100,
                Part::Text => 101,
                Part::Base(i) => 200 + i,
                Part::Custom(i) => 300 + i,
                Part::Add => 400,
            }));
        }
        // `rect_of` reads the LAST COMPLETED frame; promote this one.
        fc.begin_frame();
        // The text id was registered exactly once (by `text_input::draw`
        // itself) — not zero (unreachable by keyboard) and not twice.
        let r = fc
            .rect_of(FocusId::of("t").item(101))
            .expect("the text id must resolve to a rect while editing");
        approx(r.x, l.text.x, 1e-4, "registered rect x");
        approx(r.y, l.text.y, 1e-4, "registered rect y");
        approx(r.w, l.text.w, 1e-4, "registered rect w");
        approx(r.h, l.text.h, 1e-4, "registered rect h");
    }

    #[test]
    fn every_part_reports_a_human_name_and_not_its_debug_variant() {
        //! The foundation pass placeholdered `AccessInfo::new(role,
        //! format!("{part:?}"))` at every registration — a screen reader
        //! would have read out `"Slider(1)"` and `"Custom(1)"`. This is
        //! the fix, checked both ways: NO part answers with its own
        //! Debug spelling, and the parts a person actually reaches for
        //! answer with the word this control's own doc comments already
        //! use for them.
        let mut p = Picker::of(Color::rgba8(0x3F, 0xE3, 0xAE, 0xCC));
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), p.slider_count(), 2);
        let custom = [Color::WHITE, Color::BLACK];
        let by_part = registered_access(&l, &mut p, &custom);
        for (part, info) in &by_part {
            assert_ne!(
                info.name,
                format!("{part:?}"),
                "{part:?} still reports its Debug-derived identity, not a name"
            );
        }
        let name_of =
            |want: Part| by_part.iter().find(|(part, _)| *part == want).unwrap().1.name.clone();
        // HSL is the one notation there is, so the bank's three sliders
        // are hue, saturation, lightness in that order —
        // `Format::slider_label`'s own order.
        assert_eq!(name_of(Part::Slider(0)), "Hue");
        assert_eq!(name_of(Part::Slider(1)), "Saturation");
        assert_eq!(name_of(Part::Slider(2)), "Lightness");
        assert_eq!(name_of(Part::Format), "Colour notation");
        assert_eq!(name_of(Part::Text), format!("{} value", p.format.word()));
        assert_eq!(name_of(Part::Base(0)), "Preset colour");
        assert_eq!(name_of(Part::Custom(1)), "Custom colour");
        assert_eq!(name_of(Part::Add), "Save current colour");
    }

    #[test]
    fn each_part_reports_the_role_it_actually_plays() {
        let mut p = Picker::of(Color::rgba8(0x3F, 0xE3, 0xAE, 0xCC));
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), p.slider_count(), 2);
        let custom = [Color::WHITE, Color::BLACK];
        let by_part = registered_access(&l, &mut p, &custom);
        for (part, info) in &by_part {
            let want = match part {
                // Dragged, which is what a bridge means by `Role::Slider`.
                Part::Slider(_) => Role::Slider,
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
        let mut p = Picker::of(Color::rgba8(0x3F, 0xE3, 0xAE, 0xCC));
        // Off the construction defaults, so a test that read back a
        // stale placeholder value would not pass by accident.
        p.pick_slider(0, 0.7);
        p.pick_slider(1, 0.2);
        p.pick_slider(2, 0.35);
        let l = layout(Rect::new(0.0, 0.0, 520.0, 0.0), p.slider_count(), 2);
        let custom = [Color::WHITE, Color::BLACK];
        let by_part = registered_access(&l, &mut p, &custom);
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

        // Each slider's own reading, off the same accessor the control
        // itself drags by — the test proves the report tracks a moved
        // slider, not that it can transcribe one fixed number.
        for i in 0..p.slider_count() {
            assert_eq!(value_of(Part::Slider(i)), format!("{:.0}%", p.slider_at(i) * 100.0));
        }
        assert_eq!(value_of(Part::Format), p.format.word());
        assert_eq!(value_of(Part::Text), p.text());
        // The swatches' readings are the colour itself, RGBA and not
        // HSL: a swatch can carry alpha (`swatch`'s own chequerboard is
        // why) and the readout carries it too.
        assert_eq!(value_of(Part::Base(0)), write(base_colours()[0], Format::Hsl));
        assert_eq!(value_of(Part::Custom(1)), write(custom[1], Format::Hsl));
        assert_eq!(value_of(Part::Add), write(p.colour(), Format::Hsl));
        // And the grid cells carry their place in the set
        // (`AccessInfo::index`'s own doc: a tab's `(2, 5)` among five).
        assert_eq!(index_of(Part::Base(0)), Some((1, base_colours().len() as u32)));
        assert_eq!(index_of(Part::Custom(1)), Some((2, custom.len() as u32)));
    }
}
