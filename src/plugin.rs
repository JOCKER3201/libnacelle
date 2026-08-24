//! The host side of the plugin boundary.
//!
//! [`host_api`] builds the table of functions a plugin draws through,
//! and [`PluginWidget`] wraps a loaded plugin so the application drives
//! it through the ordinary [`Widget`](crate::Widget) contract, exactly
//! like a script.
//!
//! Opening the library is the one part that is not here: `dlopen` is a
//! platform call, so the loader lives in the application beside its
//! other platform code. What it must do is fixed, though — see
//! [`crate::runtime::ATTACH_SYMBOL`].

use crate::runtime::{
    ActionC, CellC, ChromeC, ColorC, HostApi, PluginApi, RectC, TermReqC, TermSelectC, TermViewC,
    ABI_VERSION, CELL_HAS_BG, CELL_SELECTED, CELL_SIZE_MIN, CELL_UNDERLINE, TERM_REQ_SIZE_MIN,
    TERM_SELECT_SIZE_MIN, TERM_VIEW_SIZE_MIN, VIEW_CURSOR, VIEW_LIVE, VIEW_TRUNCATED,
    ACTION_BYTES, ACTION_CAPTURE, ACTION_EXIT, SIZING_ROWS, ACTION_NONE, ACTION_OPEN_DIR,
    ACTION_OPEN_FILE,
    ACTION_OPEN_SETTINGS, ACTION_PASTE_PRIMARY, ACTION_SCROLL_TERMINAL, ACTION_SELECT_TAB,
    ACTION_TERM_SELECT, BUTTON_PRESS, BUTTON_RELEASE, CHROME_BUTTONS_CLOSE,
    CHROME_BUTTONS_MIN_CLOSE,
    CHROME_BUTTONS_MIN_MAX_CLOSE, DRAG_BEGIN, DRAG_END, DRAG_MOVE, MASK_QUAD_ADD,
    PLUGIN_API_HAS_BUTTON, PLUGIN_API_HAS_CHROME, PLUGIN_API_HAS_DRAG, PLUGIN_API_HAS_KEY,
    PLUGIN_API_HAS_POINTER, PLUGIN_API_SIZE_MIN,
    SELECT_KIND_LINES,
    SELECT_KIND_WORDS, SELECT_OP_BEGIN, SELECT_OP_END, SELECT_OP_EXTEND, StateStyleC, keys,
};
use crate::font::{FontSystem, FONT_COUNT};
use crate::term::{Cell, SelKind, FLAG_UNDERLINE, FLAG_WIDE_LEAD, FLAG_WIDE_SPACER};
use crate::draw::{ring_segments, Corner};
use crate::theme::Color;
use crate::widget::{DragPhase, SelectOp, Sizing};
use crate::{Action, Ctx, Host, Rect, Widget};
use std::ffi::c_void;
use std::path::PathBuf;

fn color_in(c: ColorC) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

fn color_out(c: Color) -> ColorC {
    ColorC { r: c.r, g: c.g, b: c.b, a: c.a }
}

fn rect_out(r: Rect) -> RectC {
    RectC { x: r.x, y: r.y, w: r.w, h: r.h }
}

/// A font id from a plugin indexes a fixed array, so it is clamped
/// rather than trusted: a typo or a plugin built when there were more
/// fonts must draw in the wrong face, not read past the end of one.
fn font_in(font: u32) -> u8 {
    (font as u8).min(FONT_COUNT - 1)
}


/// Reads a UTF-8 span a plugin passed. Anything that is not valid UTF-8
/// is dropped rather than trusted: this crossed a library boundary.
fn text_in<'a>(p: *const u8, len: u32) -> &'a str {
    if p.is_null() || len == 0 {
        return "";
    }
    let bytes = unsafe { std::slice::from_raw_parts(p, len as usize) };
    std::str::from_utf8(bytes).unwrap_or("")
}

/// Turns the opaque handle back into the drawing context.
///
/// # Safety
/// Only valid inside a call the host made, where the pointer is the
/// context it passed.
unsafe fn ctx_of<'a>(p: *mut c_void) -> Option<&'a mut Ctx<'a>> {
    (p as *mut Ctx).as_mut()
}

macro_rules! with_ctx {
    ($p:expr, $ctx:ident, $body:expr) => {{
        let Some($ctx) = (unsafe { ctx_of($p) }) else { return Default::default() };
        $body
    }};
}

extern "C" fn h_rect(p: *mut c_void, r: RectC, c: ColorC) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    ctx.dl.rect(r.x, r.y, r.w, r.h, color_in(c));
}

extern "C" fn h_rect_outline(p: *mut c_void, r: RectC, t: f32, c: ColorC) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    ctx.dl.rect_outline(r.x, r.y, r.w, r.h, t, color_in(c));
}

extern "C" fn h_quad(p: *mut c_void, pts: *const f32, c: ColorC) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    if pts.is_null() {
        return;
    }
    let v = unsafe { std::slice::from_raw_parts(pts, 8) };
    ctx.dl.quad(
        [[v[0], v[1]], [v[2], v[3]], [v[4], v[5]], [v[6], v[7]]],
        color_in(c),
    );
}

extern "C" fn h_line(p: *mut c_void, x0: f32, y0: f32, x1: f32, y1: f32, t: f32, c: ColorC) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    ctx.dl.line(x0, y0, x1, y1, t, color_in(c));
}

#[allow(clippy::too_many_arguments)]
extern "C" fn h_polyline(
    p: *mut c_void,
    pts: *const f32,
    count: u32,
    t: f32,
    c: ColorC,
    closed: bool,
) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    if pts.is_null() || count < 2 || count > POLYLINE_MAX {
        return;
    }
    let v = unsafe { std::slice::from_raw_parts(pts, count as usize * 2) };
    let points: Vec<[f32; 2]> = v.chunks_exact(2).map(|c| [c[0], c[1]]).collect();
    ctx.dl.polyline(&points, t, color_in(c), closed);
}

#[allow(clippy::too_many_arguments)]
extern "C" fn h_text(
    p: *mut c_void,
    font: u32,
    px: f32,
    x: f32,
    y: f32,
    text: *const u8,
    len: u32,
    c: ColorC,
    spacing: f32,
    align: u32,
) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    let s = text_in(text, len);
    let font = font_in(font);
    let color = color_in(c);
    match align {
        1 => ctx.dl.text_center(ctx.fonts, font, px, x, y, s, color, spacing),
        2 => ctx.dl.text_right(ctx.fonts, font, px, x, y, s, color, spacing),
        _ => {
            ctx.dl.text(ctx.fonts, font, px, x, y, s, color, spacing);
        }
    }
}

extern "C" fn h_measure(
    p: *mut c_void,
    font: u32,
    px: f32,
    text: *const u8,
    len: u32,
    spacing: f32,
) -> f32 {
    with_ctx!(
        p,
        ctx,
        ctx.fonts.measure(font_in(font), px, text_in(text, len), spacing)
    )
}

#[allow(clippy::too_many_arguments)]
extern "C" fn h_module_title(
    p: *mut c_void,
    x: f32,
    y: f32,
    w: f32,
    px: f32,
    left: *const u8,
    left_len: u32,
    right: *const u8,
    right_len: u32,
    c: ColorC,
) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    // The ABI keeps the underline on; a plugin that wants a different
    // header has the text and line primitives to build its own.
    ctx.dl.module_title(
        ctx.fonts,
        x,
        y,
        w,
        px,
        text_in(left, left_len),
        text_in(right, right_len),
        color_in(c),
        true,
    );
}

// ---- ABI 5: the theme as tokens ------------------------------------------
// A plugin's ctx pointer is irrelevant to these — the resolved theme is the
// process's — but the parameter stays in the signatures so the calls read
// like every other HostApi entry and a future per-window theme has room.

extern "C" fn h_theme_token(name: *const u8, len: u32) -> u32 {
    if name.is_null() || len == 0 || len > 256 {
        return u32::MAX;
    }
    let bytes = unsafe { std::slice::from_raw_parts(name, len as usize) };
    match std::str::from_utf8(bytes).ok().and_then(crate::theme::id) {
        Some(t) => t.0 as u32,
        None => u32::MAX,
    }
}

fn tid(id: u32) -> crate::theme::TokenId {
    if id > u16::MAX as u32 {
        crate::theme::TokenId::MISSING
    } else {
        crate::theme::TokenId(id as u16)
    }
}

fn ccol(c: crate::theme::ThemeColor) -> ColorC {
    ColorC { r: c.r, g: c.g, b: c.b, a: c.a }
}

extern "C" fn h_theme_color(_p: *mut c_void, id: u32) -> ColorC {
    ccol(crate::theme::resolved().color(tid(id)))
}

extern "C" fn h_theme_bed(_p: *mut c_void, id: u32) -> ColorC {
    ccol(crate::theme::resolved().bed(tid(id)))
}

extern "C" fn h_theme_px(_p: *mut c_void, id: u32) -> f32 {
    crate::theme::resolved().px(tid(id))
}

extern "C" fn h_theme_flag(_p: *mut c_void, id: u32) -> u32 {
    crate::theme::resolved().flag(tid(id)) as u32
}

extern "C" fn h_theme_enum(_p: *mut c_void, id: u32) -> u32 {
    crate::theme::resolved().enum_of(tid(id)) as u32
}

extern "C" fn h_theme_class(name: *const u8, len: u32) -> u32 {
    if name.is_null() || len == 0 || len > 256 {
        return u32::MAX;
    }
    let bytes = unsafe { std::slice::from_raw_parts(name, len as usize) };
    match std::str::from_utf8(bytes).ok().and_then(crate::theme::class_id) {
        Some(c) => c as u32,
        None => u32::MAX,
    }
}

extern "C" fn h_theme_class_state(
    _p: *mut c_void,
    class: u32,
    state: u32,
    out: *mut StateStyleC,
    out_size: u32,
) -> u32 {
    if out.is_null() || out_size == 0 {
        return 0;
    }
    let st = if class <= u16::MAX as u32 && state < 7 {
        let s = crate::theme::parse::STATE_NAMES[state as usize];
        let s = crate::theme::parse::State::from_name(s).unwrap_or(crate::theme::parse::State::Idle);
        crate::theme::resolved().class_state(class as u16, s)
    } else {
        crate::theme::bake::StateStyle::RAW
    };
    let c = StateStyleC {
        fill: ccol(st.fill),
        edge: ccol(st.edge),
        text: ccol(st.text),
        glyph: ccol(st.glyph),
        edge_width: st.edge_width,
        glow_radius: st.glow_radius,
        glow_alpha: st.glow_alpha,
        elevation: st.elevation,
    };
    // Prefix write: an older caller with a smaller struct gets the front of
    // it, which is what lets StateStyleC grow by appending later.
    let n = (out_size as usize).min(std::mem::size_of::<StateStyleC>());
    unsafe {
        std::ptr::copy_nonoverlapping(&c as *const StateStyleC as *const u8, out as *mut u8, n);
    }
    n as u32
}

extern "C" fn h_theme_epoch(_p: *mut c_void) -> u32 {
    crate::theme::epoch()
}

// ---- ABI 6, appended: the enum WORD, and the mask sprite ------------------
// Past HOST_API_SIZE_MIN; a plugin asks `HostApi::has_*` before calling.

extern "C" fn h_theme_enum_word(_p: *mut c_void, id: u32, buf: *mut u8, cap: u32) -> u32 {
    if buf.is_null() || cap == 0 {
        return 0;
    }
    let Some(word) = crate::theme::enum_word_of(tid(id)) else { return 0 };
    let n = word.len().min(cap as usize);
    unsafe { std::ptr::copy_nonoverlapping(word.as_ptr(), buf, n) };
    n as u32
}

/// A TEXT token's value, out to a plugin. The token id is resolved on
/// the plugin's side by `theme_token`, exactly as every other theme entry
/// takes it, and the NAME is recovered here because a text token is
/// stored by name in the diagnostics rather than in the baked table — the
/// same cold path `ui::ellipsis` reads, and the reason this call is
/// documented as init-time.
extern "C" fn h_theme_text(_p: *mut c_void, id: u32, buf: *mut u8, cap: u32) -> u32 {
    if buf.is_null() || cap == 0 {
        return 0;
    }
    let Some(name) = crate::theme::name_of(tid(id)) else { return 0 };
    let text = crate::ui::theme_text_named(&name);
    let n = text.len().min(cap as usize);
    unsafe { std::ptr::copy_nonoverlapping(text.as_ptr(), buf, n) };
    n as u32
}

/// The plugin half of the sprite glow/shadow path: four corners, four
/// sprite-space texcoords, one colour. The mapping into the atlas's mask
/// band happens in [`DrawList::mask_quad`](crate::draw::DrawList::mask_quad),
/// on this side of the boundary, so the numbers a plugin passes can
/// address the soft disk and nothing else.
extern "C" fn h_mask_quad(p: *mut c_void, pts: *const f32, uv: *const f32, c: ColorC, flags: u32) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    if pts.is_null() || uv.is_null() {
        return;
    }
    let pv = unsafe { std::slice::from_raw_parts(pts, 8) };
    let tv = unsafe { std::slice::from_raw_parts(uv, 8) };
    let corners = |s: &[f32]| -> [[f32; 2]; 4] {
        std::array::from_fn(|i| [s[i * 2], s[i * 2 + 1]])
    };
    ctx.dl.mask_quad(
        corners(pv),
        corners(tv),
        FontSystem::mask_soft_uv(),
        color_in(c),
        flags & MASK_QUAD_ADD != 0,
    );
}

/// The plugin half of the icon path (K8): interns `name` on THIS ctx's
/// own [`FontSystem`], parsing `svg` only the first time the name is
/// seen — see [`FontSystem::icon_id`] for the caching this rests on.
/// `u32::MAX`, never a real id, on a null pointer, on `name` that is not
/// UTF-8, or on an `svg` [`crate::icon::IconSource::parse`] refuses.
extern "C" fn h_icon_register(
    p: *mut c_void,
    name: *const u8,
    name_len: u32,
    svg: *const u8,
    svg_len: u32,
) -> u32 {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return u32::MAX };
    if name.is_null() || svg.is_null() {
        return u32::MAX;
    }
    let name = unsafe { std::slice::from_raw_parts(name, name_len as usize) };
    let Ok(name) = std::str::from_utf8(name) else { return u32::MAX };
    let svg = unsafe { std::slice::from_raw_parts(svg, svg_len as usize) };
    ctx.fonts.icon_id(name, svg).unwrap_or(u32::MAX)
}

/// The plugin half of the icon path's draw call: four corners, eight
/// floats, exactly like [`h_quad`] — [`crate::draw::DrawList::icon_quad`]
/// on this side of the boundary, so the numbers a plugin passes can
/// never name an atlas texel directly, only an icon id it already holds.
extern "C" fn h_icon_quad(p: *mut c_void, id: u32, px: f32, pts: *const f32, c: ColorC) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    if pts.is_null() {
        return;
    }
    let pv = unsafe { std::slice::from_raw_parts(pts, 8) };
    let p4: [[f32; 2]; 4] = std::array::from_fn(|i| [pv[i * 2], pv[i * 2 + 1]]);
    ctx.dl.icon_quad(ctx.fonts, id, px.max(0.0).round() as u32, p4, color_in(c));
}

/// The clip pair, forwarding [`DrawList::push_clip`](crate::draw::DrawList::push_clip)
/// straight across the boundary — the nesting, the intersection and the
/// run boundary are all the draw list's, so a plugin's clip behaves
/// exactly like the host's own. A widget that early-returns between the
/// two leaves the stack deep; `PluginWidget::draw` unwinds it, which is
/// why this function needs no bookkeeping of its own.
extern "C" fn h_push_clip(p: *mut c_void, r: RectC) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    ctx.dl.push_clip(r.x, r.y, r.w, r.h);
}

extern "C" fn h_pop_clip(p: *mut c_void) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    // Popping more than was pushed is the plugin's error, not a reason
    // to lose the host's own clip: the draw list forgives an empty pop,
    // and the depth check around `draw` catches the imbalance.
    ctx.dl.pop_clip();
}

/// The corner vocabulary of the boundary, turned into the toolkit's own.
/// Anything the plugin invents degrades to Square — the same rule the
/// theme's enum words follow when a vocabulary does not name a word, and
/// the same table: [`crate::corner::of_code`] walks `corner::WORDS`, so a
/// cut added there is understood at this door without a second edit.
fn corners_in(style: u32, radius: f32, r: RectC) -> ([Corner; 4], u8) {
    let style = crate::corner::of_code(style);
    // Half the short side is the geometric ceiling: past it the arcs of
    // two corners would cross and the outline would fold on itself.
    //
    // §5.0's `pill` is translated on the SENDING side (`AbiSurface::
    // ring_fill`), so a length is what should arrive — but a plugin that
    // builds this call by hand, in a language with no libnacelle in it,
    // can still forward a raw `*.corner` token. Reading the sentinel
    // here as well costs one compare and is the difference between the
    // capsule the theme wrote and a silent square.
    let size = crate::theme::corner_radius(radius, r.w, r.h).min(r.w.min(r.h) / 2.0);
    // The ceiling is the theme's `corner.segments`, exactly as it is on
    // the host's own surface: a plugin's ring and a panel's ring are the
    // same shape drawn through two doors, and a number written here would
    // be the one place a theme could not reach.
    let seg = ring_segments(size, 0.25, crate::view::surface::corner_segments());
    ([Corner { style, size }; 4], seg)
}

extern "C" fn h_ring_fill(p: *mut c_void, r: RectC, style: u32, radius: f32, c: ColorC) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    if r.w <= 0.0 || r.h <= 0.0 {
        return;
    }
    let (corners, seg) = corners_in(style, radius, r);
    ctx.dl.ring_fill(Rect::new(r.x, r.y, r.w, r.h), &corners, seg, color_in(c));
}

extern "C" fn h_ring(p: *mut c_void, r: RectC, style: u32, radius: f32, w: f32, c: ColorC) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    if r.w <= 0.0 || r.h <= 0.0 || w <= 0.0 {
        return;
    }
    let (corners, seg) = corners_in(style, radius, r);
    ctx.dl.ring(Rect::new(r.x, r.y, r.w, r.h), &corners, seg, w, color_in(c));
}

/// The glow of the same family — [`DrawList::glow_ring`] across the
/// boundary, through the same [`corners_in`] translation `ring_fill` and
/// `ring` use, so a plugin's chamfered badge glows on the exact octagon
/// its fill and stroke drew rather than a second approximation of it.
extern "C" fn h_ring_glow(
    p: *mut c_void,
    r: RectC,
    style: u32,
    radius: f32,
    glow_radius: f32,
    c: ColorC,
) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    if r.w <= 0.0 || r.h <= 0.0 || !(glow_radius > 0.0) || c.a <= 0.0 {
        return;
    }
    let (corners, seg) = corners_in(style, radius, r);
    ctx.dl.glow_ring(
        Rect::new(r.x, r.y, r.w, r.h),
        &corners,
        seg,
        glow_radius,
        color_in(c),
        FontSystem::mask_soft_uv(),
    );
}

/// A plugin's tooltip request, filed with the application's manager —
/// the same call `CtxSurface::tooltip` makes, because it is the same
/// request: the plugin may not draw the box (it would be covered by the
/// panels drawn after it), so the host does.
///
/// The containment test is repeated here rather than trusted, exactly as
/// on the host's own surface: a request for a rectangle the pointer is
/// nowhere near would explain the wrong thing, and one comparison is
/// cheaper than finding that out on screen.
extern "C" fn h_tooltip(p: *mut c_void, id: u64, anchor: RectC, text: *const u8, len: u32) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    let r = Rect::new(anchor.x, anchor.y, anchor.w, anchor.h);
    if !ctx.mouse.over(r) {
        return;
    }
    let s = text_in(text, len);
    if s.is_empty() {
        return;
    }
    let now = ctx.t;
    if let Some(tips) = ctx.tips.as_deref_mut() {
        tips.request(id, r, s, now);
    }
}

/// The channel's two entries. They take no drawing context because the
/// board is the PROCESS's, like the theme's token table: a value stated
/// while drawing must still be there when another widget's click reads
/// it three frames later.
extern "C" fn h_channel_publish(
    topic: *const u8,
    topic_len: u32,
    data: *const u8,
    data_len: u32,
) -> u64 {
    let topic = text_in(topic, topic_len);
    if topic.is_empty() {
        return 0;
    }
    // An empty payload is a legitimate value ("nothing is selected
    // now"), so no bytes is a publication rather than a refusal — and a
    // null pointer is no bytes whatever its length claims, because
    // `from_raw_parts` on one is undefined behaviour before anything is
    // ever read. A length past the ceiling is refused outright rather
    // than clamped: half a message is not the message.
    let bytes: &[u8] = if data.is_null() || data_len == 0 {
        &[]
    } else if data_len as usize > crate::runtime::CHANNEL_VALUE_MAX {
        return 0;
    } else {
        unsafe { std::slice::from_raw_parts(data, data_len as usize) }
    };
    crate::channel::publish(topic, bytes)
}

extern "C" fn h_channel_read(
    topic: *const u8,
    topic_len: u32,
    buf: *mut u8,
    cap: u32,
    seq: *mut u64,
) -> u32 {
    let topic = text_in(topic, topic_len);
    if topic.is_empty() {
        return 0;
    }
    // A caller wanting only the sequence number passes no buffer, which
    // is what `channel::seq` does on this side too.
    let mut none: [u8; 0] = [];
    let out: &mut [u8] = if buf.is_null() || cap == 0 {
        &mut none
    } else {
        unsafe { std::slice::from_raw_parts_mut(buf, cap as usize) }
    };
    let (len, s) = crate::channel::read_into(topic, out);
    unsafe {
        if let Some(seq) = seq.as_mut() {
            *seq = s;
        }
    }
    len.min(u32::MAX as usize) as u32
}

/// The settings pair. Like the channel's, they take neither the drawing
/// context nor the host handle: the directories are the PROCESS's, and a
/// widget reads its settings in `create`, before any frame exists and
/// before the host has a pointer to hand it.
///
/// This is where the plugin's inability to open a file is actually
/// enforced. Everything the caller passes is a NAME; the path, the
/// search order and the size ceiling are decided on this side and never
/// crossed back.
#[allow(clippy::too_many_arguments)]
extern "C" fn h_settings_read(
    addon: *const u8,
    addon_len: u32,
    file: *const u8,
    file_len: u32,
    buf: *mut u8,
    cap: u32,
    status: *mut u32,
) -> u32 {
    let addon = text_in(addon, addon_len);
    // An empty `file` is the addon's own single settings file, so unlike
    // a topic, empty here is a legitimate request rather than a refusal.
    let file = text_in(file, file_len);
    // A caller wanting only the status — "is there a file at all?" —
    // passes no buffer, exactly as a caller wanting only a sequence
    // number does on the channel.
    let mut none: [u8; 0] = [];
    let out: &mut [u8] = if buf.is_null() || cap == 0 {
        &mut none
    } else {
        unsafe { std::slice::from_raw_parts_mut(buf, cap as usize) }
    };
    let (len, origin) = crate::settings::local_read_into(addon, file, out);
    unsafe {
        if let Some(status) = status.as_mut() {
            *status = origin.code();
        }
    }
    len.min(u32::MAX as usize) as u32
}

extern "C" fn h_settings_epoch() -> u32 {
    crate::settings::local_epoch()
}

// The two v4 theme entries. Append-only means they stay at their table
// positions forever and keep answering what the retired seven-field
// bridge answered: `accent.primary` and `surface.base`.
extern "C" fn h_theme_base(_p: *mut c_void) -> ColorC {
    color_out(crate::theme::resolved().color(crate::theme::ids::accent_primary()))
}

extern "C" fn h_theme_bg(_p: *mut c_void) -> ColorC {
    color_out(crate::theme::resolved().color(crate::theme::ids::surface_base()))
}

extern "C" fn h_vh(p: *mut c_void, v: f32) -> f32 {
    with_ctx!(p, ctx, ctx.vh(v))
}

extern "C" fn h_font_px(p: *mut c_void, v: f32) -> f32 {
    with_ctx!(p, ctx, ctx.font_px(v))
}

/// The pointer AS THIS PLUGIN MAY SEE IT ([`crate::pointer`]).
///
/// The routing has to happen on this side of the boundary, and this line
/// is why the routing exists as a value rather than as advice: a plugin
/// tests the two numbers it is handed against a rectangle it has just
/// drawn (`krect.contains(mx, my)`), and there is no version of the ABI
/// in which it could know that a window is standing over it. Handed the
/// covered position, every plugin already compiled answers "not me"
/// without a line of its own changing.
extern "C" fn h_mouse(p: *mut c_void, x: *mut f32, y: *mut f32) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    let at = ctx.mouse.at();
    unsafe {
        if !x.is_null() {
            *x = at.0;
        }
        if !y.is_null() {
            *y = at.1;
        }
    }
}

extern "C" fn h_window(p: *mut c_void, w: *mut f32, h: *mut f32) {
    let Some(ctx) = (unsafe { ctx_of(p) }) else { return };
    unsafe {
        if !w.is_null() {
            *w = ctx.w;
        }
        if !h.is_null() {
            *h = ctx.h;
        }
    }
}

extern "C" fn h_elapsed(p: *mut c_void) -> f64 {
    with_ctx!(p, ctx, ctx.t)
}

/// The host data behind the opaque handle a plugin is given.
unsafe fn host_of<'a>(p: *const c_void) -> Option<&'a Host<'a>> {
    (p as *const Host).as_ref()
}

extern "C" fn h_shell_cwd(p: *const c_void, buf: *mut u8, cap: u32) -> u32 {
    let (Some(host), false) = (unsafe { host_of(p) }, buf.is_null()) else {
        return 0;
    };
    let Some(cwd) = host.shell_cwd.as_ref() else { return 0 };
    let bytes = cwd.as_os_str().as_encoded_bytes();
    let n = bytes.len().min(cap as usize);
    unsafe { std::ptr::copy_nonoverlapping(bytes.as_ptr(), buf, n) };
    n as u32
}

extern "C" fn h_emit_sound(event: u32) {
    if let Some(e) = crate::sound::Event::from_id(event) {
        crate::sound::emit(e);
    }
}

extern "C" fn h_panel_count() -> u32 {
    crate::base::panel_count() as u32
}

const CELL_BYTES: usize = std::mem::size_of::<CellC>();

/// A polyline longer than any icon or frame in the interface. Without a
/// bound, `from_raw_parts` on a bad count is a multi-gigabyte read.
const POLYLINE_MAX: u32 = 8192;

fn cell_out(cell: &Cell) -> CellC {
    let (fg, bg) = crate::term::resolve(cell);
    let mut flags = 0u32;
    if cell.flags & FLAG_UNDERLINE != 0 {
        flags |= CELL_UNDERLINE;
    }
    if bg.is_some() {
        flags |= CELL_HAS_BG;
    }
    CellC {
        ch: cell.ch as u32,
        flags,
        width: if cell.flags & FLAG_WIDE_SPACER != 0 {
            0
        } else if cell.flags & FLAG_WIDE_LEAD != 0 {
            2
        } else {
            1
        },
        font: crate::font::FONT_MONO,
        reserved: 0,
        fg: color_out(fg),
        // The FLAG says whether there is a background; this value is
        // only ever read when it does, so nothing depends on what goes
        // here when there is none.
        bg: color_out(bg.unwrap_or_else(|| {
            crate::theme::resolved().color(crate::theme::ids::term_bg())
        })),
    }
}

extern "C" fn h_term_view(
    hp: *const c_void,
    cp: *mut c_void,
    req: *const TermReqC,
    req_size: u32,
    out: *mut TermViewC,
    out_size: u32,
) -> u32 {
    if req.is_null() || out.is_null() {
        return 0;
    }
    if (req_size as usize) < TERM_REQ_SIZE_MIN || (out_size as usize) < TERM_VIEW_SIZE_MIN {
        return 0;
    }
    // The caller's structs may be shorter than this build's, so no
    // reference to either is ever formed: only the prefix both sides
    // agree on is copied, in each direction.
    let mut r = TermReqC::empty();
    unsafe {
        std::ptr::copy_nonoverlapping(
            req as *const u8,
            &mut r as *mut TermReqC as *mut u8,
            (req_size as usize).min(std::mem::size_of::<TermReqC>()),
        );
    }
    if r.session != 0 || r.flags != 0 {
        return 0;
    }
    let Some(ctx) = (unsafe { ctx_of(cp) }) else { return 0 };

    // The cell's size is the theme's — `terminal.cell_font` floored by
    // `terminal.min_px`, in a line box `terminal.line_height` sets. It
    // used to be `vh(1.45)` with an `8.0` floor written right here, which
    // is the same arithmetic with the numbers kept out of the theme's
    // reach. The user's own multiplier still stands above the token:
    // `TermFontSize=` scales what the theme chose (§Z03, §Z18).
    let g = crate::term::Grid::measure(ctx.fonts, ctx.term_font_scale);
    let t = crate::theme::resolved();

    let mut v = TermViewC::empty();
    v.cell_w = g.cell_w;
    v.cell_h = g.cell_h;
    v.px = g.px;
    v.ascent = g.ascent;
    // One call for both axes: the grid's bound is on the pair, because
    // the pair is what gets allocated as one buffer below.
    (v.cols, v.rows) = g.span(r.area.w, r.area.h);
    v.cursor_bg = color_out(t.color(crate::theme::ids::term_cursor()));
    v.cursor_fg = color_out(t.color(crate::theme::ids::term_bg()));
    v.cursor_ch = b' ' as u32;

    let mut written = 0u32;
    if let Some(host) = unsafe { host_of(hp) } {
        v.tab_count = host.tabs.len().min(32) as u32;
        for (i, on) in host.tabs.iter().take(32).enumerate() {
            if *on {
                v.tabs |= 1u32 << i;
            }
        }
        v.tab_active = host.tab_active.min(u32::MAX as usize) as u32;

        if let Some(term) = host.term {
            v.flags |= VIEW_LIVE;
            v.view_offset = term.view_offset.min(u32::MAX as usize) as u32;
            // The id of the first delivered row, for the widget to echo
            // back in a TermSelect (the drag-vs-feed race fix, §2.7).
            let first_id = term.line_id_of_view_row(0);
            v.first_id_lo = first_id as u32;
            v.first_id_hi = (first_id >> 32) as u32;
            let vcols = (v.cols as usize).min(term.cols);
            let vrows = (v.rows as usize).min(term.rows);

            if term.cursor_visible && term.view_offset == 0 && term.cur_y < vrows {
                v.flags |= VIEW_CURSOR;
                v.cursor_col = term.cur_x.min(u32::MAX as usize) as u32;
                v.cursor_row = term.cur_y as u32;
                // Read here rather than out of the delivered cells: the
                // cursor may sit past `view_cols`, where the widget has
                // nothing to look at and has never clipped the block.
                v.cursor_ch = term
                    .view_row(term.cur_y)
                    .and_then(|row| row.get(term.cur_x))
                    .map(|c| c.ch as u32)
                    .unwrap_or(b' ' as u32);
            }

            let stride = r.cell_stride as usize;
            // The capacity is in BYTES, so whatever stride the caller
            // claims, `room * stride <= cells_bytes` — the two numbers
            // cannot disagree into a write past the end.
            let room = if r.cells.is_null() || r.cell_stride < CELL_SIZE_MIN {
                0
            } else {
                r.cells_bytes as usize / stride
            };
            let fit_rows = if vcols == 0 { 0 } else { (room / vcols).min(vrows) };
            let n = CELL_BYTES.min(stride);
            for y in 0..fit_rows {
                let row = term.view_row(y);
                // The selected span on this row, from the ONE span
                // authority the copied text also reads. Endpoints are
                // inclusive; the open end clamps to the delivered
                // width. Spacer cells inside the span carry the flag
                // too, so the wash has no gap in a wide character.
                let span = term
                    .selection_span_on_line(term.line_id_of_view_row(y))
                    .map(|(c0, c1)| (c0, c1.min(vcols.saturating_sub(1))));
                for x in 0..vcols {
                    // Scrollback rows keep the width they scrolled off
                    // with — `resize` never touches them — so a short
                    // row is PADDED here rather than trusted anywhere.
                    // An absent cell draws nothing, which is exactly
                    // what breaking out of the row used to produce.
                    let mut c = match row.and_then(|rw| rw.get(x)) {
                        Some(cell) => cell_out(cell),
                        None => CellC::absent(),
                    };
                    if let Some((c0, c1)) = span {
                        if x >= c0 && x <= c1 {
                            c.flags |= CELL_SELECTED;
                        }
                    }
                    unsafe {
                        std::ptr::copy_nonoverlapping(
                            &c as *const CellC as *const u8,
                            (r.cells as *mut u8).add((y * vcols + x) * stride),
                            n,
                        );
                    }
                }
            }
            v.view_cols = vcols as u32;
            v.view_rows = fit_rows as u32;
            if fit_rows < vrows {
                v.flags |= VIEW_TRUNCATED;
            }
            written = (fit_rows * vcols).min(u32::MAX as usize) as u32;
        }
    }

    unsafe {
        std::ptr::copy_nonoverlapping(
            &v as *const TermViewC as *const u8,
            out as *mut u8,
            (out_size as usize).min(std::mem::size_of::<TermViewC>()),
        );
    }
    written
}

/// The interface handed to every plugin. Its address must stay valid for
/// as long as any plugin is loaded, which is why it is a static.
pub fn host_api() -> &'static HostApi {
    static API: HostApi = HostApi {
        abi_version: ABI_VERSION,
        api_size: std::mem::size_of::<HostApi>() as u32,
        emit_sound: h_emit_sound,
        panel_count: h_panel_count,
        rect: h_rect,
        rect_outline: h_rect_outline,
        quad: h_quad,
        line: h_line,
        polyline: h_polyline,
        text: h_text,
        measure: h_measure,
        module_title: h_module_title,
        theme_base: h_theme_base,
        theme_bg: h_theme_bg,
        vh: h_vh,
        font_px: h_font_px,
        mouse: h_mouse,
        window: h_window,
        elapsed: h_elapsed,
        shell_cwd: h_shell_cwd,
        term_view: h_term_view,
        theme_token: h_theme_token,
        theme_color: h_theme_color,
        theme_bed: h_theme_bed,
        theme_px: h_theme_px,
        theme_flag: h_theme_flag,
        theme_enum: h_theme_enum,
        theme_class: h_theme_class,
        theme_class_state: h_theme_class_state,
        theme_epoch: h_theme_epoch,
        theme_enum_word: h_theme_enum_word,
        mask_quad: h_mask_quad,
        push_clip: h_push_clip,
        pop_clip: h_pop_clip,
        ring_fill: h_ring_fill,
        ring: h_ring,
        tooltip: h_tooltip,
        channel_publish: h_channel_publish,
        channel_read: h_channel_read,
        settings_read: h_settings_read,
        settings_epoch: h_settings_epoch,
        theme_text: h_theme_text,
        ring_glow: h_ring_glow,
        icon_register: h_icon_register,
        icon_quad: h_icon_quad,
    };
    &API
}

fn action_in(a: &ActionC) -> Action {
    let bytes = || -> Vec<u8> {
        if a.data.is_null() || a.data_len == 0 {
            Vec::new()
        } else {
            unsafe { std::slice::from_raw_parts(a.data, a.data_len as usize) }.to_vec()
        }
    };
    let path = || -> PathBuf {
        PathBuf::from(String::from_utf8_lossy(&bytes()).into_owned())
    };
    match a.kind {
        ACTION_BYTES => Action::Bytes(bytes()),
        ACTION_OPEN_DIR => Action::OpenDir(path()),
        ACTION_OPEN_FILE => Action::OpenFile(path()),
        ACTION_SELECT_TAB => Action::SelectTab(a.index as usize),
        ACTION_EXIT => Action::Exit,
        ACTION_OPEN_SETTINGS => Action::OpenSettings,
        ACTION_SCROLL_TERMINAL => Action::ScrollTerminal(a.lines),
        ACTION_TERM_SELECT => {
            // The payload rides in `data` like a path's bytes do. Only
            // the prefix both sides agree on is read; anything shorter
            // than the minimum is malformed and means nothing.
            if a.data.is_null() || (a.data_len as usize) < TERM_SELECT_SIZE_MIN {
                return Action::None;
            }
            let mut s = TermSelectC {
                op: 0, kind: 0, col: 0, row: 0, base_lo: 0, base_hi: 0,
            };
            unsafe {
                std::ptr::copy_nonoverlapping(
                    a.data,
                    &mut s as *mut TermSelectC as *mut u8,
                    (a.data_len as usize).min(std::mem::size_of::<TermSelectC>()),
                );
            }
            let kind = match s.kind {
                SELECT_KIND_WORDS => SelKind::Words,
                SELECT_KIND_LINES => SelKind::Lines,
                _ => SelKind::Cells,
            };
            let op = match s.op {
                SELECT_OP_BEGIN => SelectOp::Begin(kind),
                SELECT_OP_EXTEND => SelectOp::Extend,
                SELECT_OP_END => SelectOp::End,
                // An op from a newer interface than this build knows
                // must not corrupt the selection it cannot mean.
                _ => return Action::None,
            };
            Action::TermSelect {
                op,
                col: s.col as usize,
                row: s.row as usize,
                base: (s.base_hi as u64) << 32 | s.base_lo as u64,
            }
        }
        ACTION_PASTE_PRIMARY => Action::PastePrimary,
        ACTION_CAPTURE => Action::Capture,
        _ => Action::None,
    }
}

fn empty_action() -> ActionC {
    ActionC {
        kind: ACTION_NONE,
        index: 0,
        lines: 0,
        data: std::ptr::null(),
        data_len: 0,
    }
}

/// A widget living in a loaded plugin.
///
/// The library itself is deliberately never unloaded: function pointers
/// into it stay reachable for the life of the program, and closing it
/// while any remain is how a plugin system earns a reputation for
/// mysterious crashes.
pub struct PluginWidget {
    api: PluginApi,
    instance: *mut c_void,
}

/// The stand-in for a [`PluginApi`] entry a plugin's table ends before:
/// no chrome, so the widget gets the plain container.
extern "C" fn chrome_absent(
    _: *mut c_void,
    _: *mut c_void,
    _: *const c_void,
    _: *mut ChromeC,
    _: u32,
) -> u32 {
    0
}

/// The stand-in for a table that ends before `drag`: every phase is
/// declined, so the press falls back to the ordinary click delivery —
/// a pre-drag widget is mouse-only, never broken.
#[allow(clippy::too_many_arguments)]
extern "C" fn drag_absent(
    _: *mut c_void,
    _: u32,
    _: f32,
    _: f32,
    _: RectC,
    _: f32,
    _: f32,
    _: *mut ActionC,
) {
}

/// The stand-in for a table that ends before `pointer`: nothing of the
/// widget is ever under the pointer, so its panel keeps the ordinary
/// cursor — a pre-pointer widget is still fully clickable.
extern "C" fn pointer_absent(
    _: *mut c_void,
    _: f32,
    _: f32,
    _: RectC,
    _: f32,
    _: f32,
) -> u32 {
    0
}

/// The stand-in for a table that ends before `key`: no key is ever
/// consumed, so the host spends every one of them on itself — which is
/// what it does today for every widget there is.
extern "C" fn key_absent(
    _: *mut c_void,
    _: u32,
    _: *const u8,
    _: u32,
    _: u32,
    _: *mut ActionC,
) -> u32 {
    0
}

/// The stand-in for a table that ends before `button`: the press and the
/// release are simply not delivered. The gesture still reaches the
/// widget as a `drag` or a `click`, so a pre-button widget loses the
/// press RUNG and nothing else.
#[allow(clippy::too_many_arguments)]
extern "C" fn button_absent(
    _: *mut c_void,
    _: u32,
    _: f32,
    _: f32,
    _: RectC,
    _: f32,
    _: f32,
    _: *mut ActionC,
) {
}

impl PluginWidget {
    /// Wraps a plugin's interface. None when the plugin reports an
    /// interface this build does not speak, or cannot make an instance.
    ///
    /// # Safety
    /// `api` must come from a plugin that is loaded and stays loaded.
    pub unsafe fn new(api: *const PluginApi) -> Option<PluginWidget> {
        if api.is_null() {
            return None;
        }
        // Only the two header words are read before the size is known:
        // a shorter table than this build's must never be dereferenced
        // whole, or the copy itself reads past the plugin's static.
        let head = api as *const u32;
        let version = unsafe { *head };
        if version != ABI_VERSION {
            eprintln!(
                "nacelle: plugin speaks interface version {version}, this build speaks {} \
                 — not loaded",
                ABI_VERSION
            );
            return None;
        }
        let size = (unsafe { *head.add(1) }) as usize;
        if size < PLUGIN_API_SIZE_MIN {
            eprintln!(
                "nacelle: plugin's interface table is {size} bytes, the version-{ABI_VERSION} \
                 minimum is {PLUGIN_API_SIZE_MIN} — not loaded"
            );
            return None;
        }
        // The prefix both sides agree on; optional entries the plugin's
        // table ends before are filled with their documented defaults.
        // Byte arithmetic, never a whole-struct read — dereferencing a
        // shorter table as `PluginApi` would itself read past its end.
        let take = size.min(std::mem::size_of::<PluginApi>());
        let mut slot = std::mem::MaybeUninit::<PluginApi>::uninit();
        let table = unsafe {
            std::ptr::copy_nonoverlapping(api as *const u8, slot.as_mut_ptr() as *mut u8, take);
            if size < PLUGIN_API_HAS_CHROME {
                std::ptr::addr_of_mut!((*slot.as_mut_ptr()).chrome).write(chrome_absent);
            }
            if size < PLUGIN_API_HAS_DRAG {
                std::ptr::addr_of_mut!((*slot.as_mut_ptr()).drag).write(drag_absent);
            }
            if size < PLUGIN_API_HAS_POINTER {
                std::ptr::addr_of_mut!((*slot.as_mut_ptr()).pointer).write(pointer_absent);
            }
            if size < PLUGIN_API_HAS_KEY {
                std::ptr::addr_of_mut!((*slot.as_mut_ptr()).key).write(key_absent);
            }
            if size < PLUGIN_API_HAS_BUTTON {
                std::ptr::addr_of_mut!((*slot.as_mut_ptr()).button).write(button_absent);
            }
            slot.assume_init()
        };
        let instance = (table.create)();
        if instance.is_null() {
            eprintln!("nacelle: plugin made no widget — not loaded");
            return None;
        }
        Some(PluginWidget { api: table, instance })
    }

    /// Whether the plugin's table reaches the `chrome` entry at all.
    fn has_chrome(&self) -> bool {
        self.api.api_size as usize >= PLUGIN_API_HAS_CHROME
    }

    /// Whether the plugin's table reaches the `drag` entry.
    fn has_drag(&self) -> bool {
        self.api.api_size as usize >= PLUGIN_API_HAS_DRAG
    }

    /// Whether the plugin's table reaches the `pointer` entry.
    fn has_pointer(&self) -> bool {
        self.api.api_size as usize >= PLUGIN_API_HAS_POINTER
    }

    /// Whether the plugin's table reaches the `key` entry.
    fn has_key(&self) -> bool {
        self.api.api_size as usize >= PLUGIN_API_HAS_KEY
    }

    /// Whether the plugin's table reaches the `button` entry.
    fn has_button(&self) -> bool {
        self.api.api_size as usize >= PLUGIN_API_HAS_BUTTON
    }

    /// The press/release pair, which differ only in their phase — the
    /// same shape `drag` has, for the same reason.
    fn button(&mut self, phase: u32, x: f32, y: f32, r: Rect, host: &Host) -> Action {
        if !self.has_button() {
            return Action::None;
        }
        let mut out = empty_action();
        (self.api.button)(
            self.instance,
            phase,
            x,
            y,
            rect_out(r),
            host.window.0,
            host.window.1,
            &mut out,
        );
        match action_in(&out) {
            // The capture is `drag`'s alone (F1 §5.1). A plugin
            // answering it here has misread the contract; taking it
            // seriously would open a second capture path, which is the
            // one thing this entry promised not to be.
            Action::Capture => Action::None,
            a => a,
        }
    }
}

impl Drop for PluginWidget {
    fn drop(&mut self) {
        (self.api.destroy)(self.instance);
    }
}

impl Widget for PluginWidget {
    fn draw(&mut self, ctx: &mut Ctx, r: Rect, host: &Host) {
        // ABI 6 lets a plugin clip (`push_clip`/`pop_clip`), and a clip
        // stack is shared state: one left deep would clip every panel
        // drawn after this one, and one popped too far would take away
        // the clip its neighbours were drawn under. So the host holds
        // the stack it handed over and puts it back, whatever happened
        // in between — said once, because a broken plugin says it every
        // frame otherwise.
        let saved = ctx.dl.clip_stack();
        let c = ctx as *mut Ctx as *mut c_void;
        let h = host as *const Host as *const c_void;
        (self.api.draw)(self.instance, c, h, rect_out(r));
        if ctx.dl.clip_stack() != saved {
            crate::ui::warn_once(
                "plugin.clip",
                "a plugin widget left its clip stack unbalanced — the host \
                 restored it; the plugin's own clipping may be wrong",
            );
            ctx.dl.restore_clips(&saved);
        }
    }

    fn chrome(&mut self, ctx: &mut Ctx, host: &Host) -> crate::widget::Chrome {
        use crate::widget::{ButtonSet, Chrome};
        if !self.has_chrome() {
            return Chrome::none();
        }
        let c = ctx as *mut Ctx as *mut c_void;
        let h = host as *const Host as *const c_void;
        let mut out = ChromeC::empty();
        let n = (self.api.chrome)(
            self.instance,
            c,
            h,
            &mut out,
            std::mem::size_of::<ChromeC>() as u32,
        );
        if n == 0 {
            return Chrome::none();
        }
        let title = text_in(out.title, out.title_len);
        let right = text_in(out.right, out.right_len);
        Chrome {
            title: (!title.is_empty()).then(|| title.to_string()),
            right: (!right.is_empty()).then(|| right.to_string()),
            buttons: match out.buttons {
                CHROME_BUTTONS_CLOSE => ButtonSet::Close,
                CHROME_BUTTONS_MIN_CLOSE => ButtonSet::MinClose,
                CHROME_BUTTONS_MIN_MAX_CLOSE => ButtonSet::MinMaxClose,
                // Unknown codes from a newer plugin mean nothing here.
                _ => ButtonSet::None,
            },
            severity: (out.severity != u32::MAX).then_some(out.severity),
        }
    }

    fn click(&mut self, x: f32, y: f32, r: Rect, host: &Host) -> Action {
        let mut out = empty_action();
        (self.api.click)(
            self.instance,
            x,
            y,
            rect_out(r),
            host.window.0,
            host.window.1,
            &mut out,
        );
        action_in(&out)
    }

    fn wheel(&mut self, dy: f32, r: Rect, host: &Host) -> Action {
        let mut out = empty_action();
        (self.api.wheel)(
            self.instance,
            dy,
            rect_out(r),
            host.window.0,
            host.window.1,
            &mut out,
        );
        action_in(&out)
    }

    fn drag(&mut self, p: DragPhase, x: f32, y: f32, r: Rect, host: &Host) -> Action {
        // A table from before the entry existed declines every drag —
        // the same degradation `has_chrome` applies, and the host then
        // falls back to click delivery.
        if !self.has_drag() {
            return Action::None;
        }
        let phase = match p {
            DragPhase::Begin => DRAG_BEGIN,
            DragPhase::Move => DRAG_MOVE,
            DragPhase::End => DRAG_END,
        };
        let mut out = empty_action();
        (self.api.drag)(
            self.instance,
            phase,
            x,
            y,
            rect_out(r),
            host.window.0,
            host.window.1,
            &mut out,
        );
        action_in(&out)
    }

    fn press(&mut self, x: f32, y: f32, r: Rect, host: &Host) -> Action {
        self.button(BUTTON_PRESS, x, y, r, host)
    }

    fn release(&mut self, x: f32, y: f32, r: Rect, host: &Host) -> Action {
        self.button(BUTTON_RELEASE, x, y, r, host)
    }

    /// The focused key, as the boundary carries it: a scalar OR one of
    /// the [`keys`] words, plus the modifier bits.
    ///
    /// Two fields of a [`crate::focus::KeyEv`] stay on this side.
    /// `text` does not cross — the boundary carries the KEY, so a
    /// platform that expanded a press into several characters (an IME
    /// commit) reaches a plugin through no entry at all, rather than
    /// through this one with the tail cut off. `repeat` does not cross
    /// either: a held key arrives as another key, which is what
    /// auto-repeat is for, and a widget that must act once per physical
    /// press cannot be written against this entry.
    fn key(&mut self, ev: &crate::focus::KeyEv) -> Option<Action> {
        if !self.has_key() {
            return None;
        }
        let name = keys::name_of(ev.key);
        let ch = match ev.key {
            crate::focus::Key::Char(c) => c as u32,
            _ => 0,
        };
        // A key this build does not carry by name and cannot spell as a
        // character is not an event to send: an empty name with a zero
        // scalar means nothing, and the contract says so.
        if name.is_none() && ch == 0 {
            return None;
        }
        let label = name.unwrap_or("");
        let mut out = empty_action();
        let taken = (self.api.key)(
            self.instance,
            ch,
            label.as_ptr(),
            label.len() as u32,
            ev.mods.bits() as u32,
            &mut out,
        );
        (taken != 0).then(|| action_in(&out))
    }

    fn pointer(&mut self, x: f32, y: f32, r: Rect, window: (f32, f32)) -> bool {
        // A table from before the entry existed has nothing under the
        // pointer — the same degradation `has_drag` applies.
        if !self.has_pointer() {
            return false;
        }
        (self.api.pointer)(self.instance, x, y, rect_out(r), window.0, window.1) != 0
    }

    fn grid(&self) -> Option<(usize, usize)> {
        let (mut c, mut r) = (0u32, 0u32);
        (self.api.grid)(self.instance, &mut c, &mut r);
        (c > 0 && r > 0).then_some((c as usize, r as usize))
    }

    fn sizing(&mut self, ctx: &mut Ctx, host: &Host) -> Sizing {
        let ctx_ptr = ctx as *mut Ctx as *mut c_void;
        let host_ptr = host as *const Host as *const c_void;
        let v = (self.api.sizing)(self.instance, ctx_ptr, host_ptr);
        if v.is_finite() && v > 0.0 {
            Sizing::Content(v)
        } else if v == SIZING_ROWS {
            Sizing::Rows
        } else {
            // Not finite, zero, or a value from a newer interface than
            // this build knows: the reference box is the answer that is
            // never wrong, only unremarkable.
            Sizing::Reference
        }
    }

    /// A plugin draws with baked token values: `h_theme_px` answers
    /// device pixels with no panel scale in them, deliberately — u2
    /// §2.12 keeps a control the same size wherever its panel is put.
    /// So a plugin's measured content does not shrink with its box, and
    /// the host must publish its `Content` want unscaled.
    fn scales_with_panel(&self) -> bool {
        false
    }

    fn key_feedback(&mut self, ch: Option<char>, label: Option<&str>) {
        let l = label.unwrap_or("");
        (self.api.key_feedback)(
            self.instance,
            ch.map(|c| c as u32).unwrap_or(0),
            l.as_ptr(),
            l.len() as u32,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Numbers a plugin passes index fixed arrays on this side, so a
    /// wrong one must be clamped rather than trusted. Getting this wrong
    /// is a read past the end of an array, from a typo in a widget.
    #[test]
    fn out_of_range_numbers_from_a_plugin_are_clamped() {
        // Every slot the master numbers is passed through untouched —
        // eight of them since §5.16's face blocks reached the atlas, and
        // a plugin naming `ui_medium` (2) must not be clamped to `mono`.
        for slot in 0..FONT_COUNT {
            assert_eq!(font_in(slot as u32), slot, "face slot {slot}");
        }
        // Past the end, and absurd, and wrapped: all land inside.
        assert_eq!(font_in(FONT_COUNT as u32), FONT_COUNT - 1);
        assert_eq!(font_in(9999), FONT_COUNT - 1);
        assert_eq!(font_in(u32::MAX), FONT_COUNT - 1);

        // A polyline count is a length for from_raw_parts, so the
        // guard has to reject before the slice is ever formed. A null
        // pointer with a huge count must simply do nothing.
        h_polyline(
            std::ptr::null_mut(),
            std::ptr::null(),
            u32::MAX,
            1.0,
            ColorC { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
            false,
        );

        // The mask quad with no context and no arrays: nothing, not a
        // read through null (the geometry itself is DrawList's test).
        h_mask_quad(
            std::ptr::null_mut(),
            std::ptr::null(),
            std::ptr::null(),
            ColorC { r: 1.0, g: 1.0, b: 1.0, a: 1.0 },
            MASK_QUAD_ADD,
        );
    }

    /// The enum WORD crosses like `shell_cwd` does: UTF-8 into the
    /// caller's buffer, a short buffer gets a prefix, and every bad
    /// input answers 0 rather than crashing.
    #[test]
    fn an_enum_word_crosses_as_bytes() {
        let _ = crate::theme::resolved(); // first use loads the master
        let name = b"type.title.window.case";
        let id = h_theme_token(name.as_ptr(), name.len() as u32);
        assert_ne!(id, u32::MAX, "the master must declare type.title.window.case");
        let expect = crate::theme::enum_word_of(tid(id)).expect("an enum token has a word");
        assert!(!expect.is_empty());

        let mut buf = [0u8; 64];
        let n = h_theme_enum_word(std::ptr::null_mut(), id, buf.as_mut_ptr(), 64) as usize;
        assert_eq!(&buf[..n], expect.as_bytes());

        // A buffer with less room than the word gets its prefix.
        let mut small = [0u8; 2];
        let n = h_theme_enum_word(std::ptr::null_mut(), id, small.as_mut_ptr(), 2) as usize;
        assert_eq!(n, expect.len().min(2));
        assert_eq!(&small[..n], &expect.as_bytes()[..n]);

        // Null buffer, no room, unknown id: 0, never a crash.
        assert_eq!(h_theme_enum_word(std::ptr::null_mut(), id, std::ptr::null_mut(), 64), 0);
        assert_eq!(h_theme_enum_word(std::ptr::null_mut(), id, buf.as_mut_ptr(), 0), 0);
        assert_eq!(h_theme_enum_word(std::ptr::null_mut(), u32::MAX, buf.as_mut_ptr(), 64), 0);
    }

    /// A TEXT token crosses the same way, and this is the road every
    /// compiled widget's trim marker now takes: the launcher's tile
    /// captions and the file browser's names both call
    /// `HostApi::theme_text_of` and appended `"\u{2026}"` out of their
    /// own source before it existed.
    #[test]
    fn a_text_token_crosses_as_bytes() {
        let _ = crate::theme::resolved();
        let name = b"type.ellipsis";
        let id = h_theme_token(name.as_ptr(), name.len() as u32);
        assert_ne!(id, u32::MAX, "the master must declare type.ellipsis");
        let expect = crate::theme::diagnostics()
            .text("type.ellipsis")
            .expect("the master states a trim marker")
            .to_string();
        assert!(!expect.is_empty());

        let mut buf = [0u8; 64];
        let n = h_theme_text(std::ptr::null_mut(), id, buf.as_mut_ptr(), 64) as usize;
        assert_eq!(&buf[..n], expect.as_bytes());

        // The plugin-side shorthand answers the same string by NAME,
        // which is the only form a widget ever writes.
        let api = host_api();
        assert_eq!(api.theme_text_of(std::ptr::null_mut(), "type.ellipsis"), expect);
        // ...and a host from before the entry answers the empty string —
        // the same answer a theme that declares no marker gives, so a
        // widget cannot tell the two apart and grow a fallback for one.
        let old = HostApi {
            api_size: crate::runtime::HOST_API_HAS_SETTINGS as u32,
            ..*api
        };
        assert_eq!(old.theme_text_of(std::ptr::null_mut(), "type.ellipsis"), "");

        // An ENUM token is not a text token and answers nothing rather
        // than its word: the two kinds are asked for separately because
        // they are separate questions.
        let e = b"type.title.window.case";
        let eid = h_theme_token(e.as_ptr(), e.len() as u32);
        assert_eq!(h_theme_text(std::ptr::null_mut(), eid, buf.as_mut_ptr(), 64), 0);

        // A short buffer gets a prefix; null buffer, no room and an
        // unknown id all answer 0 rather than crashing.
        let mut small = [0u8; 1];
        let n = h_theme_text(std::ptr::null_mut(), id, small.as_mut_ptr(), 1) as usize;
        assert_eq!(n, expect.len().min(1));
        assert_eq!(h_theme_text(std::ptr::null_mut(), id, std::ptr::null_mut(), 64), 0);
        assert_eq!(h_theme_text(std::ptr::null_mut(), id, buf.as_mut_ptr(), 0), 0);
        assert_eq!(h_theme_text(std::ptr::null_mut(), u32::MAX, buf.as_mut_ptr(), 64), 0);
    }

    /// The host table grows at the END only, and its `api_size` is what
    /// says how far it reaches — the version-6 growth contract, from the
    /// side that fills the table.
    #[test]
    fn the_host_table_grows_at_the_end_only() {
        use crate::runtime::{
            HOST_API_HAS_CHANNEL, HOST_API_HAS_CLIP, HOST_API_HAS_ENUM_WORD, HOST_API_HAS_ICON,
            HOST_API_HAS_MASK_QUAD, HOST_API_HAS_RING, HOST_API_HAS_RING_GLOW,
            HOST_API_HAS_SETTINGS, HOST_API_HAS_THEME_TEXT, HOST_API_HAS_TOOLTIP,
            HOST_API_SIZE_MIN,
        };
        let api = host_api();
        assert_eq!(api.api_size as usize, std::mem::size_of::<HostApi>());
        assert!(api.has_theme_enum_word());
        assert!(api.has_mask_quad());
        assert!(api.has_clip());
        assert!(api.has_ring());
        assert!(api.has_tooltip());
        assert!(api.has_channel());
        assert!(api.has_settings());
        assert!(api.has_theme_text());
        assert!(api.has_ring_glow());
        assert!(api.has_icon());
        // The appended entries sit past the mandatory prefix, in order,
        // with the icon pair (K8) the current end of the table.
        assert!(HOST_API_SIZE_MIN < HOST_API_HAS_ENUM_WORD);
        assert!(HOST_API_HAS_ENUM_WORD < HOST_API_HAS_MASK_QUAD);
        assert!(HOST_API_HAS_MASK_QUAD < HOST_API_HAS_CLIP);
        assert!(HOST_API_HAS_CLIP < HOST_API_HAS_RING);
        assert!(HOST_API_HAS_RING < HOST_API_HAS_TOOLTIP);
        assert!(HOST_API_HAS_TOOLTIP < HOST_API_HAS_CHANNEL);
        assert!(HOST_API_HAS_CHANNEL < HOST_API_HAS_SETTINGS);
        assert!(HOST_API_HAS_SETTINGS < HOST_API_HAS_THEME_TEXT);
        assert!(HOST_API_HAS_THEME_TEXT < HOST_API_HAS_RING_GLOW);
        assert!(HOST_API_HAS_RING_GLOW < HOST_API_HAS_ICON);
        assert_eq!(HOST_API_HAS_ICON, std::mem::size_of::<HostApi>());
        // A host that stopped at the version-6 minimum answers none of
        // them, which is what a plugin's `has_*` gate is for.
        let old = HostApi { api_size: HOST_API_SIZE_MIN as u32, ..*api };
        assert!(!old.has_theme_enum_word());
        assert!(!old.has_mask_quad());
        assert!(!old.has_clip());
        assert!(!old.has_ring());
        assert!(!old.has_tooltip());
        assert!(!old.has_channel());
        assert!(!old.has_settings());
        assert!(!old.has_theme_text());
        assert!(!old.has_ring_glow());
        assert!(!old.has_icon());
        // A host that carries the text entry but stops there — the
        // table as it stood before THIS growth — answers everything
        // through `theme_text` and nothing past it.
        let pre_ring_glow = HostApi { api_size: HOST_API_HAS_THEME_TEXT as u32, ..*api };
        assert!(pre_ring_glow.has_theme_text());
        assert!(!pre_ring_glow.has_ring_glow());
        assert!(!pre_ring_glow.has_icon());
        // The table as it stood before the icon pair — a shipped plugin
        // measured exactly this, and the new gate must answer false for
        // it or that plugin reads past the end of the table.
        let pre_icon = HostApi { api_size: HOST_API_HAS_RING_GLOW as u32, ..*api };
        assert!(pre_icon.has_ring_glow());
        assert!(!pre_icon.has_icon());
        // Half the icon pair is no icon: a table that reaches
        // `icon_register` and stops must be called for neither entry —
        // a plugin that could ask for an id and never draw it is no
        // better off than one with neither call.
        let half_icon = HostApi {
            api_size: (std::mem::offset_of!(HostApi, icon_quad)) as u32,
            ..*api
        };
        assert!(!half_icon.has_icon());
        // And a host from before the ring pair keeps the clips.
        let pre_ring = HostApi { api_size: HOST_API_HAS_CLIP as u32, ..*api };
        assert!(pre_ring.has_clip());
        assert!(!pre_ring.has_ring());
        // The table as it stood before THIS growth — everything through
        // the rings, and none of the four holes it closes. This is the
        // old table a shipped plugin measured, and every new gate must
        // answer false for it or that plugin reads past the end.
        // The table as it stood before the TEXT entry — a plugin built
        // against it must read the empty string rather than past the end.
        let pre_text = HostApi { api_size: HOST_API_HAS_SETTINGS as u32, ..*api };
        assert!(pre_text.has_settings());
        assert!(!pre_text.has_theme_text());
        let pre_growth = HostApi { api_size: HOST_API_HAS_RING as u32, ..*api };
        assert!(pre_growth.has_theme_enum_word());
        assert!(pre_growth.has_mask_quad());
        assert!(pre_growth.has_clip());
        assert!(pre_growth.has_ring());
        assert!(!pre_growth.has_tooltip());
        assert!(!pre_growth.has_channel());
        // Half the channel pair is no channel: a table that reaches
        // `channel_publish` and stops must be called for neither, or a
        // widget would state facts nothing in the process can read.
        let half_channel = HostApi {
            api_size: (std::mem::offset_of!(HostApi, channel_read)) as u32,
            ..*api
        };
        assert!(half_channel.has_tooltip());
        assert!(!half_channel.has_channel());
        // The table as it stood before the settings pair — a shipped
        // plugin measured exactly this, and the new gate must answer
        // false for it or that plugin reads past the end of the table.
        let pre_settings = HostApi { api_size: HOST_API_HAS_CHANNEL as u32, ..*api };
        assert!(pre_settings.has_channel());
        assert!(!pre_settings.has_settings());
        // Half the settings pair is no settings: a table that reaches
        // the read and stops leaves a plugin caching a parsed document
        // with no way to learn that the file under it changed.
        let half_settings = HostApi {
            api_size: (std::mem::offset_of!(HostApi, settings_epoch)) as u32,
            ..*api
        };
        assert!(half_settings.has_channel());
        assert!(!half_settings.has_settings());
        // A host from before the clip pair — the whole of ABI 6 as it
        // stood — still answers everything it did answer.
        let pre_clip = HostApi { api_size: HOST_API_HAS_MASK_QUAD as u32, ..*api };
        assert!(pre_clip.has_theme_enum_word());
        assert!(pre_clip.has_mask_quad());
        assert!(!pre_clip.has_clip(), "the pair is gated as one, and it is not there");
        // Half a pair is no pair: a table that reaches `push_clip` and
        // stops must not be called for either.
        let half = HostApi {
            api_size: (std::mem::offset_of!(HostApi, pop_clip)) as u32,
            ..*api
        };
        assert!(!half.has_clip());
    }

    /// The clip stack a plugin may reach is shared state, so the host
    /// holds the one it handed over and puts it back — whether the
    /// plugin left it too deep or popped what it never pushed. This is
    /// the arithmetic `PluginWidget::draw` performs around every call;
    /// the forwarding itself is two lines and needs a real context,
    /// which a windowless test has no business building.
    #[test]
    fn an_unbalanced_plugin_cannot_take_its_neighbours_clip() {
        let mut dl = crate::draw::DrawList::new();
        // The host clips a panel, then hands the list over.
        dl.push_clip(10.0, 10.0, 100.0, 100.0);
        let saved = dl.clip_stack();
        assert_eq!(saved, vec![[10.0, 10.0, 100.0, 100.0]]);

        // A plugin that forgot its pop.
        dl.push_clip(20.0, 20.0, 10.0, 10.0);
        assert_ne!(dl.clip_stack(), saved);
        dl.restore_clips(&saved);
        assert_eq!(dl.clip_stack(), saved);

        // A plugin that popped more than it pushed: without the restore
        // every panel drawn after it would lose the host's clip.
        dl.pop_clip();
        dl.pop_clip();
        assert!(dl.clip_stack().is_empty());
        dl.restore_clips(&saved);
        assert_eq!(dl.clip_stack(), saved);

        // A balanced plugin costs nothing: the stack compares equal and
        // no run is stamped for a restore that changes nothing.
        let runs_before = dl.run_count();
        dl.restore_clips(&saved);
        assert_eq!(dl.run_count(), runs_before);
    }

    /// Neither clip entry dereferences a null context — the rule every
    /// entry in this table follows.
    #[test]
    fn the_clip_entries_survive_a_null_context() {
        h_push_clip(std::ptr::null_mut(), RectC { x: 0.0, y: 0.0, w: 1.0, h: 1.0 });
        h_pop_clip(std::ptr::null_mut());
    }

    extern "C" fn t_create() -> *mut c_void {
        1 as *mut c_void
    }
    extern "C" fn t_create_none() -> *mut c_void {
        std::ptr::null_mut()
    }
    extern "C" fn t_destroy(_: *mut c_void) {}
    extern "C" fn t_draw(_: *mut c_void, _: *mut c_void, _: *const c_void, _: RectC) {}
    extern "C" fn t_click(
        _: *mut c_void, _: f32, _: f32, _: RectC, _: f32, _: f32, _: *mut ActionC,
    ) {}
    extern "C" fn t_wheel(_: *mut c_void, _: f32, _: RectC, _: f32, _: f32, _: *mut ActionC) {}
    extern "C" fn t_grid(_: *mut c_void, _: *mut u32, _: *mut u32) {}
    extern "C" fn t_key(_: *mut c_void, _: u32, _: *const u8, _: u32) {}
    extern "C" fn t_sizing(_: *mut c_void, _: *mut c_void, _: *const c_void) -> f32 {
        SIZING_ROWS
    }
    extern "C" fn t_chrome(
        _: *mut c_void,
        _: *mut c_void,
        _: *const c_void,
        out: *mut ChromeC,
        out_size: u32,
    ) -> u32 {
        static TITLE: &[u8] = b"FILESYSTEM";
        static RIGHT: &[u8] = b"/var/home";
        let Some(out) = (unsafe { out.as_mut() }) else { return 0 };
        out.title = TITLE.as_ptr();
        out.title_len = TITLE.len() as u32;
        out.right = RIGHT.as_ptr();
        out.right_len = RIGHT.len() as u32;
        (out_size as usize).min(std::mem::size_of::<ChromeC>()) as u32
    }

    /// Answers a TermSelect the way the shell widget does: a payload
    /// the plugin owns, echoed base, phase mapped to an op.
    extern "C" fn t_drag(
        _: *mut c_void,
        phase: u32,
        x: f32,
        y: f32,
        _: RectC,
        _: f32,
        _: f32,
        out: *mut ActionC,
    ) {
        static SEL: TermSelectC = TermSelectC {
            op: 0, kind: 0, col: 0, row: 0, base_lo: 0, base_hi: 0,
        };
        // The test payload is static and immutable; a real widget keeps
        // a per-instance one, valid until its next call.
        let mut s = SEL;
        s.op = phase; // DRAG_* and SELECT_OP_* line up by construction
        s.col = x as u32;
        s.row = y as u32;
        s.base_lo = 7;
        s.base_hi = 1;
        let Some(out) = (unsafe { out.as_mut() }) else { return };
        // Leak-free because 'static: the boxed payload lives for the
        // test process, standing in for the plugin's instance field.
        let payload: &'static TermSelectC = Box::leak(Box::new(s));
        out.kind = ACTION_TERM_SELECT;
        out.data = payload as *const TermSelectC as *const u8;
        out.data_len = std::mem::size_of::<TermSelectC>() as u32;
    }

    /// Answers the hover question from the widget's own geometry: the
    /// left half of the rect is a control, the right half is not.
    extern "C" fn t_pointer(
        _: *mut c_void,
        x: f32,
        _: f32,
        r: RectC,
        _: f32,
        _: f32,
    ) -> u32 {
        u32::from(x < r.x + r.w / 2.0)
    }

    /// Answers the focused key the way a text field would: Enter is
    /// consumed and asks the application for something, Ctrl+A is
    /// consumed and asks for nothing, everything else is left alone.
    extern "C" fn t_key_focused(
        _: *mut c_void,
        ch: u32,
        label: *const u8,
        label_len: u32,
        mods: u32,
        out: *mut ActionC,
    ) -> u32 {
        let name = text_in(label, label_len);
        if name == keys::ENTER {
            if let Some(out) = unsafe { out.as_mut() } {
                out.kind = ACTION_EXIT;
            }
            return 1;
        }
        if ch == 'a' as u32 && mods == crate::runtime::MODS_CTRL {
            return 1; // consumed, nothing asked
        }
        0
    }

    /// Answers the two phases differently, so a test can tell which one
    /// arrived — and answers CAPTURE on the press, which the bridge must
    /// refuse to pass on (the capture is `drag`'s alone).
    #[allow(clippy::too_many_arguments)]
    extern "C" fn t_button(
        _: *mut c_void,
        phase: u32,
        _: f32,
        _: f32,
        _: RectC,
        _: f32,
        _: f32,
        out: *mut ActionC,
    ) {
        let Some(out) = (unsafe { out.as_mut() }) else { return };
        if phase == crate::runtime::BUTTON_PRESS {
            out.kind = ACTION_CAPTURE;
        } else {
            out.kind = ACTION_SELECT_TAB;
            out.index = 7;
        }
    }

    fn t_api() -> PluginApi {
        PluginApi {
            abi_version: ABI_VERSION,
            api_size: std::mem::size_of::<PluginApi>() as u32,
            create: t_create,
            destroy: t_destroy,
            draw: t_draw,
            click: t_click,
            wheel: t_wheel,
            grid: t_grid,
            key_feedback: t_key,
            sizing: t_sizing,
            chrome: t_chrome,
            drag: t_drag,
            pointer: t_pointer,
            key: t_key_focused,
            button: t_button,
        }
    }

    fn t_host() -> Host<'static> {
        static SNAP: std::sync::OnceLock<crate::telemetry::Snapshot> =
            std::sync::OnceLock::new();
        Host {
            snap: SNAP.get_or_init(crate::telemetry::Snapshot::default),
            term: None,
            tabs: &[true],
            tab_active: 0,
            shell_cwd: None,
            t: 0.0,
            window: (800.0, 600.0),
        }
    }

    #[test]
    fn a_plugin_speaking_another_version_is_refused() {
        let mut api = t_api();
        api.abi_version = ABI_VERSION + 1;
        assert!(unsafe { PluginWidget::new(&api) }.is_none());
        assert!(unsafe { PluginWidget::new(std::ptr::null()) }.is_none());
        // The right version is accepted.
        api.abi_version = ABI_VERSION;
        assert!(unsafe { PluginWidget::new(&api) }.is_some());
    }

    #[test]
    fn a_plugin_that_makes_nothing_is_refused() {
        let api = PluginApi { create: t_create_none, ..t_api() };
        assert!(unsafe { PluginWidget::new(&api) }.is_none());
    }

    /// `api_size` is how the table grows without another version break:
    /// a plugin whose table ends before `chrome` gets the documented
    /// default (no chrome), one that reaches it is asked.
    #[test]
    fn a_shorter_table_means_no_chrome_a_full_one_answers() {
        use crate::runtime::{PLUGIN_API_HAS_CHROME, PLUGIN_API_SIZE_MIN};
        let short = PluginApi { api_size: PLUGIN_API_SIZE_MIN as u32, ..t_api() };
        let w = unsafe { PluginWidget::new(&short) }.expect("a pre-chrome table still loads");
        assert!(!w.has_chrome());

        let full = t_api();
        assert!(full.api_size as usize >= PLUGIN_API_HAS_CHROME);
        let w = unsafe { PluginWidget::new(&full) }.expect("full table loads");
        assert!(w.has_chrome());

        // A table shorter than the version's own minimum is refused.
        let broken = PluginApi { api_size: 8, ..t_api() };
        assert!(unsafe { PluginWidget::new(&broken) }.is_none());
    }

    /// `api_size` gates `drag` exactly like `chrome`: a pre-drag table
    /// loads, declines every drag, and the click path is what remains.
    #[test]
    fn a_table_without_drag_declines_the_capture() {
        use crate::runtime::{PLUGIN_API_HAS_CHROME, PLUGIN_API_HAS_DRAG};
        // The appended entries sit past the mandatory prefix, in order.
        assert!(PLUGIN_API_HAS_CHROME < PLUGIN_API_HAS_DRAG);

        let host = Host {
            snap: &crate::telemetry::Snapshot::default(),
            term: None,
            tabs: &[true],
            tab_active: 0,
            shell_cwd: None,
            t: 0.0,
            window: (800.0, 600.0),
        };
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);

        let short = PluginApi { api_size: PLUGIN_API_HAS_CHROME as u32, ..t_api() };
        let mut w = unsafe { PluginWidget::new(&short) }.expect("a pre-drag table loads");
        assert!(!w.has_drag());
        assert_eq!(w.drag(DragPhase::Begin, 5.0, 5.0, r, &host), Action::None);

        // A full table is asked, and the payload survives the crossing:
        // op from the phase, cells from the coordinates, the echoed
        // 64-bit base reassembled from its two words.
        let mut w = unsafe { PluginWidget::new(&t_api()) }.expect("full table loads");
        assert!(w.has_drag());
        assert_eq!(
            w.drag(DragPhase::Begin, 3.0, 2.0, r, &host),
            Action::TermSelect {
                op: SelectOp::Begin(SelKind::Cells),
                col: 3,
                row: 2,
                base: (1u64 << 32) | 7,
            }
        );
        assert_eq!(
            w.drag(DragPhase::End, 4.0, 2.0, r, &host),
            Action::TermSelect { op: SelectOp::End, col: 4, row: 2, base: (1u64 << 32) | 7 }
        );
    }

    /// `api_size` gates `pointer` exactly like `drag`: a table from
    /// before the entry loads, is never asked, and its panel keeps the
    /// ordinary cursor; a full one answers from its own rectangles.
    #[test]
    fn a_table_without_pointer_never_claims_the_cursor() {
        use crate::runtime::{PLUGIN_API_HAS_DRAG, PLUGIN_API_HAS_POINTER};
        // The appended entries sit past the mandatory prefix, in order;
        // `pointer` is no longer the table's end (`button` is, and its
        // own test says so), which is exactly what appending means.
        assert!(PLUGIN_API_HAS_DRAG < PLUGIN_API_HAS_POINTER);
        assert!(PLUGIN_API_HAS_POINTER <= std::mem::size_of::<PluginApi>());

        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let short = PluginApi { api_size: PLUGIN_API_HAS_DRAG as u32, ..t_api() };
        let mut w = unsafe { PluginWidget::new(&short) }.expect("a pre-pointer table loads");
        assert!(!w.has_pointer());
        assert!(!w.pointer(5.0, 5.0, r, (800.0, 600.0)));

        let mut w = unsafe { PluginWidget::new(&t_api()) }.expect("full table loads");
        assert!(w.has_pointer());
        assert!(w.pointer(5.0, 5.0, r, (800.0, 600.0)), "over the control");
        assert!(!w.pointer(95.0, 5.0, r, (800.0, 600.0)), "past it");
    }

    /// `api_size` gates `key` like every append before it: a table from
    /// before the entry loads, is never asked, and every key stays the
    /// host's — which is exactly the behaviour of every plugin shipped
    /// so far. A full table is asked, answers whether it CONSUMED the
    /// key, and the modifiers arrive with it.
    #[test]
    fn a_table_without_key_leaves_every_key_to_the_host() {
        use crate::focus::{Key, KeyEv, Mods};
        use crate::runtime::{PLUGIN_API_HAS_KEY, PLUGIN_API_HAS_POINTER};
        assert!(PLUGIN_API_HAS_POINTER < PLUGIN_API_HAS_KEY);

        let ev = |key, mods| KeyEv { key, mods, repeat: false, text: None };

        let short = PluginApi { api_size: PLUGIN_API_HAS_POINTER as u32, ..t_api() };
        let mut w = unsafe { PluginWidget::new(&short) }.expect("a pre-key table loads");
        assert!(!w.has_key());
        assert_eq!(w.key(&ev(Key::Enter, Mods::NONE)), None);

        let mut w = unsafe { PluginWidget::new(&t_api()) }.expect("full table loads");
        assert!(w.has_key());
        // A named key, spelled with the contract's word, consumed and
        // asking the application for something.
        assert_eq!(w.key(&ev(Key::Enter, Mods::NONE)), Some(Action::Exit));
        // The modifiers cross: without them this is an ordinary 'a'.
        assert_eq!(w.key(&ev(Key::Char('a'), Mods::CTRL)), Some(Action::None));
        assert_eq!(w.key(&ev(Key::Char('a'), Mods::NONE)), None, "a plain 'a' is not the chord");
        // A key the boundary carries neither by name nor as a scalar is
        // not an event at all, so the widget is not even asked.
        assert_eq!(w.key(&ev(Key::F(6), Mods::NONE)), None);
        assert_eq!(w.key(&ev(Key::Menu, Mods::NONE)), None);
    }

    /// `api_size` gates `button` the same way, and the press/release
    /// pair must not become a second capture path: CAPTURE answered from
    /// either phase means nothing, exactly as it does from `click`.
    #[test]
    fn press_and_release_arrive_but_never_capture() {
        use crate::runtime::{PLUGIN_API_HAS_BUTTON, PLUGIN_API_HAS_KEY};
        // `button` is the current end of the table.
        assert!(PLUGIN_API_HAS_KEY < PLUGIN_API_HAS_BUTTON);
        assert_eq!(PLUGIN_API_HAS_BUTTON, std::mem::size_of::<PluginApi>());

        let host = t_host();
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);

        let short = PluginApi { api_size: PLUGIN_API_HAS_KEY as u32, ..t_api() };
        let mut w = unsafe { PluginWidget::new(&short) }.expect("a pre-button table loads");
        assert!(!w.has_button());
        assert_eq!(w.press(5.0, 5.0, r, &host), Action::None);
        assert_eq!(w.release(5.0, 5.0, r, &host), Action::None);

        let mut w = unsafe { PluginWidget::new(&t_api()) }.expect("full table loads");
        assert!(w.has_button());
        // The press answers CAPTURE, which the bridge refuses to pass
        // on: the capture is `drag`'s, and a second path to it is the
        // one thing this entry promised not to be.
        assert_eq!(w.press(5.0, 5.0, r, &host), Action::None);
        // The release's own answer crosses untouched, and the two
        // phases are told apart.
        assert_eq!(w.release(5.0, 5.0, r, &host), Action::SelectTab(7));
    }

    /// The channel entries are reachable from a plugin and defensive
    /// about what crosses: an unknown topic is nothing, a null buffer is
    /// a sequence query, and neither is a crash.
    #[test]
    fn the_channel_crosses_as_bytes_under_a_named_topic() {
        let topic = b"test.abi.pick";
        let value = b"Utility";
        let seq = h_channel_publish(
            topic.as_ptr(),
            topic.len() as u32,
            value.as_ptr(),
            value.len() as u32,
        );
        assert!(seq >= 1, "a published value gets a sequence number");

        let mut buf = [0u8; 32];
        let mut got = 0u64;
        let n = h_channel_read(
            topic.as_ptr(),
            topic.len() as u32,
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut got,
        );
        assert_eq!(&buf[..n as usize], value);
        assert_eq!(got, seq);

        // A buffer shorter than the value is told the FULL length, so
        // truncation is detectable rather than silent.
        let mut small = [0u8; 3];
        let n = h_channel_read(
            topic.as_ptr(),
            topic.len() as u32,
            small.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
        );
        assert_eq!(n as usize, value.len());
        assert_eq!(&small, b"Uti");

        // No buffer at all: a sequence query, which is how a reader asks
        // "has anything changed" without copying.
        let mut got = 0u64;
        assert_eq!(
            h_channel_read(
                topic.as_ptr(),
                topic.len() as u32,
                std::ptr::null_mut(),
                0,
                &mut got
            ),
            value.len() as u32
        );
        assert_eq!(got, seq);

        // A topic nobody published to: 0 length, 0 sequence — absent,
        // which is a different thing from an empty value.
        let unknown = b"test.abi.silent";
        let mut got = 7u64;
        assert_eq!(
            h_channel_read(
                unknown.as_ptr(),
                unknown.len() as u32,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut got
            ),
            0
        );
        assert_eq!(got, 0);

        // Nothing here dereferences a null: an empty topic is refused
        // both ways, and a null payload publishes an empty value.
        assert_eq!(h_channel_publish(std::ptr::null(), 0, value.as_ptr(), 7), 0);
        assert_eq!(
            h_channel_read(std::ptr::null(), 0, buf.as_mut_ptr(), 32, std::ptr::null_mut()),
            0
        );
        let empty = b"test.abi.empty";
        assert!(h_channel_publish(empty.as_ptr(), empty.len() as u32, std::ptr::null(), 0) >= 1);
        let mut got = 0u64;
        assert_eq!(
            h_channel_read(
                empty.as_ptr(),
                empty.len() as u32,
                buf.as_mut_ptr(),
                32,
                &mut got
            ),
            0,
            "an empty value is zero bytes long"
        );
        assert!(got >= 1, "but it HAS a sequence number, which is what makes it present");
    }

    /// The settings entry is defensive about everything that crosses it,
    /// and never lets a name be a path.
    ///
    /// What is NOT asserted here is which file answered: the settings
    /// directories are process-wide, `settings`'s own test installs and
    /// removes some, and the two tests run in parallel under the default
    /// harness. So this checks the properties that hold whatever is
    /// installed — that a status is always written, that it is always
    /// one of the four, and that a name with a separator in it is
    /// refused whether or not such a file could exist.
    #[test]
    fn a_settings_name_is_a_name_and_never_a_path() {
        use crate::runtime::{
            SETTINGS_ABSENT, SETTINGS_MALFORMED, SETTINGS_OK, SETTINGS_REFUSED,
        };
        let addon = b"test-abi-addon";
        let mut buf = [0u8; 64];

        // A status is written on every path, so a caller never reads a
        // stale one and mistakes it for an answer.
        let mut status = 99u32;
        h_settings_read(
            addon.as_ptr(),
            addon.len() as u32,
            std::ptr::null(),
            0,
            buf.as_mut_ptr(),
            buf.len() as u32,
            &mut status,
        );
        assert!(
            matches!(
                status,
                SETTINGS_OK | SETTINGS_ABSENT | SETTINGS_MALFORMED | SETTINGS_REFUSED
            ),
            "a status must always be one of the four"
        );

        // Every escape a plugin could attempt is refused at the name,
        // which is what makes "the host holds the only path" true.
        for bad in [&b".."[..], b"../../etc/shadow", b"a/b", b"Addon", b"a.ron"] {
            let mut status = 99u32;
            let n = h_settings_read(
                bad.as_ptr(),
                bad.len() as u32,
                std::ptr::null(),
                0,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut status,
            );
            assert_eq!(status, SETTINGS_REFUSED, "{bad:?} must be refused");
            assert_eq!(n, 0, "a refused name delivers no bytes");
        }

        // Nothing dereferences a null: no name, no buffer, no status.
        let mut status = 99u32;
        assert_eq!(
            h_settings_read(
                std::ptr::null(),
                0,
                std::ptr::null(),
                0,
                buf.as_mut_ptr(),
                buf.len() as u32,
                &mut status
            ),
            0
        );
        assert_eq!(status, SETTINGS_REFUSED, "an empty addon name is not a name");
        h_settings_read(
            addon.as_ptr(),
            addon.len() as u32,
            std::ptr::null(),
            0,
            std::ptr::null_mut(),
            0,
            std::ptr::null_mut(),
        );

        // The epoch is a number a caller compares, so the only promise
        // it makes is that asking twice with nothing between answers
        // the same thing.
        assert_eq!(h_settings_epoch(), h_settings_epoch());
    }

    /// The tooltip entry survives the two things a plugin can do wrong
    /// with it — no context, and no application manager to file with.
    /// Whether the box appears is `object::tooltip`'s own test; what is
    /// tested here is that neither answer is a crash.
    #[test]
    fn the_tooltip_entry_survives_a_null_context() {
        let text = b"a name too long for its column";
        h_tooltip(
            std::ptr::null_mut(),
            1,
            RectC { x: 0.0, y: 0.0, w: 10.0, h: 10.0 },
            text.as_ptr(),
            text.len() as u32,
        );
        h_tooltip(
            std::ptr::null_mut(),
            1,
            RectC { x: 0.0, y: 0.0, w: 10.0, h: 10.0 },
            std::ptr::null(),
            0,
        );
    }

    /// The TermSelect payload is read defensively: null data, a payload
    /// shorter than the minimum, and an unknown op all mean nothing.
    #[test]
    fn a_malformed_term_select_means_nothing() {
        let empty = ActionC { kind: ACTION_TERM_SELECT, ..empty_action() };
        assert_eq!(action_in(&empty), Action::None);

        let s = TermSelectC { op: 0, kind: 0, col: 1, row: 1, base_lo: 0, base_hi: 0 };
        let short = ActionC {
            kind: ACTION_TERM_SELECT,
            data: &s as *const TermSelectC as *const u8,
            data_len: 4,
            ..empty_action()
        };
        assert_eq!(action_in(&short), Action::None);

        let wild = TermSelectC { op: 99, ..s };
        let unknown = ActionC {
            kind: ACTION_TERM_SELECT,
            data: &wild as *const TermSelectC as *const u8,
            data_len: std::mem::size_of::<TermSelectC>() as u32,
            ..empty_action()
        };
        assert_eq!(action_in(&unknown), Action::None);

        // An unknown KIND on a valid op degrades to Cells rather than
        // dying: the selection kinds may grow.
        let odd = TermSelectC { op: crate::runtime::SELECT_OP_BEGIN, kind: 42, ..s };
        let a = ActionC {
            kind: ACTION_TERM_SELECT,
            data: &odd as *const TermSelectC as *const u8,
            data_len: std::mem::size_of::<TermSelectC>() as u32,
            ..empty_action()
        };
        assert_eq!(
            action_in(&a),
            Action::TermSelect { op: SelectOp::Begin(SelKind::Cells), col: 1, row: 1, base: 0 }
        );

        // PastePrimary carries nothing and crosses as itself.
        assert_eq!(
            action_in(&ActionC { kind: ACTION_PASTE_PRIMARY, ..empty_action() }),
            Action::PastePrimary
        );
        // So does the capture answer — the one reply that asks for
        // nothing and still is not None, which is what tells the host
        // to hold the pointer for the widget.
        assert_eq!(
            action_in(&ActionC { kind: ACTION_CAPTURE, ..empty_action() }),
            Action::Capture
        );
        assert_ne!(
            action_in(&ActionC { kind: ACTION_CAPTURE, ..empty_action() }),
            Action::None
        );
    }

    #[test]
    fn actions_survive_the_crossing() {
        let bytes = b"hello";
        let a = ActionC {
            kind: ACTION_BYTES,
            index: 0,
            lines: 0,
            data: bytes.as_ptr(),
            data_len: 5,
        };
        assert_eq!(action_in(&a), Action::Bytes(b"hello".to_vec()));
        let t = ActionC { kind: ACTION_SELECT_TAB, index: 3, ..empty_action() };
        assert_eq!(action_in(&t), Action::SelectTab(3));
        let s = ActionC { kind: ACTION_SCROLL_TERMINAL, lines: -4, ..empty_action() };
        assert_eq!(action_in(&s), Action::ScrollTerminal(-4));
        // An unknown code is nothing, not a panic.
        assert_eq!(action_in(&ActionC { kind: 9999, ..empty_action() }), Action::None);
        // A null payload where bytes were promised yields no bytes.
        assert_eq!(
            action_in(&ActionC { kind: ACTION_BYTES, ..empty_action() }),
            Action::Bytes(Vec::new())
        );
    }
}
