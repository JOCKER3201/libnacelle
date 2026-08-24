//! A blind longer than its frame SCROLLS: the body stops at
//! `menu.max_h_frac` of the viewport (floored by `menu.max_h_min_px`),
//! the wheel moves the offset the toolkit way, the toolkit's scrollbar
//! stands in the inset lane beside the slats, and an element outside
//! the frame is exactly as dead as an element under a foreign clip —
//! no area, no answered centre, no place in the focus chain.
//!
//! The defect this file pins down (M8 of the settings-window spec): the
//! accordion's height was `pitch × names.len()` with no ceiling, so a
//! list of forty themes hung past the desktop's edge and stayed
//! pressable where it could not be seen. The frame is the ceiling; what
//! does not fit arrives by wheel.
//!
//! What is asserted, stage by stage:
//!
//! * a SHORT list is untouched — the anchor's width to the pixel, every
//!   element whole, no bar drawn, the frame's clip simply the body's
//!   own height;
//! * a LONG list stops at the frame: nothing reported below it, the
//!   element on the line cut TO the line, everything past it a ghost
//!   with no area — the very assertions the foreign-clip file makes,
//!   because the frame cuts hits the same way that clip does;
//! * the WHEEL moves the column by `scroll.wheel_px` a notch, clamps at
//!   the end, and lands the last element exactly on the frame's bottom
//!   edge — the tail is reachable without hanging anywhere;
//! * the BAR is the toolkit's: its thumb stands at the very rectangle
//!   [`scroll::scrollbar`] answers for the same offset, and the slats
//!   make room for its lane (`scrollbar.mode = inset` in the master);
//! * a FOREIGN clip and the frame cut together, tighter edge wins, and
//!   the host's clip stack is left as it was built;
//! * the TOKENS move the frame — the fixture that lowers `max_h_frac`
//!   lowers the last answered pixel, and the px floor holds it up when
//!   the fraction collapses;
//! * elements outside the frame REGISTER NOTHING: the focus chain holds
//!   exactly the elements reported whole, and nothing else;
//! * the CORD AND THE OFFSET COMPOSE as `stowed + min(p·D, d_i) −
//!   offset` and in that order — scrolling a half-open list translates
//!   the whole column, piled slats and landed ones alike, and the cord
//!   still reaches the end. Every other stage here draws at `p = 1.0`,
//!   where the two candidate compositions agree; this one separates
//!   them.
//!
//! One test in a binary of its own: the resolved theme is process-wide
//! (§7.1 hands every draw path the same `&'static ResolvedTheme`), and
//! this file swaps fixture themes, so it shares a process with nothing.

use nacelle::draw::{DrawCmd, DrawList};
use nacelle::focus::{FocusCtl, FocusId};
use nacelle::font::FontSystem;
use nacelle::object::dropdown::{self, AccordionStyle};
use nacelle::theme;
use nacelle::view::scroll::{self, ScrollPhysics, ScrollView, ScrollbarLook};
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const ITEM_H: f32 = 30.0;
/// The anchor the blind hangs from — wide enough that `menu.min_w`
/// cannot move it, and high enough that the master's frame fits whole
/// between it and the window's bottom edge.
const ANCHOR: Rect = Rect { x: 200.0, y: 300.0, w: 400.0, h: 36.0 };
/// Off screen: nothing hovers, so the bar is measured at resting width.
const AWAY: (f32, f32) = (-1.0, -1.0);
/// Forty, because forty theme names is the owner's own example of a
/// list that outgrew the window — and more than any frame the master
/// states at this viewport, so every stage has elements past the line.
const LONG: usize = 40;
/// Nine fits the master's frame with room to spare: the negative
/// control for every claim about what scrolling changes.
const SHORT: usize = 9;

fn names(n: usize) -> Vec<String> {
    (0..n).map(|i| format!("THEME {i:02}")).collect()
}

/// Loads a fixture theme whose base is the master, so every token but
/// the ones in `body` is the master's own.
fn skin(body: &str) {
    let path = std::env::temp_dir().join(format!("nacelle-dscroll-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        format!("[meta]\nschema = 1\nname = \"Fixture\"\nbase = \"default\"\n\n{body}"),
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(theme::LoadRequest { path: Some(path.clone()), ..Default::default() });
    let _ = std::fs::remove_file(&path);
    theme::set_viewport(H, 1.0);
}

fn master() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
}

fn px_of(name: &str) -> f32 {
    let t = theme::resolved();
    t.px(theme::id(name).unwrap_or_else(|| panic!("the master declares {name}")))
}

/// The frame's numbers, read the way the object reads them: one pitch
/// per element, capped at the fraction of the viewport, floored in
/// device px. Asked of the theme so the fixture stages move them.
fn frame(n: usize) -> (f32, f32, f32) {
    let gap = px_of("menu.anchor_gap");
    let pitch = ITEM_H + gap;
    let content = pitch * n as f32;
    let cap = (H * px_of("menu.max_h_frac")).max(px_of("menu.max_h_min_px"));
    (pitch, content, content.min(cap))
}

/// One drawing of the FULLY OPEN blind — [`shoot_at`] at `p = 1.0`,
/// which is the progress every stage but the composition one wants.
fn shoot(
    fonts: &mut FontSystem,
    n: usize,
    sv: &mut ScrollView,
    clip: Option<Rect>,
) -> (DrawList, Vec<(Rect, bool)>) {
    shoot_at(fonts, n, 1.0, sv, clip)
}

/// One drawing of the blind at unfold progress `p`: the register it
/// wrote and the rectangles it handed back, through the caller's own
/// scroll state and (optionally) under a foreign clip.
fn shoot_at(
    fonts: &mut FontSystem,
    n: usize,
    p: f32,
    sv: &mut ScrollView,
    clip: Option<Rect>,
) -> (DrawList, Vec<(Rect, bool)>) {
    let names = names(n);
    let mut dl = DrawList::recording();
    if let Some(c) = clip {
        dl.push_clip(c.x, c.y, c.w, c.h);
    }
    let rects = {
        let mut ctx = Ctx {
            access: None,
            dl: &mut dl,
            fonts,
            w: W,
            h: H,
            t: 0.0,
            mouse: nacelle::pointer::Pointer::new(AWAY.0, AWAY.1),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        };
        dropdown::accordion(&mut ctx, ANCHOR, ITEM_H, &names, p, &AccordionStyle::default(), sv)
    };
    match clip {
        Some(c) => assert_eq!(
            dl.clip_stack(),
            vec![[c.x, c.y, c.w, c.h]],
            "the list unbalanced its host's clip stack"
        ),
        None => assert_eq!(dl.clip_stack(), Vec::<[f32; 4]>::new(), "the list left a clip behind"),
    }
    (dl, rects)
}

/// The clip the list pushed for its own body — the first command it
/// issues, exactly as the blind file already pins.
fn own_clip(dl: &DrawList) -> [f32; 4] {
    dl.cmds()
        .iter()
        .find_map(|c| match c {
            DrawCmd::ClipPush { r } => Some(*r),
            _ => None,
        })
        .expect("the list pushed no clip at all")
}

/// Every RingFill in the register. A slat's plate is one, and so is the
/// bar's thumb — told apart by their rectangles, never by their order.
fn ring_fills(dl: &DrawList) -> Vec<[f32; 4]> {
    dl.cmds()
        .iter()
        .filter_map(|c| match c {
            DrawCmd::RingFill { r, .. } => Some(*r),
            _ => None,
        })
        .collect()
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 0.01
}

// =====================================================================

/// One test in the binary, stages in order: `skin` swaps the
/// process-wide resolved theme, so no stage may run beside another.
#[test]
fn a_long_list_scrolls_inside_its_frame() {
    master();
    let mut fonts = FontSystem::new();
    a_short_list_is_untouched(&mut fonts);
    a_long_list_stops_at_its_frame(&mut fonts);
    the_wheel_moves_the_column_and_reaches_the_end(&mut fonts);
    the_bar_is_the_toolkits_and_the_slats_make_room(&mut fonts);
    a_foreign_clip_and_the_frame_cut_together(&mut fonts);
    the_tokens_move_the_frame(&mut fonts);
    elements_outside_the_frame_join_no_chain(&mut fonts);
    the_cord_and_the_offset_compose(&mut fonts);
}

/// THE TWO LAWS COMPOSE, AND IN ONE ORDER. The unfold (`min(p·D, d_i)`,
/// the two-phase cord) is written in the BODY's coordinates and the
/// offset slides the FRAME over that body, so element `i` stands at
/// `stowed + min(p·D, d_i) − offset`. Scrolling a HALF-OPEN list
/// therefore translates the whole column rigidly: a slat still piled
/// under the cord rides the wheel exactly as far as one that has landed.
///
/// The negative control is the other composition, `min(p·D, d_i −
/// offset)`. Under it a landed slat would still stand at `d_i − offset`
/// — so every stage of this file, which draws at `p = 1.0` where every
/// slat has landed, would pass unchanged — while a PILED one stayed at
/// `p·D`, unmoved by the wheel, and the cord would run short by the
/// offset so the list jammed before its end. Nothing on either side of
/// the merge that produced this law measured it: the scroll stages are
/// all at rest, and the blind file's phase stages are a list short
/// enough never to scroll. This stage is the seam between them.
fn the_cord_and_the_offset_compose(fonts: &mut FontSystem) {
    let (pitch, content, body) = frame(LONG);
    let horizon = ANCHOR.bottom();
    let stowed = horizon - ITEM_H;
    let ph = ScrollPhysics::from_theme();
    // The whole cord: `d_(n-1)`, and `d_i = item_h + gap + pitch·i` is
    // `pitch·(i+1)`, so the run is `pitch·n`.
    let total = pitch * LONG as f32;
    assert!(content > body + 0.5, "the long list stopped scrolling — this stage is vacuous");

    // A payout that leaves the column MID-UNFOLD with both phases on
    // screen: past a notch and two pitches, so landed slats survive the
    // scroll above the horizon, and no further than the frame, so what
    // this stage measures is the law and not the frame's cutting.
    let notch = ph.wheel_px;
    let payout = (ITEM_H + notch + 2.0 * pitch).max(body * 0.5).min(body);
    assert!(
        payout >= ITEM_H + notch + 2.0 * pitch,
        "the master's frame ({body} px) is too short to hold a notch ({notch} px) and \
         two pitches — this stage cannot see both phases at once"
    );
    let p = payout / total;

    let (_, still) = shoot_at(fonts, LONG, p, &mut ScrollView::new(), None);
    let mut sv = ScrollView::new();
    sv.wheel(1.0, &ph, 0.0);
    let (_, moved) = shoot_at(fonts, LONG, p, &mut sv, None);
    let offset = sv.offset();
    assert!(offset > 1.0, "the wheel did not move the half-open list at all");

    let (mut landed, mut piled) = (0, 0);
    for i in 0..LONG {
        let d_i = pitch * (i + 1) as f32;
        // Where the composition puts this slat, unscrolled.
        let want = stowed + payout.min(d_i);
        if still[i].1 {
            assert!(
                close(still[i].0.y, want),
                "element {i} stands at {} mid-unfold where min(payout {payout}, d_i \
                 {d_i}) says {want}",
                still[i].0.y
            );
        }
        // …and the wheel moves it by the offset, WHATEVER phase it is
        // in. Only slats reported whole in both shots: a cut one is
        // reported at the frame's edge, which is the frame speaking and
        // not the law.
        if !still[i].1 || !moved[i].1 {
            continue;
        }
        assert!(
            close(still[i].0.y - moved[i].0.y, offset),
            "element {i} moved {} px on an offset of {offset} — a slat must ride the \
             wheel the same whether it has landed or is still under the cord",
            still[i].0.y - moved[i].0.y
        );
        if d_i <= payout {
            landed += 1;
        } else {
            piled += 1;
        }
    }
    assert!(
        landed > 0 && piled > 0,
        "the stage measured {landed} landed and {piled} piled slats — it must see \
         both phases at once or it rules the wrong composition out of nothing"
    );

    // AND THE CORD STILL REACHES. The offset is not inside the `min`,
    // so the payout is untouched by scrolling: fully open and scrolled
    // to the end, the last slat still lands on the frame's bottom edge
    // — the jam the wrong composition would cause, measured directly.
    let mut sv = ScrollView::new();
    for _ in 0..1000 {
        sv.wheel(1.0, &ph, 0.0);
    }
    let (_, at_end) = shoot_at(fonts, LONG, 1.0, &mut sv, None);
    let (last, last_full) = at_end[LONG - 1];
    assert!(
        last_full && close(last.y + last.h, horizon + body),
        "scrolled to the end of a fully open list, the last slat is {last:?} against a \
         frame ending at {} — the cord ran short by the offset",
        horizon + body
    );
}

// ---------------------------------------------------------------------

/// The negative control first: a list that fits its frame is the list
/// the three older dropdown files already measure, to the pixel.
fn a_short_list_is_untouched(fonts: &mut FontSystem) {
    let (pitch, content, body) = frame(SHORT);
    assert!(
        content < body + 0.5 && close(content, body),
        "nine elements no longer fit the master's frame — every claim in this \
         stage (and in the older dropdown files) is measuring scrolling instead"
    );
    let (dl, rects) = shoot(fonts, SHORT, &mut ScrollView::new(), None);
    let horizon = ANCHOR.bottom();
    let gap = px_of("menu.anchor_gap");
    assert_eq!(rects.len(), SHORT);
    for (i, (r, full)) in rects.iter().enumerate() {
        assert!(
            *full && close(r.h, ITEM_H) && close(r.w, ANCHOR.w),
            "element {i} of a fitting list is not whole at the anchor's width: {r:?}"
        );
        assert!(close(r.y, horizon + gap + pitch * i as f32), "element {i} moved");
    }
    // The frame's clip is the body's own height — no window-bottom clip
    // any more, and no lane taken from anybody.
    let c = own_clip(&dl);
    assert!(close(c[1], horizon) && close(c[3], body), "the clip is not the body's frame: {c:?}");
    // And no bar: nothing in the register is narrower than a slat.
    let thin = ring_fills(&dl).into_iter().find(|r| r[2] < ANCHOR.w / 2.0);
    assert_eq!(thin, None, "a list that fits drew a scrollbar");
}

/// The frame is a ceiling: at rest, nothing below it answers anything.
fn a_long_list_stops_at_its_frame(fonts: &mut FontSystem) {
    let (pitch, content, body) = frame(LONG);
    assert!(content > body + 0.5, "forty elements fit the frame — nothing to measure");
    let (dl, rects) = shoot(fonts, LONG, &mut ScrollView::new(), None);
    let horizon = ANCHOR.bottom();
    let bottom = horizon + body;
    let gap = px_of("menu.anchor_gap");
    assert_eq!(rects.len(), LONG, "the frame cost an entry: index-to-act mapping is broken");
    let c = own_clip(&dl);
    assert!(close(c[1], horizon) && close(c[3], body), "the picture's clip is not the frame: {c:?}");
    for (i, (r, full)) in rects.iter().enumerate() {
        let top = horizon + gap + pitch * i as f32;
        if top + ITEM_H <= bottom + 0.01 {
            // Wholly inside: whole, at its resting place.
            assert!(*full && close(r.y, top) && close(r.h, ITEM_H), "element {i} inside the frame: {r:?}");
        } else if top < bottom {
            // On the line: cut TO the line, and no longer whole — a
            // sliver must not claim to be the whole object.
            assert!(
                close(r.y + r.h, bottom) && r.h < ITEM_H - 0.5 && !*full,
                "element {i} straddles the frame's bottom and is reported as {r:?}"
            );
        } else {
            // Past the line: the same death the foreign-clip file
            // demands — no area, no answered centre, not whole.
            assert!(
                r.w <= 0.0 || r.h <= 0.0,
                "element {i} is wholly past the frame and still reports area {r:?}"
            );
            assert!(
                !r.contains(ANCHOR.cx(), top + ITEM_H / 2.0),
                "element {i} is invisible and still answers its resting centre — a \
                 ghost target over whatever the desktop drew there"
            );
            assert!(!*full, "an element the frame took whole claims to be all out");
        }
        assert!(
            r.h <= 0.0 || r.y + r.h <= bottom + 0.01,
            "element {i}'s hit {r:?} leaves the frame that cut the picture"
        );
    }
}

/// The wheel is the toolkit's: one notch is `scroll.wheel_px`, the end
/// clamps, and the last element lands exactly on the frame's bottom.
fn the_wheel_moves_the_column_and_reaches_the_end(fonts: &mut FontSystem) {
    let (_, content, body) = frame(LONG);
    let horizon = ANCHOR.bottom();
    let ph = ScrollPhysics::from_theme();
    assert_eq!(ph.fling_scale, 0.0, "the master's wheel is direct — this stage counts on it");

    // ONE NOTCH. An element visible before and after shifts up by
    // exactly `wheel_px` — the offset is the toolkit's number, not a
    // per-object invention. The caller-negates convention lives with
    // the CALLER (the settings window already hands its pages
    // `-notches`); by the time the state reaches this object a positive
    // notch simply means "toward the end", as it does everywhere in
    // `view::scroll`.
    let (_, at_rest) = shoot(fonts, LONG, &mut ScrollView::new(), None);
    let mut sv = ScrollView::new();
    sv.wheel(1.0, &ph, 0.0);
    let (_, nudged) = shoot(fonts, LONG, &mut sv, None);
    let probe = 5; // visible whole at offset 0 and after one notch
    assert!(at_rest[probe].1 && nudged[probe].1, "the probe element left the frame — pick another");
    assert!(
        close(at_rest[probe].0.y - nudged[probe].0.y, ph.wheel_px),
        "one notch moved element {probe} by {} px and scroll.wheel_px says {}",
        at_rest[probe].0.y - nudged[probe].0.y,
        ph.wheel_px
    );
    // ...and the first element lost ground: the column went UP.
    assert!(
        nudged[0].0.h < at_rest[0].0.h - 0.5,
        "the column did not move toward the end on a positive notch"
    );

    // THE END. However hard the wheel is spun, the offset clamps, the
    // last element stands exactly on the frame's bottom edge, whole —
    // the tail is reachable without hanging past anything.
    let mut sv = ScrollView::new();
    for _ in 0..1000 {
        sv.wheel(1.0, &ph, 0.0);
    }
    let (_, at_end) = shoot(fonts, LONG, &mut sv, None);
    assert!(close(sv.offset(), content - body), "the offset did not clamp to the content's end");
    let (last, last_full) = at_end[LONG - 1];
    assert!(
        last_full && close(last.y + last.h, horizon + body),
        "scrolled to the end, the last element is reported as {last:?} against a \
         frame ending at {}",
        horizon + body
    );
    // ...and the first is gone the way the frame kills things: no area.
    assert!(at_end[0].0.h <= 0.0, "the first element survived a scroll to the end");
    // A further notch changes nothing — the clamp is the toolkit's.
    sv.wheel(3.0, &ph, 0.0);
    let (_, still) = shoot(fonts, LONG, &mut sv, None);
    assert!(close(still[LONG - 1].0.y, at_end[LONG - 1].0.y), "the end is not the last stop");
}

/// The bar is [`scroll::scrollbar`]'s geometry painted by the toolkit,
/// and the slats give up exactly the lane `scrollbar.mode = inset` asks
/// for — the owner's bar stands BESIDE the content, never over it.
fn the_bar_is_the_toolkits_and_the_slats_make_room(fonts: &mut FontSystem) {
    let (_, content, body) = frame(LONG);
    let horizon = ANCHOR.bottom();
    let look = ScrollbarLook::from_theme();
    assert_eq!(look.mode, scroll::ScrollbarMode::Inset, "the master moved its bar mode");
    let lane = scroll::inset_w(&look);
    assert!(lane > 0.0, "an inset bar that costs no lane — nothing to measure");

    // The wheel has just moved, so the auto-hiding bar is at full
    // strength in this very frame.
    let ph = ScrollPhysics::from_theme();
    let mut sv = ScrollView::new();
    sv.wheel(2.0, &ph, 0.0);
    let (dl, rects) = shoot(fonts, LONG, &mut sv, None);

    // THE LANE. Every slat is the anchor's width minus the bar's lane —
    // compare the short list's stage, where the width is the anchor's
    // to the pixel.
    for (i, (r, _)) in rects.iter().enumerate() {
        if r.h > 0.0 {
            assert!(
                close(r.w, ANCHOR.w - lane),
                "element {i} of a scrolling list is {} px wide; the anchor minus the \
                 bar's lane is {}",
                r.w,
                ANCHOR.w - lane
            );
        }
    }

    // THE THUMB. The rectangle the toolkit answers for this offset,
    // this viewport and this content is IN the register — geometry from
    // `scroll::scrollbar`, paint from `paint::scrollbar`, nothing
    // invented in between.
    let area = Rect::new(ANCHOR.x, horizon, ANCHOR.w, body);
    let geom = scroll::scrollbar(area, &look, sv.offset(), body, content, false)
        .expect("a longer content with a real bar mode answers geometry");
    let fills = ring_fills(&dl);
    assert!(
        fills.iter().any(|r| close(r[0], geom.thumb.x)
            && close(r[1], geom.thumb.y)
            && close(r[2], geom.thumb.w)
            && close(r[3], geom.thumb.h)),
        "no RingFill in the register stands at the toolkit's thumb {:?} — the bar \
         is drawn by some other arithmetic, or not at all",
        geom.thumb
    );
    // And the thumb stands in the lane, past the slats' right edge.
    assert!(
        geom.thumb.x >= ANCHOR.x + (ANCHOR.w - lane) - 0.01,
        "the thumb stands over the slats instead of in its lane"
    );
}

/// The frame and a foreign clip cut TOGETHER, tighter edge winning —
/// the fix the foreign-clip file pinned, extended to the frame's line.
fn a_foreign_clip_and_the_frame_cut_together(fonts: &mut FontSystem) {
    let (_, _, body) = frame(LONG);
    let horizon = ANCHOR.bottom();

    // A host clip ABOVE the frame's bottom: the tighter line, so it is
    // the one every answer ends at.
    let tight = horizon + body * 0.5;
    let (_, rects) = shoot(fonts, LONG, &mut ScrollView::new(), Some(Rect::new(0.0, 0.0, W, tight)));
    for (i, (r, _)) in rects.iter().enumerate() {
        assert!(
            r.h <= 0.0 || r.y + r.h <= tight + 0.01,
            "element {i} answers past the foreign clip at {tight}: {r:?}"
        );
    }
    let cut = rects.iter().filter(|(r, _)| r.h > 0.0).last().expect("something is visible").0;
    assert!(close(cut.y + cut.h, tight), "no element was cut to the foreign line");

    // A host clip BELOW it: the frame is the tighter line and nothing
    // leaks out to the looser one.
    let loose = horizon + body + 200.0;
    let (_, rects) = shoot(fonts, LONG, &mut ScrollView::new(), Some(Rect::new(0.0, 0.0, W, loose)));
    for (i, (r, _)) in rects.iter().enumerate() {
        assert!(
            r.h <= 0.0 || r.y + r.h <= horizon + body + 0.01,
            "element {i} slipped past the frame toward a looser host clip: {r:?}"
        );
    }
}

/// The numbers are the TOKENS': a fixture that lowers the fraction
/// lowers the frame, and the px floor holds it up when the fraction
/// collapses. Zero of this arithmetic lives in Rust.
fn the_tokens_move_the_frame(fonts: &mut FontSystem) {
    let horizon = ANCHOR.bottom();
    let last_answered = |fonts: &mut FontSystem| {
        let (_, rects) = shoot(fonts, LONG, &mut ScrollView::new(), None);
        rects
            .iter()
            .filter(|(r, _)| r.h > 0.0)
            .map(|(r, _)| r.y + r.h)
            .fold(0.0f32, f32::max)
    };

    skin("[menu]\nmax_h_frac = 20%\n");
    let lowered = (H * px_of("menu.max_h_frac")).max(px_of("menu.max_h_min_px"));
    assert!(close(px_of("menu.max_h_frac"), 0.20), "the fixture's fraction did not bake");
    assert!(
        close(last_answered(fonts), horizon + lowered),
        "max_h_frac = 20% did not move the frame's bottom to {}",
        horizon + lowered
    );

    // The floor. A fraction the viewport makes tiny is held up by the
    // device-px companion — the 3.2 rule, same as every other _min_px.
    skin("[menu]\nmax_h_frac = 2%\nmax_h_min_px = 300px\n");
    assert!(H * 0.02 < 300.0, "the floor is under the fraction — nothing to see");
    assert!(
        close(last_answered(fonts), horizon + 300.0),
        "max_h_min_px did not hold the collapsing frame at 300 px"
    );

    master();
}

/// The chain holds what the frame shows: every element reported whole
/// is registered, and NOTHING else is — an element past the line joins
/// no Tab order, exactly as a row scrolled off a settings page does not.
fn elements_outside_the_frame_join_no_chain(fonts: &mut FontSystem) {
    let names = names(LONG);
    let base = FocusId::of("dropdown-scroll-test");
    let mut fc = FocusCtl::new();
    fc.begin_frame();
    let mut dl = DrawList::new();
    let mut sv = ScrollView::new();
    let rects = {
        let mut ctx = Ctx {
            access: None,
            dl: &mut dl,
            fonts,
            w: W,
            h: H,
            t: 0.0,
            mouse: nacelle::pointer::Pointer::new(AWAY.0, AWAY.1),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: Some(&mut fc),
            tips: None,
        };
        dropdown::accordion(
            &mut ctx,
            ANCHOR,
            ITEM_H,
            &names,
            1.0,
            &AccordionStyle { focus: Some(base), ..Default::default() },
            &mut sv,
        )
    };
    // Close the frame, so the chain built while drawing is the one the
    // registry answers for.
    fc.begin_frame();
    let whole = rects.iter().filter(|(_, full)| *full).count();
    assert!(whole > 0 && whole < LONG, "the frame shows all or nothing — the claim is vacuous");
    for (i, (_, full)) in rects.iter().enumerate() {
        assert_eq!(
            fc.rect_of(base.item(i)).is_some(),
            *full,
            "element {i}: reported {} and {} the chain",
            if *full { "whole" } else { "cut" },
            if fc.rect_of(base.item(i)).is_some() { "joined" } else { "missed" }
        );
    }
}
