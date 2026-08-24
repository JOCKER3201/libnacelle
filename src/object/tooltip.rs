//! Tooltip (F2 §8.1): the label that explains what the pointer is
//! resting on, after it has rested long enough to be asking.
//!
//! One manager per application, drawn LAST — the menu's rule, for the
//! menu's reason: the draw list is immediate and draw order is z-order,
//! so a tooltip drawn anywhere else would sit under whatever came after
//! it.
//!
//! There is no registry of hover-able rectangles. Whoever owns a rect
//! already knows where it is and whether the pointer is inside it, so it
//! files a [`Tooltips::request`] while it draws and the manager decides
//! — which target the pointer has actually settled on, whether the delay
//! has elapsed, and where the box fits on screen. Two requests in one
//! frame are answered by the LAST one: it was drawn later, so it is on
//! top, so it is the one under the pointer.
//!
//! Everything visual comes from `[tooltip]` and `component.tooltip.*`;
//! the module holds no literal of its own.
//!
//! # There is no `motion.tooltip_*`, and there will not be
//!
//! This header used to promise one. §5.22's catalogue is CLOSED — its own
//! prohibition list ends with "new effect ids (an unknown id is reported
//! and ignored)" — so the promise was for a token that would have had to
//! be argued into the master, and the argument does not hold up:
//!
//! * The catalogue names EVENTS, not objects. It has `hover`, not
//!   `button_hover` and `row_hover`; `menu_unfold`, read by the context
//!   menu and the drop-down alike. A tooltip appearing is a small window
//!   appearing, which is `motion.window_open`, and a tooltip going away is
//!   `motion.window_close`. Those are the two ids this object binds to.
//! * Four ids for one event is four durations a theme can put out of step.
//!   The first author who shortened `window_open` and not `tooltip_in`
//!   would have a desktop whose windows and tooltips appear at different
//!   speeds, having changed one number and been given two behaviours.
//! * The DELAY before a tooltip appears was never motion. It is
//!   `tooltip.delay_ms` and `tooltip.linger_ms` in `[tooltip]`, both read
//!   in [`Tooltips::step`], and neither is a curve: they decide WHETHER
//!   the box is due, not how it gets there.
//!
//! The seam is [`crate::object::winframe::present`], which is where the
//! two ids are read. What is not done yet is this file drawing through it,
//! and it is a real piece of work rather than a line: a tooltip follows
//! the pointer, so the box moves every frame and cannot be the fade's key
//! — the ANCHOR is what stays still while the pointer rests on it — and
//! fading OUT means keeping the last box and its laid-out lines for as
//! long as the gate takes to fall, which [`Tooltips`] does not remember
//! today. Until then appearing is instantaneous, which is honest rather
//! than half-animated.

use crate::focus::FocusId;
use crate::theme::{self, Color, TokenId};
use crate::{ui, Ctx, Rect};
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::OnceLock;

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// The engine's colour, in the draw list's clothes.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// An id for a target named by a string — a table heading, a tab label.
///
/// Callers with a natural number (a row index, a widget handle) should
/// use it directly; this is for the ones whose only stable name is text.
pub fn key(name: &str) -> u64 {
    let mut h = DefaultHasher::new();
    name.hash(&mut h);
    h.finish()
}

/// An id for one cell of a view: which view drew it, which column it is,
/// and the row it belongs to (empty for a heading).
///
/// A table's cells cannot be named by their text — two rows may hold the
/// same word, and the text is exactly what changes when the model is
/// rewritten — so the identity is the PLACE, and the row's key is the
/// part of the place that survives a sort.
pub fn cell_key(view: u32, col: usize, row: &str) -> u64 {
    let mut h = DefaultHasher::new();
    view.hash(&mut h);
    col.hash(&mut h);
    row.hash(&mut h);
    h.finish()
}

/// What a requester asked for this frame.
struct Pending {
    id: u64,
    anchor: Rect,
    text: String,
    /// When the pointer was found inside `anchor` — the caller's clock,
    /// so a caller drawing at a different time than the manager still
    /// measures one delay.
    t: f64,
}

/// The target the pointer is currently resting on.
struct Armed {
    id: u64,
    /// When it was reached.
    since: f64,
    /// Whether the delay is being skipped because the pointer stepped
    /// here straight off another explained target (`tooltip.linger_ms`).
    instant: bool,
}

/// The application's one tooltip manager.
#[derive(Default)]
pub struct Tooltips {
    armed: Option<Armed>,
    pending: Option<Pending>,
    /// The last moment a tooltip was actually on screen. The grace
    /// window that lets the next neighbour skip the delay is measured
    /// from here, so walking along a row of controls explains each one
    /// without a pause between them.
    last_shown: Option<f64>,
    /// What the last [`Tooltips::draw`] put on screen, for a caller
    /// that needs to know whether the pointer is over a tooltip.
    rect: Option<Rect>,
    /// The text in that box. Kept beside the rectangle because the draw
    /// list holds glyph quads and not words: without it, nothing outside
    /// this module can say WHICH explanation reached the screen.
    shown: Option<String>,
}

impl Tooltips {
    pub fn new() -> Tooltips {
        Tooltips::default()
    }

    /// Files a request while drawing, when the pointer is inside
    /// `anchor`. Empty text is not a request — a cell with nothing more
    /// to say than what is already drawn says nothing.
    pub fn request(&mut self, id: u64, anchor: Rect, text: &str, t: f64) {
        if text.is_empty() {
            return;
        }
        self.pending = Some(Pending { id, anchor, text: text.to_string(), t });
    }

    /// [`Tooltips::request`] with the pointer test done here — the form
    /// almost every caller wants, since almost every caller has a `Ctx`
    /// in hand and a rect it has just drawn.
    pub fn hover(&mut self, ctx: &Ctx, id: u64, anchor: Rect, text: &str) {
        if ctx.mouse.over(anchor) {
            self.request(id, anchor, text, ctx.t);
        }
    }

    /// Drops everything: no request stands, nothing is armed, and the
    /// next target pays the full delay. For the moments a tooltip must
    /// not survive — a menu opening over it, a window closing.
    pub fn clear(&mut self) {
        self.armed = None;
        self.pending = None;
        self.last_shown = None;
        self.rect = None;
        self.shown = None;
    }

    /// The box drawn by the last [`Tooltips::draw`], if any.
    pub fn rect(&self) -> Option<Rect> {
        self.rect
    }

    /// The text in that box — what the tooltip actually SAID.
    pub fn shown(&self) -> Option<&str> {
        self.shown.as_deref()
    }

    /// The tooltip text currently on screen FOR THE CONTROL `id` names —
    /// the accessible DESCRIPTION half of the name/description split
    /// [`crate::access::AccessInfo`] draws: words that enrich an
    /// ALREADY-focusable control, not a second name and not a second
    /// Tab stop (this file's header explains why a tooltip is neither).
    ///
    /// `id` is [`FocusId`]'s own `u64` — nothing here hashes a name or
    /// walks a registry, because there isn't one (see the header again:
    /// "There is no registry of hover-able rectangles"). A control is
    /// found only if it files its [`Tooltips::request`] /
    /// [`Tooltips::hover`] with `id.0`, the very value it already hands
    /// `FocusCtl::register` — a convention this file can honour but not
    /// enforce from here.
    ///
    /// Answers from the same two fields that already back
    /// [`Tooltips::rect`] and [`Tooltips::shown`], not a new registry:
    /// `armed` is WHICH target the pointer is resting on, `shown` is
    /// the words that target is due, and the two agree exactly while a
    /// box is visible (see [`Tooltips::step`]) — so requiring both is
    /// how a caller here is told apart from every other control, with
    /// nothing added to remember. A control still waiting out
    /// `tooltip.delay_ms`, or one the pointer has left, has nothing to
    /// report yet, same as it has nothing on screen yet.
    pub fn description_of(&self, id: FocusId) -> Option<&str> {
        match (&self.armed, &self.shown) {
            (Some(a), Some(text)) if a.id == id.0 => Some(text.as_str()),
            _ => None,
        }
    }

    // FOLLOW-UP, out of THIS file's scope: `description_of` only reads
    // as far as `Tooltips` itself goes. Landing its answer in an actual
    // AT-SPI description means two things neither belongs here:
    //   1. `crate::access::AccessInfo` gaining a `description: Option<String>`
    //      field alongside `name` — a shape change to a type this file
    //      does not own, and the foundation pass's call, not this one's.
    //   2. Each control that both calls `FocusCtl::register` AND wants a
    //      spoken description passing its `FocusId.0` as the id it hands
    //      `Tooltips::request`/`Tooltips::hover` (today's callers mostly
    //      use `tooltip::key`/`cell_key` string hashes instead, which do
    //      NOT line up with any `FocusId`), then reading `description_of`
    //      back with that same id when it builds its `AccessInfo` — i.e.
    //      touching every such widget's own file, not this one.

    /// The rung a tooltip is a surface of, dressed in the tooltip's own
    /// key names.
    ///
    /// `[elev.popover]` is Elev 5, whose gloss names a tooltip in the
    /// master itself. What the tooltip states for itself is exactly what
    /// it stated before it joined the ladder — the same five tokens the
    /// old private copy read — so joining moved no pixel; what it gains is
    /// everything the rung says and the object never could: the glass pair
    /// (`elev.popover.glass.*`, rank 0 in the master, so nothing is drawn
    /// today), the panel-edge bloom, and every key the ladder grows next.
    fn level() -> &'static super::elev::Level {
        static LEVEL: OnceLock<super::elev::Level> = OnceLock::new();
        LEVEL.get_or_init(|| {
            super::elev::Level::of("elev.popover").worn_as(
                "component.tooltip.fill",
                "tooltip.corner_mode",
                "tooltip.corner",
                "component.tooltip.edge",
                "tooltip.border",
            )
        })
    }

    /// The frame's decision, as arithmetic: which request (if any) is
    /// due to be shown, given the two themed times in milliseconds.
    ///
    /// Separated from the drawing so the delay, the disarming and the
    /// grace window can be tested without a window, a font or a theme —
    /// and `self.shown` is kept current HERE, for the same reason:
    /// [`Tooltips::description_of`] answers from it too, and must be
    /// exercisable by the same light-weight harness rather than needing
    /// a real [`Ctx`] just to learn WHO the pointer is resting on.
    fn step(&mut self, now: f64, delay_ms: f32, linger_ms: f32) -> Option<(Rect, String)> {
        let Some(p) = self.pending.take() else {
            // Nothing asked this frame: the pointer left every anchor.
            self.armed = None;
            self.shown = None;
            return None;
        };
        let fresh = match &self.armed {
            Some(a) if a.id == p.id => false,
            _ => true,
        };
        if fresh {
            let instant = self
                .last_shown
                .is_some_and(|t0| (now - t0) * 1000.0 <= linger_ms as f64);
            self.armed = Some(Armed { id: p.id, since: p.t, instant });
        }
        let Some(a) = self.armed.as_ref() else {
            self.shown = None;
            return None;
        };
        let due = a.instant || (now - a.since) * 1000.0 >= delay_ms as f64;
        if !due {
            self.shown = None;
            return None;
        }
        self.last_shown = Some(now);
        self.shown = Some(p.text.clone());
        Some((p.anchor, p.text))
    }

    /// End of frame: shows the request whose pointer has rested long
    /// enough, near the pointer, flipped to stay on screen.
    pub fn draw(&mut self, ctx: &mut Ctx) {
        static DELAY: OnceLock<TokenId> = OnceLock::new();
        static LINGER: OnceLock<TokenId> = OnceLock::new();
        static H: OnceLock<TokenId> = OnceLock::new();
        static PAD_X: OnceLock<TokenId> = OnceLock::new();
        static PAD_Y: OnceLock<TokenId> = OnceLock::new();
        static OFFSET: OnceLock<TokenId> = OnceLock::new();
        static MAX_W: OnceLock<TokenId> = OnceLock::new();
        static ROLE: OnceLock<TokenId> = OnceLock::new();
        static INK: OnceLock<TokenId> = OnceLock::new();

        let t = theme::resolved();
        let delay = t.px(tok(&DELAY, "tooltip.delay_ms"));
        let linger = t.px(tok(&LINGER, "tooltip.linger_ms"));
        self.rect = None;
        // `self.shown` is `step`'s to keep current (see its own doc
        // comment) — nothing to reset here before calling it.
        let Some((anchor, text)) = self.step(ctx.t, delay, linger) else { return };

        // ---- metrics ----------------------------------------------------
        let pad_x = t.px(tok(&PAD_X, "tooltip.pad_x")).max(0.0);
        let pad_y = t.px(tok(&PAD_Y, "tooltip.pad_y")).max(0.0);
        let offset = t.px(tok(&OFFSET, "tooltip.offset")).max(0.0);
        let max_w = t.px(tok(&MAX_W, "tooltip.max_w")).max(0.0);
        let min_h = t.px(tok(&H, "tooltip.h")).max(0.0);
        let role = ui::bound_role(&ROLE, "tooltip.role");
        // No `ui_font_scale`: the viewport carries the user's scale into u,
        // and the role's size is written in u — applying it here too squares it.
        let px = role.px(ctx, 1.0);
        let track = role.tracking_px(px);
        let leading = role.leading();
        // `tooltip.role`'s own face: the box wraps, measures and draws in
        // one family, which it could not while the wrap read the role and
        // the measure wrote FONT_UI.
        let face = role.font();
        // …and `tooltip.role`'s figure box with it. The box has to reach
        // all THREE of the readings below — the break, the width and the
        // draw — or the box wraps to one ruler and paints to another: a
        // tooltip is where a trimmed address or version number is
        // finally read whole, and those are exactly the runs a boxed
        // role widens. Read once, before the lines exist.
        let tabular = role.tabular();
        let fig = role.figures(ctx.fonts, face, px);

        // ---- the lines --------------------------------------------------
        let lines = ui::wrap_text_tab(ctx, face, px, &text, max_w, track, tabular);
        let mut text_w: f32 = 0.0;
        for l in &lines {
            text_w = text_w.max(ctx.fonts.measure_fig(face, px, l, track, &fig));
        }
        let line_h = px * leading;
        let block_h = line_h * lines.len() as f32;
        let w = text_w + 2.0 * pad_x;
        // `tooltip.h` is the box's MINIMUM: one line is the height the
        // theme wrote, and every line after it grows the box.
        let h = (block_h + 2.0 * pad_y).max(min_h);

        // ---- place ------------------------------------------------------
        // PLACEMENT, so the device position: the box opens on whichever
        // side of the cursor it fits, and the cursor is where the cursor
        // is. What may be explained at all was decided by the hover that
        // filed the request.
        let (x, y) = place(ctx.mouse.raw(), anchor, (w, h), offset, (ctx.w, ctx.h));
        let r = Rect::new(x, y, w, h);
        self.rect = Some(r);
        // `self.shown` already holds `text` — `step` set it.

        // ---- the box ----------------------------------------------------
        // Elev 5, the popover rung — a tooltip is one of the four surfaces
        // the master's own `[elev.popover]` gloss names ("menu, tooltip,
        // context menu, drag ghost"), and until 2026-08-17 it was the only
        // kind of surface in the toolkit that stood outside the ladder
        // altogether: no glass, no shadow, no rung. It wore its own copy of
        // the rules instead, which is the drift `elev.rs`'s header is about.
        //
        // The body, cut and ring stay on the tooltip's OWN keys, so the
        // picture does not move: a tooltip is the same floating chrome a
        // menu is (the master points `tooltip.corner_mode` at the menu's
        // rather than letting two boxes that appear side by side answer
        // differently) but it keeps its own tighter radius.
        Self::level().draw(ctx, r);

        // ---- the text ---------------------------------------------------
        let ink = col(t.color(tok(&INK, "component.tooltip.text")));
        let mut ty = r.y + (h - block_h) / 2.0;
        for l in &lines {
            ctx.dl.text_fig(ctx.fonts, face, px, r.x + pad_x, ty, l, ink, track, &fig);
            ty += line_h;
        }
    }
}

/// Where a tooltip of `size` lands for a pointer at `at` explaining
/// `anchor`: below and to the right of the pointer by `offset`, flipped
/// when there is no room, clamped to the window as a last resort.
///
/// Two departures from the menu's [`super::menu`] placement, both
/// because a tooltip explains something that must stay visible:
///
/// * the flip keeps the gap on the far side too, instead of putting the
///   box's far edge on the point — a tooltip under the cursor it is
///   explaining is a tooltip in the way;
/// * flipping UP goes above the ANCHOR, not above the pointer, so the
///   target the user is pointing at is not the thing that gets covered.
fn place(
    at: (f32, f32),
    anchor: Rect,
    size: (f32, f32),
    offset: f32,
    win: (f32, f32),
) -> (f32, f32) {
    let x = if at.0 + offset + size.0 <= win.0 {
        at.0 + offset
    } else if at.0 - offset - size.0 >= 0.0 {
        at.0 - offset - size.0
    } else {
        (win.0 - size.0).max(0.0)
    };
    let y = if at.1 + offset + size.1 <= win.1 {
        at.1 + offset
    } else if anchor.y - offset - size.1 >= 0.0 {
        anchor.y - offset - size.1
    } else {
        (win.1 - size.1).max(0.0)
    };
    (x, y)
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    // The face probe of this batch, written once in the field's own
    // file and used by all three of its objects.
    use crate::object::text_input::tests::{
        all_in, drawn_runs, face_follows_the_theme, measure_in_child, report, role_word,
    };
    use crate::draw::DrawCmd;

    /// USTERKA 3, the no-move proof — the tooltip's half of the claim
    /// `menu.rs` makes in the same words. A tooltip is a surface of Elev 5
    /// since 2026-08-17; joining the ladder had to leave the picture
    /// exactly where it was under the master, and this compares the rung
    /// against the private copy it replaced, command for command and
    /// vertex for vertex.
    ///
    /// Conditional, and deliberately so, on what the master ships:
    /// `elev.popover.glass.rank = 0` and `glow.panel_edge.enabled =
    /// false`. A theme that raises either is MEANT to move the tooltip —
    /// that it now can is the whole of what joining bought.
    #[test]
    fn joining_the_ladder_moved_no_pixel() {
        use crate::draw::DrawList;
        use crate::object::elev::tests::{same_picture, the_private_copy, AT_REST};
        let t = theme::resolved();
        let r = Rect::new(40.0, 25.0, 180.0, 30.0);
        let mut was = DrawList::recording();
        the_private_copy(
            &mut was,
            t,
            r,
            "component.tooltip.fill",
            "tooltip.corner_mode",
            "tooltip.corner",
            "component.tooltip.edge",
            "tooltip.border",
        );
        let mut now = DrawList::recording();
        Tooltips::level().draw_in(&mut now, t, r, r, AT_REST);
        same_picture(&was, &now);
    }

    const DELAY: f32 = 600.0;
    const LINGER: f32 = 120.0;

    fn anchor() -> Rect {
        Rect::new(10.0, 10.0, 100.0, 20.0)
    }

    /// One frame: file a request (or not) and take the decision.
    fn frame(
        tips: &mut Tooltips,
        now: f64,
        req: Option<(u64, &str)>,
    ) -> Option<(Rect, String)> {
        if let Some((id, text)) = req {
            tips.request(id, anchor(), text, now);
        }
        tips.step(now, DELAY, LINGER)
    }

    // ---- placement ----

    #[test]
    fn placement_keeps_its_gap_on_whichever_side_it_lands() {
        let win = (500.0, 300.0);
        let a = Rect::new(0.0, 270.0, 100.0, 20.0);
        // Room everywhere: down and right of the pointer, by the offset.
        assert_eq!(place((10.0, 20.0), a, (100.0, 40.0), 5.0, win), (15.0, 25.0));
        // No room on the right: the box's RIGHT edge keeps the gap.
        assert_eq!(place((450.0, 20.0), a, (100.0, 40.0), 5.0, win), (345.0, 25.0));
        // No room below: above the ANCHOR (270), not above the pointer.
        assert_eq!(place((10.0, 280.0), a, (100.0, 40.0), 5.0, win), (15.0, 225.0));
        // A box wider than the window: clamped, never negative.
        assert_eq!(place((10.0, 20.0), a, (900.0, 40.0), 5.0, win), (0.0, 25.0));
        // No room anywhere on the vertical: clamped to the window.
        let tall = Rect::new(0.0, 10.0, 100.0, 20.0);
        assert_eq!(place((10.0, 280.0), tall, (100.0, 40.0), 5.0, win), (15.0, 260.0));
    }

    // ---- the delay ----

    #[test]
    fn nothing_shows_before_the_delay_has_elapsed() {
        let mut tips = Tooltips::new();
        assert!(frame(&mut tips, 0.0, Some((1, "CPU"))).is_none());
        assert!(frame(&mut tips, 0.3, Some((1, "CPU"))).is_none());
        // 600 ms exactly: due.
        let out = frame(&mut tips, 0.6, Some((1, "CPU"))).expect("due at the delay");
        assert_eq!(out.1, "CPU");
        // The anchor comes back untouched — the box is placed against it.
        assert_eq!((out.0.x, out.0.y, out.0.w, out.0.h), (10.0, 10.0, 100.0, 20.0));
    }

    #[test]
    fn leaving_the_anchor_disarms_and_the_next_rest_pays_again() {
        let mut tips = Tooltips::new();
        frame(&mut tips, 0.0, Some((1, "CPU")));
        // The pointer leaves: no request at all, for long enough that
        // the grace window has closed too.
        assert!(frame(&mut tips, 0.4, None).is_none());
        assert!(frame(&mut tips, 2.0, None).is_none());
        // Back on the same target: the clock starts again.
        assert!(frame(&mut tips, 2.1, Some((1, "CPU"))).is_none());
        assert!(frame(&mut tips, 2.6, Some((1, "CPU"))).is_none());
        assert!(frame(&mut tips, 2.71, Some((1, "CPU"))).is_some());
    }

    #[test]
    fn a_new_target_restarts_the_delay_when_nothing_was_shown() {
        let mut tips = Tooltips::new();
        frame(&mut tips, 0.0, Some((1, "CPU")));
        frame(&mut tips, 0.5, Some((1, "CPU")));
        // Straight onto a neighbour before the first ever appeared:
        // there is no grace to inherit, so the delay is paid in full.
        assert!(frame(&mut tips, 0.5, Some((2, "RAM"))).is_none());
        assert!(frame(&mut tips, 1.0, Some((2, "RAM"))).is_none());
        assert!(frame(&mut tips, 1.11, Some((2, "RAM"))).is_some());
    }

    // ---- the grace window ----

    #[test]
    fn a_neighbour_reached_within_the_grace_window_shows_at_once() {
        let mut tips = Tooltips::new();
        frame(&mut tips, 0.0, Some((1, "CPU")));
        assert!(frame(&mut tips, 0.6, Some((1, "CPU"))).is_some());
        // 100 ms later, onto the next control: no second wait.
        let out = frame(&mut tips, 0.7, Some((2, "RAM"))).expect("within linger");
        assert_eq!(out.1, "RAM");
    }

    #[test]
    fn past_the_grace_window_the_neighbour_waits_like_the_first() {
        let mut tips = Tooltips::new();
        frame(&mut tips, 0.0, Some((1, "CPU")));
        assert!(frame(&mut tips, 0.6, Some((1, "CPU"))).is_some());
        // Pointer wandered over nothing for a while, then a neighbour.
        assert!(frame(&mut tips, 0.8, None).is_none());
        assert!(frame(&mut tips, 0.9, Some((2, "RAM"))).is_none());
        assert!(frame(&mut tips, 1.51, Some((2, "RAM"))).is_some());
    }

    #[test]
    fn clear_forgets_the_grace_window_too() {
        let mut tips = Tooltips::new();
        frame(&mut tips, 0.0, Some((1, "CPU")));
        assert!(frame(&mut tips, 0.6, Some((1, "CPU"))).is_some());
        tips.clear();
        assert!(frame(&mut tips, 0.65, Some((2, "RAM"))).is_none());
    }

    // ---- requests ----

    #[test]
    fn empty_text_is_not_a_request() {
        let mut tips = Tooltips::new();
        assert!(frame(&mut tips, 0.0, Some((1, ""))).is_none());
        assert!(tips.armed.is_none());
    }

    #[test]
    fn the_last_request_of_a_frame_wins() {
        let mut tips = Tooltips::new();
        tips.request(1, anchor(), "UNDER", 0.0);
        tips.request(2, Rect::new(0.0, 0.0, 5.0, 5.0), "OVER", 0.0);
        assert!(tips.step(0.0, DELAY, LINGER).is_none());
        tips.request(2, Rect::new(0.0, 0.0, 5.0, 5.0), "OVER", 0.0);
        let out = tips.step(0.6, DELAY, LINGER).expect("due");
        assert_eq!(out.1, "OVER");
    }

    // ---- who the target IS ----

    #[test]
    fn a_cells_identity_is_its_place_and_not_the_words_in_it() {
        // Two rows of one column are two targets, and the pointer moving
        // between them pays the delay again (or the grace window, which
        // is the same decision made on the id).
        assert_ne!(cell_key(1, 0, "1471"), cell_key(1, 0, "1472"));
        // The same row after a sort moved it: one target, still. The
        // identity is the ROW's key, which is what survives the reorder.
        assert_eq!(cell_key(1, 0, "1471"), cell_key(1, 0, "1471"));
        // A heading (no row) is not the cell under it, one column is not
        // its neighbour, and two views drawing the same cell are two
        // things to explain.
        assert_ne!(cell_key(1, 0, ""), cell_key(1, 0, "1471"));
        assert_ne!(cell_key(1, 0, "1471"), cell_key(1, 1, "1471"));
        assert_ne!(cell_key(1, 0, "1471"), cell_key(2, 0, "1471"));
    }

    #[test]
    fn a_target_that_keeps_its_identity_says_its_new_words_without_waiting_again() {
        // The model is rewritten under a resting pointer — a table
        // refreshes every frame, and a trimmed cell files its request
        // again with whatever it now holds. The place did not move, so
        // the delay is not paid twice and the box says the NEW text.
        let mut tips = Tooltips::new();
        let cell = cell_key(1, 2, "1471");
        assert!(frame(&mut tips, 0.0, Some((cell, "12.4 MB"))).is_none());
        let out = frame(&mut tips, 0.6, Some((cell, "12.9 MB"))).expect("due at the delay");
        assert_eq!(out.1, "12.9 MB");
        // And the row under it, reached at once, is a different target
        // with its own words.
        let next = cell_key(1, 2, "1472");
        let out = frame(&mut tips, 0.65, Some((next, "907 kB"))).expect("within linger");
        assert_eq!(out.1, "907 kB");
    }

    #[test]
    fn hover_files_only_when_the_pointer_is_inside() {
        // The pointer test is `Rect::contains`; the ids are the caller's.
        assert!(anchor().contains(11.0, 11.0));
        assert!(!anchor().contains(9.0, 11.0));
        assert_ne!(key("CPU"), key("RAM"));
        assert_eq!(key("CPU"), key("CPU"));
    }

    // ---- description_of: the accessible-description lookup -----------

    #[test]
    fn description_of_answers_nothing_before_a_target_is_shown() {
        let mut tips = Tooltips::new();
        let id = FocusId(1);
        // Nothing was ever filed for `id`.
        assert!(tips.description_of(id).is_none());
        // Filed, but the delay has not elapsed yet: armed, not shown.
        frame(&mut tips, 0.0, Some((1, "CPU load")));
        assert!(tips.description_of(id).is_none());
    }

    #[test]
    fn description_of_answers_the_target_actually_shown() {
        let mut tips = Tooltips::new();
        let id = FocusId(1);
        frame(&mut tips, 0.0, Some((1, "CPU load")));
        let out = frame(&mut tips, 0.6, Some((1, "CPU load"))).expect("due at the delay");
        assert_eq!(out.1, "CPU load");
        assert_eq!(tips.description_of(id), Some("CPU load"));
        // A different control's `FocusId` finds nothing, even while
        // this one is on screen — `description_of` is not a registry
        // of everything ever explained, only of what is showing now.
        assert!(tips.description_of(FocusId(2)).is_none());
    }

    #[test]
    fn description_of_follows_a_targets_refiled_words() {
        // Same story as `a_target_that_keeps_its_identity_says_its_new_words…`
        // above, read back through `description_of` instead of `step`'s
        // own return value.
        let mut tips = Tooltips::new();
        let id = FocusId(1);
        frame(&mut tips, 0.0, Some((1, "12.4 MB")));
        frame(&mut tips, 0.6, Some((1, "12.4 MB")));
        assert_eq!(tips.description_of(id), Some("12.4 MB"));
        frame(&mut tips, 0.65, Some((1, "12.9 MB")));
        assert_eq!(tips.description_of(id), Some("12.9 MB"));
    }

    #[test]
    fn description_of_forgets_a_target_the_pointer_has_left() {
        let mut tips = Tooltips::new();
        let id = FocusId(1);
        frame(&mut tips, 0.0, Some((1, "CPU load")));
        frame(&mut tips, 0.6, Some((1, "CPU load")));
        assert!(tips.description_of(id).is_some());
        // The pointer leaves every anchor: no request at all this frame.
        frame(&mut tips, 0.65, None);
        assert!(tips.description_of(id).is_none());
    }

    #[test]
    fn description_of_clears_with_everything_else() {
        let mut tips = Tooltips::new();
        let id = FocusId(1);
        frame(&mut tips, 0.0, Some((1, "CPU load")));
        frame(&mut tips, 0.6, Some((1, "CPU load")));
        assert!(tips.description_of(id).is_some());
        tips.clear();
        assert!(tips.description_of(id).is_none());
    }

    // ---- the type ladder reaches the tooltip -------------------------
    //
    // The box wraps, measures and draws in ONE family, and that family
    // is `tooltip.role`'s `face`. The draw above has read the role since
    // the type ladder landed; what it did not have is a MEASUREMENT,
    // and "it reads the role" is a claim a `FONT_UI` written at the call
    // site satisfies just as well for as long as the master happens to
    // point the role at the interface face. So the claim is put to a
    // theme that points it somewhere else, in a process of its own —
    // the harness is the field's, one definition of proof for the three
    // objects of this batch.

    /// Long enough to WRAP: every line is a text call of its own, so a
    /// wrap that ruled in one face and a draw that inked in another
    /// would show up here as two slots in one box.
    ///
    /// Mostly FIGURES, because the second claim below is about the box
    /// they step by: on a text of letters a break ruled without the box
    /// and drawn with it would overrun by a hair, and a hair is a thing
    /// a test can miss. Addresses are what a trimmed cell holds anyway —
    /// they are why the tooltip exists.
    const TIP: &str = "192.168.000.101 10.000.000.255 172.016.254.001 unreachable since \
                       00:00:01 — 255.255.255.000 answered 11.011.011.011 instead";

    /// The tooltip's text is set in the face `tooltip.role` names, and
    /// follows a theme that moves it.
    #[test]
    fn the_box_is_set_in_the_face_its_role_names() {
        face_follows_the_theme("tooltip", "object::tooltip::tests::child_tip_face");
    }

    /// The figure box reaches all THREE readings this object makes of
    /// one string: the break, the width of the box that holds it, and
    /// the draw. `type.<tooltip.role>.tabular` is off in the master, so
    /// the advance is zero until a theme asks — the negative control —
    /// and the line that decides the claim is `OVER`: the widest line
    /// MEASURED UNDER THE BOX IT WAS DRAWN WITH, against the width the
    /// wrap was given. A break ruled proportionally and inked under a
    /// box overruns by the difference, and on a text of addresses that
    /// difference is a character wide.
    #[test]
    fn the_lines_are_broken_under_the_box_they_are_drawn_with() {
        const CHILD: &str = "object::tooltip::tests::child_tip_face";
        let master = measure_in_child(CHILD, None);
        let plain: f32 = master.field("ADVANCE=").parse().expect("ADVANCE= is a number");
        assert_eq!(
            plain, 0.0,
            "the master ships `type.{}.tabular = false` and the lines were boxed anyway",
            master.role
        );
        let path = std::env::temp_dir()
            .join(format!("nacelle-box-tooltip-{}.theme", std::process::id()));
        std::fs::write(
            &path,
            format!(
                "[meta]\nschema = 1\nname = \"box tooltip\"\nbase = \"default\"\n\n\
                 [type]\n{}.tabular = true\n",
                master.role
            ),
        )
        .expect("the fixture theme must be writable");
        let boxed = measure_in_child(CHILD, Some(&path));
        let _ = std::fs::remove_file(&path);
        let a: f32 = boxed.field("ADVANCE=").parse().expect("ADVANCE= is a number");
        assert!(
            a > 0.0,
            "a theme put `type.{}.tabular = true` and the lines were still drawn \
             proportionally:\n{}",
            master.role,
            boxed.log
        );
        let over: f32 = boxed.field("OVER=").parse().expect("OVER= is a number");
        assert!(
            over <= 0.0,
            "a line came out {over} px wider than the `tooltip.max_w` it was broken \
             to fit: the wrap ruled one way and the draw inked another\n{}",
            boxed.log
        );
    }

    /// The children of the two tests above: one tooltip that has waited
    /// out its delay, drawn for real. `drawn_runs` keeps the whole
    /// command, so the face slot, the size, the tracking and the FIGURE
    /// ADVANCE each line was made under are read from the register.
    #[test]
    #[ignore = "measured in a process of its own by the tests above"]
    fn child_tip_face() {
        let role = role_word("tooltip.role");
        let want = crate::ui::role(&role).font();
        let cmds = drawn_runs(|ctx| {
            let mut tips = Tooltips::new();
            // The pointer settled two seconds ago — the caller's clock
            // is what `request` takes — so this one frame both arms the
            // target and shows it.
            tips.request(key("cpu"), Rect::new(100.0, 100.0, 200.0, 40.0), TIP, ctx.t - 2.0);
            tips.draw(ctx);
        });
        let runs: Vec<(u8, f32, f32, f32, String)> = cmds
            .iter()
            .map(|c| match c {
                DrawCmd::Text { font, px, tracking, tabular, text, .. } => {
                    (*font, *px, *tracking, *tabular, text.clone())
                }
                _ => unreachable!("drawn_runs answers text commands"),
            })
            .collect();
        let drawn: Vec<(u8, String)> =
            runs.iter().map(|r| (r.0, r.4.clone())).collect();
        assert!(
            drawn.len() > 1,
            "the text fitted on one line, so the WRAP — the one ruler that is taken \
             apart from the draw — was never exercised: {drawn:?}"
        );
        all_in(&drawn, want);
        // Every line re-measured under the box it carries, against the
        // budget the wrap was given. The theme is this process's own, so
        // `tooltip.max_w` is the number the object worked to.
        let mut fonts = crate::font::FontSystem::new();
        let max_w = theme::resolved().px(theme::id("tooltip.max_w").unwrap_or(TokenId::MISSING));
        let mut widest = 0.0f32;
        for (font, px, track, adv, text) in &runs {
            let fig = crate::ui::figures(&mut fonts, *font, *px, *adv > 0.0);
            widest = widest.max(fonts.measure_fig(*font, *px, text, *track, &fig));
        }
        println!("ADVANCE={}", runs[0].3);
        println!("OVER={}", widest - max_w);
        report(&role, drawn[0].0, &drawn);
    }
}
