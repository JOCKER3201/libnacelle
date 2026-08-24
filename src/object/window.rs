//! Window objects: the dimmed backdrop and the window frame.
//!
//! The frame takes its geometry from the theme as well as its colour — how
//! thick a border is, how far its corners are cut — so two themes can differ
//! in shape and not only in hue. There is no fallback underneath any read:
//! a missing token degrades through the engine's per-kind default and is
//! allowed to look raw, which is what keeps every design decision in the
//! theme files.

use crate::access::{AccessInfo, Role};
use crate::draw::{Corner, CornerStyle, DrawList};
use crate::focus::FocusId;
use crate::font::FontSystem;
use crate::theme::{self, Color, TokenId};
use crate::{Ctx, Rect};
use std::sync::OnceLock;

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// A baked theme colour in the draw list's own colour type.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// The [`CornerStyle`] a corner-mode enum token resolves to. Enum words
/// intern in load order with the master's own word at index 0, so an index
/// is only meaningful against the vocabulary (`theme::enum_index`), never
/// as a bare number. A missing token — or a word the vocabulary does not
/// name — degrades to Square, the raw look of an unstyled rect.
pub(crate) fn corner_style(
    t: &theme::ResolvedTheme,
    mode: TokenId,
    idx: &'static OnceLock<(Option<u16>, Option<u16>)>,
) -> CornerStyle {
    cut_of(t, mode, *idx.get_or_init(|| vocabulary(mode)))
}

/// The `(round, chamfer)` indices in one corner-mode token's vocabulary.
///
/// Taken apart from [`corner_style`] because a caller that reads a WHOLE
/// dictionary at once — [`super::elev::Level`], which memoises every key
/// of one `[elev.*]` level in a single struct — has nowhere to hang a
/// `static` per token and no reason to: the vocabulary is the master's,
/// so it is settled once with the ids beside it.
pub(crate) fn vocabulary(mode: TokenId) -> (Option<u16>, Option<u16>) {
    (theme::enum_index(mode, "round"), theme::enum_index(mode, "chamfer"))
}

/// The largest ARC on a ring, which is the only thing its tessellation
/// count has to answer for.
///
/// [`crate::draw::ring_points`] takes ONE count for all four corners, so
/// something has to reconcile four sizes into it, and the honest reducer
/// is the largest one that is actually curved: a square corner is a
/// point and a chamfer is a single straight cut, and neither is improved
/// by a finer arc. Reading the plain maximum instead would let a theme
/// that chamfers one corner deeply raise the segment count of the three
/// round ones it never mentioned — a change to a corner it did not name.
pub(crate) fn round_reach(c: &[Corner; 4]) -> f32 {
    c.iter()
        .filter(|k| k.style == CornerStyle::Round)
        .fold(0.0f32, |m, k| m.max(k.size))
}

/// [`corner_style`] with the vocabulary already in hand.
pub(crate) fn cut_of(
    t: &theme::ResolvedTheme,
    mode: TokenId,
    (round, chamfer): (Option<u16>, Option<u16>),
) -> CornerStyle {
    let cur = Some(t.enum_of(mode));
    if cur == round {
        CornerStyle::Round
    } else if cur == chamfer {
        CornerStyle::Chamfer
    } else {
        // "square", plus anything the vocabulary does not name.
        CornerStyle::Square
    }
}

/// The arc tessellation for a corner of size `size` — the name this
/// file's own callers reach it by.
///
/// The RULE moved to [`crate::corner::segments`] on 2026-08-18, where
/// the vocabulary already lives, because a drawing outside this crate
/// needs it and `pub(crate)` was as far as this could reach. Kept as a
/// forwarder rather than replaced at nine call sites: the corner
/// resolver is one module, and which door a caller in this file uses to
/// it is not a decision worth spending a diff on.
pub(crate) fn corner_segments(
    t: &theme::ResolvedTheme,
    cell: &'static OnceLock<TokenId>,
    size: f32,
) -> u8 {
    crate::corner::segments(t, cell, size)
}

/// The seven token ids one `[glow]` class needs before its light can be a
/// LIT TUBE rather than a soft halo.
///
/// Ids and not names, so a class memoises its own and calls the same
/// reader; the reader ([`tube_dress`]) knows nothing about panels.
///
/// TO ADD A CONSUMER — the owner's "first the frame of the whole object,
/// the rest later", written down so the rest is a recipe and not a
/// rediscovery:
///
/// 1. in the master, add `tube` to that class's `falloff` `enum:` list
///    and declare `<class>.tube_decay`, `<class>.tube_aura`,
///    `<class>.tube_aura_reach`, `<class>.tube_bands` and
///    `<class>.tube_cutoff` beside it. The class already declares
///    `boost` — all fifteen do.
/// 2. build a `TubeKeys` from the seven ids in a `OnceLock` of its own,
///    beside the ids that class already memoises.
/// 3. where the class strokes its ring, ask [`tube_dress`]; on `Some`,
///    stroke [`Tube::core`] over the ring at the ring's own width and
///    hand [`Tube::profile`] to `DrawList::glow_ring_with` instead of
///    calling `glow_ring`.
///
/// That is the whole of it: no drawing code moves, because there is none
/// outside this file and `draw.rs`'s one emitter. What a consumer must
/// bring is two things:
///
/// * a WIDTH — a tube's core is the stroke it is made of, so a glow whose
///   caller has no stroke (a text bloom, say) has no core to burn and
///   should keep asking for a halo;
/// * a MASK BAND. `glow_ring_with` re-maps the soft disk's own profile,
///   and with no band to sample it falls back to the maskless shell,
///   which draws the halo's shape and DROPS the profile silently. Every
///   consumer that passes `FontSystem::mask_soft_uv()` is safe by
///   construction; one that computes a band of its own must check it.
pub(crate) struct TubeKeys {
    falloff: TokenId,
    boost: TokenId,
    decay: TokenId,
    aura: TokenId,
    aura_reach: TokenId,
    bands: TokenId,
    cutoff: TokenId,
}

/// One class's tube, dressed — the light's shape and the drive on its core.
pub(crate) struct Tube {
    profile: crate::draw::GlowProfile,
    boost: f32,
}

impl Tube {
    /// The core of the tube, given the colour of the glass.
    ///
    /// A pixel driven at `boost` times what a display can show is
    /// CLIPPED, and the clip is the whole effect: the strongest channel
    /// of a saturated colour reaches 1 first, the others follow as the
    /// drive rises, and the core goes pale and then white while the light
    /// around it — never driven — keeps the hue. So there is no "how much
    /// white" arithmetic here and no number chosen in Rust: the amount of
    /// white is what clipping the theme's own colour at the theme's own
    /// drive comes to.
    ///
    /// THE CLIP IS TAKEN HERE, not left to a display, and the difference
    /// matters enough to say: `.min(1.0)` per channel means the picture
    /// is the same on every target this toolkit can be drawn to, and no
    /// stage downstream has to be taught what a colour above 1 means. It
    /// also means an HDR swapchain gets the clipped colour like everybody
    /// else — a tube that stays bright on R16F needs this clamp lifted
    /// AND a `grade()` that can take a sample above 1, and that second
    /// half lives in the renderer's repository. The master says so at
    /// `glow.panel_edge.boost`; nothing here claims otherwise.
    ///
    /// Alpha is the edge's, untouched. A drive is on the LIGHT; coverage
    /// is a different question and the tube covers exactly what the
    /// border covered.
    fn core(&self, edge: Color) -> Color {
        Color {
            r: (edge.r * self.boost).min(1.0),
            g: (edge.g * self.boost).min(1.0),
            b: (edge.b * self.boost).min(1.0),
            a: edge.a,
        }
    }
}

/// The tube dress of one glow class, or `None` when its `falloff` names
/// any other profile.
///
/// The ONE place the word `tube` becomes a picture. `word` memoises the
/// word's index in that token's own vocabulary, which is the only
/// meaningful form of an enum value (`theme::enum_index`); a master with
/// no `tube` in the list answers `None` and the class keeps its halo,
/// which is what a theme engine loaded against an older master must do.
pub(crate) fn tube_dress(
    t: &theme::ResolvedTheme,
    k: &TubeKeys,
    word: &'static OnceLock<Option<u16>>,
) -> Option<Tube> {
    let tube = *word.get_or_init(|| theme::enum_index(k.falloff, "tube"));
    if Some(t.enum_of(k.falloff)) != tube {
        return None;
    }
    Some(Tube {
        // Clamped at the ends the master documents, and clamped here
        // rather than trusted, because a theme file is a user file. The
        // clamps are the token's declared range and no more: a decay
        // below 1 would spread the light WIDER than the halo it is a
        // sharpening of, an aura below 1 would dim the glass it is a
        // saturation of, a reach or a cutoff outside 0..1 is not a
        // fraction, and a band count is a number of ring strokes this
        // process has to emit — the one clamp of the six that is about
        // the machine rather than the picture, which is why its ceiling
        // is `GlowProfile::MAX_BANDS` and not a number spelt here.
        profile: crate::draw::GlowProfile {
            decay: t.px(k.decay).max(1.0),
            aura: t.px(k.aura).max(1.0),
            aura_reach: t.px(k.aura_reach).clamp(0.0, 1.0),
            bands: t.px(k.bands).clamp(1.0, crate::draw::GlowProfile::MAX_BANDS as f32) as u32,
            cutoff: t.px(k.cutoff).clamp(0.0, 1.0),
        },
        boost: t.px(k.boost).max(1.0),
    })
}

/// The panel-edge light — `[glow] panel_edge`, family A's signature.
///
/// Every frame that strokes a panel-class ring calls this right after the
/// stroke, with the ring's own colour and width: an additive soft-sprite
/// ring at the theme's radius, tinted with the edge's own resolved colour
/// (the `element` rule — no variant theme names a different tint) at
/// `panel_edge.alpha`, scaled by the one global knob `glow.alpha_scale`.
/// Default ships it off; a theme opts in, and a raw master draws nothing
/// because a missing flag reads false.
///
/// TWO PROFILES, ONE CALL. `panel_edge.falloff` decides which:
///
/// * anything but `tube` — the soft halo this has always drawn, a blurred
///   copy of the edge in the edge's own colour. The theme editor calls
///   this kind GLOW.
/// * `tube` — a lit glass tube. Its core is the border stroke re-laid in
///   the colour the display shows when that colour is DRIVEN at
///   `panel_edge.boost` (white, at enough drive, whatever the hue); its
///   colour is carried by the saturated band `tube_aura` lays just
///   outside the glass; and its light stops rather than fades, by
///   `tube_decay`. The theme editor calls this kind NEON.
///
/// The tube's core is the BORDER and not a line of its own: a neon sign
/// is a tube of glass with a current in it, and the width of that glass
/// is the width the theme already stated at `edge.width`. That is why
/// nothing here names a core thickness — there is no such decision to
/// take, in this file or in a theme.
///
/// It is laid as a SECOND stroke over the caller's, rather than by the
/// caller stroking the driven colour in the first place, and that is a
/// choice with two costs and one reason. The reason: a tube must be
/// drawable from ONE place, or every future consumer re-implements it
/// (the recipe on [`TubeKeys`] would otherwise be four steps instead of
/// three). The costs, both known and both small:
///
/// * A GRADIENT BORDER's core comes out flat. `elev::Level` strokes two
///   colours along an axis when the theme asks; the drive has one colour
///   to give back, because the halo over it has one too. A tube whose
///   glass changes colour along its length is a real thing and a
///   different task — it needs a driven gradient and a gradient halo,
///   neither of which exists.
/// * In the VECTOR lane the core is a second shape record over the
///   border's own, which is R4's double edge-AA — one of the bounded,
///   known costs `render.vector`'s master token comment names as shipped
///   rather than blocking, as of K3d (2026-08-23). It costs nothing today
///   and it is written down so it is not rediscovered.
///
/// A theme written before `tube` existed cannot reach any of it: the word
/// is not in its falloff, so [`tube_dress`] answers `None` and the halo
/// is drawn from the same four tokens it always was. `boost` in
/// particular is read ONLY on the tube road, which is what lets the
/// master state a real drive for a tube while every existing glow theme
/// keeps its picture to the bit.
///
/// `now` is the caller's clock (`Ctx.t`) and it drives ONE thing:
/// `motion.glow_pulse`, §5.22's breathing halo, whose `amplitude` key is
/// documented as "± swing applied to glow_alpha" and had no reader
/// anywhere. The swing is on the halo's ALPHA and nothing else — a
/// breathing RADIUS is a different sprite every frame, and the master's
/// own prohibition list has "anything that affects layout" for the same
/// reason. `glow_pulse` ships disabled and so does `glow.panel_edge`, so
/// the master's picture is what it was, and a theme has to ask twice
/// before this costs a token read.
///
/// TWO KEYS THIS CLASS DECLARES AND NOBODY READS, LEFT THAT WAY ON
/// PURPOSE. `panel_edge.mode` is a rendering TECHNIQUE (`shell` under
/// 0.85u, `sprite` above) and not a shape: giving it a reader would
/// change which emitter every existing glow theme lands on at small
/// radii, which is a change to their picture and not to this task's.
/// `panel_edge.color` is the halo's TINT, and its `element` rule already
/// describes what the caller passes in — a reader for it belongs to the
/// halo and the tube alike, would move the picture of any theme that
/// wrote a literal there, and is its own task. Neither is a place a tube
/// needed; neither was given a second token meaning the same thing.
pub(crate) fn panel_edge_glow(
    dl: &mut DrawList,
    t: &theme::ResolvedTheme,
    r: Rect,
    c: &[Corner; 4],
    segments: u8,
    edge: Color,
    width: f32,
    now: f64,
) {
    static ON: OnceLock<TokenId> = OnceLock::new();
    static RADIUS: OnceLock<TokenId> = OnceLock::new();
    static ALPHA: OnceLock<TokenId> = OnceLock::new();
    static SCALE: OnceLock<TokenId> = OnceLock::new();
    static KEYS: OnceLock<TubeKeys> = OnceLock::new();
    static TUBE_WORD: OnceLock<Option<u16>> = OnceLock::new();
    if !t.flag(tok(&ON, "glow.panel_edge.enabled")) {
        return;
    }
    let radius = t.px(tok(&RADIUS, "glow.panel_edge.radius")).max(0.0);
    let alpha = (t.px(tok(&ALPHA, "glow.panel_edge.alpha"))
        * t.px(tok(&SCALE, "glow.alpha_scale")))
    .clamp(0.0, 1.0);
    if radius <= 0.0 || alpha <= 0.0 {
        return;
    }
    // The breath, applied last so the theme's own number is the one it
    // swings about. A frozen pulse — off, no amplitude, or reduced motion
    // — answers exactly 1.0, and `alpha * 1.0` is `alpha`.
    let alpha =
        (alpha * crate::motion::Effect::of("glow_pulse").cyclic_amplitude(now)).clamp(0.0, 1.0);
    if alpha <= 0.0 {
        return;
    }
    let keys = KEYS.get_or_init(|| TubeKeys {
        falloff: theme::id("glow.panel_edge.falloff").unwrap_or(TokenId::MISSING),
        boost: theme::id("glow.panel_edge.boost").unwrap_or(TokenId::MISSING),
        decay: theme::id("glow.panel_edge.tube_decay").unwrap_or(TokenId::MISSING),
        aura: theme::id("glow.panel_edge.tube_aura").unwrap_or(TokenId::MISSING),
        aura_reach: theme::id("glow.panel_edge.tube_aura_reach").unwrap_or(TokenId::MISSING),
        bands: theme::id("glow.panel_edge.tube_bands").unwrap_or(TokenId::MISSING),
        cutoff: theme::id("glow.panel_edge.tube_cutoff").unwrap_or(TokenId::MISSING),
    });
    let profile = match tube_dress(t, keys, &TUBE_WORD) {
        // The core FIRST, then the light over it: the burned stroke is
        // opaque where the border was and the halo composes additively,
        // so laying the halo first would put light under a cover and
        // lose it. A width of zero is a caller with no stroke to burn —
        // the tube then has no core, and says so by drawing none rather
        // than by inventing a thickness.
        Some(tube) => {
            if width > 0.0 {
                dl.ring(r, c, segments, width, tube.core(edge));
            }
            tube.profile
        }
        None => crate::draw::GlowProfile::HALO,
    };
    dl.glow_ring_with(
        r,
        c,
        segments,
        radius,
        edge.alpha(alpha),
        FontSystem::mask_soft_uv(),
        profile,
    );
    // A tube lights the frame it edges, not only the dark outside it: NEON
    // throws the same profile inward, over the body just drawn, so the
    // border glows on BOTH sides. A halo is a one-way bleed and asks for no
    // such thing — the gate is the profile, which is the theme's own
    // `glow.panel_edge.falloff = tube` and nothing decided in Rust.
    if !profile.is_halo() {
        dl.glow_ring_inward_with(
            r,
            c,
            segments,
            radius,
            edge.alpha(alpha),
            FontSystem::mask_soft_uv(),
            profile,
        );
    }
}

/// Dims everything behind a modal window.
///
/// The theme owns both the tint and the strength: `modal.scrim_alpha` states
/// how far the desktop darkens, so three call sites cannot carry three
/// designs. The caller's historical alpha is ignored for that reason; the
/// parameter stays only so existing embedders keep compiling.
///
/// It also claims the whole screen for the pointer ([`crate::pointer`]),
/// which is what MODAL means said in the one place it is drawn: nothing
/// behind the scrim is under the hand, including the parts of the desktop
/// the window itself does not stand on.
pub fn backdrop(ctx: &mut Ctx, _alpha: f32) {
    static SCRIM: OnceLock<TokenId> = OnceLock::new();
    static STRENGTH: OnceLock<TokenId> = OnceLock::new();
    ctx.mouse.cover(Rect::new(0.0, 0.0, ctx.w, ctx.h));
    let t = theme::resolved();
    let scrim = col(t.bed(tok(&SCRIM, "component.modal.scrim")));
    let strength = t.px(tok(&STRENGTH, "modal.scrim_alpha")).clamp(0.0, 1.0);
    ctx.dl.rect(0.0, 0.0, ctx.w, ctx.h, scrim.alpha(strength));
}

/// The rung a window frame is a surface of, dressed in the window's own
/// key names.
///
/// `[elev.panel]` is Elev 2, and its gloss is "the bordered panel body" —
/// which a window frame is, at the same elevation and out of the same
/// material. That was already true by hand: this file read
/// `component.panel.fill` for the body and `elev.panel.glass.*` for the
/// glass trio, key for key what the rung says, because the owner's scope
/// for a background is "windows and widgets" and one decision has to
/// serve both. What it had was a PRIVATE COPY of the rules, and
/// `elev.rs`'s header names this file as the copy that drifted — it drew
/// its body whatever the alpha where `panel.rs` guarded it, and it drew
/// its ring FLAT where the rung had grown a second colour.
///
/// So the frame states its five older key names once, here, and takes
/// everything else from the rung: the two-stop edge (`edge.mode`,
/// `edge.color2`, `edge.axis` — dead on this surface until now, which is
/// how a theme could write `edge.mode = gradient` and get a flat window
/// border), the glass pair, and every key the ladder grows after this
/// line.
///
/// ONE MODEL OF A WINDOW, and this is where it is kept: a window built
/// into the desktop and a window of an outside application are drawn by
/// this one function, so a rule stated here cannot reach one of them and
/// miss the other.
///
/// `shape.panel` is named as well, and it is the only thing this file
/// still says about a corner: it is the preset that gets the LAST word
/// on the rung's four corners, one at a time (f3 K6). The rung settles
/// what all four are — `panel.corner_mode` and `panel.corner`, above —
/// and the preset may then move any one of them without touching the
/// other three. `shape.panel` and `[elev.panel]` are the same surface
/// under two spellings, which is why the pairing is stated once here
/// rather than guessed from the rung's own name inside the ladder.
fn level() -> &'static super::elev::Level {
    static LEVEL: OnceLock<super::elev::Level> = OnceLock::new();
    LEVEL.get_or_init(|| {
        super::elev::Level::of("elev.panel")
            .worn_as(
                "component.panel.fill",
                "panel.corner_mode",
                "panel.corner",
                "component.panel.border",
                "panel.border",
            )
            .shaped_by("shape.panel")
    })
}

/// Opaque window frame: a shaped background and its border, both from the
/// theme.
///
/// `panel.corner` is a length already baked to device pixels, so a theme that
/// wants square corners sets it to `0u` and one that wants a deep cut sets it
/// large; `panel.corner_mode` says HOW that length is cut — a tessellated arc
/// or a 45° chamfer — and `panel.border` is the stroke. All five are read
/// through [`level`], which is what makes them the SAME five a panel, a
/// menu and a tooltip are drawn from.
///
/// The box is claimed for the pointer ([`crate::pointer`]) before anything
/// is drawn into it: an OPAQUE frame is exactly the statement "what was
/// under this rectangle can no longer be seen", and a control that cannot
/// be seen is not the one the hand is on. Claimed here rather than by each
/// caller so that every window in every application gets it — including
/// the ones written after this line — and claimed FIRST so the window's
/// own contents, drawn into it afterwards, keep the pointer.
///
/// It is also registered PASSIVE with [`crate::access`] — one of the two
/// candidate ROOT nodes an AT-SPI tree needs, [`super::winframe`] being
/// the other. `AccessCtl`, not `FocusCtl`: a window frame is a container a
/// screen reader should announce, never a Tab stop of its own, and the
/// two registries exist precisely so a structural node cannot become one
/// by accident.
pub fn frame(ctx: &mut Ctx, r: Rect) {
    ctx.mouse.cover(r);
    level().draw(ctx, r);
    // Registered with an EMPTY name: `frame` takes only `(ctx, r)`, and no
    // title string reaches it today. Every caller draws its own title
    // text AFTER calling `frame` instead of handing it in — the toaster
    // (`toaster.rs`), the layout editor and the settings modal
    // (nacelle-desktop's `editor.rs` and `settings.rs`) all do this. This
    // is the same gap the foundation pass left open on `slider.rs` and
    // `text_input.rs`: filling it in needs a new optional `title: &str`
    // parameter threaded through `frame` and out to every one of those
    // call sites, which ripples wider than this file and is its own task.
    if let Some(ac) = ctx.access.as_deref_mut() {
        ac.register(FocusId::of("window.root"), r, AccessInfo::new(Role::Window, ""));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{DrawCmd, DrawList};
    use crate::object::elev::tests::{same_picture, AT_REST};

    /// The box every proof below draws into. Any box would do — what is
    /// read off it is which COMMANDS a frame emits and in which colours,
    /// never where a particular vertex landed.
    fn box_() -> Rect {
        Rect::new(30.0, 18.0, 240.0, 150.0)
    }

    /// What this file drew before it joined the ladder: the body of the
    /// old `frame`, transcribed statement for statement — the glass
    /// branch, the fill under it, the ring, and the bloom over the ring.
    ///
    /// Its OWN transcript, and not the one `menu.rs` and `tooltip.rs`
    /// share ([`crate::object::elev::tests::the_private_copy`]), because
    /// a window's copy was never their copy. Theirs departed from the
    /// rung in TWO places — the body drawn whatever its alpha, the ring
    /// drawn on the width alone — and a window's departed in FOUR: it
    /// also stroked its ring whatever the EDGE's alpha, and it laid the
    /// edge bloom unconditionally where the rung asks for a visible edge
    /// first. Borrowing their transcript would have made this file's
    /// no-move proof a proof about a picture it never drew, and would
    /// have left the two extra departures — the two a theme that lights
    /// `glow.panel_edge` can see — untested.
    fn the_frames_private_copy(dl: &mut DrawList, t: &theme::ResolvedTheme, r: Rect, now: f64) {
        static SEG: OnceLock<TokenId> = OnceLock::new();
        let id = |n: &str| theme::id(n).unwrap_or(TokenId::MISSING);
        let fill = col(t.bed(id("component.panel.fill")));
        let line = col(t.color(id("component.panel.border")));
        let mode = id("panel.corner_mode");
        let corner = Corner::sized(cut_of(t, mode, vocabulary(mode)), t.px(id("panel.corner")), r);
        let width = t.px(id("panel.border")).max(0.0);
        let c = [corner; 4];
        let seg = corner_segments(t, &SEG, corner.size);
        let rank = t.px(id("elev.panel.glass.rank")).clamp(0.0, 3.0);
        if rank > 0.0 {
            dl.glass_fill(r, &c, seg, rank, col(t.color(id("elev.panel.glass.tint"))));
            let wash = col(t.color(id("elev.panel.glass.wash")));
            if wash.a > 0.0 {
                dl.ring_fill(r, &c, seg, wash);
            }
        } else {
            dl.ring_fill(r, &c, seg, fill);
        }
        dl.ring(r, &c, seg, width, line);
        panel_edge_glow(dl, t, r, &c, seg, line, width, now);
    }

    /// The no-move proof, in the words `menu.rs` and `tooltip.rs` already
    /// use: a window frame is a surface of Elev 2, and joining the ladder
    /// had to leave the picture exactly where it was under the master.
    /// Compared against [`the_frames_private_copy`], command for command
    /// and vertex for vertex.
    ///
    /// Under the master ALONE, which is half the claim and the weaker
    /// half: the master leaves `elev.panel.glass.rank` at 0 and the base
    /// `glow.panel_edge.enabled` at false, so two of the four things this
    /// file used to do are not reached at all.
    /// [`joining_the_ladder_moved_no_pixel_with_the_glass_and_the_glow_lit`]
    /// is where they are.
    #[test]
    fn joining_the_ladder_moved_no_pixel() {
        let t = theme::resolved();
        let mut was = DrawList::recording();
        the_frames_private_copy(&mut was, t, box_(), AT_REST);
        let mut now = DrawList::recording();
        level().draw_in(&mut now, t, box_(), box_(), AT_REST);
        same_picture(&was, &now);
    }

    /// The same proof where the master cannot make it.
    ///
    /// Two of the frame's four departures from the rung are invisible
    /// under a theme that ships the glass off and the bloom unlit, and
    /// `[mood.alert]` — which the engine ships and a host may select at
    /// any moment — lights the bloom. So the picture is taken again over
    /// a theme that raises the rung's glass rank AND turns
    /// `glow.panel_edge` on, and the two lists still have to agree: the
    /// old ring-then-bloom pair and the rung's guarded one draw the same
    /// thing whenever the edge is there to be drawn.
    ///
    /// Both commands are asserted present first, because two pictures
    /// that agree by both being empty prove nothing.
    #[test]
    fn joining_the_ladder_moved_no_pixel_with_the_glass_and_the_glow_lit() {
        let t = theme::bake_over_master(
            "[elev.panel]\n\
             glass.rank = 2\n\
             glass.wash = #40FFC0 / 0.5\n\
             [glow]\n\
             panel_edge.enabled = true\n\
             panel_edge.radius = 2.0u\n\
             panel_edge.alpha = 0.6\n",
        );
        let mut was = DrawList::recording();
        the_frames_private_copy(&mut was, &t, box_(), AT_REST);
        let has = |dl: &DrawList, what: fn(&DrawCmd) -> bool| dl.cmds().iter().any(what);
        assert!(
            has(&was, |c| matches!(c, DrawCmd::GlassFill { .. })),
            "the raised rank drew no glass, so this proves nothing: {:?}",
            was.cmds()
        );
        assert!(
            has(&was, |c| matches!(c, DrawCmd::GlowRing { .. })),
            "the lit bloom drew nothing, so this proves nothing: {:?}",
            was.cmds()
        );
        let mut now = DrawList::recording();
        level().draw_in(&mut now, &t, box_(), box_(), AT_REST);
        same_picture(&was, &now);
    }

    /// Z16 on the surface that shows it most: a window's ring is the one
    /// a user looks at all day, and until 2026-08-17 it was drawn by this
    /// file's own `dl.ring` call, which has one colour. A theme writing
    /// `edge.mode = gradient` beside a second colour got a flat border
    /// and no word about it — the complaint the audit records against the
    /// cockpit theme, which shipped exactly that pair.
    ///
    /// The overlay restates the two `enum:` lists because a
    /// re-declaration in the same stage replaces the token whole
    /// (`cascade.rs`'s `declare`) and an enum's baked value is an INDEX
    /// into the list it was declared with.
    #[test]
    fn a_gradient_edge_reaches_the_window_frame() {
        let t = theme::bake_over_master(
            "[elev.panel]\n\
             edge.mode = gradient    # · enum: solid | gradient ·\n\
             edge.color2 = #FF00FF / 1.0\n\
             edge.axis = y    # · enum: x | y | diag_down | diag_up ·\n",
        );
        let mut dl = DrawList::recording();
        level().draw_in(&mut dl, &t, box_(), box_(), AT_REST);
        // Filtered to the GRADIENT ring specifically, and not `Ring` too:
        // the master's own panel_edge ships lit since 2026-08-23, and a
        // window now also burns a plain, solid-colour `Ring` core for its
        // glow — a real second ring this test is not about.
        let rings: Vec<_> = dl
            .cmds()
            .iter()
            .filter(|c| matches!(c, DrawCmd::RingGrad { .. }))
            .cloned()
            .collect();
        assert_eq!(rings.len(), 1, "a window strokes its gradient ring once: {rings:?}");
        match &rings[0] {
            DrawCmd::RingGrad { near, far, dir, .. } => {
                let want = t.color(theme::id("component.panel.border").unwrap());
                assert!((near.r - want.r).abs() < 1e-6, "near {near:?} is not the panel border");
                for (got, want) in [(far.r, 1.0), (far.g, 0.0), (far.b, 1.0)] {
                    assert!((got - want).abs() < 1e-6, "far {far:?} is not #FF00FF");
                }
                assert_eq!(*dir, [0.0, 1.0], "the theme said y, which is DOWN the screen");
            }
            other => panic!("the theme asked for a gradient window border and got {other}"),
        }
    }

    /// The frame's five keys are the window's OWN spellings and not the
    /// rung's: `component.panel.fill` is the seam both a window and a
    /// panel read (the master derives `[elev.panel] fill` from it), so a
    /// frame that started reading `elev.panel.fill` instead would sever
    /// the derivation the theme editor's background depends on. Read off
    /// the picture rather than off the source: an overlay moves the
    /// window's key, and the body has to follow it.
    #[test]
    fn the_window_keeps_reading_the_shared_seam_for_its_body() {
        let t = theme::bake_over_master("[component.panel]\nfill = #00FF00 / 1.0\n");
        let mut dl = DrawList::recording();
        level().draw_in(&mut dl, &t, box_(), box_(), AT_REST);
        let body = dl
            .cmds()
            .iter()
            .find_map(|c| match c {
                DrawCmd::RingFill { color, .. } => Some(*color),
                _ => None,
            })
            .expect("a window with an opaque body draws one");
        assert!(body.g > 0.99 && body.r < 0.01, "the body {body:?} is not component.panel.fill");
    }

    /// `frame` registers its box as a PASSIVE `Role::Window` node — one of
    /// the two candidate AT-SPI roots — and does so through `AccessCtl`,
    /// never `FocusCtl`: a bridge should announce the box, but Tab must
    /// never land on it.
    ///
    /// The name comes back empty, which is the honest answer today: no
    /// title string reaches `frame`'s own `(ctx, r)` signature, so there
    /// is nothing better to hand `AccessInfo::new` yet.
    #[test]
    fn frame_registers_a_passive_window_root() {
        use crate::access::AccessCtl;
        use crate::pointer::Pointer;
        let r = box_();
        let mut dl = DrawList::recording();
        let mut fonts = FontSystem::new();
        let mut ac = AccessCtl::new();
        {
            let mut ctx = Ctx {
                access: Some(&mut ac),
                dl: &mut dl,
                fonts: &mut fonts,
                w: 1920.0,
                h: 1080.0,
                t: 0.0,
                mouse: Pointer::new(0.0, 0.0),
                term_font_scale: 1.0,
                ui_font_scale: 1.0,
                panel_scale: 1.0,
                focus: None,
                tips: None,
            };
            frame(&mut ctx, r);
        }
        ac.begin_frame();
        let got: Vec<_> = ac.entries().collect();
        assert_eq!(got.len(), 1, "frame registered {} passive nodes, not one", got.len());
        let (id, rect, info) = &got[0];
        assert_eq!(*id, FocusId::of("window.root"));
        assert_eq!((rect.x, rect.y, rect.w, rect.h), (r.x, r.y, r.w, r.h));
        assert_eq!(info.role, Role::Window);
        assert_eq!(info.name, "", "no title reaches `frame` today; the gap is left in a comment");
    }

    /// `ctx.access` stays `None` in most of this file's own tests (and in
    /// every caller that draws headless), and `frame` must not need it:
    /// the registration is a `map` over an `Option`, so drawing without an
    /// accessibility tree draws exactly the same picture as drawing with
    /// one wired up.
    #[test]
    fn frame_draws_the_same_picture_whether_or_not_access_is_wired_up() {
        use crate::access::AccessCtl;
        use crate::pointer::Pointer;
        let r = box_();

        let mut wired = DrawList::recording();
        let mut wired_fonts = FontSystem::new();
        let mut ac = AccessCtl::new();
        let mut ctx = Ctx {
            access: Some(&mut ac),
            dl: &mut wired,
            fonts: &mut wired_fonts,
            w: 1920.0,
            h: 1080.0,
            t: 0.0,
            mouse: Pointer::new(0.0, 0.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        };
        frame(&mut ctx, r);

        let mut bare = DrawList::recording();
        let mut bare_fonts = FontSystem::new();
        let mut ctx = Ctx {
            access: None,
            dl: &mut bare,
            fonts: &mut bare_fonts,
            w: 1920.0,
            h: 1080.0,
            t: 0.0,
            mouse: Pointer::new(0.0, 0.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        };
        frame(&mut ctx, r);

        same_picture(&wired, &bare);
    }

    // ------------------------------------------------- the lit tube
    //
    // `[glow] panel_edge` grew a second profile on 2026-08-18: the halo
    // it had always drawn, and a lit glass tube. The editor's kind that
    // used to be called NEON is the halo and is now called GLOW; NEON is
    // the tube. What follows proves both halves of that sentence — that
    // the rename moved no pixel, and that the new word draws something a
    // Gaussian blur cannot.

    /// The width the proofs below stroke their border at.
    ///
    /// Any width would do. It is written down rather than taken off a
    /// theme because the claims are about the COLOUR of the core and the
    /// SHAPE of the light, and a border that measured zero would silently
    /// remove the core from every one of them.
    const A_BORDER: f32 = 2.0;

    /// A theme with the halo lit the way a theme lit it before the tube
    /// existed: the flag, a reach and an amount, and not one word about a
    /// falloff.
    ///
    /// This IS the owner's own file. `~/.local/share/nacelle-desktop/`
    /// carries a theme whose whole `[glow]` section is
    /// `panel_edge.enabled = true` — the editor's old NEON, saved before
    /// the kind had a second meaning, and silent about a falloff. Until
    /// 2026-08-23 that silence inherited the master's `gauss` and this
    /// theme drew a halo; the master's own falloff is `tube` now, so the
    /// same file inherits NEON instead — the name below says what the
    /// token declares (nothing), not what the picture used to be.
    fn a_silent_falloff_theme() -> theme::ResolvedTheme {
        theme::bake_over_master(
            "[glow]\n\
             panel_edge.enabled = true\n\
             panel_edge.radius = 2.0u\n\
             panel_edge.alpha = 0.34\n",
        )
    }

    /// The same theme with the halo's own word, spoken rather than
    /// inherited — for a proof that needs the shape and not the silence.
    fn a_gauss_theme() -> theme::ResolvedTheme {
        theme::bake_over_master(
            "[glow]\n\
             panel_edge.enabled = true\n\
             panel_edge.radius = 2.0u\n\
             panel_edge.alpha = 0.34\n\
             panel_edge.falloff = gauss # enum: linear | quad | gauss | halo | tube\n",
        )
    }

    /// The same theme with the one word that makes it a tube.
    ///
    /// The `enum:` list is restated because a re-declaration in the same
    /// stage replaces the token whole, list included (`cascade.rs`'s
    /// `declare`), and an enum's baked value is an INDEX into the list it
    /// was declared with. A SAVED file does not restate anything — the
    /// writer patches the bytes of a value span and leaves the master's
    /// declaration where it is — so this line is an artefact of baking an
    /// overlay, not of what a user's theme looks like.
    fn a_tube_theme() -> theme::ResolvedTheme {
        theme::bake_over_master(
            "[glow]\n\
             panel_edge.enabled = true\n\
             panel_edge.radius = 2.0u\n\
             panel_edge.alpha = 0.34\n\
             panel_edge.falloff = tube # enum: linear | quad | gauss | halo | tube\n",
        )
    }

    /// What this function did before it could draw a tube, transcribed
    /// statement for statement: four token reads, the pulse, and one
    /// unshaped glow ring.
    ///
    /// The emitter it calls is proved separately and against its own
    /// transcript ([`crate::draw::tests::the_shaped_emitter_still_draws_the_unshaped_halo`]);
    /// what is proved here is the layer above it — that a theme's four
    /// keys still reach the same call with the same numbers, and that
    /// nothing new was laid beside it.
    fn the_halos_own_transcript(
        dl: &mut DrawList,
        t: &theme::ResolvedTheme,
        r: Rect,
        c: &[Corner; 4],
        segments: u8,
        edge: Color,
        now: f64,
    ) {
        let id = |n: &str| theme::id(n).unwrap_or(TokenId::MISSING);
        if !t.flag(id("glow.panel_edge.enabled")) {
            return;
        }
        let radius = t.px(id("glow.panel_edge.radius")).max(0.0);
        let alpha = (t.px(id("glow.panel_edge.alpha")) * t.px(id("glow.alpha_scale")))
            .clamp(0.0, 1.0);
        if radius <= 0.0 || alpha <= 0.0 {
            return;
        }
        let alpha = (alpha
            * crate::motion::Effect::of("glow_pulse").cyclic_amplitude(now))
        .clamp(0.0, 1.0);
        if alpha <= 0.0 {
            return;
        }
        dl.glow_ring(r, c, segments, radius, edge.alpha(alpha), FontSystem::mask_soft_uv());
    }

    /// THE RENAME MOVED NO PIXEL.
    ///
    /// The owner's condition on calling the old kind GLOW was that a
    /// theme already wearing it look exactly the same afterwards. So the
    /// theme that WAS the old NEON is drawn twice — once by the
    /// transcript of the code that drew it, once by the code that now
    /// offers two profiles — and the two lists are compared command for
    /// command and vertex for vertex.
    ///
    /// The master's `boost` moved from 1.0 to 2.6 on the same day, which
    /// is exactly the kind of change this proof exists to catch: a drive
    /// read on the halo road would have burned the border of every theme
    /// that had ever turned the glow on. It is read only under `tube`,
    /// and this is where that is checked.
    #[test]
    fn renaming_the_halo_moved_no_pixel() {
        let t = a_gauss_theme();
        let edge = Color { r: 0.35, g: 0.62, b: 0.94, a: 1.0 };
        let c = [Corner::round(9.0); 4];
        let mut was = DrawList::recording();
        the_halos_own_transcript(&mut was, &t, box_(), &c, 6, edge, AT_REST);
        assert!(
            was.cmds().iter().any(|c| matches!(c, DrawCmd::GlowRing { .. })),
            "the halo theme drew no glow, so this proves nothing: {:?}",
            was.cmds()
        );
        let mut now = DrawList::recording();
        panel_edge_glow(&mut now, &t, box_(), &c, 6, edge, A_BORDER, AT_REST);
        same_picture(&was, &now);
    }

    /// THE OWNER'S SAVED THEME NOW OPENS ON THE TUBE, NOT THE HALO.
    ///
    /// Until 2026-08-23 a file that said `panel_edge.enabled = true` and
    /// nothing else about a falloff inherited the master's `gauss`. The
    /// master's own falloff moved to `tube` that day (the neon-by-default
    /// change), so the same sparse file now inherits NEON — the picture
    /// is asserted equal to a theme that names `tube` outright, not just
    /// "not the halo," so a build where inheritance quietly stopped
    /// reaching the master would fail here too.
    ///
    /// `gauss` did not stop being a real profile; it stopped being what
    /// silence resolves to. A theme that still asks for it by name gets
    /// exactly the halo it always did, core-less ring included.
    #[test]
    fn a_theme_that_names_no_falloff_inherits_the_tube() {
        let edge = Color { r: 0.35, g: 0.62, b: 0.94, a: 1.0 };
        let c = [Corner::round(9.0); 4];
        let profiles = |t: &theme::ResolvedTheme| -> Vec<crate::draw::GlowProfile> {
            let mut dl = DrawList::recording();
            panel_edge_glow(&mut dl, t, box_(), &c, 6, edge, A_BORDER, AT_REST);
            dl.cmds()
                .iter()
                .filter_map(|cmd| match cmd {
                    DrawCmd::GlowRing { profile, .. } => Some(*profile),
                    _ => None,
                })
                .collect()
        };
        let silent = a_silent_falloff_theme();
        let spoken = profiles(&a_tube_theme());
        assert_eq!(
            profiles(&silent),
            spoken,
            "a theme that says nothing about a falloff no longer matches the word `tube`"
        );
        assert!(
            !spoken[0].is_halo(),
            "the word `tube` reached nothing — {:?} is still the halo, so the \
             claim above is about a build where the word does not resolve",
            spoken[0]
        );

        let named_gauss = a_gauss_theme();
        assert_eq!(
            profiles(&named_gauss),
            vec![crate::draw::GlowProfile::HALO],
            "a theme that names gauss outright was not given the halo"
        );
        let mut dl = DrawList::recording();
        panel_edge_glow(&mut dl, &named_gauss, box_(), &c, 6, edge, A_BORDER, AT_REST);
        assert!(
            !dl.cmds().iter().any(|c| matches!(c, DrawCmd::Ring { .. })),
            "the halo burned a core over the border: {:?}",
            dl.cmds()
        );
    }

    /// THE MASTER CARRIES THE TUBE, AND RUST CARRIES NONE OF IT.
    ///
    /// The governing principle asked of the one feature most likely to
    /// break it: a tube is a LOOK, and a look lives in a theme file. Every
    /// number the tube is made of is read back through the same reader
    /// the frame uses and asserted to describe a tube rather than a halo
    /// wearing the word — a drive above rest, light that falls faster
    /// than the disk's own, a band that lifts, and a reach for it to lift
    /// over.
    ///
    /// FIVE SEPARATE CLAIMS, deliberately: the tube degrades knob by knob
    /// and each degradation is silent. A master that lost `tube_decay`
    /// alone would still draw a burned core inside a lifted band and pass
    /// every other proof in this file, because a missing token reads
    /// zero, a zero decay clamps to the halo's own 1.0, and nothing
    /// anywhere says a word about it.
    #[test]
    fn the_master_carries_the_tubes_whole_dress() {
        static WORD: OnceLock<Option<u16>> = OnceLock::new();
        let id = |n: &str| theme::id(n).unwrap_or(TokenId::MISSING);
        let keys = TubeKeys {
            falloff: id("glow.panel_edge.falloff"),
            boost: id("glow.panel_edge.boost"),
            decay: id("glow.panel_edge.tube_decay"),
            aura: id("glow.panel_edge.tube_aura"),
            aura_reach: id("glow.panel_edge.tube_aura_reach"),
            bands: id("glow.panel_edge.tube_bands"),
            cutoff: id("glow.panel_edge.tube_cutoff"),
        };
        let t = a_tube_theme();
        let tube = tube_dress(&t, &keys, &WORD)
            .expect("the master does not name `tube` in its own falloff list");
        assert!(
            tube.boost > 1.0,
            "the master drives the core at {}, which is no drive at all",
            tube.boost
        );
        assert!(
            tube.profile.decay > 1.0,
            "the master's decay is {}, which is the gauss halo",
            tube.profile.decay
        );
        assert!(
            tube.profile.aura > 1.0,
            "the master's aura is {}, which lifts nothing",
            tube.profile.aura
        );
        assert!(
            tube.profile.aura_reach > 0.0,
            "the master's aura reaches {}, so it lifts nothing",
            tube.profile.aura_reach
        );
        // The fifth claim, and the one that used to be a constant in
        // draw.rs: at one band the decay has nowhere to land and the
        // tube IS the halo, whatever the other four numbers say.
        assert!(
            tube.profile.bands > 1,
            "the master cuts the light into {} band(s), so its decay reaches nothing",
            tube.profile.bands
        );
        // The sixth claim: the master spends less than the whole of
        // `radius` on visible light — a cutoff of 1.0 is the identity
        // (no cutoff), and the master asked for a delicate glow, not
        // the full reach lit end to end.
        assert!(
            (0.0..1.0).contains(&tube.profile.cutoff),
            "the master's cutoff is {}, which spends the whole of radius",
            tube.profile.cutoff
        );
        assert!(!tube.profile.is_halo(), "the master's tube {:?} is a halo", tube.profile);
    }

    /// EVERY NUMBER OF A TUBE COMES FROM THE THEME THAT ASKED FOR IT, and
    /// an impossible one is stopped without becoming a design decision.
    ///
    /// `the_master_carries_the_tubes_whole_dress` above proves the master
    /// DECLARES the dress; it cannot prove anything READS it. A reader
    /// that ignored a token and answered the master's own number from
    /// Rust passes it, and passes every other proof in this file — which
    /// is exactly how the band count came to be a constant in `draw.rs`
    /// in the first place. So every number asked for here is deliberately
    /// NOT the master's, and the test says so rather than trusting it.
    ///
    /// The clamps are guards on a USER FILE, not looks: a decay below 1
    /// would spread the light wider than the halo it sharpens, an aura
    /// below 1 would dim the glass it saturates, a reach outside 0..1 is
    /// not a fraction, and a band count is how many ring strokes this
    /// process is asked to emit. A theme inside every declared range
    /// meets none of them.
    #[test]
    fn every_number_of_a_tube_comes_from_the_theme_that_asked() {
        static WORD: OnceLock<Option<u16>> = OnceLock::new();
        let id = |n: &str| theme::id(n).unwrap_or(TokenId::MISSING);
        let keys = TubeKeys {
            falloff: id("glow.panel_edge.falloff"),
            boost: id("glow.panel_edge.boost"),
            decay: id("glow.panel_edge.tube_decay"),
            aura: id("glow.panel_edge.tube_aura"),
            aura_reach: id("glow.panel_edge.tube_aura_reach"),
            bands: id("glow.panel_edge.tube_bands"),
            cutoff: id("glow.panel_edge.tube_cutoff"),
        };
        // One key overridden at a time, so a reader that answered the
        // right number for the wrong key is caught too.
        let dressed = |key: &str, value: &str| -> Tube {
            let t = theme::bake_over_master(&format!(
                "[glow]\n\
                 panel_edge.falloff = tube # enum: linear | quad | gauss | halo | tube\n\
                 panel_edge.{key} = {value}\n"
            ));
            tube_dress(&t, &keys, &WORD).expect("the theme names `tube` and got no tube")
        };
        let master = {
            let t = a_tube_theme();
            tube_dress(&t, &keys, &WORD).expect("the master names `tube` and got no tube")
        };
        // Read out of the profile, so the assertions below compare a
        // theme's number against the reader's answer and nothing else.
        let seen: [(&str, fn(&Tube) -> f32, f32, [f32; 3]); 6] = [
            ("boost", |t| t.boost, master.boost, [1.0, 1.9, 4.0]),
            ("tube_decay", |t| t.profile.decay, master.profile.decay, [1.0, 2.25, 6.0]),
            ("tube_aura", |t| t.profile.aura, master.profile.aura, [1.0, 1.4, 3.5]),
            (
                "tube_aura_reach",
                |t| t.profile.aura_reach,
                master.profile.aura_reach,
                [0.0, 0.6, 1.0],
            ),
            ("tube_bands", |t| t.profile.bands as f32, master.profile.bands as f32, [
                1.0, 3.0, 9.0,
            ]),
            ("tube_cutoff", |t| t.profile.cutoff, master.profile.cutoff, [0.0, 0.55, 1.0]),
        ];
        for (key, read, mine, asked) in seen {
            for want in asked {
                assert_ne!(
                    want, mine,
                    "{key}: this proof needs a number the master does not already say"
                );
                let got = read(&dressed(key, &format!("{want}")));
                assert!(
                    (got - want).abs() < 1e-6,
                    "a theme asked for {key} = {want} and the reader answered {got}"
                );
            }
        }
        // And the guards, each at both ends where the token has two.
        for (key, value, want) in [
            ("boost", "0.4", 1.0),
            ("tube_decay", "0.5", 1.0),
            ("tube_aura", "0.25", 1.0),
            ("tube_aura_reach", "2.0", 1.0),
            ("tube_bands", "0", 1.0),
            ("tube_bands", "1000", crate::draw::GlowProfile::MAX_BANDS as f32),
            ("tube_cutoff", "-0.5", 0.0),
            ("tube_cutoff", "2.0", 1.0),
        ] {
            let t = dressed(key, value);
            let got = match key {
                "boost" => t.boost,
                "tube_decay" => t.profile.decay,
                "tube_aura" => t.profile.aura,
                "tube_aura_reach" => t.profile.aura_reach,
                "tube_cutoff" => t.profile.cutoff,
                _ => t.profile.bands as f32,
            };
            assert!(
                (got - want).abs() < 1e-6,
                "{key} = {value} left this function as {got}, not the guarded {want}"
            );
        }
    }

    /// THE TUBE'S CORE IS BRIGHTER THAN ITS GLASS, AND PALER.
    ///
    /// Two relations, no numbers. Brighter: every channel of the core is
    /// at least the edge's and one of them is strictly above it — that is
    /// what a drive above 1 means. Paler: the spread between the core's
    /// strongest and weakest channel is SMALLER than the edge's, which is
    /// the arithmetic of clipping and the reason a photographed neon sign
    /// has a white core whatever colour it is.
    ///
    /// Coverage is not light: the core's alpha is the edge's, untouched.
    #[test]
    fn the_tube_burns_a_core_brighter_and_paler_than_its_glass() {
        let t = a_tube_theme();
        // A saturated colour, because the claim is about what happens to
        // a HUE under a drive; a grey has no spread to close.
        let edge = Color { r: 0.42, g: 0.14, b: 0.86, a: 0.9 };
        let c = [Corner::round(9.0); 4];
        let mut dl = DrawList::recording();
        panel_edge_glow(&mut dl, &t, box_(), &c, 6, edge, A_BORDER, AT_REST);
        let core = dl
            .cmds()
            .iter()
            .find_map(|cmd| match cmd {
                DrawCmd::Ring { color, stroke, .. } => Some((*color, *stroke)),
                _ => None,
            })
            .unwrap_or_else(|| panic!("the tube burned no core: {:?}", dl.cmds()));
        let (core, stroke) = core;
        assert_eq!(stroke, A_BORDER, "the core is not the border it is made of");
        for (got, was, ch) in
            [(core.r, edge.r, 'r'), (core.g, edge.g, 'g'), (core.b, edge.b, 'b')]
        {
            assert!(got >= was - 1e-6, "the core's {ch} {got} is darker than the glass {was}");
        }
        assert!(
            core.r > edge.r + 1e-6 || core.g > edge.g + 1e-6 || core.b > edge.b + 1e-6,
            "the core {core:?} is not brighter than the glass {edge:?} anywhere"
        );
        let spread = |c: Color| c.r.max(c.g).max(c.b) - c.r.min(c.g).min(c.b);
        assert!(
            spread(core) < spread(edge) - 1e-6,
            "the core {core:?} kept the glass's full saturation; a driven pixel \
             clips toward white"
        );
        assert!((core.a - edge.a).abs() < 1e-6, "the drive moved the coverage, not the light");
    }

    /// EACH OF THE THREE SWITCHES SILENCES THE TUBE ON ITS OWN.
    ///
    /// Until 2026-08-23 the master shipped `panel_edge` disabled, at a
    /// reach of zero and an amount of zero, so naming `tube` alone drew
    /// nothing and that WAS the governing principle: a tube is a theme's
    /// decision, never a default. That guarantee is retired on purpose —
    /// the master itself is now an enabled, reached, amounted tube, which
    /// is the neon-by-default change this file's own name argues against
    /// and lost. What survives it: turn any ONE of the three off
    /// explicitly, whatever the other two say, and the glow still draws
    /// nothing.
    #[test]
    fn any_one_switch_off_silences_the_tube() {
        let edge = Color { r: 0.42, g: 0.14, b: 0.86, a: 0.9 };
        let c = [Corner::round(9.0); 4];
        let decl = "panel_edge.falloff = tube # enum: linear | quad | gauss | halo | tube\n";
        for (what, extra) in [
            (
                "disabled",
                format!(
                    "panel_edge.enabled = false\npanel_edge.radius = 2.0u\n\
                     panel_edge.alpha = 0.34\n{decl}"
                ),
            ),
            (
                "no reach",
                format!(
                    "panel_edge.enabled = true\npanel_edge.radius = 0u\n\
                     panel_edge.alpha = 0.34\n{decl}"
                ),
            ),
            (
                "no amount",
                format!(
                    "panel_edge.enabled = true\npanel_edge.radius = 2.0u\n\
                     panel_edge.alpha = 0.0\n{decl}"
                ),
            ),
        ] {
            let t = theme::bake_over_master(&format!("[glow]\n{extra}"));
            let mut dl = DrawList::recording();
            panel_edge_glow(&mut dl, &t, box_(), &c, 6, edge, A_BORDER, AT_REST);
            assert!(
                dl.cmds().is_empty() && dl.verts.is_empty(),
                "{what} drew {:?}",
                dl.cmds()
            );
        }
    }

    /// LINE, GLOW AND NEON ARE THREE PICTURES.
    ///
    /// Driven from the EDITOR'S OWN MODEL rather than from hand-written
    /// token text: each kind's edit set is turned into an overlay, baked,
    /// and drawn. That is what makes this a proof about the three names
    /// the owner sees in a list — a kind that wrote the wrong word, or
    /// wrote nothing, fails here and not in a review.
    ///
    /// Each step, not just the ends. LINE to GLOW is the light arriving;
    /// GLOW to NEON is the same light spent differently, and it is the
    /// step a rename could have left standing still.
    #[test]
    fn the_three_border_kinds_are_three_pictures() {
        use crate::theme::edit::{border_edits, Border, Scope};
        use crate::theme::color::Oklch;
        let colour = Oklch { l: 0.62, c: 0.19, h: 285.0, alpha: 1.0 };
        let edge = Color { r: 0.42, g: 0.14, b: 0.86, a: 0.9 };
        let c = [Corner::round(9.0); 4];
        let dump = |kind: Border| {
            let mut glow = String::new();
            let mut elev = String::new();
            let mut border = String::new();
            for e in border_edits(Scope::Theme, kind, colour, false) {
                // The falloff carries its declaration back, for the same
                // reason `a_tube_theme` does: an overlay re-declares the
                // token, a saved file patches a value span and does not.
                let tail = if e.token.ends_with("falloff") {
                    " # enum: linear | quad | gauss | halo | tube"
                } else {
                    ""
                };
                if let Some(k) = e.token.strip_prefix("glow.") {
                    glow.push_str(&format!("{k} = {}{tail}\n", e.value));
                } else if let Some(k) = e.token.strip_prefix("elev.panel.") {
                    elev.push_str(&format!("{k} = {}\n", e.value));
                } else if let Some(k) = e.token.strip_prefix("border.") {
                    // The colour now lands on the shared root `border.default`
                    // (a `[border]` token), not the `elev.panel` leaf. It
                    // does not reach this picture — `panel_edge_glow` takes
                    // its `edge` as a parameter — but the overlay must still
                    // bake the whole edit set the kind wrote.
                    border.push_str(&format!("{k} = {}\n", e.value));
                } else {
                    panic!("a border kind wrote {}, which this proof cannot bake", e.token);
                }
            }
            let t = theme::bake_over_master(&format!(
                "[border]\n{border}[elev.panel]\n{elev}[glow]\n{glow}"
            ));
            let mut dl = DrawList::recording();
            panel_edge_glow(&mut dl, &t, box_(), &c, 6, edge, A_BORDER, AT_REST);
            (
                dl.cmds().iter().map(|c| c.to_string()).collect::<Vec<_>>().join("\n"),
                dl.verts.iter().map(|v| (v.pos, v.uv, v.color)).collect::<Vec<_>>(),
            )
        };
        let line = dump(Border::Line);
        let glow = dump(Border::Glow);
        let neon = dump(Border::Neon);
        assert!(line.0.is_empty(), "LINE lit something: {}", line.0);
        assert_ne!(glow.0, line.0, "GLOW drew what LINE draws");
        assert_ne!(neon.0, glow.0, "NEON drew what GLOW draws");
        assert_ne!(neon.1, glow.1, "NEON and GLOW put their vertices in the same place");
    }
}
