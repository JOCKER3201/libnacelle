//! The toaster draws the popup's pixels — proved, not assumed.
//!
//! F2 §8.2 moves the desktop's warning popup into the toolkit as a queue
//! whose visible end is `toast.max_visible` deep. The condition on that
//! move is that the master theme, which ships `max_visible = 1`, comes
//! out unchanged: the same box in the same place with the same two lines
//! of text in the same colours.
//!
//! "Unchanged" is checked here mechanically rather than by eye. The
//! LEGACY drawing is reproduced verbatim below — it is `popup.rs`'s
//! `Popup::draw` as it stood before the port, down to its own `Role`
//! helper and its hard-coded role names — and both paths draw into a
//! draw list through the same fonts and the same theme. The two vertex
//! buffers must agree exactly: same positions, same atlas coordinates,
//! same colours, same runs.

use nacelle::draw::DrawList;
use nacelle::font::{FontSystem, FONT_UI};
use nacelle::object::toaster::{Toast, Toaster};
use nacelle::pointer::Pointer;
use nacelle::theme::{self, Color, TokenId};
use nacelle::{Ctx, Rect};
use std::sync::OnceLock;

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const MSG: &str = "Cannot save the board: no space left on device";

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

// ------------------------------------------------------- the legacy draw

/// `popup.rs`'s own type-role helper: the theme's px times the panel's
/// container query, floored by the ROLE's own `min_px`.
///
/// It used to multiply by `Ctx::ui_font_scale` as well, because popup.rs
/// did. That was the double-apply — the user's setting is already inside
/// every baked size as `metric.ui_scale` — and the toaster no longer
/// does it. The mirror follows, so the two state ONE rule: at the 1.0
/// this binary runs at the numbers agree either way, which is exactly
/// why a stale mirror would sit here silently until someone set the
/// scale and got a failure that blamed the wrong side.
struct Role {
    name: &'static str,
    size: OnceLock<TokenId>,
    min: OnceLock<TokenId>,
    track: OnceLock<TokenId>,
}

impl Role {
    const fn new(name: &'static str) -> Self {
        Role { name, size: OnceLock::new(), min: OnceLock::new(), track: OnceLock::new() }
    }
    fn px(&self, ctx: &Ctx) -> f32 {
        let t = theme::resolved();
        let s = *self.size.get_or_init(|| {
            theme::id(&format!("type.{}.size", self.name)).unwrap_or(TokenId::MISSING)
        });
        let m = *self.min.get_or_init(|| {
            theme::id(&format!("type.{}.min_px", self.name)).unwrap_or(TokenId::MISSING)
        });
        (t.px(s) * ctx.panel_scale).max(t.px(m))
    }
    fn tracking(&self, px: f32) -> f32 {
        let t = theme::resolved();
        let k = *self.track.get_or_init(|| {
            theme::id(&format!("type.{}.tracking", self.name)).unwrap_or(TokenId::MISSING)
        });
        px * t.px(k)
    }
}

static ROLE_TOAST_TITLE: Role = Role::new("label.section");
static ROLE_TOAST_BODY: Role = Role::new("body");

/// `Popup::draw`, verbatim, minus the expiry check (which decides
/// whether to draw, not what to draw).
fn legacy_popup_draw(ctx: &mut Ctx, msg: &str) {
    static MIN_W: OnceLock<TokenId> = OnceLock::new();
    static MAX_W: OnceLock<TokenId> = OnceLock::new();
    static TH: OnceLock<TokenId> = OnceLock::new();
    static TOP: OnceLock<TokenId> = OnceLock::new();
    static PAD_X: OnceLock<TokenId> = OnceLock::new();
    static TITLE_GAP: OnceLock<TokenId> = OnceLock::new();
    static MSG_GAP: OnceLock<TokenId> = OnceLock::new();
    static TITLE_C: OnceLock<TokenId> = OnceLock::new();
    static TEXT_C: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();

    let px = ROLE_TOAST_BODY.px(ctx);
    let title_px = ROLE_TOAST_TITLE.px(ctx);
    let text_w = ctx.fonts.measure(FONT_UI, px, msg, ROLE_TOAST_BODY.tracking(px));
    let bw = (text_w + 2.0 * t.px(tok(&PAD_X, "toast.pad_x")))
        .max(ctx.w * t.px(tok(&MIN_W, "toast.min_w_frac")))
        .min(ctx.w * t.px(tok(&MAX_W, "toast.max_w_frac")));
    let bh = t.px(tok(&TH, "toast.h"));
    let bx = (ctx.w - bw) / 2.0;
    let by = t.px(tok(&TOP, "toast.top"));

    nacelle::object::window::frame(ctx, Rect::new(bx, by, bw, bh));
    ctx.dl.text_center(
        ctx.fonts,
        FONT_UI,
        title_px,
        bx + bw / 2.0,
        by + t.px(tok(&TITLE_GAP, "toast.title_gap")),
        "WARNING",
        col(t.color(tok(&TITLE_C, "component.toast.title"))),
        ROLE_TOAST_TITLE.tracking(title_px),
    );
    ctx.dl.text_center(
        ctx.fonts,
        FONT_UI,
        px,
        bx + bw / 2.0,
        by + t.px(tok(&MSG_GAP, "toast.msg_gap")),
        msg,
        col(t.color(tok(&TEXT_C, "component.toast.text"))),
        ROLE_TOAST_BODY.tracking(px),
    );
}

// ------------------------------------------------------------- the diff

/// Every difference between two draw lists, as human-readable lines.
fn diff(a: &DrawList, b: &DrawList) -> Vec<String> {
    let mut out = Vec::new();
    if a.verts.len() != b.verts.len() {
        out.push(format!("vertex count {} vs {}", a.verts.len(), b.verts.len()));
    }
    for (i, (va, vb)) in a.verts.iter().zip(b.verts.iter()).enumerate() {
        if va.pos != vb.pos || va.uv != vb.uv || va.color != vb.color {
            out.push(format!(
                "vertex {i}: pos {:?}/{:?} uv {:?}/{:?} col {:?}/{:?}",
                va.pos, vb.pos, va.uv, vb.uv, va.color, vb.color
            ));
        }
        if out.len() > 8 {
            out.push("...".to_string());
            break;
        }
    }
    if a.runs.len() != b.runs.len() {
        out.push(format!("run count {} vs {}", a.runs.len(), b.runs.len()));
    }
    for (i, (ra, rb)) in a.runs.iter().zip(b.runs.iter()).enumerate() {
        if ra.end != rb.end || ra.image != rb.image || ra.clip != rb.clip {
            out.push(format!("run {i}: end {}/{} clip {:?}/{:?}", ra.end, rb.end, ra.clip, rb.clip));
        }
    }
    out
}

fn ctx<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem) -> Ctx<'a> {
    Ctx {
        access: None,
        dl,
        fonts,
        w: W,
        h: H,
        t: 0.0,
        mouse: Pointer::new(0.0, 0.0),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    }
}

#[test]
fn the_single_toast_of_the_master_is_the_popup_vertex_for_vertex() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();

    // The glyphs both paths use, rasterised once: an atlas that grows
    // between the two draws would move every UV and prove nothing.
    let mut warm = DrawList::new();
    {
        let mut c = ctx(&mut warm, &mut fonts);
        legacy_popup_draw(&mut c, MSG);
    }

    let mut before = DrawList::new();
    {
        let mut c = ctx(&mut before, &mut fonts);
        legacy_popup_draw(&mut c, MSG);
    }

    let mut after = DrawList::new();
    let mut toaster = Toaster::new();
    toaster.push(Toast::warning(MSG.to_string()));
    {
        let mut c = ctx(&mut after, &mut fonts);
        toaster.draw(&mut c);
    }

    assert!(!before.verts.is_empty(), "the legacy popup drew nothing — the harness is broken");
    let d = diff(&before, &after);
    assert!(d.is_empty(), "the toaster no longer draws the popup's pixels:\n{}", d.join("\n"));
}

#[test]
fn the_stack_is_off_in_the_master_so_the_second_toast_waits() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();

    let mut one = DrawList::new();
    let mut t1 = Toaster::new();
    t1.push(Toast::warning(MSG.to_string()));
    {
        let mut c = ctx(&mut one, &mut fonts);
        t1.draw(&mut c);
    }

    let mut two = DrawList::new();
    let mut t2 = Toaster::new();
    t2.push(Toast::warning(MSG.to_string()));
    t2.push(Toast::warning("and another thing".to_string()));
    {
        let mut c = ctx(&mut two, &mut fonts);
        t2.draw(&mut c);
    }

    // `toast.max_visible = 1`: the second notice is queued, not drawn,
    // so the screen is exactly the one-toast screen.
    let d = diff(&one, &two);
    assert!(d.is_empty(), "a queued toast reached the screen:\n{}", d.join("\n"));
    assert_eq!(t2.len(), 2, "the queued toast was dropped instead of waiting");
}

#[test]
fn the_master_declares_every_token_the_toaster_and_the_tooltip_read() {
    // The first read is what loads the master; `theme::id` before it
    // would answer None for every name in the file.
    let t = theme::resolved();
    for name in [
        "toast.h",
        "toast.top",
        "toast.pad_x",
        "toast.min_w_frac",
        "toast.max_w_frac",
        "toast.title_gap",
        "toast.msg_gap",
        "toast.dwell_ms",
        "toast.title.role",
        "toast.body.role",
        "toast.max_visible",
        "toast.gap",
        "component.toast.title",
        "component.toast.text",
        "tooltip.h",
        "tooltip.pad_x",
        "tooltip.pad_y",
        "tooltip.corner",
        "tooltip.border",
        "tooltip.offset",
        "tooltip.role",
        "tooltip.max_w",
        "tooltip.delay_ms",
        "tooltip.linger_ms",
        "component.tooltip.fill",
        "component.tooltip.edge",
        "component.tooltip.text",
    ] {
        assert!(theme::id(name).is_some(), "the master declares no {name}");
    }
    // The stack is off and the single box is the popup's: the two values
    // the pixel-identity rule rests on.
    assert_eq!(t.px(theme::id("toast.max_visible").unwrap()), 1.0);
    assert!(t.px(theme::id("tooltip.delay_ms").unwrap()) > 0.0);
}
