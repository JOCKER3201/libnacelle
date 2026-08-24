//! One ladder: the same kind of thing is the same size in every widget.
//!
//! Eight scripted widgets drew the value half of a key:value line at three
//! different sizes, and every one of the three was chosen at the CALL:
//!
//!   network.rhai   rows(…, #{ value_role: "data"    })   1.87u
//!   hardware.rhai  rows(…, #{ value_role: "data"    })   1.87u
//!   memory.rhai    rows(…, #{ value_role: "caption" })   1.77u
//!   uptime.rhai    rows(…)                               3.25u
//!
//! — so MEMORY's headline figure, the amount of memory in use, was set
//! smaller than its own swap readout underneath it, and NETWORK's address
//! was 74% short of UPTIME's uptime. None of it was a theme's decision and
//! no theme file could undo any of it.
//!
//! What follows is the rule that replaced those options, measured rather
//! than described: the slot a string sits in decides the role, the role
//! decides the size, and every binding that names the same kind of slot
//! lands on the same number. The numbers themselves are NOT written here —
//! a theme is free to move the whole ladder, and this file would still
//! pass. What it does not allow is two rungs where the master has one.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::theme::TokenId;
use nacelle::ui;
use nacelle::Ctx;
use std::sync::OnceLock;

/// Runs one question on a thread of its own, for the reason
/// tests/role_bindings_chrome.rs sets out: the toolkit memoises a resolved
/// role per thread against the theme epoch, so the answers must not be
/// gathered across a reload. Nothing here reloads, and the isolation keeps
/// it that way should a neighbour ever do so.
fn fresh<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| s.spawn(f).join().expect("the measuring thread panicked"))
}

/// The resolved px of whatever role a binding token names, at no scaling
/// of its own — the master's own number for that slot.
fn px_of(binding: &'static str) -> f32 {
    fresh(|| {
        let mut dl = DrawList::new();
        let mut fonts = FontSystem::new();
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
        // `bound_role` memoises the token id in a cell the CALL SITE
        // owns, which a loop over binding names cannot give it. One
        // leaked cell per question costs a pointer and asks the engine
        // afresh every time — the memo is a draw-path economy and this
        // is not a draw path.
        let cell: &'static OnceLock<TokenId> = Box::leak(Box::new(OnceLock::new()));
        ui::bound_role(cell, binding).px(&c, 1.0)
    })
}

/// The value half of a key:value line, wherever one is drawn. A `rows`
/// line, a `columns` cell, a `meter`'s readout and a `gauge`'s readout are
/// the same statement — this is the measurement — and the four bindings
/// that set them must land on one role.
const VALUE_SLOTS: [&str; 4] = [
    "script.rows_value_role",
    "script.columns_value_role",
    "script.meter_value_role",
    "gauge.value_role",
];

/// The word that names the reading beside it, wherever one is drawn.
const LABEL_SLOTS: [&str; 4] = [
    "script.rows_label_role",
    "script.columns_label_role",
    "script.meter_label_role",
    "gauge.label_role",
];

#[test]
fn every_value_slot_in_the_master_is_one_size() {
    let px = px_of(VALUE_SLOTS[0]);
    assert!(px > 0.0, "{} resolves to no role at all", VALUE_SLOTS[0]);
    for slot in VALUE_SLOTS {
        assert_eq!(
            px_of(slot),
            px,
            "{slot} sets a value at a size of its own — the ladder has two rungs where \
             the master has one, which is what let four panels of one kind read at three \
             different sizes"
        );
    }
}

#[test]
fn every_label_slot_in_the_master_is_one_size() {
    let px = px_of(LABEL_SLOTS[0]);
    assert!(px > 0.0, "{} resolves to no role at all", LABEL_SLOTS[0]);
    for slot in LABEL_SLOTS {
        assert_eq!(px_of(slot), px, "{slot} sets a label at a size of its own");
    }
}

#[test]
fn a_label_is_smaller_than_the_value_it_names() {
    // The one relation between the two rungs that is not a theme's free
    // choice: a key set larger than its own value inverts the reading
    // order of every panel at once. MEMORY shipped exactly that
    // inversion — a caption-sized headline over a value-sized aside.
    assert!(
        px_of(LABEL_SLOTS[0]) < px_of(VALUE_SLOTS[0]),
        "the label rung is not below the value rung"
    );
}

// ------------------------------------------------------- the kind vocabulary

#[test]
fn a_kind_a_script_names_lands_on_the_slot_it_belongs_to() {
    // A `runs` item that says it is a reading is set exactly as the value
    // half of a `rows` line is: "LOAD 0.42 0.38 0.31" and "IPV4
    // 10.0.0.4" are the same sentence in two arrangements, and the
    // widgets used to write them 74% apart.
    assert_eq!(
        px_of("script.kind_reading_role"),
        px_of("script.rows_value_role"),
        "a reading on a line of runs is not the size of a reading in a row"
    );
    assert_eq!(
        px_of("script.kind_label_role"),
        px_of("script.rows_label_role"),
        "a label on a line of runs is not the size of a label in a row"
    );
    assert_eq!(
        px_of("script.kind_text_role"),
        px_of("script.text_role"),
        "prose in a run is not the size of prose in a text element"
    );
}

#[test]
fn the_clock_is_the_one_kind_that_stands_above_the_ladder() {
    // The exception, and it is declared rather than taken: a clock read
    // across a room is its own role, bound like every other kind, and a
    // theme moves it by editing one word.
    assert!(
        px_of("script.kind_clock_role") > px_of("script.kind_reading_role"),
        "the clock is not larger than an ordinary reading"
    );
    assert!(px_of("script.kind_date_role") > 0.0, "the date kind binds no role");
}

#[test]
fn the_vocabulary_is_closed_and_holds_no_type_role() {
    // `data`, `caption` and `value` are the three type roles the scripts
    // used to name at the call. As KINDS they are nothing: there is no
    // binding to reach them through, so the door is shut in the master
    // and not only in the code that reads it.
    for word in ["data", "caption", "value", "display.clock", "body"] {
        assert!(
            nacelle::theme::id(&format!("script.kind_{word}_role")).is_none(),
            "script.kind_{word}_role exists — a script can name the type role \
             \"{word}\" again, and the sizes will drift apart again with it"
        );
    }
    // The vocabulary itself, in full. A kind is added by declaring one
    // token here; nothing in the library learns a new word.
    for word in ["clock", "date", "label", "reading", "text"] {
        assert!(
            nacelle::theme::id(&format!("script.kind_{word}_role")).is_some(),
            "the master binds no script.kind_{word}_role"
        );
    }
}

#[test]
fn the_role_a_script_used_to_reach_for_is_still_the_smaller_one() {
    // The record of why any of this happened, kept as an assertion: the
    // `data` role — the only one carrying tabular figures that a script
    // could name — is far below the value rung. network.rhai took that
    // cut to stop an address shivering, and `type.value.tabular` is what
    // holds it still now. If a theme ever brings the two together this
    // test says so, because the argument in every comment around it stops
    // being true.
    let data = fresh(|| {
        let mut dl = DrawList::new();
        let mut fonts = FontSystem::new();
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
        (ui::role("data").px(&c, 1.0), ui::role("value").px(&c, 1.0), ui::role("value").tabular())
    });
    assert!(data.0 < data.1, "type.data is no longer the smaller role");
    assert!(data.2, "type.value.tabular is off — a reading in the value role shivers again");
}
