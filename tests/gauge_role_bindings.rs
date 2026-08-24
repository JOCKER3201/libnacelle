//! A gauge row's two halves are two roles, and both are the THEME's to
//! name.
//!
//! `gauge_grid` and `gauge_rows` read `type.caption.size`, `.leading` and
//! `.tracking` by NAME, for the key half AND the value half alike. Three
//! things followed from that one shortcut:
//!
//! * `C0` and `12%` were drawn at one size — the only instrument row in
//!   the program whose label and reading are the same height. Every other
//!   one (`script.meter_*`, `columns.*`) puts the label on `caption` and
//!   the reading on `value`, which at 1080 lines is 9.6 px against
//!   17.6 px.
//! * `gauge.label_role` was declared by the master and read by nothing, so
//!   a theme could move it and no pixel followed.
//! * There was no `gauge.value_role` at all: the one half a theme most
//!   wants to retune had no key.
//!
//! Each stage below moves ONE of the two bindings and requires the drawing
//! to move with it — and one stage goes the other way, restating a role
//! under a different name so that the picture must come out identical.
//! That is what separates "the code noticed a token change" from "the code
//! reads the ladder of whichever role the binding lands on".
//!
//! ONE test function, on purpose: the resolved theme is process-wide, so a
//! test that switches it must not run beside a test that reads it — the
//! same reason tests/role_bindings_chrome.rs gives.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::theme::{self, LoadRequest};
use nacelle::ui::{self, GaugeKind, GaugeLabels, GaugeStyle, GaugeValueFmt};
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;

/// The block the stages draw: four readings wide enough apart that a
/// figure box and a proportional run cannot measure the same, laid out in
/// one column so the label, the track and the value share a row.
const VALUES: [f32; 4] = [11.0, 88.0, 4.0, 100.0];

/// Runs one question on a thread of its own.
///
/// The toolkit memoises a resolved role per thread and the WORD an enum
/// token stands at per (token, index); a reload renumbers the open word
/// set a binding lives in, so asking twice on one thread answers the FIRST
/// fixture's role for the second fixture's question.
fn fresh<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| s.spawn(f).join().expect("the drawing thread panicked"))
}

/// Every command one gauge block puts on screen, as readable lines.
fn gauges(kind: GaugeKind) -> Vec<String> {
    fresh(move || {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::recording();
        {
            let mut c = Ctx {
                access: None,
                dl: &mut dl,
                fonts: &mut fonts,
                w: W,
                h: H,
                t: 0.0,
                mouse: Pointer::new(-1.0, -1.0),
                term_font_scale: 1.0,
                ui_font_scale: 1.0,
                panel_scale: 1.0,
                focus: None,
                tips: None,
            };
            let st = GaugeStyle {
                cols: 1,
                kind,
                labels: GaugeLabels::Index("C".to_string()),
                value_fmt: GaugeValueFmt::Percent,
                shrink: 1.0,
            };
            ui::gauge_grid(&mut c, Rect::new(40.0, 40.0, 400.0, 320.0), &VALUES, &st);
        }
        dl.cmds().iter().map(|c| c.to_string()).collect()
    })
}

/// The px and the string of every text command in a block, in the order
/// they were drawn.
///
/// The string is here because a reading is TWO runs since 2026-08-17: the
/// number, and the unit that follows it in `num.unit.*`'s own size. A
/// test that counted runs and stepped through them by twos was reading
/// the unit of one row as the label of the next.
fn text_runs(kind: GaugeKind) -> Vec<(f32, String)> {
    fresh(move || {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::recording();
        {
            let mut c = Ctx {
                access: None,
                dl: &mut dl,
                fonts: &mut fonts,
                w: W,
                h: H,
                t: 0.0,
                mouse: Pointer::new(-1.0, -1.0),
                term_font_scale: 1.0,
                ui_font_scale: 1.0,
                panel_scale: 1.0,
                focus: None,
                tips: None,
            };
            let st = GaugeStyle {
                cols: 1,
                kind,
                labels: GaugeLabels::Index("C".to_string()),
                value_fmt: GaugeValueFmt::Percent,
                shrink: 1.0,
            };
            ui::gauge_grid(&mut c, Rect::new(40.0, 40.0, 400.0, 320.0), &VALUES, &st);
        }
        dl.cmds()
            .iter()
            .filter_map(|c| match c {
                nacelle::draw::DrawCmd::Text { px, text, .. } => Some((*px, text.clone())),
                _ => None,
            })
            .collect()
    })
}

/// The three kinds of run a gauge block puts on screen, told apart by
/// what they SAY rather than by where they fall in the list: the keys are
/// `C0`..`C3`, the unit is the percent sign, and everything else is a
/// reading.
fn split(runs: &[(f32, String)]) -> (Vec<f32>, Vec<f32>, Vec<f32>) {
    let of = |f: &dyn Fn(&str) -> bool| -> Vec<f32> {
        runs.iter().filter(|(_, t)| f(t)).map(|(px, _)| *px).collect()
    };
    (
        of(&|t: &str| t.starts_with('C')),
        of(&|t: &str| t.contains('%')),
        of(&|t: &str| !t.starts_with('C') && !t.contains('%')),
    )
}

/// The px a role resolves to under whatever theme is loaded.
fn role_px(name: &str) -> f32 {
    let owned = name.to_string();
    fresh(move || {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::new();
        let c = Ctx {
            access: None,
            dl: &mut dl,
            fonts: &mut fonts,
            w: W,
            h: H,
            t: 0.0,
            mouse: Pointer::new(0.0, 0.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        };
        ui::role(&owned).px(&c, 1.0)
    })
}

/// Loads the master, or a theme based on it that rewrites a few keys.
fn apply(fixture: Option<&str>) {
    match fixture {
        None => {
            let _ = theme::load();
        }
        Some(text) => {
            let path = std::env::temp_dir()
                .join(format!("nacelle-gauge-binding-{}.theme", std::process::id()));
            std::fs::write(&path, text).expect("the fixture theme must be writable");
            let _ = theme::load_with(LoadRequest { path: Some(path), ..Default::default() });
        }
    }
}

/// Every difference between two screens. Empty means the same screen.
fn diff(a: &[String], b: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if a.len() != b.len() {
        out.push(format!("command count {} vs {}", a.len(), b.len()));
    }
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y {
            out.push(format!("command {i}:\n  {x}\n  {y}"));
        }
        if out.len() > 4 {
            out.push("...".to_string());
            break;
        }
    }
    out
}

const HEAD: &str = "[meta]\nschema = 1\nname = \"Gauge binding fixture\"\nbase = \"default\"\n\n";

/// The two bindings, each moved on its own so that a difference can only
/// be that one binding's doing. `badge` and `display.hero` are shipped
/// roles that differ from `caption` and `value` in size, tracking, case
/// and leading at once.
const LABEL_ROLE: &str = "[gauge]\nlabel_role = badge\n";
const VALUE_ROLE: &str = "[gauge]\nvalue_role = display.hero\n";

/// `value` restated on `spare0`, key for key, with the binding moved to
/// it. The role the binding names is a DIFFERENT role and the ladder
/// underneath is the same, so the picture must come out identical — a
/// reader that follows the binding passes this, and one that memorised
/// "value" does not.
///
/// `spare0` and not an invented name: the schema is closed (a key no
/// `type.*` block declares is dropped with a warning), and the master
/// ships four spare roles for exactly this kind of question.
const RENAMED: &str = "[gauge]\nvalue_role = spare0\n\n\
     [type]\n\
     spare0.face = @type.value.face\n\
     spare0.size = @type.value.size\n\
     spare0.min_px = @type.value.min_px\n\
     spare0.max_px = @type.value.max_px\n\
     spare0.tracking = @type.value.tracking\n\
     spare0.case = @type.value.case\n\
     spare0.smallcaps_ratio = @type.value.smallcaps_ratio\n\
     spare0.leading = @type.value.leading\n\
     spare0.tabular = @type.value.tabular\n\
     spare0.fg = @type.value.fg\n\
     spare0.alpha = @type.value.alpha\n\
     spare0.synthetic_bold = @type.value.synthetic_bold\n";

#[test]
fn a_gauge_reads_the_two_roles_the_theme_binds_it_to() {
    // ---- the master's own picture, and the sizes in it ----------------
    apply(None);
    let master_rows = gauges(GaugeKind::Row);
    let master_cells = gauges(GaugeKind::Cell);
    let caption = role_px("caption");
    let value = role_px("value");

    // The defect this file exists for, stated as the number it was: the
    // key half and the reading half must NOT be one size, and the reading
    // is the one the rest of the master calls `value`.
    let rows = text_runs(GaugeKind::Row);
    let (labels, units, readings) = split(&rows);
    assert_eq!(
        rows.len(),
        VALUES.len() * 3,
        "a row is a label, a reading and the reading's unit"
    );
    assert_eq!(units.len(), VALUES.len(), "every reading carries its unit run");
    assert!(
        labels.iter().all(|p| (*p - caption).abs() < 0.01),
        "a gauge's key half is not `gauge.label_role`'s size: {labels:?} vs {caption}"
    );
    assert!(
        readings.iter().all(|p| (*p - value).abs() < 0.01),
        "a gauge's reading is not `gauge.value_role`'s size: {readings:?} vs {value}"
    );
    assert!(
        (value - caption).abs() > 1.0,
        "the two roles the master binds a gauge to resolve to one size, so this \
         file cannot tell a gauge that reads its bindings from one that does not"
    );

    // The cell form draws the reading and no key, and in the same role.
    let (keys, cell_units, cells) = split(&text_runs(GaugeKind::Cell));
    assert!(keys.is_empty(), "the cell form draws no key half");
    assert_eq!(cells.len(), VALUES.len(), "the cell form draws one reading per gauge");
    assert_eq!(cell_units.len(), VALUES.len(), "and one unit run with it");
    assert!(
        cells.iter().all(|p| (*p - value).abs() < 0.01),
        "the cell form's reading is not `gauge.value_role`'s size: {cells:?} vs {value}"
    );
    // The unit is set from the READING's px through `num.unit.scale`, so
    // moving `gauge.value_role` has to move it too — which is what makes
    // the unit a run of the reading and not a run of its own.
    assert!(
        cell_units.iter().all(|p| *p < value && *p > 0.0),
        "the unit run does not follow `num.unit.scale` off the reading: \
         {cell_units:?} vs {value}"
    );

    // ---- moving one binding moves that half and only that half --------
    apply(Some(&format!("{HEAD}{LABEL_ROLE}")));
    let moved = gauges(GaugeKind::Row);
    assert!(
        !diff(&master_rows, &moved).is_empty(),
        "gauge.label_role moved and the key half did not: the role is still \
         spelled out at the call site"
    );
    let (_, _, readings_now) = split(&text_runs(GaugeKind::Row));
    assert_eq!(
        readings_now, readings,
        "moving the KEY's binding moved the READING too — the two halves are \
         still sharing one resolved size"
    );

    apply(Some(&format!("{HEAD}{VALUE_ROLE}")));
    let moved = gauges(GaugeKind::Row);
    assert!(
        !diff(&master_rows, &moved).is_empty(),
        "gauge.value_role moved and the reading did not"
    );
    let moved = gauges(GaugeKind::Cell);
    assert!(
        !diff(&master_cells, &moved).is_empty(),
        "gauge.value_role moved and the cell form's reading did not"
    );

    // ---- and a role RESTATED under another name draws the same picture
    apply(Some(&format!("{HEAD}{RENAMED}")));
    let renamed = gauges(GaugeKind::Row);
    let d = diff(&master_rows, &renamed);
    assert!(
        d.is_empty(),
        "`value` restated key for key under another name drew a different gauge, \
         so the reading is not following the binding to whatever role it names:\n{}",
        d.join("\n")
    );

    apply(None);
}
