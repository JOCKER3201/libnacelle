//! The table's arithmetic and its interaction state.
//!
//! Two things live here, and neither of them draws:
//!
//! * [`solve_widths`] — the column-width solver `ui::table` has used
//!   since u2 §2.7, lifted out unchanged so it can be held still by a
//!   test and reached by a second drawing path. The equivalence test in
//!   `tests/table_widths.rs` runs the arithmetic as it stood beside this
//!   function on the same inputs and demands the same floats: that test
//!   is the whole justification for moving the code at all.
//! * [`TableState`] — what a table remembers between frames: the sort,
//!   the columns the user has dragged, the selected row and the scroll
//!   offset. A table with the default state draws exactly what a table
//!   with no state drew, which is why the master's look does not move.
//!
//! The sort is the RENDERER's, never the script's: a script stays a pure
//! function of its data (the sandbox and the per-frame cache both rest
//! on that), so it hands over rows in its own order and the view shows
//! them in the user's. Selection is by KEY rather than by index for the
//! same reason — the model is rebuilt every snapshot and an index means
//! nothing across two of them.

use crate::view::scroll::ScrollView;
use crate::Rect;
use std::cmp::Ordering;

/// Which way a sorted column runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SortDir {
    Asc,
    Desc,
}

impl SortDir {
    pub fn flip(self) -> SortDir {
        match self {
            SortDir::Asc => SortDir::Desc,
            SortDir::Desc => SortDir::Asc,
        }
    }

    /// The word a script writes and reads it as.
    pub fn word(self) -> &'static str {
        match self {
            SortDir::Asc => "asc",
            SortDir::Desc => "desc",
        }
    }

    /// The word back to the direction. Anything else is ascending —
    /// a typo in a script must not be an error dialog.
    pub fn from_word(w: &str) -> SortDir {
        if w.eq_ignore_ascii_case("desc") {
            SortDir::Desc
        } else {
            SortDir::Asc
        }
    }
}

// --------------------------------------------------------- the solver

/// What one column measured, in the terms the solver works in.
///
/// The MEASURING is the drawer's job — only it has the fonts — and it
/// is also the one part of the old code that must not move, so this
/// struct is deliberately dumb: three numbers, no opinions.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColMeasure {
    /// Width of the heading text.
    pub head: f32,
    /// Width of the widest cell the drawer measured, or `head` for a
    /// column that is measured from its heading alone (`ColWidth::
    /// Heading`) — the solver has no other way to tell the two apart,
    /// and the arithmetic it inherited did not either.
    pub content: f32,
    /// Whether the column reserves room for a bar track beside its
    /// number.
    pub bar: bool,
}

impl ColMeasure {
    /// A column measured from its heading only.
    pub fn heading(head: f32) -> ColMeasure {
        ColMeasure { head, content: head, bar: false }
    }
}

/// The `table.*` metrics the solver reads.
///
/// **The shrink factor is applied by the CALLER, and not to everything.**
/// `col_gap`, `cell_pad` and `bar_w` arrive already multiplied by
/// `TableStyle::shrink`; `elastic_min_w` and `col_min_w` arrive raw.
/// That asymmetry is not a design — it is what `ui::table` has always
/// computed, and reproducing it exactly is the condition for the master
/// rendering the same pixels after this move as before it. Changing it
/// is a look change and belongs in a theme, not here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TableTokens {
    /// `table.col_gap`, shrunk.
    pub col_gap: f32,
    /// `table.cell_pad`, shrunk.
    pub cell_pad: f32,
    /// `table.bar_w`, shrunk.
    pub bar_w: f32,
    /// `table.elastic_min_w`, RAW.
    pub elastic_min_w: f32,
    /// `table.col_min_w`, RAW.
    pub col_min_w: f32,
}

impl TableTokens {
    /// What every column's width reserves beyond its content, so a
    /// right-aligned column ends a full gap before its neighbour.
    pub fn extra(&self) -> f32 {
        self.col_gap + self.cell_pad
    }
}

/// Which column absorbs the leftover width.
///
/// Normally the one the caller named. A column the user has dragged to
/// a width is PINNED — it is the one thing on screen the user said out
/// loud — so when the elastic column itself is pinned the elasticity
/// moves to the last column that is not. When every column is pinned
/// nothing absorbs, and the answer is out of range on purpose: the table
/// then simply does not fill its box, which is honest.
fn elastic_of(n: usize, want: usize, overrides: &[Option<f32>]) -> usize {
    let pinned = |i: usize| overrides.get(i).copied().flatten().is_some();
    if want >= n || !pinned(want) {
        return want;
    }
    (0..n).rev().find(|i| !pinned(*i)).unwrap_or(n)
}

/// The column-width solver `ui::table` has always used, held still for
/// tests: measure → slack ladder (bar reservations first, then content
/// measure back toward the heading) → the elastic column takes whatever
/// is left, floored at `table.col_min_w`.
///
/// `overrides[i]` — a width the user dragged a divider to — wins over
/// the measure and yields no slack. A short (or empty) `overrides` is
/// read as "none": `ui::table` passes `&[]`.
pub fn solve_widths(
    measured: &[ColMeasure],
    avail: f32,
    elastic: usize,
    overrides: &[Option<f32>],
    t: &TableTokens,
) -> Vec<f32> {
    let n = measured.len();
    let extra = t.extra();
    let elastic = elastic_of(n, elastic, overrides);
    // Beside each width, the slack it can give back when the panel is
    // narrow: a bar cell's track reservation (the track is a second
    // reading of a number that stays), and the content measure's excess
    // over the heading.
    let mut widths: Vec<f32> = Vec::with_capacity(n);
    let mut bar_slack: Vec<f32> = vec![0.0; n];
    let mut content_slack: Vec<f32> = vec![0.0; n];
    for (i, m) in measured.iter().enumerate() {
        if let Some(w) = overrides.get(i).copied().flatten() {
            widths.push(w.max(t.col_min_w + extra));
            continue;
        }
        let mut w = m.head;
        if i != elastic {
            w = w.max(m.content);
            content_slack[i] = w - m.head;
            if m.bar {
                bar_slack[i] = t.bar_w + t.col_gap;
                w += t.bar_w + t.col_gap;
            }
        }
        widths.push(w + extra);
    }
    let sum_fixed = |ws: &[f32]| -> f32 {
        ws.iter()
            .enumerate()
            .filter(|(i, _)| *i != elastic)
            .map(|(_, w)| *w)
            .sum()
    };
    // The elastic column carries prose — u2 §2.7's NAME — and a content
    // measure that starves it drops whole strings, not just their tails.
    // Below `table.elastic_min_w` the fixed columns yield their slack in
    // a stated order, each rung proportionally, so the columns keep
    // their relative widths on the way down.
    let elastic_min = t.elastic_min_w + extra;
    let mut deficit = elastic_min - (avail - sum_fixed(&widths));
    for slack in [&bar_slack, &content_slack] {
        if deficit <= 0.0 {
            break;
        }
        let total: f32 = slack.iter().sum();
        if total <= 0.0 {
            continue;
        }
        let k = (deficit / total).min(1.0);
        for (w, s) in widths.iter_mut().zip(slack.iter()) {
            *w -= s * k;
        }
        deficit -= total * k;
    }
    let leftover = avail - sum_fixed(&widths);
    if let Some(w) = widths.get_mut(elastic) {
        // Whatever the yield freed — floored so the column shows a
        // trimmed string, never a bare ellipsis, even when the panel is
        // narrower than the headings themselves.
        *w = leftover.max(t.col_min_w + extra);
    }
    widths
}

// ---------------------------------------------------------- the order

/// The number at the front of a formatted cell (`"41.2%"` → 41.2), for
/// a bar cell reading the value it also prints and for a sort that must
/// put 9 before 10. `None` when the cell does not start with one — a bar
/// of nothing is drawn empty, never invented.
pub fn leading_number(text: &str) -> Option<f32> {
    let end = text
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == '-'))
        .unwrap_or(text.len());
    text[..end].parse::<f32>().ok().filter(|v| v.is_finite())
}

/// How two cells of the same column compare.
///
/// Numbers first: a column of `1471` and `987` is a column of numbers
/// whatever the strings say, and sorting it as text is the classic
/// defect. Everything else compares as text without regard to case,
/// because `Firefox` and `firefox` belong next to each other.
pub fn compare(a: &str, b: &str) -> Ordering {
    match (leading_number(a), leading_number(b)) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        _ => a
            .chars()
            .flat_map(char::to_lowercase)
            .cmp(b.chars().flat_map(char::to_lowercase)),
    }
}

// ---------------------------------------------------------- the state

/// What identifies the cached order. Recomputing the order costs a sort
/// of up to `rhai::max_array_size` rows; doing it per frame at sixty
/// frames a second is the performance trap §11 names by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct OrderKey {
    generation: u64,
    len: usize,
    col: usize,
    desc: bool,
    sorted: bool,
}

/// What identifies the cached content-width measurement — [`OrderKey`]'s
/// pattern, reused rather than reinvented for a second per-frame cost a
/// model-backed table carries: measuring every visible cell's text with
/// the font system is not free, and doing it sixty times a second for a
/// screen that has not moved is the same trap `OrderKey` already exists
/// to keep the sort out of.
///
/// It is not simply `OrderKey` renamed, because the measure depends on
/// something the order does not: WHICH rows are on screen. The order is
/// a permutation of every row and does not care what the viewport shows;
/// the content measure only ever looked at the visible window (u2 §2.7's
/// "measured from its widest CELL" has only ever meant the widest cell
/// that could be seen), so a row scrolling into view — one that might be
/// the widest the column has had all along — has to invalidate the
/// cache even when the model's generation and length have not moved.
/// `window_first`/`window_count` carry that.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WidthKey {
    pub generation: u64,
    pub len: usize,
    pub cols: usize,
    pub window_first: usize,
    pub window_count: usize,
}

/// What the last draw put on screen.
///
/// Input arrives BETWEEN frames, with no drawing context and no theme
/// lookup of its own — a click on the scrollbar track has to page by one
/// viewport, and a press on the thumb has to know where the thumb is. So
/// the drawing records what it decided and the input reads it back,
/// which is the same discipline the host follows when it hands a widget
/// the rectangle its last draw used.
#[derive(Clone, Copy, Debug, Default)]
pub struct Extent {
    /// Whether the body scrolls at all — a table that was never given
    /// `scroll: true` ignores the wheel exactly as it always has.
    pub scrollable: bool,
    /// The body's height, without the header.
    pub viewport: f32,
    /// The height every row together occupies.
    pub content: f32,
    /// The bar's track and thumb, when one was drawn.
    pub bar: Option<(Rect, Rect)>,
}

/// Everything one table remembers between frames.
///
/// A [`TableState::default`] is the table that exists today: no sort, no
/// dragged columns, nothing selected and an offset of zero.
#[derive(Debug, Default)]
pub struct TableState {
    /// The sorted column and its direction; `None` is the order the
    /// script handed over.
    pub sort: Option<(usize, SortDir)>,
    /// Widths the user dragged a divider to, per column. Per session:
    /// persistence belongs to the phase that owns paths and settings.
    pub widths: Vec<Option<f32>>,
    /// The KEY of the selected row — never an index, because the model
    /// is rebuilt every snapshot.
    pub selected: Option<String>,
    pub scroll: ScrollView,
    /// What the last draw put on screen ([`Extent`]).
    pub extent: Extent,
    /// Bumped by every change above. A script's answer is cached per
    /// frame, and a click has to invalidate that cache within the frame
    /// it lands in, or the panel shows the state before the click.
    pub interact_epoch: u64,
    /// `order[i]` is the model row shown at display position `i`.
    order: Vec<usize>,
    order_key: Option<OrderKey>,
    /// The last `ColWidth::Content` measurement, and the [`WidthKey`] it
    /// was built from — `None` until the first draw. Skipped entirely
    /// for `WidthKey.generation == 0` (see [`TableState::cached_measure`]),
    /// so a caller with no stable generation pays no bookkeeping for a
    /// cache it can never hit.
    width_cache: Option<(WidthKey, Vec<ColMeasure>)>,
    /// The column heading under a press that has not been released —
    /// the `press` rung of the `table.head` class.
    pressed_head: Option<usize>,
    /// A divider being dragged: the column, the pointer x it was
    /// grabbed at, and the width the column had then. Absolute, so a
    /// dropped frame cannot make the column drift.
    grabbed_divider: Option<(usize, f32, f32)>,
}

impl TableState {
    pub fn new() -> TableState {
        TableState::default()
    }

    /// Something the user did changed what this table shows.
    fn touch(&mut self) {
        self.interact_epoch = self.interact_epoch.wrapping_add(1);
    }

    /// A click on column `col`'s heading: sort by it ascending, then
    /// descending, then back to the script's own order. Three states,
    /// because `sort = None` has to be reachable — a user who sorted a
    /// live table must be able to hand it back to the script.
    pub fn click_head(&mut self, col: usize) {
        self.sort = match self.sort {
            Some((c, SortDir::Asc)) if c == col => Some((col, SortDir::Desc)),
            Some((c, SortDir::Desc)) if c == col => None,
            _ => Some((col, SortDir::Asc)),
        };
        self.touch();
    }

    /// Selects a row by key, or clears the selection with `None`.
    pub fn select(&mut self, key: Option<String>) {
        if self.selected != key {
            self.selected = key;
            self.touch();
        }
    }

    /// Whether `key` is the selected row.
    pub fn is_selected(&self, key: &str) -> bool {
        self.selected.as_deref() == Some(key)
    }

    /// The heading being pressed, for the class ladder.
    pub fn pressed_head(&self) -> Option<usize> {
        self.pressed_head
    }

    pub fn press_head(&mut self, col: usize) {
        self.pressed_head = Some(col);
    }

    pub fn release_head(&mut self) {
        self.pressed_head = None;
    }

    /// The width override of a column, if the user set one.
    pub fn width_of(&self, col: usize) -> Option<f32> {
        self.widths.get(col).copied().flatten()
    }

    /// Sets — or with `None` clears — a column's width.
    pub fn set_width(&mut self, col: usize, w: Option<f32>) {
        if self.width_of(col) == w {
            return;
        }
        if self.widths.len() <= col {
            self.widths.resize(col + 1, None);
        }
        self.widths[col] = w;
        self.touch();
    }

    /// The pointer took hold of the divider on the right of column
    /// `col`, which is `width` wide right now.
    pub fn grab_divider(&mut self, col: usize, x: f32, width: f32) {
        self.grabbed_divider = Some((col, x, width));
    }

    pub fn dragging_divider(&self) -> Option<usize> {
        self.grabbed_divider.map(|(c, _, _)| c)
    }

    /// The pointer moved while holding a divider: the column becomes as
    /// wide as the hand has taken it, floored at `min_w` so it cannot be
    /// dragged out of existence.
    pub fn drag_divider(&mut self, x: f32, min_w: f32) {
        let Some((col, x0, w0)) = self.grabbed_divider else { return };
        self.set_width(col, Some((w0 + (x - x0)).max(min_w)));
    }

    pub fn release_divider(&mut self) {
        self.grabbed_divider = None;
    }

    /// Rebuilds the display order, but only when it can have changed:
    /// the model was rewritten (`generation`), it changed length, or the
    /// sort moved. §11's rule, and the reason the sort is a permutation
    /// of indices rather than a copy of the rows.
    ///
    /// `cell` answers the sorted column's text for a model row. The sort
    /// is STABLE: process rows are rewritten once a second and equal
    /// keys must not make rows trade places every time.
    ///
    /// A model rewritten WITHOUT its generation moving (a script that
    /// invents rows from the clock) keeps the previous permutation until
    /// the generation does move. Every row is still shown exactly once —
    /// it is a permutation — it is only not freshly sorted, which is the
    /// price of not sorting four thousand rows sixty times a second.
    pub fn refresh_order<F: Fn(usize) -> String>(
        &mut self,
        generation: u64,
        len: usize,
        cell: F,
    ) {
        let (col, desc, sorted) = match self.sort {
            Some((c, d)) => (c, d == SortDir::Desc, true),
            None => (0, false, false),
        };
        let key = OrderKey { generation, len, col, desc, sorted };
        if self.order_key == Some(key) && self.order.len() == len {
            return;
        }
        self.order_key = Some(key);
        self.order.clear();
        self.order.extend(0..len);
        if sorted {
            // The keys are pulled once per row rather than once per
            // comparison: a comparison sort asks O(n log n) times, and
            // each ask would otherwise walk a rhai array.
            let keys: Vec<String> = (0..len).map(&cell).collect();
            self.order.sort_by(|a, b| {
                let o = compare(&keys[*a], &keys[*b]);
                if desc {
                    o.reverse()
                } else {
                    o
                }
            });
        }
    }

    /// The display order: `order()[i]` is the model row shown at `i`.
    /// Identity until a sort is set, and always exactly as long as the
    /// model was at the last [`TableState::refresh_order`].
    pub fn order(&self) -> &[usize] {
        &self.order
    }

    /// The display position of a model row, for putting the selection
    /// back in view. Linear — used on a click, never per frame.
    pub fn display_of(&self, row: usize) -> Option<usize> {
        self.order.iter().position(|r| *r == row)
    }

    /// The cached `ColWidth::Content` measurement for `key`, or `None` on
    /// a miss: no cache built yet, a `key` that does not match the one
    /// the cache was built from, or `key.generation == 0`.
    ///
    /// Generation 0 is never served from cache on purpose — it is the
    /// trait's own "no opinion" default ([`super::table_model::
    /// TableModel::generation`]'s doc), and a model that cannot say when
    /// it last changed gets exactly what `ui::table_surface` has always
    /// given it: a fresh measure every frame, not a stale one because two
    /// unrelated snapshots happened to share the same window and length.
    pub fn cached_measure(&self, key: WidthKey) -> Option<&[ColMeasure]> {
        if key.generation == 0 {
            return None;
        }
        match &self.width_cache {
            Some((k, m)) if *k == key => Some(m),
            _ => None,
        }
    }

    /// Remembers `measured` against `key`, for the next frame's
    /// [`TableState::cached_measure`] to find. A no-op at
    /// `key.generation == 0` — the same case [`TableState::cached_measure`]
    /// never serves, so there is nothing to gain by holding onto it.
    pub fn set_width_cache(&mut self, key: WidthKey, measured: Vec<ColMeasure>) {
        if key.generation == 0 {
            return;
        }
        self.width_cache = Some((key, measured));
    }
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn tokens() -> TableTokens {
        TableTokens {
            col_gap: 13.0,
            cell_pad: 3.0,
            bar_w: 32.0,
            elastic_min_w: 48.0,
            col_min_w: 14.0,
        }
    }

    fn m(head: f32, content: f32) -> ColMeasure {
        ColMeasure { head, content, bar: false }
    }

    /// The plain case: fixed columns take their content plus the gap
    /// and the padding, the elastic one takes the rest, and the row
    /// adds up to exactly the width it was given.
    #[test]
    fn the_elastic_column_takes_what_is_left() {
        let t = tokens();
        let cols = [m(30.0, 60.0), m(40.0, 40.0), m(20.0, 90.0)];
        let w = solve_widths(&cols, 400.0, 1, &[], &t);
        assert_eq!(w[0], 60.0 + t.extra());
        assert_eq!(w[2], 90.0 + t.extra());
        assert_eq!(w[1], 400.0 - w[0] - w[2]);
        assert!((w.iter().sum::<f32>() - 400.0).abs() < 1e-3);
    }

    /// A bar column reserves its track, and gives that reservation back
    /// FIRST when the panel is too narrow for the elastic column.
    #[test]
    fn the_bar_reservation_is_the_first_slack_to_go() {
        let t = tokens();
        let bar = ColMeasure { head: 30.0, content: 30.0, bar: true };
        let wide = solve_widths(&[bar, m(20.0, 200.0)], 600.0, 1, &[], &t);
        assert_eq!(wide[0], 30.0 + t.bar_w + t.col_gap + t.extra());
        // Narrow enough that the elastic column is under its floor: the
        // bar column gives its whole reservation back before the content
        // measure of anyone else is touched.
        let tight = solve_widths(&[bar, m(20.0, 200.0)], 120.0, 1, &[], &t);
        assert!(tight[0] < wide[0], "the bar column yielded");
        assert!(tight[0] >= 30.0 + t.extra() - 0.01, "but never below its text");
    }

    /// A dragged width wins over the measure and yields nothing.
    #[test]
    fn a_pinned_column_keeps_the_width_the_user_gave_it() {
        let t = tokens();
        let cols = [m(30.0, 60.0), m(40.0, 40.0)];
        let w = solve_widths(&cols, 300.0, 1, &[Some(123.0), None], &t);
        assert_eq!(w[0], 123.0);
        assert_eq!(w[1], 300.0 - 123.0);
        // Even when the panel is far too narrow to honour it: the floor
        // is the column minimum, not the available width.
        let tight = solve_widths(&cols, 40.0, 1, &[Some(123.0), None], &t);
        assert_eq!(tight[0], 123.0);
        assert_eq!(tight[1], t.col_min_w + t.extra());
    }

    /// Pinning the elastic column moves the elasticity to the last
    /// column that is not pinned; pinning every column leaves nobody to
    /// absorb, and the table simply does not fill its box.
    #[test]
    fn elasticity_moves_off_a_pinned_column() {
        let t = tokens();
        let cols = [m(30.0, 30.0), m(40.0, 40.0), m(20.0, 20.0)];
        let w = solve_widths(&cols, 400.0, 0, &[Some(100.0), None, None], &t);
        assert_eq!(w[0], 100.0);
        assert_eq!(w[1], 40.0 + t.extra());
        assert_eq!(w[2], 400.0 - 100.0 - w[1], "the last free column absorbs");

        let all = solve_widths(&cols, 400.0, 0, &[Some(50.0), Some(60.0), Some(70.0)], &t);
        assert_eq!(all, vec![50.0, 60.0, 70.0]);
    }

    /// An elastic index the script made up points at no column, and
    /// nothing absorbs — the arithmetic `ui::table` has always run for
    /// `table(h, r, 99)`.
    #[test]
    fn an_elastic_index_past_the_end_absorbs_nothing() {
        let t = tokens();
        let cols = [m(30.0, 30.0), m(40.0, 40.0)];
        let w = solve_widths(&cols, 400.0, 99, &[], &t);
        assert_eq!(w, vec![30.0 + t.extra(), 40.0 + t.extra()]);
    }

    /// Numbers sort as numbers even though they arrive as text — the
    /// whole reason the comparator is not `str::cmp`.
    #[test]
    fn cells_that_start_with_a_number_sort_as_numbers() {
        assert_eq!(compare("9", "10"), Ordering::Less);
        assert_eq!(compare("41.2%", "9.5%"), Ordering::Greater);
        assert_eq!(compare("-3", "2"), Ordering::Less);
        assert_eq!(compare("Firefox", "firefox"), Ordering::Equal);
        assert_eq!(compare("alpha", "beta"), Ordering::Less);
        // One side numeric, one side not: text, and no panic.
        assert_eq!(compare("12", "twelve"), Ordering::Less);
    }

    /// The order is a permutation, it is stable, and it is rebuilt only
    /// when it can have changed.
    #[test]
    fn the_order_is_a_stable_permutation_cached_per_generation() {
        let rows = ["b", "a", "b", "a"];
        let cell = |i: usize| rows[i].to_string();
        let mut st = TableState::new();

        st.refresh_order(1, 4, cell);
        assert_eq!(st.order(), &[0, 1, 2, 3], "no sort is the script's order");

        st.sort = Some((0, SortDir::Asc));
        st.refresh_order(1, 4, cell);
        assert_eq!(st.order(), &[1, 3, 0, 2], "stable: equal keys keep their order");

        st.sort = Some((0, SortDir::Desc));
        st.refresh_order(1, 4, cell);
        assert_eq!(st.order(), &[0, 2, 1, 3], "descending, still stable");

        // Same generation, same length, same sort: the cached order,
        // whatever the cell function would now say.
        let lying = |_: usize| "z".to_string();
        st.refresh_order(1, 4, lying);
        assert_eq!(st.order(), &[0, 2, 1, 3]);
        // A new generation rebuilds it.
        st.refresh_order(2, 4, lying);
        assert_eq!(st.order(), &[0, 1, 2, 3]);
        // A model that shrank is never a permutation of the old length.
        st.refresh_order(3, 2, cell);
        assert_eq!(st.order().len(), 2);
        assert!(st.order().iter().all(|i| *i < 2));
    }

    /// A heading cycles ascending → descending → the script's order, and
    /// every step bumps the epoch the element cache is keyed by.
    #[test]
    fn a_heading_click_cycles_and_bumps_the_epoch() {
        let mut st = TableState::new();
        let e0 = st.interact_epoch;
        st.click_head(2);
        assert_eq!(st.sort, Some((2, SortDir::Asc)));
        st.click_head(2);
        assert_eq!(st.sort, Some((2, SortDir::Desc)));
        st.click_head(2);
        assert_eq!(st.sort, None, "back to the order the script gave");
        st.click_head(1);
        assert_eq!(st.sort, Some((1, SortDir::Asc)), "a new column starts over");
        assert!(st.interact_epoch > e0);
    }

    /// Selection is by key, and setting the same key twice is not an
    /// interaction — the cache must not be invalidated for nothing.
    #[test]
    fn selection_is_by_key_and_only_a_change_counts() {
        let mut st = TableState::new();
        st.select(Some("1471".into()));
        let e = st.interact_epoch;
        assert!(st.is_selected("1471"));
        st.select(Some("1471".into()));
        assert_eq!(st.interact_epoch, e, "the same selection changed nothing");
        st.select(None);
        assert!(!st.is_selected("1471"));
        assert!(st.interact_epoch > e);
    }

    fn width_key(generation: u64) -> WidthKey {
        WidthKey { generation, len: 40, cols: 3, window_first: 0, window_count: 12 }
    }

    /// The plain case: nothing cached answers a miss, a `set` then an
    /// identical `key` answers a hit, and the cached slice is exactly
    /// what was stored — the whole reason `ui::table_surface` would ever
    /// reach for this instead of measuring again.
    #[test]
    fn the_width_cache_hits_on_the_same_key() {
        let mut st = TableState::new();
        let key = width_key(1);
        assert!(st.cached_measure(key).is_none(), "nothing stored yet");
        let measured = vec![ColMeasure::heading(10.0), ColMeasure::heading(20.0)];
        st.set_width_cache(key, measured.clone());
        assert_eq!(st.cached_measure(key), Some(measured.as_slice()));
    }

    /// Every field of the key is load-bearing: a moved window, a changed
    /// length, a rewritten generation and a changed column count each
    /// invalidate on their own, because each is a real reason the widest
    /// cell on screen may have changed.
    #[test]
    fn any_change_to_the_key_is_a_miss() {
        let mut st = TableState::new();
        let key = width_key(1);
        st.set_width_cache(key, vec![ColMeasure::heading(10.0)]);
        assert!(st.cached_measure(WidthKey { generation: 2, ..key }).is_none(), "generation");
        assert!(st.cached_measure(WidthKey { len: 41, ..key }).is_none(), "len");
        assert!(st.cached_measure(WidthKey { cols: 4, ..key }).is_none(), "cols");
        assert!(
            st.cached_measure(WidthKey { window_first: 1, ..key }).is_none(),
            "a scrolled window may reveal a wider cell"
        );
        assert!(
            st.cached_measure(WidthKey { window_count: 13, ..key }).is_none(),
            "window_count"
        );
        // The exact key that was stored still hits.
        assert!(st.cached_measure(key).is_some());
    }

    /// `generation() == 0` is the trait's own "no opinion": a table
    /// backed by a model that cannot say when it last changed must
    /// recompute every frame exactly as it always has, so the cache is
    /// never written to and never read from at that generation, even
    /// for a key that would otherwise match perfectly.
    #[test]
    fn generation_zero_never_caches() {
        let mut st = TableState::new();
        let key = width_key(0);
        st.set_width_cache(key, vec![ColMeasure::heading(10.0)]);
        assert!(
            st.cached_measure(key).is_none(),
            "a model with no generation must recompute every frame"
        );
    }

    /// A divider drag is absolute — the column is as wide as the hand
    /// took it from where it grabbed it — so a dropped frame cannot make
    /// it drift, and it cannot be dragged out of existence.
    #[test]
    fn a_divider_drag_is_absolute_and_floored() {
        let mut st = TableState::new();
        st.grab_divider(1, 200.0, 80.0);
        assert_eq!(st.dragging_divider(), Some(1));
        st.drag_divider(230.0, 10.0);
        assert_eq!(st.width_of(1), Some(110.0));
        st.drag_divider(210.0, 10.0);
        assert_eq!(st.width_of(1), Some(90.0), "absolute, not accumulated");
        st.drag_divider(0.0, 10.0);
        assert_eq!(st.width_of(1), Some(10.0), "floored");
        st.release_divider();
        assert_eq!(st.dragging_divider(), None);
        st.set_width(1, None);
        assert_eq!(st.width_of(1), None, "the double-click clears it");
    }
}
