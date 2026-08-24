//! Tabs: a strip of sheared plates, one per page, of which exactly one
//! is showing.
//!
//! This is the shell's session strip as a general object. It reads the
//! same `[tab]` section that strip reads today — `h`, `skew`, `pad`,
//! `gap`, `rule`, `rule_gap`, `role`, `underline_active` — and wears the
//! same `tab` class ladder, so the two can eventually become one drawing
//! path; until a pixel-for-pixel proof exists, the terminal keeps its
//! own (F2 §11) and this object serves everything else.
//!
//! Written against [`Surface`], like every other view in this crate: the
//! same code draws a strip for the desktop and, across the ABI, a strip
//! for a plugin. The state lives in the caller — [`StripState`] — which
//! is what makes a strip cheap enough to draw immediate.
//!
//! Two differences to settle before that migration, noted here so the
//! agent who attempts it does not rediscover them: the shell divides its
//! rect into equal tabs where this object measures each from its label,
//! and the shell applies the optical centring bias only under an
//! upper-case or smallcaps role where [`paint::center_line_y`] applies
//! it whenever `rhythm.center_mode` is optical. They agree on the master
//! (`type.button.case = upper`) and may not on a theme that moves it.

use super::focus_ring;
use crate::access::{AccessInfo, Role, States};
use crate::focus::{Caps, FocusId, Key, KeyEv, Mods};
use crate::theme::parse::State;
use crate::theme::{self, Color, TokenId};
use crate::ui::Align;
use crate::view::paint::{self, RoleLook};
use crate::view::surface::{CtxSurface, Surface};
use crate::view::{Hit, Hits};
use crate::{Ctx, Rect};
use std::sync::OnceLock;

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// Floating-point slack for the width solver: a fraction of a pixel is
/// not an overflow.
const EPS: f32 = 0.01;

// ---------------------------------------------------------------------
// State
// ---------------------------------------------------------------------

/// What a strip remembers between frames: which page is showing, which
/// cell the pointer is over, and which one is flashing under a click.
///
/// Focus is deliberately absent. A strip is ONE control in the focus
/// chain — the arrows move the active cell INSIDE it — so the ring is
/// the chain's answer at draw time and never a field here, exactly as
/// `object::button` carries no `focused` either.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StripState {
    pub active: usize,
    pub hover: Option<usize>,
    pub flash: Option<usize>,
}

impl StripState {
    pub fn new(active: usize) -> StripState {
        StripState { active, hover: None, flash: None }
    }

    /// Which ladder rung cell `i` occupies. A decaying click flash IS
    /// press, and the showing page keeps its selection under the
    /// pointer as selected_hover — `object::button`'s rule, one strip
    /// wide.
    pub fn rung(&self, i: usize) -> State {
        if self.flash == Some(i) {
            State::Press
        } else if self.active == i && self.hover == Some(i) {
            State::SelectedHover
        } else if self.active == i {
            State::Selected
        } else if self.hover == Some(i) {
            State::Hover
        } else {
            State::Idle
        }
    }

    /// Moves the active cell by `delta` among `n` cells, wrapping at
    /// either end — what an arrow key does inside the one control the
    /// strip registers as.
    pub fn step(&mut self, n: usize, delta: isize) {
        if n == 0 {
            return;
        }
        let n_i = n as isize;
        let at = self.active.min(n - 1) as isize;
        self.active = (((at + delta) % n_i + n_i) % n_i) as usize;
    }
}

/// The caller's runtime facts. Everything a strip LOOKS like is read
/// from the theme inside it.
#[derive(Clone, Copy, Debug)]
pub struct StripStyle {
    /// A shrink on the LABEL and nothing else, exactly as in
    /// `object::button`: a strip's own metrics belong to the theme, and a
    /// text factor may not move them. A label that no longer fits is
    /// trimmed, which is what the fit floor is for.
    ///
    /// NOT the interface font-size setting. That one is `metric.ui_scale`
    /// and is already inside every baked size; a caller that passed
    /// `Ctx::ui_font_scale` here would apply it twice.
    pub text_scale: f32,
}

impl Default for StripStyle {
    fn default() -> StripStyle {
        StripStyle { text_scale: 1.0 }
    }
}

/// Where a strip records the rectangles it drew, for the click that
/// arrives between frames with no geometry of its own.
pub struct StripView<'a> {
    pub hits: &'a mut Hits,
    /// Which view recorded a rectangle: one [`Hits`] may serve every
    /// view in a widget.
    pub id: u32,
}

// ---------------------------------------------------------------------
// Metrics
// ---------------------------------------------------------------------

/// The `[tab]` metrics, read ONCE per draw — the `Look::read` pattern
/// every view in this crate follows.
struct Look {
    h: f32,
    skew: f32,
    corner: f32,
    corner_style: crate::draw::CornerStyle,
    pad: f32,
    gap: f32,
    min_w: f32,
    rule_w: f32,
    rule_gap: f32,
    underline: f32,
    rule_c: Color,
    label: RoleLook,
}

impl Look {
    fn read(sf: &mut impl Surface, text_scale: f32) -> Look {
        Look {
            h: sf.px("tab.h").max(0.0),
            // Deliberately NOT `button.skew`: the master says so where
            // the token stands, and two shapes are allowed to differ.
            skew: sf.px("tab.skew").max(0.0),
            // Read RAW, on purpose. This number is only ever handed to
            // `Surface::ring_fill`/`ring`, which is where §5.0's `pill`
            // becomes half the cell it is about — and it cannot become
            // anything at all if a clamp has already spelled it zero.
            // The one place that knows what a capsule is, is the place
            // that has the box.
            corner: sf.px("tab.corner"),
            // The frames' shape, reached the same way every control
            // reaches it — the master points tab.corner_style at the
            // button's, which points at the panel's — and decoded the
            // same way too: this file used to carry its own `match` over
            // the three words, which is three quarters of a rule.
            corner_style: paint::corner_style(sf, "tab.corner_style"),
            pad: sf.px("tab.pad").max(0.0),
            gap: sf.px("tab.gap").max(0.0),
            min_w: sf.px("tab.min_w").max(0.0),
            // A hairline is a hairline at every scale, so the rule and
            // the underline are not scaled by anything.
            rule_w: sf.px("tab.rule").max(0.0),
            rule_gap: sf.px("tab.rule_gap").max(0.0),
            underline: sf.px("tab.underline_active").max(0.0),
            rule_c: sf.color("tab.rule_color"),
            label: paint::bound_role(sf, "tab.role", text_scale),
        }
    }
}

/// The height a strip occupies: one tab, the gap under it and the rule.
///
/// The box [`strip`] wants. A caller that hands it a shorter one gets a
/// shorter tab and a rule below the box — the strip draws what it was
/// asked for and does not silently move the theme's metrics.
pub fn natural_h(sf: &mut impl Surface) -> f32 {
    let h = sf.px("tab.h").max(0.0);
    let rule = sf.px("tab.rule").max(0.0);
    if rule <= 0.0 {
        h
    } else {
        h + sf.px("tab.rule_gap").max(0.0) + rule
    }
}

/// The parallelogram of a tab rectangle: `skew` shears the top edge
/// right, so the left side leans exactly as the shell's session tabs
/// do.
pub fn quad(r: &Rect, skew: f32) -> [[f32; 2]; 4] {
    [
        [r.x + skew, r.y],
        [r.right(), r.y],
        [r.right() - skew, r.bottom()],
        [r.x, r.bottom()],
    ]
}

/// Fits natural cell widths into `avail`, floored at `min_w`.
///
/// Cells never GROW here — a strip is content-measured and a short
/// label keeps its short tab. When the row is too wide, every cell that
/// still has room above the floor gives back proportionally, so the
/// cells keep their relative widths on the way down; a cell already at
/// its floor gives nothing. When every cell has reached the floor the
/// strip simply overflows its box, which is the honest answer and the
/// point at which the arrows a future step may add would earn their
/// place (F2 §5: strip scrolling is out of scope).
///
/// A cell whose natural width is already under `min_w` floors at that
/// natural width instead: a floor is a trim limit, not a minimum size.
pub fn fit_widths(natural: &[f32], avail: f32, min_w: f32) -> Vec<f32> {
    let mut w = natural.to_vec();
    if w.is_empty() || w.iter().sum::<f32>() <= avail + EPS {
        return w;
    }
    let floors: Vec<f32> = natural.iter().map(|n| n.min(min_w.max(0.0))).collect();
    // A pass that floors nothing lands exactly on `avail`, so every pass
    // but the last floors at least one cell: `n` passes at the worst.
    // The `moved` guard ends it whatever the arithmetic does.
    loop {
        if w.iter().sum::<f32>() <= avail + EPS {
            break;
        }
        let mut fixed = 0.0;
        let mut flex = 0.0;
        for (wi, fi) in w.iter().zip(&floors) {
            if *wi > *fi + EPS {
                flex += *wi;
            } else {
                fixed += *wi;
            }
        }
        if flex <= EPS {
            break;
        }
        let k = ((avail - fixed) / flex).clamp(0.0, 1.0);
        let mut moved = false;
        for (wi, fi) in w.iter_mut().zip(&floors) {
            if *wi > *fi + EPS {
                let next = (*wi * k).max(*fi);
                if next < *wi - EPS {
                    moved = true;
                }
                *wi = next;
            }
        }
        if !moved {
            break;
        }
    }
    w
}

/// Which cell a point lands in — the strip's own hit test, for a caller
/// that keeps the rectangles rather than a [`Hits`].
pub fn hit(cells: &[Rect], x: f32, y: f32) -> Option<usize> {
    cells.iter().position(|c| c.contains(x, y))
}

/// The keys a FOCUSED strip eats: the arrows move the choice along it,
/// Home and End jump to its ends. Answers whether the strip took the
/// key; anything else is the router's, unchanged.
///
/// This is the other half of `Caps::GREEDY_ARROWS`: the chain sends the
/// arrows here instead of moving focus with them, so the caller has one
/// call to make and no arithmetic to repeat. A modified arrow is NOT
/// eaten — those belong to the application's own bindings.
pub fn key(st: &mut StripState, n: usize, ev: &KeyEv) -> bool {
    if n == 0 || ev.mods != Mods::NONE {
        return false;
    }
    match ev.key {
        Key::Left | Key::Up => st.step(n, -1),
        Key::Right | Key::Down => st.step(n, 1),
        Key::Home => st.active = 0,
        Key::End => st.active = n - 1,
        _ => return false,
    }
    true
}

// ---------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------

/// Draws the strip into the TOP of `r` and answers the cell rectangles,
/// in label order.
///
/// Cells are content-measured and laid left to right from `r.x` — they
/// do not stretch to fill the box, because a strip of two pages is a
/// strip of two pages and not a half-window plate each. The rule runs
/// under the cells that were drawn, from the first cell's left edge to
/// the last cell's right, which is where the shell's own strip puts it.
pub fn strip<S: Surface>(
    sf: &mut S,
    r: Rect,
    labels: &[&str],
    st: &StripState,
    style: &StripStyle,
    view: Option<StripView>,
) -> Vec<Rect> {
    let look = Look::read(sf, style.text_scale);
    let n = labels.len();
    let h = look.h.min(r.h);
    if n == 0 || h <= 0.0 || r.w <= 0.0 {
        return Vec::new();
    }
    // Natural width: the label, its padding either side, and the room
    // the shear takes out of the cell's middle.
    let natural: Vec<f32> = labels
        .iter()
        .map(|l| {
            sf.measure(look.label.face, look.label.px, l, look.label.track)
                + 2.0 * look.pad
                + look.skew
        })
        .collect();
    let avail = r.w - look.gap * n.saturating_sub(1) as f32;
    let widths = fit_widths(&natural, avail, look.min_w);
    let mut cells: Vec<Rect> = Vec::with_capacity(n);
    let mut x = r.x;
    for w in &widths {
        cells.push(Rect::new(x, r.y, *w, h));
        x += w + look.gap;
    }

    let (mut hits, view_id) = match view {
        Some(v) => (Some(v.hits), v.id),
        None => (None, 0),
    };
    for (i, cell) in cells.iter().enumerate() {
        // The rung reached over time: a tab does not snap between chosen
        // and resting, it crossfades under `motion.select` (and under
        // `motion.hover` when only the pointer moved).
        let ink = sf.class_ink("tab", st.rung(i), *cell);
        // A sheared tab is a quad; an unsheared one is the family's
        // rounded or chamfered rect, like every other control.
        let q = quad(cell, look.skew);
        if ink.fill.a > 0.0 {
            if look.skew > 0.0 {
                sf.quad(q, ink.fill);
            } else {
                sf.ring_fill(*cell, look.corner_style, look.corner, ink.fill);
            }
        }
        // The rung's ring last, so a theme's `selected.edge` reaches the
        // showing tab through the ladder and not through a special case.
        if ink.edge_width > 0.0 && ink.edge.a > 0.0 {
            if look.skew > 0.0 {
                sf.polyline(&q, ink.edge_width, ink.edge, true);
            } else {
                sf.ring(*cell, look.corner_style, look.corner, ink.edge_width, ink.edge);
            }
        }
        // The showing page's underline, wholly INSIDE the tab: a bold
        // one drawn on the edge would bleed into the rule below.
        if i == st.active && look.underline > 0.0 && ink.edge.a > 0.0 {
            let y = cell.bottom() - look.underline / 2.0;
            sf.line(cell.x, y, cell.right() - look.skew, y, look.underline, ink.edge);
        }
        let inner = (cell.w - 2.0 * look.pad - look.skew).max(0.0);
        let text =
            paint::fit_end(sf, look.label.face, look.label.px, labels[i], inner, look.label.track);
        // A plate too narrow for its page's name gives the name in full
        // when the pointer rests on it (F2 §8.1). A strip trims to fit,
        // so a trimmed tab is precisely the case where the label has
        // stopped doing its one job; an untrimmed one says nothing,
        // because it is already saying everything.
        paint::explain_trim(sf, super::tooltip::key(labels[i]), *cell, &text, labels[i]);
        let ty = paint::center_line_y_in(sf, look.label.face, cell.y, cell.h, look.label.px, look.label.leading);
        paint::cell_text(
            sf,
            cell.x,
            ty,
            cell.w,
            Align::Center,
            look.label.face,
            look.label.px,
            &text,
            ink.text,
            look.label.track,
        );
        if let Some(hs) = hits.as_deref_mut() {
            hs.push(*cell, Hit::Tab { id: view_id, index: i });
        }
    }
    if look.rule_w > 0.0 && look.rule_c.a > 0.0 {
        let y = r.y + h + look.rule_gap;
        let x0 = cells[0].x;
        let x1 = cells[cells.len() - 1].right();
        sf.line(x0, y, x1, y, look.rule_w, look.rule_c);
    }
    cells
}

/// [`strip`] on the host's own surface.
///
/// A caller that wants the rectangles recorded for a later click builds
/// its own [`CtxSurface`] and calls [`strip`] with a [`StripView`]; this
/// is the short form for the common case, where the returned cells are
/// all the caller needs.
pub fn draw(ctx: &mut Ctx, r: Rect, labels: &[&str], st: &StripState) -> Vec<Rect> {
    // `text_scale` is the strip's own shrink, not the user's interface
    // scale: that one rides in `metric.ui_scale` and is already in every
    // baked size. Feeding it here as well drew 125 % at 156 %.
    let style = StripStyle::default();
    strip(&mut CtxSurface::new(ctx), r, labels, st, &style, None)
}

/// [`draw`], joined to the world's focus chain.
///
/// ONE registration for the whole strip: a tab strip is a single stop in
/// the Tab order, and the arrows move the active cell inside it
/// (`Caps::GREEDY_ARROWS`) — which is why the index lives in
/// [`StripState`] rather than in the chain. The ring is drawn around the
/// SHOWING cell, and only when the chain says a ring is due; focus is
/// never a rung of the ladder.
///
/// The [`AccessInfo`] this registers speaks for the ACTIVE cell, not
/// the strip as a whole — a roving-tabindex control has only the one
/// `FocusId` to report through, so it reports the cell a Tab press
/// would land the ring on. `States::SELECTED` says that cell is the
/// showing page, and `with_index` gives its one-based position among
/// `labels.len()` — "2 of 5" is what a screen reader turns that pair
/// into. Both are recomputed every draw from `st.active`, so the
/// announcement follows the arrows without a second registration.
pub fn draw_focusable(
    ctx: &mut Ctx,
    r: Rect,
    labels: &[&str],
    st: &StripState,
    id: FocusId,
) -> Vec<Rect> {
    static SKEW: OnceLock<TokenId> = OnceLock::new();
    let active_label = labels.get(st.active).copied().unwrap_or("");
    let f = ctx.focus.as_deref_mut().map(|fc| {
        fc.register(
            id,
            r,
            Caps::GREEDY_ARROWS,
            AccessInfo::new(Role::Tab, active_label)
                .with_states(States::SELECTED)
                .with_index(st.active as u32 + 1, labels.len() as u32),
        )
    });
    let cells = draw(ctx, r, labels, st);
    if let Some(cell) = cells.get(st.active) {
        let skew = theme::resolved().px(tok(&SKEW, "tab.skew")).max(0.0);
        focus_ring::draw_quad_faded(ctx, quad(cell, skew), f.map_or(false, |f| f.ring));
    }
    cells
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn px(name: &str) -> f32 {
        crate::theme::resolved().px(crate::theme::id(name).unwrap())
    }

    fn word(name: &str) -> String {
        crate::theme::enum_word_of(crate::theme::id(name).unwrap()).unwrap_or_default()
    }

    #[test]
    fn the_master_declares_every_metric_a_strip_draws_from() {
        assert!(px("tab.h") > 0.0);
        // skew is 0 in the master now — a tab wears the frames'
        // corners — so what matters is that both shape tokens exist.
        assert!(crate::theme::id("tab.skew").is_some());
        assert!(px("tab.corner") >= 0.0 && crate::theme::id("tab.corner_style").is_some());
        assert!(px("tab.pad") > 0.0);
        assert!(px("tab.rule") > 0.0 && px("tab.rule_gap") > 0.0);
        assert!(px("tab.underline_active") > 0.0);
        // NEW in F2 §9, and neutral: it only ever trims, and nothing in
        // the master's own render is a strip drawn by this object.
        assert!(px("tab.min_w") > 0.0);
        assert_eq!(word("tab.role"), "button");
        // The master's strip has no gap between tabs, which is why the
        // shell's tabs meet along their sheared edges.
        assert_eq!(px("tab.gap"), 0.0);
    }

    #[test]
    fn the_rung_is_the_buttons_rung_one_strip_wide() {
        let mut st = StripState::new(1);
        assert_eq!(st.rung(1), State::Selected);
        assert_eq!(st.rung(0), State::Idle);
        st.hover = Some(0);
        assert_eq!(st.rung(0), State::Hover);
        assert_eq!(st.rung(1), State::Selected);
        st.hover = Some(1);
        assert_eq!(st.rung(1), State::SelectedHover);
        // A flash outranks everything: it IS the press.
        st.flash = Some(1);
        assert_eq!(st.rung(1), State::Press);
        st.flash = Some(0);
        assert_eq!(st.rung(0), State::Press);
        assert_eq!(st.rung(1), State::SelectedHover);
    }

    #[test]
    fn the_arrows_wrap_around_the_strip() {
        let mut st = StripState::new(0);
        st.step(3, -1);
        assert_eq!(st.active, 2);
        st.step(3, 1);
        assert_eq!(st.active, 0);
        st.step(3, 2);
        assert_eq!(st.active, 2);
        // An empty strip has nowhere to step to, and a stale index out
        // of range lands back inside.
        st.step(0, 1);
        assert_eq!(st.active, 2);
        st.active = 9;
        st.step(3, 1);
        assert_eq!(st.active, 0);
    }

    #[test]
    fn a_focused_strip_eats_the_arrows_and_nothing_else() {
        fn ev(key: Key, mods: Mods) -> KeyEv {
            KeyEv { key, mods, repeat: false, text: None }
        }
        let mut st = StripState::new(0);
        assert!(key(&mut st, 3, &ev(Key::Right, Mods::NONE)));
        assert_eq!(st.active, 1);
        assert!(key(&mut st, 3, &ev(Key::End, Mods::NONE)));
        assert_eq!(st.active, 2);
        assert!(key(&mut st, 3, &ev(Key::Home, Mods::NONE)));
        assert_eq!(st.active, 0);
        assert!(key(&mut st, 3, &ev(Key::Left, Mods::NONE)));
        assert_eq!(st.active, 2, "the arrows wrap here as they do in step");
        // Enter is the router's — activation is not a strip's business.
        assert!(!key(&mut st, 3, &ev(Key::Enter, Mods::NONE)));
        // A modified arrow belongs to the application's bindings.
        assert!(!key(&mut st, 3, &ev(Key::Right, Mods::CTRL)));
        assert_eq!(st.active, 2);
        // A strip with no cells has no key to eat.
        assert!(!key(&mut st, 0, &ev(Key::Right, Mods::NONE)));
    }

    /// `draw_focusable`'s ONE registration has to speak for whichever
    /// cell is active, since a roving-tabindex strip has no second
    /// `FocusId` to hang a per-cell report from. Drives the real
    /// function end to end — through a live `FocusCtl` — rather than
    /// re-deriving the `AccessInfo` by hand, so a future edit that
    /// changes what gets built there also breaks this.
    #[test]
    fn draw_focusable_reports_the_active_cell_selected_and_indexed() {
        use crate::draw::DrawList;
        use crate::focus::FocusCtl;
        use crate::font::FontSystem;
        use crate::pointer::Pointer;

        let mut dl = DrawList::new();
        let mut fonts = FontSystem::new();
        let mut fc = FocusCtl::new();
        let labels = ["General", "Network", "About"];
        // The middle page is showing: a one-based "2 of 3" is what a
        // screen reader should turn `st.active == 1` into.
        let st = StripState::new(1);
        let id = FocusId::of("test.tabs");
        let r = Rect::new(0.0, 0.0, 300.0, 32.0);
        let mut ctx = Ctx {
            access: None,
            dl: &mut dl,
            fonts: &mut fonts,
            w: 1920.0,
            h: 1080.0,
            t: 0.0,
            mouse: Pointer::new(0.0, 0.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: Some(&mut fc),
            tips: None,
        };
        draw_focusable(&mut ctx, r, &labels, &st, id);

        // The registration lands in `cur`; `begin_frame` is the frame
        // boundary that promotes it to what `entries()` answers, same
        // as every other FocusCtl/AccessCtl consumer in this crate.
        fc.begin_frame();
        let got: Vec<_> = fc.entries().collect();
        assert_eq!(got.len(), 1);
        let info = got[0].2;
        assert_eq!(info.role, Role::Tab);
        assert_eq!(info.name, "Network");
        assert!(info.states.contains(States::SELECTED));
        assert_eq!(info.index, Some((2, 3)));
    }

    #[test]
    fn cells_that_fit_are_left_exactly_as_they_were_measured() {
        let nat = [40.0, 60.0, 30.0];
        assert_eq!(fit_widths(&nat, 130.0, 20.0), vec![40.0, 60.0, 30.0]);
        // Room to spare does not stretch them either: a strip of three
        // pages is three pages wide, not the box's width.
        assert_eq!(fit_widths(&nat, 400.0, 20.0), vec![40.0, 60.0, 30.0]);
    }

    #[test]
    fn a_strip_too_wide_gives_back_proportionally() {
        let nat = [40.0, 60.0, 100.0];
        let w = fit_widths(&nat, 100.0, 10.0);
        assert!((w.iter().sum::<f32>() - 100.0).abs() < 0.05, "fills the box: {w:?}");
        // Halved, each of them, because none is anywhere near the floor.
        for (got, want) in w.iter().zip([20.0, 30.0, 50.0]) {
            assert!((got - want).abs() < 0.05, "{w:?}");
        }
    }

    #[test]
    fn the_floor_stops_the_trim_and_the_strip_overflows_honestly() {
        // 30 available for three cells that may not go under 20: the
        // two wide ones stop at the floor and the short one keeps its
        // own width, so the strip is wider than its box on purpose.
        let nat = [100.0, 100.0, 8.0];
        let w = fit_widths(&nat, 30.0, 20.0);
        assert!((w[0] - 20.0).abs() < 0.05, "{w:?}");
        assert!((w[1] - 20.0).abs() < 0.05, "{w:?}");
        // A cell narrower than the floor is left alone: a floor is a
        // trim limit, never a minimum size.
        assert!((w[2] - 8.0).abs() < 0.05, "{w:?}");
    }

    #[test]
    fn one_cell_reaching_the_floor_hands_its_share_to_the_others() {
        // 60 wide, floor 10: the short cell reaches the floor on the
        // first pass, and the rest of the deficit comes out of the two
        // that still have room — 25 each, not 20 and 30.
        let nat = [12.0, 40.0, 40.0];
        let w = fit_widths(&nat, 60.0, 10.0);
        assert!((w[0] - 10.0).abs() < 0.05, "{w:?}");
        assert!((w.iter().sum::<f32>() - 60.0).abs() < 0.05, "{w:?}");
        assert!((w[1] - w[2]).abs() < 0.05, "equal naturals stay equal: {w:?}");
        assert!((w[1] - 25.0).abs() < 0.05, "{w:?}");
    }
}
