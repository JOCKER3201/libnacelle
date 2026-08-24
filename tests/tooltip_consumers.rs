//! The tooltip's CONSUMERS — the half of F2 §8.1 that makes the manager
//! more than a module.
//!
//! `tests/tooltip_view.rs` proves the box is placed, sized and delayed
//! correctly when something asks for it. This one proves something asks:
//! that an interactive table's trimmed heading and trimmed cell, a list
//! or tree row's trimmed name, a tab strip's trimmed label and a panel
//! band's trimmed path file the request themselves while they draw — so
//! that resting the pointer on text the ellipsis cut short really does
//! put the whole of it on screen.
//!
//! Everything runs through the real master theme and the real fonts,
//! because "was this trimmed?" is a question only a real measure can
//! answer.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::object::tooltip::Tooltips;
use nacelle::object::{panel, segmented, tabs};
use nacelle::pointer::Pointer;
use nacelle::theme;
use nacelle::ui::{self, Align, CellKind, ColWidth, Column, TableStyle, TableView};
use nacelle::view::list::{ListState, ListStyle, ListView};
use nacelle::view::tree::{MemNode, MemTree};
use nacelle::view::{CtxSurface, FlatTree, Hit, Hits, RowBuf, Rows, TableState};
use nacelle::widget::Chrome;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;

/// A name no narrow column can show in full.
const LONG: &str = "a-very-long-process-name-that-no-column-will-ever-show-in-full";

/// One frame: the caller draws, the manager answers at the end of it —
/// exactly the order the desktop keeps. Gives back whatever the drawing
/// returned and the text that reached the screen, if any.
fn frame<R, F>(
    tips: &mut Tooltips,
    fonts: &mut FontSystem,
    mouse: (f32, f32),
    t: f64,
    body: F,
) -> (R, Option<String>)
where
    F: FnOnce(&mut Ctx) -> R,
{
    let mut dl = DrawList::new();
    let mut ctx = Ctx {
        access: None,
        dl: &mut dl,
        fonts,
        w: W,
        h: H,
        t,
        mouse: Pointer::new(mouse.0, mouse.1),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: Some(tips),
    };
    let out = body(&mut ctx);
    // Taken out before it is drawn, as the desktop does: the manager
    // cannot be lent to the frame and draw into it at the same time.
    let m = ctx.tips.take().expect("the manager was lent to this frame");
    m.draw(&mut ctx);
    (out, m.shown().map(|s| s.to_string()))
}

fn columns() -> Vec<Column> {
    vec![
        Column {
            title: "PROCESS IDENTIFIER".into(),
            align: Align::Right,
            kind: CellKind::Text,
            width: ColWidth::Content,
        },
        Column {
            title: "NAME".into(),
            align: Align::Left,
            kind: CellKind::Text,
            width: ColWidth::Content,
        },
    ]
}

fn rows() -> Vec<Vec<String>> {
    vec![
        vec!["1471".into(), LONG.into()],
        vec!["7".into(), "sh".into()],
    ]
}

fn style() -> TableStyle {
    TableStyle { elastic: 1, zebra: false, severity_col: None, shrink: 1.0 }
}

/// Draws the table into `r` with the view options the shipped process
/// widget uses, and records where everything landed.
fn table(
    ctx: &mut Ctx,
    r: Rect,
    state: &mut TableState,
    hits: &mut Hits,
    explain: bool,
) {
    hits.clear();
    ui::table_view(
        ctx,
        r,
        &columns(),
        &rows(),
        &style(),
        TableView {
            state,
            hits,
            id: 0,
            generation: 1,
            interactive: true,
            select: true,
            key_col: Some(0),
            scroll: false,
            tooltip: explain,
        },
    );
}

// ---- the table -------------------------------------------------------

#[test]
fn a_cell_the_ellipsis_cut_short_says_the_whole_of_itself() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let mut state = TableState::new();
    let mut hits = Hits::new();
    let r = Rect::new(40.0, 60.0, 300.0, 400.0);

    // A frame with the pointer nowhere near it, to learn where the rows
    // landed — the same thing a click does between frames.
    frame(&mut tips, &mut fonts, (0.0, 0.0), 0.0, |ctx| {
        table(ctx, r, &mut state, &mut hits, true);
    });
    let row = hits
        .rect_of(&Hit::Row { id: 0, key: "1471".into() })
        .expect("the table records a rectangle for every row it drew");
    // The elastic column is the last one, so the table's right edge is
    // inside it whatever the first column measured.
    let at = (r.right() - 10.0, row.y + row.h / 2.0);

    // Resting starts the clock and shows nothing.
    let (_, now) = frame(&mut tips, &mut fonts, at, 0.0, |ctx| {
        table(ctx, r, &mut state, &mut hits, true);
    });
    assert_eq!(now, None, "a tooltip before the delay is a tooltip in the way");

    // A second later, the whole name.
    let (_, now) = frame(&mut tips, &mut fonts, at, 1.0, |ctx| {
        table(ctx, r, &mut state, &mut hits, true);
    });
    assert_eq!(now.as_deref(), Some(LONG));
}

#[test]
fn a_cell_that_fits_explains_nothing() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let mut state = TableState::new();
    let mut hits = Hits::new();
    // Room for the whole name: nothing was trimmed, so there is nothing
    // to add, and a tooltip repeating what is on screen is noise.
    let r = Rect::new(40.0, 60.0, 1600.0, 400.0);

    frame(&mut tips, &mut fonts, (0.0, 0.0), 0.0, |ctx| {
        table(ctx, r, &mut state, &mut hits, true);
    });
    let row = hits
        .rect_of(&Hit::Row { id: 0, key: "1471".into() })
        .expect("the table records a rectangle for every row it drew");
    let at = (r.right() - 10.0, row.y + row.h / 2.0);

    for t in [0.0, 1.0, 2.0] {
        let (_, now) = frame(&mut tips, &mut fonts, at, t, |ctx| {
            table(ctx, r, &mut state, &mut hits, true);
        });
        assert_eq!(now, None, "an untrimmed cell has nothing to say");
    }
}

#[test]
fn a_table_that_was_not_asked_to_explain_itself_stays_quiet() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let mut state = TableState::new();
    let mut hits = Hits::new();
    let r = Rect::new(40.0, 60.0, 300.0, 400.0);

    frame(&mut tips, &mut fonts, (0.0, 0.0), 0.0, |ctx| {
        table(ctx, r, &mut state, &mut hits, false);
    });
    let row = hits
        .rect_of(&Hit::Row { id: 0, key: "1471".into() })
        .expect("the table records a rectangle for every row it drew");
    let at = (r.right() - 10.0, row.y + row.h / 2.0);

    for t in [0.0, 1.0, 2.0] {
        let (_, now) = frame(&mut tips, &mut fonts, at, t, |ctx| {
            table(ctx, r, &mut state, &mut hits, false);
        });
        assert_eq!(now, None, "`tooltip` is opt-in, like every other view option");
    }
}

#[test]
fn a_heading_squeezed_by_a_dragged_width_says_what_it_is() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let mut state = TableState::new();
    let mut hits = Hits::new();
    let r = Rect::new(40.0, 60.0, 600.0, 400.0);
    // The user dragged the first column down to a sliver: its heading no
    // longer fits, which is the one case where a heading needs saying.
    state.set_width(0, Some(30.0));

    frame(&mut tips, &mut fonts, (0.0, 0.0), 0.0, |ctx| {
        table(ctx, r, &mut state, &mut hits, true);
    });
    let head = hits
        .rect_of(&Hit::TableHead { id: 0, col: 0 })
        .expect("an interactive table records a rectangle for every heading");
    let at = (head.x + 2.0, head.y + head.h / 2.0);

    let (_, now) = frame(&mut tips, &mut fonts, at, 0.0, |ctx| {
        table(ctx, r, &mut state, &mut hits, true);
    });
    assert_eq!(now, None);
    let (_, now) = frame(&mut tips, &mut fonts, at, 1.0, |ctx| {
        table(ctx, r, &mut state, &mut hits, true);
    });
    assert_eq!(now.as_deref(), Some("PROCESS IDENTIFIER"));
}

// ---- the list and the tree -------------------------------------------

/// Draws a one-row list into `r` and records where the row landed.
fn list(
    ctx: &mut Ctx,
    r: Rect,
    state: &mut ListState,
    hits: &mut Hits,
    explain: bool,
) {
    hits.clear();
    let mut row = RowBuf::new();
    row.key = "nm".into();
    row.label = LONG.into();
    let model = Rows::new(vec![row]);
    nacelle::view::list::list(
        &mut CtxSurface::new(ctx),
        r,
        &model,
        &ListStyle::default(),
        Some(ListView {
            state,
            hits,
            id: 0,
            select: true,
            scroll: false,
            tree: false,
            tooltip: explain,
        }),
    );
}

#[test]
fn a_row_name_the_ellipsis_cut_short_says_the_whole_of_itself() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let mut state = ListState::new();
    let mut hits = Hits::new();
    let r = Rect::new(40.0, 60.0, 240.0, 400.0);

    frame(&mut tips, &mut fonts, (0.0, 0.0), 0.0, |ctx| {
        list(ctx, r, &mut state, &mut hits, true);
    });
    let row = hits
        .rect_of(&Hit::Row { id: 0, key: "nm".into() })
        .expect("the list records a rectangle for every row it drew");
    // Inside the label's own run, which starts after `list.pad_x`.
    let at = (row.x + row.w / 2.0, row.y + row.h / 2.0);

    let (_, now) = frame(&mut tips, &mut fonts, at, 0.0, |ctx| {
        list(ctx, r, &mut state, &mut hits, true);
    });
    assert_eq!(now, None, "a tooltip before the delay is a tooltip in the way");
    let (_, now) = frame(&mut tips, &mut fonts, at, 1.0, |ctx| {
        list(ctx, r, &mut state, &mut hits, true);
    });
    assert_eq!(now.as_deref(), Some(LONG));
}

#[test]
fn a_list_that_was_not_asked_to_explain_itself_stays_quiet() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let mut state = ListState::new();
    let mut hits = Hits::new();
    let r = Rect::new(40.0, 60.0, 240.0, 400.0);

    frame(&mut tips, &mut fonts, (0.0, 0.0), 0.0, |ctx| {
        list(ctx, r, &mut state, &mut hits, false);
    });
    let row = hits
        .rect_of(&Hit::Row { id: 0, key: "nm".into() })
        .expect("the list records a rectangle for every row it drew");
    let at = (row.x + row.w / 2.0, row.y + row.h / 2.0);

    for t in [0.0, 1.0, 2.0] {
        let (_, now) = frame(&mut tips, &mut fonts, at, t, |ctx| {
            list(ctx, r, &mut state, &mut hits, false);
        });
        assert_eq!(now, None, "`tooltip` is opt-in, like every other view option");
    }
}

#[test]
fn a_tree_row_narrowed_by_its_indent_says_the_whole_of_itself() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let mut state = ListState::new();
    let mut hits = Hits::new();
    let r = Rect::new(40.0, 60.0, 240.0, 400.0);

    // A child of an open root: its label starts one indent and one
    // expander in, which is exactly what a tree trims that a list does
    // not.
    let mut flat = FlatTree::new(MemTree::new(vec![MemNode::leaf("usr")
        .with_children(vec![MemNode::leaf(LONG)])]));
    flat.sync();
    flat.expand("usr");
    flat.sync();

    let draw = |ctx: &mut Ctx, hits: &mut Hits, state: &mut ListState| {
        hits.clear();
        nacelle::view::list::list(
            &mut CtxSurface::new(ctx),
            r,
            &flat,
            &ListStyle::default(),
            Some(ListView {
                state,
                hits,
                id: 0,
                select: true,
                scroll: false,
                tree: true,
                tooltip: true,
            }),
        );
    };

    frame(&mut tips, &mut fonts, (0.0, 0.0), 0.0, |ctx| {
        draw(ctx, &mut hits, &mut state)
    });
    let row = hits
        .rect_of(&Hit::Row { id: 0, key: format!("usr/{LONG}") })
        .or_else(|| hits.rect_of(&Hit::Row { id: 0, key: LONG.into() }))
        .expect("the flattened tree records a rectangle for the child row");
    let at = (row.right() - 20.0, row.y + row.h / 2.0);

    let (_, now) = frame(&mut tips, &mut fonts, at, 0.0, |ctx| {
        draw(ctx, &mut hits, &mut state)
    });
    assert_eq!(now, None);
    let (_, now) = frame(&mut tips, &mut fonts, at, 1.0, |ctx| {
        draw(ctx, &mut hits, &mut state)
    });
    assert_eq!(now.as_deref(), Some(LONG));
}

// ---- the `rows` block and the `columns` strip ------------------------

/// A reading far too long for the room a label leaves beside it.
const READING: &str = "1471 of 4096 kilobytes, sampled every four seconds";

#[test]
fn a_reading_the_ellipsis_cut_short_says_the_whole_of_itself() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    // Narrow: the label is drawn whole and the value is what runs out of
    // room, which is the only case a `rows` block has to explain.
    let r = Rect::new(40.0, 60.0, 200.0, 120.0);
    let rows = vec![ui::RowItem { label: "MEMORY".into(), value: READING.into(), sev: None }];
    let st = || ui::RowsStyle {
        label_role: ui::role("label"),
        value_role: ui::role("value"),
        columns: 1,
        label_width: ui::LabelWidth::Max,
        row_h: 24.0,
        shrink: 1.0,
    };
    // The block is centred vertically in its box and one line tall.
    let at = (r.right() - 10.0, r.y + r.h / 2.0);

    let (_, now) = frame(&mut tips, &mut fonts, at, 0.0, |ctx| {
        ui::rows_label_value(ctx, r, &rows, &st());
    });
    assert_eq!(now, None);
    let (_, now) = frame(&mut tips, &mut fonts, at, 1.0, |ctx| {
        ui::rows_label_value(ctx, r, &rows, &st());
    });
    assert_eq!(now.as_deref(), Some(READING));
}

#[test]
fn a_reading_with_room_beside_its_label_stays_quiet() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let r = Rect::new(40.0, 60.0, 1400.0, 120.0);
    let rows = vec![ui::RowItem { label: "MEMORY".into(), value: "41%".into(), sev: None }];
    let st = || ui::RowsStyle {
        label_role: ui::role("label"),
        value_role: ui::role("value"),
        columns: 1,
        label_width: ui::LabelWidth::Max,
        row_h: 24.0,
        shrink: 1.0,
    };
    let at = (r.x + 300.0, r.y + r.h / 2.0);

    for t in [0.0, 1.0, 2.0] {
        let (_, now) = frame(&mut tips, &mut fonts, at, t, |ctx| {
            ui::rows_label_value(ctx, r, &rows, &st());
        });
        assert_eq!(now, None, "a reading that fits is already saying everything");
    }
}

#[test]
fn a_strip_cell_squeezed_by_its_neighbours_says_the_whole_reading() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    // Three cells sharing a box far too narrow for the first one's
    // value: the strip is content-measured, so this is the case where
    // `fit_end` actually reaches a cell.
    let r = Rect::new(40.0, 60.0, 180.0, 80.0);
    let cells = vec![
        ui::ColumnCell { label: "UPTIME".into(), value: READING.into(), sev: None },
        ui::ColumnCell { label: "LOAD".into(), value: "0.42".into(), sev: None },
        ui::ColumnCell { label: "TASKS".into(), value: "311".into(), sev: None },
    ];
    let st = || ui::ColumnsStyle {
        label_role: ui::role("label"),
        value_role: ui::role("value"),
        align: Some(Align::Center),
        dividers: false,
        shrink: 1.0,
    };
    // Inside the first cell, which is the one that was cut.
    let at = (r.x + 10.0, r.y + r.h / 2.0);

    let (_, now) = frame(&mut tips, &mut fonts, at, 0.0, |ctx| {
        ui::columns(ctx, r, &cells, &st());
    });
    assert_eq!(now, None);
    let (_, now) = frame(&mut tips, &mut fonts, at, 1.0, |ctx| {
        ui::columns(ctx, r, &cells, &st());
    });
    assert_eq!(now.as_deref(), Some(READING));
}

// ---- the panel band --------------------------------------------------

/// A path no narrow band can show whole. The tail is what survives the
/// trim, so the root is precisely what the pointer has to ask for.
const PATH: &str = "/home/michael/Documents/Archive/2024/invoices/quarter-four";

fn px(name: &str) -> f32 {
    theme::resolved().px(theme::id(name).unwrap())
}

/// The right end of a titled panel's band — where the trimmed text is,
/// and nowhere near the title. The band sits `panel.title.block_h` above
/// the content box and spans the same width, which is the whole of what
/// the placement guarantees a caller.
fn band_right(r: Rect) -> (f32, f32) {
    let content = panel::content_box(r, true);
    let block = px("panel.title.block_h");
    let band_h = px("panel.title.band_h").min(block);
    (
        content.right() - px("panel.title.inset_x") - 2.0,
        content.y - block + band_h / 2.0,
    )
}

#[test]
fn a_band_whose_path_lost_its_root_gives_the_path_back() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let r = Rect::new(40.0, 60.0, 320.0, 400.0);
    let chrome = Chrome {
        title: Some("FILESYSTEM".into()),
        right: Some(PATH.into()),
        ..Chrome::default()
    };

    let at = band_right(r);
    let (_, now) = frame(&mut tips, &mut fonts, at, 0.0, |ctx| {
        panel::draw(ctx, r, &chrome, 0);
    });
    assert_eq!(now, None);
    let (_, now) = frame(&mut tips, &mut fonts, at, 1.0, |ctx| {
        panel::draw(ctx, r, &chrome, 0);
    });
    // The band upper-cases through `type.title.panel.case`, so what is
    // offered is what would have been DRAWN, not the raw string.
    assert_eq!(now.as_deref(), Some(PATH.to_uppercase().as_str()));
}

#[test]
fn a_band_with_room_for_its_path_stays_quiet() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let r = Rect::new(40.0, 60.0, 1600.0, 400.0);
    let chrome = Chrome {
        title: Some("FILESYSTEM".into()),
        right: Some("/tmp".into()),
        ..Chrome::default()
    };

    let at = band_right(r);
    for t in [0.0, 1.0, 2.0] {
        let (_, now) = frame(&mut tips, &mut fonts, at, t, |ctx| {
            panel::draw(ctx, r, &chrome, 0);
        });
        assert_eq!(now, None, "a path that fits is already saying everything");
    }
}

// ---- the tab strip ---------------------------------------------------

#[test]
fn a_tab_too_narrow_for_its_page_gives_the_name_in_full() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let st = tabs::StripState::new(0);
    let labels = ["TELEMETRY AND DIAGNOSTICS", "SHELL"];
    // Narrow enough that the solver floors both plates and the first
    // label is cut short.
    let r = Rect::new(0.0, 0.0, 160.0, 120.0);

    let (cells, _) = frame(&mut tips, &mut fonts, (0.0, 0.0), 0.0, |ctx| {
        tabs::draw(ctx, r, &labels, &st)
    });
    let cell = cells[0];
    let at = (cell.x + cell.w / 2.0, cell.y + cell.h / 2.0);

    let (_, now) = frame(&mut tips, &mut fonts, at, 0.0, |ctx| {
        tabs::draw(ctx, r, &labels, &st)
    });
    assert_eq!(now, None);
    let (_, now) = frame(&mut tips, &mut fonts, at, 1.0, |ctx| {
        tabs::draw(ctx, r, &labels, &st)
    });
    assert_eq!(now.as_deref(), Some(labels[0]));
}

#[test]
fn a_tab_with_room_for_its_label_stays_quiet() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let st = tabs::StripState::new(0);
    let labels = ["ONE", "TWO"];
    let r = Rect::new(0.0, 0.0, 900.0, 120.0);

    let (cells, _) = frame(&mut tips, &mut fonts, (0.0, 0.0), 0.0, |ctx| {
        tabs::draw(ctx, r, &labels, &st)
    });
    let cell = cells[0];
    let at = (cell.x + cell.w / 2.0, cell.y + cell.h / 2.0);

    for t in [0.0, 1.0, 2.0] {
        let (_, now) = frame(&mut tips, &mut fonts, at, t, |ctx| {
            tabs::draw(ctx, r, &labels, &st)
        });
        assert_eq!(now, None, "a label that fits is already saying everything");
    }
}

// ---- the segmented control -------------------------------------------

#[test]
fn a_segment_too_narrow_for_its_choice_gives_the_word_in_full() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let st = segmented::StripState::new(0);
    let labels = ["EVERYTHING AT ONCE", "SOME", "NONE"];
    let r = Rect::new(0.0, 0.0, 150.0, 80.0);

    let (cells, _) = frame(&mut tips, &mut fonts, (0.0, 0.0), 0.0, |ctx| {
        segmented::draw(ctx, r, &labels, &st)
    });
    let cell = cells[0];
    let at = (cell.x + cell.w / 2.0, cell.y + cell.h / 2.0);

    let (_, now) = frame(&mut tips, &mut fonts, at, 0.0, |ctx| {
        segmented::draw(ctx, r, &labels, &st)
    });
    assert_eq!(now, None);
    let (_, now) = frame(&mut tips, &mut fonts, at, 1.0, |ctx| {
        segmented::draw(ctx, r, &labels, &st)
    });
    assert_eq!(now.as_deref(), Some(labels[0]));
}
