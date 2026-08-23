//! Window-driven responsive layout — "like a website".
//!
//! Every frame the layout is computed from the ACTUAL window size, so
//! resizing or moving the window reflows the interface live. Layouts
//! are flexbox column descriptions (FlexLayaut) solved by the `taffy`
//! crate — the same layout algorithm web pages use: columns have real
//! min/max pixel widths (the side columns shrink before the work
//! surface does) and collapse priorities (when a column can no longer
//! fit its minimum width it disappears — collapse=1 first, then 2,
//! ...). An instance anchored as the BAR comes back full width at the
//! bottom if it loses its column. On portrait windows the visible
//! instances restack vertically (the body columns merge from the right
//! until they fit the width; nothing is ever hidden for being short).
//! The generated default arrangement and custom flexbox .layaut files
//! share this engine; rectangle boards (fixed x/y/w/h at the 16:9
//! reference) are re-adapted with an edge-anchored transform on
//! landscape and the flex restack on portrait.
//!
//! What the engine places are INSTANCES, not widgets: a board may hold
//! two terminals, and it is their identities that keep the solver's two
//! rectangles apart. What each of them ASKS for — its column, its place
//! in it, its share of it, the edge it is pinned to — is a property of
//! the widget KIND, declared by the addon and read off the registry
//! (see `widget::registry`). No widget is named anywhere in here.

use super::instance::{Instance, InstanceList};
use crate::base::{
    ColumnItem, FlexColumn, FlexLayaut, Layout, LayoutMode, Panel, PanelAnchor, PanelSlot,
    PanelSpec, Placed, Rect, SizeTable, WidgetCategory,
};
use taffy::prelude::{auto, length, percent};
use taffy::style::{AvailableSpace, FlexDirection};
use taffy::{Size, Style, TaffyTree};

/// CSS-like pixel constraints of the generated default columns.
const SIDE_MIN: f32 = 168.0;
const SIDE_MAX: f32 = 340.0;
const CENTER_MIN: f32 = 430.0;

/// The instances the generated default arrangement places: ONE per
/// installed board widget, all of them on home.
///
/// A machine's board is whatever it has installed, so the arrangement
/// the program falls back to has to be composed rather than written
/// down — and composing it means minting the instances too. One per
/// addon, in registry order, exactly as many as before this list could
/// hold two of anything; the moment the user drags a second terminal
/// out, that is an instance the editor adds and the file records.
///
/// A widget of another category is not placed here: its home is the
/// fixture its category names, and the fixtures are composed elsewhere.
pub fn default_instances() -> InstanceList {
    let mut out = InstanceList::new();
    for p in Panel::all() {
        if p.category() == WidgetCategory::Board {
            // Composed, not saved: the identity is the widget's own
            // registry position in the generated range, so it is the
            // same on every start and belongs to no file.
            out.add_generated(p, (0, 0), p.idx() as u32);
        }
    }
    out
}

/// The generated default arrangement over a given set of instances, as
/// a flexbox description — the same structure a theme author writes in
/// a flexbox .layaut file.
///
/// There is no table of widgets here and there is none anywhere else in
/// the toolkit: the shape below is three columns — two instrument sides
/// and a wide work surface — and WHICH instance stands where follows
/// from its widget's own declaration (`slot`, `order`, `weight`,
/// `anchor`; see `widget::registry`). An instance whose widget names no
/// column joins the emptier side, so whatever a board holds is laid out
/// and a board that holds nothing gets an empty arrangement rather than
/// an invented one.
///
/// Nothing is filtered out: the caller's list IS the board's content.
/// Two instances of one widget are two entries here, and they keep the
/// order they were placed in.
pub fn compose(insts: &[Instance]) -> FlexLayaut {
    let mut stacks: [Vec<Instance>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut adrift: Vec<Instance> = Vec::new();
    for i in insts {
        match i.widget.slot() {
            PanelSlot::Left => stacks[0].push(*i),
            PanelSlot::Center => stacks[1].push(*i),
            PanelSlot::Right => stacks[2].push(*i),
            PanelSlot::Auto => adrift.push(*i),
        }
    }
    // An instance whose widget asked for no column goes to the emptier
    // side, in placement order — the arrangement holds everything the
    // board carries without the engine having an opinion about any of
    // it.
    for i in adrift {
        let side = if stacks[2].len() < stacks[0].len() { 2 } else { 0 };
        stacks[side].push(i);
    }
    // Stable, so instances that asked for the same place keep the order
    // the layout placed them in.
    for stack in stacks.iter_mut() {
        stack.sort_by(|a, b| {
            a.widget
                .order()
                .partial_cmp(&b.widget.order())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }
    let col = |basis, min, max, grow, collapse, gap, stack: &[Instance]| FlexColumn {
        basis,
        min,
        max,
        grow,
        collapse,
        gap,
        panels: stack.iter().map(|i| item(*i)).collect(),
    };
    FlexLayaut {
        columns: vec![
            // Left: an instrument stack. Weights only matter between
            // the growers; a Content-sized panel takes exactly what it
            // measures whatever its number says.
            col(16.4, SIDE_MIN, SIDE_MAX, 0.0, 2, 1.0, &stacks[0]),
            // Centre: the work surface, the column that grows. Whatever
            // asked to be pinned is re-anchored here by normalize().
            col(65.0, CENTER_MIN, f32::INFINITY, 1.0, 0, 1.7, &stacks[1]),
            // Right: the second instrument stack.
            col(16.4, SIDE_MIN, SIDE_MAX, 0.0, 1, 2.5, &stacks[2]),
        ],
        units_px: false,
        pad_x: None,
    }
}

/// One instance as a column entry, asking for the share its widget
/// declared.
fn item(i: Instance) -> ColumnItem {
    ColumnItem { id: i.id, widget: i.widget, weight: i.widget.weight() }
}

/// The generated default arrangement of THIS installation, written out
/// — the shape `--print-layaut` shows and the shipped console.layaut
/// mirrors.
pub fn default_flex() -> FlexLayaut {
    compose(default_instances().all())
}

/// The reference and minimum heights (vh) of the generated default —
/// what every widget it places declared for itself.
///
/// Sizes belong to the LAYOUT, not to the widget (base.rs keeps them
/// apart for exactly this reason), and a .layaut names its own in its
/// ref/min column. The generated arrangement has no numbers of its own
/// to name: it is composed out of what is installed, so its table is
/// the installation's.
pub fn builtin_sizes() -> Vec<(Panel, f32, f32)> {
    // The REGISTRY's numbers, not the table a layout happens to have
    // installed: this is what the generated arrangement asks for, and
    // asking the live table would make it echo whatever came last.
    let declared = crate::base::default_sizes();
    let mut out: Vec<(Panel, f32, f32)> = Vec::new();
    for i in default_instances().iter() {
        // One row per WIDGET: the table is per kind, so a board holding
        // two terminals still names the terminal's heights once.
        if out.iter().any(|(p, _, _)| *p == i.widget) {
            continue;
        }
        if let Some((r, m)) = declared.get(i.widget.idx()) {
            out.push((i.widget, *r, *m));
        }
    }
    out
}

/// A registry for the toolkit's own tests: twelve board widgets that
/// declare a composition of the shape the engine is written for.
///
/// The names are this test's own — the engine knows none, and a test
/// that borrowed the shipped addons' names would be proving the engine
/// knows them. EVERY test in this binary that touches a panel must call
/// this: the process-wide registry is fixed by the first call *or the
/// first read*, and a test reading it first would freeze it empty for
/// all the others.
#[cfg(test)]
pub(crate) fn install_test_registry() {
    use crate::base::{PanelAnchor as A, PanelSlot as S, WidgetDef};
    // A composition of the shape the engine is written for: two
    // instrument sides, a pinned work surface, a bar.
    let spec: [(&str, S, A, f32, f32); 12] = [
        ("w01", S::Left, A::Flow, 4.5, 4.5),
        ("w02", S::Left, A::Flow, 6.5, 6.5),
        ("w03", S::Left, A::Flow, 15.5, 9.0),
        ("w04", S::Left, A::Flow, 12.0, 9.0),
        ("w05", S::Left, A::Flow, 8.0, 7.0),
        ("w06", S::Left, A::Bar, 13.0, 13.0),
        ("w07", S::Center, A::Top, 60.0, 12.0),
        ("w08", S::Center, A::Bottom, 28.0, 12.0),
        ("w09", S::Right, A::Flow, 40.0, 10.0),
        ("w10", S::Right, A::Flow, 22.0, 8.0),
        ("w11", S::Right, A::Flow, 8.0, 8.0),
        ("w12", S::Right, A::Flow, 7.0, 5.5),
    ];
    crate::base::set_registry(
        spec.iter()
            .enumerate()
            .map(|(i, (name, slot, anchor, r, m))| WidgetDef {
                name: (*name).to_string(),
                label: name.to_uppercase(),
                ref_h_vh: *r,
                min_h_vh: *m,
                category: WidgetCategory::Board,
                slot: *slot,
                order: i as f32,
                weight: None,
                anchor: *anchor,
                essential: false,
            })
            .collect(),
    );
    crate::base::set_panel_sizes(&builtin_sizes());
}

/// Device-independent layout unit. `min` / `max` in a .layaut and the
/// built-in column constants are written at a 1080-line reference and
/// scale with the window height, so one composition comes out at 720p
/// and at 4K instead of two thin ribbons around a giant terminal.
/// Clamped, so a 300-line window still gets usable columns and an 8K
/// wall does not get 1200px side columns.
fn lu(h: f32) -> f32 {
    (h / 1080.0).clamp(0.75, 2.5)
}

/// Layout for the current window size, recomputed every frame. `pad`
/// is the widget padding: every panel is kept tall enough for the
/// padding on both sides plus a minimum of content. `insts` is what the
/// board holds.
pub fn compute(w: f32, h: f32, mode: &LayoutMode, pad: f32, insts: &[Instance]) -> Layout {
    compute_in(w, h, mode, pad, &crate::base::size_table(), insts)
}

/// The same solve against a CALLER's size table — the per-world form
/// (u3 L2); `compute` above is its process-wide shorthand.
pub fn compute_in(
    w: f32,
    h: f32,
    mode: &LayoutMode,
    pad: f32,
    t: &SizeTable,
    insts: &[Instance],
) -> Layout {
    // `draw_screen` probes a board's content size and then lays it out
    // for real in the SAME frame, with nothing about the window or the
    // instances changed between the two calls — so the second call
    // would re-clone the layaut and re-run the taffy solver over the
    // exact result the first call just produced. One remembered solve,
    // keyed on everything it actually read; a real change to any of
    // those still falls straight through and resolves fresh.
    let kind = match mode {
        LayoutMode::Flex => Some(ModeKind::Flex),
        LayoutMode::Rects => Some(ModeKind::Rects),
        LayoutMode::Custom(_) => None,
    };
    let key = kind.map(|k| (k, format!("{insts:?}"), format!("{t:?}")));
    if let Some((k, id, tb)) = &key {
        let hit = SOLVE_CACHE.with(|c| {
            c.borrow().as_ref().and_then(|e| {
                (e.w == w && e.h == h && e.pad == pad && e.kind == *k
                    && &e.insts_dbg == id
                    && &e.table_dbg == tb)
                    .then(|| e.rebuild(w, h))
            })
        });
        if let Some(out) = hit {
            return out;
        }
    }
    let out = match mode {
        LayoutMode::Flex => engine(&compose(insts), w, h, pad, t),
        LayoutMode::Custom(fl) => engine(fl, w, h, pad, t),
        LayoutMode::Rects => {
            if h > w {
                // Portrait: restack the instances VISIBLE on the board
                // using the flex engine.
                let vis: Vec<Instance> =
                    insts.iter().filter(|i| !i.hidden()).copied().collect();
                portrait_flex(&compose(&vis), w, h, pad, t)
            } else {
                rect_layout(w, h, &edge_adapt(insts, w / h))
            }
        }
    };
    if let Some((kind, insts_dbg, table_dbg)) = key {
        SOLVE_CACHE.with(|c| {
            *c.borrow_mut() = Some(SolveCacheEntry {
                w,
                h,
                pad,
                kind,
                insts_dbg,
                table_dbg,
                placed: out.all().to_vec(),
            });
        });
    }
    out
}

/// `Flex` and `Rects` are trivially comparable; `Custom` carries a whole
/// [`FlexLayaut`] with no cheap equality of its own, so it always misses
/// [`SOLVE_CACHE`] rather than growing one here.
#[derive(Clone, Copy, PartialEq)]
enum ModeKind {
    Flex,
    Rects,
}

/// One remembered solve of [`compute_in`]. `Layout` is not `Clone`, so
/// what is kept is what rebuilds one: the placements themselves.
struct SolveCacheEntry {
    w: f32,
    h: f32,
    pad: f32,
    kind: ModeKind,
    insts_dbg: String,
    table_dbg: String,
    placed: Vec<Placed>,
}

impl SolveCacheEntry {
    fn rebuild(&self, w: f32, h: f32) -> Layout {
        let mut out = Layout::empty(w, h);
        for p in &self.placed {
            out.place(p.id, p.widget, p.rect);
        }
        out
    }
}

thread_local! {
    static SOLVE_CACHE: std::cell::RefCell<Option<SolveCacheEntry>> = std::cell::RefCell::new(None);
}

fn engine(fl: &FlexLayaut, w: f32, h: f32, pad: f32, t: &SizeTable) -> Layout {
    let fl = normalize(fl);
    if h > w {
        portrait_flex(&fl, w, h, pad, t)
    } else {
        landscape(&fl, w, h, pad, t)
    }
}

/// Enforces the anchor rules of the flex layout, for the instances of
/// the layaut whose widget asked for one: a `Top` widget goes to the
/// very TOP of the CENTER column, a `Bottom` one to its very BOTTOM,
/// and a `Bar` one to the bottom of the FIRST column — from where a
/// lost column brings it back as a full-width bar. Everything that
/// asked for nothing flows wherever the algorithm puts it, and a layaut
/// whose panels all flow comes out of here unchanged.
///
/// Which widget that is, is never asked here: the anchor is the addon's
/// own declaration, so an installation with no terminal simply pins
/// nothing to the top.
fn normalize(fl: &FlexLayaut) -> FlexLayaut {
    let mut fl = fl.clone();
    let mut pinned: Vec<ColumnItem> = Vec::new();
    for c in fl.columns.iter_mut() {
        c.panels.retain(|it| {
            if it.widget.anchor() == PanelAnchor::Flow {
                return true;
            }
            pinned.push(*it);
            false
        });
    }
    if fl.columns.is_empty() {
        return fl;
    }
    // The CENTER column: the growing one, else the widest basis.
    let center = fl
        .columns
        .iter()
        .enumerate()
        .max_by(|(_, a), (_, b)| {
            (a.grow, a.basis)
                .partial_cmp(&(b.grow, b.basis))
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .map(|(i, _)| i)
        .unwrap_or(0);
    // Tops in the order they were declared in, so several of them stack
    // in that order rather than in reverse.
    for (i, it) in anchored(&pinned, PanelAnchor::Top).into_iter().enumerate() {
        fl.columns[center].panels.insert(i, it);
    }
    for it in anchored(&pinned, PanelAnchor::Bar) {
        fl.columns[0].panels.push(it);
    }
    for it in anchored(&pinned, PanelAnchor::Bottom) {
        fl.columns[center].panels.push(it);
    }
    fl.columns.retain(|c| !c.panels.is_empty());
    fl
}

/// The pinned entries asking for one edge, in declaration order.
fn anchored(pinned: &[ColumnItem], a: PanelAnchor) -> Vec<ColumnItem> {
    pinned.iter().filter(|it| it.widget.anchor() == a).copied().collect()
}

/// The entries of a layaut whose widget asked for the given edge.
fn anchored_in(fl: &FlexLayaut, a: PanelAnchor) -> Vec<ColumnItem> {
    fl.columns
        .iter()
        .flat_map(|c| c.panels.iter())
        .filter(|it| it.widget.anchor() == a)
        .copied()
        .collect()
}

/// Splits `span` between stacked panels by their weights, enforcing the
/// per-panel minimum heights (content + padding). Space for the minimums
/// is taken from panels above their minimum; when even the minimums do
/// not fit, everything scales down proportionally to them.
fn stack_heights(
    weights: &[f32],
    mins: &[f32],
    wants: &[Option<f32>],
    gap_units: f32,
    span: f32,
) -> (Vec<f32>, f32) {
    let n = weights.len() as f32;
    let total: f32 = weights.iter().sum::<f32>() + gap_units * (n - 1.0).max(0.0);
    let gap_px = gap_units / total.max(0.001) * span;
    let content_span = (span - gap_px * (n - 1.0).max(0.0)).max(1.0);
    let min_sum: f32 = mins.iter().sum();
    if min_sum >= content_span {
        let k = content_span / min_sum.max(0.001);
        return (mins.iter().map(|m| m * k).collect(), gap_px);
    }
    let wsum: f32 = weights.iter().sum();
    // A widget that measured itself takes exactly what its content
    // needs; the ones that grow into whatever they get share the rest,
    // by weight. That is what keeps the box around a clock the size of
    // a clock and gives the height it did not need to the process list.
    let asked: f32 = wants.iter().flatten().sum();
    let grow_sum: f32 = (0..weights.len())
        .filter(|i| wants.get(*i).copied().flatten().is_none())
        .map(|i| weights[i])
        .sum();
    let mut hs: Vec<f32> = if grow_sum > 0.0 && asked < content_span {
        let left = content_span - asked;
        (0..weights.len())
            .map(|i| match wants.get(i).copied().flatten() {
                Some(h) => h,
                None => weights[i] / grow_sum * left,
            })
            .collect()
    } else if asked > 0.0 && grow_sum <= 0.0 {
        // Every panel measured itself and NONE of them grows — so they
        // share the WHOLE column in the proportions they asked for: shrunk
        // together when they do not fit, and STRETCHED together when they
        // fit with room to spare. The stretch is the fix for a column the
        // responsive reflow left with no grower to absorb the slack (a
        // second monitor that pushed the growing panel to another column):
        // capping `k` at 1 kept the panels at their content size and left
        // the rest of the column empty — the gaps the owner saw moving the
        // desktop to a wider screen. `k = content_span / asked` fills the
        // column exactly, up or down, and the min pass below still holds
        // every panel above its floor.
        let k = content_span / asked;
        wants
            .iter()
            .enumerate()
            .map(|(i, w)| w.unwrap_or(weights[i]) * k)
            .collect()
    } else {
        weights
            .iter()
            .map(|wt| wt / wsum.max(0.001) * content_span)
            .collect()
    };
    for _ in 0..4 {
        let mut deficit = 0.0;
        for i in 0..hs.len() {
            if hs[i] < mins[i] {
                deficit += mins[i] - hs[i];
                hs[i] = mins[i];
            }
        }
        if deficit <= 0.5 {
            break;
        }
        let surplus: f32 = (0..hs.len()).map(|i| (hs[i] - mins[i]).max(0.0)).sum();
        if surplus <= 0.0 {
            break;
        }
        let k = (deficit / surplus).min(1.0);
        for i in 0..hs.len() {
            let s = (hs[i] - mins[i]).max(0.0);
            hs[i] -= s * k;
        }
    }
    (hs, gap_px)
}

/// Stacks a run of column entries into a box, and places them.
fn stack_into(out: &mut Layout, run: &[ColumnItem], r: Rect, h: f32, pad: f32, gap_units: f32, t: &SizeTable) {
    let weights: Vec<f32> = run.iter().map(|it| it.weight).collect();
    let mins: Vec<f32> = run.iter().map(|it| min_outer(*it, h, pad, t)).collect();
    let wants: Vec<Option<f32>> = run
        .iter()
        .map(|it| t.intrinsic_of(it.id, it.widget).map(|ih| ih + 2.0 * pad))
        .collect();
    let (hs, gap_px) = stack_heights(&weights, &mins, &wants, gap_units, r.h);
    let mut y = r.y;
    for (it, ph) in run.iter().zip(&hs) {
        out.place(it.id, it.widget, Rect::new(r.x, y, r.w, *ph));
        y += ph + gap_px;
    }
}

/// Fills one of portrait's pinned bands. A single instance takes the
/// band whole — the band was sized for it — and several share it by the
/// weights they asked for, never squeezed under their minimums.
fn stack_band(out: &mut Layout, band: &[ColumnItem], r: Rect, h: f32, pad: f32, t: &SizeTable) {
    match band {
        [] => {}
        [it] => out.place(it.id, it.widget, r),
        _ => stack_into(out, band, r, h, pad, 1.0, t),
    }
}

/// The outer height an instance must never be squeezed under: its
/// widget's minimum CONTENT (min_h_vh names the last content row), plus
/// the container's chrome around that content — border, padding, the
/// title band — plus the widget padding on both sides. The published
/// wants carry the chrome already (the sizing pass adds `chrome_extra`);
/// before the chrome term the minimums did not, so a stacked column
/// under pressure was solved as if every band were free, and each
/// titled panel came out exactly one band short of its own content.
fn min_outer(it: ColumnItem, h: f32, pad: f32, t: &SizeTable) -> f32 {
    t.min_h_vh(it.widget) / 100.0 * h + t.chrome_of(it.id, it.widget) + 2.0 * pad
}

/// A stacked column needs room for its own instances' minimums; below
/// that it is better dropped than crushed. Asked per column, not once
/// for the window — the old flat `h >= 520` test dropped every
/// collapsible column of a 3840×500 strip at once, nine widgets gone
/// with 3793 px of width available.
fn column_fits(c: &FlexColumn, h: f32, pad: f32, span: f32, t: &SizeTable) -> bool {
    let need: f32 = c.panels.iter().map(|it| min_outer(*it, h, pad, t)).sum();
    // The same gap accounting stack_heights() does: gaps compete with the
    // panels for `span` by weight before the minimums are compared, so a
    // column that "fits" here must still fit once stack_heights() takes
    // its cut for the gaps between panels.
    let n = c.panels.len() as f32;
    let weights: f32 = c.panels.iter().map(|it| it.weight).sum();
    let total = weights + c.gap * (n - 1.0).max(0.0);
    let gap_px = c.gap / total.max(0.001) * span;
    let content_span = span - gap_px * (n - 1.0).max(0.0);
    need <= content_span
}

/// Landscape flexbox layout: the columns in a row, solved by taffy.
fn landscape(fl: &FlexLayaut, w: f32, h: f32, pad: f32, t: &SizeTable) -> Layout {
    // Page padding: the layout's own when it names one (pad_x in the
    // file, percent per side), the engine's thin margin otherwise.
    let pad_x = match fl.pad_x {
        Some(p) => (w * p / 100.0).max(4.0),
        None => (w * 0.006).max(4.0),
    };
    let gap = (w * 0.005).max(4.0);
    let inner = w - 2.0 * pad_x;
    // min/max are device-independent (a 1080-line reference) unless the
    // file said `units = px`.
    let u = if fl.units_px { 1.0 } else { lu(h) };
    // The vertical span a column stacks into (the classic 2.5vh..97vh).
    let span = h * (0.97 - 0.025);

    // Collapse, two questions in order. First HEIGHT, per column: a
    // stacked column whose panels' minimums do not fit the span is
    // dropped rather than crushed — lowest collapse value first, never
    // the collapse = 0 columns.
    let mut vis: Vec<&FlexColumn> = fl.columns.iter().collect();
    loop {
        let idx = vis
            .iter()
            .enumerate()
            .filter(|(_, c)| c.collapse > 0 && !column_fits(c, h, pad, span, t))
            .min_by_key(|(_, c)| c.collapse)
            .map(|(i, _)| i);
        match idx {
            Some(i) => {
                vis.remove(i);
            }
            None => break,
        }
    }
    // Then WIDTH: drop columns (lowest collapse value first) while the
    // visible minimum widths do not fit.
    loop {
        let mins: f32 = vis.iter().map(|c| (c.min * u).max(60.0)).sum::<f32>()
            + gap * (vis.len().saturating_sub(1)) as f32;
        let any_collapsible = vis.iter().any(|c| c.collapse > 0);
        if mins <= inner || !any_collapsible {
            break;
        }
        let idx = vis
            .iter()
            .enumerate()
            .filter(|(_, c)| c.collapse > 0)
            .min_by_key(|(_, c)| c.collapse)
            .map(|(i, _)| i)
            .unwrap();
        vis.remove(idx);
    }

    // A layout that lost a bar instance's column gets a full-width bar
    // at the bottom instead — the way back to the settings survives the
    // column that held it.
    let dropped: Vec<ColumnItem> = anchored_in(fl, PanelAnchor::Bar)
        .into_iter()
        .filter(|it| !vis.iter().any(|c| c.panels.iter().any(|k| k.id == it.id)))
        .collect();
    let bar_h = if dropped.is_empty() { 0.0 } else { h * 0.135 };

    // Column widths via taffy (flex-basis/grow/shrink + min/max).
    let mut tf: TaffyTree<()> = TaffyTree::new();
    let mut nodes = Vec::new();
    for c in &vis {
        // Sanitize NaN/negative values from a malformed .layaut so taffy
        // never produces NaN geometry (which would render off-screen or
        // panic downstream comparisons).
        let basis = if c.basis.is_finite() { c.basis.max(0.0) } else { 16.0 };
        let grow = if c.grow.is_finite() { c.grow.max(0.0) } else { 0.0 };
        let min = if c.min.is_finite() { (c.min * u).max(60.0) } else { 60.0 };
        let style = Style {
            flex_basis: percent(basis / 100.0),
            flex_grow: grow,
            flex_shrink: 1.0,
            min_size: Size { width: length(min), height: auto() },
            max_size: Size {
                width: if c.max.is_finite() { length(c.max * u) } else { auto() },
                height: auto(),
            },
            ..Default::default()
        };
        nodes.push(tf.new_leaf(style).unwrap());
    }
    let root = tf
        .new_with_children(
            Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: length(w), height: length(h) },
                padding: taffy::Rect {
                    left: length(pad_x),
                    right: length(pad_x),
                    top: length(0.0),
                    bottom: length(0.0),
                },
                gap: Size { width: length(gap), height: length(0.0) },
                ..Default::default()
            },
            &nodes,
        )
        .unwrap();
    tf.compute_layout(
        root,
        Size { width: AvailableSpace::Definite(w), height: AvailableSpace::Definite(h) },
    )
    .unwrap();

    // Vertical placement: panels stacked by their height weights; gaps
    // count as weight units, so the classic proportions (a 94.5vh span
    // from 2.5vh to 97vh) come out exactly for the default layout.
    let top = h * 0.025;
    let mut content_bottom = h * 0.97;
    if bar_h > 0.0 {
        content_bottom -= bar_h + h * 0.015;
    }
    let hi = (content_bottom - top).max(1.0);

    let mut out = Layout::empty(w, h);
    for (c, node) in vis.iter().zip(&nodes) {
        let tl = tf.layout(*node).unwrap();
        let r = Rect::new(tl.location.x, top, tl.size.width, hi);
        stack_into(&mut out, &c.panels, r, h, pad, c.gap, t);
    }
    // The bar itself: one instance takes the whole width, several share.
    if bar_h > 0.0 {
        let n = dropped.len() as f32;
        let bw = (inner - gap * (n - 1.0)) / n;
        for (i, it) in dropped.iter().enumerate() {
            let x = pad_x + (bw + gap) * i as f32;
            out.place(it.id, it.widget, Rect::new(x, content_bottom + h * 0.015, bw, bar_h));
        }
    }
    out
}

/// Portrait restack honouring the anchor rules: the `Top` panels in a
/// band at the very top, the `Bottom` ones at the very bottom, the
/// `Bar` ones as their own full-width band between the body row and
/// that, and everything that flows in a row of columns in between. The
/// row is NEVER dropped: hiding eight of twelve widgets because the
/// window is short is a content loss dressed as responsiveness (u1
/// §2.3). A short window only re-proportions the bands, and a narrow
/// one merges the row's columns from the right until they fit.
fn portrait_flex(fl: &FlexLayaut, w: f32, h: f32, pad: f32, t: &SizeTable) -> Layout {
    let small = h < 900.0;
    let edge = (w * 0.008).max(4.0);
    let gap = (h * 0.012).max(4.0);
    let iw = w - 2.0 * edge;
    let mut out = Layout::empty(w, h);

    // Row columns: each source column contributes the panels that flow
    // (the pinned ones take their own bands), one chunk per source
    // column, merged from the right until at most max_chunks remain. No
    // fixed split at 4 — a five-panel column is one chunk, not 4 + a
    // lonely 1.
    let mut chunks: Vec<Vec<ColumnItem>> = Vec::new();
    for c in &fl.columns {
        let body: Vec<ColumnItem> = c
            .panels
            .iter()
            .filter(|it| it.widget.anchor() == PanelAnchor::Flow)
            .copied()
            .collect();
        if !body.is_empty() {
            chunks.push(body);
        }
    }
    let max_chunks = ((iw / (280.0 * lu(h))).floor() as usize).clamp(1, 3);
    while chunks.len() > max_chunks {
        let tail = chunks.pop().unwrap();
        if let Some(last) = chunks.last_mut() {
            last.extend(tail);
        }
    }

    let tops = anchored_in(fl, PanelAnchor::Top);
    let bots = anchored_in(fl, PanelAnchor::Bottom);
    let bars = anchored_in(fl, PanelAnchor::Bar);
    let has_row = !chunks.is_empty();

    // Band proportions: `small` only changes them, never what exists.
    // (top, row, bar, bottom) as fractions of the height; the top band
    // absorbs the slack the gaps leave.
    let (_top_f, row_f, bar_f, bot_f) = if small {
        (0.15, 0.50, 0.12, 0.17)
    } else {
        (0.25, 0.40, 0.13, 0.16)
    };

    let mut bot_h = if bots.is_empty() { 0.0 } else { h * bot_f };
    // A bar panel is ALWAYS its own full-width band in portrait, between
    // the row and the bottom band — never inside a chunk, where the two
    // control buttons took 41 % of the row's height and crushed the
    // instruments.
    let mut bar_h = if bars.is_empty() { 0.0 } else { h * bar_f };
    let mut row_h = if has_row {
        if !tops.is_empty() {
            h * row_f
        } else {
            // Nothing pinned to the top: the row takes that band too.
            let mut rest = h - 2.0 * gap;
            if bot_h > 0.0 {
                rest -= bot_h + gap;
            }
            if bar_h > 0.0 {
                rest -= bar_h + gap;
            }
            rest.max(h * row_f)
        }
    } else {
        0.0
    };

    let mut used = 0.0;
    let mut bands = 0.0;
    for ph in [bot_h, row_h, bar_h] {
        if ph > 0.0 {
            used += ph + gap;
            bands += 1.0;
        }
    }
    let top_h = (h - 2.0 * gap - used).max(h * 0.2);
    if !tops.is_empty() && top_h > h - 2.0 * gap - used {
        // The floor left less room than the row/bar/bottom bands need:
        // shrink them together instead of letting the row spill into the
        // bands below it — the re-proportioning this function promises,
        // not a top band bought at the row's expense.
        let s = bot_h + row_h + bar_h;
        if s > 0.0 {
            let k = (h - (2.0 + bands) * gap - top_h).max(0.0) / s;
            bot_h *= k;
            row_h *= k;
            bar_h *= k;
        }
    }

    // The top band at the very top.
    let mut y = gap;
    if !tops.is_empty() {
        stack_band(&mut out, &tops, Rect::new(edge, y, iw, top_h), h, pad, t);
        y += top_h + gap;
    }

    // The bottom band at the very bottom.
    if bot_h > 0.0 {
        let r = Rect::new(edge, h - gap - bot_h, iw, bot_h);
        stack_band(&mut out, &bots, r, h, pad, t);
    }

    // The bar directly above it (or at the bottom when there is no
    // bottom band).
    if bar_h > 0.0 {
        let mut by = h - gap - bar_h;
        if bot_h > 0.0 {
            by -= bot_h + gap;
        }
        stack_band(&mut out, &bars, Rect::new(edge, by, iw, bar_h), h, pad, t);
    }

    if has_row {
        // Column headers (e.g. NETWORK) draw above their rect — start
        // the columns slightly lower to leave room for them.
        let d = h * 0.025;
        let cgap = (w * 0.01).max(4.0);
        let units: f32 = chunks
            .iter()
            .map(|body| if body.len() >= 4 { 1.2 } else { 1.0 })
            .sum();
        let ncols = chunks.len() as f32;
        let cw = (iw - cgap * (ncols - 1.0).max(0.0)) / units.max(0.5);
        let mut x = edge;
        for body in chunks.iter() {
            let this_w = cw * if body.len() >= 4 { 1.2 } else { 1.0 };
            // Stack the body panels by their weights, with per-panel
            // minimum heights (content + widget padding). When even the
            // minimums do not fit — nine instruments in a phone-sized
            // window — stack_heights scales them down together: small,
            // but present, which the amendment's content test demands.
            let r = Rect::new(x, y + d, this_w, row_h - d);
            stack_into(&mut out, body, r, h, pad, 1.0, t);
            x += this_w + cgap;
        }
    }
    out
}

/// A rectangle board's instances at this window size: each one's own
/// vw/vh box, in physical pixels. An instance with no rectangle of its
/// own has no place on a rectangle board and is parked outside.
fn rect_layout(w: f32, h: f32, insts: &[Instance]) -> Layout {
    let (vw, vh) = (w / 100.0, h / 100.0);
    let mut out = Layout::empty(w, h);
    for i in insts {
        let ps = i.rect.unwrap_or(crate::base::OFF_SPEC);
        out.place(
            i.id,
            i.widget,
            Rect::new(ps.x * vw, ps.y * vh, ps.w * vw, ps.h * vh),
        );
    }
    out
}

/// Landscape adaptation of rectangle boards (authored at the 16:9
/// reference): an edge-anchored horizontal transform — instances keep
/// their distance to the nearer window edge, so side columns keep a
/// sane width on any aspect ratio.
fn edge_adapt(insts: &[Instance], ratio: f32) -> Vec<Instance> {
    let f = ((16.0 / 9.0) / ratio).clamp(0.5, 1.4);
    if (f - 1.0).abs() < 0.001 {
        return insts.to_vec();
    }
    insts
        .iter()
        .map(|i| {
            let Some(p) = i.rect else { return *i };
            if p.x >= 100.0 {
                return *i;
            }
            let a = p.x;
            let b = p.x + p.w;
            let na = if a <= 50.0 { a * f } else { 100.0 - (100.0 - a) * f };
            let nb = if b <= 50.0 { b * f } else { 100.0 - (100.0 - b) * f };
            Instance {
                rect: Some(PanelSpec { x: na, y: p.y, w: (nb - na).max(1.0), h: p.h }),
                ..*i
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::base::WidgetCategory;
    use crate::layout::InstanceId;

    fn home() -> Vec<Instance> {
        default_instances().all().to_vec()
    }

    fn solved(w: f32, h: f32) -> Layout {
        compute(w, h, &LayoutMode::Flex, 8.0, &home())
    }

    fn id_of(insts: &[Instance], name: &str) -> InstanceId {
        let p = Panel::from_name(name).expect(name);
        insts.iter().find(|i| i.widget == p).expect(name).id
    }

    fn placed(l: &Layout, id: InstanceId, w: f32) -> bool {
        l.of(id).x < w
    }

    /// u1 §5.5 (1): every registered BOARD widget is on the HOME board.
    /// This is the test that failed before the §1.1 arrangement —
    /// `uptime` was registered, drawable and on no board at all. A
    /// widget of another category is not homeless when HOME does not
    /// hold it: its home is the fixture its category names, and the
    /// fixtures ship empty by design.
    #[test]
    fn every_registered_widget_is_placed() {
        install_test_registry();
        let insts = home();
        let l = solved(1920.0, 1080.0);
        for p in Panel::all().into_iter().filter(|p| p.category() == WidgetCategory::Board) {
            let id = id_of(&insts, p.name());
            assert!(placed(&l, id, 1920.0), "{} is not on the board at 1920x1080", p.name());
        }
    }

    /// u1 §5.5 (2): the same holds in portrait, tall and phone-sized.
    /// This is the test the old `small = h < 900` failed — it hid eight
    /// of the twelve widgets on any window under 900 lines.
    #[test]
    fn every_widget_placed_in_portrait_too() {
        install_test_registry();
        let insts = home();
        for (w, h) in [(1080.0, 1920.0), (720.0, 1280.0), (400.0, 800.0)] {
            let l = solved(w, h);
            for p in Panel::all().into_iter().filter(|p| p.category() == WidgetCategory::Board) {
                let id = id_of(&insts, p.name());
                assert!(placed(&l, id, w), "{} is not on the board at {}x{}", p.name(), w, h);
            }
        }
    }

    /// The arrangement is COMPOSED, never written down: which column a
    /// widget stands in, where in it and how much of it it takes are the
    /// addon's own declarations, and the engine reads them off the
    /// registry without knowing one name.
    #[test]
    fn the_arrangement_is_composed_from_what_the_addons_declared() {
        install_test_registry();
        let fl = default_flex();
        let col = |i: usize| -> Vec<&'static str> {
            fl.columns[i].panels.iter().map(|it| it.widget.name()).collect()
        };
        assert_eq!(col(0), ["w01", "w02", "w03", "w04", "w05", "w06"], "slot: left");
        assert_eq!(col(1), ["w07", "w08"], "slot: center");
        assert_eq!(col(2), ["w09", "w10", "w11", "w12"], "slot: right");
        // A widget that named no share of its column asked to be as
        // tall as it says it is.
        assert_eq!(fl.columns[0].panels[0].weight, 4.5);
    }

    /// A COLUMN WITH NO GROWER STILL FILLS. When the reflow leaves a column
    /// of content-sized panels and no growing one to take the slack — a
    /// second monitor that pushed the growing panel into another column —
    /// they stretch together to fill it, instead of keeping their content
    /// size and leaving the rest of the column empty. That empty rest was
    /// the gaps the owner saw moving the desktop to a wider screen.
    #[test]
    fn a_column_of_content_panels_fills_with_no_grower() {
        let weights = [1.0, 1.0, 1.0];
        let mins = [10.0, 10.0, 10.0];
        let wants = [Some(50.0), Some(50.0), Some(50.0)]; // 150 asked
        let span = 600.0; // the column is far taller than the content
        let (hs, gap_px) = stack_heights(&weights, &mins, &wants, 0.0, span);
        let filled: f32 = hs.iter().sum::<f32>() + gap_px * (hs.len() as f32 - 1.0);
        assert!((filled - span).abs() < 0.5, "the column left {} px empty", span - filled);
        assert!(hs[0] > 50.0, "the panel kept its content size and left a gap: {}", hs[0]);
    }

    /// The pinned edges are the addons' request, not the engine's
    /// opinion: the `Top` panel opens the work column, the `Bottom` one
    /// closes it, and the `Bar` panel sits at the foot of the first
    /// column — from where a lost column brings it back as a full-width
    /// bar rather than dropping it.
    #[test]
    fn declared_anchors_pin_the_panels_that_asked() {
        install_test_registry();
        let insts = home();
        let (w, h) = (1920.0, 1080.0);
        let l = solved(w, h);
        let top = id_of(&insts, "w07");
        let bottom = id_of(&insts, "w08");
        let bar = id_of(&insts, "w06");
        let centre: Vec<Rect> = l
            .iter()
            .filter(|p| (p.rect.x - l.of(top).x).abs() < 0.5)
            .map(|p| p.rect)
            .collect();
        assert!(
            centre.iter().all(|r| r.y >= l.of(top).y - 0.5),
            "the top anchor must open its column"
        );
        assert!(
            centre.iter().all(|r| r.bottom() <= l.of(bottom).bottom() + 0.5),
            "the bottom anchor must close its column"
        );
        let left: Vec<Rect> = l
            .iter()
            .filter(|p| (p.rect.x - l.of(bar).x).abs() < 0.5)
            .map(|p| p.rect)
            .collect();
        assert!(
            left.iter().all(|r| r.bottom() <= l.of(bar).bottom() + 0.5),
            "the bar anchor must close the first column"
        );
        // A landscape window too narrow for the side columns loses them
        // both — and the bar panel comes back full width at the foot of
        // the window instead of going with them.
        let (nw, nh) = (450.0, 300.0);
        let narrow = solved(nw, nh);
        assert!(narrow.of(bar).w > nw * 0.9, "the bar must span the window");
        assert!(narrow.of(bar).y > nh * 0.8, "and sit at its foot");
    }

    /// u1 §5.5 (5): the side column keeps the same proportion to the
    /// reference width (h * 0.30) at every screen size. Before the
    /// device-independent units it was 0.97 at 1080p and 0.62 at 4K —
    /// the console became a terminal with two ribbons.
    #[test]
    fn proportions_are_resolution_independent() {
        install_test_registry();
        let insts = home();
        let side = id_of(&insts, "w01");
        let ws_at = |w: f32, h: f32| -> f32 { solved(w, h).of(side).w / (h * 0.30) };
        let all = [
            ws_at(1280.0, 720.0),
            ws_at(1920.0, 1080.0),
            ws_at(2560.0, 1440.0),
            ws_at(3840.0, 2160.0),
        ];
        let lo = all.iter().cloned().fold(f32::INFINITY, f32::min);
        let hi = all.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        assert!(hi - lo <= 0.02, "side column proportion drifts with resolution: {all:?}");
    }

    /// The 400x800 portrait row is the honest limit: one merged chunk,
    /// nine instruments, scaled below their minimums together rather
    /// than hidden. The last of them must land in the row, not
    /// off-screen.
    #[test]
    fn phone_sized_portrait_merges_into_one_chunk() {
        install_test_registry();
        let insts = home();
        let l = solved(400.0, 800.0);
        let last = id_of(&insts, "w12");
        let first = id_of(&insts, "w01");
        // One chunk means the first and the last instrument share a
        // column: same x.
        assert!((l.of(last).x - l.of(first).x).abs() < 0.5);
    }

    /// The minimums the solver keeps are OUTER heights: content plus
    /// the container's chrome around it. Before the chrome term they
    /// were content-only, so a squeezed portrait column was solved as
    /// if every title band were free — and each titled panel came out
    /// exactly one band short, its widget painting over the neighbour
    /// below (FILESYSTEM's tiles over the process header, 900x1600).
    #[test]
    fn published_chrome_raises_a_panels_minimum() {
        install_test_registry();
        let (w, h, pad) = (900.0, 1600.0, 8.0);
        let insts = home();
        let net = Panel::from_name("w11").unwrap();
        let net_id = id_of(&insts, "w11");
        let sizes = crate::base::default_sizes();
        let n = sizes.len();
        let mut chrome = vec![0.0; n];
        chrome[net.idx()] = 40.0;
        let bare = SizeTable::new(sizes.clone(), vec![None; n], vec![0.0; n]);
        let dressed = SizeTable::new(sizes, vec![None; n], chrome);
        let l0 = compute_in(w, h, &LayoutMode::Flex, pad, &bare, &insts);
        let l1 = compute_in(w, h, &LayoutMode::Flex, pad, &dressed, &insts);
        // The band is not free: the panel's share must grow by (most
        // of) the published chrome, not stay at the content minimum.
        assert!(
            l1.of(net_id).h > l0.of(net_id).h + 20.0,
            "chrome ignored: {} -> {}",
            l0.of(net_id).h,
            l1.of(net_id).h
        );
        // And the column still holds: instances sharing that column may
        // not overlap each other.
        let col: Vec<Rect> = l1
            .iter()
            .filter(|p| (p.rect.x - l1.of(net_id).x).abs() < 0.5)
            .map(|p| p.rect)
            .collect();
        for a in 0..col.len() {
            for b in (a + 1)..col.len() {
                let (ra, rb) = (col[a], col[b]);
                assert!(
                    ra.y + ra.h <= rb.y + 0.5 || rb.y + rb.h <= ra.y + 0.5,
                    "column panels overlap: {ra:?} vs {rb:?}"
                );
            }
        }
    }

    /// THE feature: the same widget, twice, on ONE board. Two entries,
    /// two identities, two rectangles that do not overlap — and neither
    /// of them is "the" terminal.
    #[test]
    fn one_widget_twice_on_one_board_gets_two_rectangles() {
        install_test_registry();
        let w = Panel::from_name("w07").unwrap();
        let mut l = InstanceList::new();
        let a = l.add(w, (0, 0), None);
        let b = l.add(w, (0, 0), None);
        let lay = compute(1920.0, 1080.0, &LayoutMode::Flex, 8.0, l.all());
        assert_eq!(lay.len(), 2, "both instances must be placed");
        let (ra, rb) = (lay.of(a), lay.of(b));
        assert!(ra.x < 1920.0 && rb.x < 1920.0, "both must be on screen");
        assert!(
            ra.y + ra.h <= rb.y + 0.5 || rb.y + rb.h <= ra.y + 0.5,
            "two instances of one widget must not overlap: {ra:?} vs {rb:?}"
        );
        // And the rectangles really are told apart by IDENTITY: asking
        // by widget kind can only ever answer with one of them.
        assert_eq!(lay.instances_of(w).len(), 2);
    }

    /// A rectangle board places each instance at its own box — so two
    /// instances of one widget can sit side by side, which a table
    /// indexed by widget could not express at all.
    #[test]
    fn a_rect_board_places_each_instance_at_its_own_box() {
        install_test_registry();
        let w = Panel::from_name("w03").unwrap();
        let mut l = InstanceList::new();
        let a = l.add(w, (0, 0), Some(PanelSpec { x: 0.0, y: 0.0, w: 40.0, h: 50.0 }));
        let b = l.add(w, (0, 0), Some(PanelSpec { x: 50.0, y: 0.0, w: 40.0, h: 50.0 }));
        let lay = compute(1600.0, 900.0, &LayoutMode::Rects, 8.0, l.all());
        assert!(lay.of(a).x < lay.of(b).x);
        assert!((lay.of(a).w - lay.of(b).w).abs() < 0.5);
    }
}
