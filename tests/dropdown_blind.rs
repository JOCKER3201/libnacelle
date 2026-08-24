//! A drop-down is a VENETIAN BLIND: an anchor, and N separate elements
//! that come out from under it.
//!
//! The morning's design put the list on a surface level of its own —
//! `[elev.popover]`, one bed, one ring, the rows kept inside it by
//! `[menu].pad` — and the owner looked at it and asked for the opposite.
//! The frame around the WHOLE is gone. What is left is the anchor plus a
//! column of complete objects, and the way they arrive is a blind: at
//! rest they are stowed under the anchor in one pile, and pulling the
//! cord slides each of them out by a distance that grows with its index,
//! so the slat on top of the pile ends up at the bottom of the blind.
//!
//! Everything below is measured out of a recording [`DrawList`] — the
//! commands the object issued and the geometry they carry — not out of a
//! claim about them:
//!
//! * NO BOX. Every shape in the picture is one element's own rectangle,
//!   and there is nothing drawn around the group;
//! * ONE GAP. `[menu].anchor_gap` stands between the anchor and the
//!   first element AND between every pair of elements — one number,
//!   measured in every seam;
//! * THE BLIND. Element `i`'s RESTING travel grows linearly with `i`; at
//!   `p→0` the whole column is stowed at one place under the anchor; the
//!   last element travels furthest and is drawn over its neighbours
//!   while the pile is still a pile;
//! * TWO PHASES (the owner's ask, 2026-08-16: out from under the anchor
//!   FIRST, then unfold). The cord pays out at one speed: while
//!   `p·D < d_0` the column slides as one pile (phase A); past it the
//!   elements land one after another from the top, each at its own
//!   `d_i`, while the rest ride on (phase B);
//! * FROM UNDER. The list clips to the anchor's bottom edge, so an
//!   element on its way out never crosses the anchor's face;
//! * WHAT IS RETURNED IS WHAT IS DRAWN. The caller hit-tests the
//!   rectangles this function answers with, so a half-out element is
//!   reported where it can be SEEN and not where it is going.
//!
//! Every claim carries its negative control: the counter-picture that
//! the same probe rejects, or the fixture theme that moves the number.
//!
//! One test in a binary of its own: the resolved theme is process-wide
//! (§7.1 hands every draw path the same `&'static ResolvedTheme`), so a
//! test that swaps themes must not run beside one that reads them.

use nacelle::draw::{Corner, CornerStyle, DrawCmd, DrawList};
use nacelle::font::FontSystem;
use nacelle::object::dropdown::{self, AccordionStyle};
use nacelle::theme::{self, Color};
use nacelle::view::ScrollView;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const ITEM_H: f32 = 30.0;
/// The anchor the blind hangs from — wide enough that `menu.min_w`
/// cannot move it, and clear of the screen edges.
const ANCHOR: Rect = Rect { x: 200.0, y: 300.0, w: 400.0, h: 36.0 };
/// Nine, because nine is what the owner's screenshot of the THEMES list
/// held — and because a claim about "grows with the index" wants more
/// than two indices to grow over.
const NAMES: [&str; 9] = [
    "DEFAULT", "COCKPIT", "INSTRUMENT", "AURORA", "GRAPHITE", "SIGNAL", "VELLUM", "NOCTURNE",
    "EMBER",
];
/// Off screen: nothing hovers unless a case says so.
const AWAY: (f32, f32) = (-1.0, -1.0);

fn names() -> Vec<String> {
    NAMES.iter().map(|s| s.to_string()).collect()
}

/// Loads a fixture theme whose base is the master, so every token but
/// the ones in `body` is the master's own.
fn skin(body: &str) {
    let path = std::env::temp_dir().join(format!("nacelle-blind-{}.theme", std::process::id()));
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

/// One drawing of the blind: the register it wrote, and the rectangles
/// it handed back for the caller to aim at.
fn shoot(fonts: &mut FontSystem, p: f32, mouse: (f32, f32)) -> (DrawList, Vec<(Rect, bool)>) {
    let names = names();
    let mut dl = DrawList::recording();
    let rects = {
        let mut ctx = Ctx {
            access: None,
            dl: &mut dl,
            fonts,
            w: W,
            h: H,
            t: 0.0,
            mouse: nacelle::pointer::Pointer::new(mouse.0, mouse.1),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        };
        // A fresh, untouched offset: nine elements fit their frame, so
        // the scroll is a passenger in every stage of this file.
        dropdown::accordion(
            &mut ctx,
            ANCHOR,
            ITEM_H,
            &names,
            p,
            &AccordionStyle::default(),
            &mut ScrollView::new(),
        )
    };
    (dl, rects)
}

/// Every rectangle any SHAPE command in the register covers, in the
/// order the commands were issued. Text is not a shape and clips are not
/// drawn, so neither is here; everything else that puts colour on the
/// screen is, which is what makes "no box" a claim about the picture
/// rather than about the two primitives this file happens to expect.
fn shapes(dl: &DrawList) -> Vec<[f32; 4]> {
    dl.cmds()
        .iter()
        .filter_map(|c| match c {
            DrawCmd::RingFill { r, .. }
            | DrawCmd::Ring { r, .. }
            | DrawCmd::Rect { r, .. }
            | DrawCmd::RectOutline { r, .. }
            | DrawCmd::ChamferFill { r, .. }
            | DrawCmd::ChamferFrame { r, .. }
            | DrawCmd::RectGrad { r, .. }
            | DrawCmd::Blur { r, .. }
            | DrawCmd::GlowRing { r, .. } => Some(*r),
            _ => None,
        })
        .collect()
}

/// The shapes that are NOT one element's own rectangle: anything taller
/// than a single element, which is what a box around the group is and
/// what an element can never be.
///
/// The tolerance is a hair rather than nothing because a ring is stroked
/// on the rectangle it is given, and the register prints the rectangle.
fn boxes(rects: &[[f32; 4]]) -> Vec<[f32; 4]> {
    rects.iter().copied().filter(|r| r[3] > ITEM_H + 0.01).collect()
}

/// The elements' rectangles AS DRAWN — before the scissor takes the part
/// of them that is still under the anchor. Read off the plate, which is
/// the first of the three shapes each element puts down.
fn drawn(dl: &DrawList) -> Vec<[f32; 4]> {
    let all = shapes(dl);
    assert_eq!(
        all.len() % NAMES.len(),
        0,
        "the elements did not draw the same number of shapes each: {} shapes over {} elements",
        all.len(),
        NAMES.len()
    );
    let per = all.len() / NAMES.len();
    all.chunks(per).map(|c| c[0]).collect()
}

fn clips(dl: &DrawList) -> Vec<[f32; 4]> {
    dl.cmds()
        .iter()
        .filter_map(|c| match c {
            DrawCmd::ClipPush { r } => Some(*r),
            _ => None,
        })
        .collect()
}

// =====================================================================

/// One test in the binary, and every stage inside it: `skin` swaps the
/// process-wide resolved theme, so two stages running in parallel
/// threads would each be measuring the other's fixture.
#[test]
fn a_drop_down_is_a_venetian_blind() {
    master();
    let mut fonts = FontSystem::new();
    no_box_at_all(&mut fonts);
    one_gap_everywhere(&mut fonts);
    out_of_one_pile(&mut fonts);
    from_under_the_anchor(&mut fonts);
    what_is_returned_is_what_is_drawn(&mut fonts);
}

// --------------------------------------------------------------------

/// The frame around the WHOLE is gone: no shape in the picture is
/// anything but one element's own rectangle.
fn no_box_at_all(fonts: &mut FontSystem) {
    let (open, _) = shoot(fonts, 1.0, AWAY);
    let all = shapes(&open);
    assert!(!all.is_empty(), "the list drew nothing — every claim below is vacuous");
    assert_eq!(
        boxes(&all),
        Vec::<[f32; 4]>::new(),
        "the list drew a shape taller than one element: that is a box around the \
         group, and the owner asked for the box to go"
    );
    // Every shape is EXACTLY an element's height — not merely "not taller".
    // A wash the height of one element but the width of the whole column
    // would pass the line above and still be a bed under the list.
    for r in &all {
        assert!(
            (r[3] - ITEM_H).abs() < 0.01,
            "a shape {r:?} is not one element's own rectangle"
        );
    }

    // The NEGATIVE CONTROL for that emptiness. The probe above has to be
    // able to SEE a box; here is the box the morning's design drew — one
    // shaped fill over the whole column, `[elev.popover]`'s bed — laid
    // into a register of its own and put through the same predicate.
    let mut counter = DrawList::recording();
    let column = Rect::new(
        ANCHOR.x,
        ANCHOR.bottom(),
        ANCHOR.w,
        ITEM_H * NAMES.len() as f32 + 2.0 * px_of("menu.pad"),
    );
    counter.ring_fill(
        column,
        &[Corner { style: CornerStyle::Round, size: 4.0 }; 4],
        8,
        Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 },
    );
    assert_eq!(
        boxes(&shapes(&counter)).len(),
        1,
        "the probe cannot see a box at all, so its silence above says nothing"
    );

    // And the elements really were drawn, all nine of them, each with
    // the same handful of shapes. `drawn` asserts the divisibility; this
    // states the count the rest of the file counts on.
    assert_eq!(drawn(&open).len(), NAMES.len());
}

/// `[menu].anchor_gap` under the anchor and between every pair, and no
/// second spacing token anywhere.
fn one_gap_everywhere(fonts: &mut FontSystem) {
    let gap = px_of("menu.anchor_gap");
    assert!(gap > 0.0, "the master's menu.anchor_gap is zero — no seam can be measured");

    let (open, _) = shoot(fonts, 1.0, AWAY);
    let rows = drawn(&open);
    // Every seam in the picture, the first one included: the anchor's
    // bottom edge is the top of the list, so the list's first seam is
    // the one under the anchor.
    let mut seams = vec![rows[0][1] - ANCHOR.bottom()];
    for pair in rows.windows(2) {
        seams.push(pair[1][1] - (pair[0][1] + pair[0][3]));
    }
    assert_eq!(seams.len(), NAMES.len(), "nine elements make nine seams with the anchor above");
    for (i, s) in seams.iter().enumerate() {
        assert!(
            (s - gap).abs() < 0.01,
            "seam {i} is {s} px and menu.anchor_gap says {gap} — the owner asked for \
             every row of the list to be spaced like the first, which is one number"
        );
    }

    // NEGATIVE CONTROL 1 · the number is the TOKEN's. A fixture that
    // moves `anchor_gap` moves every seam, by the amount it moved.
    skin("[menu]\nanchor_gap = 4u\n");
    let wide = px_of("menu.anchor_gap");
    assert!((wide - gap).abs() > 0.5, "the fixture's own gap did not bake");
    let (open, _) = shoot(fonts, 1.0, AWAY);
    let rows = drawn(&open);
    assert!((rows[0][1] - ANCHOR.bottom() - wide).abs() < 0.01, "the seam under the anchor is fixed");
    for pair in rows.windows(2) {
        assert!(
            (pair[1][1] - (pair[0][1] + pair[0][3]) - wide).abs() < 0.01,
            "a seam between two elements is fixed"
        );
    }

    // NEGATIVE CONTROL 2 · and it is NOT `[list].gap`. That token is the
    // furniture of a list drawn as one body; this one is not drawn as one
    // body and must not answer it, or the seam under the anchor and the
    // seam between two elements would be two numbers again.
    skin("[list]\ngap = 9u\nrule = @stroke.hair\nrule_every = 1\n");
    let (open, _) = shoot(fonts, 1.0, AWAY);
    let rows = drawn(&open);
    let plain = px_of("menu.anchor_gap");
    for pair in rows.windows(2) {
        assert!(
            (pair[1][1] - (pair[0][1] + pair[0][3]) - plain).abs() < 0.01,
            "[list].gap moved the seam between two elements — the list is reading a \
             second spacing token and the owner's one number is two again"
        );
    }
    assert_eq!(
        open.cmds().iter().filter(|c| matches!(c, DrawCmd::Line { .. })).count(),
        0,
        "[list].rule drew a separator: a column of complete objects has nothing \
         between them to rule"
    );

    master();
}

/// Stowed in one pile at `p→0`, and a travel that grows with the index.
fn out_of_one_pile(fonts: &mut FontSystem) {
    let gap = px_of("menu.anchor_gap");
    let pitch = ITEM_H + gap;

    // Stowed. At a hair off zero the whole column is under the anchor —
    // one pile, every element within a hair's travel of the same place,
    // and none of it standing anywhere near where it will end up.
    let (stowing, _) = shoot(fonts, 0.001, AWAY);
    let pile = drawn(&stowing);
    let stowed = ANCHOR.bottom() - ITEM_H;
    for (i, r) in pile.iter().enumerate() {
        assert!(
            r[1] >= stowed - 0.01 && r[1] < ANCHOR.bottom(),
            "element {i} at p→0 sits at {} and the anchor's underside runs {stowed}..{}",
            r[1],
            ANCHOR.bottom()
        );
    }

    // Travel. Element i's RESTING distance is `item_h + gap + pitch·i`:
    // linear in i, so the last goes furthest — which is what makes the
    // top slat of the pile the bottom row of the blind.
    let (open, _) = shoot(fonts, 1.0, AWAY);
    let rest = drawn(&open);
    let travel: Vec<f32> = rest.iter().map(|r| r[1] - stowed).collect();
    for (i, t) in travel.iter().enumerate() {
        let want = ITEM_H + gap + pitch * i as f32;
        assert!(
            (t - want).abs() < 0.01,
            "element {i} travelled {t} px where a blind gives it {want}"
        );
    }
    // Strictly increasing, by exactly one pitch each time. The negative
    // control AT REST: the owner sent back the version that arrived as
    // one translated column and STAYED one — under the 2026-08-16 ask
    // the one-body ride is legal ONLY as phase A, on the way, and a
    // column whose differences were still zero here would be a blind
    // that never unfolded at all.
    for pair in travel.windows(2) {
        assert!(
            (pair[1] - pair[0] - pitch).abs() < 0.01,
            "two neighbours travelled {} apart and one pitch is {pitch} — a column \
             that RESTS as one body is the accordion the owner sent back, not the \
             blind",
            pair[1] - pair[0]
        );
    }

    // PHASE A: while the cord's payout `p·D` is short of the first
    // element's own distance, every element has travelled exactly the
    // payout — the column is one pile on its way out from under the
    // anchor, which is the owner's "come out first, then unfold".
    let total = pitch * NAMES.len() as f32; // d_(n-1) = item_h + gap + pitch·(n-1)
    let p_a = (ITEM_H * 0.5) / total; // payout item_h/2: under the anchor still
    let (sliding, _) = shoot(fonts, p_a, AWAY);
    let payout = p_a * total;
    for (i, r) in drawn(&sliding).iter().enumerate() {
        assert!(
            (r[1] - stowed - payout).abs() < 0.01,
            "element {i} travelled {} in phase A where the pile's payout is {payout} — \
             the stack is spreading before it is out",
            r[1] - stowed
        );
    }

    // PHASE B: with the cord paid out past the first two distances, the
    // landed elements stand at their OWN `d_i` and everything still
    // flying is one pile at the payout. The blind fills from the top.
    let p_b = (2.5 * pitch) / total;
    let (landing, _) = shoot(fonts, p_b, AWAY);
    let payout = p_b * total;
    for (i, r) in drawn(&landing).iter().enumerate() {
        let d_i = ITEM_H + gap + pitch * i as f32;
        let want = d_i.min(payout);
        assert!(
            (r[1] - stowed - want).abs() < 0.01,
            "element {i} travelled {} mid-unfold where min(d_i {d_i}, payout {payout}) \
             says {want}",
            r[1] - stowed
        );
    }

    // The order of the NAMES is untouched by any of it: DEFAULT is the
    // first element at rest, exactly as it is today.
    let labels: Vec<String> = open
        .cmds()
        .iter()
        .filter_map(|c| match c {
            DrawCmd::Text { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(labels, NAMES.to_vec(), "the blind reordered the names");

    // Z ORDER. While the pile is a pile the element that ENDS UP
    // furthest is the one on top of it, which in a painter's list means
    // drawn LAST. The register is in that order, so the y of the first
    // shape of each element never falls down the register. In phase A
    // the pile rides as ONE body (the owner's 2026-08-16 ask), so at a
    // hair off zero the tops COINCIDE — equality is the pile being a
    // pile, and only an element drawn ABOVE its later neighbour would
    // put the wrong slat on top.
    for (i, pair) in pile.windows(2).enumerate() {
        assert!(
            pair[1][1] >= pair[0][1],
            "element {} is drawn after element {i} but sits above it — the slat on \
             top of the pile is not the one that comes out furthest",
            i + 1
        );
    }
}

/// The clip that keeps an emerging element off the anchor's face.
fn from_under_the_anchor(fonts: &mut FontSystem) {
    let (mid, _) = shoot(fonts, 0.05, AWAY);
    let horizon = ANCHOR.bottom();
    let cs = clips(&mid);
    assert_eq!(cs.len(), 1, "the list pushed {} clips; it needs exactly one", cs.len());
    assert_eq!(cs[0][1], horizon, "the clip's top edge is not the anchor's bottom edge");
    assert!(cs[0][0] <= ANCHOR.x && cs[0][0] + cs[0][2] >= ANCHOR.right(), "the clip is too narrow");
    assert_eq!(
        mid.cmds().first(),
        Some(&DrawCmd::ClipPush { r: cs[0] }),
        "the clip is not the first thing the list does, so something was drawn outside it"
    );
    assert_eq!(mid.cmds().last(), Some(&DrawCmd::ClipPop), "the list left its clip on the stack");

    // The clip is LOAD-BEARING, and this is the proof: at this `p` every
    // element's own rectangle is still above the horizon — that is, over
    // the anchor's face — and only the scissor keeps it off. Take the
    // clip away and the picture is the elements sliding down the
    // anchor's front, which is the thing the owner said must not happen.
    for (i, r) in drawn(&mid).iter().enumerate() {
        assert!(
            r[1] < horizon,
            "element {i} is drawn at {} which is already below the anchor — this \
             stage is not measuring a clipped element at all",
            r[1]
        );
    }
}

/// The caller aims the mouse at what it was handed, so what it was
/// handed has to be what is on the screen.
fn what_is_returned_is_what_is_drawn(fonts: &mut FontSystem) {
    let horizon = ANCHOR.bottom();

    for step in 1..=20 {
        let p = step as f32 / 20.0;
        let (dl, rects) = shoot(fonts, p, AWAY);
        let rows = drawn(&dl);
        assert_eq!(rects.len(), rows.len(), "p={p}: a rectangle was returned for no element");
        for (i, (got, full)) in rects.iter().enumerate() {
            let r = rows[i];
            // The visible part: the drawn rectangle, cut at the horizon
            // by the same scissor the picture was cut by.
            let top = r[1].max(horizon);
            let seen = (r[1] + r[3] - top).max(0.0);
            assert!(
                (got.x - r[0]).abs() < 0.01
                    && (got.w - r[2]).abs() < 0.01
                    && (got.y - top).abs() < 0.01
                    && (got.h - seen).abs() < 0.01,
                "p={p}: element {i} is drawn as {r:?}, shows as ({top}, {seen}) and is \
                 reported as ({}, {}) — the caller would aim the mouse at a place the \
                 element is not",
                got.y,
                got.h
            );
            assert_eq!(*full, seen >= ITEM_H - 0.5, "p={p}: element {i}'s 'all out' flag lies");
        }
    }

    // NEGATIVE CONTROL. Halfway through the unfold the first element is
    // NOT where it will end up, so "returned = drawn" is a claim with a
    // wrong answer available: a version that reported resting places
    // would return the same rectangles at every `p`.
    let (_, half) = shoot(fonts, 0.5, AWAY);
    let (_, done) = shoot(fonts, 1.0, AWAY);
    assert_ne!(
        half.iter().map(|(r, _)| r.y).collect::<Vec<_>>(),
        done.iter().map(|(r, _)| r.y).collect::<Vec<_>>(),
        "the list reports the same rectangles half-open as open — it is answering \
         with destinations, not with what is on the screen"
    );
    // ...and a moving element is reported SHORTER than a resting one,
    // which is the part that is out from under the anchor. The probe
    // sits in PHASE A — by `p = 0.5` the cord has already landed the
    // first element at its own distance, so the claim "part-way out"
    // has to be made while the pile is still emerging.
    let gap = px_of("menu.anchor_gap");
    let total = (ITEM_H + gap) * NAMES.len() as f32;
    let (_, early) = shoot(fonts, (ITEM_H * 0.5) / total, AWAY);
    assert!(!early[0].1 && early[0].0.h < ITEM_H, "the first element is not part-way out");

    // A closed blind is nothing at all: no rectangle to aim at, no
    // command in the register.
    let (shut, none) = shoot(fonts, 0.0, AWAY);
    assert!(none.is_empty(), "a closed list handed back {} rectangles", none.len());
    assert_eq!(shut.cmds().len(), 0, "a closed list drew {} commands", shut.cmds().len());
}
