//! Checkbox object: an outlined box with a filled square when checked,
//! plus a label. The whole row is the click target.

use super::focus_ring;
use crate::access::{AccessInfo, Role};
use crate::corner::Cuts;
use crate::draw::Corner;
use crate::focus::{Caps, FocusId};
use crate::theme::{self, bake::StateStyle, parse::State, Color, TokenId};
use crate::{ui, Ctx, Rect};
use std::sync::OnceLock;

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// The engine's colour, in the draw list's clothes.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// The box's four corners and their tessellation — BOTH halves of the
/// pair from the theme: `checkbox.corner` is the radius and
/// `checkbox.corner_style` is the cut.
///
/// The cut used to be `CornerStyle::Round`, written here and defended by
/// [corner]'s old rule that a radius with no sibling is cut round. The
/// checkbox has a sibling now, pointed at the button's, because the box
/// IS the control: a rounded box beside a chamfered button is two corner
/// languages in one row. Zero is still spelled Square whatever the theme
/// says — a zero-radius arc is a square corner drawn the cheap way.
///
/// The length goes through [`Corner::sized`], which is where §5.0's
/// `pill` is translated: `pill` bakes to a NEGATIVE number, so a box that
/// clamped the token at zero would answer a theme writing `pill` with the
/// square it wrote to avoid.
fn shape(t: &theme::ResolvedTheme, bx: Rect) -> ([Corner; 4], u8) {
    static CORNER: OnceLock<TokenId> = OnceLock::new();
    static SEGMENTS: OnceLock<TokenId> = OnceLock::new();
    static CUT: OnceLock<TokenId> = OnceLock::new();
    static CUT_IDX: OnceLock<Cuts> = OnceLock::new();
    let cut = crate::corner::style(t, tok(&CUT, "checkbox.corner_style"), &CUT_IDX);
    let c = Corner::sized(cut, t.px(tok(&CORNER, "checkbox.corner")), bx);
    let c = if c.size > 0.0 { c } else { Corner::SQUARE };
    ([c; 4], super::window::corner_segments(t, &SEGMENTS, c.size))
}

/// Draws the checked mark inside `m`, the box already inset by
/// `checkbox.tick_inset`, in the shape `checkbox.tick_shape` names.
///
/// The two stroked marks take their line weight from
/// `checkbox.tick_stroke`, which the master sends after `checkbox.border`
/// — so the shipped mark is drawn in the same hand as the box around it,
/// and a theme wanting a thin tick inside a heavy ring can now say so.
fn tick(ctx: &mut Ctx, m: Rect, color: Color) {
    static SHAPE: OnceLock<TokenId> = OnceLock::new();
    static IDX: OnceLock<(Option<u16>, Option<u16>)> = OnceLock::new();
    static STROKE: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let id = tok(&SHAPE, "checkbox.tick_shape");
    let (check, cross) =
        *IDX.get_or_init(|| (theme::enum_index(id, "check"), theme::enum_index(id, "cross")));
    let cur = Some(t.enum_of(id));
    let w = t.px(tok(&STROKE, "checkbox.tick_stroke"));
    if cur == check {
        // The glyph's own proportions, as with menu.rs's chevron: where
        // the stroke turns is what makes a tick a tick.
        ctx.dl.polyline(
            &[
                [m.x, m.y + m.h * 0.55],
                [m.x + m.w * 0.38, m.bottom()],
                [m.right(), m.y],
            ],
            w,
            color,
            false,
        );
    } else if cur == cross {
        ctx.dl.polyline(&[[m.x, m.y], [m.right(), m.bottom()]], w, color, false);
        ctx.dl.polyline(&[[m.right(), m.y], [m.x, m.bottom()]], w, color, false);
    } else {
        // "square", plus anything the vocabulary does not name.
        ctx.dl.rect(m.x, m.y, m.w, m.h, color);
    }
}

/// Draws a checkbox row. The whole row is the hit target, which the
/// caller already has.
pub fn draw(ctx: &mut Ctx, row: Rect, label: &str, checked: bool, hover: bool) {
    static SIZE: OnceLock<TokenId> = OnceLock::new();
    static BORDER: OnceLock<TokenId> = OnceLock::new();
    static TICK: OnceLock<TokenId> = OnceLock::new();
    static TICK_INSET: OnceLock<TokenId> = OnceLock::new();
    static LABEL_GAP: OnceLock<TokenId> = OnceLock::new();
    static ROLE: OnceLock<TokenId> = OnceLock::new();
    static CLASS: OnceLock<Option<u16>> = OnceLock::new();
    let t = theme::resolved();
    // The box is its own length now, not a cut of the caller's row.
    let s = t.px(tok(&SIZE, "checkbox.size"));
    let bx = Rect::new(row.x, row.y + (row.h - s) / 2.0, s, s);
    // The ring's colour crossfades under `motion.hover` rather than
    // snapping. Keyed by the BOX, not the row: the box is what the ladder
    // dresses, and a row that changed width around a still box is the
    // same control.
    let cls = *CLASS.get_or_init(|| theme::class_id("checkbox"));
    let style: StateStyle = crate::motion::state_ink(
        "checkbox",
        bx,
        if hover { State::Hover } else { State::Idle },
        ctx.t,
        |s| {
            crate::view::surface::StateInk::from(match cls {
                Some(c) => t.class_state(c, s),
                None => StateStyle::RAW,
            })
        },
    )
    .into();
    let (corners, seg) = shape(t, bx);
    ctx.dl.ring(
        bx,
        &corners,
        seg,
        t.px(tok(&BORDER, "checkbox.border")),
        col(style.edge),
    );
    if checked {
        // checkbox.tick_inset bakes against checkbox.size, which `s` is.
        let m = t.px(tok(&TICK_INSET, "checkbox.tick_inset"));
        let mark = Rect::new(bx.x + m, bx.y + m, s - 2.0 * m, s - 2.0 * m);
        tick(ctx, mark, col(t.color(tok(&TICK, "component.checkbox.tick"))));
    }
    let role = ui::bound_role(&ROLE, "checkbox.role");
    // No `ui_font_scale`: the viewport carries the user's scale into u,
    // and the role's size is written in u — applying it here too squares it.
    let px = role.px(ctx, 1.0);
    let leading = role.leading();
    // The FACE comes down the same ladder as the size. `type.<role>.face`
    // names one of the master's eight slots; naming FONT_UI here answered
    // `ui` whatever the token said, so a theme could move this role's
    // family and the box's label would not follow it.
    let font = role.font();
    let track = role.tracking_px(px);
    // MEASURED WITH WHAT IT DRAWS: this row measures nothing of its own —
    // the label starts at the box's edge plus `checkbox.label_gap` — so
    // the box goes to the ONE place that steps the pen, and the row is
    // proportional exactly when the role says it is.
    let fig = role.figures(ctx.fonts, font, px);
    ctx.dl.text_fig(
        ctx.fonts,
        font,
        px,
        bx.right() + t.px(tok(&LABEL_GAP, "checkbox.label_gap")),
        row.y + (row.h - px * leading) / 2.0,
        label,
        col(style.text),
        track,
        &fig,
    );
}

/// [`draw`], joined to the world's focus chain: the whole row registers
/// — it is already the click target, and the ring wraps the same rect
/// the pointer hits. A checkbox eats no keys (toggling is the router's
/// Space/Enter), and focus never feeds `hover` — the ring is the only
/// focus signal.
pub fn draw_focusable(
    ctx: &mut Ctx,
    row: Rect,
    label: &str,
    checked: bool,
    hover: bool,
    id: FocusId,
) {
    let f = ctx
        .focus
        .as_deref_mut()
        .map(|fc| fc.register(id, row, Caps::NONE, AccessInfo::new(Role::CheckBox, label)));
    draw(ctx, row, label, checked, hover);
    focus_ring::draw_faded(ctx, row, f.map_or(false, |f| f.ring));
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    //! A checkbox label is set in the face `type.<checkbox.role>.face`
    //! names, at that role's size, tracking and figure box.
    //!
    //! This row measures NOTHING: the label starts at the box's right
    //! edge plus `checkbox.label_gap`, both tokens, and where the pen
    //! goes from there is the font layer's business. So the proof is in
    //! two halves — the pen is the two tokens and nothing else, and the
    //! glyphs from that pen onward are the run a reference draw in the
    //! register's own face makes, glyph for glyph. Any other face lays
    //! the same string out differently, which is what stops the second
    //! half passing for every face at once.
    //!
    //! A theme is process-wide, so the fixture stages run in a CHILD
    //! process with `NACELLE_THEME_PATH` pointing at the fixture: this is
    //! a unit-test binary of 450-odd tests running in parallel threads,
    //! and swapping the resolved theme under them would prove one thing
    //! by breaking another.

    use super::*;
    use crate::draw::{DrawCmd, DrawList, TextAnchor, Vertex};
    use crate::font::{FontSystem, Figures, FONT_MONO, FONT_UI};
    use crate::pointer::Pointer;

    const ROW: Rect = Rect { x: 60.0, y: 200.0, w: 380.0, h: 34.0 };
    /// No space anywhere: a blank draws no quad, and the glyph sequence
    /// is what every comparison here is made of.
    const LABEL: &str = "Pokazuj_sekundy";
    /// The narrowest and the widest figure of most faces, four of each:
    /// the pair that tells a fixed advance from a proportional one.
    const ONES: &str = "1111";
    const EIGHTS: &str = "8888";

    /// What the register kept about the one text run a label is.
    struct Run {
        font: u8,
        px: f32,
        track: f32,
        /// The figure box the run was stepped by; 0.0 for a proportional one.
        fig: f32,
        /// The pen the label started from, and its baseline box top.
        x: f32,
        y: f32,
        /// The left edge of every glyph quad the label put on the screen.
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

    /// Draws one row and reports the run it made. The box, its ring and
    /// its tick are drawn before the label and do not depend on it, so a
    /// row with an empty label measures where the glyphs begin.
    fn label_of(fonts: &mut FontSystem, text: &str) -> Run {
        let plate = {
            let mut dl = DrawList::new();
            draw(&mut ctx(&mut dl, fonts), ROW, "", true, false);
            dl.verts.len()
        };
        let mut dl = DrawList::recording();
        draw(&mut ctx(&mut dl, fonts), ROW, text, true, false);
        let run = dl
            .cmds()
            .iter()
            .find_map(|c| match c {
                DrawCmd::Text { at, anchor, font, px, tracking, tabular, .. } => {
                    assert!(matches!(anchor, TextAnchor::Left), "a label starts at the gap");
                    Some(Run {
                        font: *font,
                        px: *px,
                        track: *tracking,
                        fig: *tabular,
                        x: at[0],
                        y: at[1],
                        xs: Vec::new(),
                    })
                }
                _ => None,
            })
            .expect("a checkbox draws exactly one text run");
        Run { xs: quad_xs(&dl.verts, plate), ..run }
    }

    /// The role `checkbox.role` binds, read the way the file reads it.
    fn role() -> crate::ui::Role {
        let id = theme::id("checkbox.role").expect("the master declares checkbox.role");
        crate::ui::role(&theme::enum_word_of(id).unwrap_or_default())
    }

    fn px_of(name: &str) -> f32 {
        theme::resolved().px(theme::id(name).unwrap_or_else(|| panic!("no {name} in the master")))
    }

    /// A bare run of `text` laid from the pen the row used.
    fn reference(
        fonts: &mut FontSystem,
        at: &Run,
        font: u8,
        fig: &Figures,
        text: &str,
    ) -> Vec<f32> {
        let mut dl = DrawList::new();
        dl.text_fig(
            fonts,
            font,
            at.px,
            at.x,
            at.y,
            text,
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

    /// The width `text` measures under the face, size, tracking and box
    /// the REGISTER says the run was drawn with.
    fn width(fonts: &mut FontSystem, at: &Run, text: &str) -> f32 {
        let fig = crate::ui::figures(fonts, at.font, at.px, at.fig > 0.0);
        fonts.measure_fig(at.font, at.px, text, at.track, &fig)
    }

    /// Every claim about one theme's row, made against whatever theme the
    /// process resolved. Called once here and once in each child.
    fn label_follows_its_role(expect: u8) {
        let mut fonts = FontSystem::new();
        let role = role();
        let run = label_of(&mut fonts, LABEL);
        assert!(!run.xs.is_empty(), "the row drew no glyphs at all");

        // 1. the FACE is the role's.
        assert_eq!(
            run.font,
            role.font(),
            "the label was drawn in slot {} and type.<checkbox.role>.face names slot {}",
            run.font,
            role.font()
        );
        assert_eq!(run.font, expect, "the role's own face moved under the test");

        // 2. the SIZE, the tracking and the figure box are the role's.
        assert_eq!(run.px, role.px(&ctx(&mut DrawList::new(), &mut fonts), 1.0));
        assert_eq!(run.track, role.tracking_px(run.px));
        let fig = role.figures(&mut fonts, run.font, run.px);
        assert_eq!(run.fig, fig.advance(), "the run was stepped by a box the role did not ask for");
        assert_eq!(fig.is_on(), role.tabular(), "type.<checkbox.role>.tabular");

        // 3. the PEN is two tokens and no measurement: the box's own
        //    length and the gap after it. This row has nothing to measure,
        //    and a `fonts.measure` appearing beside the draw one day would
        //    move this line.
        assert_eq!(run.x, ROW.x + px_of("checkbox.size") + px_of("checkbox.label_gap"));

        // 4. and from that pen the glyphs are the run this face makes.
        assert_eq!(
            run.xs,
            reference(&mut fonts, &run, run.font, &fig, LABEL),
            "the label's glyphs are not the ones its own face, size, tracking \
             and box lay down"
        );
        let wrong = other(run.font);
        let wrong_fig = crate::ui::figures(&mut fonts, wrong, run.px, fig.is_on());
        assert_ne!(
            run.xs,
            reference(&mut fonts, &run, wrong, &wrong_fig, LABEL),
            "slot {wrong} lays the label out exactly like slot {} — this machine \
             cannot tell the two faces apart and the test above proves nothing",
            run.font
        );
    }

    /// Writes `body` as a theme based on the master and runs `test` — an
    /// `#[ignore]`d sibling of this module — in a child process under it.
    fn under_theme(body: &str, test: &str) {
        let path = std::env::temp_dir()
            .join(format!("nacelle-checkbox-face-{}-{}.theme", std::process::id(), test));
        std::fs::write(
            &path,
            format!(
                "[meta]\nschema = 1\nname = \"Checkbox face fixture\"\nbase = \"default\"\n\n{body}"
            ),
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
    fn a_label_is_set_in_the_face_its_role_names() {
        for v in ["NACELLE_THEME_PATH", "NACELLE_THEME_NAME", "NACELLE_THEME_MASTER"] {
            assert!(std::env::var_os(v).is_none(), "{v} is set — this stage reads the master");
        }
        // `checkbox.role = body` and `type.body.face = ui`, so the master's
        // own answer here is the interface slot — which is why the master
        // alone cannot tell "asked the role" from "named FONT_UI", and why
        // the two fixtures below are the load-bearing half of this file.
        label_follows_its_role(FONT_UI);

        // The role does not ask for a figure box, so the run has none —
        // the control for the stage below, which turns the token on.
        assert!(!role().tabular(), "type.body.tabular is false in the master");
        let mut fonts = FontSystem::new();
        let ones = label_of(&mut fonts, ONES);
        let eights = label_of(&mut fonts, EIGHTS);
        assert_eq!(ones.fig, 0.0);
        assert_ne!(
            steps(&ones.xs),
            steps(&eights.xs),
            "a proportional label stepped 1111 and 8888 identically — this face \
             has uniform figures and the box below cannot be witnessed"
        );
        assert_ne!(
            width(&mut fonts, &ones, ONES),
            width(&mut fonts, &eights, EIGHTS),
            "a proportional label measured 1111 and 8888 the same width"
        );

        // ---- and the token is what decides, not this file ----------
        under_theme(
            "[type]\nbody.face = mono\n",
            "object::checkbox::tests::a_label_in_a_mono_theme_is_mono",
        );
        under_theme(
            "[type]\nbody.tabular = true\n",
            "object::checkbox::tests::a_label_under_a_tabular_role_is_boxed",
        );
        // ...and the BINDING is what says which role that is. `data` is a
        // shipped monospace role of another size entirely, so a row that
        // still answers `body` fails on both counts.
        under_theme(
            "[checkbox]\nrole = data\n",
            "object::checkbox::tests::a_rebound_row_follows_the_binding",
        );
    }

    // --------------------------------------------------------- the fixtures
    //
    // Run by the stage above, in a child process, under a theme of its
    // own. `#[ignore]` keeps them out of the ordinary pass, where the
    // master is what is resolved and they would be measuring nothing.

    #[test]
    #[ignore = "run by a_label_is_set_in_the_face_its_role_names under a fixture theme"]
    fn a_label_in_a_mono_theme_is_mono() {
        label_follows_its_role(FONT_MONO);
    }

    #[test]
    #[ignore = "run by a_label_is_set_in_the_face_its_role_names under a fixture theme"]
    fn a_label_under_a_tabular_role_is_boxed() {
        label_follows_its_role(FONT_UI);
        let mut fonts = FontSystem::new();
        let ones = label_of(&mut fonts, ONES);
        let eights = label_of(&mut fonts, EIGHTS);
        assert!(ones.fig > 0.0, "type.body.tabular = true and the run carried no box");
        // The STEP is the box, so a row of ones advances exactly as a row of
        // eights does. (The glyphs sit centred in their boxes, so a narrow
        // figure still starts a fraction further in — that offset is what a
        // fixed advance buys, not what it costs.)
        assert_eq!(
            steps(&ones.xs),
            steps(&eights.xs),
            "a boxed label still steps by the glyph: 1111 and 8888 advanced differently"
        );
        assert_eq!(
            width(&mut fonts, &ones, ONES),
            width(&mut fonts, &eights, EIGHTS),
            "a boxed label measured 1111 and 8888 at different widths"
        );
    }

    #[test]
    #[ignore = "run by a_label_is_set_in_the_face_its_role_names under a fixture theme"]
    fn a_rebound_row_follows_the_binding() {
        // `type.data.face = mono` and `type.data.size` is nothing like
        // `type.body.size`; `label_follows_its_role` checks both against
        // whatever role the binding lands on.
        label_follows_its_role(FONT_MONO);
        let mut fonts = FontSystem::new();
        assert_eq!(
            label_of(&mut fonts, LABEL).px,
            crate::ui::role("data").px(&ctx(&mut DrawList::new(), &mut fonts), 1.0),
            "the row kept `body`'s size after the binding moved to `data`"
        );
    }
}
