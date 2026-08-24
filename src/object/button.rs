//! Parallelogram button — the standard clickable object of the
//! interface (terminal-tab style: slanted sides, hover highlight,
//! flash on click, optional "selected" state).

use super::focus_ring;
use crate::access::{AccessInfo, Role};
use crate::focus::{Caps, FocusId};
use crate::draw::Corner;
use crate::theme::{self, bake::StateStyle, parse::State, Color, TokenId};
use crate::motion;
use crate::ui;
use crate::view::surface::StateInk;
use crate::{Ctx, Rect};
use std::sync::OnceLock;

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// The engine's colour, in the draw list's clothes.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

#[derive(Clone, Copy, Default)]
pub struct ButtonState {
    pub hover: bool,
    pub flash: bool,
    pub selected: bool,
}

impl ButtonState {
    /// Which ladder slot this button occupies. A decaying click flash IS
    /// press; selection persists under the pointer as selected_hover.
    fn state(self) -> State {
        if self.flash {
            State::Press
        } else if self.hover && self.selected {
            State::SelectedHover
        } else if self.hover {
            State::Hover
        } else if self.selected {
            State::Selected
        } else {
            State::Idle
        }
    }
}

/// The outline points of a button rectangle. With `button.skew` at
/// zero — where the master leaves it, because a button now wears the
/// same corners as the frames around it — these are the rectangle's
/// own four points, and the SHAPE comes from [`corners`] instead. A
/// theme that wants the old parallelogram back only has to give the
/// token a width again.
pub fn quad(r: &Rect) -> [[f32; 2]; 4] {
    // The dropdown reads the same token, so the accordion stays flush
    // with its anchor's edge whichever shape that edge has.
    static SKEW: OnceLock<TokenId> = OnceLock::new();
    let skew = theme::resolved().px(tok(&SKEW, "button.skew"));
    [
        [r.x + skew, r.y],
        [r.right(), r.y],
        [r.right() - skew, r.bottom()],
        [r.x, r.bottom()],
    ]
}

/// A button's four corners: the same STYLE and the same RADIUS the
/// frames use, because a control that sits inside a rounded panel and
/// answers with a sharp corner reads as a different material. The
/// tokens are the button's own (`button.corner_style`,
/// `button.corner`), and the master points both at the panel's — so
/// the shape is one decision, made once, in the theme.
pub fn corners(t: &theme::ResolvedTheme) -> ([Corner; 4], u8) {
    static MODE: OnceLock<TokenId> = OnceLock::new();
    static IDX: OnceLock<crate::corner::Cuts> = OnceLock::new();
    static RADIUS: OnceLock<TokenId> = OnceLock::new();
    static SEGMENTS: OnceLock<TokenId> = OnceLock::new();
    let cut = t.px(tok(&RADIUS, "button.corner")).max(0.0);
    let style = crate::corner::style(t, tok(&MODE, "button.corner_style"), &IDX);
    (
        [Corner { style, size: cut }; 4],
        super::window::corner_segments(t, &SEGMENTS, cut),
    )
}

/// Everything a button is EXCEPT its label: the opaque plate, the
/// ladder's state wash over it, and the ring that rung states — all
/// three on the corners [`corners`] settles. Answers the rung it drew,
/// so a caller that sets its own label sets it in the ink the ladder
/// chose rather than in a second reading of the same class.
///
/// Split out of [`draw`] because a button is not the only object that
/// IS one. A drop-down's rows are the anchor seen N times over: the
/// owner asked for the anchor's own background, its own frame and its
/// own corner on every row, and a second reading of `shape.button.fill`,
/// `button.corner` and the `button` class at that call site would be a
/// private copy of these three rules — the drift [`super::elev`] was
/// pulled out of `panel.rs`/`window.rs` to end. The label is NOT in
/// here, because a row's label is set in the role its own list binds
/// (`list.label_role`) while a cap is set in `button.role`: the dress is
/// shared, the type ladder is not.
pub fn dress(ctx: &mut Ctx, r: Rect, st: ButtonState) -> StateStyle {
    static PLATE: OnceLock<TokenId> = OnceLock::new();
    static CLASS: OnceLock<Option<u16>> = OnceLock::new();
    let t = theme::resolved();
    let (corners, seg) = corners(t);
    // Opaque plate first, the ladder's state wash on top of it. Both
    // ride the same corners as the frames — one shape, drawn by the
    // same primitive a panel's ring uses.
    ctx.dl.ring_fill(r, &corners, seg, col(t.color(tok(&PLATE, "shape.button.fill"))));
    // The rung, reached over TIME: hover, press and selection crossfade
    // under their own `motion.*` entries instead of snapping. The wash is
    // the ladder's own at rest and at the end of every fade — only the
    // arrival is animated (`motion::state_ink`).
    let cls = *CLASS.get_or_init(|| theme::class_id("button"));
    let style: StateStyle = crate::motion::state_ink(
        "button",
        r,
        st.state(),
        ctx.t,
        |s| {
            StateInk::from(match cls {
                Some(c) => t.class_state(c, s),
                None => StateStyle::RAW,
            })
        },
    )
    .into();
    ctx.dl.ring_fill(r, &corners, seg, col(style.fill));
    if style.edge_width > 0.0 {
        ctx.dl.ring(r, &corners, seg, style.edge_width, col(style.edge));
    }
    style
}

/// The type ladder a cap is set in, resolved once.
///
/// One struct because the ladder has to be resolved in TWO places that
/// must not be allowed to disagree — [`draw`], which puts the run on
/// screen, and [`cap_width`], which tells a caller how wide that run
/// will be. Every member below was read inline in `draw` before there
/// was a second reader, and every member of it is a thing the answer
/// depends on: measure a cap in a different face, a different size, a
/// different tracking, a different figure box or a different CASE than
/// the one drawn and the plate is the wrong width.
struct Ladder {
    font: u8,
    px: f32,
    track: f32,
    leading: f32,
    fig: crate::font::Figures,
    case: ui::Case,
}

fn ladder(ctx: &mut Ctx) -> Ladder {
    static ROLE: OnceLock<TokenId> = OnceLock::new();
    // The cap is set in the role `button.role` NAMES, not in the role that
    // happens to share the object's name: repointing the binding moves the
    // label's whole ladder at once, which is the only reason the binding
    // exists. Config scaling (UIFontSize=, panel container query) is
    // behaviour, not design, so it rides the role's own arithmetic.
    let role = ui::bound_role(&ROLE, "button.role");
    // The role's own px floor and ceiling are `Role::px`'s business now,
    // and one place is the point of them: this file used to spell the key
    // from the binding's word by hand, which every other consumer of every
    // other binding did not.
    // No `ui_font_scale`: the viewport carries the user's scale into u,
    // and the role's size is written in u — applying it here too squares it.
    let px = role.px(ctx, 1.0);
    // The FACE is the role's too. `type.<role>.face` names one of the
    // master's eight slots and the master sends a cap to `ui_medium`;
    // naming FONT_UI here answered `ui` whatever the token said, so the
    // ladder the theme writes ended at this line — the size came down it
    // and the family did not.
    let font = role.font();
    Ladder {
        font,
        px,
        track: role.tracking_px(px),
        leading: role.leading(),
        // MEASURED WITH WHAT IT DRAWS: `text_center_fig` measures the run
        // to place it, so the box goes in with the face and the px rather
        // than beside them. A role that asks for no figures answers
        // `Figures::NONE` and this is the proportional run it has always
        // been.
        fig: role.figures(ctx.fonts, font, px),
        // WHETHER THIS CAP SHOUTS is the role's to say, like its size, its
        // face and its tracking. It was the one member of that ladder the
        // button could not read, so every caller in the program settled the
        // question in its own source — `"SAVE"`, `"ADD WIDGET"`, two hundred
        // and fifty-odd literals — and a theme setting `type.button.case`
        // moved nothing at all. Asking here does not change a single one of
        // those strings under the shipped master, which says `upper`; it
        // makes a master that says `none` mean something.
        case: role.case(),
    }
}

/// The run a button DRAWS for `label` — the label under the case
/// transform `button.role`'s role asks for.
///
/// Public because the case transform CUT A SEAM the moment it moved
/// inside [`draw`]: anything that reserves room for a cap, or writes one
/// beside a plate this file drew, was measuring and drawing a string the
/// button no longer puts on screen. A caller that needs the width wants
/// [`cap_width`] or [`plate_w`]; this is for the caller that needs the
/// TEXT — the disabled inscription a host draws in the ladder's own ink
/// rather than through [`draw`].
///
/// Borrowed under `case = none`, which is [`ui::recase`]'s own bargain:
/// most roles ask for no transform and a label that is not transformed
/// must not cost an allocation per frame to say so.
pub fn cap_of(label: &str) -> std::borrow::Cow<'_, str> {
    static ROLE: OnceLock<TokenId> = OnceLock::new();
    ui::bound_role(&ROLE, "button.role").cased(label)
}

/// How wide the cap of `label` is DRAWN.
///
/// Measured on exactly the run [`draw`] puts on screen, in exactly the
/// face, size, tracking and figure box it uses — which is the whole
/// point of the function existing. Before it did, the one place in the
/// program that sized a button to its label measured the label as its
/// author spelled it, with `FontSystem::measure` and no figure box, and
/// then handed the same string to `draw` to be recased and boxed. Both
/// halves of that were free while the master said `upper` and every
/// literal was already in capitals; a master saying `lower`, or one
/// literal written in mixed case, made a plate that no longer fits what
/// is on it. A width is a promise about a run, so the run has to be the
/// one that is drawn.
pub fn cap_width(ctx: &mut Ctx, label: &str) -> f32 {
    let l = ladder(ctx);
    ctx.fonts.measure_fig(l.font, l.px, &ui::recase(l.case, label), l.track, &l.fig)
}

/// The plate `label` needs: its [`cap_width`] with `button.pad_x` on
/// each side, never narrower than `button.min_w`.
///
/// The padding and the floor are the BUTTON's own tokens, so a bar that
/// sizes its plates asks the object how big it is rather than reading
/// two of the object's keys itself and hoping it applied them the same
/// way. No length in here belongs to the caller.
pub fn plate_w(ctx: &mut Ctx, label: &str) -> f32 {
    static PAD_X: OnceLock<TokenId> = OnceLock::new();
    static MIN_W: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let pad = t.px(tok(&PAD_X, "button.pad_x"));
    let min_w = t.px(tok(&MIN_W, "button.min_w"));
    (cap_width(ctx, label) + 2.0 * pad).max(min_w)
}

/// Draws an opaque parallelogram button with a centered label.
/// Nothing behind the button shows through it.
pub fn draw(ctx: &mut Ctx, r: Rect, label: &str, st: ButtonState) {
    let style = dress(ctx, r, st);
    let l = ladder(ctx);
    let cap = ui::recase(l.case, label);
    ctx.dl.text_center_fig(
        ctx.fonts,
        l.font,
        l.px,
        r.cx(),
        r.y + (r.h - l.px * l.leading) / 2.0,
        &cap,
        col(style.text),
        l.track,
        &l.fig,
    );
}

/// [`draw`], joined to the world's focus chain: `id` is the caller's
/// stable path (`"settings.btn.reset"` — a path, never an index). A
/// button eats no keys — activation is the router's Enter/Space — so it
/// registers with no capabilities. Focus never touches the state ladder
/// (`ButtonState` grows no `focused` field); the ring overlay is the
/// only focus signal, drawn around the same slanted quad.
pub fn draw_focusable(ctx: &mut Ctx, r: Rect, label: &str, st: ButtonState, id: FocusId) {
    let f = ctx
        .focus
        .as_deref_mut()
        .map(|fc| fc.register(id, r, Caps::NONE, AccessInfo::new(Role::Button, label)));
    draw(ctx, r, label, st);
    // Asked EVERY frame, ring or no ring: the band fades in and out on
    // `motion.focus`, and a call made only while focused would have
    // nothing left on screen to fade away.
    focus_ring::draw_quad_faded(ctx, quad(&r), f.map_or(false, |f| f.ring));
}

// ------------------------------------------------------- hold to confirm

/// A button that must be HELD, not clicked — `motion.hold`'s reader, and
/// the shape image 5's SYSTEM LOCKDOWN wants.
///
/// `down` is the caller's answer to "is the pointer (or the keyboard's
/// activation) still on this control", asked EVERY frame: releasing it is
/// what cancels, and a hold is not resumable. The answer is true on the
/// one frame the ramp completes, so the caller acts on it exactly once.
///
/// # What it draws, and why it needs no token of its own
///
/// The plate is the button's own [`dress`] in the state the caller asked
/// for, and the readout is the class's PRESS ring swept round the same
/// outline, one arc-length at a time. The two rungs are the ones the theme
/// already distinguishes — `[state] press.edge` against whatever the
/// control is resting on — so a control that is half-held is drawn half in
/// the colour of a held control, which is the statement being made. There
/// is no `hold.*` anything: the closed catalogue's `motion.hold` owns the
/// TIME and `[state]` owns the LOOK, which is where each of them lives for
/// every other control in this file.
///
/// A theme whose press rung has no ring — `press.edge_width` at zero, or a
/// transparent `press.edge` — draws no readout, exactly as [`dress`] draws
/// no ring in that case. That is the house rule about fallbacks, applied
/// to a control that would otherwise be the one place with a hard-coded
/// colour in it.
///
/// WHERE the sweep starts is layout, not theme (§5.22's own line): it
/// begins at the outline's top-left and runs clockwise, because that is
/// where [`crate::draw::ring_points`] starts and a ring has to start
/// somewhere.
pub fn draw_hold(ctx: &mut Ctx, r: Rect, label: &str, st: ButtonState, id: &str, down: bool) -> bool {
    static CLASS: OnceLock<Option<u16>> = OnceLock::new();
    // A control being held IS pressed. Forcing it here rather than asking
    // the caller keeps the two from disagreeing: a button under a finger
    // that does not look pressed is wrong however it got that way.
    let held = motion::hold(id, r, down, ctx.t);
    draw(ctx, r, label, ButtonState { flash: st.flash || down, ..st });
    if held.progress <= 0.0 {
        return held.fired;
    }
    let t = theme::resolved();
    let cls = *CLASS.get_or_init(|| theme::class_id("button"));
    let press: StateStyle = match cls {
        Some(c) => t.class_state(c, State::Press),
        None => StateStyle::RAW,
    };
    let width = press.edge_width.max(0.0);
    let ink = col(press.edge);
    if width > 0.0 && ink.a > 0.0 {
        let (corners, seg) = corners(t);
        let mut path = Vec::new();
        // The stroke is centred on its path, so the path is the ring's own
        // centreline — half a width inside the box, which is where the
        // solid ring `dress` drew has its middle. Otherwise the readout
        // sits a hair outside the ring it is filling in.
        let h = width * 0.5;
        let c = Rect::new(r.x + h, r.y + h, (r.w - width).max(0.0), (r.h - width).max(0.0));
        crate::draw::ring_points(c, &[corners[0].inset(h); 4], seg, &mut path);
        if let Some(swept) = prefix(&path, held.progress) {
            ctx.dl.polyline(&swept, width, ink, false);
        }
    }
    held.fired
}

/// The first `f` of a closed outline's LENGTH, as an open polyline.
///
/// By arc length rather than by point count: a rounded corner is a dozen
/// short segments and a side is one long one, so a readout stepped by
/// index would race round the corners and crawl along the edges — the ramp
/// would be linear in time and visibly not linear on screen.
///
/// The outline is closed, so the return leg back to the first point is
/// part of the length; `None` for a degenerate box with no perimeter.
fn prefix(path: &[[f32; 2]], f: f32) -> Option<Vec<[f32; 2]>> {
    if path.len() < 2 {
        return None;
    }
    let leg = |a: [f32; 2], b: [f32; 2]| ((b[0] - a[0]).powi(2) + (b[1] - a[1]).powi(2)).sqrt();
    let total: f32 = path
        .iter()
        .zip(path.iter().cycle().skip(1))
        .take(path.len())
        .map(|(a, b)| leg(*a, *b))
        .sum();
    if !(total > 0.0) {
        return None;
    }
    let mut budget = total * f.clamp(0.0, 1.0);
    let mut out = vec![path[0]];
    for i in 0..path.len() {
        let (a, b) = (path[i], path[(i + 1) % path.len()]);
        let d = leg(a, b);
        if d <= 0.0 {
            continue;
        }
        if budget >= d {
            budget -= d;
            out.push(b);
            continue;
        }
        let k = budget / d;
        out.push([a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k]);
        break;
    }
    Some(out)
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    //! A cap is set in the face `type.<button.role>.face` names, and the
    //! run is centred on a width measured in that same face, at that same
    //! size, under that same figure box.
    //!
    //! The measurement is the whole point of the file: `text_center_fig`
    //! measures what it is about to draw and puts the pen at `cx - w/2`,
    //! so the pen is a WITNESS to the arguments the measurement was made
    //! with. A reference run drawn at that pen, in the face the register
    //! says the cap was drawn in, lands glyph for glyph on the cap — and
    //! the same reference in any other face does not. That is what
    //! separates "the label moved" from "the label followed the token".
    //!
    //! A theme is process-wide, so the fixture stages run in a CHILD
    //! process with `NACELLE_THEME_PATH` pointing at the fixture: this is
    //! a unit-test binary of 450-odd tests running in parallel threads,
    //! and swapping the resolved theme under them would prove one thing
    //! by breaking another.

    use super::*;
    use crate::draw::{DrawCmd, DrawList, TextAnchor, Vertex};
    use crate::font::{FontSystem, Figures, FACE_UI_MEDIUM, FONT_MONO, FONT_UI};
    use crate::pointer::Pointer;

    const BOX: Rect = Rect { x: 40.0, y: 40.0, w: 320.0, h: 48.0 };
    /// No space anywhere: a blank draws no quad, and the glyph sequence
    /// is what every comparison here is made of.
    const CAP: &str = "START";
    /// The narrowest and the widest figure of most faces, four of each:
    /// the pair that tells a fixed advance from a proportional one.
    const ONES: &str = "1111";
    const EIGHTS: &str = "8888";

    /// What the register kept about the one text run a cap is.
    struct Run {
        font: u8,
        px: f32,
        track: f32,
        /// The figure box the run was stepped by; 0.0 for a proportional one.
        fig: f32,
        /// The point the run was centred on, and its baseline box top.
        cx: f32,
        y: f32,
        /// The left edge of every glyph quad the cap put on the screen.
        xs: Vec<f32>,
    }

    fn ctx<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem) -> Ctx<'a> {
        Ctx {
            access: None,
            dl,
            fonts,
            w: 1920.0,
            h: 1080.0,
            t: 0.0,
            mouse: Pointer::new(0.0, 0.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        }
    }

    /// The left edge of every glyph quad in `verts[from..]`. A quad is six
    /// vertices and its vertex 0 is the left edge; the fake-bold second
    /// copy is a quad of its own and is kept, because the reference run is
    /// drawn the same way and a face that fakes its weight has to compare
    /// as the face it is.
    fn quad_xs(verts: &[Vertex], from: usize) -> Vec<f32> {
        verts[from..].chunks(6).map(|q| q[0].pos[0]).collect()
    }

    /// Draws one cap and reports the run it made. The plate and the ring
    /// are drawn before the label and do not depend on it, so a cap with
    /// an empty label measures where the glyphs begin.
    fn cap(fonts: &mut FontSystem, label: &str) -> Run {
        let plate = {
            let mut dl = DrawList::new();
            draw(&mut ctx(&mut dl, fonts), BOX, "", ButtonState::default());
            dl.verts.len()
        };
        let mut dl = DrawList::recording();
        draw(&mut ctx(&mut dl, fonts), BOX, label, ButtonState::default());
        let text = dl
            .cmds()
            .iter()
            .find_map(|c| match c {
                DrawCmd::Text { at, anchor, font, px, tracking, tabular, .. } => {
                    assert!(
                        matches!(anchor, TextAnchor::Centre),
                        "a cap is centred in its plate"
                    );
                    Some(Run {
                        font: *font,
                        px: *px,
                        track: *tracking,
                        fig: *tabular,
                        cx: at[0],
                        y: at[1],
                        xs: Vec::new(),
                    })
                }
                _ => None,
            })
            .expect("a cap draws exactly one text run");
        Run { xs: quad_xs(&dl.verts, plate), ..text }
    }

    /// The role `button.role` binds, read the way the file reads it.
    fn role() -> crate::ui::Role {
        let id = theme::id("button.role").expect("the master declares button.role");
        crate::ui::role(&theme::enum_word_of(id).unwrap_or_default())
    }

    /// A bare run of `label` in `font`, centred on `at.cx` exactly as
    /// `DrawList::text_center_fig` centres one: the pen is `cx` less half
    /// the width MEASURED in that font, at that px, under that box.
    fn reference(
        fonts: &mut FontSystem,
        at: &Run,
        font: u8,
        fig: &Figures,
        label: &str,
    ) -> Vec<f32> {
        let w = fonts.measure_fig(font, at.px, label, at.track, fig);
        let mut dl = DrawList::new();
        dl.text_fig(
            fonts,
            font,
            at.px,
            at.cx - w / 2.0,
            at.y,
            label,
            Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
            at.track,
            fig,
        );
        quad_xs(&dl.verts, 0)
    }

    /// The slot that is NOT the one under test — the control every
    /// assertion below is paired with, so that "they match" cannot be the
    /// answer to every question.
    fn other(font: u8) -> u8 {
        if font == FONT_MONO { FONT_UI } else { FONT_MONO }
    }

    /// How far the pen moved between one glyph and the next. The advance
    /// of the run, with the glyphs' own left bearings cancelled out.
    fn steps(xs: &[f32]) -> Vec<f32> {
        xs.windows(2).map(|w| w[1] - w[0]).collect()
    }

    /// The width `label` was measured at, with the face, size, tracking
    /// and figure box the REGISTER says the run was drawn under — the one
    /// call `DrawList::text_center_fig` makes to place its pen.
    fn width(fonts: &mut FontSystem, at: &Run, label: &str) -> f32 {
        let fig = crate::ui::figures(fonts, at.font, at.px, at.fig > 0.0);
        fonts.measure_fig(at.font, at.px, label, at.track, &fig)
    }

    /// Every claim about one theme's cap, made against whatever theme the
    /// process resolved. Called once here and once in each child.
    fn cap_follows_its_role(expect: u8) {
        let mut fonts = FontSystem::new();
        let role = role();
        let run = cap(&mut fonts, CAP);
        assert!(!run.xs.is_empty(), "the cap drew no glyphs at all");

        // 1. the FACE is the role's.
        assert_eq!(
            run.font,
            role.font(),
            "the cap was drawn in slot {} and type.<{}>.face names slot {}",
            run.font,
            "button.role",
            role.font()
        );
        assert_eq!(run.font, expect, "the role's own face moved under the test");

        // 2. the SIZE and the tracking are the role's, and the figure box
        //    is the one the role asks for — not a box invented here.
        assert_eq!(run.px, role.px(&ctx(&mut DrawList::new(), &mut fonts), 1.0));
        assert_eq!(run.track, role.tracking_px(run.px));
        let fig = role.figures(&mut fonts, run.font, run.px);
        assert_eq!(run.fig, fig.advance(), "the run was stepped by a box the role did not ask for");
        assert_eq!(fig.is_on(), role.tabular(), "type.<button.role>.tabular");

        // 3. MEASURED WITH WHAT IT DREW. The pen of a centred run is `cx`
        //    less half the measured width, so a reference run laid at the
        //    pen the register's own numbers imply must land glyph for
        //    glyph on the cap.
        assert_eq!(
            run.xs,
            reference(&mut fonts, &run, run.font, &fig, CAP),
            "the cap was not centred on a width measured in the face, size and \
             box it was drawn with"
        );
        // ...and the same reference in the other face does not, which is
        // what stops the line above passing for any face at all.
        let wrong = other(run.font);
        let wrong_fig = crate::ui::figures(&mut fonts, wrong, run.px, fig.is_on());
        assert_ne!(
            run.xs,
            reference(&mut fonts, &run, wrong, &wrong_fig, CAP),
            "slot {wrong} lays the cap out exactly like slot {} — this machine \
             cannot tell the two faces apart and the test above proves nothing",
            run.font
        );
    }

    /// Writes `body` as a theme based on the master and runs `test` — an
    /// `#[ignore]`d sibling of this module — in a child process under it.
    fn under_theme(body: &str, test: &str) {
        let path = std::env::temp_dir()
            .join(format!("nacelle-button-face-{}-{}.theme", std::process::id(), test));
        std::fs::write(
            &path,
            format!("[meta]\nschema = 1\nname = \"Button face fixture\"\nbase = \"default\"\n\n{body}"),
        )
        .expect("the fixture theme must be writable");
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args([test, "--exact", "--ignored", "--test-threads=1"])
            .env("NACELLE_THEME_PATH", &path)
            .output()
            .expect("the child test process must start");
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let _ = std::fs::remove_file(&path);
        assert!(out.status.success(), "under this theme:\n{body}\n{log}");
        // A filter that matched nothing exits 0 as well, and a stage that
        // never ran is the one way a fixture proves nothing quietly.
        assert!(log.contains("1 passed"), "the child ran no stage:\n{log}");
    }

    // ---------------------------------------------------------- the master

    #[test]
    fn a_cap_is_set_in_the_face_its_role_names() {
        for v in ["NACELLE_THEME_PATH", "NACELLE_THEME_NAME", "NACELLE_THEME_MASTER"] {
            assert!(std::env::var_os(v).is_none(), "{v} is set — this stage reads the master");
        }
        // The master says `type.button.face = ui_medium`. Until this file
        // asked the role, the cap was drawn in FONT_UI whatever the token
        // said, so slot 0 is the measured before and slot 2 the after.
        cap_follows_its_role(FACE_UI_MEDIUM);
        assert_ne!(
            FACE_UI_MEDIUM, FONT_UI,
            "the master moved button.face onto the interface slot and this \
             stage can no longer tell the two apart"
        );

        // The role does not ask for a figure box, so the run has none —
        // the control for the stage below, which turns the token on.
        assert!(!role().tabular(), "type.button.tabular is false in the master");
        let mut fonts = FontSystem::new();
        let ones = cap(&mut fonts, ONES);
        let eights = cap(&mut fonts, EIGHTS);
        assert_eq!(ones.fig, 0.0);
        assert_ne!(
            steps(&ones.xs),
            steps(&eights.xs),
            "a proportional cap stepped 1111 and 8888 identically — this face has \
             uniform figures and the box below cannot be witnessed"
        );
        assert_ne!(
            width(&mut fonts, &ones, ONES),
            width(&mut fonts, &eights, EIGHTS),
            "a proportional cap measured 1111 and 8888 the same width"
        );

        // ---- and the token is what decides, not this file ----------
        under_theme("[type]\nbutton.face = mono\n", "object::button::tests::a_cap_in_a_mono_theme_is_mono");
        under_theme(
            "[type]\nbutton.tabular = true\n",
            "object::button::tests::a_cap_under_a_tabular_role_is_boxed",
        );
    }

    // --------------------------------------------------------- the fixtures
    //
    // Run by the stage above, in a child process, under a theme of its
    // own. `#[ignore]` keeps them out of the ordinary pass, where the
    // master is what is resolved and they would be measuring nothing.

    #[test]
    #[ignore = "run by a_cap_is_set_in_the_face_its_role_names under a fixture theme"]
    fn a_cap_in_a_mono_theme_is_mono() {
        cap_follows_its_role(FONT_MONO);
    }

    #[test]
    #[ignore = "run by a_cap_is_set_in_the_face_its_role_names under a fixture theme"]
    fn a_cap_under_a_tabular_role_is_boxed() {
        // The face is the master's still; only the box moved.
        cap_follows_its_role(FACE_UI_MEDIUM);
        let mut fonts = FontSystem::new();
        let ones = cap(&mut fonts, ONES);
        let eights = cap(&mut fonts, EIGHTS);
        assert!(ones.fig > 0.0, "type.button.tabular = true and the run carried no box");
        // The STEP is the box, so a cap of ones advances exactly as a cap of
        // eights does. (The glyphs themselves sit centred in their boxes, so
        // a narrow figure still starts a fraction further in — that offset is
        // what a fixed advance buys, not what it costs.)
        assert_eq!(
            steps(&ones.xs),
            steps(&eights.xs),
            "a boxed cap still steps by the glyph: 1111 and 8888 advanced differently"
        );
        // ...and the WIDTH the run was centred on is the same width, measured
        // with the same face, px, tracking and box the register recorded.
        assert_eq!(
            width(&mut fonts, &ones, ONES),
            width(&mut fonts, &eights, EIGHTS),
            "a boxed cap measured 1111 and 8888 at different widths"
        );
    }

    // ------------------------------------------------------------ case
    //
    // WHETHER A BUTTON SHOUTS is the theme's to say. Twelve of the
    // master's twenty-five roles ask for `upper` or `smallcaps` and no
    // control outside the two title bands honoured any of them, so the
    // interface answered the question in its own source instead.

    /// The one text run a cap made, as the register recorded it.
    fn cap_text(label: &str) -> String {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::recording();
        draw(&mut ctx(&mut dl, &mut fonts), BOX, label, ButtonState::default());
        dl.cmds()
            .iter()
            .find_map(|c| match c {
                DrawCmd::Text { text, .. } => Some(text.clone()),
                _ => None,
            })
            .expect("a cap draws exactly one text run")
    }

    #[test]
    fn a_theme_that_asks_for_no_case_gets_no_capitals_on_a_cap() {
        crate::draw::arm_cmds();
        // The shipped master says `type.button.case = upper`, so a cap
        // written in mixed case reaches the screen shouting — which is
        // the whole reason two hundred and fifty-seven labels in the
        // desktop are spelled in capitals: until now the token could not
        // do it for them.
        assert_eq!(cap_text("Save"), "SAVE");
        // A theme that asks for none gets none. Not "gets the string the
        // caller happened to write" — the caller wrote the same string
        // both times, and only the theme moved.
        crate::ui::seed_theme_word("type.button.case", "none");
        assert_eq!(cap_text("Save"), "Save");
        // ...and a theme with a TYPO in that key gets no transform
        // either, where the five hand-rolled `match`es this replaced all
        // ended on `_ => to_uppercase()`.
        crate::ui::seed_theme_word("type.button.case", "uper");
        assert_eq!(cap_text("Save"), "Save");
        crate::ui::seed_theme_word("type.button.case", "lower");
        assert_eq!(cap_text("Save"), "save");
    }

    // -------------------------------------------- the measure/draw seam
    //
    // A transform applied INSIDE `draw` cut a seam the moment it landed:
    // every caller that reserves room for a cap was sizing the string as
    // its author spelled it and then handing that same string over to be
    // recased. Under the shipped master the two agree by luck — `upper`
    // applied to a literal already in capitals is the identity — and the
    // luck runs out on the first label written in mixed case, which is
    // exactly what the case token exists to allow. The three functions
    // below are the seam closed: what is drawn, how wide it is, and how
    // wide the plate under it has to be.

    #[test]
    fn the_cap_a_button_offers_is_the_cap_it_draws() {
        crate::draw::arm_cmds();
        // Under the master (`upper`) and under a theme that says none:
        // both times the answer is what reached the register, never the
        // string the caller passed in.
        assert_eq!(cap_of("Save"), cap_text("Save"));
        assert_eq!(cap_of("Save"), "SAVE");
        crate::ui::seed_theme_word("type.button.case", "lower");
        assert_eq!(cap_of("Save"), cap_text("Save"));
        assert_eq!(cap_of("Save"), "save");
    }

    #[test]
    fn a_cap_is_measured_on_the_run_that_reaches_the_screen() {
        crate::draw::arm_cmds();
        let mut fonts = FontSystem::new();
        // A label whose case the master CHANGES: "Save" is four glyphs
        // either way, but a capital is wider than its lower-case twin in
        // every proportional face, so the two widths are tellable apart.
        // That difference is the defect: the one caller that sized a
        // plate measured the untransformed spelling.
        // The width of a spelling, measured under the ladder the button
        // itself resolves — the reading `text_center_fig` makes to place
        // its pen, taken here for a string of the test's choosing.
        let mut spelling = |s: &str| {
            let mut dl = DrawList::new();
            let mut c = ctx(&mut dl, &mut fonts);
            let l = ladder(&mut c);
            c.fonts.measure_fig(l.font, l.px, s, l.track, &l.fig)
        };
        let (mixed, shouted, lowered) = (spelling("Save"), spelling("SAVE"), spelling("save"));
        assert!(
            mixed != shouted && mixed != lowered,
            "this face measures Save, SAVE and save identically, so the \
             seam below cannot be witnessed on it"
        );

        let offered = |fonts: &mut FontSystem| {
            let mut dl = DrawList::new();
            cap_width(&mut ctx(&mut dl, fonts), "Save")
        };
        // Under the master (`upper`) the plate is sized for SAVE — the
        // run that reaches the screen — and NOT for the spelling the
        // caller wrote, which is the whole defect.
        assert!(
            (offered(&mut fonts) - shouted).abs() < 1e-3,
            "a cap was measured on the caller's spelling, not on the drawn run"
        );
        // ...and it MOVES with the theme, in both directions, because
        // the case is read at measuring time and not baked in.
        crate::ui::seed_theme_word("type.button.case", "lower");
        assert!(
            (offered(&mut fonts) - lowered).abs() < 1e-3,
            "the offered width did not follow type.button.case to lower"
        );
        crate::ui::seed_theme_word("type.button.case", "none");
        assert!(
            (offered(&mut fonts) - mixed).abs() < 1e-3,
            "a role asking for no case measured something other than \"Save\""
        );
    }

    #[test]
    fn a_plate_is_its_cap_plus_the_buttons_own_padding() {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::new();
        let mut c = ctx(&mut dl, &mut fonts);
        let t = theme::resolved();
        let pad = t.px(theme::id("button.pad_x").expect("the master declares it"));
        let min_w = t.px(theme::id("button.min_w").expect("the master declares it"));
        assert!(pad > 0.0 && min_w > 0.0, "the master states both lengths");
        // A long label clears the floor, so the padding is what shows.
        let long = "SAVE AS A NEW THEME FILE";
        assert!(
            (plate_w(&mut c, long) - (cap_width(&mut c, long) + 2.0 * pad)).abs() < 1e-3,
            "a plate is not its cap plus button.pad_x on each side"
        );
        // A two-letter label does not clear it, which is what min_w says
        // in the master's own words.
        assert!(cap_width(&mut c, "OK") + 2.0 * pad < min_w);
        assert!((plate_w(&mut c, "OK") - min_w).abs() < 1e-3, "a short cap fell under button.min_w");
    }
}
