//! Theming.
//!
//! One engine: a `.theme` file format, a cascade over a master
//! `default.theme`, and a resolved struct with no strings and no per-frame
//! lookups. The seven-field eDEX-shaped `Theme` the program was built on is
//! gone; `theme::Color` — the four-`f32` colour every draw call takes — is
//! [`color::Color`], exactly as [`color`]'s docs promised when the two
//! engines still shared this module.
//!
//! # `default.theme` is the schema
//!
//! §2.1 of the specification asks for a **generated `tokens.rs`** holding enums
//! for ~2 190 tokens, and §7 for a `ResolvedTheme` with one named field per
//! token. That table would have to be kept byte-identical with `default.theme`
//! by hand, forever, against the owner's own requirement that `default.theme`
//! carry absolutely every setting. It does not exist here. Instead:
//!
//! * `default.theme` is embedded with `include_str!` and parsed at startup.
//!   **The set of tokens that exist is exactly the set of keys it declares.**
//! * A token's *type* is inferred from the form of its default value.
//! * Its [`TokenId`] is its index in `default.theme`'s declaration order,
//!   interned into a name -> id map at load.
//! * A key in a user theme that `default.theme` does not declare is an unknown
//!   key and warns, exactly as §4.2 requires — the check falls out of the
//!   design instead of needing a second table to fall out of sync with it.
//!
//! [`ResolvedTheme`] is therefore four parallel arrays indexed by `TokenId`
//! (`colors`, `scalars`, `flags`, `enums`) with no strings, no `Vec`, no
//! `HashMap` and no allocation on any draw path; the strings live in
//! [`ThemeDiagnostics`], published as a separate `Arc` beside it. Hot draw paths
//! hold their ids in a `static OnceLock<TokenId>` resolved once by name at load
//! (see [`ids`]), so a per-frame read is one bounds-checked slice index — the
//! same cost as a struct field, and with none of the maintenance. Every promise
//! §7.2 makes about the per-frame budget is kept: no hashing, no strings, no
//! allocation while drawing.
//!
//! # Deliberately not in this stage
//!
//! Each has a comment where it belongs, in the module that will call it:
//!
//! | module | why later | noted in |
//! |---|---|---|
//! | `encode.rs` | keyed on the live swapchain format (§6.3) | [`bake`] |
//! | `enforce.rs` | must run *after* encode, on the pixels the GPU blends (§2.2, §4.4) | [`resolve`] |
//! | `abi.rs` | `ThemeC` + the 19 appended `HostApi` entries (§7.4) | here |
//! | `mask.rs` | procedural R8 masks in the glyph atlas (M0, Appendix B) | here |
//!
//! Everything the enforcement passes need already exists — [`color::Color`]'s
//! `wcag_contrast`, `apca_lc`, `delta_e_ok` and `composite_as_rendered`, and
//! §6's `ensure()` — so `enforce.rs` is a pass over baked values and nothing
//! here has to move for it. The engine is complete and useful without all five.

pub mod bake;
pub mod cascade;
pub mod color;
pub mod edit;
pub mod expr;
pub mod mood;
pub mod parse;
pub mod plate;
pub mod resolve;

pub use bake::{BakeInput, ResolvedTheme, Viewport};
pub use mood::{MoodInput, MoodRule, MoodWhen};
pub use plate::Plate;
pub use cascade::{Schema, ThemeSpec, TokenId};
pub use color::Color;
pub use color::Color as ThemeColor;
pub use expr::{Expr, Kind, Value};
pub use parse::{Diagnostic, Level, Span};

/// THE RAW LOOK — what the program is when no theme has answered.
///
/// "Without a theme it should look like HTML with no CSS": legible,
/// visibly undesigned, and never a guess at what an author meant. These
/// are the only colours in the engine that are not a token, and they live
/// in ONE place because they were four — a grey in `color.rs`, a bed in
/// `bake.rs`, a darker grey in `resolve.rs` and a stroke width inside
/// `StateStyle::RAW` — and four copies of "unstyled" are four different
/// unstyled programs, only one of which anybody ever looks at.
pub mod raw {
    use super::color::Color;

    /// What a colour that gets DRAWN answers with nothing behind it —
    /// text, lines, glyphs, edges. Mid grey: unstyled at a glance, still
    /// legible on either bed.
    pub const INK: Color = Color::GREY;

    /// What a colour that gets FILLED answers with nothing behind it —
    /// beds, backgrounds, the canvas. Near-black, the way an unstyled web
    /// page is white: one grey for everything made the themeless program
    /// an unreadable slab, and legible-but-undesigned needs exactly two
    /// achromatic values, not one.
    pub const BED: Color = Color { r: 0.05, g: 0.05, b: 0.05, a: 1.0 };

    /// The one hairline an unstyled control wears. A device pixel: this
    /// is the absence of a stroke token, not a stroke ladder in hiding.
    pub const EDGE_WIDTH: f32 = 1.0;
}

use cascade::ThemeSource;
use parse::{Document, LangTag, SectionKind, Sources};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

/// The master theme. It is the documentation as much as the configuration
/// (§1.1(8), §5.0b), and it is the schema (see the module docs).
const DEFAULT_THEME: &str = include_str!("default.theme");

/// The master's own text, for the tests that read it as a DOCUMENT rather
/// than through the engine.
///
/// A resolved theme cannot answer "how many keys does §5.22 declare": the
/// baker fills every declared token whether the file wrote it or not, and
/// a section's own arithmetic is a fact about the FILE. The one reader is
/// `motion::tests::the_catalogue_is_closed_and_counted`, which is what
/// keeps §5.22's header count and §5.22's body from parting again.
#[cfg(test)]
pub(crate) fn master_source() -> &'static str {
    DEFAULT_THEME
}

// --------------------------------------------------------------- diagnostics

/// Everything about a loaded theme that is a **string**, published as its own
/// `Arc` beside the POD [`ResolvedTheme`] (§7.1).
///
/// It is a separate type rather than a few extra fields because the POD
/// guarantee has to be a property of the type: a `Vec<String>` is not `Copy`,
/// memcpy-ing one produces two owners of one heap allocation, and a locale count
/// would make `size_of::<ResolvedTheme>()` depend on how many languages a theme
/// declares. Nothing on a draw path can reach this.
#[derive(Default)]
pub struct ThemeDiagnostics {
    pub name: Vec<(LangTag, String)>,
    pub description: Vec<(LangTag, String)>,
    pub author: String,
    pub family: String,
    pub schema: u32,
    /// Where the selected theme came from, for the settings panel.
    pub path: Option<PathBuf>,
    /// §4.2's report list, already rendered with `file:line:col` and a caret.
    pub warnings: Vec<String>,
    /// The cold-path text tokens — font families, file names, separators. Off
    /// every draw path by construction: they are not in `ResolvedTheme`.
    pub texts: Vec<(String, String)>,
    /// The moods and variants this theme resolved into, in selection order.
    /// Index 0 is always the plain theme.
    pub siblings: Vec<String>,
}

impl ThemeDiagnostics {
    /// The theme's name in the user's language, falling back to the untagged
    /// one and then to the file stem.
    pub fn localised_name(&self, lang: &str) -> &str {
        self.name
            .iter()
            .find(|(l, _)| l == lang)
            .or_else(|| self.name.iter().find(|(l, _)| l.is_empty()))
            .map(|(_, v)| v.as_str())
            .unwrap_or("default")
    }

    pub fn text(&self, token: &str) -> Option<&str> {
        self.texts.iter().find(|(k, _)| k == token).map(|(_, v)| v.as_str())
    }
}

// ------------------------------------------------------------------- engine

struct Sibling {
    label: String,
    mood: Option<String>,
    variant: Option<String>,
    spec: ThemeSpec,
    explicit_density: (bool, bool),
}

struct Engine {
    schema: Schema,
    sources: Sources,
    siblings: Vec<Sibling>,
    /// The declarative triggers of §5.24, parsed once at load, in the order
    /// the theme declares its moods. Cold: a host reads them when the theme
    /// changes and evaluates its own copy against its own telemetry.
    moods: Vec<MoodRule>,
    active: usize,
    viewport: Viewport,
    /// One leaked `ResolvedTheme` per (sibling, quantised `u`). Bounded by the
    /// handful of distinct unit sizes a session ever sees, which is what makes
    /// handing out `&'static` from [`resolved`] affordable: a resize storm
    /// re-uses a bake instead of leaking one per event.
    cache: HashMap<(usize, u32), &'static ResolvedTheme>,
    /// The same bakes the map above holds, reached WITHOUT the resolve that
    /// computes their key. `cache` is keyed on the `u` a resolve has to
    /// produce first, so every miss on [`set_viewport`]'s repeat check paid
    /// for resolving every token in the theme — and two monitors of unequal
    /// height alternate viewports every frame, which is never a repeat. What
    /// a viewport bakes to cannot change while the schema and the siblings
    /// stand, so the viewport itself is a sound key.
    by_viewport: HashMap<(usize, u32, u32), &'static ResolvedTheme>,
    /// Values the theme editor is trying out, laid over the active sibling.
    /// Empty whenever nobody is editing, which is every run of the program
    /// that never opens the editor.
    ///
    /// A preview is NOT a theme change: the file on disk still says what it
    /// said, and switching mood or variant must keep working underneath. So
    /// these sit beside the siblings rather than becoming one, and
    /// [`content_epoch`] does not move for them — the font slots are named by
    /// the file, and a colour being tried out cannot rename them.
    preview: Vec<(TokenId, expr::Expr)>,
    /// Moves when the preview SET changes; keys `preview_cache` so a set
    /// that has not changed re-uses its bake instead of leaking one per
    /// frame. Without this, `set_viewport`'s per-screen call re-baked the
    /// preview on every frame of every screen — the morning's 100 % CPU
    /// fault, reintroduced through a different door, plus ~9 MB/s of
    /// leaked bakes on a two-monitor desktop.
    preview_rev: u32,
    preview_cache: HashMap<(usize, u32, u32, u32), &'static ResolvedTheme>,
    diagnostics: Arc<ThemeDiagnostics>,
}

static ENGINE: OnceLock<Mutex<Engine>> = OnceLock::new();
static ACTIVE: AtomicPtr<ResolvedTheme> = AtomicPtr::new(std::ptr::null_mut());
static EPOCH: AtomicU32 = AtomicU32::new(0);
static RESOLVES: AtomicU32 = AtomicU32::new(0);
/// Bumped when the theme's CONTENT changes — a load, a mood, a variant — and
/// never when the viewport does. See [`content_epoch`].
static CONTENT_EPOCH: AtomicU32 = AtomicU32::new(0);
/// The live [`Viewport`] as ONE word — `screen_h`'s bits in the high half,
/// `ui_scale`'s in the low — so a hot path can ask which viewport is
/// standing without taking the engine's lock. See [`viewport_key`].
static VIEWPORT_KEY: AtomicU64 = AtomicU64::new(0);
static DIAGS: OnceLock<Mutex<Arc<ThemeDiagnostics>>> = OnceLock::new();

fn publish_viewport(v: Viewport) {
    VIEWPORT_KEY.store(
        ((v.screen_h.to_bits() as u64) << 32) | v.ui_scale.to_bits() as u64,
        Ordering::Release,
    );
}

/// Which viewport the engine is baking for, as an opaque word: equal
/// between two frames of one screen, different between two screens of
/// different height or interface scale.
///
/// The cheap half of [`set_viewport`]'s question. A caller that needs to
/// tell one SCREEN from another cannot ask [`epoch`] (it names the bake,
/// and on a mixed-height desktop it alternates every frame by design) and
/// must not ask the engine (a mutex per drawn control is a mutex per
/// control per frame). This is a relaxed-cost atomic load of the numbers
/// the host itself passed in.
///
/// **It names a viewport, not a monitor.** Two identical screens share
/// one word, because they bake the same and the engine has no reason to
/// tell them apart. Anything that has to tell THEM apart needs the
/// host's own word for it — `crate::motion::set_surface` is the first
/// caller with that problem, and its header carries the reasoning.
///
/// The value is opaque on purpose: it is an identity to compare, never a
/// pair of numbers to compute with.
pub fn viewport_key() -> u64 {
    VIEWPORT_KEY.load(Ordering::Acquire)
}

fn diags_slot() -> &'static Mutex<Arc<ThemeDiagnostics>> {
    DIAGS.get_or_init(|| Mutex::new(Arc::new(ThemeDiagnostics::default())))
}

/// The theme this frame is drawn from (§2.3 tier 1: statically linked widgets
/// index Rust structs directly — no call, no copy, no marshalling).
///
/// The first call loads the theme. The reference is valid for the life of the
/// process; a widget may cache *values* across frames but should re-read after
/// [`epoch`] changes.
pub fn resolved() -> &'static ResolvedTheme {
    let p = ACTIVE.load(Ordering::Acquire);
    if !p.is_null() {
        // SAFETY: every value stored in ACTIVE is a `Box::leak`ed
        // `ResolvedTheme` that is never freed and never mutated after publish.
        return unsafe { &*p };
    }
    load();
    let p = ACTIVE.load(Ordering::Acquire);
    if p.is_null() {
        return empty_theme();
    }
    unsafe { &*p }
}

/// The last-resort theme: no tokens at all, so every accessor returns its kind's
/// fallback. Reached only if `default.theme` itself declares nothing.
fn empty_theme() -> &'static ResolvedTheme {
    static E: OnceLock<&'static ResolvedTheme> = OnceLock::new();
    E.get_or_init(|| {
        let mut out = Vec::new();
        let mut src = Sources::new();
        let f = src.add("<empty>", "");
        let doc = parse::parse(&mut src, f, None, &mut out);
        let schema = Schema::from_default(&doc, &mut out);
        let r = resolve::resolve_default(&schema, &mut out);
        Box::leak(Box::new(bake::bake(&schema, &r, &BakeInput::default(), &mut out)))
    })
}

/// Increments whenever the host swaps the resolved theme: reload, mood, variant,
/// resize, format change (§7.4).
pub fn epoch() -> u32 {
    EPOCH.load(Ordering::Acquire)
}

/// Moves when the theme's CONTENT changes: a load, a mood, a variant. It does
/// NOT move when the viewport does.
///
/// [`epoch`] answers a different question — WHICH BAKE is published — and on a
/// desktop whose monitors are unequal heights there are two live bakes, one
/// per unit size, published in turn as each screen draws. Its value therefore
/// alternates every frame, forever, by design.
///
/// That makes [`epoch`] a correct cache key and a ruinous change-detector. A
/// consumer holding ONE remembered epoch and asking "has anything changed?"
/// gets `true` on every frame of a mixed-height desktop. Anything asking
/// whether the theme has changed under it — which families the face slots
/// name, which colours a palette holds — wants this counter instead.
///
/// (Written after the font system, guarding its face reload with [`epoch`],
/// walked the font directories and re-parsed every face file sixty times a
/// second and put `--desktop` at 100 % CPU.)
pub fn content_epoch() -> u32 {
    CONTENT_EPOCH.load(Ordering::Acquire)
}

/// How many times the engine has resolved the theme in this process.
///
/// A diagnostic, and the only witness this one has. A resolve walks every
/// token in the file, so it belongs to a theme load, a mood, a variant or a
/// screen height the session has not seen before — never to a frame. Nothing
/// else the engine exposes can tell a re-resolve from a cache hit: the
/// published pointer and [`epoch`] read exactly the same either way, which is
/// how `--desktop` came to resolve the whole theme twice a frame on a
/// mixed-height desktop without a single observable saying so.
///
/// Rises monotonically. Compare two readings; the absolute value means
/// nothing.
pub fn resolves() -> u32 {
    RESOLVES.load(Ordering::Relaxed)
}

/// The strings that came with the loaded theme.
pub fn diagnostics() -> Arc<ThemeDiagnostics> {
    diags_slot().lock().map(|g| g.clone()).unwrap_or_default()
}

/// The token id for a name, or `None` if `default.theme` does not declare it.
///
/// **Call this at init and cache the id**, never inside a draw loop — that is
/// the whole reason [`TokenId`] exists. [`ids`] does exactly that for the hot
/// set.
/// The index of an interaction class — `class_id("button")`, `class_id
/// ("slider.knob")` — for [`ResolvedTheme::class_state`]. Resolved once at
/// init, exactly like a [`TokenId`]; the order is the master's declaration
/// order of its `class.*` tokens.
pub fn class_id(name: &str) -> Option<u16> {
    let engine = ENGINE.get()?.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let want = format!("class.{name}");
    let mut i = 0u16;
    for n in engine.schema.names() {
        if n == want {
            return Some(i);
        }
        if n.starts_with("class.") {
            i += 1;
        }
    }
    None
}

pub fn id(name: &str) -> Option<TokenId> {
    // There is no schema until a theme has been loaded, and `resolved` is
    // what loads it. Almost every caller memoises this answer in a
    // `'static OnceLock` — `ui::tok`, `plate`, `term_ansi`, `data_series`
    // and a dozen copies of `tok` across the objects — so an id asked one
    // moment too early does not merely fail once: it pins "no such token"
    // for the life of the process, and the token silently keeps whatever
    // the consumer falls back to. Nothing in the load path reaches this
    // function, so forcing the load here cannot re-enter.
    if ENGINE.get().is_none() {
        let _ = resolved();
    }
    let e = ENGINE.get()?;
    let g = e.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    g.schema.id(name)
}

/// A baked corner RADIUS as a length on the box it is about to cut.
///
/// Radius tokens carry the LENGTH alone — how the corner is cut is the
/// `*_corner_style` / `*_corner_mode` sibling's word — but one of the
/// lengths a theme may write is `pill`, and `pill` has no value until
/// there is a box: it means "as round as this one can be", which is half
/// the short side, the largest radius the ring generator honours. It
/// bakes to §5.0's negative sentinel, so every consumer testing
/// `radius > 0.0` has silently drawn a rectangle instead.
///
/// Any other sentinel is the ABSENCE of a length rather than a length,
/// and absence answers zero: nothing here invents a radius the theme did
/// not ask for.
pub fn corner_radius(radius: f32, w: f32, h: f32) -> f32 {
    match expr::sentinel("pill") {
        Some(pill) if radius == pill => (w.min(h) * 0.5).max(0.0),
        _ if radius > 0.0 => radius,
        _ => 0.0,
    }
}

/// A role's resolved px: `raw` under the role's own ceiling and floor,
/// with `type.min_px` beneath a role whose theme states no floor of its
/// own.
///
/// Here for the same reason [`corner_radius`] is: the library has TWO
/// role resolvers — [`crate::ui::Role::px`] for objects drawing against
/// `Ctx`, [`crate::view::paint::role_look`] for every view, script table
/// and ABI widget drawing against `Surface` — and a rule written twice
/// is a rule that stops matching itself somewhere. It already had: one
/// side read a stated `min_px` of zero as "no floor" and the other as
/// "unstated, take the global", so one theme sized the same role two
/// ways in two halves of one screen.
///
/// Zero is "unstated" in both slots, which is how the master spells an
/// absent bound: `0px` on the ceiling is uncapped, and a floor of zero
/// is no floor a theme wrote. Neither is a length this file invents —
/// the caller passes what the theme holds, MISSING included, which reads
/// as zero.
///
/// The floor is applied AFTER the ceiling: a theme that caps a role
/// below the readable floor has contradicted itself, and `type.min_px`
/// is the master's last defence against unreadable type, so it wins.
pub fn role_px(raw: f32, own_floor: f32, global_floor: f32, ceiling: f32) -> f32 {
    let capped = if ceiling > 0.0 { raw.min(ceiling) } else { raw };
    capped.max(if own_floor > 0.0 { own_floor } else { global_floor })
}

/// The word behind an enum token's baked index, for diagnostics and for a
/// caller that wants to compare by name once at init.
pub fn enum_index(token: TokenId, word: &str) -> Option<u16> {
    let e = ENGINE.get()?;
    let g = e.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    g.schema.enum_index(token, word)
}

/// The NAME a token id stands for — [`id`] read backwards.
///
/// The one caller is the plugin boundary: a TEXT token is stored by name
/// in [`diagnostics`] rather than in the baked table, so answering
/// `theme_text` for an id means finding the name that id was interned
/// under. Nothing on the host needs it — every host reader already holds
/// the name it asked with.
pub fn name_of(token: TokenId) -> Option<String> {
    let e = ENGINE.get()?;
    let g = e.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let name = g.schema.name(token);
    (!name.is_empty()).then(|| name.to_string())
}

/// The word an enum token currently resolves to. This is how OPEN word sets
/// are read — a type-role binding (`script.rows_label_role = caption`) names a
/// role, not a member of a closed enum, so the consumer wants the word itself
/// rather than an index to compare. The resolved index is taken before the
/// engine lock: [`resolved`] may itself load on first use.
pub fn enum_word_of(token: TokenId) -> Option<String> {
    let i = resolved().enum_of(token);
    let e = ENGINE.get()?;
    let g = e.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    g.schema.enum_word(token, i).map(|s| s.to_string())
}

impl ResolvedTheme {
    /// The cold path: resolve a token by name. `Some` for every key
    /// `default.theme` declares. Call at widget init, cache the id, invalidate
    /// on [`epoch`].
    pub fn id(&self, name: &str) -> Option<TokenId> {
        crate::theme::id(name)
    }
}

// --------------------------------------------------------------------- load

/// Which theme to load, and where from.
#[derive(Clone, Debug, Default)]
pub struct LoadRequest {
    /// A theme name (`aurora`), looked up on the search path. `None` uses
    /// `NACELLE_THEME_NAME`, then `default`.
    pub name: Option<String>,
    /// A path to a `.theme` file, which wins over `name`.
    pub path: Option<PathBuf>,
    /// `None` keeps the viewport the running engine already learned from
    /// [`set_viewport`] — the right choice for every reload whose window
    /// did not change, which is all of them but the host's resize path.
    /// `Viewport::default()` is a real 1080-line request, not a sentinel:
    /// passing it here re-bakes every u-derived length at reference size.
    pub viewport: Option<Viewport>,
}

/// Parse, cascade, resolve and bake. **Always succeeds** (§4.2): a missing or
/// broken theme degrades to `default`, and a broken `default` degrades to the
/// per-kind fallback of `resolve::fallback`.
pub fn load() -> Arc<ThemeDiagnostics> {
    load_with(LoadRequest::default())
}

/// The master with `extra` appended, baked and handed back — WITHOUT
/// touching the published theme.
///
/// The one way a test drives a drawing routine from a theme other than the
/// process's own. The alternatives are both worse: `set_preview` publishes,
/// so a test using it would decide what every other test running beside it
/// draws from; and a hand-built `ResolvedTheme` would be a second answer to
/// "what does this file mean", which is the drift these modules are about.
///
/// Appending is an override because §4.1 gives the LAST declaration in a
/// stage the token — and because it declares no new names, the ids of the
/// returned theme are the ids of [`id`], so a `Level` built from the
/// process's schema reads this theme correctly.
#[cfg(test)]
pub(crate) fn bake_over_master(extra: &str) -> bake::ResolvedTheme {
    let mut out = Vec::new();
    let mut src = Sources::new();
    let f = src.add("default.theme", format!("{DEFAULT_THEME}\n{extra}\n"));
    let doc = parse::parse(&mut src, f, None, &mut out);
    let mut schema = Schema::from_default(&doc, &mut out);
    let d = resolve::resolve_default(&schema, &mut out);
    schema.adopt_kinds(&d.values);
    let r = resolve::resolve(&schema, &schema.base_spec(), &mut out);
    bake::bake(&schema, &r, &BakeInput::default(), &mut out)
}

pub fn load_with(req: LoadRequest) -> Arc<ThemeDiagnostics> {
    let mut out: Vec<Diagnostic> = Vec::new();
    let mut src = Sources::new();

    // ---- stage 2: default.theme, dense --------------------------------
    // NACELLE_THEME_MASTER substitutes the embedded master with a file — the
    // governing principle's own test facility: run with a [meta]-only master
    // and the program must come up RAW (grey ink, kind defaults) rather than
    // in anybody's design. A theme file is data, so this opens no door a
    // theme could not already walk through.
    let master_text: String = std::env::var_os("NACELLE_THEME_MASTER")
        .and_then(|p| std::fs::read_to_string(p).ok())
        .unwrap_or_else(|| DEFAULT_THEME.to_string());
    let f = src.add("default.theme", master_text);
    let default_doc = parse::parse(&mut src, f, None, &mut out);
    let mut schema = Schema::from_default(&default_doc, &mut out);
    {
        // Settle the kinds a reference cannot declare syntactically, by
        // resolving `default` once. §6.3 asserts this walk is cycle-free.
        let r = resolve::resolve_default(&schema, &mut out);
        schema.adopt_kinds(&r.values);
    }

    // ---- stage 3: the selected theme, and its [meta] base chain -------
    let mut fs = FsThemes::new();
    let chosen = req
        .path
        .clone()
        .or_else(|| std::env::var_os("NACELLE_THEME_PATH").map(PathBuf::from));
    let name = req
        .name
        .clone()
        .or_else(|| std::env::var("NACELLE_THEME_NAME").ok())
        .unwrap_or_else(|| "default".into());

    let theme_doc: Option<Document> = match &chosen {
        Some(p) => {
            let d = parse::parse_file(&mut src, p, &mut out);
            if d.is_none() {
                out.push(Diagnostic::warn(
                    Span::default(),
                    format!("theme file {} could not be read — using default", p.display()),
                ));
            }
            d
        }
        None if name != "default" => {
            let d = fs.open(&name, &mut src, &mut out);
            if d.is_none() {
                out.push(Diagnostic::warn(
                    Span::default(),
                    format!("theme \"{name}\" was not found on the search path — using default"),
                ));
            }
            d
        }
        None => None,
    };

    let chain: Vec<Document> = match &theme_doc {
        Some(d) => cascade::base_chain(d, &mut fs, &mut src, &mut out),
        None => Vec::new(),
    };

    // ---- stage 5: the user overlay ------------------------------------
    let user_doc = user_overlay_path().and_then(|p| parse::parse_file(&mut src, &p, &mut out));

    let strict = theme_doc.as_ref().and_then(|d| d.meta_bool("meta.strict")).unwrap_or(false);
    let opts = cascade::Options { strict };

    // ---- the plain sibling, and one per mood / variant ----------------
    let mut stages: Vec<cascade::Stage> = Vec::new();
    for c in &chain {
        stages.push(cascade::Stage::Document(c));
    }
    if let Some(d) = &theme_doc {
        stages.push(cascade::Stage::Document(d));
    }
    if let Some(d) = &user_doc {
        stages.push(cascade::Stage::Document(d));
    }

    let mut siblings: Vec<Sibling> = Vec::new();
    let plain = cascade::cascade(&mut schema, &stages, opts, &mut out);
    let explicit = explicit_density(&schema, &plain);
    siblings.push(Sibling {
        label: "plain".into(),
        mood: None,
        variant: None,
        spec: plain,
        explicit_density: explicit,
    });

    // WHOSE moods and variants these are, PER KIND. The combos below were
    // built from the SELECTED theme alone, and not one theme in the
    // catalogue declares a `[mood.*]` or a `[variant.*]` — they live in the
    // master. So every session resolved exactly one sibling and every
    // `set_mood` answered false: the alarm skin was written, documented,
    // baked and unreachable.
    //
    // The master's moods are the right moods for a theme that declares none,
    // because a mood is a sparse RE-MAP of roles the active theme resolves:
    // `[mood.alert]`'s `alpha(@severity.critical.text, 0.18)` is whichever
    // red the running theme chose, which is the same mechanism that gives
    // one layout four hues. Per kind, because a theme that ships a mood of
    // its own must not thereby lose the high-contrast variant it never
    // mentioned — that would make an accessibility setting disappear as a
    // side effect of an alarm colour. Within a kind there is no merge: a
    // theme that declares moods declares all of them.
    let declares = |d: &Document, k: SectionKind| !d.overlays(k).is_empty();
    let mood_doc: &Document = match &theme_doc {
        Some(d) if declares(d, SectionKind::Mood) => d,
        _ => &default_doc,
    };
    let variant_doc: &Document = match &theme_doc {
        Some(d) if declares(d, SectionKind::Variant) => d,
        _ => &default_doc,
    };
    let mood_rules = cascade::mood_rules(mood_doc, &mut out);
    {
        let declared = cascade::sibling_names(mood_doc, &mut out);
        let moods: Vec<String> = declared
            .iter()
            .filter(|(k, _)| *k == SectionKind::Mood)
            .map(|(_, n)| n.clone())
            .collect();
        // One call where one document declares both, so the "more than
        // eight" report is not printed twice for the same file.
        let variants: Vec<String> = if std::ptr::eq(mood_doc, variant_doc) {
            declared
                .iter()
                .filter(|(k, _)| *k == SectionKind::Variant)
                .map(|(_, n)| n.clone())
                .collect()
        } else {
            cascade::sibling_names(variant_doc, &mut out)
                .into_iter()
                .filter(|(k, _)| *k == SectionKind::Variant)
                .map(|(_, n)| n)
                .collect()
        };
        // §4.1: `[mood.<m>]` applies BEFORE `[variant.hc]`, so high contrast
        // always wins over an alarm's decoration.
        let mut combos: Vec<(Option<String>, Option<String>)> = Vec::new();
        for m in &moods {
            combos.push((Some(m.clone()), None));
        }
        for v in &variants {
            combos.push((None, Some(v.clone())));
        }
        for m in &moods {
            for v in &variants {
                combos.push((Some(m.clone()), Some(v.clone())));
            }
        }
        for (m, v) in combos {
            if siblings.len() >= cascade::MAX_SIBLINGS {
                out.push(Diagnostic::warn(
                    Span::default(),
                    format!(
                        "more than {} resolved siblings; the rest are dropped (not a load failure)",
                        cascade::MAX_SIBLINGS
                    ),
                ));
                break;
            }
            let mut st: Vec<cascade::Stage> = Vec::new();
            for c in &chain {
                st.push(cascade::Stage::Document(c));
            }
            // The master is never a stage of its own: its plain values ARE
            // the base spec. Only a SELECTED theme is a document here, and
            // it is pushed whether or not it is the one declaring the mood.
            if let Some(d) = &theme_doc {
                st.push(cascade::Stage::Document(d));
            }
            if let Some(n) = &m {
                st.push(cascade::Stage::Overlay {
                    doc: mood_doc,
                    kind: SectionKind::Mood,
                    name: n.clone(),
                });
            }
            if let Some(n) = &v {
                st.push(cascade::Stage::Overlay {
                    doc: variant_doc,
                    kind: SectionKind::Variant,
                    name: n.clone(),
                });
            }
            if let Some(d) = &user_doc {
                st.push(cascade::Stage::Document(d));
            }
            let spec = cascade::cascade(&mut schema, &st, opts, &mut out);
            let explicit = explicit_density(&schema, &spec);
            siblings.push(Sibling {
                label: label_of(&m, &v),
                mood: m,
                variant: v,
                spec,
                explicit_density: explicit,
            });
        }
    }

    // ---- diagnostics ---------------------------------------------------
    let mut meta = ThemeDiagnostics {
        siblings: siblings.iter().map(|s| s.label.clone()).collect(),
        path: chosen,
        ..Default::default()
    };
    collect_meta(&mut meta, &schema, &default_doc, theme_doc.as_ref());

    // ---- publish -------------------------------------------------------
    // A request that names no viewport keeps the one the running engine
    // already learned from [`set_viewport`]: a theme switch happens in a
    // window whose height did not change with it, and resetting to the
    // 1080-line default here is how every u-derived length used to snap
    // back to reference size on a settings click. `None` is the sentinel —
    // `Viewport::default()` is 1080.0, a value a numeric guard cannot tell
    // apart from an explicit request, which is exactly how the snap-back
    // came back for every window that was not 1080 lines tall.
    let viewport = req.viewport.unwrap_or_else(|| {
        ENGINE
            .get()
            .and_then(|slot| slot.lock().ok().map(|g| g.viewport))
            .unwrap_or_default()
    });
    // The word the hot paths read follows the engine's own field, here as
    // well as in [`set_viewport`] — a load that keeps the running
    // viewport must not leave the published word naming the last one.
    publish_viewport(viewport);
    let mut engine = Engine {
        schema,
        sources: src,
        siblings,
        moods: mood_rules,
        active: 0,
        viewport,
        cache: HashMap::new(),
        by_viewport: HashMap::new(),
        preview: Vec::new(),
        preview_rev: 0,
        preview_cache: HashMap::new(),
        diagnostics: Arc::new(ThemeDiagnostics::default()),
    };

    let theme = engine.bake_active(&mut out);
    for d in &out {
        meta.warnings.push(d.render(&engine.sources));
    }
    collect_texts(&mut meta, &engine);
    report(&meta);

    let diags = Arc::new(meta);
    engine.diagnostics = diags.clone();
    publish(theme);
    // A load replaces the file the slots are named in, so everything derived
    // from the theme's CONTENT — the face slots above all — is now stale.
    CONTENT_EPOCH.fetch_add(1, Ordering::Release);
    *diags_slot().lock().unwrap() = diags.clone();

    match ENGINE.get() {
        Some(slot) => {
            if let Ok(mut g) = slot.lock() {
                *g = engine;
            }
        }
        None => {
            let _ = ENGINE.set(Mutex::new(engine));
        }
    }
    diags
}

fn label_of(m: &Option<String>, v: &Option<String>) -> String {
    match (m, v) {
        (Some(m), Some(v)) => format!("{m}+{v}"),
        (Some(m), None) => m.clone(),
        (None, Some(v)) => v.clone(),
        (None, None) => "plain".into(),
    }
}

/// §5.3's precedence rule: an *explicit* `density_space` / `density_type` — one
/// appearing in any stage after `default` — replaces the enum-supplied value for
/// that axis only. Detectable because a stage that set it replaced the whole
/// node, so the spec's expression is no longer `default`'s.
fn explicit_density(schema: &Schema, spec: &ThemeSpec) -> (bool, bool) {
    let differs = |name: &str| {
        schema
            .id(name)
            .map(|id| spec.get(id) != schema.default_expr(id))
            .unwrap_or(false)
    };
    (differs("metric.density_space"), differs("metric.density_type"))
}

impl Engine {
    fn bake_active(&mut self, out: &mut Vec<Diagnostic>) -> &'static ResolvedTheme {
        let i = self.active.min(self.siblings.len().saturating_sub(1));
        // Asked BEFORE the resolve, never after. The `u` that keys `cache`
        // is what a resolve produces, so reaching that cache costs a full
        // resolve of the whole theme — and `set_viewport` only drops a
        // REPEAT of the last viewport, which two alternating monitor
        // heights never are. Without this the program resolved every token
        // twice a frame, for as long as it ran, on any desktop whose
        // screens are not the same height.
        // A preview bypasses the two STANDING caches — reading either would
        // answer with the file's colours while the editor shows other ones —
        // and keeps its own, keyed additionally by `preview_rev`. The
        // revision is what makes keeping safe: every change to the set bumps
        // it and drains the map, so a bake can never outlive the values it
        // was made of. Without this memo a STANDING preview was re-baked by
        // `set_viewport` on every frame of every screen — the morning's
        // 100 % CPU fault through another door, plus ~9 MB/s of leaked
        // bakes on a two-monitor desktop, plus a plate re-bake per frame.
        //
        // Verified limit, recorded not fixed: with THREE or more screens the
        // frozen per-bake epochs can collide so that `poll_plates` misses
        // one re-bake of the decoration after a preview pulse. Unreachable
        // with two screens; the same class exists on the ordinary path at
        // `set_sibling`.
        if !self.preview.is_empty() {
            let pk = (
                i,
                self.viewport.screen_h.to_bits(),
                self.viewport.ui_scale.to_bits(),
                self.preview_rev,
            );
            if let Some(&t) = self.preview_cache.get(&pk) {
                return t;
            }
            let (mut spec, explicit_density) = {
                let s = &self.siblings[i];
                (s.spec.clone(), s.explicit_density)
            };
            for (id, e) in &self.preview {
                if let Some(slot) = spec.exprs.get_mut(id.index()) {
                    *slot = e.clone();
                }
            }
            let r = resolve::resolve(&self.schema, &spec, out);
            let input = BakeInput {
                viewport: self.viewport,
                epoch: EPOCH.load(Ordering::Acquire).wrapping_add(1),
                explicit_density,
            };
            RESOLVES.fetch_add(1, Ordering::Relaxed);
            let baked: &'static ResolvedTheme =
                Box::leak(Box::new(bake::bake(&self.schema, &r, &input, out)));
            self.preview_cache.insert(pk, baked);
            return baked;
        }
        let vk = (
            i,
            self.viewport.screen_h.to_bits(),
            self.viewport.ui_scale.to_bits(),
        );
        if let Some(&t) = self.by_viewport.get(&vk) {
            return t;
        }
        RESOLVES.fetch_add(1, Ordering::Relaxed);
        let (r, explicit_density) = {
            let s = &self.siblings[i];
            (
                resolve::resolve(&self.schema, &s.spec, out),
                s.explicit_density,
            )
        };
        let input = BakeInput {
            viewport: self.viewport,
            epoch: EPOCH.load(Ordering::Acquire).wrapping_add(1),
            explicit_density,
        };
        let probe = bake::metrics(&self.schema, &r, &input, &mut Vec::new());
        let key = (i, probe.u.to_bits());
        if let Some(&t) = self.cache.get(&key) {
            self.by_viewport.insert(vk, t);
            return t;
        }
        let baked: &'static ResolvedTheme = Box::leak(Box::new(bake::bake(
            &self.schema,
            &r,
            &input,
            out,
        )));
        self.cache.insert(key, baked);
        self.by_viewport.insert(vk, baked);
        baked
    }
}

fn publish(t: &'static ResolvedTheme) {
    ACTIVE.store(t as *const _ as *mut ResolvedTheme, Ordering::Release);
    EPOCH.store(t.epoch, Ordering::Release);
}

// ------------------------------------------------- viewport, mood, variant

/// Re-bake for a new window height or ui scale. **Runs on resize, never per
/// frame** (§2.2 step 4). A height that produces the same `u` re-uses the
/// existing bake and does not bump the epoch.
pub fn set_viewport(screen_h: f32, ui_scale: f32) {
    let Some(slot) = ENGINE.get() else { return };
    let Ok(mut g) = slot.lock() else { return };
    let next = Viewport { screen_h, ui_scale };
    if (g.viewport.screen_h - next.screen_h).abs() < f32::EPSILON
        && (g.viewport.ui_scale - next.ui_scale).abs() < f32::EPSILON
    {
        return;
    }
    g.viewport = next;
    publish_viewport(next);
    let mut out = Vec::new();
    let t = g.bake_active(&mut out);
    let cur = ACTIVE.load(Ordering::Acquire);
    if cur != t as *const _ as *mut ResolvedTheme {
        publish(t);
    }
}

/// Lays a set of values over the theme, so the editor can be SEEN rather than
/// described. Returns what it refused, empty when it took everything.
///
/// Each entry is a token name and the text a person just produced — the same
/// text that would be written into the file — read by the file's own parser
/// (`parse::parse_value`), so nothing can be previewed that could not be
/// saved. An unknown token or a value the parser rejects is refused by name
/// and the rest still applies.
///
/// # Not a theme change
///
/// [`content_epoch`] does not move. A preview cannot rename a font slot: the
/// face names come from the file, and the file has not changed. Moving it
/// would wake the whole face reload — a walk of the font directories and an
/// atlas reset — behind every slider.
///
/// # WHY THE CALLER STILL PACES ITSELF
///
/// A bake is 76 031 bytes, measured, and nothing ever frees one: they are
/// handed out as `&'static` so that reading the theme costs one atomic load
/// with no lifetime to thread through a draw call. The revision memo means a
/// STANDING set costs nothing per frame — but every CHANGED set is a fresh
/// bake per screen, permanently. A caller applying every motion event at
/// sixty a second would leak 4.5 MB/s; the desktop's editor pulses at ten,
/// which is ~0.8 MB for a second of active dragging and nothing once the
/// hand stops.
///
/// If a preview on every tick is ever wanted, the thing to change is not this
/// function: it is that bakes are leaked. Retiring them means the lock-free
/// `&'static` read path everything draws through has to grow a lifetime or a
/// refcount, which is a change to the engine's shape rather than an addition.
pub fn set_preview(values: &[(&str, &str)]) -> Vec<String> {
    let Some(slot) = ENGINE.get() else {
        return values.iter().map(|(n, _)| format!("{n}: no theme loaded")).collect();
    };
    let Ok(mut g) = slot.lock() else {
        return values.iter().map(|(n, _)| format!("{n}: the theme engine is poisoned")).collect();
    };
    let mut refused = Vec::new();
    let mut taken = Vec::with_capacity(values.len());
    for (name, text) in values {
        let Some(id) = g.schema.id(name) else {
            refused.push(format!("{name}: no such token"));
            continue;
        };
        let mut out = Vec::new();
        let e = parse::parse_value(text, Span::default(), &mut out);
        // The parser reports trailing text as a warning rather than an error,
        // and for a file that is right — a person can see the line. Here the
        // text came from a control, so anything left over means the control
        // produced something it did not mean to, and it is refused.
        if let Some(d) = out.first() {
            refused.push(format!("{name}: {}", d.message));
            continue;
        }
        taken.push((id, e));
    }
    g.preview = taken;
    // A new set is a new revision; the old revision's bakes will never be
    // asked for again, so the map is drained rather than left to grow.
    g.preview_rev = g.preview_rev.wrapping_add(1);
    g.preview_cache.clear();
    let mut out = Vec::new();
    let t = g.bake_active(&mut out);
    publish(t);
    refused
}

/// Puts the theme back the way the file has it. What CANCEL is made of.
///
/// Cheap and total: the preview never touched the siblings, so there is
/// nothing to undo — the overrides are dropped and the next bake comes from
/// the cache the preview was bypassing.
pub fn clear_preview() {
    let Some(slot) = ENGINE.get() else { return };
    let Ok(mut g) = slot.lock() else { return };
    if g.preview.is_empty() {
        return;
    }
    g.preview.clear();
    g.preview_rev = g.preview_rev.wrapping_add(1);
    g.preview_cache.clear();
    let mut out = Vec::new();
    let t = g.bake_active(&mut out);
    publish(t);
}

/// Whether values are currently laid over the file's own.
pub fn previewing() -> bool {
    ENGINE
        .get()
        .and_then(|s| s.lock().ok())
        .map(|g| !g.preview.is_empty())
        .unwrap_or(false)
}

/// Every resolved sibling, in selection order. Index 0 is the plain theme.
pub fn siblings() -> Vec<String> {
    ENGINE
        .get()
        .and_then(|s| s.lock().ok())
        .map(|g| g.siblings.iter().map(|s| s.label.clone()).collect())
        .unwrap_or_default()
}

/// Select a sibling by index. Switching is one store: no recomputation, no
/// per-draw branch (§5.24). Returns `false` for an index that does not exist.
pub fn set_sibling(i: usize) -> bool {
    let Some(slot) = ENGINE.get() else { return false };
    let Ok(mut g) = slot.lock() else { return false };
    if i >= g.siblings.len() {
        return false;
    }
    if g.active == i {
        return true;
    }
    g.active = i;
    let mut out = Vec::new();
    let t = g.bake_active(&mut out);
    publish(t);
    // A sibling is a different mood or variant, so its `[face]` block may
    // name other families than the one standing — a content change, unlike
    // the viewport swaps that pass through `set_viewport`.
    CONTENT_EPOCH.fetch_add(1, Ordering::Release);
    true
}

/// The declarative triggers of §5.24, parsed at load, in the order the theme
/// declares its moods.
///
/// **Read on a theme change, not per tick** — it allocates, and the answer
/// only changes when a theme is loaded ([`epoch`] moves with it).
///
/// The list is deliberately not pre-sorted by precedence. §5.24 fixes one
/// ordering, `lockdown > alert > normal`, and that is the master's own
/// declaration order read backwards; a host that must choose between two
/// rules holding at once takes the LAST that holds, which gives the
/// specification's answer for the shipped moods and gives a theme with moods
/// of its own an ordering it can see in its own file.
pub fn mood_rules() -> Vec<MoodRule> {
    ENGINE
        .get()
        .and_then(|s| s.lock().ok())
        .map(|g| g.moods.clone())
        .unwrap_or_default()
}

/// §5.24's explicit API. `None` clears the mood, keeping the current variant.
/// A mood the theme does not declare is refused rather than guessed at.
pub fn set_mood(name: Option<&str>) -> bool {
    select(name, current_variant().as_deref())
}

/// The contrast variant, `high_contrast` being the one the engine ships.
pub fn set_variant(name: Option<&str>) -> bool {
    select(current_mood().as_deref(), name)
}

pub fn current_mood() -> Option<String> {
    ENGINE
        .get()
        .and_then(|s| s.lock().ok())
        .and_then(|g| g.siblings.get(g.active).and_then(|s| s.mood.clone()))
}

pub fn current_variant() -> Option<String> {
    ENGINE
        .get()
        .and_then(|s| s.lock().ok())
        .and_then(|g| g.siblings.get(g.active).and_then(|s| s.variant.clone()))
}

fn select(mood: Option<&str>, variant: Option<&str>) -> bool {
    let want = {
        let Some(slot) = ENGINE.get() else { return false };
        let Ok(g) = slot.lock() else { return false };
        g.siblings.iter().position(|s| {
            s.mood.as_deref() == mood && s.variant.as_deref() == variant
        })
    };
    match want {
        Some(i) => set_sibling(i),
        None => false,
    }
}

/// The mood's transition tint (§5.24): one full-screen quad animated from its
/// declared alpha to zero over `motion.mood_change.duration`, drawn last.
pub fn mood_wash() -> Option<color::Color> {
    let slot = ENGINE.get()?;
    let g = slot.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let s = g.siblings.get(g.active)?;
    let mut out = Vec::new();
    let r = resolve::resolve(&g.schema, &s.spec, &mut out);
    r.wash.map(|c| c.to_srgb())
}

// ------------------------------------------------------------ the search path

/// The folder the whole nacelle family keeps its data in, and the one
/// this program alone used before them. Both are READ and only the
/// first is ever written to — the rule the desktop's own search path
/// already follows for configuration, sounds and layauts, said here
/// for themes as well.
const FAMILY_DIR: &str = "nacelle";
const LEGACY_FAMILY_DIR: &str = "nacelle-desktop";

struct FsThemes {
    dirs: Vec<PathBuf>,
}

impl FsThemes {
    fn new() -> FsThemes {
        FsThemes {
            dirs: theme_search_path(
                std::env::var_os("NACELLE_THEME_DIR").map(PathBuf::from),
                data_home(),
                home_dir().map(|h| h.join(".config")),
                data_dirs_var().as_deref(),
            ),
        }
    }
}

/// `$XDG_DATA_HOME`, or `~/.local/share` — the BASE, without the family
/// name, because the search path needs it to build both names.
fn data_home() -> Option<PathBuf> {
    match std::env::var_os("XDG_DATA_HOME") {
        Some(d) if !d.is_empty() => Some(PathBuf::from(d)),
        _ => home_dir().map(|h| h.join(".local/share")),
    }
}

/// `$XDG_DATA_DIRS` as written, or nothing — [`theme_search_path`] owns
/// the default, so that the default is a thing a test can read.
fn data_dirs_var() -> Option<String> {
    std::env::var("XDG_DATA_DIRS").ok().filter(|v| !v.is_empty())
}

/// The system data prefixes when `XDG_DATA_DIRS` says nothing, from the
/// XDG base directory specification and — the reason it matters here —
/// from `nacelle-themes/Makefile`, whose `PREFIX ?= /usr/local` under
/// `sudo make install` puts its files in the FIRST of them.
const SYSTEM_DATA_DIRS: &str = "/usr/local/share:/usr/share";

/// Every directory a theme file is looked for in, most specific first.
///
/// Split from [`FsThemes::new`] so the ORDER can be read — and tested —
/// without an environment: what belongs on the path is decided here,
/// and `new` only says where the bases come from.
///
/// The family folder comes FIRST at every level and the program's own
/// old name directly behind it. That pairing is not this function's
/// invention; it is the contract `nacelle-themes/Makefile` states in
/// its own head, about this program: "An older release installed into
/// nacelle-desktop/ instead. Those files are NOT moved or removed by
/// this installer: the program searches both names, the new one first."
/// Themes were the one asset that did not — the list asked the old name
/// at all three levels and the family name at none — so a theme in
/// `<data>/nacelle/themes`, which `nacelle-themes/config/nacelle-desktop.ron`
/// documents as THE place a theme file lives (`<data>/themes/<name>.theme`,
/// with `<data>` spelled out there as `$XDG_DATA_HOME/nacelle` and every
/// `$XDG_DATA_DIRS/nacelle`), could not be found. The embedder said the
/// same thing out loud: `nacelle-desktop`'s `warn_once_about_legacy`
/// prints, on any machine whose data is still under the old name, that
/// "its place from now on is ~/.local/share/nacelle" — and a theme moved
/// there on that advice stopped being found. Nothing is copied and
/// nothing is deleted: both names are read, one is written.
///
/// Note what that installer does NOT do: it ships sounds and layauts
/// and no theme at all. `<data>/nacelle/themes` is a directory a person
/// fills — by hand or through the editor, which now saves there — and
/// the contract above is the whole reason it has to be searched, since
/// no `make install` will ever create it.
///
/// The system end expands `XDG_DATA_DIRS` rather than naming one
/// prefix, for the same reason [`crate::assets::AssetRoots::xdg`] does
/// three modules over: `sudo make install` defaults to `PREFIX=/usr/local`,
/// so a hard-coded `/usr/share` misses the directory the documented
/// install command writes to. One prefix on that list was the same
/// class of installer-versus-resolver drift as the old name at the
/// user's end, one rung further down.
///
/// The config end is deliberately old-name-only. A theme is DATA and
/// belongs under the data dirs; `~/.config/nacelle-desktop/themes` is
/// on the path because themes once landed there, and giving that
/// mistake a new-name twin would invite it back.
fn theme_search_path(
    explicit: Option<PathBuf>,
    data_home: Option<PathBuf>,
    config_home: Option<PathBuf>,
    data_dirs: Option<&str>,
) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let mut push = |dir: PathBuf| {
        if !dirs.contains(&dir) {
            dirs.push(dir);
        }
    };
    if let Some(d) = explicit {
        push(d);
    }
    if let Some(data) = &data_home {
        push(data.join(FAMILY_DIR).join("themes"));
        push(data.join(LEGACY_FAMILY_DIR).join("themes"));
    }
    if let Some(config) = &config_home {
        push(config.join(LEGACY_FAMILY_DIR).join("themes"));
    }
    let system = data_dirs.filter(|v| !v.is_empty()).unwrap_or(SYSTEM_DATA_DIRS);
    for base in system.split(':').filter(|b| !b.is_empty()) {
        push(PathBuf::from(base).join(FAMILY_DIR).join("themes"));
        push(PathBuf::from(base).join(LEGACY_FAMILY_DIR).join("themes"));
    }
    dirs
}

impl cascade::ThemeSource for FsThemes {
    fn open(
        &mut self,
        name: &str,
        src: &mut Sources,
        out: &mut Vec<Diagnostic>,
    ) -> Option<Document> {
        // A theme name is a bare identifier, never a path: a `[meta] base` that
        // could name `../../etc/passwd` would be a file-read primitive.
        if name.is_empty() || !name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-') {
            out.push(Diagnostic::warn(
                Span::default(),
                format!("\"{name}\" is not a theme name (letters, digits, _ and - only)"),
            ));
            return None;
        }
        for d in &self.dirs {
            let p = d.join(format!("{name}.theme"));
            if p.is_file() {
                return parse::parse_file(src, &p, out);
            }
        }
        // Nothing else. The master is the ONE look compiled in; every other
        // theme is a file the person made — through the editor or by hand —
        // and lives on the search path. The eight shipped variants left on
        // 2026-08-16 at the owner's decision: a clean slate where `default`
        // is the only built-in and the editor is how themes come to be.
        None
    }
}

/// Every theme this program can load, by name: `default` — the embedded
/// master, always first and never a file — plus every `<name>.theme` on the
/// search path.
///
/// For the settings panel. It touches the filesystem, so it is not for a draw
/// path.
pub fn available_themes() -> Vec<String> {
    let mut out: Vec<String> = vec!["default".to_string()];
    for d in FsThemes::new().dirs {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) != Some("theme") {
                continue;
            }
            if let Some(stem) = p.file_stem().and_then(|x| x.to_str()) {
                if !out.iter().any(|n| n == stem) {
                    out.push(stem.to_string());
                }
            }
        }
    }
    out[1..].sort();
    out
}

/// The directory the editor SAVES to: the user's own themes, first on the
/// search path after the explicit env override, so a saved theme is found
/// by the same walk that loads every other.
///
/// The family folder, and the old name never. One directory is written
/// to and both are read, so a machine keeps exactly the theme files it
/// had and gains one directory the first time a theme is saved; a theme
/// that was in the old folder and is saved again is answered from the
/// new one from then on, because that is where the walk looks first.
pub fn user_themes_dir() -> Option<PathBuf> {
    Some(data_home()?.join(FAMILY_DIR).join("themes"))
}

/// The first lines of a theme file the editor CREATED. A file that already
/// exists keeps its own opening — this banner is written once, when there
/// was nothing to patch, and never again.
const SAVE_BANNER: &str = "\
# Written by the theme editor. A later save PATCHES this file: the values
# the editor owns are replaced where they stand, and every other byte —
# these notes, hand-written tokens, moods, variants — is left alone.
";

/// The one line that introduces tokens the file did not have. Recognised on
/// the next save so a second block does not bring a second copy of it.
const SAVE_APPEND_BANNER: &str = "# Added by the theme editor.";

/// What a theme file's previous contents are called once a save has replaced
/// them. `.theme.bak`, so neither the loader's `<name>.theme` join nor
/// [`available_themes`]' extension test can mistake it for a theme.
///
/// One rescue copy per file, overwritten by each save, in the manner of the
/// RON store's. It exists because a save is now a PATCH of the person's own
/// file — their notes, their hand-written tokens, their moods — where before
/// it was a generator whose output could be produced again by pressing the
/// button. The value of the bytes went up; the cost of losing them had to
/// come down with it.
const SAVE_BACKUP_SUFFIX: &str = ".theme.bak";

/// Where a save's bytes are assembled before they become the theme. Renamed
/// over the target, which is atomic on every filesystem this program runs on,
/// so a save interrupted halfway leaves the old file whole instead of a
/// truncated one. `std::fs::write` opens with `O_TRUNC`: it destroys before
/// it writes, and there is no moment at which it is safe to be killed.
const SAVE_TEMP_SUFFIX: &str = ".theme.part";

/// WHAT A SAVE MEANS FOR A TOKEN NOBODY TOUCHED — settled 2026-08-18.
///
/// It is left exactly as it was, byte for byte. A save PATCHES: every value
/// the edit set names is replaced where it stands in the file, tokens the
/// file did not have are appended, and nothing else in the file is read,
/// rewritten or reordered.
///
/// The alternative — generate the file whole and require the edit set to
/// mention everything that must survive — is what this function used to do,
/// and it is the whole of the owner's report on 2026-08-17: "the halo does
/// not blink any more, but it disappears when I press save". `edit.rs`
/// deliberately WITHHOLDS `glow.panel_edge.radius`/`.alpha` from a theme
/// that dressed its own halo, so that the editor's seeds do not flatten the
/// author's numbers. Laid over a bake that is a keep; written into a
/// regenerated file it was a delete, and the halo went out. One edit set
/// cannot serve an overlay and a whole-file rewrite, because the two read
/// silence as opposite instructions. Patching makes the file read silence
/// the way the overlay does, which is what lets ONE set answer the three
/// callers `edit.rs`' header names.
///
/// It is also the only way the plan's other promise can be kept: the author's
/// comments survive a save (`.gap-program/decyzja-edytor-motywu.md` §2-3,
/// which forbids regeneration outright). `parse.rs` cuts comments off before
/// parsing and `Document` has no field for them, so text is the only place
/// they exist to be preserved.
///
/// The bytes a save starts from are THE EDITED THEME'S own file — the file
/// the preview on screen is standing on — taken from the user's own
/// directory first and from the loader's search path second, so a theme
/// installed system-wide is COPIED into the user's directory on its first
/// save instead of being replaced by a file holding only the editor's dozen
/// values. That is `layout/store.rs`' materialisation, in the words of the
/// other store.
///
/// EDITED THEME, not "the name being written": the two are the same for
/// SAVE and different for SAVE AS, and reading the second where the first
/// was meant is how a patch invents a theme nobody asked for. Saving a
/// dressed theme AS a name that already carries a file would keep THAT
/// file's halo — the set is silent about a dress, and the silence would be
/// answered by the wrong file — so the saved theme would match neither the
/// preview nor the theme it was saved over. That is [`save_theme_as`]'
/// whole reason to exist, and why [`save_theme`] is nothing but the case
/// where the two names are one.
///
/// So SAVE AS IS A COPY OF THIS THEME, settled here rather than left open,
/// because the alternative reading — "a new theme, the edit set and nothing
/// else" — is the 2026-08-17 report again under a different button: the
/// dress the set withholds would go out. A name with no file anywhere still
/// generates, but that is the absence of a source, not a second meaning.
///
/// A generated file groups keys into the section the master declares them
/// under by splitting at the FIRST dot, which the loader's `section.key`
/// concatenation makes exact. The round-trip is pinned by an integration
/// test, not assumed.
///
/// `default` is refused as a TARGET name: the master is the one look
/// compiled in, and a file called `default.theme` would shadow it into
/// confusion — the caller offers SAVE AS instead. As a SOURCE it is legal
/// and means what it says: the master is not a file, so there is nothing to
/// copy and the set is the whole of the new theme.
pub fn save_theme(name: &str, edits: &[edit::Edit]) -> std::io::Result<PathBuf> {
    save_theme_as(Some(name), name, edits)
}

/// SAVE AS: the theme `source` was showing, written out under `name`.
///
/// `source` is the theme the edit set describes — the one the editor opened
/// and the preview is standing on. `None`, and `Some("default")`, both name
/// the embedded master, which is not a file: there the edit set is the whole
/// of the new theme.
///
/// See [`save_theme`] for what a save means for a token nobody touched; this
/// is the same function with the two names told apart.
pub fn save_theme_as(
    source: Option<&str>,
    name: &str,
    edits: &[edit::Edit],
) -> std::io::Result<PathBuf> {
    use std::io::{Error, ErrorKind};
    let name = name.trim();
    if name.is_empty() || name.eq_ignore_ascii_case("default") {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "the master is not a file; save under another name",
        ));
    }
    if !is_theme_name(name) {
        return Err(Error::new(
            ErrorKind::InvalidInput,
            "a theme's name is its file's: ascii letters, digits, - and _",
        ));
    }
    // The source becomes a path too, so it meets the same charset for the
    // same reason `FsThemes::open` states: a name that could spell `..` is a
    // file-read primitive. `default` is not refused here — it is simply not
    // a file, and drops out of the filter below into the generator.
    let source = source
        .map(str::trim)
        .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("default"));
    if let Some(s) = source {
        if !is_theme_name(s) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "a theme's name is its file's: ascii letters, digits, - and _",
            ));
        }
    }
    // Checked before a byte is written, and for every edit, so a set with a
    // bare name cannot leave half a save on disk. Refused loudly, not
    // dropped: today no edit produces one, and the day one does, a silent
    // skip would be a value the person set and the file never learned about.
    for e in edits {
        if !e.token.contains('.') {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                format!("token without a section: {}", e.token),
            ));
        }
    }
    let dir = user_themes_dir()
        .ok_or_else(|| Error::new(ErrorKind::NotFound, "no home, nowhere to save"))?;
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("{name}.theme"));
    let base = source.and_then(|s| {
        // The user's own copy first — a theme saved here before is patched
        // where it was written — then the walk, which is what materialises
        // one that has only ever been installed.
        std::fs::read_to_string(dir.join(format!("{s}.theme")))
            .ok()
            .or_else(|| installed_theme_text(s))
    });
    let text = match base {
        Some(b) if !b.trim().is_empty() => patch_theme_text(name, &b, edits),
        _ => generated_theme_text(edits),
    };
    write_theme_file(&dir, name, &text)?;
    Ok(path)
}

/// A theme name is a bare identifier, never a path — the rule
/// `FsThemes::open` enforces on load, in one place so a save cannot enforce
/// a different one.
fn is_theme_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Replaces `<name>.theme` with `text`, keeping what stood there as
/// `<name>[SAVE_BACKUP_SUFFIX]`.
///
/// The backup is written FIRST and its failure fails the save: a save with
/// no rescue copy is exactly the one that must not happen, now that the
/// bytes being replaced are the person's own work rather than this module's
/// output. Then the new text goes to a temporary file and is RENAMED over
/// the target, so the moment of replacement is one syscall wide.
///
/// A file identical to what is already there is still rewritten — the
/// rename makes that harmless — but it does not move the backup, so pressing
/// SAVE twice cannot cost the person the copy the first press made.
fn write_theme_file(dir: &std::path::Path, name: &str, text: &str) -> std::io::Result<()> {
    let path = dir.join(format!("{name}.theme"));
    if let Ok(old) = std::fs::read_to_string(&path) {
        if !old.trim().is_empty() && old != text {
            std::fs::write(dir.join(format!("{name}{SAVE_BACKUP_SUFFIX}")), &old)?;
        }
    }
    let tmp = dir.join(format!("{name}{SAVE_TEMP_SUFFIX}"));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &path)
}

/// The text of `<name>.theme` as the LOADER would find it, for a theme that
/// has never been saved into the user's own directory. Same walk, same
/// precedence, so the file a save starts from is the file the person is
/// looking at.
fn installed_theme_text(name: &str) -> Option<String> {
    FsThemes::new()
        .dirs
        .iter()
        .map(|d| d.join(format!("{name}.theme")))
        .find(|p| p.is_file())
        .and_then(|p| std::fs::read_to_string(p).ok())
}

/// A brand-new theme file: the edit set and nothing else.
fn generated_theme_text(edits: &[edit::Edit]) -> String {
    let mut text = String::from(SAVE_BANNER);
    for (section, keys) in group_by_section(edits) {
        text.push_str(&format!("\n[{section}]\n"));
        for (k, v) in keys {
            text.push_str(&format!("{k} = {v}\n"));
        }
    }
    text
}

/// `elev.panel.glass.rank` under `[elev]` as `panel.glass.rank`: the split is
/// at the FIRST dot and the loader concatenates `section.key` back, so any
/// depth of key survives one section header. Sections keep the order the
/// edits arrived in, which is the order `edit.rs` builds its groups in.
fn group_by_section<'a>(edits: &'a [edit::Edit]) -> Vec<(&'a str, Vec<(&'a str, &'a str)>)> {
    let mut out: Vec<(&str, Vec<(&str, &str)>)> = Vec::new();
    for e in edits {
        // Unreachable: `save_theme` refuses a set holding a bare name before
        // it opens a file, so nothing is dropped here that a caller was not
        // already told about.
        let Some((section, key)) = e.token.split_once('.') else { continue };
        match out.iter_mut().find(|(s, _)| *s == section) {
            Some((_, keys)) => keys.push((key, e.value.as_str())),
            None => out.push((section, vec![(key, e.value.as_str())])),
        }
    }
    out
}

/// The file with the edits' values swapped in where they already stand, and
/// appended where they do not. Everything else is copied through untouched.
///
/// Three rules decide what "where they already stand" means, and each one is
/// a way a naive patch would corrupt a file:
///
/// * An OVERLAY section is not the base value. `[mood.alert]` and
///   `[variant.x]` re-declare absolute tokens for one sibling only
///   (`default.theme:4157` does exactly this to the panel-edge halo), and
///   writing the editor's number there would change one mood and leave the
///   theme itself alone.
/// * A LOCALISED key (`meta.name[pl]`) addresses the same token in another
///   language and is never what an edit means.
/// * A value that does not sit whole on one line is left alone and the token
///   is appended instead. `value_span.len` is the length of the JOINED text
///   of a multi-line array while `.line` remembers only the first line, so
///   patching that span by its numbers would cut the file mid-value. No
///   `.theme` in the tree has one today; the guard is what makes that a fact
///   about today rather than a bet ([`value_byte_range`]).
///
/// The first two are live shapes with a test each. The third is live too,
/// and the rest of what [`value_byte_range`] checks is NOT — it is arithmetic
/// about `parse.rs`' spans that holds by construction, written down as a
/// refusal so that the day it stops holding the save appends instead of
/// cutting. Which is which is said there, key by key, because an untestable
/// check described as a live case reads as coverage nobody has.
///
/// WHAT IT DOES NOT FOLLOW: `@include`. The text is parsed by
/// `parse::parse_text`, which hands the parser no base directory, so an
/// include is skipped with a warning and the tokens it carries are not in
/// this document at all. They therefore read as MISSING and are appended to
/// the parent — where, being last, the cascade gives them the editor's
/// value, so the save takes. The hole left is narrow and real: a token
/// declared in the parent AND overridden by an include is patched in the
/// parent and still loses to the include, so that one edit silently does not
/// take. Following an include means deciding which file owns a token and
/// writing into a file the caller did not name; it wants the owner, not a
/// guess here.
///
/// The LAST plain declaration wins, because that is the one the cascade
/// keeps: `cascade::apply_document` walks keys in file order and each one
/// overwrites the last.
fn patch_theme_text(name: &str, base: &str, edits: &[edit::Edit]) -> String {
    let mut src = Sources::new();
    let mut diags = Vec::new();
    let doc = parse::parse_text(&mut src, name, base, &mut diags);
    let starts = line_starts(base);

    let mut spans: HashMap<String, (usize, usize)> = HashMap::new();
    if let Some(doc) = doc.as_ref() {
        for kv in &doc.keys {
            if kv.locale.is_some() {
                continue;
            }
            let overlay = doc
                .sections
                .get(kv.section as usize)
                .map(parse::Section::is_overlay)
                .unwrap_or(false);
            if overlay {
                continue;
            }
            // File 0 is the text handed in, and today it is the only file
            // there is: `parse_text` passes no base directory, so `do_include`
            // warns and skips rather than opening anything, and `Sources` was
            // made empty two lines above — the first `add` is 0. So this
            // never fires. It stands as the seatbelt for the day an include
            // IS followed here, because a span measured in another file would
            // address arbitrary bytes of this one and `value_byte_range`
            // would have no way to know: its checks all pass on a line that
            // merely happens to look like a value.
            if kv.value_span.file != 0 {
                continue;
            }
            if let Some(r) = value_byte_range(base, &starts, kv.value_span) {
                spans.insert(kv.token(), r);
            }
        }
    }

    let mut swaps: Vec<(usize, usize, &str)> = Vec::new();
    let mut missing: Vec<edit::Edit> = Vec::new();
    for e in edits {
        match spans.get(e.token) {
            // A repeated token in one set is the LAST one's value, the same
            // rule the file itself is read by.
            Some(&(a, b)) => match swaps.iter_mut().find(|(s, _, _)| *s == a) {
                Some(slot) => slot.2 = e.value.as_str(),
                None => swaps.push((a, b, e.value.as_str())),
            },
            None => match missing.iter_mut().find(|m| m.token == e.token) {
                Some(slot) => slot.value = e.value.clone(),
                None => missing.push(e.clone()),
            },
        }
    }

    let mut out = base.to_string();
    swaps.sort_by_key(|(a, _, _)| *a);
    // Back to front, so an earlier swap's offsets are still the offsets of
    // the string being edited.
    for (a, b, v) in swaps.into_iter().rev() {
        out.replace_range(a..b, v);
    }

    if !missing.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        if !out.contains(SAVE_APPEND_BANNER) {
            out.push('\n');
            out.push_str(SAVE_APPEND_BANNER);
            out.push_str(
                " Tokens the file did not carry yet;\n\
                 # from here on they are patched in place like every other.\n",
            );
        }
        for (section, keys) in group_by_section(&missing) {
            out.push_str(&format!("\n[{section}]\n"));
            for (k, v) in keys {
                out.push_str(&format!("{k} = {v}\n"));
            }
        }
    }
    out
}

/// Byte offset of the start of every line, indexed the way `Span::line` is
/// (line 1 is entry 0).
fn line_starts(text: &str) -> Vec<usize> {
    let mut out = vec![0usize];
    out.extend(text.match_indices('\n').map(|(i, _)| i + 1));
    out
}

/// Where a value's text actually lies in the file, or `None` when the span
/// cannot be trusted to say.
///
/// `Span::col` and `Span::len` are BYTES (`parse.rs`' `indent` and
/// `find_assign` both count them), and `strip_comment` returns a PREFIX of
/// its line, so an offset measured on the code half of a line is an offset
/// into the raw line as well.
///
/// ONE of the checks below answers a shape a `.theme` can really have: the
/// span running past the end of the code is how a multi-line value gives
/// itself away, because `parse.rs` measures such a span on the JOINED text
/// and the join is always longer than what the first line has left. That one
/// is pinned by a test.
///
/// The others cannot be reached from `parse.rs` as it stands — a value span
/// starts at the first non-blank byte after `=` and is exactly the trimmed
/// value long, so it can neither carry whitespace at an end nor leave code
/// behind it — and they are here as a CHECKSUM on that arithmetic rather
/// than as handling for a case. If a future `parse.rs` makes a span mean
/// something else, the patch refuses the token and appends it instead of
/// cutting the file at numbers it no longer understands. Saying so plainly
/// beats calling them guards, which reads as coverage that does not exist:
/// remove them today and every test still passes.
fn value_byte_range(text: &str, starts: &[usize], span: Span) -> Option<(usize, usize)> {
    let idx = (span.line as usize).checked_sub(1)?;
    let start = *starts.get(idx)?;
    let end_of_line = text[start..].find('\n').map(|i| start + i).unwrap_or(text.len());
    let line = &text[start..end_of_line];
    let line = line.strip_suffix('\r').unwrap_or(line);
    let code = line.get(..parse::code_len(line))?;
    let a = (span.col as usize).checked_sub(1)?;
    let b = a.checked_add(span.len as usize)?;
    // The live one: a multi-line value.
    if b > code.len() {
        return None;
    }
    // Checksum from here down; see this function's note.
    let value = code.get(a..b)?;
    if value.is_empty() || value.trim().len() != value.len() {
        return None;
    }
    if !code.get(b..)?.trim().is_empty() {
        return None;
    }
    Some((start + a, start + b))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// §4.1 stage 5. One file, always the last word.
///
/// The family folder first and the old name behind it, for the reason
/// [`theme_search_path`] gives: the overlay is the last stage of the
/// same cascade, and a stage that had been left pointing at the old
/// folder alone would go on answering from it after everything else
/// had moved.
fn user_overlay_path() -> Option<PathBuf> {
    // Returned whether or not it is there, which is deliberate and
    // unchanged: somebody who names a file outright is told by the
    // reader a rung up that it could not be read, where a silent skip
    // would look exactly like an overlay that had no effect.
    if let Some(p) = std::env::var_os("NACELLE_THEME_LOCAL") {
        return Some(PathBuf::from(p));
    }
    let base = match std::env::var_os("XDG_CONFIG_HOME") {
        Some(c) if !c.is_empty() => PathBuf::from(c),
        _ => home_dir()?.join(".config"),
    };
    overlay_candidates(&base).into_iter().find(|p| p.is_file())
}

/// The overlay's two candidates, in order, the file system not asked
/// yet.
///
/// Split out for the same reason [`theme_search_path`] is: what the
/// ORDER is, is a decision, and a decision visible only through an
/// environment and a disk is one no test can state plainly.
fn overlay_candidates(config_home: &Path) -> Vec<PathBuf> {
    [FAMILY_DIR, LEGACY_FAMILY_DIR]
        .iter()
        .map(|name| config_home.join(name).join("theme.local"))
        .collect()
}

// ------------------------------------------------------------------- reports

fn collect_meta(
    meta: &mut ThemeDiagnostics,
    schema: &Schema,
    default_doc: &Document,
    theme_doc: Option<&Document>,
) {
    let mut take = |doc: &Document| {
        if let Some(v) = doc.meta_text("meta.name") {
            meta.name.retain(|(l, _)| !l.is_empty());
            meta.name.insert(0, (String::new(), v));
        }
        if let Some(v) = doc.meta_text("meta.description") {
            meta.description.retain(|(l, _)| !l.is_empty());
            meta.description.insert(0, (String::new(), v));
        }
        if let Some(v) = doc.meta_text("meta.author") {
            meta.author = v;
        }
        if let Some(v) = doc.meta_text("meta.family") {
            meta.family = v;
        }
        if let Some(Expr::Num(v)) = doc.meta(&"meta.schema".to_string()) {
            meta.schema = *v as u32;
        }
        for kv in &doc.keys {
            let (Some(lang), Expr::Text(t)) = (&kv.locale, &kv.value) else { continue };
            match kv.key.as_str() {
                "meta.name" => meta.name.push((lang.clone(), t.clone())),
                "meta.description" => meta.description.push((lang.clone(), t.clone())),
                _ => {}
            }
        }
    };
    take(default_doc);
    if let Some(d) = theme_doc {
        take(d);
    }
    for (k, lang, v) in &schema.localised {
        if k == "meta.name" && !meta.name.iter().any(|(l, _)| l == lang) {
            meta.name.push((lang.clone(), v.clone()));
        }
    }
}

fn collect_texts(meta: &mut ThemeDiagnostics, engine: &Engine) {
    let s = &engine.siblings[engine.active.min(engine.siblings.len().saturating_sub(1))];
    let mut out = Vec::new();
    let r = resolve::resolve(&engine.schema, &s.spec, &mut out);
    for (i, v) in r.values.iter().enumerate() {
        if let Value::Text(t) = v {
            meta.texts.push((engine.schema.name(TokenId(i as u16)).to_string(), t.clone()));
        }
    }
}

/// §4.2: reports go to four places, and stderr at load is the first. Printed
/// once, in §4.3's shape.
fn report(meta: &ThemeDiagnostics) {
    if meta.warnings.is_empty() {
        return;
    }
    let name = meta.localised_name("");
    eprintln!("theme \"{name}\"");
    for w in &meta.warnings {
        eprint!("{w}");
    }
}

// ----------------------------------------------------------------- hot ids

/// The hot set: the tokens a draw path reads every frame.
///
/// Each helper resolves **by name at load** and caches the id in a
/// `static OnceLock<TokenId>`, so a per-frame read is
/// `theme.color(ids::text_primary())` — one atomic load of an already-set
/// `OnceLock` plus one bounds-checked slice index. A name `default.theme` does
/// not declare degrades to [`TokenId::MISSING`], which every accessor tolerates,
/// and warns once.
///
/// Everything outside this list goes through [`ResolvedTheme::id`] at widget
/// init and is cached by the caller. Nothing here is a hard-coded value — only
/// a hard-coded *question*.
pub mod ids {
    use super::TokenId;
    use std::sync::OnceLock;

    macro_rules! hot {
        ($($fname:ident => $token:literal),* $(,)?) => {
            $(
                #[doc = concat!("`", $token, "`")]
                #[inline]
                pub fn $fname() -> TokenId {
                    static ID: OnceLock<TokenId> = OnceLock::new();
                    *ID.get_or_init(|| super::hot_id($token))
                }
            )*
            /// Every name in the hot set, for the startup check.
            pub const HOT_SET: &[&str] = &[$($token),*];
        };
    }

    hot! {
        // the five seeds (§5.2)
        palette_black   => "palette.black",
        palette_white   => "palette.white",
        palette_accent  => "palette.accent",
        // surfaces (§5.5)
        surface_base    => "surface.base",
        surface_panel   => "surface.panel",
        surface_scrim   => "surface.scrim",
        // text roles (§5.6)
        text_title      => "text.title",
        text_primary    => "text.primary",
        text_secondary  => "text.secondary",
        text_muted      => "text.muted",
        text_disabled   => "text.disabled",
        text_inverse    => "text.inverse",
        // chrome (§5.7, §5.8)
        accent_primary  => "accent.primary",
        accent_hover    => "accent.hover",
        border_default  => "border.default",
        border_width    => "border.edge.width",
        focus_ring_width => "focus.ring.width",
        // the terminal (§5.11) — the 12 000-cell inner loop
        term_fg         => "term.fg",
        term_bg         => "term.bg",
        term_cursor     => "term.cursor",
        // the ladders a widget reaches for constantly (§5.4)
        space_2         => "space.2",
        space_4         => "space.4",
        size_md         => "size.md",
        stroke_hair     => "stroke.hair",
        corner_md       => "corner.md",
    }

    /// `term.ansi[i]`, resolved once per slot. The sixteen live in the same
    /// token space as everything else, so they are addressable from an icon
    /// layer or a type role's `fg` exactly like any other colour (§7.1).
    pub fn term_ansi(i: usize) -> TokenId {
        static IDS: OnceLock<[TokenId; 16]> = OnceLock::new();
        let all = IDS.get_or_init(|| {
            let mut v = [TokenId::MISSING; 16];
            for (k, slot) in v.iter_mut().enumerate() {
                *slot = super::id(&format!("term.ansi[{k}]")).unwrap_or(TokenId::MISSING);
            }
            v
        });
        all.get(i).copied().unwrap_or(TokenId::MISSING)
    }

    /// `data.series[i]`, the eight-colour plot ramp.
    pub fn data_series(i: usize) -> TokenId {
        static IDS: OnceLock<[TokenId; 8]> = OnceLock::new();
        let all = IDS.get_or_init(|| {
            let mut v = [TokenId::MISSING; 8];
            for (k, slot) in v.iter_mut().enumerate() {
                *slot = super::id(&format!("data.series[{k}]")).unwrap_or(TokenId::MISSING);
            }
            v
        });
        all.get(i).copied().unwrap_or(TokenId::MISSING)
    }
}

fn hot_id(name: &str) -> TokenId {
    match id(name) {
        Some(t) => t,
        None => {
            eprintln!(
                "nacelle::theme: default.theme does not declare \"{name}\" — \
                 drawing falls back to this token's kind default"
            );
            TokenId::MISSING
        }
    }
}

/// Report every hot-set name `default.theme` does not declare. Called once at
/// startup by the application so the omission is a line in the log rather than
/// a colour nobody can explain.
pub fn check_hot_set() -> Vec<String> {
    ids::HOT_SET
        .iter()
        .filter(|n| id(n).is_none())
        .map(|n| format!("hot token \"{n}\" is not declared by default.theme"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A capsule is a radius the box has to be asked for, and the sentinel
    /// that asks for it is the one every `radius > 0.0` test has been
    /// reading as "no corner".
    #[test]
    fn a_pill_becomes_a_radius_only_once_there_is_a_box_to_measure() {
        let pill = expr::sentinel("pill").unwrap();
        assert_eq!(corner_radius(pill, 200.0, 8.0), 4.0);
        // Half the SHORT side, whichever way round the box lies.
        assert_eq!(corner_radius(pill, 8.0, 200.0), 4.0);
        // A stated length is itself; clamping to the box is the ring
        // generator's job and stays there.
        assert_eq!(corner_radius(3.0, 200.0, 8.0), 3.0);
        // The other sentinels are the ABSENCE of a length. Answering one of
        // them with a guessed radius would be this file choosing a shape.
        for word in ["none", "auto", "same_as_parent"] {
            let v = expr::sentinel(word).unwrap();
            assert_eq!(corner_radius(v, 200.0, 8.0), 0.0, "{word}");
        }
    }

    /// The bounds a role's size meets, in the one place both resolvers
    /// meet them. The last line is the reason this is a function at all:
    /// a stated floor of zero used to mean "no floor" on one side of the
    /// library and "unstated, take the global" on the other, so one theme
    /// sized one role two ways in two halves of one screen.
    #[test]
    fn a_role_meets_its_own_bounds_first_and_the_global_floor_last() {
        // Unbounded on both ends: `0` is how the master spells an absent
        // bound, and must never read as a ceiling of nothing.
        assert_eq!(role_px(10.0, 0.0, 0.0, 0.0), 10.0);
        // The role's own ceiling caps it, its own floor lifts it.
        assert_eq!(role_px(10.0, 0.0, 0.0, 9.0), 9.0);
        assert_eq!(role_px(4.0, 6.0, 0.0, 0.0), 6.0);
        // A ceiling under the floor is a theme contradicting itself, and
        // the floor is the last defence against unreadable type.
        assert_eq!(role_px(10.0, 12.0, 8.0, 4.0), 12.0);
        // The global floor holds up a role that states none of its own.
        // A MISSING token and a stated `0px` reach here as the same
        // number on purpose — that is the silence both resolvers now read
        // the same way, and neither of them may shrink type to nothing
        // because a theme wrote a bound it does not have.
        assert_eq!(role_px(4.0, 0.0, 8.0, 0.0), 8.0);
    }

    /// Every name in the hot set must be a token `default.theme` declares.
    ///
    /// A hot id that resolves to MISSING does not crash and does not warn on a
    /// draw path — it silently falls back, which is how `border.width` was
    /// wrong for a whole release without anybody seeing it: the borders simply
    /// kept the hard-coded thickness and the theme looked like it only changed
    /// colour. The check belongs in the test suite, not in the log.
    #[test]
    fn every_hot_token_is_a_token_the_master_declares() {
        let mut out = Vec::new();
        let mut src = Sources::new();
        let f = src.add("default.theme", DEFAULT_THEME);
        let doc = parse::parse(&mut src, f, None, &mut out);
        let schema = Schema::from_default(&doc, &mut out);
        let missing: Vec<&str> = ids::HOT_SET
            .iter()
            .copied()
            .filter(|n| schema.id(n).is_none())
            .collect();
        assert!(missing.is_empty(), "hot names default.theme does not declare: {missing:?}");
    }

    // `every_shipped_theme_loads_over_the_master...` left with the shipped
    // themes themselves (2026-08-16): there is nothing compiled in to iterate
    // but the master, and the editor's saved files are pinned by their own
    // round-trip integration test instead.

    /// The class x state matrix is real: a button's hover rung derives from
    /// the ACCENT (its class base) while a panel's derives from the BORDER
    /// colour — two different classes, two different ladders, from one
    /// [state] section. Before the [class] block existed every rung baked
    /// against white, which is exactly what this asserts never returns.
    #[test]
    fn the_state_ladder_bakes_per_class_not_against_white() {
        let mut out = Vec::new();
        let mut src = Sources::new();
        let f = src.add("default.theme", DEFAULT_THEME);
        let doc = parse::parse(&mut src, f, None, &mut out);
        let mut schema = Schema::from_default(&doc, &mut out);
        let spec = schema.base_spec();
        let r = {
            let rr = resolve::resolve_default(&schema, &mut out);
            schema.adopt_kinds(&rr.values);
            let _ = spec;
            resolve::resolve(&schema, &schema.base_spec(), &mut out)
        };
        let t = bake::bake(&schema, &r, &BakeInput::default(), &mut out);
        assert!(t.class_count() >= 20, "class matrix missing: {}", t.class_count());

        let button = r
            .class_ids
            .iter()
            .position(|&id| schema.name(id) == "class.button")
            .unwrap() as u16;
        let panel = r
            .class_ids
            .iter()
            .position(|&id| schema.name(id) == "class.panel")
            .unwrap() as u16;

        let bh = t.class_state(button, parse::State::Hover);
        let ph = t.class_state(panel, parse::State::Hover);
        // hover.text = base — so it must BE each class's base, not white.
        assert!(bh.text.r < 0.99 || bh.text.g < 0.99 || bh.text.b < 0.99,
            "button hover text baked against white: {:?}", bh.text);
        // The two bases share a hue by design (@border.default IS the accent
        // at 0.55), so the distance must include alpha or it proves nothing.
        let d = (bh.text.r - ph.text.r).abs()
            + (bh.text.g - ph.text.g).abs()
            + (bh.text.b - ph.text.b).abs()
            + (bh.text.a - ph.text.a).abs();
        assert!(d > 0.05, "two classes share one ladder: {:?} vs {:?}", bh.text, ph.text);
        // And the ladder's own arithmetic survives: press fills stronger than idle.
        let bi = t.class_state(button, parse::State::Idle);
        let bp = t.class_state(button, parse::State::Press);
        assert!(bp.fill.a > bi.fill.a, "press ({}) not above idle ({})", bp.fill.a, bi.fill.a);
    }

    /// §5.21b's reason to exist: `button` and `checkbox` share the exact
    /// same base colour (`@accent.primary`), so any difference between
    /// their baked state styles can only come from the FORMULA — proof
    /// that `[family]` actually routes a class to a different ladder,
    /// isolated from the confound the button-vs-panel test above accepts
    /// (those two also differ in base colour).
    #[test]
    fn a_class_named_in_family_bakes_against_its_own_ladder_not_state() {
        let mut out = Vec::new();
        let mut src = Sources::new();
        let f = src.add("default.theme", DEFAULT_THEME);
        let doc = parse::parse(&mut src, f, None, &mut out);
        let mut schema = Schema::from_default(&doc, &mut out);
        let rr = resolve::resolve_default(&schema, &mut out);
        schema.adopt_kinds(&rr.values);
        let r = resolve::resolve(&schema, &schema.base_spec(), &mut out);
        let t = bake::bake(&schema, &r, &BakeInput::default(), &mut out);

        let button = r.class_ids.iter().position(|&id| schema.name(id) == "class.button").unwrap();
        let checkbox = r.class_ids.iter().position(|&id| schema.name(id) == "class.checkbox").unwrap();
        let window = r.class_ids.iter().position(|&id| schema.name(id) == "class.window").unwrap();

        assert_eq!(r.class_family[button], 0, "button must stay on the bare [state] ladder");
        assert_eq!(r.class_family[checkbox], 1, "checkbox must read [family] and land on ladder 1 (input)");
        assert_eq!(r.class_family[window], 2, "window must read [family] and land on ladder 2 (window)");

        let button_base = r.values[r.class_ids[button].index()].as_color();
        let checkbox_base = r.values[r.class_ids[checkbox].index()].as_color();
        assert_eq!(button_base, checkbox_base, "the test's premise: button and checkbox must share one base colour");

        let b_sel = t.class_state(button as u16, parse::State::Selected);
        let c_sel = t.class_state(checkbox as u16, parse::State::Selected);
        // [state] selected.fill = alpha(base, 0.14); [state.input] selected.fill
        // = alpha(base, 0.55) — a same-base pair four times apart in alpha is
        // not measurement noise, it is two different formulas.
        assert!(
            c_sel.fill.a > b_sel.fill.a * 2.0,
            "checkbox selected ({}) is not meaningfully louder than button selected ({}) despite an identical base colour — the family split is not taking effect",
            c_sel.fill.a, b_sel.fill.a
        );

        // window's own ladder: dragging lifts exactly one rank, never two —
        // §5.21d's point that a container is not a pressed button.
        let w_drag = t.class_state(window as u16, parse::State::Dragging);
        assert_eq!(w_drag.elevation, 1.0, "window dragging must lift exactly one rank, not the button ladder's two");
    }

    /// The governing principle's own acceptance test: a [meta]-only master
    /// still parses, resolves and bakes — into an EMPTY table, whose every
    /// lookup answers the per-kind raw default. The program with no design
    /// anywhere must run and look unstyled, never crash and never look like
    /// yesterday's theme.
    #[test]
    fn an_empty_master_bakes_raw_and_nothing_panics() {
        let mut out = Vec::new();
        let mut src = Sources::new();
        let f = src.add("default.theme", "[meta]\nschema = 1\nname = \"raw\"\n");
        let doc = parse::parse(&mut src, f, None, &mut out);
        let mut schema = Schema::from_default(&doc, &mut out);
        let r = resolve::resolve_default(&schema, &mut out);
        schema.adopt_kinds(&r.values);
        let spec = schema.base_spec();
        let rr = resolve::resolve(&schema, &spec, &mut out);
        let t = bake::bake(&schema, &rr, &BakeInput::default(), &mut out);
        // The two [meta] keys intern like any others; nothing else exists.
        assert!(t.len() <= 2, "{} tokens from an empty master", t.len());
        assert_eq!(t.class_count(), 0);
        // Every read degrades to the kind default — grey ink, zero lengths.
        assert_eq!(t.color(TokenId::MISSING), bake::StateStyle::RAW.text);
        let st = t.class_state(0, parse::State::Hover);
        assert_eq!(st.edge_width, 1.0);
    }

    #[test]
    fn the_embedded_master_parses_resolves_and_bakes() {
        let mut out = Vec::new();
        let mut src = Sources::new();
        let f = src.add("default.theme", DEFAULT_THEME);
        let doc = parse::parse(&mut src, f, None, &mut out);
        let rendered: String = out.iter().map(|d| d.render(&src)).collect();
        assert!(out.is_empty(), "default.theme must parse clean:\n{rendered}");

        let mut schema = Schema::from_default(&doc, &mut out);
        let r = resolve::resolve_default(&schema, &mut out);
        schema.adopt_kinds(&r.values);
        let rendered: String = out.iter().map(|d| d.render(&src)).collect();
        // §6.3 step 4: "The compiled-in `default` is cycle-free by
        // construction, which a unit test asserts by resolving it."
        assert!(
            !rendered.contains("reference cycle"),
            "default.theme must be cycle-free:\n{rendered}"
        );
        let t = bake::bake(&schema, &r, &BakeInput::default(), &mut out);
        assert_eq!(t.len(), schema.len());
        assert!(t.unit_px > 0.0);
    }

    #[test]
    fn every_token_declared_by_the_master_is_addressable_by_name() {
        let mut out = Vec::new();
        let mut src = Sources::new();
        let f = src.add("default.theme", DEFAULT_THEME);
        let doc = parse::parse(&mut src, f, None, &mut out);
        let schema = Schema::from_default(&doc, &mut out);
        for n in schema.names() {
            assert!(schema.id(n).is_some(), "{n} interned but not addressable");
        }
    }

    #[test]
    fn resolved_never_returns_null_and_never_panics() {
        let t = resolved();
        // Whatever default.theme currently holds, the accessors are total.
        let _ = t.color(TokenId::MISSING);
        let _ = t.px(TokenId::MISSING);
        let _ = t.flag(TokenId::MISSING);
        let _ = t.enum_of(TokenId::MISSING);
        assert!(epoch() >= 1);
    }

    #[test]
    fn the_hot_set_degrades_rather_than_panicking() {
        let _ = resolved();
        // Absent tokens come back MISSING and read as their kind's fallback;
        // present ones index the arrays.
        let idx = ids::palette_accent();
        let t = resolved();
        let _ = t.color(idx);
        let _ = ids::term_ansi(99);
        let _ = ids::data_series(99);
        // check_hot_set names what is missing rather than hiding it
        for line in check_hot_set() {
            assert!(line.contains("is not declared"));
        }
    }

    /// EVERY LEVEL ASKS BOTH NAMES, THE FAMILY ONE FIRST.
    ///
    /// It asked neither: the walk wore the old name at all three of its
    /// levels and the family name at none, so `<data>/nacelle/themes` —
    /// the location `nacelle-themes/config/nacelle-desktop.ron` gives
    /// for a theme file, and the folder the desktop's own startup
    /// message tells people to move their data into — was the one place
    /// a theme could sit unread. What the pairing
    /// answers to is that repository's own Makefile, which says of this
    /// program that it "searches both names, the new one first" and
    /// declines to move anybody's files on the strength of it. Nothing
    /// is moved here either: the old rungs are all still on the list,
    /// one place further down.
    ///
    /// No theme is INSTALLED by anything — that Makefile ships sounds
    /// and layauts only — so this directory is one a person fills and
    /// the editor writes to, which is exactly why a resolver that never
    /// looks in it is a defect nobody would see reported as one.
    #[test]
    fn the_family_folder_is_on_the_theme_search_path_and_the_old_one_stays() {
        let data = PathBuf::from("/x/data");
        let config = PathBuf::from("/x/config");
        let dirs = theme_search_path(None, Some(data.clone()), Some(config.clone()), None);
        let new = data.join("nacelle/themes");
        let old = data.join("nacelle-desktop/themes");
        let at = |p: &PathBuf| dirs.iter().position(|d| d == p);
        assert!(at(&new).is_some(), "the documented folder is not searched: {dirs:?}");
        assert!(at(&old).is_some(), "the old folder is read, not dropped: {dirs:?}");
        assert!(at(&new) < at(&old), "the family folder answers first: {dirs:?}");
        // Both names at the system end too, same order.
        let sys_new = PathBuf::from("/usr/share/nacelle/themes");
        let sys_old = PathBuf::from("/usr/share/nacelle-desktop/themes");
        assert!(at(&sys_new) < at(&sys_old), "{dirs:?}");
        assert!(at(&sys_old).is_some(), "{dirs:?}");
        // The config rung is the old name ALONE: a theme is data, and
        // that entry exists only because themes once landed there.
        assert!(dirs.contains(&config.join("nacelle-desktop/themes")), "{dirs:?}");
        assert!(!dirs.contains(&config.join("nacelle/themes")), "{dirs:?}");
        // Every rung of it is below the data ones, which is what makes
        // an installed theme beat a leftover.
        assert!(at(&config.join("nacelle-desktop/themes")) > at(&old), "{dirs:?}");

        // The explicit override still outranks the lot, and a machine
        // with neither variable set still searches the family folder.
        let forced = PathBuf::from("/tmp/one-theme-dir");
        let dirs = theme_search_path(Some(forced.clone()), None, None, None);
        assert_eq!(dirs.first(), Some(&forced));
        assert!(dirs.contains(&sys_new), "{dirs:?}");

        // And the path the engine actually walks, however this machine
        // is set up, has the family folder on it.
        let live = FsThemes::new().dirs;
        assert!(
            live.iter().any(|d| d.ends_with("nacelle/themes")),
            "the live search path missed the family folder: {live:?}"
        );
    }

    /// THE PREFIX `sudo make install` USES IS ON THE PATH.
    ///
    /// `nacelle-themes/Makefile` documents `sudo make install` and
    /// defaults it to `PREFIX = /usr/local` for root, so its files land
    /// in `/usr/local/share/nacelle`. A system rung written as one
    /// hard-coded `/usr/share` — which is what stood here, and what the
    /// first pass at this repair left standing — misses that directory
    /// entirely: the same installer-versus-resolver drift as the old
    /// folder name, one rung lower down. `XDG_DATA_DIRS` is the list
    /// that answers it, exactly as `AssetRoots::xdg` reads it for every
    /// other asset in this crate.
    #[test]
    fn the_system_end_follows_xdg_data_dirs_and_not_one_hard_coded_prefix() {
        // Unset: the two standard prefixes, /usr/local first, because
        // that is the one `sudo make install` writes to.
        let dirs = theme_search_path(None, None, None, None);
        let local = PathBuf::from("/usr/local/share/nacelle/themes");
        let usr = PathBuf::from("/usr/share/nacelle/themes");
        let at = |p: &PathBuf| dirs.iter().position(|d| d == p);
        assert!(at(&local).is_some(), "sudo make install writes here: {dirs:?}");
        assert!(at(&local) < at(&usr), "the nearer prefix answers first: {dirs:?}");
        assert!(
            dirs.contains(&PathBuf::from("/usr/local/share/nacelle-desktop/themes")),
            "both names at every prefix: {dirs:?}"
        );

        // Set: what it says, in its order, and nothing that it does not
        // say. A packager who moves the tree is obeyed.
        let dirs = theme_search_path(None, None, None, Some("/opt/n/share:/usr/share"));
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/opt/n/share/nacelle/themes"),
                PathBuf::from("/opt/n/share/nacelle-desktop/themes"),
                PathBuf::from("/usr/share/nacelle/themes"),
                PathBuf::from("/usr/share/nacelle-desktop/themes"),
            ],
            "the variable is the list"
        );

        // Empty means unset, per the specification, and an empty member
        // is skipped rather than turned into a relative directory —
        // `"".join("nacelle")` is `nacelle`, which would be resolved
        // against the working directory.
        assert_eq!(theme_search_path(None, None, None, Some("")), theme_search_path(None, None, None, None));
        for d in theme_search_path(None, None, None, Some("/a::/b")) {
            assert!(d.is_absolute(), "{d:?} is relative");
        }
    }

    /// WHAT ONE NAME LOOKUP COSTS, COUNTED.
    ///
    /// `FsThemes::open` walks the search path and asks `is_file` about
    /// `<dir>/<name>.theme` in each until one answers, so the number of
    /// failed probes for a name that is not installed is the length of
    /// this list — nothing else about it is a cost. The audit of
    /// 2026-08-18 saw four sweeps in one session, which is what turns
    /// the per-lookup figures below into the per-session ones.
    ///
    /// Stated as exact equalities rather than as "more than before" so
    /// that the price of the two repairs above is a number in the
    /// record and not an impression: a rung added carelessly here is a
    /// syscall on every lookup for the life of the program.
    #[test]
    fn the_price_of_a_lookup_is_the_length_of_the_search_path() {
        // The shape this machine's kind of setup produces: a data home,
        // a config home, no explicit override, no XDG_DATA_DIRS.
        let dirs = theme_search_path(
            None,
            Some(PathBuf::from("/x/data")),
            Some(PathBuf::from("/x/config")),
            None,
        );
        // 2 (data) + 1 (config) + 2 x 2 (the two standard prefixes) = 7.
        // Before this branch it was 3, all of them under the old name;
        // this is 4 probes more per lookup, 16 more per session at the
        // four sweeps the audit counted — and the two of them that can
        // ever answer are the two it was missing.
        assert_eq!(dirs.len(), 7, "{dirs:?}");
        // Nothing is asked twice, which is the only way this list can
        // grow a cost that buys nothing: a duplicate is a syscall whose
        // answer is already known.
        let mut once = dirs.clone();
        once.sort();
        once.dedup();
        assert_eq!(once.len(), dirs.len(), "a directory is probed twice: {dirs:?}");

        // A packager's single prefix costs less than the default pair,
        // and the explicit override costs one probe more, deliberately.
        assert_eq!(
            theme_search_path(None, None, None, Some("/usr/share")).len(),
            2
        );
        assert_eq!(
            theme_search_path(Some(PathBuf::from("/one")), None, None, Some("/usr/share")).len(),
            3
        );
    }

    /// A theme is SAVED into the family folder and never into the old
    /// name: one directory is written, both are read.
    #[test]
    fn the_editor_saves_into_the_family_folder() {
        let Some(dir) = user_themes_dir() else { return };
        assert!(dir.ends_with("nacelle/themes"), "{dir:?}");
        assert!(
            FsThemes::new().dirs.contains(&dir),
            "what is written must be found by the walk that loads"
        );
    }

    /// THE LAST STAGE OF THE CASCADE MOVED WITH THE REST OF IT.
    ///
    /// `theme.local` is stage 5, the overlay that has the last word
    /// over everything a theme file says. It was left pointing at
    /// `~/.config/nacelle-desktop/theme.local` alone while the settings
    /// window had moved to `~/.config/nacelle`, so the file the program
    /// documents as its overlay was a file it never read — and the one
    /// it did read is one nothing writes any more.
    ///
    /// Both, new name first, for the reason every other rung has both:
    /// a machine that has the old file keeps it working, and neither is
    /// moved or deleted.
    #[test]
    fn the_overlay_asks_the_family_folder_first_and_the_old_one_after() {
        let base = PathBuf::from("/x/config");
        assert_eq!(
            overlay_candidates(&base),
            vec![
                base.join("nacelle").join("theme.local"),
                base.join("nacelle-desktop").join("theme.local"),
            ]
        );
    }

    #[test]
    fn a_theme_name_may_not_be_a_path() {
        let mut fs = FsThemes::new();
        let mut src = Sources::new();
        let mut out = Vec::new();
        assert!(fs.open("../../etc/passwd", &mut src, &mut out).is_none());
        assert!(out[0].message.contains("is not a theme name"));
    }

    /// A session that picked no theme runs the master, and the master
    /// declares three moods and a contrast variant. They used to be dropped
    /// on the floor: siblings were built from the SELECTED theme only, so
    /// the shipped alarm skin resolved to nothing and every `set_mood` on a
    /// default install answered false. Read-only on purpose — it must not
    /// move the mood out from under a test running beside it.
    #[test]
    fn the_masters_own_moods_are_siblings_a_host_can_select() {
        let _ = resolved();
        let labels = siblings();
        assert_eq!(labels.first().map(String::as_str), Some("plain"));
        for want in ["normal", "alert", "lockdown", "hc", "alert+hc"] {
            assert!(labels.iter().any(|l| l == want), "no sibling {want:?} in {labels:?}");
        }
        let rules = mood_rules();
        let alert = rules.iter().find(|r| r.name == "alert").expect("no alert rule");
        assert_eq!(alert.when, MoodWhen::SeverityAtLeast(3));
        // Image 5's mood answers to the host and to nothing else.
        let lockdown = rules.iter().find(|r| r.name == "lockdown").expect("no lockdown rule");
        assert_eq!(lockdown.when, MoodWhen::Never);
    }

    #[test]
    fn selecting_a_sibling_that_does_not_exist_is_refused_not_guessed() {
        let _ = resolved();
        assert!(!set_mood(Some("no-such-mood")));
        assert!(!set_sibling(9999));
        // the plain theme is always index 0 and always selectable
        assert!(set_sibling(0));
    }

    /// A realistic master, in the shape §5.0b prescribes. It stands in for the
    /// embedded `default.theme` so the whole pipeline is exercised end to end
    /// even while the real master is still being written.
    const MASTER: &str = r#"
[meta]
schema = 1
name = "Aurora"
name[pl] = "Zorza"
description = "Mint console, reference image 1."
family = console
strict = false

[palette]
black   = #0A100E
white   = #EAF6F1
accent  = #3FE3AE
data    = #35A7FF
neutral = #74707E

[metric]
unit_pct_h  = 0.5
unit_min_px = 4px
unit_max_px = 10px
ui_scale    = 1.0
density     = compact
density_space = 1.00
density_type  = 1.00
level.airy.space = 1.30
level.airy.type = 1.06
level.comfortable.space = 1.15
level.comfortable.type = 1.00
level.compact.space = 1.00
level.compact.type = 1.00
level.dense.space = 0.85
level.dense.type = 0.96
level.instrument.space = 0.72
level.instrument.type = 0.90

[surface]
base  = mix(@palette.black, @palette.accent, 0.06)
panel = alpha(mix(@palette.black, @palette.accent, 0.10), 0.82)
scrim = alpha(@palette.black, 0.66)

[text]
title     = lum_min(@palette.accent, 0.87)
primary   = tint(@palette.accent, 0.55)
secondary = alpha(@text.primary, 0.78)
muted     = alpha(@text.primary, 0.52)
disabled  = alpha(@text.primary, 0.30)
inverse   = contrast_on(@accent.primary, @palette.black, @palette.white)

[accent]
primary = @palette.accent
hover   = tint(@accent.primary, 0.18)
on      = contrast_on(@accent.primary, @palette.black, @palette.white)

[border]
default = alpha(@accent.primary, 0.55)
width   = @stroke.hair

[space]
0 = 0u
2 = 1u
4 = 2u
6 = 4u

[size]
md = 5.2u
xl = 8.4u

[stroke]
hair    = 0.2u
regular = 0.4u
bold    = 0.7u

[corner]
md   = 1.2u
pill = pill

[focus]
ring.width = @stroke.thin
ring.enabled = true

[panel]
content_pad   = 2.8u
content_pad_x = same_as_parent
title_h       = 5.2u
title_pad     = 0.35x @panel.title_h

[a11y]
min_hit        = 4.8u
min_hit_min_px = 24px

[state]
idle.fill  = alpha(base, 0.07)
idle.edge  = alpha(base, 0.40)
hover.fill = alpha(base, 0.22)

[glow]
alpha_scale = 1.0

[decor]
enabled = true
vignette.strength = 55%

[term]
bg     = @surface.base
fg     = @text.primary
cursor = @accent.primary
ansi = [
  #0A100E, #CD3131, #0DBC79, #E5E510,
  #2472C8, #BC3FBC, #11A8CD, #E5E5E5,
  #666666, #F14C4C, #23D18B, #F5F543,
  #3B8EEA, #D670D6, #29B8DB, #FFFFFF,
]

[data]
line = @palette.data
series = [ #3FE3AE, #35A7FF, #E8B33A, #FF7A00,
           #BC3FBC, #11A8CD, #74707E, #FF2A35 ]

[mood.alert]
palette.accent = #FF2A35
decor.enabled  = false
wash = #FF2A35 / 0.22

[variant.hc]
state.idle.edge  = alpha(base, 0.72)
glow.alpha_scale = 0.50
border.width     = @stroke.regular
decor.enabled    = false
"#;

    /// The whole pipeline on a realistic master: parse, intern, resolve,
    /// cascade a theme over it, resolve a mood sibling, and bake two screens.
    #[test]
    fn end_to_end_over_a_realistic_master() {
        let mut out = Vec::new();
        let mut src = Sources::new();
        let f = src.add("default.theme", MASTER);
        let doc = parse::parse(&mut src, f, None, &mut out);
        let show = |o: &Vec<Diagnostic>, s: &Sources| -> String {
            o.iter().map(|d| d.render(s)).collect()
        };
        assert!(out.is_empty(), "master must parse clean:\n{}", show(&out, &src));

        let mut schema = Schema::from_default(&doc, &mut out);
        let r0 = resolve::resolve_default(&schema, &mut out);
        schema.adopt_kinds(&r0.values);

        // `focus.ring.width = @stroke.thin` is a dangling reference on purpose.
        // §4.2 says warn and fall back; §4.3 says print the source line with a
        // caret under the offending span. Both, or the diagnostic is a rumour.
        assert!(schema.id("stroke.thin").is_none());
        let printed = show(&out, &src);
        assert!(printed.contains("unknown token \"stroke.thin\""), "{printed}");
        assert!(printed.contains("ring.width = @stroke.thin"), "{printed}");
        assert!(printed.contains('^'), "{printed}");
        assert!(printed.contains("default.theme:78:14"), "{printed}");
        assert!(
            out.iter().all(|d| d.message.contains("stroke.thin")),
            "the master must be clean apart from the deliberate dangler:\n{printed}"
        );
        // and it still produced a usable theme
        assert_eq!(r0.values.len(), schema.len());

        // The indexed families are contiguous, addressable per slot, and typed.
        assert_eq!(schema.family("term.ansi").map(|f| f.len()), Some(16));
        assert_eq!(schema.family("data.series").map(|f| f.len()), Some(8));
        assert_eq!(schema.kind(schema.id("term.ansi[4]").unwrap()), Kind::Color);
        // References take the kind of what they point at, after adopt_kinds.
        assert_eq!(schema.kind(schema.id("border.width").unwrap()), Kind::Scalar);
        assert_eq!(schema.kind(schema.id("term.fg").unwrap()), Kind::Color);
        // The state ladder is a template, not a value.
        assert!(schema.deferred(schema.id("state.idle.fill").unwrap()));

        // A theme that says one thing re-derives everything downstream of it.
        let g = src.add("crimson.theme", "[palette]\naccent = #FF2A35\n");
        let theme = parse::parse(&mut src, g, None, &mut out);
        let spec = cascade::cascade(
            &mut schema,
            &[cascade::Stage::Document(&theme)],
            cascade::Options::default(),
            &mut out,
        );
        let r = resolve::resolve(&schema, &spec, &mut out);
        let red = ThemeColor::from_hex("#FF2A35").unwrap().to_linear().to_oklch().h;
        for tok in ["accent.hover", "text.title", "border.default", "term.cursor"] {
            let c = r.get(schema.id(tok).unwrap()).unwrap().as_color().unwrap();
            assert!(
                (c.to_oklch().h - red).abs() < 30.0,
                "{tok} did not follow the seed: hue {} vs {red}",
                c.to_oklch().h
            );
        }
        // `.on` picked the readable side of the new chip by measurement.
        let on = r.get(schema.id("accent.on").unwrap()).unwrap().as_color().unwrap();
        let chip = r.get(schema.id("accent.primary").unwrap()).unwrap().as_color().unwrap();
        assert!(ThemeColor::wcag_contrast(on, chip) > 4.0);

        // The mood is a complete sibling, resolved separately.
        let mspec = cascade::cascade(
            &mut schema,
            &[
                cascade::Stage::Document(&theme),
                cascade::Stage::Overlay {
                    doc: &doc,
                    kind: SectionKind::Mood,
                    name: "alert".into(),
                },
            ],
            cascade::Options::default(),
            &mut out,
        );
        let mr = resolve::resolve(&schema, &mspec, &mut out);
        assert_eq!(mr.get(schema.id("decor.enabled").unwrap()), Some(&Value::Bool(false)));
        assert!(mr.wash.is_some());
        assert_eq!(r.get(schema.id("decor.enabled").unwrap()), Some(&Value::Bool(true)));

        // And two screen heights bake to two whole themes.
        let at = |h: f32, out: &mut Vec<Diagnostic>| {
            bake::bake(
                &schema,
                &r,
                &BakeInput {
                    viewport: Viewport { screen_h: h, ui_scale: 1.0 },
                    ..Default::default()
                },
                out,
            )
        };
        let lo = at(720.0, &mut out);
        let hi = at(2160.0, &mut out);
        let md = schema.id("size.md").unwrap();
        assert!((lo.px(md) - 20.8).abs() < 1e-3, "{}", lo.px(md));
        assert!((hi.px(md) - 52.0).abs() < 1e-3, "{}", hi.px(md));
        // strokes are whole physical pixels at both
        assert_eq!(lo.px(schema.id("stroke.hair").unwrap()), 1.0);
        assert_eq!(hi.px(schema.id("stroke.hair").unwrap()), 2.0);
        // the min-hit floor bites at 720p and not at 4K
        assert_eq!(lo.px(schema.id("a11y.min_hit").unwrap()), 24.0);
        assert_eq!(hi.px(schema.id("a11y.min_hit").unwrap()), 48.0);
        // sentinels folded, colours encoded, nothing NaN
        assert_eq!(lo.px(schema.id("panel.content_pad_x").unwrap()), -3.0);
        assert_eq!(lo.px(schema.id("corner.pill").unwrap()), -2.0);
        assert_eq!(
            lo.color(schema.id("palette.accent").unwrap()).to_hex(),
            "#FF2A35"
        );
        for i in 0..lo.len() {
            let id = TokenId(i as u16);
            assert!(lo.px(id).is_finite() && lo.color(id).is_finite());
        }

        // The only diagnostics are the deliberate dangling reference, once per
        // resolution, and nothing else.
        let msgs: Vec<&str> = out.iter().map(|d| d.message.as_str()).collect();
        assert!(
            msgs.iter().all(|m| m.contains("stroke.thin")),
            "unexpected diagnostics: {msgs:#?}"
        );
        assert!(!msgs.is_empty(), "the dangling reference must be reported");
    }

    // ------------------------------------- one hue, three shades (m-basic)

    /// The master with one theme file over it, resolved and baked. Every
    /// colour question below is asked of the REAL master, because the whole
    /// claim under test is a claim about the master's cascade.
    fn baked(overlay: &str) -> (Schema, resolve::Resolved, bake::ResolvedTheme) {
        let mut out = Vec::new();
        let mut src = Sources::new();
        let f = src.add("default.theme", DEFAULT_THEME);
        let doc = parse::parse(&mut src, f, None, &mut out);
        let mut schema = Schema::from_default(&doc, &mut out);
        let r0 = resolve::resolve_default(&schema, &mut out);
        schema.adopt_kinds(&r0.values);
        let spec = if overlay.is_empty() {
            schema.base_spec()
        } else {
            let g = src.add("basic.theme", overlay);
            let d = parse::parse(&mut src, g, None, &mut out);
            cascade::cascade(
                &mut schema,
                &[cascade::Stage::Document(&d)],
                cascade::Options::default(),
                &mut out,
            )
        };
        let r = resolve::resolve(&schema, &spec, &mut out);
        let t = bake::bake(&schema, &r, &BakeInput::default(), &mut out);
        (schema, r, t)
    }

    /// An edit list as a theme file: `palette.accent` is `[palette]` +
    /// `accent`, `severity.ok.text` is `[severity]` + `ok.text`. The split is
    /// at the FIRST dot, which is exactly how the master spells its own
    /// sections.
    fn as_theme_file(edits: &[edit::Edit]) -> String {
        let mut out = String::new();
        let mut section = String::new();
        for e in edits {
            let (sec, key) = e.token.split_once('.').expect("a token is section.key");
            if sec != section {
                out.push_str(&format!("[{sec}]\n"));
                section = sec.to_string();
            }
            out.push_str(&format!("{key} = {}\n", e.value));
        }
        out
    }

    fn lch(c: ThemeColor) -> color::Oklch {
        c.to_linear().to_oklch()
    }

    /// The shortest way round the circle, in degrees.
    fn hue_gap(a: f32, b: f32) -> f32 {
        let d = (a - b).rem_euclid(360.0);
        d.min(360.0 - d)
    }

    /// How far a bed stands off pure black, as WCAG 2.x contrast.
    ///
    /// WHY THIS AND NOT OKLab L, which is what the rest of this file
    /// measures shades with. OKLab L is a good ruler for the DIFFERENCE
    /// between two beds and a bad one for "is this bed black", because it
    /// has no opinion about where black stops: L 0.115 and L 0.178 are
    /// 0.063 apart, an honest step by that ruler, and on the screen their
    /// brightest channels are the sRGB codes 6 and 19 — two blacks.
    /// Contrast against black
    /// is `(Y + 0.05)/0.05`, and the 0.05 flare pedestal is exactly the
    /// term that makes the ratio move fastest in the region OKLab L
    /// flattens. 1.00 IS black; anything above it is how far from black
    /// the bed got.
    ///
    /// ALPHA IS NOT IN IT, and does not need to be. A band is painted over
    /// the window body and is LIGHTER than it, so compositing can only
    /// move the pixel from the body's reading toward the band's — the body
    /// clears this gate itself, so the composite clears it too.
    fn off_black(c: ThemeColor) -> f32 {
        let black = ThemeColor::from_hex("#000000").unwrap();
        ThemeColor::wcag_contrast(c.to_linear(), black.to_linear())
    }

    /// The floor a COLUMN's bed must clear to be a shade and not a black.
    ///
    /// WHERE THE NUMBER COMES FROM, and it is not a taste. It is set
    /// between two of the master's own rungs, measured: `@surface.panel`
    /// (the window body — the darkest bed this master ever stands a
    /// control on) reads 1.26, and `@surface.base` (the desktop field,
    /// one of the two rungs the columns used to point at) reads 1.12.
    /// Anything a whole column of interface is painted with belongs on the
    /// body's side of that line. It is deliberately NOT a floor for every
    /// bed in the file: `@surface.sunken` is a progress trough and a field
    /// interior, a few hundred pixels of recess inside a lit control, and
    /// it is under this line on purpose.
    const NOT_BLACK: f32 = 1.15;

    /// ŻYCZENIE 1, RE-ARGUED THREE TIMES FROM TWO SCREENSHOTS AND A
    /// MOCK-UP. The settings window's NAVIGATION is ONE bed standing one
    /// step off the window body the page keeps — and neither of them is
    /// black.
    ///
    /// WHAT CHANGED ON 2026-08-18, in two moves. First the owner looked at
    /// the three-shade staircase this test used to demand and rejected it:
    /// "mają być po całości i obie w jednakowym kolorze, tym w środkowej
    /// kolumnie". Two adjacent strips of one navigation at two shades read
    /// as a seam through one object, so the step gate between the rail and
    /// the sub-page column became its opposite — the SAME COLOUR, compared
    /// as colours and not as token names. Then the two columns became ONE:
    /// a section's pages unfold under it, indented, and
    /// `component.settings.sub_fill` went with the column it bedded. What
    /// this test asserts about that is that the name is GONE and not merely
    /// unused — an equal-colour claim about a column nothing draws is a
    /// claim that cannot fail, and a token nothing can paint reads as a knob.
    ///
    /// WHAT THE OWNER SAW THE DAY BEFORE, and what this test could not see
    /// before that. The two navigation columns were pointed at
    /// `@surface.void` and `@surface.base`, and by the only ruler this test
    /// carried — OKLab L, two even steps of about 0.06 — they were three
    /// shades. Encoded to the swapchain their brightest channel reads 6, 19
    /// and 32 of 255: two black stripes beside a coloured page. So a step
    /// gate alone was never the claim. The claim is that no band is BLACK,
    /// and [`off_black`] is the ruler for that half of it.
    ///
    /// AND ONE ANCHOR, NOT TWO. Both beds are the window body — the page's
    /// is it, and the navigation's is it lifted once by `settings.band_lift`
    /// — so whatever moves the body — a theme, BASIC's sliders, or the
    /// editor's BACKGROUND section writing `component.panel.fill` as a
    /// literal — moves both together. The 2026-08-17 screenshot's second
    /// fault was exactly this: the page followed the BACKGROUND sliders and
    /// the two pinned columns did not.
    ///
    /// THE PAGE'S NAME IS DECLARED AND IS THE SENTINEL. Both columns are
    /// named so a theme may bed either of them; the master leaves the
    /// page's at `none`, because the body is already standing there and
    /// `panel.fill` is translucent — a second coat composes its alpha twice.
    ///
    /// The folded case is in the same claim: with no columns there is
    /// nothing to bed, and the interior is the body — the bed the page was
    /// on all along.
    #[test]
    fn the_navigation_band_is_one_bed_over_the_body() {
        let (schema, _, t) = baked("");
        let raw = |n: &str| t.color(schema.id(n).expect(n));
        let band = |n: &str| lch(raw(n));
        let rail = band("component.settings.rail_fill");
        let page = band("component.panel.fill");

        // THE SECOND COLUMN'S BED IS NOT MERELY UNUSED, IT IS GONE. The
        // window has one navigation column since 2026-08-18, so a theme
        // that set this would be turning a knob attached to nothing.
        assert!(
            schema.id("component.settings.sub_fill").is_none(),
            "`component.settings.sub_fill` still has a name and no column to bed"
        );

        // THE PAGE'S BED IS NAMED, and what it is set to is the sentinel:
        // the token exists so a theme may bed the page, and the master
        // declines because the window body already is that bed. `none`
        // answers `color()` with alpha 0, which is how every reader in this
        // toolkit spells "there is nothing to draw here"
        // (`tests/sentinel_none_colour.rs`).
        let page_tok =
            schema.id("component.settings.page_fill").expect("the page's bed has no name");
        assert_eq!(
            t.color(page_tok).a,
            0.0,
            "the master bedded the page a second time; the body's `panel.fill` is it"
        );

        // ONE HUE — both sit on @surface.hue, which is @hue.accent, and so
        // does the line that brackets an unfolded section's pages.
        let accent = lch(t.color(schema.id("palette.accent").unwrap()));
        let guide = band("component.settings.rail_guide");
        for (name, c) in [("rail", rail), ("page", page), ("guide", guide)] {
            assert!(
                hue_gap(c.h, accent.h) < 2.0,
                "the {name} band left the interface's hue: {} vs {}",
                c.h,
                accent.h
            );
        }

        // AND THE NAVIGATION STANDS OFF THE PAGE. One step, not none: a bed
        // the eye cannot find is the same fault as a seam through it.
        assert!(
            page.l < rail.l,
            "the navigation did not climb off the body: {} {}",
            page.l,
            rail.l
        );
        assert!(
            rail.l - page.l > 0.03,
            "the navigation and the page are too close to tell apart: {}",
            rail.l - page.l
        );

        // THE GUIDE IS A MARK ON THAT BED AND NOT A SECOND BED. It has to
        // be visible against the rail it is drawn on — a bracket nobody can
        // see brackets nothing — and it has to be a LINE colour, which here
        // means it carries alpha of its own rather than the body's: a
        // hairline that inherited the window's translucency would disappear
        // exactly where the window is most glass.
        let g_raw = raw("component.settings.rail_guide");
        assert!(
            (guide.l - rail.l).abs() > 0.05,
            "the guide and the rail it is drawn on are one shade: {} vs {}",
            guide.l,
            rail.l
        );
        assert!(g_raw.a > 0.0, "the guide is invisible: alpha {}", g_raw.a);

        // AND NEITHER OF THEM IS BLACK. This is the assertion the owner's
        // screenshot is about, and the one the old master fails.
        for (name, c) in [
            ("rail", raw("component.settings.rail_fill")),
            ("page", raw("component.panel.fill")),
        ] {
            assert!(
                off_black(c) >= NOT_BLACK,
                "the {name} band reads {} against pure black — a black stripe, not a shade",
                off_black(c)
            );
        }
        // The gate is the one that catches the defect and not a gate that
        // everything passes: the two rungs the bands used to be pinned to
        // are BELOW it, measured here rather than asserted from memory.
        for name in ["surface.void", "surface.base"] {
            assert!(
                off_black(raw(name)) < NOT_BLACK,
                "`{name}` now clears the black floor, so this gate no longer \
                 separates a column's bed from the nothing behind the picture"
            );
        }

        // THE BODY'S RUNG IS TRANSLUCENT, which is why the page's own name
        // stays at the sentinel: two coats of it are not one coat. It is
        // also what carries a blurred or translucent window ACROSS both
        // columns — the painted band inherits this alpha through `lum()`.
        assert!(raw("component.panel.fill").a < 1.0, "the body's rung went opaque");
        assert!(
            (raw("component.settings.rail_fill").a - raw("component.panel.fill").a).abs()
                < 1e-4,
            "the rail stopped carrying the body's alpha; a translucent window would go \
             opaque under its own navigation"
        );

        // And a theme moves them: the seed alone re-skins both...
        let (s2, _, t2) = baked("[palette]\naccent = #FF2A35\n");
        let red = ThemeColor::from_hex("#FF2A35").unwrap().to_linear().to_oklch().h;
        for name in ["component.settings.rail_fill", "component.panel.fill"] {
            let c = lch(t2.color(s2.id(name).unwrap()));
            assert!(hue_gap(c.h, red) < 3.0, "{name} did not follow the seed: {}", c.h);
        }
        // ...and the rail can be re-pointed on its own, which is the whole
        // reason these are named tokens instead of two rungs named in Rust.
        let (s3, _, t3) = baked("[component]\nsettings.rail_fill = @surface.raised\n");
        let one = lch(t3.color(s3.id("component.settings.rail_fill").unwrap()));
        let other = lch(t3.color(s3.id("component.panel.fill").unwrap()));
        assert!(one.l > other.l, "the theme could not lift the navigation alone");

        // ONE ANCHOR: MOVE THE BODY AND BOTH FOLLOW. This is the editor's
        // own case written as a theme — `edit::glass_edits`' SOLID writes
        // `component.panel.fill` as exactly such a literal — and it is the
        // divergence the owner photographed: the page went with the sliders
        // and the navigation stayed behind.
        let (s5, _, t5) =
            baked("[component]\npanel.fill = oklch(0.4200, 0.0400, 292.00 / 0.820)\n");
        let moved = |n: &str| t5.color(s5.id(n).expect(n));
        let body = lch(moved("component.panel.fill"));
        assert!((body.l - 0.42).abs() < 0.01, "the body did not take the literal: {}", body.l);
        let r5 = lch(moved("component.settings.rail_fill"));
        assert!(
            hue_gap(r5.h, body.h) < 2.0,
            "the rail kept the theme's old hue while the body took a new one: {} vs {}",
            r5.h,
            body.h
        );
        assert!(
            r5.l - body.l > 0.03,
            "the navigation collapsed onto the body it follows: {} {}",
            body.l,
            r5.l
        );

        // ONE NUMBER STATES THE STEP — its size AND its direction. At 1.0
        // the whole interior is one colour, which is the honest way for a
        // theme to say "no bands"; under 1.0 the step runs the other way,
        // which is the escape a LIGHT theme takes instead of a second set
        // of tokens.
        let (s6, _, t6) = baked("[settings]\nband_lift = 1.0\n");
        let flat = |n: &str| lch(t6.color(s6.id(n).expect(n)));
        let (fr, fp) =
            (flat("component.settings.rail_fill"), flat("component.panel.fill"));
        assert!(
            (fr.l - fp.l).abs() < 1e-3,
            "band_lift = 1.0 left a step behind: {} {}",
            fp.l,
            fr.l
        );
        let (s7, _, t7) = baked("[settings]\nband_lift = 0.85\n");
        let down = |n: &str| lch(t7.color(s7.id(n).expect(n)));
        assert!(
            down("component.settings.rail_fill").l < down("component.panel.fill").l,
            "a lift under 1.0 did not put the navigation bed under the page"
        );

        // AND THE PAGE CAN BE BEDDED AFTER ALL, by the theme that wants it:
        // the name is not decoration. What the master declines, a file may
        // ask for.
        let (s8, _, t8) = baked("[component]\nsettings.page_fill = @surface.sunken\n");
        let bedded = t8.color(s8.id("component.settings.page_fill").unwrap());
        assert!(bedded.a > 0.0, "a theme could not bed the page at all");
    }

    /// ŻYCZENIE 2b, MEASURED. After BASIC's HUE slider has moved, a column's
    /// container and a control's plate carry THE SAME hue and two clearly
    /// different shades — checked at four positions of the slider, not one.
    ///
    /// The second half of the claim is the exception the owner carved out:
    /// the severity roles keep their own hues apart, because those carry
    /// MEANING and not style.
    #[test]
    fn a_basic_hue_move_gives_one_hue_to_the_chrome_and_keeps_severity_apart() {
        // The seeds, off the master itself — BASIC is relative, so this is
        // what it is relative to.
        let (s0, _, t0) = baked("");
        let seed_of = |n: &str| lch(t0.color(s0.id(n).expect(n)));
        let seeds = edit::ToneSeeds {
            accent: seed_of("palette.accent"),
            black: seed_of("palette.black"),
            white: seed_of("palette.white"),
            neutral: seed_of("palette.neutral"),
            surface_lift: t0.px(s0.id("surface.lift").unwrap()),
            text_lift: t0.px(s0.id("text.lift").unwrap()),
        };

        for turn in [0.0f32, 47.0, 133.0, 251.0] {
            let file = as_theme_file(&edit::tone_edits(
                edit::Scope::Theme,
                &seeds,
                edit::Tone { hue_deg: turn, ..edit::Tone::NEUTRAL },
            ));
            let (schema, r, t) = baked(&file);

            // The CONTAINER: the settings navigation's bed.
            let container = lch(t.color(schema.id("component.settings.rail_fill").unwrap()));
            // The PLATE: what the renderer actually lays under a button —
            // the class ladder's idle fill, whose colour is the button's
            // class base (`class.button = @accent.primary`).
            let button = r
                .class_ids
                .iter()
                .position(|&id| schema.name(id) == "class.button")
                .unwrap() as u16;
            let plate = lch(t.class_state(button, parse::State::Idle).fill);

            // ONE BARWA.
            assert!(
                hue_gap(container.h, plate.h) < 2.0,
                "at {turn} deg the column and the button are two colours: {} vs {}",
                container.h,
                plate.h
            );
            // RÓŻNE ODCIENIE — both lightness and chroma, well clear of any
            // rounding, so the two never read as one material.
            assert!(
                (plate.l - container.l).abs() > 0.20,
                "at {turn} deg the column and the button share a lightness: {} vs {}",
                container.l,
                plate.l
            );
            assert!(
                (plate.c - container.c).abs() > 0.05,
                "at {turn} deg the column and the button share a chroma: {} vs {}",
                container.c,
                plate.c
            );
            // And the interface really did turn: the whole family moved by
            // the slider's own degrees.
            assert!(
                hue_gap(container.h, seeds.accent.h + turn) < 2.0,
                "the interface did not turn {turn} deg: {}",
                container.h
            );

            // AND NEITHER COLUMN IS BLACK, at any position of the wheel. A
            // HUE move is a rotation and rotations do not darken, so a band
            // that comes out black here came out black from the master —
            // which is the owner's screenshot, and which this test used to
            // walk straight past because a ROTATION check cannot see a
            // lightness.
            //
            // GATED ON THE LIGHTNESS SLIDER STANDING STILL, which it does
            // here (`Tone::NEUTRAL` but for the hue). Dragged to its floor
            // BASIC really can take the whole interface down to a black
            // theme, and a black theme's beds are allowed to be black —
            // "not black when the theme is not black" is the claim, not
            // "never black".
            for name in ["component.settings.rail_fill", "component.panel.fill"] {
                let bed = t.color(schema.id(name).unwrap());
                assert!(
                    off_black(bed) >= NOT_BLACK,
                    "at {turn} deg `{name}` reads {} against pure black — the columns \
                     turned but one of them is a black stripe",
                    off_black(bed)
                );
            }
            // Still one step off the page, wherever the wheel stopped: the
            // navigation's bed comes out of one expression off the body's
            // own anchor, so a turn cannot flatten it onto the page.
            let bed = |n: &str| lch(t.color(schema.id(n).unwrap()));
            let (page, rail) =
                (bed("component.panel.fill"), bed("component.settings.rail_fill"));
            assert!(
                rail.l - page.l > 0.03,
                "at {turn} deg the navigation flattened onto the page: {} {}",
                page.l,
                rail.l
            );

            // THE EXCEPTION, AND IT GOT STRONGER ON 2026-08-18. It used to
            // read "severity is a rotation, not a flattening": the roles
            // turned the whole way with the interface and the test only
            // asked that they stay APART. Apart they were — green `ok` sat
            // in red and red `critical` in blue, exactly as far from each
            // other as before, and the check could not see it.
            //
            // The claim now is the one that matters: a role never leaves
            // its own hue at all. Its LEAN is the theme's, capped by the
            // theme's own `severity.pull_clamp`, and the ceiling below is
            // read from the token rather than written here — a literal 7
            // would go on passing if the master changed its mind.
            //
            // THE CANONICAL HUE IS THE THEME'S OWN ANSWER, asked for with
            // `severity.pull = 0` — which the master documents as "0 =
            // never move" — rather than a literal copied into this file.
            // The frozen bake is checked to be genuinely frozen just below,
            // so it cannot quietly become a second copy of the pull.
            let cap = t.px(schema.id("severity.pull_clamp").unwrap());
            assert!(cap > 0.0, "the master stopped capping the severity lean");
            let (sf, _, frozen) = baked(&format!("{file}\n[severity]\npull = 0.0\n"));
            let sev = |n: &str| lch(t.color(schema.id(n).unwrap()));
            for name in ROLES_THAT_MEAN_SOMETHING {
                let canonical = lch(frozen.color(sf.id(name).unwrap()));
                let moved = hue_gap(sev(name).h, canonical.h);
                assert!(
                    moved <= cap + 0.5,
                    "at {turn} deg `{name}` left its own hue by {moved} deg, past the \
                     theme's own cap of {cap}"
                );
            }
        }

        // AND THE FROZEN READING IS REALLY FROZEN. Two accents a third of
        // the wheel apart must give a role with `pull = 0` the SAME hue, or
        // the reference the loop just measured against was moving too and
        // the whole check was comparing a thing to itself.
        let frozen_at = |turn: f32| {
            let file = as_theme_file(&edit::tone_edits(
                edit::Scope::Theme,
                &seeds,
                edit::Tone { hue_deg: turn, ..edit::Tone::NEUTRAL },
            ));
            let (sf, _, tf) = baked(&format!("{file}\n[severity]\npull = 0.0\n"));
            ROLES_THAT_MEAN_SOMETHING
                .map(|n| lch(tf.color(sf.id(n).unwrap())).h)
        };
        let (a, b) = (frozen_at(0.0), frozen_at(120.0));
        for (i, name) in ROLES_THAT_MEAN_SOMETHING.iter().enumerate() {
            assert!(
                hue_gap(a[i], b[i]) < 0.05,
                "`{name}` moved with the accent at pull = 0: {} vs {}",
                a[i],
                b[i]
            );
        }
    }

    /// ZGŁOSZENIE 5, THE OWNER'S OWN SENTENCE: "in BASIC I pick only the
    /// base colour and the other colours adapt to it".
    ///
    /// The severity roles are the family that could not adapt, because
    /// nothing carried them: they were six frozen literals, so a
    /// re-coloured theme left them where the master put them, and the only
    /// machine that ever moved them was the editor, turning them the whole
    /// way and destroying what they mean. They ADAPT now — `toward()` in
    /// the master, `@severity.pull` degrees of lean — and this is the test
    /// of the adaptation itself, with no editor anywhere near it: two
    /// theme files that differ in ONE LINE, the accent.
    #[test]
    fn a_theme_that_changes_its_accent_carries_the_severity_roles_with_it() {
        let at = |accent: &str| {
            let (sc, _, t) = baked(&format!("[palette]\naccent = {accent}\n"));
            ROLES_THAT_MEAN_SOMETHING.map(|n| lch(t.color(sc.id(n).unwrap())).h)
        };
        // The master's mint, and a red a third of the wheel away.
        let mint = at("oklch(0.820, 0.153, 166.5)");
        let red = at("oklch(0.680, 0.190, 29.0)");
        for (i, name) in ROLES_THAT_MEAN_SOMETHING.iter().enumerate() {
            assert!(
                hue_gap(mint[i], red[i]) > 1.0,
                "`{name}` did not move at all when the theme changed colour \
                 ({} under mint, {} under red) — the role is deaf to the palette",
                mint[i],
                red[i]
            );
        }
    }

    /// …AND THE ADAPTATION IS A LEAN, WHICH IS THE OTHER HALF. A role that
    /// followed the accent all the way would "adapt" too, and it is exactly
    /// what the editor used to do: mint -> red sent green `ok` to 10.5 deg
    /// and red `critical` to 249.5, so a successful job was drawn in the
    /// colour of an alarm and the alarm in the colour of a hyperlink. The
    /// bands below are the CONVENTION each role exists to speak — green,
    /// azure, amber, red, amber, violet — and no accent may take a role out
    /// of its own band.
    #[test]
    fn no_accent_can_take_a_role_out_of_the_band_that_gives_it_its_meaning() {
        // (role, the arc of the hue circle the convention lives in)
        let bands: [(&str, f32, f32); 6] = [
            ("severity.ok.text", 120.0, 175.0),        // green
            ("severity.info.text", 210.0, 260.0),      // azure
            ("severity.warning.text", 55.0, 100.0),    // amber
            ("severity.critical.text", 5.0, 50.0),     // red
            ("severity.contained.text", 65.0, 115.0),  // the dimmer amber
            ("severity.unknown.text", 285.0, 335.0),   // violet
        ];
        for accent_h in [0.0f32, 60.0, 120.0, 180.0, 240.0, 300.0] {
            let (sc, _, t) =
                baked(&format!("[palette]\naccent = oklch(0.760, 0.170, {accent_h})\n"));
            for (name, lo, hi) in bands {
                let h = lch(t.color(sc.id(name).unwrap())).h;
                assert!(
                    h >= lo && h <= hi,
                    "with the accent at {accent_h} deg, `{name}` came out at {h} deg — \
                     outside {lo}..{hi}, which is the convention it exists to speak"
                );
            }
        }
    }

    /// The three GROUNDS of the palette, and the owner's most visible
    /// symptom: "I change the colour and the background stays as it was".
    ///
    /// `palette.black` and `palette.white` are the only targets `shade()`
    /// and `tint()` have, and the master reaches for `shade()` ten times —
    /// every badge interior, every sunk plate. Both are hex literals on the
    /// ACCENT's hue (measured h 172.6 and 169.2 against the accent's 166.5),
    /// so a theme that re-colours without them keeps its old bed hue and
    /// drags every shaded thing back toward it. Nothing in the cascade can
    /// carry them — §5.2 keeps them literal so `shade()`/`tint()` cannot
    /// close a cycle — so the EDITOR writes them, and this is that write
    /// arriving on the screen.
    #[test]
    fn the_editors_move_carries_the_grounds_the_cascade_cannot() {
        let (s0, _, t0) = baked("");
        let seed_of = |n: &str| lch(t0.color(s0.id(n).expect(n)));
        let seeds = edit::ToneSeeds {
            accent: seed_of("palette.accent"),
            black: seed_of("palette.black"),
            white: seed_of("palette.white"),
            neutral: seed_of("palette.neutral"),
            surface_lift: t0.px(s0.id("surface.lift").unwrap()),
            text_lift: t0.px(s0.id("text.lift").unwrap()),
        };
        let turn = 150.0f32;
        let file = as_theme_file(&edit::tone_edits(
            edit::Scope::Theme,
            &seeds,
            edit::Tone { hue_deg: turn, ..edit::Tone::NEUTRAL },
        ));
        let (sc, _, t) = baked(&file);
        for name in ["palette.black", "palette.white", "palette.neutral"] {
            let before = seed_of(name).h;
            let after = lch(t.color(sc.id(name).unwrap())).h;
            assert!(
                hue_gap(after, (before + turn).rem_euclid(360.0)) < 2.0,
                "`{name}` stayed behind the move: {before} -> {after}, wanted {}",
                (before + turn).rem_euclid(360.0)
            );
        }
        // AND IT REACHES THE PICTURE. A badge's interior is
        // `alpha(shade(@severity.<r>.text, 0.78), 0.88)` — three quarters of
        // the way to `palette.black` — so it is the shortest road from that
        // token to something a person looks at.
        let pill = |t: &bake::ResolvedTheme, sc: &Schema| {
            lch(t.color(sc.id("severity.critical.fill").unwrap())).h
        };
        assert!(
            hue_gap(pill(&t, &sc), pill(&t0, &s0)) > 20.0,
            "the badge interiors kept the old bed's hue: {} -> {}",
            pill(&t0, &s0),
            pill(&t, &sc)
        );
        // The two POLES keep their lightness through all of it: they are the
        // theme's polarity, not a shade of the accent.
        for name in ["palette.black", "palette.white"] {
            assert!(
                (lch(t.color(sc.id(name).unwrap())).l - seed_of(name).l).abs() < 0.005,
                "`{name}` changed the theme's polarity on a hue move"
            );
        }
    }

    /// The master's own promise about a PINNED role, kept by construction:
    /// "a theme that writes its own `severity.<r>.text` pins that role and
    /// the pull no longer touches it — that is exactly image 4, everything
    /// red by derivation with an amber `contained` written out on one line".
    ///
    /// A pull applied in `bake.rs` could not have kept it: the baker has no
    /// provenance and cannot tell the master's own literal from a theme's.
    /// Written as an expression the theme OVERRIDES, the pin needs no
    /// machinery at all — and the editor's own per-role control (ADVANCED's
    /// SEVERITY group) rides the same road, so the colour a person picks
    /// there is the colour that gets drawn.
    #[test]
    fn a_role_a_theme_writes_out_is_the_colour_it_wrote() {
        let (sc, _, t) = baked(
            "[palette]\naccent = oklch(0.680, 0.190, 29.0)\n\
             [severity]\ncontained.text = oklch(0.700, 0.105, 92)\n",
        );
        let pinned = lch(t.color(sc.id("severity.contained.text").unwrap()));
        assert!(
            hue_gap(pinned.h, 92.0) < 0.5,
            "the pinned amber was pulled anyway: 92 -> {}",
            pinned.h
        );
        // …while the role beside it, left to the theme, leaned toward the
        // new accent. Without this half the test would pass on a build where
        // nothing pulls anything.
        let (s0, _, t0) = baked("");
        let free = lch(t.color(sc.id("severity.warning.text").unwrap()));
        let was = lch(t0.color(s0.id("severity.warning.text").unwrap()));
        assert!(
            hue_gap(free.h, was.h) > 1.0,
            "no role moved at all, so the pin proved nothing: {} vs {}",
            was.h,
            free.h
        );
    }

    /// The six roles whose hue IS their meaning: green success, azure
    /// notice, amber warning, red alarm, the dimmer amber of a contained
    /// alarm, and the violet no other role uses. `offline` is not among
    /// them — it is the hue-free anchor and rides `palette.neutral`.
    const ROLES_THAT_MEAN_SOMETHING: [&str; 6] = [
        "severity.ok.text",
        "severity.info.text",
        "severity.warning.text",
        "severity.critical.text",
        "severity.contained.text",
        "severity.unknown.text",
    ];

    /// A LOCALISED key is a different string in the same token's slot, and
    /// `KeyVal::token` deliberately does not spell the locale — `meta.name`
    /// and `meta.name[pl]` answer to the same name. So the map of "where
    /// this token stands" would take whichever came LAST in the file, and
    /// with the translation last that is the Polish line. Then a save of
    /// `meta.name` would rewrite the translation and leave the name it was
    /// asked to change exactly where it was: two wrongs in one swap.
    ///
    /// Pure text, no engine: `patch_theme_text` reads a `Sources` it makes
    /// itself, which is what lets this be a unit test rather than a process.
    #[test]
    fn a_translation_is_not_the_slot_a_save_writes_into() {
        let base = "[meta]\nname = \"Base\"\nname[pl] = \"Polski\"\n";
        let out = patch_theme_text(
            "t",
            base,
            &[edit::Edit { token: "meta.name", value: "\"Nowy\"".to_string() }],
        );
        assert!(out.contains("name = \"Nowy\""), "the name the save named did not change:\n{out}");
        assert!(
            out.contains("name[pl] = \"Polski\""),
            "the save wrote the new name into the Polish translation:\n{out}"
        );
    }

    /// `@include` is not followed here (`parse_text` hands the parser no
    /// base directory), so the tokens it carries are invisible to the patch
    /// and read as missing. Appending them is the right answer — last
    /// declaration wins the cascade — and the include line itself has to
    /// survive, because rewriting a file must never cost it a directive.
    #[test]
    fn an_include_survives_a_save_and_what_it_carries_is_appended() {
        let base = "[palette]\naccent = #FF2A35\n@include \"reszta.theme\"\n";
        let out = patch_theme_text(
            "t",
            base,
            &[edit::Edit { token: "corner.mode", value: "chamfer".to_string() }],
        );
        assert!(
            out.contains("@include \"reszta.theme\""),
            "the include directive was dropped by the save:\n{out}"
        );
        assert!(out.contains("mode = chamfer"), "the edit was lost:\n{out}");
        assert!(
            out.find("mode = chamfer") > out.find("@include"),
            "the appended token landed before the include, which would then \
             overwrite it:\n{out}"
        );
    }

    #[test]
    fn the_draw_colour_api_still_works_because_the_program_calls_it() {
        // `theme::Color` is [`color::Color`] now, and the five methods the
        // draw calls were built on are unchanged.
        let c = Color::rgb8(170, 207, 209);
        assert_eq!(c.to_array()[3], 1.0);
        assert!(Color::from_hex("#05080d").is_some());
        assert_eq!(c.alpha(0.5).a, 0.5);
        let _ = c.dim(0.5);
    }
}
