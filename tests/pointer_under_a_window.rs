//! A control with a window drawn over it is not the control the pointer
//! is on.
//!
//! The owner's report, from a screenshot: the settings window open and on
//! top, the on-screen keyboard underneath it, and the cap the cursor
//! happened to stand over — "6" — lit up through the window. The window
//! is translucent, which is the only reason the fault was visible at all;
//! an opaque one would have hidden the same defect rather than fixed it.
//!
//! What is measured here is therefore not a colour. Two things are drawn
//! one over the other, the pointer is put on the area they share, and
//! each is asked the one question every control asks — "is the pointer on
//! me?" — through [`Surface::mouse`], which is the single funnel the
//! toolkit's own views, the Rhai script renderer and (across the ABI, as
//! `HostApi::mouse`) every compiled plugin all read the pointer through.
//! The keyboard is a plugin and asks exactly this way.
//!
//! The answer must name ONE of them, and it must be the one on top —
//! whatever the two of them happen to be. Nothing below is about the
//! settings window or about a keyboard: the cases stack a window over a
//! cap, a window over a window, and nothing at all over a cap, and the
//! same rule has to answer all three.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::object::window;
use nacelle::pointer::Pointer;
use nacelle::theme;
use nacelle::view::{CtxSurface, Surface};
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;

/// A cap of the on-screen keyboard, in the place the screenshot put it:
/// under the window.
const CAP: Rect = Rect { x: 700.0, y: 500.0, w: 60.0, h: 60.0 };

/// The application's window, drawn over it.
const WINDOW: Rect = Rect { x: 400.0, y: 200.0, w: 1100.0, h: 700.0 };

/// A row of that window — "LOOK AND FEEL" in the screenshot — standing
/// over the cap.
const ROW: Rect = Rect { x: 640.0, y: 505.0, w: 400.0, h: 44.0 };

/// A second window over the first: the pair the rule must also answer,
/// so that nothing here is a special case for one window and one widget.
const OVER_WINDOW: Rect = Rect { x: 600.0, y: 460.0, w: 500.0, h: 300.0 };
const OVER_ROW: Rect = Rect { x: 660.0, y: 505.0, w: 300.0, h: 44.0 };

/// The point all of them hold.
const SHARED: (f32, f32) = (730.0, 530.0);

/// A cap standing clear of every window, to catch the opposite mistake:
/// a rule that answers "nothing is hovered" is as wrong as one that
/// answers "everything is".
const CAP_OUTSIDE: Rect = Rect { x: 120.0, y: 940.0, w: 60.0, h: 60.0 };
const OUTSIDE: (f32, f32) = (150.0, 970.0);

/// Which of the stacked rectangles took the pointer for its own.
#[derive(Debug, Default, PartialEq, Eq)]
struct Seen {
    /// The cap, drawn first — the bottom of the stack.
    cap: bool,
    /// A row of the window drawn over it.
    row: bool,
    /// A row of the window drawn over THAT one, when the case stacks
    /// three.
    over_row: bool,
    /// The cap standing clear of every window.
    outside: bool,
}

/// What one control asks, the way every one of them asks it.
///
/// Deliberately not a call to some new predicate: this is the shape the
/// keyboard plugin's own line has (`krect.contains(mx, my)` on the
/// position `HostApi::mouse` handed it), so a fix that leaves this
/// reading true has not fixed anything.
fn hovered(ctx: &mut Ctx, r: Rect) -> bool {
    let (x, y) = CtxSurface::new(ctx).mouse();
    r.contains(x, y)
}

/// How many windows the frame draws over the cap, and whether the first
/// of them dims the desktop behind it (a modal).
#[derive(Clone, Copy)]
struct Stack {
    windows: u8,
    modal: bool,
}

/// Draws one frame of a desktop with `stack` windows over a keyboard cap,
/// and answers who considered themselves hovered.
///
/// Everything is drawn in the order it reaches the screen, because that
/// order IS the z-order in an immediate-mode frame: the board's widgets
/// first, the application's own windows over them.
///
/// `p` is the application's own pointer, lent to the frame and taken back
/// at the end of it — the contract the host keeps, written out here so
/// the test exercises it rather than a private shortcut.
fn shot(fonts: &mut FontSystem, p: &mut Pointer, at: (f32, f32), stack: Stack) -> Seen {
    p.begin(at);
    let mut dl = DrawList::new();
    let mut seen = Seen::default();
    let mut ctx = Ctx {
        access: None,
        dl: &mut dl,
        fonts,
        w: W,
        h: H,
        t: 0.0,
        mouse: std::mem::take(p),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    };

    // The board: a keyboard cap under whatever comes next, and a second
    // one standing clear of it.
    seen.cap = hovered(&mut ctx, CAP);
    seen.outside = hovered(&mut ctx, CAP_OUTSIDE);

    if stack.windows >= 1 {
        if stack.modal {
            window::backdrop(&mut ctx, 1.0);
        }
        window::frame(&mut ctx, WINDOW);
        seen.row = hovered(&mut ctx, ROW);
    }
    if stack.windows >= 2 {
        window::frame(&mut ctx, OVER_WINDOW);
        seen.over_row = hovered(&mut ctx, OVER_ROW);
    }
    *p = std::mem::take(&mut ctx.mouse);
    seen
}

/// The same desktop, drawn twice, answering for the second frame.
///
/// Draw order is only known once a frame has been drawn: the cap is
/// painted before the window that covers it, so what covers it is a fact
/// about the frame just gone. A window that has been open for longer than
/// one frame — which is every window a hand can point at — is therefore
/// answered exactly.
fn steady(fonts: &mut FontSystem, at: (f32, f32), stack: Stack) -> Seen {
    let mut p = Pointer::default();
    let _ = shot(fonts, &mut p, at, stack);
    shot(fonts, &mut p, at, stack)
}

fn master() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
}

// ---------------------------------------------------------------------

#[test]
fn the_cap_under_the_window_is_not_the_one_the_pointer_is_on() {
    master();
    let mut fonts = FontSystem::new();
    let seen = steady(&mut fonts, SHARED, Stack { windows: 1, modal: true });
    assert_eq!(
        seen,
        Seen { cap: false, row: true, over_row: false, outside: false },
        "the pointer stands on one point of one screen, so exactly one of \
         the stacked controls may call itself hovered — the one on top. \
         `cap: true` is the owner's screenshot: the cap lit through the window."
    );
}

#[test]
fn with_nothing_over_it_the_cap_is_hovered_as_before() {
    master();
    let mut fonts = FontSystem::new();
    let seen = steady(&mut fonts, SHARED, Stack { windows: 0, modal: false });
    assert_eq!(
        seen,
        Seen { cap: true, row: false, over_row: false, outside: false },
        "with no window over it the cap is what the pointer is on, and a \
         rule that takes the pointer away from it has broken the interface \
         it was written to fix"
    );
}

#[test]
fn a_window_takes_only_the_ground_it_covers() {
    master();
    let mut fonts = FontSystem::new();
    // A window that does not dim the desktop claims its own rectangle and
    // nothing else; the cap standing clear of it is still pointed at.
    let seen = steady(&mut fonts, OUTSIDE, Stack { windows: 1, modal: false });
    assert_eq!(
        seen,
        Seen { cap: false, row: false, over_row: false, outside: true },
        "the pointer is outside the window, on a cap the window does not \
         cover — that cap is hovered and the window's row is not"
    );
}

#[test]
fn the_rule_is_about_what_is_on_top_and_not_about_windows() {
    master();
    let mut fonts = FontSystem::new();
    let seen = steady(&mut fonts, SHARED, Stack { windows: 2, modal: true });
    assert_eq!(
        seen,
        Seen { cap: false, row: false, over_row: true, outside: false },
        "three things share the point; the topmost is the answer. A fix \
         that only knows the pair \"window over widget\" leaves the middle \
         one lit and looks finished"
    );
}
