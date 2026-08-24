//! What a TABLE asks of the data behind it — [`super::model::RowModel`]'s
//! twin, not its tenant.
//!
//! `RowModel` already answers this question for a list and a tree: pull a
//! row by index into a reused buffer, so a virtualised view materialises
//! forty rows out of four thousand rather than all four thousand every
//! frame. A table needs exactly that discipline and cannot use `RowModel`
//! to get it — [`super::model::RowBuf`] carries one label, one status, one
//! severity and one bar fraction, because that is everything a list or a
//! tree row ever draws, and a table row is the SCRIPT's own columns,
//! however many of them there are. Forcing an N-column row through a
//! four-field struct would mean either inventing fields `RowBuf` was
//! never built to hold or falling back to the `Vec<String>` this trait
//! exists to avoid materialising.
//!
//! [`super::tree::TreeModel`] already sits beside `RowModel` for the same
//! reason — a tree's `child_count`/`child` shape does not fit `RowModel`
//! either — so a second, differently-shaped trait living next to it is
//! not a new idea here, it is the second time this codebase has reached
//! for it.

/// A table's rows, pulled by index rather than handed over as a `Vec`.
///
/// `ui::table_surface` already draws only the rows a scrolled window
/// shows (`view::virt::row_window`) — the drawing side of virtualisation
/// has existed since the table learned to scroll. What it never had is a
/// MODEL that could take advantage of that window: `rows: &[Vec<String>]`
/// meant every caller had already built every row's every cell before
/// `table_surface` was ever called, whether forty of them reached a pixel
/// or forty thousand did. A `TableModel` that formats a row from an index
/// on demand — a process table read from `/proc`, a log tail read from a
/// ring buffer — pays only for the rows the window actually asks for.
pub trait TableModel {
    /// How many rows the model has. `usize::MAX` is not a valid answer
    /// for "I don't know" — a model that cannot count its rows cannot be
    /// scrolled, selected by key, or measured for a scrollbar's thumb,
    /// so it is not a table.
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// How many cells a row of this model carries — informational, and
    /// never consulted by the drawer to validate what [`TableModel::row`]
    /// actually wrote. `ui::table_surface` reads cells positionally
    /// through `Vec::get`, so a row that is shorter than `cols()` said
    /// simply leaves its missing columns blank instead of panicking; a
    /// caller riding a severity word past `columns.len()`
    /// (`TableStyle::severity_col`) answers `columns.len() + 1` here
    /// exactly as today's `Vec<Vec<String>>` rows already sometimes do.
    fn cols(&self) -> usize;

    /// Writes row `index`'s cells into `out`, reusing its capacity — the
    /// same discipline [`super::model::RowModel::row`] holds a list's
    /// buffer to, and for the same reason: a table redraws its visible
    /// window sixty times a second, and an allocation per cell per row
    /// per frame is the cost that discipline exists to avoid.
    ///
    /// Out of range: leave `out` empty rather than panicking. A view
    /// mid-resize, or mid-scroll past a data refresh that shrank the
    /// model, legitimately asks for a row that has just gone — the same
    /// contract `RowModel::row` keeps.
    fn row(&self, index: usize, out: &mut Vec<String>);

    /// The model's rewrite counter — [`super::model::RowModel::generation`]'s
    /// twin, down to the default: 0 means "no opinion", and a reader
    /// caching against it (the sort order, the content-width measure)
    /// rebuilds every time rather than risk showing stale cells for a
    /// model that cannot say when it changed. A caller with a real
    /// snapshot counter opts in to the caching by reporting it here.
    fn generation(&self) -> u64 {
        0
    }
}

/// Rows already in memory, addressed as a SLICE rather than an owned
/// `Vec` — what a caller holding a sub-range, or data that arrived across
/// the plugin ABI as borrowed memory, can implement the trait for without
/// copying into an owned `Vec` first.
///
/// [`TableModel`] is implemented for BOTH this and `Vec<Vec<String>>`
/// below on purpose, not by oversight: a bare `&[Vec<String>]`-typed
/// parameter (`ui::table`'s and `ui::table_view`'s own signature) infers
/// `M = [Vec<String>]` at their call into `table_surface`, while a
/// `&some_vec` where `some_vec: Vec<Vec<String>>` (`src/script.rs`'s
/// `rows` local, built fresh from a script's answer every frame) infers
/// `M = Vec<Vec<String>>` — two different concrete types for `M`, and
/// implementing only one would silently break source compatibility for
/// whichever call shape went uncovered.
impl TableModel for [Vec<String>] {
    fn len(&self) -> usize {
        <[Vec<String>]>::len(self)
    }

    fn cols(&self) -> usize {
        self.first().map(Vec::len).unwrap_or(0)
    }

    fn row(&self, index: usize, out: &mut Vec<String>) {
        out.clear();
        if let Some(r) = self.get(index) {
            out.extend(r.iter().cloned());
        }
    }
}

/// [`TableModel`] for the OWNED shape — `src/script.rs`'s `rows: Vec<
/// Vec<String>>`, built from a script's answer and passed as `&rows`.
/// Delegates to the slice impl above rather than repeating it, so the two
/// shapes cannot answer a row differently.
impl TableModel for Vec<Vec<String>> {
    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn cols(&self) -> usize {
        TableModel::cols(self.as_slice())
    }

    fn row(&self, index: usize, out: &mut Vec<String>) {
        TableModel::row(self.as_slice(), index, out)
    }
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<Vec<String>> {
        vec![
            vec!["1471".into(), "firefox".into()],
            vec!["22".into(), "nacelle-desktop".into()],
        ]
    }

    #[test]
    fn a_slice_and_its_owned_vec_answer_a_row_the_same_way() {
        let owned = sample();
        let mut a = Vec::new();
        let mut b = Vec::new();
        TableModel::row(&owned, 1, &mut a);
        TableModel::row(owned.as_slice(), 1, &mut b);
        assert_eq!(a, b);
        assert_eq!(a, vec!["22".to_string(), "nacelle-desktop".to_string()]);
        assert_eq!(TableModel::len(&owned), 2);
        assert_eq!(TableModel::len(owned.as_slice()), 2);
        assert_eq!(TableModel::cols(&owned), 2);
    }

    #[test]
    fn a_reused_buffer_keeps_nothing_of_the_row_before_it() {
        let rows = sample();
        let mut buf = vec!["stale".to_string(), "leftover".to_string(), "extra".to_string()];
        TableModel::row(&rows, 0, &mut buf);
        assert_eq!(buf, vec!["1471".to_string(), "firefox".to_string()]);
    }

    #[test]
    fn a_row_past_the_end_is_empty_rather_than_a_panic() {
        let rows = sample();
        let mut buf = Vec::new();
        TableModel::row(&rows, 9, &mut buf);
        assert!(buf.is_empty());
        assert_eq!(TableModel::len(&rows), 2);
    }

    #[test]
    fn an_empty_model_answers_zero_columns_rather_than_guessing() {
        let empty: Vec<Vec<String>> = Vec::new();
        assert_eq!(TableModel::cols(&empty), 0);
        assert!(TableModel::is_empty(&empty));
    }
}
