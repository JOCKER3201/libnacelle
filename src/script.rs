//! Widgets written as scripts.
//!
//! A widget is a directory holding a `<name>.rhai` script and whatever
//! assets it needs. The script defines one function:
//!
//! ```text
//! fn draw() {
//!     [
//!         title("UPTIME"),
//!         rows([
//!             ["UP",   uptime(host.uptime)],
//!             ["HOST", upper(host.name)],
//!         ]),
//!     ]
//! }
//! ```
//!
//! It RETURNS a list of elements rather than drawing them. That is what
//! keeps this fast enough to run every frame: one call per widget, no
//! crossing back and forth for each primitive. It is also what keeps the
//! interface stable — the elements are a small vocabulary that can grow
//! without invalidating scripts, unlike a binary interface where moving
//! a field breaks every widget silently.
//!
//! Scripts are sandboxed by what they are given: `host`, `view`, and the
//! element and formatting functions, and nothing else. A widget cannot
//! read a file, open a socket or run a program, because no such function
//! exists in its world.
//!
//! A script does not choose type, either. The ELEMENT KIND chooses the
//! role, the role carries the size, the face and the weight, and every
//! one of those bindings is in the master — `script.rows_value_role` and
//! its neighbours. A call may say WHAT a string is, never how big it is:
//!
//! ```text
//! rows([["IPV4", "10.0.0.4"]])                   // a row is a row
//! runs([#{ t: "LOAD", role: "label" },           // a label
//!       #{ t: "0.42", role: "reading" }], "left")// a measured quantity
//! ```
//!
//! `role` there is a KIND from the master's closed `script.kind_*_role`
//! vocabulary, not a `type.*` role. It used to be the latter, and that is
//! how four panels of one kind came to show their value at three
//! different sizes: `rows(…, #{ value_role: "data" })` was the only way
//! to ask for tabular figures before `type.<role>.tabular` was
//! implemented, and it bought them at 1.87u against the 3.25u every other
//! panel used — the value role is 74% taller than the one it settled
//! for, and no theme file could undo the difference.
//!
//! `view` is the second constant in that scope and the answer to a
//! question the sandbox raises: if a script is a pure function of its
//! data, where does the sort the user clicked live? Not in the script —
//! in [`ViewState`], beside the widget, keyed by the `id` an interactive
//! element names. The script writes the OPENING arrangement as options
//! and reads what the user did to it back through `view`:
//!
//! ```text
//! fn draw() {
//!     let sel = view.procs.selected;      // () until a row is picked
//!     [ table(headings, rows, 1, #{ id: "procs", interactive: true,
//!                                   select: "row", key: 0, scroll: true }) ]
//! }
//! ```

use crate::ui::{self, Align};
use crate::view::{self, Hit};
use crate::widget::{Action, DragPhase, Sizing};
use crate::{Host, Widget};
use crate::telemetry::{fmt_bytes, fmt_rate, fmt_uptime};
use crate::theme::{self, Color, TokenId};
use crate::{Ctx, Rect};
use rhai::{Array, Dynamic, Engine, Map, Scope, AST};
use std::cell::RefCell;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::OnceLock;

/// Token id resolved once by name; MISSING degrades through the engine's
/// per-kind fallback rather than panicking.
fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// A colour token, delivered in the `Color` the draw calls take.
fn col(cell: &'static OnceLock<TokenId>, name: &'static str) -> Color {
    let c = theme::resolved().color(tok(cell, name));
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// Compiled widget script.
pub struct Script {
    engine: Engine,
    ast: AST,
    /// Reported once: a script that keeps failing must not flood the
    /// terminal with the same message sixty times a second.
    failed: bool,
}

/// Builds the engine every script runs in. Everything a script can do is
/// registered here; there is no other way in.
fn engine() -> Engine {
    let mut engine = Engine::new();
    // A runaway script must not take the frame with it.
    engine.set_max_operations(200_000);
    engine.set_max_expr_depths(64, 64);
    engine.set_max_string_size(10_000);
    engine.set_max_array_size(4_000);

    // --- elements -------------------------------------------------
    let el = |kind: &str| {
        let mut m = Map::new();
        m.insert("kind".into(), kind.into());
        m
    };

    // The header is the script author's choice, piece by piece:
    //   title("CPU")               — text with the underline
    //   title("CPU", false)        — text alone
    //   title("", true) / title("") — the underline alone
    //   (no title element)         — neither
    fn title_map(left: &str, right: &str, line: bool) -> Map {
        let mut m = Map::new();
        m.insert("kind".into(), "title".into());
        m.insert("left".into(), left.into());
        m.insert("right".into(), right.into());
        m.insert("line".into(), line.into());
        m
    }
    engine.register_fn("title", move |text: &str| title_map(text, "", true));
    engine.register_fn("title", move |text: &str, line: bool| title_map(text, "", line));
    engine.register_fn("title", move |left: &str, right: &str| title_map(left, right, true));
    engine.register_fn("title", move |left: &str, right: &str, line: bool| {
        title_map(left, right, line)
    });
    // Copies a call's option map into the element, keys the element has
    // not already claimed. Unknown options ride along unread — a script
    // written against a NEWER vocabulary still parses here.
    fn merge_opts(m: &mut Map, opts: Map) {
        for (k, v) in opts {
            m.entry(k).or_insert(v);
        }
    }
    engine.register_fn("rows", move |rows: Array| {
        let mut m = Map::new();
        m.insert("kind".into(), "rows".into());
        m.insert("rows".into(), Dynamic::from_array(rows));
        m
    });
    // rows(items, #{ columns, label_width, align, density }) — u2 §3.1
    // #4. An item may be [label, value] or [label, value, severity].
    //
    // No `label_role` and no `value_role`: a row's two halves are set in
    // whatever `script.rows_label_role` and `script.rows_value_role`
    // name, on every panel, in every widget. The options are gone rather
    // than deprecated — an ignored option is a script that still reads as
    // though it chose something.
    engine.register_fn("rows", move |rows: Array, opts: Map| {
        let mut m = Map::new();
        m.insert("kind".into(), "rows".into());
        m.insert("rows".into(), Dynamic::from_array(rows));
        merge_opts(&mut m, opts);
        m
    });
    engine.register_fn("text", move |content: &str| {
        let mut m = Map::new();
        m.insert("kind".into(), "text".into());
        m.insert("content".into(), content.into());
        // No alignment stored: a script that names none defers to the
        // theme's `script.text_align`, which the renderer reads.
        m.insert("size".into(), Dynamic::from_float(1.0));
        m
    });
    engine.register_fn("text", move |content: &str, align: &str, size: f64| {
        let mut m = Map::new();
        m.insert("kind".into(), "text".into());
        m.insert("content".into(), content.into());
        m.insert("align".into(), align.into());
        m.insert("size".into(), Dynamic::from_float(size));
        m
    });
    // text(content, align, #{ role, severity }) — u2 §3.1 #2. `role` is
    // a KIND from the master's `script.kind_*_role` vocabulary — "date",
    // "reading", "label" — and never a `type.*` role: the theme decides
    // what a date is set in, here and in every other widget at once.
    //
    // The option is spelt `role` and not `kind` because an element map
    // already carries its own `kind` — "text" — and `merge_opts` keeps
    // what the element claimed, so a `#{ kind: … }` option would be
    // dropped in silence.
    engine.register_fn("text", move |content: &str, align: &str, opts: Map| {
        let mut m = Map::new();
        m.insert("kind".into(), "text".into());
        m.insert("content".into(), content.into());
        m.insert("align".into(), align.into());
        merge_opts(&mut m, opts);
        m
    });
    // runs(items, align) — u2 §3.1 #3, NEW: one line of styled runs
    // sharing a baseline, aligned as a unit. Each item is
    // #{ t, role, severity, blink, align }, where `role` is a KIND from
    // `script.kind_*_role` — an item says it is a label or a reading and
    // the theme sets it; blink names a motion.* effect and drives the
    // run's ALPHA, never its glyph (I13). An item's align: "right" pins
    // it to the line's right end — u2 §2.5's temperature run — while the
    // rest align as one unit.
    //
    // The gap between two runs is `script.runs_gap`. A script must not
    // put a run of spaces between two readings to make one: that spends
    // the theme's gap AND a space of the face's own width, and the width
    // of a space is not a number a widget is allowed to know.
    engine.register_fn("runs", move |items: Array| {
        let mut m = Map::new();
        m.insert("kind".into(), "runs".into());
        m.insert("items".into(), Dynamic::from_array(items));
        m
    });
    engine.register_fn("runs", move |items: Array, align: &str| {
        let mut m = Map::new();
        m.insert("kind".into(), "runs".into());
        m.insert("items".into(), Dynamic::from_array(items));
        m.insert("align".into(), align.into());
        m
    });
    // rule() — u2 §3.1 #12, NEW: a horizontal hairline as a stack element
    // in its own right. Until now the only rule was welded to `title`.
    engine.register_fn("rule", move || el("rule"));
    // group(label, elements) — u2 §3.1 #13, NEW: a labelled sub-block —
    // a section caption, an optional rule, and a nested element list
    // measured as one unit.
    engine.register_fn("group", move |label: &str, elements: Array| {
        let mut m = Map::new();
        m.insert("kind".into(), "group".into());
        m.insert("label".into(), label.into());
        m.insert("elements".into(), Dynamic::from_array(elements));
        m
    });
    // badge(text, #{ severity, style }) — u2 §3.1 #11, NEW: the status
    // pill of images 1, 3 and 4. The string is content; the severity is
    // the script's judgement of it; every colour is the theme's.
    engine.register_fn("badge", move |text: &str| {
        let mut m = Map::new();
        m.insert("kind".into(), "badge".into());
        m.insert("text".into(), text.into());
        m
    });
    engine.register_fn("badge", move |text: &str, opts: Map| {
        let mut m = Map::new();
        m.insert("kind".into(), "badge".into());
        m.insert("text".into(), text.into());
        merge_opts(&mut m, opts);
        m
    });
    engine.register_fn("meter", move |label: &str, frac: f64, value: &str| {
        let mut m = Map::new();
        m.insert("kind".into(), "meter".into());
        m.insert("label".into(), label.into());
        m.insert("fraction".into(), Dynamic::from_float(frac));
        m.insert("value".into(), value.into());
        m
    });
    // meter(label, frac, value, #{ severity, track }) — u2 §3.1 #6.
    engine.register_fn("meter", move |label: &str, frac: f64, value: &str, opts: Map| {
        let mut m = Map::new();
        m.insert("kind".into(), "meter".into());
        m.insert("label".into(), label.into());
        m.insert("fraction".into(), Dynamic::from_float(frac));
        m.insert("value".into(), value.into());
        merge_opts(&mut m, opts);
        m
    });
    engine.register_fn("gauges", move |values: Array, columns: i64| {
        let mut m = Map::new();
        m.insert("kind".into(), "gauges".into());
        m.insert("values".into(), Dynamic::from_array(values));
        m.insert("columns".into(), Dynamic::from_int(columns));
        m
    });
    // gauges(values, #{ columns, style, label, value_fmt }) — u2 §3.1 #7,
    // style ∈ { row, cell, bar, donut }.
    engine.register_fn("gauges", move |values: Array, opts: Map| {
        let mut m = Map::new();
        m.insert("kind".into(), "gauges".into());
        m.insert("values".into(), Dynamic::from_array(values));
        merge_opts(&mut m, opts);
        m
    });
    engine.register_fn("dots", move |frac: f64| {
        let mut m = Map::new();
        m.insert("kind".into(), "dots".into());
        m.insert("fraction".into(), Dynamic::from_float(frac));
        m
    });
    engine.register_fn("table", move |headings: Array, rows: Array, elastic: i64| {
        let mut m = Map::new();
        m.insert("kind".into(), "table".into());
        m.insert("headings".into(), Dynamic::from_array(headings));
        m.insert("rows".into(), Dynamic::from_array(rows));
        m.insert("elastic".into(), Dynamic::from_int(elastic));
        m
    });
    // table(headings, rows, elastic, #{ zebra, severity_col }) — u2 §3.1
    // #10. A heading may be [name, align] or [name, align, #{ kind,
    // width, of }], kind ∈ { text, bar, badge }.
    //
    // F2 §2.2 adds the VIEW options to the same map, none of which
    // changes anything unless it is named:
    //
    //   id: "procs"      the view's identity between frames; without one
    //                    it is the table's place among the answer's
    //                    tables, which is stable only for a script that
    //                    always returns the same elements
    //   interactive: true   headings sort and answer the pointer
    //   sort: 1, dir: "desc"   the OPENING arrangement; the user's
    //                    clicks own it from then on
    //   select: "row"    a row may be selected ("none" is the default)
    //   key: 0           the column whose text identifies a row — the
    //                    selection is by that string, never by index
    //   scroll: true     an offset window instead of the truncation at
    //                    the bottom edge
    //   tooltip: true    a heading or a cell the ellipsis cut short
    //                    explains itself when the pointer rests on it
    //                    (F2 §8.1)
    //
    // The state they produce comes back to the script in the `view`
    // constant: `view.procs.selected`, `.sort`, `.dir`, `.scroll`.
    engine.register_fn(
        "table",
        move |headings: Array, rows: Array, elastic: i64, opts: Map| {
            let mut m = Map::new();
            m.insert("kind".into(), "table".into());
            m.insert("headings".into(), Dynamic::from_array(headings));
            m.insert("rows".into(), Dynamic::from_array(rows));
            m.insert("elastic".into(), Dynamic::from_int(elastic));
            merge_opts(&mut m, opts);
            m
        },
    );
    // list(items) / list(items, #{ id, select, scroll, tooltip }) — F2
    // §3. The
    // `[list]` section of the master has described this row since the
    // theme engine landed and nothing has ever drawn it.
    //
    //   item: "label" | [label] | [label, status]
    //       | #{ label, status, severity, bar: 0.42, key }
    //
    // `key` defaults to the label: selection is by STRING, so two rows
    // that read the same must be told apart by the script.
    // `select: "row"` lets one be picked, `scroll: true` gives the list
    // an offset instead of a bottom edge, `tooltip: true` lets a name
    // the ellipsis cut short give itself in full when the pointer rests
    // on it (F2 §8.1); without any of them it is a fixed block of rows
    // and draws through the same path it would anyway.
    engine.register_fn("list", move |items: Array| {
        let mut m = Map::new();
        m.insert("kind".into(), "list".into());
        m.insert("items".into(), Dynamic::from_array(items));
        m
    });
    engine.register_fn("list", move |items: Array, opts: Map| {
        let mut m = Map::new();
        m.insert("kind".into(), "list".into());
        m.insert("items".into(), Dynamic::from_array(items));
        merge_opts(&mut m, opts);
        m
    });
    // tree(nodes) / tree(nodes, #{ id, select, tooltip }) — F2 §4. A
    // tree is not a second view: the renderer FLATTENS it against the
    // widget's expand set and draws the rows `list` draws — including
    // the tooltip a trimmed name files, which the indent brings on
    // sooner here than anywhere else.
    //
    //   node: #{ label, children: [ … ], status, severity, bar, key }
    //
    // The whole answer is bounded by `max_array_size`, so scripts hand
    // over SMALL trees; a real file tree belongs to a plugin with a lazy
    // TreeModel.
    engine.register_fn("tree", move |nodes: Array| {
        let mut m = Map::new();
        m.insert("kind".into(), "tree".into());
        m.insert("nodes".into(), Dynamic::from_array(nodes));
        m
    });
    engine.register_fn("tree", move |nodes: Array, opts: Map| {
        let mut m = Map::new();
        m.insert("kind".into(), "tree".into());
        m.insert("nodes".into(), Dynamic::from_array(nodes));
        merge_opts(&mut m, opts);
        m
    });
    engine.register_fn("columns", move |cells: Array| {
        let mut m = Map::new();
        m.insert("kind".into(), "columns".into());
        m.insert("cells".into(), Dynamic::from_array(cells));
        m
    });
    // columns(cells, #{ align, dividers }) — u2 §3.1 #5. A cell may be
    // [label, value] or [label, value, severity]. Its two halves are set
    // in `script.columns_label_role` and `script.columns_value_role`, for
    // the reason `rows` above states.
    engine.register_fn("columns", move |cells: Array, opts: Map| {
        let mut m = Map::new();
        m.insert("kind".into(), "columns".into());
        m.insert("cells".into(), Dynamic::from_array(cells));
        merge_opts(&mut m, opts);
        m
    });
    engine.register_fn("space", move |size: f64| {
        let mut m = el("space");
        m.insert("size".into(), Dynamic::from_float(size));
        m
    });

    // --- formatting -----------------------------------------------
    engine.register_fn("bytes", |n: f64| fmt_bytes(n.max(0.0) as u64));
    engine.register_fn("bytes", |n: i64| fmt_bytes(n.max(0) as u64));
    engine.register_fn("rate", |n: f64| fmt_rate(n));
    engine.register_fn("uptime", |n: f64| fmt_uptime(n.max(0.0) as u64));
    engine.register_fn("uptime", |n: i64| fmt_uptime(n.max(0) as u64));
    engine.register_fn("upper", |s: &str| s.to_uppercase());
    engine.register_fn("lower", |s: &str| s.to_lowercase());
    // `round(v, places)` says HOW MANY places, which is the script's
    // business; what the mark between them is, and whether the thousands
    // are grouped, is the theme's (§5.17). The clamp matches the master's
    // own 0..6 range on `num.decimals`.
    engine.register_fn("round", |n: f64, places: i64| {
        crate::num::format(n, places.clamp(0, 6) as usize)
    });
    engine
}

/// The host data every script on this frame reads, built once and
/// handed out as a shared value.
///
/// It used to be built per widget, and it is not small: the process
/// table alone is a map per process with its name copied into it. Eight
/// scripted widgets meant eight copies of the whole machine's state
/// sixty times a second — more than half of everything the program did,
/// measured — while only the process list widget looked at the heaviest
/// part of it. The data is the same for all of them within a frame, so
/// now it is made once; time comes from the host, so a new frame is a
/// new map.
fn host_shared(host: &Host) -> Dynamic {
    thread_local! {
        static CACHE: std::cell::RefCell<Option<(f64, Dynamic)>> =
            const { std::cell::RefCell::new(None) };
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some((t, d)) = c.as_ref() {
            if *t == host.t {
                return d.clone();
            }
        }
        // Shared, so handing it to each script is a reference count and
        // not another copy of the process table.
        let d = Dynamic::from_map(host_map(host)).into_shared();
        *c = Some((host.t, d.clone()));
        d
    })
}

/// The two parts of the host data that are lists rather than numbers,
/// kept until the collector replaces the snapshot they came from.
///
/// The clock in the map has to be rebuilt every frame, but the process
/// table does not: it is rewritten once a second and copying it sixty
/// times a second was the single most expensive thing the program did.
fn host_lists(host: &Host) -> (Dynamic, Dynamic) {
    thread_local! {
        static CACHE: std::cell::RefCell<Option<(u64, Dynamic, Dynamic)>> =
            const { std::cell::RefCell::new(None) };
    }
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        let s = host.snap;
        if let Some((g, procs, each)) = c.as_ref() {
            if *g == s.generation {
                return (procs.clone(), each.clone());
            }
        }
        let procs = Dynamic::from_array(
            s.top
                .iter()
                .map(|p| {
                    let mut e = Map::new();
                    e.insert("pid".into(), Dynamic::from_int(p.pid as i64));
                    e.insert("name".into(), p.name.clone().into());
                    e.insert("cpu".into(), Dynamic::from_float(p.cpu as f64));
                    e.insert("mem".into(), Dynamic::from_float(p.mem_pct as f64));
                    Dynamic::from_map(e)
                })
                .collect(),
        )
        .into_shared();
        let each = Dynamic::from_array(
            s.cpu_per_core
                .iter()
                .map(|v| Dynamic::from_float(*v as f64))
                .collect(),
        )
        .into_shared();
        *c = Some((s.generation, procs.clone(), each.clone()));
        (procs, each)
    })
}

/// The host data a script can read, as a plain map. Rebuilt per frame:
/// a script sees a snapshot, never a live handle it could hold on to.
fn host_map(host: &Host) -> Map {
    let s = host.snap;
    let mut m = Map::new();
    let mut put = |k: &str, v: Dynamic| {
        m.insert(k.into(), v);
    };
    put("cpu_name", s.cpu_name.clone().into());
    let (procs, cpu_each) = host_lists(host);
    put("cpu_each", cpu_each);
    put("cpu_cores", Dynamic::from_int(s.cpu_per_core.len() as i64));
    put("load1", Dynamic::from_float(s.load_avg[0]));
    put("load5", Dynamic::from_float(s.load_avg[1]));
    put("load15", Dynamic::from_float(s.load_avg[2]));
    put(
        "temp",
        s.temp_c
            .map(|t| Dynamic::from_float(t as f64))
            .unwrap_or(Dynamic::UNIT),
    );
    put("mem_used", Dynamic::from_float(s.mem_used as f64));
    put("mem_total", Dynamic::from_float(s.mem_total as f64));
    put("mem_fraction", Dynamic::from_float(frac(s.mem_used, s.mem_total)));
    put("swap_used", Dynamic::from_float(s.swap_used as f64));
    put("swap_total", Dynamic::from_float(s.swap_total as f64));
    put(
        "swap_fraction",
        Dynamic::from_float(frac(s.swap_used, s.swap_total)),
    );
    put("uptime", Dynamic::from_float(s.uptime as f64));
    put("iface", s.iface.clone().into());
    put(
        "ipv4",
        s.ipv4.clone().map(Dynamic::from).unwrap_or(Dynamic::UNIT),
    );
    put(
        "ping",
        s.ping_ms
            .map(|p| Dynamic::from_int(p as i64))
            .unwrap_or(Dynamic::UNIT),
    );
    put("online", s.online.into());
    put("net_up", Dynamic::from_float(s.net_up_rate));
    put("net_down", Dynamic::from_float(s.net_down_rate));
    put("manufacturer", s.manufacturer.clone().into());
    put("model", s.model.clone().into());
    put("chassis", s.chassis.clone().into());
    put("name", s.hostname.clone().into());
    put("user", s.username.clone().into());
    put("os", s.os_name.clone().into());
    put("kernel", s.kernel.clone().into());
    put(
        "battery",
        s.battery
            .map(|(p, _)| Dynamic::from_int(p as i64))
            .unwrap_or(Dynamic::UNIT),
    );
    put(
        "charging",
        s.battery.map(|(_, c)| Dynamic::from(c)).unwrap_or(Dynamic::UNIT),
    );
    put("proc_count", Dynamic::from_int(s.proc_count as i64));
    // The wall clock and the animation clock: widgets that show the
    // time or blink need them, and nothing else in the host data does.
    let now = chrono::Local::now();
    use chrono::{Datelike, Timelike};
    put("hour", Dynamic::from_int(now.hour() as i64));
    put("minute", Dynamic::from_int(now.minute() as i64));
    put("second", Dynamic::from_int(now.second() as i64));
    put("day", Dynamic::from_int(now.day() as i64));
    put("date", now.format("%a %b %d").to_string().into());
    put("date_long", now.format("%A %d %B %Y").to_string().into());
    put("t", Dynamic::from_float(host.t));
    put("processes", procs);
    m
}

fn frac(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 / whole as f64
    }
}

impl Script {
    /// Compiles a widget script. A script that will not compile is
    /// reported once and the widget stays blank; a broken widget must
    /// not take the program down with it.
    pub fn load(path: &Path) -> Option<Script> {
        let src = std::fs::read_to_string(path).ok()?;
        let engine = engine();
        match engine.compile(&src) {
            Ok(ast) => Some(Script { engine, ast, failed: false }),
            Err(e) => {
                eprintln!("nacelle-desktop: {}: {e}", path.display());
                None
            }
        }
    }
}

fn str_of(m: &Map, key: &str) -> String {
    m.get(key)
        .map(|v| v.clone().into_string().unwrap_or_default())
        .unwrap_or_default()
}

fn f32_of(m: &Map, key: &str, def: f32) -> f32 {
    m.get(key)
        .and_then(|v| v.as_float().ok())
        .map(|f| f as f32)
        .filter(|f| f.is_finite())
        .unwrap_or(def)
}

fn align_of(s: &str) -> Align {
    match s {
        "right" => Align::Right,
        "center" => Align::Center,
        _ => Align::Left,
    }
}

fn bool_of(m: &Map, key: &str, def: bool) -> bool {
    m.get(key).and_then(|v| v.as_bool().ok()).unwrap_or(def)
}

fn int_of(m: &Map, key: &str, def: i64) -> i64 {
    m.get(key).and_then(|v| v.as_int().ok()).unwrap_or(def)
}

/// The severity an element or item carries, if it names one. The word is
/// from the closed set; an unknown word resolves through
/// `script.severity_fallback` — to `unknown`, NEVER to `ok` (§5.10).
fn sev_opt(m: &Map, key: &str) -> Option<ui::Sev> {
    let word = str_of(m, key);
    if word.is_empty() {
        return None;
    }
    Some(ui::sev_of(&word).unwrap_or_else(|| {
        ui::warn_once(
            &format!("sev:{word}"),
            &format!("unknown severity \"{word}\" — resolving to the fallback, never ok"),
        );
        ui::sev_fallback()
    }))
}

/// A number a script wrote either way round: rhai keeps `1` an integer
/// and `1.0` a float, and a fraction is a fraction whichever the author
/// typed.
fn num_of(v: &Dynamic) -> Option<f32> {
    v.as_float()
        .ok()
        .map(|f| f as f32)
        .or_else(|| v.as_int().ok().map(|i| i as f32))
        .filter(|f| f.is_finite())
}

/// One `list` item — or one `tree` node's own row — in any of the forms
/// the elements accept.
///
/// The map form is the whole vocabulary; the string and array forms are
/// the shorthands a script reaches for when a row is only a label, and
/// they exist for the same reason `rows` accepts `[label, value]`.
fn list_item(v: &Dynamic) -> view::RowBuf {
    let mut row = view::RowBuf::new();
    if let Some(m) = v.read_lock::<Map>() {
        row.label = str_of(&m, "label");
        row.status = str_of(&m, "status");
        row.severity = sev_opt(&m, "severity");
        row.bar = m.get("bar").and_then(num_of);
        row.key = str_of(&m, "key");
    } else if let Some(a) = v.read_lock::<Array>() {
        row.label = a.first().map(|x| x.to_string()).unwrap_or_default();
        row.status = a.get(1).map(|x| x.to_string()).unwrap_or_default();
    } else {
        row.label = v.to_string();
    }
    // The key defaults to the label: a row has to have an identity, and
    // the only one a bare string carries is what it says.
    if row.key.is_empty() {
        row.key = row.label.clone();
    }
    row
}

/// How deep a `tree` element's nesting is followed.
///
/// A rhai map cannot be cyclic, so this is not a safety net against a
/// loop; it is a bound on a script that builds a thousand-deep spine by
/// accident, where the indent alone would have pushed every label off
/// the panel long before.
const TREE_MAX_DEPTH: usize = 32;

/// One `tree` node: a list row, plus whatever `children` it declares.
fn tree_node(v: &Dynamic, depth: usize) -> view::tree::MemNode {
    let row = list_item(v);
    let mut children = Vec::new();
    if depth < TREE_MAX_DEPTH {
        if let Some(m) = v.read_lock::<Map>() {
            if let Some(a) = m.get("children").and_then(|c| c.read_lock::<Array>()) {
                children = a.iter().map(|c| tree_node(c, depth + 1)).collect();
            }
        }
    }
    view::tree::MemNode { row, children }
}

/// The type role one KIND word stands for, through the master's
/// `script.kind_<word>_role` binding.
///
/// This is the whole of a script's say over type, and it is a say about
/// CONTENT: a run is a label, a reading, the clock. How big a reading is,
/// in which face and at which weight, is one line of the master and is
/// the same line for every widget on the screen.
///
/// It used to be [`ui::role`] on the script's own word, which let a call
/// name any of the twenty-four type roles — and that is how the value
/// half of four panels of the same kind came to be set at three different
/// sizes. The vocabulary is CLOSED by the master: a word it does not bind
/// warns once and draws nothing, because there is no spare role and a
/// misspelt kind must show as a hole rather than as a plausible line
/// nobody chose (§5.16).
fn kind_role(word: &str) -> ui::Role {
    thread_local! {
        /// One entry per kind a script has ever named. The token id is
        /// what is cached and not the role: the id is fixed for the life
        /// of the process — the schema is stage 1 of the cascade and does
        /// not move — while the WORD behind it changes when a theme
        /// re-points the binding, so that is read on every draw, exactly
        /// as [`ui::bound_role`] reads its own.
        static IDS: RefCell<HashMap<String, Option<TokenId>>> = RefCell::new(HashMap::new());
    }
    let known = IDS.with(|c| c.borrow().get(word).copied());
    let id = match known {
        Some(id) => id,
        None => {
            let id = theme::id(&format!("script.kind_{word}_role"))
                // A word that is not a kind may still be a `type.*` ROLE:
                // that is what every script in this repository said before
                // the vocabulary closed, and what an addon outside this
                // repository still says. See [`kind_for_legacy_role`].
                .or_else(|| kind_for_legacy_role(word));
            if id.is_none() {
                // Once per word, on first sight — a broken script draws
                // sixty times a second and must not say this that often.
                ui::warn_once(
                    &format!("script.kind:{word}"),
                    &format!(
                        "a script names the kind \"{word}\", which no \
                         script.kind_{word}_role binds — it is drawn in \
                         script.text_role. Name what the string IS: {}",
                        kind_vocabulary()
                    ),
                );
            }
            IDS.with(|c| c.borrow_mut().insert(word.to_string(), id));
            id
        }
    };
    match id {
        Some(id) => ui::role(&ui::theme_word(id)),
        // The word named neither a kind nor a role the master binds. The
        // line is still DRAWN, in the role a call naming nothing gets:
        // this is a repository of addons, third-party files are the point,
        // and a widget that silently loses its text on the day the
        // vocabulary closed is a worse answer than one whose text is a
        // size somebody chose. The rule the deprecated `text(content,
        // align, size)` form already follows, applied to the other half of
        // the same migration.
        None => ui::bound_role(&FALLBACK_TEXT_ROLE, "script.text_role"),
    }
}

/// The binding a script's word falls back to when it names no kind.
static FALLBACK_TEXT_ROLE: OnceLock<TokenId> = OnceLock::new();

/// The KIND whose master binding already points at the `type.*` role
/// `word` names, if there is one.
///
/// The migration path for an addon outside this tree. Every script in this
/// repository used to name type roles directly — `#{ role: "data" }`,
/// `rows(…, #{ value_role: "data" })` — and the vocabulary that replaced
/// them is five kinds. A third-party script still holding the old word
/// would otherwise resolve to nothing and draw nothing, which is a widget
/// going blank on an upgrade.
///
/// The mapping is DERIVED, never tabulated: the master already says
/// `kind_reading_role = value`, so a script asking for `value` is asking
/// for the reading kind and can be told so by name. Nothing here is a
/// look, and a theme that re-points a binding re-points this with it —
/// which a table of twenty pairs in this file could not do.
fn kind_for_legacy_role(word: &str) -> Option<TokenId> {
    // Not a role name at all: `kind_role`'s own sentinel, and any word
    // with a space in it, which no `type.*` role has.
    if word.is_empty() || word.contains(' ') {
        return None;
    }
    for kind in KINDS {
        let Some(id) = theme::id(&format!("script.kind_{kind}_role")) else { continue };
        if ui::theme_word(id) == word {
            ui::warn_once(
                &format!("script.legacy:{word}"),
                &format!(
                    "a script names the type role \"{word}\" where a KIND belongs — \
                     drawn as \"{kind}\", which is what the master binds to it. \
                     Write #{{ role: \"{kind}\" }}: a call names what a string IS, \
                     and the master decides how big it is"
                ),
            );
            return Some(id);
        }
    }
    None
}

/// The kinds this file knows to ask the master about.
///
/// Five words, and they are the master's — `script.kind_<word>_role` is
/// the binding, so this list is the set of bindings a lookup by kind can
/// hit. It is spelled out because the schema publishes tokens by NAME and
/// there is no way to enumerate "every key matching a pattern"; a kind the
/// master adds and this list does not carry still WORKS (the direct lookup
/// in [`kind_role`] finds it), it simply cannot be reached by the legacy
/// role name it replaced, which is the one path this list serves.
const KINDS: [&str; 5] = ["clock", "date", "label", "reading", "text"];

/// The kinds, for the warning that turns a script author away from role
/// names. Built on the cold path only — this runs once per bad word.
fn kind_vocabulary() -> String {
    KINDS
        .iter()
        .filter(|k| theme::id(&format!("script.kind_{k}_role")).is_some())
        .map(|k| format!("\"{k}\""))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The role one `runs` item — or a `text` element, which is a line of one
/// run — draws in: the kind it names, or `script.text_role` for a call
/// that names none.
fn run_role(m: &Map) -> ui::Role {
    static TEXT_ROLE: OnceLock<TokenId> = OnceLock::new();
    let kind = str_of(m, "role");
    if kind.is_empty() {
        ui::bound_role(&TEXT_ROLE, "script.text_role")
    } else {
        kind_role(&kind)
    }
}

/// The theme's default text alignment (`script.text_align`), for a call
/// that names none.
fn theme_text_align() -> Align {
    static ALIGN: OnceLock<TokenId> = OnceLock::new();
    static CENTER: OnceLock<Option<u16>> = OnceLock::new();
    static RIGHT: OnceLock<Option<u16>> = OnceLock::new();
    let id = tok(&ALIGN, "script.text_align");
    let cur = theme::resolved().enum_of(id);
    if *CENTER.get_or_init(|| theme::enum_index(id, "center")) == Some(cur) {
        Align::Center
    } else if *RIGHT.get_or_init(|| theme::enum_index(id, "right")) == Some(cur) {
        Align::Right
    } else {
        Align::Left
    }
}

/// The role a `text` element draws in: the kind it names, or the theme's
/// `script.text_role` (u2 §3.1 #2).
///
/// The third form — `text(content, align, size)`, a raw multiplier on the
/// panel's font px — used to be honoured by mapping the number to the
/// nearest role on the type ladder. It is honoured no longer, and the
/// mapping is gone with it: it was the last path by which a CALL decided
/// how big something was, it landed wherever the ladder happened to sit
/// under that theme, and a size chosen by arithmetic is a size no theme
/// file can account for. The call still draws — in the bound role, warned
/// once — because a script losing a line of text is worse than a script
/// showing it at the theme's size.
fn text_role_of(m: &Map) -> ui::Role {
    // The deprecated form is recognisable by carrying BOTH an alignment
    // and a size: the one-argument form stores only its default size.
    if str_of(m, "role").is_empty() && m.contains_key("size") && m.contains_key("align") {
        ui::warn_once(
            "text.size",
            "text(content, align, size) is deprecated and its size is IGNORED — a call \
             does not choose a size. Name what the string is instead: \
             text(content, align, #{ role: \"reading\" }); until then it is drawn in \
             script.text_role",
        );
    }
    run_role(m)
}

/// What a script's INTERACTIVE elements keep between frames.
///
/// A script is a pure function of its data — that is the sandbox, and it
/// is what lets the answer be cached per frame — so the sort a user
/// chose, the row they picked and the offset they scrolled to cannot
/// live in the script. They live here, keyed by the element's `id`, and
/// the script READS them back through the `view` constant.
/// A `tree` element's state.
///
/// The list state every row view has, plus the flattener that turns the
/// script's nested answer into rows. The EXPANSION lives on the
/// flattener and is keyed by path, so the script rebuilding its nodes
/// every frame — which it does — leaves the shape the user opened
/// exactly where it was (F2 §4).
#[derive(Default)]
struct TreeView {
    list: view::ListState,
    flat: view::FlatTree<view::tree::MemTree>,
}

#[derive(Default)]
pub struct ViewState {
    /// One state per interactive `table`, by its `id`.
    tables: BTreeMap<String, view::TableState>,
    /// One per interactive `list`.
    lists: BTreeMap<String, view::ListState>,
    /// One per `tree`.
    trees: BTreeMap<String, TreeView>,
    /// The rectangles the last draw recorded, for the input that arrives
    /// between frames.
    hits: view::Hits,
    /// `ids[n]` is the id of the nth view of the last answer — a `Hit`
    /// carries the ordinal, because a u32 crosses an ABI and a String
    /// does not.
    ids: Vec<String>,
    /// Where the pointer was at the last draw. A wheel event carries no
    /// coordinates, and a widget with two scrolling tables has to know
    /// which one the pointer is over.
    mouse: (f32, f32),
    /// What a press landed on, remembered until the drag that follows
    /// it, WITH the point it landed on: the thumb is grabbed where the
    /// hand took it and not where the hand is by the first `Move` (see
    /// [`ScriptWidget::drag`], which decides there whether the pointer
    /// is asked for at all).
    press: Option<(Hit, f32, f32)>,
}

impl ViewState {
    /// The sum of every view's interaction epoch: a number that changes
    /// whenever anything the user did changed, and the second half of
    /// the element cache's key.
    fn epoch(&self) -> u64 {
        let tables = self
            .tables
            .values()
            .fold(0u64, |a, t| a.wrapping_add(t.interact_epoch));
        let lists = self
            .lists
            .values()
            .fold(0u64, |a, l| a.wrapping_add(l.interact_epoch));
        self.trees
            .values()
            .fold(tables.wrapping_add(lists), |a, t| {
                a.wrapping_add(t.list.interact_epoch)
            })
    }

    /// A new draw: the rectangles of the last one are stale the moment
    /// the model can have changed under them.
    fn begin(&mut self, mouse: (f32, f32)) {
        self.hits.clear();
        self.ids.clear();
        self.mouse = mouse;
    }

    /// The table a hit's ordinal names, if the last draw drew one.
    fn table_of(&mut self, ordinal: u32) -> Option<&mut view::TableState> {
        let id = self.ids.get(ordinal as usize)?.clone();
        self.tables.get_mut(&id)
    }

    /// The list state a hit's ordinal names — a `list`'s own, or the one
    /// inside a `tree`, since a tree scrolls and selects as a list.
    fn list_of(&mut self, ordinal: u32) -> Option<&mut view::ListState> {
        let id = self.ids.get(ordinal as usize)?.clone();
        if self.lists.contains_key(&id) {
            return self.lists.get_mut(&id);
        }
        self.trees.get_mut(&id).map(|t| &mut t.list)
    }

    /// The tree a hit's ordinal names — only an expander asks.
    fn tree_of(&mut self, ordinal: u32) -> Option<&mut TreeView> {
        let id = self.ids.get(ordinal as usize)?.clone();
        self.trees.get_mut(&id)
    }

    /// Every gesture-scoped state let go. Called when a gesture ends and
    /// again when the next one starts: a host may deliver a press whose
    /// release lands somewhere else entirely (outside the panel, on
    /// another window), and a heading stuck in its `press` rung for the
    /// rest of the session is worse than a heading that lets go early.
    fn release_all(&mut self) {
        for t in self.tables.values_mut() {
            t.release_head();
            t.release_divider();
            t.scroll.release();
        }
        for l in self.lists.values_mut() {
            l.scroll.release();
        }
        for t in self.trees.values_mut() {
            t.list.scroll.release();
        }
        self.press = None;
    }

    /// What the script sees as the `view` constant: one entry per view,
    /// under the id the script named it by.
    fn as_map(&self) -> Map {
        let mut out = Map::new();
        for (id, t) in self.tables.iter() {
            let mut m = Map::new();
            m.insert(
                "selected".into(),
                match &t.selected {
                    Some(k) => Dynamic::from(k.clone()),
                    None => Dynamic::UNIT,
                },
            );
            match t.sort {
                Some((c, d)) => {
                    m.insert("sort".into(), Dynamic::from_int(c as i64));
                    m.insert("dir".into(), Dynamic::from(d.word().to_string()));
                }
                None => {
                    m.insert("sort".into(), Dynamic::UNIT);
                    m.insert("dir".into(), Dynamic::UNIT);
                }
            }
            m.insert("scroll".into(), Dynamic::from_float(t.scroll.offset() as f64));
            out.insert(id.as_str().into(), Dynamic::from_map(m));
        }
        for (id, l) in self.lists.iter() {
            out.insert(id.as_str().into(), Dynamic::from_map(list_map(l)));
        }
        for (id, t) in self.trees.iter() {
            let mut m = list_map(&t.list);
            // A tree also tells the script what is OPEN, so a script can
            // fetch only the branches that are showing — the seam a lazy
            // model would grow through.
            m.insert(
                "expanded".into(),
                Dynamic::from_array(
                    t.flat.expansion().into_iter().map(Dynamic::from).collect(),
                ),
            );
            out.insert(id.as_str().into(), Dynamic::from_map(m));
        }
        out
    }
}

/// What a `list` (and the list half of a `tree`) tells the script back.
fn list_map(l: &view::ListState) -> Map {
    let mut m = Map::new();
    m.insert(
        "selected".into(),
        match &l.selected {
            Some(k) => Dynamic::from(k.clone()),
            None => Dynamic::UNIT,
        },
    );
    m.insert("scroll".into(), Dynamic::from_float(l.scroll.offset() as f64));
    m
}

/// A widget drawn by its script.
pub struct ScriptWidget {
    script: Script,
    /// What the script last answered, the moment it answered and the
    /// interaction epoch it answered under.
    /// A frame asks twice — once to measure, once to draw — and the
    /// script is not cheap: running it again would double the cost and
    /// let the two answers disagree, which is worse. Time comes from
    /// the host, so a new frame is a new answer; the epoch is there so a
    /// click that lands WITHIN a frame invalidates the answer too,
    /// instead of showing the state from before it.
    cached: Option<(f64, u64, Array)>,
    views: ViewState,
}

impl ScriptWidget {
    pub fn new(script: Script) -> Self {
        ScriptWidget { script, cached: None, views: ViewState::default() }
    }
}

impl ScriptWidget {
    /// Runs the script's `draw` and hands back the elements it asked
    /// for. None when the script has failed — said once, then the
    /// widget goes quiet, because sixty identical lines a second would
    /// bury everything else.
    fn elements(&mut self, host: &Host) -> Option<Array> {
        if self.script.failed {
            return None;
        }
        let epoch = self.views.epoch();
        if let Some((t, e, elements)) = &self.cached {
            if *t == host.t && *e == epoch {
                return Some(elements.clone());
            }
        }
        let mut scope = Scope::new();
        scope.push_constant("host", host_shared(host));
        // The other half of the conversation: what the user has done to
        // the views this widget drew. A script that shows a detail panel
        // for the selected row reads it from here.
        scope.push_constant("view", Dynamic::from_map(self.views.as_map()));
        let result: Result<Array, _> =
            self.script
                .engine
                .call_fn(&mut scope, &self.script.ast, "draw", ());
        match result {
            Ok(a) => {
                self.cached = Some((host.t, epoch, a.clone()));
                Some(a)
            }
            Err(e) => {
                eprintln!("nacelle-desktop: widget script failed: {e}");
                self.script.failed = true;
                None
            }
        }
    }
}

impl Widget for ScriptWidget {
    fn draw(&mut self, ctx: &mut Ctx, r: Rect, host: &Host) {
        let Some(elements) = self.elements(host) else { return };
        self.views.begin(ctx.mouse.at());
        let mut pass = ViewPass {
            state: &mut self.views,
            generation: host.snap.generation,
            unnamed: 0,
        };
        render(ctx, r, &elements, &mut pass);
        if pass.unnamed > 1 {
            // §11's trap: without an `id` a view is known by its place
            // among the answer's views, and a script that changes how
            // many it returns hands one view's state to another.
            let who = chrome_of(&elements).title.unwrap_or_else(|| "(untitled)".into());
            ui::warn_once(
                &format!("script.view_id.{who}"),
                &format!(
                    "script widget {who}: {} interactive views (table / list / tree) \
                     with no `id` — their state is keyed by position and will be \
                     swapped the moment the script returns a different number of \
                     them; give each one an id",
                    pass.unnamed
                ),
            );
        }
    }

    /// The script's `title` element, read as a chrome declaration: the
    /// host's title band shows the same two strings, from the same host
    /// data (u2 §3.1 #1, §4). The element list is cached per frame, so
    /// asking here and drawing later runs the script once, and the two
    /// answers cannot disagree.
    fn chrome(&mut self, _ctx: &mut Ctx, host: &Host) -> crate::widget::Chrome {
        match self.elements(host) {
            Some(elements) => chrome_of(&elements),
            None => crate::widget::Chrome::none(),
        }
    }

    fn sizing(&mut self, ctx: &mut Ctx, host: &Host) -> Sizing {
        let Some(elements) = self.elements(host) else { return Sizing::Rows };
        let maps: Vec<Map> = elements
            .iter()
            .filter_map(|e| e.clone().try_cast::<Map>())
            .collect();
        let (fixed, flexible) = measure(ctx, &maps, &metrics());
        // One growing element and the widget has no height of its own:
        // a table takes as many rows as it is given, and giving it a
        // fixed height would be inventing a limit.
        if flexible > 0 {
            Sizing::Rows
        } else {
            Sizing::Content(fixed)
        }
    }

    /// A click, tested against the rectangles the last draw recorded —
    /// the filesystem widget's pattern, with a typed hit instead of an
    /// index. Always [`Action::None`]: a sort, a selection and a scroll
    /// are state INSIDE this view, never a request to the application.
    fn click(&mut self, x: f32, y: f32, _r: Rect, host: &Host) -> Action {
        // A press that ended in a click is over, whatever it was aimed
        // at — nothing may outlive the gesture that started it.
        self.views.release_all();
        let Some(hit) = self.views.hits.at(x, y).cloned() else { return Action::None };
        match hit {
            Hit::TableHead { id, col } => {
                if let Some(t) = self.views.table_of(id) {
                    t.click_head(col);
                }
            }
            Hit::TableDivider { id, col } => {
                // A CLICK on a grip (as opposed to a drag through it)
                // hands the column back to the measure. The usual
                // gesture for that is a double click, and a widget is
                // not told about those — the host owns click counting —
                // so the single click is what this has.
                if let Some(t) = self.views.table_of(id) {
                    t.set_width(col, None);
                }
            }
            Hit::Row { id, key } => {
                // A row belongs to whichever view recorded it; the two
                // families answer the same way, which is the point of
                // selecting by key rather than by index.
                if let Some(t) = self.views.table_of(id) {
                    let already = t.is_selected(&key);
                    t.select((!already).then_some(key));
                } else if let Some(l) = self.views.list_of(id) {
                    let already = l.is_selected(&key);
                    l.select((!already).then_some(key));
                }
            }
            Hit::Disclosure { id, key } => {
                // The expander opens and closes; the SELECTION is not
                // touched, because a user opening a folder has not
                // stopped looking at the file they had picked.
                if let Some(t) = self.views.tree_of(id) {
                    t.flat.toggle(&key);
                    // The flat list is about to be a different length,
                    // and the per-frame answer cache has to know within
                    // the frame the click landed in — the expansion is
                    // the tree's half of what `interact_epoch` counts.
                    t.list.interact_epoch = t.list.interact_epoch.wrapping_add(1);
                }
            }
            Hit::Track { id, toward_end } => {
                if let Some(t) = self.views.table_of(id) {
                    let page = t.extent.viewport;
                    t.scroll.page(toward_end, page, host.t);
                } else if let Some(l) = self.views.list_of(id) {
                    let page = l.extent.viewport;
                    l.scroll.page(toward_end, page, host.t);
                }
            }
            _ => {}
        }
        Action::None
    }

    /// The wheel turned. A wheel event carries no position, so the view
    /// it moves is the one the pointer was over at the last draw —
    /// `ctx.mouse`, recorded then, exactly as the hover states are read.
    fn wheel(&mut self, dy: f32, _r: Rect, host: &Host) -> Action {
        let mouse = self.views.mouse;
        let Some(hit) = self.views.hits.at(mouse.0, mouse.1).cloned() else {
            return Action::None;
        };
        let phys = view::scroll::ScrollPhysics::from_theme();
        if let Some(t) = self.views.table_of(hit.id()) {
            if t.extent.scrollable {
                // Positive `dy` scrolls toward the START of the content,
                // whichever way the platform spells its deltas — the
                // sign the filesystem widget has always used.
                t.scroll.wheel(-dy, &phys, host.t);
            }
        } else if let Some(l) = self.views.list_of(hit.id()) {
            if l.extent.scrollable {
                l.scroll.wheel(-dy, &phys, host.t);
            }
        }
        Action::None
    }

    /// A pointer drag. `Begin` REMEMBERS what is under the press, and
    /// asks for the pointer only when the press landed on something this
    /// widget drives itself.
    ///
    /// That distinction is the whole gesture. A host sends `Move` only
    /// to a widget that answered [`Action::Capture`], so declining every
    /// press — which this did, unconditionally, on the reasoning that
    /// "the grab waits for the first `Move`" — meant the `Move` never
    /// came and the thumb branch below was unreachable code. A script's
    /// scrollbar could be seen, hovered and paged, and not dragged.
    ///
    /// So the thumb, and ONLY the thumb, takes the pointer. Everything
    /// else still answers `None` and falls through to the ordinary click
    /// delivery, which is where sorting a column and selecting a row
    /// happen — a captured press ends in no click at all, so capturing
    /// wider than the grab would cost every one of them.
    fn drag(&mut self, p: DragPhase, x: f32, y: f32, _r: Rect, _host: &Host) -> Action {
        match p {
            DragPhase::Begin => {
                self.views.release_all();
                self.views.press = self.views.hits.at(x, y).cloned().map(|h| (h, x, y));
                if let Some((Hit::TableHead { id, col }, _, _)) = self.views.press.clone() {
                    if let Some(t) = self.views.table_of(id) {
                        t.press_head(col);
                    }
                }
                match self.views.press {
                    // Mine, and I want nothing: the host holds the
                    // pointer for me, the board does not turn under the
                    // hand, and the release is a release rather than a
                    // click on whatever row the thumb was over.
                    Some((Hit::Thumb { .. }, _, _)) => Action::Capture,
                    _ => Action::None,
                }
            }
            DragPhase::Move => {
                let Some((hit, px, py)) = self.views.press.clone() else {
                    return Action::None;
                };
                match hit {
                    Hit::Thumb { id } => {
                        let thumb = self.views.hits.rect_of(&Hit::Thumb { id });
                        if let (Some(thumb), Some(t)) = (thumb, self.views.table_of(id)) {
                            // Grabbed where the PRESS landed, not where
                            // the pointer is now: by the first Move it
                            // may already have left the thumb, and the
                            // grab has to remember how far down it.
                            if !t.scroll.dragging() {
                                t.scroll.press_thumb(py, thumb);
                            }
                            if let Some((track, _)) = t.extent.bar {
                                let (v, c) = (t.extent.viewport, t.extent.content);
                                t.scroll.drag(y, v, c, track);
                            }
                        } else if let (Some(thumb), Some(l)) =
                            (thumb, self.views.list_of(id))
                        {
                            if !l.scroll.dragging() {
                                l.scroll.press_thumb(py, thumb);
                            }
                            if let Some((track, _)) = l.extent.bar {
                                let (v, c) = (l.extent.viewport, l.extent.content);
                                l.scroll.drag(y, v, c, track);
                            }
                        }
                    }
                    Hit::TableDivider { id, col } => {
                        static COL_MIN_W: OnceLock<TokenId> = OnceLock::new();
                        let min_w = theme::resolved().px(tok(&COL_MIN_W, "table.col_min_w"));
                        let w0 = self
                            .views
                            .hits
                            .rect_of(&Hit::TableHead { id, col })
                            .map(|r| r.w);
                        if let (Some(w0), Some(t)) = (w0, self.views.table_of(id)) {
                            if t.dragging_divider() != Some(col) {
                                t.grab_divider(col, px, w0);
                            }
                            t.drag_divider(x, min_w);
                        }
                    }
                    _ => {}
                }
                Action::None
            }
            DragPhase::End => {
                self.views.release_all();
                Action::None
            }
        }
    }
}

/// The first `title` element in a script's answer, as the host's chrome
/// declaration. `title("")` — the underline alone — declares nothing: a
/// rule is not a heading, and an empty band would take height from a
/// panel that asked for a line.
fn chrome_of(elements: &Array) -> crate::widget::Chrome {
    for e in elements.iter() {
        let Some(m) = e.read_lock::<Map>() else { continue };
        if str_of(&m, "kind") != "title" {
            continue;
        }
        let left = str_of(&m, "left");
        let right = str_of(&m, "right");
        if left.is_empty() && right.is_empty() {
            continue;
        }
        return crate::widget::Chrome {
            title: (!left.is_empty()).then_some(left),
            right: (!right.is_empty()).then_some(right),
            ..crate::widget::Chrome::none()
        };
    }
    crate::widget::Chrome::none()
}

/// The stack metrics every element height comes from, read from the
/// theme once per pass. Measure and draw walk the same numbers, so they
/// are gathered here rather than looked up twice and allowed to drift.
struct Metrics {
    row_h: f32,
    row_compact: f32,
    title_block: f32,
    columns_block: f32,
    spacer: f32,
    rule_block: f32,
    group_gap: f32,
    /// The air between two stack elements: the implicit one and the two
    /// an element may claim for itself. See [`stack_gap`].
    element_gap: f32,
    meter_gap: f32,
    dots_gap: f32,
    /// `list.row_h` and `list.gap`: what a `list` element measures at.
    /// The measure pass runs a frame before there is a draw list, so it
    /// reads the two tokens here rather than through a
    /// [`view::Surface`], and [`view::list::height`] is the one place
    /// the arithmetic lives.
    list_row_h: f32,
    list_gap: f32,
    /// A multiplier on the type size, not a length — never scaled.
    text_leading: f32,
    min_flex_h: f32,
}

fn metrics() -> Metrics {
    static ROW_H: OnceLock<TokenId> = OnceLock::new();
    static ROW_COMPACT: OnceLock<TokenId> = OnceLock::new();
    static TITLE_BLOCK: OnceLock<TokenId> = OnceLock::new();
    static COLUMNS_BLOCK: OnceLock<TokenId> = OnceLock::new();
    static SPACER: OnceLock<TokenId> = OnceLock::new();
    static RULE_BLOCK: OnceLock<TokenId> = OnceLock::new();
    static GROUP_GAP: OnceLock<TokenId> = OnceLock::new();
    static ELEMENT_GAP: OnceLock<TokenId> = OnceLock::new();
    static METER_GAP: OnceLock<TokenId> = OnceLock::new();
    static DOTS_GAP: OnceLock<TokenId> = OnceLock::new();
    static LIST_ROW_H: OnceLock<TokenId> = OnceLock::new();
    static LIST_GAP: OnceLock<TokenId> = OnceLock::new();
    static TEXT_LEADING: OnceLock<TokenId> = OnceLock::new();
    static MIN_FLEX_H: OnceLock<TokenId> = OnceLock::new();
    static MIN_FLEX_H_MIN: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    Metrics {
        row_h: t.px(tok(&ROW_H, "script.row_h")),
        row_compact: t.px(tok(&ROW_COMPACT, "rhythm.row_compact")),
        title_block: t.px(tok(&TITLE_BLOCK, "script.title_block")),
        columns_block: t.px(tok(&COLUMNS_BLOCK, "script.columns_block")),
        spacer: t.px(tok(&SPACER, "script.spacer")),
        rule_block: t.px(tok(&RULE_BLOCK, "script.rule_block")),
        group_gap: t.px(tok(&GROUP_GAP, "script.group_gap")),
        element_gap: t.px(tok(&ELEMENT_GAP, "script.element_gap")),
        meter_gap: t.px(tok(&METER_GAP, "script.meter_gap")),
        dots_gap: t.px(tok(&DOTS_GAP, "script.dots_gap")),
        list_row_h: t.px(tok(&LIST_ROW_H, "list.row_h")),
        list_gap: t.px(tok(&LIST_GAP, "list.gap")),
        text_leading: t.px(tok(&TEXT_LEADING, "script.text_leading")),
        min_flex_h: t
            .px(tok(&MIN_FLEX_H, "script.min_flex_h"))
            .max(t.px(tok(&MIN_FLEX_H_MIN, "script.min_flex_h_min_px"))),
    }
}

impl Metrics {
    /// The shrink-to-fit pass scales lengths, not ratios.
    fn scaled(&self, k: f32) -> Metrics {
        Metrics {
            row_h: self.row_h * k,
            row_compact: self.row_compact * k,
            title_block: self.title_block * k,
            columns_block: self.columns_block * k,
            spacer: self.spacer * k,
            rule_block: self.rule_block * k,
            group_gap: self.group_gap * k,
            element_gap: self.element_gap * k,
            meter_gap: self.meter_gap * k,
            dots_gap: self.dots_gap * k,
            list_row_h: self.list_row_h * k,
            list_gap: self.list_gap * k,
            text_leading: self.text_leading,
            min_flex_h: self.min_flex_h * k,
        }
    }

    /// One `rows` line at the element's declared density.
    fn rows_line_h(&self, m: &Map) -> f32 {
        if str_of(m, "density") == "compact" {
            self.row_compact
        } else {
            self.row_h
        }
    }
}

/// How many items a `list` element carries.
fn list_len(m: &Map) -> usize {
    m.get("items")
        .and_then(|v| v.read_lock::<Array>().map(|a| a.len()))
        .unwrap_or(0)
}

/// Lines a `rows` element occupies: its items flowed row-major into its
/// grid columns (u2 §2.3).
fn rows_lines(m: &Map) -> usize {
    let n = m
        .get("rows")
        .and_then(|v| v.read_lock::<Array>().map(|a| a.len()))
        .unwrap_or(0);
    let cols = int_of(m, "columns", 1).max(1) as usize;
    n.div_ceil(cols)
}

/// The tallest role on a `runs` line, at shrink 1 — the line's height is
/// that px under the stack's text leading.
fn runs_px(ctx: &Ctx, m: &Map) -> f32 {
    m.get("items")
        .and_then(|v| v.read_lock::<Array>())
        .map(|items| {
            items
                .iter()
                // The SAME resolver the draw uses: a line measured through
                // one path and drawn through another is a line that clips
                // the day the two disagree.
                .map(|it| {
                    it.read_lock::<Map>()
                        .map(|im| run_role(&im).px(ctx, 1.0))
                        .unwrap_or(0.0)
                })
                .fold(0.0, f32::max)
        })
        .unwrap_or(0.0)
}

/// The gap the theme puts between two neighbours of the stack, named in
/// the order they are drawn.
///
/// `script.element_gap` is the IMPLICIT one — what the theme spends
/// between any two elements that ask for nothing else. An element with a
/// gap token of its own overrides it rather than adding to it, and where
/// two such elements meet the wider claim wins, the way two margins
/// collapse: a meter under a meter is spaced once.
///
/// Pure, and taking words rather than maps, so a test can hold the whole
/// rule still the way one holds [`stack_fit`] still.
fn stack_gap(above: &str, below: &str, met: &Metrics) -> f32 {
    // A `space` element IS gap, asked for by name and sized by the
    // script: surrounding it with more of the theme's own would honour
    // one request three times.
    if above == "space" || below == "space" {
        return 0.0;
    }
    match (own_gap(above, met), own_gap(below, met)) {
        (None, None) => met.element_gap,
        (a, b) => a.unwrap_or(0.0).max(b.unwrap_or(0.0)),
    }
}

/// The gap an element kind claims around itself, where the theme gives
/// it one — `None` for the kinds that live on the implicit gap.
fn own_gap(kind: &str, met: &Metrics) -> Option<f32> {
    match kind {
        "meter" => Some(met.meter_gap),
        "dots" => Some(met.dots_gap),
        // A section's opening air (u2 §3.3) is the same air that
        // separates it from what stands above it, so `group` claims it
        // here instead of adding it inside the element.
        "group" => Some(met.group_gap),
        _ => None,
    }
}

/// Height the fixed elements need, and how many elements grow into
/// whatever is left. Walked before drawing, and again a frame earlier
/// by [`ScriptWidget::sizing`] — a widget with nothing growing has
/// a height of its own, and the layout gives it exactly that.
/// Recursive: a `group`'s children are measured as one unit (§3.1 #13).
fn measure(ctx: &Ctx, maps: &[Map], met: &Metrics) -> (f32, usize) {
    let mut fixed = 0.0;
    let mut flexible = 0usize;
    // The element above, for the gap — `None` until one has taken height,
    // because the stack's air goes BETWEEN elements and the panel's own
    // edge is `panel.content_pad`'s business.
    let mut above: Option<String> = None;
    for m in maps {
        let kind = str_of(m, "kind");
        if kind != "title" {
            if let Some(prev) = &above {
                fixed += stack_gap(prev, &kind, met);
            }
            above = Some(kind.clone());
        }
        match kind.as_str() {
            // A `title` is a chrome declaration, consumed by the host's
            // band (u2 §3.1 #1, §4): it takes no body height — the band's
            // block is what `chrome_extra` adds around the content box.
            "title" => {}
            "rows" => fixed += met.rows_line_h(m) * rows_lines(m) as f32,
            "text" => {
                // The role decides the height, through the same resolver
                // the draw uses (text_role_of), so measure and draw
                // cannot disagree.
                fixed += text_role_of(m).px(ctx, 1.0) * met.text_leading;
            }
            "runs" => fixed += runs_px(ctx, m) * met.text_leading,
            // A `list` that does not scroll is exactly its rows, the way
            // `rows` is exactly its lines; one that does takes whatever
            // it is given, like a table. A `tree` is ALWAYS flexible: how
            // tall it is depends on what the user has opened, which lives
            // in the widget's state and not in the element.
            "list" if !bool_of(m, "scroll", false) => {
                fixed += view::list::height(met.list_row_h, met.list_gap, list_len(m))
            }
            "columns" => fixed += met.columns_block,
            "meter" => fixed += met.row_h,
            "badge" => fixed += met.row_h,
            "rule" => fixed += met.rule_block,
            "group" => {
                fixed += met.row_h;
                let children: Vec<Map> = m
                    .get("elements")
                    .and_then(|v| v.read_lock::<Array>())
                    .map(|a| a.iter().filter_map(|e| e.clone().try_cast::<Map>()).collect())
                    .unwrap_or_default();
                let (f, fl) = measure(ctx, &children, met);
                fixed += f;
                flexible += fl;
            }
            "space" => fixed += met.spacer * f32_of(m, "size", 1.0),
            _ => flexible += 1,
        }
    }
    (fixed, flexible)
}

/// The stack-fit arithmetic, pure so a test can hold it still: the
/// height each flexible element receives, the shrink factor, and whether
/// the panel must still clip. The ladder, in order:
/// 1. flexible elements keep `min_flex_h` and the WHOLE stack shrinks
///    toward the `floor` — type included, so nothing is dropped;
/// 2. at the floor the `min_flex_h` guarantee is the next thing to
///    yield: the flexible elements give height back, down to nothing,
///    before a FIXED element — memory's SWAP meter, cpu's LOAD line,
///    exactly the last rows u1 §5.5 check 4 names — is pushed past the
///    bottom edge;
/// 3. only when the fixed rows ALONE overrun the panel at the floor
///    does the panel clip — and [`render`] says so on stderr, once,
///    because a silently dropped row reads as missing data.
fn stack_fit(
    h: f32,
    fixed: f32,
    flexible: usize,
    min_flex: f32,
    floor: f32,
    scales: bool,
) -> (f32, f32, bool) {
    let mut share = if flexible > 0 {
        ((h - fixed) / flexible as f32).max(min_flex)
    } else {
        0.0
    };
    let natural = fixed + share * flexible as f32;
    let raw = if natural > h && natural > 0.0 { h / natural } else { 1.0 };
    let scale = if scales { raw.max(floor) } else { 1.0 };
    if natural * scale > h + 0.5 && flexible > 0 {
        share = ((h / scale - fixed) / flexible as f32).max(0.0);
    }
    let clipped = (fixed + share * flexible as f32) * scale > h + 0.5;
    (share, scale, clipped)
}

/// Draws the element list a script returned.
fn render(ctx: &mut Ctx, r: Rect, elements: &Array, v: &mut ViewPass) {
    static PAD_X: OnceLock<TokenId> = OnceLock::new();
    static STACK_ALIGN: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let met = metrics();
    // `script.pad_x` is the stack's own horizontal inset, inside whatever
    // the host's panel already left. It was declared and read by nothing,
    // so a theme that asked for breathing room around script content got
    // none — and the master states zero, which is exactly why nobody
    // noticed.
    let pad_x = t.px(tok(&PAD_X, "script.pad_x")).max(0.0);
    let r = Rect::new(r.x + pad_x, r.y, (r.w - 2.0 * pad_x).max(0.0), r.h);

    // Fixed-height elements take what they need; the rest share what is
    // left, and the whole stack is fitted to the panel so a widget can
    // never spill onto its neighbours.
    // Cast out of the Dynamics once: read_lock hands back a guard, and
    // the whole list is walked twice (measure, then draw).
    let maps: Vec<Map> = elements
        .iter()
        .filter_map(|e| e.clone().try_cast::<Map>())
        .collect();
    let (fixed, flexible) = measure(ctx, &maps, &met);
    // The overflow policy: `scale` shrinks the stack to fit but no
    // further than the floor — type shrunk past it stops being legible,
    // so from there the flexible elements yield and, at the very end,
    // the panel clips (see `stack_fit`). Any other policy keeps full
    // size (`scroll` needs scroll state that does not exist yet and
    // degrades the same way).
    static OVERFLOW: OnceLock<TokenId> = OnceLock::new();
    static OVERFLOW_SCALE: OnceLock<Option<u16>> = OnceLock::new();
    static MIN_SCALE: OnceLock<TokenId> = OnceLock::new();
    let ov = tok(&OVERFLOW, "script.overflow");
    let scales = OVERFLOW_SCALE
        .get_or_init(|| theme::enum_index(ov, "scale"))
        .is_none_or(|i| t.enum_of(ov) == i);
    // u2 §6.4: the master pins `script.overflow_min_scale` at 0.62 for
    // now — the clamp `panel_font_scale` already applies and the one
    // `uptime` and `hardware` sit on. The specification's 0.72 floor
    // would CLIP those two panels under the default theme; raise the
    // master's value to 0.72 only after the compact arrangements of
    // u2 §2.3/§2.4 land and take them off the clamp.
    let floor = t.px(tok(&MIN_SCALE, "script.overflow_min_scale"));
    let (share, scale, clipped) =
        stack_fit(r.h, fixed, flexible, met.min_flex_h, floor, scales);
    if clipped {
        // Clipping here DROPS content — the tail is one of u1 §5.5's
        // last rows — so it is never silent: one line per widget, the
        // way the panel ladder's report_step announces its steps.
        let who = chrome_of(elements)
            .title
            .unwrap_or_else(|| "(untitled)".into());
        ui::warn_once(
            &format!("script.clip.{who}"),
            &format!(
                "script widget {who}: fixed rows overrun the panel even at \
                 the overflow floor — clipping the tail"
            ),
        );
        ctx.dl.push_clip(r.x, r.y, r.w, r.h);
    }
    let pass = Pass { share: share * scale, scale, met: met.scaled(scale) };
    // `script.stack_align` says where a stack shorter than its panel
    // stands. The word decides — an enum's indices intern in load order —
    // and the master says `middle`, which is what this line did on its
    // own before the key was read at all.
    let vy = ui::vy_of(&ui::theme_word(tok(&STACK_ALIGN, "script.stack_align")));
    let y = ui::block_top_aligned(&r, (fixed + share * flexible as f32) * scale, vy);
    draw_stack(ctx, &r, y, &maps, &pass, v);
    if clipped {
        ctx.dl.pop_clip();
    }
}

/// What one drawing pass carries down the stack — the measured share for
/// flexible elements and the shrink factor everything scales by. One
/// struct, because `group` recurses (u2 §3.1 #13) and its children draw
/// under exactly the numbers their parent measured with.
struct Pass {
    /// The height each flexible element receives, already shrunk.
    share: f32,
    /// The shrink factor itself, for role sizes and paddings.
    scale: f32,
    /// The stack metrics, already shrunk.
    met: Metrics,
}

/// What the INTERACTIVE elements of one answer need while they draw.
///
/// Separate from [`Pass`] because it is `&mut`: the views write their
/// hit rectangles and read their own state, where `Pass` is the same
/// handful of numbers for everyone.
struct ViewPass<'a> {
    state: &'a mut ViewState,
    /// The snapshot's rewrite counter, which is what tells a table's
    /// sort cache "new data" from "the same data again".
    generation: u64,
    /// How many views of this answer named no `id` — two of them share
    /// an ordinal identity, and that is worth saying once (§11).
    unnamed: usize,
}

impl ViewPass<'_> {
    /// Claims the next view slot for `id`, answering the ordinal a
    /// [`Hit`] will carry. The DEFAULT id is that ordinal, which is
    /// stable for a script that always returns the same elements and
    /// treacherous for one that does not — hence the count.
    fn claim(&mut self, id: &str) -> (String, u32) {
        let ordinal = self.state.ids.len() as u32;
        let id = if id.is_empty() {
            self.unnamed += 1;
            ordinal.to_string()
        } else {
            id.to_string()
        };
        self.state.ids.push(id.clone());
        (id, ordinal)
    }
}

/// Draws one element list downwards from `y`; returns the y below the
/// last element. `group` re-enters with its children.
fn draw_stack(
    ctx: &mut Ctx,
    r: &Rect,
    mut y: f32,
    maps: &[Map],
    p: &Pass,
    v: &mut ViewPass,
) -> f32 {
    let t = theme::resolved();
    let (share, scale) = (p.share, p.scale);
    let met = &p.met;
    // The same walk `measure` made, so the gap it counted is the gap
    // drawn: two rules would drift the moment one of them changed.
    let mut above: Option<String> = None;
    for m in maps {
        let kind = str_of(m, "kind");
        if kind != "title" {
            if let Some(prev) = &above {
                y += stack_gap(prev, &kind, met);
            }
            above = Some(kind.clone());
        }
        match kind.as_str() {
            "title" => {
                // Re-homed (u2 §3.1 #1): the element is the chrome
                // declaration the host's title band draws — same strings,
                // same data — and draws NOTHING in the body. The widgets
                // stopped drawing their own titles when the band arrived;
                // drawing here again would show every heading twice.
            }
            "rows" => {
                static LABEL_ROLE: OnceLock<TokenId> = OnceLock::new();
                static VALUE_ROLE: OnceLock<TokenId> = OnceLock::new();
                let items: Vec<ui::RowItem> = m
                    .get("rows")
                    .and_then(|v| v.read_lock::<Array>())
                    .map(|a| {
                        a.iter()
                            .map(|row| {
                                let entry = row.read_lock::<Array>();
                                let get = |i: usize| {
                                    entry
                                        .as_ref()
                                        .and_then(|p| p.get(i))
                                        .map(|v| v.to_string())
                                        .unwrap_or_default()
                                };
                                let sev = entry
                                    .as_ref()
                                    .and_then(|p| p.get(2))
                                    .map(|v| v.to_string())
                                    .filter(|w| !w.is_empty())
                                    .map(|w| {
                                        ui::sev_of(&w).unwrap_or_else(ui::sev_fallback)
                                    });
                                ui::RowItem { label: get(0), value: get(1), sev }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let row_h = met.rows_line_h(m);
                let h = row_h * rows_lines(m) as f32;
                let st = ui::RowsStyle {
                    // The two halves of a row are the theme's, on every
                    // panel: a `rows` line is one KIND of thing, so it is
                    // set at one size, and the call has no say. The
                    // options that used to overrule these were how the
                    // same line came to read at 1.77u in one widget and
                    // 3.25u in the next.
                    label_role: ui::bound_role(&LABEL_ROLE, "script.rows_label_role"),
                    value_role: ui::bound_role(&VALUE_ROLE, "script.rows_value_role"),
                    columns: int_of(m, "columns", 1).max(1) as usize,
                    label_width: if str_of(m, "label_width") == "max" {
                        ui::LabelWidth::Max
                    } else {
                        ui::LabelWidth::Auto
                    },
                    row_h,
                    shrink: scale,
                };
                ui::rows_label_value(ctx, Rect::new(r.x, y, r.w, h), &items, &st);
                y += h;
            }
            "text" => {
                let content = str_of(m, "content");
                let role = text_role_of(m);
                let fpx = role.px(ctx, scale);
                let spacing = role.tracking_px(fpx);
                // The face is the ROLE's too (`type.<role>.face`), not
                // this file's: a widget that wants its readings in a
                // fixed-width family says so in the theme, on every panel
                // at once, and never here.
                let font = role.font();
                // The role's figure box (§5.17). The clock's date line is
                // a `text` element in a tabular role, and a date drawn
                // proportionally moves sideways when the day rolls over
                // from a 1 to a 2 — the same jitter the clock itself had.
                let fig = role.figures(ctx.fonts, font, fpx);
                let color = match sev_opt(m, "severity") {
                    Some(s) => ui::sev_text(s),
                    // A named role writes in its own ink; the older forms
                    // keep the component colour they always had.
                    None if m.contains_key("role") => role.color(),
                    None => {
                        static VALUE: OnceLock<TokenId> = OnceLock::new();
                        col(&VALUE, "component.script.value")
                    }
                };
                let align = match str_of(m, "align").as_str() {
                    "right" => Align::Right,
                    "center" => Align::Center,
                    "left" => Align::Left,
                    // A script that names no alignment gets the theme's.
                    _ => theme_text_align(),
                };
                match align {
                    Align::Left => {
                        ctx.dl.text_fig(
                            ctx.fonts, font, fpx, r.x, y, &content, color, spacing, &fig,
                        );
                    }
                    Align::Right => {
                        ctx.dl.text_right_fig(
                            ctx.fonts, font, fpx, r.right(), y, &content, color, spacing, &fig,
                        );
                    }
                    Align::Center => {
                        ctx.dl.text_center_fig(
                            ctx.fonts, font, fpx, r.cx(), y, &content, color, spacing, &fig,
                        );
                    }
                }
                y += role.px(ctx, 1.0) * met.text_leading * scale;
            }
            "runs" => {
                let items: Vec<ui::Run> = m
                    .get("items")
                    .and_then(|v| v.read_lock::<Array>())
                    .map(|a| {
                        a.iter()
                            .filter_map(|it| {
                                let im = it.read_lock::<Map>()?;
                                Some(ui::Run {
                                    text: str_of(&im, "t"),
                                    role: run_role(&im),
                                    sev: sev_opt(&im, "severity"),
                                    blink: Some(str_of(&im, "blink"))
                                        .filter(|b| !b.is_empty()),
                                    end: str_of(&im, "align") == "right",
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let h = runs_px(ctx, m) * met.text_leading * scale;
                let align = match str_of(m, "align").as_str() {
                    "right" => Align::Right,
                    "center" => Align::Center,
                    "left" => Align::Left,
                    _ => theme_text_align(),
                };
                ui::runs(ctx, Rect::new(r.x, y, r.w, h), &items, align, scale);
                y += h;
            }
            "rule" => {
                ui::rule(ctx, Rect::new(r.x, y, r.w, met.rule_block));
                y += met.rule_block;
            }
            "badge" => {
                // u2 §2.8's STATE line: an optional `label` opt puts a
                // rows-style label on the badge's line and the pill at the
                // right edge, so a key:value line may carry a pill as its
                // value — `STATE   [ ONLINE ]`, both strings exactly what
                // the rows line showed. Without a label the pill stands
                // alone, aligned by the theme, as before.
                let label = str_of(m, "label");
                let align = if label.is_empty() {
                    theme_text_align()
                } else {
                    static LABEL_ROLE: OnceLock<TokenId> = OnceLock::new();
                    static LABEL_C: OnceLock<TokenId> = OnceLock::new();
                    let role = ui::bound_role(&LABEL_ROLE, "script.rows_label_role");
                    let lpx = role.px(ctx, scale);
                    let lsp = role.tracking_px(lpx);
                    // Centred by the role's OWN line height and the
                    // theme's centring mode — the same primitive every
                    // other object on this line uses. It used to guess a
                    // cap height of 1.3 here, which was a look no theme
                    // file could account for.
                    let ty = ui::center_line_y(ctx, role.font(), y, met.row_h, lpx, role.leading());
                    // The label is the rows label, in the rows label's
                    // role: the same string on the same line above or
                    // below a `rows` element must be set the same way,
                    // figure box included.
                    let fig = role.figures(ctx.fonts, role.font(), lpx);
                    ctx.dl.text_fig(
                        ctx.fonts, role.font(), lpx, r.x, ty, &label,
                        col(&LABEL_C, "component.script.label"), lsp, &fig,
                    );
                    Align::Right
                };
                ui::badge(
                    ctx,
                    Rect::new(r.x, y, r.w, met.row_h),
                    &str_of(m, "text"),
                    sev_opt(m, "severity"),
                    match str_of(m, "style").as_str() {
                        "solid" => ui::BadgeStyle::Solid,
                        "hollow" | "outlined" => ui::BadgeStyle::Hollow,
                        _ => ui::BadgeStyle::FromTheme,
                    },
                    align,
                    scale,
                );
                y += met.row_h;
            }
            "group" => {
                ui::group_header(
                    ctx,
                    Rect::new(r.x, y, r.w, met.row_h),
                    &str_of(m, "label"),
                    scale,
                );
                y += met.row_h;
                let children: Vec<Map> = m
                    .get("elements")
                    .and_then(|v| v.read_lock::<Array>())
                    .map(|a| a.iter().filter_map(|e| e.clone().try_cast::<Map>()).collect())
                    .unwrap_or_default();
                y = draw_stack(ctx, r, y, &children, p, v);
            }
            "meter" => {
                static LABEL: OnceLock<TokenId> = OnceLock::new();
                static VALUE: OnceLock<TokenId> = OnceLock::new();
                static LABEL_GAP: OnceLock<TokenId> = OnceLock::new();
                static VALUE_GAP: OnceLock<TokenId> = OnceLock::new();
                static BAR_H: OnceLock<TokenId> = OnceLock::new();
                static LABEL_ROLE: OnceLock<TokenId> = OnceLock::new();
                static VALUE_ROLE: OnceLock<TokenId> = OnceLock::new();
                let label = str_of(m, "label");
                let value = str_of(m, "value");
                let f = f32_of(m, "fraction", 0.0);
                // Both strings are ROLES, the way `text` and `rows` take
                // theirs: the size and the tracking come from whatever
                // role the binding names, so retyping the label is a
                // theme's decision and not a rewrite here.
                let label_role = ui::bound_role(&LABEL_ROLE, "script.meter_label_role");
                let value_role = ui::bound_role(&VALUE_ROLE, "script.meter_value_role");
                let lpx = label_role.px(ctx, scale);
                let vpx = value_role.px(ctx, scale);
                let lsp = label_role.tracking_px(lpx);
                let vsp = value_role.tracking_px(vpx);
                // Each role's figure box, resolved once and used by BOTH
                // the measuring and the drawing below. A meter's readout
                // is a live number — SWAP's bytes change while the panel
                // stands still — so the box is what keeps the bar's right
                // edge from breathing in and out with the digits; and a
                // width measured proportionally under a run drawn
                // tabularly would leave the bar overlapping the number.
                // The face each role names, for the reason the `text`
                // element above gives.
                let lfont = label_role.font();
                let vfont = value_role.font();
                let lfig = label_role.figures(ctx.fonts, lfont, lpx);
                let vfig = value_role.figures(ctx.fonts, vfont, vpx);
                let lw = ctx.fonts.measure_fig(lfont, lpx, &label, lsp, &lfig)
                    + t.px(tok(&LABEL_GAP, "meter.label_gap")) * scale;
                let vw = ctx.fonts.measure_fig(vfont, vpx, &value, vsp, &vfig)
                    + t.px(tok(&VALUE_GAP, "meter.value_gap")) * scale;
                // Each string centres on ITS OWN size and its own line
                // height: two roles on one line sit on one axis only if
                // each is measured by its own. The primitive is the
                // shared one, so the theme's centring mode reaches here
                // as it reaches every other line in the program.
                let lty = ui::center_line_y(ctx, lfont, y, met.row_h, lpx, label_role.leading());
                let vty = ui::center_line_y(ctx, vfont, y, met.row_h, vpx, value_role.leading());
                ctx.dl.text_fig(
                    ctx.fonts, lfont, lpx, r.x, lty, &label,
                    col(&LABEL, "component.script.label"), lsp, &lfig,
                );
                // `meter.bar_align` says where the bar stands in its row.
                // The offset used to be `script.meter_track_h`, a token
                // the master describes as a HEIGHT — so the one key that
                // was about this placement was unread and a key about
                // something else was doing its job.
                static BAR_ALIGN: OnceLock<TokenId> = OnceLock::new();
                let bar_h = t.px(tok(&BAR_H, "script.meter_bar_h")) * scale;
                let row = Rect::new(r.x, y, r.w, met.row_h);
                let vy = ui::vy_of(&ui::theme_word(tok(&BAR_ALIGN, "meter.bar_align")));
                let bar = Rect::new(
                    r.x + lw,
                    ui::block_top_aligned(&row, bar_h, vy),
                    (r.w - lw - vw).max(1.0),
                    bar_h,
                );
                // ui::meter reads its own track and fill; the element
                // only says where the bar sits, how full it is, and — the
                // script's judgement — how it stands (u2 §3.1 #6).
                ui::meter(ctx, bar, f, sev_opt(m, "severity"), bool_of(m, "track", true));
                ctx.dl.text_right_fig(
                    ctx.fonts, vfont, vpx, r.right(), vty, &value,
                    col(&VALUE, "component.script.value"), vsp, &vfig,
                );
                y += met.row_h;
            }
            "gauges" => {
                let values: Vec<f32> = m
                    .get("values")
                    .and_then(|v| v.read_lock::<Array>())
                    .map(|a| {
                        a.iter()
                            .map(|v| v.as_float().unwrap_or(0.0) as f32)
                            .collect()
                    })
                    .unwrap_or_default();
                static COLS: OnceLock<TokenId> = OnceLock::new();
                static STYLE: OnceLock<TokenId> = OnceLock::new();
                let cols = m
                    .get("columns")
                    .and_then(|v| v.as_int().ok())
                    .unwrap_or_else(|| t.px(tok(&COLS, "gauge.cols")) as i64)
                    .clamp(1, 16) as usize;
                // The form: the script's arrangement choice, defaulting to
                // the theme's `gauge.style`. `bar` and `donut` cannot yet
                // carry the per-core number they owe, so they degrade to
                // `row` with one warning — a stated fallback, never a
                // silent content drop (u2 §2.5).
                let style_word = {
                    let w = str_of(m, "style");
                    if w.is_empty() {
                        ui::theme_word(tok(&STYLE, "gauge.style"))
                    } else {
                        w
                    }
                };
                let kind = match style_word.as_str() {
                    "row" => ui::GaugeKind::Row,
                    "cell" | "" => ui::GaugeKind::Cell,
                    "bar" | "donut" => {
                        ui::warn_once(
                            "gauges.style",
                            &format!(
                                "gauge style \"{style_word}\" cannot carry its value \
                                 labels yet — drawing rows instead"
                            ),
                        );
                        ui::GaugeKind::Row
                    }
                    other => {
                        ui::warn_once(
                            "gauges.style",
                            &format!("unknown gauge style \"{other}\" — drawing cells"),
                        );
                        ui::GaugeKind::Cell
                    }
                };
                let labels = match m.get("label") {
                    Some(v) if v.is_array() => ui::GaugeLabels::Text(
                        v.read_lock::<Array>()
                            .map(|a| a.iter().map(|x| x.to_string()).collect())
                            .unwrap_or_default(),
                    ),
                    Some(v) => {
                        let w = v.to_string();
                        if w.is_empty() {
                            ui::GaugeLabels::None
                        } else {
                            ui::GaugeLabels::Index(w)
                        }
                    }
                    None => ui::GaugeLabels::None,
                };
                let st = ui::GaugeStyle {
                    cols,
                    kind,
                    labels,
                    value_fmt: if str_of(m, "value_fmt") == "raw" {
                        ui::GaugeValueFmt::Raw
                    } else {
                        ui::GaugeValueFmt::Percent
                    },
                    shrink: scale,
                };
                // The gauges are data, not chrome — that is why [data]
                // exists; gauge_grid reads its own colours and metrics.
                ui::gauge_grid(ctx, Rect::new(r.x, y, r.w, share), &values, &st);
                y += share;
            }
            "dots" => {
                // ui::dot_matrix reads its own pitch and cell colours;
                // only the stack's shrink factor travels with the call,
                // so the pitch shrinks in step with everything else.
                ui::dot_matrix(
                    ctx,
                    Rect::new(r.x, y, r.w, share),
                    f32_of(m, "fraction", 0.0),
                    scale,
                );
                y += share;
            }
            "table" => {
                let cols: Vec<ui::Column> = m
                    .get("headings")
                    .and_then(|v| v.read_lock::<Array>())
                    .map(|a| {
                        a.iter()
                            .map(|h| {
                                let entry = h.read_lock::<Array>();
                                let name = entry
                                    .as_ref()
                                    .and_then(|p| p.first())
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
                                let al = entry
                                    .as_ref()
                                    .and_then(|p| p.get(1))
                                    .map(|v| v.to_string())
                                    .unwrap_or_default();
                                let opts = entry
                                    .as_ref()
                                    .and_then(|p| p.get(2))
                                    .and_then(|v| v.clone().try_cast::<Map>());
                                let kind = match opts.as_ref().map(|o| str_of(o, "kind")) {
                                    Some(k) if k == "bar" => ui::CellKind::Bar {
                                        of: opts
                                            .as_ref()
                                            .map(|o| f32_of(o, "of", 100.0))
                                            .unwrap_or(100.0),
                                    },
                                    Some(k) if k == "badge" => ui::CellKind::Badge,
                                    _ => ui::CellKind::Text,
                                };
                                // Content-measured widths are the default
                                // (u2 §2.7); `heading` keeps the old rule.
                                let width = match opts.as_ref().map(|o| str_of(o, "width")) {
                                    Some(w) if w == "heading" => ui::ColWidth::Heading,
                                    _ => ui::ColWidth::Content,
                                };
                                ui::Column { title: name, align: align_of(&al), kind, width }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let rows: Vec<Vec<String>> = m
                    .get("rows")
                    .and_then(|v| v.read_lock::<Array>())
                    .map(|a| {
                        a.iter()
                            .map(|row| {
                                row.read_lock::<Array>()
                                    .map(|c| c.iter().map(|v| v.to_string()).collect())
                                    .unwrap_or_default()
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let st = ui::TableStyle {
                    elastic: int_of(m, "elastic", 0).max(0) as usize,
                    zebra: bool_of(m, "zebra", false),
                    severity_col: m
                        .get("severity_col")
                        .and_then(|v| v.as_int().ok())
                        .filter(|i| *i >= 0)
                        .map(|i| i as usize),
                    shrink: scale,
                };
                // The interactive options (F2 §2.2). Every one of them
                // is OFF by default, so a script written before this
                // phase draws through the same path it always did.
                let interactive = bool_of(m, "interactive", false);
                let select = str_of(m, "select") == "row";
                let scroll = bool_of(m, "scroll", false);
                // A tooltip needs no state of its own, but it does need
                // the view path — the plain `ui::table` has nowhere to
                // file a request from — so it opens that path like the
                // rest of them.
                let tooltip = bool_of(m, "tooltip", false);
                let rect = Rect::new(r.x, y, r.w, share);
                if interactive || select || scroll || tooltip {
                    let key_col = m
                        .get("key")
                        .and_then(|v| v.as_int().ok())
                        .filter(|i| *i >= 0)
                        .map(|i| i as usize);
                    // The script's OPENING arrangement — read once, when
                    // the state is made. After that the user's clicks
                    // own it, or a script would undo them every frame.
                    let opening = m
                        .get("sort")
                        .and_then(|v| v.as_int().ok())
                        .filter(|i| *i >= 0)
                        .map(|i| {
                            (i as usize, view::SortDir::from_word(&str_of(m, "dir")))
                        });
                    let (id, ordinal) = v.claim(&str_of(m, "id"));
                    let generation = v.generation;
                    let ViewState { tables, hits, .. } = &mut *v.state;
                    let state = tables.entry(id).or_insert_with(|| {
                        let mut s = view::TableState::new();
                        s.sort = opening;
                        s
                    });
                    ui::table_view(
                        ctx,
                        rect,
                        &cols,
                        &rows,
                        &st,
                        ui::TableView {
                            state,
                            hits,
                            id: ordinal,
                            generation,
                            interactive,
                            select,
                            key_col,
                            scroll,
                            tooltip,
                        },
                    );
                } else {
                    ui::table(ctx, rect, &cols, &rows, &st);
                }
                y += share;
            }
            "list" => {
                let items: Vec<view::RowBuf> = m
                    .get("items")
                    .and_then(|v| v.read_lock::<Array>())
                    .map(|a| a.iter().map(list_item).collect())
                    .unwrap_or_default();
                let select = str_of(m, "select") == "row";
                let scroll = bool_of(m, "scroll", false);
                // The table's option, on the element that trims the most
                // text of any: a row's name is what the ellipsis reaches
                // first (F2 §8.1). It needs the view path for the same
                // reason the table's does — the plain call has nowhere to
                // file a request from.
                let tooltip = bool_of(m, "tooltip", false);
                // A list that does not scroll took a FIXED height in the
                // measure and takes the same one here; one that does is a
                // flexible element and takes the share.
                let h = if scroll {
                    share
                } else {
                    view::list::height(met.list_row_h, met.list_gap, items.len())
                };
                let model = view::Rows::new(items).with_generation(v.generation);
                let st = view::list::ListStyle { shrink: scale };
                let rect = Rect::new(r.x, y, r.w, h);
                if select || scroll || tooltip {
                    let (id, ordinal) = v.claim(&str_of(m, "id"));
                    let ViewState { lists, hits, .. } = &mut *v.state;
                    let state = lists.entry(id).or_default();
                    view::list::list(
                        &mut view::CtxSurface::new(ctx),
                        rect,
                        &model,
                        &st,
                        Some(view::list::ListView {
                            state,
                            hits,
                            id: ordinal,
                            select,
                            scroll,
                            tree: false,
                            tooltip,
                        }),
                    );
                } else {
                    view::list::list(&mut view::CtxSurface::new(ctx), rect, &model, &st, None);
                }
                y += h;
            }
            "tree" => {
                let roots: Vec<view::tree::MemNode> = m
                    .get("nodes")
                    .and_then(|v| v.read_lock::<Array>())
                    .map(|a| a.iter().map(|n| tree_node(n, 0)).collect())
                    .unwrap_or_default();
                let select = str_of(m, "select") == "row";
                let scroll = bool_of(m, "scroll", false);
                // A tree indents, so its names are trimmed sooner than a
                // flat list's — the deeper the row, the narrower its
                // label (F2 §8.1).
                let tooltip = bool_of(m, "tooltip", false);
                let rect = Rect::new(r.x, y, r.w, share);
                let (id, ordinal) = v.claim(&str_of(m, "id"));
                let generation = v.generation;
                let ViewState { trees, hits, .. } = &mut *v.state;
                let tv = trees.entry(id).or_default();
                // The script rebuilds its nodes every frame — it is a
                // pure function of its data — and the expansion lives
                // HERE, keyed by path, so the shape the user opened
                // survives every rebuild (F2 §4).
                tv.flat
                    .set_model(view::tree::MemTree::new(roots).with_generation(generation));
                tv.flat.sync();
                view::list::list(
                    &mut view::CtxSurface::new(ctx),
                    rect,
                    &tv.flat,
                    &view::list::ListStyle { shrink: scale },
                    Some(view::list::ListView {
                        state: &mut tv.list,
                        hits,
                        id: ordinal,
                        select,
                        scroll,
                        tree: true,
                        tooltip,
                    }),
                );
                y += share;
            }
            "columns" => {
                static LABEL_ROLE: OnceLock<TokenId> = OnceLock::new();
                static VALUE_ROLE: OnceLock<TokenId> = OnceLock::new();
                let cells: Vec<ui::ColumnCell> = m
                    .get("cells")
                    .and_then(|v| v.read_lock::<Array>())
                    .map(|a| {
                        a.iter()
                            .map(|c| {
                                let entry = c.read_lock::<Array>();
                                let get = |i: usize| {
                                    entry
                                        .as_ref()
                                        .and_then(|p| p.get(i))
                                        .map(|v| v.to_string())
                                        .unwrap_or_default()
                                };
                                let sev = entry
                                    .as_ref()
                                    .and_then(|p| p.get(2))
                                    .map(|v| v.to_string())
                                    .filter(|w| !w.is_empty())
                                    .map(|w| {
                                        ui::sev_of(&w).unwrap_or_else(ui::sev_fallback)
                                    });
                                ui::ColumnCell { label: get(0), value: get(1), sev }
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let st = ui::ColumnsStyle {
                    // The theme's, not the call's — the ruling `rows`
                    // above sets out. A cell of the instrument strip and
                    // a line of a key:value block are the same two
                    // halves, and they are set by the same two bindings.
                    label_role: ui::bound_role(&LABEL_ROLE, "script.columns_label_role"),
                    value_role: ui::bound_role(&VALUE_ROLE, "script.columns_value_role"),
                    align: match str_of(m, "align").as_str() {
                        "" => None,
                        w => Some(align_of(w)),
                    },
                    dividers: bool_of(m, "dividers", false),
                    shrink: scale,
                };
                ui::columns(ctx, Rect::new(r.x, y, r.w, met.columns_block), &cells, &st);
                y += met.columns_block;
            }
            "space" => y += met.spacer * f32_of(m, "size", 1.0),
            _ => {}
        }
    }
    y
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pointer::Pointer;

    fn snapshot() -> crate::telemetry::Snapshot {
        crate::telemetry::Snapshot {
            hostname: "desktop".into(),
            uptime: 3661,
            mem_used: 2 * 1024 * 1024 * 1024,
            mem_total: 8 * 1024 * 1024 * 1024,
            cpu_per_core: vec![10.0, 20.0],
            ..Default::default()
        }
    }

    fn run(src: &str) -> Result<Array, String> {
        run_with_views(src, &ViewState::default())
    }

    /// The real scope a script draws in: `host` and `view`. A first
    /// frame's `view` is empty, which is what `run` passes.
    fn run_with_views(src: &str, views: &ViewState) -> Result<Array, String> {
        let engine = engine();
        let ast = engine.compile(src).map_err(|e| e.to_string())?;
        let snap = snapshot();
        let host = Host {
            snap: &snap,
            term: None,
            tabs: &[],
            tab_active: 0,
            shell_cwd: None,
            t: 0.0,
            window: (1280.0, 720.0),
        };
        let mut scope = Scope::new();
        scope.push_constant("host", host_shared(&host));
        scope.push_constant("view", Dynamic::from_map(views.as_map()));
        engine
            .call_fn::<Array>(&mut scope, &ast, "draw", ())
            .map_err(|e| e.to_string())
    }

    #[test]
    fn a_script_builds_elements_from_host_data() {
        let out = run(r#"
            fn draw() {
                [
                    title("UPTIME", upper(host.name)),
                    rows([["UP", uptime(host.uptime)]]),
                    meter("MEM", host.mem_fraction, bytes(host.mem_used)),
                    gauges(host.cpu_each, 2),
                ]
            }
        "#)
        .unwrap();
        assert_eq!(out.len(), 4);
        let m = out[0].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&m, "left"), "UPTIME");
        assert_eq!(str_of(&m, "right"), "DESKTOP");
        let rows = out[1].read_lock::<Map>().unwrap();
        let r = rows.get("rows").unwrap().read_lock::<Array>().unwrap();
        let first = r[0].read_lock::<Array>().unwrap();
        assert_eq!(first[1].to_string(), "01:01:01");
        let meter = out[2].read_lock::<Map>().unwrap();
        // Since 2026-08-17 the byte formatter writes its reading through
        // `[num]`, so the spelling is now the theme's — and the master
        // says `unit.case = none`, because a unit SYMBOL is not a label:
        // the small `i` of GiB is what makes it the IEC binary prefix and
        // the small `s` of MB/s is the second. A theme that wants shouty
        // units writes `upper` and gets it everywhere at once, which is
        // the whole point of the key having a reader.
        assert_eq!(str_of(&meter, "value"), "2.00 GiB");
        assert!((f32_of(&meter, "fraction", 0.0) - 0.25).abs() < 0.001);
    }

    #[test]
    fn scripts_cannot_reach_the_system_and_cannot_hang() {
        // No file, network or process functions exist in a script's world.
        for forbidden in [
            r#"fn draw() { open_file("/etc/passwd") }"#,
            r#"fn draw() { import "std::fs" as fs; [] }"#,
        ] {
            assert!(run(forbidden).is_err(), "{forbidden} should not run");
        }
        // A runaway loop is cut off rather than freezing the frame.
        let out = run(r#"fn draw() { let i = 0; while true { i += 1; } [] }"#);
        assert!(out.is_err(), "an endless loop must be stopped");
    }

    /// The six title items that MOVE to the host's band (u2 §6.1) come
    /// out of the same element the script has always answered with —
    /// same strings, same data. A script with no title, or with only
    /// the underline, declares no band.
    #[test]
    fn a_title_element_is_the_chrome_declaration() {
        let out = run(r#"
            fn draw() {
                [ title("UPTIME", "CHARGING"), rows([["UP", "01:01:01"]]) ]
            }
        "#)
        .unwrap();
        let c = chrome_of(&out);
        assert_eq!(c.title.as_deref(), Some("UPTIME"));
        assert_eq!(c.right.as_deref(), Some("CHARGING"));

        let no_right = run(r#"fn draw() { [ title("HARDWARE") ] }"#).unwrap();
        let c = chrome_of(&no_right);
        assert_eq!(c.title.as_deref(), Some("HARDWARE"));
        assert_eq!(c.right, None);

        let untitled = run(r#"fn draw() { [ text("21:57:30", "center", 2.4) ] }"#).unwrap();
        assert_eq!(chrome_of(&untitled).title, None);

        // The underline alone is a rule, not a band.
        let rule_only = run(r#"fn draw() { [ title("") ] }"#).unwrap();
        assert_eq!(chrome_of(&rule_only).title, None);
    }

    #[test]
    fn a_broken_script_is_an_error_not_a_crash() {
        assert!(run("fn draw() { this is not rhai }").is_err());
        // A script without draw() is an error, not a panic.
        assert!(run("fn other() { [] }").is_err());
    }

    /// The four NEW elements of u2 §3.1 — runs, rule, group, badge —
    /// build the maps the renderer walks, from the same host data the
    /// old vocabulary reads.
    #[test]
    fn the_four_new_elements_parse() {
        let out = run(r#"
            fn draw() {
                [
                    runs([
                        #{ t: "LOAD", role: "label" },
                        #{ t: ":", role: "clock", blink: "value_blink" },
                        #{ t: "42", role: "reading", severity: "warning" },
                        #{ t: "47°C", role: "reading", align: "right" },
                    ], "center"),
                    rule(),
                    group("SWAP", [
                        rows([["USED", "128 MiB"]]),
                    ]),
                    badge("ONLINE", #{ severity: "ok" }),
                    badge("OFFLINE"),
                ]
            }
        "#)
        .unwrap();
        assert_eq!(out.len(), 5);
        let runs = out[0].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&runs, "kind"), "runs");
        assert_eq!(str_of(&runs, "align"), "center");
        let items = runs.get("items").unwrap().read_lock::<Array>().unwrap();
        assert_eq!(items.len(), 4);
        let colon = items[1].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&colon, "blink"), "value_blink");
        // u2 §2.5's right-aligned temperature run: the item pins itself
        // to the line's right end.
        let temp = items[3].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&temp, "align"), "right");
        assert_eq!(str_of(&out[1].read_lock::<Map>().unwrap(), "kind"), "rule");
        let group = out[2].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&group, "kind"), "group");
        assert_eq!(str_of(&group, "label"), "SWAP");
        let children = group.get("elements").unwrap().read_lock::<Array>().unwrap();
        assert_eq!(children.len(), 1);
        let badge = out[3].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&badge, "kind"), "badge");
        assert_eq!(str_of(&badge, "text"), "ONLINE");
        assert_eq!(str_of(&badge, "severity"), "ok");
        // The one-argument badge carries no severity at all.
        assert_eq!(str_of(&out[4].read_lock::<Map>().unwrap(), "severity"), "");
    }

    /// u2 §2.8's STATE line: a badge may carry the row's label as an
    /// option, so a key:value line can have a pill for its value. The
    /// two strings are exactly the two the old rows line showed — the
    /// pill is presentation, never new content.
    #[test]
    fn a_badge_may_carry_its_rows_label() {
        let out = run(
            r#"fn draw() { [ badge("ONLINE", #{ label: "STATE", severity: "ok" }) ] }"#,
        )
        .unwrap();
        let b = out[0].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&b, "label"), "STATE");
        assert_eq!(str_of(&b, "text"), "ONLINE");
        assert_eq!(str_of(&b, "severity"), "ok");
    }

    /// The EXTENDED forms of u2 §3.1 are added overloads: the options
    /// ride on the element map without displacing what the old form
    /// stored, so both generations of script read back the same way.
    #[test]
    fn the_extended_forms_parse_beside_the_old_ones() {
        let out = run(r#"
            fn draw() {
                [
                    rows([["UP", "01:01:01", "ok"], ["HOST", "ORION"]],
                         #{ columns: 2, label_width: "max", density: "compact" }),
                    columns([["POWER", "87% +", "warning"]], #{ dividers: true }),
                    meter("SWAP", 0.5, "128 MiB", #{ severity: "critical", track: false }),
                    gauges(host.cpu_each, #{ columns: 2, style: "row", label: "C" }),
                    table([["PID", "right"], ["CPU", "right", #{ kind: "bar", of: 100.0 }]],
                          [["1", "41.2%", "warning"]], 0,
                          #{ zebra: true, severity_col: 2 }),
                    text("21:57:30", "center", #{ role: "clock" }),
                ]
            }
        "#)
        .unwrap();
        assert_eq!(out.len(), 6);
        let rows = out[0].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&rows, "label_width"), "max");
        assert_eq!(str_of(&rows, "density"), "compact");
        let first = rows.get("rows").unwrap().read_lock::<Array>().unwrap()[0]
            .read_lock::<Array>()
            .unwrap()
            .get(2)
            .unwrap()
            .to_string();
        assert_eq!(first, "ok");
        let cols = out[1].read_lock::<Map>().unwrap();
        assert!(cols.get("dividers").unwrap().as_bool().unwrap());
        let meter = out[2].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&meter, "severity"), "critical");
        assert!(!meter.get("track").unwrap().as_bool().unwrap());
        let gauges = out[3].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&gauges, "style"), "row");
        assert_eq!(str_of(&gauges, "label"), "C");
        let table = out[4].read_lock::<Map>().unwrap();
        assert!(table.get("zebra").unwrap().as_bool().unwrap());
        assert_eq!(table.get("severity_col").unwrap().as_int().unwrap(), 2);
        let text = out[5].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&text, "role"), "clock");
        // The named kind replaces the free size entirely.
        assert!(!text.contains_key("size"));
    }

    /// A script's own measuring stick, built the way tests/role_size_bounds
    /// builds one: no scaling of its own, so a px is the theme's number.
    fn measuring_ctx<T>(f: impl FnOnce(&Ctx) -> T) -> T {
        let mut dl = crate::draw::DrawList::new();
        let mut fonts = crate::font::FontSystem::new();
        let c = Ctx {
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
            focus: None,
            tips: None,
        };
        f(&c)
    }

    /// The kinds a script may name are the master's closed list, and a
    /// TYPE ROLE is not one of them.
    ///
    /// This is the door the old `role_opt` left open: it handed the
    /// script's word straight to `ui::role`, so `#{ role: "data" }` — or
    /// `rows(…, #{ value_role: "data" })` — chose a size at the call.
    /// Naming the same word now reaches only the sizes the MASTER binds to
    /// a kind, so the call has stopped deciding; the old spelling is
    /// carried to the kind the master already points at that role, and
    /// where no kind points at it, to `script.text_role`.
    ///
    /// The invariant is not "an old word draws nothing". It is that no
    /// word a script can write reaches a size the master has not bound to
    /// a kind — which is what stopped the value half of four panels of
    /// one kind being set at three different sizes, and which a silent
    /// hole in a third-party addon was never needed to enforce.
    #[test]
    fn a_kind_is_the_master_s_word_and_never_a_type_role() {
        measuring_ctx(|c| {
            // The kinds the shipped widgets name.
            for kind in ["clock", "date", "label", "reading", "text"] {
                assert!(
                    kind_role(kind).px(c, 1.0) > 0.0,
                    "the master binds no script.kind_{kind}_role"
                );
            }
            // Every size a script can ask for is one of the kinds', and
            // the kinds are the master's five.
            let ladder: Vec<f32> = ["clock", "date", "label", "reading", "text"]
                .iter()
                .map(|k| kind_role(k).px(c, 1.0))
                .collect();
            for role in ["data", "caption", "value", "display.clock", "value.large", "terminal"] {
                assert!(ui::role(role).px(c, 1.0) > 0.0, "type.{role} is a real role");
                let got = kind_role(role).px(c, 1.0);
                assert!(
                    ladder.contains(&got),
                    "the type role \"{role}\" reached {got} px, which no kind binds"
                );
            }
            // Where the master DOES bind a kind to the role a legacy word
            // names, the word is carried there rather than to the default:
            // `kind_reading_role = value`, so an addon still writing
            // `value` keeps the size it has always drawn at.
            assert_eq!(kind_role("value").px(c, 1.0), kind_role("reading").px(c, 1.0));
            assert_eq!(
                kind_role("display.clock").px(c, 1.0),
                kind_role("clock").px(c, 1.0)
            );
            // And a role no kind binds lands on the text role, not on its
            // own ladder: `type.data` is 10.10 px and must not be reachable.
            static TEXT: OnceLock<TokenId> = OnceLock::new();
            let text_px = ui::bound_role(&TEXT, "script.text_role").px(c, 1.0);
            assert_eq!(kind_role("data").px(c, 1.0), text_px);
            assert_ne!(kind_role("data").px(c, 1.0), ui::role("data").px(c, 1.0));
            // A word that is neither a kind nor a role does the same, so
            // a misspelling costs a line of text and not the widget.
            assert_eq!(kind_role("no such thing at all").px(c, 1.0), text_px);
            // A reading is the value half of a row, to the pixel: this is
            // the whole of what NETWORK lost 74% for.
            static VALUE: OnceLock<TokenId> = OnceLock::new();
            assert_eq!(
                kind_role("reading").px(c, 1.0),
                ui::bound_role(&VALUE, "script.rows_value_role").px(c, 1.0),
            );
        });
    }

    /// The severity words a script may use are the closed set of §5.10,
    /// and an unknown word resolves to the fallback — never to ok.
    #[test]
    fn severity_is_a_closed_set_with_a_safe_fallback() {
        for (i, name) in ui::SEVERITY_ROLES.iter().enumerate() {
            assert_eq!(ui::sev_of(name), Some(ui::Sev(i as u16)));
        }
        assert_eq!(ui::sev_of("fine"), None);
        assert_ne!(ui::sev_fallback(), ui::Sev(0), "the fallback must never be ok");
    }

    /// memory at 1280×800: two fixed rows and one flexible matrix in a
    /// panel shorter than fixed + min_flex even at the 0.62 floor. The
    /// flexible share must yield below its minimum so the SWAP meter —
    /// the last fixed element, the exact row u1 §5.5 check 4 protects —
    /// stays inside the panel instead of past the clip.
    #[test]
    fn stack_fit_yields_the_flexible_share_before_the_fixed_tail() {
        let (share, scale, clipped) = stack_fit(40.0, 45.4, 1, 28.1, 0.62, true);
        assert!(!clipped, "the fixed tail fits once the flexible yields");
        assert!(share >= 0.0 && share < 28.1, "the min_flex_h guarantee gave way");
        assert!((45.4 + share) * scale <= 40.5, "the whole stack sits inside the panel");
    }

    /// Only when the fixed rows ALONE overrun the panel at the floor may
    /// the panel clip — and then the flexible elements have nothing left.
    #[test]
    fn stack_fit_clips_only_when_the_fixed_rows_cannot_fit() {
        let (share, scale, clipped) = stack_fit(20.0, 45.4, 1, 28.1, 0.62, true);
        assert_eq!(share, 0.0);
        assert_eq!(scale, 0.62);
        assert!(clipped);
    }

    /// A panel with room keeps today's arithmetic: the flexible share is
    /// the leftover, nothing shrinks, nothing clips.
    #[test]
    fn stack_fit_leaves_a_roomy_panel_alone() {
        let (share, scale, clipped) = stack_fit(100.0, 45.4, 1, 28.1, 0.62, true);
        assert_eq!(share, 100.0 - 45.4);
        assert_eq!(scale, 1.0);
        assert!(!clipped);
    }

    /// Metrics whose gaps are told apart by sight, so a wrong token is a
    /// wrong number and not a rounding argument.
    fn gap_metrics() -> Metrics {
        Metrics {
            row_h: 0.0,
            row_compact: 0.0,
            title_block: 0.0,
            columns_block: 0.0,
            spacer: 0.0,
            rule_block: 0.0,
            group_gap: 300.0,
            element_gap: 1.0,
            meter_gap: 20.0,
            dots_gap: 100.0,
            list_row_h: 0.0,
            list_gap: 0.0,
            text_leading: 1.0,
            min_flex_h: 0.0,
        }
    }

    /// Which token pays for the air between two elements. The picture
    /// this arithmetic makes is proved against the theme in
    /// `tests/script_stack_gaps.rs`; here it is the RULE that is held
    /// still.
    #[test]
    fn an_elements_own_gap_overrides_the_implicit_one_and_two_of_them_collapse() {
        let met = gap_metrics();
        // Nothing claimed: the theme's implicit gap.
        assert_eq!(stack_gap("text", "rows", &met), met.element_gap);
        // One claim wins over the implicit gap in both directions —
        // wider or narrower, it is the more specific token.
        assert_eq!(stack_gap("text", "meter", &met), met.meter_gap);
        assert_eq!(stack_gap("meter", "text", &met), met.meter_gap);
        // Two claims collapse to the wider, so the pair is spaced once.
        assert_eq!(stack_gap("meter", "dots", &met), met.dots_gap);
        assert_eq!(stack_gap("group", "meter", &met), met.group_gap);
        // A `space` element is the gap, and is not padded with more.
        assert_eq!(stack_gap("text", "space", &met), 0.0);
        assert_eq!(stack_gap("space", "meter", &met), 0.0);
    }

    /// The view options of F2 §2.2 ride on the table's map beside the
    /// old ones, and a script that names none of them gets none — the
    /// condition for every table written before this phase drawing
    /// through the path it always did.
    #[test]
    fn the_view_options_ride_on_the_table_element() {
        let out = run(r#"
            fn draw() {
                [
                    table([["PID", "right"], ["NAME", "left"]],
                          [["1471", "firefox"]], 1,
                          #{ id: "procs", interactive: true, sort: 1, dir: "desc",
                             select: "row", key: 0, scroll: true, tooltip: true,
                             zebra: true }),
                    table([["A", "left"]], [["x"]], 0),
                ]
            }
        "#)
        .unwrap();
        let live = out[0].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&live, "id"), "procs");
        assert!(bool_of(&live, "interactive", false));
        assert!(bool_of(&live, "scroll", false));
        assert!(bool_of(&live, "zebra", false), "the old options are untouched");
        assert_eq!(int_of(&live, "sort", -1), 1);
        assert_eq!(str_of(&live, "dir"), "desc");
        assert_eq!(str_of(&live, "select"), "row");
        assert_eq!(int_of(&live, "key", -1), 0);
        assert!(bool_of(&live, "tooltip", false));
        // The plain three-argument form: not one of them is set, so
        // `draw_stack` takes the `ui::table` branch.
        let plain = out[1].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&plain, "id"), "");
        assert!(!bool_of(&plain, "interactive", false));
        assert!(!bool_of(&plain, "scroll", false));
        assert_eq!(str_of(&plain, "select"), "");
        assert!(!bool_of(&plain, "tooltip", false));
    }

    /// The table's tooltip option reaches the two elements that trim the
    /// most text of anything the renderer draws: a list row's name, and
    /// a tree row's name, narrowed further by every level of its indent
    /// (F2 §8.1).
    #[test]
    fn a_list_and_a_tree_can_be_asked_to_explain_their_trimmed_names() {
        let out = run(r#"
            fn draw() {
                [
                    list(["org.freedesktop.NetworkManager"],
                         #{ id: "svc", tooltip: true }),
                    tree([#{ label: "usr", children: [#{ label: "share" }] }],
                         #{ tooltip: true }),
                    list(["short"]),
                ]
            }
        "#)
        .unwrap();
        assert!(bool_of(&out[0].read_lock::<Map>().unwrap(), "tooltip", false));
        assert_eq!(str_of(&out[0].read_lock::<Map>().unwrap(), "id"), "svc");
        assert!(bool_of(&out[1].read_lock::<Map>().unwrap(), "tooltip", false));
        // Off unless it is named, like every other view option: a list
        // written before this phase is a fixed block of rows still.
        assert!(!bool_of(&out[2].read_lock::<Map>().unwrap(), "tooltip", false));
    }

    /// The `view` constant is the other half of the conversation: the
    /// script writes options, the user's clicks write state, and the
    /// script reads that state back on the next frame.
    #[test]
    fn a_script_reads_its_views_back_through_the_view_constant() {
        let mut views = ViewState::default();
        let mut t = view::TableState::new();
        t.select(Some("1471".into()));
        t.click_head(1);
        t.click_head(1);
        t.scroll.set_offset(312.0);
        views.tables.insert("procs".into(), t);

        let out = run_with_views(
            r#"
            fn draw() {
                let v = view.procs;
                [ text(`${v.selected}/${v.sort}/${v.dir}/${v.scroll}`) ]
            }
        "#,
            &views,
        )
        .unwrap();
        let m = out[0].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&m, "content"), "1471/1/desc/312.0");

        // A first frame has no state at all, and a script asking for a
        // view it has not drawn yet must get nothing rather than an
        // error — the widget would go quiet for good.
        let empty = run_with_views(
            r#"fn draw() { [ text(if view.procs == () { "none" } else { "some" }) ] }"#,
            &ViewState::default(),
        )
        .unwrap();
        assert_eq!(
            str_of(&empty[0].read_lock::<Map>().unwrap(), "content"),
            "none"
        );
    }

    /// A view named by the script keeps its state under that name; one
    /// that is not is known by its place in the answer, and two of those
    /// are counted so the widget can say so once.
    #[test]
    fn a_view_without_an_id_is_known_by_its_place() {
        let mut state = ViewState::default();
        state.begin((0.0, 0.0));
        let mut pass = ViewPass { state: &mut state, generation: 7, unnamed: 0 };
        assert_eq!(pass.claim("procs"), ("procs".to_string(), 0));
        assert_eq!(pass.claim(""), ("1".to_string(), 1));
        assert_eq!(pass.claim(""), ("2".to_string(), 2));
        assert_eq!(pass.unnamed, 2, "two views the script did not name");
        assert_eq!(state.ids, vec!["procs", "1", "2"]);
        // The ordinal a Hit carries finds the state back.
        state.tables.insert("1".into(), view::TableState::new());
        assert!(state.table_of(1).is_some());
        assert!(state.table_of(0).is_none(), "no state made for `procs` yet");
        assert!(state.table_of(9).is_none(), "an ordinal from a stale frame");
    }

    /// The element cache is keyed by the frame AND by what the user has
    /// done: a click inside a frame must not be answered with the list
    /// from before it.
    #[test]
    fn the_interaction_epoch_is_part_of_the_element_cache_key() {
        let mut state = ViewState::default();
        assert_eq!(state.epoch(), 0);
        let mut t = view::TableState::new();
        t.click_head(0);
        let after_click = t.interact_epoch;
        assert!(after_click > 0);
        state.tables.insert("a".into(), t);
        assert_eq!(state.epoch(), after_click);
        // A second view's interactions count too, or a click on one
        // table would be answered with an answer built for the other.
        let mut u = view::TableState::new();
        u.select(Some("x".into()));
        state.tables.insert("b".into(), u);
        assert!(state.epoch() > after_click);
    }

    // ------------------------------------------------------ list / tree

    /// Every shorthand a `list` item may be written in, and the map form
    /// that carries the rest. The key falls back to the label, because a
    /// row selected by string has to have one.
    #[test]
    fn a_list_item_is_read_in_every_form_the_element_accepts() {
        let out = run(r#"
            fn draw() {
                [ list([
                    "plain",
                    [ "arrayed" ],
                    [ "with", "status" ],
                    #{ label: "full", status: "done", severity: "warning",
                       bar: 0.25, key: "k7" },
                    #{ label: "int bar", bar: 1 },
                ]) ]
            }
        "#).unwrap();
        let m = out[0].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&m, "kind"), "list");
        let items = m.get("items").unwrap().read_lock::<Array>().unwrap();
        assert_eq!(items.len(), 5);
        let rows: Vec<view::RowBuf> = items.iter().map(list_item).collect();
        assert_eq!(rows[0].label, "plain");
        assert_eq!(rows[0].key, "plain", "the key falls back to the label");
        assert_eq!(rows[1].label, "arrayed");
        assert_eq!(rows[2].label, "with");
        assert_eq!(rows[2].status, "status");
        assert_eq!(rows[3].key, "k7");
        assert_eq!(rows[3].status, "done");
        assert_eq!(rows[3].severity, ui::sev_of("warning"));
        assert_eq!(rows[3].bar, Some(0.25));
        assert_eq!(rows[4].bar, Some(1.0), "`1` is a number too");
        assert_eq!(rows[0].bar, None, "a row that states no fraction gets no bar");
    }

    /// The old, argument-only forms of every element this phase touched
    /// still answer exactly what they answered — the options are an
    /// addition, never a replacement.
    #[test]
    fn a_list_without_options_declares_none() {
        let out = run(r#"fn draw() { [ list(["a", "b"]) ] }"#).unwrap();
        let m = out[0].read_lock::<Map>().unwrap();
        assert_eq!(list_len(&m), 2);
        assert!(!m.contains_key("select") && !m.contains_key("scroll"));
        assert_eq!(str_of(&m, "id"), "");
        // Without `scroll` a list is a FIXED block: as many rows as it
        // has, at the row height, with the gaps between them.
        assert_eq!(view::list::height(20.0, 2.0, list_len(&m)), 42.0);
    }

    #[test]
    fn a_tree_element_nests_and_the_renderer_flattens_it() {
        use view::RowModel as _;
        let out = run(r#"
            fn draw() {
                [ tree([
                    #{ label: "usr", children: [
                        #{ label: "share", children: [ #{ label: "fonts" } ] },
                        #{ label: "lib" },
                    ] },
                    #{ label: "etc", severity: "warning" },
                ], #{ id: "fs", select: "row" }) ]
            }
        "#).unwrap();
        let m = out[0].read_lock::<Map>().unwrap();
        assert_eq!(str_of(&m, "kind"), "tree");
        assert_eq!(str_of(&m, "id"), "fs");
        assert_eq!(str_of(&m, "select"), "row");
        let roots: Vec<view::tree::MemNode> = m
            .get("nodes")
            .unwrap()
            .read_lock::<Array>()
            .unwrap()
            .iter()
            .map(|n| tree_node(n, 0))
            .collect();
        let mut flat = view::FlatTree::new(view::tree::MemTree::new(roots));
        flat.sync();
        assert_eq!(flat.len(), 2, "a tree opens closed");
        flat.expand("usr");
        flat.expand("usr/share");
        flat.sync();
        let paths: Vec<String> = (0..flat.len()).map(|i| flat.key(i)).collect();
        assert_eq!(paths, vec!["usr", "usr/share", "usr/share/fonts", "usr/lib", "etc"]);
    }

    /// The requirement of §4, at the level the script actually works at:
    /// the script is a pure function of its data and hands over a WHOLE
    /// new set of nodes every frame, and neither the expansion nor the
    /// selection may notice.
    #[test]
    fn expanding_a_tree_survives_the_script_rebuilding_its_nodes() {
        use view::RowModel as _;
        fn nodes(src: &str) -> Vec<view::tree::MemNode> {
            let out = run(src).unwrap();
            let m = out[0].clone().try_cast::<Map>().unwrap();
            let arr = m.get("nodes").unwrap().clone().try_cast::<Array>().unwrap();
            arr.iter().map(|n| tree_node(n, 0)).collect()
        }
        let first = r#"
            fn draw() {
                [ tree([
                    #{ label: "usr", children: [
                        #{ label: "share", children: [ #{ label: "fonts" } ] },
                    ] },
                    #{ label: "etc" },
                ]) ]
            }
        "#;
        // The same shape a moment later: a new leaf under `share`, a new
        // status on `etc` — the everyday case of a widget refreshing.
        let second = r#"
            fn draw() {
                [ tree([
                    #{ label: "usr", children: [
                        #{ label: "share", children: [
                            #{ label: "fonts" }, #{ label: "icons" },
                        ] },
                    ] },
                    #{ label: "etc", status: "changed" },
                ]) ]
            }
        "#;
        let mut tv = TreeView::default();
        tv.flat.set_model(view::tree::MemTree::new(nodes(first)).with_generation(1));
        tv.flat.sync();
        tv.flat.expand("usr");
        tv.flat.expand("usr/share");
        tv.flat.sync();
        tv.list.select(Some("usr/share/fonts".into()));
        let epoch = tv.list.interact_epoch;
        let before: Vec<String> = (0..tv.flat.len()).map(|i| tv.flat.key(i)).collect();
        assert_eq!(before, vec!["usr", "usr/share", "usr/share/fonts", "etc"]);

        // The refresh — exactly what `draw_stack` does every frame.
        tv.flat.set_model(view::tree::MemTree::new(nodes(second)).with_generation(2));
        tv.flat.sync();
        let after: Vec<String> = (0..tv.flat.len()).map(|i| tv.flat.key(i)).collect();
        assert_eq!(
            after,
            vec!["usr", "usr/share", "usr/share/fonts", "usr/share/icons", "etc"],
            "the tree stayed open exactly where it was"
        );
        assert!(tv.list.is_selected("usr/share/fonts"), "and the row stayed picked");
        assert_eq!(tv.list.interact_epoch, epoch, "a refresh is not an interaction");
        let mut buf = view::RowBuf::new();
        tv.flat.row(4, &mut buf);
        assert_eq!(buf.status, "changed", "but the DATA is the new data");

        // Collapsing takes the descendants away and leaves the rest
        // alone, including a selection that is no longer on screen: the
        // user has not stopped having picked it.
        tv.flat.collapse("usr");
        tv.flat.sync();
        let closed: Vec<String> = (0..tv.flat.len()).map(|i| tv.flat.key(i)).collect();
        assert_eq!(closed, vec!["usr", "etc"]);
        assert!(tv.list.is_selected("usr/share/fonts"));
        tv.flat.expand("usr");
        tv.flat.sync();
        assert_eq!(tv.flat.len(), 5, "and reopening puts the whole shape back");
    }

    #[test]
    fn a_script_reads_its_lists_and_trees_back_through_the_view_constant() {
        let mut views = ViewState::default();
        let mut l = view::ListState::new();
        l.select(Some("beta".into()));
        l.scroll.set_offset(48.0);
        views.lists.insert("tasks".into(), l);
        let mut tv = TreeView::default();
        tv.list.select(Some("usr/lib".into()));
        tv.flat.set_expansion(["usr".to_string(), "usr/share".to_string()]);
        views.trees.insert("fs".into(), tv);

        let out = run_with_views(
            r#"
            fn draw() {
                [ text(`${view.tasks.selected}/${view.tasks.scroll}`),
                  text(`${view.fs.selected}/${view.fs.expanded}`) ]
            }
        "#,
            &views,
        )
        .unwrap();
        assert_eq!(
            str_of(&out[0].read_lock::<Map>().unwrap(), "content"),
            "beta/48.0"
        );
        assert_eq!(
            str_of(&out[1].read_lock::<Map>().unwrap(), "content"),
            "usr/lib/[\"usr\", \"usr/share\"]"
        );
    }

    /// Every view's interactions count toward the element cache's key,
    /// whichever family the view belongs to.
    #[test]
    fn a_list_or_a_tree_moves_the_interaction_epoch_too() {
        let mut state = ViewState::default();
        assert_eq!(state.epoch(), 0);
        let mut l = view::ListState::new();
        l.select(Some("x".into()));
        state.lists.insert("a".into(), l);
        let after_list = state.epoch();
        assert!(after_list > 0);
        let mut tv = TreeView::default();
        tv.list.select(Some("y".into()));
        state.trees.insert("b".into(), tv);
        assert!(state.epoch() > after_list);
        // And the ordinal a Hit carries finds either family back.
        state.ids = vec!["a".into(), "b".into()];
        assert!(state.list_of(0).is_some());
        assert!(state.list_of(1).is_some(), "a tree scrolls as a list");
        assert!(state.tree_of(0).is_none(), "but only a tree has an expander");
        assert!(state.tree_of(1).is_some());
        assert!(state.table_of(0).is_none());
    }
}
