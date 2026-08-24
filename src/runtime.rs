//! Process-wide state, and how a plugin shares the host's copy of it.
//!
//! Some state has to exist exactly once per running program — today
//! that is the queue of sound events. In a
//! single binary a static is the obvious way to hold it.
//!
//! A compiled plugin breaks that assumption. A `.so` widget links its
//! own copy of this toolkit, and that copy has its own statics. A plugin
//! calling `sound::emit` would push into a queue the host never drains:
//! no error, no log line, the sound simply never plays.
//!
//! Failures that leave no trace are the worst kind to chase, so the
//! shared state is reached through this module rather than touched
//! directly:
//!
//! * A copy owns the state until it is told otherwise, so an ordinary
//!   application needs to do nothing at all.
//! * When the application loads a plugin it calls the plugin's exported
//!   attach point with a [`HostApi`], and the plugin's copy calls
//!   [`attach`]. From then on that copy forwards instead of keeping its
//!   own state, and the plugin's `sound::emit` reaches the host's queue.
//! * A plugin that forgets to attach would silently become its own host
//!   again, so the decision is not left to it: the loader REFUSES to
//!   load a plugin that does not export the attach point. That is the
//!   enforcement, and it lives on the side that can be trusted to run.
//!
//! `HostApi` is `#[repr(C)]` and made of plain function pointers,
//! because it crosses a dynamic library boundary where Rust's own
//! layout guarantees do not apply.

use std::ffi::c_void;
use std::sync::atomic::{AtomicBool, AtomicPtr, Ordering};

/// A colour crossing the boundary. `Color` itself is a Rust type whose
/// layout is not promised; this one is.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ColorC {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

/// A rectangle crossing the boundary.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct RectC {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// What a plugin asks the application to do, as a number and a payload,
/// because [`crate::Action`] is a Rust enum and its layout is not
/// something to rely on here. The codes match `Action`'s order.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ActionC {
    pub kind: u32,
    /// Tab index for SelectTab.
    pub index: u32,
    /// Scrollback lines for ScrollTerminal.
    pub lines: i32,
    /// Bytes or a path, owned by the plugin until the next call.
    pub data: *const u8,
    pub data_len: u32,
}

pub const ACTION_NONE: u32 = 0;
pub const ACTION_BYTES: u32 = 1;
pub const ACTION_OPEN_DIR: u32 = 2;
pub const ACTION_OPEN_FILE: u32 = 3;
pub const ACTION_SELECT_TAB: u32 = 4;
pub const ACTION_EXIT: u32 = 5;
pub const ACTION_OPEN_SETTINGS: u32 = 6;
pub const ACTION_SCROLL_TERMINAL: u32 = 7;
/// `Action::TermSelect`: `data` points at a [`TermSelectC`] the plugin
/// owns until its next call, `data_len` is its size — the same
/// discipline as a path's bytes, and what lets the payload GROW without
/// touching `ActionC` itself (whose writers cannot be size-checked).
pub const ACTION_TERM_SELECT: u32 = 8;
/// `Action::PastePrimary`: no payload.
pub const ACTION_PASTE_PRIMARY: u32 = 9;
/// `Action::Capture`: no payload. The one answer to
/// [`PluginApi::drag`]`(DRAG_BEGIN)` that takes the gesture and asks the
/// application for NOTHING — a scroll thumb, a column edge, a knob. F2
/// §6 wanted a `press -> bool` for exactly this; the F1 ledger merged
/// press into the single capture path, and this is that boolean in the
/// channel the path already has. Answering it from `click`, `wheel` or
/// [`PluginApi::button`] means nothing and does nothing — `button`
/// carries the OTHER half of a press (the state-ladder rung, and a
/// release that arrives whether or not the drag was ever accepted), and
/// leaving the capture here is what keeps it from becoming a second
/// path to the same thing.
pub const ACTION_CAPTURE: u32 = 10;

/// [`PluginApi::drag`] phases — `DragPhase` as numbers.
pub const DRAG_BEGIN: u32 = 0;
pub const DRAG_MOVE: u32 = 1;
pub const DRAG_END: u32 = 2;

/// [`PluginApi::button`] phases — the pointer button going down and
/// coming up. Numbered in their own space rather than continuing the
/// `DRAG_*` one, because they are a different question asked on the same
/// gesture: a reader that confused the two would be told "move" and hear
/// "release".
pub const BUTTON_PRESS: u32 = 0;
pub const BUTTON_RELEASE: u32 = 1;

/// The modifier mask [`PluginApi::key`] carries — `focus::Mods` as a
/// number. The bits ARE that type's bits (asserted below), so the mask
/// crosses the boundary without a translation table that could drift;
/// the constants exist so neither side has to write `1 << 2` and hope.
pub const MODS_NONE: u32 = 0;
pub const MODS_CTRL: u32 = 1 << 0;
pub const MODS_SHIFT: u32 = 1 << 1;
pub const MODS_ALT: u32 = 1 << 2;
pub const MODS_SUPER: u32 = 1 << 3;

// The one place the two vocabularies are married. If a bit ever moves in
// `focus::Mods`, this stops the build instead of shipping a Ctrl that a
// plugin reads as Shift.
const _: () = assert!(crate::focus::Mods::CTRL.bits() as u32 == MODS_CTRL);
const _: () = assert!(crate::focus::Mods::SHIFT.bits() as u32 == MODS_SHIFT);
const _: () = assert!(crate::focus::Mods::ALT.bits() as u32 == MODS_ALT);
const _: () = assert!(crate::focus::Mods::SUPER.bits() as u32 == MODS_SUPER);

/// The words the boundary spells its NAMED keys with — the contract
/// [`PluginApi::key`] and [`PluginApi::key_feedback`] both carry.
///
/// It lives here, in one place, because it has to be read on both sides
/// and until now it was written on neither: the host spelled five words
/// inline where it built its feedback pair, and every plugin guessed the
/// same five back in a `match` of its own. Two independent guesses at an
/// unwritten table is how a plugin comes to understand HOME while the
/// host has never sent it — which is precisely the state this replaces.
///
/// A key crosses as a CHARACTER or as a NAME, never as both: `ch` is a
/// Unicode scalar and what a field inserts, a name is what a widget
/// obeys. The whole set of names is:
///
/// | word | key |
/// |------|-----|
/// | `ENTER` | Enter / Return |
/// | `ESC` | Escape |
/// | `BACK` | Backspace |
/// | `SPACE` | the space bar |
/// | `TAB` | Tab |
/// | `UP` `DOWN` `LEFT` `RIGHT` | the arrows |
/// | `HOME` `END` | line ends |
/// | `DELETE` | forward delete |
/// | `PAGE_UP` `PAGE_DOWN` | paging |
///
/// Rules a reader may rely on:
///
/// * ASCII upper case, and a reader that upper-cases before comparing is
///   exactly as correct — the host never sends anything else.
/// * A key this table does not name is not delivered as a name at all.
///   Insert, the menu key and the function keys stay the application's
///   shortcuts; a widget that saw them would be competing with the
///   shortcut registry for them.
/// * An empty name with `ch == 0` is not an event and means nothing.
pub mod keys {
    use crate::focus::Key;

    pub const ENTER: &str = "ENTER";
    pub const ESC: &str = "ESC";
    pub const BACK: &str = "BACK";
    pub const SPACE: &str = "SPACE";
    pub const TAB: &str = "TAB";
    pub const UP: &str = "UP";
    pub const DOWN: &str = "DOWN";
    pub const LEFT: &str = "LEFT";
    pub const RIGHT: &str = "RIGHT";
    pub const HOME: &str = "HOME";
    pub const END: &str = "END";
    pub const DELETE: &str = "DELETE";
    pub const PAGE_UP: &str = "PAGE_UP";
    pub const PAGE_DOWN: &str = "PAGE_DOWN";

    /// Every word, in the order the table above lists them — for a
    /// caller that wants to check its own coverage against the contract
    /// rather than against its memory of it.
    pub const ALL: [&str; 14] = [
        ENTER, ESC, BACK, SPACE, TAB, UP, DOWN, LEFT, RIGHT, HOME, END, DELETE, PAGE_UP,
        PAGE_DOWN,
    ];

    /// The word for a neutral key, or None for one that crosses as a
    /// character (or does not cross at all).
    pub fn name_of(k: Key) -> Option<&'static str> {
        Some(match k {
            Key::Enter => ENTER,
            Key::Escape => ESC,
            Key::Backspace => BACK,
            Key::Space => SPACE,
            Key::Tab => TAB,
            Key::Up => UP,
            Key::Down => DOWN,
            Key::Left => LEFT,
            Key::Right => RIGHT,
            Key::Home => HOME,
            Key::End => END,
            Key::Delete => DELETE,
            Key::PageUp => PAGE_UP,
            Key::PageDown => PAGE_DOWN,
            // A character rides in `ch`; the rest are not delivered.
            Key::Char(_) | Key::Insert | Key::Menu | Key::F(_) => return None,
        })
    }

    /// The key a word means, case-insensitively. None for anything the
    /// table does not hold — a word from a newer host must read as "no
    /// key I know" rather than as the wrong one.
    pub fn from_name(word: &str) -> Option<Key> {
        Some(match word.to_ascii_uppercase().as_str() {
            ENTER => Key::Enter,
            ESC => Key::Escape,
            BACK => Key::Backspace,
            SPACE => Key::Space,
            TAB => Key::Tab,
            UP => Key::Up,
            DOWN => Key::Down,
            LEFT => Key::Left,
            RIGHT => Key::Right,
            HOME => Key::Home,
            END => Key::End,
            DELETE => Key::Delete,
            PAGE_UP => Key::PageUp,
            PAGE_DOWN => Key::PageDown,
            _ => return None,
        })
    }
}

/// `TermSelectC::op`.
pub const SELECT_OP_BEGIN: u32 = 0;
pub const SELECT_OP_EXTEND: u32 = 1;
pub const SELECT_OP_END: u32 = 2;

/// `TermSelectC::kind`, meaningful on BEGIN — `term::SelKind` as
/// numbers. The HOST may override it from its click count (a widget
/// cannot see double clicks).
pub const SELECT_KIND_CELLS: u32 = 0;
pub const SELECT_KIND_WORDS: u32 = 1;
pub const SELECT_KIND_LINES: u32 = 2;

/// The payload of [`ACTION_TERM_SELECT`]: one selection step in the
/// coordinates of the view the widget DREW. `base_lo`/`base_hi` are the
/// 64-bit line id of that view's first row, split into two words so the
/// struct keeps four-byte alignment on every target — echoed from
/// [`TermViewC::first_id_lo`], and what the host resolves `row` against
/// (never the live `view_offset`; a PTY feed between the draw and the
/// event would shift every row otherwise).
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TermSelectC {
    pub op: u32,
    pub kind: u32,
    pub col: u32,
    pub row: u32,
    pub base_lo: u32,
    pub base_hi: u32,
}

/// The prefix of [`TermSelectC`] a reader requires; a shorter payload
/// is malformed and reads as no action at all.
pub const TERM_SELECT_SIZE_MIN: usize =
    std::mem::offset_of!(TermSelectC, base_hi) + std::mem::size_of::<u32>();

/// The functions a plugin uses to reach the host's shared state.
///
/// Adding a field to the END of this struct is compatible with plugins
/// built against an older version. Through version 5 that cost an
/// [`ABI_VERSION`] bump; from 6 on `api_size` says how much of the table
/// the host filled, so an APPENDED entry needs no bump — a plugin asks
/// (`has_theme_enum_word`, `has_mask_quad`) before calling one its host
/// may end before, and treats an absent entry like a MISSING token:
/// degrade, don't demand. Reordering or removing a field is still a
/// break: that is what the version check exists to catch.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct HostApi {
    /// Version of this interface, checked before anything else is used.
    pub abi_version: u32,
    /// The host's own `sizeof(HostApi)`. A plugin treats an entry past
    /// this size as absent — the mirror of [`PluginApi::api_size`], and
    /// what lets THIS table grow by appending without a version break.
    pub api_size: u32,
    /// Reports a sound event into the host's queue.
    pub emit_sound: extern "C" fn(event: u32),
    /// Number of registered widgets in the host.
    pub panel_count: extern "C" fn() -> u32,

    // --- drawing -------------------------------------------------
    //
    // The drawing context is passed as an opaque pointer: it holds Rust
    // references the plugin has no way to describe, so the host keeps
    // it and the plugin only names it. Every call below is cheap — a
    // function call, not an interpreter — which is why a plugin can
    // afford to draw a terminal cell by cell where a script cannot.
    pub rect: extern "C" fn(ctx: *mut c_void, r: RectC, c: ColorC),
    pub rect_outline: extern "C" fn(ctx: *mut c_void, r: RectC, t: f32, c: ColorC),
    /// Four corners, eight floats.
    pub quad: extern "C" fn(ctx: *mut c_void, pts: *const f32, c: ColorC),
    pub line:
        extern "C" fn(ctx: *mut c_void, x0: f32, y0: f32, x1: f32, y1: f32, t: f32, c: ColorC),
    /// A run of points, two floats each. `closed` joins the last back to
    /// the first — how the file browser's icons are drawn.
    pub polyline: extern "C" fn(
        ctx: *mut c_void,
        pts: *const f32,
        count: u32,
        t: f32,
        c: ColorC,
        closed: bool,
    ),
    /// `align`: 0 left, 1 centre, 2 right. `text` is UTF-8 with a length,
    /// not a C string, so a widget can draw anything a font can.
    pub text: extern "C" fn(
        ctx: *mut c_void,
        font: u32,
        px: f32,
        x: f32,
        y: f32,
        text: *const u8,
        len: u32,
        c: ColorC,
        spacing: f32,
        align: u32,
    ),
    pub measure: extern "C" fn(
        ctx: *mut c_void,
        font: u32,
        px: f32,
        text: *const u8,
        len: u32,
        spacing: f32,
    ) -> f32,
    pub module_title: extern "C" fn(
        ctx: *mut c_void,
        x: f32,
        y: f32,
        w: f32,
        px: f32,
        left: *const u8,
        left_len: u32,
        right: *const u8,
        right_len: u32,
        c: ColorC,
    ),

    // --- context -------------------------------------------------
    pub theme_base: extern "C" fn(ctx: *mut c_void) -> ColorC,
    pub theme_bg: extern "C" fn(ctx: *mut c_void) -> ColorC,
    pub vh: extern "C" fn(ctx: *mut c_void, v: f32) -> f32,
    pub font_px: extern "C" fn(ctx: *mut c_void, v: f32) -> f32,
    pub mouse: extern "C" fn(ctx: *mut c_void, x: *mut f32, y: *mut f32),
    pub window: extern "C" fn(ctx: *mut c_void, w: *mut f32, h: *mut f32),
    pub elapsed: extern "C" fn(ctx: *mut c_void) -> f64,

    // --- session --------------------------------------------------
    /// Working directory of the active shell, written into `buf` as
    /// UTF-8; returns its length, or 0 when there is none. Takes the
    /// HOST handle rather than the drawing one, because this is about
    /// the session rather than the frame.
    pub shell_cwd: extern "C" fn(host: *const c_void, buf: *mut u8, cap: u32) -> u32,

    // --- terminal -------------------------------------------------
    /// The visible terminal, resolved.
    ///
    /// Cells are written row-major, `view_cols` per row, `cell_stride`
    /// bytes apart, ragged scrollback rows padded with absent cells so
    /// addressing never desynchronises. Returns the number of cells
    /// written, and 0 when there is no terminal or the request is
    /// malformed.
    ///
    /// One call per frame. Fetching a cell at a time would repeat the
    /// whole view_row dispatch two hundred times a row for nothing;
    /// DRAWING a cell at a time is a different question and stays the
    /// widget's, because every glyph carries its own colour.
    ///
    /// Both sizes are passed by value rather than read out of the
    /// structs, so this never touches caller memory to learn how much
    /// caller memory it may touch.
    pub term_view: extern "C" fn(
        host: *const c_void,
        ctx: *mut c_void,
        req: *const TermReqC,
        req_size: u32,
        out: *mut TermViewC,
        out_size: u32,
    ) -> u32,

    // ------------------------------------------------------------------
    // ABI 5: the theme crosses the boundary as TOKENS, not as two colours.
    // Appended only — removing or reordering anything above would shift
    // every offset a compiled plugin memorised (see `attach`).
    // ------------------------------------------------------------------
    /// Resolves a dotted token name to a stable id, once, at attach or
    /// first use. u32::MAX = the master declares no such token; every
    /// accessor below answers the engine's raw default for it.
    pub theme_token: extern "C" fn(name: *const u8, len: u32) -> u32,
    /// A colour used as ink. Missing id: the raw mid grey.
    pub theme_color: extern "C" fn(ctx: *mut c_void, id: u32) -> ColorC,
    /// A colour used as a bed. Missing id: the raw near-black.
    pub theme_bed: extern "C" fn(ctx: *mut c_void, id: u32) -> ColorC,
    /// A length in device px, a scalar, a duration, an angle, a fraction —
    /// whatever the token's kind bakes to. Missing id: 0.0.
    pub theme_px: extern "C" fn(ctx: *mut c_void, id: u32) -> f32,
    pub theme_flag: extern "C" fn(ctx: *mut c_void, id: u32) -> u32,
    /// The index of the token's word in its declared enum list — the
    /// `enum: a | b | c` its master comment declares, in that order. A
    /// token declaring no list numbers its words in order of first use,
    /// the master's own value at 0.
    pub theme_enum: extern "C" fn(ctx: *mut c_void, id: u32) -> u32,
    /// Resolves an interaction class name ("button", "key", "tab") to its
    /// index in the baked class x state matrix. u32::MAX = no such class.
    pub theme_class: extern "C" fn(name: *const u8, len: u32) -> u32,
    /// One rung of the state ladder for one class, written whole into
    /// `out`. `state` indexes the seven states in declaration order (idle,
    /// hover, press, selected, selected_hover, dragging, disabled).
    /// Returns the bytes written; a caller with a smaller `out_size` gets
    /// a prefix, which is what lets this struct grow later.
    pub theme_class_state: extern "C" fn(
        ctx: *mut c_void,
        class: u32,
        state: u32,
        out: *mut StateStyleC,
        out_size: u32,
    ) -> u32,
    /// Bumped on every theme swap (reload, mood, resize). A plugin caching
    /// resolved values invalidates when this moves.
    pub theme_epoch: extern "C" fn(ctx: *mut c_void) -> u32,

    // ------------------------------------------------------------------
    // Appended past the version-6 minimum. `api_size` gates each entry:
    // a plugin asks `has_*` first, and an absent entry degrades like a
    // MISSING token — the widget draws without it, never through it.
    // ------------------------------------------------------------------
    /// The WORD an enum token currently resolves to, written into `buf`
    /// as UTF-8; returns the bytes written — `min(word, cap)`, so a
    /// short buffer gets a prefix, exactly like `shell_cwd` — and 0 for
    /// a token with no word. The compiled twin of the script renderer's
    /// `theme_word`: an OPEN word set (a role binding, a corner mode)
    /// names a word rather than a member of a closed list, so the caller
    /// wants the text itself where [`HostApi::theme_enum`] answers the
    /// index. Init-time, like [`HostApi::theme_token`]: call at widget
    /// init, cache, invalidate on [`HostApi::theme_epoch`] — never
    /// inside a draw loop.
    pub theme_enum_word:
        extern "C" fn(ctx: *mut c_void, id: u32, buf: *mut u8, cap: u32) -> u32,
    /// One quad sampling the soft-mask sprite — the piece of the host's
    /// sprite glow and shadow path a plugin can reach, so a plugin panel
    /// can glow the way `object::window::panel_edge_glow` does. `pts` is
    /// four corners, eight floats, exactly like [`HostApi::quad`]; `uv`
    /// is eight floats in the SPRITE's own 0..1 space, clamped by the
    /// host and mapped into the atlas's mask band — the atlas layout
    /// never crosses the boundary, and glyph texels are unreachable
    /// whatever numbers arrive. `flags`: [`MASK_QUAD_ADD`] renders the
    /// quad additively (light — the glow path); without it the quad
    /// covers (the shadow path). The sprite is the 64-texel soft radial
    /// disk whose stretchable middle is texels 31..33, so strips lie
    /// exactly as the host's own nine-slice lays them.
    pub mask_quad: extern "C" fn(
        ctx: *mut c_void,
        pts: *const f32,
        uv: *const f32,
        c: ColorC,
        flags: u32,
    ),
    /// Nested clip rectangles, forwarding `DrawList::push_clip` across
    /// the boundary. The one primitive smooth pixel scrolling cannot
    /// exist without: a partially visible row must not paint outside its
    /// container. Clips NEST — each is intersected with the one below it
    /// — and every push must be matched by a [`HostApi::pop_clip`].
    /// Gated by [`HostApi::has_clip`]; an old host degrades the caller
    /// to whole-row snapping (`view::Snap::Row`), which is the
    /// filesystem widget's behaviour today — stated, never silent.
    pub push_clip: extern "C" fn(ctx: *mut c_void, r: RectC),
    /// The matching pop. An unbalanced plugin cannot spoil its
    /// neighbours: the host counts the depth around
    /// [`PluginApi::draw`] and unwinds whatever is left over.
    pub pop_clip: extern "C" fn(ctx: *mut c_void),
    /// A filled rectangle wearing the family's corners, and its stroke.
    /// Until these existed a plugin could draw a sharp rect or hand-roll
    /// a chamfer out of polylines — which is why the file browser's
    /// tiles carry their own corner code — and no plugin could round
    /// anything at all. `style` is [`CORNER_SQUARE`], [`CORNER_ROUND`]
    /// or [`CORNER_CHAMFER`]; anything else degrades to square, the
    /// look of an unstyled rect. `radius` is in device pixels, clamped
    /// by the host to half the short side, and the arc tessellation is
    /// the host's own quarter-pixel rule — a plugin never has to know
    /// how many segments an arc needs at this size.
    pub ring_fill:
        extern "C" fn(ctx: *mut c_void, r: RectC, style: u32, radius: f32, c: ColorC),
    /// The stroke of the same shape, drawn inward like every other
    /// border in this toolkit.
    pub ring: extern "C" fn(
        ctx: *mut c_void,
        r: RectC,
        style: u32,
        radius: f32,
        w: f32,
        c: ColorC,
    ),
    /// "The pointer is resting on `anchor`, and what I drew there really
    /// says `text`" — [`crate::view::Surface::tooltip`] across the
    /// boundary.
    ///
    /// The HOST draws it, and that is the whole reason the entry has to
    /// exist rather than the plugin simply painting a box: a tooltip is
    /// drawn OVER its neighbours and outside the rectangle its widget
    /// was given, draw order is z-order, and a plugin draws in the
    /// middle of the frame — anything it painted there would be covered
    /// by the panels drawn after it, which is exactly why
    /// [`crate::object::tooltip`] is drawn last of everything. So the
    /// plugin states the fact and the host owns the timing, the
    /// placement and the paint.
    ///
    /// `id` is what tells two neighbouring targets apart across frames:
    /// the same id in the next frame is one target the pointer has not
    /// left, a different one restarts the delay. `text` is UTF-8 with a
    /// length, not a C string; empty is not a request. Filing it while
    /// the pointer is outside `anchor` is not an error — the host tests
    /// containment itself and drops it, because a box explaining a
    /// rectangle the pointer is nowhere near is worse than no box.
    pub tooltip: extern "C" fn(
        ctx: *mut c_void,
        id: u64,
        anchor: RectC,
        text: *const u8,
        len: u32,
    ),
    /// Publishes `data` under `topic`, replacing whatever stood there,
    /// and answers the topic's new sequence number — 0 when the call was
    /// refused (a topic that is not UTF-8, or longer than
    /// [`CHANNEL_TOPIC_MAX`]; a payload past [`CHANNEL_VALUE_MAX`]).
    ///
    /// The channel is a BOARD, not a queue: one value per topic, the
    /// last one written, standing until it is written again. That is the
    /// simplest shape that does all four things a widget-to-widget
    /// channel has to do here, and every richer one fails at least one:
    ///
    /// * it survives a `.so` boundary — the value lives in the HOST's
    ///   copy of the toolkit, so plugins opened `RTLD_LOCAL` with a
    ///   static each still read ONE value. A shared static cannot: that
    ///   is why the launcher's category selection stopped working the
    ///   day its two widgets became two files.
    /// * the topic is text and the payload is bytes, so neither side
    ///   needs a type the other was compiled against.
    /// * nobody has to be listening. A retained value is read whenever
    ///   the reader next draws, so "which widget loaded first" and
    ///   "which drew first" stop being questions.
    /// * it cannot block drawing. There is no queue to fill, no reader
    ///   to wait for, no wakeup to deliver: a publisher writes and
    ///   returns, and a reader picks the value up on its next frame —
    ///   at most one frame of latency, which is what every other fact in
    ///   an immediate-mode interface already costs.
    ///
    /// A queue fails the last two (an undrained one grows forever, and
    /// bounding it makes the publisher wait); a callback fails them too,
    /// because the host would have to call into one plugin from
    /// another's stack, mid-frame.
    pub channel_publish: extern "C" fn(
        topic: *const u8,
        topic_len: u32,
        data: *const u8,
        data_len: u32,
    ) -> u64,
    /// The value standing under `topic`, written into `buf`.
    ///
    /// Answers the value's FULL length while writing `min(len, cap)`
    /// bytes — deliberately unlike [`HostApi::shell_cwd`], which answers
    /// what it wrote. A cwd's prefix is a shorter path; a payload's
    /// prefix is a broken message, and half a message read as a whole
    /// one is worse than none. So the caller can always TELL a
    /// truncation from a fit and ask again with room.
    ///
    /// `seq`, when not null, receives the topic's sequence number: 0
    /// means nothing was ever published there (so an empty payload and
    /// an absent one are distinguishable), and a number that has not
    /// moved since the last read means the value has not changed — how a
    /// reader skips work without comparing payloads.
    pub channel_read: extern "C" fn(
        topic: *const u8,
        topic_len: u32,
        buf: *mut u8,
        cap: u32,
        seq: *mut u64,
    ) -> u32,
    /// The TEXT of one of an addon's settings files, written into `buf`
    /// — never a path, never an open handle.
    ///
    /// That asymmetry is the whole point of the entry, and it is the
    /// clipboard's reasoning again (przynależność, the clipboard
    /// verdict): a plugin reaches the host's state through a call that
    /// NAMES what it wants, not through a handle that would let it
    /// decide. Here the plugin names an addon and a file WITHIN the
    /// settings directory; the host is the only side that ever holds a
    /// path, so no argument a plugin can pass reaches `/etc/shadow`, and
    /// the search order across `~/.config` and `/etc/xdg` stays one
    /// decision made in one place.
    ///
    /// Addressing follows the on-disk arrangement exactly. `addon` is
    /// the addon's own name; `file` is empty for an addon with ONE
    /// settings file and names the member for an addon with a
    /// directory of them:
    ///
    /// | call | file read |
    /// |------|-----------|
    /// | `("shell", "")` | `<config>/addons/shell.ron` |
    /// | `("search", "engines")` | `<config>/addons/search/engines.ron` |
    ///
    /// Both names are plain names, not path fragments: lower-case
    /// ASCII, digits, `_` and `-`, at most [`SETTINGS_NAME_MAX`] bytes,
    /// and the `.ron` suffix is the host's to add. Anything else is
    /// [`SETTINGS_REFUSED`] — a dot is rejected with everything else,
    /// so `..` cannot be spelled at all rather than being filtered
    /// after the fact. The program's own file (`nacelle-desktop.ron`)
    /// sits a level ABOVE `addons/` and is therefore unreachable
    /// through this entry by construction, not by a check.
    ///
    /// The payload is the file's RON SOURCE, and the plugin parses it
    /// into its own type. The host does not resolve keys and does not
    /// hand over values — see [`crate::settings`] for why that contract
    /// was chosen over a typed key lookup, because it is the decision
    /// this entry's shape rests on.
    ///
    /// Answers the text's FULL length while writing `min(len, cap)`
    /// bytes, exactly like [`HostApi::channel_read`] and deliberately
    /// unlike [`HostApi::shell_cwd`]: a prefix of a cwd is a shorter
    /// path, a prefix of a document is a document that will not parse,
    /// so the caller must be able to tell a truncation from a fit and
    /// ask again with room.
    ///
    /// `status`, when not null, receives [`SETTINGS_OK`],
    /// [`SETTINGS_ABSENT`], [`SETTINGS_MALFORMED`] or
    /// [`SETTINGS_REFUSED`]. It is not a nicety: absent and malformed
    /// both end in "use your defaults", and a caller that could not
    /// tell them apart would have no way to know that the user has a
    /// file which is being ignored. Text is delivered WHATEVER the
    /// status says — the host's parse is a diagnostic, not a gate; see
    /// [`SETTINGS_MALFORMED`].
    pub settings_read: extern "C" fn(
        addon: *const u8,
        addon_len: u32,
        file: *const u8,
        file_len: u32,
        buf: *mut u8,
        cap: u32,
        status: *mut u32,
    ) -> u32,
    /// Bumped whenever a settings file may have changed — the host
    /// rewrote one from its settings window, or was asked to reload.
    /// A plugin caching its parsed settings invalidates when this
    /// moves, exactly as it does for [`HostApi::theme_epoch`].
    ///
    /// It shares one gate with [`HostApi::settings_read`] because
    /// reading without it is not the mechanism: parsing a document per
    /// frame is not something an immediate-mode widget can afford, so
    /// every caller caches — and a cache with no invalidation means the
    /// settings window's Apply button changes a file and nothing on
    /// screen. That is the clip pair's argument in another costume: a
    /// half-present mechanism is worse than an absent one, because the
    /// absent one degrades where it is stated to.
    pub settings_epoch: extern "C" fn() -> u32,
    /// A TEXT token's value, written into `buf` as UTF-8; returns the
    /// bytes written — `min(text, cap)`, so a short buffer gets a prefix
    /// exactly like [`HostApi::theme_enum_word`] — and 0 for a token that
    /// is not of this kind, is absent, or holds nothing.
    ///
    /// The kind [`HostApi::theme_px`], `theme_color`, `theme_flag` and
    /// `theme_enum` between them could not reach: a text token is a
    /// STRING the theme states, not a member of a list and not a number.
    /// Two keys are of it today and one of them, `type.ellipsis`, is why
    /// this entry exists — every widget that trims a name to its tile
    /// appended `"…"` out of its own source, so a console theme asking
    /// for `>` got the character it did not ask for from four places at
    /// once, in two processes' worth of code that had no road to the key.
    ///
    /// Init-time, like [`HostApi::theme_token`] and
    /// [`HostApi::theme_enum_word`]: call at widget init, cache,
    /// invalidate on [`HostApi::theme_epoch`]. A text token is found on
    /// the host by a scan of every text key the theme declares — cheap
    /// once per theme, wrong once per frame.
    pub theme_text: extern "C" fn(ctx: *mut c_void, id: u32, buf: *mut u8, cap: u32) -> u32,
    /// The glow of the family's ring — [`DrawList::glow_ring`] across the
    /// boundary, wearing the same [`CORNER_SQUARE`]/[`CORNER_ROUND`]/
    /// [`CORNER_CHAMFER`] vocabulary and the same `radius` translation
    /// [`HostApi::ring_fill`] and [`HostApi::ring`] already carry —
    /// `style`, `radius` name the SAME shape a fill or a stroke on this
    /// rect would wear, and `glow_radius` is how far the light reaches
    /// past it. Until this existed, a plugin wanting a glow on a
    /// chamfered corner had no door through the boundary for it at all
    /// and extruded the octagon by hand out of [`HostApi::mask_quad`] —
    /// which is why the file browser's tiles carried their own glow
    /// code beside their own corner code. Gated by
    /// [`HostApi::has_ring_glow`]; an old host draws no glow, the same
    /// degradation [`HostApi::ring_fill`] already answers for the ring
    /// itself when [`HostApi::has_ring`] is false — never a hand-rolled
    /// approximation of one.
    pub ring_glow: extern "C" fn(
        ctx: *mut c_void,
        r: RectC,
        style: u32,
        radius: f32,
        glow_radius: f32,
        c: ColorC,
    ),
}

/// The longest topic name the channel accepts. A name is a constant in
/// somebody's source, never user input; the bound is here so a plugin
/// passing a wild length cannot make the host allocate from it.
pub const CHANNEL_TOPIC_MAX: usize = 128;

/// The largest value the channel carries. A selection, a path, a small
/// list — this is a place for facts, not a transport for files.
pub const CHANNEL_VALUE_MAX: usize = 64 * 1024;

/// How many distinct topics the board holds. Publishing to a new topic
/// past this is refused rather than evicting somebody else's: a widget
/// that quietly stopped hearing its partner is the failure this whole
/// entry exists to end.
pub const CHANNEL_TOPICS_MAX: usize = 256;

/// [`HostApi::settings_read`]: a file was found and the host's own
/// parse of it succeeded. The text delivered is that file's source.
pub const SETTINGS_OK: u32 = 0;

/// [`HostApi::settings_read`]: no such file in any settings directory,
/// and no bytes were delivered. NOT an error and NOT reported to the
/// user: an empty `~/.config` with nothing packaged beside it is the
/// ordinary state of a fresh install, and the addon's own defaults are
/// the whole answer. This is the status almost every call gets.
pub const SETTINGS_ABSENT: u32 = 1;

/// [`HostApi::settings_read`]: a file was found and the HOST could not
/// parse it. The user has already been told, with the path and the
/// line — the host is the only side that knows either.
///
/// The text is still delivered, and a caller should still try its own
/// parse on it. The host's parse is generic (it validates the document,
/// not the caller's type), so it is the right place to produce a message
/// about a stray bracket and the wrong place to have the last word about
/// whether a document is usable. Withholding text on a generic parse
/// failure would let a disagreement between two parsers turn a working
/// file into a missing one, silently — which is the failure mode this
/// whole status set exists to prevent.
pub const SETTINGS_MALFORMED: u32 = 2;

/// [`HostApi::settings_read`]: the addon or file name was not a plain
/// name, or the host holds no settings directories at all (nothing was
/// installed — a test, a headless run). A programming error rather than
/// a user's, and no bytes were delivered.
pub const SETTINGS_REFUSED: u32 = 3;

/// The longest addon or file name [`HostApi::settings_read`] accepts.
/// A name is a constant in somebody's source, never user input, so
/// anything long is a bug; the bound is here so a plugin passing a wild
/// length cannot make the host build a path out of it.
pub const SETTINGS_NAME_MAX: usize = 64;

/// The largest settings file the host will read. A settings file is
/// written by a person or by the settings window; one past this is not
/// a settings file, and reading it would be the host spending memory on
/// a plugin's behalf with no ceiling.
pub const SETTINGS_FILE_MAX: usize = 256 * 1024;

/// Corner styles for [`HostApi::ring_fill`] and [`HostApi::ring`]. The
/// numbers are the boundary's own vocabulary, not the theme's enum
/// indices — those intern in load order and mean nothing across a
/// library edge.
pub const CORNER_SQUARE: u32 = 0;
pub const CORNER_ROUND: u32 = 1;
pub const CORNER_CHAMFER: u32 = 2;

/// The prefix of [`HostApi`] every version-6 host must fill — everything
/// up to and including `theme_epoch`. [`attach`] refuses a table shorter
/// than this; entries appended after it are optional by `api_size`.
pub const HOST_API_SIZE_MIN: usize = std::mem::offset_of!(HostApi, theme_enum_word);

/// The prefix that includes `theme_enum_word`; a host whose `api_size`
/// reaches this far answers it.
pub const HOST_API_HAS_ENUM_WORD: usize =
    std::mem::offset_of!(HostApi, theme_enum_word) + std::mem::size_of::<usize>();

/// The prefix that includes `mask_quad`.
pub const HOST_API_HAS_MASK_QUAD: usize =
    std::mem::offset_of!(HostApi, mask_quad) + std::mem::size_of::<usize>();

/// The prefix that includes BOTH clip entries. One gate, not two: a
/// caller that can push and cannot pop would wedge the frame, so the
/// pair is either wholly there or wholly absent.
pub const HOST_API_HAS_CLIP: usize =
    std::mem::offset_of!(HostApi, pop_clip) + std::mem::size_of::<usize>();

/// The prefix that includes BOTH ring entries. One gate for the pair,
/// like the clips: a caller that can fill a rounded rect but not stroke
/// it would draw half a control.
pub const HOST_API_HAS_RING: usize =
    std::mem::offset_of!(HostApi, ring) + std::mem::size_of::<usize>();

/// The prefix that includes `tooltip`.
pub const HOST_API_HAS_TOOLTIP: usize =
    std::mem::offset_of!(HostApi, tooltip) + std::mem::size_of::<usize>();

/// The prefix that includes BOTH channel entries. One gate for the pair,
/// like the clips and the rings: a widget that can publish and cannot
/// read is talking to itself, and one that can read and cannot publish
/// is waiting for a message nobody in its process can send.
pub const HOST_API_HAS_CHANNEL: usize =
    std::mem::offset_of!(HostApi, channel_read) + std::mem::size_of::<usize>();

/// The prefix that includes BOTH settings entries — the read and the
/// epoch it is cached against. One gate for the pair, for the reason
/// [`HostApi::settings_epoch`] states.
pub const HOST_API_HAS_SETTINGS: usize =
    std::mem::offset_of!(HostApi, settings_epoch) + std::mem::size_of::<usize>();

/// The prefix that includes `theme_text`.
pub const HOST_API_HAS_THEME_TEXT: usize =
    std::mem::offset_of!(HostApi, theme_text) + std::mem::size_of::<usize>();

/// The prefix that includes `ring_glow`.
pub const HOST_API_HAS_RING_GLOW: usize =
    std::mem::offset_of!(HostApi, ring_glow) + std::mem::size_of::<usize>();

/// [`HostApi::mask_quad`]: blend additively — the quad adds light, the
/// way the host's own glow does. Without it the quad covers, the way its
/// shadows do.
pub const MASK_QUAD_ADD: u32 = 1;

impl HostApi {
    /// Whether this host's table reaches `theme_enum_word` at all.
    /// Asked once, at attach or first use, never per frame.
    pub fn has_theme_enum_word(&self) -> bool {
        self.api_size as usize >= HOST_API_HAS_ENUM_WORD
    }

    /// Whether this host's table reaches `mask_quad`. Absent: no glow —
    /// the same degradation the renderer's texture-miss failsafe applies.
    pub fn has_mask_quad(&self) -> bool {
        self.api_size as usize >= HOST_API_HAS_MASK_QUAD
    }

    /// Whether this host's table reaches the clip pair. Absent: the
    /// caller must not scroll by fractions of a row — it snaps to whole
    /// rows, which is exactly what every widget does today, and says so
    /// once rather than painting outside its box.
    pub fn has_clip(&self) -> bool {
        self.api_size as usize >= HOST_API_HAS_CLIP
    }

    /// Whether this host can draw the family's corners for a plugin.
    /// Absent: draw a plain rect and say so — a sharp control among
    /// rounded ones is a visible degradation, not a silent one.
    pub fn has_ring(&self) -> bool {
        self.api_size as usize >= HOST_API_HAS_RING
    }

    /// Whether this host draws tooltips for a plugin. Absent: the
    /// request is not made and a trimmed label simply stays trimmed —
    /// the degradation `Surface::tooltip`'s default already describes.
    pub fn has_tooltip(&self) -> bool {
        self.api_size as usize >= HOST_API_HAS_TOOLTIP
    }

    /// Whether this host carries the widget-to-widget channel. Absent:
    /// a publish reaches nobody and a read finds nothing, so a widget
    /// that steers another one falls back to whatever it shows when
    /// nothing has been chosen — never to a wrong choice.
    pub fn has_channel(&self) -> bool {
        self.api_size as usize >= HOST_API_HAS_CHANNEL
    }

    /// Whether this host can hand a plugin its own settings. Absent: an
    /// addon runs on the values baked into its type and says so once —
    /// which is exactly the state every addon was in before this entry
    /// existed, so an old host degrades to yesterday rather than to a
    /// blank panel.
    pub fn has_settings(&self) -> bool {
        self.api_size as usize >= HOST_API_HAS_SETTINGS
    }

    /// Whether this host answers text tokens. Absent: the caller reads
    /// the empty string, which is the SAME answer a theme that declares
    /// no such key gives — a widget must not be able to tell an old host
    /// from a quiet theme, or it would grow a fallback for one of them.
    pub fn has_theme_text(&self) -> bool {
        self.api_size as usize >= HOST_API_HAS_THEME_TEXT
    }

    /// Whether this host can glow the family's ring for a plugin. Absent:
    /// no glow at all — a plain ring or fill without the halo around it,
    /// which is a visible but honest degradation, never an approximation
    /// built out of [`HostApi::mask_quad`] by hand.
    pub fn has_ring_glow(&self) -> bool {
        self.api_size as usize >= HOST_API_HAS_RING_GLOW
    }

    /// A text token by NAME, resolved and copied out — the plugin-side
    /// shorthand for [`HostApi::theme_token`] + [`HostApi::theme_text`].
    ///
    /// Here rather than in each widget because the two that trim names
    /// to tiles wrote the ellipsis out of their own source, and a helper
    /// each would be the same duplication one layer down. Init-time:
    /// cache the answer against [`HostApi::theme_epoch`].
    pub fn theme_text_of(&self, ctx: *mut c_void, name: &str) -> String {
        if !self.has_theme_text() {
            return String::new();
        }
        let id = (self.theme_token)(name.as_ptr(), name.len() as u32);
        // Longer than any trim marker or figure set a theme states; a
        // longer one arrives cut rather than growing a buffer for a key
        // nobody writes.
        let mut buf = [0u8; 64];
        let n = (self.theme_text)(ctx, id, buf.as_mut_ptr(), buf.len() as u32);
        String::from_utf8_lossy(&buf[..(n as usize).min(buf.len())]).into_owned()
    }
}

/// What a plugin exports: how to make one of its widgets, draw it and
/// give it input. Returned from the attach point.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct PluginApi {
    pub abi_version: u32,
    /// The plugin's own `sizeof(PluginApi)`. The host reads
    /// `min(its own, this)`: an entry past a plugin's size is treated as
    /// absent (`chrome` today answers [`crate::widget::Chrome::none`]),
    /// which is what lets this struct GROW by appending without another
    /// version break.
    pub api_size: u32,
    /// Creates one widget instance.
    pub create: extern "C" fn() -> *mut c_void,
    /// Destroys an instance. Never called while a frame is in flight.
    pub destroy: extern "C" fn(instance: *mut c_void),
    /// `host` is the opaque handle for the session data a widget may
    /// read; `ctx` is the drawing context. They are separate because
    /// one outlives the frame and the other does not.
    pub draw: extern "C" fn(
        instance: *mut c_void,
        ctx: *mut c_void,
        host: *const c_void,
        r: RectC,
    ),
    /// Input arrives outside a frame, so there is no drawing context to
    /// pass — which is why the window size is given directly. A widget
    /// that sizes itself against the window has to hit-test against the
    /// same number it drew with.
    pub click: extern "C" fn(
        instance: *mut c_void,
        x: f32,
        y: f32,
        r: RectC,
        win_w: f32,
        win_h: f32,
        out: *mut ActionC,
    ),
    pub wheel: extern "C" fn(
        instance: *mut c_void,
        dy: f32,
        r: RectC,
        win_w: f32,
        win_h: f32,
        out: *mut ActionC,
    ),
    /// The character grid the widget settled on, or zero for none.
    pub grid: extern "C" fn(instance: *mut c_void, cols: *mut u32, rows: *mut u32),
    /// A key pressed on the physical keyboard, for the on-screen one.
    pub key_feedback:
        extern "C" fn(instance: *mut c_void, ch: u32, label: *const u8, label_len: u32),
    /// How the widget answers being resized. Asked before the layout
    /// runs, which is why no height is passed: it is what the host is
    /// about to decide.
    ///
    /// The answer is one number, because the interface is C:
    ///
    /// * `> 0` — the height the content needs, at scale 1. The panel is
    ///   made exactly that tall and the content is scaled to fit it,
    ///   whichever axis runs out first.
    /// * [`SIZING_ROWS`] — grows downwards. The width decides how big
    ///   the rows are, the height how many.
    /// * anything else, [`SIZING_REFERENCE`] included — sized against
    ///   the reference box on both axes.
    pub sizing: extern "C" fn(
        instance: *mut c_void,
        ctx: *mut c_void,
        host: *const c_void,
    ) -> f32,
    /// What the host should draw around this widget this frame (u2 §4):
    /// the panel container's title band texts, controls and alarm state,
    /// prefix-written into `out`. Answers the bytes written; 0 = no
    /// chrome, and the widget gets the plain container. Asked once per
    /// frame, before `draw`, with the same host data.
    pub chrome: extern "C" fn(
        instance: *mut c_void,
        ctx: *mut c_void,
        host: *const c_void,
        out: *mut ChromeC,
        out_size: u32,
    ) -> u32,
    /// A pointer drag over the widget — `Widget::drag` across the
    /// boundary, and the host's SINGLE capture path (F1 §5.1: F2's
    /// press/release append after this entry and are synthesized
    /// through the same capture, never a second one). `phase` is a
    /// `DRAG_*` code; a `Begin` answered with `ACTION_NONE` declines
    /// the capture and the press falls back to the click delivery.
    /// A widget that drives the gesture itself and wants nothing from
    /// the application accepts with [`ACTION_CAPTURE`] — the file
    /// panel's scroll thumb is the first.
    /// Appended past `chrome`, `api_size`-gated: a plugin whose table
    /// ends before it simply never receives drags.
    pub drag: extern "C" fn(
        instance: *mut c_void,
        phase: u32,
        x: f32,
        y: f32,
        r: RectC,
        win_w: f32,
        win_h: f32,
        out: *mut ActionC,
    ),
    /// Is one of my controls under this point? — `Widget::pointer`
    /// across the boundary. The host asks before it asks anything else,
    /// so the cursor can become a hand the same frame the pointer
    /// arrives; nonzero = yes. No drawing context and no host data are
    /// passed, because this is a question about pixels: the widget
    /// answers from the rectangles IT draws, which is what keeps the
    /// application from computing somebody else's geometry.
    /// Appended past `drag`, `api_size`-gated: a plugin whose table
    /// ends before it is never asked, and its panel keeps the ordinary
    /// cursor.
    pub pointer: extern "C" fn(
        instance: *mut c_void,
        x: f32,
        y: f32,
        r: RectC,
        win_w: f32,
        win_h: f32,
    ) -> u32,
    /// A key delivered to the widget that OWNS THE KEYBOARD — the entry
    /// [`PluginApi::key_feedback`] could never be.
    ///
    /// `key_feedback` stays exactly what it was and is not touched: a
    /// BROADCAST to every instance, so an on-screen keyboard can light
    /// up the key somebody else is typing. This is its opposite in the
    /// three ways that matter, which is why it is a second entry and not
    /// a wider first one:
    ///
    /// * it is delivered to ONE widget, so two text fields on one board
    ///   stop both eating the same character;
    /// * it carries `mods` — the [`MODS_CTRL`] bits, `focus::Mods` as a
    ///   number — so select-all, undo and the clipboard chords are
    ///   reachable inside a field at all;
    /// * it ANSWERS. Nonzero means the key was consumed and the host
    ///   must not also spend it on focus navigation, a shortcut or the
    ///   shell's bytes; `out` may ask the application for something the
    ///   way `click` does (a search field's Enter launching what it
    ///   found is the first). Zero leaves the key entirely to the host.
    ///
    /// `label`/`label_len` is one of the [`keys`] words (UTF-8 with a
    /// length, not a C string) for a named key and empty for a
    /// character; `ch` is a Unicode scalar and 0 for a named key.
    /// Rebuild it with `char::from_u32(v)`, never a transmute — an
    /// invalid scalar value is undefined behaviour.
    ///
    /// What it deliberately does NOT carry: the platform's composed
    /// TEXT (a multi-character IME commit is not one key and does not
    /// come through here), and the auto-repeat bit (a held key arrives
    /// as another key, which is what auto-repeat is for). Both are
    /// stated so a widget author reads a limit rather than discovering
    /// one.
    ///
    /// Appended past `pointer`, `api_size`-gated: a plugin whose table
    /// ends before it is never called, and the host spends the key on
    /// itself exactly as it does today.
    pub key: extern "C" fn(
        instance: *mut c_void,
        ch: u32,
        label: *const u8,
        label_len: u32,
        mods: u32,
        out: *mut ActionC,
    ) -> u32,
    /// A pointer button going down and coming up over the widget —
    /// `Widget::press` and `Widget::release` across the boundary.
    ///
    /// ONE entry carrying a phase, because that is what `drag` already
    /// is: the press, the motion and the release are phases of a single
    /// gesture, and giving half of it a second shape is how two
    /// mechanisms for one thing begin. `phase` is [`BUTTON_PRESS`] or
    /// [`BUTTON_RELEASE`], and the coordinates, rect and window size are
    /// `drag`'s, in the same order, for the same reason.
    ///
    /// It is NOT a second capture path (F1 §5.1: there is one). The
    /// capture is `drag`'s alone: on a press the host delivers
    /// [`BUTTON_PRESS`] and then asks `drag(`[`DRAG_BEGIN`]`)`, whose
    /// answer alone decides who owns the gesture; on the button coming
    /// up it delivers [`BUTTON_RELEASE`] before `click`, so a widget
    /// tracking its own down/up pair has closed it before the click that
    /// concludes it arrives. Answering [`ACTION_CAPTURE`] here means
    /// nothing and does nothing — exactly what it already means from
    /// `click` and `wheel`.
    ///
    /// What it carries that a capture cannot is the half of a press that
    /// is not a gesture: the PRESS rung of the state ladder (a control
    /// that darkens while it is held), and a release that arrives even
    /// when the press was never accepted as a drag.
    pub button: extern "C" fn(
        instance: *mut c_void,
        phase: u32,
        x: f32,
        y: f32,
        r: RectC,
        win_w: f32,
        win_h: f32,
        out: *mut ActionC,
    ),
}

/// The prefix of [`PluginApi`] every version-6 plugin must fill —
/// everything up to and including `sizing`. A table shorter than this is
/// refused; entries appended after it (`chrome` is the first) are
/// optional by `api_size`, absent ones answered by their documented
/// defaults.
pub const PLUGIN_API_SIZE_MIN: usize =
    std::mem::offset_of!(PluginApi, chrome);

/// The prefix that includes `chrome`; a plugin whose `api_size` reaches
/// this far declared the entry.
pub const PLUGIN_API_HAS_CHROME: usize =
    std::mem::offset_of!(PluginApi, chrome) + std::mem::size_of::<usize>();

/// The prefix that includes `drag`.
pub const PLUGIN_API_HAS_DRAG: usize =
    std::mem::offset_of!(PluginApi, drag) + std::mem::size_of::<usize>();

/// The prefix that includes `pointer`.
pub const PLUGIN_API_HAS_POINTER: usize =
    std::mem::offset_of!(PluginApi, pointer) + std::mem::size_of::<usize>();

/// The prefix that includes `key`.
pub const PLUGIN_API_HAS_KEY: usize =
    std::mem::offset_of!(PluginApi, key) + std::mem::size_of::<usize>();

/// The prefix that includes `button`.
pub const PLUGIN_API_HAS_BUTTON: usize =
    std::mem::offset_of!(PluginApi, button) + std::mem::size_of::<usize>();

/// The attach point every plugin must export:
///
/// ```ignore
/// #[no_mangle]
/// pub extern "C" fn nacelle_plugin_attach(host: *const HostApi) -> *const PluginApi
/// ```
///
/// It calls [`attach`] with the host interface and returns its own, or
/// null when the versions do not match.
pub type AttachFn = unsafe extern "C" fn(*const HostApi) -> *const PluginApi;

/// One character cell, already resolved against the theme.
///
/// What crosses is what gets painted, never an index into a palette the
/// plugin would have to keep its own copy of. The alternative would put
/// half the terminal emulator in a `.so`: that bold brightens an index
/// below 8 but not a Default or an Rgb, that dim multiplies BEFORE
/// inverse, that an unset background means no rectangle at all rather
/// than `term_bg` painted. Those are xterm semantics, defined by the
/// same specification as the escape sequence that produced them, and a
/// second copy of them is a shade that is quietly wrong on cells nobody
/// stares at.
///
/// Colours are [`ColorC`] rather than packed bytes because the renderer
/// works in `f32` throughout. Packing would be the one place the
/// pipeline narrows, and it would narrow permanently.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct CellC {
    /// Unicode scalar value. Rebuild it with
    /// `char::from_u32(v).unwrap_or(' ')` — never a transmute, because
    /// an invalid scalar value is undefined behaviour, and never an
    /// `unwrap`, because a panic across `extern "C"` is a dead process.
    pub ch: u32,
    /// `CELL_*` bits. A bit this build does not know is ignored, not
    /// rejected — that is what makes adding one free.
    pub flags: u32,
    /// Columns the cell covers: 1 ordinarily, 2 for the first half of a
    /// double-width character, 0 for anything with nothing to draw —
    /// the second half of a pair, or a position no cell exists at.
    pub width: u8,
    /// Font id to pass to [`HostApi::text`]. The monospace font today;
    /// if the toolkit ever loads a bold face, bold cells start arriving
    /// with a different id and no plugin is rebuilt.
    pub font: u8,
    pub reserved: u16,
    pub fg: ColorC,
    /// Only meaningful with `CELL_HAS_BG`. A flag rather than a zero
    /// alpha, so a translucent cell background stays expressible; an
    /// overloaded sentinel cannot be taken back once it ships.
    pub bg: ColorC,
}

/// The cell is underlined.
pub const CELL_UNDERLINE: u32 = 1;
/// `bg` is a colour to paint. Without it the panel shows through, which
/// is what an unset SGR background has always meant here.
pub const CELL_HAS_BG: u32 = 2;
/// No cell exists at this position — a scrollback row that kept an
/// older, shorter width, or a row past the end of the view.
pub const CELL_ABSENT: u32 = 4;
/// The cell is inside the terminal selection. A FLAG, never a colour
/// baked into `bg`: `term.selection.mode = invert` needs the ORIGINAL
/// colours to invert, so the widget applies the selection look itself
/// from the `term.selection*` tokens. Old plugins ignore unknown bits —
/// the documented contract — and simply draw no wash.
pub const CELL_SELECTED: u32 = 8;

impl CellC {
    /// A position with nothing in it. `width` 0 is what makes it draw
    /// nothing, matching the row the terminal view simply stopped
    /// drawing; `CELL_ABSENT` is what tells a later selection or copy
    /// feature that this is not a space anyone typed.
    pub const fn absent() -> CellC {
        const NIL: ColorC = ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
        CellC {
            ch: b' ' as u32,
            flags: CELL_ABSENT,
            width: 0,
            font: 0,
            reserved: 0,
            fg: NIL,
            bg: NIL,
        }
    }
}

/// The smallest `cell_stride` [`HostApi::term_view`] will fill. `CellC`
/// may GROW: the host writes `min(stride, its own size)` bytes per cell
/// and advances by the CALLER's stride, so a plugin built before a field
/// existed gets the prefix it knows, and one built after it keeps
/// whatever it initialised the tail to.
pub const CELL_SIZE_MIN: u32 = std::mem::size_of::<CellC>() as u32;

// The size is arithmetic a plugin performs, so it is pinned here rather
// than assumed. Four-byte alignment throughout means no interior
// padding on any target this program runs on.
const _: () = assert!(std::mem::size_of::<CellC>() == 44);
const _: () = assert!(std::mem::align_of::<CellC>() == 4);

/// What a widget asks [`HostApi::term_view`] for.
///
/// A struct rather than a parameter list because parameters cannot be
/// appended and fields can: a session index, a starting row for a split
/// view, a "unchanged since" hint all land here later for free.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TermReqC {
    /// Pixel rectangle the grid is to fill. The HOST divides it, because
    /// the division needs font metrics no plugin can see and because its
    /// result is what the user's PTY is resized to.
    pub area: RectC,
    /// Where the cells go, row-major, `view_cols` per row.
    pub cells: *mut CellC,
    /// Capacity of `cells` in BYTES, not in cells. Bytes is what makes
    /// the two numbers unable to disagree: whatever `cell_stride` the
    /// caller claims, the host writes at most this many bytes.
    pub cells_bytes: u32,
    /// Distance between cells, at least [`CELL_SIZE_MIN`].
    pub cell_stride: u32,
    /// The active session. Reserved; anything but 0 is refused until it
    /// means something.
    pub session: u32,
    /// Reserved, must be 0.
    pub flags: u32,
}

impl TermReqC {
    /// A request with nowhere to put cells, for a caller that only wants
    /// the metrics back.
    pub const fn empty() -> TermReqC {
        TermReqC {
            area: RectC { x: 0.0, y: 0.0, w: 0.0, h: 0.0 },
            cells: std::ptr::null_mut(),
            cells_bytes: 0,
            cell_stride: 0,
            session: 0,
            flags: 0,
        }
    }
}

/// Everything about the terminal view that is not a cell.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct TermViewC {
    /// `VIEW_*` bits.
    pub flags: u32,
    /// The character grid that fits `area`. This is what the widget
    /// reports back through [`PluginApi::grid`], and what the PTY is
    /// resized to.
    pub cols: u32,
    pub rows: u32,
    /// The grid actually delivered: `cols`/`rows` clamped to what the
    /// terminal has. They differ for exactly one frame after a resize,
    /// because the reported grid is applied on the NEXT frame. Rows are
    /// `view_cols` cells apart.
    pub view_cols: u32,
    pub view_rows: u32,
    pub cell_w: f32,
    pub cell_h: f32,
    /// Glyph size for [`HostApi::text`].
    pub px: f32,
    /// Ascent at `px`, for a widget that wants to place something on the
    /// baseline itself. `text` already applies it.
    pub ascent: f32,
    /// Cursor cell, meaningful only with `VIEW_CURSOR`. `cursor_col` may
    /// sit outside `view_cols`: the block is deliberately not clipped to
    /// the grid, which is what the terminal view reports.
    pub cursor_col: u32,
    pub cursor_row: u32,
    /// The scalar under the cursor, so drawing it needs no lookup into a
    /// buffer that may not reach that far.
    pub cursor_ch: u32,
    /// Scrollback lines the view is scrolled up by; 0 when live.
    pub view_offset: u32,
    /// One bit per session tab, bit 0 = tab 0, set when occupied.
    pub tabs: u32,
    /// How many of those bits mean anything. A widget divides its tab
    /// strip by THIS, so the host can grow past five tabs without a
    /// rebuild.
    pub tab_count: u32,
    pub tab_active: u32,
    pub cursor_fg: ColorC,
    pub cursor_bg: ColorC,
    // ------------------------------------------------------------------
    // Appended past TERM_VIEW_SIZE_MIN — prefix-written, so an old
    // caller gets the front it knows and a new caller on an old host
    // keeps the zeros it initialised (`TermViewC::empty`).
    // ------------------------------------------------------------------
    /// The 64-bit monotonic line id of the FIRST delivered view row,
    /// split into two words to keep four-byte alignment on every
    /// target. A widget echoes it back in [`TermSelectC`] so a drag
    /// resolves against the view it actually drew — fast output between
    /// the draw and the input event must not make the selection jump
    /// under the cursor (F1 §2.7 red-team). Zero on a host older than
    /// this field, where `ACTION_TERM_SELECT` is unknown anyway.
    pub first_id_lo: u32,
    pub first_id_hi: u32,
}

impl TermViewC {
    /// Zeroed. A caller MUST start from this: a host older than the
    /// header the caller was built against leaves the tail untouched,
    /// and stack garbage read as a colour is the kind of failure that
    /// leaves no trace.
    pub const fn empty() -> TermViewC {
        const NIL: ColorC = ColorC { r: 0.0, g: 0.0, b: 0.0, a: 0.0 };
        TermViewC {
            flags: 0,
            cols: 0,
            rows: 0,
            view_cols: 0,
            view_rows: 0,
            cell_w: 0.0,
            cell_h: 0.0,
            px: 0.0,
            ascent: 0.0,
            cursor_col: 0,
            cursor_row: 0,
            cursor_ch: 0,
            view_offset: 0,
            tabs: 0,
            tab_count: 0,
            tab_active: 0,
            cursor_fg: NIL,
            cursor_bg: NIL,
            first_id_lo: 0,
            first_id_hi: 0,
        }
    }
}

/// There is a terminal. Without this the struct carries only metrics and
/// the tab strip, and the terminal view draws nothing at all.
pub const VIEW_LIVE: u32 = 1;
/// The cursor is visible, the view is live, and its row is inside the
/// delivered grid. Blinking is the widget's business.
pub const VIEW_CURSOR: u32 = 2;
/// Fewer rows were delivered than `view_rows` would have been, because
/// the buffer was too small. Grow it to `cols * rows` cells and ask
/// again.
pub const VIEW_TRUNCATED: u32 = 4;

/// The prefix of each struct that a v2 host understands. Expressed as an
/// offset rather than a literal so it stays right on every target — and
/// so that appending a field does not move it.
pub const TERM_REQ_SIZE_MIN: usize =
    std::mem::offset_of!(TermReqC, flags) + std::mem::size_of::<u32>();
pub const TERM_VIEW_SIZE_MIN: usize =
    std::mem::offset_of!(TermViewC, cursor_bg) + std::mem::size_of::<ColorC>();

/// Version of the plugin interface. Raised whenever [`HostApi`] changes
/// in a way an existing plugin would not survive.
/// One baked rung of the state ladder, as it crosses the boundary. Prefix-
/// writable: the host writes min(out_size, size_of) bytes, so fields may only
/// ever be APPENDED here, exactly like the api structs themselves.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct StateStyleC {
    pub fill: ColorC,
    pub edge: ColorC,
    pub text: ColorC,
    pub glyph: ColorC,
    pub edge_width: f32,
    pub glow_radius: f32,
    pub glow_alpha: f32,
    pub elevation: f32,
}

// 6: BOTH tables gained `api_size` right after `abi_version` — the host
// measures a plugin's table, a plugin measures the host's. Inserting a
// field moved every later one, which is exactly the break the version
// number exists to name; from here on an APPENDED entry on either side
// needs no bump, because `api_size` says how much of a table its writer
// actually filled. `PluginApi` grows past `chrome` that way, `HostApi`
// past `theme_epoch` (`theme_enum_word` and `mask_quad` are the first,
// gated by `HostApi::has_*` on the plugin side). Four holes closed at
// once — the focused key with its modifiers, the press/release pair, the
// host-drawn tooltip and the widget-to-widget channel — appended in ONE
// growth for one reason: every separate growth is a separate migration
// of eight plugins, and the migration is the expensive part, not the
// four function pointers. The settings pair came later and alone,
// because it closed a hole the other four did not share: a plugin could
// be told what the theme says and what its neighbour published, and had
// no way at all to be told what its own user asked for.
pub const ABI_VERSION: u32 = 6;

/// A widget's container declaration, crossing the boundary (u2 §4.3).
///
/// The host asks [`PluginApi::chrome`] once per frame, before `draw`;
/// the plugin prefix-writes this and answers the bytes written, 0 for
/// "no chrome". The two strings live in the PLUGIN, valid until its next
/// `chrome` call — the same discipline the click path's `last_path`
/// already uses. The `{version, size}` header is what lets fields be
/// APPENDED later without a new ABI break.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct ChromeC {
    /// [`CHROME_VERSION`] of the writer.
    pub version: u32,
    /// The writer's own sizeof; readers touch `min(their own, this)`.
    pub size: u32,
    /// The title band's left text (UTF-8, not a C string); null = none.
    pub title: *const u8,
    pub title_len: u32,
    /// The band's right text — a cwd, a model name; null = none. The
    /// HOST trims it to the room the title leaves.
    pub right: *const u8,
    pub right_len: u32,
    /// `CHROME_BUTTONS_*`. Declared, not yet drawn by the host.
    pub buttons: u32,
    /// Index into the severity set, or `u32::MAX` for none.
    pub severity: u32,
}

pub const CHROME_VERSION: u32 = 1;
pub const CHROME_BUTTONS_NONE: u32 = 0;
pub const CHROME_BUTTONS_CLOSE: u32 = 1;
pub const CHROME_BUTTONS_MIN_CLOSE: u32 = 2;
pub const CHROME_BUTTONS_MIN_MAX_CLOSE: u32 = 3;

/// The prefix of [`ChromeC`] every reader understands.
pub const CHROME_SIZE_MIN: usize =
    std::mem::offset_of!(ChromeC, severity) + std::mem::size_of::<u32>();

impl ChromeC {
    /// No chrome at all. A caller MUST start from this, so a writer
    /// built against a shorter struct leaves a well-defined tail.
    pub const fn empty() -> ChromeC {
        ChromeC {
            version: CHROME_VERSION,
            size: std::mem::size_of::<ChromeC>() as u32,
            title: std::ptr::null(),
            title_len: 0,
            right: std::ptr::null(),
            right_len: 0,
            buttons: CHROME_BUTTONS_NONE,
            severity: u32::MAX,
        }
    }
}

/// [`PluginApi::sizing`]: the widget grows downwards.
pub const SIZING_ROWS: f32 = -1.0;
/// [`PluginApi::sizing`]: the widget is sized against its reference box.
pub const SIZING_REFERENCE: f32 = -2.0;

/// The name a plugin must export for the host to attach it. A library
/// without it is not a widget plugin and is not loaded.
pub const ATTACH_SYMBOL: &[u8] = b"nacelle_plugin_attach";

static API: AtomicPtr<HostApi> = AtomicPtr::new(std::ptr::null_mut());
static ATTACHED: AtomicBool = AtomicBool::new(false);
static WARNED: AtomicBool = AtomicBool::new(false);

/// Whether this copy owns the shared state. True until it is attached
/// to a host, so an application is the host by simply existing and a
/// plugin becomes a forwarder the moment it attaches.
pub fn is_host() -> bool {
    !ATTACHED.load(Ordering::Acquire)
}

/// Points this copy at the host's state. A plugin calls this from its
/// attach point; the pointer must stay valid for the program's life,
/// which it does because the host owns it.
///
/// Returns false when the interface versions do not match, in which case
/// nothing is attached and the plugin must not be used.
///
/// # Safety
/// `api` must point at a `HostApi` the host keeps alive.
pub unsafe fn attach(api: *const HostApi) -> bool {
    // A NEWER host is fine: fields are only ever appended, so every
    // entry this copy was built against is still where it was. An older
    // one is not — the entries it expects may not be there at all, and
    // reading one runs off the end of the host's table. This is the
    // check the header has always promised and never performed.
    if api.is_null() || (*api).abi_version < ABI_VERSION {
        return false;
    }
    // A version-6 table carries `api_size`. One that does not even reach
    // the version's mandatory prefix is malformed rather than merely
    // old, and reading any entry from it would run off its end.
    if ((*api).api_size as usize) < HOST_API_SIZE_MIN {
        return false;
    }
    API.store(api as *mut HostApi, Ordering::Release);
    ATTACHED.store(true, Ordering::Release);
    true
}

/// The host's interface, if this copy is attached to one.
fn api() -> Option<&'static HostApi> {
    let p = API.load(Ordering::Acquire);
    if p.is_null() {
        None
    } else {
        // The host owns this for the life of the program.
        Some(unsafe { &*p })
    }
}

/// Says once that this copy is orphaned. Called on the paths where a
/// plugin would otherwise fail silently.
fn warn_detached(what: &str) {
    if !WARNED.swap(true, Ordering::Relaxed) {
        eprintln!(
            "nacelle: {what} was called by a copy of the toolkit that attached \
             to a host and then lost it — the call is being dropped."
        );
    }
}

/// Routes a shared-state call: runs `local` in the host, forwards
/// through the host interface in an attached plugin, and complains
/// exactly once in a copy that is neither.
pub(crate) fn shared<T>(
    what: &str,
    local: impl FnOnce() -> T,
    forwarded: impl FnOnce(&HostApi) -> T,
    dropped: T,
) -> T {
    if is_host() {
        return local();
    }
    match api() {
        Some(api) => forwarded(api),
        None => {
            warn_detached(what);
            dropped
        }
    }
}

/// The same routing decision handed to ONE closure — `None` meaning "you
/// are the host, do it yourself".
///
/// [`shared`] takes the two paths as two closures, which is the nicer
/// shape until a call needs a `&mut` buffer: both closures would have to
/// capture it, and two live mutable borrows of one slice is not a thing
/// that compiles. The channel's `read_into` is exactly that call, so the
/// router has a second form rather than the caller having a second copy
/// of the routing rule.
pub(crate) fn shared_with<T>(
    what: &str,
    f: impl FnOnce(Option<&HostApi>) -> T,
    dropped: T,
) -> T {
    if is_host() {
        return f(None);
    }
    match api() {
        Some(api) => f(Some(api)),
        None => {
            warn_detached(what);
            dropped
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An application is the host without saying anything, which is
    /// what keeps the ordinary case free of ceremony.
    #[test]
    fn a_plain_program_owns_its_state() {
        assert!(is_host());
    }

    /// A right-versioned table still has to reach the version's own
    /// mandatory prefix: `api_size` is how a truncated or garbage table
    /// is caught before any entry is read off its end.
    #[test]
    fn a_host_table_shorter_than_the_minimum_is_refused() {
        let mut wrong = *crate::plugin::host_api();
        wrong.api_size = 8;
        assert!(!unsafe { attach(&wrong) }, "a truncated host table must be refused");
        assert!(api().is_none());
        assert!(is_host());
    }

    /// The key-name contract, from both ends. It exists because the
    /// host and the plugins used to guess it independently; a test that
    /// only checked one direction would let them start guessing again.
    #[test]
    fn every_named_key_round_trips_through_its_word() {
        use crate::focus::Key;
        let named = [
            (Key::Enter, keys::ENTER),
            (Key::Escape, keys::ESC),
            (Key::Backspace, keys::BACK),
            (Key::Space, keys::SPACE),
            (Key::Tab, keys::TAB),
            (Key::Up, keys::UP),
            (Key::Down, keys::DOWN),
            (Key::Left, keys::LEFT),
            (Key::Right, keys::RIGHT),
            (Key::Home, keys::HOME),
            (Key::End, keys::END),
            (Key::Delete, keys::DELETE),
            (Key::PageUp, keys::PAGE_UP),
            (Key::PageDown, keys::PAGE_DOWN),
        ];
        assert_eq!(named.len(), keys::ALL.len(), "the table and the list are one table");
        for (key, word) in named {
            assert_eq!(keys::name_of(key), Some(word));
            assert_eq!(keys::from_name(word), Some(key));
            // A reader that lower-cases, or that never thought about
            // case at all, must still agree with the host.
            assert_eq!(keys::from_name(&word.to_ascii_lowercase()), Some(key));
            assert!(keys::ALL.contains(&word));
        }
        // A character rides in `ch`, and the keys the contract does not
        // name are not delivered by name at all — an application's
        // shortcuts stay the application's.
        assert_eq!(keys::name_of(Key::Char('a')), None);
        assert_eq!(keys::name_of(Key::Insert), None);
        assert_eq!(keys::name_of(Key::Menu), None);
        assert_eq!(keys::name_of(Key::F(6)), None);
        // A word from a newer host reads as "no key I know", never as
        // the wrong one.
        assert_eq!(keys::from_name("PAGE_SIDEWAYS"), None);
        assert_eq!(keys::from_name(""), None);
    }

    /// The modifier mask is `focus::Mods` carried as a number, and it
    /// survives the round trip. The bits themselves are asserted at
    /// COMPILE time beside their constants; this is the other half —
    /// that a mask built from them means the same set again.
    #[test]
    fn the_modifier_mask_is_the_toolkits_own_set() {
        use crate::focus::Mods;
        let m = Mods::CTRL | Mods::SHIFT;
        assert_eq!(m.bits() as u32, MODS_CTRL | MODS_SHIFT);
        assert_eq!(Mods::from_bits(m.bits()), m);
        assert_eq!(Mods::from_bits(MODS_NONE as u8), Mods::NONE);
        // A bit from a newer build is dropped rather than kept: an
        // unknown modifier must not stop a chord this build understands
        // from matching.
        assert_eq!(Mods::from_bits(0b1111_0001), Mods::CTRL);
    }

    #[test]
    fn an_older_interface_refuses_to_attach() {
        // The version is checked before any other field is touched, so
        // a table of the right shape with the wrong number is enough.
        let mut wrong = *crate::plugin::host_api();
        wrong.abi_version = ABI_VERSION - 1;
        assert!(!unsafe { attach(&wrong) }, "an older interface must be refused");
        assert!(!unsafe { attach(std::ptr::null()) }, "null must be refused");
        // Refusing to attach must leave the copy owning its own state
        // rather than half-attached to something it cannot speak to.
        assert!(api().is_none());
        assert!(is_host());
    }
}
