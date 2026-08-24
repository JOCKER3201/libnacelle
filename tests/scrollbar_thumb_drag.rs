//! A script widget's scrollbar can be dragged by its thumb.
//!
//! Everything under this was already written and already tested: the
//! grab model (`view/scroll.rs` — `press_thumb`, `drag`, `release`), the
//! bar's geometry (`view::scroll::scrollbar`), and the hit rectangle the
//! draw records for it (`Hit::Thumb`). What was missing was one word in
//! the routing. `ScriptWidget::drag(Begin)` answered [`Action::None`]
//! for every press, with a comment calling the decline deliberate and
//! the grab "waiting for the first `Move`" — but a host sends `Move`
//! only to a widget that answered [`Action::Capture`], so the `Move`
//! never arrived and the whole `Hit::Thumb` branch was unreachable. The
//! owner reported it as "the scrollbar should be draggable with the
//! mouse and it is not".
//!
//! Two claims, and the second is why this is not a one-line assertion on
//! a return value:
//!
//! * a press ON the thumb asks for the pointer, and a press beside it
//!   does not — an ordinary click still reaches the click path, which is
//!   where selecting a row happens, and a captured press ends in no
//!   click at all;
//! * and the gesture that capture unlocks actually MOVES the view: the
//!   thumb is where the hand left it on the next frame.
//!
//! ONE test in a binary of its own: the resolved theme is process-wide
//! (§7.1), and this one loads it.

use nacelle::draw::{DrawCmd, DrawList};
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::script::{Script, ScriptWidget};
use nacelle::telemetry::Snapshot;
use nacelle::theme;
use nacelle::widget::{Action, DragPhase, Host, Widget};
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
/// Tall enough to draw a bar, short enough that fifty rows overflow it
/// several times over — the thumb is then a small fraction of the track
/// and has room to travel.
const PANEL: Rect = Rect { x: 120.0, y: 90.0, w: 460.0, h: 300.0 };

/// A script that draws ONE scrolling list and nothing else, so the last
/// rectangle in the frame is the bar's.
fn widget() -> ScriptWidget {
    let path = std::env::temp_dir()
        .join(format!("nacelle-thumb-drag-{}.rhai", std::process::id()));
    std::fs::write(
        &path,
        r#"
        fn draw() {
            let rows = [];
            for i in 0..50 { rows.push(`LINE ${i}`); }
            [ list(rows, #{ id: "log", scroll: true, select: "row" }) ]
        }
        "#,
    )
    .expect("the fixture script must be writable");
    let script = Script::load(&path).expect("the fixture script must compile");
    let _ = std::fs::remove_file(&path);
    ScriptWidget::new(script)
}

/// One frame, and the thumb rectangle it drew.
///
/// The bar is laid down LAST, over the rows it covers (u2 §2.10), and
/// the thumb is the fill in it — so the final `ring_fill` of the frame
/// is the thumb, whatever the theme decided its groove looks like.
fn frame(widget: &mut ScriptWidget, fonts: &mut FontSystem) -> Rect {
    let snap = Snapshot::default();
    let host = Host {
        snap: &snap,
        term: None,
        tabs: &[],
        tab_active: 0,
        shell_cwd: None,
        t: 0.0,
        window: (W, H),
    };
    let mut dl = DrawList::recording();
    {
        let mut ctx = Ctx {
            access: None,
            dl: &mut dl,
            fonts,
            w: W,
            h: H,
            t: 0.0,
            // Resting on the bar's own band. `scrollbar.auto_hide` is
            // on in the master, so an unhovered bar fades to nothing and
            // draws nothing at all — a hand that is about to grab the
            // thumb is over it by definition, and the geometry the hits
            // record is the geometry this frame drew.
            mouse: Pointer::new(PANEL.right() - 4.0, PANEL.y + PANEL.h / 2.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        };
        widget.draw(&mut ctx, PANEL, &host);
    }
    dl.cmds()
        .iter()
        .rev()
        .find_map(|c| match c {
            DrawCmd::RingFill { r, .. } => Some(Rect::new(r[0], r[1], r[2], r[3])),
            _ => None,
        })
        .expect("the list drew no scrollbar thumb — the fixture does not overflow")
}

fn press(widget: &mut ScriptWidget, x: f32, y: f32) -> Action {
    let snap = Snapshot::default();
    let host = Host {
        snap: &snap,
        term: None,
        tabs: &[],
        tab_active: 0,
        shell_cwd: None,
        t: 0.0,
        window: (W, H),
    };
    widget.drag(DragPhase::Begin, x, y, PANEL, &host)
}

fn drag_to(widget: &mut ScriptWidget, x: f32, y: f32) {
    let snap = Snapshot::default();
    let host = Host {
        snap: &snap,
        term: None,
        tabs: &[],
        tab_active: 0,
        shell_cwd: None,
        t: 0.0,
        window: (W, H),
    };
    let _ = widget.drag(DragPhase::Move, x, y, PANEL, &host);
}

#[test]
fn a_press_on_the_thumb_asks_for_the_pointer_and_a_press_beside_it_does_not() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut w = widget();

    let thumb = frame(&mut w, &mut fonts);
    assert!(
        thumb.h > 0.0 && thumb.h < PANEL.h,
        "a thumb that fills its track is a list that does not scroll: {thumb:?}"
    );
    let (tx, ty) = (thumb.x + thumb.w / 2.0, thumb.y + thumb.h / 2.0);

    // ---- the thumb ----------------------------------------------------
    assert_eq!(
        press(&mut w, tx, ty),
        Action::Capture,
        "the press on the thumb declined the pointer, so no Move will ever arrive"
    );

    // ---- and a row beside it ------------------------------------------
    // Same height, well inside the content: this is an ordinary click and
    // has to stay one, or a script's list could no longer be selected in.
    let row_x = PANEL.x + PANEL.w * 0.25;
    assert_eq!(
        press(&mut w, row_x, ty),
        Action::None,
        "a press on a row asked for the pointer, which would swallow its click"
    );
    // The corners of the panel, which no view claims at all.
    assert_eq!(press(&mut w, PANEL.x + 1.0, PANEL.y + 1.0), Action::None);

    // ---- and the gesture the capture unlocks --------------------------
    // Grab the thumb and pull it a third of the panel down. Before the
    // routing was fixed this is the branch that could not be reached; if
    // it is still unreachable the thumb stands exactly where it was.
    let travel = PANEL.h / 3.0;
    assert_eq!(press(&mut w, tx, ty), Action::Capture);
    drag_to(&mut w, tx, ty + travel);
    let moved = frame(&mut w, &mut fonts);
    assert!(
        moved.y > thumb.y + 1.0,
        "the thumb did not follow the hand: {} -> {}",
        thumb.y,
        moved.y
    );

    // And it stops where the hand does, rather than running on: the
    // travel is bounded by the track, so the thumb's bottom stays inside
    // the panel it belongs to.
    assert!(
        moved.bottom() <= PANEL.bottom() + 1.0,
        "the thumb left its track: {moved:?} against {PANEL:?}"
    );
}
