//! `motion.hold` — the ramp that stands between a click and image 5's
//! SYSTEM LOCKDOWN.
//!
//! The entry has been in §5.22's closed catalogue since it was written,
//! with `duration_ms = 5000ms`, a range of 1500..8000 and a comment
//! naming the control it is for, and nothing read it. This binary is that
//! reader's proof, and half of it is about a rule the rest of the module
//! does not follow: a hold IGNORES `motion.scale`.
//!
//! Everywhere else, reduced motion means "jump to the end state" — the
//! travel is decoration, the destination is the point. Here the travel IS
//! the safeguard, and a jump to the end state would fire the most
//! destructive control in the program on the first frame the button went
//! down, for the user who asked for less animation. `the_ramp_ignores_the
//! _global_scale` is that assertion, and it is the one test in this file
//! that is about safety rather than about looks.
//!
//! Time is a parameter throughout; nothing sleeps.
//!
//! One test in a binary of its own: the resolved theme is process-wide.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::object::button::{self, ButtonState};
use nacelle::pointer::Pointer;
use nacelle::{motion, theme, Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const BOX: Rect = Rect { x: 60.0, y: 40.0, w: 260.0, h: 44.0 };
const CAP: &str = "SYSTEM LOCKDOWN";

fn skin(body: &str) {
    let path = std::env::temp_dir().join(format!("nacelle-hold-{}.theme", std::process::id()));
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

/// The ramp, asked the way a control asks it.
fn ramp(down: bool, now: f64) -> motion::Held {
    motion::hold("test.lockdown", BOX, down, now)
}

/// One frame of a 60 Hz host.
const FRAME: f64 = 1.0 / 60.0;

/// A finger on the button, driven the way a host drives it: the control
/// is drawn EVERY frame, so the ramp is asked every frame.
///
/// A test that jumped straight from the press to the fire would not be
/// measuring the ramp at all. The registry reclaims a key it has not been
/// asked about for `KEEP_SECS` — that is what "the control left the
/// screen" looks like from inside, and for a hold it is a cancellation,
/// which is the safe reading and the one a half-second gap would trip.
struct Finger {
    t: f64,
    last: motion::Held,
}

impl Finger {
    fn press(t0: f64) -> Finger {
        Finger { t: t0, last: ramp(true, t0) }
    }

    /// Holds on until `t`, a frame at a time, answering the ramp there.
    fn to(&mut self, t: f64) -> motion::Held {
        while self.t + FRAME < t {
            self.t += FRAME;
            self.last = ramp(true, self.t);
        }
        self.t = t;
        self.last = ramp(true, t);
        self.last
    }

    /// Holds on until `t` and answers whether it fired ANYWHERE along the
    /// way — the question a caller actually asks, since the frame the ramp
    /// completes is whichever frame the host happened to draw.
    fn fires_by(&mut self, t: f64) -> bool {
        let mut fired = self.last.fired;
        while self.t + FRAME < t {
            self.t += FRAME;
            fired |= ramp(true, self.t).fired;
        }
        self.t = t;
        fired | ramp(true, t).fired
    }

    fn release(&mut self, t: f64) -> motion::Held {
        self.t = t;
        self.last = ramp(false, t);
        self.last
    }
}

fn ctx<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem, now: f64) -> Ctx<'a> {
    Ctx {
        access: None,
        dl,
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
    }
}

/// The vertices `draw_hold` puts on the screen, from a registry with no
/// memory: every call below is a first sighting, so nothing here is
/// measuring a state fade left over from the call before it.
fn held_verts(fonts: &mut FontSystem, down: bool, now: f64) -> usize {
    motion::forget_fades();
    let mut dl = DrawList::new();
    let mut c = ctx(&mut dl, fonts, now);
    button::draw_hold(&mut c, BOX, CAP, ButtonState::default(), "test.lockdown", down);
    dl.verts.len()
}

// =====================================================================

#[test]
fn a_hold_takes_the_time_the_catalogue_says() {
    let mut fonts = FontSystem::new();
    let _ = motion::set_platform_reduce_motion(false);

    the_master_s_five_seconds(&mut fonts);
    the_ramp_ignores_the_global_scale();
    the_range_at_the_key_is_a_clamp();
    disabled_is_the_one_way_to_make_it_immediate();
    a_release_starts_over_and_does_not_resume();
    a_control_born_under_a_finger_starts_now();
    the_curve_is_linear_whatever_the_theme_writes();
    the_readout_costs_nothing_at_rest(&mut fonts);
    master();
}

/// The master's own entry: 5 000 ms, linear, and the ramp is the fraction
/// of that time elapsed. `fired` is answered on the ask that arrives, once
/// — and again to a second ask at the SAME clock, because one frame asked
/// twice is one event.
fn the_master_s_five_seconds(_fonts: &mut FontSystem) {
    master();
    assert_eq!(ramp(false, 100.0), motion::Held { progress: 0.0, fired: false });
    let mut f = Finger::press(100.0);
    assert_eq!(f.last.progress, 0.0, "the ramp began before the finger did");
    assert!((f.to(101.25).progress - 0.25).abs() < 1e-4);
    assert!((f.to(102.5).progress - 0.5).abs() < 1e-4, "half the time is half the ramp");
    assert!(!f.fires_by(104.999), "it fired before the five seconds were up");
    let done = f.to(105.0);
    assert_eq!(done.progress, 1.0);
    assert!(done.fired, "five seconds passed and nothing fired");
    assert!(ramp(true, 105.0).fired, "one frame asked twice answered two different things");
    assert!(!f.to(105.0 + FRAME).fired, "it fired twice for one hold");
    assert!(!f.fires_by(106.0), "…and again later");
    // The host is owed frames while a ramp is in flight.
    master();
    let _ = ramp(true, 10.0);
    assert!(motion::pending(10.0), "a running ramp asked for no frames");
}

/// THE safety assertion. `motion.scale = 0` and `a11y.reduced_motion = on`
/// are both "jump to the end state" for every fade in the toolkit; if the
/// hold obeyed either, one frame with the button down would fire it.
///
/// Mutation: delete the paragraph in `motion::hold` that omits
/// `motion_scale()` and this fails on the first line.
fn the_ramp_ignores_the_global_scale() {
    for body in ["[motion]\nscale = 0.0\n", "[a11y]\nreduced_motion = on\n"] {
        skin(body);
        let mut f = Finger::press(50.0);
        assert_eq!(f.last.progress, 0.0);
        let one_frame = f.to(50.0 + FRAME);
        assert!(one_frame.progress < 1.0, "reduced motion fired the lockdown on one frame");
        assert!(!one_frame.fired, "…and reported it");
        assert!((one_frame.progress - FRAME as f32 / 5.0).abs() < 1e-4, "the ramp left wall time");
        assert!(f.fires_by(55.0), "…but it still gets there, in the time it says");
    }
    // The same, asked for by the platform rather than by the file.
    skin("[a11y]\nreduced_motion = system\n");
    motion::set_platform_reduce_motion(true);
    let mut f = Finger::press(60.0);
    assert!(f.last.progress == 0.0 && f.to(60.0 + FRAME).progress < 1.0);
    motion::set_platform_reduce_motion(false);
}

/// `duration_ms` is documented `1500ms .. 8000ms`, and the bound is what
/// makes "at least a second and a half" true however the file reads. A
/// theme outside it is brought back rather than refused: the control keeps
/// working.
fn the_range_at_the_key_is_a_clamp() {
    skin("[motion.hold]\nduration_ms = 10ms\n");
    let mut f = Finger::press(70.0);
    assert!(f.last.progress == 0.0 && !f.fires_by(70.5), "10 ms was honoured");
    assert!(f.fires_by(71.5), "the floor is 1500 ms and it was not reached");

    skin("[motion.hold]\nduration_ms = 60000ms\n");
    let mut f = Finger::press(80.0);
    assert!(!f.fires_by(87.9), "the ceiling is 8000 ms and something fired early");
    assert!(f.fires_by(88.0), "the ceiling is 8000 ms and it was not honoured");
}

/// `enabled = false` is the ONE way a theme may make the control
/// immediate, and the master says so at the key: "setting false makes the
/// control fire immediately — a safety decision". A decision is allowed to
/// be written down; a curve is not allowed to imply one.
fn disabled_is_the_one_way_to_make_it_immediate() {
    skin("[motion.hold]\nenabled = false\n");
    let mut f = Finger::press(90.0);
    assert_eq!(f.last.progress, 1.0);
    assert!(f.last.fired, "a disabled hold did not fire at once");
    assert!(!f.to(90.0 + FRAME).fired, "…and it still fires only once");
}

/// A hold is not resumable. Letting go is what cancels, and the next press
/// starts from nothing — which is the difference between "hold this for
/// five seconds" and "click this five hundred times".
fn a_release_starts_over_and_does_not_resume() {
    master();
    let mut f = Finger::press(10.0);
    assert!((f.to(14.0).progress - 0.8).abs() < 1e-4, "four fifths of the way");
    assert_eq!(f.release(14.0 + FRAME).progress, 0.0, "letting go left the ramp standing");
    let mut f = Finger::press(14.0 + 2.0 * FRAME);
    assert_eq!(f.last.progress, 0.0, "the second press resumed the first");
    assert!(!f.fires_by(15.0), "a resumed ramp fired a second early");
    assert!(f.fires_by(19.1), "…and the fresh five seconds never arrived");
}

/// The born-settled rule's safety-critical twin: a control the registry
/// has never seen, asked about while ALREADY down, starts its clock now.
/// A dialog opening under a finger that is already on the button cannot
/// confirm anything.
fn a_control_born_under_a_finger_starts_now() {
    master();
    motion::forget_fades();
    let mut f = Finger::press(500.0);
    assert_eq!(f.last.progress, 0.0, "a newborn control was already part-held");
    assert!(!f.fires_by(504.9));
    assert!(f.fires_by(505.0), "…and it never got there either");
}

/// The ramp runs LINEAR whatever `easing` says, and the reason is not
/// taste: `step` with a zero duty answers 1.0 on the first frame, and
/// `custom` with an overshoot answers it early. The number this returns is
/// the interlock as well as the readout.
fn the_curve_is_linear_whatever_the_theme_writes() {
    skin("[motion.hold]\neasing = step\nduty = 0.0\nfloor = 1.0\n");
    let mut f = Finger::press(300.0);
    assert_eq!(f.last.progress, 0.0, "a step curve fired the lockdown on the first frame");
    assert!(!f.last.fired);
    assert!((f.to(302.5).progress - 0.5).abs() < 1e-4, "the ramp is not linear in time");

    skin("[motion.hold]\neasing = custom\neasing_p = [0.0, 3.0, 1.0, 1.0]\n");
    let mut f = Finger::press(400.0);
    assert!((f.to(402.5).progress - 0.5).abs() < 1e-4, "an overshoot got through");
    let _ = f.release(403.0);
}

/// At rest the control is a button and nothing else: the same vertices,
/// exactly, that `draw` puts on the screen. The readout appears only once
/// the ramp has left zero, and it is a stroke — same shape, more line.
fn the_readout_costs_nothing_at_rest(fonts: &mut FontSystem) {
    master();
    // Warm the atlas: the first sighting of a glyph rasterises it, and
    // that is a difference between two draws that has nothing to do with
    // this file.
    let _ = held_verts(fonts, false, 0.0);

    let plain = {
        motion::forget_fades();
        let mut dl = DrawList::new();
        let mut c = ctx(&mut dl, fonts, 600.0);
        button::draw(&mut c, BOX, CAP, ButtonState::default());
        dl.verts.len()
    };
    assert_eq!(held_verts(fonts, false, 600.0), plain, "a released hold drew a readout");

    // The frame the finger lands: pressed, but nothing swept yet.
    let pressed = {
        motion::forget_fades();
        let mut dl = DrawList::new();
        let mut c = ctx(&mut dl, fonts, 600.0);
        button::draw(&mut c, BOX, CAP, ButtonState { flash: true, ..Default::default() });
        dl.verts.len()
    };
    motion::forget_fades();
    let mut dl = DrawList::new();
    let mut c = ctx(&mut dl, fonts, 700.0);
    button::draw_hold(&mut c, BOX, CAP, ButtonState::default(), "test.lockdown", true);
    assert_eq!(dl.verts.len(), pressed, "the first frame of a hold drew a sweep of nothing");

    // …and part-way round there is more line on the screen, growing with
    // the ramp rather than appearing whole. Drawn every frame, because a
    // control that is not drawn is a control that left the screen.
    let mut at = |secs: f64| {
        let mut n = 0;
        let mut t = 700.0 + FRAME;
        while t <= secs + 1e-9 {
            let mut dl = DrawList::new();
            let mut c = ctx(&mut dl, fonts, t);
            button::draw_hold(&mut c, BOX, CAP, ButtonState::default(), "test.lockdown", true);
            n = dl.verts.len();
            t += FRAME;
        }
        n
    };
    let quarter = at(701.25);
    let half = at(702.5);
    let full = at(705.0);
    assert!(quarter > pressed, "a quarter of the way round, nothing was drawn");
    assert!(half > quarter && full > half, "the sweep does not grow with the ramp");
}
