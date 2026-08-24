//! What a bridge reads off an open blind, per element: EXPANDED,
//! SELECTED and the element's place among its siblings.
//!
//! The foundation pass gave every element `AccessInfo::new(Role::ComboBox,
//! name)` and nothing past the role and the name — a screen reader could
//! say WHAT an element was, not whether the set it belongs to is open,
//! which one is already chosen, or where it sits among the rest. This
//! file pins down the three additions:
//!
//! * EXPANDED is not a per-element fact, it is the same fact repeated at
//!   every element that reaches the register call — `accordion`'s own
//!   `at_rest` (`p >= 1.0`, "the blind has stopped moving, this is a list
//!   and not still an animation") gates that call already, so every
//!   element a bridge ever sees is honestly reporting an open set;
//! * SELECTED tracks [`AccordionStyle::current`] — the same index
//!   `ButtonState.selected` draws the `selected` rung from — and moves
//!   with the INDEX, not with a position, exactly as the ladder does;
//! * the index is the element's 1-based place among `names.len()`
//!   siblings (`AccessInfo::with_index`), so "item 2 of 7" is arithmetic
//!   a bridge does not have to reconstruct from draw order.
//!
//! Read through a real [`FocusCtl`], the same registry a bridge will
//! eventually read `entries()` off, and not by reaching into the
//! object's internals — the claim is "what the chain says", not "what
//! the function happens to compute".
//!
//! One test in the binary: the resolved theme is process-wide, so this
//! file shares a process with no other theme reader.

use nacelle::access::{Role, States};
use nacelle::draw::DrawList;
use nacelle::focus::{FocusCtl, FocusId};
use nacelle::font::FontSystem;
use nacelle::object::dropdown::{self, AccordionStyle};
use nacelle::theme;
use nacelle::view::ScrollView;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const ITEM_H: f32 = 30.0;
/// Wide enough that `menu.min_w` cannot move it, clear of the edges.
const ANCHOR: Rect = Rect { x: 200.0, y: 300.0, w: 400.0, h: 36.0 };
/// Off screen: nothing hovers unless a case says so.
const AWAY: (f32, f32) = (-1.0, -1.0);
const NAMES: [&str; 4] = ["ALPHA", "BETA", "GAMMA", "DELTA"];

fn names() -> Vec<String> {
    NAMES.iter().map(|s| s.to_string()).collect()
}

fn master() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
}

/// One fully-open, at-rest blind, drawn through a real [`FocusCtl`] so
/// every stage below reads the same `entries()` a bridge will. Returns
/// the chain's own report, in registration order — which is index order,
/// since the elements register `0..names.len()` as they are drawn.
fn open_and_read(fc: &mut FocusCtl, fonts: &mut FontSystem, current: Option<usize>) -> Vec<(Rect, nacelle::access::AccessInfo)> {
    let names = names();
    let base = FocusId::of("dropdown-access-test");
    fc.begin_frame();
    let mut dl = DrawList::new();
    let mut sv = ScrollView::new();
    {
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
            focus: Some(fc),
            tips: None,
        };
        dropdown::accordion(
            &mut ctx,
            ANCHOR,
            ITEM_H,
            &names,
            1.0,
            &AccordionStyle { focus: Some(base), current, ..Default::default() },
            &mut sv,
        )
    };
    // Close the frame: the chain built while drawing becomes the one
    // `entries()` answers for.
    fc.begin_frame();
    fc.entries().map(|(_, r, info)| (r, info.clone())).collect()
}

/// One test in the binary, stages in order.
#[test]
fn an_open_elements_report_states_and_index() {
    master();
    let mut fonts = FontSystem::new();
    every_element_reports_expanded_and_its_place(&mut fonts);
    only_the_current_element_is_selected(&mut fonts);
    the_selected_mark_travels_with_the_index_not_a_position(&mut fonts);
}

/// Every open element: still `Role::ComboBox` (untouched by this file),
/// carrying `EXPANDED`, and its own `(position, count)` among
/// `NAMES.len()` siblings — position counted from 1, so a bridge can say
/// "item 2 of 4" without doing the off-by-one arithmetic itself.
fn every_element_reports_expanded_and_its_place(fonts: &mut FontSystem) {
    let mut fc = FocusCtl::new();
    let got = open_and_read(&mut fc, fonts, None);
    assert_eq!(got.len(), NAMES.len(), "not every element joined the chain");
    for (i, (_, info)) in got.iter().enumerate() {
        assert_eq!(info.role, Role::ComboBox, "element {i} changed role — out of scope for this fix");
        assert_eq!(info.name, NAMES[i], "element {i} reported the wrong label");
        assert!(
            info.states.contains(States::EXPANDED),
            "element {i} did not report EXPANDED although the blind that drew it is at rest"
        );
        assert_eq!(
            info.index,
            Some((i as u32 + 1, NAMES.len() as u32)),
            "element {i} reported {:?}, not its 1-based place among {} siblings",
            info.index,
            NAMES.len()
        );
    }
}

/// `current` names the one element already in force; every other element
/// stays unselected, and a list with no current member marks nothing.
fn only_the_current_element_is_selected(fonts: &mut FontSystem) {
    let mut fc = FocusCtl::new();

    let none_current = open_and_read(&mut fc, fonts, None);
    assert!(
        none_current.iter().all(|(_, info)| !info.states.contains(States::SELECTED)),
        "an element reported SELECTED although the list has no current member"
    );

    let second_current = open_and_read(&mut fc, fonts, Some(1));
    for (i, (_, info)) in second_current.iter().enumerate() {
        assert_eq!(
            info.states.contains(States::SELECTED),
            i == 1,
            "element {i}'s SELECTED bit does not match current = Some(1)"
        );
        // SELECTED never displaces EXPANDED — the two are independent bits.
        assert!(info.states.contains(States::EXPANDED), "element {i} lost EXPANDED under a current mark");
    }
}

/// The mark travels with the INDEX, not with a position: moving `current`
/// from one element to another flips exactly those two entries and
/// leaves every other element's states untouched — the same claim
/// [`nacelle::object::button`]'s own ladder test makes for the dress.
fn the_selected_mark_travels_with_the_index_not_a_position(fonts: &mut FontSystem) {
    let mut fc = FocusCtl::new();
    let first = open_and_read(&mut fc, fonts, Some(0));
    let last = open_and_read(&mut fc, fonts, Some(NAMES.len() - 1));

    assert!(first[0].1.states.contains(States::SELECTED));
    assert!(!last[0].1.states.contains(States::SELECTED), "element 0 kept SELECTED after current moved away");
    assert!(!first[NAMES.len() - 1].1.states.contains(States::SELECTED));
    assert!(
        last[NAMES.len() - 1].1.states.contains(States::SELECTED),
        "the last element did not pick up SELECTED when current pointed at it"
    );

    // Untouched middle elements: neither states nor index moved.
    for i in 1..NAMES.len() - 1 {
        assert_eq!(first[i].1.states, last[i].1.states, "element {i} moved when current did not name it");
        assert_eq!(first[i].1.index, last[i].1.index, "element {i}'s index moved when nothing about it changed");
    }
}
