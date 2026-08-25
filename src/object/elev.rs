//! One SURFACE LEVEL, drawn in one place.
//!
//! `[elev.backdrop]` … `[elev.fixture]` is the master's ladder of
//! surfaces (§5.12). Every rung states the same dictionary — a body
//! (`fill`), a shape (`corner`, `radius`), a ring (`edge.color`,
//! `edge.width`, and the two-stop pair `edge.mode` / `edge.color2` /
//! `edge.axis`), and beyond them the glass pair, the two glows, the
//! drop shadow and the reflection. An object that assembles its rung out
//! of primitives at its own call site therefore owns a PRIVATE COPY of
//! those rules, and the copies drift. They did: `panel.rs` read the fill
//! as a bed and guarded it on alpha where `window.rs` did neither, and
//! `window.rs` went on stroking a FLAT ring for a year after the rung
//! grew a second colour — whichever copy the next level's author read
//! was the one that level came to resemble.
//!
//! [`Level`] is the one reader, and as of 2026-08-17 there are no
//! others: the panel, the window frame, the menu and the tooltip all
//! name a rung and take what it says. A consumer names it once — `"elev.
//! popover"` — and gets the whole dictionary, so when the glass ranks
//! and the shadow 9-slice land they land for every rung at once instead
//! of for whichever object was being edited that week.
//!
//! What is NOT here is any decision: no fallback colour, no minimum
//! radius, no "if the theme says nothing draw a hairline". A rung whose
//! `fill` is `none` and whose `edge.width` is `0` draws nothing, which
//! is the raw look the governing principle asks for.

use crate::corner::Cuts;
use crate::draw::Corner;
use crate::theme::{self, Color, TokenId};
use crate::{Ctx, Rect};
use std::sync::OnceLock;

/// The engine's colour, in the draw list's clothes.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// The keys of one `[elev.*]` rung, resolved to ids once.
///
/// Ids and enum vocabularies are both stable for the life of the process
/// — [`theme::id`] says so of the first, and the second is interned out
/// of the master, which no user theme replaces — so a `Level` is built
/// inside a `OnceLock` at the call site exactly like the bare `TokenId`
/// statics everywhere else. What is NOT cached is any resolved VALUE:
/// the colour, the radius and the corner word are read from the live
/// [`theme::ResolvedTheme`] on every draw, so a theme swap moves them.
#[derive(Clone, Copy)]
pub(crate) struct Level {
    fill: TokenId,
    corner: TokenId,
    radius: TokenId,
    edge_color: TokenId,
    edge_color2: TokenId,
    edge_mode: TokenId,
    edge_axis: TokenId,
    edge_width: TokenId,
    glass_rank: TokenId,
    glass_tint: TokenId,
    glass_wash: TokenId,
    /// Where each cut's word sits in `corner`'s vocabulary — see
    /// [`crate::corner::Cuts`], which is the one reader of it.
    words: Cuts,
    /// The four `shape.<preset>.corners_tl/tr/br/bl` PAIRS a preset gets
    /// the last word with, as `[style, length]` ids in `ring_points`'
    /// order, or `None` for a rung no preset was pointed at
    /// ([`Level::shaped_by`]).
    per: Option<[TokenId; 8]>,
    /// `edge.mode`'s index for the word `gradient`.
    mode_gradient: Option<u16>,
    /// `edge.axis`'s indices for `x, y, diag_down, diag_up`, in that
    /// order, against [`AXES`].
    axis_words: [Option<u16>; 4],
}

/// What each word of `edge.axis` means as a direction, y DOWN — the same
/// screen space [`crate::draw::DrawList::rect_grad`] projects in.
///
/// Definitions of the four words, not lengths or colours: `diag_down`
/// travels down as it travels right, which is what the word says. The
/// vectors are not normalised because the ring normalises `t` against the
/// box's own extent, so only the DIRECTION is read here.
const AXES: [(&str, [f32; 2]); 4] =
    [("x", [1.0, 0.0]), ("y", [0.0, 1.0]), ("diag_down", [1.0, 1.0]), ("diag_up", [1.0, -1.0])];

impl Level {
    /// The rung named by `prefix`, e.g. `"elev.popover"`.
    ///
    /// A name and not a `TokenId` because a rung is a DICTIONARY: the
    /// caller states which surface it is, once, and the five keys under
    /// it are this module's business. A key the master does not declare
    /// degrades through [`TokenId::MISSING`], which is the engine's raw
    /// look and not a design.
    pub(crate) fn of(prefix: &str) -> Level {
        let id = |key: &str| theme::id(&format!("{prefix}.{key}")).unwrap_or(TokenId::MISSING);
        let corner = id("corner");
        let edge_mode = id("edge.mode");
        let edge_axis = id("edge.axis");
        Level {
            fill: id("fill"),
            corner,
            radius: id("radius"),
            edge_color: id("edge.color"),
            edge_color2: id("edge.color2"),
            edge_mode,
            edge_axis,
            edge_width: id("edge.width"),
            glass_rank: id("glass.rank"),
            glass_tint: id("glass.tint"),
            glass_wash: id("glass.wash"),
            words: Cuts::of(corner),
            per: None,
            mode_gradient: theme::enum_index(edge_mode, "gradient"),
            axis_words: [
                theme::enum_index(edge_axis, AXES[0].0),
                theme::enum_index(edge_axis, AXES[1].0),
                theme::enum_index(edge_axis, AXES[2].0),
                theme::enum_index(edge_axis, AXES[3].0),
            ],
        }
    }

    /// The COMPONENT this rung is worn by, where its keys are older than
    /// the ladder's.
    ///
    /// `elev::Level` is the one place a surface is drawn, but the two
    /// oldest floating surfaces in the toolkit — the menu and the tooltip
    /// — were written before the ladder existed and the master still
    /// spells their body, ring and cut under their own names
    /// (`component.menu.fill`, `[menu].corner` for a RADIUS where the
    /// ladder's `corner` is a CUT MODE). Renaming those keys would break
    /// every theme and every embedder that names them; keeping a private
    /// copy of the drawing rules is what this module exists to stop. So
    /// the object states which of its own tokens stand in for which of the
    /// rung's, once, here — and everything it does NOT name (the glass
    /// pair, and every key the ladder grows after this line) comes from
    /// the rung, which is what "participates in the elevation hierarchy"
    /// means.
    ///
    /// The ring's SECOND colour, its mode and its axis stay on the rung on
    /// purpose: a two-stop edge is a property of the surface class, the
    /// component names only the ring it already had, and a component key
    /// that does not exist would resolve to `MISSING` — whose colour is
    /// the engine's raw ink, which is not what "the theme said nothing"
    /// should paint.
    ///
    /// The window frame (`window.rs`, 2026-08-17) is the third caller and
    /// its reason is not age but a SEAM: `component.panel.fill` is the one
    /// token a window and a panel share, because the master derives
    /// `[elev.panel] fill` from it. A frame that stopped naming it and read
    /// the rung's own `fill` instead would sever that derivation, and the
    /// theme editor's background — which writes the shared token on
    /// purpose — would stop colouring windows.
    pub(crate) fn worn_as(
        mut self,
        fill: &str,
        corner: &str,
        radius: &str,
        edge_color: &str,
        edge_width: &str,
    ) -> Level {
        let id = |name: &str| theme::id(name).unwrap_or(TokenId::MISSING);
        self.fill = id(fill);
        self.corner = id(corner);
        self.radius = id(radius);
        self.edge_color = id(edge_color);
        self.edge_width = id(edge_width);
        self.words = Cuts::of(self.corner);
        self
    }

    /// Override JUST the ring's COLOUR token, leaving the fill, the shape
    /// and the width on the rung.
    ///
    /// ONE MODEL OF A WINDOW (rule 11): a widget's own panel and the
    /// settings window frame are the same surface, so a widget's ring must
    /// read the SAME border the frame does — `component.panel.border`, the
    /// shared root the theme editor writes — and not the rung's raw
    /// `elev.panel.edge.color`. The two are one value on a clean theme
    /// (`elev.panel.edge.color = @component.panel.border`), but an older
    /// border-colour save PINNED the leaf to a literal, and after it a
    /// widget's ring sat on a colour the window — reading the root — had
    /// already left. This is the narrow half of `worn_as`: only the colour
    /// moves, because only the colour ever diverged.
    pub(crate) fn with_edge_color(mut self, token: &str) -> Level {
        self.edge_color = theme::id(token).unwrap_or(TokenId::MISSING);
        self
    }

    /// The `shape.*` preset that gets the LAST WORD on this rung's four
    /// corners, one corner at a time (f3 K6).
    ///
    /// **This is where `shape.<preset>.corners_tl/tr/br/bl` reach the
    /// screen.** Sixteen presets have carried the four keys since the
    /// theme engine was written, each with a comment saying it overrides
    /// one corner; [`crate::view::paint::preset`] gave them a reader, and
    /// a reader nobody calls changes no picture — the key was still dead
    /// where it counts. A rung named here is drawn through them, so a
    /// theme writing `shape.panel.corners_tl = [ chamfer, 2u ]` cuts one
    /// corner of every window frame and leaves the other three where they
    /// were.
    ///
    /// It is stated by the CONSUMER, next to `worn_as`, and for the same
    /// reason: which preset is the same surface as which rung is a fact
    /// about the theme's vocabulary, not one the ladder can derive from a
    /// rung's name — `shape.window` exists as well, and the frame is
    /// `[elev.panel]` wearing `shape.panel`. Deriving it here would make
    /// that a coincidence of spelling.
    ///
    /// The rung's own `corner` / `radius` stay the BASE all four start
    /// from, so this ADDS a say rather than moving one. Each per-corner
    /// key is a PAIR whose two slots inherit separately, so a slot left
    /// at `same_as_parent` — which is every slot the master ships but
    /// `button_alt`'s and `tab`'s — answers exactly the corner that
    /// arrived, and the shipped picture is bit for bit what it was.
    ///
    /// A preset that declares no such keys keeps the base on all four and
    /// says so once: reading a token that is not there gives zero, and
    /// zero is a square corner nobody asked for.
    pub(crate) fn shaped_by(mut self, preset: &str) -> Level {
        let mut ids = [TokenId::MISSING; 8];
        for (i, slot) in ["tl", "tr", "br", "bl"].iter().enumerate() {
            for (j, part) in ["[0]", "[1]"].iter().enumerate() {
                ids[2 * i + j] = theme::id(&format!("{preset}.corners_{slot}{part}"))
                    .unwrap_or(TokenId::MISSING);
            }
        }
        if ids.iter().any(|id| *id == TokenId::MISSING) {
            crate::ui::warn_once(
                &format!("shaped_by:{preset}"),
                &format!(
                    "\"{preset}\" declares no corners_tl/tr/br/bl pair: its corners \
                     cannot be set one at a time"
                ),
            );
            return self;
        }
        self.per = Some(ids);
        self
    }

    /// The far end of a two-stop ring and the direction it travels, or
    /// `None` for the flat ring every rung draws by default.
    ///
    /// Three tokens have to agree, and the master says so at each of them:
    /// `edge.mode` must be the word `gradient`; `edge.color2` must hold a
    /// COLOUR (its default `same_as_parent` is a §5.0 sentinel, which bakes
    /// to a negative scalar and means "copy edge.color", i.e. a flat ring);
    /// and `edge.axis` must be one of [`AXES`]' four words. A direction the
    /// vocabulary does not name is not a direction, so the ring stays flat
    /// rather than being drawn along a guess — the same degradation
    /// [`Cuts::read`] applies to a corner word.
    ///
    /// What is NOT read is `edge.gradient`, the NAMED multi-stop slot
    /// (`@grad.<name>`), and TWO doors are shut in front of it rather than
    /// the one this comment named until 2026-08-17. Both were measured,
    /// and [`tests::the_named_gradient_has_two_doors_shut_and_this_is_which`]
    /// holds the measurement down:
    ///
    /// * THE NAME HAS NOTHING TO POINT AT. `@grad.spectrum` is §3.2's
    ///   reference production, which resolves a TOKEN — and `grad.spectrum`
    ///   is a SECTION. `theme::id` answers `None` for it, so the value a
    ///   theme would write here cannot be resolved at all, let alone read.
    ///   Naming a section needs either a language production or a per-rung
    ///   `enum:` of the gradients the file declares.
    /// * AND THE STOPS ARE NOT BAKED. `[grad]`'s `<name>.stops` is an array
    ///   of `[position, colour]` pairs; `cascade.rs` declares each PAIR as a
    ///   token, and `bake.rs`'s `Value::Array(_) => {}` drops the pair — so
    ///   `grad.spectrum.stops[1]`, whose position the master writes as 0.34,
    ///   bakes to a scalar of 0 and an unwritten colour. `ResolvedTheme` has
    ///   no place to hold a stop list and no accessor to answer one with.
    ///
    /// Opening either alone buys nothing, which is why this is a
    /// theme-engine job and not a reader's. Until then the sugar pair is the
    /// whole of what a theme can ask for here, which is what the master's
    /// own comment calls "the color/color2 pair".
    fn edge_gradient(&self, t: &theme::ResolvedTheme) -> Option<(Color, [f32; 2])> {
        if self.mode_gradient? != t.enum_of(self.edge_mode) {
            return None;
        }
        // A word in the colour slot, not a colour: §5.0's sentinels fold to
        // a negative scalar and leave the colour empty.
        if t.px(self.edge_color2) < 0.0 {
            return None;
        }
        let axis = t.enum_of(self.edge_axis);
        let i = self.axis_words.iter().position(|w| *w == Some(axis))?;
        Some((col(t.color(self.edge_color2)), AXES[i].1))
    }

    /// The cut this rung makes on the box `r`, and the tessellation of
    /// its arcs.
    ///
    /// Through [`Corner::sized`] and not a clamp: §5.0's `pill` is a
    /// word about a box ("as round as this one can be") and bakes to a
    /// negative sentinel, so a floor at zero answers a master writing
    /// `pill` with the square it wrote to avoid.
    ///
    /// Four corners and not one repeated: [`Level::shaped_by`] may have
    /// given a `shape.*` preset the last word on each of them
    /// separately. The count comes from the biggest ARC on the ring
    /// ([`super::window::round_reach`]) rather than from the base,
    /// because one count serves all four and reading it off the base
    /// alone would under-tessellate a corner the preset made rounder.
    pub(crate) fn cut(&self, t: &theme::ResolvedTheme, r: Rect) -> ([Corner; 4], u8) {
        static SEGMENTS: OnceLock<TokenId> = OnceLock::new();
        let style = self.words.read(t, self.corner);
        let c = self.per_corner(t, Corner::sized(style, t.px(self.radius), r), r);
        (c, super::window::corner_segments(t, &SEGMENTS, super::window::round_reach(&c)))
    }

    /// `base` with each corner passed under the preset's own say, if a
    /// preset was named ([`Level::shaped_by`]) and `base` unchanged four
    /// times if none was.
    ///
    /// The RULE — what a half-stated pair means — is
    /// [`crate::view::paint::override_corner`] and is not repeated here:
    /// the surface layer reads the same four keys for anything drawing
    /// through the plugin ABI, and two answers to one question is the
    /// drift every shared reader in this crate was pulled out to end.
    /// What is local is only HOW the readings are taken, which on this
    /// side is a memoised token and a borrowed word rather than a string
    /// key and an allocation.
    ///
    /// The WORD is asked for only once the style slot's scalar says a
    /// style was stated at all — a sentinel bakes to its own negative
    /// whatever kind of slot it sits in, so the question can be put to
    /// the number first. That is what keeps the master's own picture, in
    /// which all thirty-two slots inherit, free of a vocabulary lookup
    /// per corner per frame. It is compared as a WORD and not as an enum
    /// index because a preset's style slot carries no `enum:` list in the
    /// master: its word table grows out of the values a theme actually
    /// loaded, and an index memoised against the master's own table would
    /// name someone else's word after a swap.
    ///
    /// The word comes out of the PUBLISHED vocabulary, which is the only
    /// place words are kept — a `ResolvedTheme` holds the index and the
    /// engine's schema holds what the index is called. So `t` decides
    /// every NUMBER here and the schema decides every NAME, and the two
    /// are one theme in every drawing path there is. They part only for
    /// a theme baked and never published, which is a test's arrangement
    /// and not a program's.
    fn per_corner(&self, t: &theme::ResolvedTheme, base: Corner, r: Rect) -> [Corner; 4] {
        let Some(ids) = self.per else { return [base; 4] };
        let inherit = crate::view::paint::inherits();
        let mut out = [base; 4];
        for (i, corner) in out.iter_mut().enumerate() {
            let (word, len) = (ids[2 * i], ids[2 * i + 1]);
            let (scalar, stated) = (t.px(word), t.px(len));
            *corner = if scalar == inherit {
                crate::view::paint::override_corner(base, r, scalar, "", stated)
            } else {
                crate::ui::with_theme_word(word, |w| {
                    crate::view::paint::override_corner(base, r, scalar, w, stated)
                })
            };
        }
        out
    }

    /// Material, ring, and family A's bloom over the ring.
    ///
    /// Answers the shape it drew, because a caller that has to fit
    /// content INSIDE the rung — a drop-down's rows, a panel's content
    /// box — would otherwise settle the same cut a second time and be
    /// free to settle it differently.
    pub(crate) fn draw(&self, ctx: &mut Ctx, r: Rect) -> ([Corner; 4], u8) {
        self.draw_in(ctx.dl, theme::resolved(), r, r, ctx.t)
    }

    /// [`Level::draw`] with the GLASS quad laid on a rectangle of its
    /// own, which is what `panel.glass.rect` and `panel.glass.inset` ask
    /// for: a frosted panel whose blur stops at the content box, so the
    /// title band stands on the bed and only the body is glass.
    ///
    /// The ring, the bloom and the cut are still the rung's and still
    /// belong to `r` — a surface has one outline whatever is poured
    /// inside it.
    pub(crate) fn draw_glassed(
        &self,
        ctx: &mut Ctx,
        r: Rect,
        glass: Rect,
    ) -> ([Corner; 4], u8) {
        self.draw_in(ctx.dl, theme::resolved(), r, glass, ctx.t)
    }

    /// [`Level::draw`] with the theme and the clock in hand and no frame
    /// around it.
    ///
    /// A rung touches nothing of a `Ctx` but its draw list and its clock,
    /// and taking the theme as an argument is what lets one rung be drawn
    /// from a theme that is not the published one — which is how the
    /// picture this rung makes is put under test at all, gradient ring
    /// included, without a test reaching into the process-wide theme every
    /// other test is reading at the same time.
    ///
    /// `now` is `Ctx::t`, seconds since application start, and it exists
    /// here for one reader: the edge bloom breathes on `motion.glow_pulse`
    /// and a cyclic effect has to be told what time it is. A caller with no
    /// frame around it — every test below — passes a time of its own
    /// choosing, which is the only way a pulse can be sampled at a stated
    /// phase instead of at whenever the suite happened to run.
    pub(crate) fn draw_in(
        &self,
        dl: &mut crate::draw::DrawList,
        t: &theme::ResolvedTheme,
        r: Rect,
        glass: Rect,
        now: f64,
    ) -> ([Corner; 4], u8) {
        let (c, seg) = self.cut(t, r);
        // Glass INSTEAD of the fill, never on top of it — the master's own
        // contract at the `fill` key ("used INSTEAD of the glass pair while
        // rank = 0") and at the ladder's head: glass is TWO quads, the tint
        // that multiplies the blurred scene (it can only darken) and the
        // wash that lays over with alpha (the only knob that brightens).
        // This is the rank's FIRST reader: until 2026-08-16 the token was
        // declared on every rung and read by nobody, so a theme asking for
        // glass on a panel got a flat fill and no word about it.
        let rank = t.px(self.glass_rank).clamp(0.0, 3.0);
        if rank > 0.0 {
            // The glass box carries its OWN cut: a quad pulled inside the
            // border wearing the border's radius is a rounded rectangle
            // drawn at the wrong size, and the gap shows at every corner.
            let (gc, gseg) = self.cut(t, glass);
            dl.glass_fill(glass, &gc, gseg, rank, col(t.color(self.glass_tint)));
            let wash = col(t.color(self.glass_wash));
            if wash.a > 0.0 {
                dl.ring_fill(glass, &gc, gseg, wash);
            }
        } else {
            let fill = col(t.bed(self.fill));
            if fill.a > 0.0 {
                dl.ring_fill(r, &c, seg, fill);
            }
        }
        let edge = col(t.color(self.edge_color));
        let width = t.px(self.edge_width).max(0.0);
        if edge.a > 0.0 && width > 0.0 {
            // Until 2026-08-17 this read `edge.color` and nothing else, so
            // a theme that wrote `edge.mode = gradient` beside a second
            // colour got a flat ring and no word about it — the master
            // declares the pair at every one of the nine rungs.
            match self.edge_gradient(t) {
                Some((far, dir)) => dl.ring_grad(r, &c, seg, width, edge, far, dir),
                None => dl.ring(r, &c, seg, width, edge),
            }
            // The bloom keeps taking the ring's OWN colour, the near end:
            // `glow_ring` is one additive sprite ring with one vertex
            // colour, so a gradient halo is not a thing this call can carry
            // and inventing a midpoint here would be a decision made in
            // Rust. Its ALPHA breathes on `motion.glow_pulse`, which is
            // what the clock is for; a two-colour ring and a breathing
            // bloom are orthogonal — the gradient decides the ring's two
            // ends, the pulse decides how brightly the halo over it is
            // laid, and neither reads the other.
            super::window::panel_edge_glow(dl, t, r, &c, seg, edge, width, now);
        }
        (c, seg)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::draw::{DrawCmd, DrawList};

    /// The clock every proof in and under this module draws at.
    ///
    /// A stated instant, not "whenever the suite ran": `draw_in`'s only
    /// reader of the clock is the edge bloom's breath on
    /// `motion.glow_pulse`, and a picture compared against another picture
    /// has to be taken at the same phase as it. The master ships
    /// `glow.panel_edge.enabled = true` (2026-08-23), so the pulse IS
    /// sampled on every draw now — which is exactly why the instant must
    /// be written down and held equal on both sides of a comparison,
    /// rather than left to whatever moment the suite happens to run at.
    pub(crate) const AT_REST: f64 = 0.0;

    /// BORDER SIZE MOVES EVERY PANEL'S RING, END TO END — the theme
    /// editor's own row through `set_preview`, through `panel.border`,
    /// through `elev.panel.edge.width`, to the width `draw_in` actually
    /// reads. Missing before 2026-08-25: `panel.border` named
    /// `@stroke.hair` directly, skipping `border.edge.width` — the token
    /// `border_width_edit`'s own doc already claimed the chain ran
    /// through, and nothing caught the gap because BOTH resolve to
    /// `stroke.hair` at every theme's own default, so an untouched
    /// panel's ring never moved either way. A live preview is the one
    /// thing that tells the two apart.
    #[test]
    fn border_size_moves_every_panel_s_ring_through_the_live_preview() {
        let _g = crate::theme::preview_test_lock();
        let before = {
            let t = crate::theme::resolved();
            t.px(t.id("elev.panel.edge.width").unwrap())
        };
        assert!(crate::theme::set_preview(&[("border.edge.width", "0.90u")]).is_empty());
        let after = {
            let t = crate::theme::resolved();
            t.px(t.id("elev.panel.edge.width").unwrap())
        };
        crate::theme::clear_preview();
        assert!(
            after > before + 1.0,
            "BORDER SIZE moved border.edge.width to 0.90u but elev.panel.edge.width \
             stayed at {before}px ({after}px after) — the chain to `panel.border` broke again"
        );
    }

    // ------------------------------------------- the no-move proof
    //
    // Shared with `menu.rs` and `tooltip.rs`, whose claim is not about a
    // gradient at all: that JOINING the ladder moved no pixel. Written
    // once, here, because two copies of what counts as proof is the same
    // mistake in the test suite that this module exists to undo in the
    // drawing code.

    /// What an object drew before it joined the ladder — the `ring_fill`
    /// + `ring` pair from its own five tokens, transcribed from the
    /// private copies `menu.rs` and `tooltip.rs` carried until
    /// 2026-08-17. It is a TRANSCRIPT, so it keeps their two departures
    /// from the rung: the body is drawn whatever its alpha, and the ring
    /// is drawn on the width alone.
    ///
    /// Those two and no others, which is why `window.rs` — the third
    /// object to join the ladder on the same day — does NOT use this and
    /// keeps a transcript of its own. Its private copy also stroked the
    /// ring whatever the edge's ALPHA and laid the edge bloom
    /// unconditionally, and a transcript that quietly dropped two of the
    /// four things an object used to do would prove the no-move claim
    /// about a picture nobody ever drew.
    ///
    /// A THIRD THING JOINED THE FOUR ON 2026-08-23, and it is not a
    /// departure: `panel_edge_glow` is no longer a per-object decision
    /// menu and tooltip's own old code could have carried or dropped — it
    /// is the master's, unconditional, on every rung — so the transcript
    /// calls it exactly as the ladder does, on the ring's own gate
    /// (width alone, matching the departure above), rather than leaving
    /// it out and proving the no-move claim about a picture the ladder
    /// does not draw either.
    pub(crate) fn the_private_copy(
        dl: &mut DrawList,
        t: &theme::ResolvedTheme,
        r: Rect,
        fill: &str,
        corner_mode: &str,
        radius: &str,
        edge: &str,
        width: &str,
    ) {
        static SEG: OnceLock<TokenId> = OnceLock::new();
        let id = |n: &str| theme::id(n).unwrap_or(TokenId::MISSING);
        let mode = id(corner_mode);
        let style = Cuts::of(mode).read(t, mode);
        let c = [Corner::sized(style, t.px(id(radius)), r); 4];
        let seg = super::super::window::corner_segments(t, &SEG, c[0].size);
        dl.ring_fill(r, &c, seg, col(t.bed(id(fill))));
        let bw = t.px(id(width)).max(0.0);
        if bw > 0.0 {
            let edge_col = col(t.color(id(edge)));
            dl.ring(r, &c, seg, bw, edge_col);
            super::super::window::panel_edge_glow(dl, t, r, &c, seg, edge_col, bw, AT_REST);
        }
    }

    /// A WIDGET'S PANEL WEARS THE SHARED BORDER ROOT, like the window frame.
    ///
    /// `panel::draw`'s rung passes `component.panel.border` through
    /// [`Level::with_edge_color`], so a widget's ring reads the same token
    /// the settings window frame wears — not the rung's own
    /// `elev.panel.edge.color`, which an older border-colour save could pin
    /// to a literal, stranding the widget on a colour the window has left.
    /// The two are one value on a clean theme; the point is that they need
    /// not be, and the widget follows the root either way. Only the colour
    /// moves — fill, shape and width stay the rung's.
    #[test]
    fn a_widget_panel_wears_the_shared_border_root() {
        theme::resolved();
        let base = Level::of("elev.panel");
        let worn = base.with_edge_color("component.panel.border");
        assert_eq!(
            worn.edge_color,
            theme::id("component.panel.border").unwrap(),
            "the widget ring must read the root the window frame wears"
        );
        assert_eq!(
            base.edge_color,
            theme::id("elev.panel.edge.color").unwrap(),
            "the rung's own leaf is what an old save could pin to a literal"
        );
        assert_ne!(worn.edge_color, base.edge_color, "the ring colour did not move to the root");
        assert_eq!(worn.fill, base.fill, "with_edge_color moved the fill");
        assert_eq!(worn.corner, base.corner, "with_edge_color moved the corner");
        assert_eq!(worn.edge_width, base.edge_width, "with_edge_color moved the width");
    }

    /// Two lists that are the same picture, checked the way the frame
    /// guard checks one: the command register AND the vertices under it.
    /// The register alone would miss a colour the commands agree on and
    /// the geometry does not; the vertices alone would miss a command
    /// that emitted none.
    pub(crate) fn same_picture(was: &DrawList, now: &DrawList) {
        let dump = |dl: &DrawList| {
            dl.cmds().iter().map(|c| c.to_string()).collect::<Vec<_>>().join("\n")
        };
        assert_eq!(dump(was), dump(now));
        let verts = |dl: &DrawList| {
            dl.verts.iter().map(|v| (v.pos, v.uv, v.color)).collect::<Vec<_>>()
        };
        assert_eq!(verts(was).len(), verts(now).len(), "the vertex count moved");
        assert_eq!(verts(was), verts(now));
    }

    /// The rung every popover wears, undressed — the ladder's own key
    /// spellings, which is what a gradient is written against.
    fn popover() -> Level {
        Level::of("elev.popover")
    }

    fn box_() -> Rect {
        Rect::new(20.0, 12.0, 160.0, 40.0)
    }

    /// The rung's OWN ring, whichever kind it is — the first one drawn.
    ///
    /// Since 2026-08-23 the master ships `panel_edge` lit, so `draw_in`
    /// may append a second, plain `Ring` after this one: its own burned
    /// core (`window.rs`'s `panel_edge_glow`, called strictly after
    /// `dl.ring`/`dl.ring_grad` in `draw_in` above — the order is the
    /// call order, not a guess about paint order). That second ring is
    /// glow's business, not this rung's, so it is not this function's.
    fn ring_cmd(dl: &DrawList) -> DrawCmd {
        let rings: Vec<_> = dl
            .cmds()
            .iter()
            .filter(|c| matches!(c, DrawCmd::Ring { .. } | DrawCmd::RingGrad { .. }))
            .cloned()
            .collect();
        assert!(!rings.is_empty(), "a rung stroked no ring at all");
        rings[0].clone()
    }

    /// An `[elev.popover]` override, with the two vocabularies restated.
    ///
    /// Restated because a re-declaration in the SAME stage replaces the
    /// token whole, `enum:` list included (`cascade.rs`'s `declare`), and
    /// an enum's baked value is an INDEX into that list — so an override
    /// that dropped the list would number its own single word 0 and mean
    /// something else than the same word means in the master.
    fn overlay(mode: &str, color2: &str, axis: &str) -> String {
        format!(
            "[elev.popover]\n\
             edge.mode = {mode}    # · enum: solid | gradient ·\n\
             edge.color2 = {color2}\n\
             edge.axis = {axis}    # · enum: x | y | diag_down | diag_up ·\n"
        )
    }

    /// A popover rung whose ring is `mode`/`color2`/`axis`, drawn once.
    fn ring_under(mode: &str, color2: &str, axis: &str) -> DrawCmd {
        let t = theme::bake_over_master(&overlay(mode, color2, axis));
        let mut dl = DrawList::recording();
        popover().draw_in(&mut dl, &t, box_(), box_(), AT_REST);
        ring_cmd(&dl)
    }

    /// **A frosted rung is one record, drawn by the rung itself.**
    ///
    /// `draw.rs` proves the weld on the three calls in isolation; this
    /// proves that the three calls `Level::draw_in` actually makes are
    /// those three, in that order, with nothing in between — which is a
    /// property of THIS file and can drift out of true here without a
    /// line of the toolkit's drawing code changing. It is also the
    /// shape of the defect K3b was gated on: the theme editor's
    /// BACKGROUND section writes exactly these three keys, and FROSTED
    /// is what raises the rank above zero.
    #[test]
    fn a_frosted_rung_welds_its_wash_and_its_ring_into_one_record() {
        let t = theme::bake_over_master(
            "[elev.popover]\n\
             glass.rank = 2\n\
             glass.tint = #88AACC / 0.6\n\
             glass.wash = #102030 / 0.25\n",
        );
        let mut dl = DrawList::new();
        dl.set_vector(true);
        popover().draw_in(&mut dl, &t, box_(), box_(), AT_REST);
        // Since 2026-08-23 the master ships `panel_edge` lit, so
        // `panel_edge_glow` (called after the weld, in `draw_in` above)
        // appends its own core-ring and glow-band shapes after this rung's
        // one welded record — this test is about the WELD, so it looks at
        // `shapes()[0]` and no longer claims that is the only shape.
        assert!(dl.shape_len() >= 1, "the rung wrote no silhouette at all");
        let rec = dl.shapes()[0];
        use crate::draw::Shape;
        assert_eq!(rec.flags & Shape::FILL, Shape::FILL, "the wash did not weld");
        assert_eq!(rec.flags & Shape::STROKE, Shape::STROKE, "the ring did not weld");
        assert_eq!(
            rec.tint,
            col(t.color(theme::id("elev.popover.glass.tint").unwrap())).to_array()
        );
        // The frost's core keeps the TINT and the quads above it carry
        // the wash: a weld that started one quad too early would have
        // washed the frost out of its own surface, and the rung would
        // still draw, still weld, still be one record.
        //
        // Sliced to `6..12`, not `6..`: the weld is 6 core verts and 6
        // wash verts, twelve in all, and glow (2026-08-23) appends its
        // own verts after them — verts this assertion is not about.
        let wash = col(t.color(theme::id("elev.popover.glass.wash").unwrap())).to_array();
        assert!(dl.verts[..6].iter().all(|v| v.color == rec.tint), "the core lost its tint");
        assert!(dl.verts[6..12].iter().all(|v| v.color == wash), "the wash did not land");
        // The core is still the tessellated glass lane; the band is the
        // field's. Both landed. (Until 2026-08-23 nothing here also
        // touched the plain shape lane; glow's own burned core does now,
        // legitimately, so that lane's presence no longer says anything
        // about the WELD and is not asserted against.)
        let lanes: Vec<_> = dl.runs.iter().filter_map(|r| r.image).collect();
        assert!(lanes.contains(&crate::draw::GLASS_RANK_2), "{lanes:?}");
        assert!(lanes.contains(&crate::draw::SHAPE_GLASS_2), "{lanes:?}");
        // And off the lane the rung draws what it always drew: fans,
        // no records at all.
        let mut old = DrawList::new();
        popover().draw_in(&mut old, &t, box_(), box_(), AT_REST);
        assert_eq!(old.shape_len(), 0);
        assert!(old.runs.iter().all(|r| r.image != Some(crate::draw::SHAPE_GLASS_2)));
    }

    /// The declaration this whole path stood on, and stood on badly until
    /// 2026-08-17: `edge.mode`'s vocabulary is the master's `enum:` list,
    /// not the words a theme happens to have used. Without the list the
    /// vocabulary grows from use, `solid` is the only word ever used, and
    /// `gradient` — the one word the key exists to carry — could never be
    /// delivered by any theme, so no reader here could have fired.
    #[test]
    fn the_master_owns_the_words_this_ring_is_switched_by() {
        for rung in ["elev.backdrop", "elev.board", "elev.panel", "elev.raised",
            "elev.focused", "elev.popover", "elev.inset", "elev.overlay", "elev.fixture"] {
            let mode = theme::id(&format!("{rung}.edge.mode")).unwrap();
            assert_eq!(theme::enum_index(mode, "solid"), Some(0), "{rung}");
            assert_eq!(theme::enum_index(mode, "gradient"), Some(1), "{rung}");
            let axis = theme::id(&format!("{rung}.edge.axis")).unwrap();
            for (i, (word, _)) in AXES.iter().enumerate() {
                assert_eq!(theme::enum_index(axis, word), Some(i as u16), "{rung} {word}");
            }
        }
    }

    /// A TRIPWIRE, not a wish: the two doors [`Level::edge_gradient`]
    /// names in front of `edge.gradient` are measured here, so the day
    /// either opens this test fails and the reader that owes the master a
    /// multi-stop ring is written instead of forgotten.
    ///
    /// `edge.gradient` is the last of Z16's four keys still without a
    /// reader — the other three (`mode`, `color2`, `axis`) gained one on
    /// 2026-08-17 — and the audit is owed the reason in numbers rather
    /// than in prose.
    #[test]
    fn the_named_gradient_has_two_doors_shut_and_this_is_which() {
        // The slot itself is real on every rung: what is missing is not
        // the declaration.
        for rung in ["elev.panel", "elev.focused", "elev.popover"] {
            assert!(
                theme::id(&format!("{rung}.edge.gradient")).is_some(),
                "{rung} stopped declaring the named-gradient slot"
            );
        }
        // Door one: `@grad.spectrum` is a reference to a SECTION, and §3.2's
        // reference production resolves tokens. There is nothing under that
        // name for a theme's value to point at.
        assert_eq!(
            theme::id("grad.spectrum"),
            None,
            "a gradient's NAME became a token — the reference door is open, \
             so `edge.gradient = @grad.<name>` can now be resolved and owes \
             `Level::edge_gradient` a reader"
        );
        // Door two: the pairs under it are tokens, and the bake drops them.
        // The master writes this stop's position as 0.34; a baked pair would
        // answer with it.
        let stop = theme::id("grad.spectrum.stops[1]")
            .expect("the master's stop list stopped declaring its slots");
        assert_eq!(
            theme::resolved().px(stop),
            0.0,
            "a `[position, colour]` pair now bakes — the stop door is open, \
             so a multi-stop ring is buildable and owes the ladder a reader"
        );
    }

    /// USTERKA 2. A gradient written in the theme reaches the ring as a
    /// gradient: two ends, and the axis the theme named.
    #[test]
    fn a_gradient_edge_is_drawn_as_one() {
        let t = theme::bake_over_master(&overlay("gradient", "#FF00FF / 1.0", "diag_down"));
        let mut dl = DrawList::recording();
        popover().draw_in(&mut dl, &t, box_(), box_(), AT_REST);
        match ring_cmd(&dl) {
            DrawCmd::RingGrad { near, far, dir, stroke, .. } => {
                // The near end is `edge.color`, untouched: the sugar pair
                // is color -> color2, in that order.
                let want = t.color(theme::id("elev.popover.edge.color").unwrap());
                assert!((near.r - want.r).abs() < 1e-6, "near {near:?} is not edge.color");
                assert!((near.a - want.a).abs() < 1e-6, "near {near:?} is not edge.color");
                // A hair, not equality: the far end went round the sRGB
                // transfer on its way through the bake.
                for (got, want) in [(far.r, 1.0), (far.g, 0.0), (far.b, 1.0), (far.a, 1.0)] {
                    assert!((got - want).abs() < 1e-6, "far {far:?} is not #FF00FF");
                }
                assert_eq!(dir, [1.0, 1.0]);
                assert!(stroke > 0.0, "the ring still takes its width from the theme");
            }
            other => panic!("the theme asked for a gradient and got {other}"),
        }
    }

    /// Each of the four words is a different direction, and `y` is DOWN —
    /// the screen's axis, not the plotter's.
    #[test]
    fn every_axis_word_is_its_own_direction() {
        for (word, dir) in AXES {
            match ring_under("gradient", "#FF00FF / 1.0", word) {
                DrawCmd::RingGrad { dir: got, .. } => assert_eq!(got, dir, "{word}"),
                other => panic!("{word} drew {other}"),
            }
        }
    }

    /// The master's own default stays FLAT, which is the whole reason the
    /// picture did not move when this reader landed: `same_as_parent` is a
    /// §5.0 sentinel — "copy edge.color" — and a copy of one colour is not
    /// a gradient. Three shipped spellings, one flat ring.
    #[test]
    fn the_master_default_and_its_neighbours_stay_flat() {
        for (mode, color2, axis) in [
            // What every one of the nine rungs ships.
            ("solid", "same_as_parent", "x"),
            // A theme that asked for a gradient and named no far end.
            ("gradient", "same_as_parent", "x"),
            // …and one that named a direction the vocabulary does not
            // have: not a direction, so not a gradient, rather than a
            // guess made in Rust.
            ("gradient", "#FF00FF / 1.0", "sideways"),
        ] {
            match ring_under(mode, color2, axis) {
                DrawCmd::Ring { .. } => {}
                other => panic!("{mode}/{color2}/{axis} drew {other}"),
            }
        }
    }

    /// A gradient ring costs what a flat one costs — the master's own
    /// claim at `[grad]` ("the same 24 verts a solid border costs"), which
    /// is why a gradient border was affordable enough to declare on all
    /// nine rungs in the first place.
    #[test]
    fn a_gradient_ring_costs_what_a_flat_ring_costs() {
        let t = theme::resolved();
        let flat = {
            let mut dl = DrawList::new();
            popover().draw_in(&mut dl, t, box_(), box_(), AT_REST);
            dl.verts.len()
        };
        let grad = {
            let g = theme::bake_over_master(&overlay("gradient", "#FF00FF / 1.0", "x"));
            let mut dl = DrawList::new();
            popover().draw_in(&mut dl, &g, box_(), box_(), AT_REST);
            dl.verts.len()
        };
        assert_eq!(flat, grad);
    }

    /// The two ends land ON the box, not somewhere inside it: `t` is
    /// normalised against the rect's own projected extent, so the near
    /// colour is exactly at the least-projected corner and the far colour
    /// exactly at the most-projected one. Read off the VERTICES, since
    /// that is what the rasteriser interpolates between.
    #[test]
    fn the_two_ends_reach_the_ends_of_the_box() {
        // `fill = none` leaves the RING alone in the list: the body under
        // it reaches the same two edges in a different colour, so a
        // reading taken over the whole list would be measuring the fill.
        let t = theme::bake_over_master(&format!(
            "{}fill = none\n",
            overlay("gradient", "#FF00FF / 1.0", "x")
        ));
        let mut dl = DrawList::new();
        popover().draw_in(&mut dl, &t, box_(), box_(), AT_REST);
        let r = box_();
        let left = dl
            .verts
            .iter()
            .filter(|v| (v.pos[0] - r.x).abs() < 1e-3)
            .map(|v| v.color[0])
            .fold(f32::INFINITY, f32::min);
        let right = dl
            .verts
            .iter()
            .filter(|v| (v.pos[0] - (r.x + r.w)).abs() < 1e-3)
            .map(|v| v.color[0])
            .fold(f32::NEG_INFINITY, f32::max);
        let near = t.color(theme::id("elev.popover.edge.color").unwrap());
        assert!(!dl.verts.is_empty(), "the ring drew nothing to measure");
        assert!((left - near.r).abs() < 1e-6, "left end {left} is not edge.color");
        assert!((right - 1.0).abs() < 1e-6, "right end {right} is not edge.color2");
    }

    // ------------------------------------------- the per-corner say
    //
    // f3 K6 lands HERE and not in `window.rs`, because a surface is
    // drawn in one place and a corner is part of a surface. What the
    // frame states is only WHICH preset (`shaped_by`); the reading is
    // the rung's, so every consumer that names one gets it, including
    // the ones written after this line. The whole road — the master's
    // own file through the shipped emitter — is
    // `tests/shape_preset_reaches_the_frame.rs`; what is proved here is
    // the part only this file can break.

    /// The panel rung wearing the preset the window frame points it at.
    fn framed() -> Level {
        Level::of("elev.panel").shaped_by("shape.panel")
    }

    /// A stated LENGTH on one corner moves that corner and no other,
    /// with the style slot left inheriting — the half-stated pair, which
    /// is the whole reason these keys are pairs and not single words.
    ///
    /// The LENGTH half and not the style half, and the reason is a real
    /// limit rather than an omission: a `bake_over_master` theme builds
    /// its own `Schema` and does not publish it, while WORDS
    /// live in the published one — `ui::with_theme_word` can
    /// only ever answer out of that. A style word this theme is the
    /// first to use is therefore unreadable from here whatever the
    /// reader does. Scalars have no such split, so the length slot is
    /// the part a rung can be put on trial for in isolation; the style
    /// word is proved on a theme that is actually LOADED, in
    /// `tests/shape_preset_reaches_the_frame.rs`.
    #[test]
    fn a_stated_length_cuts_one_corner_of_the_rung() {
        let t = theme::bake_over_master(
            "[shape.panel]\ncorners_bl = [ same_as_parent, 0.6u ]\n",
        );
        let (c, _) = framed().cut(&t, box_());
        assert!(c[0].size > 0.0, "the rung's own radius arrived as nothing: {:?}", c[0]);
        assert_eq!(c[..3], [c[0]; 3], "a corner the theme did not name moved: {c:?}");
        assert_eq!(c[3].style, c[0].style, "the inheriting slot lost the rung's own cut");
        assert!(c[3].size < c[0].size, "the stated length never reached the corner: {c:?}");
    }

    /// The say is the CONSUMER's to give: a rung nobody pointed at a
    /// preset reads the same overlay and does not move.
    ///
    /// `shape.*` has sixteen presets and the audit's §7.2 leaves open
    /// whether every object moves onto them, so a rung must not acquire
    /// one by standing next to it — `Level::of("elev.panel")` alone is
    /// still four corners of one answer.
    #[test]
    fn a_rung_no_preset_was_named_for_keeps_its_four_equal_corners() {
        let t = theme::bake_over_master(
            "[shape.panel]\ncorners_bl = [ same_as_parent, 0.6u ]\n",
        );
        let (c, _) = Level::of("elev.panel").cut(&t, box_());
        assert_eq!(c, [c[0]; 4], "an unshaped rung read a preset nobody gave it: {c:?}");
    }

    /// The master's own picture does not move: all thirty-two slots of
    /// `shape.panel` inherit, so the rung answers exactly the four equal
    /// corners it answered before it could be asked for anything else —
    /// tessellation included, which now comes off `round_reach` rather
    /// than off the base.
    #[test]
    fn the_preset_the_master_ships_moves_no_corner() {
        let t = theme::resolved();
        assert_eq!(framed().cut(t, box_()), Level::of("elev.panel").cut(t, box_()));
    }
}
