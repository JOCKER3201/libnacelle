//! Window objects: the dimmed backdrop and the window frame.
//!
//! The frame takes its geometry from the theme as well as its colour — how
//! thick a border is, how far its corners are cut — so two themes can differ
//! in shape and not only in hue. There is no fallback underneath any read:
//! a missing token degrades through the engine's per-kind default and is
//! allowed to look raw, which is what keeps every design decision in the
//! theme files.

use crate::access::{AccessInfo, Role};
use crate::draw::{Corner, CornerStyle};
use crate::focus::FocusId;
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
    use crate::font::FontSystem;
    use crate::object::elev::tests::{same_picture, AT_REST};

    /// The box every proof below draws into. Any box would do — what is
    /// read off it is which COMMANDS a frame emits and in which colours,
    /// never where a particular vertex landed.
    fn box_() -> Rect {
        Rect::new(30.0, 18.0, 240.0, 150.0)
    }

    /// What this file drew before it joined the ladder: the body of the
    /// old `frame`, transcribed statement for statement — the glass
    /// branch, the fill under it, and the ring.
    ///
    /// Its OWN transcript, and not the one `menu.rs` and `tooltip.rs`
    /// share ([`crate::object::elev::tests::the_private_copy`]), because
    /// a window's copy was never their copy. Theirs departed from the
    /// rung in TWO places — the body drawn whatever its alpha, the ring
    /// drawn on the width alone — and a window's also stroked its ring
    /// whatever the EDGE's alpha. Borrowing their transcript would have
    /// made this file's no-move proof a proof about a picture it never
    /// drew. (The edge bloom the transcript used to lay after the ring
    /// is gone with the whole panel-edge effect, 2026-08-27, the owner's
    /// order — from the transcript and from the ladder both, so the
    /// comparison still reads picture against picture.)
    fn the_frames_private_copy(dl: &mut DrawList, t: &theme::ResolvedTheme, r: Rect) {
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
    }

    /// The no-move proof, in the words `menu.rs` and `tooltip.rs` already
    /// use: a window frame is a surface of Elev 2, and joining the ladder
    /// had to leave the picture exactly where it was under the master.
    /// Compared against [`the_frames_private_copy`], command for command
    /// and vertex for vertex.
    ///
    /// Under the master ALONE, which is half the claim and the weaker
    /// half: the master leaves `elev.panel.glass.rank` at 0, so the glass
    /// branch is not reached at all.
    /// [`joining_the_ladder_moved_no_pixel_with_the_glass_lit`] is where
    /// it is.
    #[test]
    fn joining_the_ladder_moved_no_pixel() {
        let t = theme::resolved();
        let mut was = DrawList::recording();
        the_frames_private_copy(&mut was, t, box_());
        let mut now = DrawList::recording();
        level().draw_in(&mut now, t, box_(), box_(), AT_REST);
        same_picture(&was, &now);
    }

    /// The same proof where the master cannot make it: the glass branch,
    /// taken over a theme that raises the rung's glass rank, and the two
    /// lists still have to agree.
    ///
    /// The command is asserted present first, because two pictures that
    /// agree by both being empty prove nothing.
    #[test]
    fn joining_the_ladder_moved_no_pixel_with_the_glass_lit() {
        let t = theme::bake_over_master(
            "[elev.panel]\n\
             glass.rank = 2\n\
             glass.wash = #40FFC0 / 0.5\n",
        );
        let mut was = DrawList::recording();
        the_frames_private_copy(&mut was, &t, box_());
        let has = |dl: &DrawList, what: fn(&DrawCmd) -> bool| dl.cmds().iter().any(what);
        assert!(
            has(&was, |c| matches!(c, DrawCmd::GlassFill { .. })),
            "the raised rank drew no glass, so this proves nothing: {:?}",
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
        // Filtered to the GRADIENT ring specifically.
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

}
