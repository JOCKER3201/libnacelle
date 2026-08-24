//! Segmented control: a row of mutually exclusive cells, of which
//! exactly one is chosen — radio semantics wearing a button's clothes.
//!
//! The `[segmented]` section has been in the master since the theme
//! engine landed and, until now, nothing read it: `h`, `gap`, `corner`,
//! `border`, `border_active`, `pad_x`, `min_cell_w`, and the `role` this
//! step adds. The class ladder is `button`'s, which is what the 5.27
//! matrix already says this control borrows — there is no `segmented`
//! class to write, and inventing one would be a second place to state
//! the same look.
//!
//! Everything else — the state, the geometry solver, the hit test — is
//! [`super::tabs`]'s: a segmented control IS a strip with square cells,
//! and two copies of that arithmetic would drift.

use super::focus_ring;
use super::tabs;
use crate::access::{AccessInfo, Role};
use crate::draw::CornerStyle;
use crate::focus::{Caps, FocusId};
use crate::ui::Align;
use crate::view::paint::{self, RoleLook};
use crate::view::surface::{CtxSurface, Surface};
use crate::view::Hit;
use crate::{Ctx, Rect};

pub use super::tabs::{hit, key, StripState, StripStyle, StripView};

/// The `[segmented]` metrics, read ONCE per draw.
///
/// The corner RADIUS is not among them, and cannot be: `@corner.pill` is
/// a word about a box (§5.0), so it is not a length until there is a cell
/// to close over — [`paint::corner_radius`] resolves it against each cell
/// in [`control`]. The CUT is a metric like any other; it is the same for
/// every cell.
struct Look {
    h: f32,
    gap: f32,
    corner_style: CornerStyle,
    border: f32,
    border_active: f32,
    pad_x: f32,
    min_cell_w: f32,
    label: RoleLook,
}

impl Look {
    fn read(sf: &mut impl Surface, text_scale: f32) -> Look {
        Look {
            h: sf.px("segmented.h").max(0.0),
            gap: sf.px("segmented.gap").max(0.0),
            // The radius was already the theme's; the CUT was not, and a
            // cell frozen on a chamfer stopped looking like the button
            // whose class ladder it borrows. Read through the shared
            // reader rather than spelled out again here: the corner
            // vocabulary is already written in more places than it should
            // be, and one word table is what stops a cell and a badge
            // disagreeing about what `chamfer` means.
            corner_style: paint::corner_style(sf, "segmented.corner_style"),
            // A ring's weight is a stroke, and a stroke does not scale.
            border: sf.px("segmented.border").max(0.0),
            border_active: sf.px("segmented.border_active").max(0.0),
            pad_x: sf.px("segmented.pad_x").max(0.0),
            min_cell_w: sf.px("segmented.min_cell_w").max(0.0),
            label: paint::bound_role(sf, "segmented.role", text_scale),
        }
    }
}

/// The height a control occupies — `segmented.h`, and nothing under it:
/// unlike a tab strip, a segmented control carries no rule.
pub fn natural_h(sf: &mut impl Surface) -> f32 {
    sf.px("segmented.h").max(0.0)
}

/// The width `labels` want: every cell measured from its own content,
/// floored at `segmented.min_cell_w`, plus the gaps between them.
///
/// For the caller laying a control out beside other things — a control
/// that is content-sized has to be measurable before it is drawn.
pub fn natural_w(sf: &mut impl Surface, labels: &[&str], style: &StripStyle) -> f32 {
    if labels.is_empty() {
        return 0.0;
    }
    let look = Look::read(sf, style.text_scale);
    let cells: f32 = natural_widths(sf, labels, &look).iter().sum();
    cells + look.gap * (labels.len() - 1) as f32
}

fn natural_widths(sf: &mut impl Surface, labels: &[&str], look: &Look) -> Vec<f32> {
    labels
        .iter()
        .map(|l| {
            (sf.measure(look.label.face, look.label.px, l, look.label.track) + 2.0 * look.pad_x)
                .max(look.min_cell_w)
        })
        .collect()
}

/// Draws the control into `r`, vertically centred, and answers the cell
/// rectangles in label order.
///
/// Cells are measured from their content and laid out from `r.x` —
/// left-aligned, never stretched to fill the box, because a control of
/// three choices is as wide as its three choices. The chosen cell wears
/// `segmented.border_active` and the ladder's Selected rung; every other
/// wears `segmented.border` and whatever rung the pointer puts it on.
pub fn control<S: Surface>(
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
    let natural = natural_widths(sf, labels, &look);
    let avail = r.w - look.gap * n.saturating_sub(1) as f32;
    let widths = tabs::fit_widths(&natural, avail, look.min_cell_w);
    let y = crate::ui::block_top(&r, h);
    let mut cells: Vec<Rect> = Vec::with_capacity(n);
    let mut x = r.x;
    for w in &widths {
        cells.push(Rect::new(x, y, *w, h));
        x += w + look.gap;
    }

    let (mut hits, view_id) = match view {
        Some(v) => (Some(v.hits), v.id),
        None => (None, 0),
    };
    for (i, cell) in cells.iter().enumerate() {
        // `button` — the class the matrix lends this control. The whole
        // ladder, including the Selected rung the chosen cell always
        // stands on.
        let ink = sf.class_ink("button", st.rung(i), *cell);
        // Per cell, and through the one translator: `segmented.corner`
        // may hold `@corner.pill`, which bakes to a NEGATIVE number and
        // means "as round as THIS cell can be". A `.max(0.0)` here would
        // have answered a theme writing `pill` with the square it wrote
        // `pill` to avoid, and said nothing about it; the ceiling of half
        // the short side is geometry, and lives with the translation.
        let cut = paint::corner_radius(sf, "segmented.corner", *cell, 1.0);
        if ink.fill.a > 0.0 {
            sf.ring_fill(*cell, look.corner_style, cut, ink.fill);
        }
        let bw = if i == st.active { look.border_active } else { look.border };
        if bw > 0.0 && ink.edge.a > 0.0 {
            sf.ring(*cell, look.corner_style, cut, bw, ink.edge);
        }
        let inner = (cell.w - 2.0 * look.pad_x).max(0.0);
        let text =
            paint::fit_end(sf, look.label.face, look.label.px, labels[i], inner, look.label.track);
        // The tab strip's rule, for the same reason (F2 §8.1): a choice
        // whose word did not fit is a choice the user cannot read.
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
            hs.push(*cell, Hit::Segment { id: view_id, index: i });
        }
    }
    cells
}

/// [`control`] on the host's own surface.
pub fn draw(ctx: &mut Ctx, r: Rect, labels: &[&str], st: &StripState) -> Vec<Rect> {
    // `text_scale` is the strip's own shrink, not the user's interface
    // scale: that one rides in `metric.ui_scale` and is already in every
    // baked size. Feeding it here as well drew 125 % at 156 %.
    let style = StripStyle::default();
    control(&mut CtxSurface::new(ctx), r, labels, st, &style, None)
}

/// [`draw`], joined to the world's focus chain.
///
/// One registration for the whole control, exactly as
/// [`super::tabs::draw_focusable`] does it: the arrows move the choice
/// INSIDE the control (`Caps::GREEDY_ARROWS`) and the ring is drawn
/// around the chosen cell only when the chain says a ring is due.
pub fn draw_focusable(
    ctx: &mut Ctx,
    r: Rect,
    labels: &[&str],
    st: &StripState,
    id: FocusId,
) -> Vec<Rect> {
    let active_label = labels.get(st.active).copied().unwrap_or("");
    let f = ctx.focus.as_deref_mut().map(|fc| {
        fc.register(
            id,
            r,
            Caps::GREEDY_ARROWS,
            AccessInfo::new(Role::RadioButton, active_label),
        )
    });
    let cells = draw(ctx, r, labels, st);
    if let Some(cell) = cells.get(st.active) {
        focus_ring::draw_faded(ctx, *cell, f.map_or(false, |f| f.ring));
    }
    cells
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    fn px(name: &str) -> f32 {
        crate::theme::resolved().px(crate::theme::id(name).unwrap())
    }

    fn word(name: &str) -> String {
        crate::theme::enum_word_of(crate::theme::id(name).unwrap()).unwrap_or_default()
    }

    #[test]
    fn the_master_declares_every_metric_a_control_draws_from() {
        assert!(px("segmented.h") > 0.0);
        assert!(px("segmented.gap") > 0.0);
        assert!(px("segmented.pad_x") > 0.0);
        assert!(px("segmented.min_cell_w") > 0.0);
        assert!(px("segmented.corner") > 0.0);
        // The cut beside the length. `tests/corner_language.rs` proves it
        // reaches the cell; here it only has to exist to be reachable.
        assert_eq!(word("segmented.corner_style"), "round");
        // The chosen cell's ring is the heavier of the two.
        assert!(px("segmented.border_active") > px("segmented.border"));
        // NEW in F2 §9: the role the 5.27 matrix already lends this
        // control, said out loud so a theme can move it.
        assert_eq!(word("segmented.role"), "button");
    }
}
