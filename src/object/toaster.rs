//! Toaster (F2 §8.2): the transient notice at the top of the screen and
//! the queue behind it.
//!
//! This is the desktop's warning popup grown up. That popup held ONE
//! message: a second warning arriving a frame later replaced the first,
//! and the user never saw what was overwritten. The queue fixes that
//! without changing what a single toast looks like — `toast.max_visible`
//! ships at 1, so the master theme draws exactly the one box it always
//! drew, in exactly the same place, and a theme that wants a stack says
//! so.
//!
//! Everything visual comes from `[toast]` and `component.toast.*`, and
//! the frame itself is [`super::window::frame`] — the same call the
//! popup made, so the port is a move rather than a redrawing.
//!
//! # There is no `motion.toast_*`, and there will not be
//!
//! This header used to promise one, and the promise is withdrawn for the
//! reason `tooltip.rs`'s header sets out at length: §5.22's catalogue is
//! CLOSED, it names EVENTS rather than objects, and a toast arriving is a
//! small window arriving. The binding is `motion.window_open` and
//! `motion.window_close`, read through
//! [`crate::object::winframe::present`] — which is the honest reading here
//! more than anywhere, since the frame this module draws IS
//! [`super::window::frame`], the same call the popup made.
//!
//! What is not done yet is this file drawing through it. The toaster has
//! the half the tooltip lacks — a toast already has a lifetime, so the
//! moment it begins to leave is known — and lacks the half the tooltip
//! has: the box is placed by the queue, so a toast leaving while the one
//! behind it moves up wants the stack's geometry settled first. Until
//! then a toast appears and disappears, which is honest rather than
//! half-animated.

use crate::theme::{self, Color, TokenId};
use crate::ui::{self, Sev};
use crate::{Ctx, Rect};
use std::collections::VecDeque;
use std::sync::OnceLock;

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// The engine's colour, in the draw list's clothes.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// One notice.
#[derive(Clone)]
pub struct Toast {
    /// The severity this notice carries, if it carries one. It colours
    /// the title through `severity.<s>.text` — the master's own comment
    /// on `component.toast.title` says as much: the toast says WARNING
    /// and `[severity]` exists for exactly this.
    pub severity: Option<Sev>,
    /// The word at the top — `"WARNING"`, `"SAVED"`. The application's
    /// vocabulary, not the theme's.
    pub title: String,
    pub body: String,
    /// When the toast first became VISIBLE, in `Ctx::t` seconds; NaN
    /// until it has been drawn once.
    ///
    /// The clock starts at the first draw rather than at the push, so a
    /// toast that waited its turn in the queue still gets its full dwell
    /// — the whole point of queueing instead of overwriting. The menu's
    /// `opened_t` uses the same sentinel for the same reason.
    born: f64,
    /// This toast's own dwell in ms; None takes `toast.dwell_ms`.
    dwell_ms: Option<f32>,
}

impl Toast {
    pub fn new(title: &str, body: &str) -> Toast {
        Toast {
            severity: None,
            title: title.to_string(),
            body: body.to_string(),
            born: f64::NAN,
            dwell_ms: None,
        }
    }

    /// The warning the desktop has always shown: the word WARNING over
    /// the message, in `component.toast.title`.
    ///
    /// The title reads `catalog.toaster.warning_title` (5.30) through the
    /// epoch-gated cache — `"WARNING"` as the fallback a theme that omits
    /// the key still draws — rather than `theme::diagnostics()` directly:
    /// this constructor is application code's own call, not a draw call,
    /// so a caller re-raising one repeating condition can run it every
    /// frame (`Toaster::push`'s own doc names exactly that caller), and an
    /// Arc clone plus a `Vec` scan on every one of those pushes is the
    /// per-frame cost 5.30 was written to avoid.
    pub fn warning(body: String) -> Toast {
        let title = ui::theme_catalog_named("catalog.toaster.warning_title", "WARNING").to_string();
        Toast { severity: None, title, body, born: f64::NAN, dwell_ms: None }
    }

    pub fn with_severity(mut self, s: Sev) -> Toast {
        self.severity = Some(s);
        self
    }

    /// Overrides `toast.dwell_ms` for this one notice.
    pub fn with_dwell_ms(mut self, ms: f32) -> Toast {
        self.dwell_ms = Some(ms);
        self
    }
}

/// The application's one toaster: what is on screen and what is waiting.
#[derive(Default)]
pub struct Toaster {
    queue: VecDeque<Toast>,
    /// The boxes the last [`Toaster::draw`] put on screen, in queue
    /// order. A click is answered against these rather than against a
    /// recomputed guess — the popup's own hit box was the minimum-width
    /// one, which missed the right end of every wider toast.
    shown: Vec<Rect>,
}

impl Toaster {
    pub fn new() -> Toaster {
        Toaster::default()
    }

    /// Queues a notice. FIFO, except that a notice identical to one
    /// already queued (same title AND body) only refreshes that one's
    /// dwell: an event repeating every frame must not build a wall of
    /// identical boxes, the same discipline `warn_once` keeps for the
    /// log.
    pub fn push(&mut self, t: Toast) {
        if let Some(dup) = self
            .queue
            .iter_mut()
            .find(|q| q.title == t.title && q.body == t.body)
        {
            // Restart the clock at the next draw: a repeat is a reason
            // to keep the notice up, not to show a second one.
            dup.born = f64::NAN;
            return;
        }
        self.queue.push_back(t);
    }

    /// Everything goes, on screen and queued.
    pub fn clear(&mut self) {
        self.queue.clear();
        self.shown.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// How many notices are on screen or waiting.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Dismisses the toast the click landed on; true when one was hit.
    pub fn click(&mut self, x: f32, y: f32) -> bool {
        let hit = self.shown.iter().position(|r| r.contains(x, y));
        match hit {
            Some(i) if i < self.queue.len() => {
                self.queue.remove(i);
                self.shown.remove(i);
                true
            }
            _ => false,
        }
    }

    /// Retires whatever has outlived its dwell and starts the clock of
    /// whatever became visible — the arithmetic of the queue, with no
    /// drawing in it, so the ageing can be tested without a window.
    ///
    /// `max_visible` is how many stand on screen at once; only those
    /// age, which is what makes the queue a queue.
    fn age(&mut self, now: f64, dwell_ms: f32, max_visible: usize) {
        let mut i = 0;
        while i < self.queue.len().min(max_visible) {
            let t = &mut self.queue[i];
            if !t.born.is_finite() {
                t.born = now;
            }
            let dwell = t.dwell_ms.unwrap_or(dwell_ms);
            if ((now - t.born) * 1000.0) as f32 > dwell {
                // The one behind moves up and is born on this frame.
                self.queue.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// Draws the visible end of the queue, stacked downwards from
    /// `toast.top`.
    pub fn draw(&mut self, ctx: &mut Ctx) {
        static DWELL: OnceLock<TokenId> = OnceLock::new();
        static MIN_W: OnceLock<TokenId> = OnceLock::new();
        static MAX_W: OnceLock<TokenId> = OnceLock::new();
        static TH: OnceLock<TokenId> = OnceLock::new();
        static TOP: OnceLock<TokenId> = OnceLock::new();
        static PAD_X: OnceLock<TokenId> = OnceLock::new();
        static TITLE_GAP: OnceLock<TokenId> = OnceLock::new();
        static MSG_GAP: OnceLock<TokenId> = OnceLock::new();
        static TITLE_C: OnceLock<TokenId> = OnceLock::new();
        static TEXT_C: OnceLock<TokenId> = OnceLock::new();
        static MAX_VISIBLE: OnceLock<TokenId> = OnceLock::new();
        static GAP: OnceLock<TokenId> = OnceLock::new();
        static TITLE_ROLE: OnceLock<TokenId> = OnceLock::new();
        static BODY_ROLE: OnceLock<TokenId> = OnceLock::new();

        let t = theme::resolved();
        // A theme silencing every toast would be a broken application,
        // not a look: one is the floor.
        let max_visible = (t.px(tok(&MAX_VISIBLE, "toast.max_visible")) as i32).max(1) as usize;
        self.age(ctx.t, t.px(tok(&DWELL, "toast.dwell_ms")), max_visible);
        self.shown.clear();
        if self.queue.is_empty() {
            return;
        }

        // ---- metrics ----------------------------------------------------
        let title_role = ui::bound_role(&TITLE_ROLE, "toast.title.role");
        let body_role = ui::bound_role(&BODY_ROLE, "toast.body.role");
        // No `ui_font_scale`: the viewport carries the user's scale into u,
        // and the role's size is written in u — applying it here too squares it.
        let px = body_role.px(ctx, 1.0);
        let title_px = title_role.px(ctx, 1.0);
        let track = body_role.tracking_px(px);
        let title_track = title_role.tracking_px(title_px);
        // Each role's own FACE and its own figure box, read once for the
        // whole queue rather than per toast: a box costs a theme read and
        // — on the first call for a (face, px) — ten glyph lookups.
        //
        // The title and the message are two roles, so they are two faces:
        // `toast.title.role` may be a medium weight over a body in the
        // interface face, which is precisely what the two bindings are
        // for, and what naming `FONT_UI` at both call sites made
        // impossible. The box goes with the face because the width below
        // is measured with it and the glyphs at the bottom of the loop
        // are stepped by it — measure in one and draw in the other and
        // the message stops being centred in the box that was sized for
        // it, but only under a theme that turns the box on.
        let title_face = title_role.font();
        let body_face = body_role.font();
        let title_fig = title_role.figures(ctx.fonts, title_face, title_px);
        let body_fig = body_role.figures(ctx.fonts, body_face, px);
        let pad_x = t.px(tok(&PAD_X, "toast.pad_x"));
        let bh = t.px(tok(&TH, "toast.h"));
        let top = t.px(tok(&TOP, "toast.top"));
        let title_gap = t.px(tok(&TITLE_GAP, "toast.title_gap"));
        let msg_gap = t.px(tok(&MSG_GAP, "toast.msg_gap"));
        let title_ink = col(t.color(tok(&TITLE_C, "component.toast.title")));
        let body_ink = col(t.color(tok(&TEXT_C, "component.toast.text")));
        // Read only when the stack is on: at max_visible = 1 there is no
        // second box for a gap to sit between.
        let gap = if max_visible > 1 { t.px(tok(&GAP, "toast.gap")) } else { 0.0 };

        let n = self.queue.len().min(max_visible);
        for i in 0..n {
            let (title, body, sev) = {
                let toast = &self.queue[i];
                (toast.title.clone(), toast.body.clone(), toast.severity)
            };
            let text_w = ctx.fonts.measure_fig(body_face, px, &body, track, &body_fig);
            let bw = (text_w + 2.0 * pad_x)
                .max(ctx.w * t.px(tok(&MIN_W, "toast.min_w_frac")))
                .min(ctx.w * t.px(tok(&MAX_W, "toast.max_w_frac")));
            let bx = (ctx.w - bw) / 2.0;
            let by = top + i as f32 * (bh + gap);
            let r = Rect::new(bx, by, bw, bh);
            self.shown.push(r);

            super::window::frame(ctx, r);
            // A toast carrying a severity says so in the title's colour;
            // one that carries none keeps the theme's toast title.
            let ink = match sev {
                Some(s) => ui::sev_text(s),
                None => title_ink,
            };
            ctx.dl.text_center_fig(
                ctx.fonts,
                title_face,
                title_px,
                bx + bw / 2.0,
                by + title_gap,
                &title,
                ink,
                title_track,
                &title_fig,
            );
            ctx.dl.text_center_fig(
                ctx.fonts,
                body_face,
                px,
                bx + bw / 2.0,
                by + msg_gap,
                &body,
                body_ink,
                track,
                &body_fig,
            );
        }
    }
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::DrawCmd;
    // The face probe of this batch, written once in the field's own
    // file and used by all three of its objects.
    use crate::object::text_input::tests::{
        all_in, drawn_runs, face_follows_the_theme, measure_in_child, report, role_word,
    };
    use std::path::PathBuf;

    const DWELL: f32 = 8000.0;

    fn t(body: &str) -> Toast {
        Toast::warning(body.to_string())
    }

    #[test]
    fn a_queued_toast_does_not_age_until_it_is_visible() {
        let mut ts = Toaster::new();
        ts.push(t("first"));
        ts.push(t("second"));
        ts.age(0.0, DWELL, 1);
        assert_eq!(ts.len(), 2);
        // Nine seconds later the first is long gone and the second has
        // only just started: it was never on screen before now.
        ts.age(9.0, DWELL, 1);
        assert_eq!(ts.len(), 1);
        assert_eq!(ts.queue[0].body, "second");
        assert_eq!(ts.queue[0].born, 9.0);
        ts.age(16.9, DWELL, 1);
        assert_eq!(ts.len(), 1);
        ts.age(17.1, DWELL, 1);
        assert!(ts.is_empty());
    }

    #[test]
    fn the_stack_ages_every_visible_toast_at_once() {
        let mut ts = Toaster::new();
        ts.push(t("a"));
        ts.push(t("b"));
        ts.push(t("c"));
        ts.age(0.0, DWELL, 3);
        assert_eq!(ts.len(), 3);
        ts.age(8.1, DWELL, 3);
        assert!(ts.is_empty());
    }

    #[test]
    fn an_identical_notice_refreshes_the_dwell_instead_of_stacking() {
        let mut ts = Toaster::new();
        ts.push(t("disk is full"));
        ts.age(0.0, DWELL, 1);
        ts.push(t("disk is full"));
        assert_eq!(ts.len(), 1);
        // The clock restarted, so the notice outlives its original dwell.
        ts.age(7.0, DWELL, 1);
        assert_eq!(ts.len(), 1);
        assert_eq!(ts.queue[0].born, 7.0);
        ts.age(14.9, DWELL, 1);
        assert_eq!(ts.len(), 1);
        ts.age(15.1, DWELL, 1);
        assert!(ts.is_empty());
    }

    #[test]
    fn a_different_body_is_a_different_notice() {
        let mut ts = Toaster::new();
        ts.push(t("one"));
        ts.push(t("two"));
        ts.push(Toast::new("SAVED", "one"));
        assert_eq!(ts.len(), 3);
    }

    #[test]
    fn a_toast_may_carry_its_own_dwell() {
        let mut ts = Toaster::new();
        ts.push(t("slow"));
        ts.push(t("quick").with_dwell_ms(500.0));
        ts.age(0.0, DWELL, 2);
        ts.age(0.6, DWELL, 2);
        assert_eq!(ts.len(), 1);
        assert_eq!(ts.queue[0].body, "slow");
    }

    #[test]
    fn a_click_dismisses_the_box_it_landed_on_and_nothing_else() {
        let mut ts = Toaster::new();
        ts.push(t("a"));
        ts.push(t("b"));
        ts.shown = vec![Rect::new(0.0, 0.0, 100.0, 20.0), Rect::new(0.0, 30.0, 100.0, 20.0)];
        assert!(!ts.click(200.0, 5.0));
        assert_eq!(ts.len(), 2);
        assert!(ts.click(50.0, 35.0));
        assert_eq!(ts.len(), 1);
        assert_eq!(ts.queue[0].body, "a");
    }

    #[test]
    fn a_click_on_nothing_drawn_hits_nothing() {
        let mut ts = Toaster::new();
        ts.push(t("a"));
        assert!(!ts.click(50.0, 5.0));
        assert_eq!(ts.len(), 1);
    }

    // ---- the type ladder reaches the toast --------------------------
    //
    // A toast is TWO roles — `toast.title.role` for the word and
    // `toast.body.role` for the message — and until now both were drawn
    // with `FONT_UI` written at the call site, so a master pointing the
    // title at a medium weight and the message at the interface face got
    // one family for both. Each is measured on its own below, because
    // one binding following its role proves nothing about the other.
    //
    // The harness is the field's: one definition of what counts as
    // proof for the whole batch, and a process of its own per run,
    // because the resolved theme is process-wide.

    /// A body that is nothing but a number and its punctuation: the
    /// string a figure box moves and a proportional run does not.
    const BODY: &str = "192.168.000.101 unreachable";

    /// A theme that inherits the master and turns ONE role's figure box
    /// on — `mono_theme`'s twin for §5.17's other half. Written here
    /// rather than beside it because it states a different claim: the
    /// face harness asks which FAMILY a run is set in, this asks
    /// whether the run's figures step by the box the role asked for.
    fn boxed_theme(tag: &str, role: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("nacelle-box-{tag}-{}.theme", std::process::id()));
        std::fs::write(
            &path,
            format!(
                "[meta]\nschema = 1\nname = \"box {tag}\"\nbase = \"default\"\n\n\
                 [type]\n{role}.tabular = true\n"
            ),
        )
        .expect("the fixture theme must be writable");
        path
    }

    /// The message is set in the face `toast.body.role` names, and
    /// follows a theme that moves it.
    #[test]
    fn the_message_is_set_in_the_face_its_role_names() {
        face_follows_the_theme("toast-body", "object::toaster::tests::child_body_face");
    }

    /// The title word is set in the face its OWN binding names — the
    /// second half of the split, and the half that shows the two are
    /// read apart.
    #[test]
    fn the_title_is_set_in_the_face_its_own_role_names() {
        face_follows_the_theme("toast-title", "object::toaster::tests::child_title_face");
    }

    /// The figure box: `type.<role>.tabular` reaches the run the message
    /// is drawn as. The master ships the message's role proportional, so
    /// the advance in the register is zero until a theme says otherwise
    /// — which is the negative control this claim needs.
    #[test]
    fn the_message_steps_by_the_box_its_role_asks_for() {
        const CHILD: &str = "object::toaster::tests::child_body_face";
        let master = measure_in_child(CHILD, None);
        let plain: f32 = master.field("ADVANCE=").parse().expect("ADVANCE= is a number");
        assert_eq!(
            plain, 0.0,
            "the master ships `type.{}.tabular = false` and the run was boxed anyway",
            master.role
        );
        let fixture = boxed_theme("toast", &master.role);
        let boxed = measure_in_child(CHILD, Some(&fixture));
        let _ = std::fs::remove_file(&fixture);
        let a: f32 = boxed.field("ADVANCE=").parse().expect("ADVANCE= is a number");
        assert!(
            a > 0.0,
            "a theme put `type.{}.tabular = true` and the message was still drawn \
             proportionally:\n{}",
            master.role,
            boxed.log
        );
    }

    /// The two children of the tests above: one toast, drawn for real,
    /// with one of its two runs reported. `drawn_runs` keeps the whole
    /// command, so the FIGURE ADVANCE the run was made under is
    /// measured beside the slot rather than inferred from it.
    #[test]
    #[ignore = "measured in a process of its own by the test above"]
    fn child_body_face() {
        child_face(1, "toast.body.role");
    }

    #[test]
    #[ignore = "measured in a process of its own by the test above"]
    fn child_title_face() {
        child_face(0, "toast.title.role");
    }

    /// One toast; `run` is 0 for the title word and 1 for the message,
    /// which is the order they are drawn in.
    fn child_face(run: usize, binding: &'static str) {
        let cmds = drawn_runs(|ctx| {
            let mut ts = Toaster::new();
            ts.push(Toast::warning(BODY.to_string()));
            ts.draw(ctx);
        });
        assert_eq!(cmds.len(), 2, "a toast is its title word and its message");
        let (font, advance, text) = match &cmds[run] {
            DrawCmd::Text { font, tabular, text, .. } => (*font, *tabular, text.clone()),
            _ => unreachable!("drawn_runs answers text commands"),
        };
        if run == 0 {
            // Regression guard (5.30): `Toast::warning`'s title reads
            // `catalog.toaster.warning_title` through the epoch-gated
            // cache now, and the shipped master's own untagged row must
            // still be byte-identical to the literal this file drew
            // before the catalogue existed.
            assert_eq!(text, "WARNING", "the shipped master's own untagged catalog row");
        }
        let drawn = [(font, text)];
        let role = role_word(binding);
        all_in(&drawn, crate::ui::role(&role).font());
        println!("ADVANCE={advance}");
        report(&role, font, &drawn);
    }
}
