//! The drawing vocabulary the views share, written once against
//! [`Surface`].
//!
//! Everything here used to live in `ui.rs` against `Ctx` and is now
//! reached from both sides of the plugin boundary. The port is
//! mechanical and deliberately so: `t.px(tok(&CELL, "table.cell_pad"))`
//! became `sf.px("table.cell_pad")`, which resolves the same token
//! through the same engine, so the host draws the same pixels it drew
//! before. `ui::meter` and `ui::badge` are now one-line wrappers around
//! [`meter`] and [`badge`] — there is one implementation of each, which
//! is the point.
//!
//! The cost of naming tokens by string is paid ONCE per draw: a view
//! reads its metrics into a `Look` struct before its row loop, never
//! inside it.

use super::surface::{StateInk, Surface};
use crate::draw::{Corner, CornerStyle, ShapeKind};
use crate::theme::parse::State;
use crate::theme::Color;
use crate::ui::{sev_of, Align, BadgeStyle, Case, Sev, SEVERITY_ROLES};
use crate::Rect;
use std::borrow::Cow;
use std::sync::OnceLock;

// ------------------------------------------------------------- severity

/// The severity a word outside the closed set resolves to:
/// `script.severity_fallback`, which §5.10 forbids ever being `ok`.
pub fn sev_fallback(sf: &mut impl Surface) -> Sev {
    let word = sf.word("script.severity_fallback");
    // The same answer, from the same place, as [`crate::ui::sev_fallback`]
    // — a plugin's table and the host's must not judge one reading two
    // ways, which is what two copies of the rule always end up doing.
    sev_of(&word).unwrap_or_else(|| crate::ui::unnamed_severity(&word))
}

fn sev_role(s: Sev) -> &'static str {
    SEVERITY_ROLES[(s.0 as usize).min(SEVERITY_ROLES.len() - 1)]
}

/// The `severity.<role>.{text,edge,fill,on}` key names, written out ONCE
/// per role and indexed thereafter — the same rung [`crate::ui::sev_tok`]
/// stops at with a `TokenId`, kept here as STRINGS because a `Surface`
/// may be the far side of the plugin ABI, where a `TokenId` means
/// nothing. `sev_text` and its three siblings used to rebuild their key
/// with `format!` on every call, which is the token-naming cost this
/// file's own header says a view pays ONCE per draw and never inside a
/// row loop.
struct SevKeys {
    text: String,
    edge: String,
    fill: String,
    on: String,
}

fn sev_keys(s: Sev) -> &'static SevKeys {
    static KEYS: OnceLock<Vec<SevKeys>> = OnceLock::new();
    let all = KEYS.get_or_init(|| {
        SEVERITY_ROLES
            .iter()
            .copied()
            .map(|role| SevKeys {
                text: format!("severity.{role}.text"),
                edge: format!("severity.{role}.edge"),
                fill: format!("severity.{role}.fill"),
                on: format!("severity.{role}.on"),
            })
            .collect()
    });
    &all[(s.0 as usize).min(all.len() - 1)]
}

/// The ink a severity writes in — the label, the value, the status word.
pub fn sev_text(sf: &mut impl Surface, s: Sev) -> Color {
    sf.color(&sev_keys(s).text)
}

/// The hairline a severity draws around a hollow pill.
pub fn sev_edge(sf: &mut impl Surface, s: Sev) -> Color {
    sf.color(&sev_keys(s).edge)
}

/// The bed a severity fills a hollow pill with.
pub fn sev_fill(sf: &mut impl Surface, s: Sev) -> Color {
    sf.bed(&sev_keys(s).fill)
}

/// The ink that reads ON a severity's solid fill.
pub fn sev_on(sf: &mut impl Surface, s: Sev) -> Color {
    sf.color(&sev_keys(s).on)
}

// ----------------------------------------------------------- type roles

/// One `type.*` role, resolved for the panel being drawn.
///
/// Read once per draw and carried into the row loop, exactly as the file
/// panel's `Look::read` does: the role is four token lookups and a row
/// loop must not repeat them.
#[derive(Clone, Copy, Debug)]
pub struct RoleLook {
    /// Size in device px, at the panel scale and the stack's shrink.
    pub px: f32,
    /// Letter spacing in px for a run at that size.
    pub track: f32,
    /// Line height as a multiple of `px`.
    pub leading: f32,
    /// Whether this role sets its figures on a fixed advance (§5.16's
    /// `tabular`). Carried as the ROLE's bool and handed to
    /// [`Surface::text_tab`], which measures the box from the face —
    /// this side of the library owns tokens, not faces.
    pub tabular: bool,
    /// The transform `type.<role>.case` asks for.
    ///
    /// Carried resolved, like every other member: a look read once per
    /// draw exists so the row loop asks the theme nothing, and the case
    /// is the member the row loop needed most — the three AI widgets each
    /// re-spelled `type.{role}.case` through the `Surface` beside a
    /// `RoleLook` they already held, and every other label on this side
    /// of the ABI settled the question by writing capitals in its source.
    pub case: Case,
    /// The slot `type.<role>.face` names — the family AND the weight this
    /// role is set in, since the master declares both on the face block.
    ///
    /// This field is why the struct exists in the shape it does: a look
    /// read once per draw is only worth having if it carries everything
    /// the row loop needs, and the row loop was writing `FONT_UI` because
    /// the face was the one thing it could not get from here.
    pub face: u8,
    /// The role's own ink: `fg` at its constant alpha.
    pub color: Color,
}

/// The look of a role the master does not declare. There is no spare
/// role and there must not be one: a role is twelve tokens, so a single
/// spare word hides a whole ladder behind a name nobody wrote, and `body`
/// — the obvious candidate — is a REAL role of plausible size, which
/// renders a broken theme as a nearly-right interface and lets it ship.
///
/// Nothing is drawn in it: zero px, no leading, no ink. The same ruling
/// [`crate::ui::role`] makes for the objects that draw against `Ctx`,
/// made once more here because this is the resolver every view, every
/// script table and the whole ABI side goes through.
pub const NO_ROLE: RoleLook = RoleLook {
    px: 0.0,
    track: 0.0,
    leading: 0.0,
    tabular: false,
    // Not capitals: shouting at a caller whose role is missing would be
    // this file choosing a look for an undesigned run, and nothing is
    // drawn in this look anyway.
    case: Case::None,
    // The interface slot, which is where an undesigned run has always
    // landed. Nothing is drawn in this look anyway — px is zero — so the
    // face is the one field here that cannot decide anything.
    face: crate::font::FONT_UI,
    color: Color::TRANSPARENT,
};

/// Resolves a type role by name. A name no `type.*` block declares warns
/// once and answers [`NO_ROLE`]: naming a role the theme does not have is
/// a defect to report, never a decision about how the text should look.
pub fn role_look(sf: &mut impl Surface, name: &str, shrink: f32) -> RoleLook {
    if !sf.has_token(&format!("type.{name}.size")) {
        // Said once, exactly as `ui::role` says it: a typo in a theme or
        // a script is worth one line and not sixty a second.
        crate::ui::warn_once(
            &format!("role:{name}"),
            &format!("unknown type role \"{name}\" — nothing is drawn in it"),
        );
        return NO_ROLE;
    }
    let raw = sf.px(&format!("type.{name}.size")) * sf.scale() * shrink;
    // The role's own ceiling and floor, the global floor beneath a role
    // whose theme states none of its own — and the arithmetic itself in
    // [`crate::theme::role_px`], which `ui::Role::px` calls too. Two
    // resolvers answering one question have to answer it identically, and
    // the only way to be sure of that is for there to be one answer.
    //
    // A floor at all only because the role EXISTS: the absent case has
    // already returned, since a floor on a role that does not exist would
    // put the hole back on screen at legible size, which is the failure
    // this whole rule was written to stop.
    let px = crate::theme::role_px(
        raw,
        sf.px(&format!("type.{name}.min_px")),
        sf.px("type.min_px"),
        sf.px(&format!("type.{name}.max_px")),
    );
    // Tracking tokens are em — a fraction of the run's own size.
    let track = px * sf.px(&format!("type.{name}.tracking"));
    // A role whose master states no `leading` measures zero: an unstated
    // line height is a broken role, and the height of a broken role is
    // not this file's to invent.
    let leading = sf.px(&format!("type.{name}.leading"));
    // §5.16's `tabular`, read here so that every view, every script table
    // and the whole ABI side gets it from ONE resolver.
    let tabular = sf.flag(&format!("type.{name}.tabular"));
    // §5.16's `case`, resolved from the WORD by the toolkit's one applier
    // — so a theme's typo warns here exactly as it warns on the object
    // side, instead of turning into capitals nobody asked for.
    let case = Case::from_word(&sf.word(&format!("type.{name}.case")));
    // §5.16's `face`, read through the same one resolver and by the same
    // word→slot rule `ui::Role::font` uses. Reading it here is what stops
    // a view and an object disagreeing about which family one role is in.
    let face = crate::font::face_slot(&sf.word(&format!("type.{name}.face")));
    let mut color = sf.color(&format!("type.{name}.fg"));
    let alpha = sf.px(&format!("type.{name}.alpha"));
    color.a *= if alpha > 0.0 { alpha.min(1.0) } else { 1.0 };
    RoleLook { px, track, leading, tabular, case, face, color }
}

impl RoleLook {
    /// This role's own string, in the case the theme set it in.
    ///
    /// The twin of [`crate::ui::Role::cased`], and the same applier
    /// underneath both: one answer to "does this label shout", whichever
    /// side of the boundary asks.
    pub fn cased<'a>(&self, s: &'a str) -> Cow<'a, str> {
        crate::ui::recase(self.case, s)
    }
}

/// The role a `*_role` binding token names — `script.table_head_role`,
/// `list.label_role`. A binding resolving to nothing answers [`NO_ROLE`].
pub fn bound_role(sf: &mut impl Surface, binding: &str, shrink: f32) -> RoleLook {
    let word = sf.word(binding);
    if word.is_empty() {
        // The BINDING is what a reader has to go and fix, and it is the
        // one thing the role-side warning cannot name: an empty word
        // means either that this key is absent from the master or that a
        // consumer asked for a key nobody declares, and both are the
        // binding's story.
        crate::ui::warn_once(
            &format!("binding:{binding}"),
            &format!("\"{binding}\" names no type role — nothing is drawn in it"),
        );
        return NO_ROLE;
    }
    role_look(sf, &word, shrink)
}

// -------------------------------------------------------------- corners

/// The cut a shape word asks for, read off a [`Surface`].
///
/// Compared as a WORD, not as an enum index, for two reasons that both
/// bite here: the ABI can only ship words, and a preset's style slot
/// (`shape.badge.corners[0]`) has no `enum:` list in the master, so its
/// word table grows out of the values actually loaded — an index
/// memoised before a variant is read would freeze at the wrong answer.
///
/// The word itself is turned into a cut by [`crate::corner::cut`], which
/// is the library's ONE reading of that vocabulary; this function is the
/// surface-shaped door to it and holds no `match` of its own. `chevron`
/// and `hexagon` are in the presets' vocabulary and in no surface's, and
/// degrade to Square there for the reason stated at that door.
pub fn corner_style(sf: &mut impl Surface, name: &str) -> CornerStyle {
    crate::corner::cut(&sf.word(name))
}

/// The three cuts a corner word can name, with `fallback` for anything
/// else.
///
/// The table is NOT here: it is `corner::WORDS`, the one vocabulary this
/// crate collapsed four independent `match` arms into. This is the door
/// the per-corner rule below knocks on, and it exists only to name the
/// fallback — see [`crate::corner::cut_or`] for why that is the caller's
/// to choose.
pub(crate) fn cut_word(w: &str, fallback: CornerStyle) -> CornerStyle {
    crate::corner::cut_or(w, fallback)
}

/// What `same_as_parent` bakes to — §5.0's sentinel, in the scalar
/// array where every slot has one whatever kind of value it holds.
pub(crate) fn inherits() -> f32 {
    crate::theme::expr::sentinel("same_as_parent").unwrap_or(-3.0)
}

/// **One per-corner override, applied to the corner that arrived.**
///
/// This is the rule `shape.<preset>.corners_tl/tr/br/bl` are written in,
/// and it lives here once because it has two readers: the surface layer
/// ([`preset`], for anything that draws through a [`Surface`]) and the
/// object layer (`object::elev::Level::cut`, which is the one place a
/// surface of the toolkit's own is drawn, so every rung a consumer
/// points at a preset with `shaped_by` arrives here). Two copies of it
/// would be two answers to "what does a half-stated pair mean", which is
/// the question the whole key exists to settle.
///
/// Each of the pair's slots inherits ON ITS OWN. `style_scalar` is the
/// style slot read as a NUMBER — a sentinel bakes to its own negative in
/// the scalar array whatever kind of slot it sits in — so the question
/// "did the theme state a style" can be asked without asking the word
/// first, and `word` is only consulted when the answer is yes.
pub(crate) fn override_corner(
    base: Corner,
    r: Rect,
    style_scalar: f32,
    word: &str,
    stated: f32,
) -> Corner {
    let inherit = inherits();
    let style =
        if style_scalar == inherit { base.style } else { cut_word(word, base.style) };
    if stated == inherit {
        Corner { style, size: base.size }
    } else {
        // Through `Corner::sized` and not straight in: `pill` is a word
        // ABOUT the box and bakes negative, so a stated slot needs the
        // rect before it is a length at all.
        Corner::sized(style, stated, r)
    }
}

/// The radius a `*.corner` token states, for the rect that wears it.
///
/// A LENGTH IS NOT A SHAPE (§5.4d): this is the radius half of the pair
/// only, and [`corner_style`] carries the cut. `pill` is not a length at
/// all — §5.0 bakes the word to a negative sentinel — and names the
/// capsule: the radius at which both ends of the rect close over, which
/// is half its shorter side, and which is also the ceiling any stated
/// radius meets before two corners would cross.
///
/// The translation itself lives in [`crate::theme::corner_radius`] and is
/// called from here rather than repeated: a capsule written four times is
/// a capsule that stops being one somewhere.
pub fn corner_radius(sf: &mut impl Surface, name: &str, r: Rect, shrink: f32) -> f32 {
    let stated = sf.px(name);
    // A stated LENGTH meets the panel's shrink before it meets the box.
    // Every sentinel is a word ABOUT the box and has nothing to scale.
    let scaled = if stated > 0.0 { stated * shrink } else { stated };
    let radius = crate::theme::corner_radius(scaled, r.w, r.h);
    if radius == 0.0 && stated < 0.0 {
        // `auto` and `same_as_parent` are the rest of §5.0's table, and
        // neither is a radius. Named out loud rather than quietly cut to
        // nothing: the key is a theme's mistake to fix, not this file's
        // to paper over.
        crate::ui::warn_once(
            &format!("corner:{name}"),
            &format!("\"{name}\" holds a sentinel that is neither a length nor `pill`"),
        );
    }
    radius.min(r.w.min(r.h).max(0.0) / 2.0)
}

// ------------------------------------------------------- shape presets

/// A `shape.*` preset resolved into what a draw list takes: the
/// silhouette's KIND and its four corners, one of which may differ from
/// the next.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preset {
    pub kind: ShapeKind,
    /// tl, tr, br, bl — `ring_points`' order, the order every
    /// `[Corner; 4]` in this toolkit is written in.
    pub corners: [Corner; 4],
}

/// One `shape.*` preset, read the way §5.0 wrote it (f3 K6).
///
/// **This is the first reader `corners_tl/tr/br/bl` have ever had.**
/// Sixteen presets have carried the four keys since the master was
/// written and nothing looked at them, so no theme could shape one
/// corner differently from the next however plainly it said so — the
/// record has had two bits per corner since K3 and there was simply no
/// road from the theme to them. This is the road.
///
/// Each per-corner key is the same PAIR as `corners` itself, and each
/// of its two slots inherits on its own: a slot holding
/// `same_as_parent` takes the preset's `corners`, anything else is that
/// slot's own answer. So a theme may chamfer one corner at the radius
/// the preset already carries, or keep the style and shorten the cut,
/// without restating the half it did not mean to change.
///
/// **The kind comes from `corners` and from nowhere else.** The master
/// spends two words of the corner vocabulary on whole SILHOUETTES —
/// `shape.taskbar` is `[ chevron, 0u ]` and `shape.hex` is
/// `[ hexagon, 0u ]`, each with a comment saying "a shape word, not a
/// corner" — and its own TODO asks whether `CornerStyle` should grow
/// two variants or the presets should grow a `generator` key. Neither:
/// the two words name a [`ShapeKind`], the record has had a field for
/// one since K3, and `CornerStyle` stays the three cuts a corner can
/// take. A shape word on a PER-CORNER key means nothing — one corner of
/// a hexagon is not a shape — so it falls back to the preset's own cut
/// rather than inventing a square.
pub fn preset(sf: &mut impl Surface, name: &str, r: Rect) -> Preset {
    let base_style = corner_style(sf, &format!("{name}.corners[0]"));
    let base = Corner::sized(base_style, sf.px(&format!("{name}.corners[1]")), r);
    let mut corners = [Corner::SQUARE; 4];
    for (i, slot) in ["tl", "tr", "br", "bl"].iter().enumerate() {
        let key = format!("{name}.corners_{slot}");
        let style_slot = format!("{key}[0]");
        let scalar = sf.px(&style_slot);
        // The word is asked for only when the scalar says a style was
        // stated: through the ABI a `word` call is a round trip, and
        // through the master it is a lock.
        let word =
            if scalar == inherits() { String::new() } else { sf.word(&style_slot) };
        corners[i] =
            override_corner(base, r, scalar, &word, sf.px(&format!("{key}[1]")));
    }
    Preset { kind: preset_kind(sf, name, r), corners }
}

/// The silhouette a preset's `corners` word names, with the numbers its
/// own preset-specific keys carry (`shape.hex.orientation`,
/// `shape.taskbar.chevron_depth` / `.chevron_dir`).
///
/// Those numbers live in keys the master declares on the TWO presets
/// that use a shape word, and the `corners` comment on all sixteen
/// offers the whole vocabulary — so a theme may write `chevron` on a
/// preset that carries no `chevron_depth`. That is a mistake with a
/// silent answer if left alone: a missing token reads zero, a chevron of
/// depth zero is the rect it was cut from, and the theme sees the shape
/// it asked for simply not happen. [`missing`] says so instead.
fn preset_kind(sf: &mut impl Surface, name: &str, r: Rect) -> ShapeKind {
    match sf.word(&format!("{name}.corners[0]")).as_str() {
        "hexagon" => {
            missing(name, "hexagon", &["orientation"]);
            // 30° is not a look value — it is what the word `pointy`
            // means on a six-fold lattice. Which of the two a theme
            // wants IS the look, and that is the token. A preset that
            // names no orientation gets NO turn, rather than one this
            // file chose for it.
            let turn = match sf.word(&format!("{name}.orientation")).as_str() {
                "pointy" => std::f32::consts::FRAC_PI_6,
                _ => 0.0,
            };
            ShapeKind::Hex { turn }
        }
        "chevron" => {
            missing(name, "chevron", &["chevron_depth", "chevron_dir"]);
            // A fraction of the HEIGHT, per end, which is what the
            // master's own comment says; `%` bakes to 0..1.
            let depth = sf.px(&format!("{name}.chevron_depth")).max(0.0) * r.h;
            let dir = sf.word(&format!("{name}.chevron_dir"));
            let end = |wanted: &str| if dir == wanted || dir == "both" { depth } else { 0.0 };
            ShapeKind::Chevron { left: end("left"), right: end("right") }
        }
        _ => ShapeKind::Box,
    }
}

/// Names the keys a shape word needs and the preset does not declare.
///
/// A shape word costs the theme nothing to write and the reader cannot
/// invent the numbers it implies; the silent answer — every absent key
/// reading zero, which is a rectangle — is exactly the "token with no
/// reader" ledger read from the other end, and it is the one a theme
/// author cannot debug because nothing happened.
fn missing(name: &str, word: &str, keys: &[&str]) {
    let absent = undeclared(name, keys);
    if absent.is_empty() {
        return;
    }
    crate::ui::warn_once(
        &format!("preset_kind:{name}"),
        &format!(
            "\"{name}.corners\" asks for a {word} but the preset declares no {} — the shape falls back to its rect",
            absent.join(", ")
        ),
    );
}

/// Which of `keys` the preset `name` does not declare at all.
///
/// Split out of [`missing`] because the diagnostic itself is a line on
/// stderr and a test can only watch what a function ANSWERS. This is the
/// half that decides, and it is the half that can be wrong.
fn undeclared<'k>(name: &str, keys: &[&'k str]) -> Vec<&'k str> {
    keys.iter()
        .copied()
        .filter(|k| crate::theme::id(&format!("{name}.{k}")).is_none())
        .collect()
}

// --------------------------------------------------------------- text

/// Trims text with a trailing ellipsis so it fits `max_w`, measured at
/// the SAME letter tracking the caller draws with.
///
/// Measuring at a different tracking is how a content-measured table
/// column came to ellipsise the very cell it was sized from.
pub fn fit_end(
    sf: &mut impl Surface,
    face: u8,
    px: f32,
    text: &str,
    max_w: f32,
    track: f32,
) -> String {
    fit_end_tab(sf, face, px, text, max_w, track, false)
}

/// [`fit_end`] measured under the role's figure box. The same rule one
/// rung further: a tabular column trimmed against proportional widths
/// ellipsises a cell that fits, because every figure it holds is drawn
/// wider than it was measured.
pub fn fit_end_tab(
    sf: &mut impl Surface,
    face: u8,
    px: f32,
    text: &str,
    max_w: f32,
    track: f32,
    tabular: bool,
) -> String {
    // No room at all is not a width to abbreviate to: the ellipsis this
    // used to answer with is a glyph as wide as any other, so it went
    // over whatever squeezed the room shut. `draw::fit_tail` and the
    // panel band's `fit_lead` have both ruled it that way since they
    // were written, and `winframe::fit_title` had to re-state it locally
    // because THIS function did not — a trimming rule stated three times
    // and contradicted once. Stated here, it is the toolkit's answer.
    if max_w <= 0.0 {
        return String::new();
    }
    if sf.measure_tab(face, px, text, track, tabular) <= max_w {
        return text.to_string();
    }
    // Read AFTER the run is known not to fit: an untrimmed label must
    // not pay for a key it will not use, and this is the one text-token
    // read on any draw path.
    let cut = trim_marker(sf);
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len().saturating_sub(1);
    while n > 1 {
        let cand: String = chars[..n].iter().collect::<String>() + cut.as_str();
        if sf.measure_tab(face, px, &cand, track, tabular) <= max_w {
            return cand;
        }
        n -= 1;
    }
    cut
}

/// `type.ellipsis` — what a trimmed run ends on, off a [`Surface`].
///
/// The master declares the key and its comment names the call sites that
/// ignored it. An absent or empty key trims with NO marker: the character
/// a cut ends on is typography, so a literal standing in for it here
/// would be a look decided in Rust, which is the one thing this file may
/// not hold.
pub fn trim_marker(sf: &mut impl Surface) -> String {
    let cut = sf.theme_text("type.ellipsis");
    if cut.is_empty() {
        crate::ui::warn_once(
            "type.ellipsis",
            "type.ellipsis is empty or absent — trimmed text ends on nothing",
        );
    }
    cut
}

/// The tooltip a TRIMMED label files (F2 §8.1): `shown` is what reached
/// the screen, `full` is what there was, and the difference between them
/// is the whole reason to say anything.
///
/// Nothing happens when the two are equal — a tooltip repeating what is
/// already legible is noise — and nothing happens when the pointer is
/// somewhere else, which is checked HERE so the string comparison is the
/// only work a row the pointer is nowhere near ever does.
///
/// One sentence, one place. A tab, a segment, a table heading and a
/// table cell each wrote it out, and the list was about to be the fifth;
/// the rule they were all writing is this function.
pub fn explain_trim(sf: &mut impl Surface, id: u64, anchor: Rect, shown: &str, full: &str) {
    if shown == full {
        return;
    }
    let (mx, my) = sf.mouse();
    if !anchor.contains(mx, my) {
        return;
    }
    sf.tooltip(id, anchor, full);
}

/// Breaks text into lines no wider than `max_w`, measured at the SAME
/// letter tracking the caller draws with.
///
/// The first text breaking in the toolkit — everything before it either
/// fitted or ellipsised ([`fit_end`]). Greedy by words, which is what a
/// tooltip and a label want: a word starts a new line when it no longer
/// fits, and a single word wider than the whole box is broken by
/// characters rather than allowed to overflow. Explicit newlines in the
/// text are kept; an empty `max_w` (or one narrower than a character)
/// answers one line per source line, unbroken, so a nonsense width
/// degrades to "no wrapping" instead of to an endless loop.
pub fn wrap(
    sf: &mut impl Surface,
    face: u8,
    px: f32,
    text: &str,
    max_w: f32,
    track: f32,
) -> Vec<String> {
    wrap_tab(sf, face, px, text, max_w, track, false)
}

/// [`wrap`] measured under the role's figure box (§5.16 `tabular`), the
/// same rung [`fit_end_tab`] is to [`fit_end`].
///
/// A break is a MEASUREMENT, and a run that is drawn with a box and
/// broken without one is broken in the wrong places: every figure of the
/// candidate line is drawn wider than it was ruled, so the box overflows
/// on the right exactly as far as the digits it holds. That is the
/// mismatch `fit_end_tab` was written for, one line-breaking further on.
#[allow(clippy::too_many_arguments)]
pub fn wrap_tab(
    sf: &mut impl Surface,
    face: u8,
    px: f32,
    text: &str,
    max_w: f32,
    track: f32,
    tabular: bool,
) -> Vec<String> {
    let mut out = Vec::new();
    for para in text.split('\n') {
        if max_w <= 0.0 {
            out.push(para.to_string());
            continue;
        }
        let mut line = String::new();
        for word in para.split_whitespace() {
            let cand = if line.is_empty() {
                word.to_string()
            } else {
                format!("{line} {word}")
            };
            if sf.measure_tab(face, px, &cand, track, tabular) <= max_w {
                line = cand;
                continue;
            }
            if !line.is_empty() {
                out.push(std::mem::take(&mut line));
            }
            // The word alone on its line: kept whole when it fits,
            // broken by characters when nothing else can be done.
            if sf.measure_tab(face, px, word, track, tabular) <= max_w {
                line = word.to_string();
                continue;
            }
            let mut piece = String::new();
            for ch in word.chars() {
                let mut cand = piece.clone();
                cand.push(ch);
                if !piece.is_empty() && sf.measure_tab(face, px, &cand, track, tabular) > max_w {
                    out.push(std::mem::take(&mut piece));
                    piece.push(ch);
                } else {
                    piece = cand;
                }
            }
            line = piece;
        }
        out.push(line);
    }
    out
}

/// Top of a single line centred in a box of `box_h`, WITHOUT the baseline
/// grid — the caller does not name a face, so no ascent is known and no
/// baseline can be put on a grid line.
///
/// Kept for the callers outside this library: this signature is the one
/// every plugin in `nacelle-addons` draws its rows with, and the grid
/// cannot be worth a break in it. Everything inside the toolkit that
/// holds a `RoleLook` — which is everything that reads a role at all —
/// calls [`center_line_y_in`].
pub fn center_line_y(sf: &mut impl Surface, y: f32, box_h: f32, px: f32, leading: f32) -> f32 {
    let mut ty = y + (box_h - px * leading) / 2.0;
    if sf.enum_is("rhythm.center_mode", "optical") {
        ty += px * sf.px("rhythm.cap_center_bias");
    }
    ty
}

/// [`center_line_y`] with the run's FACE named, so the line's baseline can
/// be laid on the theme's grid.
pub fn center_line_y_in(
    sf: &mut impl Surface,
    face: u8,
    y: f32,
    box_h: f32,
    px: f32,
    leading: f32,
) -> f32 {
    let ty = center_line_y(sf, y, box_h, px, leading);
    let ascent = sf.ascent(face, px);
    snap_baseline(sf, ty, ascent)
}

/// `rhythm.baseline`, `rhythm.snap_baseline` and `rhythm.snap_origin` —
/// the vertical grid a line's BASELINE is laid on, and where that grid is
/// measured from.
///
/// Three keys the master has declared since §5.25 was written and none of
/// them had a reader: a theme could ask for a 1u grid, say where it was
/// measured from, and turn it off for the family whose cards float over a
/// live background — and every line in the program still landed wherever
/// the centring arithmetic left it. `snap_baseline = false` is exactly
/// the picture drawn before this function existed, which is what makes
/// the key an honest off switch and not a new default in disguise.
///
/// The BASELINE and not the line's top, which is the whole reason
/// [`Surface::ascent`] exists: a row carrying a small label beside a large
/// reading has two line tops and one baseline, and a grid that pulled the
/// tops together would align the two runs by their ascenders — further
/// from what the eye reads as one row than the centring it replaced.
///
/// Answers the TOP, because a top is what every text call takes.
pub fn snap_baseline(sf: &mut impl Surface, ty: f32, ascent: f32) -> f32 {
    if !sf.flag("rhythm.snap_baseline") {
        return ty;
    }
    let step = sf.px("rhythm.baseline");
    // A grid of no width has no lines to land on, and dividing by it is
    // what must not happen. Not a look decision: a step has to be
    // positive to be a step.
    if step <= 0.0 {
        return ty;
    }
    let origin = if sf.enum_is("rhythm.snap_origin", "screen_top") { 0.0 } else { grid_origin() };
    let baseline = ty + ascent;
    origin + ((baseline - origin) / step).round() * step - ascent
}

thread_local! {
    /// Where `rhythm.snap_origin = panel_content_top` measures the grid
    /// from — published by whoever opened the content box, read by
    /// [`snap_to_grid`].
    static GRID_ORIGIN: std::cell::Cell<f32> = const { std::cell::Cell::new(0.0) };
}

/// The origin the panel-relative grid currently stands at.
pub fn grid_origin() -> f32 {
    GRID_ORIGIN.with(|o| o.get())
}

/// Declares where a panel's content begins, for the grid.
///
/// A thread-local and not a field on `Ctx` for one reason: `Ctx` is the
/// host's struct, constructed by the application and by every test that
/// draws, and a grid origin is not something any of them knows or should
/// have to state. The panel object knows it — it is the one place that
/// settles a content box — so it publishes it there and this file reads
/// it. Drawing is sequential, so "the panel most recently opened" is the
/// panel whose content is being drawn.
pub fn set_grid_origin(y: f32) {
    GRID_ORIGIN.with(|o| o.set(y));
}

/// One aligned run inside a cell of width `w` starting at `x`.
#[allow(clippy::too_many_arguments)]
pub fn cell_text(
    sf: &mut impl Surface,
    x: f32,
    y: f32,
    w: f32,
    align: Align,
    face: u8,
    px: f32,
    text: &str,
    color: Color,
    track: f32,
) {
    cell_text_tab(sf, x, y, w, align, face, px, text, color, track, false);
}

/// [`cell_text`] under the role's figure box — the form a numeric column
/// takes. `PID 1471` and `PID 1888` then occupy the same width, which is
/// the difference between a column that stands still and one that moves
/// a pixel or two every time a process is replaced.
#[allow(clippy::too_many_arguments)]
pub fn cell_text_tab(
    sf: &mut impl Surface,
    x: f32,
    y: f32,
    w: f32,
    align: Align,
    face: u8,
    px: f32,
    text: &str,
    color: Color,
    track: f32,
    tabular: bool,
) {
    let tx = match align {
        Align::Left => x,
        Align::Center => x + w / 2.0,
        Align::Right => x + w,
    };
    sf.text_tab(face, px, tx, y, text, color, track, align, tabular);
}

/// The number at the front of a formatted cell (`"41.2%"` → 41.2), for a
/// bar reading the value it also prints. `None` when the text does not
/// start with one — a bar of nothing is drawn empty, never invented.
pub fn leading_number(text: &str) -> Option<f32> {
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(text.len());
    text[..end].parse::<f32>().ok().filter(|v| v.is_finite())
}

// -------------------------------------------------------------- shapes

/// The framed bar: a recessed track and a fill to `frac`.
///
/// A severity is the model's judgement of the DATA (an index into the
/// closed set, not a colour) and tints the fill; `track = false` says the
/// value has no meaningful whole, so no outline claims one.
pub fn meter(sf: &mut impl Surface, r: Rect, frac: f32, sev: Option<Sev>, track: bool) {
    let frac = if frac.is_finite() { frac.clamp(0.0, 1.0) } else { 0.0 };
    let bw = sf.px("progress.border");
    // The fill sits `progress.inset` behind the ring, so it never
    // touches it.
    let inset = bw + sf.px("progress.inset");
    // `progress.corner` was declared and read by nothing, so a theme that
    // rounded its bars got squares. The fill wears the track's own corner
    // inset by the same distance the fill is inset — a square-ended fill
    // inside a rounded track hangs out past the cap, which is the bug the
    // slider's groove already had to fix.
    let cut = corner_style(sf, "progress.corner_style");
    let radius = corner_radius(sf, "progress.corner", r, 1.0);
    if track {
        let c = sf.color("component.bar.track");
        sf.ring(r, cut, radius, bw, c);
    }
    let inner = (r.w - 2.0 * inset).max(0.0);
    let fill = match sev {
        Some(s) => sev_text(sf, s),
        None => sf.color("component.bar.fill"),
    };
    let bar = Rect::new(r.x + inset, r.y + inset, inner * frac, (r.h - 2.0 * inset).max(0.0));
    sf.ring_fill(bar, cut, (radius - inset).max(0.0), fill);
}

/// The CRITICAL / CONTAINED pill: a filled, ringed capsule around a
/// short text, its four colours from the severity at draw time. Returns
/// the pill's width.
///
/// The corner is the theme's: `badge.corner` for the radius — `pill`
/// included, which is what the master ships and what makes the capsule
/// this thing is named after — and `shape.badge.corners`' style slot for
/// the cut.
pub fn badge(
    sf: &mut impl Surface,
    r: Rect,
    text: &str,
    sev: Option<Sev>,
    style: BadgeStyle,
    align: Align,
    shrink: f32,
) -> f32 {
    let role = bound_role(sf, "script.badge_role", shrink);
    let tw = sf.measure_tab(role.face, role.px, text, role.track, role.tabular);
    let pad = sf.px("badge.pad_x") * shrink;
    // No floor under either: a `.max(1.0)` here is a one-pixel badge
    // nobody's theme asked for. `badge.h = 0` means the master wants no
    // badge, and that is a look it is entitled to state; the width is
    // the measured text plus the theme's padding, which is already a
    // length rather than a guess.
    let h = (sf.px("badge.h") * shrink).min(r.h);
    let w = (tw + 2.0 * pad).min(r.w);
    let x = match align {
        Align::Left => r.x,
        Align::Center => r.x + (r.w - w) / 2.0,
        Align::Right => r.right() - w,
    };
    let y = r.y + (r.h - h) / 2.0;
    let solid = match style {
        BadgeStyle::Solid => true,
        BadgeStyle::Hollow => false,
        BadgeStyle::FromTheme => match sev {
            Some(s) if sf.flag("badge.style_from_severity") => {
                sf.word(&format!("severity.{}.badge_style", sev_role(s))) == "solid"
            }
            _ => false,
        },
    };
    let (fill, edge, ink) = match (sev, solid) {
        (Some(s), true) => (sev_text(sf, s), sev_text(sf, s), sev_on(sf, s)),
        (Some(s), false) => (sev_fill(sf, s), sev_edge(sf, s), sev_text(sf, s)),
        (None, true) => (
            sf.bed("component.badge.solid_fill"),
            sf.bed("component.badge.solid_fill"),
            sf.color("component.badge.solid_text"),
        ),
        (None, false) => (
            sf.bed("component.badge.fill"),
            sf.color("component.badge.edge"),
            sf.color("component.badge.text"),
        ),
    };
    let pill = Rect::new(x, y, w, h);
    // A badge is the one element that states its shape in two places:
    // `badge.corner` is the radius, and the style half of the preset's
    // `shape.badge.corners` is the cut. Both are read, so a theme that
    // moves either one moves the badge.
    let cut = corner_style(sf, "shape.badge.corners[0]");
    let radius = corner_radius(sf, "badge.corner", pill, shrink);
    let bw = sf.px("badge.border");
    sf.ring_fill(pill, cut, radius, fill);
    if bw > 0.0 && !solid {
        sf.ring(pill, cut, radius, bw, edge);
    }
    let ty = center_line_y_in(sf, role.face, y, h, role.px, role.leading);
    sf.text_tab(
        role.face, role.px, x + w / 2.0, ty, text, ink, role.track, Align::Center,
        role.tabular,
    );
    w
}

/// The little triangle beside a sorted heading: point up for ascending,
/// down for descending. An outline, drawn with the polyline every icon in
/// this project is drawn with, so it inherits the same hairline.
pub fn sort_marker(
    sf: &mut impl Surface,
    x: f32,
    y: f32,
    size: f32,
    line_px: f32,
    dir: super::SortDir,
    color: Color,
) {
    if size <= 0.0 {
        return;
    }
    // Centred on the heading's own type size — the same optical guess the
    // rest of this vocabulary makes until a shared centring primitive
    // exists.
    let top = y + (line_px - size).max(0.0) / 2.0;
    let half = size / 2.0;
    let pts = match dir {
        super::SortDir::Asc => [[x + half, top], [x + size, top + size], [x, top + size]],
        super::SortDir::Desc => [[x, top], [x + size, top], [x + half, top + size]],
    };
    let hair = sf.px("stroke.hair");
    sf.polyline(&pts, hair, color, true);
}

/// Which GRAMMAR a [`disclosure`] triangle is drawn in.
///
/// The two consumers draw the same three points and mean opposite things
/// by them, and that is the whole of the difference — so this is one
/// primitive with a parameter and not two triangles. Two copies would
/// let a theme's hairline, its centring rule and its winding drift apart
/// for no reason, and the caller would still have to choose between
/// them: the choice does not go away, it only stops being named.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Disclosure {
    /// A node in a TREE. Closed points along the row — "there is more
    /// inside this" — and open points down at the children it just
    /// revealed. Every file tree ever drawn reads this way and nothing
    /// here changes it.
    Tree,
    /// The caret on a DROP-DOWN's anchor. Closed points DOWN, at the
    /// direction the list will unfold, because a caret announces where
    /// the list goes and not the fact that it is currently shut — GTK,
    /// Qt and HTML's `select` all agree, and a `▷` here reads as "go
    /// into this row", which is the tree's sentence and not this one's.
    /// Open points back up, at the edge the list folds into.
    Drop,
}

/// The triangle that says a thing opens: the expander beside a tree row,
/// the caret on a drop-down's anchor. `kind` picks which of the two
/// sentences the shape is speaking ([`Disclosure`]); `expanded` is the
/// thing's own state.
///
/// The state turns the GLYPH, not its colour: rotation is geometry, and
/// geometry is the one thing a theme does not have to say twice.
pub fn disclosure(
    sf: &mut impl Surface,
    x: f32,
    y: f32,
    size: f32,
    line_px: f32,
    kind: Disclosure,
    expanded: bool,
    color: Color,
) {
    if size <= 0.0 {
        return;
    }
    let top = y + (line_px - size).max(0.0) / 2.0;
    let half = size / 2.0;
    // Named once and used by both grammars, so "down" is one shape in
    // this file and cannot end up two slightly different ones.
    let down = [[x, top], [x + size, top], [x + half, top + size]];
    let pts = match (kind, expanded) {
        // Along the row, toward what opening would reveal.
        (Disclosure::Tree, false) => [[x, top], [x + size, top + half], [x, top + size]],
        (Disclosure::Tree, true) => down,
        (Disclosure::Drop, false) => down,
        // Back at the anchor the open list folds into.
        (Disclosure::Drop, true) => [[x + half, top], [x + size, top + size], [x, top + size]],
    };
    let hair = sf.px("stroke.hair");
    sf.polyline(&pts, hair, color, true);
}

/// The scrollbar, from the geometry [`super::scroll::scrollbar`] worked
/// out: the groove when the theme asks for one, then the thumb on the
/// `scrollbar.thumb` class's ladder.
pub fn scrollbar(
    sf: &mut impl Surface,
    geom: &super::scroll::ScrollbarGeom,
    alpha: f32,
    hovered: bool,
    dragging: bool,
) {
    if alpha <= 0.0 {
        return;
    }
    if sf.enum_is("scrollbar.track", "on") {
        let mut c = sf.bed("component.scrollbar.track");
        c.a *= alpha;
        sf.rect(geom.track, c);
    }
    let rung = if dragging {
        State::Dragging
    } else if hovered {
        State::Hover
    } else {
        State::Idle
    };
    // The thumb draws its idle rung too, so the plain fade applies: no
    // resting rung has to be invented for it. While the thumb is being
    // DRAGGED it moves, and a moved control is a new key born at its
    // rung — so the grab reads instantly and the travel never lags.
    let style: StateInk = sf.class_ink("scrollbar.thumb", rung, geom.thumb);
    // The master asks for `@corner.pill` here and got a rectangle: the
    // radius was declared and read by nothing. The pair is the ordinary
    // one — the radius from `scrollbar.corner`, the cut from
    // `scrollbar.corner_style` — so a capsule thumb is a capsule.
    let cut = corner_style(sf, "scrollbar.corner_style");
    let radius = corner_radius(sf, "scrollbar.corner", geom.thumb, 1.0);
    let mut fill = style.fill;
    fill.a *= alpha;
    if fill.a > 0.0 {
        sf.ring_fill(geom.thumb, cut, radius, fill);
    }
    let mut edge = style.edge;
    edge.a *= alpha;
    if style.edge_width > 0.0 && edge.a > 0.0 {
        sf.ring(geom.thumb, cut, radius, style.edge_width, edge);
    }
}

// ---------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::font::FONT_UI;

    /// A surface that only measures: half an em a character, which is
    /// wrong about fonts and right about monotonicity — all the breaking
    /// arithmetic asks of it. Nothing here draws.
    ///
    /// It does state ONE token, `type.ellipsis`, because a trimming
    /// routine reads the marker off the surface and a ruler answering
    /// nothing would be a theme that declares no marker — a case worth
    /// its own test, not the state every other test runs under.
    struct Ruler;

    /// What this ruler's theme says `type.ellipsis` holds.
    ///
    /// A constant, because this ruler is the FIXTURE and not the subject:
    /// the tests that vary the marker — a theme stating a comma, a theme
    /// stating nothing — drive `FakeSurface::text_at`, which is the seam
    /// built for exactly that. A ruler that could be restated would be a
    /// second such seam with one shape of question fewer.
    const RULER_CUT: &str = "\u{2026}";

    impl Surface for Ruler {
        fn rect(&mut self, _r: Rect, _c: Color) {}
        fn rect_outline(&mut self, _r: Rect, _w: f32, _c: Color) {}
        fn line(&mut self, _x0: f32, _y0: f32, _x1: f32, _y1: f32, _w: f32, _c: Color) {}
        fn polyline(&mut self, _p: &[[f32; 2]], _w: f32, _c: Color, _closed: bool) {}
        fn text(&mut self, _face: u8, _px: f32, _x: f32, _y: f32, _s: &str, _c: Color, _t: f32, _a: Align) {}
        fn measure(&mut self, _face: u8, px: f32, s: &str, _track: f32) -> f32 {
            s.chars().count() as f32 * px * 0.5
        }
        /// A box that is WIDER than the proportional run it replaces,
        /// which is the one property a caller can rely on: a real box is
        /// the widest figure of the face, so a boxed run never measures
        /// narrower. Doubling makes the difference impossible to miss —
        /// the default implementation of this method ignores `tabular`
        /// entirely, and a break that went through it would be silent.
        fn measure_tab(&mut self, face: u8, px: f32, s: &str, track: f32, tabular: bool) -> f32 {
            let w = self.measure(face, px, s, track);
            if tabular {
                w * 2.0
            } else {
                w
            }
        }
        fn clip(&mut self, _r: Rect) -> bool {
            false
        }
        fn unclip(&mut self) {}
        fn has_token(&mut self, _name: &str) -> bool {
            false
        }
        fn px(&mut self, _name: &str) -> f32 {
            0.0
        }
        fn color(&mut self, _name: &str) -> Color {
            Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }
        }
        fn bed(&mut self, _name: &str) -> Color {
            Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }
        }
        fn flag(&mut self, _name: &str) -> bool {
            false
        }
        fn word(&mut self, _name: &str) -> String {
            String::new()
        }
        fn theme_text(&mut self, name: &str) -> String {
            match name {
                "type.ellipsis" => RULER_CUT.to_string(),
                _ => String::new(),
            }
        }
        fn class_state(&mut self, _class: &str, _state: State) -> StateInk {
            StateInk::raw()
        }
        fn epoch(&mut self) -> u32 {
            0
        }
        fn now(&self) -> f64 {
            0.0
        }
        fn mouse(&self) -> (f32, f32) {
            (0.0, 0.0)
        }
        fn scale(&self) -> f32 {
            1.0
        }
    }

    // ---- the trim marker, and the case, off a SURFACE ----
    //
    // The `Surface` half of both keys: this is the road every view, every
    // script table and every compiled widget on the far side of the ABI
    // takes, so a key honoured on the object side and ignored here would
    // be honoured in half the interface.

    #[test]
    fn a_trim_off_a_surface_ends_on_the_character_the_theme_states() {
        use crate::view::surface::tests::FakeSurface;
        // A surface whose theme says a comma. Four characters at 10 px
        // are 20 px wide under this ruler, so "SESSION" has to cut.
        let mut sf = FakeSurface::new().text_at("type.ellipsis", ",");
        assert_eq!(fit_end(&mut sf, FONT_UI, 10.0, "SESSION", 20.0, 0.0), "SES,");
        // ...and a theme that states NOTHING trims with nothing rather
        // than with a character this file chose. The cut still happens —
        // the run is still made to fit — it simply goes unmarked, which
        // is the honest reading of a key nobody wrote.
        let mut bare = FakeSurface::new();
        assert_eq!(fit_end(&mut bare, FONT_UI, 10.0, "SESSION", 20.0, 0.0), "SESS");
    }

    #[test]
    fn a_look_off_a_surface_carries_the_case_its_role_asks_for() {
        use crate::view::surface::tests::FakeSurface;
        // A role exists for this resolver when it has a size; the rest of
        // the ladder is what the surface states.
        let sf = || {
            FakeSurface::new()
                .token("type.button.size", 14.0)
                .token("type.button.alpha", 1.0)
        };
        let mut up = sf().word_at("type.button.case", "upper");
        assert_eq!(role_look(&mut up, "button", 1.0).cased("Save"), "SAVE");
        let mut none = sf().word_at("type.button.case", "none");
        assert_eq!(role_look(&mut none, "button", 1.0).cased("Save"), "Save");
        // A word the list does not hold transforms nothing — the same
        // ruling `ui::Case::from_word` makes, because it IS that ruling.
        let mut typo = sf().word_at("type.button.case", "uper");
        assert_eq!(role_look(&mut typo, "button", 1.0).cased("Save"), "Save");
        // And nothing is drawn in a role that does not exist, so it does
        // not shout either.
        assert_eq!(NO_ROLE.cased("Save"), "Save");
    }

    // ---- wrapping ----

    #[test]
    fn text_that_fits_is_one_line_and_keeps_its_spacing_rules() {
        // 10 characters at 10 px = 50 px wide.
        assert_eq!(wrap(&mut Ruler, FONT_UI, 10.0, "abcdefghij", 50.0, 0.0), ["abcdefghij"]);
        assert_eq!(wrap(&mut Ruler, FONT_UI, 10.0, "", 50.0, 0.0), [""]);
    }

    #[test]
    fn a_line_breaks_at_the_last_word_that_fits() {
        // At 10 px a character is 5 px wide: 35 px holds "one two".
        assert_eq!(wrap(&mut Ruler, FONT_UI, 10.0, "one two three", 35.0, 0.0), ["one two", "three"]);
        // 30 px does not, so every word gets its own line.
        assert_eq!(
            wrap(&mut Ruler, FONT_UI, 10.0, "one two three", 30.0, 0.0),
            ["one", "two", "three"]
        );
    }

    #[test]
    fn a_word_wider_than_the_box_is_broken_rather_than_left_hanging() {
        assert_eq!(wrap(&mut Ruler, FONT_UI, 10.0, "abcdefghij", 25.0, 0.0), ["abcde", "fghij"]);
        // A short word before it still gets its own line first.
        assert_eq!(wrap(&mut Ruler, FONT_UI, 10.0, "x abcdef", 25.0, 0.0), ["x", "abcde", "f"]);
    }

    #[test]
    fn explicit_newlines_are_kept_and_a_nonsense_width_stops_wrapping() {
        assert_eq!(wrap(&mut Ruler, FONT_UI, 10.0, "one\ntwo", 500.0, 0.0), ["one", "two"]);
        // Zero width would otherwise break every character forever.
        assert_eq!(wrap(&mut Ruler, FONT_UI, 10.0, "one two", 0.0, 0.0), ["one two"]);
    }

    /// A break is a measurement, so the role's figure box has to reach
    /// it. The pair is the proof: the same string, the same width, and
    /// the only difference between the two calls is the box.
    #[test]
    fn a_break_is_measured_under_the_box_the_run_will_be_drawn_with() {
        // 60 px holds "one two" proportionally; under a box every run is
        // twice as wide, so each word takes a line of its own.
        assert_eq!(
            wrap_tab(&mut Ruler, FONT_UI, 10.0, "one two three", 60.0, 0.0, false),
            ["one two", "three"]
        );
        assert_eq!(
            wrap_tab(&mut Ruler, FONT_UI, 10.0, "one two three", 60.0, 0.0, true),
            ["one", "two", "three"]
        );
        // The character break inside one long word answers the box too:
        // five characters fit at 25 px, two under the box.
        assert_eq!(
            wrap_tab(&mut Ruler, FONT_UI, 10.0, "abcdefghij", 25.0, 0.0, true),
            ["ab", "cd", "ef", "gh", "ij"]
        );
    }

    /// No room is not a width to abbreviate to. Three trimmers in this
    /// library answered that way and this one answered "…", so the
    /// objects that wanted the toolkit's answer had to write it out
    /// themselves; the rule is stated once now.
    #[test]
    fn a_trim_with_no_room_at_all_draws_nothing_rather_than_an_ellipsis() {
        for room in [0.0, -1.0, -400.0] {
            assert_eq!(
                fit_end(&mut Ruler, FONT_UI, 10.0, "SESSION", room, 0.0),
                "",
                "room {room} produced a glyph to draw"
            );
        }
        // A width that holds something still holds the ellipsis: the
        // rule above is about NO room, not about tight room.
        assert_eq!(fit_end(&mut Ruler, FONT_UI, 10.0, "SESSION", 20.0, 0.0), "SES\u{2026}");
    }

    #[test]
    fn the_leading_number_is_read_and_never_invented() {
        assert_eq!(leading_number("41.2%"), Some(41.2));
        assert_eq!(leading_number("-3 of 4"), Some(-3.0));
        assert_eq!(leading_number("firefox"), None);
        assert_eq!(leading_number(""), None);
        assert_eq!(leading_number("..."), None);
    }

    // ---- severity ----

    use crate::view::surface::tests::FakeSurface;

    /// §5.10's fallback is the master's word, and the one answer this
    /// file may not give is a number of its own. A key naming a severity
    /// the closed set does not hold lands on its LAST rung — counted off
    /// the set rather than written down, so a master that adds a rung
    /// cannot leave a stale index pointing at somebody else's colour.
    #[test]
    fn an_unnameable_severity_fallback_lands_on_the_last_rung_and_never_on_ok() {
        let mut sf = FakeSurface::new().word_at("script.severity_fallback", "chartreuse");
        let sev = sev_fallback(&mut sf);
        assert_eq!(
            sev,
            Sev(SEVERITY_ROLES.len() as u16 - 1),
            "the fallback must be the last rung of the set, not a number in this file"
        );
        assert_ne!(SEVERITY_ROLES[sev.0 as usize], "ok", "§5.10 forbids the fallback being `ok`");
        // A word the set DOES hold still wins outright — the fallback is
        // the exception, not the rule.
        let mut sf = FakeSurface::new().word_at("script.severity_fallback", "warning");
        assert_eq!(sev_fallback(&mut sf), sev_of("warning").unwrap());
    }

    // ---- corners ----

    /// A badge 12 px tall and wider than it is tall — three characters
    /// at 5 px and 4 px of padding a side — on a surface that can draw
    /// rings. Wider matters: the capsule closes over the SHORTER side.
    fn badged(sf: FakeSurface) -> FakeSurface {
        let mut sf = sf
            .token("badge.h", 12.0)
            .token("badge.pad_x", 4.0)
            .token("type.body.size", 10.0)
            .token("type.body.leading", 1.0)
            .word_at("script.badge_role", "body");
        badge(
            &mut sf,
            Rect::new(0.0, 0.0, 100.0, 20.0),
            "hot",
            None,
            BadgeStyle::Hollow,
            Align::Left,
            1.0,
        );
        sf
    }

    #[test]
    fn a_pill_is_the_capsule_its_name_says_and_a_length_is_a_length() {
        let r = Rect::new(0.0, 0.0, 40.0, 12.0);
        // `pill` is a word, not a number: the sentinel it bakes to means
        // half the shorter side, which closes both ends.
        let pill = crate::theme::expr::sentinel("pill").unwrap();
        let mut sf = FakeSurface::new().token("x.corner", pill);
        assert_eq!(corner_radius(&mut sf, "x.corner", r, 1.0), 6.0);
        // A stated radius is itself, shrunk with everything else...
        let mut sf = FakeSurface::new().token("x.corner", 3.0);
        assert_eq!(corner_radius(&mut sf, "x.corner", r, 1.0), 3.0);
        assert_eq!(corner_radius(&mut sf, "x.corner", r, 0.5), 1.5);
        // ...and never past the point where two corners would cross.
        let mut sf = FakeSurface::new().token("x.corner", 100.0);
        assert_eq!(corner_radius(&mut sf, "x.corner", r, 1.0), 6.0);
        // The rest of the sentinel table is not a radius at all, and
        // says so on the way to drawing nothing.
        let auto = crate::theme::expr::sentinel("auto").unwrap();
        let mut sf = FakeSurface::new().token("x.corner", auto);
        assert_eq!(corner_radius(&mut sf, "x.corner", r, 1.0), 0.0);
    }

    // ---- shape presets ----

    /// A preset standing where `default.theme` stands it: a pair on
    /// `corners`, and all four per-corner keys inheriting both halves.
    fn preset_at(style: &str, size: f32) -> FakeSurface {
        let same = crate::theme::expr::sentinel("same_as_parent").unwrap();
        let mut sf = FakeSurface::new()
            .word_at("shape.p.corners[0]", style)
            .token("shape.p.corners[1]", size);
        for slot in ["tl", "tr", "br", "bl"] {
            sf = sf
                .token(&format!("shape.p.corners_{slot}[0]"), same)
                .token(&format!("shape.p.corners_{slot}[1]"), same);
        }
        sf
    }

    /// The four corners come out the preset's own until a per-corner key
    /// says otherwise — and each HALF of that key is independent, which
    /// is the whole reason the master writes it as a pair.
    #[test]
    fn four_corners_inherit_the_preset_and_each_slot_overrides_alone() {
        let r = Rect::new(0.0, 0.0, 80.0, 40.0);
        let mut sf = preset_at("round", 6.0);
        let p = preset(&mut sf, "shape.p", r);
        assert_eq!(p.kind, ShapeKind::Box);
        assert_eq!(p.corners, [Corner::round(6.0); 4], "an inheriting preset is uniform");

        // The STYLE alone, at the radius the preset already carries —
        // the case that a one-slot token could never have expressed.
        // A stated slot no longer reads the sentinel — the scalar is
        // what says "inherit", so a real word arrives with it cleared.
        let mut sf = preset_at("round", 6.0)
            .token("shape.p.corners_tr[0]", 0.0)
            .word_at("shape.p.corners_tr[0]", "chamfer");
        let p = preset(&mut sf, "shape.p", r);
        assert_eq!(p.corners[1], Corner::chamfer(6.0), "the tr style did not stand alone");
        assert_eq!(p.corners[0], Corner::round(6.0), "the tl corner moved with it");

        // The SIZE alone, keeping the preset's cut.
        let mut sf = preset_at("round", 6.0).token("shape.p.corners_bl[1]", 2.0);
        let p = preset(&mut sf, "shape.p", r);
        assert_eq!(p.corners[3], Corner::round(2.0));
        assert_eq!(p.corners[2], Corner::round(6.0));

        // `pill` on one corner alone: §5.0's sentinel is a length ABOUT
        // the box, and it is resolved on the box the caller passed.
        let pill = crate::theme::expr::sentinel("pill").unwrap();
        let mut sf = preset_at("round", 6.0).token("shape.p.corners_br[1]", pill);
        let p = preset(&mut sf, "shape.p", r);
        assert_eq!(p.corners[2], Corner::round(20.0), "pill is half the short side");
    }

    /// `chevron` and `hexagon` are SHAPE words, and this is where the
    /// master's own open question is answered: they name a kind, they do
    /// not grow `CornerStyle`, and on a per-corner key they mean nothing
    /// and inherit rather than squaring the corner off.
    #[test]
    fn a_shape_word_names_the_kind_and_never_one_corner() {
        let r = Rect::new(0.0, 0.0, 80.0, 40.0);
        let mut sf = preset_at("hexagon", 0.0).word_at("shape.p.orientation", "pointy");
        assert_eq!(
            preset(&mut sf, "shape.p", r).kind,
            ShapeKind::Hex { turn: std::f32::consts::FRAC_PI_6 }
        );
        // A preset that names no orientation gets no turn — not one this
        // file picked for it.
        let mut sf = preset_at("hexagon", 0.0);
        assert_eq!(preset(&mut sf, "shape.p", r).kind, ShapeKind::Hex { turn: 0.0 });

        // Depth is a fraction of the HEIGHT, per end, and the direction
        // says which ends get it.
        let mut sf = preset_at("chevron", 0.0)
            .token("shape.p.chevron_depth", 0.5)
            .word_at("shape.p.chevron_dir", "both");
        assert_eq!(
            preset(&mut sf, "shape.p", r).kind,
            ShapeKind::Chevron { left: 20.0, right: 20.0 }
        );
        let mut sf = preset_at("chevron", 0.0)
            .token("shape.p.chevron_depth", 1.0)
            .word_at("shape.p.chevron_dir", "right");
        assert_eq!(
            preset(&mut sf, "shape.p", r).kind,
            ShapeKind::Chevron { left: 0.0, right: 40.0 },
            "the master's own paging arrow"
        );

        // A shape word where a CORNER was expected is not a corner: the
        // preset's own cut answers instead, and no square appears from
        // nowhere.
        let mut sf = preset_at("round", 6.0)
            .token("shape.p.corners_tl[0]", 0.0)
            .word_at("shape.p.corners_tl[0]", "hexagon");
        let p = preset(&mut sf, "shape.p", r);
        assert_eq!(p.kind, ShapeKind::Box, "one corner cannot make a hexagon");
        assert_eq!(p.corners[0], Corner::round(6.0));
    }

    // ---- roles ----

    /// A theme with `body` fully stated and a readable global floor —
    /// everything a fallback to `body` would need to look plausible.
    fn typeset() -> FakeSurface {
        FakeSurface::new()
            .token("type.body.size", 10.0)
            .token("type.body.leading", 1.4)
            .token("type.min_px", 8.0)
    }

    #[test]
    fn a_binding_that_names_no_role_draws_nothing_at_all() {
        // A binding standing at no word: neither the master nor the theme
        // said what this text is, so nothing is what it looks like.
        let mut sf = typeset();
        let look = bound_role(&mut sf, "list.label_role", 1.0);
        assert_eq!(look.px, 0.0, "the global floor must not apply to a role that is absent");
        assert_eq!(look.leading, 0.0);
        assert_eq!(look.color.a, 0.0);

        // A binding standing at a name no `type.*` block declares is the
        // same hole reached through the other door — and `body` sitting
        // right there, fully stated, is exactly the trap: a fallback to
        // it renders a broken theme as a nearly-right interface.
        let mut sf = typeset().word_at("list.label_role", "no_such_role");
        let look = bound_role(&mut sf, "list.label_role", 1.0);
        assert_eq!(look.px, 0.0);
        assert_eq!(look.color.a, 0.0);
    }

    #[test]
    fn a_declared_role_obeys_its_own_floor_and_its_own_ceiling() {
        let mut sf = typeset().word_at("list.label_role", "body");
        assert_eq!(bound_role(&mut sf, "list.label_role", 1.0).px, 10.0);
        // Shrunk under the GLOBAL floor, a role the master declares still
        // stops there: `type.min_px` is the last defence against
        // unreadable type.
        assert_eq!(bound_role(&mut sf, "list.label_role", 0.1).px, 8.0);
        // The role's own floor wins over the global one when it is higher.
        let mut sf = typeset().word_at("list.label_role", "body").token("type.body.min_px", 12.0);
        assert_eq!(bound_role(&mut sf, "list.label_role", 1.0).px, 12.0);
        // A ceiling caps the size...
        let mut sf = typeset().word_at("list.label_role", "body").token("type.body.max_px", 9.0);
        assert_eq!(bound_role(&mut sf, "list.label_role", 1.0).px, 9.0);
        // ...and `0px` is how the master spells "uncapped", which must
        // never read as a ceiling of nothing.
        let mut sf = typeset().word_at("list.label_role", "body").token("type.body.max_px", 0.0);
        assert_eq!(bound_role(&mut sf, "list.label_role", 1.0).px, 10.0);
        // A ceiling under the floor is a theme contradicting itself, and
        // the floor is the one that wins.
        let mut sf = typeset()
            .word_at("list.label_role", "body")
            .token("type.body.min_px", 12.0)
            .token("type.body.max_px", 4.0);
        assert_eq!(bound_role(&mut sf, "list.label_role", 1.0).px, 12.0);
    }

    #[test]
    fn the_cut_is_the_word_the_theme_wrote_and_nothing_else() {
        let mut sf = FakeSurface::new().word_at("x.corner_style", "round");
        assert_eq!(corner_style(&mut sf, "x.corner_style"), CornerStyle::Round);
        let mut sf = FakeSurface::new().word_at("x.corner_style", "chamfer");
        assert_eq!(corner_style(&mut sf, "x.corner_style"), CornerStyle::Chamfer);
        // `chevron` is in the presets' vocabulary and in no surface's,
        // and a theme that says nothing has said nothing.
        let mut sf = FakeSurface::new().word_at("x.corner_style", "chevron");
        assert_eq!(corner_style(&mut sf, "x.corner_style"), CornerStyle::Square);
        let mut sf = FakeSurface::new();
        assert_eq!(corner_style(&mut sf, "x.corner_style"), CornerStyle::Square);
    }

    #[test]
    fn a_badge_wears_the_radius_and_the_cut_the_theme_states() {
        let pill = crate::theme::expr::sentinel("pill").unwrap();
        // The master's own pair: the pill sentinel and `round`.
        let sf = badged(
            FakeSurface::new()
                .token("badge.corner", pill)
                .word_at("shape.badge.corners[0]", "round"),
        );
        assert!(sf.rects.is_empty(), "a shaped badge is not a rectangle");
        assert_eq!(sf.rings.len(), 1);
        let (r, style, radius) = sf.rings[0];
        assert_eq!(r.h, 12.0);
        assert_eq!(style, CornerStyle::Round);
        assert_eq!(radius, 6.0, "half the 12 px height: the capsule");
        // Move the radius token, and the pill stops being one.
        let sf = badged(
            FakeSurface::new()
                .token("badge.corner", 2.0)
                .word_at("shape.badge.corners[0]", "round"),
        );
        assert_eq!(sf.rings[0].2, 2.0);
        // Move the style token, and the same radius is cut differently.
        let sf = badged(
            FakeSurface::new()
                .token("badge.corner", pill)
                .word_at("shape.badge.corners[0]", "chamfer"),
        );
        assert_eq!(sf.rings[0].1, CornerStyle::Chamfer);
        assert_eq!(sf.rings[0].2, 6.0);
    }

    #[test]
    fn a_hollow_badge_strokes_the_same_shape_it_filled() {
        // Two rings, one geometry: a ring drawn on a different radius
        // from its fill is the two-shapes bug in miniature.
        let pill = crate::theme::expr::sentinel("pill").unwrap();
        let sf = badged(
            FakeSurface::new()
                .token("badge.corner", pill)
                .token("badge.border", 1.0)
                .word_at("shape.badge.corners[0]", "chamfer"),
        );
        assert_eq!(sf.strokes.len(), 1);
        let (fill_r, fill_style, fill_radius) = sf.rings[0];
        let (edge_r, edge_style, edge_radius) = sf.strokes[0];
        assert_eq!((fill_r.x, fill_r.y, fill_r.w, fill_r.h), (edge_r.x, edge_r.y, edge_r.w, edge_r.h));
        assert_eq!(fill_style, edge_style);
        assert_eq!(fill_radius, edge_radius);
    }

    #[test]
    fn the_master_states_both_halves_of_a_badges_corner() {
        // The proof the drawing tests stand on: these are the values the
        // shipped theme really holds, so the pill above is the pill the
        // user sees.
        let t = crate::theme::resolved();
        let id = |n: &str| crate::theme::id(n).expect("declared in the master");
        assert_eq!(
            t.px(id("badge.corner")),
            crate::theme::expr::sentinel("pill").unwrap(),
            "the master asks for a capsule, which used to bake to a square"
        );
        // Asked the way `CtxSurface::word` asks it, so this is the
        // answer the drawing really gets and not a second reading of the
        // same file.
        assert_eq!(
            crate::ui::theme_word(id("shape.badge.corners[0]")),
            "round",
            "the style half of the preset, which the badge now reads"
        );
    }

    /// The thumb and the bar, whose corner tokens the audit found
    /// declared and read by NOTHING: the master writes `@corner.pill` on
    /// a scrollbar thumb and the drawing was `rect`, so the one token a
    /// theme has for the shape of a thumb moved nothing at all. Both
    /// halves of the pair are measured — the radius the token states and
    /// the cut its `*_corner_style` sibling names — because a radius
    /// with no cut is the same silence one step along.
    #[test]
    fn the_thumb_and_the_bar_wear_the_corner_their_own_tokens_state() {
        let pill = crate::theme::expr::sentinel("pill").unwrap();
        // A plate, because a thumb with no fill colour is never drawn
        // and a test about its SHAPE would then measure nothing.
        let thumbed = |radius: f32, cut: &str| -> FakeSurface {
            let geom = crate::view::scroll::ScrollbarGeom {
                track: Rect::new(90.0, 0.0, 10.0, 100.0),
                thumb: Rect::new(90.0, 20.0, 10.0, 40.0),
            };
            let mut sf = FakeSurface::new()
                .plate(Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 })
                .token("scrollbar.corner", radius)
                .word_at("scrollbar.corner_style", cut);
            scrollbar(&mut sf, &geom, 1.0, false, false);
            sf
        };

        let sf = thumbed(pill, "round");
        assert!(sf.rects.is_empty(), "a shaped thumb is not a rectangle");
        assert_eq!(sf.rings.len(), 1);
        assert_eq!(sf.rings[0].1, CornerStyle::Round);
        assert_eq!(sf.rings[0].2, 5.0, "half the thumb's 10 px width: the capsule");
        // A stated length is itself, and the cut is the sibling's word:
        // move either token and the thumb moves with it.
        assert_eq!(thumbed(2.0, "round").rings[0].2, 2.0);
        assert_eq!(thumbed(pill, "chamfer").rings[0].1, CornerStyle::Chamfer);
        // `@corner.none` is a literal zero and stays the slab it asks
        // for: reading a token is not deciding for the theme.
        assert_eq!(thumbed(0.0, "round").rings[0].2, 0.0);

        // The bar is the same pair on the other element, and its fill
        // wears the track's corner inset by its own inset — a
        // square-ended fill inside a rounded track hangs out past the cap.
        let r = Rect::new(0.0, 0.0, 100.0, 10.0);
        let mut sf = FakeSurface::new()
            .token("progress.corner", pill)
            .word_at("progress.corner_style", "round");
        meter(&mut sf, r, 1.0, None, true);
        assert_eq!(sf.strokes.len(), 1, "the track's ring");
        assert_eq!(sf.strokes[0].1, CornerStyle::Round);
        assert_eq!(sf.strokes[0].2, 5.0, "half the bar's 10 px height");
        assert_eq!(sf.rings.len(), 1, "the fill");
        assert_eq!(sf.rings[0].2, 5.0);
        // The shipped radius is zero, and the bar drawn from it is the
        // rectangle the master asked for.
        let mut sf = FakeSurface::new().word_at("progress.corner_style", "round");
        meter(&mut sf, r, 1.0, None, true);
        assert_eq!(sf.rings[0].2, 0.0);
    }

    /// The values behind the test above, in the shipped file: the tokens
    /// the audit found unread are read, and what they say is what the
    /// user sees.
    #[test]
    fn the_master_states_a_corner_for_the_thumb_and_for_the_bar() {
        let t = crate::theme::resolved();
        let id = |n: &str| crate::theme::id(n).expect("declared in the master");
        assert_eq!(
            t.px(id("scrollbar.corner")),
            crate::theme::expr::sentinel("pill").unwrap(),
            "the master asks for a capsule thumb, which used to bake to a slab"
        );
        // Asked the way `CtxSurface::word` asks it, so this is the answer
        // the drawing really gets and not a second reading of the file.
        assert_eq!(crate::ui::theme_word(id("scrollbar.corner_style")), "round");
        assert_eq!(t.px(id("progress.corner")), 0.0, "`@corner.none` is a length of zero");
        assert!(
            !crate::ui::theme_word(id("progress.corner_style")).is_empty(),
            "a radius with no cut is the same silence one step along"
        );
    }

    // ---- the triangle that says a thing opens ----

    /// The one triangle `disclosure` drew, as its three points.
    fn triangle(kind: Disclosure, expanded: bool) -> Vec<[f32; 2]> {
        let mut sf = FakeSurface::new();
        // A 10 px glyph in a 10 px line box: the box drops out of the
        // arithmetic, so what is left is the shape and only the shape.
        disclosure(&mut sf, 0.0, 0.0, 10.0, 10.0, kind, expanded, Color::TRANSPARENT);
        assert_eq!(sf.polylines.len(), 1, "a disclosure is one closed outline");
        sf.polylines.remove(0)
    }

    /// Where a three-point triangle points, read off its own geometry:
    /// the apex is the corner that shares no coordinate with the other
    /// two, and the direction is where it sits relative to them.
    fn points(pts: &[[f32; 2]]) -> &'static str {
        assert_eq!(pts.len(), 3);
        let same = |a: f32, b: f32| (a - b).abs() < 0.01;
        // The apex stands alone on both axes; the other two hold the
        // edge it points away from.
        let apex = *pts
            .iter()
            .find(|p| {
                pts.iter().filter(|q| same(q[0], p[0])).count() == 1
                    && pts.iter().filter(|q| same(q[1], p[1])).count() == 1
            })
            .expect("a triangle with no apex is not one of ours");
        let base: Vec<[f32; 2]> = pts.iter().copied().filter(|p| *p != apex).collect();
        if same(base[0][0], base[1][0]) {
            "right"
        } else if apex[1] > base[0][1] {
            "down"
        } else {
            "up"
        }
    }

    /// The two grammars, which are the whole reason the parameter
    /// exists. A tree says "there is more inside this row" and points
    /// ALONG it when shut; a drop-down's caret says "the list comes out
    /// downwards" and points DOWN when shut — the convention GTK, Qt and
    /// `select` share. Drawing a tree's `▷` on a list anchor tells the
    /// user to walk into the row, which is a different offer entirely.
    #[test]
    fn a_caret_points_where_its_own_convention_says_and_not_where_a_trees_does() {
        assert_eq!(points(&triangle(Disclosure::Tree, false)), "right");
        assert_eq!(points(&triangle(Disclosure::Tree, true)), "down");
        assert_eq!(points(&triangle(Disclosure::Drop, false)), "down");
        assert_eq!(points(&triangle(Disclosure::Drop, true)), "up");
        // Open and shut are different SHAPES in both grammars — the
        // state turns the glyph, and a caret that never turned would
        // leave the open list unannounced.
        assert_ne!(triangle(Disclosure::Tree, false), triangle(Disclosure::Tree, true));
        assert_ne!(triangle(Disclosure::Drop, false), triangle(Disclosure::Drop, true));
        // The shut drop and the open tree are the same "down" arrow, and
        // that is the point of one primitive: there is exactly one of it.
        assert_eq!(triangle(Disclosure::Drop, false), triangle(Disclosure::Tree, true));
    }

    #[test]
    fn a_disclosure_with_no_box_to_draw_in_draws_nothing() {
        let mut sf = FakeSurface::new();
        disclosure(&mut sf, 0.0, 0.0, 0.0, 10.0, Disclosure::Drop, false, Color::TRANSPARENT);
        assert!(sf.polylines.is_empty(), "a glyph the theme sized to nothing is not drawn");
    }

    // ---- the rule every trimmed label follows ----


    const FULL: &str = "org.freedesktop.NetworkManager";
    const CUT: &str = "org.freedesk\u{2026}";

    fn anchor() -> Rect {
        Rect::new(10.0, 10.0, 100.0, 20.0)
    }

    #[test]
    fn a_label_the_ellipsis_cut_short_asks_for_the_whole_of_itself() {
        let mut sf = FakeSurface::new().at(20.0, 15.0);
        explain_trim(&mut sf, 7, anchor(), CUT, FULL);
        assert_eq!(sf.tips.len(), 1);
        let (id, r, text) = &sf.tips[0];
        assert_eq!(*id, 7, "the caller's identity, untouched");
        assert_eq!(text, FULL, "the whole of it — the stump is already on screen");
        // The anchor comes back as it went in: the box is placed against
        // what was pointed at, not against the pointer alone.
        assert_eq!((r.x, r.y, r.w, r.h), (10.0, 10.0, 100.0, 20.0));
    }

    #[test]
    fn a_label_that_arrived_whole_says_nothing() {
        // The pointer is resting on it, and there is still nothing to
        // add: a tooltip repeating what is already legible is noise.
        let mut sf = FakeSurface::new().at(20.0, 15.0);
        explain_trim(&mut sf, 7, anchor(), FULL, FULL);
        assert!(sf.tips.is_empty());
        // The empty label is the same case and not a special one.
        explain_trim(&mut sf, 7, anchor(), "", "");
        assert!(sf.tips.is_empty());
    }

    #[test]
    fn a_trimmed_label_the_pointer_left_asks_for_nothing() {
        // Off the anchor entirely: this is how a tooltip goes away —
        // the frame files no request, and the manager disarms.
        let mut sf = FakeSurface::new().at(200.0, 15.0);
        explain_trim(&mut sf, 7, anchor(), CUT, FULL);
        assert!(sf.tips.is_empty());
        // The far edges are the containment rule's, not a second one:
        // the top-left corner is inside, the bottom-right is not.
        let mut on = FakeSurface::new().at(10.0, 10.0);
        explain_trim(&mut on, 7, anchor(), CUT, FULL);
        assert_eq!(on.tips.len(), 1);
        let mut off = FakeSurface::new().at(110.0, 30.0);
        explain_trim(&mut off, 7, anchor(), CUT, FULL);
        assert!(off.tips.is_empty());
    }

    /// A shape word on a preset that carries none of the numbers it
    /// implies degrades to the rect, and the reader has to SAY so.
    ///
    /// The master offers `chevron | hexagon` in the `corners` comment of
    /// all sixteen presets and declares the keys those two words need on
    /// exactly two of them. A theme that takes the comment at its word
    /// on any of the other fourteen reads zero for every missing key,
    /// and a chevron of depth zero is the rectangle it was cut from — so
    /// the shape it asked for simply does not happen, with nothing said.
    #[test]
    fn a_shape_word_without_its_numbers_is_named_and_not_swallowed() {
        let _ = crate::theme::resolved();
        // The two presets the master equips: they have what their own
        // word needs, so nothing is reported for them.
        assert!(undeclared("shape.taskbar", &["chevron_depth", "chevron_dir"]).is_empty());
        assert!(undeclared("shape.hex", &["orientation"]).is_empty());
        // And the fourteen that do not. `shape.card` is one of them, and
        // its `corners` comment offers `chevron` all the same.
        assert_eq!(
            undeclared("shape.card", &["chevron_depth", "chevron_dir"]),
            ["chevron_depth", "chevron_dir"],
            "shape.card grew the chevron's keys — pick another preset for this test"
        );
        assert_eq!(undeclared("shape.panel", &["orientation"]), ["orientation"]);
        // A key that IS declared is never named, even beside one that is
        // not: the report is the list of what is actually missing.
        assert_eq!(undeclared("shape.taskbar", &["chevron_depth", "orientation"]), ["orientation"]);
    }
}
