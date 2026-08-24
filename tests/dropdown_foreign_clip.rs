//! An open list inside SOMEBODY ELSE'S clip: the rectangles it hands
//! back are cut by that clip, exactly as the picture is.
//!
//! The defect this file pins down: [`dropdown::accordion`] used to cut
//! its answers with its own horizon alone (`top = y.max(horizon)`),
//! while the PICTURE was cut by the whole clip stack — `push_clip`
//! intersects with whatever is already clipping. An element pushed out
//! of an enclosing scrolled body's box was therefore invisible and
//! still reported clickable: a ghost target OVER whatever really stood
//! there, which is exactly what a list unfolding across a settings
//! column must not leave behind.
//!
//! What is asserted:
//!
//! * an element wholly under a foreign clip is reported with NO AREA —
//!   the entry stays (the caller maps index to act) but nothing of it
//!   answers a point;
//! * an element the clip cuts at its edge is cut in the HIT too, to the
//!   clip's own line;
//! * an element the clip leaves whole is answered exactly as a free
//!   list answers it — and only such an element says `full`;
//! * the clip's OWN stack is left as the caller built it: the list
//!   pushed and popped its horizon inside, and touched nothing else.
//!
//! The free list is the negative control throughout: the same anchor,
//! the same names, no clip — full-height rectangles at the very places
//! the clipped call reports empty, so a version that answers resting
//! places would fail the clipped stages against a probe that has seen
//! the difference.
//!
//! One test in the binary: the resolved theme is process-wide, so this
//! file shares a process with no other theme reader.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::object::dropdown::{self, AccordionStyle};
use nacelle::theme;
use nacelle::view::ScrollView;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const ITEM_H: f32 = 30.0;
/// The anchor the blind hangs from — wide enough that `menu.min_w`
/// cannot move it, and clear of the screen edges.
const ANCHOR: Rect = Rect { x: 200.0, y: 300.0, w: 400.0, h: 36.0 };
/// Off screen: nothing hovers unless a case says so.
const AWAY: (f32, f32) = (-1.0, -1.0);
const NAMES: [&str; 9] = [
    "DEFAULT", "COCKPIT", "INSTRUMENT", "AURORA", "GRAPHITE", "SIGNAL", "VELLUM", "NOCTURNE",
    "EMBER",
];

fn names() -> Vec<String> {
    NAMES.iter().map(|s| s.to_string()).collect()
}

fn master() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
}

/// One drawing of the fully open blind under `clip` (`None` = free),
/// answering the rectangles the caller would hit-test.
fn shoot(fonts: &mut FontSystem, clip: Option<Rect>) -> Vec<(Rect, bool)> {
    let names = names();
    let mut dl = DrawList::new();
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
        // A fresh, untouched offset: nine elements fit their frame, so
        // the frame's own cut never fires here and every cut this file
        // measures is the foreign clip's.
        dropdown::accordion(
            &mut ctx,
            ANCHOR,
            ITEM_H,
            &names,
            1.0,
            &AccordionStyle::default(),
            &mut ScrollView::new(),
        )
    };
    // The list is a guest under this clip: it pushed its horizon and
    // popped it, and the stack it was called under still stands.
    match clip {
        Some(c) => assert_eq!(
            dl.clip_stack(),
            vec![[c.x, c.y, c.w, c.h]],
            "the list unbalanced its host's clip stack"
        ),
        None => assert_eq!(dl.clip_stack(), Vec::<[f32; 4]>::new(), "the list left a clip behind"),
    }
    rects
}

/// One test in the binary, stages in order: every clipped stage reads
/// the free list first, so its claims are against a measured control
/// and not against arithmetic repeated from the object.
#[test]
fn a_foreign_clip_cuts_the_hits_as_it_cuts_the_picture() {
    master();
    let mut fonts = FontSystem::new();
    let free = shoot(&mut fonts, None);
    the_free_list_is_the_control(&free);
    a_body_box_cuts_the_hits(&mut fonts, &free);
    a_side_clip_cuts_the_width(&mut fonts, &free);
}

/// The probe can see fullness: a list under no foreign clip answers
/// every element whole. This is the control the clipped stages cut.
fn the_free_list_is_the_control(free: &[(Rect, bool)]) {
    assert_eq!(free.len(), NAMES.len(), "the free list dropped an element");
    for (i, (r, full)) in free.iter().enumerate() {
        assert!(
            (r.h - ITEM_H).abs() < 0.01 && *full,
            "free element {i} is not whole — every claim below is vacuous"
        );
        // …and each element really answers its own centre, which is the
        // point the clipped stages will show answered by NOTHING.
        assert!(r.contains(r.cx(), r.y + r.h / 2.0), "free element {i} does not answer its centre");
    }
}

/// A clip that ends mid-list — an enclosing scrolled body whose box the
/// blind has outgrown. Above the line: untouched. On the line: cut to
/// it. Below the line: no area, no click.
fn a_body_box_cuts_the_hits(fonts: &mut FontSystem, free: &[(Rect, bool)]) {
    // The body's bottom edge runs through the middle of element 2.
    let cut = free[2].0.y + ITEM_H / 2.0;
    let body = Rect::new(0.0, 0.0, W, cut);
    let rects = shoot(fonts, Some(body));
    assert_eq!(rects.len(), NAMES.len(), "the clip cost an entry: index-to-act mapping is broken");

    // Whole above the line: exactly the free list's answer.
    for i in 0..2 {
        let (got, full) = rects[i];
        let (want, _) = free[i];
        assert!(
            (got.x - want.x).abs() < 0.01
                && (got.y - want.y).abs() < 0.01
                && (got.w - want.w).abs() < 0.01
                && (got.h - want.h).abs() < 0.01
                && full,
            "element {i} sits wholly inside the body's box and was answered differently \
             from the free list: {got:?} against {want:?}"
        );
    }

    // On the line: cut in the hit to the very line the picture is cut
    // at, and no longer `full` — a sliver must not join the focus chain
    // as if it were the whole object.
    let (edge, edge_full) = rects[2];
    assert!(
        (edge.y - free[2].0.y).abs() < 0.01 && (edge.y + edge.h - cut).abs() < 0.01,
        "the element on the body's edge is drawn to {cut} and reported to {}",
        edge.y + edge.h
    );
    assert!(edge.h < ITEM_H - 0.5, "the edge element reports more than the scissor left of it");
    assert!(!edge_full, "a cut element claims to be all out");

    // Below the line: the scissor took the whole of it, so there is
    // nothing to press — no area, and the centre of where the element
    // WOULD stand answers nothing. The free control proved that very
    // point was answered when no clip stood there, so a version
    // reporting horizon-only rects fails here, not vacuously.
    for i in 3..NAMES.len() {
        let (ghost, full) = rects[i];
        let (was, _) = free[i];
        assert!(
            ghost.w <= 0.0 || ghost.h <= 0.0,
            "element {i} is wholly under the body's clip and still reports area {ghost:?}"
        );
        assert!(
            !ghost.contains(was.cx(), was.y + was.h / 2.0),
            "element {i} is invisible and still answers its old centre — a ghost target \
             over whatever really stands there"
        );
        assert!(!full, "an element the clip took whole claims to be all out");
    }

    // And nothing the list answered leaves the body's box.
    for (i, (r, _)) in rects.iter().enumerate() {
        if r.w > 0.0 && r.h > 0.0 {
            assert!(
                r.y >= body.y - 0.01 && r.y + r.h <= body.bottom() + 0.01,
                "element {i}'s hit {r:?} stands outside the clip that cut the picture"
            );
        }
    }
}

/// The cut is the stack's, not a bottom-edge special case: a clip that
/// takes the right half of every element narrows every answer to its
/// own edge, and nothing under it says `full`.
fn a_side_clip_cuts_the_width(fonts: &mut FontSystem, free: &[(Rect, bool)]) {
    let half = free[0].0.x + free[0].0.w / 2.0;
    let side = Rect::new(0.0, 0.0, half, H);
    let rects = shoot(fonts, Some(side));
    for (i, (r, full)) in rects.iter().enumerate() {
        assert!(
            (r.x + r.w - half).abs() < 0.01,
            "element {i} is drawn to the clip's edge at {half} and reported to {}",
            r.x + r.w
        );
        assert!((r.h - free[i].0.h).abs() < 0.01, "a side clip moved element {i}'s height");
        assert!(!full, "element {i} is half taken and still claims to be whole");
    }
}
