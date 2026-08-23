//! The decoration engine (u3 L6): what a frame paints that is not a
//! widget — the clear under everything, the fixtures' frosted glass,
//! the board ride's clock and easing. WHERE things sit is the layout
//! engine's; WHAT the stage furniture looks like is decided here, and
//! every value is a theme token, per the governing principle. The
//! backdrop PLATE (traces, grid, vignette) is `theme::plate` — baked
//! pixels, not per-frame geometry — and the backdrop WALLPAPER
//! (`backdrop.source = image`) is its sibling `theme::backdrop`, baked
//! the same way and composited by [`board_ground`] just beneath it.
//!
//! EVERY board paints its ground ([`board_ground`]), standing or
//! moving. This header used to say the opposite — "a board standing
//! still paints NO ground of its own: the clear and the plate already
//! fill the screen behind it" — and the host believed it. They do not
//! fill it, and the gap is a whole rung of the ladder: the frame clears
//! to `surface.void` ([`clear_color`]) while the ground a board stands
//! on is `backdrop.solid`, which the master derives from
//! `@surface.base`. Measured on the master 2026-08-18: sRGB(0.0096,
//! 0.0240, 0.0171) against sRGB(0.0418, 0.0758, 0.0613). So the same
//! board showed one ground standing and another the instant it turned,
//! and `backdrop.solid` and `elev.board.fill` had no reader at all on
//! the path 99% of frames take.
//!
//! It is also what a FROSTED surface samples. The renderer's base scene
//! is everything drawn before the first glass run; a theme that ships
//! no decoration bakes no plate, so a standing frame that paints no
//! ground opens its list with the first frosted panel, the base scene
//! is empty, and every glass quad on the screen reads a pyramid holding
//! nothing but the clear.
//!
//! A board turning SIDEWAYS is still a different thing — a WALL of a
//! solid — and takes its ground with it over the flat [`ride_void`] the
//! whole turn happens in; without that the walls are panes of glass
//! with the frame's own clear showing through them.

use crate::draw::{DrawList, ImageId};
use crate::theme::{self, Color, TokenId};
use std::sync::OnceLock;

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// A baked theme colour in the draw list's own colour type.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// The colour every frame clears to: `surface.void`, read as a BED so
/// a raw master clears near-black rather than mid-grey.
pub fn clear_color() -> Color {
    static VOID: OnceLock<TokenId> = OnceLock::new();
    col(theme::resolved().bed(tok(&VOID, "surface.void")))
}

/// The ground one board stands on, screen-sized, in the theme's own
/// order: `backdrop.solid` — what lies behind the board — then the
/// board's field `elev.board.fill`, then the wallpaper (`backdrop.source
/// = image`, `theme::backdrop::bake_wallpaper`), then the baked
/// decoration plate, the traces/grid/stars/vignette that live over it
/// (5.5, 5.15). Emitted before a board's panels, standing or moving: by
/// the FRAME once, under everything, and again per FACE by a board
/// riding sideways, so that caller's yaw and perspective carry ground
/// and panels together and the face turns as one solid wall. The two do
/// not fight — a sideways ride lays [`ride_void`] over the whole screen
/// before its first face, so the frame's own copy is covered for as
/// long as a cube is up. Two levels rather than one because a family-B
/// board paints NOTHING of its own (`elev.board.fill` at alpha 0) and a
/// wall of nothing is a pane of glass, not a wall: what that theme puts
/// behind its panes is the backdrop, and the backdrop is what the wall
/// carries.
///
/// `wallpaper` and `plate` are both the HOST's textures — this function
/// only composites, it never bakes (`theme::backdrop::bake_wallpaper`
/// and `theme::plate::bake_backdrop` do that, off the theme and the
/// surface size, whenever either changes) — and both are `None` when
/// their theme has nothing to show: no `source = image`, or no `decor.*`
/// layer turned on. A theme that sets neither draws exactly what it did
/// before `backdrop.source` had a reader at all: `backdrop.solid`, flat.
/// One that sets ONLY the wallpaper gets a photo with no traces or
/// vignette over it; one that sets both — images 7, 8 and 10's own
/// pairing — gets the photo with the theme's decoration drawn on top of
/// it, in that order, because a theme's traces are meant to read as
/// sitting ON a wallpaper, not under one.
pub fn board_ground(
    dl: &mut DrawList,
    w: f32,
    h: f32,
    wallpaper: Option<ImageId>,
    plate: Option<ImageId>,
) {
    static SOLID: OnceLock<TokenId> = OnceLock::new();
    static FILL: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    for id in [tok(&SOLID, "backdrop.solid"), tok(&FILL, "elev.board.fill")] {
        let c = col(t.bed(id));
        if c.a > 0.0 {
            dl.rect(0.0, 0.0, w, h, c);
        }
    }
    // White at 1.0 is the multiplicative identity on both images: the
    // plates' pixels — the wallpaper's already fitted and graded,
    // decor's own layers — ARE the colours to put on screen.
    const IDENTITY: Color = Color { r: 1.0, g: 1.0, b: 1.0, a: 1.0 };
    if let Some(id) = wallpaper {
        dl.image(0.0, 0.0, w, h, id, IDENTITY);
    }
    if let Some(id) = plate {
        dl.image(0.0, 0.0, w, h, id, IDENTITY);
    }
}

/// The flat colour the sideways ride happens in: painted once under the
/// whole cube, and the colour a wall settles toward as it turns away
/// from the viewer, so a wall edge-on melts into the space behind it
/// instead of into grey. Read as a BED — a raw master rides through
/// near-black rather than mid-grey.
///
/// WHAT THE MASTER PUTS THERE AND WHY IT MOVED. `motion.board_ride.void`
/// derived from `@surface.void` — the swapchain clear — for as long as a
/// standing frame painted nothing else, and the two were the same colour
/// by accident of that. They are not the same rung: the ground a board
/// stands on is `backdrop.solid`, and once the frame started laying it
/// ([`board_ground`], 2026-08-18) a ride that opened on the clear dropped
/// the whole screen fourfold darker for its 300 ms and put it back. The
/// master now derives this from `@backdrop.solid`, which is what its own
/// comment always meant by "the ground the frame already stands on"; a
/// theme that wants the cube to turn in a darker room still says so here,
/// which is the point of the token.
pub fn ride_void() -> Color {
    static VOID: OnceLock<TokenId> = OnceLock::new();
    col(theme::resolved().bed(tok(&VOID, "motion.board_ride.void")))
}

/// A fixture's face: the sheet `[elev.fixture]` asks for, over whatever
/// sits beneath. `wash_scale` is the USER's opacity setting — a
/// multiplier on the wash's alpha, nothing else (the BlurOpacity
/// slider's contract). The glass is sampled by screen position, so a
/// ride may carry the quad and the frost stays put.
pub fn fixture_glass(dl: &mut DrawList, w: f32, h: f32, wash_scale: f32) {
    fixture_glass_in(dl, theme::resolved(), w, h, wash_scale);
}

/// [`fixture_glass`] with the theme in hand.
///
/// Split for the same reason `elev::Level::draw_in` is: a face drawn
/// from a theme that is not the published one is the only way this
/// picture can be put under test at all, without a test reaching into
/// the process-wide theme every other test is reading at the same time.
///
/// THE RANK DECIDES WHETHER THERE IS GLASS. `elev.fixture.glass.rank`
/// says so in the master's own words — "0 emits no blur() run at all",
/// and at `fill`, "used INSTEAD of the glass pair while rank = 0" — and
/// until 2026-08-17 this function blurred whatever the key said, which
/// left BOTH those keys dead: a theme could not turn the frost off, and
/// the sheet the master describes (`alpha(@surface.base, 0.92)`,
/// legible with no offscreen pass) could never be drawn. The master's
/// rank moved 0 -> 1 on the same day, so the shipped picture is
/// unchanged and the file now states what it was already showing.
///
/// Ranks 1..3 are ONE picture on this path, and that is a gap rather
/// than a design: the fixture is a full-screen sheet and takes the flat
/// `blur()` run, whose softness is the renderer's single blur target,
/// while the pyramid's three ranks are `glass_fill`'s. Moving the
/// fixture onto `glass_fill` would also put it on the rung's `corner`
/// and `radius` — rounded screen corners — and that is a change to the
/// picture, which belongs to the owner and not to this line.
pub(crate) fn fixture_glass_in(
    dl: &mut DrawList,
    t: &theme::ResolvedTheme,
    w: f32,
    h: f32,
    wash_scale: f32,
) {
    static RANK: OnceLock<TokenId> = OnceLock::new();
    static FILL: OnceLock<TokenId> = OnceLock::new();
    static TINT: OnceLock<TokenId> = OnceLock::new();
    static WASH: OnceLock<TokenId> = OnceLock::new();
    if t.px(tok(&RANK, "elev.fixture.glass.rank")).clamp(0.0, 3.0) <= 0.0 {
        // The bed the rung names, read as a BED, and drawn only when
        // there is something in it: a rung whose `fill` is `none` draws
        // nothing, which is the raw look the governing principle asks
        // for and the same guard every other rung is under.
        let fill = col(t.bed(tok(&FILL, "elev.fixture.fill")));
        if fill.a > 0.0 {
            dl.rect(0.0, 0.0, w, h, fill);
        }
        return;
    }
    dl.blur(0.0, 0.0, w, h, col(t.color(tok(&TINT, "elev.fixture.glass.tint"))));
    let wash = t.color(tok(&WASH, "elev.fixture.glass.wash"));
    let a = wash.a * wash_scale;
    if a > 0.0 {
        dl.rect(0.0, 0.0, w, h, col(wash).alpha(a));
    }
}

/// The board ride's clock: seconds for the full move, after the
/// theme's global motion scale. Zero — disabled, scale 0, or no
/// token — is a hard cut, which is exactly what reduced motion asks
/// for. A thin shell over [`crate::motion::Effect::one_shot_secs`],
/// kept because the desktop calls it by this name.
pub fn ride_secs() -> f32 {
    crate::motion::Effect::of("board_ride").one_shot_secs()
}

/// The ride's easing, picked by the motion token's word. The "shared
/// motion resolver" this header used to promise exists now
/// (`crate::motion`), and this is its board-ride door: the curve is
/// chosen by the live theme's WORD — the enum-index cache that froze it
/// across a theme swap is gone — and `custom`'s cubic-bezier control
/// points finally have their reader.
pub fn ride_ease(t01: f32) -> f32 {
    crate::motion::Effect::of("board_ride").ease(t01)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::{DrawCmd, DrawList};

    /// A screen, in the only two numbers this function takes.
    const W: f32 = 1920.0;
    const H: f32 = 1080.0;

    /// The user's BlurOpacity at rest, so the wash carries the theme's
    /// own alpha and no other.
    const FULL: f32 = 1.0;

    fn drawn(theme_text: &str) -> Vec<DrawCmd> {
        let t = theme::bake_over_master(theme_text);
        let mut dl = DrawList::recording();
        fixture_glass_in(&mut dl, &t, W, H, FULL);
        dl.cmds().to_vec()
    }

    fn blurs(cmds: &[DrawCmd]) -> usize {
        cmds.iter().filter(|c| matches!(c, DrawCmd::Blur { .. })).count()
    }

    /// Z15/Z16's neighbour on the ladder: the rung's rank decides whether
    /// there is glass. Until 2026-08-17 this function laid a blur run
    /// whatever the key said, so a theme asking for a plain sheet got
    /// frost and no word about it.
    #[test]
    fn a_rank_of_zero_lays_the_rungs_own_bed_and_no_blur() {
        let cmds = drawn("[elev.fixture]\nglass.rank = 0\nfill = #FF00FF / 1.0\n");
        assert_eq!(blurs(&cmds), 0, "rank 0 still blurred: {cmds:?}");
        let bed = cmds
            .iter()
            .find_map(|c| match c {
                DrawCmd::Rect { color, .. } => Some(*color),
                _ => None,
            })
            .expect("rank 0 draws the rung's fill instead of the glass pair");
        assert!(
            bed.r > 0.99 && bed.g < 0.01 && bed.b > 0.99,
            "the sheet {bed:?} is not elev.fixture.fill"
        );
    }

    /// …and a rung whose bed is `none` at rank 0 draws NOTHING, which is
    /// the raw look the governing principle asks for rather than a
    /// hairline invented here. A §5.0 sentinel empties the colour slot,
    /// so this is the same `a > 0.0` guard every other rung is under.
    #[test]
    fn a_rank_of_zero_over_an_empty_bed_draws_nothing_at_all() {
        let cmds = drawn("[elev.fixture]\nglass.rank = 0\nfill = none\n");
        assert!(cmds.is_empty(), "an empty rung painted something: {cmds:?}");
    }

    /// The master's own fixture is frosted, and says so since 2026-08-17:
    /// the rank moved 0 -> 1 the day it gained a reader, so the shipped
    /// picture did not move — one blur run, exactly as before.
    #[test]
    fn the_master_ships_a_frosted_fixture_and_the_key_now_says_so() {
        let cmds = drawn("");
        assert_eq!(blurs(&cmds), 1, "the master's fixture stopped frosting: {cmds:?}");
    }

    /// A raised rank frosts, and the wash rides the USER's opacity on top
    /// of the theme's own alpha — the BlurOpacity slider's whole
    /// contract, and the reason the scale is a parameter and not a token.
    #[test]
    fn the_users_opacity_scales_the_wash_and_nothing_else() {
        let theme_text = "[elev.fixture]\nglass.rank = 2\nglass.wash = #FFFFFF / 0.8\n";
        let t = theme::bake_over_master(theme_text);
        let alpha_at = |scale: f32| {
            let mut dl = DrawList::recording();
            fixture_glass_in(&mut dl, &t, W, H, scale);
            dl.cmds().iter().find_map(|c| match c {
                DrawCmd::Rect { color, .. } => Some(color.a),
                _ => None,
            })
        };
        let full = alpha_at(1.0).expect("a wash with alpha draws its quad");
        let half = alpha_at(0.5).expect("a wash with alpha draws its quad");
        assert!((full - 0.8).abs() < 1e-6, "the theme's wash alpha arrived as {full}");
        assert!((half - 0.4).abs() < 1e-6, "the user's opacity did not scale the wash: {half}");
        assert_eq!(alpha_at(0.0), None, "an opacity of zero still drew the wash quad");
    }

    /// A RIDE IS A TURN, NOT A FLASH: the space the cube turns in is the
    /// ground the standing board was already on.
    ///
    /// `motion.board_ride.void` derived from `@surface.void` — the
    /// swapchain clear — and that was the same colour as the ground only
    /// while a standing frame painted no ground at all. It paints one now
    /// ([`board_ground`], from the frame as well as from each face), and
    /// the two tokens are a rung of the ladder apart: measured on the
    /// master, sRGB(0.0096, 0.0240, 0.0171) against sRGB(0.0418, 0.0758,
    /// 0.0613). A board pushed sideways therefore dropped the whole
    /// screen fourfold darker for the length of the ride and put it back
    /// — a picture nobody asked for, on a path with no test to notice.
    ///
    /// Both halves are the theme's and neither is a number written here:
    /// the master's own value has to BE the ground, and it has to be the
    /// ground BY REFERENCE, so a theme that moves its backdrop moves the
    /// space its cube turns in with it. A copied literal passes the first
    /// claim and fails the second.
    #[test]
    fn the_cube_turns_in_the_ground_a_standing_board_lays() {
        let same = |t: &theme::ResolvedTheme, note: &str| {
            let ground = col(t.bed(theme::id("backdrop.solid").expect("backdrop.solid")));
            let void = col(t.bed(theme::id("motion.board_ride.void").expect("the void")));
            for (ch, g, v) in
                [('r', ground.r, void.r), ('g', ground.g, void.g), ('b', ground.b, void.b)]
            {
                assert!(
                    (g - v).abs() < 1e-4,
                    "{note}: the ride's void is not the ground the board stands on: \
                     {ch} {v} against the ground's {g}"
                );
            }
        };
        same(&theme::bake_over_master(""), "the master");
        // …and it follows the backdrop, because it is written as a
        // reference to it and not as a copy of today's colour.
        same(
            &theme::bake_over_master("[backdrop]\nsolid = #FF00FF / 1.0\n"),
            "a theme that moves its backdrop",
        );
    }
}
