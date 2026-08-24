//! Keyboard focus, navigation and shortcuts (F1 §1).
//!
//! Two orthogonal levels, exactly as the theme's §5.21 draws them:
//!
//! * **Container focus** — which panel/window owns the keyboard — stays a
//!   bool the APPLICATION keeps per container; it consumes `border.focus`,
//!   `panel.border_focused` and `focus.unfocused_dim` in the container
//!   chrome pass and never appears in this module.
//! * **Control focus** — which control inside the focused container takes
//!   keys — is [`FocusCtl`]: one focused [`FocusId`] per world, a chain
//!   rebuilt every frame from registration order (= draw order = visual
//!   order), and the focus-visible bit that keeps the boot frame
//!   pixel-identical: the ring exists only after keyboard navigation has
//!   happened, and a pointer press hides it again.
//!
//! Everything here is platform-neutral. The application translates its
//! window library's events into [`KeyEv`]; libnacelle never learns winit.
//! Focus is NOT a state-ladder slot (the ladder has seven rungs and §5.21
//! forbids an eighth) — the only focus signal a control draws is the
//! overlay ring, [`crate::object::focus_ring`].

use crate::access::AccessInfo;
use crate::Rect;

// ---------------------------------------------------------------------
// Identity
// ---------------------------------------------------------------------

/// A control's identity in the focus chain: FNV-1a of a STABLE path
/// string — `"settings.tab.look"`, `"panel.shell"`, `"editor.btn.save"`.
/// Paths, never indices: panels reorder, and focus must survive a board
/// ride or a layout switch. A `u64`, so the value is already
/// C-representable for the plugin ABI append a later phase makes.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct FocusId(pub u64);

impl FocusId {
    /// The id of a stable path string.
    pub fn of(path: &str) -> FocusId {
        FocusId(fnv1a(0xcbf2_9ce4_8422_2325, path.as_bytes()))
    }

    /// A child id derived from this one — the i-th row of an open
    /// dropdown. An index is legal HERE because a list's order is its
    /// content's order; it is panels that reorder, not a list's rows.
    pub fn item(self, i: usize) -> FocusId {
        FocusId(fnv1a(self.0, &(i as u64).to_le_bytes()))
    }
}

fn fnv1a(seed: u64, bytes: &[u8]) -> u64 {
    let mut h = seed;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100_0000_01b3);
    }
    h
}

// ---------------------------------------------------------------------
// Capabilities — what the focused control eats
// ---------------------------------------------------------------------

/// What the focused control consumes before navigation sees a key.
/// A `u8` bitset (hand-rolled — no new dependency), C-representable for
/// the later plugin append. The terminal registers all three: while it
/// owns focus, Tab and arrows become PTY bytes, exactly as before focus
/// existed — that is what keeps the boot default behaviourally identical.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Caps(u8);

impl Caps {
    pub const NONE: Caps = Caps(0);
    /// Typed text goes to the control (a field, the terminal).
    pub const TEXT: Caps = Caps(1 << 0);
    /// Tab is the control's, not the chain's.
    pub const GREEDY_TAB: Caps = Caps(1 << 1);
    /// Arrows are the control's (a slider adjusts, a terminal sends bytes).
    pub const GREEDY_ARROWS: Caps = Caps(1 << 2);

    pub const fn contains(self, other: Caps) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn bits(self) -> u8 {
        self.0
    }
    pub const fn from_bits(bits: u8) -> Caps {
        Caps(bits & 0b111)
    }
}

impl std::ops::BitOr for Caps {
    type Output = Caps;
    fn bitor(self, rhs: Caps) -> Caps {
        Caps(self.0 | rhs.0)
    }
}

// ---------------------------------------------------------------------
// The chain
// ---------------------------------------------------------------------

/// What the chain answers a control at registration, same frame.
#[derive(Clone, Copy, Debug)]
pub struct FocusState {
    /// This control owns the keyboard.
    pub focused: bool,
    /// Draw the overlay ring: focused, AND keyboard navigation has
    /// happened since the last pointer press (the focus-visible rule),
    /// AND nothing suppresses rings (a board ride mid-flight does).
    pub ring: bool,
}

/// Where navigation goes. `Next`/`Prev` walk the chain in registration
/// order; the four directions are spatial — nearest rect centre in the
/// half plane, which serves the settings grid today and a gamepad later.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Nav {
    Next,
    Prev,
    Left,
    Right,
    Up,
    Down,
}

impl Nav {
    /// The navigation a key means, if any: Tab and Shift+Tab walk the
    /// chain, BARE arrows move spatially. Arrows or Tab under any other
    /// modifier are a shortcut's business, never navigation.
    pub fn of(ev: &KeyEv) -> Option<Nav> {
        let dir = match ev.key {
            Key::Tab if ev.mods == Mods::NONE => Nav::Next,
            Key::Tab if ev.mods == Mods::SHIFT => Nav::Prev,
            Key::Left => Nav::Left,
            Key::Right => Nav::Right,
            Key::Up => Nav::Up,
            Key::Down => Nav::Down,
            _ => return None,
        };
        if dir != Nav::Next && dir != Nav::Prev && ev.mods != Mods::NONE {
            return None;
        }
        Some(dir)
    }
}

#[derive(Clone)]
struct Entry {
    id: FocusId,
    rect: Rect,
    caps: Caps,
    group: u16,
    /// This control's accessible role, name and state — carried beside
    /// the Tab-chain entry rather than through [`crate::access::AccessCtl`]
    /// (see that module's header) because a FOCUSABLE control's report
    /// must stay reachable exactly where its focus state already is.
    access: AccessInfo,
}

/// The per-world focus chain — owned by the application beside its other
/// per-world state (`SizeTable`), reaching draw code through
/// [`crate::Ctx::focus`]. Objects are immediate-mode, so the chain is
/// rebuilt every frame from registration order.
///
/// Frame contract: controls register WHILE DRAWING and are answered from
/// the same frame's focus (no one-frame lag); [`FocusCtl::begin_frame`]
/// is called once per frame at the frame BOUNDARY — after the world has
/// drawn, before the next frame's events — so navigation always walks
/// the last COMPLETED chain, never the half-built one (Tab must not
/// depend on which panel drew first).
#[derive(Default)]
pub struct FocusCtl {
    focused: Option<FocusId>,
    /// The focus-visible bit: true after keyboard navigation, false
    /// after any pointer press. At boot nothing has happened → false →
    /// no ring in the boot frame, which the pixel rule demands.
    visible: bool,
    /// Rings withheld while rects are mid-flight (the board cube ride).
    ring_suppressed: bool,
    /// Stamp for subsequent registrations; reset each frame.
    group: u16,
    /// This frame's chain, in registration order.
    cur: Vec<Entry>,
    /// The last completed frame — what navigation walks.
    prev: Vec<Entry>,
}

impl FocusCtl {
    pub fn new() -> FocusCtl {
        FocusCtl::default()
    }

    /// The frame boundary: the chain built this frame becomes the one
    /// navigation walks, and the next frame starts empty. A focused
    /// control that did not register in the completed frame has
    /// vanished (layout switch, settings closed): focus drops to None
    /// and the next Tab restarts at the chain head.
    pub fn begin_frame(&mut self) {
        std::mem::swap(&mut self.prev, &mut self.cur);
        self.cur.clear();
        self.group = 0;
        if let Some(id) = self.focused {
            if !self.prev.iter().any(|e| e.id == id) {
                self.focused = None;
            }
        }
    }

    /// Called by a control WHILE DRAWING: registers into this frame's
    /// chain and answers from this frame's focus — no one-frame lag.
    /// `access` is the control's accessible role/name/state
    /// ([`crate::access::AccessInfo`]) — it rides beside the Tab-chain
    /// entry rather than through [`crate::access::AccessCtl`], see that
    /// module's header for why a focusable control never uses the
    /// structural registry.
    pub fn register(
        &mut self,
        id: FocusId,
        r: Rect,
        caps: Caps,
        access: AccessInfo,
    ) -> FocusState {
        self.cur.push(Entry { id, rect: r, caps, group: self.group, access });
        let focused = self.focused == Some(id);
        FocusState {
            focused,
            ring: focused && self.visible && !self.ring_suppressed,
        }
    }

    /// Stamps every registration that follows with a navigation group —
    /// the "focused container's group" Tab wraps within. The container
    /// pass sets it per panel/layer before the container's controls
    /// draw; ungrouped worlds leave everything at 0 and Tab walks the
    /// whole chain. Reset to 0 at every frame boundary.
    pub fn set_group(&mut self, g: u16) {
        self.group = g;
    }

    /// Keyboard navigation over the last completed chain. Makes the
    /// ring visible. Returns whether focus moved (or landed).
    pub fn nav(&mut self, n: Nav) -> bool {
        self.visible = true;
        if self.prev.is_empty() {
            return false;
        }
        let at = self.focused.and_then(|id| self.prev.iter().position(|e| e.id == id));
        let Some(i) = at else {
            // Nothing focused: any navigation lands on the chain head.
            self.focused = Some(self.prev[0].id);
            return true;
        };
        let g = self.prev[i].group;
        match n {
            Nav::Next | Nav::Prev => {
                let idxs: Vec<usize> =
                    (0..self.prev.len()).filter(|&j| self.prev[j].group == g).collect();
                let pos = idxs.iter().position(|&j| j == i).unwrap_or(0);
                let step = if n == Nav::Next { 1 } else { idxs.len() - 1 };
                let j = idxs[(pos + step) % idxs.len()];
                self.focused = Some(self.prev[j].id);
                true
            }
            _ => {
                // Spatial: nearest rect centre in the direction's half
                // plane, within the group. No wrap — an arrow past the
                // edge is simply not navigation.
                let c = centre(self.prev[i].rect);
                let mut best: Option<(f32, usize)> = None;
                for (j, e) in self.prev.iter().enumerate() {
                    if j == i || e.group != g {
                        continue;
                    }
                    let ec = centre(e.rect);
                    let ahead = match n {
                        Nav::Left => ec.0 < c.0 - 0.5,
                        Nav::Right => ec.0 > c.0 + 0.5,
                        Nav::Up => ec.1 < c.1 - 0.5,
                        Nav::Down => ec.1 > c.1 + 0.5,
                        Nav::Next | Nav::Prev => unreachable!(),
                    };
                    if !ahead {
                        continue;
                    }
                    let (dx, dy) = (ec.0 - c.0, ec.1 - c.1);
                    let d = dx * dx + dy * dy;
                    if best.map_or(true, |(bd, _)| d < bd) {
                        best = Some((d, j));
                    }
                }
                match best {
                    Some((_, j)) => {
                        self.focused = Some(self.prev[j].id);
                        true
                    }
                    None => false,
                }
            }
        }
    }

    /// Pointer or programmatic focus: typing lands where the user
    /// clicked, but a click never summons the ring (focus-visible
    /// becomes false again).
    pub fn focus(&mut self, id: Option<FocusId>) {
        self.focused = id;
        self.visible = false;
    }

    /// Keyboard-driven focus jump (F6 landing on a container's first
    /// control): like [`FocusCtl::focus`], but the ring shows — the
    /// user is navigating by key.
    pub fn focus_by_key(&mut self, id: Option<FocusId>) {
        self.focused = id;
        self.visible = true;
    }

    pub fn focused(&self) -> Option<FocusId> {
        self.focused
    }

    /// The focus-visible bit — the container chrome pass reads it too:
    /// until the first Tab/F6 the desktop draws exactly as today.
    pub fn visible(&self) -> bool {
        self.visible
    }

    /// The focused control's capabilities, from the last completed
    /// frame. NONE while nothing is focused (keys route to the app).
    pub fn caps(&self) -> Caps {
        self.focused
            .and_then(|id| self.prev.iter().find(|e| e.id == id))
            .map(|e| e.caps)
            .unwrap_or(Caps::NONE)
    }

    /// Where a control sat in the last completed frame — the anchor for
    /// a Shift+F10 context menu, the IME cursor area.
    pub fn rect_of(&self, id: FocusId) -> Option<Rect> {
        self.prev.iter().find(|e| e.id == id).map(|e| e.rect)
    }

    /// Every focusable control's accessible report, from the last
    /// completed frame — the per-frame read a future AT-SPI bridge in
    /// nacelle-desktop will use. Building that bridge is not this
    /// crate's job; this accessor only provides what it will need.
    pub fn entries(&self) -> impl Iterator<Item = (FocusId, Rect, &AccessInfo)> {
        self.prev.iter().map(|e| (e.id, e.rect, &e.access))
    }

    /// Withholds every ring while rects are mid-flight — the board cube
    /// ride redraws panels at moving rects; focus survives the ride,
    /// the ring waits it out.
    pub fn set_ring_suppressed(&mut self, on: bool) {
        self.ring_suppressed = on;
    }
}

fn centre(r: Rect) -> (f32, f32) {
    (r.x + r.w / 2.0, r.y + r.h / 2.0)
}

// ---------------------------------------------------------------------
// Neutral key events
// ---------------------------------------------------------------------

/// Modifier set — a hand-rolled `u8` bitset like [`Caps`].
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct Mods(u8);

impl Mods {
    pub const NONE: Mods = Mods(0);
    pub const CTRL: Mods = Mods(1 << 0);
    pub const SHIFT: Mods = Mods(1 << 1);
    pub const ALT: Mods = Mods(1 << 2);
    pub const SUPER: Mods = Mods(1 << 3);

    pub const fn contains(self, other: Mods) -> bool {
        self.0 & other.0 == other.0
    }

    /// The raw bits — what [`crate::runtime::PluginApi::key`] carries
    /// across the boundary, where a Rust type means nothing. `const`
    /// because the ABI's own constants are checked against it at compile
    /// time (`runtime::MODS_CTRL` and friends).
    pub const fn bits(self) -> u8 {
        self.0
    }

    /// A set from bits that crossed the boundary. Bits this build does
    /// not know are dropped rather than kept, exactly like [`Caps`]: an
    /// unknown modifier held down must not stop a chord this build DOES
    /// understand from matching.
    pub const fn from_bits(bits: u8) -> Mods {
        Mods(bits & 0b1111)
    }
}

impl std::ops::BitOr for Mods {
    type Output = Mods;
    fn bitor(self, rhs: Mods) -> Mods {
        Mods(self.0 | rhs.0)
    }
}

/// The neutral key set — what the application translates its window
/// library's codes into. `Menu` is the dedicated context-menu key: §1.4
/// (red-team) requires `shift+f10`/`menu` bindable OVER_GREEDY, so the
/// key exists in the neutral set.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Key {
    Char(char),
    Enter,
    Escape,
    Tab,
    Backspace,
    Delete,
    Space,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Menu,
    F(u8),
}

/// One key event, already translated from the platform.
#[derive(Clone, PartialEq, Debug)]
pub struct KeyEv {
    pub key: Key,
    pub mods: Mods,
    pub repeat: bool,
    /// The text the platform produced for this press, when any — what a
    /// TEXT-caps control inserts. Kept beside the key, not instead of
    /// it, so shortcuts match the key while fields consume the text.
    pub text: Option<String>,
}

/// A parsed binding: modifiers plus one key.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Chord {
    pub mods: Mods,
    pub key: Key,
}

impl Chord {
    /// Parses `"ctrl+shift+c"` — case-insensitive, modifiers in any
    /// order, the key last. Modifier words: `ctrl`/`control`, `shift`,
    /// `alt`, `super`. Key words: a single character, `enter`/`return`,
    /// `escape`/`esc`, `tab`, `backspace`, `delete`/`del`, `space`,
    /// `left`/`right`/`up`/`down`, `home`, `end`, `pageup`, `pagedown`,
    /// `insert`, `menu`, `f1`..`f24`.
    pub fn parse(s: &str) -> Option<Chord> {
        let mut mods = Mods::NONE;
        let mut key: Option<Key> = None;
        for part in s.split('+') {
            let p = part.trim().to_ascii_lowercase();
            if p.is_empty() || key.is_some() {
                return None; // empty part, or a modifier after the key
            }
            match p.as_str() {
                "ctrl" | "control" => mods = mods | Mods::CTRL,
                "shift" => mods = mods | Mods::SHIFT,
                "alt" => mods = mods | Mods::ALT,
                "super" => mods = mods | Mods::SUPER,
                w => key = Some(key_word(w)?),
            }
        }
        Some(Chord { mods, key: key? })
    }

    /// Whether this chord is the event: modifiers exactly equal, keys
    /// equal with characters compared case-insensitively (Shift is
    /// already in the modifier set; the produced character's case must
    /// not double-count it).
    pub fn matches(&self, ev: &KeyEv) -> bool {
        if self.mods != ev.mods {
            return false;
        }
        match (self.key, ev.key) {
            (Key::Char(a), Key::Char(b)) => {
                a.to_ascii_lowercase() == b.to_ascii_lowercase()
            }
            (a, b) => a == b,
        }
    }

    /// The human-readable form — `"Ctrl+Shift+C"` — printed beside menu
    /// rows. One source of truth with matching, so labels cannot drift.
    pub fn label(&self) -> String {
        let mut s = String::new();
        if self.mods.contains(Mods::CTRL) {
            s.push_str("Ctrl+");
        }
        if self.mods.contains(Mods::SHIFT) {
            s.push_str("Shift+");
        }
        if self.mods.contains(Mods::ALT) {
            s.push_str("Alt+");
        }
        if self.mods.contains(Mods::SUPER) {
            s.push_str("Super+");
        }
        match self.key {
            Key::Char(c) => s.push(c.to_ascii_uppercase()),
            Key::F(n) => s.push_str(&format!("F{n}")),
            Key::Enter => s.push_str("Enter"),
            Key::Escape => s.push_str("Escape"),
            Key::Tab => s.push_str("Tab"),
            Key::Backspace => s.push_str("Backspace"),
            Key::Delete => s.push_str("Delete"),
            Key::Space => s.push_str("Space"),
            Key::Left => s.push_str("Left"),
            Key::Right => s.push_str("Right"),
            Key::Up => s.push_str("Up"),
            Key::Down => s.push_str("Down"),
            Key::Home => s.push_str("Home"),
            Key::End => s.push_str("End"),
            Key::PageUp => s.push_str("PageUp"),
            Key::PageDown => s.push_str("PageDown"),
            Key::Insert => s.push_str("Insert"),
            Key::Menu => s.push_str("Menu"),
        }
        s
    }
}

fn key_word(w: &str) -> Option<Key> {
    Some(match w {
        "enter" | "return" => Key::Enter,
        "escape" | "esc" => Key::Escape,
        "tab" => Key::Tab,
        "backspace" => Key::Backspace,
        "delete" | "del" => Key::Delete,
        "space" => Key::Space,
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "home" => Key::Home,
        "end" => Key::End,
        "pageup" => Key::PageUp,
        "pagedown" => Key::PageDown,
        "insert" => Key::Insert,
        "menu" => Key::Menu,
        _ => {
            if let Some(n) = w.strip_prefix('f').and_then(|d| d.parse::<u8>().ok()) {
                if (1..=24).contains(&n) {
                    return Some(Key::F(n));
                }
            }
            let mut cs = w.chars();
            match (cs.next(), cs.next()) {
                (Some(c), None) => Key::Char(c),
                _ => return None,
            }
        }
    })
}

// ---------------------------------------------------------------------
// The shortcut registry
// ---------------------------------------------------------------------

/// Where a binding applies. The router builds the scope list top-down —
/// `[Layer(top), Panel(focused container), Global]` — and the first
/// scope holding a match wins, so a layer's Escape shadows a global one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Scope {
    Global,
    Layer(u32),
    Panel(u16),
    /// Dispatched only against the focused control (Ctrl+Z to a
    /// TEXT-caps field, while a terminal still gets its literal bytes).
    Focused,
}

/// Binding flags — a `u8` bitset like [`Caps`].
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct ShortcutFlags(u8);

impl ShortcutFlags {
    pub const NONE: ShortcutFlags = ShortcutFlags(0);
    /// Matches even while the focused control is greedy — the escape
    /// hatches (Ctrl+Shift+Q, F11, F6, copy/paste, the menu key) that
    /// must work over the terminal.
    pub const OVER_GREEDY: ShortcutFlags = ShortcutFlags(1 << 0);

    pub const fn contains(self, other: ShortcutFlags) -> bool {
        self.0 & other.0 == other.0
    }
}

struct Bind {
    scope: Scope,
    chord: Chord,
    cmd: u32,
    flags: ShortcutFlags,
}

/// The declarative shortcut registry: chords bound to application
/// command ids per scope. One map per world, owned by the application's
/// router; menu rows print [`ShortcutMap::hint`] beside their labels so
/// a changed binding can never leave a lying menu.
#[derive(Default)]
pub struct ShortcutMap {
    binds: Vec<Bind>,
}

impl ShortcutMap {
    pub fn new() -> ShortcutMap {
        ShortcutMap::default()
    }

    /// Binds `chord` (the [`Chord::parse`] grammar) to `cmd` in `scope`.
    /// An unparseable chord is a programmer error: rejected loudly in
    /// debug builds, ignored in release (bindings come from code, not
    /// from user input).
    pub fn bind(&mut self, scope: Scope, chord: &str, cmd: u32, flags: ShortcutFlags) {
        match Chord::parse(chord) {
            Some(c) => self.binds.push(Bind { scope, chord: c, cmd, flags }),
            None => debug_assert!(false, "unparseable chord {chord:?}"),
        }
    }

    /// The label of `cmd`'s first binding — what a menu row prints.
    pub fn hint(&self, cmd: u32) -> Option<String> {
        self.binds.iter().find(|b| b.cmd == cmd).map(|b| b.chord.label())
    }

    /// The first command matching `ev`, walking `scopes` in the given
    /// (top-down) order. All bindings are candidates — the router calls
    /// this while the focused control is NOT greedy for the key.
    pub fn lookup(&self, scopes: &[Scope], ev: &KeyEv) -> Option<u32> {
        self.find(scopes, ev, ShortcutFlags::NONE)
    }

    /// The same walk restricted to OVER_GREEDY bindings — the router's
    /// step while a greedy control (the terminal) owns focus, so Tab
    /// and plain chords keep becoming bytes.
    pub fn lookup_over_greedy(&self, scopes: &[Scope], ev: &KeyEv) -> Option<u32> {
        self.find(scopes, ev, ShortcutFlags::OVER_GREEDY)
    }

    fn find(&self, scopes: &[Scope], ev: &KeyEv, need: ShortcutFlags) -> Option<u32> {
        for s in scopes {
            for b in &self.binds {
                if b.scope == *s && b.flags.contains(need) && b.chord.matches(ev) {
                    return Some(b.cmd);
                }
            }
        }
        None
    }
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn ev(key: Key, mods: Mods) -> KeyEv {
        KeyEv { key, mods, repeat: false, text: None }
    }

    fn r(x: f32, y: f32) -> Rect {
        Rect::new(x, y, 10.0, 10.0)
    }

    // ---- ids ----

    #[test]
    fn ids_are_stable_and_distinct() {
        assert_eq!(FocusId::of("settings.tab.look"), FocusId::of("settings.tab.look"));
        assert_ne!(FocusId::of("settings.tab.look"), FocusId::of("settings.tab.term"));
        let base = FocusId::of("settings.dropdown.theme");
        assert_ne!(base.item(0), base.item(1));
        assert_ne!(base.item(0), base);
    }

    // ---- chords ----

    #[test]
    fn chord_parses_mods_and_key() {
        let c = Chord::parse("ctrl+shift+c").unwrap();
        assert_eq!(c.mods, Mods::CTRL | Mods::SHIFT);
        assert_eq!(c.key, Key::Char('c'));
        assert_eq!(Chord::parse("F11").unwrap().key, Key::F(11));
        assert_eq!(Chord::parse("esc").unwrap().key, Key::Escape);
        assert_eq!(Chord::parse("escape").unwrap().key, Key::Escape);
        assert_eq!(Chord::parse("ctrl+tab").unwrap().key, Key::Tab);
        assert_eq!(Chord::parse("shift+f10").unwrap().mods, Mods::SHIFT);
        assert_eq!(Chord::parse("menu").unwrap().key, Key::Menu);
        assert_eq!(Chord::parse("f").unwrap().key, Key::Char('f'));
    }

    #[test]
    fn chord_rejects_nonsense() {
        assert!(Chord::parse("").is_none());
        assert!(Chord::parse("ctrl+").is_none());
        assert!(Chord::parse("ctrl+shift").is_none()); // modifiers, no key
        assert!(Chord::parse("c+ctrl").is_none()); // key must be last
        assert!(Chord::parse("f25").is_none());
        assert!(Chord::parse("banana").is_none());
    }

    #[test]
    fn chord_matches_case_insensitively_never_across_mods() {
        let c = Chord::parse("ctrl+shift+c").unwrap();
        assert!(c.matches(&ev(Key::Char('C'), Mods::CTRL | Mods::SHIFT)));
        assert!(c.matches(&ev(Key::Char('c'), Mods::CTRL | Mods::SHIFT)));
        assert!(!c.matches(&ev(Key::Char('c'), Mods::CTRL)));
        assert!(!c.matches(&ev(Key::Char('x'), Mods::CTRL | Mods::SHIFT)));
    }

    #[test]
    fn chord_labels_read_like_menu_hints() {
        assert_eq!(Chord::parse("ctrl+shift+c").unwrap().label(), "Ctrl+Shift+C");
        assert_eq!(Chord::parse("f6").unwrap().label(), "F6");
        assert_eq!(Chord::parse("shift+f10").unwrap().label(), "Shift+F10");
        assert_eq!(Chord::parse("ctrl+tab").unwrap().label(), "Ctrl+Tab");
    }

    // ---- navigation keys ----

    #[test]
    fn nav_of_maps_tab_and_bare_arrows_only() {
        assert_eq!(Nav::of(&ev(Key::Tab, Mods::NONE)), Some(Nav::Next));
        assert_eq!(Nav::of(&ev(Key::Tab, Mods::SHIFT)), Some(Nav::Prev));
        assert_eq!(Nav::of(&ev(Key::Left, Mods::NONE)), Some(Nav::Left));
        assert_eq!(Nav::of(&ev(Key::Tab, Mods::CTRL)), None); // a shortcut
        assert_eq!(Nav::of(&ev(Key::Left, Mods::SHIFT)), None);
        assert_eq!(Nav::of(&ev(Key::Char('a'), Mods::NONE)), None);
    }

    // ---- the chain ----

    #[test]
    fn register_answers_same_frame() {
        let mut fc = FocusCtl::new();
        let a = FocusId::of("a");
        // Frame 1: nothing focused yet.
        let st = fc.register(a, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        assert!(!st.focused && !st.ring);
        fc.begin_frame(); // frame boundary
        assert!(fc.nav(Nav::Next)); // first Tab lands on the head
        assert_eq!(fc.focused(), Some(a));
        // Frame 2: the control is answered focused the same frame.
        let st = fc.register(a, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        assert!(st.focused && st.ring);
    }

    #[test]
    fn tab_walks_registration_order_and_wraps() {
        let mut fc = FocusCtl::new();
        let (a, b, c) = (FocusId::of("a"), FocusId::of("b"), FocusId::of("c"));
        for id in [a, b, c] {
            fc.register(id, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        }
        fc.begin_frame();
        fc.nav(Nav::Next);
        assert_eq!(fc.focused(), Some(a));
        fc.nav(Nav::Next);
        assert_eq!(fc.focused(), Some(b));
        fc.nav(Nav::Next);
        fc.nav(Nav::Next); // wraps past c
        assert_eq!(fc.focused(), Some(a));
        fc.nav(Nav::Prev); // and back around
        assert_eq!(fc.focused(), Some(c));
    }

    #[test]
    fn pointer_focus_never_summons_the_ring() {
        let mut fc = FocusCtl::new();
        let a = FocusId::of("a");
        fc.register(a, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.begin_frame();
        fc.focus(Some(a)); // a click
        let st = fc.register(a, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        assert!(st.focused && !st.ring, "click moves focus silently");
        fc.begin_frame();
        fc.nav(Nav::Next); // keyboard navigation happened
        let st = fc.register(a, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        assert!(st.ring, "keyboard summons the ring");
        fc.focus(Some(a)); // any pointer press hides it again
        assert!(!fc.visible());
    }

    #[test]
    fn vanished_control_drops_focus_and_tab_restarts_at_head() {
        let mut fc = FocusCtl::new();
        let (a, b) = (FocusId::of("a"), FocusId::of("b"));
        fc.register(a, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.register(b, r(20.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.begin_frame();
        fc.focus_by_key(Some(a));
        // Next frame the layout switched: only b drew.
        fc.register(b, r(20.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.begin_frame();
        assert_eq!(fc.focused(), None);
        fc.nav(Nav::Next);
        assert_eq!(fc.focused(), Some(b));
    }

    #[test]
    fn spatial_nav_picks_nearest_centre_in_the_half_plane() {
        let mut fc = FocusCtl::new();
        // A 2x2 settings grid.
        let (a, b) = (FocusId::of("a"), FocusId::of("b"));
        let (c, d) = (FocusId::of("c"), FocusId::of("d"));
        fc.register(a, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.register(b, r(100.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.register(c, r(0.0, 100.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.register(d, r(100.0, 100.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.begin_frame();
        fc.focus_by_key(Some(a));
        assert!(fc.nav(Nav::Right));
        assert_eq!(fc.focused(), Some(b));
        assert!(fc.nav(Nav::Down));
        assert_eq!(fc.focused(), Some(d));
        assert!(fc.nav(Nav::Left));
        assert_eq!(fc.focused(), Some(c));
        assert!(fc.nav(Nav::Up));
        assert_eq!(fc.focused(), Some(a));
        assert!(!fc.nav(Nav::Up), "no wrap past the grid's edge");
        assert_eq!(fc.focused(), Some(a));
    }

    #[test]
    fn tab_wraps_within_the_group() {
        let mut fc = FocusCtl::new();
        let (a, b) = (FocusId::of("s.a"), FocusId::of("s.b"));
        let (x, y) = (FocusId::of("p.x"), FocusId::of("p.y"));
        fc.set_group(1);
        fc.register(a, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.register(b, r(20.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.set_group(2);
        fc.register(x, r(0.0, 50.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.register(y, r(20.0, 50.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.begin_frame();
        fc.focus_by_key(Some(b));
        fc.nav(Nav::Next); // wraps to a, never crosses into group 2
        assert_eq!(fc.focused(), Some(a));
        fc.nav(Nav::Prev);
        assert_eq!(fc.focused(), Some(b));
        // Spatial navigation honours the fence too.
        assert!(!fc.nav(Nav::Down));
        assert_eq!(fc.focused(), Some(b));
    }

    #[test]
    fn caps_and_rect_come_from_the_completed_frame() {
        let mut fc = FocusCtl::new();
        let term = FocusId::of("panel.shell");
        let greedy = Caps::TEXT | Caps::GREEDY_TAB | Caps::GREEDY_ARROWS;
        fc.register(term, Rect::new(5.0, 6.0, 300.0, 200.0), greedy, AccessInfo::new(crate::access::Role::Button, ""));
        fc.begin_frame();
        fc.focus(Some(term));
        assert!(fc.caps().contains(Caps::GREEDY_TAB));
        assert!(fc.caps().contains(Caps::TEXT));
        let rr = fc.rect_of(term).unwrap();
        assert_eq!((rr.x, rr.y, rr.w, rr.h), (5.0, 6.0, 300.0, 200.0));
        assert_eq!(fc.rect_of(FocusId::of("gone")).map(|r| r.x), None);
    }

    #[test]
    fn ring_suppression_wins_over_visibility() {
        let mut fc = FocusCtl::new();
        let a = FocusId::of("a");
        fc.register(a, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        fc.begin_frame();
        fc.nav(Nav::Next);
        fc.set_ring_suppressed(true); // the cube ride starts
        let st = fc.register(a, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        assert!(st.focused && !st.ring);
        fc.set_ring_suppressed(false); // and lands
        let st = fc.register(a, r(0.0, 0.0), Caps::NONE, AccessInfo::new(crate::access::Role::Button, ""));
        assert!(st.ring);
    }

    // ---- the registry ----

    #[test]
    fn lookup_walks_scopes_top_down() {
        let mut m = ShortcutMap::new();
        m.bind(Scope::Global, "escape", 1, ShortcutFlags::NONE);
        m.bind(Scope::Layer(7), "escape", 2, ShortcutFlags::NONE);
        let e = ev(Key::Escape, Mods::NONE);
        assert_eq!(m.lookup(&[Scope::Layer(7), Scope::Global], &e), Some(2));
        assert_eq!(m.lookup(&[Scope::Global], &e), Some(1));
        assert_eq!(m.lookup(&[Scope::Layer(9)], &e), None);
    }

    #[test]
    fn greedy_focus_keeps_tab_and_loses_only_over_greedy_chords() {
        // The desktop's worst possible regression: Tab must keep
        // reaching the shell. While the focused control is greedy the
        // router asks only for OVER_GREEDY bindings — Tab has none and
        // falls through to the terminal as bytes.
        let mut m = ShortcutMap::new();
        m.bind(Scope::Global, "tab", 10, ShortcutFlags::NONE);
        m.bind(Scope::Global, "ctrl+shift+q", 11, ShortcutFlags::OVER_GREEDY);
        m.bind(Scope::Global, "f6", 12, ShortcutFlags::OVER_GREEDY);
        let tab = ev(Key::Tab, Mods::NONE);
        let quit = ev(Key::Char('q'), Mods::CTRL | Mods::SHIFT);
        let cycle = ev(Key::F(6), Mods::NONE);
        assert_eq!(m.lookup_over_greedy(&[Scope::Global], &tab), None);
        assert_eq!(m.lookup_over_greedy(&[Scope::Global], &quit), Some(11));
        assert_eq!(m.lookup_over_greedy(&[Scope::Global], &cycle), Some(12));
        // A non-greedy focus sees everything.
        assert_eq!(m.lookup(&[Scope::Global], &tab), Some(10));
    }

    #[test]
    fn hints_come_from_the_binding() {
        let mut m = ShortcutMap::new();
        m.bind(Scope::Global, "ctrl+shift+c", 21, ShortcutFlags::OVER_GREEDY);
        assert_eq!(m.hint(21).as_deref(), Some("Ctrl+Shift+C"));
        assert_eq!(m.hint(99), None);
    }
}
