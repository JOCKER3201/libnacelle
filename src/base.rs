//! Core widget framework: geometry, the panel/layout model and the
//! drawing context shared by every nacelle widget.

use crate::draw::DrawList;
use crate::focus::FocusCtl;
use crate::font::{FontSystem, FONT_UI};
use std::sync::{OnceLock, RwLock};

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn cx(&self) -> f32 {
        self.x + self.w / 2.0
    }
}

/// How large the interface type is before any setting touches it.
///
/// Every size a widget asks for is a multiple of this, so it moves the
/// whole interface at once rather than one label at a time — and it is
/// separate from UIFontSize= so that setting still means what it says:
/// 100% is the size the interface was designed at, not a correction.
pub const UI_FONT_BASE: f32 = 1.3;

/// Panel position and size in vw/vh units (percent of the window).
#[derive(Clone, Copy, Debug)]
pub struct PanelSpec {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

/// A KIND of widget — one entry of the widget registry.
///
/// This is only an index into the registry, which is built at startup
/// by scanning the addons directory. Everything a widget is — its name,
/// label, default sizes and how it draws — comes from that registry, so
/// adding a widget never means touching this type.
///
/// It is deliberately NOT a placement. Where a widget stands, and how
/// many times it stands there, is one [`crate::layout::Instance`] per
/// appearance ([`crate::layout::InstanceList`]); a `Panel` only says
/// which of them is running.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Panel(pub u16);

impl Panel {
    pub fn idx(self) -> usize {
        self.0 as usize
    }

    /// Every registered widget KIND, in registry order. Says nothing
    /// about what a layout places: a kind may be placed twice, or not
    /// at all — ask the layout's instance list for that.
    pub fn all() -> Vec<Panel> {
        (0..panel_count() as u16).map(Panel).collect()
    }

    fn def(self) -> Option<&'static WidgetDef> {
        registry().get(self.idx())
    }

    /// Name used in .layaut files.
    pub fn name(self) -> &'static str {
        self.def().map(|d| d.name.as_str()).unwrap_or("?")
    }

    /// Label shown in the layout editor.
    pub fn label(self) -> &'static str {
        self.def().map(|d| d.label.as_str()).unwrap_or("?")
    }

    /// Which kind of board this widget may be placed on.
    pub fn category(self) -> WidgetCategory {
        self.def().map(|d| d.category).unwrap_or_default()
    }

    /// Which column of a generated composition the widget asked for.
    pub fn slot(self) -> PanelSlot {
        self.def().map(|d| d.slot).unwrap_or_default()
    }

    /// Where in its column the widget asked to sit; lower comes first.
    pub fn order(self) -> f32 {
        self.def().map(|d| d.order).unwrap_or(0.0)
    }

    /// How much of a shared column the widget asked for. A widget that
    /// named no weight wants as much of the column as it is tall — the
    /// only answer that needs nothing beyond what it already declared.
    pub fn weight(self) -> f32 {
        self.def()
            .map(|d| d.weight.unwrap_or(d.ref_h_vh))
            .unwrap_or(0.0)
    }

    /// Where the layout engine pins the widget, if anywhere.
    pub fn anchor(self) -> PanelAnchor {
        self.def().map(|d| d.anchor).unwrap_or_default()
    }

    /// Whether the widget declared itself impossible to switch off.
    pub fn essential(self) -> bool {
        self.def().map(|d| d.essential).unwrap_or(false)
    }

    pub fn from_name(name: &str) -> Option<Panel> {
        registry()
            .iter()
            .position(|d| d.name.eq_ignore_ascii_case(name))
            .map(|i| Panel(i as u16))
    }

    /// Reference height (vh) at which the widget renders at 100% scale.
    /// Enlarging a panel past its reference box scales the whole widget,
    /// fonts included. This is a LAYOUT property, not a widget one — a
    /// layout may give the same widget a different reference — so it
    /// comes from the size table the current layout installed.
    pub fn ref_h_vh(self) -> f32 {
        sizes()
            .read()
            .ok()
            .and_then(|s| s.get(self.idx()).map(|(r, _)| *r))
            .unwrap_or(10.0)
    }

    /// The height this widget's content actually needs at the width it
    /// has been given, or None when the widget grows to whatever height
    /// it gets — a table that shows more rows, a terminal that shows
    /// more lines. Measured once a frame, before the layout runs.
    pub fn intrinsic_h(self) -> Option<f32> {
        intrinsic()
            .read()
            .ok()
            .and_then(|v| v.get(self.idx()).copied())
            .flatten()
    }

    /// Minimum content height (vh) the layout engine keeps for it.
    pub fn min_h_vh(self) -> f32 {
        sizes()
            .read()
            .ok()
            .and_then(|s| s.get(self.idx()).map(|(_, m)| *m))
            .unwrap_or(6.0)
    }

}

/// Which kind of board a widget can be placed on. The widget itself
/// declares it — a `category` line in its own metadata — because a
/// launcher belongs on the APPGRID board wherever its file happens to
/// live: `Board` for the ordinary boards, `Appgrid` for the bottom
/// fixture board, `SearchAi` for the top one. Naming none, or naming
/// one this version has never heard of, is a board widget.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum WidgetCategory {
    /// Home and the horizontal arms.
    #[default]
    Board,
    /// The bottom fixture board.
    Appgrid,
    /// The top fixture board.
    SearchAi,
}

/// Which column of a GENERATED composition a widget asks for.
///
/// A machine's board is whatever it has installed, so the arrangement
/// the program falls back to has to be composed rather than written
/// down. The three columns are the shape of that composition — two
/// instrument sides and a wide work surface — and a widget names the
/// one it belongs in exactly as it names its board. Naming none is the
/// honest answer for most widgets: the composition drops them into the
/// emptier side, so nothing an installation holds is left off the
/// board.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PanelSlot {
    #[default]
    Auto,
    Left,
    Center,
    Right,
}

/// Where the layout engine pins a panel, whatever column it was put in.
///
/// The engine has three pinned positions because the composition has
/// three edges that mean something — the top and the bottom of the work
/// surface, and the bar under everything — and a widget asks for one
/// the same way it asks for a column. Nothing here is a widget's name:
/// an installation with no terminal simply pins nothing to the top.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PanelAnchor {
    /// Flows with the column it was put in.
    #[default]
    Flow,
    /// Pinned to the top of the work column.
    Top,
    /// Pinned to the bottom of the work column.
    Bottom,
    /// Pinned to the bottom of the first column, and brought back as a
    /// full-width bar at the bottom of the window when that column
    /// collapses. Portrait gives it a band of its own.
    Bar,
}

/// Everything the program knows about one widget. The widget itself is
/// its file — `addons/scripts/<name>.rhai` or `addons/plugins/<name>.so`
/// — and every field here is declared by that addon (see
/// `widget::registry`); what is kept is only what the layout engine and
/// the editor need to know about it before it draws.
#[derive(Clone, Debug)]
pub struct WidgetDef {
    /// Name used in .layaut files and as the directory name.
    pub name: String,
    /// Label shown in the layout editor.
    pub label: String,
    pub ref_h_vh: f32,
    pub min_h_vh: f32,
    /// Which kind of board this widget may be placed on.
    pub category: WidgetCategory,
    /// Which column of a generated composition it asks for.
    pub slot: PanelSlot,
    /// Where in that column it asks to sit; lower comes first, and
    /// widgets that ask for the same place keep registry order.
    pub order: f32,
    /// How much of a shared column it asks for. None = as much as it is
    /// tall (`ref_h_vh`).
    pub weight: Option<f32>,
    /// Where the layout engine pins it, if anywhere.
    pub anchor: PanelAnchor,
    /// The widget declares that switching it off would leave the user
    /// with no way back — the editor never offers to remove it.
    pub essential: bool,
}

static REGISTRY: OnceLock<Vec<WidgetDef>> = OnceLock::new();

/// Per-panel (reference height, minimum height) in vh, indexed like the
/// registry. Held apart from the registry and mutable, because these
/// belong to the LAYOUT: selecting another layout replaces them, while
/// the registry itself is fixed once panel indices are in use.
fn sizes() -> &'static RwLock<Vec<(f32, f32)>> {
    static S: OnceLock<RwLock<Vec<(f32, f32)>>> = OnceLock::new();
    S.get_or_init(|| RwLock::new(default_sizes()))
}

/// The sizes a layout falls back to when it names none of its own —
/// each widget's own defaults, straight from the registry that the
/// directory scan built.
pub fn default_sizes() -> Vec<(f32, f32)> {
    registry().iter().map(|d| (d.ref_h_vh, d.min_h_vh)).collect()
}

/// What each widget measured itself at this frame, indexed like the
/// registry. None means the widget grows into whatever it is given.
fn intrinsic() -> &'static RwLock<Vec<Option<f32>>> {
    static I: OnceLock<RwLock<Vec<Option<f32>>>> = OnceLock::new();
    I.get_or_init(|| RwLock::new(Vec::new()))
}

/// The height the host's container adds around each panel's content —
/// border, vertical padding, and the title band when the widget
/// declares one. Indexed like the registry; 0.0 for a panel nobody
/// measured. The layout engine adds it to the content minimums, so a
/// panel kept "at its minimum" still shows its last content row under
/// a band, instead of losing exactly the band's height of content.
fn chrome() -> &'static RwLock<Vec<f32>> {
    static C: OnceLock<RwLock<Vec<f32>>> = OnceLock::new();
    C.get_or_init(|| RwLock::new(Vec::new()))
}

/// The per-world size table (u3 §3): what the flex solver solves
/// against and what `panel_font_scale` rescales by. Per-WORLD, not
/// per-process — two outputs under a compositor hold two of these,
/// each with its own intrinsic measurements. The process-wide setters
/// below keep feeding one global instance for the desktop of today;
/// `size_table()` snapshots it, and an embedder that owns its world
/// builds its own.
#[derive(Clone, Debug, Default)]
pub struct SizeTable {
    sizes: Vec<(f32, f32)>,
    intrinsic: Vec<Option<f32>>,
    chrome: Vec<f32>,
    /// What individual INSTANCES measured themselves at, when the host
    /// measures them one by one: (id, intrinsic, chrome).
    ///
    /// Two terminals hold two different amounts of text, so the honest
    /// measurement is per instance. The three vectors above are the
    /// per-KIND answer a host that still runs one widget of each kind
    /// publishes, and they stay the fallback — a host adopts this seam
    /// instance by instance rather than all at once.
    by_instance: Vec<(crate::layout::InstanceId, Option<f32>, f32)>,
}

impl SizeTable {
    pub fn new(
        sizes: Vec<(f32, f32)>,
        intrinsic: Vec<Option<f32>>,
        chrome: Vec<f32>,
    ) -> Self {
        Self { sizes, intrinsic, chrome, by_instance: Vec::new() }
    }

    /// Records what ONE instance measured itself at this frame,
    /// replacing an earlier measurement for the same identity.
    pub fn set_instance(
        &mut self,
        id: crate::layout::InstanceId,
        intrinsic: Option<f32>,
        chrome: f32,
    ) {
        match self.by_instance.iter_mut().find(|(i, _, _)| *i == id) {
            Some(slot) => *slot = (id, intrinsic, chrome),
            None => self.by_instance.push((id, intrinsic, chrome)),
        }
    }

    /// What THIS instance measured itself at; the widget kind's
    /// measurement while nobody has measured this instance itself.
    pub fn intrinsic_of(&self, id: crate::layout::InstanceId, p: Panel) -> Option<f32> {
        match self.by_instance.iter().find(|(i, _, _)| *i == id) {
            Some((_, ih, _)) => *ih,
            None => self.intrinsic_h(p),
        }
    }

    /// What the container draws around THIS instance; the kind's
    /// chrome while nobody has measured this instance itself.
    pub fn chrome_of(&self, id: crate::layout::InstanceId, p: Panel) -> f32 {
        match self.by_instance.iter().find(|(i, _, _)| *i == id) {
            Some((_, _, c)) => *c,
            None => self.chrome_h(p),
        }
    }

    /// Reference height (vh); 10.0 for a panel the table does not name.
    pub fn ref_h_vh(&self, p: Panel) -> f32 {
        self.sizes.get(p.idx()).map(|s| s.0).unwrap_or(10.0)
    }

    /// Minimum content height (vh); 6.0 when unnamed.
    pub fn min_h_vh(&self, p: Panel) -> f32 {
        self.sizes.get(p.idx()).map(|s| s.1).unwrap_or(6.0)
    }

    /// What the widget measured itself at this frame; None grows.
    pub fn intrinsic_h(&self, p: Panel) -> Option<f32> {
        self.intrinsic.get(p.idx()).copied().flatten()
    }

    /// What the host's container adds around this panel's content
    /// (border, padding, title band); 0.0 when nobody measured it.
    pub fn chrome_h(&self, p: Panel) -> f32 {
        self.chrome.get(p.idx()).copied().unwrap_or(0.0)
    }
}

/// A snapshot of the process-wide table the setters below feed.
pub fn size_table() -> SizeTable {
    SizeTable {
        sizes: sizes().read().map(|s| s.clone()).unwrap_or_default(),
        intrinsic: intrinsic().read().map(|i| i.clone()).unwrap_or_default(),
        chrome: chrome().read().map(|c| c.clone()).unwrap_or_default(),
        by_instance: Vec::new(),
    }
}

/// Publishes this frame's measurements. The layout engine reads them to
/// give a widget with finite content exactly the height it needs, and
/// to share what is left among the ones that can use more.
pub fn set_panel_intrinsic(new: &[Option<f32>]) {
    if let Ok(mut i) = intrinsic().write() {
        i.clear();
        i.extend_from_slice(new);
    }
}

/// Publishes what the container will draw around each panel this frame.
/// Kept apart from the intrinsic heights: clearing the measurements for
/// a probe pass must not also forget the chrome — chrome depends on the
/// widget's title declaration and the theme, not on the box the panel
/// happened to get, so it cannot feed back into the probe.
pub fn set_panel_chrome(new: &[f32]) {
    if let Ok(mut c) = chrome().write() {
        c.clear();
        c.extend_from_slice(new);
    }
}

/// Installs the sizes the current layout asks for. Entries past the end
/// keep their defaults, so a layout only has to name what it changes.
pub fn set_panel_sizes(new: &[(Panel, f32, f32)]) {
    let mut table = default_sizes();
    for (p, r, m) in new {
        if let Some(slot) = table.get_mut(p.idx()) {
            *slot = (
                if r.is_finite() && *r > 0.0 { *r } else { slot.0 },
                if m.is_finite() && *m > 0.0 { *m } else { slot.1 },
            );
        }
    }
    if let Ok(mut s) = sizes().write() {
        *s = table;
    }
}

/// Installs the widget registry. The FIRST call wins; later ones are
/// ignored, because panel indices are baked into layouts and rectangles
/// the moment the first frame is drawn.
pub fn set_registry(defs: Vec<WidgetDef>) {
    let _ = REGISTRY.set(defs);
}

/// The widget registry — whatever the embedder installed, and nothing
/// else. There is no fallback set and no table of known names: a widget
/// is an addon on disk (or a plugin crate linked into the program), so
/// a machine with none installed has none, exactly as a machine with no
/// theme installed draws like a page with no stylesheet. The embedder
/// says so out loud; the toolkit does not invent widgets to hide it.
pub fn registry() -> &'static [WidgetDef] {
    REGISTRY.get_or_init(Vec::new)
}

pub fn panel_count() -> usize {
    registry().len()
}

/// A panel placed far outside the window = hidden.
pub const OFF_SPEC: PanelSpec = PanelSpec { x: 200.0, y: 0.0, w: 20.0, h: 25.0 };

/// Panel layout — positions of all panels as a VERSION 1 `.layaut` file
/// held them: one rectangle per widget, indexed by registry position.
///
/// This is the shape the format has grown out of ([`crate::layout::
/// InstanceList`] replaced it, so that a widget can be placed twice).
/// It survives as what the version 1 reader produces, and lives only
/// long enough for the migration to turn it into instances.
#[derive(Clone)]
pub struct LayoutSpec {
    pub panels: Vec<PanelSpec>,
}

impl LayoutSpec {
    pub fn p(&self, p: Panel) -> &PanelSpec {
        self.panels.get(p.idx()).unwrap_or(&OFF_SPEC)
    }
    pub fn set(&mut self, p: Panel, s: PanelSpec) {
        if self.panels.len() <= p.idx() {
            self.panels.resize(p.idx() + 1, OFF_SPEC);
        }
        self.panels[p.idx()] = s;
    }
}

impl Default for LayoutSpec {
    fn default() -> Self {
        LayoutSpec { panels: vec![OFF_SPEC; panel_count()] }
    }
}

/// One entry of a flex column: WHICH instance stands there, and how
/// much of the column it asked for.
///
/// The instance, not the widget: two terminals may share one column,
/// and only their identities tell the solver's two rectangles apart.
/// `widget` rides along because everything the solver asks — the
/// anchor, the minimum height, the intrinsic measurement — is a
/// property of the KIND, and looking it up through the layout's list on
/// every access would buy nothing.
#[derive(Clone, Copy, Debug)]
pub struct ColumnItem {
    pub id: crate::layout::InstanceId,
    pub widget: Panel,
    /// Share of the column the instance asked for (its height weight).
    pub weight: f32,
}

/// One flexbox column: CSS-like width constraints plus instances
/// stacked top to bottom with height weights.
#[derive(Clone)]
pub struct FlexColumn {
    /// Preferred width as a percentage of the row (flex-basis).
    pub basis: f32,
    /// Minimum width in px (min-width).
    pub min: f32,
    /// Maximum width in px (max-width); INFINITY = unlimited.
    pub max: f32,
    /// Share of the leftover space (flex-grow).
    pub grow: f32,
    /// Collapse priority when space runs out: 1 disappears first,
    /// then 2, ...; 0 = never hidden.
    pub collapse: u32,
    /// Vertical gap between the panels, in height weight units.
    pub gap: f32,
    /// Instances top to bottom with their height weights.
    pub panels: Vec<ColumnItem>,
}

/// A flexbox layout: columns laid out left to right.
#[derive(Clone)]
pub struct FlexLayaut {
    pub columns: Vec<FlexColumn>,
    /// `units = px` in the file: min/max are literal device pixels. Default
    /// false = device-independent units scaled by the window height
    /// (flex.rs::lu), so one composition comes out at 720p and at 4K.
    pub units_px: bool,
    /// `pad_x = <percent>` in the file: page padding per side, percent of
    /// the window width. None = the engine's own margin. A layout that
    /// wants clear outer margin (u1 §4.3's instrument arrangement, room
    /// for decor.dump columns) is the one that names it.
    pub pad_x: Option<f32>,
}

/// How a board's instances are placed (see src/layout/flex.rs).
#[derive(Clone)]
pub enum LayoutMode {
    /// Built-in responsive default: a flexbox tree composed from what
    /// the board's instances declare, computed from the actual window
    /// size every frame.
    Flex,
    /// A custom flexbox .layaut file — same engine as the default.
    Custom(FlexLayaut),
    /// Explicit rectangles: every instance the board holds sits at its
    /// own `rect`, re-adapted to the window every frame.
    ///
    /// The rectangles are NOT here. They are on the instances
    /// ([`crate::layout::Instance::rect`]), which is what lets the same
    /// widget hold two of them; this variant only says that the board
    /// reads them.
    Rects,
}

impl Default for LayoutMode {
    fn default() -> Self {
        LayoutMode::Flex
    }
}

/// One solved instance: who it is, what it runs, and the OUTER
/// rectangle the engine gave it, in physical pixels.
#[derive(Clone, Copy, Debug)]
pub struct Placed {
    pub id: crate::layout::InstanceId,
    pub widget: Panel,
    pub rect: Rect,
}

/// Computed rectangles of one solved board — one entry per placed
/// INSTANCE, in the order the board places them.
///
/// Keyed by instance and not by widget: a board with two terminals has
/// two entries whose `widget` is the same, and the host tells its two
/// shells apart by `id`.
pub struct Layout {
    placed: Vec<Placed>,
    /// Index from instance identity to its position in `placed`, so
    /// `place()` finds an existing entry without scanning the vector.
    index: std::collections::HashMap<crate::layout::InstanceId, usize>,
    /// Where an instance this layout does not hold is reported to be:
    /// far to the RIGHT of the window. Every presence scan in the
    /// program asks `rect.x < w`, so the absent must answer from
    /// outside — a negative sentinel would read as "on screen".
    off: Rect,
}

impl Layout {
    /// A board with nothing placed yet — the starting point of every
    /// layout engine.
    pub fn empty(w: f32, h: f32) -> Layout {
        Layout {
            placed: Vec::new(),
            index: std::collections::HashMap::new(),
            off: Rect::new(w * 2.0, 0.0, w * 0.16, h * 0.6),
        }
    }

    /// Places one instance, replacing an earlier rectangle for the same
    /// identity.
    pub fn place(&mut self, id: crate::layout::InstanceId, widget: Panel, rect: Rect) {
        match self.index.get(&id) {
            Some(&i) => self.placed[i].rect = rect,
            None => {
                self.index.insert(id, self.placed.len());
                self.placed.push(Placed { id, widget, rect });
            }
        }
    }

    /// The rectangle of one instance; off-screen when this board does
    /// not hold it.
    pub fn of(&self, id: crate::layout::InstanceId) -> Rect {
        self.placed.iter().find(|p| p.id == id).map(|p| p.rect).unwrap_or(self.off)
    }

    /// The rectangle of the FIRST instance of a widget kind.
    ///
    /// For the questions that really are about the kind — "is this
    /// widget on any board at all", the presence scan widget lifetime
    /// hangs on. A caller that draws, or that answers a click, wants
    /// [`Layout::of`]: with two terminals on the board, "the first one"
    /// is an arbitrary one.
    pub fn p(&self, p: Panel) -> Rect {
        self.placed
            .iter()
            .find(|x| x.widget == p)
            .map(|x| x.rect)
            .unwrap_or(self.off)
    }

    /// Every placed instance, in placement order.
    pub fn iter(&self) -> std::slice::Iter<'_, Placed> {
        self.placed.iter()
    }

    pub fn all(&self) -> &[Placed] {
        &self.placed
    }

    /// Every placed instance running the given widget kind — one entry
    /// per terminal on the board, not "the terminal".
    pub fn instances_of(&self, p: Panel) -> Vec<Placed> {
        self.placed.iter().filter(|x| x.widget == p).copied().collect()
    }

    pub fn len(&self) -> usize {
        self.placed.len()
    }

    pub fn is_empty(&self) -> bool {
        self.placed.is_empty()
    }

    /// The instance under a point, last placed first — the reading a
    /// click wants, since later placements draw over earlier ones.
    pub fn hit(&self, x: f32, y: f32) -> Option<Placed> {
        self.placed.iter().rev().find(|p| p.rect.contains(x, y)).copied()
    }

    /// Derives the INNER content containers from the OUTER instance
    /// rectangles: the inner container is exactly the widget's content
    /// area, and the outer rectangle (the resize edge) is ALWAYS `pad`
    /// larger than it on every side. Drawing and hit-testing use the
    /// inner containers; the outer rectangles stay authoritative for
    /// layout files and the grid editor (which keeps panels large
    /// enough for the padding plus some content).
    pub fn padded(&self, pad: f32) -> Layout {
        let pad = pad.max(0.0);
        let ins = |r: Rect| {
            Rect::new(
                r.x + pad,
                r.y + pad,
                (r.w - 2.0 * pad).max(2.0),
                (r.h - 2.0 * pad).max(2.0),
            )
        };
        Layout {
            placed: self
                .placed
                .iter()
                .map(|p| Placed { rect: ins(p.rect), ..*p })
                .collect(),
            index: self.index.clone(),
            off: self.off,
        }
    }
}

/// Drawing context passed to the panels.
pub struct Ctx<'a> {
    pub dl: &'a mut DrawList,
    pub fonts: &'a mut FontSystem,
    /// Window width/height in px.
    pub w: f32,
    pub h: f32,
    /// Time since application start, in seconds.
    pub t: f64,
    /// The pointer — and who is allowed to see it ([`crate::pointer`]).
    ///
    /// It was a bare `(f32, f32)`, and that was the fault: a position on
    /// its own answers every control the same way, so a keyboard cap
    /// under an open window lit up exactly as readily as the window's own
    /// row over it. A control asks [`Pointer::at`] (or, better,
    /// [`Pointer::over`]) and is answered against what has been drawn
    /// over it; a caller PLACING something at the cursor — a tooltip
    /// choosing a side, a menu opening where the click landed — asks
    /// [`Pointer::raw`].
    ///
    /// Owned by the frame and handed back to the application afterwards:
    /// what covered the pointer is the one thing about a frame the next
    /// frame needs.
    pub mouse: crate::pointer::Pointer,
    /// Terminal font size multiplier (TermFontSize= in nacelle-desktop.conf).
    pub term_font_scale: f32,
    /// Interface font size multiplier (UIFontSize= in nacelle-desktop.conf).
    ///
    /// **Already applied to every theme length.** The host hands this same
    /// number to [`crate::theme::set_viewport`] as `metric.ui_scale`, which
    /// multiplies u — and u is what every size, gap and row height in the
    /// master is written in. So a token, a role's `px` or anything derived
    /// from either must NOT be multiplied by it again; doing so squares the
    /// user's setting, and 125 % draws at 156 %.
    ///
    /// It survives on the context for [`Ctx::font_px`] alone — the vh-based
    /// size the plugin ABI offers a script, which no bake can reach.
    pub ui_font_scale: f32,
    /// Font scale of the panel being drawn (container-query style):
    /// narrow columns shrink their text. Panels set it on entry and
    /// reset it to 1.0 when done; full-width panels leave it at 1.0.
    pub panel_scale: f32,
    /// The focus chain of the world being drawn — how a control asks
    /// "am I focused?" and joins the Tab order ([`crate::focus`]).
    /// Per-world like `SizeTable`, owned by the application. None while
    /// a caller draws without one (tests, an embedder with no keyboard)
    /// — every control treats that as "never focused".
    pub focus: Option<&'a mut FocusCtl>,
    /// Where a control files "the pointer is resting on me and there is
    /// more to say than what I drew" ([`crate::object::tooltip`]).
    /// Owned by the application, which draws the manager LAST — the
    /// tooltip covers whatever it explains, so nothing may be drawn over
    /// it. None while a caller draws without one: a request is then
    /// simply not made, which is what a headless test and a plugin's
    /// own surface both want.
    pub tips: Option<&'a mut crate::object::tooltip::Tooltips>,
    /// Where a control reports its accessible role, name and state
    /// (crate::access) for a future AT-SPI bridge to read — structural/
    /// passive containers only; a FOCUSABLE control's AccessInfo travels
    /// through crate::focus::FocusCtl::register instead, because
    /// FocusCtl::nav() must never turn a passive node into a Tab stop.
    /// None while a caller draws without one, same convention as `tips`
    /// and `focus`.
    pub access: Option<&'a mut crate::access::AccessCtl>,
}

impl<'a> Ctx<'a> {
    pub fn vh(&self, v: f32) -> f32 {
        self.h / 100.0 * v
    }
    pub fn vw(&self, v: f32) -> f32 {
        self.w / 100.0 * v
    }
    /// Interface font size: scaled by UIFontSize= (text only) and by the
    /// width of the panel being drawn, min 8 px.
    ///
    /// The one place `ui_font_scale` is still a factor by hand, and the
    /// reason it is: this size is a fraction of the WINDOW, not a multiple
    /// of u, so it rides past the bake that carries the user's scale to
    /// everything else. Anything reading a token instead must leave it out.
    pub fn font_px(&self, v: f32) -> f32 {
        (self.vh(v) * UI_FONT_BASE * self.ui_font_scale * self.panel_scale).max(8.0)
    }
    /// Panel-relative font scale (like a CSS container query): 100% when
    /// the panel matches its reference box (a classic side-column width =
    /// 30% of the window height, and the panel's default height).
    /// Enlarging the panel scales the whole widget UP proportionally
    /// (the smaller of the two axes wins, so proportions are kept);
    /// narrow columns still shrink down to 62%.
    /// Both axes, smaller wins: a widget keeps its proportions whichever
    /// way its panel is stretched. Width alone would blow the on-screen
    /// keyboard up to two and a half times its size the moment it got a
    /// wide panel.
    pub fn panel_font_scale(&self, r: &Rect, p: Panel) -> f32 {
        self.panel_font_scale_in(r, p, &size_table())
    }

    /// The same rescale against a CALLER's size table — the per-world
    /// form (u3 L2); the method above is its process-wide shorthand.
    pub fn panel_font_scale_in(&self, r: &Rect, p: Panel, t: &SizeTable) -> f32 {
        let ws = r.w / (self.h * 0.30);
        let hs = r.h / (self.h * t.ref_h_vh(p) / 100.0);
        ws.min(hs).clamp(0.62, 3.0)
    }
}

/// Trims text (with a trailing ellipsis) so it fits the given width —
/// shared by the telemetry widgets.
pub fn fit_end(ctx: &mut Ctx, px: f32, text: &str, max_w: f32) -> String {
    if ctx.fonts.measure(FONT_UI, px, text, px * 0.06) <= max_w {
        return text.to_string();
    }
    // `type.ellipsis`, read once the run is known not to fit. The
    // character was written out here, and in three more trimmers beside
    // it, while the master declared the key and its comment named these
    // very call sites.
    let cut = crate::ui::ellipsis();
    let chars: Vec<char> = text.chars().collect();
    let mut n = chars.len().saturating_sub(1);
    while n > 1 {
        let cand: String = chars[..n].iter().collect::<String>() + cut.as_ref();
        if ctx.fonts.measure(FONT_UI, px, &cand, px * 0.06) <= max_w {
            return cand;
        }
        n -= 1;
    }
    cut.to_string()
}
