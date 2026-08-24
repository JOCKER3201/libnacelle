//! The shared drawing vocabulary widgets are built from.
//!
//! These are the shapes that kept being written out by hand in every
//! widget: a block centred in its panel, rows of label and value, a
//! framed meter, a matrix of lit cells. Having them here means a widget
//! is a short description of WHAT to show rather than a page of layout
//! arithmetic — and it is the vocabulary the Rhai script renderer
//! ([`crate::script`]) interprets its elements into.
//!
//! Every colour and metric below comes from the theme: this file is the
//! single place where the look of every board panel is decided, so a
//! literal here would be a literal everywhere.

use crate::font::{Figures, FontSystem, FONT_UI};
use crate::num;
use crate::theme::{self, Color, TokenId};
use crate::view::paint;
use crate::view::surface::{CtxSurface, Surface};
use crate::view::table_model::TableModel;
use crate::{Ctx, Rect};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::OnceLock;

/// Token id resolved once by name; MISSING degrades through the engine's
/// per-kind fallback rather than panicking.
fn tok(cell: &'static OnceLock<TokenId>, name: &'static str) -> TokenId {
    *cell.get_or_init(|| theme::id(name).unwrap_or(TokenId::MISSING))
}

/// A colour token, delivered in the `Color` the draw calls take.
fn col(cell: &'static OnceLock<TokenId>, name: &'static str) -> Color {
    let c = theme::resolved().color(tok(cell, name));
    Color { r: c.r, g: c.g, b: c.b, a: c.a }
}

/// Said once, then quiet: the widgets run every frame, and the second
/// copy of a diagnostic is already noise.
pub(crate) fn warn_once(key: &str, msg: &str) {
    thread_local! {
        static SAID: RefCell<HashSet<String>> = RefCell::new(HashSet::new());
    }
    SAID.with(|s| {
        if s.borrow_mut().insert(key.to_string()) {
            eprintln!("nacelle-desktop: {msg}");
        }
    });
}

/// The word an enum token currently resolves to, memoised per (epoch,
/// token, index) so a draw loop pays the engine lock once per distinct
/// value, not per frame.
///
/// The EPOCH belongs in that key and its absence was a live bug. An index
/// only names a word against the schema it was interned in, and every
/// `theme::load_with` builds the schema afresh: an OPEN word set — which
/// is what every `*_role` binding is — renumbers, so index 1 means the
/// first theme's word under the first schema and the second theme's word
/// under the second. Without the epoch, swapping themes in a running
/// program left every role binding answering the PREVIOUS theme's role,
/// for the life of the thread.
pub(crate) fn theme_word(token: TokenId) -> String {
    word_of(token)
}

/// The same word, LENT rather than handed over.
///
/// `theme_word` clones out of the cache, which is right for a caller that
/// keeps the string — and wrong for the ones that only compare it. Motion
/// asks for the easing word on every frame of every fade, so a clone there
/// is an allocation per control per frame; borrowing inside the map's own
/// borrow costs nothing and cannot outlive it.
pub(crate) fn with_theme_word<R>(token: TokenId, f: impl FnOnce(&str) -> R) -> R {
    let i = theme::resolved().enum_of(token);
    let epoch = theme::epoch();
    WORDS.with(|w| {
        let mut w = w.borrow_mut();
        let word = w
            .entry((epoch, token.index(), i))
            .or_insert_with(|| theme::enum_word_of(token).unwrap_or_default());
        f(word)
    })
}

fn word_of(token: TokenId) -> String {
    with_theme_word(token, str::to_string)
}

// The one word memo both readers above share. It was written out twice —
// two maps holding the same triples, filled from the same engine call —
// and a test that stated a word for one of them would have been telling
// half the file.
thread_local! {
    static WORDS: RefCell<HashMap<(u32, usize, u16), String>> = RefCell::new(HashMap::new());
}

/// States an enum token's word for the CALLING THREAD only — the seam a
/// test drives a theme's `case` through, and the twin of
/// [`seed_theme_text`]. Same argument: the memo is a thread_local, one
/// `#[test]` is one thread, and publishing a theme would decide what
/// every test running beside it draws from.
#[cfg(test)]
pub(crate) fn seed_theme_word(name: &str, word: &str) {
    let Some(id) = theme::id(name) else { return };
    let i = theme::resolved().enum_of(id);
    let epoch = theme::epoch();
    WORDS.with(|w| w.borrow_mut().insert((epoch, id.index(), i), word.to_string()));
}

// ---------------------------------------------------------------- severity
//
// §5.10's closed set, in the master's declaration order. A severity is an
// INDEX into this set — never a colour: the script (or plugin) judges the
// data, the theme decides what the judgement looks like.

/// The severity roles the master declares, in declaration order.
pub const SEVERITY_ROLES: [&str; 7] =
    ["ok", "info", "warning", "critical", "contained", "offline", "unknown"];

/// An index into [`SEVERITY_ROLES`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Sev(pub u16);

/// The severity for a name from the closed set — `None` for a word outside
/// it, so the CALLER decides the fallback ([`sev_fallback`], never `ok`).
pub fn sev_of(name: &str) -> Option<Sev> {
    SEVERITY_ROLES
        .iter()
        .position(|s| *s == name)
        .map(|i| Sev(i as u16))
}

/// What an unrecognised severity resolves to: `script.severity_fallback`,
/// which the master pins to `unknown` and §5.10 forbids ever being `ok`.
///
/// The theme's word is the whole answer. When the key names something
/// the closed set does not hold there is no answer to give, so the LAST
/// role is drawn — §5.10 puts `unknown` there for exactly this — and the
/// key is named out loud rather than papered over: a severity picks the
/// colour of a reading, and a wrong one that never says so is the kind
/// of finished-looking mistake the audit exists to catch. Counted off
/// the set rather than written as a number, so a master that adds a rung
/// does not leave a stale index behind.
pub fn sev_fallback() -> Sev {
    static FB: OnceLock<TokenId> = OnceLock::new();
    let word = word_of(tok(&FB, "script.severity_fallback"));
    sev_of(&word).unwrap_or_else(|| unnamed_severity(&word))
}

/// The rung an unnameable `script.severity_fallback` lands on, and the
/// warning that says so. Shared with [`crate::view::paint`], which asks
/// the same question through the ABI and must not answer it differently.
pub(crate) fn unnamed_severity(word: &str) -> Sev {
    let last = SEVERITY_ROLES.len() - 1;
    warn_once(
        "severity:script.severity_fallback",
        &format!(
            "\"script.severity_fallback\" holds \"{word}\", which §5.10's closed set does not \
             name — \"{}\" is drawn instead",
            SEVERITY_ROLES[last]
        ),
    );
    Sev(last as u16)
}

/// The `text` token id of each severity role, resolved once per role.
///
/// The other four members (`edge`, `fill`, `on`, `badge_style`) are read
/// by [`crate::view::paint`], which names them by string because it has
/// to work on the far side of the plugin ABI, where a `TokenId` means
/// nothing. Only the ink is asked for often enough on the host to be
/// worth a static.
fn sev_tok(s: Sev) -> TokenId {
    static TOKS: OnceLock<Vec<TokenId>> = OnceLock::new();
    let all = TOKS.get_or_init(|| {
        SEVERITY_ROLES
            .iter()
            .map(|n| theme::id(&format!("severity.{n}.text")).unwrap_or(TokenId::MISSING))
            .collect()
    });
    all[(s.0 as usize).min(all.len() - 1)]
}

/// The ink a severity writes in — the label, the value, the status word.
pub fn sev_text(s: Sev) -> Color {
    theme::resolved().color(sev_tok(s))
}

// --------------------------------------------------------------------- case
//
// One enum and one applier, for the whole toolkit and for every widget on
// the far side of the ABI. Before this there were five copies of
//
//     match word { "none" => …, "lower" => …, _ => s.to_uppercase() }
//
// — the panel band, the window title, the unit suffix and three AI widgets
// — and every one of them ended on that same `_`. A theme that misspelt
// `uper` therefore got SHOUTING, with nothing said about why, which is the
// silent degradation this whole family of keys exists to make visible.

/// The transform a `*.case` token names. The master declares three such
/// keys and this enum is what every one of them means:
///
/// - `type.<role>.case`, read by [`Role::case`] — twenty-five of them,
///   one per role.
/// - `num.unit.case`, read where a unit run is dressed — the symbol
///   beside a reading.
/// - `type.suffix.case`, which has NO reader yet. It belongs to the
///   `[type.suffix]` block — a status word in brackets — and none of
///   that block is read: not `brackets`, not `paren_alpha`, not `gap`,
///   not `face`. Wiring the case alone would honour a quarter of a
///   sentence, so it waits for the object that draws the whole of it.
///
/// Resolved from the WORD and never from the enum index. Each key
/// declares its own `enum:` list, so an index memoised across keys names
/// a different transform in each — the trap `theme_word` was written for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum Case {
    /// The string as its author wrote it. Also what an unknown word
    /// answers: a transform nobody named is no transform.
    #[default]
    None,
    Upper,
    Lower,
    /// Approximated as [`Case::Upper`] until the font layer can set true
    /// small caps — fontdue exposes no OpenType features, and the master
    /// says so at `type.smallcaps_ratio`. The approximation lives HERE,
    /// in one function, so the day the font layer grows it there is one
    /// line to change rather than five.
    SmallCaps,
}

impl Case {
    /// The case a word names.
    ///
    /// An EMPTY word is a token that is not there at all — a missing key,
    /// or a role binding that resolved to nothing — and the reader that
    /// could not find it has already said so ([`bound_role`], [`role`]).
    /// Saying it twice under a second name would send a reader looking
    /// for a case key that is not the defect. A NON-EMPTY word the list
    /// does not hold is the typo this warning is for.
    pub fn from_word(word: &str) -> Case {
        match word {
            "none" | "" => Case::None,
            "upper" => Case::Upper,
            "lower" => Case::Lower,
            "smallcaps" => Case::SmallCaps,
            other => {
                warn_once(
                    &format!("case:{other}"),
                    &format!(
                        "\"{other}\" names no case transform — the list is \
                         none | upper | lower | smallcaps; the text is drawn as written"
                    ),
                );
                Case::None
            }
        }
    }
}

/// The one place a case transform is applied.
///
/// Borrowed for [`Case::None`], which is what most roles ask for and what
/// every unthemed and every mis-typed one now answers: the master's own
/// note on `[type]` objects to a caller that "folds case on its own side
/// [and] allocates a String per label per frame", and the borrow is how
/// that objection is answered until the transform can move inside
/// `FontSystem::text`.
pub fn recase(case: Case, s: &str) -> Cow<'_, str> {
    match case {
        Case::None => Cow::Borrowed(s),
        Case::Lower => Cow::Owned(s.to_lowercase()),
        Case::Upper | Case::SmallCaps => Cow::Owned(s.to_uppercase()),
    }
}

/// The case an enum token currently resolves to.
pub(crate) fn case_of(token: TokenId) -> Case {
    with_theme_word(token, Case::from_word)
}

// ---------------------------------------------------------------- type roles
//
// A role is named by a STRING — scripts name their own (`display.clock`) and
// role-binding tokens resolve to one — so the ids cannot live in per-site
// statics. They are memoised by name instead; token ids are stable for the
// life of the process, so the map never goes stale.

/// The token ids behind one `type.*` role.
#[derive(Clone, Copy)]
pub struct Role {
    size: TokenId,
    /// `type.<role>.face` — WHICH FACE this role is set in.
    ///
    /// Declared once per role in the master and read, until now, by
    /// nothing on this side of the plugin boundary: every text call in
    /// the toolkit and in the desktop named [`FONT_UI`] or [`FONT_MONO`]
    /// by hand. The plugin side has always read it (`tile::face_slot`),
    /// so `type.data.face = mono` meant monospace in a widget's own
    /// drawing and the interface face in the toolkit's — one role, two
    /// families, which is exactly the split `tabular` had.
    face: TokenId,
    min_px: TokenId,
    max_px: TokenId,
    tracking: TokenId,
    /// `type.<role>.case` — WHETHER THIS ROLE SHOUTS.
    ///
    /// It hung off the role's NAME and not off the role until now, so the
    /// two objects that honoured it — the panel band and the window title
    /// — each re-spelled `type.{word}.case` through a `Surface` beside a
    /// `Role` they already had, and everything else in the interface
    /// settled the question by writing capitals into the source. Twelve
    /// of the master's twenty-five roles ask for `upper` or `smallcaps`;
    /// with the key on the role, asking is all a theme has to do.
    case: TokenId,
    leading: TokenId,
    tabular: TokenId,
    fg: TokenId,
    alpha: TokenId,
}

/// The role for a name the master does not declare. There is no spare role
/// and there must not be one: a role is TWELVE tokens, so a single spare
/// word hides a whole ladder behind a name nobody wrote, and `body` — the
/// obvious candidate — is a REAL role of plausible size, which renders a
/// broken theme as a nearly-right interface and lets it ship. Every
/// member is MISSING and every accessor below answers zero px and no
/// ink for it, so the defect shows as a hole rather than as a near-miss.
const NO_ROLE: Role = Role {
    size: TokenId::MISSING,
    face: TokenId::MISSING,
    min_px: TokenId::MISSING,
    max_px: TokenId::MISSING,
    tracking: TokenId::MISSING,
    case: TokenId::MISSING,
    leading: TokenId::MISSING,
    tabular: TokenId::MISSING,
    fg: TokenId::MISSING,
    alpha: TokenId::MISSING,
};

/// The role for a name. A name no `type.*` block declares warns once and
/// answers [`NO_ROLE`]: naming a role the theme does not have is a defect
/// to report, never a decision about how the text should look.
pub fn role(name: &str) -> Role {
    thread_local! {
        static ROLES: RefCell<HashMap<String, Role>> = RefCell::new(HashMap::new());
    }
    fn lookup(name: &str) -> Option<Role> {
        // A name is resolved against the schema, and there is no schema
        // until a theme has been loaded — `resolved` is what loads it, the
        // same order `theme::enum_word_of` takes. The answer is memoised
        // for the life of the process, so asking one moment too early
        // would otherwise pin "no such role" on a role that exists.
        let _ = theme::resolved();
        Some(Role {
            size: theme::id(&format!("type.{name}.size"))?,
            face: theme::id(&format!("type.{name}.face")).unwrap_or(TokenId::MISSING),
            min_px: theme::id(&format!("type.{name}.min_px")).unwrap_or(TokenId::MISSING),
            max_px: theme::id(&format!("type.{name}.max_px")).unwrap_or(TokenId::MISSING),
            tracking: theme::id(&format!("type.{name}.tracking")).unwrap_or(TokenId::MISSING),
            case: theme::id(&format!("type.{name}.case")).unwrap_or(TokenId::MISSING),
            leading: theme::id(&format!("type.{name}.leading")).unwrap_or(TokenId::MISSING),
            tabular: theme::id(&format!("type.{name}.tabular")).unwrap_or(TokenId::MISSING),
            fg: theme::id(&format!("type.{name}.fg")).unwrap_or(TokenId::MISSING),
            alpha: theme::id(&format!("type.{name}.alpha")).unwrap_or(TokenId::MISSING),
        })
    }
    ROLES.with(|r| {
        if let Some(role) = r.borrow().get(name) {
            return *role;
        }
        let resolved = lookup(name).unwrap_or_else(|| {
            warn_once(
                &format!("role:{name}"),
                &format!("unknown type role \"{name}\" — nothing is drawn in it"),
            );
            NO_ROLE
        });
        r.borrow_mut().insert(name.to_string(), resolved);
        resolved
    })
}

/// The role a `*_role` binding token resolves to. Read through [`word_of`],
/// so a theme switching the binding lands on the next frame.
pub fn bound_role(cell: &'static OnceLock<TokenId>, binding: &'static str) -> Role {
    let word = word_of(tok(cell, binding));
    if word.is_empty() {
        // The BINDING is what a reader has to go and fix, and it is the one
        // thing the role-side warning cannot name: an empty word means
        // either that this key is absent from the master or that a consumer
        // asked for a key nobody declares, and both are the binding's story.
        warn_once(
            &format!("binding:{binding}"),
            &format!("\"{binding}\" names no type role — nothing is drawn in it"),
        );
        return NO_ROLE;
    }
    role(&word)
}

impl Role {
    /// Whether this is [`NO_ROLE`]. The size token is the discriminant:
    /// `lookup` refuses a role without one, so a missing size is never a
    /// role that merely resolves small.
    fn absent(&self) -> bool {
        self.size.is_missing()
    }

    /// The role's px for the panel being drawn, at the stack's shrink
    /// factor. The baked size carries the unit, density and user scale;
    /// `panel_scale` and `shrink` are runtime state, so they multiply here.
    pub fn px(&self, ctx: &Ctx, shrink: f32) -> f32 {
        static MIN: OnceLock<TokenId> = OnceLock::new();
        // `type.min_px` is the floor under a role the master DECLARES;
        // applying it to a role that does not exist would put the hole
        // back on screen at legible size.
        if self.absent() {
            return 0.0;
        }
        let t = theme::resolved();
        let raw = t.px(self.size) * ctx.panel_scale * shrink;
        // The master gives every role its OWN floor and ceiling, and the
        // shipped file writes each floor as `@type.min_px` — so reading the
        // role's own is the same number until a theme says otherwise, which
        // is exactly the point: a display face may want a higher floor than
        // running text, and until now it had no way to ask.
        //
        // The arithmetic itself is [`theme::role_px`] and is called rather
        // than repeated: the other resolver, `view::paint::role_look`, has
        // to answer this identically, and a rule written on both sides of
        // the library is a rule that stops matching itself somewhere.
        theme::role_px(
            raw,
            t.px(self.min_px),
            t.px(tok(&MIN, "type.min_px")),
            t.px(self.max_px),
        )
    }

    /// Letter spacing in px for a run of this role at `px`. Tracking tokens
    /// are em — a fraction of the run's own size.
    pub fn tracking_px(&self, px: f32) -> f32 {
        px * theme::resolved().px(self.tracking)
    }

    /// The case transform this role asks for.
    ///
    /// A role the master does not declare asks for nothing, which is
    /// [`Case::None`] and not capitals: shouting at a caller whose role
    /// is missing would be this file choosing a look, and the look is
    /// exactly what it may never choose.
    pub fn case(&self) -> Case {
        if self.absent() {
            return Case::None;
        }
        case_of(self.case)
    }

    /// This role's own string, in the case the theme set it in — the whole
    /// point of carrying [`Role::case`], and the call every object with a
    /// label should be making instead of writing capitals in its source.
    pub fn cased<'a>(&self, s: &'a str) -> Cow<'a, str> {
        recase(self.case(), s)
    }

    /// Line height as a multiple of the resolved px. A role whose master
    /// states no `leading` measures zero: an unstated line height is a
    /// broken role, and the height of a broken role is not this file's to
    /// invent — the same ruling as [`NO_ROLE`], one rung down.
    pub fn leading(&self) -> f32 {
        theme::resolved().px(self.leading)
    }

    /// The font slot this role's `face` names.
    ///
    /// A face is a CLOSED word set of eight — the master declares eight
    /// `[face.*]` blocks and numbers them itself — and it is read as a
    /// WORD and not as an index: an index would turn `display` into
    /// monospace on the day a theme reordered its face blocks. The word
    /// goes to [`crate::font::face_slot`], which is the one place that
    /// knows the master's numbering.
    ///
    /// It used to answer only `FONT_UI` or `FONT_MONO`, because those were
    /// the only two slots the atlas had. That is what collapsed
    /// `ui_medium` (500), `ui_bold` (700) and `display` (600) onto the one
    /// Regular file: the master's four weights had two boxes to arrive in.
    ///
    /// The SAME rule the plugin side applies (`launcher-core`'s
    /// `face_slot`, `ai`'s, `filesystem`'s), said once here so that the
    /// two sides of the boundary cannot answer "which family is this role"
    /// differently.
    ///
    /// A role the master does not declare has no face to name, and the
    /// interface slot is the one an undesigned run has always landed in.
    pub fn font(&self) -> u8 {
        if self.absent() {
            return FONT_UI;
        }
        crate::font::face_slot(&word_of(self.face))
    }

    /// Whether this role sets its figures on a fixed advance (§5.16's
    /// `tabular`). A role the master does not declare has no figures to
    /// box — the same ruling every other accessor here makes.
    pub fn tabular(&self) -> bool {
        !self.absent() && theme::resolved().flag(self.tabular)
    }

    /// The figure box to draw and measure this role's text under, at the
    /// px the caller resolved. [`Figures::NONE`] for a role that does not
    /// ask for one, which is the proportional run every text path drew
    /// before the token was implemented.
    ///
    /// Read ONCE per draw and carried into the row loop beside `px` and
    /// `track`: the box costs a theme read and — on the first call for a
    /// (face, px) — ten glyph lookups.
    pub fn figures(&self, fonts: &mut FontSystem, font: u8, px: f32) -> Figures {
        figures(fonts, font, px, self.tabular())
    }

    /// The colour this role draws in: fg × its constant alpha.
    pub fn color(&self) -> Color {
        if self.absent() {
            return Color::TRANSPARENT;
        }
        let t = theme::resolved();
        let c = t.color(self.fg);
        let a = t.px(self.alpha);
        Color {
            r: c.r,
            g: c.g,
            b: c.b,
            a: c.a * if a > 0.0 { a.min(1.0) } else { 1.0 },
        }
    }
}

// ------------------------------------------------------------ tabular figures
//
// The two `num.*` tokens that say WHICH characters a tabular role boxes.
// The mechanism — how wide the box is and how a glyph sits in it — is
// [`crate::font::Figures`]; this is the theme's half of it, kept here so
// that the font layer owns faces and this one owns tokens.

/// `num.tabular_set` — the characters a tabular role advances by the
/// figure box.
///
/// A TEXT token, so it lives in the cold-path diagnostics and is found
/// there by a linear scan of every text token the theme declares. Memoised
/// per theme epoch: this must be read once per theme, never per draw.
///
/// An `Rc<str>` rather than a `String` because the answer is handed out on
/// a DRAW path — one text call per label per frame — and a `String` here
/// is an allocation per label per frame behind a library whose stated rule
/// is zero per-draw allocation. Cloning the handle is a refcount bump.
fn tabular_set() -> Rc<str> {
    // A master that declares no set has said nothing about which
    // characters get the box, and inventing "0123456789" here would put a
    // look in this file — the one thing it may never hold. An empty set
    // answers `Figures::NONE`, so the defect shows.
    let v = theme_text_named("num.tabular_set");
    if v.is_empty() {
        warn_once(
            "num.tabular_set",
            "num.tabular_set is empty or absent — no role sets tabular figures",
        );
    }
    v
}

/// `type.ellipsis` — the string a trimmed run ends on.
///
/// Every trimming function in this library used to append `"\u{2026}"`
/// out of its own source: four of them here, two more in the widgets, all
/// six ignoring a key the master has declared since it was written and
/// whose comment names those very call sites ("a console theme may prefer
/// `...` or `>`"). The token is the answer for all of them now, and this
/// is where they get it.
///
/// An absent or empty key trims with NO marker rather than with a
/// borrowed one — the same ruling `tabular_set` makes, and the same
/// reason: the character a cut ends on is typography, so a literal here
/// would be a look decided in Rust.
pub fn ellipsis() -> Rc<str> {
    let v = theme_text_named("type.ellipsis");
    if v.is_empty() {
        warn_once(
            "type.ellipsis",
            "type.ellipsis is empty or absent — trimmed text ends on nothing",
        );
    }
    v
}

/// A TEXT token, memoised per theme epoch.
///
/// Text tokens live in the cold-path diagnostics and are found there by a
/// linear scan of every text token the theme declares, so this must be
/// read once per theme and never per draw. The WARNING for an unstated
/// key is each reader's own, because the sentence that helps is the one
/// that names what will not be drawn.
///
/// An `Rc<str>` rather than a `String` because the answer is handed out on
/// a DRAW path — one text call per label per frame — and a `String` here
/// is an allocation per label per frame behind a library whose stated rule
/// is zero per-draw allocation. Cloning the handle is a refcount bump.
///
/// Keyed by name with the epoch stored BESIDE the value rather than
/// inside the key: a lookup then borrows the caller's `&str` instead of
/// building a `String` to ask a question whose answer is usually already
/// there.
pub(crate) fn theme_text_named(name: &str) -> Rc<str> {
    let epoch = theme::epoch();
    TEXTS.with(|c| {
        if let Some((e, v)) = c.borrow().get(name) {
            if *e == epoch {
                return v.clone();
            }
        }
        let v: Rc<str> = theme::diagnostics().text(name).unwrap_or_default().into();
        c.borrow_mut().insert(name.to_string(), (epoch, v.clone()));
        v
    })
}

thread_local! {
    static TEXTS: RefCell<HashMap<String, (u32, Rc<str>)>> = RefCell::new(HashMap::new());
}

/// States a text token for the CALLING THREAD only — the seam the trim
/// tests drive `type.ellipsis` through.
///
/// The memo is a thread_local and one `#[test]` is one thread, so a value
/// seeded here reaches that test and no other. The alternative — publishing
/// a theme — would decide what every test running beside it draws from,
/// which is the reason `theme::bake_over_master` exists and does not
/// publish either.
#[cfg(test)]
pub(crate) fn seed_theme_text(name: &str, v: &str) {
    let epoch = theme::epoch();
    TEXTS.with(|c| c.borrow_mut().insert(name.to_string(), (epoch, v.into())));
}

/// The figure box for a run at `px` in `font`, or [`Figures::NONE`] when
/// the role does not ask for one.
///
/// The single resolver: the objects drawing against `Ctx` reach it through
/// [`Role::figures`], the views through [`CtxSurface`]. Two answers to
/// "how wide is a figure here" is how a measured column comes to be drawn
/// at a width it was not measured at.
pub fn figures(fonts: &mut FontSystem, font: u8, px: f32, tabular: bool) -> Figures {
    static PUNCT: OnceLock<TokenId> = OnceLock::new();
    if !tabular {
        return Figures::NONE;
    }
    let punct = theme::resolved().flag(tok(&PUNCT, "num.tabular_punct"));
    fonts.figures(font, px, &tabular_set(), punct)
}

// ---------------------------------------------------------------- motion
//
// The blink a `runs` item may carry (§5.29): a 0..1 factor from
// `motion.<id>`, applied to the run's ALPHA — the glyph keeps its advance,
// which is what stops the clock jittering. Frozen fully visible under
// reduced motion (`motion.scale = 0`) or when the effect is disabled.

/// The 0..1 factor of the cyclic motion effect `motion.<id>` at time `t`.
///
/// A door into the shared resolver: [`crate::motion::Effect::cyclic`]
/// carries the whole contract now — the memoised token lookup, the
/// warn-once on an id outside the catalogue, the freeze at fully visible
/// (reduced motion, a disabled effect, a zero period: a separator that
/// never returns is a content change), and the step curve the cyclic
/// sources run on.
pub fn blink_factor(id: &str, t: f64) -> f32 {
    crate::motion::Effect::of(id).cyclic(t)
}

// `role_px` stood here: one `type.<name>.size` read by NAME, with the
// global floor and neither the role's own floor nor its ceiling. Its last
// caller was the gauge, which now follows `gauge.label_role` and
// `gauge.value_role` like everything else — so every type size in this
// file comes through [`Role`], which is the only reader that answers the
// whole ladder. A helper that resolves half a role is how two halves of
// one program come to disagree about how big a role is.

/// Top of a single line centred in a box of `box_h`. The line occupies
/// its role's leading; in optical mode the cap-height bias nudges it.
/// True optical centring wants the font's cap height, which the draw
/// list does not expose yet — the bias is the part that can draw today.
///
/// The arithmetic itself lives in [`paint::center_line_y`], where the
/// views on the far side of the plugin boundary reach it too; this is
/// the host's way in — `pub(crate)` because the script host centres rows
/// of its own, and a second guess at a cap height there is how the whole
/// program came to have two of them.
pub(crate) fn center_line_y(
    ctx: &mut Ctx,
    face: u8,
    y: f32,
    box_h: f32,
    px: f32,
    leading: f32,
) -> f32 {
    paint::center_line_y_in(&mut CtxSurface::new(ctx), face, y, box_h, px, leading)
}

/// Top edge for a block of known natural height, centred vertically in
/// `r` and never pushed above it.
pub fn block_top(r: &Rect, natural: f32) -> f32 {
    block_top_aligned(r, natural, Vy::Middle)
}

/// Where a block of known height stands in a box taller than itself.
/// `top | middle | bottom` is the vocabulary every alignment key in the
/// master uses, and the words are compared as words.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Vy {
    Top,
    Middle,
    Bottom,
    /// `baseline`, which the master spells for `meter.bar_align` and
    /// which has no shared baseline to sit on yet. It reads as `middle`
    /// until the optical-centring primitive exists — the same standing
    /// `smallcaps` has against `upper`.
    Baseline,
}

/// The word an alignment token stands at. Unknown words read as `top`:
/// the block goes where the box begins, which is the one placement that
/// invents nothing.
pub fn vy_of(word: &str) -> Vy {
    match word {
        "middle" => Vy::Middle,
        "bottom" => Vy::Bottom,
        "baseline" => Vy::Baseline,
        _ => Vy::Top,
    }
}

/// [`block_top`] with the placement stated rather than assumed. Never
/// pushed above the box: a block taller than its room starts at the top
/// whatever the theme asked for, because the alternative is a block whose
/// head is off screen.
pub fn block_top_aligned(r: &Rect, natural: f32, vy: Vy) -> f32 {
    let slack = (r.h - natural).max(0.0);
    r.y + match vy {
        Vy::Top => 0.0,
        Vy::Middle | Vy::Baseline => slack / 2.0,
        Vy::Bottom => slack,
    }
}

/// Trims text with a trailing ellipsis so it fits `max_w`, measured in the
/// SAME face, at the same tracking and under the same figure box the
/// caller draws with. `base::fit_end` measures at a fixed legacy tracking
/// in a fixed face; under a role that states either differently, a string
/// would trim against one width and draw at another — which is how a
/// content-measured table column came to ellipsise the very cell it was
/// sized from.
fn fit_end_tracked_tab(
    ctx: &mut Ctx,
    face: u8,
    px: f32,
    text: &str,
    max_w: f32,
    track: f32,
    tabular: bool,
) -> String {
    paint::fit_end_tab(&mut CtxSurface::new(ctx), face, px, text, max_w, track, tabular)
}

/// [`paint::explain_trim`] for the primitives that still draw against
/// `Ctx`: whatever [`fit_end_tracked`] shortened says the whole of
/// itself when the pointer rests on it (F2 §8.1).
///
/// The pair belongs together — a call to the first without a call to the
/// second is a value the user can neither read nor ask about — and both
/// go through the same host surface, so the rule is the one the views
/// obey and not a second copy of it.
fn explain_trim(ctx: &mut Ctx, id: u64, anchor: Rect, shown: &str, full: &str) {
    paint::explain_trim(&mut CtxSurface::new(ctx), id, anchor, shown, full);
}

/// Breaks text into lines no wider than `max_w`, greedily by words —
/// the host's way into [`paint::wrap`], where the arithmetic lives so
/// that a view on the far side of the plugin boundary shares it.
///
/// The tooltip is its first caller; the text phase will be its second,
/// which is why it is public vocabulary rather than a private helper.
pub fn wrap_text(
    ctx: &mut Ctx,
    face: u8,
    px: f32,
    text: &str,
    max_w: f32,
    track: f32,
) -> Vec<String> {
    wrap_text_tab(ctx, face, px, text, max_w, track, false)
}

/// [`wrap_text`] broken under the role's figure box — the box the caller
/// is about to DRAW the lines with, so the width a line was accepted at
/// is the width it comes out at.
#[allow(clippy::too_many_arguments)]
pub fn wrap_text_tab(
    ctx: &mut Ctx,
    face: u8,
    px: f32,
    text: &str,
    max_w: f32,
    track: f32,
    tabular: bool,
) -> Vec<String> {
    paint::wrap_tab(&mut CtxSurface::new(ctx), face, px, text, max_w, track, tabular)
}

/// How a `rows` block sizes its label column (u2 §3.1 #4).
#[derive(Clone, Copy, PartialEq)]
pub enum LabelWidth {
    /// Each value placed against its own label — today's ragged cloud,
    /// values right-aligned at the row's edge.
    Auto,
    /// Every label in the block measured once; all values start at one x,
    /// left-aligned — the images' tight label column (u2 §2.3).
    Max,
}

/// One `rows` line: a label, a value, and the script's judgement of the
/// value's severity — an index into the closed set, never a colour.
pub struct RowItem {
    pub label: String,
    pub value: String,
    pub sev: Option<Sev>,
}

/// How a `rows` block is arranged. Metrics that shrink with the stack
/// arrive pre-scaled from the caller ([`crate::script`]'s fit pass);
/// everything else is read from the theme here.
pub struct RowsStyle {
    pub label_role: Role,
    pub value_role: Role,
    pub columns: usize,
    pub label_width: LabelWidth,
    /// One line's height, already at the stack's shrink factor.
    pub row_h: f32,
    /// The stack's shrink factor, for the type sizes.
    pub shrink: f32,
}

/// Rows of `label` and `value`, flowed into `st.columns` grid columns
/// row-major, the whole block centred vertically. A line with fewer cells
/// than the grid spans the width it has (u2 §2.3's 2+1 case). Values are
/// trimmed to the space their label leaves. Returns the height used.
pub fn rows_label_value(ctx: &mut Ctx, r: Rect, rows: &[RowItem], st: &RowsStyle) -> f32 {
    if rows.is_empty() {
        return 0.0;
    }
    static LABEL_PAD: OnceLock<TokenId> = OnceLock::new();
    static COL_GAP: OnceLock<TokenId> = OnceLock::new();
    static LABEL_C: OnceLock<TokenId> = OnceLock::new();
    static VALUE_C: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let cols = st.columns.max(1);
    let lines = rows.len().div_ceil(cols);
    let row_h = st.row_h.min(r.h / lines as f32);
    let lpx = st.label_role.px(ctx, st.shrink);
    let vpx = st.value_role.px(ctx, st.shrink);
    let ltrack = st.label_role.tracking_px(lpx);
    let vtrack = st.value_role.tracking_px(vpx);
    // The figure boxes of the two halves of the line, resolved once and
    // carried into the loop below. This is the line the owner was
    // looking at: NETWORK's address and UPTIME's counter are both a
    // `value` half, and without the box each of them changes width
    // whenever a 1 replaces an 8 in it.
    // Each half in its ROLE's face, not this call site's guess at one.
    // `gauge_rows` two hundred lines down already asked, and the same file
    // answering the same question two ways is how `script.table_cell_role
    // = data` came to be drawn in the interface face while the master said
    // `type.data.face = mono`.
    let lface = st.label_role.font();
    let vface = st.value_role.font();
    let lfig = st.label_role.figures(ctx.fonts, lface, lpx);
    let vfig = st.value_role.figures(ctx.fonts, vface, vpx);
    let vtab = st.value_role.tabular();
    let pad = t.px(tok(&LABEL_PAD, "rhythm.label_pad")) * st.shrink;
    let gap = t.px(tok(&COL_GAP, "script.rows_col_gap")) * st.shrink;
    let label_c = col(&LABEL_C, "component.script.label");
    let value_c = col(&VALUE_C, "component.script.value");
    // The shared label column: the widest label per GRID column, so all
    // values in that column start at one x. A spanning cell aligns with
    // column 0 — its value keeps the left column's x.
    let mut label_w = vec![0.0f32; cols];
    if st.label_width == LabelWidth::Max {
        for (i, row) in rows.iter().enumerate() {
            let line = i / cols;
            let cells_on_line = (rows.len() - line * cols).min(cols);
            let j = if cells_on_line < cols { 0 } else { i % cols };
            let w = ctx.fonts.measure_fig(lface, lpx, &row.label, ltrack, &lfig);
            label_w[j] = label_w[j].max(w);
        }
    }
    let natural = row_h * lines as f32;
    let top = block_top(&r, natural);
    for (i, row) in rows.iter().enumerate() {
        let line = i / cols;
        let cells_on_line = (rows.len() - line * cols).min(cols);
        let j = i % cols;
        let cell_w = (r.w - gap * (cells_on_line as f32 - 1.0)) / cells_on_line as f32;
        let cx = r.x + (cell_w + gap) * j as f32;
        let y = top + row_h * line as f32;
        let lty = center_line_y(ctx, lface, y, row_h, lpx, st.label_role.leading());
        let vty = center_line_y(ctx, vface, y, row_h, vpx, st.value_role.leading());
        ctx.dl.text_fig(ctx.fonts, lface, lpx, cx, lty, &row.label, label_c, ltrack, &lfig);
        let vc = row.sev.map(sev_text).unwrap_or(value_c);
        match st.label_width {
            LabelWidth::Max => {
                let colw = if cells_on_line < cols { label_w[0] } else { label_w[j] };
                let vx = cx + colw + pad;
                let room = (cx + cell_w - vx).max(pad);
                let shown = fit_end_tracked_tab(ctx, vface, vpx, &row.value, room, vtrack, vtab);
                ctx.dl.text_fig(ctx.fonts, vface, vpx, vx, vty, &shown, vc, vtrack, &vfig);
                // The label is drawn whole and the value is what the
                // room runs out on, so the value's own box is what the
                // pointer has to rest on to be answered. The identity is
                // the table's, with no view to belong to: the PLACE in
                // the block, named by the label — the one half of the
                // pair that does not change when the reading does, and
                // the place tells two rows with no label apart.
                let id = crate::object::tooltip::cell_key(0, i, &row.label);
                explain_trim(ctx, id, Rect::new(vx, y, room, row_h), &shown, &row.value);
            }
            LabelWidth::Auto => {
                let lw = ctx.fonts.measure_fig(lface, lpx, &row.label, ltrack, &lfig);
                let room = (cell_w - lw - pad).max(pad);
                let shown = fit_end_tracked_tab(ctx, vface, vpx, &row.value, room, vtrack, vtab);
                ctx.dl.text_right_fig(
                    ctx.fonts, vface, vpx, cx + cell_w, vty, &shown, vc, vtrack, &vfig,
                );
                // Right-aligned, so the value's box ends at the cell's
                // right edge and starts `room` before it.
                let id = crate::object::tooltip::cell_key(0, i, &row.label);
                let box_r = Rect::new(cx + cell_w - room, y, room, row_h);
                explain_trim(ctx, id, box_r, &shown, &row.value);
            }
        }
    }
    natural
}

/// A framed meter with a proportional fill: the outline shows the whole,
/// the fill shows `frac` of it (clamped, so bad data cannot overdraw).
/// Track and fill come from `component.bar.*` — read here, not passed:
/// a caller with a colour in hand is a caller doing the theme's job.
/// A severity is the script's judgement of the DATA (an index into the
/// closed set, not a colour) and tints the fill; `track = false` says the
/// value has no meaningful whole, so no outline claims one.
pub fn meter(ctx: &mut Ctx, r: Rect, frac: f32, sev: Option<Sev>, track: bool) {
    paint::meter(&mut CtxSurface::new(ctx), r, frac, sev, track);
}

/// A grid of cells of which the first `frac` are lit — the dot matrix
/// used for memory. The preferred pitch is `script.dots_cell` with its
/// `script.dots_cell_min_px` floor, read here; `shrink` is the stack's
/// shrink-to-fit factor — runtime state like `panel_scale`, never a look
/// decision. The grid is fitted to `r` and always keeps at least one cell.
pub fn dot_matrix(ctx: &mut Ctx, r: Rect, frac: f32, shrink: f32) {
    let frac = if frac.is_finite() {
        frac.clamp(0.0, 1.0)
    } else {
        0.0
    };
    static PITCH: OnceLock<TokenId> = OnceLock::new();
    static PITCH_MIN: OnceLock<TokenId> = OnceLock::new();
    static CELL: OnceLock<TokenId> = OnceLock::new();
    static CELL_MIN: OnceLock<TokenId> = OnceLock::new();
    static FILL_RATIO: OnceLock<TokenId> = OnceLock::new();
    static FILL_MIN: OnceLock<TokenId> = OnceLock::new();
    static GAP_MIN: OnceLock<TokenId> = OnceLock::new();
    static ON: OnceLock<TokenId> = OnceLock::new();
    static OFF: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let cell = (t.px(tok(&PITCH, "script.dots_cell")) * shrink)
        .max(t.px(tok(&PITCH_MIN, "script.dots_cell_min_px")));
    let step = cell.max(t.px(tok(&CELL_MIN, "dotmatrix.cell_min_px")));
    // The pitch is a DIVISOR, and the two theme floors above are what a
    // master states to keep it off zero. A `.max(1.0)` here would be a
    // one-pixel cell written in Rust; refusing to draw is the honest
    // reading of a matrix whose cell the theme sized to nothing — and
    // the guard the division needs, since `r.w / 0.0` is infinity and an
    // infinite column count is a frame that never ends.
    if !(step > 0.0) {
        warn_once(
            "dotmatrix:cell",
            "`script.dots_cell` and `dotmatrix.cell_min_px` leave the dot pitch at zero — \
             the matrix has no cell to draw",
        );
        return;
    }
    let cols = ((r.w / step).floor() as usize).max(1);
    let rows = ((r.h / step).floor() as usize).max(1);
    let total = cols * rows;
    let lit = (frac * total as f32).round() as usize;
    // fill_ratio is baked against the theme's own cell, so it is turned
    // back into a fraction and applied to the pitch actually in use;
    // gap_min_px is what stops adjacent lit rows fusing into bars.
    let cell_ref = t.px(tok(&CELL, "dotmatrix.cell"));
    let ratio = if cell_ref > 0.0 {
        t.px(tok(&FILL_RATIO, "dotmatrix.fill_ratio")) / cell_ref
    } else {
        0.0
    };
    // `dotmatrix.fill_min_px` is the theme's own floor under the dot;
    // the trailing clamp is at zero, not at a pixel, because a gap wider
    // than the pitch is arithmetic going negative and not a size anyone
    // stated.
    let size = (step * ratio)
        .max(t.px(tok(&FILL_MIN, "dotmatrix.fill_min_px")))
        .min(step - t.px(tok(&GAP_MIN, "dotmatrix.gap_min_px")))
        .max(0.0);
    let on = col(&ON, "component.matrix.cell_on");
    let off = col(&OFF, "component.matrix.cell_off");
    for i in 0..total {
        let cx = r.x + (i % cols) as f32 * step;
        let cy = r.y + (i / cols) as f32 * step;
        ctx.dl.rect(cx, cy, size, size, if i < lit { on } else { off });
    }
}

/// What one gauge is drawn as (u2 §2.5). `bar` and `donut` exist in the
/// vocabulary but cannot yet carry the per-core number they owe (content
/// preservation), so the caller degrades them to `Row` with a warning.
#[derive(Clone, Copy, PartialEq)]
pub enum GaugeKind {
    /// label + thin track + value — image 1's resource row.
    Row,
    /// A framed box with the number inside at the far end — today's look.
    Cell,
}

/// Where a row-style gauge's label comes from. The label is arrangement
/// data from the script (a core is `C0` because the script says so).
pub enum GaugeLabels {
    None,
    /// `prefix` + the gauge's index: `C0`, `C1`, …
    Index(String),
    /// One label per value.
    Text(Vec<String>),
}

/// How the numeric readout is written. The number itself is the same value
/// the fill encodes — a second presentation, not new data.
#[derive(Clone, Copy, PartialEq)]
pub enum GaugeValueFmt {
    /// `{v:.0}%` — today's format.
    Percent,
    /// `{v:.0}` — a plain number.
    Raw,
}

/// How a `gauges` element is arranged. Everything visual is read from the
/// theme inside; this struct carries the script's arrangement choices and
/// the stack's runtime shrink factor.
pub struct GaugeStyle {
    pub cols: usize,
    pub kind: GaugeKind,
    pub labels: GaugeLabels,
    pub value_fmt: GaugeValueFmt,
    pub shrink: f32,
}

/// A grid of gauges, one per value, flowed into `st.cols` columns. `Cell`
/// is a framed meter with its value written inside, flipping to
/// `component.bar.text_on_fill` where the fill would swallow it; `Row` is
/// label + thin track + value, the images' instrument row, where the
/// number always fits and so is always drawn (u2 §2.5). The colours and
/// metrics are all read here.
pub fn gauge_grid(ctx: &mut Ctx, r: Rect, values: &[f32], st: &GaugeStyle) {
    if values.is_empty() {
        return;
    }
    if st.kind == GaugeKind::Row {
        return gauge_rows(ctx, r, values, st);
    }
    let cols = st.cols;
    static GAP: OnceLock<TokenId> = OnceLock::new();
    static VALUE_ROLE: OnceLock<TokenId> = OnceLock::new();
    static MIN_H: OnceLock<TokenId> = OnceLock::new();
    static BODY_H: OnceLock<TokenId> = OnceLock::new();
    static BORDER: OnceLock<TokenId> = OnceLock::new();
    static CLEARANCE: OnceLock<TokenId> = OnceLock::new();
    static INSET: OnceLock<TokenId> = OnceLock::new();
    static TEXT_C: OnceLock<TokenId> = OnceLock::new();
    static ON_FILL_C: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let cols = cols.max(1);
    let rows = values.len().div_ceil(cols);
    let gap = t.px(tok(&GAP, "gauge.gap"));
    let gw = (r.w - gap * (cols as f32 - 1.0)) / cols as f32;
    let gh = ((r.h - gap * (rows as f32 - 1.0)) / rows as f32).max(1.0);
    // The readout is a NUMBER, so it is set in the role the master binds
    // every other number to. Spelling `type.caption.*` out here read the
    // ladder of whichever role happened to be named `caption`, left
    // `gauge.value_role` with no reader at all, and — since the row form
    // below spelled the same three tokens out for its LABEL — set a
    // reading and its own key at one size, which no other instrument row
    // in the program does.
    let value_role = bound_role(&VALUE_ROLE, "gauge.value_role");
    let px = value_role.px(ctx, 1.0);
    let leading = value_role.leading();
    let track = value_role.tracking_px(px);
    // The role's own FACE, not this call site's guess at one: a theme
    // that sets its readouts in the monospace face gets monospace here.
    let face = value_role.font();
    let fig = value_role.figures(ctx.fonts, face, px);
    // min_h_for_value is baked from the readout's resting size, so it
    // follows the same container-query factor the drawn px does.
    //
    // Held at `gauge.h`, the height the master declares a gauge body to
    // be. The threshold is a ratio of the READING and the body height is
    // a length, so nothing kept the two in step: the master's own numbers
    // ask to drop the reading below 4.55u out of a body declared 4u tall,
    // which says a gauge drawn at its own declared size must throw its
    // number away. Two keys of one section contradicting each other is a
    // defect whatever the right size turns out to be — and `gauge.h`,
    // which until now no line of this program read, is what says which of
    // them is the ceiling.
    //
    // Taking the smaller of the two changes no drawn text: the reading's
    // SIZE stays `gauge.value_role`'s, which is the theme's to set and
    // (at 3.25u today against the 1.77u it was set at before the type
    // ladder was unified) the owner's to settle.
    let min_h = t
        .px(tok(&MIN_H, "gauge.min_h_for_value"))
        .min(t.px(tok(&BODY_H, "gauge.h")))
        * ctx.panel_scale;
    let bw = t.px(tok(&BORDER, "gauge.border"));
    let clearance = t.px(tok(&CLEARANCE, "gauge.value_clearance"));
    let inset = t.px(tok(&INSET, "gauge.value_inset"));
    let text_c = col(&TEXT_C, "component.gauge.text");
    let on_fill_c = col(&ON_FILL_C, "component.bar.text_on_fill");
    for (i, v) in values.iter().enumerate() {
        let gx = r.x + (i % cols) as f32 * (gw + gap);
        let gy = r.y + (i / cols) as f32 * (gh + gap);
        let cell = Rect::new(gx, gy, gw, gh);
        meter(ctx, cell, v / 100.0, None, true);
        // The number only fits — and is only worth drawing — when the
        // gauge is tall enough for it.
        if gh < min_h {
            continue;
        }
        let text = gauge_value(*v, st.value_fmt);
        let u = unit_run(ctx, face, px, &text);
        // Measured under the role's OWN figure box: the readout is drawn
        // through it, and a contrast flip decided on a proportional width
        // fires at a different fill for `11%` than for `88%`. The unit
        // run is part of that width — it stands on the same fill.
        let tw = reading_w(ctx, face, px, track, &fig, &text, &u);
        let fill_w = (gw - 2.0 * bw) * (v / 100.0).clamp(0.0, 1.0);
        // The number sits at the far END of the gauge, where the fill
        // arrives last. On the near end — where it used to be — every
        // small reading had its own first digit painted over by the few
        // pixels of fill that were the whole point of the gauge.
        let swallowed = fill_w >= gw - 2.0 * bw - tw - clearance;
        let c = if swallowed { on_fill_c } else { text_c };
        let ty = center_line_y(ctx, face, gy, gh, px, leading);
        // On the fill the unit gives up its own step-back colour: the
        // step back exists so the number reads first against the panel,
        // and against the fill both halves need the contrast ink.
        let unit_c = swallowed.then_some(on_fill_c);
        draw_reading_right(
            ctx,
            face,
            px,
            track,
            &fig,
            gx + gw - inset,
            ty,
            &text,
            c,
            unit_c,
            &u,
        );
    }
}

/// A gauge's readout, written down under the theme's number policy.
///
/// `format!("{v:.0}%")` was the whole of it until 2026-08-17: the places,
/// the decimal mark and the unit's letters were all decided here, in
/// Rust, while `[num]` declared fourteen keys nobody read. A gauge is the
/// master's own example of "where room is tight", so the places are
/// `num.decimals_compact` and not `num.decimals`.
fn gauge_value(v: f32, fmt: GaugeValueFmt) -> num::Reading {
    match fmt {
        GaugeValueFmt::Percent => num::Reading::compact(v as f64, "%"),
        GaugeValueFmt::Raw => num::Reading::compact(v as f64, ""),
    }
}

/// The unit suffix's own typography — the six `num.unit.*` keys, resolved
/// against the px of the number it follows.
///
/// A unit is a SECOND RUN, and that is the whole reason this struct
/// exists: `TB` set at 0.72 of the number's size, a step back in colour so
/// the number reads first, its own tracking and its own baseline can none
/// of them be expressed by appending characters to a string. The master
/// has described that run since the block was written; nothing drew it.
struct UnitRun {
    px: f32,
    track: f32,
    /// Distance between the number and the unit, already zero where the
    /// reading is attached (`num.unit.percent_attached`).
    gap: f32,
    /// Baseline offset, positive DOWN — the master's `0.0em` default says
    /// units sit on the baseline and are never superscript.
    shift: f32,
    color: Color,
    width: f32,
}

impl UnitRun {
    /// Nothing to draw: the reading is a bare number.
    const NONE: UnitRun = UnitRun {
        px: 0.0,
        track: 0.0,
        gap: 0.0,
        shift: 0.0,
        color: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 },
        width: 0.0,
    };
}

/// The unit half of `reading`, measured in the face the value's role
/// names — a unit belongs to its number's run and does not get a face of
/// its own to be set in.
fn unit_run(ctx: &mut Ctx, face: u8, value_px: f32, reading: &num::Reading) -> UnitRun {
    static SCALE: OnceLock<TokenId> = OnceLock::new();
    static GAP: OnceLock<TokenId> = OnceLock::new();
    static TRACKING: OnceLock<TokenId> = OnceLock::new();
    static SHIFT: OnceLock<TokenId> = OnceLock::new();
    static COLOR: OnceLock<TokenId> = OnceLock::new();
    if reading.unit.is_empty() {
        return UnitRun::NONE;
    }
    let t = theme::resolved();
    // `unit.scale` is a fraction of the VALUE's px and `unit.tracking` /
    // `unit.baseline_shift` are ems of the unit's own — which is what
    // "the unit suffix's px as a fraction of the value's px" and "letter
    // spacing inside the unit suffix" say in the master.
    let px = (value_px * t.px(tok(&SCALE, "num.unit.scale"))).max(0.0);
    let gap = if reading.attached() { 0.0 } else { value_px * t.px(tok(&GAP, "num.unit.gap")) };
    let track = px * t.px(tok(&TRACKING, "num.unit.tracking"));
    let shift = px * t.px(tok(&SHIFT, "num.unit.baseline_shift"));
    let width = ctx.fonts.measure(face, px, &reading.unit, track);
    UnitRun { px, track, gap, shift, color: col(&COLOR, "num.unit.color"), width }
}

/// How wide the whole reading is: the number under its figure box, plus
/// the joint and the unit run.
///
/// One function, because a readout measured one way and drawn another is
/// how a contrast flip comes to fire at a different fill for `11%` than
/// for `88%` — the very defect the figure box was introduced here to fix.
fn reading_w(
    ctx: &mut Ctx,
    face: u8,
    px: f32,
    track: f32,
    fig: &Figures,
    reading: &num::Reading,
    u: &UnitRun,
) -> f32 {
    ctx.fonts.measure_fig(face, px, &reading.number, track, fig) + u.gap + u.width
}

/// Draws `reading` with its right edge at `right`, the number under the
/// role's figure box and the unit in its own run.
///
/// `unit_c` is `None` where the caller has no opinion and the unit takes
/// `num.unit.color`; a gauge whose fill has swallowed the readout passes
/// the on-fill ink for BOTH halves, because the unit is standing on the
/// same fill the number is.
#[allow(clippy::too_many_arguments)]
fn draw_reading_right(
    ctx: &mut Ctx,
    face: u8,
    px: f32,
    track: f32,
    fig: &Figures,
    right: f32,
    y: f32,
    reading: &num::Reading,
    number_c: Color,
    unit_c: Option<Color>,
    u: &UnitRun,
) {
    let mut edge = right;
    if !reading.unit.is_empty() {
        ctx.dl.text_right(
            ctx.fonts,
            face,
            u.px,
            edge,
            y + u.shift,
            &reading.unit,
            unit_c.unwrap_or(u.color),
            u.track,
        );
        edge -= u.width + u.gap;
    }
    ctx.dl.text_right_fig(ctx.fonts, face, px, edge, y, &reading.number, number_c, track, fig);
}

/// The `Row` gauge form: label + thin track + value per cell, flowed into
/// the same grid the cells use. The label and value columns are measured
/// once across the block so every track starts and ends at one x — the
/// images align, they do not centre (u2 §2.5).
fn gauge_rows(ctx: &mut Ctx, r: Rect, values: &[f32], st: &GaugeStyle) {
    static GAP: OnceLock<TokenId> = OnceLock::new();
    static LABEL_ROLE: OnceLock<TokenId> = OnceLock::new();
    static VALUE_ROLE: OnceLock<TokenId> = OnceLock::new();
    static LABEL_GAP: OnceLock<TokenId> = OnceLock::new();
    static VALUE_GAP: OnceLock<TokenId> = OnceLock::new();
    static BAR_H: OnceLock<TokenId> = OnceLock::new();
    static TEXT_C: OnceLock<TokenId> = OnceLock::new();
    static LABEL_ALIGN: OnceLock<TokenId> = OnceLock::new();
    static VALUE_ALIGN: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let cols = st.cols.max(1);
    let rows = values.len().div_ceil(cols);
    let gap = t.px(tok(&GAP, "gauge.gap")) * st.shrink;
    let gw = (r.w - gap * (cols as f32 - 1.0)) / cols as f32;
    let gh = ((r.h - gap * (rows as f32 - 1.0)) / rows as f32).max(1.0);
    // A gauge row is label + track + value — the same three parts a
    // `meter` row has, so it reads the same PAIR of bindings the master
    // gives that row (`gauge.label_role` / `gauge.value_role`, the
    // siblings of `script.meter_label_role` / `script.meter_value_role`).
    // Both halves used to be `type.caption.*` spelled out by name, which
    // set `C0` and `12%` at one size and left both bindings unread.
    let label_role = bound_role(&LABEL_ROLE, "gauge.label_role");
    let value_role = bound_role(&VALUE_ROLE, "gauge.value_role");
    let lpx = label_role.px(ctx, st.shrink);
    let vpx = value_role.px(ctx, st.shrink);
    let llead = label_role.leading();
    let vlead = value_role.leading();
    let ltrack = label_role.tracking_px(lpx);
    let vtrack = value_role.tracking_px(vpx);
    // Each half in its OWN face, for the same reason each is at its own
    // size: which family a role is set in is the role's to say.
    let lface = label_role.font();
    let vface = value_role.font();
    // The readout's figure box, resolved once and carried into the row
    // loop: a column of numbers measured proportionally is a column that
    // moves every time a reading changes width.
    let vfig = value_role.figures(ctx.fonts, vface, vpx);
    let lgap = t.px(tok(&LABEL_GAP, "meter.label_gap")) * st.shrink;
    let vgap = t.px(tok(&VALUE_GAP, "meter.value_gap")) * st.shrink;
    let bar_h = t.px(tok(&BAR_H, "script.meter_bar_h")) * st.shrink;
    let text_c = col(&TEXT_C, "component.gauge.text");
    let label_of = |i: usize| -> String {
        match &st.labels {
            GaugeLabels::None => String::new(),
            GaugeLabels::Index(prefix) => format!("{prefix}{i}"),
            GaugeLabels::Text(v) => v.get(i).cloned().unwrap_or_default(),
        }
    };
    // One label column and one value column for the whole block, so the
    // tracks line up between rows and between grid columns.
    let mut label_w = 0.0f32;
    let mut value_w = 0.0f32;
    for (i, v) in values.iter().enumerate() {
        label_w = label_w.max(ctx.fonts.measure(lface, lpx, &label_of(i), ltrack));
        let val = gauge_value(*v, st.value_fmt);
        // Measured under the box it is DRAWN under, so the column is
        // sized at the width the readings really occupy.
        let u = unit_run(ctx, vface, vpx, &val);
        value_w = value_w.max(reading_w(ctx, vface, vpx, vtrack, &vfig, &val, &u));
    }
    let label_col = if label_w > 0.0 { label_w + lgap } else { 0.0 };
    // Where the two halves sit inside the columns just measured. Both
    // are the theme's to say and neither had a reader: the label was
    // flushed left and the reading right because that is what the code
    // was written to do, whatever `[rhythm]` asked for.
    let label_align = rhythm_align(&LABEL_ALIGN, "rhythm.label_align");
    let value_align = rhythm_align(&VALUE_ALIGN, "rhythm.value_align");
    for (i, v) in values.iter().enumerate() {
        let gx = r.x + (i % cols) as f32 * (gw + gap);
        let gy = r.y + (i / cols) as f32 * (gh + gap);
        // Two roles now, so two line boxes: each half is centred on the
        // row in its OWN leading, which is what put a `rows` line's key
        // and value on one optical centre and what a single shared px
        // could not say.
        let lty = center_line_y(ctx, lface, gy, gh, lpx, llead);
        let vty = center_line_y(ctx, vface, gy, gh, vpx, vlead);
        let label = label_of(i);
        if !label.is_empty() {
            // Inside the shared label column, which is what makes an
            // alignment mean anything: `right` is flush with the track's
            // left edge, `left` is flush with the block's.
            let lx = align_in(gx, label_w, ctx.fonts.measure(lface, lpx, &label, ltrack), label_align);
            ctx.dl.text(ctx.fonts, lface, lpx, lx, lty, &label, text_c, ltrack);
        }
        let bar = Rect::new(
            gx + label_col,
            gy + (gh - bar_h).max(0.0) / 2.0,
            (gw - label_col - value_w - vgap).max(1.0),
            bar_h.min(gh),
        );
        meter(ctx, bar, v / 100.0, None, true);
        // A row always has room for its number, so the number is always
        // drawn — item 4 of the cpu inventory stops being conditional.
        let val = gauge_value(*v, st.value_fmt);
        let u = unit_run(ctx, vface, vpx, &val);
        let vw = reading_w(ctx, vface, vpx, vtrack, &vfig, &val, &u);
        // The reading is drawn from its RIGHT edge whichever way it is
        // aligned — the run is laid out right to left because the unit
        // hangs off the number's end — so the alignment moves that edge
        // inside the value column rather than the pen.
        let right = align_in(gx + gw - value_w, value_w, vw, value_align) + vw;
        draw_reading_right(
            ctx, vface, vpx, vtrack, &vfig, right, vty, &val, text_c, None, &u,
        );
    }
}

/// `rhythm.label_align` / `rhythm.value_align` — how a run sits in the
/// column reserved for it.
///
/// The master declares `left | right` at both keys and this reads the
/// WORD, for the reason every other enum reader in the file does: an
/// index memoised across two keys names a different word in each. A word
/// the pair does not name is a defect in the theme and is said out loud
/// once, not silently taken for `left`.
///
/// TWO arms and not three: `center` is a word the master does not declare
/// at either key, and `tests/enum_vocabulary_declared.rs` is the reason a
/// theme writing it would not get it anyway. An arm for a word no theme
/// can legally spell is dead code held open for a look nobody asked for —
/// when a column wants centring, the master's line grows the word first.
fn rhythm_align(cell: &'static OnceLock<TokenId>, name: &'static str) -> Align {
    with_theme_word(tok(cell, name), |w| match w {
        "left" => Align::Left,
        "right" => Align::Right,
        other => {
            warn_once(name, &format!("{name} = {other} names no alignment — drawing it left"));
            Align::Left
        }
    })
}

/// Where a run of width `run` starts inside a column of width `col` that
/// begins at `x`.
fn align_in(x: f32, col: f32, run: f32, align: Align) -> f32 {
    match align {
        Align::Left => x,
        Align::Right => x + (col - run).max(0.0),
        Align::Center => x + (col - run).max(0.0) / 2.0,
    }
}

/// Horizontal alignment of a table column.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Align {
    Left,
    Right,
    Center,
}

/// How a table cell renders its string (u2 §3.1 #10). The string is the
/// content and is never changed by the kind — a bar or a badge is a second
/// reading of the same value, not a replacement.
#[derive(Clone, Copy, PartialEq)]
pub enum CellKind {
    Text,
    /// The numeric value also drawn as a hairline track filled to
    /// `value / of` — image 1's resource rows.
    Bar { of: f32 },
    /// The string drawn as a status pill carrying the row's severity.
    Badge,
}

/// Where a fixed column's width comes from (u2 §2.7). `Content` — the
/// widest actual cell, not the heading — is the default: measuring from
/// headings is what ellipsised every five-digit pid.
#[derive(Clone, Copy, PartialEq)]
pub enum ColWidth {
    Heading,
    Content,
}

/// One table column: its heading, and how its cells are laid and drawn.
pub struct Column {
    pub title: String,
    pub align: Align,
    pub kind: CellKind,
    pub width: ColWidth,
}

/// The script's arrangement choices for a `table`, plus the stack's
/// runtime shrink factor. Everything visual is read from the theme inside.
pub struct TableStyle {
    /// Index of the column that absorbs the leftover width.
    pub elastic: usize,
    /// Whether every Nth row is tinted (`script.table_zebra_every` is the
    /// N and `component.table.zebra` the tint; the script only says the
    /// striping makes sense for this data).
    pub zebra: bool,
    /// Index into each ROW at which the script placed a severity word for
    /// that row. The word is consumed as style, never drawn as a cell.
    pub severity_col: Option<usize>,
    pub shrink: f32,
}

/// The view riding on a table: what it remembers between frames, where
/// it records the rectangles it drew, and which of its interactions the
/// script turned on.
///
/// F2 §2.1 gives this struct three fields (`state`, `hits`, `id`); the
/// per-table OPTIONS live here too rather than in [`TableStyle`],
/// because `TableStyle` is the shape every existing caller builds by
/// hand and growing it would break them for a look that has not moved.
///
/// Every option is OFF in a table built this way with `Default`, and a
/// table drawn with all of them off draws what [`table`] draws, to the
/// pixel — the two share one implementation, which is the only way that
/// claim stays true.
pub struct TableView<'a> {
    pub state: &'a mut crate::view::table::TableState,
    pub hits: &'a mut crate::view::hits::Hits,
    /// Which view recorded a rectangle: one [`crate::view::hits::Hits`]
    /// may serve every view in a widget.
    pub id: u32,
    /// The model's rewrite counter (`Snapshot::generation`). The sort is
    /// cached against it; a caller with no generation of its own passes
    /// 0 and gets an order rebuilt only when the sort itself moves.
    pub generation: u64,
    /// Headings sort and answer the pointer.
    pub interactive: bool,
    /// Rows answer the pointer and one of them may be selected.
    pub select: bool,
    /// The column whose text identifies a row. `None`: the row's
    /// position in the model, which is all there is to go on.
    pub key_col: Option<usize>,
    /// Scroll the body instead of truncating it at the bottom edge.
    pub scroll: bool,
    /// A heading or a cell the ellipsis cut short explains itself when
    /// the pointer rests on it (F2 §8.1). Only what was TRIMMED asks:
    /// a tooltip repeating text already on screen is noise.
    pub tooltip: bool,
}

/// A table: one heading per column, then rows. The column marked
/// `elastic` absorbs the leftover width and is trimmed with an ellipsis;
/// everything else is measured from its content or its heading (u2 §2.7).
pub fn table(ctx: &mut Ctx, r: Rect, columns: &[Column], rows: &[Vec<String>], st: &TableStyle) {
    table_surface(&mut CtxSurface::new(ctx), r, columns, rows, st, None);
}

/// [`table`] with a view riding on it: an offset window instead of the
/// top-of-list truncation, a sorted and pointer-aware header, and the
/// selected row's `script.row` wash. With a default [`TableView`] it
/// draws exactly what [`table`] draws — same function, same branches.
pub fn table_view(
    ctx: &mut Ctx,
    r: Rect,
    columns: &[Column],
    rows: &[Vec<String>],
    st: &TableStyle,
    view: TableView,
) {
    table_surface(&mut CtxSurface::new(ctx), r, columns, rows, st, Some(view));
}

/// The table on any [`Surface`] — the ONE implementation [`table`] and
/// [`table_view`] both are, and the one a plugin reaches through
/// [`crate::view::surface::AbiSurface`].
///
/// The port from `Ctx` was mechanical on purpose: every
/// `t.px(tok(&CELL, "table.cell_pad"))` became `sf.px("table.cell_pad")`,
/// resolving the same token through the same engine, so the host's
/// pixels did not move. What it buys is that the interactive table
/// cannot exist twice — the fate `ui::fit_end_tracked` and the file
/// panel's `fit_name` already met.
///
/// Generic over [`TableModel`] for the same reason [`crate::view::list::
/// list`] is generic over `RowModel`: a table with a scrolled window
/// shows perhaps forty rows, and `rows: &[Vec<String>]` meant every
/// caller had already materialised every row's every cell before this
/// function ever ran, whatever fraction of them actually drew. `table`
/// and `table_view` below keep their own signatures — `rows: &[Vec<
/// String>]` — unchanged; that concrete type is what lets a
/// `Vec<Vec<String>>` caller (`src/script.rs`'s `table` builtin) keep
/// compiling without a single line moved, because `M` is inferred as
/// `[Vec<String>]` at their call into this function exactly as it always
/// resolved the rest of the signature.
pub fn table_surface<S: Surface, M: TableModel + ?Sized>(
    sf: &mut S,
    r: Rect,
    columns: &[Column],
    model: &M,
    st: &TableStyle,
    mut view: Option<TableView>,
) {
    if columns.is_empty() {
        return;
    }
    // Header and body each have a type role of their own, from the
    // `script.table_head_role` / `script.table_cell_role` bindings.
    let head_role = paint::bound_role(sf, "script.table_head_role", st.shrink);
    let cell_role = paint::bound_role(sf, "script.table_cell_role", st.shrink);
    let (head_px, head_track) = (head_role.px, head_role.track);
    let (cell_px, cell_track) = (cell_role.px, cell_role.track);
    // §5.16's `tabular`, carried beside px and track because a table is
    // the surface it was written for: `script.table_cell_role` binds a
    // column of pids, ports and percentages, and a seven-digit pid that
    // measures 33 px in one row and 35 px in the next drags its whole
    // column sideways as the process list turns over. The MEASURE below
    // reads it too — trimming against proportional widths and drawing
    // under a box would ellipsise a cell that fits.
    let (head_tab, cell_tab) = (head_role.tabular, cell_role.tabular);
    // And §5.16's `face`, carried the same way and for the same reason.
    // The master sets `script.table_cell_role = data` and `type.data.face
    // = mono`; this path drew the interface face regardless, so one role
    // came out in two families depending on which side of the library
    // reached the pixel.
    let (head_face, cell_face) = (head_role.face, cell_role.face);
    // The severity column is style, not content: the display columns are
    // the row's cells with that entry removed — but only when the row
    // actually carries the extra entry, so a script that declares the
    // column and then forgets the word loses nothing visible.
    let sev_slot = |row: &[String]| -> Option<usize> {
        match st.severity_col {
            Some(sc) if row.len() > columns.len() && sc < row.len() => Some(sc),
            _ => None,
        }
    };
    let cell_of = |slot: Option<usize>, i: usize| -> usize {
        match slot {
            Some(sc) if i >= sc => i + 1,
            _ => i,
        }
    };
    // The view, taken apart before anything is drawn: the display ORDER
    // is READ from the state while the hit list is WRITTEN to, and the
    // borrow checker is right to insist those are two different things.
    // Without a view every one of these is the "off" value, and every
    // branch below that tests one is not taken.
    let (
        mut state,
        mut hits,
        view_id,
        interactive,
        select,
        key_col,
        wants_scroll,
        generation,
        explain,
    ) = match view.take() {
        Some(v) => (
            Some(v.state),
            Some(v.hits),
            v.id,
            v.interactive,
            v.select,
            v.key_col,
            v.scroll,
            v.generation,
            v.tooltip,
        ),
        None => (None, None, 0, false, false, None, false, 0, false),
    };
    // `generation` here is `TableView::generation`, not `model.generation()`
    // — deliberately the one source of truth rather than two ways to say
    // the same thing. `TableModel::generation` exists for a model type
    // that carries its own snapshot counter; the two shapes this crate
    // ships (`[Vec<String>]`, `Vec<Vec<String>>`) are plain data with no
    // such counter and answer the trait's `0` default. `src/script.rs`'s
    // real generation (`host.snap.generation`) already had a place to
    // ride — `TableView::generation`, exactly as `refresh_order`'s
    // caching has always taken it — and a model that DOES track its own
    // generation loses nothing by having its caller pass that same
    // number into `TableView::generation` too, so this function only
    // ever reads it from the one place.

    // The model's row count, read once: every place below that used to
    // read `rows.len()` reads this instead, and it is the one call that
    // must stay cheap regardless of how the model answers `row` — a
    // lazy model still has to be able to COUNT its rows for free.
    let total = model.len();

    // The table spans its box: as many rows as fit after the header,
    // sharing the height exactly, starting at the top edge. Only when
    // the data runs out before the space does does it keep its natural
    // row height and leave the remainder empty — stretching four rows
    // over a tall panel would look like a fault rather than a table.
    let head_h = sf.px("table.head_h") * st.shrink;
    let natural_h = sf.px("table.row_h") * st.shrink;
    let fits = ((r.h - head_h).max(0.0) / natural_h.max(1.0)).floor() as usize;
    let shown = total.min(fits);
    let fitted_h = if shown >= fits && shown > 0 {
        (r.h - head_h) / shown as f32
    } else {
        natural_h
    };
    // The header block: the headings sit on the top edge, `head_gap`
    // above the rule, `head_gap_below` under it. `head_h` is what the
    // FIT arithmetic reserves for the header and is a different number
    // — that is how this function has always measured, and changing it
    // would move every table by a pixel.
    let head_gap = sf.px("table.head_gap") * st.shrink;
    let head_gap_below = sf.px("table.head_gap_below") * st.shrink;
    let body_y = r.y + head_gap + head_gap_below;
    let body_h = (r.bottom() - body_y).max(0.0);
    // A scrolled body keeps its natural row height: stretching the rows
    // to divide the box exactly is what a table does when it shows
    // everything it has, and it is meaningless once there is an offset.
    let scrolling = wants_scroll && body_h > 0.0;
    let row_h = if scrolling { natural_h } else { fitted_h };
    // A surface that cannot clip must not paint half a row outside its
    // box, so it scrolls by whole rows instead — the file panel's
    // behaviour, and the honest degradation of an old host.
    let can_clip = sf.can_clip();

    // The display order. The sort is the RENDERER's (F2 §2.1): the
    // script hands over rows in its own order and this decides which
    // one is shown where — rebuilt only when the model was rewritten or
    // the sort moved, never per frame.
    if let Some(s) = state.as_deref_mut() {
        let sc = s.sort.map(|(c, _)| c).unwrap_or(0);
        // `Fn`, not `FnMut` — `refresh_order` may call this once per row
        // of a sort — so the row buffer is a fresh `Vec` per call rather
        // than a reused one. That is only paid on the cold path this
        // closure already was: `refresh_order` itself skips the sort
        // entirely when its `OrderKey` still matches, so this runs once
        // per row per RESORT, not once per row per frame.
        s.refresh_order(generation, total, |i| {
            if i >= total {
                return String::new();
            }
            let mut cell_row = Vec::new();
            model.row(i, &mut cell_row);
            let slot = sev_slot(&cell_row);
            cell_row.get(cell_of(slot, sc)).cloned().unwrap_or_default()
        });
    }

    // The window of rows the body shows. Without scrolling it is the
    // top of the list, truncated where the box ends — today's `shown`,
    // expressed as a window so the drawing loop has one shape.
    let mut window = crate::view::virt::RowWindow { first: 0, count: shown, y0: 0.0 };
    let mut scroll_geom = None;
    let mut bar_look = None;
    if let Some(s) = state.as_deref_mut() {
        s.extent = crate::view::table::Extent {
            scrollable: scrolling,
            viewport: body_h,
            content: crate::view::virt::content_h(row_h, total),
            bar: None,
        };
    }
    if scrolling {
        let phys = crate::view::scroll::ScrollPhysics::read(sf);
        let look = crate::view::scroll::ScrollbarLook::read(sf);
        let now = sf.now();
        let mouse = sf.mouse();
        if let Some(s) = state.as_deref_mut() {
            let content = crate::view::virt::content_h(row_h, total);
            // A clipping surface leaves the offset free and a row may be
            // half visible; one that cannot snaps to whole rows.
            let snap = if can_clip {
                crate::view::Snap::None
            } else {
                crate::view::Snap::Row(row_h)
            };
            s.scroll.tick(now, body_h, content, snap, &phys);
            window = crate::view::virt::row_window(s.scroll.offset(), body_h, row_h, total);
            let area = Rect::new(r.x, body_y, r.w, body_h);
            // The band the bar could occupy at its WIDEST, on whichever
            // edge the theme puts it: a bar that grows under the pointer
            // must not shrink out from under it and start flickering.
            let reach = look.w_hover.max(look.w) + look.margin;
            let band = match look.edge {
                crate::view::scroll::ScrollbarEdge::Left => {
                    Rect::new(area.x, area.y, reach, area.h)
                }
                crate::view::scroll::ScrollbarEdge::Right => {
                    Rect::new(area.right() - reach, area.y, reach, area.h)
                }
            };
            let hovered = band.contains(mouse.0, mouse.1);
            scroll_geom = crate::view::scroll::scrollbar(
                area,
                &look,
                s.scroll.offset(),
                body_h,
                content,
                hovered || s.scroll.dragging(),
            );
            s.extent.bar = scroll_geom.as_ref().map(|g| (g.track, g.thumb));
            bar_look = Some((look, hovered));
        }
    }

    // From here on the state is only READ, which is what lets the order
    // be borrowed for the whole of the drawing below.
    let order: &[usize] = match state.as_deref() {
        Some(s) if s.order().len() == total => s.order(),
        _ => &[],
    };
    // `order[d]` when there is one, `d` when there is not: an identity
    // permutation is not worth a vector per frame.
    let model_of = |d: usize| -> usize { order.get(d).copied().unwrap_or(d) };
    let sort = state.as_deref().and_then(|s| s.sort);
    let pressed_head = state.as_deref().and_then(|s| s.pressed_head());
    let overrides: &[Option<f32>] = state.as_deref().map(|s| &s.widths[..]).unwrap_or(&[]);
    let selected_key: Option<&str> = state.as_deref().and_then(|s| s.selected.as_deref());
    let dragging_thumb = state.as_deref().is_some_and(|s| s.scroll.dragging());
    let now = sf.now();
    let bar_alpha = match (state.as_deref(), &bar_look) {
        (Some(s), Some((look, hovered))) => {
            if *hovered || dragging_thumb {
                1.0
            } else {
                s.scroll.fade_alpha(now, look.auto_hide, look.fade_ms)
            }
        }
        _ => 1.0,
    };

    // Fixed columns are measured from their WIDEST CELL (u2 §2.7), not
    // from their heading — measuring from headings is what made `PID` as
    // narrow as the word and ellipsised every five-digit pid. `Heading`
    // keeps the old rule for a column that asks for it; the elastic one
    // absorbs whatever is left either way.
    let col_gap = sf.px("table.col_gap") * st.shrink;
    let cell_pad = sf.px("table.cell_pad") * st.shrink;
    let bar_w = sf.px("table.bar_w") * st.shrink;
    let tokens = crate::view::table::TableTokens {
        col_gap,
        cell_pad,
        bar_w,
        // Raw, not shrunk — the asymmetry this function has always had.
        elastic_min_w: sf.px("table.elastic_min_w"),
        col_min_w: sf.px("table.col_min_w"),
    };
    // The rows the measure looks at: what the body is about to show.
    // Without a window that is `take(shown.max(1))`, exactly as before.
    let measured_span = if scrolling {
        window.first..window.first + window.count
    } else {
        0..shown.max(1).min(total.max(1))
    };
    // `TableState::cached_measure`'s key: the window bounds ride along
    // beside the model's own generation and length because the measure,
    // unlike the sort order, only ever looked at the rows ON SCREEN — a
    // row scrolling into view may be the widest the column has had all
    // along, so a moved window is as real a reason to remeasure as a
    // rewritten model.
    let width_key = crate::view::table::WidthKey {
        generation,
        len: total,
        cols: columns.len(),
        window_first: measured_span.start,
        window_count: measured_span.end - measured_span.start,
    };
    let cached_measured =
        state.as_deref().and_then(|s| s.cached_measure(width_key)).map(|m| m.to_vec());
    let measure_was_cached = cached_measured.is_some();
    let measured: Vec<crate::view::table::ColMeasure> = match cached_measured {
        Some(m) => m,
        None => {
            // Heads first, unconditionally — every column has one
            // whether or not it is `ColWidth::Content`.
            let mut measured: Vec<crate::view::table::ColMeasure> = columns
                .iter()
                .map(|c| {
                    let head = sf.measure_tab(head_face, head_px, &c.title, head_track, head_tab);
                    crate::view::table::ColMeasure {
                        head,
                        content: head,
                        bar: matches!(c.kind, CellKind::Bar { .. }),
                    }
                })
                .collect();
            // Rows on the OUTSIDE, columns on the inside: one
            // `model.row` call per row in the window rather than one per
            // (row, `Content`-column) pair. For a materialised
            // `Vec<Vec<String>>` the difference is nothing — an index is
            // an index — but for a model that FORMATS a row on demand
            // (§6's lazy test model) it is the difference between paying
            // for the window once and paying for it once per column.
            let mut cell_row: Vec<String> = Vec::new();
            for d in measured_span.clone() {
                let mi = model_of(d);
                if mi >= total {
                    continue;
                }
                model.row(mi, &mut cell_row);
                let slot = sev_slot(&cell_row);
                for (i, c) in columns.iter().enumerate() {
                    if c.width == ColWidth::Content && i != st.elastic {
                        if let Some(text) = cell_row.get(cell_of(slot, i)) {
                            let w =
                                sf.measure_tab(cell_face, cell_px, text, cell_track, cell_tab);
                            if w > measured[i].content {
                                measured[i].content = w;
                            }
                        }
                    }
                }
            }
            measured
        }
    };
    let widths = crate::view::table::solve_widths(&measured, r.w, st.elastic, overrides, &tokens);

    // Every column's width reserved `col_gap + cell_pad` beyond its
    // content, so every cell draws inside the CONTENT SPAN — a
    // right-aligned column ends a full gap before its neighbour instead
    // of touching it (u2 §2.7's `1471  firefox`, not `1471firefox`).
    // The TRIM budget keeps the cell_pad as headroom: a content-measured
    // column's widest cell measures exactly its own column, and trimming
    // at exactly its own width is a coin-toss on float rounding.
    let span = |w: f32| (w - col_gap - cell_pad).max(1.0);
    let trim_w = |w: f32| (w - col_gap).max(1.0);

    // The heading row, its rule, then the body.
    {
        let head_c = sf.color("component.table.head");
        let glyph = sf.px("table.sort_glyph") * st.shrink;
        let glyph_gap = sf.px("table.sort_glyph_gap") * st.shrink;
        let grip = sf.px("table.resize_grip") * st.shrink;
        let mouse = sf.mouse();
        let band_h = head_gap.max(0.0);
        let mut x = r.x;
        for (i, (c, w)) in columns.iter().zip(widths.iter()).enumerate() {
            let band = Rect::new(x, r.y, *w, band_h);
            let sorted = sort.map(|(sc, _)| sc) == Some(i);
            // The class ladder answers only for a heading the pointer
            // can actually reach; a table without `interactive` draws
            // the resting heading it has always drawn.
            let mut text_c = head_c;
            if interactive {
                let hovered = band.contains(mouse.0, mouse.1);
                let rung = match (pressed_head == Some(i), hovered, sorted) {
                    (true, _, _) => theme::parse::State::Press,
                    (_, true, true) => theme::parse::State::SelectedHover,
                    (_, true, false) => theme::parse::State::Hover,
                    (_, false, true) => theme::parse::State::Selected,
                    _ => theme::parse::State::Idle,
                };
                // The resting heading is the one this file has always
                // drawn: no band behind it, and the head role's own ink
                // in front. Handing that in as the idle rung is what
                // lets the wash fade out to nothing and the label fade
                // back to its resting colour, instead of snapping —
                // while a table nobody is pointing at keeps every pixel
                // it had.
                let rest = crate::view::surface::StateInk {
                    text: head_c,
                    ..crate::view::surface::StateInk::CLEAR
                };
                let style = sf.class_ink_resting("table.head", rung, band, rest);
                if style.fill.a > 0.0 {
                    sf.rect(band, style.fill);
                }
                if style.text.a > 0.0 {
                    text_c = style.text;
                }
                if let Some(h) = hits.as_deref_mut() {
                    h.push(band, crate::view::Hit::TableHead { id: view_id, col: i });
                    // The grip straddles the join, so both neighbours
                    // reach it; recorded AFTER the heading because the
                    // last rectangle drawn is the one that takes the
                    // press.
                    if grip > 0.0 && i + 1 < columns.len() {
                        h.push(
                            Rect::new(x + w - grip, r.y, grip * 2.0, band_h),
                            crate::view::Hit::TableDivider { id: view_id, col: i },
                        );
                    }
                }
            }
            // The sort marker takes its room out of the trim budget, so
            // a sorted heading is trimmed rather than overdrawn. It
            // reports the ORDER, so it is drawn whenever there is one —
            // a script that opened the table sorted says so even where
            // the user cannot re-sort it.
            let marker = if sorted { glyph + glyph_gap } else { 0.0 };
            let budget = (trim_w(*w) - marker).max(1.0);
            let cell_w = (span(*w) - marker).max(1.0);
            let text =
                paint::fit_end_tab(sf, head_face, head_px, &c.title, budget, head_track, head_tab);
            // A heading the ellipsis cut short finishes its sentence when
            // the pointer rests on it (F2 §8.1). The sort marker is part
            // of the budget, so a heading that fits until it is sorted
            // starts explaining itself the moment it is — which is the
            // frame in which the ellipsis actually reached it.
            if explain {
                paint::explain_trim(
                    sf,
                    crate::object::tooltip::cell_key(view_id, i, ""),
                    band,
                    &text,
                    &c.title,
                );
            }
            paint::cell_text_tab(
                sf, x, r.y, cell_w, c.align, head_face, head_px, &text, text_c, head_track,
                head_tab,
            );
            if marker > 0.0 {
                if let Some((_, dir)) = sort {
                    paint::sort_marker(sf, x + span(*w) - glyph, r.y, glyph, head_px, dir, text_c);
                }
            }
            x += w;
        }
    }
    let mut y = r.y + head_gap;
    let rule_w = sf.px("table.rule");
    let rule_c = sf.color("component.table.rule");
    sf.line(r.x, y, r.right(), y, rule_w, rule_c);
    y += head_gap_below;
    let row_c = sf.color("component.table.row");
    let zebra_c = sf.bed("component.table.zebra");
    let zebra_every = sf.px("script.table_zebra_every").max(0.0) as usize;
    let bar_h = sf.px("script.meter_bar_h") * st.shrink;
    let vgap = sf.px("meter.value_gap") * st.shrink;
    let mouse = sf.mouse();
    // A window that starts part-way down a row needs the body clipped,
    // or the first row paints over the rule above it.
    let clipped = scrolling && sf.clip(Rect::new(r.x, body_y, r.w, body_h));
    // Reused across the whole window rather than allocated per row — the
    // same buffer discipline `RowModel::row` asks of a list's reader,
    // and for the same reason: this loop runs every visible row, every
    // frame.
    let mut row_buf: Vec<String> = Vec::new();
    for d in window.first..window.first + window.count {
        let mi = model_of(d);
        if mi >= total {
            continue;
        }
        model.row(mi, &mut row_buf);
        let row: &[String] = &row_buf;
        let row_y = if scrolling { body_y + window.y_of(d, row_h) } else { y };
        let rect = Rect::new(r.x, row_y, r.w, row_h);
        // Zebra follows the DISPLAY position, not the loop counter, so
        // the stripes stay put while the body scrolls under them.
        if st.zebra && zebra_every > 0 && (d + 1) % zebra_every == 0 {
            sf.rect(rect, zebra_c);
        }
        let slot = sev_slot(row);
        // The row's identity: the key column's text, or its place in the
        // model when the script named none.
        let key = match key_col.and_then(|k| row.get(cell_of(slot, k))) {
            Some(k) => k.clone(),
            None => model_of(d).to_string(),
        };
        if select {
            let hovered = rect.contains(mouse.0, mouse.1)
                && mouse.1 >= body_y
                && mouse.1 < body_y + body_h;
            let chosen = selected_key == Some(key.as_str());
            let rung = match (chosen, hovered) {
                (true, true) => theme::parse::State::SelectedHover,
                (true, false) => theme::parse::State::Selected,
                (false, true) => theme::parse::State::Hover,
                _ => theme::parse::State::Idle,
            };
            // `script.row` — the class the master already declares for
            // "a selectable row a script widget draws". No new selection
            // colour exists, or needs to. A resting row wears the zebra
            // and nothing else, so its idle rung is CLEAR (list.rs's
            // reason, in full).
            let style = sf.class_ink_resting(
                "script.row",
                rung,
                rect,
                crate::view::surface::StateInk::CLEAR,
            );
            if style.fill.a > 0.0 {
                sf.rect(rect, style.fill);
            }
        }
        // Recorded whatever `select` says: a row rectangle is also how
        // the wheel finds out WHICH view the pointer is over, and a
        // table that scrolls without selecting still has to answer that.
        if let Some(h) = hits.as_deref_mut() {
            h.push(rect, crate::view::Hit::Row { id: view_id, key: key.clone() });
        }
        let sev = match slot.and_then(|sc| row.get(sc)) {
            Some(w) => Some(match sev_of(w) {
                Some(s) => s,
                None => paint::sev_fallback(sf),
            }),
            None => None,
        };
        let color = match sev {
            Some(s) => paint::sev_text(sf, s),
            None => row_c,
        };
        let mut x = r.x;
        for (i, (c, w)) in columns.iter().zip(widths.iter()).enumerate() {
            let Some(text) = row.get(cell_of(slot, i)) else {
                x += w;
                continue;
            };
            match c.kind {
                CellKind::Text => {
                    let shown =
                        paint::fit_end_tab(
                            sf, cell_face, cell_px, text, trim_w(*w), cell_track, cell_tab,
                        );
                    // The elastic column is the one the ellipsis usually
                    // reaches, but any column can be cut short by a
                    // dragged width, so the test is what HAPPENED rather
                    // than which column it was.
                    // The vertical test first: a row half outside a
                    // scrolled window is half not there, and answering
                    // for the half that is clipped away would explain
                    // something nobody can see.
                    if explain && mouse.1 >= body_y && mouse.1 < body_y + body_h {
                        paint::explain_trim(
                            sf,
                            crate::object::tooltip::cell_key(view_id, i, &key),
                            Rect::new(x, row_y, *w, row_h),
                            &shown,
                            text,
                        );
                    }
                    paint::cell_text_tab(
                        sf, x, row_y, span(*w), c.align, cell_face, cell_px, &shown, color,
                        cell_track, cell_tab,
                    );
                }
                CellKind::Bar { of } => {
                    // The number is unchanged; the track behind it is a
                    // second reading of the same value (u2 §2.7).
                    let tw = sf.measure_tab(cell_face, cell_px, text, cell_track, cell_tab);
                    let avail = (span(*w) - tw - vgap).max(0.0).min(bar_w);
                    if avail > 1.0 && of > 0.0 {
                        let v = paint::leading_number(text).unwrap_or(0.0);
                        let bar = Rect::new(
                            x + span(*w) - tw - vgap - avail,
                            row_y + (row_h - bar_h).max(0.0) / 2.0,
                            avail,
                            bar_h.min(row_h),
                        );
                        paint::meter(sf, bar, v / of, sev, true);
                    }
                    sf.text_tab(
                        cell_face,
                        cell_px,
                        x + span(*w),
                        row_y,
                        text,
                        color,
                        cell_track,
                        Align::Right,
                        cell_tab,
                    );
                }
                CellKind::Badge => {
                    paint::badge(
                        sf,
                        Rect::new(x, row_y, span(*w), row_h),
                        text,
                        sev,
                        BadgeStyle::FromTheme,
                        c.align,
                        st.shrink,
                    );
                }
            }
            x += w;
        }
        if !scrolling {
            y += row_h;
        }
    }
    if clipped {
        sf.unclip();
    }
    // The bar last (u2 §2.10), over the rows it covers — which is why
    // its rectangles are recorded last too: the pointer points at what
    // it can see.
    if let (Some(geom), Some((_, hovered))) = (scroll_geom, bar_look) {
        paint::scrollbar(sf, &geom, bar_alpha, hovered, dragging_thumb);
        if let Some(h) = hits.as_deref_mut() {
            let mid = geom.thumb.y + geom.thumb.h / 2.0;
            h.push(
                Rect::new(geom.track.x, geom.track.y, geom.track.w, mid - geom.track.y),
                crate::view::Hit::Track { id: view_id, toward_end: false },
            );
            h.push(
                Rect::new(geom.track.x, mid, geom.track.w, geom.track.bottom() - mid),
                crate::view::Hit::Track { id: view_id, toward_end: true },
            );
            h.push(geom.thumb, crate::view::Hit::Thumb { id: view_id });
        }
    }

    // The measurement this frame settled on, kept for the next one — only
    // when it was actually recomputed: a hit that just re-stores what was
    // already there buys nothing. `order`, `overrides` and `selected_key`
    // have made their last read by here, which is what frees `state` for
    // a second, mutable borrow this late in the function.
    if !measure_was_cached {
        if let Some(s) = state.as_deref_mut() {
            s.set_width_cache(width_key, measured);
        }
    }
}

/// One cell of a `columns` strip: a small label, a larger value, and the
/// script's judgement of the value (u2 §2.2's POWER severity).
pub struct ColumnCell {
    pub label: String,
    pub value: String,
    pub sev: Option<Sev>,
}

/// A `columns` strip's arrangement. Roles come through the caller from the
/// `script.columns_*_role` bindings (or the script's own naming); align of
/// `None` defers to the theme's `columns.align`.
pub struct ColumnsStyle {
    pub label_role: Role,
    pub value_role: Role,
    pub align: Option<Align>,
    /// Hairline dividers between cells — arrangement furniture the script
    /// opts into; `columns.divider` and its colour decide the look.
    pub dividers: bool,
    pub shrink: f32,
}

/// Columns of a small label above a larger value — the shape used for
/// at-a-glance readouts. Cells are sized by their CONTENT, the leftover
/// shared evenly: the images' strips are runs of values separated by
/// dividers (u2 §2.2, image 7's pipes), not equal thirds — equal thirds
/// is how a long date ends in an ellipsis while `AC` hoards a third of
/// the strip. The images divide and align; whether this strip does
/// either is `st`.
pub fn columns(ctx: &mut Ctx, r: Rect, cells: &[ColumnCell], st: &ColumnsStyle) {
    if cells.is_empty() {
        return;
    }
    static BLOCK: OnceLock<TokenId> = OnceLock::new();
    static LABEL_GAP: OnceLock<TokenId> = OnceLock::new();
    static GUTTER: OnceLock<TokenId> = OnceLock::new();
    static ALIGN: OnceLock<TokenId> = OnceLock::new();
    static DIVIDER: OnceLock<TokenId> = OnceLock::new();
    static DIVIDER_INSET: OnceLock<TokenId> = OnceLock::new();
    static LABEL_C: OnceLock<TokenId> = OnceLock::new();
    static VALUE_C: OnceLock<TokenId> = OnceLock::new();
    static DIVIDER_C: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let natural = (t.px(tok(&BLOCK, "script.columns_block")) * st.shrink).min(r.h);
    let lp = st.label_role.px(ctx, st.shrink);
    let vp = st.value_role.px(ctx, st.shrink);
    let ltrack = st.label_role.tracking_px(lp);
    let vtrack = st.value_role.tracking_px(vp);
    // Face and figure box, resolved once and carried into the cell loop —
    // the same pair `rows` resolves, because a `columns` value is the same
    // KIND of thing as a `rows` value and the master binds both to
    // `value`. Until this line the strip drew FONT_UI proportionally, so
    // SYSINFO's clock crept 8 px sideways between `11:11:11` and
    // `88:88:88` while the comment in the script promised it would not.
    let lface = st.label_role.font();
    let vface = st.value_role.font();
    let lfig = st.label_role.figures(ctx.fonts, lface, lp);
    let vfig = st.value_role.figures(ctx.fonts, vface, vp);
    let vtab = st.value_role.tabular();
    // The baseline step from label to value is the shape itself, so it
    // has a token of its own rather than riding on the label's size.
    let vgap = t.px(tok(&LABEL_GAP, "columns.label_gap")) * st.shrink;
    let gutter = t.px(tok(&GUTTER, "rhythm.value_gutter")) * st.shrink;
    let label_c = col(&LABEL_C, "component.columns.label");
    let value_c = col(&VALUE_C, "component.columns.value");
    let align = st.align.unwrap_or_else(|| {
        match word_of(tok(&ALIGN, "columns.align")).as_str() {
            "left" => Align::Left,
            "right" => Align::Right,
            _ => Align::Center,
        }
    });
    // Each cell's natural width, then the leftover shared evenly. An
    // overfull strip shrinks every cell evenly instead, and `fit_end`
    // trims inside the cell as before.
    let widths: Vec<f32> = {
        let nat: Vec<f32> = cells
            .iter()
            .map(|cell| {
                // Measured under the box the cell is DRAWN under, so a
                // content-sized strip is sized at the width its readings
                // really occupy.
                let lw = ctx.fonts.measure_fig(lface, lp, &cell.label, ltrack, &lfig);
                let vw = ctx.fonts.measure_fig(vface, vp, &cell.value, vtrack, &vfig);
                lw.max(vw) + 2.0 * gutter
            })
            .collect();
        let extra = (r.w - nat.iter().sum::<f32>()) / cells.len() as f32;
        nat.into_iter().map(|w| (w + extra).max(1.0)).collect()
    };
    let y = block_top(&r, natural);
    let mut x0 = r.x;
    for (i, (cell, cw)) in cells.iter().zip(widths.iter().copied()).enumerate() {
        let vc = cell.sev.map(sev_text).unwrap_or(value_c);
        let shown =
            fit_end_tracked_tab(ctx, vface, vp, &cell.value, cw - 2.0 * gutter, vtrack, vtab);
        match align {
            Align::Center => {
                let cx = x0 + cw / 2.0;
                ctx.dl.text_center_fig(
                    ctx.fonts, lface, lp, cx, y, &cell.label, label_c, ltrack, &lfig,
                );
                ctx.dl.text_center_fig(
                    ctx.fonts, vface, vp, cx, y + vgap, &shown, vc, vtrack, &vfig,
                );
            }
            Align::Left => {
                let cx = x0 + gutter;
                ctx.dl
                    .text_fig(ctx.fonts, lface, lp, cx, y, &cell.label, label_c, ltrack, &lfig);
                ctx.dl
                    .text_fig(ctx.fonts, vface, vp, cx, y + vgap, &shown, vc, vtrack, &vfig);
            }
            Align::Right => {
                let cx = x0 + cw - gutter;
                ctx.dl.text_right_fig(
                    ctx.fonts, lface, lp, cx, y, &cell.label, label_c, ltrack, &lfig,
                );
                ctx.dl.text_right_fig(
                    ctx.fonts, vface, vp, cx, y + vgap, &shown, vc, vtrack, &vfig,
                );
            }
        }
        // The strip is sized from its CONTENT, so a cell is trimmed only
        // when the whole strip is overfull — and then the reading, which
        // is the entire point of the cell, is the part that was cut. The
        // anchor is the cell's whole column, label included: label and
        // value are one readout, and the user points at the readout.
        let id = crate::object::tooltip::cell_key(0, i, &cell.label);
        explain_trim(ctx, id, Rect::new(x0, r.y, cw, r.h), &shown, &cell.value);
        x0 += cw;
    }
    if st.dividers {
        let stroke = t.px(tok(&DIVIDER, "columns.divider"));
        if stroke > 0.0 {
            let inset = t.px(tok(&DIVIDER_INSET, "columns.divider_inset")) * st.shrink;
            let c = col(&DIVIDER_C, "component.columns.divider");
            // On the boundary between two cells, wherever content sizing
            // put it.
            let mut x = r.x;
            for w in widths.iter().take(cells.len() - 1) {
                x += w;
                ctx.dl
                    .line(x, r.y + inset, x, r.bottom() - inset, stroke, c);
            }
        }
    }
}

// ---------------------------------------------------------------- runs

/// One styled run of a `runs` line (u2 §3.1 #3): its text, its role, the
/// script's severity judgement, and the id of the `motion.*` effect that
/// drives its alpha — never its glyph, so the advance holds and the line
/// cannot jitter (I13).
pub struct Run {
    pub text: String,
    pub role: Role,
    pub sev: Option<Sev>,
    pub blink: Option<String>,
    /// Drawn flush to the line's RIGHT edge, after every start run — u2
    /// §2.5's right-aligned temperature. An arrangement flag, not a look.
    pub end: bool,
}

/// One line of styled runs, aligned as a unit. Sizes may differ between
/// runs; their em boxes are bottom-aligned, which is the closest thing to
/// a shared baseline the draw list can do until the cap-height primitive
/// lands (F021). Runs marked `end` form a trailing cluster on the line's
/// right edge; the rest align as one unit in the room that cluster leaves
/// (u2 §2.5's LOAD line). Returns the width drawn.
pub fn runs(ctx: &mut Ctx, r: Rect, items: &[Run], align: Align, shrink: f32) -> f32 {
    if items.is_empty() {
        return 0.0;
    }
    static VALUE_C: OnceLock<TokenId> = OnceLock::new();
    // Each run in ITS OWN role's face, at its own size, under its own
    // figure box — resolved once here and carried into the draw loop, so a
    // run is measured by the same rule it is drawn by.
    //
    // This line is the desktop clock. `clock.rhai` draws the time through
    // `runs` and nothing else, its runs name `display.clock` (tabular in
    // the master), and this function measured and drew FONT_UI
    // proportionally: `11:11:11` came to 135.61 px and `88:88:88` to
    // 182.25 px, so a centred clock jumped 22 px sideways the moment a 1
    // appeared in the time. The comment in the script said the tabular
    // role was what stopped that happening. It is now.
    let sized: Vec<(f32, f32, f32, u8, Figures)> = items
        .iter()
        .map(|run| {
            let px = run.role.px(ctx, shrink);
            let track = run.role.tracking_px(px);
            let face = run.role.font();
            let fig = run.role.figures(ctx.fonts, face, px);
            let w = ctx.fonts.measure_fig(face, px, &run.text, track, &fig);
            (px, track, w, face, fig)
        })
        .collect();
    // The gap between two neighbouring runs. It used to be faked with
    // literal spaces inside the scripts themselves — which put a piece of
    // the layout in the content, at whatever width the font gave a space.
    static RUNS_GAP: OnceLock<TokenId> = OnceLock::new();
    let gap = theme::resolved().px(tok(&RUNS_GAP, "script.runs_gap")) * shrink;
    let cluster = |end: bool| -> f32 {
        let ws: Vec<f32> = items
            .iter()
            .zip(&sized)
            .filter(|(run, _)| run.end == end)
            .map(|(_, (_, _, w, _, _))| *w)
            .collect();
        ws.iter().sum::<f32>() + gap * ws.len().saturating_sub(1) as f32
    };
    let start_w = cluster(false);
    let end_w = cluster(true);
    let max_px = sized.iter().map(|(px, _, _, _, _)| *px).fold(0.0, f32::max);
    // The start cluster aligns in the room the end cluster leaves.
    let room = r.w - end_w;
    let mut x = match align {
        Align::Left => r.x,
        Align::Center => r.x + (room - start_w) / 2.0,
        Align::Right => r.x + room - start_w,
    };
    let mut ex = r.right() - end_w;
    let fallback = col(&VALUE_C, "component.script.value");
    for (run, (px, track, w, face, fig)) in items.iter().zip(sized.iter()) {
        let mut c = run.sev.map(sev_text).unwrap_or_else(|| {
            let rc = run.role.color();
            // A role with no ink of its own (empty theme) still shows.
            if rc.a > 0.0 { rc } else { fallback }
        });
        if let Some(id) = &run.blink {
            c.a *= blink_factor(id, ctx.t);
        }
        // Bottom-aligned em boxes stand in for the shared baseline.
        let y = r.y + (max_px - px);
        let cursor = if run.end { &mut ex } else { &mut x };
        ctx.dl.text_fig(ctx.fonts, *face, *px, *cursor, y, &run.text, c, *track, fig);
        *cursor += w + gap;
    }
    start_w + end_w
}

// ---------------------------------------------------------------- badge

/// How a badge is filled. `FromTheme` asks `badge.style_from_severity`
/// and the severity's own `badge_style`; the script may insist on solid
/// or hollow, which is arrangement, not colour. `hatched` and
/// `hollow_dashed` degrade to hollow until the renderer can draw them.
#[derive(Clone, Copy, PartialEq)]
pub enum BadgeStyle {
    FromTheme,
    Solid,
    Hollow,
}

/// The CRITICAL / CONTAINED pill of images 1, 3 and 4 (u2 §3.1 #11): a
/// filled, ringed capsule around a short text, its four colours from the
/// severity at draw time. The pill's corner is the theme's, both halves
/// of it: `badge.corner` for the radius — `pill` included, which is what
/// makes the capsule this thing is named after — and the style slot of
/// `shape.badge.corners` for the cut. Returns the pill width.
pub fn badge(
    ctx: &mut Ctx,
    r: Rect,
    text: &str,
    sev: Option<Sev>,
    style: BadgeStyle,
    align: Align,
    shrink: f32,
) -> f32 {
    paint::badge(&mut CtxSurface::new(ctx), r, text, sev, style, align, shrink)
}

// ---------------------------------------------------------------- rule

/// A horizontal hairline as a stack element in its own right (u2 §3.1
/// #12) — until now the only rule in the vocabulary was welded to `title`.
/// Drawn across the middle of `r`; the stroke does not shrink with the
/// stack, a hairline being a hairline at every scale.
pub fn rule(ctx: &mut Ctx, r: Rect) {
    static W: OnceLock<TokenId> = OnceLock::new();
    static C: OnceLock<TokenId> = OnceLock::new();
    let stroke = theme::resolved().px(tok(&W, "script.rule_width"));
    if stroke <= 0.0 {
        return;
    }
    let y = r.y + r.h / 2.0;
    ctx.dl
        .line(r.x, y, r.right(), y, stroke, col(&C, "component.script.rule"));
}

// ---------------------------------------------------------------- group

/// A `group`'s caption line with its optional rule (u2 §3.1 #13): a
/// section label in `script.group_label_role`, and — when
/// `script.group_rule` says so — a hairline along the bottom edge of the
/// header's box. The nested elements are the caller's to draw below.
pub fn group_header(ctx: &mut Ctx, r: Rect, label: &str, shrink: f32) {
    static ROLE: OnceLock<TokenId> = OnceLock::new();
    static RULE_W: OnceLock<TokenId> = OnceLock::new();
    static LABEL_C: OnceLock<TokenId> = OnceLock::new();
    static RULE_C: OnceLock<TokenId> = OnceLock::new();
    let t = theme::resolved();
    let role = bound_role(&ROLE, "script.group_label_role");
    let px = role.px(ctx, shrink);
    let track = role.tracking_px(px);
    // The group caption is a section label like any other, so it is set in
    // the face its role names — a header is not a place the toolkit gets
    // to pick a family.
    let face = role.font();
    let ty = center_line_y(ctx, face, r.y, r.h, px, role.leading());
    let fig = role.figures(ctx.fonts, face, px);
    ctx.dl.text_fig(
        ctx.fonts, face, px, r.x, ty, label,
        col(&LABEL_C, "component.script.label"), track, &fig,
    );
    let stroke = t.px(tok(&RULE_W, "script.group_rule"));
    if stroke > 0.0 {
        let y = r.bottom() - stroke / 2.0;
        ctx.dl
            .line(r.x, y, r.right(), y, stroke, col(&RULE_C, "component.script.rule"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::draw::DrawList;
    use crate::font::FontSystem;
    use crate::pointer::Pointer;

    // ------------------------------------------------------------- case

    #[test]
    fn a_word_the_list_does_not_hold_transforms_nothing() {
        assert_eq!(Case::from_word("none"), Case::None);
        assert_eq!(Case::from_word("upper"), Case::Upper);
        assert_eq!(Case::from_word("lower"), Case::Lower);
        assert_eq!(Case::from_word("smallcaps"), Case::SmallCaps);
        // The finding this replaced: five copies of the transform, all
        // of them ending on `_ => to_uppercase()`, so a theme with
        // `uper` — or `Upper`, or a word from a future master — got
        // capitals and no diagnostic. A transform nobody named is no
        // transform.
        assert_eq!(Case::from_word("uper"), Case::None);
        assert_eq!(Case::from_word("Upper"), Case::None);
        assert_eq!(recase(Case::from_word("uper"), "Save"), "Save");
        // An EMPTY word is a missing token, which the reader that could
        // not find it has already reported; it is not a typo in a case
        // key and must not be reported as one.
        assert_eq!(Case::from_word(""), Case::None);
    }

    #[test]
    fn the_applier_borrows_when_it_has_nothing_to_do() {
        // `Case::None` is what most roles ask for and what every typo now
        // answers, so it is the path the master objects to allocating on:
        // "a caller that folds case on its own side allocates a String
        // per label per frame".
        assert!(matches!(recase(Case::None, "Save"), Cow::Borrowed("Save")));
        assert_eq!(recase(Case::Upper, "Save"), "SAVE");
        assert_eq!(recase(Case::Lower, "Save"), "save");
        // Smallcaps is drawn as capitals until the font layer can set
        // true small caps — stated once, here, instead of in five files.
        assert_eq!(recase(Case::SmallCaps, "Save"), "SAVE");
    }

    #[test]
    fn a_role_carries_the_case_the_master_gives_it() {
        // The master: `type.button.case = upper`, `type.body.case = none`.
        assert_eq!(role("button").case(), Case::Upper);
        assert_eq!(role("body").case(), Case::None);
        assert_eq!(role("title.panel").case(), Case::SmallCaps);
        assert_eq!(role("button").cased("Save"), "SAVE");
        assert_eq!(role("body").cased("Save"), "Save");
        // A role the master does not declare asks for nothing: shouting
        // at an undesigned run would be this file choosing a look.
        assert_eq!(role("no.such.role").case(), Case::None);
    }

    /// A frame's worth of context, at the reference viewport and with no
    /// scaling of its own, so a measured px is the theme's alone.
    fn ctx<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem) -> Ctx<'a> {
        Ctx {
            dl,
            fonts,
            w: 1920.0,
            h: 1080.0,
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
    fn a_role_the_master_does_not_declare_measures_nothing_and_inks_nothing() {
        let mut dl = DrawList::new();
        let mut fonts = FontSystem::new();
        let c = ctx(&mut dl, &mut fonts);

        let body = role("body");
        assert!(body.px(&c, 1.0) > 0.0);
        assert!(body.leading() > 0.0);
        assert!(body.color().a > 0.0);

        // This used to answer body's own ladder: a legible, plausible,
        // wrong interface that a release would have walked straight past.
        let ghost = role("no.such.role.the.master.declares");
        assert_eq!(ghost.px(&c, 1.0), 0.0);
        assert_eq!(ghost.leading(), 0.0);
        assert_eq!(ghost.color().a, 0.0);
    }

    #[test]
    fn a_binding_picks_the_ladder_and_a_binding_that_names_nothing_draws_nothing() {
        static ITEM: OnceLock<TokenId> = OnceLock::new();
        static TITLE: OnceLock<TokenId> = OnceLock::new();
        static ABSENT: OnceLock<TokenId> = OnceLock::new();
        let mut dl = DrawList::new();
        let mut fonts = FontSystem::new();
        let c = ctx(&mut dl, &mut fonts);

        // Two bindings, two words, two sizes on screen: the word inside the
        // token is what picks the ladder, so repointing it repaints.
        let item = bound_role(&ITEM, "menu.item.role");
        let title = bound_role(&TITLE, "winframe.title.role");
        assert!(item.px(&c, 1.0) > 0.0);
        assert!(title.px(&c, 1.0) != item.px(&c, 1.0));

        // A binding the master does not declare names no role at all, and
        // an unnamed role is a defect to report rather than a look to pick.
        let none = bound_role(&ABSENT, "no.such.binding.in.the.master");
        assert_eq!(none.px(&c, 1.0), 0.0);
        assert_eq!(none.leading(), 0.0);
        assert_eq!(none.color().a, 0.0);
    }

    /// There are TWO role resolvers in this library — this file's, for
    /// objects drawing against [`Ctx`], and [`paint::bound_role`], for
    /// every view, every script table and the whole ABI side drawing
    /// against [`Surface`] — and they are one ruling, not two. A fix
    /// applied to one of them reads as done and leaves the other half of
    /// the program painting a `body` ladder behind a name nobody wrote,
    /// which is a worse state than fixing neither: it is invisible. So
    /// the two are pinned to each other here, on the bindings the
    /// SHIPPED master declares, and a divergence fails a test rather
    /// than a screen.
    #[test]
    fn the_two_resolvers_answer_one_binding_the_same_way() {
        // One cell per binding: `bound_role` memoises the id in it, the
        // same way its callers do at module scope.
        static CELLS: [OnceLock<TokenId>; 6] = [
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
            OnceLock::new(),
        ];
        // Both halves' own bindings: the first two are read through this
        // file, the rest through `paint` — a list of one side's only
        // would prove the resolvers agree where nobody looks.
        const BINDINGS: [&str; 6] = [
            "menu.item.role",
            "winframe.title.role",
            "list.label_role",
            "script.table_head_role",
            "tab.role",
            "segmented.role",
        ];

        let mut dl = DrawList::new();
        let mut fonts = FontSystem::new();
        let mut c = ctx(&mut dl, &mut fonts);
        for (cell, binding) in CELLS.iter().zip(BINDINGS) {
            let here = bound_role(cell, binding);
            let px = here.px(&c, 1.0);
            let (track, leading, color) = (here.tracking_px(px), here.leading(), here.color());
            assert!(px > 0.0, "{binding} draws nothing: the master lost a binding");
            let mut sf = CtxSurface::new(&mut c);
            let there = paint::bound_role(&mut sf, binding, 1.0);
            assert_eq!(px, there.px, "{binding}");
            assert_eq!(track, there.track, "{binding}");
            assert_eq!(leading, there.leading, "{binding}");
            assert_eq!(color.a, there.color.a, "{binding}");
            // The CASE too, now that both sides carry it: two answers to
            // "does this label shout" is exactly the drift the rest of
            // this loop exists to catch.
            assert_eq!(here.case(), there.case, "{binding}");
            assert_eq!(here.cased("Save"), there.cased("Save"), "{binding}");
        }

        // And the hole is the same hole on both sides. Measured against
        // the SHIPPED theme rather than a fixture, because the thing
        // that used to put the hole back on screen was the global floor:
        // `type.min_px` is 8 px here, and a role that does not exist has
        // no size for it to lift.
        let mut sf = CtxSurface::new(&mut c);
        assert!(
            sf.px("type.min_px") > 0.0,
            "with no floor in the theme this proves nothing"
        );
        let ghost = paint::role_look(&mut sf, "no.such.role.the.master.declares", 1.0);
        assert_eq!(ghost.px, 0.0);
        assert_eq!(ghost.leading, 0.0);
        assert_eq!(ghost.color.a, 0.0);
        let ghost = paint::bound_role(&mut sf, "no.such.binding.in.the.master", 1.0);
        assert_eq!(ghost.px, 0.0);
        assert_eq!(ghost.leading, 0.0);
        assert_eq!(ghost.color.a, 0.0);
    }

    // ----------------------------------------------------------- table

    /// A table with no `Vec<Vec<String>>` behind it at all: every cell is
    /// FORMATTED from the row index the moment `row` is asked for it,
    /// the shape a live `/proc` read or a ring-buffer log tail actually
    /// takes. `calls` counts every `row` call, so the test below can ask
    /// the one question the whole of this change exists to answer: does
    /// drawing this through a small window touch the model roughly
    /// `window.count` times, or does it — as `rows: &[Vec<String>]`
    /// always forced — touch it `len()` times regardless of what fits
    /// on screen.
    struct LazyRows {
        total: usize,
        calls: std::cell::Cell<usize>,
    }

    impl TableModel for LazyRows {
        fn len(&self) -> usize {
            self.total
        }

        fn cols(&self) -> usize {
            2
        }

        fn row(&self, index: usize, out: &mut Vec<String>) {
            self.calls.set(self.calls.get() + 1);
            out.clear();
            if index < self.total {
                out.push(index.to_string());
                out.push(format!("item-{index}"));
            }
        }
    }

    #[test]
    fn a_lazy_model_is_touched_about_once_per_visible_row_never_once_per_row_it_has() {
        let mut dl = DrawList::new();
        let mut fonts = FontSystem::new();
        let mut c = ctx(&mut dl, &mut fonts);
        let cols = vec![
            Column { title: "IDX".into(), align: Align::Right, kind: CellKind::Text, width: ColWidth::Content },
            Column { title: "NAME".into(), align: Align::Left, kind: CellKind::Text, width: ColWidth::Content },
        ];
        let st = TableStyle { elastic: 1, zebra: false, severity_col: None, shrink: 1.0 };
        let model = LazyRows { total: 100_000, calls: std::cell::Cell::new(0) };
        let mut state = crate::view::table::TableState::new();
        let mut hits = crate::view::Hits::new();
        let r = Rect::new(0.0, 0.0, 300.0, 200.0);

        // The exact window `table_surface` itself will compute, worked
        // out here through the SAME public arithmetic
        // (`view::virt::row_window`) rather than guessed at — a fresh
        // `TableState`'s scroll offset starts at 0, so this is the
        // window the draw below settles on too.
        let (row_h, head_gap, head_gap_below) = {
            let mut probe = CtxSurface::new(&mut c);
            (probe.px("table.row_h"), probe.px("table.head_gap"), probe.px("table.head_gap_below"))
        };
        let body_h = (r.h - head_gap - head_gap_below).max(0.0);
        let expected = crate::view::virt::row_window(0.0, body_h, row_h, model.total);
        assert!(
            expected.count > 0 && expected.count < model.total,
            "the test window ({}) must show some rows but not all {} of them",
            expected.count,
            model.total
        );

        table_surface(
            &mut CtxSurface::new(&mut c),
            r,
            &cols,
            &model,
            &st,
            Some(TableView {
                state: &mut state,
                hits: &mut hits,
                id: 1,
                generation: 0,
                interactive: false,
                select: false,
                key_col: None,
                scroll: true,
                tooltip: false,
            }),
        );

        let calls = model.calls.get();
        assert!(calls > 0, "the model was never asked for a row");
        assert!(
            calls < model.total,
            "{calls} calls to draw a window of {}: every one of {} rows was touched, which is \
             not virtualisation",
            expected.count,
            model.total
        );
        // Two passes over the window and nothing else: the content-width
        // measure, then the draw. No sort was requested, so
        // `refresh_order` never reaches the model at all — this is the
        // "or close to it" the row count allows for, named exactly.
        assert_eq!(
            calls,
            2 * expected.count,
            "expected one measure pass and one draw pass over the {}-row window",
            expected.count
        );
    }
}
