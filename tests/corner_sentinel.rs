//! `pill` is a WORD about the box, and it has to survive every door a
//! radius leaves the toolkit by.
//!
//! §5.0 bakes `@corner.pill` to a negative sentinel (-2.0), which means
//! "as round as this box can be" and has no value until there is a box.
//! Two doors used to eat it without a sound:
//!
//!   * `Surface::ring_fill` / `Surface::ring` on the host's own surface,
//!     where `ring_parts` clamped the number at zero — so an object that
//!     handed its raw `*.corner` token to the trait drew the square the
//!     master wrote `pill` to avoid;
//!   * the same pair on `AbiSurface`, which shipped the raw number down
//!     the ABI. The host on the far side has no business knowing what
//!     -2.0 spells: the sentinel is libnacelle's private notation, and a
//!     negative stroke width is not a shape any host can draw.
//!
//! Both are guarded here, and the second is guarded at the boundary
//! ITSELF — the probe below is a host that records the number without
//! interpreting it, which is the only way to tell "translated before it
//! left" from "translated after it arrived".

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use nacelle::draw::{CornerStyle, DrawList};
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::runtime::{ColorC, HostApi, RectC};
use nacelle::theme;
use nacelle::view::{AbiSurface, CtxSurface, Surface};
use nacelle::theme::Color;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;

/// A wide, short box: half its short side is 10 px, which is what a
/// capsule of this shape measures and what no other reading of -2.0
/// produces.
const BOX: Rect = Rect { x: 40.0, y: 60.0, w: 200.0, h: 20.0 };
const HALF_SHORT: f32 = 10.0;

const INK: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };

fn ctx<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem) -> Ctx<'a> {
    Ctx {
        access: None,
        dl,
        fonts,
        w: W,
        h: H,
        t: 0.0,
        mouse: Pointer::new(-1.0, -1.0),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    }
}

/// The radius of the first ring in the command register. The register
/// prints the corner a command CARRIED (`corners round:10.00 …`), not
/// the triangles it fanned into, so a capsule that came out square
/// cannot hide behind an equal vertex count.
fn first_ring_radius(dl: &DrawList) -> Option<f32> {
    dl.cmds().iter().find_map(|c| {
        let line = c.to_string();
        let rest = line.strip_prefix("ring_fill at")?;
        let word = rest.split(" corners ").nth(1)?.split_whitespace().next()?;
        word.split(':').nth(1)?.parse::<f32>().ok()
    })
}

/// The sentinel as the theme engine bakes it — asked for rather than
/// written as -2.0, so this test keeps testing the same thing if §5.0's
/// table ever renumbers.
fn pill() -> f32 {
    theme::expr::sentinel("pill").expect("§5.0 declares `pill`")
}

#[test]
fn the_host_surface_draws_the_capsule_the_master_wrote() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();

    // A raw `*.corner` token, exactly as an object reads it: the number
    // is the sentinel, and the box is what turns it into a length.
    let mut dl = DrawList::recording();
    let mut c = ctx(&mut dl, &mut fonts);
    CtxSurface::new(&mut c).ring_fill(BOX, CornerStyle::Round, pill(), INK);
    let drawn = first_ring_radius(&dl).expect("the surface drew no ring at all");
    assert!(
        (drawn - HALF_SHORT).abs() < 0.01,
        "`pill` drew a radius of {drawn} px where the capsule of this box is {HALF_SHORT} px"
    );

    // A stated LENGTH still arrives untouched — the translation must not
    // become a second opinion about radii that are already radii.
    let mut dl = DrawList::recording();
    let mut c = ctx(&mut dl, &mut fonts);
    CtxSurface::new(&mut c).ring_fill(BOX, CornerStyle::Round, 4.0, INK);
    let drawn = first_ring_radius(&dl).expect("the surface drew no ring at all");
    assert!((drawn - 4.0).abs() < 0.01, "a stated 4 px came out as {drawn} px");

    // And a sentinel that is NOT a length stays nothing: `auto` and
    // `same_as_parent` are the absence of a radius, and absence must not
    // be promoted to the largest one on the box.
    let auto = theme::expr::sentinel("auto").expect("§5.0 declares `auto`");
    let mut dl = DrawList::recording();
    let mut c = ctx(&mut dl, &mut fonts);
    CtxSurface::new(&mut c).ring_fill(BOX, CornerStyle::Round, auto, INK);
    let drawn = first_ring_radius(&dl).expect("the surface drew no ring at all");
    assert!((drawn - 0.0).abs() < 0.01, "`auto` invented a {drawn} px radius");
}

/// What the last host to be called was handed, in raw bits — a host that
/// only records cannot accidentally repair the number on the way in.
static SEEN: AtomicU32 = AtomicU32::new(0);

extern "C" fn probe_ring_fill(_p: *mut c_void, _r: RectC, _s: u32, radius: f32, _c: ColorC) {
    SEEN.store(radius.to_bits(), Ordering::SeqCst);
}

extern "C" fn probe_ring(_p: *mut c_void, _r: RectC, _s: u32, radius: f32, _w: f32, _c: ColorC) {
    SEEN.store(radius.to_bits(), Ordering::SeqCst);
}

#[test]
fn nothing_but_a_length_crosses_the_plugin_boundary() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut dl = DrawList::recording();
    let mut c = ctx(&mut dl, &mut fonts);

    // The real host table with its two ring entries replaced by probes:
    // everything else a surface asks at construction (`mouse`, `elapsed`,
    // `theme_epoch`) stays the genuine article, so this is the shipped
    // path with a witness standing in the doorway.
    let mut api: HostApi = *nacelle::plugin::host_api();
    api.ring_fill = probe_ring_fill;
    api.ring = probe_ring;
    let handle = (&mut c as *mut Ctx) as *mut c_void;
    let mut sf = AbiSurface::new(&api, handle);

    sf.ring_fill(BOX, CornerStyle::Round, pill(), INK);
    let crossed = f32::from_bits(SEEN.load(Ordering::SeqCst));
    assert!(
        (crossed - HALF_SHORT).abs() < 0.01,
        "{crossed} crossed the ABI where the capsule of this box is {HALF_SHORT} px — a host \
         cannot know that a negative number spells `pill`"
    );

    sf.ring(BOX, CornerStyle::Round, pill(), 1.0, INK);
    let crossed = f32::from_bits(SEEN.load(Ordering::SeqCst));
    assert!(
        (crossed - HALF_SHORT).abs() < 0.01,
        "the stroke half of the pair shipped {crossed} px"
    );

    sf.ring_fill(BOX, CornerStyle::Round, 4.0, INK);
    let crossed = f32::from_bits(SEEN.load(Ordering::SeqCst));
    assert!((crossed - 4.0).abs() < 0.01, "a stated 4 px crossed as {crossed} px");
}

/// The far side of the same door. A plugin written against the C table
/// by hand has no libnacelle in it to translate anything, so the host
/// reads the sentinel too — one compare, and the difference between the
/// theme's capsule and a silent square.
#[test]
fn the_host_reads_the_sentinel_a_hand_written_plugin_forwards() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();
    let mut dl = DrawList::recording();
    let mut c = ctx(&mut dl, &mut fonts);
    let api = nacelle::plugin::host_api();
    let handle = (&mut c as *mut Ctx) as *mut c_void;
    let r = RectC { x: BOX.x, y: BOX.y, w: BOX.w, h: BOX.h };
    let ink = ColorC { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    (api.ring_fill)(handle, r, nacelle::runtime::CORNER_ROUND, pill(), ink);
    let drawn = first_ring_radius(&dl).expect("the host drew no ring at all");
    assert!(
        (drawn - HALF_SHORT).abs() < 0.01,
        "a forwarded sentinel drew {drawn} px, not the {HALF_SHORT} px capsule"
    );
}
