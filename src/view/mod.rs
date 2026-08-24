//! Model/view core: the arithmetic and the state behind every container
//! that shows a MODEL rather than a fixed handful of things — tables,
//! lists, trees, tab strips.
//!
//! Nothing here draws. That is the point: the same window arithmetic,
//! the same scroll physics and the same hit list serve a script element
//! drawn through [`crate::Ctx`], an object drawn by the application, and
//! a plugin drawing through the ABI with its own copy of this crate.
//! The alternative is the one this project has already lived through:
//! `ui::fit_end_tracked` and the filesystem widget's `fit_name` doing
//! the same job in two places, and drifting.
//!
//! * [`surface`] — [`surface::Surface`]: the one wall between view logic
//!   and painting. One impl draws through the host's draw list, the
//!   other through the plugin ABI, and the view cannot tell which.
//! * [`model`] — [`model::RowModel`]: where a view's rows come from,
//!   pulled by index so a virtualised view materialises forty of four
//!   thousand.
//! * [`virt`] — [`virt::RowWindow`]: which rows a viewport shows.
//! * [`scroll`] — [`scroll::ScrollView`]: the offset, its physics and
//!   the scrollbar's geometry.
//! * [`hits`] — [`hits::Hits`]: rectangles recorded while drawing,
//!   tested when a click or a press arrives.
//! * [`paint`] — the drawing vocabulary the views share, written once
//!   against [`surface::Surface`].
//! * [`table`] — [`table::solve_widths`], the column-width solver lifted
//!   out of `ui::table` unchanged, and [`table::TableState`], what a
//!   table remembers between frames.
//! * [`table_model`] — [`table_model::TableModel`]: where a table's rows
//!   come from, `RowModel`'s twin for a row of N arbitrary columns
//!   instead of one label and one status.
//! * [`list`] — the virtualised row list: chip, label, status, bar.
//! * [`tree`] — [`tree::FlatTree`]: nested data flattened to a row list,
//!   so a tree is a MODEL and never a second view.
//!
//! Every value that decides how any of this LOOKS or FEELS arrives from
//! the caller, read from theme tokens once per frame
//! ([`scroll::ScrollPhysics`], [`scroll::ScrollbarLook`]). This module
//! holds no literal but the ones the clock and the pixel grid force on
//! it, and each of those says so where it stands.

pub mod hits;
pub mod list;
pub mod model;
pub mod paint;
pub mod scroll;
pub mod surface;
pub mod table;
pub mod table_model;
pub mod tree;
pub mod virt;

pub use hits::{Hit, Hits};
pub use list::ListState;
pub use model::{RowBuf, RowModel, Rows};
pub use scroll::{ScrollView, Snap};
pub use surface::{AbiSurface, CtxSurface, Surface};
pub use table::{Extent, SortDir, TableState};
pub use table_model::TableModel;
pub use tree::{FlatTree, TreeModel};
pub use virt::{row_window, RowWindow};
