//! `motion.window_open` and `motion.window_close` — the pair §5.22 has
//! carried since it was written with nothing reading either half.
//!
//! They are two entries rather than one on purpose, and the master says
//! why at the keys: the exit is 140 ms of `ease_in` against the entry's
//! 180 ms of `ease_out`, "so the exit accelerates away". A gate that took
//! one effect could not have expressed that, which is why
//! `motion::gate_dir` had to exist before `winframe::present` could.
//!
//! What is measured here is the SEAM: the two durations, the two curves,
//! the rigid transform on the drawn box, the exact ends, the birth rule a
//! window needs and a control must not have, and the promise that the
//! rectangle handed in for hit-testing is never the one that moves.
//!
//! Time is a parameter; every clock is a literal.
//!
//! One test in a binary of its own: the resolved theme is process-wide.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::object::winframe::{self, Present};
use nacelle::pointer::Pointer;
use nacelle::{motion, theme, Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const BOX: Rect = Rect { x: 300.0, y: 200.0, w: 800.0, h: 500.0 };

fn skin(body: &str) {
    let path = std::env::temp_dir().join(format!("nacelle-present-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        format!("[meta]\nschema = 1\nname = \"Fixture\"\nbase = \"default\"\n\n{body}"),
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(theme::LoadRequest { path: Some(path.clone()), ..Default::default() });
    let _ = std::fs::remove_file(&path);
    theme::set_viewport(H, 1.0);
    motion::forget_fades();
}

fn master() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
    motion::forget_fades();
}

/// The window asked about at `now`, the way a host asks: every frame,
/// with `open` false as readily as true.
fn at(fonts: &mut FontSystem, open: bool, now: f64) -> Present {
    let mut dl = DrawList::new();
    let mut ctx = Ctx {
        access: None,
        dl: &mut dl,
        fonts,
        w: W,
        h: H,
        t: now,
        mouse: Pointer::new(0.0, 0.0),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    };
    winframe::present(&mut ctx, BOX, open)
}

/// One frame of a 60 Hz host.
const FRAME: f64 = 1.0 / 60.0;

/// A host's frame loop, wound by hand.
///
/// Every phase below drives CONTIGUOUS frames rather than jumping to the
/// interesting instant, and it has to: the registry reclaims a key it has
/// not been asked about for half a second — that is what "the window is
/// gone" looks like from inside — so a test that skipped from the entry
/// to the exit would be measuring a second, freshly born window.
struct Host<'a> {
    fonts: &'a mut FontSystem,
    t: f64,
    open: bool,
}

impl<'a> Host<'a> {
    /// A world with nothing in it, and the window's FIRST frame.
    fn opening(fonts: &'a mut FontSystem, t0: f64) -> Host<'a> {
        motion::forget_fades();
        let mut h = Host { fonts, t: t0, open: true };
        let _ = h.now();
        h
    }

    fn now(&mut self) -> Present {
        at(self.fonts, self.open, self.t)
    }

    /// Draws every frame up to `t` and answers the one at `t`.
    fn to(&mut self, t: f64) -> Present {
        while self.t + FRAME < t {
            self.t += FRAME;
            let _ = self.now();
        }
        self.t = t;
        self.now()
    }

    /// Asks for the other direction from the NEXT frame on, and answers
    /// when that frame was.
    ///
    /// The next frame, not this one: the registry treats two asks at one
    /// clock as one frame asked twice and jumps rather than fading, which
    /// is what keeps a repeated draw bit for bit what it was. So a
    /// direction change takes effect on the following frame, and that is
    /// the instant every duration below is measured from.
    fn set(&mut self, open: bool) -> f64 {
        self.open = open;
        self.t += FRAME;
        let _ = self.now();
        self.t
    }
}

/// The drawn box's scale, as a fraction of the settled one.
fn scale(p: &Present) -> f32 {
    p.rect.w / BOX.w
}

/// The centre does not move: the transform is rigid about it, so a window
/// grows into place rather than sliding into it.
fn centred(p: &Present) {
    assert!((p.rect.x + p.rect.w * 0.5 - (BOX.x + BOX.w * 0.5)).abs() < 1e-3, "the centre moved");
    assert!((p.rect.y + p.rect.h * 0.5 - (BOX.y + BOX.h * 0.5)).abs() < 1e-3, "the centre moved");
    assert!((p.rect.w / BOX.w - p.rect.h / BOX.h).abs() < 1e-4, "the box changed proportions");
}

// =====================================================================

#[test]
fn a_window_arrives_and_leaves_on_two_different_clocks() {
    let mut fonts = FontSystem::new();
    let _ = motion::set_platform_reduce_motion(false);

    // ---- arriving. A window's FIRST sighting has not arrived yet: the
    // registry's born-at-its-value rule is right for a control that
    // scrolled into view and wrong for a window, which did not exist a
    // frame ago. This is the assertion that separates the two.
    master();
    let mut h = Host::opening(&mut fonts, 10.0);
    let born = h.now();
    assert_eq!(born.alpha, 0.0, "a window was born already open");
    assert!((scale(&born) - 0.96).abs() < 1e-5, "…and at its settled size");
    centred(&born);

    // Halfway through the master's 180 ms of ease_out: 1-(1-t)^2 at 0.5.
    let half = h.to(10.09);
    assert!((half.alpha - 0.75).abs() < 1e-3, "ease_out at half the entry is 0.75");
    assert!((scale(&half) - (0.96 + 0.04 * 0.75)).abs() < 1e-4, "the box lags its own alpha");
    centred(&half);

    // …and the ENDS ARE EXACT. A frame arriving at 0.99998 of its size is
    // chrome that no longer lines up with the client area inside it.
    let there = h.to(10.18);
    assert_eq!(there.alpha, 1.0);
    assert_eq!(scale(&there), 1.0, "the settled box is not the box it was given");
    assert!(there.rect.x == BOX.x && there.rect.y == BOX.y && there.rect.h == BOX.h);

    // ---- leaving, on its own faster clock. 140 ms of ease_in: t^2 at
    // half is 0.25, so a quarter of the way DOWN is three quarters left.
    let out = h.set(false);
    let going = h.to(out + 0.07);
    assert!((going.alpha - 0.75).abs() < 1e-3, "ease_in at half the exit leaves 0.75");
    // The exit is the faster of the two, and one number proves both: at
    // 140 ms the exit is over, and the entry — measured below — is not.
    let gone = h.to(out + 0.14);
    assert_eq!(gone.alpha, 0.0, "exactly zero is the host's signal to forget the window");
    assert!((scale(&gone) - 0.96).abs() < 1e-5);
    let mut h = Host::opening(&mut fonts, 30.0);
    assert!(h.now().alpha == 0.0 && h.to(30.14).alpha < 1.0, "the entry took the exit's 140 ms");

    // ---- the two entries are read separately. A fixture that moves ONE
    // of them moves one direction only.
    skin("[motion.window_close]\nduration_ms = 1000ms\neasing = linear\n");
    let mut h = Host::opening(&mut fonts, 40.0);
    assert_eq!(h.to(40.2).alpha, 1.0, "the entry took the exit's duration");
    let out = h.set(false);
    assert!((h.to(out + 0.5).alpha - 0.5).abs() < 1e-3, "the exit is not the fixture's");
    assert_eq!(h.to(out + 1.0).alpha, 0.0);

    // ---- an exit interrupted by a second thought turns round where it
    // stands, and finishes on the ENTRY's effect: the direction the user
    // is now watching is the one that names the curve.
    master();
    let mut h = Host::opening(&mut fonts, 50.0);
    assert_eq!(h.to(50.2).alpha, 1.0);
    let out = h.set(false);
    let mid = h.to(out + 0.07);
    let turned = at(h.fonts, true, h.t + 1e-9);
    assert!((mid.alpha - turned.alpha).abs() < 1e-3, "the reopen jumped: {mid:?} -> {turned:?}");
    h.t += 1e-9;
    h.open = true;
    assert_eq!(h.to(h.t + 0.2).alpha, 1.0, "…and it did not come back");

    // ---- reduced motion, both roads in: instant, and the box untouched.
    for body in ["[motion]\nscale = 0.0\n", "[a11y]\nreduced_motion = on\n"] {
        skin(body);
        let mut h = Host::opening(&mut fonts, 60.0);
        let open = h.now();
        assert_eq!(open.alpha, 1.0, "reduced motion left a window half-arrived");
        assert_eq!(scale(&open), 1.0, "…and the box grown from nothing");
        let out = h.set(false);
        assert_eq!(h.to(out).alpha, 0.0, "…and never let it leave");
    }

    // A theme may switch the entry off, and that is the ONE knob the
    // closed catalogue gives it for the growth: no presence, no transform.
    skin("[motion.window_open]\nenabled = false\n");
    let open = Host::opening(&mut fonts, 70.0).now();
    assert_eq!((open.alpha, scale(&open)), (1.0, 1.0), "a disabled entry still animated");

    // ---- and the contract the whole thing rests on: the rectangle the
    // caller keeps is never the one that moved. `present` answers a box
    // to DRAW in; the box handed in is what a hit test still sees, so a
    // pointer arriving during the entry — the ordinary case, since a
    // window usually opens because the hand is on its way — lands where
    // the window will be rather than where it momentarily is.
    master();
    let mid = Host::opening(&mut fonts, 80.0).now();
    assert!(mid.rect.w < BOX.w, "nothing was measured: the box did not shrink");
    assert!(!mid.rect.contains(BOX.x + 1.0, BOX.y + 1.0), "the drawn box covers the hit box");
    assert_eq!((BOX.x, BOX.y, BOX.w, BOX.h), (300.0, 200.0, 800.0, 500.0), "`r` was mutated");
    master();
}
