//! Context menu (F1 §4): a retained model drawn immediate — the
//! dropdown's idiom grown up. Items, separators, shortcut hints and one
//! open submenu level per node (arbitrary depth by nesting — each level
//! is a [`MenuState`]).
//!
//! Division of labour with the application:
//!
//! * The application owns the OPEN menu as its top layer: while one is
//!   up, keys and clicks reach [`MenuState::key`] / [`MenuState::click`]
//!   before anything else, and a click that lands outside every level
//!   answers [`MenuOut::Close`] *and is consumed* (no click-through —
//!   deliberate).
//! * [`MenuState::draw`] runs LAST in the frame: the draw list is
//!   immediate and draw order is z-order, so anything drawn after the
//!   menu would sit on top of it.
//! * Hints are the application's [`crate::focus::ShortcutMap::hint`]
//!   strings — never hand-written, or the day a binding changes the
//!   menu lies.
//!
//! Everything visual comes from `[menu]` / `component.menu.*` /
//! class `menu.item` tokens and the `motion.menu_unfold` clock; the
//! module holds no literal of its own.

use crate::access::{AccessInfo, Role, States};
use crate::focus::{Caps, FocusId, Key, KeyEv, Mods};
use crate::theme::{self, bake::StateStyle, parse::State, Color, TokenId};
use crate::{ui, Ctx, Rect};
use std::sync::OnceLock;

fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// The engine's colour, in the draw list's clothes.
fn col(c: theme::ThemeColor) -> Color {
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

// ---------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------

/// One selectable row.
#[derive(Clone)]
pub struct MenuItem {
    pub label: String,
    /// Application command id — the F1 §1 registry's namespace, so a
    /// picked row and a pressed shortcut land in the same dispatcher.
    pub cmd: u32,
    /// `"Ctrl+Shift+C"` — from `ShortcutMap::hint`, or None for a row
    /// with no binding.
    pub hint: Option<String>,
    pub disabled: bool,
    /// The next level, opened at this row's right edge.
    pub submenu: Option<Vec<MenuItem>>,
}

impl MenuItem {
    pub fn new(label: &str, cmd: u32) -> MenuItem {
        MenuItem {
            label: label.to_string(),
            cmd,
            hint: None,
            disabled: false,
            submenu: None,
        }
    }

    pub fn with_hint(mut self, hint: Option<String>) -> MenuItem {
        self.hint = hint;
        self
    }

    pub fn with_disabled(mut self, disabled: bool) -> MenuItem {
        self.disabled = disabled;
        self
    }

    pub fn with_submenu(mut self, items: Vec<MenuItem>) -> MenuItem {
        self.submenu = Some(items);
        self
    }
}

/// A row of the menu: an item, or the separator rule between groups.
#[derive(Clone)]
pub enum MenuEntry {
    Item(MenuItem),
    Rule,
}

/// What [`MenuState::key`] / [`MenuState::click`] answer the router.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MenuOut {
    /// Consumed inside the menu; nothing for the application yet.
    None,
    /// Dismiss the whole menu. For a click this also means the press
    /// landed outside every level — the router consumes it anyway.
    Close,
    /// A row was picked: dismiss and run the command.
    Pick(u32),
}

/// What [`MenuState::draw`] answers: where this level landed and what
/// the pointer is on (the deepest open level reports through its own
/// return — the application rarely needs either, clicks and keys route
/// through the state itself).
pub struct MenuHit {
    /// The level's placed box, at its full unfolded size.
    pub rect: Rect,
    /// The entry index under the pointer, if any.
    pub hover: Option<usize>,
}

/// One open menu level. The application keeps the root; submenus hang
/// off their parent's `sub`.
pub struct MenuState {
    entries: Vec<MenuEntry>,
    /// Opening point (cursor, control corner). Preferred growth is
    /// right+down from here; [`place`] flips when out of room.
    at: (f32, f32),
    /// This level's [`FocusId`] root for [`MenuState::draw`]'s per-row
    /// accessible reporting: a row registers as `path.item(row index)`,
    /// and [`MenuState::open_sub`] gives the child level `path` =
    /// `self.path.item(parent row)`, so a menu and its one open submenu
    /// never hand a bridge the same id for two different rows. Fixed at
    /// `"menu"` for the root rather than taken from a caller, unlike a
    /// dropdown's `AccordionStyle::focus` — this module's own doc is
    /// that the application keeps at most ONE open menu at a time, so
    /// there is only ever one root to name.
    path: FocusId,
    /// The `motion.menu_unfold` clock: the moment this level opened.
    /// Non-finite = not stamped yet; the first draw stamps it (a
    /// submenu is opened by key/click handlers that hold no clock).
    opened_t: f64,
    /// Keyboard/hover highlight, an index into `entries`.
    hi: Option<usize>,
    /// The highlight came from the keyboard: draw the SELECTED rung,
    /// not hover — keyboard never produces hover.
    hi_kbd: bool,
    /// Keyboard motion suppresses pointer-hover until the pointer
    /// actually moves again (the small dance every toolkit does).
    kbd_hold: bool,
    /// The one open submenu level: (parent row index, its state).
    sub: Option<(usize, Box<MenuState>)>,
    // ---- view bookkeeping, filled by draw ---------------------------
    /// Placed by the parent's draw: the row rect a submenu anchors to.
    anchor_row: Option<Rect>,
    /// This level's box at full size, once placed.
    rect: Rect,
    /// Row rects index-aligned with `entries`, at full size, plus
    /// whether the row is fully unfolded (a mid-unfold row is not
    /// clickable, exactly the dropdown's rule).
    rows: Vec<(Rect, bool)>,
    /// The first draw happened; before it, clicks are consumed rather
    /// than judged against zeroed geometry.
    placed: bool,
    /// The pointer position of the last draw (hover delta detection).
    last_mouse: Option<(f32, f32)>,
    /// The row the pointer was over at the last draw: hover updates
    /// `hi` only when the pointer ENTERS A DIFFERENT ROW, so a 1px
    /// jitter cannot yank the highlight from under the arrow keys.
    hover_row: Option<usize>,
}

impl MenuState {
    /// A menu opened at `(x, y)` — the right-click position or a
    /// control's rect corner — with `t` (seconds, the frame clock) as
    /// the unfold's start.
    pub fn open_at(entries: Vec<MenuEntry>, x: f32, y: f32, t: f64) -> MenuState {
        MenuState {
            entries,
            at: (x, y),
            path: FocusId::of("menu"),
            opened_t: t,
            hi: None,
            hi_kbd: false,
            kbd_hold: false,
            sub: None,
            anchor_row: None,
            rect: Rect::new(0.0, 0.0, 0.0, 0.0),
            rows: Vec::new(),
            placed: false,
            last_mouse: None,
            hover_row: None,
        }
    }

    /// Keyboard, routed to the deepest open level first. Down/Up move
    /// the highlight skipping rules and disabled rows (wrapping);
    /// Right/Enter opens the submenu or picks; Left/Escape closes one
    /// level. Typing is ignored in F1 (mnemonics later) — but consumed:
    /// the open menu is a grab.
    pub fn key(&mut self, ev: &KeyEv) -> MenuOut {
        if let Some((_, sub)) = &mut self.sub {
            return match sub.key(ev) {
                MenuOut::Close => {
                    self.sub = None;
                    MenuOut::None
                }
                out => out,
            };
        }
        if ev.mods != Mods::NONE {
            return MenuOut::None;
        }
        match ev.key {
            Key::Down => {
                self.move_hi(1);
                MenuOut::None
            }
            Key::Up => {
                self.move_hi(-1);
                MenuOut::None
            }
            Key::Right => {
                if let Some(i) = self.hi {
                    if self.openable(i) {
                        self.open_sub(i);
                    }
                }
                MenuOut::None
            }
            Key::Enter => match self.hi {
                Some(i) if self.openable(i) => {
                    self.open_sub(i);
                    MenuOut::None
                }
                Some(i) => match &self.entries[i] {
                    MenuEntry::Item(it) if !it.disabled => MenuOut::Pick(it.cmd),
                    _ => MenuOut::None,
                },
                None => MenuOut::None,
            },
            Key::Left | Key::Escape => MenuOut::Close,
            _ => MenuOut::None,
        }
    }

    /// A pointer press, routed to the deepest level containing it.
    /// Inside a level: a plain row picks, a submenu row opens its
    /// level, a rule or disabled row is a consumed no-op. Outside every
    /// level: [`MenuOut::Close`] — and the router still consumes the
    /// press (no click-through).
    pub fn click(&mut self, x: f32, y: f32) -> MenuOut {
        if !self.placed {
            // Between opening and the first draw there is no geometry
            // to judge against; swallow rather than misjudge.
            return MenuOut::None;
        }
        if let Some((_, sub)) = &mut self.sub {
            return match sub.click(x, y) {
                MenuOut::Close => {
                    if self.rect.contains(x, y) {
                        // Outside the subtree but on THIS level: the
                        // subtree folds and the row acts.
                        self.sub = None;
                        self.act_on(x, y)
                    } else {
                        MenuOut::Close
                    }
                }
                out => out,
            };
        }
        if self.rect.contains(x, y) {
            self.act_on(x, y)
        } else {
            MenuOut::Close
        }
    }

    /// The rung a menu is a surface of, dressed in the menu's own key
    /// names.
    ///
    /// `[elev.popover]` is Elev 5, and the master's gloss on it opens with
    /// the word "menu". What the menu states for itself is the same five
    /// tokens its private copy of the rules read before 2026-08-17, so
    /// joining the ladder moved no pixel; what it gains is the glass pair
    /// (`elev.popover.glass.*`, rank 0 in the master, so nothing is drawn
    /// today), the panel-edge bloom, and every key the ladder grows next
    /// — the shadow and the reflection the master already declares on the
    /// rung and no object could read.
    ///
    /// One `Level` for every level of every menu: it is a table of token
    /// ids, and every open menu is the same kind of surface. A submenu
    /// that dressed differently from its parent would be the drift again.
    fn level() -> &'static super::elev::Level {
        static LEVEL: OnceLock<super::elev::Level> = OnceLock::new();
        LEVEL.get_or_init(|| {
            super::elev::Level::of("elev.popover").worn_as(
                "component.menu.fill",
                "menu.corner_mode",
                "menu.corner",
                "component.menu.border",
                "menu.border",
            )
        })
    }

    /// Immediate draw + hit info, deepest level last (on top). Hover
    /// tracking happens here — the draw pass is the one place that
    /// knows this frame's geometry.
    pub fn draw(&mut self, ctx: &mut Ctx) -> MenuHit {
        static BORDER_C: OnceLock<TokenId> = OnceLock::new();
        static ROW_H: OnceLock<TokenId> = OnceLock::new();
        static PAD: OnceLock<TokenId> = OnceLock::new();
        static MIN_W: OnceLock<TokenId> = OnceLock::new();
        static ITEM_INSET: OnceLock<TokenId> = OnceLock::new();
        static TEXT_THRESHOLD: OnceLock<TokenId> = OnceLock::new();
        static RULE_W: OnceLock<TokenId> = OnceLock::new();
        static RULE_PAD: OnceLock<TokenId> = OnceLock::new();
        static HINT_GAP: OnceLock<TokenId> = OnceLock::new();
        static HINT_C: OnceLock<TokenId> = OnceLock::new();
        static CHEVRON_W: OnceLock<TokenId> = OnceLock::new();
        static CHEVRON_S: OnceLock<TokenId> = OnceLock::new();
        static OVERLAP: OnceLock<TokenId> = OnceLock::new();
        static ROLE: OnceLock<TokenId> = OnceLock::new();
        static HINT_ROLE: OnceLock<TokenId> = OnceLock::new();
        static CLASS: OnceLock<Option<u16>> = OnceLock::new();

        if !self.opened_t.is_finite() {
            self.opened_t = ctx.t;
        }
        let t = theme::resolved();
        let class = *CLASS.get_or_init(|| theme::class_id("menu.item"));

        // ---- metrics ----------------------------------------------------
        let row_h = t.px(tok(&ROW_H, "menu.row_h")).max(0.0);
        let pad = t.px(tok(&PAD, "menu.pad")).max(0.0);
        let min_w = t.px(tok(&MIN_W, "menu.min_w")).max(0.0);
        let inset = t.px(tok(&ITEM_INSET, "menu.item_inset")).max(0.0);
        let text_threshold = t.px(tok(&TEXT_THRESHOLD, "menu.item_text_threshold"));
        let rule_w = t.px(tok(&RULE_W, "menu.rule")).max(0.0);
        let rule_pad = t.px(tok(&RULE_PAD, "menu.rule_pad")).max(0.0);
        let hint_gap = t.px(tok(&HINT_GAP, "menu.hint_gap")).max(0.0);
        let chevron_w = t.px(tok(&CHEVRON_W, "menu.chevron_w")).max(0.0);
        let chevron_s = t.px(tok(&CHEVRON_S, "menu.chevron_stroke")).max(0.0);
        let overlap = t.px(tok(&OVERLAP, "menu.submenu_overlap")).max(0.0);

        let role = ui::bound_role(&ROLE, "menu.item.role");
        // No `ui_font_scale`: the viewport carries the user's scale into u,
        // and the role's size is written in u — applying it here too squares it.
        let px = role.px(ctx, 1.0);
        let track = role.tracking_px(px);
        let leading = role.leading();
        // The label's FACE and figure box, read once here and carried
        // through the measuring pass and the row loop alike. The rows
        // used to name `FONT_UI`, so `menu.item.role` could be repointed
        // at a monospace role and the menu stayed in the interface face.
        let face = role.font();
        let fig = role.figures(ctx.fonts, face, px);
        let hint_role = ui::bound_role(&HINT_ROLE, "menu.hint_role");
        let hpx = hint_role.px(ctx, 1.0);
        let htrack = hint_role.tracking_px(hpx);
        let hleading = hint_role.leading();
        // The hint is a SECOND role and gets its own answers: the
        // shortcut column is right-aligned, so it is exactly the column
        // `tabular` exists for — `Ctrl+1` and `Ctrl+8` have to keep one
        // left edge — and it may well be set in a face the label is not.
        let hface = hint_role.font();
        let hfig = hint_role.figures(ctx.fonts, hface, hpx);

        // ---- measure ----------------------------------------------------
        let mut label_max: f32 = 0.0;
        let mut hint_max: f32 = 0.0;
        let mut any_sub = false;
        for e in &self.entries {
            if let MenuEntry::Item(it) = e {
                // The menu sizes itself to its widest label, so this is
                // the measure that decides the box the rows are drawn
                // in. It has to be taken in the face and under the box
                // the row loop below draws with, or a wider face runs
                // its labels off the end of the menu that measured them.
                label_max =
                    label_max.max(ctx.fonts.measure_fig(face, px, &it.label, track, &fig));
                if let Some(hint) = &it.hint {
                    hint_max =
                        hint_max.max(ctx.fonts.measure_fig(hface, hpx, hint, htrack, &hfig));
                }
                any_sub |= it.submenu.is_some();
            }
        }
        // Column layout: [inset][label][gap hint][gap chevron][inset].
        // A context menu always sizes to content over the min_w floor;
        // `menu.anchor_width = anchor` applies only to the dropdown.
        let mut content_w = inset + label_max + inset;
        if hint_max > 0.0 {
            content_w += hint_gap + hint_max;
        }
        if any_sub {
            content_w += hint_gap + chevron_w;
        }
        let w = content_w.max(min_w);
        let rows_h: f32 = self
            .entries
            .iter()
            .map(|e| entry_h(e, row_h, rule_pad, rule_w))
            .sum();
        let h = pad * 2.0 + rows_h;

        // ---- place ------------------------------------------------------
        let pos = match self.anchor_row {
            Some(row) => place_sub(row, (w, h), overlap, (ctx.w, ctx.h)),
            None => place(self.at, (w, h), (ctx.w, ctx.h)),
        };
        self.rect = Rect::new(pos.0, pos.1, w, h);

        // ---- unfold -----------------------------------------------------
        // The shared resolver (`crate::motion`): reduced motion, a
        // disabled effect and a zero duration all FREEZE AT FULLY OPEN —
        // "already open", never "never opens".
        let p = crate::motion::Effect::of("menu_unfold").one_shot(self.opened_t, ctx.t);
        let visible_h = p * rows_h;

        // The box claims the ground it covers ([`crate::pointer`]) before
        // the rows below read the pointer, so an open menu takes the
        // hover away from whatever it opened over — the same grab its
        // clicks and its wheel already are — while the rows of the menu
        // itself keep it. As much of the box as has UNFOLDED: a menu
        // halfway open covers half of what it will.
        ctx.mouse.cover(Rect::new(self.rect.x, self.rect.y, w, pad * 2.0 + visible_h));

        // ---- rows' geometry (full size — hit data) ----------------------
        self.rows.clear();
        let mut top = 0.0;
        for e in &self.entries {
            let eh = entry_h(e, row_h, rule_pad, rule_w);
            let full = top + eh <= visible_h + 0.5;
            self.rows.push((
                Rect::new(self.rect.x, self.rect.y + pad + top, w, eh),
                full,
            ));
            top += eh;
        }

        // ---- hover tracking ---------------------------------------------
        // Pointer motion updates the highlight only when it enters a
        // DIFFERENT row (index, not raw position), and keyboard motion
        // holds hover off until the pointer actually moves again.
        let mouse = ctx.mouse.at();
        if self.last_mouse != Some(mouse) {
            self.kbd_hold = false;
            self.last_mouse = Some(mouse);
        }
        let under = self
            .rows
            .iter()
            .enumerate()
            .find(|(i, (r, full))| {
                *full && r.contains(mouse.0, mouse.1) && self.selectable(*i)
            })
            .map(|(i, _)| i);
        if !self.kbd_hold && under != self.hover_row {
            self.hover_row = under;
            match under {
                Some(i) => {
                    self.hi = Some(i);
                    self.hi_kbd = false;
                }
                // The pointer left the rows: a keyboard highlight
                // stays, a hover one goes out.
                None if !self.hi_kbd => self.hi = None,
                None => {}
            }
        }

        // ---- the box ----------------------------------------------------
        let drawn = Rect::new(self.rect.x, self.rect.y, w, pad * 2.0 + visible_h);
        // Elev 5, the popover rung — the first surface the master's own
        // `[elev.popover]` gloss names ("menu, tooltip, context menu, drag
        // ghost"). Drawn through the ladder's one reader since 2026-08-17;
        // before that the menu carried a private copy of the rules and so
        // could not be given glass, a shadow or a rung by any theme.
        //
        // Its own keys still say what its BODY, CUT and RING are, and they
        // say exactly what they said before, so the picture did not move:
        // the radius says how far, `menu.corner_mode` says how — and the
        // master points that at the window frame's, so the menu the window
        // draws (winframe.rs) and this one cannot disagree about shape.
        // (`Corner::sized`, inside the rung, is still where §5.0's `pill`
        // becomes a radius rather than being clamped to a square.)
        Self::level().draw(ctx, drawn);

        // ---- rows -------------------------------------------------------
        let hint_ink = col(t.color(tok(&HINT_C, "component.menu.hint")));
        let sub_open = self.sub.as_ref().map(|(i, _)| *i);
        // Accessible position-in-set counts ITEMS only — a separator is
        // not a stop a screen reader numbers. `item_count` is fixed once
        // over the same rule `item_pos` advances by, below, so the two
        // never disagree about what a "set" is.
        let item_count =
            self.entries.iter().filter(|e| matches!(e, MenuEntry::Item(_))).count() as u32;
        let mut item_pos: u32 = 0;
        for (i, e) in self.entries.iter().enumerate() {
            let (full_r, _) = self.rows[i];
            let top = full_r.y - (self.rect.y + pad);
            if top >= visible_h {
                break;
            }
            // Mid-unfold the row keeps its top and loses its bottom,
            // exactly the accordion.
            let rh = (visible_h - top).min(full_r.h);
            let r = Rect::new(full_r.x, full_r.y, full_r.w, rh);
            let full = rh >= full_r.h - 0.5;
            match e {
                MenuEntry::Rule => {
                    // The separator draws once whole — half a hairline
                    // is noise, and its breathing is already the row.
                    if full && rule_w > 0.0 {
                        let y = r.y + (r.h - rule_w) / 2.0;
                        ctx.dl.rect(
                            r.x,
                            y,
                            r.w,
                            rule_w,
                            col(t.color(tok(&BORDER_C, "component.menu.border"))),
                        );
                    }
                }
                MenuEntry::Item(it) => {
                    // Keyboard highlight is the SELECTED rung, pointer
                    // hover the hover rung — keyboard never produces
                    // hover. The parent row of an open submenu stays
                    // on the selected rung too.
                    let state = if it.disabled {
                        State::Disabled
                    } else if sub_open == Some(i) {
                        State::Selected
                    } else if self.hi == Some(i) {
                        if self.hi_kbd {
                            State::Selected
                        } else {
                            State::Hover
                        }
                    } else {
                        State::Idle
                    };
                    item_pos += 1;
                    // Accessible reporting rides `ctx.focus`, not
                    // `ctx.access`: a row here is exactly the FOCUSABLE
                    // case `crate::access`'s header carves out for
                    // `FocusCtl::register` — one of several, with a
                    // position — even though the router never calls
                    // `FocusCtl::nav()` on it: this module's own `key`
                    // and `click` own every key and click outright while
                    // a menu is open (see the module doc), so Tab never
                    // reaches these rows from outside it. Registered at
                    // `full_r`, not the mid-unfold `r` — the logical row,
                    // not this frame's animation.
                    if let Some(fc) = ctx.focus.as_deref_mut() {
                        let mut states = States::NONE;
                        if self.hi == Some(i) {
                            states = states | States::SELECTED;
                        }
                        let access = AccessInfo::new(Role::MenuItem, it.label.as_str())
                            .with_states(states)
                            .with_index(item_pos, item_count);
                        fc.register(self.path.item(i), full_r, Caps::NONE, access);
                    }
                    // Crossfaded under `motion.hover` / `.select` /
                    // `.disable`. The row's IDLE rung keeps the ladder's
                    // text — a resting label is a themed colour — but no
                    // fill: an idle row rests on the menu's own bed (the
                    // window-menu idiom, winframe.rs), and fading back
                    // into `idle.fill` would paint a wash under every
                    // row. So the highlight fades out to nothing, and at
                    // rest its alpha is exactly zero.
                    let style: StateStyle = crate::motion::state_ink(
                        "menu.item",
                        r,
                        state,
                        ctx.t,
                        |s| {
                            let ink = crate::view::surface::StateInk::from(match class {
                                Some(cl) => t.class_state(cl, s),
                                None => StateStyle::RAW,
                            });
                            match s {
                                State::Idle => crate::view::surface::StateInk {
                                    fill: crate::theme::Color::TRANSPARENT,
                                    ..ink
                                },
                                _ => ink,
                            }
                        },
                    )
                    .into();
                    // The highlight wash; a row at rest has none.
                    if style.fill.a > 0.0 {
                        ctx.dl.rect(r.x, r.y, r.w, r.h, col(style.fill));
                    }
                    if rh >= text_threshold {
                        ctx.dl.text_fig(
                            ctx.fonts,
                            face,
                            px,
                            r.x + inset,
                            r.y + (rh - px * leading) / 2.0,
                            &it.label,
                            col(style.text),
                            track,
                            &fig,
                        );
                        let mut right = r.x + r.w - inset;
                        if any_sub {
                            if it.submenu.is_some() {
                                // The chevron: a `>` of two strokes in
                                // the glyph box at the row's right edge.
                                let cx = right - chevron_w;
                                let cy = r.y + rh / 2.0;
                                let half = chevron_w / 2.0;
                                ctx.dl.polyline(
                                    &[
                                        [cx + half * 0.5, cy - half],
                                        [cx + half * 1.5, cy],
                                        [cx + half * 0.5, cy + half],
                                    ],
                                    chevron_s,
                                    col(style.glyph),
                                    false,
                                );
                            }
                            right -= chevron_w + hint_gap;
                        }
                        if let Some(hint) = &it.hint {
                            // Right-aligned in the hint column, muted
                            // by design — secondary to the label.
                            let ink = if it.disabled { col(style.text) } else { hint_ink };
                            ctx.dl.text_right_fig(
                                ctx.fonts,
                                hface,
                                hpx,
                                right,
                                r.y + (rh - hpx * hleading) / 2.0,
                                hint,
                                ink,
                                htrack,
                                &hfig,
                            );
                        }
                    }
                }
            }
        }
        self.placed = true;

        // ---- the open submenu, drawn after = on top ---------------------
        if let Some((i, sub)) = &mut self.sub {
            let anchor = self.rows.get(*i).map(|(r, _)| *r);
            if let Some(anchor) = anchor {
                sub.anchor_row = Some(anchor);
                sub.draw(ctx);
            }
        }
        MenuHit { rect: self.rect, hover: under }
    }

    // ---- internals ------------------------------------------------------

    fn selectable(&self, i: usize) -> bool {
        matches!(&self.entries[i], MenuEntry::Item(it) if !it.disabled)
    }

    fn openable(&self, i: usize) -> bool {
        matches!(&self.entries[i], MenuEntry::Item(it) if !it.disabled && it.submenu.is_some())
    }

    /// Moves the keyboard highlight by `dir`, skipping rules and
    /// disabled rows, wrapping at either end. From nothing, Down lands
    /// on the first selectable row and Up on the last.
    fn move_hi(&mut self, dir: isize) {
        let n = self.entries.len();
        if n == 0 || !(0..n).any(|i| self.selectable(i)) {
            return;
        }
        let mut i = match self.hi {
            Some(cur) => cur as isize,
            None if dir > 0 => -1,
            None => n as isize,
        };
        loop {
            i += dir;
            if i < 0 {
                i = n as isize - 1;
            } else if i >= n as isize {
                i = 0;
            }
            if self.selectable(i as usize) {
                break;
            }
        }
        self.hi = Some(i as usize);
        self.hi_kbd = true;
        self.kbd_hold = true;
    }

    /// Opens row `i`'s submenu. The unfold clock is stamped by the
    /// submenu's first draw (key and click handlers hold no clock),
    /// and the parent's draw anchors it to the row each frame.
    fn open_sub(&mut self, i: usize) {
        // The submenu keeps its own submenus: arbitrary depth, one
        // open level per node.
        let items = match &self.entries[i] {
            MenuEntry::Item(it) => it
                .submenu
                .as_ref()
                .map(|items| items.iter().cloned().map(MenuEntry::Item).collect::<Vec<_>>()),
            MenuEntry::Rule => None,
        };
        if let Some(entries) = items {
            let mut sub = MenuState::open_at(entries, 0.0, 0.0, f64::NAN);
            // A child level's own root, so its rows' accessible ids never
            // collide with this level's (see `path`'s doc).
            sub.path = self.path.item(i);
            // Keyboard entry into a submenu starts on its first row.
            if self.hi_kbd {
                sub.move_hi(1);
            }
            self.sub = Some((i, Box::new(sub)));
            self.hi = Some(i);
        }
    }

    /// Acts on a press inside this level's box: the row under the
    /// point, with the hit rect grown to the a11y floor when the theme
    /// says `grow` — drawn geometry untouched, exactly the `[a11y]`
    /// contract (`menu.row_h` sits under the 24 px floor).
    fn act_on(&mut self, x: f32, y: f32) -> MenuOut {
        static MIN_HIT: OnceLock<TokenId> = OnceLock::new();
        static PAD_MODE: OnceLock<TokenId> = OnceLock::new();
        static GROW: OnceLock<Option<u16>> = OnceLock::new();
        let hit = self
            .rows
            .iter()
            .enumerate()
            .find(|(_, (r, full))| *full && r.contains(x, y))
            .map(|(i, _)| i);
        let hit = hit.or_else(|| {
            // Between rows or on the padding: the grown hit rects get a
            // say, nearest centre winning where they overlap.
            let t = theme::resolved();
            let mode = tok(&PAD_MODE, "a11y.hit_pad_mode");
            let grow = *GROW.get_or_init(|| theme::enum_index(mode, "grow"));
            if grow.is_none() || Some(t.enum_of(mode)) != grow {
                return None;
            }
            let min_hit = t.px(tok(&MIN_HIT, "a11y.min_hit"));
            let mut best: Option<(f32, usize)> = None;
            for (i, (r, full)) in self.rows.iter().enumerate() {
                if !*full || !self.selectable(i) {
                    continue;
                }
                let g = grown(*r, min_hit);
                if !g.contains(x, y) {
                    continue;
                }
                let (cx, cy) = (r.x + r.w / 2.0, r.y + r.h / 2.0);
                let d = (cx - x) * (cx - x) + (cy - y) * (cy - y);
                if best.map_or(true, |(bd, _)| d < bd) {
                    best = Some((d, i));
                }
            }
            best.map(|(_, i)| i)
        });
        match hit {
            Some(i) => match &self.entries[i] {
                MenuEntry::Item(it) if !it.disabled => {
                    if it.submenu.is_some() {
                        self.hi = Some(i);
                        self.hi_kbd = false;
                        self.open_sub(i);
                        MenuOut::None
                    } else {
                        MenuOut::Pick(it.cmd)
                    }
                }
                _ => MenuOut::None,
            },
            None => MenuOut::None,
        }
    }
}

/// One entry's slot height: a row for an item, the separator's
/// breathing plus its stroke for a rule.
fn entry_h(e: &MenuEntry, row_h: f32, rule_pad: f32, rule_w: f32) -> f32 {
    match e {
        MenuEntry::Item(_) => row_h,
        MenuEntry::Rule => rule_pad * 2.0 + rule_w,
    }
}

/// Where a `size` box lands for the opening point `at`: prefer
/// right+down; flip to the other side of the point when the box would
/// leave the window; clamp inside as a last resort.
fn place(at: (f32, f32), size: (f32, f32), win: (f32, f32)) -> (f32, f32) {
    let x = if at.0 + size.0 <= win.0 {
        at.0
    } else if at.0 - size.0 >= 0.0 {
        at.0 - size.0
    } else {
        (win.0 - size.0).max(0.0)
    };
    let y = if at.1 + size.1 <= win.1 {
        at.1
    } else if at.1 - size.1 >= 0.0 {
        at.1 - size.1
    } else {
        (win.1 - size.1).max(0.0)
    };
    (x, y)
}

/// Where a submenu of `size` lands for its parent `row`: at the row's
/// right edge tucked under by `overlap`, top-aligned with the row;
/// flipped to the row's left edge / upwards when out of room, clamped
/// as a last resort.
fn place_sub(row: Rect, size: (f32, f32), overlap: f32, win: (f32, f32)) -> (f32, f32) {
    let x = if row.right() - overlap + size.0 <= win.0 {
        row.right() - overlap
    } else if row.x + overlap - size.0 >= 0.0 {
        row.x + overlap - size.0
    } else {
        (win.0 - size.0).max(0.0)
    };
    let y = if row.y + size.1 <= win.1 {
        row.y
    } else if row.bottom() - size.1 >= 0.0 {
        row.bottom() - size.1
    } else {
        (win.1 - size.1).max(0.0)
    };
    (x, y)
}

/// The symmetric a11y growth of a hit rect to at least `min_hit` a
/// side — the DRAWN rect never moves.
fn grown(r: Rect, min_hit: f32) -> Rect {
    let gw = (min_hit - r.w).max(0.0) / 2.0;
    let gh = (min_hit - r.h).max(0.0) / 2.0;
    Rect::new(r.x - gw, r.y - gh, r.w + 2.0 * gw, r.h + 2.0 * gh)
}

// The unfold progress used to be resolved here, by a private copy of the
// easing table over a cache of ENUM INDICES — the pattern scroll.rs
// documents as broken across a theme swap. `crate::motion::Effect` is
// that resolver shared, index-cache excised; the freeze-at-visible rule
// this file first wrote down now lives on `Effect::one_shot`.

// ---------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    // The face harness lives beside the panel container and is used from
    // here rather than copied: what counts as proof that a run followed
    // its role is one rule, and three copies of it drift.
    use crate::object::panel::tests::{
        all_in, drawn_text, face_follows_the_theme, measure_in_child, report, role_word,
    };

    /// USTERKA 3, the no-move proof. A menu is a surface of Elev 5 since
    /// 2026-08-17; before that it carried a private copy of the rules and
    /// so could be given neither glass nor a shadow nor a rung by any
    /// theme. Joining had to change the picture by NOTHING under the
    /// master, and this is that claim as arithmetic: the rung and the
    /// copy it replaced draw the same commands over the same vertices.
    ///
    /// It also pins what "no change" is CONDITIONAL on — the master's
    /// `elev.popover.glass.rank = 0` and `glow.panel_edge.enabled =
    /// false`. Neither is a fallback in Rust: raise either in a theme and
    /// the menu is meant to move, which is the point of joining.
    #[test]
    fn joining_the_ladder_moved_no_pixel() {
        use crate::draw::DrawList;
        use crate::object::elev::tests::{same_picture, the_private_copy, AT_REST};
        let t = theme::resolved();
        let r = Rect::new(64.0, 40.0, 220.0, 132.0);
        let mut was = DrawList::recording();
        the_private_copy(
            &mut was,
            t,
            r,
            "component.menu.fill",
            "menu.corner_mode",
            "menu.corner",
            "component.menu.border",
            "menu.border",
        );
        let mut now = DrawList::recording();
        MenuState::level().draw_in(&mut now, t, r, r, AT_REST);
        same_picture(&was, &now);
    }

    fn item(label: &str, cmd: u32) -> MenuEntry {
        MenuEntry::Item(MenuItem::new(label, cmd))
    }

    fn ev(key: Key) -> KeyEv {
        KeyEv { key, mods: Mods::NONE, repeat: false, text: None }
    }

    /// Copy / rule / Clear (disabled) / Paste — the nav fixture.
    fn menu() -> MenuState {
        MenuState::open_at(
            vec![
                item("COPY", 1),
                MenuEntry::Rule,
                MenuEntry::Item(MenuItem::new("CLEAR", 2).with_disabled(true)),
                item("PASTE", 3),
            ],
            10.0,
            10.0,
            0.0,
        )
    }

    // ---- placement ----

    #[test]
    fn placement_prefers_right_down_and_flips() {
        // Room everywhere: right+down from the point.
        assert_eq!(place((10.0, 20.0), (100.0, 50.0), (500.0, 300.0)), (10.0, 20.0));
        // Out of room on the right: flip to the point's left.
        assert_eq!(place((450.0, 20.0), (100.0, 50.0), (500.0, 300.0)), (350.0, 20.0));
        // Out of room below: flip above the point.
        assert_eq!(place((10.0, 280.0), (100.0, 50.0), (500.0, 300.0)), (10.0, 230.0));
        // Both: both flips.
        assert_eq!(place((450.0, 280.0), (100.0, 50.0), (500.0, 300.0)), (350.0, 230.0));
    }

    #[test]
    fn placement_clamps_when_no_side_fits() {
        // No room right of the point AND none left of it: clamp to the
        // window's edge, never negative.
        let (x, _) = place((50.0, 10.0), (400.0, 50.0), (410.0, 300.0));
        assert_eq!(x, 10.0); // 410 - 400
        let (x, y) = place((5.0, 5.0), (500.0, 400.0), (400.0, 300.0));
        assert_eq!((x, y), (0.0, 0.0)); // wider than the window: pinned at 0
    }

    #[test]
    fn submenu_tucks_under_the_row_edge_and_flips() {
        let row = Rect::new(100.0, 50.0, 200.0, 24.0);
        // Room on the right: at right edge minus the overlap, top-aligned.
        assert_eq!(place_sub(row, (150.0, 80.0), 6.0, (800.0, 600.0)), (294.0, 50.0));
        // No room right: the mirror position off the row's LEFT edge.
        let right = Rect::new(300.0, 50.0, 200.0, 24.0);
        assert_eq!(place_sub(right, (150.0, 80.0), 6.0, (500.0, 600.0)), (156.0, 50.0));
        // No room below: bottom-aligned with the row instead.
        let low = Rect::new(100.0, 500.0, 200.0, 24.0);
        assert_eq!(place_sub(low, (150.0, 80.0), 6.0, (800.0, 550.0)), (294.0, 444.0));
        // Neither: clamped inside the window.
        assert_eq!(place_sub(row, (150.0, 80.0), 6.0, (400.0, 100.0)), (250.0, 20.0));
    }

    // ---- keyboard ----

    #[test]
    fn arrows_skip_rules_and_disabled_and_wrap() {
        let mut m = menu();
        m.key(&ev(Key::Down));
        assert_eq!(m.hi, Some(0), "Down from nothing lands on the head");
        m.key(&ev(Key::Down));
        assert_eq!(m.hi, Some(3), "skips the rule AND the disabled row");
        m.key(&ev(Key::Down));
        assert_eq!(m.hi, Some(0), "wraps");
        m.key(&ev(Key::Up));
        assert_eq!(m.hi, Some(3), "wraps backwards");
    }

    #[test]
    fn up_from_nothing_lands_on_the_tail() {
        let mut m = menu();
        m.key(&ev(Key::Up));
        assert_eq!(m.hi, Some(3));
    }

    #[test]
    fn keyboard_highlight_is_marked_keyboard() {
        let mut m = menu();
        m.key(&ev(Key::Down));
        assert!(m.hi_kbd && m.kbd_hold, "arrows suppress hover until the pointer moves");
    }

    #[test]
    fn enter_picks_only_enabled_rows() {
        let mut m = menu();
        assert_eq!(m.key(&ev(Key::Enter)), MenuOut::None, "nothing highlighted");
        m.key(&ev(Key::Down));
        assert_eq!(m.key(&ev(Key::Enter)), MenuOut::Pick(1));
        // Force the highlight onto the disabled row: Enter still refuses.
        m.hi = Some(2);
        assert_eq!(m.key(&ev(Key::Enter)), MenuOut::None);
    }

    #[test]
    fn escape_and_left_close_a_level() {
        let mut m = menu();
        assert_eq!(m.key(&ev(Key::Escape)), MenuOut::Close);
        assert_eq!(m.key(&ev(Key::Left)), MenuOut::Close);
    }

    #[test]
    fn modified_keys_are_consumed_not_acted_on() {
        let mut m = menu();
        let e = KeyEv {
            key: Key::Down,
            mods: Mods::CTRL,
            repeat: false,
            text: None,
        };
        assert_eq!(m.key(&e), MenuOut::None);
        assert_eq!(m.hi, None, "a modified arrow is a shortcut's business");
    }

    #[test]
    fn typing_is_ignored_but_consumed() {
        let mut m = menu();
        assert_eq!(m.key(&ev(Key::Char('x'))), MenuOut::None);
        assert_eq!(m.hi, None);
    }

    // ---- submenu ----

    fn with_sub() -> MenuState {
        MenuState::open_at(
            vec![
                item("PLAIN", 1),
                MenuEntry::Item(
                    MenuItem::new("MORE", 0)
                        .with_submenu(vec![MenuItem::new("A", 10), MenuItem::new("B", 11)]),
                ),
            ],
            0.0,
            0.0,
            0.0,
        )
    }

    #[test]
    fn right_opens_enter_picks_left_closes_one_level() {
        let mut m = with_sub();
        m.key(&ev(Key::Down));
        assert_eq!(m.key(&ev(Key::Right)), MenuOut::None, "no submenu on a plain row");
        assert!(m.sub.is_none());
        m.key(&ev(Key::Down)); // onto MORE
        assert_eq!(m.key(&ev(Key::Right)), MenuOut::None);
        assert!(m.sub.is_some(), "Right opens the submenu");
        // Keyboard entry pre-highlights the submenu's first row.
        assert_eq!(m.sub.as_ref().unwrap().1.hi, Some(0));
        // Keys now route to the deepest level.
        m.key(&ev(Key::Down));
        assert_eq!(m.sub.as_ref().unwrap().1.hi, Some(1));
        assert_eq!(m.key(&ev(Key::Enter)), MenuOut::Pick(11), "picks bubble up");
        // Left closes ONE level: consumed by the parent, menu stays.
        assert_eq!(m.key(&ev(Key::Left)), MenuOut::None);
        assert!(m.sub.is_none());
        // And at the root it closes the menu.
        assert_eq!(m.key(&ev(Key::Left)), MenuOut::Close);
    }

    #[test]
    fn enter_opens_a_submenu_rather_than_picking_it() {
        let mut m = with_sub();
        m.key(&ev(Key::Down));
        m.key(&ev(Key::Down));
        assert_eq!(m.key(&ev(Key::Enter)), MenuOut::None);
        assert!(m.sub.is_some());
    }

    // ---- clicks (geometry fabricated — draw needs a window) ----

    /// Lays the fixture out by hand exactly as draw would at full
    /// unfold: rows 24 px tall from y=10, box padded by 8.
    fn placed(mut m: MenuState) -> MenuState {
        let row_h = 24.0;
        let mut y = 18.0;
        m.rows = m
            .entries
            .iter()
            .map(|e| {
                let h = match e {
                    MenuEntry::Item(_) => row_h,
                    MenuEntry::Rule => 10.0,
                };
                let r = Rect::new(10.0, y, 200.0, h);
                y += h;
                (r, true)
            })
            .collect();
        m.rect = Rect::new(10.0, 10.0, 200.0, y - 10.0 + 8.0);
        m.placed = true;
        m
    }

    #[test]
    fn click_outside_closes_inside_picks() {
        let mut m = placed(menu());
        assert_eq!(m.click(500.0, 500.0), MenuOut::Close);
        // Row 0 (COPY) spans y 18..42.
        assert_eq!(m.click(50.0, 30.0), MenuOut::Pick(1));
        // The disabled row is a consumed no-op.
        let (r, _) = m.rows[2];
        assert_eq!(m.click(r.x + 5.0, r.y + r.h / 2.0), MenuOut::None);
    }

    #[test]
    fn click_before_first_draw_is_consumed_not_judged() {
        let mut m = menu(); // never drawn: no geometry
        assert_eq!(m.click(500.0, 500.0), MenuOut::None);
    }

    #[test]
    fn click_routes_to_the_deepest_level_first() {
        let mut m = placed(with_sub());
        // Open MORE's submenu, then fabricate its geometry beside it.
        m.key(&ev(Key::Down));
        m.key(&ev(Key::Down));
        m.key(&ev(Key::Right));
        {
            let (_, sub) = m.sub.as_mut().unwrap();
            let s = placed(std::mem::replace(
                sub.as_mut(),
                MenuState::open_at(Vec::new(), 0.0, 0.0, 0.0),
            ));
            **sub = s;
            // Move the submenu's box clear of the parent's.
            let dx = 300.0;
            sub.rect.x += dx;
            for (r, _) in sub.rows.iter_mut() {
                r.x += dx;
            }
        }
        // A point inside the submenu picks there.
        let (r, _) = m.sub.as_ref().unwrap().1.rows[0];
        assert_eq!(m.click(r.x + 5.0, r.y + 5.0), MenuOut::Pick(10));
        // A point on the PARENT with the submenu open folds the
        // subtree and acts on the parent row.
        assert!(m.sub.is_some());
        let (r0, _) = m.rows[0];
        assert_eq!(m.click(r0.x + 5.0, r0.y + 5.0), MenuOut::Pick(1));
        assert!(m.sub.is_none());
        // Outside everything: close.
        assert_eq!(m.click(700.0, 700.0), MenuOut::Close);
    }

    // ---- a11y growth ----

    #[test]
    fn hit_rects_grow_symmetrically_and_never_shrink() {
        let g = grown(Rect::new(10.0, 10.0, 200.0, 23.9), 24.0);
        assert!((g.y - 9.95).abs() < 1e-4);
        assert!((g.h - 24.0).abs() < 1e-4);
        assert_eq!(g.x, 10.0, "already past the floor: untouched");
        assert_eq!(g.w, 200.0);
        let same = grown(Rect::new(0.0, 0.0, 30.0, 30.0), 24.0);
        assert_eq!((same.x, same.y, same.w, same.h), (0.0, 0.0, 30.0, 30.0));
    }

    // ---- entry heights ----

    #[test]
    fn a_rule_takes_its_breathing_plus_stroke() {
        assert_eq!(entry_h(&MenuEntry::Rule, 24.0, 8.4, 1.2), 18.0);
        assert_eq!(entry_h(&item("X", 1), 24.0, 8.4, 1.2), 24.0);
    }

    // ---- accessible reporting --------------------------------------------

    /// Draws `f` twice against a real [`crate::focus::FocusCtl`]
    /// (`drawn_text`'s harness leaves `focus: None`, which is right for
    /// the face tests but answers nothing here) and hands back the
    /// second, completed frame's registrations.
    ///
    /// Twice, not once: a level's `opened_t` is stamped on its OWN first
    /// draw ([`MenuState::draw`]'s very first lines) — a submenu opened
    /// by `Key::Right` inside `f` reaches that first draw with a clock
    /// that has not started yet, so `motion.menu_unfold` reads 0 elapsed
    /// and nothing of it is visible (or registered) THAT frame, exactly
    /// as `click_routes_to_the_deepest_level_first` above has to fabricate
    /// geometry rather than draw once for the same reason. A second draw,
    /// a long stretch of clock later, catches every level at rest.
    fn registered(mut f: impl FnMut(&mut Ctx)) -> Vec<(FocusId, Rect, AccessInfo)> {
        use crate::draw::DrawList;
        use crate::focus::FocusCtl;
        use crate::font::FontSystem;
        use crate::pointer::Pointer;
        let mut fc = FocusCtl::new();
        for t in [1000.0, 2000.0] {
            let mut dl = DrawList::new();
            let mut fonts = FontSystem::new();
            let mut ctx = Ctx {
                access: None,
                dl: &mut dl,
                fonts: &mut fonts,
                w: 1920.0,
                h: 1080.0,
                t,
                mouse: Pointer::new(-1.0, -1.0),
                term_font_scale: 1.0,
                ui_font_scale: 1.0,
                panel_scale: 1.0,
                focus: Some(&mut fc),
                tips: None,
            };
            f(&mut ctx);
            fc.begin_frame();
        }
        fc.entries().map(|(id, r, a)| (id, r, a.clone())).collect()
    }

    /// One row per ITEM — never the rule — each `Role::MenuItem`, named
    /// for its label, and positioned in a set that does not count the
    /// rule either: COPY / (rule) / CLEAR(disabled) / PASTE is items
    /// 1..3, not 1..4.
    #[test]
    fn rows_register_as_menu_items_positioned_among_items_only() {
        let mut m = menu();
        let rows = registered(|ctx| {
            m.draw(ctx);
        });
        assert_eq!(rows.len(), 3, "COPY, CLEAR, PASTE — the rule registers nothing");
        let names: Vec<&str> = rows.iter().map(|(_, _, a)| a.name.as_str()).collect();
        assert_eq!(names, ["COPY", "CLEAR", "PASTE"]);
        for (_, _, a) in &rows {
            assert_eq!(a.role, Role::MenuItem);
        }
        assert_eq!(rows[0].2.index, Some((1, 3)));
        assert_eq!(rows[1].2.index, Some((2, 3)));
        assert_eq!(rows[2].2.index, Some((3, 3)));
    }

    /// The keyboard highlight — and only it — carries `States::SELECTED`;
    /// an unhighlighted row's states stay empty, disabled or not.
    #[test]
    fn only_the_highlighted_row_reports_selected() {
        let mut m = menu();
        m.key(&ev(Key::Down)); // highlights COPY (row 0, item 1)
        let rows = registered(|ctx| {
            m.draw(ctx);
        });
        assert!(rows[0].2.states.contains(States::SELECTED), "COPY is highlighted");
        assert!(!rows[1].2.states.contains(States::SELECTED), "CLEAR is not");
        assert!(!rows[2].2.states.contains(States::SELECTED), "PASTE is not");
    }

    /// A parent level and its one open submenu draw in the same frame
    /// (submenu on top); their rows must not collide on id, or a bridge
    /// reading both would see one overwrite the other.
    #[test]
    fn a_submenu_s_rows_never_share_an_id_with_its_parent_s() {
        let mut m = with_sub();
        m.key(&ev(Key::Down)); // PLAIN
        m.key(&ev(Key::Down)); // MORE
        m.key(&ev(Key::Right)); // opens the submenu
        let rows = registered(|ctx| {
            m.draw(ctx);
        });
        // PLAIN, MORE (parent level) + A, B (submenu level).
        assert_eq!(rows.len(), 4);
        let ids: Vec<_> = rows.iter().map(|(id, ..)| *id).collect();
        for (idx, id) in ids.iter().enumerate() {
            assert!(
                ids.iter().skip(idx + 1).all(|other| other != id),
                "id {id:?} (row {idx}) repeats in the same frame: {ids:?}"
            );
        }
    }

    // ---- face -----------------------------------------------------------
    //
    // A menu row is TWO roles — `menu.item.role` for the label and
    // `menu.hint_role` for the shortcut column — so the claim is made
    // twice, once per role, over a run in which both are on screen.

    /// The four rows of the face fixtures: labels that cannot be
    /// mistaken for hints, and hints that are shortcut strings with
    /// FIGURES in them, since the hint column is right-aligned and
    /// therefore the column `tabular` exists for.
    fn hinted() -> MenuState {
        MenuState::open_at(
            vec![
                MenuEntry::Item(
                    MenuItem::new("COPY", 1).with_hint(Some("Ctrl+1".to_string())),
                ),
                MenuEntry::Item(
                    MenuItem::new("PASTE", 2).with_hint(Some("Ctrl+8".to_string())),
                ),
                MenuEntry::Rule,
                // The widest label and the widest hint on ONE row, and
                // both long enough that the menu is sized by its content
                // rather than resting on `menu.min_w`: on this row the
                // slack between the two columns is the master's gap
                // exactly, which is what makes the check below tight.
                MenuEntry::Item(
                    MenuItem::new("SELECT EVERYTHING IN THIS BUFFER", 3)
                        .with_hint(Some("Ctrl+Shift+188".to_string())),
                ),
            ],
            40.0,
            40.0,
            0.0,
        )
    }

    fn is_hint(s: &str) -> bool {
        s.starts_with("Ctrl+")
    }

    /// Row labels are set in the face `menu.item.role` names, and follow
    /// a theme that moves it.
    #[test]
    fn a_row_label_is_set_in_the_face_its_role_names() {
        face_follows_the_theme("menu-item", "object::menu::tests::child_label_face");
    }

    /// The shortcut column is set in the face `menu.hint_role` names —
    /// its OWN role, not the label's — and follows a theme that moves it.
    #[test]
    fn a_shortcut_hint_is_set_in_the_face_its_own_role_names() {
        face_follows_the_theme("menu-hint", "object::menu::tests::child_hint_face");
    }

    #[test]
    #[ignore = "measured in a process of its own by the test above"]
    fn child_label_face() {
        static PROBE: OnceLock<TokenId> = OnceLock::new();
        let want = ui::bound_role(&PROBE, "menu.item.role").font();
        let drawn: Vec<(u8, String)> = drawn_text(|ctx| {
            hinted().draw(ctx);
        })
        .into_iter()
        .filter(|(_, s)| !is_hint(s))
        .collect();
        assert_eq!(drawn.len(), 3, "three labels are on screen: {drawn:?}");
        all_in(&drawn, want);
        report(&role_word("menu.item.role"), want, &drawn);
    }

    #[test]
    #[ignore = "measured in a process of its own by the test above"]
    fn child_hint_face() {
        static PROBE: OnceLock<TokenId> = OnceLock::new();
        let want = ui::bound_role(&PROBE, "menu.hint_role").font();
        let drawn: Vec<(u8, String)> = drawn_text(|ctx| {
            hinted().draw(ctx);
        })
        .into_iter()
        .filter(|(_, s)| is_hint(s))
        .collect();
        assert_eq!(drawn.len(), 3, "three hints are on screen: {drawn:?}");
        all_in(&drawn, want);
        report(&role_word("menu.hint_role"), want, &drawn);
    }

    // ---- figures --------------------------------------------------------

    /// `type.<hint role>.tabular` reaches the shortcut column, and the
    /// column the menu SIZED is the column it DREW.
    ///
    /// The second half is the half a slot number cannot witness. The
    /// menu's width comes from a measuring pass over the same strings;
    /// if that pass measured proportionally while the row loop drew
    /// under a figure box — the state this batch found the file in one
    /// rung down, with the face — every hint would be drawn wider than
    /// the column it was sized for and would reach back over its own
    /// label. The child computes both edges from the register and the
    /// fonts, so the overlap is a number rather than a look.
    #[test]
    fn the_shortcut_column_is_drawn_at_the_width_it_was_sized_at() {
        let child = "object::menu::tests::child_hint_columns";
        let master = measure_in_child(child, None);
        assert_eq!(
            master.field("FIG="),
            "0",
            "the master leaves the hint column proportional, so a box here \
             comes from somewhere the theme cannot see:\n{}",
            master.log
        );
        let role = role_word("menu.hint_role");
        let path = std::env::temp_dir()
            .join(format!("nacelle-face-menu-fig-{}.theme", std::process::id()));
        std::fs::write(
            &path,
            format!(
                "[meta]\nschema = 1\nname = \"figure fixture\"\nbase = \"default\"\n\n\
                 [type]\n{role}.tabular = true\n"
            ),
        )
        .expect("the fixture theme must be writable");
        let boxed = measure_in_child(child, Some(&path));
        let _ = std::fs::remove_file(&path);
        let adv: f32 = boxed.field("FIG=").parse().expect("FIG= must be a length");
        assert!(
            adv > 0.0,
            "a theme put `type.{role}.tabular = true` and the hints reached the \
             draw list with no figure box at all:\n{}",
            boxed.log
        );
    }

    /// The child of the test above: draws the hinted menu, then checks
    /// every hint's left edge against its own row's label, measured with
    /// the very face, px, tracking and box the register says each run
    /// was drawn with. Reports the figure advance the hints carried.
    #[test]
    #[ignore = "measured in a process of its own by the test above"]
    fn child_hint_columns() {
        use crate::draw::{DrawCmd, TextAnchor};
        use crate::font::FontSystem;
        static LABEL: OnceLock<TokenId> = OnceLock::new();
        static HINT: OnceLock<TokenId> = OnceLock::new();
        let runs = crate::object::panel::tests::drawn_runs(|ctx| {
            hinted().draw(ctx);
        });
        let label_role = ui::bound_role(&LABEL, "menu.item.role");
        let hint_role = ui::bound_role(&HINT, "menu.hint_role");
        let mut fonts = FontSystem::new();
        // Each run measured back in the role it belongs to — the same
        // face, the same figure box, through the same single resolver
        // (`ui::figures`) the drawing side used. The px and the tracking
        // come from the register, so they are what was drawn and not a
        // second reading of the theme.
        let mut width = |c: &DrawCmd| match c {
            DrawCmd::Text { font, px, tracking, text, .. } => {
                let role = if is_hint(text) { hint_role } else { label_role };
                let fig = role.figures(&mut fonts, *font, *px);
                fonts.measure_fig(*font, *px, text, *tracking, &fig)
            }
            _ => 0.0,
        };
        let is_h = |c: &&DrawCmd| matches!(c, DrawCmd::Text { text, .. } if is_hint(text));
        let labels: Vec<&DrawCmd> = runs.iter().filter(|c| !is_h(c)).collect();
        let hints: Vec<&DrawCmd> = runs.iter().filter(is_h).collect();
        assert_eq!(labels.len(), 3);
        assert_eq!(hints.len(), 3);
        // The gap the master keeps between the two columns is the whole
        // of the box's slack: the menu's width is `inset + widest label +
        // gap + widest hint + inset`, so on the widest row the distance
        // between the label's right edge and the hint's left edge is the
        // gap EXACTLY. Anything less means the width was computed from a
        // measure the draw did not use, and the shortfall is the
        // difference between the two.
        let t = theme::resolved();
        let px_of = |k: &str| t.px(theme::id(k).expect("the master declares this key"));
        let gap = px_of("menu.hint_gap");
        let min_w = px_of("menu.min_w");
        let mut adv = 0.0f32;
        let mut tightest = f32::INFINITY;
        for (l, h) in labels.iter().zip(&hints) {
            let (DrawCmd::Text { at: la, .. }, DrawCmd::Text { at: ha, anchor, tabular, .. }) =
                (l, h)
            else {
                unreachable!()
            };
            assert_eq!(*anchor, TextAnchor::Right, "the hint column is right-aligned");
            adv = adv.max(*tabular);
            let label_right = la[0] + width(l);
            let hint_left = ha[0] - width(h);
            tightest = tightest.min(hint_left - label_right);
        }
        // The fixture's own precondition: a menu resting on `menu.min_w`
        // would have slack that no measuring mistake could eat, and the
        // check above would pass without meaning anything.
        let widest = labels.iter().map(|c| width(c)).fold(0.0f32, f32::max)
            + hints.iter().map(|c| width(c)).fold(0.0f32, f32::max)
            + gap
            + 2.0 * px_of("menu.item_inset");
        assert!(
            widest > min_w,
            "these rows fit inside `menu.min_w` ({min_w} px), so the menu was sized \
             by its floor and not by its content: the check below proves nothing"
        );
        assert!(
            tightest >= gap - 0.01,
            "the columns are {tightest} px apart where the master asks for {gap}: the \
             menu was sized by one measure and drawn by another, and the shortfall \
             is the difference between them"
        );
        println!("ROLE={}", crate::object::panel::tests::role_word("menu.hint_role"));
        println!("FACE={}", hint_role.font());
        println!("FIG={adv}");
    }
}
