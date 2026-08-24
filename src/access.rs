//! Accessible role, name and state — libnacelle's own vocabulary, read
//! by a future AT-SPI (or UIA, or whatever a given desktop speaks)
//! bridge, but not a wrapper of any such crate.
//!
//! The reason is the same one [`crate::focus::Caps`] and
//! [`crate::focus::Mods`] already argue for keyboard input: libnacelle
//! stays neutral of the platform it eventually reports to, exactly as
//! it stays neutral of winit. A backend crate would pin one accessibility
//! stack into a library that also wants to run headless in a test, or
//! embedded in a host that speaks something else. So [`Role`], [`States`]
//! and [`AccessInfo`] are hand-rolled here, in the same bitset style as
//! `focus.rs`, and a later bridge translates them outward.
//!
//! Two registries exist for two different KINDS of accessible node, not
//! one generalised over both, because [`crate::focus::FocusCtl`]'s own
//! contract forbids merging them: `FocusCtl::nav()`'s Next/Prev walks
//! EVERY entry in `prev` regardless of its capabilities — that is what
//! makes a plain, caps-less registration a legal Tab stop today. A
//! structural node (a panel group, a window frame) must never become
//! one. So passive/structural accessibles get their OWN per-frame
//! registry, [`AccessCtl`], deliberately smaller than `FocusCtl`: no
//! `Caps`, no `group`, no navigation, no focused control, no ring — just
//! "this rect is this role with this name, read it if you're a bridge."
//! A FOCUSABLE control's [`AccessInfo`] does not live here at all; it
//! travels through `FocusCtl::register`'s own `access` argument instead,
//! stored beside the entry the Tab chain already owns.
//!
//! Frame contract: identical to `FocusCtl`'s. Controls register WHILE
//! DRAWING; [`AccessCtl::begin_frame`] swaps the just-drawn frame into
//! the one a bridge reads, once per frame, at the frame boundary.

use crate::focus::FocusId;
use crate::Rect;

// ---------------------------------------------------------------------
// Role
// ---------------------------------------------------------------------

/// What kind of control or container a node is — the closed set every
/// widget in this crate can be today. New widgets add a variant here
/// rather than reusing a near-enough one: a bridge maps each variant to
/// its backend's own role constant, and a wrong mapping reads worse to
/// a screen reader than a missing one.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Button,
    CheckBox,
    RadioButton,
    Slider,
    TextInput,
    ComboBox,
    ListItem,
    Tab,
    MenuItem,
    Menu,
    Group,
    Window,
    Dialog,
    ToolTip,
    Alert,
    Status,
}

// ---------------------------------------------------------------------
// States — a hand-rolled bitset, same shape as focus::Caps
// ---------------------------------------------------------------------

/// A node's accessible state, as a `u16` bitset (hand-rolled — no new
/// dependency, exactly [`crate::focus::Caps`]'s reasoning). Wider than
/// `Caps` because a screen reader distinguishes more states than a
/// keyboard router needs to.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct States(u16);

impl States {
    pub const NONE: States = States(0);
    pub const DISABLED: States = States(1 << 0);
    pub const CHECKED: States = States(1 << 1);
    pub const SELECTED: States = States(1 << 2);
    pub const EXPANDED: States = States(1 << 3);
    pub const READ_ONLY: States = States(1 << 4);
    pub const REQUIRED: States = States(1 << 5);
    pub const INVALID: States = States(1 << 6);
    pub const BUSY: States = States(1 << 7);

    pub const fn contains(self, other: States) -> bool {
        self.0 & other.0 == other.0
    }
    pub const fn bits(self) -> u16 {
        self.0
    }
    pub const fn from_bits(bits: u16) -> States {
        States(bits & 0b1111_1111)
    }
}

impl std::ops::BitOr for States {
    type Output = States;
    fn bitor(self, rhs: States) -> States {
        States(self.0 | rhs.0)
    }
}

// ---------------------------------------------------------------------
// Live region politeness
// ---------------------------------------------------------------------

/// How urgently a live-region change should interrupt — the toaster's
/// and the alert's vocabulary, mirrored from AT-SPI's own two-level
/// scheme without depending on it.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Live {
    Polite,
    Assertive,
}

// ---------------------------------------------------------------------
// AccessInfo
// ---------------------------------------------------------------------

/// One node's accessible report: what a bridge reads for a single
/// control or container. `index` is `(position, size)` within a set —
/// a tab's `(2, 5)` among five, a list row's place among its siblings —
/// left `None` when the caller has not sorted that out yet.
#[derive(Clone, Debug)]
pub struct AccessInfo {
    pub role: Role,
    pub name: String,
    pub states: States,
    pub value: Option<String>,
    pub index: Option<(u32, u32)>,
}

impl AccessInfo {
    pub fn new(role: Role, name: impl Into<String>) -> AccessInfo {
        AccessInfo { role, name: name.into(), states: States::NONE, value: None, index: None }
    }

    pub fn with_states(mut self, states: States) -> Self {
        self.states = states;
        self
    }

    pub fn with_value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    pub fn with_index(mut self, pos: u32, size: u32) -> Self {
        self.index = Some((pos, size));
        self
    }
}

// ---------------------------------------------------------------------
// AccessCtl — the passive/structural registry
// ---------------------------------------------------------------------

struct Entry {
    id: FocusId,
    rect: Rect,
    info: AccessInfo,
}

/// The per-world registry of PASSIVE/STRUCTURAL accessible nodes —
/// panel groups, window frames, anything a bridge should describe but
/// that must never answer a Tab press. Structurally parallel to
/// [`crate::focus::FocusCtl`] (double-buffered, register-while-drawing,
/// swapped at the frame boundary) but with everything `FocusCtl` needs
/// for KEYBOARD navigation removed, because removing it is what keeps
/// this registry safe to register a node into: there is no `caps`, no
/// `group`, no navigation and no focused id here to accidentally make a
/// structural node reachable by Tab.
#[derive(Default)]
pub struct AccessCtl {
    /// This frame's nodes, in registration order.
    cur: Vec<Entry>,
    /// The last completed frame — what a bridge reads.
    prev: Vec<Entry>,
}

impl AccessCtl {
    pub fn new() -> AccessCtl {
        AccessCtl::default()
    }

    /// The frame boundary: the nodes registered this frame become what
    /// [`AccessCtl::entries`] answers, and the next frame starts empty.
    /// Same swap-and-clear as [`crate::focus::FocusCtl::begin_frame`].
    pub fn begin_frame(&mut self) {
        std::mem::swap(&mut self.prev, &mut self.cur);
        self.cur.clear();
    }

    /// Called by a container WHILE DRAWING: registers into this frame's
    /// list. There is nothing to answer back — unlike `FocusCtl`, a
    /// structural node has no focus state a caller could be waiting on.
    pub fn register(&mut self, id: FocusId, r: Rect, info: AccessInfo) {
        self.cur.push(Entry { id, rect: r, info });
    }

    /// The last completed frame's nodes — what a future AT-SPI bridge
    /// reads. Not built here: this crate only provides the read.
    pub fn entries(&self) -> impl Iterator<Item = (FocusId, Rect, &AccessInfo)> {
        self.prev.iter().map(|e| (e.id, e.rect, &e.info))
    }
}

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn r(x: f32, y: f32) -> Rect {
        Rect::new(x, y, 10.0, 10.0)
    }

    #[test]
    fn register_before_the_first_begin_frame_is_not_yet_visible() {
        let mut ac = AccessCtl::new();
        let a = FocusId::of("a");
        ac.register(a, r(0.0, 0.0), AccessInfo::new(Role::Group, "panel"));
        assert_eq!(ac.entries().count(), 0);
        ac.begin_frame();
        let got: Vec<_> = ac.entries().collect();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, a);
        assert_eq!(got[0].2.name, "panel");
    }

    #[test]
    fn begin_frame_swaps_and_clears() {
        let mut ac = AccessCtl::new();
        let a = FocusId::of("a");
        let b = FocusId::of("b");
        ac.register(a, r(0.0, 0.0), AccessInfo::new(Role::Group, "one"));
        ac.begin_frame();
        assert_eq!(ac.entries().count(), 1);
        // Frame 2: only b registers.
        ac.register(b, r(1.0, 1.0), AccessInfo::new(Role::Window, "two"));
        ac.begin_frame();
        let got: Vec<_> = ac.entries().collect();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, b);
        assert_eq!(got[0].2.name, "two");
    }

    #[test]
    fn states_bitor_and_contains() {
        let s = States::DISABLED | States::REQUIRED;
        assert!(s.contains(States::DISABLED));
        assert!(s.contains(States::REQUIRED));
        assert!(!s.contains(States::CHECKED));
        assert!(States::NONE.bits() == 0);
        assert_eq!(States::from_bits(s.bits()), s);
    }

    #[test]
    fn access_info_new_defaults() {
        let info = AccessInfo::new(Role::Button, "OK");
        assert_eq!(info.role, Role::Button);
        assert_eq!(info.name, "OK");
        assert_eq!(info.states, States::NONE);
        assert_eq!(info.value, None);
        assert_eq!(info.index, None);
    }

    #[test]
    fn builders_set_only_their_own_field() {
        let info = AccessInfo::new(Role::Slider, "Volume").with_states(States::CHECKED);
        assert_eq!(info.states, States::CHECKED);
        assert_eq!(info.value, None);
        assert_eq!(info.index, None);

        let info = AccessInfo::new(Role::Slider, "Volume").with_value("50%");
        assert_eq!(info.value.as_deref(), Some("50%"));
        assert_eq!(info.states, States::NONE);
        assert_eq!(info.index, None);

        let info = AccessInfo::new(Role::Tab, "General").with_index(1, 4);
        assert_eq!(info.index, Some((1, 4)));
        assert_eq!(info.states, States::NONE);
        assert_eq!(info.value, None);
    }
}
