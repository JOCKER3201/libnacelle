//! The tooltip, actually drawn.
//!
//! The unit tests in `object/tooltip.rs` hold the delay, the grace
//! window and the placement still; this one runs the drawing itself,
//! through the real master theme and the real fonts, and looks at what
//! came out — that nothing is drawn before the pointer has rested long
//! enough, that the box the theme sizes stays on screen wherever the
//! pointer is, and that a long text grows the box downwards instead of
//! running off the side.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::object::tooltip::{key, Tooltips};
use nacelle::pointer::Pointer;
use nacelle::theme;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;

fn ctx<'a>(
    dl: &'a mut DrawList,
    fonts: &'a mut FontSystem,
    mouse: (f32, f32),
    t: f64,
) -> Ctx<'a> {
    Ctx {
        access: None,
        dl,
        fonts,
        w: W,
        h: H,
        t,
        mouse: Pointer::new(mouse.0, mouse.1),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    }
}

/// One frame: the owner of `anchor` files its request, the manager
/// draws. Answers the box that reached the screen, if any.
fn frame(
    tips: &mut Tooltips,
    fonts: &mut FontSystem,
    mouse: (f32, f32),
    t: f64,
    anchor: Rect,
    text: &str,
) -> (Option<Rect>, usize) {
    let mut dl = DrawList::new();
    let mut c = ctx(&mut dl, fonts, mouse, t);
    tips.hover(&c, key(text), anchor, text);
    tips.draw(&mut c);
    (tips.rect(), dl.verts.len())
}

#[test]
fn nothing_reaches_the_screen_until_the_delay_has_passed() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut tips = Tooltips::new();
    let anchor = Rect::new(100.0, 100.0, 200.0, 40.0);
    let delay = theme::resolved().px(theme::id("tooltip.delay_ms").unwrap()) as f64 / 1000.0;

    let (r, verts) = frame(&mut tips, &mut fonts, (150.0, 110.0), 0.0, anchor, "CPU LOAD");
    assert!(r.is_none() && verts == 0, "the tooltip drew before its delay");

    let (r, verts) = frame(&mut tips, &mut fonts, (150.0, 110.0), delay - 0.05, anchor, "CPU LOAD");
    assert!(r.is_none() && verts == 0, "the tooltip drew a frame early");

    let (r, verts) = frame(&mut tips, &mut fonts, (150.0, 110.0), delay, anchor, "CPU LOAD");
    assert!(r.is_some(), "the tooltip never appeared");
    assert!(verts > 0, "the tooltip claimed a box and drew nothing");

    // The pointer leaves the anchor: the owner files nothing, so the
    // tooltip goes with it.
    let mut dl = DrawList::new();
    {
        let mut c = ctx(&mut dl, &mut fonts, (900.0, 900.0), delay + 0.1);
        tips.draw(&mut c);
    }
    assert!(tips.rect().is_none() && dl.verts.is_empty(), "the tooltip outlived the pointer");
}

#[test]
fn the_box_stays_on_screen_in_every_corner() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let delay = theme::resolved().px(theme::id("tooltip.delay_ms").unwrap()) as f64 / 1000.0;
    let text = "the full text of a heading the column was too narrow to show";

    for (mx, my) in [(4.0, 4.0), (W - 4.0, 4.0), (4.0, H - 4.0), (W - 4.0, H - 4.0)] {
        let mut tips = Tooltips::new();
        let anchor = Rect::new(mx - 20.0, my - 10.0, 40.0, 20.0);
        frame(&mut tips, &mut fonts, (mx, my), 0.0, anchor, text);
        let (r, _) = frame(&mut tips, &mut fonts, (mx, my), delay, anchor, text);
        let r = r.expect("the tooltip never appeared");
        assert!(r.x >= 0.0, "off the left edge at ({mx}, {my}): {}", r.x);
        assert!(r.y >= 0.0, "off the top edge at ({mx}, {my}): {}", r.y);
        assert!(r.right() <= W + 0.01, "off the right edge at ({mx}, {my}): {}", r.right());
        assert!(r.bottom() <= H + 0.01, "off the bottom edge at ({mx}, {my}): {}", r.bottom());
    }
}

#[test]
fn one_line_is_the_themed_height_and_a_long_text_grows_downwards() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let t = theme::resolved();
    let delay = t.px(theme::id("tooltip.delay_ms").unwrap()) as f64 / 1000.0;
    let min_h = t.px(theme::id("tooltip.h").unwrap());
    let max_w = t.px(theme::id("tooltip.max_w").unwrap());
    let pad_x = t.px(theme::id("tooltip.pad_x").unwrap());
    let anchor = Rect::new(400.0, 400.0, 200.0, 40.0);

    let mut tips = Tooltips::new();
    frame(&mut tips, &mut fonts, (450.0, 410.0), 0.0, anchor, "CPU");
    let (short, _) = frame(&mut tips, &mut fonts, (450.0, 410.0), delay, anchor, "CPU");
    let short = short.expect("the short tooltip never appeared");
    // `tooltip.h` is the floor a single line sits at — the token the
    // popup pattern always had and nothing ever read.
    assert!((short.h - min_h).abs() < 0.01, "one line is not tooltip.h: {} vs {min_h}", short.h);

    let long = "This heading was trimmed to fit its column, and the tooltip \
                is where the whole of it is finally readable, however long \
                the words behind the ellipsis turn out to be.";
    let mut tips = Tooltips::new();
    frame(&mut tips, &mut fonts, (450.0, 410.0), 0.0, anchor, long);
    let (tall, _) = frame(&mut tips, &mut fonts, (450.0, 410.0), delay, anchor, long);
    let tall = tall.expect("the long tooltip never appeared");
    assert!(tall.h > short.h, "the long text did not grow the box: {} ", tall.h);
    // It wrapped instead of running on: the box never passes max_w plus
    // its two paddings.
    assert!(
        tall.w <= max_w + 2.0 * pad_x + 0.01,
        "the text ran past tooltip.max_w: {} > {}",
        tall.w,
        max_w + 2.0 * pad_x
    );
}
