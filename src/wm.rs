//! The window-management vocabulary: WHAT can be asked of a window, kept
//! apart from WHO carries it out.
//!
//! JEDEN MODEL OKNA — "window behaviours live in libnacelle and act
//! globally, not in nacelle-desktop." This module is that seam, and it
//! is the same discipline [`crate::clipboard`] already runs on: the
//! toolkit owns the vocabulary and the trait, the application supplies
//! the backend that actually speaks to a compositor.
//!
//! # The seam
//!
//! One vocabulary — [`Verb`] for what can be asked, [`Act`] for asking,
//! [`Window`] for what came back — and one trait, [`Backend`], for the
//! thing that actually speaks to a compositor. [`Connector`] is the
//! frame-by-frame discipline held around whichever backend the host
//! picked.
//!
//! What does NOT live here is picking a backend. `wayland-client`,
//! `x11rb`, `smithay-client-toolkit` and winit's own window handles are
//! platform concerns, and this crate stays platform-independent (see
//! the crate's own header). So the carriers themselves — today, an
//! `ext-foreign-toplevel-list-v1` reader and an EWMH-over-XWayland
//! reader — live in the application (`nacelle-desktop/src/fullscreen/`:
//! `wayland.rs`, `x11.rs`, and `host.rs` for reading which window is the
//! application's own out of its window library). They implement
//! [`Backend`] and hand the result to [`Connector::over`]; deciding
//! WHICH carrier to try, in what order, and with which fallback, is
//! `fullscreen::connect` in that same crate, because that decision is
//! made of concrete carrier types this crate cannot name. A third seat
//! is left for the compositor of our own, which will need no protocol
//! at all — it will hold the window list itself and implement
//! [`Backend`] against its own state.
//!
//! A window built into the desktop and a window of an outside
//! application are meant to answer to the same [`Verb`]s. Nothing here
//! is only-for-strangers: the vocabulary carries no assumption that the
//! window is somebody else's, and the day nacelle's own compositor
//! lists its own toplevels through this same trait, an own window and a
//! foreign one are one more line in one [`Backend::windows`] snapshot.
//!
//! # Why a snapshot and an epoch, not callbacks
//!
//! The desktop draws every frame from state it owns. A backend that
//! called back into the interface would need the interface to be
//! reachable from a Wayland dispatch, and the ordering between "a
//! window appeared" and "the frame is being laid out" would be nobody's
//! to state. So: [`Connector::poll`] once a frame drains the carrier,
//! [`Connector::windows`] hands back a snapshot, and
//! [`Connector::epoch`] only moves when something actually changed.
//!
//! That last part is not decoration. The same shape, gotten wrong,
//! is what pinned a CPU at 100 %: `theme::epoch()` answered "which bake
//! is published", which alternates every frame with two screens of
//! different heights, and the font system re-read every font on disk
//! sixty times a second. An epoch that ticks when nothing happened is
//! not a harmless epoch.

use std::collections::HashMap;

/// A window's identity for as long as it is mapped.
///
/// Minted by [`Names`], never the server's own number. Both carriers
/// have a native key — an X11 window id, a Wayland object id — and both
/// reuse them: X11 hands the same id to a new client once the old one
/// is gone, and the Wayland object id is a slot in a table. A number
/// the interface is holding on to must not quietly start meaning a
/// different window, so the native key is kept private to the carrier
/// and this is what travels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct WindowId(pub u64);

/// Where a window is, in the carrier's own coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Place {
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
}

/// The four states a window can be told to be in and asked about.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct State {
    pub active: bool,
    pub minimized: bool,
    pub maximized: bool,
    pub fullscreen: bool,
}

/// What a window looks like in a list, as far as a carrier can say.
///
/// Two shapes because the two carriers this crate ships against answer
/// differently and neither answer is a paraphrase of the other: EWMH
/// puts the pixels on the window itself (`_NET_WM_ICON`), the Wayland
/// protocol gives an app id and expects the icon theme to be searched.
/// Flattening one into the other would mean inventing pixels or
/// inventing a name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Icon {
    /// A name to look up in the icon theme.
    Named(String),
    /// Pre-multiplied ARGB, row-major, `w * h` long.
    Pixels { w: u32, h: u32, argb: Vec<u32> },
}

/// One thing the desktop knows how to want from a window.
///
/// A closed list, and every carrier answers [`Backend::can`] for each
/// one. That is the same discipline the COLOR page runs on: a control
/// the carrier cannot honour is not drawn enabled, because a button
/// that does nothing is worse than a button that is not there.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Verb {
    /// There is a list of windows at all.
    List,
    Title,
    App,
    Icon,
    Focus,
    Close,
    Minimize,
    Maximize,
    Fullscreen,
    /// Move and resize; one verb, because every carrier that can do
    /// either can do both.
    Place,
    /// Which board (virtual desktop) the window sits on.
    Board,
}

impl Verb {
    /// Every verb, once. Tests walk this so a verb added to the
    /// vocabulary cannot be forgotten by a carrier in silence.
    pub const ALL: [Verb; 11] = [
        Verb::List,
        Verb::Title,
        Verb::App,
        Verb::Icon,
        Verb::Focus,
        Verb::Close,
        Verb::Minimize,
        Verb::Maximize,
        Verb::Fullscreen,
        Verb::Place,
        Verb::Board,
    ];

    /// The name to write in a log line or under a greyed-out control.
    pub fn label(self) -> &'static str {
        match self {
            Verb::List => "list",
            Verb::Title => "title",
            Verb::App => "app id",
            Verb::Icon => "icon",
            Verb::Focus => "focus",
            Verb::Close => "close",
            Verb::Minimize => "minimize",
            Verb::Maximize => "maximize",
            Verb::Fullscreen => "fullscreen",
            Verb::Place => "move and resize",
            Verb::Board => "board",
        }
    }
}

/// An order, addressed to one window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Act {
    Focus(WindowId),
    Close(WindowId),
    Minimize(WindowId, bool),
    Maximize(WindowId, bool),
    Fullscreen(WindowId, bool),
    Place(WindowId, Place),
    SendToBoard(WindowId, u32),
}

impl Act {
    pub fn verb(self) -> Verb {
        match self {
            Act::Focus(..) => Verb::Focus,
            Act::Close(..) => Verb::Close,
            Act::Minimize(..) => Verb::Minimize,
            Act::Maximize(..) => Verb::Maximize,
            Act::Fullscreen(..) => Verb::Fullscreen,
            Act::Place(..) => Verb::Place,
            Act::SendToBoard(..) => Verb::Board,
        }
    }

    pub fn who(self) -> WindowId {
        match self {
            Act::Focus(id)
            | Act::Close(id)
            | Act::Minimize(id, _)
            | Act::Maximize(id, _)
            | Act::Fullscreen(id, _)
            | Act::Place(id, _)
            | Act::SendToBoard(id, _) => id,
        }
    }

    /// A specimen of every verb, for tests that must walk the whole
    /// vocabulary against a carrier.
    pub fn specimen(verb: Verb, id: WindowId) -> Option<Act> {
        Some(match verb {
            Verb::List | Verb::Title | Verb::App | Verb::Icon => return None,
            Verb::Focus => Act::Focus(id),
            Verb::Close => Act::Close(id),
            Verb::Minimize => Act::Minimize(id, true),
            Verb::Maximize => Act::Maximize(id, true),
            Verb::Fullscreen => Act::Fullscreen(id, true),
            Verb::Place => Act::Place(id, Place { x: 0, y: 0, w: 640, h: 480 }),
            Verb::Board => Act::SendToBoard(id, 0),
        })
    }
}

/// What came of an order.
///
/// Four answers and not two. "The carrier does not do this at all" and
/// "the carrier tried and it did not work" are different sentences to
/// write under a control, and "I have never heard of that window" is a
/// third — it is what a stale identity earns, and it must not read as a
/// failure of the compositor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Sent. Wayland and X11 are both asynchronous and neither answers
    /// an order, so this says the request left, not that it was obeyed.
    Sent,
    Unsupported,
    Unknown(WindowId),
    Failed(String),
}

/// One window, as the interface sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Window {
    pub id: WindowId,
    pub title: String,
    pub app: String,
    /// None where the carrier cannot say — not "board zero".
    pub board: Option<u32>,
    pub state: State,
    /// Where the window is. Kept current in [`Backend::windows`], and
    /// deliberately **not** part of what moves the epoch — see
    /// [`reads_differently`]. Anything laid out from these numbers has
    /// to read them every frame; it cannot memoise on
    /// [`Connector::epoch`].
    pub place: Option<Place>,
}

impl Window {
    pub fn new(id: WindowId) -> Window {
        Window {
            id,
            title: String::new(),
            app: String::new(),
            board: None,
            state: State::default(),
            place: None,
        }
    }

    /// Whether two readings of one window say the same thing —
    /// geometry excluded. See [`reads_differently`] for why.
    pub fn reads_same(&self, other: &Window) -> bool {
        // Destructured, not compared field by field, on purpose: a
        // field added to `Window` stops this compiling until somebody
        // has decided which side of the line it belongs on. A silent
        // `&&` chain would simply stop noticing it.
        let Window { id, title, app, board, state, place: _ } = self;
        let Window { id: id2, title: title2, app: app2, board: board2, state: state2, place: _ } =
            other;
        id == id2 && title == title2 && app == app2 && board == board2 && state == state2
    }
}

/// Whether a list just read says anything the last one did not.
///
/// **Geometry is not news, and that is the whole point of this
/// function.** A window being dragged sends a `ConfigureNotify` per
/// frame, so its rectangle really is different sixty times a second —
/// and comparing whole [`Window`] values, geometry included, would move
/// [`Connector::epoch`] on every one of those frames. Everything
/// memoising on the epoch would then rebuild sixty times a second while
/// somebody drags a window, which is the exact shape that pinned this
/// program's CPU at 100 % once already: `theme::epoch()` answered
/// "which bake is published", it alternated every frame on two screens
/// of unequal height, and the font system re-read every font on disk
/// for it (`.gap-program/usterka-cpu-desktop.md`; 100,7 % → 10,6 %).
///
/// What is lost by the exclusion is stated where it can be seen:
/// [`Window::place`] is still current in [`Backend::windows`], it just
/// does not announce itself. A reader that draws from geometry reads it
/// every frame — which it would have to do anyway, since a rectangle
/// that moves every frame cannot be memoised on anything.
///
/// One rule, shared by every carrier, so "what counts as news" is
/// answered in one place rather than once per protocol.
pub fn reads_differently(now: &[Window], before: &[Window]) -> bool {
    now.len() != before.len() || now.iter().zip(before).any(|(a, b)| !a.reads_same(b))
}

/// Whoever actually talks to a compositor.
///
/// Implemented by the application — `wayland::Toplevels` and
/// `x11::Ewmh` in nacelle-desktop today, the compositor's own state
/// once nacelle is the compositor — and handed to [`Connector::over`],
/// the same pattern [`crate::clipboard::ClipboardBackend`] runs on.
pub trait Backend {
    /// For a log line and for the settings page.
    fn carrier(&self) -> &'static str;

    /// Whether this carrier does this verb at all. Asked BEFORE a
    /// control is drawn.
    fn can(&self, verb: Verb) -> bool;

    /// What this carrier cannot see, in words fit to print under an
    /// empty list. An empty list with no explanation reads as "no
    /// windows are open", which is a lie an EWMH carrier tells on a
    /// Wayland session every time.
    fn blind_spot(&self) -> Option<&'static str>;

    /// Drain whatever the compositor has said. True when the list came
    /// out different.
    fn poll(&mut self) -> bool;

    fn windows(&self) -> &[Window];

    /// Fetched on demand, not on every poll: EWMH icons are megabytes
    /// of pixels sitting on a window property.
    ///
    /// `want` is the size the caller is about to draw at, in pixels,
    /// and it decides which of the sizes an application shipped comes
    /// back. It is an argument and not a constant here because it is an
    /// appearance value: it comes from the row height in the theme, and
    /// a number picked in Rust is a number no theme can change.
    fn icon(&mut self, id: WindowId, want: u32) -> Option<Icon>;

    fn act(&mut self, act: Act) -> Outcome;
}

/// The identity mint, shared by every carrier so there is one rule.
#[derive(Default)]
pub struct Names {
    next: u64,
    by_native: HashMap<u64, WindowId>,
}

impl Names {
    pub fn new() -> Names {
        Names { next: 1, by_native: HashMap::new() }
    }

    /// The id for a native key, minting one the first time.
    pub fn of(&mut self, native: u64) -> WindowId {
        if let Some(&id) = self.by_native.get(&native) {
            return id;
        }
        if self.next == 0 {
            self.next = 1;
        }
        let id = WindowId(self.next);
        self.next += 1;
        self.by_native.insert(native, id);
        id
    }

    /// The key is dead. The next window to be handed the same native
    /// key gets a NEW identity.
    pub fn forget(&mut self, native: u64) {
        self.by_native.remove(&native);
    }

    /// Everything not in `alive` is dead. The X11 carrier learns of
    /// departures by a list arriving without them, never by an event.
    pub fn retain(&mut self, alive: &[u64]) {
        self.by_native.retain(|k, _| alive.contains(k));
    }

    pub fn native(&self, id: WindowId) -> Option<u64> {
        self.by_native.iter().find(|(_, &v)| v == id).map(|(&k, _)| k)
    }
}

/// The connector the desktop holds: one carrier, and the frame-by-frame
/// discipline around it.
///
/// Built with [`Connector::over`] a concrete [`Backend`] the
/// application chose. Picking WHICH backend to try, and in what order,
/// is not this crate's business — it is made of carrier types this
/// crate cannot name — so it lives with the carriers themselves
/// (`fullscreen::connect` in nacelle-desktop).
pub struct Connector {
    back: Box<dyn Backend>,
    epoch: u64,
}

impl Connector {
    pub fn over(back: Box<dyn Backend>) -> Connector {
        Connector { back, epoch: 0 }
    }

    /// Once a frame. The epoch moves only when the list did.
    pub fn poll(&mut self) {
        if self.back.poll() {
            self.epoch = self.epoch.wrapping_add(1);
        }
    }

    /// What to compare against last frame's. Never a clock, never a
    /// counter of polls — and never a window merely moving, which
    /// happens every frame of every drag ([`reads_differently`]).
    pub fn epoch(&self) -> u64 {
        self.epoch
    }

    pub fn windows(&self) -> &[Window] {
        self.back.windows()
    }

    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.back.windows().iter().find(|w| w.id == id)
    }

    pub fn can(&self, verb: Verb) -> bool {
        self.back.can(verb)
    }

    /// `want` is the size it is about to be drawn at, which the theme
    /// answers and this seam does not.
    pub fn icon(&mut self, id: WindowId, want: u32) -> Option<Icon> {
        self.back.icon(id, want)
    }

    pub fn act(&mut self, act: Act) -> Outcome {
        self.back.act(act)
    }

    pub fn carrier(&self) -> &'static str {
        self.back.carrier()
    }

    pub fn blind_spot(&self) -> Option<&'static str> {
        self.back.blind_spot()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A carrier that says yes to some verbs and no to others, and
    /// counts how often it was asked to look.
    struct Toy {
        list: Vec<Window>,
        yes: Vec<Verb>,
        news: Vec<bool>,
        polls: usize,
    }

    impl Toy {
        fn new(yes: &[Verb]) -> Toy {
            Toy { list: Vec::new(), yes: yes.to_vec(), news: Vec::new(), polls: 0 }
        }
    }

    impl Backend for Toy {
        fn carrier(&self) -> &'static str {
            "toy"
        }
        fn can(&self, verb: Verb) -> bool {
            self.yes.contains(&verb)
        }
        fn blind_spot(&self) -> Option<&'static str> {
            None
        }
        fn poll(&mut self) -> bool {
            self.polls += 1;
            if self.news.is_empty() {
                false
            } else {
                self.news.remove(0)
            }
        }
        fn windows(&self) -> &[Window] {
            &self.list
        }
        fn icon(&mut self, _: WindowId, _: u32) -> Option<Icon> {
            None
        }
        fn act(&mut self, act: Act) -> Outcome {
            if !self.can(act.verb()) {
                return Outcome::Unsupported;
            }
            Outcome::Sent
        }
    }

    /// A carrier shaped like the real ones: it re-reads a whole list
    /// every poll and answers news through [`reads_differently`],
    /// which is what both the wayland and x11 carriers in
    /// nacelle-desktop do.
    ///
    /// [`Toy`] cannot stand in for this — it is handed the answer — so
    /// nothing riding on it can say whether a list that came back with
    /// one rectangle moved counts as a change.
    struct Carrier {
        script: Vec<Vec<Window>>,
        snapshot: Vec<Window>,
    }

    impl Backend for Carrier {
        fn carrier(&self) -> &'static str {
            "carrier"
        }
        fn can(&self, _: Verb) -> bool {
            true
        }
        fn blind_spot(&self) -> Option<&'static str> {
            None
        }
        fn poll(&mut self) -> bool {
            if self.script.is_empty() {
                return false;
            }
            let before = std::mem::replace(&mut self.snapshot, self.script.remove(0));
            reads_differently(&self.snapshot, &before)
        }
        fn windows(&self) -> &[Window] {
            &self.snapshot
        }
        fn icon(&mut self, _: WindowId, _: u32) -> Option<Icon> {
            None
        }
        fn act(&mut self, _: Act) -> Outcome {
            Outcome::Sent
        }
    }

    fn window(title: &str, at: i32) -> Window {
        let mut w = Window::new(WindowId(1));
        w.title = title.to_string();
        w.app = "org.kde.dolphin".to_string();
        w.place = Some(Place { x: at, y: at, w: 800, h: 600 });
        w
    }

    /// **An epoch that moves when nothing happened is the bug that
    /// pinned a CPU at 100 %.**
    ///
    /// `theme::epoch()` answered "which bake is published", which
    /// alternates every frame on two screens of unequal height, and the
    /// font system took that for news and re-read every font on disk
    /// sixty times a second (`.gap-program/usterka-cpu-desktop.md`;
    /// measured 100,7 % → 10,6 %). Whatever the window list is fed
    /// into will memoise on this number exactly the same way, so the
    /// number has to be silent on a quiet frame — polling is not news.
    ///
    /// The assertion is on a hundred polls and not one, because a
    /// single quiet poll can be got right by an epoch that ticks every
    /// other time.
    #[test]
    fn a_quiet_frame_does_not_move_the_epoch() {
        let mut c = Connector::over(Box::new(Toy::new(&[])));
        let start = c.epoch();
        for _ in 0..100 {
            c.poll();
        }
        assert_eq!(
            c.epoch(),
            start,
            "the epoch moved on frames where the carrier reported no change — \
             every reader memoising on it will rebuild sixty times a second"
        );

        let mut noisy = Toy::new(&[]);
        noisy.news = vec![false, true, false];
        let mut c = Connector::over(Box::new(noisy));
        c.poll();
        assert_eq!(c.epoch(), 0, "silence counted as news");
        c.poll();
        assert_eq!(c.epoch(), 1, "news did not count");
        c.poll();
        assert_eq!(c.epoch(), 1, "silence after news counted as news");
    }

    /// **Dragging a window must not move the epoch, for as long as the
    /// drag lasts.**
    ///
    /// This is the example the epoch's own comment gives itself, and
    /// until this test existed nothing checked it: the carriers
    /// compared whole [`Window`] values, [`Window::place`] included, so
    /// a window under the pointer — a fresh rectangle per frame, per
    /// `ConfigureNotify` — moved the epoch on every single frame. Every
    /// reader memoising on that number would have rebuilt sixty times a
    /// second for as long as somebody held the mouse down.
    ///
    /// A hundred frames and not one, because an epoch that ticks every
    /// other frame passes a one-frame test.
    #[test]
    fn a_hundred_frames_of_dragging_do_not_move_the_epoch() {
        let script: Vec<Vec<Window>> = (0..100).map(|i| vec![window("Files", i)]).collect();
        let start = vec![window("Files", 0)];
        let mut c = Connector::over(Box::new(Carrier { script, snapshot: start }));
        for _ in 0..100 {
            c.poll();
        }
        assert_eq!(
            c.epoch(),
            0,
            "a hundred frames of a window being dragged moved the epoch — \
             everything memoising on it rebuilds for as long as the drag lasts"
        );
        assert_eq!(
            c.windows()[0].place,
            Some(Place { x: 99, y: 99, w: 800, h: 600 }),
            "the rectangle handed to the interface went stale — it is silent, \
             not out of date"
        );

        // And the same carrier, on a change that IS news, still says so.
        let mut c = Connector::over(Box::new(Carrier {
            script: vec![vec![window("Downloads", 99)]],
            snapshot: vec![window("Files", 99)],
        }));
        c.poll();
        assert_eq!(c.epoch(), 1, "a rename was not news");
    }

    /// **What counts as news is everything about a window except where
    /// it is.**
    ///
    /// The line has to be drawn somewhere, and drawn wrong in the other
    /// direction it is worse than the bug it fixes: a window that
    /// closed, was renamed, was minimized or moved to another board and
    /// did not move the epoch is a list that stays wrong until
    /// something else happens to disturb it.
    #[test]
    fn what_counts_as_news_is_everything_but_the_rectangle() {
        let here = window("Files", 0);
        let there = window("Files", 40);
        assert!(
            !reads_differently(std::slice::from_ref(&there), std::slice::from_ref(&here)),
            "a window that only moved was called news"
        );

        let mut renamed = here.clone();
        renamed.title = "Downloads".into();
        let mut reapped = here.clone();
        reapped.app = "org.gnome.Nautilus".into();
        let mut boarded = here.clone();
        boarded.board = Some(3);
        let mut hidden = here.clone();
        hidden.state.minimized = true;
        let mut lit = here.clone();
        lit.state.active = true;
        let mut reborn = here.clone();
        reborn.id = WindowId(2);
        for (what, w) in [
            ("a rename", renamed),
            ("an app id changing", reapped),
            ("a window moving to another board", boarded),
            ("a window being minimized", hidden),
            ("focus moving", lit),
            ("a new identity on the same row", reborn),
        ] {
            assert!(
                reads_differently(std::slice::from_ref(&w), std::slice::from_ref(&here)),
                "{what} was not news — the list stays wrong until something \
                 else disturbs it"
            );
        }

        assert!(
            reads_differently(&[here.clone(), here.clone()], std::slice::from_ref(&here)),
            "a window opening was not news"
        );
        assert!(reads_differently(&[], std::slice::from_ref(&here)), "a window closing was not news");
        assert!(!reads_differently(&[], &[]), "two empty lists differed");
    }

    /// **An order for a verb the toy says no to comes back
    /// `Unsupported`, and [`Act::verb`] is what decides which verb an
    /// order is.**
    ///
    /// Said narrowly on purpose. The carrier here agrees with itself by
    /// construction — [`Toy::act`] asks its own `can` — so this cannot
    /// and does not hold two separately written tables together. What
    /// it does hold is the map from an order to its verb: route
    /// [`Act::Focus`] to [`Verb::Board`] and this test goes red, which
    /// is worth having, because that map is what decides whether a
    /// click is allowed at all.
    ///
    /// The real agreement between "what is offered" and "what can be
    /// done" is checked against the real tables, per carrier, in
    /// nacelle-desktop's `fullscreen::x11::tests`.
    #[test]
    fn a_carrier_that_says_no_does_nothing_and_says_so() {
        let allowed = [Verb::Focus, Verb::Close];
        let mut toy = Toy::new(&allowed);
        let id = WindowId(1);
        for verb in Verb::ALL {
            let Some(act) = Act::specimen(verb, id) else { continue };
            let answer = toy.act(act);
            if allowed.contains(&verb) {
                assert_ne!(
                    answer,
                    Outcome::Unsupported,
                    "the carrier offers '{}' and then refuses to do it — \
                     the control would be drawn live and do nothing",
                    verb.label()
                );
            } else {
                assert_eq!(
                    answer,
                    Outcome::Unsupported,
                    "the carrier does not offer '{}' and did it anyway",
                    verb.label()
                );
            }
        }
    }

    /// **Every verb has a specimen, or is a reading verb.**
    ///
    /// [`Act::specimen`] is what the walk above rides on. A verb added
    /// to [`Verb::ALL`] with no specimen and no place among the reading
    /// four would be skipped by every carrier's agreement test in
    /// silence — the vocabulary would grow a word nothing checks.
    #[test]
    fn every_verb_is_either_read_or_has_an_order() {
        let reading = [Verb::List, Verb::Title, Verb::App, Verb::Icon];
        for verb in Verb::ALL {
            let has = Act::specimen(verb, WindowId(1)).is_some();
            assert_eq!(
                has,
                !reading.contains(&verb),
                "'{}' is neither a reading verb nor an order that can be \
                 built — no carrier's agreement test will ever look at it",
                verb.label()
            );
            if let Some(act) = Act::specimen(verb, WindowId(1)) {
                assert_eq!(act.verb(), verb, "the specimen of '{}' is a different verb", verb.label());
                assert_eq!(act.who(), WindowId(1), "the specimen lost its window");
            }
        }
    }

    /// **A native key that dies and comes back is a different window.**
    ///
    /// X11 hands window ids out again once a client is gone, and the
    /// Wayland object id is a slot in a table that is reused the moment
    /// it is freed. If the mint were the native key, an interface still
    /// holding the id of a window that closed would find itself
    /// addressing whatever took its place — closing, moving or
    /// fullscreening a stranger. That is why the native key never
    /// leaves this file.
    #[test]
    fn a_reused_native_key_is_never_the_same_window() {
        let mut names = Names::new();
        let first = names.of(0x0120_0007);
        assert_eq!(names.of(0x0120_0007), first, "the same live window changed identity");

        names.forget(0x0120_0007);
        let second = names.of(0x0120_0007);
        assert_ne!(
            first, second,
            "the window id the X server reused brought the old identity back \
             with it — an order meant for a window that closed would land on \
             whatever opened next"
        );
        assert_eq!(names.native(second), Some(0x0120_0007));
        assert_eq!(names.native(first), None, "a dead identity still resolves to a live window");
    }

    /// **A list arriving without a window is that window dying.**
    ///
    /// EWMH never says "closed"; `_NET_CLIENT_LIST` simply comes back
    /// shorter. [`Names::retain`] is the only place that turns absence
    /// into death, so a carrier that forgot to call it would keep
    /// handing out an identity for a window that is gone — and, worse,
    /// would hand the SAME one out again when the id is reused.
    #[test]
    fn a_window_missing_from_the_list_loses_its_identity() {
        let mut names = Names::new();
        let a = names.of(10);
        let b = names.of(20);
        names.retain(&[20]);
        assert_eq!(names.native(a), None, "a window that left the list kept its identity");
        assert_eq!(names.native(b), Some(20), "a window still in the list lost its identity");
        assert_ne!(names.of(10), a, "the identity came back with the id");
    }
}
