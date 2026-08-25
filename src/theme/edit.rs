//! THE MODEL BEHIND THE THEME EDITOR: what a control means in tokens.
//!
//! The editor shows four controls — a border kind, a border colour, a
//! background kind, and a background colour. None of those is a token. Each
//! is a NAME for a set of tokens that have to move together, and this module
//! is the only place that knows the sets. The view picks names and drags
//! sliders; the file is written from what comes out of here.
//!
//! WHY A MODULE AND NOT A FEW LINES IN THE SETTINGS WINDOW: the same question
//! — "what does `neon` mean" — is asked when the editor OPENS (to show the
//! current state), while a slider MOVES (to preview), and when SAVE writes.
//! Three callers, one answer, or they drift.
//!
//! AND ALL THREE READ SILENCE THE SAME WAY: a token this module does not
//! mention is a token left standing. That is what an overlay does by
//! construction, and since 2026-08-18 it is what a save does too — `theme::
//! save_theme` PATCHES the file where a value stands instead of generating
//! it whole. The two used to disagree, and the disagreement was a bug the
//! owner could see: the halo a theme dressed itself survived every preview
//! and went out on the first SAVE, because withholding a value means "keep
//! it" to a bake and "delete it" to a rewrite. One set cannot serve three
//! callers who read its silences differently, so the file was taught to read
//! silence like the others rather than this module taught to stop being
//! silent.
//!
//! That question changed its answer on 2026-08-18, which is the best argument
//! for the module there has been: `neon` used to mean a blurred copy of the
//! border and now means a lit glass tube, the blurred copy is called `glow`,
//! and a theme file saved under the old name has to keep opening on the thing
//! it actually draws. All of that is [`Border`] and [`border_edits`] — three
//! callers, one answer, and one place to change it.
//!
//! TWO PAGES, ONE MODEL. The sets above are the editor's ADVANCED page: one
//! control per thing. The BASIC page at the bottom of this file is the same
//! theme asked three questions — HUE, SATURATION, LIGHTNESS — and answers
//! them by moving the tokens the others are DERIVED from ([`tone_edits`]).
//! Neither page eats the other's work: BASIC is a move RELATIVE to the theme
//! as it stands, so leaving it folds the move into the file and re-opens at
//! rest ([`ToneSeeds::shifted`]).
//!
//! # What this module deliberately does NOT offer
//!
//! Only what the renderer actually draws. The theme language declares far more
//! than the code reads — the `[elev.*]` ladder has nine rungs of about thirty
//! keys and seven of them reach the screen — and a control wired to a token
//! nobody reads is worse than a missing control, because it looks like it
//! works. Measured 2026-08-16, with the anchors kept next to each set below.
//!
//! The gap narrows as the renderer learns: on 2026-08-16 the panel rung's
//! glass became real (`elev::Level` and `window::frame` both read
//! `elev.panel.glass.*`, and `glass.rank` gained its first reader), so the
//! background sets below write it. The fixture rung keeps its own hand-made
//! path in `deco.rs` and is deliberately NOT addressed here — the owner's
//! scope for a background is windows and widgets, never the desktop's own
//! decoration.

use super::color::Oklch;

/// WHERE an edit lands.
///
/// One value today, and the type exists anyway. The owner's plan is per-widget
/// and per-window settings later, and the difference between "write these
/// tokens" and "write these tokens IN SCOPE S" is a rewrite of the save path
/// if it arrives late and one more match arm if it arrives early. The editor
/// shows no scope picker; the model already has one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    /// Every surface at once — the whole theme.
    Theme,
}

/// The three borders the editor offers.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Border {
    /// A ring and nothing else.
    Line,
    /// The same ring with a halo around it — a blurred copy of the edge
    /// in the edge's own colour.
    ///
    /// CALLED `Neon` UNTIL 2026-08-18, and the name was wrong twice over:
    /// it is not what a neon sign looks like, and it took the word the
    /// owner wanted for the thing that is. The tokens it writes have not
    /// moved and neither has the picture — the halo is drawn from the
    /// same four keys it always was, so a theme file written under the
    /// old name opens on this kind and looks the same to the bit.
    ///
    /// The halo has NO COLOUR OF ITS OWN. `object/window.rs` passes the
    /// ring's colour into the emitter, and `glow.panel_edge.color` is
    /// declared in the master and read by nobody. So one colour drives
    /// both, which is why the editor has one set of colour sliders for
    /// the border rather than two.
    Glow,
    /// A lit glass tube: a core burned toward white by the drive on it, a
    /// saturated band of colour against the glass, and light that stops
    /// instead of fading.
    ///
    /// THE KIND WRITES A PROFILE, NOT A LOOK. Every number the tube is
    /// made of — how hard the core is driven, how strong the band is, how
    /// far it reaches, how fast the light falls — is a token of the
    /// theme's, and this module names none of them: the one word
    /// `glow.panel_edge.falloff = tube` is what turns the four keys the
    /// halo already used into a tube, and the master carries the tube's
    /// own dress beside them. A theme that wants a different tube edits
    /// the theme, not this list.
    Neon,
}

/// The three backgrounds the editor offers.
///
/// These are PRESETS over the glass pair, not tokens: nothing in the theme
/// language selects "blur" as a word. Glass is two quads — `glass.tint`
/// multiplies (so it can only darken and hue-shift) and `glass.wash` lays
/// over with alpha (the only one that can brighten) — and the master says a
/// single "glass colour" token would be a bug. A preset is the honest way to
/// give the three names the owner asked for: each one is a shape of that pair.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Glass {
    /// No glass: the surface's own fill, opaque.
    Solid,
    /// The scene behind, blurred, with the tint left neutral.
    Blur,
    /// Blurred, tinted and washed, so nothing reads through.
    Frosted,
}

/// One token, and the text to write for it.
///
/// The value is TEXT, not a parsed value, because it is going into a file that
/// is patched byte by byte — a save replaces the bytes of a value span and
/// leaves every comment and every other byte where it was
/// (`theme::save_theme`, and `parse::code_len` is how it finds where the
/// author's note begins). Handing a `Color` to the writer would mean the
/// writer decides how a colour is spelled, and then two places know.
///
/// This paragraph described the intent for a year and the code for none of
/// it: until 2026-08-18 the writer threw the file away and printed a new one
/// from the set. Anything read here as a promise about the file is a promise
/// the save now actually keeps.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Edit {
    pub token: &'static str,
    pub value: String,
}

impl Edit {
    fn new(token: &'static str, value: impl Into<String>) -> Self {
        Self { token, value: value.into() }
    }
}

/// A colour as the theme language spells it: `oklch(L, C, H)`.
///
/// Written rather than assembled from a `Color`, so the three sliders land in
/// the file as the three numbers the owner moved. The alternative — convert to
/// sRGB and write a hex triple — throws the chroma and hue away and makes the
/// next edit start from a rounded value.
///
/// The `/ a` tail is the FUNCTION's alpha and never a component suffix
/// (`parse.rs:1087`), so it is only written when the colour is not opaque.
pub fn oklch_literal(c: Oklch) -> String {
    if c.alpha >= 1.0 {
        format!("oklch({:.4}, {:.4}, {:.2})", c.l, c.c, c.h)
    } else {
        format!("oklch({:.4}, {:.4}, {:.2} / {:.3})", c.l, c.c, c.h, c.alpha)
    }
}

// ---------------------------------------------------------------- the sets

/// Tokens that carry the border, for one scope.
///
/// `elev.panel.edge.color` and `.width` are what `elev::Level` reads; the
/// glow keys are what `panel_edge_glow` reads (`object/window.rs`). Of the
/// four other `edge.*` keys the master declares, THREE gained a reader on
/// 2026-08-17 — `color2`, `mode` and `axis` are the two-stop sugar pair, and
/// `elev::Level::edge_gradient` reads all three — but the editor has no
/// GRADIENT border kind to offer yet, so it still writes none of them and
/// the two it does write stay neutral about them. `gradient` (the named
/// multi-stop slot) remains unread: the engine bakes no stop arrays at all,
/// which is a theme-engine job and is written up at `edge_gradient`.
/// Writing a key nothing reads would put a value in the file that changes
/// nothing — exactly how the cockpit theme (shipped until 2026-08-16) asked
/// for a gradient border and got a flat one.
/// The one edit that changes the border's COLOUR and nothing else.
///
/// Split out for the state the editor opens in: no border kind chosen yet.
/// Sending a kind then would not be neutral — LINE's switch turns the halo
/// off — so a colour slider moved before the list is touched must move the
/// colour alone. (Verified finding, 2026-08-16: the earlier shape mapped
/// "no choice" to LINE and a colour drag switched five themes' halos off
/// as a side effect.)
///
/// IT WRITES THE SHARED ROOT `border.default`, NOT `elev.panel.edge.color`.
/// The leaf is what `elev::Level` reads for the window's own ring, but it is
/// only ONE frame: every rung takes its `edge.color` from
/// `@component.panel.border -> @border.default`, and the [`class`] ladder
/// gives `panel`, `window` and `dialog` — a plugin widget's frame among
/// them — a base of `@border.default` too. Writing the leaf moved the
/// settings window's edge and left every widget and every other rung on the
/// theme's own colour, so the frames stopped matching and a widget's ring
/// did not follow the picker. Writing the root moves them together, which is
/// what "one model of a window" asks for and is symmetric with
/// [`border_width_edit`], which has always written the root `border.edge.width`.
pub fn border_colour_edit(scope: Scope, colour: Oklch) -> Edit {
    let Scope::Theme = scope;
    Edit::new("border.default", oklch_literal(colour))
}

/// `halo_dressed` answers "does the theme already draw a visible halo" —
/// resolved radius AND alpha both above zero. The caller reads it ONCE, off
/// the theme as the file has it, when the editor opens; this function stays
/// pure so the tests need no engine.
///
/// The "once, off the file" half is not a detail. Asking the LIVE bake made
/// this set an input to itself — the preview it produced was the answer the
/// next call read — and the halo blinked five times a second while a slider
/// moved (`.gap-program/usterka-edytor-suwaki-glow.md`, usterka 2). What is
/// answered here has to be a fact about the THEME, never about the preview
/// standing on it.
pub fn border_edits(scope: Scope, kind: Border, colour: Oklch, halo_dressed: bool) -> Vec<Edit> {
    let mut out = vec![border_colour_edit(scope, colour)];
    match kind {
        // The theme's own radius and alpha are left standing: LINE only
        // takes the light away, and `enabled = false` is the whole of that
        // (`panel_edge_glow` returns before either is read). The falloff
        // is left standing for the same reason — a kind that draws no
        // light has no opinion about its shape.
        Border::Line => out.push(Edit::new("glow.panel_edge.enabled", "false")),
        // NEON dresses the halo ONLY where the theme has not: the default
        // master ships `radius = 0u` and `alpha = 0.0` and `window.rs:104`
        // returns at zero, so a bare switch was invisible there. A theme
        // that has dressed its own halo keeps its dress — the shipped
        // variants (removed 2026-08-16) each wore their own numbers, from
        // azure's 0.6u/0.16 to cockpit's 1.6u/0.34, and writing the seeds
        // over all five was the earlier shape's mistake, found in
        // verification; a user's theme deserves the same respect.
        //
        // KEEPING A DRESS IS SAYING NOTHING, and the save has to hear that
        // the way a bake does. It does since 2026-08-18: the file is
        // patched, so the author's `radius = 2.40u` is simply not one of
        // the lines a save rewrites — and the file that is patched is THIS
        // theme's, under whatever name the save lands (`theme::
        // save_theme_as`). SAVE AS is a copy of the theme on screen, which
        // is what makes that true for a new name and for a name that
        // already carries someone else's file alike. The first draft of
        // this comment claimed only a brand-new name could still cost the
        // dress; saving over an EXISTING theme cost it too, and worse — the
        // saved theme wore that file's halo and matched neither what was on
        // screen nor what it was saved over.
        //
        // What is left is not a hole but the absence of a source: saved off
        // the master, which is not a file, the set IS the whole theme and
        // the halo is the seed below, because there was never a dress to
        // keep.
        //
        // The two lit kinds differ in ONE token, which is the point: a
        // tube is the same light spent differently, so switching between
        // them must not disturb the radius, the alpha or the colour the
        // user chose for either.
        //
        // GLOW WRITES ITS WORD OUT LOUD rather than leaving the key
        // alone. The kind is a promise about the profile, and the only
        // way to keep it on a file that already says `tube` is to say
        // `gauss` — the master's own word for the soft halo. The cost is
        // that a theme which had written `halo` or `quad` there has that
        // word replaced; those three words differ only in a reader that
        // does not exist, so the picture is the same either way, and a
        // key that silently disagreed with the list would be worse.
        Border::Glow | Border::Neon => {
            out.push(Edit::new("glow.panel_edge.enabled", "true"));
            out.push(Edit::new(
                "glow.panel_edge.falloff",
                if kind == Border::Neon { "tube" } else { "gauss" },
            ));
            // A lit kind dresses the light ONLY where the theme has not:
            // the default master ships `radius = 0u` and `alpha = 0.0`
            // and `panel_edge_glow` returns at zero, so a bare switch was
            // invisible there. A theme that has dressed its own keeps its
            // dress — the shipped variants (removed 2026-08-16) each wore
            // their own numbers, from azure's 0.6u/0.16 to cockpit's
            // 1.6u/0.34, and writing the seeds over all five was the
            // earlier shape's mistake, found in verification; a user's
            // theme deserves the same respect.
            //
            // The two seeds are the REACH and the AMOUNT of the light,
            // which both kinds need and neither owns. Nothing seeds the
            // tube's own dress — its drive, its band and its decay come
            // from the master, which states them for exactly this reason:
            // a kind picked in a list must not be a place where a look is
            // decided in code.
            if !halo_dressed {
                out.push(Edit::new("glow.panel_edge.radius", "1.6u"));
                out.push(Edit::new("glow.panel_edge.alpha", "0.34"));
            }
        }
    }
    out
}

/// The BORDER'S OWN THICKNESS — `border.edge.width`, and nothing else.
///
/// SEVEN KEYS OF GEOMETRY AND NOT ONE OF THEM IS A RADIUS. `[border.edge]`
/// (default.theme:432-446) declares `width`, `style`, `dash`, `gap`,
/// `phase`, `bracket_len` and `bracket_inset`. A border in this theme
/// language has a WIDTH; the only radius anywhere near one is the REACH of
/// its light ([`glow_reach_edit`]), which is a different token in a
/// different section and answers a different question. The owner asked for
/// "ustawienie promienia borderu" on 2026-08-18 and both readings were
/// delivered, because both were missing and both are useful.
///
/// THE READER IS NAMED AND IT IS HOT. `border.edge.width` is one of the
/// tokens the bake resolves into its fast table (`theme/mod.rs:1939`,
/// `border_width`), and the master hangs the panel's own ring off it:
/// `elev.panel.edge.width = @panel.border -> @border.edge.width ->
/// @stroke.hair` (default.theme:1767). So this is the one number that
/// moves every container's ring at once — which is exactly the size of
/// question BASIC asks.
///
/// AND IT IS NOT `stroke.hair`. The kerf is the GLOBAL one, worn by 72
/// derivations including `menu.border` and `tooltip.border`; the editor
/// already offers it under HAIRLINE, on the page for one-token questions.
/// Writing the border's own key instead is what lets a person thicken the
/// frames without thickening every rule and separator on the screen.
///
/// THE WALL IS THE MASTER'S, not a taste: `[stroke]` tops out at `bold =
/// 0.7u` (default.theme:251), so 1u is past every weight the file states —
/// the same wall, for the same reason, that [`shape_edits`] puts on the
/// kerf.
pub fn border_width_edit(scope: Scope, width_u: f32) -> Edit {
    let Scope::Theme = scope;
    Edit::new("border.edge.width", format!("{:.2}u", width_u.clamp(0.0, 1.0)))
}

/// HOW FAR A LIT BORDER'S LIGHT REACHES — `glow.panel_edge.radius`.
///
/// The second reading of "promień borderu", and the only token in this
/// build that a border and a radius both have a claim on. It is read by
/// `object/window.rs:104`, which returns at zero, and it is the number
/// [`border_edits`] SEEDS at `1.6u` on a theme that has never dressed its
/// halo. That seeding is a floor under a switch, not an answer to "how
/// wide" — until now nothing could answer that, which is why a person who
/// picked GLOW got one reach and no say in it.
///
/// THE RANGE IS DECLARED IN THE FILE, not chosen here: `panel_edge.radius`
/// says `u, 0u .. 8.76u` (its own doc carries the derivation — the
/// editor's 4mm calibration of 2026-08-25, raised the same day by the
/// owner's "o 300% po jednej i drugiej stronie" from the 2.19u it first
/// landed on), and 0u is the master's own `none` sentinel — draw
/// nothing. A caller that hands this zero is asking for an unlit border
/// by way of the reach, which is a legal thing to ask and reads on
/// screen exactly like NONE.
///
/// WHO WINS. This is written by the editor AFTER [`border_edits`], over
/// the top of the seed, because a number a person moved outranks a floor
/// the model put under a switch. The caller does that merge — one
/// assignment per token, or a file would carry the key twice.
pub fn glow_reach_edit(scope: Scope, radius_u: f32) -> Edit {
    let Scope::Theme = scope;
    Edit::new("glow.panel_edge.radius", format!("{:.2}u", radius_u.clamp(0.0, 8.76)))
}

/// How far a background answer's COLOUR travels along the `[elev.*]`
/// ladder — the owner's ZGŁOSZENIE 7, 2026-08-18.
///
/// "W trybie BASIC zmiana przezroczystości wpływa TYLKO na główne tło
/// obiektu." The rule is about a PAGE, which is why it is a parameter and
/// not a change to what a background edit means: ADVANCED is the page for
/// "what exactly should this one token do" and its answer still dresses
/// every reachable rung, which is what keeps a menu from being the one
/// flat plate over a frosted window.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum GlassReach {
    /// Every reachable rung wears the same glass: rank, tint and wash.
    EveryRung,
    /// The RANK travels and the COLOURS do not. Every rung still learns
    /// that the theme is glassy — so a float over a frosted window is
    /// frosted too — but the tint and the wash, which is where the alpha
    /// a person moved lives, land on the body's own rung alone. The other
    /// rungs keep whatever the theme's file says (the master ships
    /// `#FFFFFF / 1.0` and `none`, default.theme:1888-1889).
    BodyOnly,
}

/// The glass trio's key names on every rung of the `[elev.*]` ladder a
/// background edit reaches: `(rank, tint, wash)`.
///
/// THE BODY'S OWN RUNG IS FIRST and that order is load-bearing:
/// [`GlassReach::BodyOnly`] means "the colours stop after index 0".
///
/// A rung is reachable exactly when some object is BUILT from it, because
/// `elev::Level` reads the trio off whichever rung it was named with and
/// nothing else reads it at all. Two qualify (measured 2026-08-17):
///
/// * `elev.panel` — every panel (`object/panel.rs`) and every window frame
///   (`object/window.rs`, which joined the ladder the same day). This is
///   the rung the editor has written since 2026-08-16.
/// * `elev.popover` — the menu and the tooltip, which joined the ladder on
///   2026-08-17 and until then drew their bodies from a private copy of
///   the rules. Adding it here is what stops the owner's FROSTED theme
///   from being a window of frosted glass with a flat opaque menu standing
///   on it.
///
/// The other seven are deliberately absent, each for its own reason.
/// `raised`, `focused`, `inset` and `overlay` have no object built from
/// them, so a value written there would sit in the user's file changing
/// nothing — the one thing this module's header forbids. `backdrop` and
/// `board` are the screen behind everything, not a surface on it.
/// `fixture` is excluded on the owner's own boundary: the desktop's
/// decoration is not what "background" means in this editor, and
/// `deco.rs` reads it by a path of its own.
const GLASS_RUNGS: [(&str, &str, &str); 2] = [
    ("elev.panel.glass.rank", "elev.panel.glass.tint", "elev.panel.glass.wash"),
    ("elev.popover.glass.rank", "elev.popover.glass.tint", "elev.popover.glass.wash"),
];

/// Tokens that carry the background, for one scope: WINDOWS AND WIDGETS,
/// never the desktop's decoration.
///
/// The seam is exact and it was found by reading, not chosen for comfort:
/// `component.panel.fill` is read directly by `window::frame` and inherited
/// by the panel rung through the master's derivation (`[elev.panel] fill =
/// @component.panel.fill`), so ONE token colours both. Writing
/// `elev.panel.fill` instead would sever that derivation for good — the
/// windows would stop following. The glass trio lives on the rungs
/// themselves ([`GLASS_RUNGS`], underived, safe to write). The fixture's
/// own glass (`elev.fixture.*`, `deco.rs`) is not touched from here, which
/// is what keeps the board's decoration out of the editor's reach by
/// construction.
///
/// A GLASSY KIND IS WRITTEN TO EVERY REACHABLE RUNG, a solid one is
/// UNWRITTEN from every one of them. Both halves matter: without the
/// first, a menu opened over a frosted window is the one flat plate on
/// the screen; without the second, going back to SOLID leaves the menu
/// frosted for good, because the rank that turned it on is still in the
/// file. Since 2026-08-18 the KIND is what travels unconditionally and
/// the COLOURS answer to `reach` ([`GlassReach`]) — both halves above
/// survive that, because both are statements about the RANK.
/// SOLID's own COLOUR stays on the single shared seam and does not
/// travel — a menu and a tooltip have bodies of their own
/// (`component.menu.fill`, `component.tooltip.fill`) and the editor has
/// no control that claims to set them, whereas glass has no colour of its
/// own to disagree about: it is whatever is behind it.
///
/// `opacity`, `depth` and `coverage` are the kind's own knobs, 0..1 and
/// 1..=3: opacity scales the whole effect (a translucent SOLID lets the
/// scene through sharp; a translucent tint blends the blur with the sharp
/// base beneath it), depth picks the pyramid rank, and coverage is the
/// wash's alpha — a slider now, where an opening literal stood before
/// verification called it out.
///
/// `reach` is ZGŁOSZENIE 7's answer and it is about the PAGE that asked,
/// never about the kind: see [`GlassReach`].
pub fn glass_edits(
    scope: Scope,
    kind: Glass,
    tint: Oklch,
    wash: Oklch,
    opacity: f32,
    depth: f32,
    coverage: f32,
    reach: GlassReach,
) -> Vec<Edit> {
    let Scope::Theme = scope;
    let op = opacity.clamp(0.0, 1.0);
    // Fractional on purpose: the emitter mixes two pyramid rungs by the
    // fraction, so 1.7 is a real depth and not a rounding of 2.
    let rank = format!("{:.2}", depth.clamp(1.0, 3.0));
    let tint_lit = oklch_literal(Oklch { alpha: op, ..tint });
    // The word, not a colour with nothing in it. BLUR is the tint alone,
    // and the master's own way of saying so at this key is `none` — the
    // same word it ships on all nine `[elev.*]` rungs.
    //
    // This used to write `oklch(0, 0, 0 / 0)` because `none` came back
    // OPAQUE BLACK and painted the panels out. The cause was in `bake.rs`,
    // not in the overlay (the master's own `none` measured the same
    // black), and it is fixed: a sentinel now empties the colour slot it
    // was leaving seeded. Held down by `tests/sentinel_none_colour.rs`,
    // which asserts the word and the transparent literal are the same
    // answer.
    let wash_lit = match kind {
        Glass::Frosted => oklch_literal(Oklch { alpha: coverage.clamp(0.0, 1.0), ..wash }),
        _ => "none".to_string(),
    };
    let mut out = Vec::new();
    if let Glass::Solid = kind {
        out.push(Edit::new("component.panel.fill", oklch_literal(Oklch { alpha: op, ..wash })));
    }
    for (i, (rank_key, tint_key, wash_key)) in GLASS_RUNGS.into_iter().enumerate() {
        // The BODY's rung is index 0 (see [`GLASS_RUNGS`]). Under
        // `BodyOnly` the rank still travels — every float learns that this
        // theme is glassy — and the two COLOUR keys, which is where the
        // alpha a person moved lives, stop here.
        let colours = matches!(reach, GlassReach::EveryRung) || i == 0;
        match kind {
            Glass::Solid => out.push(Edit::new(rank_key, "0")),
            Glass::Blur | Glass::Frosted => {
                out.push(Edit::new(rank_key, rank.clone()));
                if colours {
                    out.push(Edit::new(tint_key, tint_lit.clone()));
                    out.push(Edit::new(wash_key, wash_lit.clone()));
                }
            }
        }
    }
    out
}

/// The main background, written LITERALLY — BASIC's one picker taken at
/// its word rather than solved for through `glass_edits`'s wash-and-opacity
/// pair. `colour`'s alpha travels unclamped: on BASIC the picker's alpha
/// channel IS the transparency knob (`ZGŁOSZENIE`, 2026-08-19 — the OPACITY
/// slider is gone from that page, and this is what replaced it), so what a
/// theme's file spells `oklch(L, C, H / a)` a person reads as the last
/// bytes of the picker's own RGBA hex.
///
/// The SAME token [`glass_edits`] writes for `Glass::Solid` — this is a
/// second author of `component.panel.fill`, not a second token, and the
/// caller decides who wins by ordering (`Settings::editor_edits` calls this
/// AFTER `glass_edits` and folds it in with `set_edit`, so BASIC's literal
/// pick outranks the wash-derived seed under it). See [`glass_edits`]'s own
/// docs for the token's readers (`window::frame`, inherited by
/// `elev.panel.fill`).
pub fn panel_fill_edit(scope: Scope, colour: Oklch) -> Edit {
    let Scope::Theme = scope;
    Edit::new("component.panel.fill", oklch_literal(colour))
}

// ------------------------------------------------- the whole-theme sets
//
// Everything below landed 2026-08-16, when the owner's wish grew from "the
// border and the panels' background" to the whole theme. The contract is
// unchanged: pure functions, no engine, and not one token without a reader.
// Every anchor was re-checked by grep on 2026-08-16 rather than taken from
// the reconnaissance that proposed the groups — which was right seven times
// and wrong once (see the severity note at `severity_role_edit`).

/// The one colour that reskins the interface: `palette.accent`.
///
/// ONE token, because the master derives everything else from it: `[accent]`
/// (default.theme:423-445) rebuilds primary/hover/active/dim/border/glow/
/// on/focus, `[border]` (371-400) the five frame colours, `[chroma]`/`[hue]`
/// (502-509) the sat()/hue() split that the surfaces and the text ride, and
/// 22 of the 25 `[class]` ladders stand on `@accent.primary` (3506-3531).
/// The readers are real and they are many: the seed itself at
/// `theme/bake.rs:859`, the hue/chroma split at `term.rs:125` and `:133`,
/// every class ladder entering Rust through `view/surface.rs:527`
/// (`class_state` — serving button.rs:111, menu.rs:505, text_input.rs:964,
/// tabs.rs:345, segmented.rs:140, checkbox.rs:93, winframe.rs:448,
/// list.rs:352, paint.rs:685, ui.rs:1471 and :1574), the focus ring at
/// `focus_ring.rs:87` (`focus.ring.color = @accent.focus`), the shared
/// panel border at `window.rs:157` (`component.panel.border =
/// @border.default`), the addon ABI at `plugin.rs:485`, the terminal cursor
/// at `plugin.rs:651` and the solid badge at `view/paint.rs:541`.
///
/// OPAQUE BY FORCE: the derivations own every alpha (`border.default =
/// alpha(@accent.primary, 0.78)` and its kin), and the three sliders this
/// edit serves have no alpha knob. A translucent seed would fade exactly
/// the places that use the seed raw — titles, cursors, class bases — and
/// none of the places that alpha() it anyway: half the UI faded, with no
/// knob saying so.
pub fn accent_edit(scope: Scope, colour: Oklch) -> Edit {
    let Scope::Theme = scope;
    Edit::new("palette.accent", oklch_literal(Oklch { alpha: 1.0, ..colour }))
}

/// Where the surface ladder takes its hue from.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum SurfaceHue {
    /// `surface.hue = @hue.accent` — the master's own wiring, restored as a
    /// REFERENCE so a later accent drag keeps moving the surfaces with it.
    FollowAccent,
    /// A number in degrees, cutting the surfaces loose from the accent for
    /// good. The master declares the override legal: the token "settles to
    /// a number and a theme may override it with plain degrees"
    /// (default.theme:317).
    Own(f32),
}

/// The three meta-knobs over the six-level surface ladder.
///
/// NOT eighteen sliders: the levels' L rungs (0.115..0.330) are fixed by
/// §5.5's ladder and the chroma coefficients are written into the six level
/// expressions (default.theme:316-346). What a theme's hand may move is the
/// hue they all sit on (`surface.hue`, read by all six expressions), a lift
/// over every L and a scale over every C — the two scalars the BAKE applies
/// to the whole ladder at `theme/bake.rs:519-520`, because the language has
/// no arithmetic and "the same ladder, lifted" cannot be an expression
/// (bake.rs:485-488). The rungs those knobs move are read for real:
/// `surface.void` is the swapchain clear colour (`deco.rs:33`) and the
/// terminal bed (`term.rs:91`, `term.bg = @surface.void`), `surface.panel`
/// the window body (`winframe.rs:414`) and the shared panel fill
/// (`window.rs:156`), `surface.raised` the menu and tooltip beds
/// (`menu.rs:445`, `tooltip.rs:266`), `surface.sunken` the bar track
/// (`view/paint.rs:482`), `surface.inset` the badge fill
/// (`view/paint.rs:546`), `surface.scrim` the modal dimmer
/// (`window.rs:126`) and `surface.base` crosses the addon ABI
/// (`plugin.rs:489`).
///
/// Numbers, not colours — the none-bakes-to-black trap does not apply.
pub fn surface_edits(scope: Scope, hue: SurfaceHue, lift: f32, chroma: f32) -> Vec<Edit> {
    let Scope::Theme = scope;
    vec![
        Edit::new(
            "surface.hue",
            match hue {
                SurfaceHue::FollowAccent => "@hue.accent".to_string(),
                SurfaceHue::Own(deg) => format!("{:.2}", deg.rem_euclid(360.0)),
            },
        ),
        // The clamps are the bake's own (bake.rs:519-520). Writing a wilder
        // number would save a file that resolves to the clamp anyway, and
        // reopens with a slider past its own wall.
        Edit::new("surface.lift", format!("{:.4}", lift.clamp(-0.09, 0.09))),
        Edit::new("surface.chroma", format!("{:.3}", chroma.clamp(0.0, 4.0))),
    ]
}

/// The two meta-knobs over the seven text roles.
///
/// No HSV per role, deliberately: the roles' L ladder is FIXED
/// (0.870/0.905/0.755/0.590/0.435/0.372, default.theme:348-370), their C is
/// the accent's chroma times per-role coefficients and their hue IS the
/// accent's — per-role colour sliders would fight the cascade and pass D's
/// contrast floors both. Changing the accent re-derives all text by itself;
/// what the master leaves to a theme's hand is `text.lift` and
/// `text.chroma`, the pair the bake applies to the whole ladder at
/// `theme/bake.rs:525-526`. The roles reach the screen through ONE reader —
/// `view/paint.rs:157` resolves `type.<role>.fg` for every piece of text
/// the toolkit draws — plus the terminal's default ink (`term.rs:42`,
/// `term.fg = @text.primary`), panel titles (`panel.rs:305`), menu hints
/// (`menu.rs:458`), tooltip text (`tooltip.rs:273`), toasts
/// (`toaster.rs:234-235`) and badge text (`view/paint.rs:548`).
pub fn text_edits(scope: Scope, hue: SurfaceHue, lift: f32, chroma: f32) -> Vec<Edit> {
    let Scope::Theme = scope;
    vec![
        // `text.hue` is the text ladder's hue seed, symmetric with
        // `surface.hue`: FollowAccent restores the master's own reference
        // so a later accent drag keeps moving the text with it; Own cuts
        // the text loose so the FONT picker can lead a colour of its own.
        // (`SurfaceHue` is the shared "follow the accent or an own degree"
        // seed; the name is the surface's only by history.)
        Edit::new(
            "text.hue",
            match hue {
                SurfaceHue::FollowAccent => "@hue.accent".to_string(),
                SurfaceHue::Own(deg) => format!("{:.2}", deg.rem_euclid(360.0)),
            },
        ),
        Edit::new("text.lift", format!("{:.4}", lift.clamp(-0.10, 0.10))),
        Edit::new("text.chroma", format!("{:.3}", chroma.clamp(0.0, 3.0))),
    ]
}

/// §5.10's closed set of severity roles, in declaration order (`ui.rs:86`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SeverityRole {
    Ok,
    Info,
    Warning,
    Critical,
    Contained,
    Offline,
    Unknown,
}

/// Pins one severity role: `.text` is the role's AUTHOR colour and the only
/// token a hand needs. The master derives `.glyph`, `.edge`, `.fill` and
/// `.on` from it (default.theme:574-583 and each role's kin), and author
/// and derivations alike are what the renderer reads: `view/paint.rs:42`
/// (.text), `:47` (.edge), `:52` (.fill), `:57` (.on), `:532`
/// (.badge_style), the status pills at `ui.rs:147`, the toast title at
/// `toaster.rs:234` — and `accent.warm` rides `severity.warning.text`
/// (default.theme:442).
///
/// WHAT THIS GROUP DOES NOT GET, AND WHY. The reconnaissance proposed a
/// MODE list and PULL / PULL_CLAMP / CHROMA sliders over `severity.mode`,
/// `severity.pull`, `severity.pull_clamp` and `severity.chroma`. Grep says
/// no: the four are declared (default.theme:546-558) and the "engine" their
/// comments describe does not exist in Rust — the only match outside the
/// theme file is a parser test (`parse.rs:1710`). Four controls over four
/// dead tokens is the exact thing this module exists to refuse, so the
/// group is the per-role pin instead, and the tests below keep the four on
/// the dead list until someone writes their reader.
///
/// Opaque by force, like the accent seed: the derived members set their own
/// alphas (`.edge` at 0.60, `.fill` at 0.88) and `.text` itself is drawn
/// raw as status ink — a translucent author would fade the label and
/// nothing else.
pub fn severity_role_edit(scope: Scope, role: SeverityRole, colour: Oklch) -> Edit {
    let Scope::Theme = scope;
    let token = match role {
        SeverityRole::Ok => "severity.ok.text",
        SeverityRole::Info => "severity.info.text",
        SeverityRole::Warning => "severity.warning.text",
        SeverityRole::Critical => "severity.critical.text",
        SeverityRole::Contained => "severity.contained.text",
        SeverityRole::Offline => "severity.offline.text",
        SeverityRole::Unknown => "severity.unknown.text",
    };
    Edit::new(token, oklch_literal(Oklch { alpha: 1.0, ..colour }))
}

/// The one cut the whole interface wears.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CornerCut {
    Square,
    Round,
    Chamfer,
}

/// The corner language and the hairline, one set.
///
/// `corner.mode` is "the one place a theme states its corner language"
/// (default.theme:287) and twelve preset siblings DERIVE from it; the
/// derived words are read at `window.rs:158` (panel), `menu.rs:438`,
/// `tooltip.rs:259`, `winframe.rs:450` and `view/paint.rs:690`
/// (scrollbar). The three radii feed the presets' `*.corner` keys (41
/// `@corner.*` references), read at `window.rs:165`, `button.rs:77`,
/// `text_input.rs:950`, `tabs.rs:161`, `checkbox.rs:34`,
/// `segmented.rs:147`, `menu.rs:442`, `tooltip.rs:263`, `winframe.rs:98`
/// and `:492`, and `view/paint.rs:691`. `corner.segments` is read raw
/// (`window.rs:74`, `focus_ring.rs:134`, `winframe.rs:452`,
/// `plugin.rs:376`) and `stroke.hair` both raw (`view/paint.rs:595` and
/// `:656`) and through 72 `@stroke.hair` derivations — `menu.border` and
/// `tooltip.border` among them, which is why those two sets get their own
/// width knobs and this one stays the global kerf.
///
/// `corner.pill` and `stroke.regular` are alive and deliberately NOT here:
/// pill is a sentinel word, not a length a slider can mean, and regular's
/// raw consumer chain (`winframe.border` = `@stroke.regular`,
/// default.theme:4314, read at `winframe.rs:95`) is window chrome the owner
/// has separate plans for (the CSD decision). Not writing an alive token
/// costs nothing; the door stays open.
pub fn shape_edits(
    scope: Scope,
    cut: CornerCut,
    sm_u: f32,
    md_u: f32,
    lg_u: f32,
    segments: u8,
    hair_u: f32,
) -> Vec<Edit> {
    let Scope::Theme = scope;
    let len = |v: f32, hi: f32| format!("{:.2}u", v.clamp(0.0, hi));
    vec![
        Edit::new(
            "corner.mode",
            match cut {
                CornerCut::Square => "square",
                CornerCut::Round => "round",
                CornerCut::Chamfer => "chamfer",
            },
        ),
        // 4u is past every radius the master states (lg = 2.2u); a wall,
        // not a style opinion.
        Edit::new("corner.sm", len(sm_u, 4.0)),
        Edit::new("corner.md", len(md_u, 4.0)),
        Edit::new("corner.lg", len(lg_u, 4.0)),
        // The declared range (default.theme:284: n, 3 .. 16), and an
        // integer — a fraction of a tessellation quad does not exist.
        Edit::new("corner.segments", format!("{}", segments.clamp(3, 16))),
        // The master's heaviest stroke is bold = 0.7u; 1u is the wall.
        Edit::new("stroke.hair", len(hair_u, 1.0)),
    ]
}

/// How the focus ring is stroked.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RingStyle {
    Solid,
    Dashed,
}

/// Everything the focus-ring page's knobs say, named instead of positional
/// — nine values in a row is where callers start swapping dash for gap.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct FocusRing {
    pub style: RingStyle,
    /// `focus.ring.width`, u. 0 is a legal knob position and an invisible
    /// ring (`focus_ring.rs:63-65` returns at 0) — the slider saying "as
    /// thin as it goes", not a defect here.
    pub width_u: f32,
    /// `focus.ring.offset`, u — declared 0u .. 2u (default.theme:3634).
    pub offset_u: f32,
    pub colour: Oklch,
    /// The dashed rhythm, u. Written only for [`RingStyle::Dashed`].
    pub dash_u: f32,
    pub gap_u: f32,
    /// The halo pair, `glow.focus_ring.*`.
    pub halo: bool,
    pub halo_alpha: f32,
    /// Whether the LIVE theme already resolves a visible halo radius — read
    /// off the theme by the caller, the same contract as `border_edits`'
    /// `halo_dressed`.
    pub halo_dressed: bool,
}

/// Readers, all in `object/focus_ring.rs`: `enabled` :60, `width` :63,
/// `offset` :67, `style` :70, `dash`/`gap` :76, `color` :87, and the halo
/// at :213 (`enabled`), :216 (`radius`), :217 (`alpha`).
///
/// The OFF branch is LINE's lesson verbatim: `enabled = false` is the whole
/// of "off" (`focus_ring.rs:60-62` returns before anything else is read),
/// so the theme's own dress — width, rhythm, colour, halo — stands, and
/// switching back on finds it as it was. The halo dresses like NEON: the
/// master ships `glow.focus_ring.radius = 0u` and `alpha = 0.0`
/// (default.theme:1030 and :1039) and the renderer returns at zero
/// (focus_ring.rs:219), so a bare switch is invisible on default — the
/// alpha is the knob's own, and the radius is seeded ONLY on a theme that
/// has not dressed its halo. `glow.focus_ring.color` has no reader (the
/// halo wears the ring's colour) and is not written; the ring's corner pair
/// derives from `@field.corner` and belongs to the shape group.
pub fn focus_ring_edits(scope: Scope, enabled: bool, ring: &FocusRing) -> Vec<Edit> {
    let Scope::Theme = scope;
    let mut out = vec![Edit::new("focus.ring.enabled", if enabled { "true" } else { "false" })];
    if !enabled {
        return out;
    }
    out.push(Edit::new(
        "focus.ring.style",
        match ring.style {
            RingStyle::Solid => "solid",
            RingStyle::Dashed => "dashed",
        },
    ));
    out.push(Edit::new("focus.ring.width", format!("{:.2}u", ring.width_u.clamp(0.0, 2.0))));
    out.push(Edit::new("focus.ring.offset", format!("{:.2}u", ring.offset_u.clamp(0.0, 2.0))));
    out.push(Edit::new("focus.ring.color", oklch_literal(ring.colour)));
    if ring.style == RingStyle::Dashed {
        // SOLID leaves the rhythm standing for the reason LINE leaves the
        // halo's dress: a trip through solid must not flatten it.
        out.push(Edit::new("focus.ring.dash", format!("{:.2}u", ring.dash_u.max(0.0))));
        out.push(Edit::new("focus.ring.gap", format!("{:.2}u", ring.gap_u.max(0.0))));
    }
    out.push(Edit::new(
        "glow.focus_ring.enabled",
        if ring.halo { "true" } else { "false" },
    ));
    if ring.halo {
        out.push(Edit::new(
            "glow.focus_ring.alpha",
            format!("{:.3}", ring.halo_alpha.clamp(0.0, 1.0)),
        ));
        if !ring.halo_dressed {
            // The same seed the border's NEON wears; one number, one place
            // to change it when the owner decides the halos should differ.
            // And the same silence, safe for the same reason: a save patches
            // the theme's own file, under any name it lands under, so a ring
            // halo the theme dressed itself is a line no save touches (see
            // NEON above).
            out.push(Edit::new("glow.focus_ring.radius", "1.6u"));
        }
    }
    out
}

/// Split out of the ring set the way `border_colour_edit` is split out of
/// `border_edits`: the dim is read on the WINDOW (`winframe.rs:467`), not
/// on the ring, and it must keep working with the ring switched off —
/// inside `focus_ring_edits` the OFF branch would swallow it. Declared
/// 0.3 .. 1.0 (default.theme:3631), and the floor is real: "dimming an
/// unfocused window must not hide it".
pub fn unfocused_dim_edit(scope: Scope, dim: f32) -> Edit {
    let Scope::Theme = scope;
    Edit::new("focus.unfocused_dim", format!("{:.3}", dim.clamp(0.3, 1.0)))
}

/// The context menu's chrome — and the WINDOW menu's, which is the same
/// object on the same tokens (`winframe.rs:683`, `:688`, `:689` read
/// `component.menu.fill` / `menu.border` / `component.menu.border` for the
/// menu the frame opens), so one set serves both.
///
/// Readers: fill `menu.rs:445` (a bed), ring width `menu.rs:446`, ring
/// colour `menu.rs:453` (and `:482`, where the separator rule wears it),
/// hint ink `menu.rs:458`. The corner pair is NOT here — `menu.corner` and
/// `menu.corner_mode` derive from the winframe's, and the winframe's from
/// `[corner]` (default.theme:4350-4351), so the shape group already moves
/// them and a second author here would sever that.
///
/// The colours pass through as given: a bed may be translucent by a theme's
/// design, and unlike the glass set there is no opacity knob here to own
/// the channel — the desktop's three sliders hand an opaque colour, and a
/// seeded colour keeps its own alpha.
pub fn menu_edits(scope: Scope, fill: Oklch, border: Oklch, border_w_u: f32, hint: Oklch) -> Vec<Edit> {
    let Scope::Theme = scope;
    vec![
        Edit::new("component.menu.fill", oklch_literal(fill)),
        Edit::new("component.menu.border", oklch_literal(border)),
        // Floored, not clamped: 0 means "no ring" (menu.rs:446-448 skips
        // the draw), which is a look a theme may mean.
        Edit::new("menu.border", format!("{:.2}u", border_w_u.max(0.0))),
        Edit::new("component.menu.hint", oklch_literal(hint)),
    ]
}

/// The tooltip's chrome, the menu's sibling float.
///
/// Readers: fill `tooltip.rs:266` (a bed), ring width `tooltip.rs:267`,
/// ring colour `tooltip.rs:269`, text ink `tooltip.rs:273`. The corner pair
/// stays with the shape group for the same reason the menu's does
/// (`tooltip.corner = @corner.sm`, `tooltip.corner_mode =
/// @menu.corner_mode`, default.theme:4801-4802).
pub fn tooltip_edits(scope: Scope, fill: Oklch, edge: Oklch, border_w_u: f32, text: Oklch) -> Vec<Edit> {
    let Scope::Theme = scope;
    vec![
        Edit::new("component.tooltip.fill", oklch_literal(fill)),
        Edit::new("component.tooltip.edge", oklch_literal(edge)),
        Edit::new("tooltip.border", format!("{:.2}u", border_w_u.max(0.0))),
        Edit::new("component.tooltip.text", oklch_literal(text)),
    ]
}

/// Whether the bar takes layout space.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrollbarMode {
    Overlay,
    Inset,
    None,
}

/// Which side of the content the bar sits on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ScrollbarEdge {
    Right,
    Left,
}

/// The scrollbar's geometry and behaviour.
///
/// Readers, in `view/scroll.rs` unless said otherwise: `mode` :559 (and by
/// word at :595, the ABI's path), `edge` :572, `w` :576, `w_hover` :577,
/// `auto_hide` :585, `fade_ms` :586; the track switch and its colour at
/// `view/paint.rs:673` and `:674`.
///
/// What stays out, and why. `scrollbar.margin` and `thumb_min` are alive
/// (scroll.rs:578-579) but no knob was asked for — not writing an alive
/// token costs nothing. The THUMB's colour is a class ladder
/// (`scrollbar.thumb`, per-rung through `class_state`, view/paint.rs:685);
/// this model writes single tokens, not ladder rungs, so the thumb keeps
/// moving with the accent instead. The corner pair derives from `[corner]`
/// (default.theme:4833-4834) and belongs to the shape group.
pub fn scrollbar_edits(
    scope: Scope,
    mode: ScrollbarMode,
    edge: ScrollbarEdge,
    w_u: f32,
    w_hover_u: f32,
    auto_hide: bool,
    fade_ms: f32,
    track: Option<Oklch>,
) -> Vec<Edit> {
    let Scope::Theme = scope;
    let mut out = vec![
        Edit::new(
            "scrollbar.mode",
            match mode {
                ScrollbarMode::Overlay => "overlay",
                ScrollbarMode::Inset => "inset",
                ScrollbarMode::None => "none",
            },
        ),
        Edit::new(
            "scrollbar.edge",
            match edge {
                ScrollbarEdge::Right => "right",
                ScrollbarEdge::Left => "left",
            },
        ),
        // Below half a unit the bar cannot be aimed at; 4u the master's
        // own w_hover doubled — walls, not opinions.
        Edit::new("scrollbar.w", format!("{:.2}u", w_u.clamp(0.5, 4.0))),
        Edit::new("scrollbar.w_hover", format!("{:.2}u", w_hover_u.clamp(0.5, 4.0))),
        Edit::new("scrollbar.auto_hide", if auto_hide { "true" } else { "false" }),
    ];
    if auto_hide {
        // The declaration itself says the fade is "read only when
        // auto_hide = true" (default.theme:4837), so the OFF trip leaves
        // the theme's own duration standing, dash-and-gap style.
        out.push(Edit::new(
            "scrollbar.fade_ms",
            format!("{:.0}ms", fade_ms.clamp(0.0, 2000.0)),
        ));
    }
    match track {
        Some(colour) => {
            out.push(Edit::new("scrollbar.track", "on"));
            out.push(Edit::new("component.scrollbar.track", oklch_literal(colour)));
        }
        // OFF is the switch alone: paint.rs:673 never reads the colour
        // then, and the theme's own groove colour survives the trip.
        None => out.push(Edit::new("scrollbar.track", "off")),
    }
    out
}

// ------------------------------------------------------------- BASIC mode
//
// THE WHOLE THEME ON THREE SLIDERS. Everything above this line is the
// ADVANCED page: nine groups, one control per thing. BASIC is the same
// theme asked three questions — HUE, SATURATION, LIGHTNESS — and the only
// way three sliders can move a hundred colours is to move the ones the
// others are DERIVED from. So this group writes AUTHORS, and lets the
// cascade do what it already does (5.0b).
//
// WHO AUTHORS WHAT, measured in default.theme on 2026-08-17:
//
//   palette.accent   the ONE author of the interface's hue and chroma.
//                    hue.accent = hue(it), chroma.accent = sat(it); the six
//                    surface rungs are oklch(<fixed L>, k*@chroma.accent,
//                    @surface.hue) and surface.hue = @hue.accent; the seven
//                    text roles are oklch(<fixed L>, k*@chroma.accent,
//                    @hue.accent); [accent], [border], [data] (via
//                    palette.data = @palette.accent), every [class] base
//                    and the whole of [component] hang off those.
//   severity.<r>.text  seven authors, one per MEANING. Each derives its own
//                    .glyph/.edge/.fill/.on, and accent.warm rides
//                    severity.warning.text.
//   surface.lift / text.lift   the two authors of LIGHTNESS that the
//                    colours above cannot reach: the rungs' L is written
//                    into each level expression and the language has no
//                    arithmetic, so "the same ladder, lifted" is a scalar
//                    the bake applies (bake.rs:519, :525).
//   palette.black / palette.white   the shade()/tint() targets. NOT touched:
//                    they are the ends of the axis every other colour is
//                    measured against, and rotating them rotates the ruler.
//   palette.neutral  the grey anchor, and its ONLY use is
//                    severity.offline.text = ensure(@palette.neutral, ...),
//                    which this group pins directly. Not written.
//
// EVERYTHING ELSE IS DERIVED and is deliberately left alone: writing a
// derived token would pin it, and the next slider move would find it deaf.
//
// WHY ONE HUE FOR THE INTERFACE AND A ROTATION FOR SEVERITY IS THE SAME
// MECHANISM (the owner's 2026-08-17 clarification, which reads like two
// rules and is one). Every author is rotated by the SAME number of degrees.
// The chrome family has exactly ONE author, so rotating it lands surfaces,
// containers, controls and text on a single shared hue — that is 5.0b's
// cascade, not an extra rule. The severity family has SEVEN authors, so the
// same rotation carries all seven and the gaps between them survive: green
// `ok` and red `critical` stay as far apart as their author wrote them.
// What tells the families apart afterwards is SHADE, and the shades are the
// master's own ladders — the six surface rungs, the seven text L's, the
// [state] rungs — none of which this group touches.

/// Every severity role, in declaration order (`ui.rs:86`).
pub const SEVERITY_ROLES: [SeverityRole; 7] = [
    SeverityRole::Ok,
    SeverityRole::Info,
    SeverityRole::Warning,
    SeverityRole::Critical,
    SeverityRole::Contained,
    SeverityRole::Offline,
    SeverityRole::Unknown,
];

/// The three BASIC knobs, as RELATIVE moves over whatever the theme says.
///
/// Relative, not absolute, by the owner's decision: an absolute hue would
/// flatten a theme's `ok`/`critical` pair into one colour, an absolute
/// chroma would flatten the accent onto the surfaces, and an absolute
/// lightness would throw away the ladder that makes a theme legible. So a
/// rotation, a multiplier and an offset — each one leaves every difference
/// the author wrote exactly where it was.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Tone {
    /// Degrees added to every author's hue. Wraps; no range.
    pub hue_deg: f32,
    /// Multiplier over every author's chroma. 1.0 is the theme unchanged.
    pub sat: f32,
    /// Offset added to every author's OKLab L, and to the two ladder lifts.
    pub light: f32,
}

impl Tone {
    /// The theme as it stands. What the three sliders read when BASIC opens,
    /// and what makes "open the editor and change nothing" a no-op.
    pub const NEUTRAL: Tone = Tone { hue_deg: 0.0, sat: 1.0, light: 0.0 };

    pub fn is_neutral(&self) -> bool {
        *self == Tone::NEUTRAL
    }

    /// The same move, snapped to what the pipeline can actually show.
    pub fn snapped(self, step: &ToneStep) -> Tone {
        Tone {
            hue_deg: snap(self.hue_deg, step.hue_deg),
            sat: snap(self.sat, step.sat),
            light: snap(self.light, step.light),
        }
    }

    /// ONE AUTHOR, MOVED — the same arithmetic [`tone_edits`] carries its
    /// ten by, offered to a caller holding an eleventh.
    ///
    /// WHO HOLDS AN ELEVENTH, AND WHY THE MODEL DOES NOT. [`tone_edits`]
    /// writes ten tokens and no more, and every one of them is an AUTHOR
    /// the master derives a family from. A host page may nonetheless have
    /// pinned a bed to an absolute colour of its own — nacelle-desktop's
    /// BACKGROUND section writes `component.panel.fill` for SOLID and the
    /// glass tint/wash otherwise — and such a bed is no longer downstream
    /// of any author. It is the same case `tone_edits` answers for
    /// `surface.hue` by re-pointing it at `@hue.accent`, except that a
    /// literal cannot be re-pointed: it has to be carried. Left behind, it
    /// is the ONE surface in the window that does not turn with the HUE
    /// slider, which is exactly the promise that slider makes.
    ///
    /// The host carries it with THIS, not with three lines of its own, for
    /// the reason the shift is written once here: hue wraps, chroma cannot
    /// go negative and L is held in 0..1 — none of the three a gamut clamp,
    /// all three what the numbers MEAN.
    pub fn shift(self, c: Oklch) -> Oklch {
        tone_shift(c, self)
    }
}

fn snap(value: f32, step: f32) -> f32 {
    if step > 0.0 && step.is_finite() {
        (value / step).round() * step
    } else {
        value
    }
}

/// The authors as the LIVE theme resolves them: what a relative move is
/// relative TO.
///
/// The caller fills this from the running theme when BASIC opens — that is
/// the "know what the source is" half of the job, and it is why the model
/// takes a struct instead of reading the engine itself: the same three
/// sliders have to work over the master, over a user's file and over a
/// preview that has not been saved, and none of those is "the theme" from
/// inside this module.
///
/// WHAT LEFT THIS STRUCT ON 2026-08-18, and why it is the whole of the
/// owner's ZGŁOSZENIE 5. `severity: [Oklch; 7]` used to stand here, because
/// BASIC turned the seven roles by hand — the roles were frozen literals in
/// the master and rotating them was the only way to make them belong to a
/// re-coloured theme. Rotating them by the FULL move is what the owner saw:
/// mint -> red sent `ok` from 148 deg to 10.5 (a green success drawn in red)
/// and `critical` from 27 to 249.5 (a red alarm drawn in blue). The roles are
/// expressions now (`toward()` in the master), so they lean toward
/// `palette.accent` on their own, by `severity.pull` and no further than
/// `severity.pull_clamp` — and BASIC has nothing to say about them. This
/// struct holds only what BASIC still WRITES.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ToneSeeds {
    /// `palette.accent`.
    pub accent: Oklch,
    /// `palette.black` — `shade()`'s target.
    pub black: Oklch,
    /// `palette.white` — `tint()`'s target.
    pub white: Oklch,
    /// `palette.neutral` — the grey anchor `severity.offline.text` rides.
    pub neutral: Oklch,
    /// `surface.lift`.
    pub surface_lift: f32,
    /// `text.lift`.
    pub text_lift: f32,
}

impl ToneSeeds {
    /// The seeds AFTER a move — the fold that makes BASIC survive a trip
    /// through ADVANCED.
    ///
    /// Leaving BASIC and coming back re-reads the seeds off the live theme,
    /// so the three sliders return to [`Tone::NEUTRAL`] while the LOOK stays
    /// where it was put: the move has become part of what the sliders are
    /// now relative to. Nothing is lost in either direction — ADVANCED edits
    /// land on the same authors and BASIC picks them up on the way back, and
    /// an ADVANCED-only token (a focus ring's colour, a scrollbar's width)
    /// is never written here at all. This function is what a test can hold
    /// that promise to, and it applies the same clamps the writes do so the
    /// two cannot drift.
    pub fn shifted(&self, tone: Tone) -> ToneSeeds {
        ToneSeeds {
            accent: tone_shift(self.accent, tone),
            black: tone_turn(self.black, tone),
            white: tone_turn(self.white, tone),
            neutral: tone_spin(self.neutral, tone),
            surface_lift: clamp_surface_lift(self.surface_lift + tone.light),
            text_lift: clamp_text_lift(self.text_lift + tone.light),
        }
    }
}

/// One author, moved. Hue wraps, chroma cannot go negative, L is a real
/// 0..1 quantity and is held there — none of the three is a gamut clamp,
/// which the owner ruled out; they are what the numbers MEAN.
fn tone_shift(c: Oklch, tone: Tone) -> Oklch {
    Oklch {
        l: (c.l + tone.light).clamp(0.0, 1.0),
        c: (c.c * tone.sat).max(0.0),
        h: (c.h + tone.hue_deg).rem_euclid(360.0),
        alpha: c.alpha,
    }
}

/// The same move with THE LIGHTNESS LEFT ALONE — `palette.black` and
/// `palette.white`.
///
/// Measured over the master, and this is why the pair may not take
/// `tone.light`: `black` is `#0A100E` (L 0.166, C 0.010, h 172.6) and
/// `white` is `#EAF6F1` (L 0.963, C 0.014, h 169.2). Neither is neutral —
/// both sit on the ACCENT's hue at 7 % and 9 % of its chroma — so both are
/// already style, frozen in hex, and a re-coloured theme that leaves them
/// there is the owner's ZGŁOSZENIE 5 exactly: "the background stays the old
/// hue". The hue and the chroma must come along.
///
/// The LIGHTNESS must not. These two are the poles the whole file is pulled
/// between (`shade()` and `tint()` take no other target, and `shade()` alone
/// has ten readers in the master), so their L is the theme's POLARITY and
/// not a shade of the accent. Moving it with the accent's lightness would
/// mean that choosing a darker ink also raised the floor everything is
/// shaded toward — two questions answered by one slider, and the answer to
/// the second one wrong. Polarity is a theme file, not a knob
/// (`.gap-program/audyt-kolory-bazowe.md` §3).
fn tone_turn(c: Oklch, tone: Tone) -> Oklch {
    Oklch {
        l: c.l,
        c: (c.c * tone.sat).max(0.0),
        h: (c.h + tone.hue_deg).rem_euclid(360.0),
        alpha: c.alpha,
    }
}

/// The hue ALONE — `palette.neutral`.
///
/// The master calls this token a "hue-free grey anchor" and
/// `severity.offline.text` lives off it, whose whole meaning is "absent, not
/// zero": a thing that is not reporting must not look like a live reading.
/// So the hue may follow the theme — at the master's C 0.020 nobody can see
/// which way a grey leans — and the CHROMA may not, because chroma is the
/// difference between grey and a colour.
///
/// This is also a bug fix. `tone_shift` multiplied every seed's chroma,
/// `offline` rode this token, and SATURATION at 200 % therefore took the
/// anchor to C 0.040: a visibly green "not reporting"
/// (`.gap-program/audyt-kolory-bazowe.md` §5.4).
fn tone_spin(c: Oklch, tone: Tone) -> Oklch {
    Oklch { h: (c.h + tone.hue_deg).rem_euclid(360.0), ..c }
}

/// The bake's own wall ([`super::bake::SURFACE_LIFT_WALL`]), not a new
/// opinion and not a second copy of the number: writing a wilder value
/// saves a file that resolves to the clamp anyway and reopens with a
/// slider past its own end.
fn clamp_surface_lift(v: f32) -> f32 {
    v.clamp(-super::bake::SURFACE_LIFT_WALL, super::bake::SURFACE_LIFT_WALL)
}

/// The bake's own wall for the text ladder ([`super::bake::TEXT_LIFT_WALL`]).
fn clamp_text_lift(v: f32) -> f32 {
    v.clamp(-super::bake::TEXT_LIFT_WALL, super::bake::TEXT_LIFT_WALL)
}

/// BASIC's one colour, as edits to the theme's authors.
///
/// SIX TOKENS AND NO MORE, every one of them on this module's ALIVE list with
/// its reader named: `palette.accent`, `palette.black`, `palette.white`,
/// `palette.neutral`, `surface.lift` and `text.lift` — plus `surface.hue` on
/// the one condition below.
///
/// IT USED TO BE TEN, AND SEVEN OF THEM WERE THE MISTAKE (ZGŁOSZENIE 5,
/// 2026-08-18). The seven `severity.<r>.text` were written here because the
/// master held them as frozen literals and nothing else would have made them
/// belong to a re-coloured theme; they were written with the FULL move, so
/// mint -> red carried green `ok` to 10.5 deg and red `critical` to 249.5.
/// The roles are `toward()` expressions in the master now and lean toward
/// `palette.accent` themselves, capped at `severity.pull_clamp` — so the
/// cascade does it, this page does not, and three things fall out at once:
///
/// * a role KEEPS ITS MEANING under any accent (a green success stays green);
/// * a theme that wrote its own role colour keeps it, because BASIC no longer
///   overwrites all seven the moment somebody opens the page — the silence of
///   an edit set means "leave it as it is", and until today this page had
///   nothing silent to say about severity;
/// * the roles still FOLLOW, which is what the owner asked for. They lean.
///
/// AND THREE TOKENS ARRIVED, which is the other half of the same report. The
/// palette's three grounds — `black`, `white`, `neutral` — are hex literals
/// sitting on the accent's own hue at 7 %, 9 % and 13 % of its chroma
/// (measured: h 172.6 / 169.2 / 169.6 against the accent's 166.5). They ARE
/// style. Left behind by a re-colour they hold the whole file back, because
/// `shade()` and `tint()` pull every derived colour toward them — which is
/// why "I change the colour and the background stays as it was" was true.
/// They cannot become expressions over the accent: §5.2 of the master keeps
/// them literal precisely so `shade()`/`tint()` are structurally incapable of
/// closing a cycle, and `expr.rs`'s `black()`/`white()` swallow a cycle into
/// plain black. So the EDITOR writes the literal, the way it already writes a
/// role's. See [`tone_turn`] for why they take the hue and the chroma but
/// never the lightness, and [`tone_spin`] for why `neutral` takes the hue
/// alone.
///
/// WHY SATURATION DOES NOT TOUCH `surface.chroma` OR `text.chroma`. Both
/// ladders take their C from `@chroma.accent = sat(@palette.accent)` and are
/// THEN scaled by those two scalars at bake. Scaling the seed and the scalar
/// both would square the slider: a 1.2 nudge would land as 1.44 on every
/// surface and every letter. So SATURATION moves the seed alone, and a
/// theme's own `surface.chroma = 1.4` survives the trip as the extra tint
/// its author meant.
///
/// WHY LIGHTNESS DOES NEED THE TWO LIFTS. The opposite case: the surface
/// rungs and the text roles carry their L as a LITERAL in each expression
/// (`oklch(0.232, …)`) and take nothing but hue and chroma from the seed, so
/// moving the seed's L moves the accent, the borders and the class bases and
/// leaves every bed and every letter exactly where they were. `surface.lift`
/// and `text.lift` are the master's own answer to that, and this is what
/// they are for.
///
/// THE ONE CONDITIONAL WRITE. `surface.hue` is a reference in the master
/// (`@hue.accent`) and a theme is allowed to override it with plain degrees,
/// cutting the beds loose from the chrome. BASIC's HUE slider promises ONE
/// hue for the whole interface, and over such a theme it could not keep that
/// promise: the accent would turn and the beds would not. So a hue move
/// re-points the token at the accent, and — the same shape as
/// [`border_colour_edit`] — a move that is NOT a hue move does not send it,
/// because arriving on the page and dragging SATURATION must not silently
/// re-weld a surface hue somebody chose on purpose.
pub fn tone_edits(scope: Scope, seeds: &ToneSeeds, tone: Tone) -> Vec<Edit> {
    let Scope::Theme = scope;
    let mut out = Vec::with_capacity(7);
    out.push(accent_edit(scope, tone_shift(seeds.accent, tone)));
    if tone.hue_deg != 0.0 {
        out.push(Edit::new("surface.hue", "@hue.accent"));
    }
    out.push(Edit::new(
        "palette.black",
        oklch_literal(tone_turn(seeds.black, tone)),
    ));
    out.push(Edit::new(
        "palette.white",
        oklch_literal(tone_turn(seeds.white, tone)),
    ));
    out.push(Edit::new(
        "palette.neutral",
        oklch_literal(tone_spin(seeds.neutral, tone)),
    ));
    out.push(Edit::new(
        "surface.lift",
        format!("{:.4}", clamp_surface_lift(seeds.surface_lift + tone.light)),
    ));
    out.push(Edit::new(
        "text.lift",
        format!("{:.4}", clamp_text_lift(seeds.text_lift + tone.light)),
    ));
    out
}

/// How far one notch of each BASIC slider moves.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct ToneStep {
    /// Degrees per notch of HUE.
    pub hue_deg: f32,
    /// Multiplier per notch of SATURATION.
    pub sat: f32,
    /// OKLab L per notch of LIGHTNESS.
    pub light: f32,
}

/// The depth to assume when nobody has said. Eight bits: the floor every
/// swapchain supports.
///
/// WHERE THIS IS APPLIED, and why not here. "Nobody has said" is a fact
/// about the CONFIGURATION, not about the pipeline, and the configuration
/// is the host program's — so this is the number the host's own default
/// takes (`nacelle-desktop/src/config/model.rs`, `ColorConf::DEPTH`) and
/// [`tone_step`] is handed a depth that has already been decided. It was
/// an `Option` here once, with the fallback in this file; nothing ever
/// passed `None`, and the number stood written out in the config besides.
/// One wall, on the side of the seam that knows whether anybody has said.
pub const DEFAULT_DEPTH_BITS: u32 = 8;

/// The slider's notch, from the swapchain's bit depth.
///
/// WHERE THE DEPTH LIVES, and why this is a PARAMETER and not a token. The
/// depth is not a look — it is what the compositor was asked for, chosen on
/// SETTINGS -> COLOR (the DEPTH chips, 8/10/12/16) and kept in the desktop's
/// own config beside the colour space, the LUT and the ICC profile. Putting
/// it in the theme would let a theme file lie about the hardware, and would
/// break the rule that a theme carries appearance and nothing else; reading
/// it from here is worse still, since libnacelle has no config and the value
/// can change while the editor is open. So it arrives as an argument, already
/// decided — [`DEFAULT_DEPTH_BITS`] is what the CONFIG answers when nobody
/// has said, and this function is never the one guessing.
///
/// THE ARITHMETIC. One code of the output channel is `q = 1/(2^bits - 1)`,
/// and a notch is the smallest move that can change one code:
///
/// * LIGHTNESS is an offset in OKLab L over the same 0..1 span, so the notch
///   is `q` itself.
/// * SATURATION is a multiplier over the seed's chroma `C`, so a notch of
///   `k` moves the output by `k*C` and the notch is `q/C`.
/// * HUE turns a colour of chroma `C` along an arc, so `C * theta = q` and
///   the notch is `q/C` radians.
///
/// `C` is the seed's own chroma, so a grey theme gets coarse notches and a
/// vivid one fine ones — which is right: a rotation of a grey moves nothing
/// however far it goes. The guard is `C.max(q)`: below one code the seed is
/// achromatic at this depth, and the two derived notches settle at 1.0 and
/// one radian (57.3 deg) rather than running away. No invented constant.
pub fn tone_step(depth_bits: u32, seed_chroma: f32) -> ToneStep {
    // 16 is the widest the DEPTH chips offer and the widest a float step is
    // worth stating; 1 keeps the shift below from eating the whole word.
    let bits = depth_bits.clamp(1, 16);
    let q = 1.0 / ((1u32 << bits) - 1) as f32;
    let c = seed_chroma.max(q);
    ToneStep { hue_deg: (q / c).to_degrees(), sat: q / c, light: q }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(l: f32, ch: f32, h: f32, a: f32) -> Oklch {
        Oklch { l, c: ch, h, alpha: a }
    }

    #[test]
    fn an_opaque_colour_is_written_without_an_alpha_tail() {
        // The tail is the function's own alpha and reads as a fourth argument;
        // writing `/ 1.000` on every colour would put noise in a file whose
        // diffs a person is meant to read.
        let s = oklch_literal(c(0.232, 0.121, 210.5, 1.0));
        assert_eq!(s, "oklch(0.2320, 0.1210, 210.50)");
        assert!(!s.contains('/'), "an opaque colour grew an alpha tail");
    }

    #[test]
    fn a_translucent_colour_keeps_its_alpha() {
        let s = oklch_literal(c(0.232, 0.121, 210.5, 0.82));
        assert_eq!(s, "oklch(0.2320, 0.1210, 210.50 / 0.820)");
    }

    #[test]
    fn the_border_colour_is_written_once_and_the_halo_wears_it() {
        // `glow.panel_edge.color` exists in the master and has no reader, so
        // a second colour here would be a value that changes nothing. If a
        // reader is ever added, THIS test is where the second write belongs.
        let neon = border_edits(Scope::Theme, Border::Neon, c(0.7, 0.15, 200.0, 1.0), false);
        // The border's colour is the shared root `border.default` (the leaf
        // `elev.panel.edge.color` and every rung reach it by reference); the
        // halo writes no colour of its own.
        let colours: Vec<_> = neon
            .iter()
            .filter(|e| e.token.ends_with("color") || e.token == "border.default")
            .collect();
        assert_eq!(
            colours.len(),
            1,
            "the border wrote {} colours; the halo has none of its own",
            colours.len()
        );
        assert_eq!(colours[0].token, "border.default");
    }

    #[test]
    fn a_lit_kind_dresses_the_light_and_line_does_not_touch_it() {
        let colour = c(0.7, 0.15, 200.0, 1.0);
        let line = border_edits(Scope::Theme, Border::Line, colour, false);
        let of = |v: &Vec<Edit>| v.iter().find(|e| e.token.ends_with("enabled")).unwrap().value.clone();
        assert_eq!(of(&line), "false");
        // Both lit kinds, because the seeding is the REACH and the AMOUNT
        // of light and neither kind owns them: a claim proved on one of
        // them says nothing about the other.
        for kind in [Border::Glow, Border::Neon] {
            let lit = border_edits(Scope::Theme, kind, colour, false);
            let dressed = border_edits(Scope::Theme, kind, colour, true);
            // A theme that has dressed its own keeps it: a theme's own
            // 0.70u must not become the seed's 1.6u because someone
            // picked a kind.
            for k in ["glow.panel_edge.radius", "glow.panel_edge.alpha"] {
                assert!(
                    !dressed.iter().any(|e| e.token == k),
                    "{kind:?} overwrote {k} on a theme that had already dressed its light"
                );
            }
            // A lit kind must write a radius and an alpha, because the
            // default master ships both at zero and the renderer draws
            // nothing at zero — a switch alone was measured invisible on
            // default and inert on Cockpit, which ships the halo on.
            for k in ["glow.panel_edge.radius", "glow.panel_edge.alpha"] {
                assert!(
                    lit.iter().any(|e| e.token == k),
                    "{kind:?} did not write {k}; on the default theme it is invisible"
                );
                // And LINE must NOT: the theme's own dress survives a trip
                // through LINE, so switching back finds it as it was.
                assert!(
                    !line.iter().any(|e| e.token == k),
                    "LINE wrote {k}, flattening the theme's own light"
                );
            }
            assert_eq!(of(&lit), "true");
        }
    }

    /// The two lit kinds differ in the FALLOFF and in nothing else.
    ///
    /// Both halves are load-bearing. That NEON says `tube` is what makes
    /// it a tube at all — the word is the only thing `panel_edge_glow`
    /// asks about. That GLOW says `gauss` is what makes the kind a
    /// promise rather than a hope: without it, picking GLOW on a file
    /// that already said `tube` would leave the tube standing under a
    /// list that reads GLOW.
    ///
    /// And that the two sets are otherwise EQUAL is what makes switching
    /// between them free: a radius, an alpha and a colour the user chose
    /// under one kind are still there under the other.
    #[test]
    fn the_two_lit_kinds_differ_in_the_falloff_alone() {
        let colour = c(0.7, 0.15, 200.0, 1.0);
        for dressed in [false, true] {
            let glow = border_edits(Scope::Theme, Border::Glow, colour, dressed);
            let neon = border_edits(Scope::Theme, Border::Neon, colour, dressed);
            let word = |v: &Vec<Edit>| {
                v.iter()
                    .find(|e| e.token == "glow.panel_edge.falloff")
                    .unwrap_or_else(|| panic!("a lit kind named no falloff: {v:?}"))
                    .value
                    .clone()
            };
            assert_eq!(word(&glow), "gauss");
            assert_eq!(word(&neon), "tube");
            let rest = |v: &Vec<Edit>| -> Vec<Edit> {
                v.iter().filter(|e| e.token != "glow.panel_edge.falloff").cloned().collect()
            };
            assert_eq!(
                rest(&glow),
                rest(&neon),
                "the two lit kinds moved something other than the falloff"
            );
        }
    }

    #[test]
    fn no_set_writes_a_token_nothing_reads() {
        // The whole point of the module. These two are declared by the master
        // and read by no Rust in the workspace (measured 2026-08-17); writing
        // them would produce a file that asks for a gradient border and gets a
        // flat one — which is what the shipped cockpit theme did, until it
        // left with the rest on 2026-08-16.
        const DEAD: [&str; 2] = ["elev.panel.edge.gradient", "glow.panel_edge.color"];
        // `glass.rank` left this list on 2026-08-16, the day it gained its
        // first reader (`elev::Level::draw`, `window::frame`).
        //
        // `edge.color2`, `edge.mode` and `edge.axis` left it on 2026-08-17,
        // the day the SUGAR PAIR gained one (`elev::Level::edge_gradient`).
        // They are still not written here — the editor has no GRADIENT
        // border kind — but a model that started writing them would now be
        // writing something the screen answers, so their absence from this
        // list is the honest state of the workspace rather than a licence.
        // `edge.gradient` stays: `[grad].<name>.stops` is an array, and
        // arrays are dropped by `bake.rs` (`Value::Array(_) => {}`), so
        // there is no baked stop list for any reader to ask for.
        let colour = c(0.7, 0.15, 200.0, 1.0);
        let mut all = Vec::new();
        for kind in [Border::Line, Border::Glow, Border::Neon] {
            all.extend(border_edits(Scope::Theme, kind, colour, false));
            all.extend(border_edits(Scope::Theme, kind, colour, true));
        }
        all.push(border_colour_edit(Scope::Theme, colour));
        for kind in [Glass::Solid, Glass::Blur, Glass::Frosted] {
            all.extend(glass_edits(Scope::Theme, kind, colour, colour, 1.0, 2.0, 0.42, GlassReach::EveryRung));
        }
        for e in &all {
            assert!(
                !DEAD.contains(&e.token),
                "the model wrote `{}`, which no renderer reads",
                e.token
            );
        }
    }

    /// The scope is windows and widgets, and the tokens are the proof: the
    /// shared fill goes through `component.panel.fill` (windows read it
    /// directly, panels inherit it), NEVER through `elev.panel.fill` —
    /// writing the rung's own fill would sever the derivation and the
    /// windows would stop following the colour. And nothing here may touch
    /// the fixture: the desktop's decoration is out of the editor's reach
    /// by construction, which this test keeps true.
    #[test]
    fn the_background_lands_on_the_shared_seam_and_never_on_the_fixture() {
        let colour = c(0.3, 0.05, 220.0, 1.0);
        for kind in [Glass::Solid, Glass::Blur, Glass::Frosted] {
            for e in glass_edits(Scope::Theme, kind, colour, colour, 1.0, 2.0, 0.42, GlassReach::EveryRung) {
                assert!(
                    !e.token.starts_with("elev.fixture"),
                    "{:?} wrote {} — the desktop's decoration is not the editor's",
                    kind,
                    e.token
                );
                assert_ne!(
                    e.token, "elev.panel.fill",
                    "{kind:?} wrote the rung's own fill, severing the derivation \
                     that keeps windows following the colour"
                );
            }
        }
        let solid = glass_edits(Scope::Theme, Glass::Solid, colour, colour, 1.0, 2.0, 0.42, GlassReach::EveryRung);
        assert!(
            solid.iter().any(|e| e.token == "component.panel.fill"),
            "SOLID does not colour the seam both windows and panels read"
        );
        assert!(
            solid.iter().any(|e| e.token == "elev.panel.glass.rank" && e.value == "0"),
            "SOLID left a previous glass standing"
        );
    }

    /// BASIC's literal write, and the one thing it must NOT do that
    /// [`accent_edit`] does: keep the alpha. The picker's alpha channel is
    /// the transparency knob now (2026-08-19, the OPACITY slider's
    /// replacement), so a caller handing this function a translucent
    /// colour must get a translucent token back, not one flattened opaque.
    #[test]
    fn the_panel_fill_write_is_the_one_token_and_keeps_its_alpha() {
        let opaque = panel_fill_edit(Scope::Theme, c(0.24, 0.02, 210.0, 1.0));
        assert_eq!(opaque.token, "component.panel.fill");
        assert!(!opaque.value.contains('/'), "an opaque pick writes no alpha");
        let seen_through = panel_fill_edit(Scope::Theme, c(0.24, 0.02, 210.0, 0.55));
        assert_eq!(seen_through.token, "component.panel.fill");
        assert!(
            seen_through.value.contains("0.550"),
            "the picker's alpha must reach the file: {}",
            seen_through.value
        );
    }

    /// The popover rung's half of the background, named LITERALLY and not
    /// through [`GLASS_RUNGS`]: a test that iterated the same list the
    /// model does would go on passing if the list shrank back to one rung,
    /// which is precisely the regression it exists to catch.
    ///
    /// The claim is the owner's picture, not a token count. A menu and a
    /// tooltip have been surfaces of `elev.popover` since 2026-08-17, so
    /// "the background is frosted glass" has to mean the same thing when a
    /// menu opens over the window as it does for the window under it. Both
    /// glassy kinds, because BLUR and FROSTED differ only in the wash.
    #[test]
    fn a_glassy_background_reaches_the_popover_rung_as_well_as_the_panel() {
        let colour = c(0.62, 0.08, 210.0, 1.0);
        for kind in [Glass::Blur, Glass::Frosted] {
            let edits = glass_edits(Scope::Theme, kind, colour, colour, 0.8, 2.0, 0.42, GlassReach::EveryRung);
            let of = |token: &str| {
                edits.iter().find(|e| e.token == token).map(|e| e.value.clone())
            };
            for key in ["rank", "tint", "wash"] {
                let panel = of(&format!("elev.panel.glass.{key}"));
                let popover = of(&format!("elev.popover.glass.{key}"));
                assert!(
                    popover.is_some(),
                    "{kind:?} frosted the window and left elev.popover.glass.{key} alone, \
                     so a menu over it stays a flat plate"
                );
                assert_eq!(
                    panel, popover,
                    "{kind:?} gave the popover rung a different {key} from the panel's; \
                     one background means one material"
                );
            }
        }
    }

    /// …and the way back. A rank is what turns the glass ON, so SOLID has
    /// to write it to zero on EVERY rung it ever raised — otherwise the
    /// menu keeps the frost it was given, the file says so, and no control
    /// in the editor can take it off again.
    #[test]
    fn going_back_to_solid_takes_the_glass_off_the_popover_rung_too() {
        let colour = c(0.3, 0.05, 220.0, 1.0);
        let solid = glass_edits(Scope::Theme, Glass::Solid, colour, colour, 1.0, 2.0, 0.42, GlassReach::EveryRung);
        assert!(
            solid.iter().any(|e| e.token == "elev.popover.glass.rank" && e.value == "0"),
            "SOLID left the popover rung frosted: {solid:?}"
        );
    }

    /// The OPACITY knob owns the tint's alpha for BOTH glassy kinds, and
    /// the COVERAGE knob owns the wash's — the sliders' own alphas are
    /// discarded on purpose, so the file always carries what the knobs say
    /// and never a stale channel smuggled in through a seeded colour.
    #[test]
    fn the_knobs_own_the_alphas_and_the_colours_do_not() {
        let tint = c(0.6, 0.05, 220.0, 0.5);
        let wash = c(0.2, 0.02, 220.0, 0.34);
        let tint_of = |v: &Vec<Edit>| {
            v.iter().find(|e| e.token.ends_with("glass.tint")).unwrap().value.clone()
        };
        // Full opacity: no alpha tail, whatever the seeded colour carried.
        let blur = glass_edits(Scope::Theme, Glass::Blur, tint, wash, 1.0, 2.0, 0.42, GlassReach::EveryRung);
        assert!(!tint_of(&blur).contains('/'), "full opacity grew an alpha tail");
        // Dialled down: the tail is the KNOB's number, for blur and frosted
        // alike — a translucent tint blends the blur with the sharp scene.
        for kind in [Glass::Blur, Glass::Frosted] {
            let v = glass_edits(Scope::Theme, kind, tint, wash, 0.6, 2.0, 0.42, GlassReach::EveryRung);
            assert!(
                tint_of(&v).contains("/ 0.600"),
                "{kind:?}: the tint's alpha is not the opacity knob's 0.6"
            );
        }
        // And the wash follows coverage, not the seeded colour's 0.34.
        let frosted = glass_edits(Scope::Theme, Glass::Frosted, tint, wash, 1.0, 2.0, 0.7, GlassReach::EveryRung);
        let wash_of = frosted
            .iter()
            .find(|e| e.token.ends_with("glass.wash"))
            .unwrap();
        assert!(
            wash_of.value.contains("/ 0.700"),
            "the wash's alpha is not the coverage knob's 0.7"
        );
        // Depth lands in the rank, clamped to the pyramid the renderer has.
        let deep = glass_edits(Scope::Theme, Glass::Blur, tint, wash, 1.0, 9.0, 0.42, GlassReach::EveryRung);
        assert!(
            deep.iter().any(|e| e.token.ends_with("glass.rank") && e.value == "3.00"),
            "a depth past the pyramid was not clamped"
        );
        // And a fraction survives to the file — the whole point of the
        // two-fan emitter.
        let mid = glass_edits(Scope::Theme, Glass::Blur, tint, wash, 1.0, 1.7, 0.42, GlassReach::EveryRung);
        assert!(
            mid.iter().any(|e| e.token.ends_with("glass.rank") && e.value == "1.70"),
            "a fractional depth was rounded away"
        );
    }

    // ---------------------------- ZGŁOSZENIE 7 (2026-08-18): how far the
    // ---------------------------- alpha a person moved is allowed to go.

    /// The owner's rule for BASIC: "zmiana przezroczystości wpływa TYLKO na
    /// główne tło obiektu".
    ///
    /// The two halves are measured apart, because they must not be traded
    /// for each other: the RANK still reaches every rung (a menu over a
    /// frosted window is frosted), and the two COLOUR keys — which is where
    /// the alpha lives — stop at the body.
    #[test]
    fn a_body_only_background_carries_the_kind_everywhere_and_the_alpha_nowhere_else() {
        let tint = c(0.7, 0.15, 200.0, 1.0);
        let wash = c(0.3, 0.05, 40.0, 1.0);
        for kind in [Glass::Blur, Glass::Frosted] {
            let narrow =
                glass_edits(Scope::Theme, kind, tint, wash, 0.4, 2.0, 0.7, GlassReach::BodyOnly);
            let wide =
                glass_edits(Scope::Theme, kind, tint, wash, 0.4, 2.0, 0.7, GlassReach::EveryRung);
            let has = |v: &[Edit], t: &str| v.iter().any(|e| e.token == t);
            // The kind travels, both ways round.
            for set in [&narrow, &wide] {
                for (rank_key, _, _) in GLASS_RUNGS {
                    assert!(
                        has(set, rank_key),
                        "{kind:?}: `{rank_key}` was not written — a float that never \
                         learns the theme is glassy is the flat plate this set exists \
                         to prevent"
                    );
                }
            }
            // The body's own rung takes the colours under both reaches.
            for key in ["elev.panel.glass.tint", "elev.panel.glass.wash"] {
                assert!(has(&narrow, key), "{kind:?}: the body lost `{key}`");
            }
            // And the float's do not, under BodyOnly alone.
            for key in ["elev.popover.glass.tint", "elev.popover.glass.wash"] {
                assert!(
                    has(&wide, key),
                    "{kind:?}: EveryRung must still dress `{key}` — ADVANCED's answer \
                     did not change"
                );
                assert!(
                    !has(&narrow, key),
                    "{kind:?}: BASIC's transparency reached `{key}`, which is the menu \
                     and the tooltip and not the object's own background"
                );
            }
        }
    }

    /// And the alpha that DOES land is the caller's number, not a rounding
    /// of it — the expectation is spelled out here from the argument, so a
    /// change to `oklch_literal` cannot move both sides of the equation at
    /// once.
    #[test]
    fn the_bodys_own_rung_wears_the_alpha_the_caller_asked_for() {
        let tint = c(0.7, 0.15, 200.0, 1.0);
        let edits =
            glass_edits(Scope::Theme, Glass::Blur, tint, tint, 0.4, 2.0, 0.7, GlassReach::BodyOnly);
        let body = edits
            .iter()
            .find(|e| e.token == "elev.panel.glass.tint")
            .expect("the body's tint");
        assert!(
            body.value.ends_with("/ 0.400)"),
            "the body's tint is `{}` and the knob said 0.4",
            body.value
        );
    }

    // ------------------- ZGŁOSZENIE 6 (2026-08-18): the border's thickness
    // ------------------- and the reach of its light — two readings, both
    // ------------------- of them tokens the master already declares.

    #[test]
    fn the_border_carries_its_own_thickness_and_not_the_global_kerf() {
        let e = border_width_edit(Scope::Theme, 0.35);
        assert_eq!(
            e.token, "border.edge.width",
            "the border's thickness must be the border's own key; `stroke.hair` is the \
             kerf 72 derivations share and the editor already offers it under HAIRLINE"
        );
        assert_eq!(e.value, "0.35u");
        // The wall is the master's heaviest stroke rounded up, the same
        // wall `shape_edits` puts on the kerf: `[stroke] bold = 0.7u`.
        assert_eq!(
            border_width_edit(Scope::Theme, 9.0).value,
            "1.00u",
            "a width past every weight the master states was let through"
        );
        assert_eq!(border_width_edit(Scope::Theme, -3.0).value, "0.00u");
    }

    #[test]
    fn a_lit_borders_reach_is_held_inside_the_range_the_master_declares() {
        let e = glow_reach_edit(Scope::Theme, 2.4);
        assert_eq!(e.token, "glow.panel_edge.radius");
        assert_eq!(e.value, "2.40u");
        // The master declares `u, 0u .. 8.76u` for this key (its own doc
        // carries the 4mm-times-four derivation, 2026-08-25), and 0u is
        // its own `none` sentinel — both ends are the FILE's.
        assert_eq!(
            glow_reach_edit(Scope::Theme, 12.0).value,
            "8.76u",
            "a reach past the master's declared 8.76u was let through"
        );
        assert_eq!(glow_reach_edit(Scope::Theme, -1.0).value, "0.00u");
    }

    /// The seed a lit kind lays under an undressed theme is a FLOOR, and a
    /// number a person moved outranks it. The merge is the caller's — this
    /// only pins that the two really do collide on one token, which is the
    /// fact that makes a merge necessary at all.
    #[test]
    fn the_reach_a_person_sets_and_the_seed_a_kind_lays_are_one_token() {
        let lit = border_edits(Scope::Theme, Border::Neon, c(0.7, 0.15, 200.0, 1.0), false);
        let seeded = lit
            .iter()
            .find(|e| e.token == "glow.panel_edge.radius")
            .expect("an undressed theme must be given a reach by the kind");
        assert_eq!(
            seeded.token,
            glow_reach_edit(Scope::Theme, 1.0).token,
            "if these two ever stop naming one token the caller's merge is a no-op \
             and the knob stops winning"
        );
    }

    // ------------------------------------ the whole-theme sets (2026-08-16)

    /// Every token the new sets may write, each with the reader that earns
    /// it a place. THE list the module's iron rule stands on: a control
    /// exists only for a token some Rust reads, and the anchors were
    /// grepped on 2026-08-16, not inherited from the reconnaissance.
    const ALIVE: &[(&str, &str)] = &[
        ("palette.accent", "theme/bake.rs:859; class ladders via view/surface.rs:527"),
        ("palette.black", "theme/expr.rs `Env::black` — shade()'s only target, 10 callers in the master"),
        ("palette.white", "theme/expr.rs `Env::white` — tint()'s only target"),
        ("palette.neutral", "severity.offline.text (default.theme) -> view/paint.rs:42; ui.rs:147"),
        ("surface.hue", "the six level exprs (default.theme:317-331); levels read at deco.rs:33, winframe.rs:414"),
        ("surface.lift", "theme/bake.rs:519"),
        ("surface.chroma", "theme/bake.rs:520"),
        ("text.hue", "the six text role exprs (default.theme:382-395); roles reach the screen at view/paint.rs:157"),
        ("text.lift", "theme/bake.rs:525"),
        ("text.chroma", "theme/bake.rs:526"),
        ("severity.ok.text", "view/paint.rs:42; ui.rs:147"),
        ("severity.info.text", "view/paint.rs:42; ui.rs:147"),
        ("severity.warning.text", "view/paint.rs:42; toaster.rs:234; accent.warm (default.theme:442)"),
        ("severity.critical.text", "view/paint.rs:42; ui.rs:147"),
        ("severity.contained.text", "view/paint.rs:42; ui.rs:147"),
        ("severity.offline.text", "view/paint.rs:42; ui.rs:147"),
        ("severity.unknown.text", "view/paint.rs:42; ui.rs:147"),
        ("corner.mode", "derived words read at window.rs:158, menu.rs:438, tooltip.rs:259, winframe.rs:450, view/paint.rs:690"),
        ("corner.sm", "tooltip.rs:263 via tooltip.corner = @corner.sm, among 41 @corner.* refs"),
        ("corner.md", "window.rs:165 via panel.corner = @corner.md"),
        ("corner.lg", "winframe.rs:98 via winframe.corner"),
        ("corner.segments", "window.rs:74; focus_ring.rs:134; winframe.rs:452"),
        ("stroke.hair", "view/paint.rs:595 and :656; 72 @stroke.hair derivations"),
        ("focus.ring.enabled", "focus_ring.rs:60"),
        ("focus.ring.style", "focus_ring.rs:70"),
        ("focus.ring.width", "focus_ring.rs:63"),
        ("focus.ring.offset", "focus_ring.rs:67"),
        ("focus.ring.color", "focus_ring.rs:87"),
        ("focus.ring.dash", "focus_ring.rs:76"),
        ("focus.ring.gap", "focus_ring.rs:76"),
        ("glow.focus_ring.enabled", "focus_ring.rs:213"),
        ("glow.focus_ring.radius", "focus_ring.rs:216"),
        ("glow.focus_ring.alpha", "focus_ring.rs:217"),
        ("focus.unfocused_dim", "winframe.rs:467"),
        ("component.menu.fill", "menu.rs:445; winframe.rs:683"),
        ("component.menu.border", "menu.rs:453 and :482; winframe.rs:689"),
        ("menu.border", "menu.rs:446; winframe.rs:688"),
        ("component.menu.hint", "menu.rs:458"),
        ("component.tooltip.fill", "tooltip.rs:266"),
        ("component.tooltip.edge", "tooltip.rs:269"),
        ("tooltip.border", "tooltip.rs:267"),
        ("component.tooltip.text", "tooltip.rs:273"),
        ("scrollbar.mode", "view/scroll.rs:559 and :595"),
        ("scrollbar.edge", "view/scroll.rs:572"),
        ("scrollbar.w", "view/scroll.rs:576"),
        ("scrollbar.w_hover", "view/scroll.rs:577"),
        ("scrollbar.auto_hide", "view/scroll.rs:585"),
        ("scrollbar.fade_ms", "view/scroll.rs:586"),
        ("scrollbar.track", "view/paint.rs:673"),
        ("component.scrollbar.track", "view/paint.rs:674"),
        // ZGŁOSZENIE 6 (2026-08-18): the two readings of "promień borderu".
        ("border.edge.width", "theme/mod.rs:1939 (hot table `border_width`); elev.panel.edge.width = @panel.border -> @border.edge.width (default.theme:1767), read at object/elev.rs:421"),
        ("glow.panel_edge.radius", "object/window.rs:104 — returns at zero, so this is the number that decides whether a lit border is visible at all"),
    ];

    /// Declared by the master and read by nothing that reaches the screen,
    /// so a control over one of them would look as if it worked and change
    /// no picture. Measured 2026-08-16, revised 2026-08-18.
    ///
    /// TWO LEFT THE LIST ON 2026-08-18, and the way they left is the point.
    /// `severity.pull` and `severity.pull_clamp` used to sit here because
    /// the "engine" their comments described did not exist — and the fix
    /// was NOT to write that engine in Rust. The master now spells the pull
    /// out where the colour is written (`severity.<r>.text =
    /// toward(oklch(...), @palette.accent, @severity.pull,
    /// @severity.pull_clamp)`), so both tokens have a reader, both change
    /// the picture, and a control over either would do what it says. The
    /// reader is arithmetic in `theme/expr.rs`; the numbers stayed in the
    /// theme file, which is where this project keeps appearance.
    ///
    /// THE OTHER TWO COULD NOT LEAVE THE SAME WAY, and this is a finding
    /// rather than an oversight. `severity.mode` selects among FOUR
    /// generations (`hue | mono | mono_plus_warning | mono_strict`), and an
    /// expression cannot select between generations — only a generator can,
    /// and `hue` is the only one of the four that anything in this workspace
    /// can produce. `severity.chroma` is declared to scale "the mono modes"
    /// and so cannot come alive before `mode` does. Both stay here until
    /// somebody writes the three mono generations; neither may be given a
    /// control before then, which is exactly what this list is for.
    const DEAD_CONTROLS: [&str; 3] = [
        "severity.mode",
        "severity.chroma",
        "glow.focus_ring.color",
    ];

    /// Every branch of every new set, for the whitelist test and the
    /// dead-token test both — a conditional write that only fires for
    /// DASHED or for a dressed halo must not escape the net.
    fn all_new_edits() -> Vec<Edit> {
        let colour = c(0.7, 0.15, 200.0, 1.0);
        let mut all = vec![accent_edit(Scope::Theme, colour)];
        for hue in [SurfaceHue::FollowAccent, SurfaceHue::Own(210.0)] {
            all.extend(surface_edits(Scope::Theme, hue, 0.02, 1.2));
        }
        all.extend(text_edits(Scope::Theme, SurfaceHue::Own(210.0), -0.03, 0.8));
        for role in [
            SeverityRole::Ok,
            SeverityRole::Info,
            SeverityRole::Warning,
            SeverityRole::Critical,
            SeverityRole::Contained,
            SeverityRole::Offline,
            SeverityRole::Unknown,
        ] {
            all.push(severity_role_edit(Scope::Theme, role, colour));
        }
        for cut in [CornerCut::Square, CornerCut::Round, CornerCut::Chamfer] {
            all.extend(shape_edits(Scope::Theme, cut, 0.8, 1.2, 2.2, 6, 0.2));
        }
        for (enabled, style, halo, dressed) in [
            (false, RingStyle::Solid, false, false),
            (true, RingStyle::Solid, false, false),
            (true, RingStyle::Dashed, true, false),
            (true, RingStyle::Dashed, true, true),
        ] {
            let ring = FocusRing {
                style,
                width_u: 0.3,
                offset_u: 0.4,
                colour,
                dash_u: 1.6,
                gap_u: 0.8,
                halo,
                halo_alpha: 0.3,
                halo_dressed: dressed,
            };
            all.extend(focus_ring_edits(Scope::Theme, enabled, &ring));
        }
        all.push(unfocused_dim_edit(Scope::Theme, 0.62));
        // ZGŁOSZENIE 6's two: the border's own thickness, and the reach of
        // its light. Both ends of both walls, so a clamp cannot smuggle a
        // token past this net.
        for w in [0.0, 0.2, 5.0] {
            all.push(border_width_edit(Scope::Theme, w));
            all.push(glow_reach_edit(Scope::Theme, w));
        }
        // BASIC's three sliders, both sides of the one conditional write.
        for tone in [Tone::NEUTRAL, Tone { hue_deg: 37.0, sat: 1.3, light: -0.02 }] {
            all.extend(tone_edits(Scope::Theme, &seeds(), tone));
        }
        all.extend(menu_edits(Scope::Theme, colour, colour, 0.2, colour));
        all.extend(tooltip_edits(Scope::Theme, colour, colour, 0.2, colour));
        for (mode, auto_hide, track) in [
            (ScrollbarMode::Overlay, true, Some(colour)),
            (ScrollbarMode::Inset, false, None),
            (ScrollbarMode::None, true, None),
        ] {
            all.extend(scrollbar_edits(
                Scope::Theme,
                mode,
                ScrollbarEdge::Right,
                1.2,
                2.0,
                auto_hide,
                260.0,
                track,
            ));
        }
        all
    }

    /// The iron rule, applied to every new set at once: only tokens off the
    /// ALIVE list, never one off the dead list. A token missing from ALIVE
    /// fails even if it is real — adding it there WITH ITS ANCHOR is the
    /// price of writing it, which is the point.
    #[test]
    fn every_token_a_new_set_writes_has_a_named_reader() {
        for e in all_new_edits() {
            assert!(
                !DEAD_CONTROLS.contains(&e.token),
                "the model wrote `{}`, which no renderer reads",
                e.token
            );
            assert!(
                ALIVE.iter().any(|(t, _)| *t == e.token),
                "`{}` is not on the ALIVE list; name its reader there before writing it",
                e.token
            );
        }
    }

    #[test]
    fn the_accent_and_a_severity_pin_write_one_opaque_author_each() {
        let translucent = c(0.7, 0.15, 200.0, 0.4);
        let a = accent_edit(Scope::Theme, translucent);
        assert_eq!(a.token, "palette.accent");
        // The derivations own the alphas (border.default at 0.78 and kin);
        // a translucent seed would fade the raw uses and nothing else.
        assert!(!a.value.contains('/'), "the accent seed kept a slider's stray alpha");
        let s = severity_role_edit(Scope::Theme, SeverityRole::Critical, translucent);
        assert_eq!(s.token, "severity.critical.text");
        assert!(!s.value.contains('/'), "the severity author kept a stray alpha");
        // Each role pins ITS text: seven roles, seven distinct tokens.
        let mut tokens: Vec<&str> = [
            SeverityRole::Ok,
            SeverityRole::Info,
            SeverityRole::Warning,
            SeverityRole::Critical,
            SeverityRole::Contained,
            SeverityRole::Offline,
            SeverityRole::Unknown,
        ]
        .map(|r| severity_role_edit(Scope::Theme, r, translucent).token)
        .to_vec();
        tokens.dedup();
        assert_eq!(tokens.len(), 7, "two severity roles collided on one token");
    }

    #[test]
    fn the_surface_hue_is_a_reference_until_a_number_cuts_it_loose() {
        let follow = surface_edits(Scope::Theme, SurfaceHue::FollowAccent, 0.0, 1.0);
        assert_eq!(
            follow[0].value, "@hue.accent",
            "FOLLOW must restore the derivation as a reference, or a later \
             accent drag stops moving the surfaces"
        );
        let own = surface_edits(Scope::Theme, SurfaceHue::Own(410.0), 0.0, 1.0);
        assert_eq!(own[0].value, "50.00", "degrees are written on the circle, 410 = 50");
        // The clamps are the bake's own (bake.rs:519-520, :525-526): a file
        // must not carry a number the resolve would clamp anyway.
        let wild = surface_edits(Scope::Theme, SurfaceHue::FollowAccent, 0.5, 9.0);
        assert_eq!(wild[1].value, "0.0900");
        assert_eq!(wild[2].value, "4.000");
        let text = text_edits(Scope::Theme, SurfaceHue::Own(410.0), -0.5, 9.0);
        assert_eq!(text[0].value, "50.00", "text hue is written on the circle, 410 = 50");
        assert_eq!(text[1].value, "-0.1000");
        assert_eq!(text[2].value, "3.000");
        // FollowAccent restores the master's reference, so a font that has
        // not been cut loose keeps moving with the accent.
        let follow = text_edits(Scope::Theme, SurfaceHue::FollowAccent, 0.0, 1.0);
        assert_eq!(follow[0].token, "text.hue");
        assert_eq!(follow[0].value, "@hue.accent");
    }

    #[test]
    fn the_shape_set_speaks_the_language() {
        let e = shape_edits(Scope::Theme, CornerCut::Chamfer, 9.0, 1.2, 2.2, 40, 9.0);
        let of = |t: &str| e.iter().find(|x| x.token == t).unwrap().value.clone();
        assert_eq!(of("corner.mode"), "chamfer");
        // Lengths carry the unit; a bare number would bake as device px
        // and shrink on every display denser than the author's.
        for t in ["corner.sm", "corner.md", "corner.lg", "stroke.hair"] {
            assert!(of(t).ends_with('u'), "{t} lost its unit: {}", of(t));
        }
        assert_eq!(of("corner.sm"), "4.00u", "the radius wall (4u) did not hold");
        assert_eq!(of("stroke.hair"), "1.00u", "the kerf wall (1u) did not hold");
        // Segments are a count with the declared range (3..16); a fraction
        // of a tessellation quad does not exist.
        assert_eq!(of("corner.segments"), "16");
        let few = shape_edits(Scope::Theme, CornerCut::Round, 0.8, 1.2, 2.2, 1, 0.2);
        assert!(few.iter().any(|x| x.token == "corner.segments" && x.value == "3"));
    }

    #[test]
    fn a_disabled_ring_is_one_flag_and_the_dress_stands() {
        let ring = FocusRing {
            style: RingStyle::Dashed,
            width_u: 0.3,
            offset_u: 0.4,
            colour: c(0.7, 0.15, 200.0, 1.0),
            dash_u: 1.6,
            gap_u: 0.8,
            halo: true,
            halo_alpha: 0.3,
            halo_dressed: false,
        };
        let off = focus_ring_edits(Scope::Theme, false, &ring);
        // focus_ring.rs:60-62 returns on the flag before anything else is
        // read, so the flag is the WHOLE of "off" — everything more would
        // flatten a dress the renderer was not even going to look at.
        assert_eq!(off.len(), 1, "OFF wrote more than the flag: {off:?}");
        assert_eq!(off[0].token, "focus.ring.enabled");
        assert_eq!(off[0].value, "false");
    }

    #[test]
    fn solid_keeps_the_dashed_rhythm_and_the_halo_dresses_like_neon() {
        let mut ring = FocusRing {
            style: RingStyle::Solid,
            width_u: 0.3,
            offset_u: 0.4,
            colour: c(0.7, 0.15, 200.0, 1.0),
            dash_u: 1.6,
            gap_u: 0.8,
            halo: true,
            halo_alpha: 0.3,
            halo_dressed: false,
        };
        let solid = focus_ring_edits(Scope::Theme, true, &ring);
        for t in ["focus.ring.dash", "focus.ring.gap"] {
            assert!(
                !solid.iter().any(|e| e.token == t),
                "SOLID wrote {t}, flattening the theme's dashed rhythm"
            );
        }
        // The halo mirrors NEON exactly: the master ships radius 0u and
        // alpha 0.0 (default.theme:1030/:1039) and the renderer returns at
        // zero, so an undressed theme gets the seed radius — and a dressed
        // one keeps its own.
        assert!(solid.iter().any(|e| e.token == "glow.focus_ring.radius"));
        assert!(solid.iter().any(|e| e.token == "glow.focus_ring.alpha" && e.value == "0.300"));
        ring.halo_dressed = true;
        let dressed = focus_ring_edits(Scope::Theme, true, &ring);
        assert!(
            !dressed.iter().any(|e| e.token == "glow.focus_ring.radius"),
            "the halo seed overwrote a theme's own radius"
        );
        ring.halo = false;
        let bare = focus_ring_edits(Scope::Theme, true, &ring);
        assert!(bare.iter().any(|e| e.token == "glow.focus_ring.enabled" && e.value == "false"));
        for t in ["glow.focus_ring.radius", "glow.focus_ring.alpha"] {
            assert!(
                !bare.iter().any(|e| e.token == t),
                "halo OFF wrote {t}, flattening the dress"
            );
        }
        // And the dim lives outside the ring set, so it works with the
        // ring off; the floor is the declared one (0.3 — a window must
        // not vanish).
        let dim = unfocused_dim_edit(Scope::Theme, 0.0);
        assert_eq!(dim.token, "focus.unfocused_dim");
        assert_eq!(dim.value, "0.300");
    }

    #[test]
    fn the_menu_and_tooltip_sets_cover_both_floats_exactly() {
        let k = c(0.4, 0.05, 220.0, 1.0);
        let menu: Vec<&str> = menu_edits(Scope::Theme, k, k, 0.2, k).iter().map(|e| e.token).collect();
        assert_eq!(
            menu,
            ["component.menu.fill", "component.menu.border", "menu.border", "component.menu.hint"],
            "the menu set drifted from the four tokens menu.rs and winframe.rs read"
        );
        let tip: Vec<&str> =
            tooltip_edits(Scope::Theme, k, k, 0.2, k).iter().map(|e| e.token).collect();
        assert_eq!(
            tip,
            [
                "component.tooltip.fill",
                "component.tooltip.edge",
                "tooltip.border",
                "component.tooltip.text"
            ],
            "the tooltip set drifted from the four tokens tooltip.rs reads"
        );
        // The widths carry their unit, and a negative slider means zero,
        // the renderer's own floor (menu.rs:446, tooltip.rs:267).
        let w = menu_edits(Scope::Theme, k, k, -1.0, k);
        assert!(w.iter().any(|e| e.token == "menu.border" && e.value == "0.00u"));
    }

    #[test]
    fn the_scrollbar_track_and_fade_follow_their_switches() {
        let k = c(0.2, 0.02, 220.0, 1.0);
        let on = scrollbar_edits(
            Scope::Theme,
            ScrollbarMode::Inset,
            ScrollbarEdge::Left,
            0.1,
            9.0,
            true,
            5000.0,
            Some(k),
        );
        let of = |v: &Vec<Edit>, t: &str| v.iter().find(|e| e.token == t).map(|e| e.value.clone());
        assert_eq!(of(&on, "scrollbar.mode").unwrap(), "inset");
        assert_eq!(of(&on, "scrollbar.edge").unwrap(), "left");
        // The walls: below 0.5u the bar cannot be aimed at, and the fade's
        // declared range ends at 2000ms — with the unit written, because
        // the token is a duration, not a length.
        assert_eq!(of(&on, "scrollbar.w").unwrap(), "0.50u");
        assert_eq!(of(&on, "scrollbar.w_hover").unwrap(), "4.00u");
        assert_eq!(of(&on, "scrollbar.fade_ms").unwrap(), "2000ms");
        assert_eq!(of(&on, "scrollbar.track").unwrap(), "on");
        assert!(of(&on, "component.scrollbar.track").is_some());
        let off = scrollbar_edits(
            Scope::Theme,
            ScrollbarMode::Overlay,
            ScrollbarEdge::Right,
            1.2,
            2.0,
            false,
            260.0,
            None,
        );
        // The declaration says the fade is read only when auto_hide is on
        // (default.theme:4837), and OFF must not repaint the groove: the
        // switch is written, the dress is not.
        assert!(of(&off, "scrollbar.fade_ms").is_none(), "the fade was written for a bar that never fades");
        assert_eq!(of(&off, "scrollbar.track").unwrap(), "off");
        assert!(
            of(&off, "component.scrollbar.track").is_none(),
            "track OFF overwrote the theme's groove colour"
        );
    }

    // ------------------------------------------------------------- BASIC

    /// The master's own authors, near enough: a mint accent, the three
    /// grounds sitting on its hue at a fraction of its chroma the way the
    /// master's really do, both ladders unlifted.
    fn seeds() -> ToneSeeds {
        ToneSeeds {
            accent: c(0.82, 0.130, 162.0, 1.0),
            black: c(0.166, 0.010, 172.6, 1.0),
            white: c(0.963, 0.014, 169.2, 1.0),
            neutral: c(0.565, 0.020, 169.6, 1.0),
            surface_lift: 0.0,
            text_lift: 0.0,
        }
    }

    fn value_of(edits: &[Edit], token: &str) -> Option<String> {
        edits.iter().find(|e| e.token == token).map(|e| e.value.clone())
    }

    /// The set is closed at six tokens plus the one conditional weld, and
    /// every one of them is an AUTHOR. The two derived families a slider
    /// might tempt somebody into — `hue.accent`/`chroma.accent`, which are
    /// literally `hue(@palette.accent)` and `sat(@palette.accent)` — must
    /// never be written: pinning them would cut the cascade at the joint
    /// and the next drag would move the seed and nothing else.
    ///
    /// It was TEN until 2026-08-18, and seven of the ten were the severity
    /// roles. They are the theme's own expressions now.
    #[test]
    fn basic_writes_authors_only_and_nothing_derived() {
        let quiet = tone_edits(Scope::Theme, &seeds(), Tone { hue_deg: 0.0, sat: 1.2, light: 0.01 });
        let names: Vec<&str> = quiet.iter().map(|e| e.token).collect();
        assert_eq!(
            names,
            [
                "palette.accent",
                "palette.black",
                "palette.white",
                "palette.neutral",
                "surface.lift",
                "text.lift",
            ],
            "BASIC's token set drifted"
        );
        // The three palette GROUNDS are gone from this list as of
        // 2026-08-18, and the reason is that they never belonged on it: they
        // are not derived and cannot be. §5.2 of the master keeps
        // `palette.black` and `palette.white` literal on purpose, so that
        // `shade()` and `tint()` — which take no other target — are
        // structurally incapable of closing a cycle, and `palette.neutral`
        // is a literal beside them. Nothing in the cascade was ever going to
        // carry them, which is why a re-coloured theme kept its old
        // background. An editor writing the literal is the only road there
        // is; see [`tone_turn`].
        for derived in [
            "hue.accent",
            "chroma.accent",
            "accent.primary",
            "text.title",
            "surface.panel",
            "surface.chroma",
            "text.chroma",
        ] {
            assert!(
                value_of(&quiet, derived).is_none(),
                "BASIC pinned `{derived}`, which the cascade derives — the next drag would find it deaf"
            );
        }
        // Doubling the slider is the trap this avoids: SATURATION scales the
        // seed, and the two ladder scalars stay the theme's own.
        let loud = tone_edits(Scope::Theme, &seeds(), Tone { hue_deg: 0.0, sat: 2.0, light: 0.0 });
        assert!(value_of(&loud, "surface.chroma").is_none());
        assert!(value_of(&loud, "text.chroma").is_none());
    }

    /// The rotation is RELATIVE, which is the whole of the owner's decision:
    /// every author turns by the same number of degrees, so the chrome —
    /// which has ONE author — lands on one hue, and severity — which has
    /// SEVEN — keeps every gap its author wrote.
    #[test]
    fn a_hue_move_turns_every_author_by_the_same_degrees() {
        let s = seeds();
        for turn in [17.0f32, 90.0, 213.0, -140.0] {
            let e = tone_edits(Scope::Theme, &s, Tone { hue_deg: turn, ..Tone::NEUTRAL });
            let hue_written = |token: &str| -> f32 {
                let v = value_of(&e, token).unwrap();
                let inner = v.trim_start_matches("oklch(").trim_end_matches(')');
                inner.split(',').nth(2).unwrap().trim().parse::<f32>().unwrap()
            };
            let want = |h: f32| (h + turn).rem_euclid(360.0);
            for (token, seed) in [
                ("palette.accent", s.accent),
                ("palette.black", s.black),
                ("palette.white", s.white),
                ("palette.neutral", s.neutral),
            ] {
                assert!(
                    (hue_written(token) - want(seed.h)).abs() < 0.02,
                    "{token} did not turn with the rest at {turn} deg"
                );
            }
        }
    }

    /// ZGŁOSZENIE 5, first half: **the three grounds come along, and they
    /// bring only what they are allowed to bring.**
    ///
    /// `palette.black`, `white` and `neutral` are hex literals on the
    /// accent's own hue — style frozen in hex — and leaving them behind is
    /// what made "I change the colour and the background does not change"
    /// true. They must turn. What they must NOT do is take the lightness
    /// (their L is the theme's polarity, the poles `shade()` and `tint()`
    /// pull everything toward) or, for `neutral`, the chroma (its whole job
    /// is to be colourless, and `severity.offline.text` rides it).
    #[test]
    fn the_palettes_three_grounds_take_the_turn_but_neither_pole_moves() {
        let s = seeds();
        let e = tone_edits(
            Scope::Theme,
            &s,
            Tone { hue_deg: 137.0, sat: 2.0, light: 0.06 },
        );
        let part = |token: &str, i: usize| -> f32 {
            let v = value_of(&e, token).unwrap();
            let inner = v.trim_start_matches("oklch(").trim_end_matches(')');
            inner.split(',').nth(i).unwrap().trim().parse::<f32>().unwrap()
        };
        // The poles keep their own lightness while the accent takes the lift.
        assert!((part("palette.black", 0) - s.black.l).abs() < 0.001, "black changed polarity");
        assert!((part("palette.white", 0) - s.white.l).abs() < 0.001, "white changed polarity");
        assert!(
            (part("palette.accent", 0) - (s.accent.l + 0.06)).abs() < 0.001,
            "the accent did not take the lightness the poles refused"
        );
        // SATURATION reaches the two poles (they are the accent's own hue at
        // a fraction of its chroma, and the fraction is the theme's) …
        assert!((part("palette.black", 1) - s.black.c * 2.0).abs() < 0.001);
        assert!((part("palette.white", 1) - s.white.c * 2.0).abs() < 0.001);
        // … and never the grey anchor, which would go visibly green at 200 %.
        assert!(
            (part("palette.neutral", 1) - s.neutral.c).abs() < 0.0005,
            "SATURATION coloured the hue-free anchor: {} -> {}",
            s.neutral.c,
            part("palette.neutral", 1)
        );
        assert!((part("palette.neutral", 0) - s.neutral.l).abs() < 0.001);
    }

    /// ZGŁOSZENIE 5, second half: **BASIC has nothing to say about the
    /// severity roles any more.**
    ///
    /// It used to write all seven with the full rotation, which is how a
    /// green success came out red. The roles lean toward `palette.accent`
    /// on their own now — `toward()` in the master, capped at
    /// `severity.pull_clamp` — so this page writing them would be a second
    /// opinion over the top of the first, and a destructive one: a theme
    /// that dressed its own `contained` amber lost it the moment anybody
    /// opened this page, because silence is what "leave it as it is" is
    /// spelled with and this set had none.
    #[test]
    fn basic_leaves_every_severity_role_to_the_theme() {
        for tone in [
            Tone::NEUTRAL,
            Tone { hue_deg: 193.0, sat: 1.4, light: -0.02 },
            Tone { hue_deg: -137.5, ..Tone::NEUTRAL }, // mint -> red, the reported one
        ] {
            let e = tone_edits(Scope::Theme, &seeds(), tone);
            let touched: Vec<_> =
                e.iter().filter(|x| x.token.starts_with("severity.")).collect();
            assert!(
                touched.is_empty(),
                "BASIC repainted the severity roles: {touched:?}"
            );
        }
    }

    /// LIGHTNESS has to reach the beds and the letters, and they do not take
    /// their L from the seed — the two ladder lifts are the only road there.
    /// The walls are the bake's own, so the file never saves a number that
    /// resolves to something else.
    #[test]
    fn lightness_moves_the_seed_and_both_ladders_and_stops_at_the_bakes_walls() {
        let e = tone_edits(Scope::Theme, &seeds(), Tone { light: 0.03, ..Tone::NEUTRAL });
        assert_eq!(value_of(&e, "surface.lift").unwrap(), "0.0300");
        assert_eq!(value_of(&e, "text.lift").unwrap(), "0.0300");
        assert!(value_of(&e, "palette.accent").unwrap().starts_with("oklch(0.8500"));
        // Past the wall the two ladders part company, because the master
        // gives them different walls (bake.rs:519 and :525).
        let far = tone_edits(Scope::Theme, &seeds(), Tone { light: 0.5, ..Tone::NEUTRAL });
        assert_eq!(value_of(&far, "surface.lift").unwrap(), "0.0900");
        assert_eq!(value_of(&far, "text.lift").unwrap(), "0.1000");
        // and L is a real quantity: it stops at white, not past it.
        assert!(value_of(&far, "palette.accent").unwrap().starts_with("oklch(1.0000"));
        let down = tone_edits(Scope::Theme, &seeds(), Tone { light: -0.5, ..Tone::NEUTRAL });
        assert_eq!(value_of(&down, "surface.lift").unwrap(), "-0.0900");
        assert_eq!(value_of(&down, "text.lift").unwrap(), "-0.1000");
        // And the two walls are the BAKE'S, taken and not copied: the
        // literals above are what those constants read today, and if one
        // moves this test says so instead of the file quietly resolving to
        // a number the editor never showed.
        assert_eq!(
            value_of(&far, "surface.lift").unwrap(),
            format!("{:.4}", crate::theme::bake::SURFACE_LIFT_WALL)
        );
        assert_eq!(
            value_of(&far, "text.lift").unwrap(),
            format!("{:.4}", crate::theme::bake::TEXT_LIFT_WALL)
        );
    }

    /// The weld is conditional, and the condition is the promise: only a HUE
    /// move claims one hue for the interface, so only a HUE move re-points
    /// `surface.hue` at the accent. Dragging SATURATION over a theme that
    /// cut its beds loose must leave them loose.
    #[test]
    fn only_a_hue_move_re_welds_the_surface_hue() {
        let s = seeds();
        for tone in [
            Tone::NEUTRAL,
            Tone { sat: 1.5, ..Tone::NEUTRAL },
            Tone { light: 0.04, ..Tone::NEUTRAL },
        ] {
            assert!(
                value_of(&tone_edits(Scope::Theme, &s, tone), "surface.hue").is_none(),
                "a move that is not a hue move re-welded the surface hue: {tone:?}"
            );
        }
        let turned = tone_edits(Scope::Theme, &s, Tone { hue_deg: -1.0, ..Tone::NEUTRAL });
        assert_eq!(
            value_of(&turned, "surface.hue").unwrap(),
            "@hue.accent",
            "the weld must be a REFERENCE — a number would cut the beds loose again on the next drag"
        );
    }

    /// BASIC and ADVANCED do not eat each other's work. Leaving BASIC folds
    /// the move into the seeds and re-opens at NEUTRAL: same file, sliders
    /// back at rest. The fold uses the same clamps as the writes, so a move
    /// that hit a wall re-opens where it actually landed.
    #[test]
    fn leaving_basic_and_coming_back_writes_the_same_theme() {
        let s = seeds();
        for tone in [
            Tone { hue_deg: 40.0, sat: 1.25, light: 0.02 },
            Tone { hue_deg: -200.0, sat: 0.4, light: -0.5 }, // past both walls
            Tone::NEUTRAL,
        ] {
            let once = tone_edits(Scope::Theme, &s, tone);
            let rebased = tone_edits(Scope::Theme, &s.shifted(tone), Tone::NEUTRAL);
            let strip = |v: Vec<Edit>| -> Vec<Edit> {
                v.into_iter().filter(|e| e.token != "surface.hue").collect()
            };
            assert_eq!(
                strip(once),
                strip(rebased),
                "a trip through ADVANCED changed the theme: {tone:?}"
            );
        }
        // Two BASIC moves in a row are one move: relative composes.
        let a = Tone { hue_deg: 30.0, sat: 1.2, light: 0.01 };
        let b = Tone { hue_deg: 45.0, sat: 1.5, light: 0.02 };
        let both = Tone { hue_deg: 75.0, sat: 1.8, light: 0.03 };
        let stepwise = s.shifted(a).shifted(b);
        let direct = s.shifted(both);
        assert!((stepwise.accent.h - direct.accent.h).abs() < 0.05);
        assert!((stepwise.accent.c - direct.accent.c).abs() < 1e-4);
        assert!((stepwise.accent.l - direct.accent.l).abs() < 1e-4);
    }

    /// The notch is the smallest move that can change one output code, so a
    /// deeper swapchain gets a finer one and nothing else about the sliders
    /// changes.
    #[test]
    fn the_notch_narrows_with_the_swapchains_depth() {
        let chroma = 0.13;
        let eight = tone_step(8, chroma);
        let ten = tone_step(10, chroma);
        let sixteen = tone_step(16, chroma);
        assert!((eight.light - 1.0 / 255.0).abs() < 1e-6);
        assert!((ten.light - 1.0 / 1023.0).abs() < 1e-6);
        assert!(sixteen.light < ten.light && ten.light < eight.light);
        assert!(sixteen.hue_deg < ten.hue_deg && ten.hue_deg < eight.hue_deg);
        assert!(sixteen.sat < ten.sat && ten.sat < eight.sat);
        // 8 bits on a mint seed: a shade under two degrees a notch, three
        // percent of chroma, one 255th of L. Numbers a hand can feel.
        assert!((eight.hue_deg - 1.728).abs() < 0.01, "{}", eight.hue_deg);
        assert!((eight.sat - 0.0302).abs() < 1e-3, "{}", eight.sat);
        // The depth nobody set is the floor every swapchain supports, and
        // it is the CONFIG that answers with it — so the two are the same
        // question here: the default IS eight bits.
        assert_eq!(tone_step(DEFAULT_DEPTH_BITS, chroma), eight);
        // A depth past the widest chip, or below one bit, is held rather
        // than allowed to eat the shift that makes `q`.
        assert_eq!(tone_step(64, chroma), tone_step(16, chroma));
        assert_eq!(tone_step(0, chroma), tone_step(1, chroma));
        // A grey seed cannot be turned however far the slider goes, so the
        // guard settles the two derived notches at one code's worth rather
        // than letting them run away: one radian and a full multiplier.
        let grey = tone_step(8, 0.0);
        assert!((grey.sat - 1.0).abs() < 1e-5, "{}", grey.sat);
        assert!((grey.hue_deg - 57.2958).abs() < 0.01, "{}", grey.hue_deg);
        // And a snapped move lands on the notch, so the file never carries
        // a number finer than the screen can show.
        let snapped = Tone { hue_deg: 5.0, sat: 1.111, light: 0.007 }.snapped(&eight);
        assert!((snapped.hue_deg / eight.hue_deg).fract().abs() < 1e-3, "{snapped:?}");
        assert!(Tone::NEUTRAL.is_neutral() && !snapped.is_neutral());
    }
}
