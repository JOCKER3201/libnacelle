//! The widget panel container — drawn by the HOST, for all twelve
//! widgets (u2 §4, spec §5.12).
//!
//! Until this existed there was no container, only four inventions of
//! one: the terminal's hand-drawn chamfer, the file browser's private
//! title arithmetic, the scripts' `title` element, and nothing at all.
//! Here the parts live once, outside in: fill/glass → edge ring → title
//! band (left text, right text, rule) → content box. The widget is then
//! handed the CONTENT BOX and draws content, never chrome.
//!
//! Every colour and metric is a theme token (`panel.*`, `elev.panel.*`,
//! the type role `panel.title.role` names, `component.panel.*`). There
//! is no fallback underneath any read: a missing token degrades through
//! the engine's per-kind default and is allowed to look raw.

use super::elev;
use crate::access::{AccessInfo, Role};
use crate::focus::FocusId;
use crate::font::Figures;
use crate::theme::{self, Color, TokenId};
use crate::ui;
use crate::view::surface::{CtxSurface, Surface};
use crate::widget::Chrome;
use crate::{Ctx, Rect};
use std::cell::RefCell;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Mutex, OnceLock};

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// A baked theme colour in the draw list's own colour type.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// A `same_as_parent` sentinel (baked negative) falls back to its parent
/// value; anything the theme actually stated is clamped to a length.
fn or_parent(v: f32, parent: f32) -> f32 {
    if v < 0.0 {
        parent
    } else {
        v
    }
}

/// The container's metrics, read fresh each call — they are what a mood,
/// a resize or a theme swap changes; the ids underneath are cached.
struct Metrics {
    border: f32,
    pad_x: f32,
    pad_y: f32,
    pad_y_min: f32,
    band_h: f32,
    band_h_min: f32,
    block_h: f32,
    min_content: f32,
}

fn metrics() -> Metrics {
    static BORDER: OnceLock<TokenId> = OnceLock::new();
    static PAD: OnceLock<TokenId> = OnceLock::new();
    static PAD_X: OnceLock<TokenId> = OnceLock::new();
    static PAD_Y: OnceLock<TokenId> = OnceLock::new();
    static PAD_Y_MIN: OnceLock<TokenId> = OnceLock::new();
    static BAND_H: OnceLock<TokenId> = OnceLock::new();
    static BAND_H_MIN: OnceLock<TokenId> = OnceLock::new();
    static BLOCK_H: OnceLock<TokenId> = OnceLock::new();
    static MIN_CONTENT: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let pad = t.px(tok(&PAD, "panel.content_pad")).max(0.0);
    Metrics {
        border: t.px(tok(&BORDER, "panel.border")).max(0.0),
        pad_x: or_parent(t.px(tok(&PAD_X, "panel.content_pad_x")), pad).max(0.0),
        pad_y: or_parent(t.px(tok(&PAD_Y, "panel.content_pad_y")), pad).max(0.0),
        // Step 1 of the degradation ladder shrinks pad_y toward this.
        pad_y_min: t.px(tok(&PAD_Y_MIN, "space.1")).max(0.0),
        band_h: t.px(tok(&BAND_H, "panel.title.band_h")).max(0.0),
        band_h_min: t.px(tok(&BAND_H_MIN, "panel.title.band_h_min")).max(0.0),
        block_h: t.px(tok(&BLOCK_H, "panel.title.block_h")).max(0.0),
        min_content: t.px(tok(&MIN_CONTENT, "panel.min_content_h")).max(0.0),
    }
}

/// What the degradation ladder settled on for one panel.
struct Placement {
    /// The widget's content box.
    content: Rect,
    /// The title band, when one survives. `(rect, collapsed)`.
    band: Option<(Rect, bool)>,
    /// The highest ladder step taken (0 = the panel had room).
    step: u8,
}

/// The height the container adds around a widget's content: what the
/// sizing pass must add to a `Sizing::Content` want before publishing it,
/// and divide back out of the box when computing the widget's scale
/// (u2 §4.2). Uses the resting metrics — the ladder exists for panels a
/// LAYOUT made short, and a panel sized from its content is not one.
pub fn chrome_extra(titled: bool) -> f32 {
    let m = metrics();
    2.0 * (m.border + m.pad_y) + if titled { m.block_h } else { 0.0 }
}

/// The ordered, stated, diagnosed ladder for panels too short for the
/// full container (u2 §4.2) — never a silent overlap:
///
/// 1. shrink the vertical content padding toward `space.1`;
/// 2. collapse the band to `panel.title.band_h_min`;
/// 3. drop the band (the widget's own inline title takes over);
/// 4. clamp the content box to `panel.min_content_h` and let the
///    widget's overflow policy decide.
fn place(r: Rect, titled: bool) -> Placement {
    let m = metrics();
    let inner_w = (r.w - 2.0 * (m.border + m.pad_x)).max(1.0);
    let cx = r.x + m.border + m.pad_x;
    let inner_h = (r.h - 2.0 * m.border).max(0.0);

    let content_h = |pad_y: f32, block: f32| inner_h - 2.0 * pad_y - block;
    let mut pad_y = m.pad_y;
    let mut block = if titled { m.block_h } else { 0.0 };
    let mut band = titled.then_some((m.band_h.min(block), false));
    let mut step = 0u8;

    // Step 1: give the padding back before touching the band.
    if content_h(pad_y, block) < m.min_content {
        step = 1;
        let need = m.min_content - content_h(pad_y, block);
        pad_y = (pad_y - need / 2.0).max(m.pad_y_min);
    }
    // Step 2: the collapsed band — smaller, but still a band.
    if titled && content_h(pad_y, block) < m.min_content {
        step = 2;
        block = m.band_h_min.min(m.band_h);
        band = Some((block, true));
    }
    // Step 3: no band; the widget's `title` element draws inline as it
    // does today, and nothing of the heading is lost.
    if titled && content_h(pad_y, block) < m.min_content {
        step = 3;
        block = 0.0;
        band = None;
    }
    // Step 4: the box will not fit even bare — clamp it and let the
    // widget's overflow policy (scale to its floor, then clip) act.
    let mut h = content_h(pad_y, block);
    if h < m.min_content {
        step = 4;
        h = m.min_content.min(inner_h.max(1.0));
    }

    let band = band.map(|(bh, collapsed)| {
        (
            Rect::new(cx, r.y + m.border + pad_y, inner_w, bh),
            collapsed,
        )
    });
    Placement {
        content: Rect::new(cx, r.y + m.border + pad_y + block, inner_w, h.max(1.0)),
        band,
        step,
    }
}

/// The content box the container leaves inside a panel rect — the same
/// arithmetic [`draw`] uses, without drawing. For code that must answer
/// geometry with no frame in flight.
pub fn content_box(r: Rect, titled: bool) -> Rect {
    place(r, titled).content
}

/// How many times each ladder step has been entered, for tests and
/// diagnostics. Index 0 = step 1.
pub fn degradation_counts() -> [u32; 4] {
    [0, 1, 2, 3].map(|i| COUNTS[i].load(Ordering::Relaxed))
}

static COUNTS: [AtomicU32; 4] =
    [AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0), AtomicU32::new(0)];

/// Says once — per panel, per theme epoch — which ladder step a panel
/// landed on, and bumps that step's counter. Sixty identical lines a
/// second would bury everything else; a theme swap is allowed to say it
/// again, because the numbers it complains about have changed.
fn report_step(panel: usize, step: u8) {
    if step == 0 {
        return;
    }
    static SEEN: Mutex<Vec<(u32, usize, u8)>> = Mutex::new(Vec::new());
    let epoch = theme::epoch();
    let Ok(mut seen) = SEEN.lock() else { return };
    if seen.iter().any(|&(e, p, s)| e == epoch && p == panel && s == step) {
        return;
    }
    seen.retain(|&(e, _, _)| e == epoch);
    seen.push((epoch, panel, step));
    COUNTS[(step - 1) as usize].fetch_add(1, Ordering::Relaxed);
    eprintln!(
        "nacelle: panel {panel} is too short for its container — degradation step {step} \
         ({})",
        match step {
            1 => "vertical padding shrunk toward space.1",
            2 => "title band collapsed to panel.title.band_h_min",
            3 => "title band dropped; the widget's inline title stands in",
            _ => "content box clamped to panel.min_content_h",
        }
    );
}

/// Draws the container for one panel and answers the content box.
///
/// `r` is the widget box — the panel rect already deflated by the USER's
/// GridPadding, which is a layout preference and not the theme's. The
/// container draws inside it: `elev.panel`'s material, `shape`d ring and
/// edge glow, then the title band from `chrome`, and the same rect this
/// returns must be the one `click` and `wheel` later receive (u2 §4.1).
pub fn draw(ctx: &mut Ctx, r: Rect, chrome: &Chrome, panel_idx: usize) -> Rect {
    let titled = chrome.title.is_some() || chrome.right.is_some();
    let placed = place(r, titled);

    // A passive landmark, not a Tab stop: FocusCtl::register is
    // deliberately never called here (a panel container is not a
    // focusable control), but AccessCtl is a second, structural-only
    // registry a bridge can still read from, so a screen reader gets
    // "you are in the <title> panel" even without one. This does not
    // model the panel's CONTENT as children of this node — full
    // parent/child nesting is a known simplification left for later,
    // not an oversight; see `crate::access`'s module header for why the
    // two registries are kept apart.
    if let Some(title) = chrome.title.as_deref() {
        if let Some(ac) = ctx.access.as_deref_mut() {
            ac.register(
                FocusId::of(&format!("panel.{panel_idx}")),
                r,
                AccessInfo::new(Role::Group, title),
            );
        }
    }

    // Material, ring, and family A's bloom over the ring — read as a
    // whole rung rather than key by key, and the rung is the one
    // `panel.elev` NAMES. `elev.panel.glass.rank` is 0 in every shipped
    // theme, so the body is the fill; the glass pair joins when the
    // renderer's blur ranks do (Appendix B R3/R6), and it joins for every
    // rung at once because there is one reader.
    rung().draw_glassed(ctx, r, glass_box(r, placed.content));

    report_step(panel_idx, placed.step);
    if let Some((band, collapsed)) = placed.band {
        draw_band(ctx, band, collapsed, r, chrome, panel_idx);
    }
    // Where the baseline grid stands for everything drawn into this
    // panel, when the theme measures it from the content and not from the
    // screen (`rhythm.snap_origin`). Published here because this is the
    // one place a content box is settled — see
    // [`crate::view::paint::set_grid_origin`].
    crate::view::paint::set_grid_origin(placed.content.y);
    placed.content
}

/// The elevation rung a resting panel draws its material from —
/// `panel.elev`.
///
/// The name was `"elev.panel"`, written here, which made the key a
/// promise the file could not keep: a theme moving a panel up or down the
/// ladder (a board of `elev.raised` cards, a dock at `elev.fixture`) got
/// the same surface it started with. The rung is now the WORD the token
/// stands at.
///
/// Memoised per (content epoch, word): a `Level` is a dozen name lookups
/// and this is a per-panel, per-frame path, but the answer has to move
/// when the theme does — and a theme swap renumbers the open word set,
/// which is why the epoch is half the key.
///
/// The CONTENT epoch, because a `Level` holds token ids and enum indices
/// and no resolved value at all (see [`elev::Level`]'s own note), so
/// nothing in it moves when the viewport does. Keyed on [`theme::epoch`]
/// it missed on every frame of a desktop with two monitor heights, which
/// is the exact shape of the bug [`theme::content_epoch`] was added for.
fn rung() -> elev::Level {
    static ELEV: OnceLock<TokenId> = OnceLock::new();
    thread_local! {
        static CACHE: RefCell<Option<(u32, u16, elev::Level)>> = const { RefCell::new(None) };
    }
    let id = tok(&ELEV, "panel.elev");
    let word = theme::resolved().enum_of(id);
    let epoch = theme::content_epoch();
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((e, w, level)) = c.as_ref() {
            if *e == epoch && *w == word {
                return *level;
            }
        }
        let word_str = ui::theme_word(id);
        let mut level = elev::Level::of(&format!("elev.{word_str}"));
        // ONE MODEL OF A WINDOW (rule 11): a widget's panel is the settings
        // window's own surface, so its ring reads the SAME border the frame
        // does — `component.panel.border`, the shared root the editor writes
        // — not the rung's raw `edge.color`, which an older save could have
        // pinned to a literal while the window moved on. Colour only, and
        // only on `elev.panel`: a widget lifted to another rung draws that
        // rung's own ring.
        if word_str == "panel" {
            level = level.with_edge_color("component.panel.border");
        }
        *c = Some((epoch, word, level));
        level
    })
}

/// Which rectangle the glass quad fills — `panel.glass.rect` — pulled in
/// by `panel.glass.inset`.
///
/// `border_box` frosts the whole container, title band included;
/// `content_box` frosts the body alone and leaves the band standing on
/// the bed, which is the reading of image 7 the key was written for.
/// Neither had a reader: the quad was laid on the widget box whatever the
/// theme said.
fn glass_box(r: Rect, content: Rect) -> Rect {
    static RECT: OnceLock<TokenId> = OnceLock::new();
    static INSET: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let base = if ui::theme_word(tok(&RECT, "panel.glass.rect")) == "content_box" {
        content
    } else {
        r
    };
    // A negative inset would grow the quad past the ring it is poured
    // into, which is not a shrink and not what the key says.
    let inset = t.px(tok(&INSET, "panel.glass.inset")).max(0.0);
    Rect::new(
        base.x + inset,
        base.y + inset,
        (base.w - 2.0 * inset).max(0.0),
        (base.h - 2.0 * inset).max(0.0),
    )
}

/// The title band: left text, right text trimmed from the LEFT to the
/// room the title leaves (a path keeps its tail), and the hairline rule
/// on the band's floor.
fn draw_band(
    ctx: &mut Ctx,
    band: Rect,
    collapsed: bool,
    panel: Rect,
    chrome: &Chrome,
    panel_idx: usize,
) {
    static ROLE: OnceLock<TokenId> = OnceLock::new();
    static INSET_X: OnceLock<TokenId> = OnceLock::new();
    static GAP: OnceLock<TokenId> = OnceLock::new();
    static LEFT_C: OnceLock<TokenId> = OnceLock::new();
    static RIGHT_C: OnceLock<TokenId> = OnceLock::new();
    static RULE_W: OnceLock<TokenId> = OnceLock::new();
    static RULE_INSET: OnceLock<TokenId> = OnceLock::new();
    static RULE_C: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();

    // The role `panel.title.role` NAMES — size, tracking, case, leading and
    // the role's own alpha all move together when a theme repoints the
    // binding, which is what the binding is for. The container-query factor
    // is runtime state, so it multiplies here, and a collapsed band caps the
    // size so the text never overruns the band it was shrunk to keep.
    let role = ui::bound_role(&ROLE, "panel.title.role");
    // One key of the role that `Role` still does not carry. It hangs off
    // the role's NAME, which the binding states in a word, so it is
    // spelled from that word rather than pinned to `title.panel` here.
    // The px floor is NOT among them any more — `Role::px` applies the
    // role's own, so a floor spelled again here would be a second answer;
    // and neither is the CASE, which `Role::case` now carries with the
    // rest of the role instead of being re-spelled beside it.
    let alpha = {
        let mut sf = CtxSurface::new(ctx);
        let word = sf.word("panel.title.role");
        sf.px(&format!("type.{word}.alpha")).clamp(0.0, 1.0)
    };
    let mut px = role.px(ctx, 1.0);
    let leading = role.leading();
    // A leading of zero is a broken role, not a short band: dividing by it
    // is what must not happen, and the role draws whatever it draws.
    if collapsed && leading > 0.0 && px * leading > band.h {
        px = (band.h / leading).max(1.0);
    }
    let spacing = role.tracking_px(px);
    // WHICH FACE the band is set in, and the figure box it steps its
    // digits by — both the role's, read once and carried to every
    // measure and every draw below. Naming `FONT_UI` here is what made
    // `type.title.panel.face = ui_medium` come out as the interface
    // Regular: the master's word had no way past this line.
    let face = role.font();
    let fig = role.figures(ctx.fonts, face, px);

    // The role's own transform, through the toolkit's one applier. This
    // was a fourth copy of the same `match`, and like the other three it
    // ended on `_ => to_uppercase()`: a theme with a typo in its `case`
    // key got a shouting title band and no word about why.
    let case = role.case();
    let cased = |s: &str| ui::recase(case, s).into_owned();

    let inset = t.px(tok(&INSET_X, "panel.title.inset_x")).max(0.0);
    let gap = t.px(tok(&GAP, "panel.title.gap")).max(0.0);
    let y = band.y + (band.h - px * leading) / 2.0;

    let left = chrome.title.as_deref().map(cased).unwrap_or_default();
    let left_c = col(t.color(tok(&LEFT_C, "component.panel.title")));
    let left_c = left_c.alpha(left_c.a * alpha);
    if !left.is_empty() {
        ctx.dl.text_fig(
            ctx.fonts,
            face,
            px,
            band.x + inset,
            y,
            &left,
            left_c,
            spacing,
            &fig,
        );
    }

    if let Some(right) = chrome.right.as_deref() {
        let right = cased(right);
        // The room the right text gets is what the LEFT one leaves, so
        // this measure has to be the one that drew it: same face, same
        // px, same box. Measured proportionally under a boxed role, the
        // gap would be short by the difference and the two texts would
        // collide at the width the master picked to keep them apart.
        let used = if left.is_empty() {
            0.0
        } else {
            ctx.fonts.measure_fig(face, px, &left, spacing, &fig) + gap
        };
        let room = (band.w - 2.0 * inset - used).max(0.0);
        let shown = fit_lead(ctx, face, px, &right, spacing, room, &fig);
        if !shown.is_empty() {
            // The one text in the chrome that is routinely cut: a cwd
            // keeps its tail and loses its root, and the root is exactly
            // what the user cannot reconstruct (F2 §8.1). The anchor is
            // the trimmed text's own box, so resting on the TITLE — a
            // different word, drawn whole — says nothing. The identity
            // is the panel's place plus the path, so two browsers open
            // on one directory are still two things to explain.
            let tw = ctx.fonts.measure_fig(face, px, &shown, spacing, &fig);
            crate::view::paint::explain_trim(
                &mut CtxSurface::new(ctx),
                crate::object::tooltip::cell_key(0, panel_idx, &right),
                Rect::new(band.right() - inset - tw, band.y, tw, band.h),
                &shown,
                &right,
            );
            let right_c = col(t.color(tok(&RIGHT_C, "panel.title_right_color")));
            let right_c = right_c.alpha(right_c.a * alpha);
            ctx.dl.text_right_fig(
                ctx.fonts,
                face,
                px,
                band.right() - inset,
                y,
                &shown,
                right_c,
                spacing,
                &fig,
            );
        }
    }

    // The rule on the band's floor. `panel.title.rule` is a stroke or
    // none; a stroke that bakes to nothing draws nothing.
    let rule_w = t.px(tok(&RULE_W, "panel.title.rule")).max(0.0);
    if rule_w > 0.0 {
        static BORDER: OnceLock<TokenId> = OnceLock::new();
        let b = t.px(tok(&BORDER, "panel.border")).max(0.0);
        let rin = t.px(tok(&RULE_INSET, "panel.title.rule_inset")).max(0.0);
        let ry = band.bottom();
        ctx.dl.line(
            panel.x + b + rin,
            ry,
            panel.right() - b - rin,
            ry,
            rule_w,
            col(t.color(tok(&RULE_C, "component.panel.header_underline"))),
        );
    }
}

/// Shortens `text` from the LEFT with a leading ellipsis until it fits —
/// the file browser's cwd trim, now in the one place that draws the band
/// (u2 §4.3): the tail of a path is the part worth keeping.
///
/// `face` and `fig` are the caller's, never this function's: a string
/// trimmed against one face and drawn in another either loses a
/// character it had room for or overruns the band it was trimmed to fit,
/// and which of the two happens depends on the theme — so nobody looking
/// at a cut path can tell whether the trim or the draw is wrong.
#[allow(clippy::too_many_arguments)]
fn fit_lead(
    ctx: &mut Ctx,
    face: u8,
    px: f32,
    text: &str,
    spacing: f32,
    max_w: f32,
    fig: &Figures,
) -> String {
    if max_w <= 0.0 {
        return String::new();
    }
    if ctx.fonts.measure_fig(face, px, text, spacing, fig) <= max_w {
        return text.to_string();
    }
    // `type.ellipsis`, the same key the trailing trimmers read — a marker
    // is a marker whichever end of the run it hangs off.
    let cut = ui::ellipsis();
    let chars: Vec<char> = text.chars().collect();
    let mut start = 1;
    while start < chars.len() {
        let cand: String = cut.chars().chain(chars[start..].iter().copied()).collect();
        if ctx.fonts.measure_fig(face, px, &cand, spacing, fig) <= max_w {
            return cand;
        }
        start += 1;
    }
    String::new()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::draw::{DrawCmd, DrawList};
    use crate::font::{FontSystem, FONT_MONO};
    use crate::pointer::Pointer;
    use std::path::{Path, PathBuf};

    // ------------------------------------------------------ face harness
    //
    // Every object in this directory has to answer the same question —
    // "did the run reach the draw list in the face its ROLE names, and
    // does it move when a theme moves the role?" — and the answer is
    // measured the same way in each. The harness is therefore written
    // once, here, and used from the other objects' own test modules: a
    // second copy is a second definition of what counts as proof.

    /// Every (font slot, string) `f` sent to the draw list, read from the
    /// COMMAND REGISTER rather than inferred from the vertex buffer: the
    /// register holds the slot the call was made with, which is exactly
    /// the claim under test, and it holds it even on a machine whose
    /// atlas could not rasterise a single glyph.
    pub(crate) fn drawn_text(f: impl FnOnce(&mut Ctx)) -> Vec<(u8, String)> {
        drawn_runs(f)
            .into_iter()
            .filter_map(|c| match c {
                DrawCmd::Text { font, text, .. } => Some((font, text)),
                _ => None,
            })
            .collect()
    }

    /// [`drawn_text`] with the whole command kept: the anchor, the px,
    /// the tracking and the figure advance a run was made with — what a
    /// claim about COLUMNS needs and a slot number cannot carry.
    pub(crate) fn drawn_runs(f: impl FnOnce(&mut Ctx)) -> Vec<DrawCmd> {
        crate::draw::arm_cmds();
        let mut dl = DrawList::new();
        let mut fonts = FontSystem::new();
        {
            let mut ctx = Ctx {
                access: None,
                dl: &mut dl,
                fonts: &mut fonts,
                w: 1920.0,
                h: 1080.0,
                // Well past every unfold in the master, so an animated
                // object is measured at rest rather than mid-open.
                t: 1000.0,
                mouse: Pointer::new(-1.0, -1.0),
                term_font_scale: 1.0,
                ui_font_scale: 1.0,
                panel_scale: 1.0,
                focus: None,
                tips: None,
            };
            f(&mut ctx);
        }
        dl.cmds()
            .iter()
            .filter(|c| matches!(c, DrawCmd::Text { .. }))
            .cloned()
            .collect()
    }

    /// What one child run reported.
    pub(crate) struct Measured {
        /// The font slot every text of the run was drawn in.
        pub face: u8,
        /// The role word the binding resolved to in that run.
        pub role: String,
        /// The whole child output, for a failure message worth reading.
        pub log: String,
    }

    impl Measured {
        /// Any other `KEY=value` the child chose to print — the figure
        /// advance, say. Read from the log rather than added to the
        /// struct, so one more measurement is one more `println` in a
        /// child and not a change every child has to follow.
        pub(crate) fn field(&self, key: &str) -> String {
            read_field(&self.log, key, "the child")
        }
    }

    /// One `KEY=value` out of a child's output. Anywhere in the line,
    /// not at its head: the test harness writes "test <name> ... "
    /// without a newline, so a child's first `println` lands on the tail
    /// of the harness's own line.
    fn read_field(log: &str, key: &str, who: &str) -> String {
        log.lines()
            .find_map(|l| l.split_once(key).map(|(_, v)| v))
            .unwrap_or_else(|| panic!("{who} printed no {key} line:\n{log}"))
            .trim()
            .to_string()
    }

    /// The role word a `*_role` binding stands at — the name whose
    /// `type.<name>.face` a fixture has to move. Read rather than
    /// written down, so this batch keeps working when the master
    /// repoints a binding.
    pub(crate) fn role_word(binding: &str) -> String {
        crate::ui::theme_word(theme::id(binding).unwrap_or(TokenId::MISSING))
    }

    /// The line a child test prints so its parent can read the slot back.
    pub(crate) fn report(role: &str, face: u8, drawn: &[(u8, String)]) {
        println!("ROLE={role}");
        println!("FACE={face}");
        for (f, s) in drawn {
            println!("drew {f} \"{s}\"");
        }
    }

    /// Asserts every text of a run went to `want`, and answers the run.
    pub(crate) fn all_in(drawn: &[(u8, String)], want: u8) {
        assert!(!drawn.is_empty(), "nothing was drawn at all — the run proves nothing");
        for (face, text) in drawn {
            assert_eq!(
                *face, want,
                "\"{text}\" reached the draw list in slot {face}; its role names {want}"
            );
        }
    }

    /// Runs one `#[ignore]`d child test in a PROCESS of its own, under
    /// `theme`, and reads back what it drew.
    ///
    /// A process of its own because the resolved theme is process-wide
    /// and `cargo test` runs a binary's tests in parallel threads: a test
    /// that swapped the theme in-process would decide what every other
    /// test in the suite was measuring. The child is this same test
    /// binary re-exec'd — no fixture crate, no second target directory,
    /// nothing for the other three authors' builds to trip over — with
    /// `NACELLE_THEME_PATH` pointing at the theme under test.
    pub(crate) fn measure_in_child(test: &str, theme: Option<&Path>) -> Measured {
        let exe = std::env::current_exe().expect("the test binary must be locatable");
        let mut cmd = std::process::Command::new(exe);
        cmd.args(["--exact", test, "--ignored", "--nocapture", "--test-threads=1"]);
        cmd.env("NACELLE_DRAW_CMDS", "1");
        match theme {
            Some(p) => cmd.env("NACELLE_THEME_PATH", p),
            None => cmd.env_remove("NACELLE_THEME_PATH"),
        };
        let out = cmd.output().expect("the child measuring process must start");
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert!(out.status.success(), "{test} failed in its own process:\n{log}");
        Measured {
            face: read_field(&log, "FACE=", test).parse().expect("FACE= is a slot number"),
            role: read_field(&log, "ROLE=", test),
            log,
        }
    }

    /// A theme that inherits the shipped master and moves ONE role's
    /// face to `mono` — the whole fixture, so nothing else can explain a
    /// change of slot.
    pub(crate) fn mono_theme(tag: &str, role: &str) -> PathBuf {
        let path = std::env::temp_dir()
            .join(format!("nacelle-face-{tag}-{}.theme", std::process::id()));
        std::fs::write(
            &path,
            format!(
                "[meta]\nschema = 1\nname = \"face {tag}\"\nbase = \"default\"\n\n\
                 [type]\n{role}.face = mono\n"
            ),
        )
        .expect("the fixture theme must be writable");
        path
    }

    /// The whole claim for one object: the run went to the face its role
    /// names under the MASTER, a theme that moves that role's face to
    /// mono moves the run with it, and the two runs are not the same
    /// slot — which is the part a `FONT_UI` written at the call site
    /// cannot satisfy however the master is wired.
    pub(crate) fn face_follows_the_theme(tag: &str, child: &str) {
        let master = measure_in_child(child, None);
        let fixture = mono_theme(tag, &master.role);
        let moved = measure_in_child(child, Some(&fixture));
        let _ = std::fs::remove_file(&fixture);
        assert_eq!(
            moved.role, master.role,
            "the fixture changed which ROLE is bound, not which face it is set in"
        );
        assert_eq!(
            moved.face, FONT_MONO,
            "a theme put `type.{}.face = mono` and {tag} drew slot {} instead:\n{}",
            master.role, moved.face, moved.log
        );
        assert_ne!(
            master.face, moved.face,
            "{tag} drew the same slot under both themes ({}), so nothing was proved: \
             the face is still being chosen at the call site",
            master.face
        );
    }

    // ------------------------------------------------------- panel's own

    /// The title band draws in the face `panel.title.role` names, and
    /// follows a theme that moves it.
    #[test]
    fn the_band_is_set_in_the_face_its_role_names() {
        face_follows_the_theme("panel", "object::panel::tests::child_band_face");
    }

    /// [`the_band_is_set_in_the_face_its_role_names`]'s child: one titled
    /// panel with both halves of a band, drawn for real.
    #[test]
    #[ignore = "measured in a process of its own by the test above"]
    fn child_band_face() {
        static PROBE: OnceLock<TokenId> = OnceLock::new();
        let role = ui::bound_role(&PROBE, "panel.title.role");
        let want = role.font();
        let drawn = drawn_text(|ctx| {
            let chrome = Chrome {
                title: Some("MONITOR ZASOBOW".to_string()),
                // Long enough to be trimmed: `fit_lead` measures with the
                // same face and box the draw uses, and a trim measured in
                // another face is exactly what this run would not catch
                // if the text always fitted.
                right: Some("/var/home/michael/.git/nacelle/src/object".to_string()),
                ..Chrome::none()
            };
            draw(ctx, Rect::new(40.0, 40.0, 420.0, 260.0), &chrome, 0);
        });
        all_in(&drawn, want);
        assert_eq!(drawn.len(), 2, "the band draws its left half and its right half");
        assert!(
            drawn[1].1.starts_with('\u{2026}'),
            "the right half fitted whole, so the trim — the one measure that is \
             taken apart from its draw — was never exercised: {:?}",
            drawn[1].1
        );
        report(&role_word("panel.title.role"), want, &drawn);
    }

    // ------------------------------------------------- passive a11y landmark

    /// Builds a bare `Ctx` wired to a live `AccessCtl`, draws one panel
    /// into it, and answers what a bridge would read back after the
    /// frame closes. Kept separate from [`drawn_runs`] (which always
    /// hands `draw` an `access: None` Ctx) because most callers of that
    /// harness have nothing to do with the structural registry, and
    /// giving every one of them a live `AccessCtl` they never read would
    /// be the thing this file's own tests exist to avoid: a check that
    /// looks like coverage but proves nothing about the field under
    /// test.
    fn drawn_access_entries(
        chrome: &Chrome,
        r: Rect,
        panel_idx: usize,
    ) -> Vec<(FocusId, AccessInfo)> {
        crate::draw::arm_cmds();
        let mut dl = DrawList::new();
        let mut fonts = FontSystem::new();
        let mut ac = crate::access::AccessCtl::new();
        {
            let mut ctx = Ctx {
                access: Some(&mut ac),
                dl: &mut dl,
                fonts: &mut fonts,
                w: 1920.0,
                h: 1080.0,
                t: 1000.0,
                mouse: Pointer::new(-1.0, -1.0),
                term_font_scale: 1.0,
                ui_font_scale: 1.0,
                panel_scale: 1.0,
                focus: None,
                tips: None,
            };
            draw(&mut ctx, r, chrome, panel_idx);
        }
        ac.begin_frame();
        ac.entries().map(|(id, _, info)| (id, info.clone())).collect()
    }

    /// A titled panel registers itself as a `Role::Group` landmark under
    /// `panel.<panel_idx>` — not a Tab stop (this never goes through
    /// `FocusCtl::register`), but enough for a bridge to say "you are in
    /// the <title> panel" while the widget it contains still has no
    /// modeled parent/child link to this node.
    #[test]
    fn a_titled_panel_registers_a_passive_group_landmark() {
        let chrome = Chrome { title: Some("Files".to_string()), ..Chrome::none() };
        let got = drawn_access_entries(&chrome, Rect::new(0.0, 0.0, 300.0, 200.0), 3);
        assert_eq!(got.len(), 1, "exactly one structural landmark per panel: {got:?}");
        assert_eq!(got[0].0, FocusId::of("panel.3"));
        assert_eq!(got[0].1.role, Role::Group);
        assert_eq!(got[0].1.name, "Files");
    }

    /// An untitled panel — no `chrome.title` — has nothing worth
    /// announcing as a landmark and registers nothing, whether or not
    /// `ctx.access` is wired up.
    #[test]
    fn an_untitled_panel_registers_nothing() {
        let got = drawn_access_entries(&Chrome::none(), Rect::new(0.0, 0.0, 300.0, 200.0), 0);
        assert!(got.is_empty(), "no title, no landmark: {got:?}");
    }

    // ---------------------------------------------------- the trim marker
    //
    // `type.ellipsis` has been in the master since it was written, and its
    // comment names the very call sites that ignored it: "a console theme
    // may prefer `...` or `>`". Four trimmers in this library appended
    // `"\u{2026}"` out of their own source instead, so a theme could ask
    // and get nothing. They are exercised together because the failure
    // that matters is not "one of them ignores the key" but "they do not
    // agree" — one trim marker in a list and another in the band above it
    // is the state this test exists to make impossible.

    /// What each of the four trims makes of one overlong string.
    fn four_trims(ctx: &mut Ctx) -> [String; 4] {
        use crate::font::FONT_UI;
        const LONG: &str = "/var/home/michael/.git/nacelle/src/object/panel.rs";
        // Narrow enough that all four have to cut, wide enough that all
        // four keep something besides the marker.
        const ROOM: f32 = 90.0;
        const PX: f32 = 14.0;
        [
            crate::view::paint::fit_end_tab(&mut CtxSurface::new(ctx), FONT_UI, PX, LONG, ROOM, 0.0, false),
            crate::base::fit_end(ctx, PX, LONG, ROOM),
            crate::draw::fit_tail(ctx.fonts, FONT_UI, PX, LONG, 0.0, ROOM),
            fit_lead(ctx, FONT_UI, PX, LONG, 0.0, ROOM, &Figures::NONE),
        ]
    }

    fn trims() -> [String; 4] {
        let mut got: Option<[String; 4]> = None;
        drawn_runs(|ctx| got = Some(four_trims(ctx)));
        got.expect("the harness ran the closure")
    }

    #[test]
    fn every_trim_in_the_toolkit_ends_on_the_character_the_theme_states() {
        let names = ["view::paint::fit_end_tab", "base::fit_end", "draw::fit_tail", "fit_lead"];
        // The shipped master states the ellipsis, so that is what all
        // four cut with — three at the tail, the band's own at the head.
        let cut = trims();
        for (i, got) in cut.iter().enumerate() {
            assert!(got.len() > 1, "{} kept nothing but a marker: {got:?}", names[i]);
            let marked = if i == 3 { got.starts_with('\u{2026}') } else { got.ends_with('\u{2026}') };
            assert!(marked, "{} did not mark its cut: {got:?}", names[i]);
        }
        // Now the theme says a comma — one key, and every one of the four
        // has to follow it. This is the assertion that fails on the code
        // this test was written against, where the character was in the
        // Rust and no theme could reach it.
        crate::ui::seed_theme_text("type.ellipsis", ",");
        let cut = trims();
        for (i, got) in cut.iter().enumerate() {
            let marked = if i == 3 { got.starts_with(',') } else { got.ends_with(',') };
            assert!(marked, "{} kept its own marker over the theme's: {got:?}", names[i]);
            assert!(
                !got.contains('\u{2026}'),
                "{} answered the theme AND its own character: {got:?}",
                names[i]
            );
        }
    }

    /// A tall panel keeps the full container: band, padding, and a
    /// content box strictly inside the widget box.
    #[test]
    fn a_tall_panel_gets_band_and_padding() {
        let r = Rect::new(10.0, 10.0, 300.0, 200.0);
        let p = place(r, true);
        assert_eq!(p.step, 0);
        let (band, collapsed) = p.band.expect("a tall titled panel keeps its band");
        assert!(!collapsed);
        assert!(band.y >= r.y);
        assert!(p.content.y >= band.bottom());
        assert!(p.content.bottom() <= r.bottom() + 0.01);
        assert!(p.content.x > r.x && p.content.right() < r.right());
        // An untitled panel gets the same box without the band.
        let q = place(r, false);
        assert!(q.band.is_none());
        assert!(q.content.h > p.content.h);
    }

    /// The ladder: shrinking panels lose padding first, then collapse
    /// the band, then drop it — stated, ordered, never a silent overlap.
    #[test]
    fn short_panels_degrade_in_ladder_order() {
        let m = metrics();
        // Comfortable: content_pad + band + a roomy content box.
        let tall = 2.0 * (m.border + m.pad_y) + m.block_h + m.min_content * 4.0;
        assert_eq!(place(Rect::new(0.0, 0.0, 300.0, tall), true).step, 0);
        // sysinfo's real height at 1080p: 48.6 px panel, 32.6 widget box.
        // The full container cannot fit; the ladder must answer, and the
        // content box must never fall under min_content while the panel
        // itself can hold it.
        let p = place(Rect::new(0.0, 0.0, 300.0, 32.6), true);
        assert!(p.step >= 1, "a short panel must take a ladder step");
        assert!(p.content.h + 0.01 >= m.min_content.min(32.6 - 2.0 * m.border));
        // Short enough that the band cannot survive at all.
        let q = place(Rect::new(0.0, 0.0, 300.0, m.min_content + 1.0), true);
        assert!(q.band.is_none(), "step 3 drops the band");
        // The band, while it survives, sits above the content.
        let mid = 2.0 * m.border + m.pad_y_min * 2.0 + m.band_h_min + m.min_content + 1.0;
        let s = place(Rect::new(0.0, 0.0, 300.0, mid), true);
        if let Some((band, _)) = s.band {
            assert!(band.bottom() <= s.content.y + 0.01);
        }
    }

    /// `chrome_extra` and `place` are the same arithmetic: a panel given
    /// exactly `content + chrome_extra` hands the content back whole.
    #[test]
    fn chrome_extra_round_trips_through_place() {
        let m = metrics();
        let want = m.min_content * 3.0;
        for titled in [false, true] {
            let r = Rect::new(0.0, 0.0, 300.0, want + chrome_extra(titled));
            let p = place(r, titled);
            assert_eq!(p.step, 0, "titled={titled}");
            assert!(
                (p.content.h - want).abs() < 0.01,
                "titled={titled}: got {} want {want}",
                p.content.h
            );
        }
    }
}
