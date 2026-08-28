//! The hand-written recursive-descent parser for the `.theme` grammar (§3.2).
//!
//! It produces a [`Document`] and **evaluates nothing**: every value is an
//! unevaluated [`Expr`] carrying its source span, exactly as §2.2's pipeline
//! step 1 requires. Type checking against a token's declared type happens in
//! `cascade.rs`, because that is the first stage that knows what a token *is*.
//!
//! It is hand-written rather than `toml`-crate-driven for the three reasons §3.1
//! gives: the crate cannot type-check colour expressions, `@` references, units
//! or `extends`; it produces generic errors without the `file:line:col` the
//! diagnostics need; and the grammar here is deliberately *smaller* than TOML's.
//!
//! ### Rules the EBNF only implies, all enforced here
//!
//! * Quotes are optional everywhere and mandatory only for `text`. A quoted
//!   value is kept as [`Expr::Text`] and **re-lexed to the target token's type**
//!   by [`relex`], which `cascade.rs` calls once the type is known; the author
//!   gets a *note*, not a warning (§4.2's last row).
//! * Units are mandatory on every length. A bare number parses as
//!   [`Expr::Num`] and becomes a diagnostic where a length was wanted.
//! * `px` is legal **only** on a token whose name ends `_min_px` / `_max_px`.
//!   That is checkable from the key alone, so it is checked here.
//! * One `@` sigil for every reference at every layer, including `@grad.focus`
//!   ([CONFLICT 1]).
//! * `/ a` is the one alpha form beyond `#RRGGBBAA` ([CONFLICT 11]), desugared
//!   to `alpha(x, a)` on the spot.
//! * A ratio's right side is a reference. A bare path is accepted for one
//!   release and warned.
//! * `%` is also an accepted spelling for a `frac` token.
//! * Dotted keys inside a *plain* section concatenate; inside `[mood.*]` /
//!   `[variant.*]` **every key is an absolute token path** (§3.2, §5.24).
//! * Indexed keys (`term.ansi[4]`) and locale-suffixed keys (`name[pt_BR]`).
//! * `@include` is depth-capped at 4 and may not escape the theme's directory.
//! * An unknown key carries a Levenshtein suggestion ([`suggest`]).

use super::expr::{Expr, Func, Unit};
use super::color::Color;
use std::path::{Component, Path, PathBuf};

// ------------------------------------------------------------------- sources

pub type FileId = u32;

/// `file:line:col` plus the length of the offending text, so §4.3 can print the
/// source line with a caret under the span. Throwing the span away at the print
/// site is the one place this engine would be stingy for no reason.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub file: FileId,
    pub line: u32,
    pub col: u32,
    pub len: u32,
}

/// Every file that contributed to a load, kept whole so diagnostics can quote
/// the offending line. Shared across `default.theme`, the selected theme, its
/// `@include`s, its `[meta] base` chain and the user overlay, so file ids never
/// collide.
#[derive(Default)]
pub struct Sources {
    files: Vec<(String, String)>,
}

impl Sources {
    pub fn new() -> Sources {
        Sources::default()
    }

    pub fn add(&mut self, name: impl Into<String>, text: impl Into<String>) -> FileId {
        self.files.push((name.into(), text.into()));
        (self.files.len() - 1) as FileId
    }

    pub fn name(&self, f: FileId) -> &str {
        self.files.get(f as usize).map(|x| x.0.as_str()).unwrap_or("<none>")
    }

    pub fn text(&self, f: FileId) -> &str {
        self.files.get(f as usize).map(|x| x.1.as_str()).unwrap_or("")
    }

    pub fn line(&self, f: FileId, line: u32) -> &str {
        if line == 0 {
            return "";
        }
        self.text(f).lines().nth(line as usize - 1).unwrap_or("")
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }
}

// --------------------------------------------------------------- diagnostics

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    /// Reported, correct result. §4.2's quoted-value row and §4.4's
    /// enforcement notes.
    Note,
    /// The default. Something was ignored or fell back; the theme still loads.
    Warn,
    /// `[meta] strict = true` raises unknown keys to this. **The theme still
    /// loads** ([CONFLICT 10]) — nothing in this engine refuses to produce one.
    Error,
}

impl Level {
    pub fn label(self) -> &'static str {
        match self {
            Level::Note => "note",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Diagnostic {
    pub level: Level,
    pub span: Span,
    pub message: String,
}

impl Diagnostic {
    pub fn new(level: Level, span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic { level, span, message: message.into() }
    }

    pub fn warn(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(Level::Warn, span, message)
    }

    pub fn note(span: Span, message: impl Into<String>) -> Diagnostic {
        Diagnostic::new(Level::Note, span, message)
    }

    /// §4.3's shape: `file:line:col  level  message`, then the source line with
    /// a caret run under the offending span.
    pub fn render(&self, src: &Sources) -> String {
        let mut out = format!(
            "  {}:{}:{}  {}  {}\n",
            src.name(self.span.file),
            self.span.line,
            self.span.col,
            self.level.label(),
            self.message
        );
        let line = src.line(self.span.file, self.span.line);
        if !line.is_empty() && self.span.len > 0 {
            let n = format!("{:>7} | ", self.span.line);
            out.push_str(&n);
            out.push_str(line);
            out.push('\n');
            // Spans are byte offsets; the caret is drawn in characters, so a
            // line with an accented word before the span still points at it.
            let byte_col = self.span.col.saturating_sub(1) as usize;
            let chars_before = line.get(..byte_col.min(line.len())).map(|s| s.chars().count());
            let lead = chars_before.unwrap_or(byte_col);
            let width = line
                .get(byte_col..(byte_col + self.span.len as usize).min(line.len()))
                .map(|s| s.chars().count())
                .unwrap_or(self.span.len as usize);
            out.push_str(&" ".repeat(n.len() + lead));
            out.push_str(&"^".repeat(width.max(1)));
            out.push('\n');
        }
        out
    }
}

// -------------------------------------------------------------------- shapes

/// The seven interaction states of §3.2's `state` production.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Idle,
    Hover,
    Press,
    Selected,
    SelectedHover,
    Dragging,
    Disabled,
}

pub const STATE_NAMES: [&str; 7] = [
    "idle", "hover", "press", "selected", "selected_hover", "dragging", "disabled",
];

impl State {
    /// The ladder in its own order — the order [`STATE_NAMES`] is written
    /// in and the order `class_states` is indexed by.
    ///
    /// A consumer that has to visit every rung (the state crossfade in
    /// [`crate::motion`] weighs all seven) would otherwise spell the list
    /// a second time, and a second spelling is a second answer to "how
    /// many rungs are there" the day an eighth arrives.
    pub const ALL: [State; STATE_NAMES.len()] = [
        State::Idle,
        State::Hover,
        State::Press,
        State::Selected,
        State::SelectedHover,
        State::Dragging,
        State::Disabled,
    ];

    pub fn from_name(s: &str) -> Option<State> {
        Some(match s {
            "idle" => State::Idle,
            "hover" => State::Hover,
            "press" => State::Press,
            "selected" => State::Selected,
            "selected_hover" => State::SelectedHover,
            "dragging" => State::Dragging,
            "disabled" => State::Disabled,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        STATE_NAMES[self as usize]
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SectionKind {
    /// `[panel]`, `[panel:hover]` — dotted keys concatenate onto the path.
    Plain,
    /// `[mood.alert]` — every key is an absolute token path.
    Mood,
    /// `[variant.hc]` — likewise.
    Variant,
}

#[derive(Clone, Debug)]
pub struct Section {
    pub kind: SectionKind,
    /// `panel.title` for a plain section, `alert` for `[mood.alert]`.
    pub path: String,
    pub state: Option<State>,
    pub span: Span,
}

impl Section {
    pub fn is_overlay(&self) -> bool {
        !matches!(self.kind, SectionKind::Plain)
    }
}

/// A locale tag: `pl`, `pt_BR`.
pub type LangTag = String;

#[derive(Clone, Debug)]
pub struct KeyVal {
    /// Index into [`Document::sections`]. `u32::MAX` means "before any section",
    /// which is legal and puts the key at the root.
    pub section: u32,
    /// The **fully qualified** token path: the section path already
    /// concatenated for a plain section, the key verbatim for an overlay.
    pub key: String,
    /// `term.ansi[4]` keeps `4` here and `term.ansi` in `key`.
    pub index: Option<u32>,
    /// `name[pl]` keeps `pl` here and `meta.name` in `key`.
    pub locale: Option<LangTag>,
    pub value: Expr,
    /// The enum word list the key's own comment declares — the `enum: a | b | c`
    /// field of the master's established comment grammar. Empty for every token
    /// whose comment declares none. Only `default.theme` is read for this: the
    /// master IS the schema, and the declared order is what `enum_of` indexes
    /// (the ABI's `theme_enum` promises "the declared enum list").
    pub declared_words: Vec<String>,
    pub key_span: Span,
    pub value_span: Span,
}

impl KeyVal {
    /// The addressable token name, index folded in: `term.ansi[4]`.
    pub fn token(&self) -> String {
        match self.index {
            Some(i) => format!("{}[{}]", self.key, i),
            None => self.key.clone(),
        }
    }
}

/// One parsed file (plus anything it `@include`d), unevaluated.
#[derive(Default)]
pub struct Document {
    pub sections: Vec<Section>,
    pub keys: Vec<KeyVal>,
    /// Every file that contributed, this one first. The ids index a [`Sources`]
    /// the loader owns, so two documents never disagree about what file 3 is.
    pub spans: Vec<FileId>,
}

impl Document {
    /// The first value declared for a key, ignoring section structure — enough
    /// for `[meta]`, which is read before the cascade exists.
    pub fn meta(&self, key: &str) -> Option<&Expr> {
        self.keys.iter().find(|k| k.key == key && k.locale.is_none()).map(|k| &k.value)
    }

    pub fn meta_text(&self, key: &str) -> Option<String> {
        match self.meta(key) {
            Some(Expr::Text(s)) => Some(s.clone()),
            Some(Expr::Word(s)) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn meta_bool(&self, key: &str) -> Option<bool> {
        match self.meta(key) {
            Some(Expr::Bool(b)) => Some(*b),
            _ => None,
        }
    }

    /// Every `[mood.<m>]` / `[variant.<v>]` name declared, in order.
    pub fn overlays(&self, kind: SectionKind) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        for s in &self.sections {
            if s.kind == kind && !out.contains(&s.path) {
                out.push(s.path.clone());
            }
        }
        out
    }
}

// -------------------------------------------------------------- entry points

/// The include depth cap of §4.1's inheritance table. Four, because a deeper
/// file-level splice is a directory-layout problem rather than a theme.
pub const MAX_INCLUDE_DEPTH: u32 = 4;

/// Parse text that has already been registered with [`Sources`].
///
/// `base_dir` is the file's own directory, used to resolve `@include`. `None`
/// (the `include_str!`ed `default.theme`) disables includes, which is correct:
/// §5.0b requires the master to be one file.
pub fn parse(
    src: &mut Sources,
    file: FileId,
    base_dir: Option<&Path>,
    out: &mut Vec<Diagnostic>,
) -> Document {
    let mut doc = Document { spans: vec![file], ..Default::default() };
    parse_into(&mut doc, src, file, base_dir, 0, out);
    doc
}

/// Read a `.theme` from disk and parse it. A missing or unreadable file is
/// **not** an error here: §4.2 says `default` is used and one line names the
/// path, and that decision belongs to the caller, which knows whether this was
/// the selected theme or an optional user overlay.
pub fn parse_file(src: &mut Sources, path: &Path, out: &mut Vec<Diagnostic>) -> Option<Document> {
    let text = std::fs::read_to_string(path).ok()?;
    let file = src.add(display_name(path), text);
    Some(parse(src, file, path.parent(), out))
}

/// Parses a theme that is not a file: the built-in `default` carried in the
/// binary, and anything a caller holds as text. `name` is what diagnostics
/// will call it. There is no base directory, so `@include` inside such a
/// theme is refused rather than resolved against the process's cwd.
pub fn parse_text(
    src: &mut Sources,
    name: &str,
    text: &str,
    out: &mut Vec<Diagnostic>,
) -> Option<Document> {
    let file = src.add(name.to_string(), text.to_string());
    Some(parse(src, file, None, out))
}

fn display_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.to_string_lossy().into_owned())
}

fn parse_into(
    doc: &mut Document,
    src: &mut Sources,
    file: FileId,
    base_dir: Option<&Path>,
    depth: u32,
    out: &mut Vec<Diagnostic>,
) {
    let text = src.text(file).to_string();
    let lines: Vec<&str> = text.lines().collect();
    let mut cur_section: u32 = u32::MAX;
    let mut i = 0usize;

    while i < lines.len() {
        let lineno = (i + 1) as u32;
        let raw = lines[i];
        i += 1;
        let code = strip_comment(raw);
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }

        // ---- section header -----------------------------------------
        if trimmed.starts_with('[') {
            let span = Span { file, line: lineno, col: (indent(&code) + 1) as u32, len: trimmed.len() as u32 };
            match parse_section(trimmed, span, out) {
                Some(s) => {
                    doc.sections.push(s);
                    cur_section = (doc.sections.len() - 1) as u32;
                }
                None => cur_section = u32::MAX,
            }
            continue;
        }

        // ---- @include -----------------------------------------------
        if let Some(rest) = trimmed.strip_prefix("@include") {
            let span = Span { file, line: lineno, col: (indent(&code) + 1) as u32, len: trimmed.len() as u32 };
            do_include(doc, src, base_dir, depth, rest.trim(), span, out);
            continue;
        }

        // ---- key = value --------------------------------------------
        let Some(eq) = find_assign(&code) else {
            out.push(Diagnostic::warn(
                Span { file, line: lineno, col: (indent(&code) + 1) as u32, len: trimmed.len() as u32 },
                format!("expected `key = value`, found \"{}\" (ignored)", clip(trimmed)),
            ));
            continue;
        };

        let (key_text, after) = code.split_at(eq);
        let val_text = after[1..].to_string(); // drop '='
        let key_col = (indent(key_text) + 1) as u32;
        let key_span = Span { file, line: lineno, col: key_col, len: key_text.trim().len() as u32 };
        let val_col = (eq + 1 + leading_ws(&val_text) + 1) as u32;

        // A value may run over lines while a bracket or paren is open (§3.2:
        // "newlines allowed" inside an array).
        let mut whole = val_text.clone();
        while unbalanced(&whole) && i < lines.len() {
            whole.push(' ');
            whole.push_str(&strip_comment(lines[i]));
            i += 1;
        }
        let val_trimmed = whole.trim().to_string();
        let value_span =
            Span { file, line: lineno, col: val_col, len: val_trimmed.len().max(1) as u32 };

        let Some(key) = parse_key(key_text.trim(), key_span, out) else { continue };

        let section = doc.sections.get(cur_section as usize);
        let overlay = section.map(Section::is_overlay).unwrap_or(false);
        let full = qualify(section, &key.path);

        if val_trimmed.is_empty() {
            out.push(Diagnostic::warn(
                value_span,
                format!("key \"{full}\" has no value (ignored)"),
            ));
            continue;
        }

        let mut p = Cursor::new(&val_trimmed, value_span);
        let value = p.value(out);
        p.ws();
        if !p.eof() {
            out.push(Diagnostic::warn(
                p.here(),
                format!("trailing text \"{}\" after the value of \"{full}\" (ignored)", clip(p.rest())),
            ));
        }

        // `px` is legal only on *_min_px / *_max_px (§3.2). This is the single
        // rule that stops raw pixels creeping back in, and it is decidable from
        // the key alone.
        check_px_rule(&full, &value, value_span, out);

        // An overlay key that resolves back into the overlay's own namespace is
        // the mistake §3.2 and §5.24 both call out, so it is caught by name.
        if overlay {
            if let Some(sec) = section {
                let own = format!("{}.{}", overlay_prefix(sec), sec.path);
                if key.path == own || key.path.starts_with(&format!("{own}.")) {
                    let suggestion = key.path[own.len()..].trim_start_matches('.').to_string();
                    out.push(Diagnostic::warn(
                        key_span,
                        format!(
                            "key inside a {} overlay must name a top-level token — did you mean \"{}\"?",
                            overlay_prefix(sec),
                            if suggestion.is_empty() { "palette.accent" } else { &suggestion }
                        ),
                    ));
                    continue;
                }
            }
        }

        doc.keys.push(KeyVal {
            section: cur_section,
            key: full,
            index: key.index,
            locale: key.locale,
            value,
            declared_words: declared_enum_words(raw),
            key_span,
            value_span,
        });
    }
}

fn overlay_prefix(s: &Section) -> &'static str {
    match s.kind {
        SectionKind::Mood => "mood",
        SectionKind::Variant => "variant",
        SectionKind::Plain => "",
    }
}

/// §3.2: dotted keys inside a *plain* section concatenate; inside an overlay
/// every key is already absolute. This is the one exception to concatenation
/// and it is the whole point of an overlay.
fn qualify(section: Option<&Section>, key: &str) -> String {
    match section {
        Some(s) if !s.is_overlay() && !s.path.is_empty() => format!("{}.{}", s.path, key),
        _ => key.to_string(),
    }
}

// ------------------------------------------------------------------ includes

fn do_include(
    doc: &mut Document,
    src: &mut Sources,
    base_dir: Option<&Path>,
    depth: u32,
    arg: &str,
    span: Span,
    out: &mut Vec<Diagnostic>,
) {
    let Some(name) = unquote(arg) else {
        out.push(Diagnostic::warn(span, "@include takes a quoted file name (skipped)"));
        return;
    };
    if depth + 1 > MAX_INCLUDE_DEPTH {
        out.push(Diagnostic::warn(
            span,
            format!(
                "@include depth > {MAX_INCLUDE_DEPTH}: {} -> \"{name}\" (the include is skipped)",
                doc.spans.iter().map(|f| src.name(*f).to_string()).collect::<Vec<_>>().join(" -> ")
            ),
        ));
        return;
    }
    // No `..`, no absolute paths: an include may not escape the theme's own
    // directory tree (§3.2).
    let p = Path::new(&name);
    if p.is_absolute() || p.components().any(|c| matches!(c, Component::ParentDir)) {
        out.push(Diagnostic::warn(
            span,
            format!("@include \"{name}\" escapes the theme's directory (skipped)"),
        ));
        return;
    }
    let Some(dir) = base_dir else {
        out.push(Diagnostic::warn(
            span,
            format!("@include \"{name}\" is not available in an embedded theme (skipped)"),
        ));
        return;
    };
    let full: PathBuf = dir.join(p);
    let Ok(text) = std::fs::read_to_string(&full) else {
        out.push(Diagnostic::warn(
            span,
            format!("@include \"{name}\" could not be read (skipped)"),
        ));
        return;
    };
    let file = src.add(display_name(&full), text);
    doc.spans.push(file);
    parse_into(doc, src, file, full.parent(), depth + 1, out);
}

// ------------------------------------------------------------------ sections

fn parse_section(text: &str, span: Span, out: &mut Vec<Diagnostic>) -> Option<Section> {
    let inner = text.strip_prefix('[').and_then(|t| t.strip_suffix(']'));
    let Some(inner) = inner else {
        out.push(Diagnostic::warn(span, format!("unterminated section header \"{}\"", clip(text))));
        return None;
    };
    let inner = inner.trim();
    let (path, state) = match inner.split_once(':') {
        Some((p, s)) => {
            let Some(st) = State::from_name(s.trim()) else {
                out.push(Diagnostic::warn(
                    span,
                    format!(
                        "unknown state \"{}\" — one of {} (section ignored)",
                        s.trim(),
                        STATE_NAMES.join(" ")
                    ),
                ));
                return None;
            };
            (p.trim(), Some(st))
        }
        None => (inner, None),
    };
    if !valid_path(path) {
        out.push(Diagnostic::warn(span, format!("malformed section path \"{path}\" (ignored)")));
        return None;
    }
    for (prefix, kind) in [("mood.", SectionKind::Mood), ("variant.", SectionKind::Variant)] {
        if let Some(name) = path.strip_prefix(prefix) {
            if name.is_empty() || name.contains('.') {
                out.push(Diagnostic::warn(
                    span,
                    format!("[{path}] names one overlay, not a path (ignored)"),
                ));
                return None;
            }
            if state.is_some() {
                out.push(Diagnostic::warn(span, format!("[{path}] may not carry a state suffix")));
            }
            return Some(Section { kind, path: name.to_string(), state: None, span });
        }
    }
    Some(Section { kind: SectionKind::Plain, path: path.to_string(), state, span })
}

// ---------------------------------------------------------------------- keys

struct ParsedKey {
    path: String,
    index: Option<u32>,
    locale: Option<LangTag>,
}

fn parse_key(text: &str, span: Span, out: &mut Vec<Diagnostic>) -> Option<ParsedKey> {
    let mut path = text;
    let mut index = None;
    let mut locale = None;
    // `key = path [index] [locale]`; both suffixes are bracketed, so they are
    // told apart by their contents rather than by their position.
    while let Some(open) = path.rfind('[') {
        if !path.ends_with(']') {
            break;
        }
        let body = &path[open + 1..path.len() - 1];
        if body.chars().all(|c| c.is_ascii_digit()) && !body.is_empty() {
            match body.parse::<u32>() {
                Ok(v) => index = Some(v),
                Err(_) => {
                    out.push(Diagnostic::warn(span, format!("index \"{body}\" is out of range")));
                    return None;
                }
            }
        } else if is_lang_tag(body) {
            locale = Some(body.to_string());
        } else {
            out.push(Diagnostic::warn(
                span,
                format!("\"[{body}]\" is neither an index nor a locale tag (key ignored)"),
            ));
            return None;
        }
        path = &path[..open];
    }
    if !valid_path(path) {
        out.push(Diagnostic::warn(span, format!("malformed key \"{}\" (ignored)", clip(text))));
        return None;
    }
    Some(ParsedKey { path: path.to_string(), index, locale })
}

fn is_lang_tag(s: &str) -> bool {
    let (a, b) = match s.split_once('_') {
        Some((a, b)) => (a, Some(b)),
        None => (s, None),
    };
    a.len() == 2
        && a.chars().all(|c| c.is_ascii_lowercase())
        && b.map(|b| b.len() == 2 && b.chars().all(|c| c.is_ascii_uppercase())).unwrap_or(true)
}

/// `path = ident { "." ident }`.
///
/// The EBNF spells `ident = lower { lower | digit | "_" }`, but §5.4's ladders
/// are `space.0`, `size.2xl` and `corner.sm`, so a segment is allowed to start
/// with a digit. Rejecting them would make the catalogue unwritable.
fn valid_path(p: &str) -> bool {
    !p.is_empty()
        && p.split('.').all(|seg| {
            !seg.is_empty() && seg.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        })
}

/// §3.2: `px` is legal only on `*_min_px` / `*_max_px`. There is no `min`
/// operator and no arithmetic — a floored length is **two tokens**.
fn check_px_rule(key: &str, value: &Expr, span: Span, out: &mut Vec<Diagnostic>) {
    fn uses_px(e: &Expr) -> bool {
        match e {
            Expr::Len(_, Unit::Px) => true,
            Expr::Array(v) | Expr::Call(_, v) => v.iter().any(uses_px),
            Expr::Ratio(_, x) => uses_px(x),
            _ => false,
        }
    }
    // The suffix may be the whole last segment (`type.min_px`) as readily as
    // the tail of a longer name (`a11y.min_hit_min_px`). The dotted form is
    // what the catalogue actually writes wherever the floor belongs to the
    // section rather than to one sibling token.
    let floored = key.ends_with("_min_px")
        || key.ends_with("_max_px")
        || key.rsplit('.').next().is_some_and(|s| s == "min_px" || s == "max_px");
    if uses_px(value) && !floored {
        out.push(Diagnostic::warn(
            span,
            format!(
                "px is legal only on a token whose name ends _min_px or _max_px — \
                 write \"{key}\" in u and give it a companion \"{key}_min_px\" (§3.2)"
            ),
        ));
    }
}

// ------------------------------------------------------------- value cursor

struct Cursor<'a> {
    s: &'a [u8],
    text: &'a str,
    i: usize,
    span: Span,
}

impl<'a> Cursor<'a> {
    fn new(text: &'a str, span: Span) -> Cursor<'a> {
        Cursor { s: text.as_bytes(), text, i: 0, span }
    }

    fn eof(&self) -> bool {
        self.i >= self.s.len()
    }

    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn peek_at(&self, n: usize) -> Option<u8> {
        self.s.get(self.i + n).copied()
    }

    fn bump(&mut self) -> Option<u8> {
        let c = self.peek();
        if c.is_some() {
            self.i += 1;
        }
        c
    }

    fn ws(&mut self) {
        while matches!(self.peek(), Some(b' ') | Some(b'\t')) {
            self.i += 1;
        }
    }

    /// A separator inside a call or an array: whitespace, or a comma.
    fn sep(&mut self) {
        self.ws();
        if self.peek() == Some(b',') {
            self.i += 1;
            self.ws();
        }
    }

    fn rest(&self) -> &str {
        &self.text[self.i.min(self.text.len())..]
    }

    fn here(&self) -> Span {
        Span { col: self.span.col + self.i as u32, len: 1, ..self.span }
    }

    fn span_from(&self, start: usize) -> Span {
        Span {
            col: self.span.col + start as u32,
            len: (self.i - start).max(1) as u32,
            ..self.span
        }
    }

    fn take_while(&mut self, f: impl Fn(u8) -> bool) -> &'a str {
        let start = self.i;
        while self.peek().map(&f).unwrap_or(false) {
            self.i += 1;
        }
        &self.text[start..self.i]
    }

    // ------------------------------------------------------------ value

    fn value(&mut self, out: &mut Vec<Diagnostic>) -> Expr {
        self.ws();
        let e = self.primary(out);
        // alpha-suffix = ws "/" ws num — sugar for alpha(x, num) [CONFLICT 11].
        let save = self.i;
        self.ws();
        if self.peek() == Some(b'/') {
            self.i += 1;
            self.ws();
            match self.number() {
                Some(a) => {
                    return match e {
                        Expr::Color(_) | Expr::Ref(..) | Expr::Call(..) | Expr::Rgb(..)
                        | Expr::Oklch(..) => Expr::Call(Func::Alpha, vec![e, Expr::Num(a)]),
                        other => {
                            out.push(Diagnostic::warn(
                                self.here(),
                                "the \"/ a\" alpha suffix follows a colour, a reference or a call",
                            ));
                            other
                        }
                    };
                }
                None => {
                    out.push(Diagnostic::warn(self.here(), "expected a number after \"/\""));
                    self.i = save;
                }
            }
        } else {
            self.i = save;
        }
        e
    }

    fn primary(&mut self, out: &mut Vec<Diagnostic>) -> Expr {
        self.ws();
        let start = self.i;
        match self.peek() {
            None => Expr::Bad("empty value".into()),
            Some(b'"') => self.quoted(out),
            Some(b'#') => self.hex(out, start),
            Some(b'@') => self.reference(out, start),
            Some(b'[') => self.array(out),
            Some(c) if c.is_ascii_digit() || c == b'-' || c == b'.' => self.numeric(out, start),
            Some(b'U') if self.peek_at(1) == Some(b'+') => self.codepoint(out, start),
            Some(c) if c.is_ascii_alphabetic() || c == b'_' => self.word_or_call(out, start),
            Some(c) => {
                self.i += 1;
                let msg = format!("unexpected character '{}' in a value", c as char);
                out.push(Diagnostic::warn(self.span_from(start), msg.clone()));
                Expr::Bad(msg)
            }
        }
    }

    /// A quoted value stays [`Expr::Text`] here and is **re-lexed to the target
    /// token's type** by [`relex`] once `cascade.rs` knows what that type is.
    ///
    /// The scan is byte-wise, which is safe because `"` and `\` are ASCII and
    /// every UTF-8 continuation byte is >= 0x80; the *content* is then taken as
    /// a string slice, so a Polish theme name survives intact.
    fn quoted(&mut self, out: &mut Vec<Diagnostic>) -> Expr {
        let start = self.i;
        self.i += 1; // opening quote
        loop {
            match self.bump() {
                None => {
                    out.push(Diagnostic::warn(self.span_from(start), "unterminated string"));
                    return Expr::Bad("unterminated string".into());
                }
                Some(b'"') => break,
                // An escape consumes the next byte, which for a multi-byte
                // character is its lead byte — and `\<non-ASCII>` is not an
                // escape anyone writes, so it round-trips as itself below.
                Some(b'\\') => {
                    self.bump();
                }
                Some(_) => {}
            }
        }
        let raw = &self.text[start + 1..self.i.saturating_sub(1)];
        if !raw.contains('\\') {
            return Expr::Text(raw.to_string());
        }
        let mut s = String::with_capacity(raw.len());
        let mut it = raw.chars();
        while let Some(c) = it.next() {
            if c != '\\' {
                s.push(c);
                continue;
            }
            match it.next() {
                Some('n') => s.push('\n'),
                Some('t') => s.push('\t'),
                Some(other) => s.push(other),
                None => {}
            }
        }
        Expr::Text(s)
    }

    fn hex(&mut self, out: &mut Vec<Diagnostic>, start: usize) -> Expr {
        self.i += 1; // '#'
        let digits = self.take_while(|c| c.is_ascii_hexdigit());
        match Color::from_hex(digits) {
            Some(c) if matches!(digits.len(), 3 | 4 | 6 | 8) => Expr::Color(c.to_linear()),
            _ => {
                let msg = format!(
                    "expected a colour: #RGB, #RGBA, #RRGGBB or #RRGGBBAA, found \"#{digits}\""
                );
                out.push(Diagnostic::warn(self.span_from(start), msg.clone()));
                Expr::Bad(msg)
            }
        }
    }

    fn reference(&mut self, out: &mut Vec<Diagnostic>, start: usize) -> Expr {
        self.i += 1; // '@'
        let path = self.take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'.');
        if !valid_path(path) {
            let msg = format!("expected a token path after \"@\", found \"{}\"", clip(path));
            out.push(Diagnostic::warn(self.span_from(start), msg.clone()));
            return Expr::Bad(msg);
        }
        let path = path.to_string();
        let mut index = None;
        if self.peek() == Some(b'[') {
            self.i += 1;
            let d = self.take_while(|c| c.is_ascii_digit());
            let ok = self.peek() == Some(b']') && !d.is_empty();
            if ok {
                self.i += 1;
                index = d.parse::<u32>().ok();
            } else {
                out.push(Diagnostic::warn(self.span_from(start), "malformed index after a reference"));
            }
        }
        Expr::Ref(path, index)
    }

    fn array(&mut self, out: &mut Vec<Diagnostic>) -> Expr {
        let start = self.i;
        self.i += 1; // '['
        let mut items = Vec::new();
        loop {
            self.sep();
            match self.peek() {
                None => {
                    out.push(Diagnostic::warn(self.span_from(start), "unterminated array"));
                    return Expr::Bad("unterminated array".into());
                }
                Some(b']') => {
                    self.i += 1;
                    return Expr::Array(items);
                }
                _ => items.push(self.value(out)),
            }
            self.ws();
            if self.peek() == Some(b',') {
                self.i += 1;
            }
        }
    }

    fn codepoint(&mut self, out: &mut Vec<Diagnostic>, start: usize) -> Expr {
        self.i += 2; // "U+"
        let d = self.take_while(|c| c.is_ascii_hexdigit());
        match u32::from_str_radix(d, 16) {
            Ok(v) if !d.is_empty() && v <= 0x10_FFFF => Expr::Codepoint(v),
            _ => {
                let msg = format!("expected a codepoint like U+2726, found \"U+{d}\"");
                out.push(Diagnostic::warn(self.span_from(start), msg.clone()));
                Expr::Bad(msg)
            }
        }
    }

    fn number(&mut self) -> Option<f32> {
        let start = self.i;
        if self.peek() == Some(b'-') {
            self.i += 1;
        }
        let a = self.take_while(|c| c.is_ascii_digit()).len();
        let mut b = 0;
        if self.peek() == Some(b'.') && self.peek_at(1).map(|c| c.is_ascii_digit()).unwrap_or(false) {
            self.i += 1;
            b = self.take_while(|c| c.is_ascii_digit()).len();
        }
        if a == 0 && b == 0 {
            self.i = start;
            return None;
        }
        self.text[start..self.i].parse::<f32>().ok()
    }

    fn numeric(&mut self, out: &mut Vec<Diagnostic>, start: usize) -> Expr {
        let Some(n) = self.number() else {
            let msg = format!("expected a number, found \"{}\"", clip(self.rest()));
            out.push(Diagnostic::warn(self.span_from(start), msg.clone()));
            return Expr::Bad(msg);
        };
        // ratio = num "x" ws reference — never ambiguous, because the right
        // side is a reference (§3.2).
        if self.peek() == Some(b'x') {
            let save = self.i;
            self.i += 1;
            self.ws();
            if self.peek() == Some(b'@') {
                let r = self.reference(out, self.i);
                return Expr::Ratio(n, Box::new(r));
            }
            let p = self.take_while(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'_' || c == b'.');
            if valid_path(p) && !p.is_empty() {
                // Accepted for one release, resolved relatively then absolutely,
                // and warned — §3.2.
                out.push(Diagnostic::warn(
                    self.span_from(start),
                    format!("ratio target \"{p}\" is relative — write \"@{p}\""),
                ));
                return Expr::Ratio(n, Box::new(Expr::Ref(p.to_string(), None)));
            }
            self.i = save;
        }
        if self.peek() == Some(b'%') {
            self.i += 1;
            return Expr::Len(n, Unit::Pct);
        }
        let unit = self.take_while(|c| c.is_ascii_lowercase());
        if unit.is_empty() {
            // Dimensionless by construction: counts and multipliers. Whether a
            // bare number is legal here depends on the token's type, which
            // `cascade.rs` knows and this parser does not.
            return Expr::Num(n);
        }
        match Unit::from_str(unit) {
            Some(u) => {
                if u.deprecated() {
                    let (repl, note) = match u {
                        Unit::Ux => (n * 0.13889, "deprecated unit: 1ux = 0.13889u"),
                        _ => (n, "deprecated migration unit; write u"),
                    };
                    out.push(Diagnostic::warn(
                        self.span_from(start),
                        format!("{note} — reading \"{n}{}\" as {repl}u", u.name()),
                    ));
                    return Expr::Len(repl, Unit::U);
                }
                Expr::Len(n, u)
            }
            None => {
                let msg = format!(
                    "unknown unit \"{unit}\" — one of u px % em deg ms s hz (§3.2)"
                );
                out.push(Diagnostic::warn(self.span_from(start), msg.clone()));
                Expr::Bad(msg)
            }
        }
    }

    fn word_or_call(&mut self, out: &mut Vec<Diagnostic>, start: usize) -> Expr {
        // A bare word may carry dots: the type roles are named in the same
        // dotted shape as everything else (`title.window`, `label.section`),
        // and a role IS the natural value of a `*_role` token. There is no
        // ambiguity with a reference — a reference starts with `@` — and a
        // call is caught by the '(' below, so the dots can only belong to
        // the word. Refusing them would make §5.16's roles unwritable.
        let name = self
            .take_while(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.')
            .trim_end_matches('.')
            .to_string();
        if self.peek() != Some(b'(') {
            return match name.as_str() {
                "true" => Expr::Bool(true),
                "false" => Expr::Bool(false),
                _ => Expr::Word(name),
            };
        }
        self.i += 1; // '('
        let mut args = Vec::new();
        let mut alpha: Option<Expr> = None;
        loop {
            self.sep();
            match self.peek() {
                None => {
                    let msg = format!("unterminated call to {name}()");
                    out.push(Diagnostic::warn(self.span_from(start), msg.clone()));
                    return Expr::Bad(msg);
                }
                Some(b')') => {
                    self.i += 1;
                    break;
                }
                Some(b'/') if matches!(name.as_str(), "rgb" | "oklch") => {
                    self.i += 1;
                    self.ws();
                    alpha = Some(self.primary(out));
                }
                // rgb()/oklch() components are numbers, so a following "/" is
                // the function's own alpha and never a component's suffix.
                _ if matches!(name.as_str(), "rgb" | "oklch") => args.push(self.primary(out)),
                _ => args.push(self.value(out)),
            }
        }
        match name.as_str() {
            "rgb" | "oklch" => {
                if args.len() != 3 {
                    let msg = format!("{name}() takes 3 components, found {}", args.len());
                    out.push(Diagnostic::warn(self.span_from(start), msg.clone()));
                    return Expr::Bad(msg);
                }
                let mut it = args.into_iter();
                let (a, b, c) = (it.next().unwrap(), it.next().unwrap(), it.next().unwrap());
                let al = alpha.map(Box::new);
                if name == "rgb" {
                    Expr::Rgb(Box::new(a), Box::new(b), Box::new(c), al)
                } else {
                    Expr::Oklch(Box::new(a), Box::new(b), Box::new(c), al)
                }
            }
            _ => match Func::from_name(&name) {
                Some(f) => {
                    if let Some(a) = alpha {
                        args.push(a);
                    }
                    let extracting =
                        matches!(f, Func::Sat | Func::Hue) && args.len() == 1;
                    if !extracting && args.len() != f.arity() {
                        let msg = format!(
                            "{}() takes {} arguments, found {}",
                            f.name(),
                            f.arity(),
                            args.len()
                        );
                        out.push(Diagnostic::warn(self.span_from(start), msg.clone()));
                        return Expr::Bad(msg);
                    }
                    Expr::Call(f, args)
                }
                None => {
                    // §4.2: names the function and lists the legal names.
                    let msg = format!(
                        "unknown function \"{name}\" — the fifteen are: {}",
                        Func::legal_names()
                    );
                    out.push(Diagnostic::warn(self.span_from(start), msg.clone()));
                    Expr::Bad(msg)
                }
            },
        }
    }
}

// ------------------------------------------------------------------- re-lex

/// §3.2: "a double-quoted value is **re-lexed as the target token's type**".
///
/// `cascade.rs` calls this when a quoted value lands on a token that is not
/// `text`, and emits a **note** — not a warning — because the result is correct.
/// `accent = "#FF2A35"` and `accent = #FF2A35` are the same theme.
pub fn relex(text: &str, span: Span, out: &mut Vec<Diagnostic>) -> Expr {
    let mut sink = Vec::new();
    let mut c = Cursor::new(text, span);
    let e = c.value(&mut sink);
    c.ws();
    if !c.eof() || matches!(e, Expr::Bad(_)) {
        out.extend(sink);
        return Expr::Bad(format!("could not re-lex the quoted value \"{}\"", clip(text)));
    }
    e
}

/// Parse one value out of a standalone string. Used by tests, by
/// `--check-theme`, and by the theme editor; the loader goes through
/// [`parse`].
///
/// The editor holds a token name and a piece of text a slider just produced,
/// and needs the same `Expr` the file would have produced — read by the SAME
/// cursor, so nothing an editor writes can be something the parser would
/// refuse from a file. Trailing text is a warning in `out`, so a caller that
/// means to accept only a whole value must look at `out` and not just at the
/// expression.
pub fn parse_value(text: &str, span: Span, out: &mut Vec<Diagnostic>) -> Expr {
    let mut c = Cursor::new(text.trim(), span);
    let e = c.value(out);
    c.ws();
    if !c.eof() {
        out.push(Diagnostic::warn(c.here(), format!("trailing text \"{}\"", clip(c.rest()))));
    }
    e
}

// ---------------------------------------------------------------- lexer bits

/// How many bytes of a line the parser reads before the comment starts.
///
/// For the theme SAVE, which patches a value where it stands and has to know
/// where the author's note begins so it never writes over one. Answering with
/// a length rather than a slice is the whole answer, because [`strip_comment`]
/// returns a PREFIX of its input — an offset measured on the code is the same
/// offset in the raw line.
pub fn code_len(line: &str) -> usize {
    strip_comment(line).len()
}

/// Strip a trailing `#` comment, but not a `#RRGGBB` colour and not a `#`
/// inside a string. `accent = #FF2A35 # the one hue` has both on one line.
fn strip_comment(line: &str) -> String {
    let b = line.as_bytes();
    let mut in_str = false;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => in_str = !in_str,
            b'\\' if in_str => i += 1,
            b'#' if !in_str => {
                let rest = &line[i + 1..];
                let n = rest.bytes().take_while(|c| c.is_ascii_hexdigit()).count();
                let ends = rest.as_bytes().get(n).map(|c| !c.is_ascii_alphanumeric()).unwrap_or(true);
                if matches!(n, 3 | 4 | 6 | 8) && ends {
                    i += 1 + n;
                    continue;
                }
                return line[..i].to_string();
            }
            _ => {}
        }
        i += 1;
    }
    line.to_string()
}

/// The enum word list a key's trailing comment declares, in declared order.
///
/// The master's comment grammar separates its fields with `·`, and the type
/// field of an enum token spells `enum: a | b | c`. That list is the schema's
/// declaration: `enum_of` indexes into it, which is what lets a compiled
/// plugin — which only ever sees the index — write `ALIGN_TOP = 0` against
/// `enum: top | middle | bottom` and be right on every load. A comment that
/// declares no list (or one word, which declares nothing) answers empty, and
/// the word list falls back to growing from the values a cascade interns.
fn declared_enum_words(line: &str) -> Vec<String> {
    // The comment is everything strip_comment removes.
    let code_len = strip_comment(line).len();
    let comment = &line[code_len..];
    let Some(at) = comment.find("enum:") else { return Vec::new() };
    let list = &comment[at + "enum:".len()..];
    // The declaration ends at the comment grammar's field separator.
    let list = list.split('·').next().unwrap_or("");
    let words: Vec<String> = list
        .split('|')
        .map(str::trim)
        .filter(|w| {
            !w.is_empty()
                && w.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
        .map(str::to_string)
        .collect();
    // One word declares nothing: a list is only a list from two entries up,
    // which also keeps prose that happens to contain "enum:" harmless.
    if words.len() < 2 || words.len() != list.split('|').count() {
        return Vec::new();
    }
    words
}

/// The `=` that assigns, skipping any inside a string.
fn find_assign(line: &str) -> Option<usize> {
    let b = line.as_bytes();
    let mut in_str = false;
    for (i, c) in b.iter().enumerate() {
        match c {
            b'"' => in_str = !in_str,
            b'=' if !in_str => return Some(i),
            _ => {}
        }
    }
    None
}

fn unbalanced(s: &str) -> bool {
    let b = s.as_bytes();
    let mut in_str = false;
    let mut depth = 0i32;
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'"' => in_str = !in_str,
            b'\\' if in_str => i += 1,
            b'[' | b'(' if !in_str => depth += 1,
            b']' | b')' if !in_str => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    depth > 0
}

fn indent(s: &str) -> usize {
    s.len() - s.trim_start().len()
}

fn leading_ws(s: &str) -> usize {
    s.len() - s.trim_start().len()
}

fn unquote(s: &str) -> Option<String> {
    let t = s.trim();
    if t.len() >= 2 && t.starts_with('"') && t.ends_with('"') {
        Some(t[1..t.len() - 1].to_string())
    } else {
        None
    }
}

fn clip(s: &str) -> String {
    let t = s.trim();
    if t.chars().count() <= 40 {
        t.to_string()
    } else {
        format!("{}…", t.chars().take(39).collect::<String>())
    }
}

// ------------------------------------------------------------- did-you-mean

/// Levenshtein distance, capped: anything past `max` returns `max + 1`.
pub fn levenshtein(a: &str, b: &str, max: usize) -> usize {
    if a == b {
        return 0;
    }
    if a.len().abs_diff(b.len()) > max {
        return max + 1;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for i in 1..=a.len() {
        cur[0] = i;
        let mut best = cur[0];
        for j in 1..=b.len() {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            cur[j] = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            best = best.min(cur[j]);
        }
        if best > max {
            return max + 1;
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// The nearest known token by Levenshtein distance <= 3 (§4.2). Consulted
/// *after* the static rename/alias table, never before it: distance cannot find
/// `panel.content_pad` from `panel.pad`.
pub fn suggest<'a>(unknown: &str, known: impl Iterator<Item = &'a str>) -> Option<&'a str> {
    let mut best: Option<(usize, &str)> = None;
    for k in known {
        let d = levenshtein(unknown, k, 3);
        if d <= 3 && best.map(|(bd, _)| d < bd).unwrap_or(true) {
            best = Some((d, k));
        }
    }
    best.map(|(_, k)| k)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::expr::{Expr, Func, Unit};

    fn parse_str(text: &str) -> (Document, Vec<Diagnostic>, Sources) {
        let mut src = Sources::new();
        let f = src.add("test.theme", text);
        let mut d = Vec::new();
        let doc = parse(&mut src, f, None, &mut d);
        (doc, d, src)
    }

    fn val(text: &str) -> (Expr, Vec<Diagnostic>) {
        let mut d = Vec::new();
        let e = parse_value(text, Span { file: 0, line: 1, col: 1, len: text.len() as u32 }, &mut d);
        (e, d)
    }

    fn only(doc: &Document, key: &str) -> Expr {
        doc.keys.iter().find(|k| k.token() == key).unwrap_or_else(|| panic!("no key {key}")).value.clone()
    }

    // ------------------------------------------------------ happy structure

    #[test]
    fn sections_concatenate_dotted_keys() {
        let (doc, d, _) = parse_str("[panel]\ntitle.band_h = 4.6u\n");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(doc.keys[0].key, "panel.title.band_h");
        assert_eq!(doc.keys[0].value, Expr::Len(4.6, Unit::U));
    }

    #[test]
    fn overlay_keys_are_absolute() {
        let text = "[mood.alert]\nmotion.alarm_blink.enabled = true\nwash = #FF2A35 / 0.22\n";
        let (doc, d, _) = parse_str(text);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(doc.keys[0].key, "motion.alarm_blink.enabled");
        assert_eq!(doc.sections[0].kind, SectionKind::Mood);
        assert_eq!(doc.sections[0].path, "alert");
        // "/ a" desugars to alpha(x, a)
        match &doc.keys[1].value {
            Expr::Call(Func::Alpha, args) => assert_eq!(args[1], Expr::Num(0.22)),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_key_inside_a_mood_that_names_the_mood_is_reported() {
        let (doc, d, _) = parse_str("[mood.alert]\nmood.alert.palette.accent = #FF2A35\n");
        assert!(doc.keys.is_empty());
        assert!(
            d[0].message.contains("must name a top-level token") && d[0].message.contains("palette.accent"),
            "{}", d[0].message
        );
    }

    #[test]
    fn state_suffix_on_a_plain_section() {
        let (doc, d, _) = parse_str("[button:hover]\nfill = @accent.primary\n");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(doc.sections[0].state, Some(State::Hover));
        assert_eq!(doc.keys[0].key, "button.fill");
    }

    #[test]
    fn indexed_and_localised_keys() {
        let text = "[term]\nansi[4] = hue(@palette.accent, 240)\n[meta]\nname[pt_BR] = \"Padrão\"\n";
        let (doc, d, _) = parse_str(text);
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(doc.keys[0].token(), "term.ansi[4]");
        assert_eq!(doc.keys[0].index, Some(4));
        assert_eq!(doc.keys[1].key, "meta.name");
        assert_eq!(doc.keys[1].locale.as_deref(), Some("pt_BR"));
    }

    #[test]
    fn comment_after_a_hex_is_a_comment_and_the_hex_survives() {
        let (doc, d, _) = parse_str("[palette]\naccent = #FF2A35   # chrome: borders\n");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(only(&doc, "palette.accent"), Expr::Color(Color::from_hex("#FF2A35").unwrap().to_linear()));
        // and a comment-only line with a hex-looking word still comments out
        let (doc2, _, _) = parse_str("# accent = #FF2A35\n");
        assert!(doc2.keys.is_empty());
    }

    #[test]
    fn arrays_may_span_lines() {
        let (doc, d, _) = parse_str("[term]\nansi = [\n  #000000,\n  #FF0000,\n]\n");
        assert!(d.is_empty(), "{d:?}");
        match only(&doc, "term.ansi") {
            Expr::Array(v) => assert_eq!(v.len(), 2),
            other => panic!("{other:?}"),
        }
    }

    // ------------------------------------------------- §4's error table

    #[test]
    fn malformed_value_bad_hex() {
        let (e, d) = val("#GG2A35");
        assert!(matches!(e, Expr::Bad(_)));
        assert!(d[0].message.contains("expected a colour"), "{}", d[0].message);
    }

    #[test]
    fn malformed_value_missing_unit_stays_a_bare_number() {
        // The parser cannot know a length was wanted; it produces Num and the
        // cascade produces "expected a length with a unit, found \"8\"".
        let (e, d) = val("8");
        assert_eq!(e, Expr::Num(8.0));
        assert!(d.is_empty());
    }

    #[test]
    fn malformed_value_unknown_unit() {
        let (e, d) = val("8rem");
        assert!(matches!(e, Expr::Bad(_)));
        assert!(d[0].message.contains("unknown unit"), "{}", d[0].message);
    }

    #[test]
    fn unknown_function_lists_the_whole_closed_set() {
        let (e, d) = val("darken(@palette.accent, 0.2)");
        assert!(matches!(e, Expr::Bad(_)));
        assert!(d[0].message.contains("unknown function \"darken\""));
        assert!(d[0].message.contains("contrast_on"), "{}", d[0].message);
    }

    #[test]
    fn wrong_arity_is_caught_at_parse_time() {
        let (e, d) = val("mix(@palette.black, @palette.accent)");
        assert!(matches!(e, Expr::Bad(_)));
        assert!(d[0].message.contains("mix() takes 3 arguments, found 2"), "{}", d[0].message);
    }

    #[test]
    fn px_is_refused_outside_min_px_and_max_px() {
        let (_, d, _) = parse_str("[panel]\ncontent_pad = 8px\n");
        assert_eq!(d.len(), 1);
        assert!(d[0].message.contains("_min_px"), "{}", d[0].message);
        // and accepted on the companion
        let (_, d2, _) = parse_str("[a11y]\nmin_hit_min_px = 24px\n");
        assert!(d2.is_empty(), "{d2:?}");
    }

    #[test]
    fn deprecated_units_are_rewritten_and_warned() {
        let (e, d) = val("2ux");
        assert!(matches!(e, Expr::Len(v, Unit::U) if (v - 0.27778).abs() < 1e-4), "{e:?}");
        assert!(d[0].message.contains("deprecated"), "{}", d[0].message);
        let (e2, d2) = val("3vh");
        assert!(matches!(e2, Expr::Len(_, Unit::U)));
        assert!(!d2.is_empty());
    }

    #[test]
    fn a_ratio_wants_a_reference_and_warns_on_a_bare_path() {
        let (e, d) = val("0.35x @menu.row_h");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(e, Expr::Ratio(0.35, Box::new(Expr::Ref("menu.row_h".into(), None))));
        let (e2, d2) = val("0.27x row_h");
        assert!(matches!(e2, Expr::Ratio(..)));
        assert!(d2[0].message.contains("is relative"), "{}", d2[0].message);
        // no space is legal too: oklch(0.870, 0.55x@chroma.accent, @hue.accent)
        let (e3, d3) = val("0.55x@chroma.accent");
        assert!(d3.is_empty(), "{d3:?}");
        assert!(matches!(e3, Expr::Ratio(..)));
    }

    #[test]
    fn unterminated_things_do_not_panic() {
        for bad in ["mix(@a, @b", "[#fff, #000", "\"unterminated", "[palette", "oklch(0.8"] {
            let (_, d) = val(bad);
            assert!(!d.is_empty(), "{bad} produced no diagnostic");
        }
        let (_, d, _) = parse_str("[palette\naccent = #fff\n");
        assert!(!d.is_empty());
    }

    #[test]
    fn a_line_with_no_assignment_is_reported_not_dropped() {
        let (doc, d, _) = parse_str("[palette]\naccent #FF2A35\n");
        assert!(doc.keys.is_empty());
        assert!(d[0].message.contains("expected `key = value`"), "{}", d[0].message);
    }

    #[test]
    fn unknown_state_in_a_section_is_reported() {
        let (_, d, _) = parse_str("[button:glowing]\nfill = #fff\n");
        assert!(d[0].message.contains("unknown state"), "{}", d[0].message);
        assert!(d[0].message.contains("selected_hover"));
    }

    #[test]
    fn malformed_key_bracket_is_neither_index_nor_locale() {
        let (doc, d, _) = parse_str("[term]\nansi[blue] = #fff\n");
        assert!(doc.keys.is_empty());
        assert!(d[0].message.contains("neither an index nor a locale"), "{}", d[0].message);
    }

    // ------------------------------------------------------------ include

    #[test]
    fn include_refuses_to_escape_the_tree() {
        let (_, d, _) = parse_str("@include \"../../etc/passwd\"\n");
        assert!(d[0].message.contains("escapes"), "{}", d[0].message);
        let (_, d2, _) = parse_str("@include \"/etc/passwd\"\n");
        assert!(d2[0].message.contains("escapes"), "{}", d2[0].message);
    }

    #[test]
    fn include_depth_is_capped_at_four() {
        let dir = std::env::temp_dir().join(format!("nacelle-theme-inc-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        // a -> b -> c -> d -> e is four splices and is legal; e -> f is the
        // fifth and is refused, so f's keys never land.
        for (n, next) in [("a", "b"), ("b", "c"), ("c", "d"), ("d", "e"), ("e", "f")] {
            std::fs::write(dir.join(format!("{n}.theme")), format!("@include \"{next}.theme\"\n")).unwrap();
        }
        std::fs::write(dir.join("f.theme"), "[palette]\naccent = #FF2A35\n").unwrap();
        let mut src = Sources::new();
        let mut d = Vec::new();
        let doc = parse_file(&mut src, &dir.join("a.theme"), &mut d).unwrap();
        assert!(
            d.iter().any(|x| x.message.contains("@include depth >")),
            "no depth diagnostic: {d:?}"
        );
        assert!(doc.keys.is_empty(), "the deepest include must not have landed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ------------------------------------------------------------- re-lex

    #[test]
    fn quoted_values_re_lex_to_the_target_type() {
        let mut d = Vec::new();
        let sp = Span::default();
        assert_eq!(
            relex("#FF2A35", sp, &mut d),
            Expr::Color(Color::from_hex("#FF2A35").unwrap().to_linear())
        );
        assert_eq!(relex("hue", sp, &mut d), Expr::Word("hue".into()));
        assert!(matches!(relex("alpha(@severity.critical.text, 0.18)", sp, &mut d), Expr::Call(Func::Alpha, _)));
        assert!(d.is_empty(), "{d:?}");
        // and a quoted value that is not a legal value of any type fails loudly
        assert!(matches!(relex("#GG", sp, &mut d), Expr::Bad(_)));
    }

    #[test]
    fn quoted_stays_text_until_the_type_is_known() {
        let (doc, d, _) = parse_str("[meta]\nname = \"Aurora\"\n");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(only(&doc, "meta.name"), Expr::Text("Aurora".into()));
    }

    #[test]
    fn non_ascii_text_survives_intact() {
        let (doc, _, _) = parse_str("[meta]\nname = \"Zorza polarna — mięta\"\n");
        assert_eq!(only(&doc, "meta.name"), Expr::Text("Zorza polarna — mięta".into()));
    }

    // -------------------------------------------------------- diagnostics

    #[test]
    fn a_diagnostic_carries_file_line_col_and_prints_a_caret() {
        let (_, d, src) = parse_str("[panel]\ncontent_pad = 8px\n");
        let g = &d[0];
        assert_eq!(src.name(g.span.file), "test.theme");
        assert_eq!(g.span.line, 2);
        let text = g.render(&src);
        assert!(text.contains("test.theme:2:"), "{text}");
        assert!(text.contains("^"), "{text}");
        assert!(text.contains("content_pad = 8px"), "{text}");
    }

    #[test]
    fn the_caret_is_drawn_in_characters_not_bytes() {
        let (_, d, src) = parse_str("[meta]\ndescription = \"Miętowa\"\nname = 8px\n");
        // `px` outside a *_min_px key, on a line after a multi-byte one.
        let g = d.iter().find(|x| x.message.contains("_min_px")).expect("no px diagnostic");
        let text = g.render(&src);
        let caret = text.lines().last().unwrap();
        let src_line = text.lines().nth(1).unwrap();
        assert_eq!(caret.find('^'), src_line.find("8px"), "{text}");
    }

    #[test]
    fn escapes_in_quoted_text_do_not_mangle_non_ascii() {
        let (doc, d, _) = parse_str("[meta]\ndescription = \"Zorza:\\tmiętowa\\nkonsola\"\n");
        assert!(d.is_empty(), "{d:?}");
        assert_eq!(
            only(&doc, "meta.description"),
            Expr::Text("Zorza:\tmiętowa\nkonsola".into())
        );
    }

    #[test]
    fn levenshtein_suggestion_finds_the_typo() {
        let known = ["panel.border", "panel.corner", "panel.content_pad"];
        assert_eq!(suggest("panel.boarder", known.iter().copied()), Some("panel.border"));
        // and refuses to guess wildly: distance 8
        assert_eq!(suggest("panel.pad", ["panel.content_pad"].iter().copied()), None);
        assert_eq!(levenshtein("abc", "abc", 3), 0);
        assert_eq!(levenshtein("abc", "abd", 3), 1);
        assert_eq!(levenshtein("kitten", "sitting", 3), 3);
        assert_eq!(levenshtein("a", "bbbbbbbb", 3), 4);
    }

    // ------------------------------------------------- the whole §3.4 file

    #[test]
    fn the_lockdown_worked_example_parses_clean() {
        let text = r#"
[meta]
schema = 1
name = "Lockdown"
name[pl] = "Blokada"
description = "Red chrome, blue data. Reference image 5."
family = console

[palette]
black   = #08060B
white   = #FFEDEB
accent  = #FF2A35
data    = #35A7FF
neutral = #74707E

[severity]
mode = hue
contained.text = #E8B33A
warning.text   = #FF7A00

[render]
hull = sat(@data.line, 0.0)
rim  = @accent.primary

[decor]
enabled = true
[decor.traces]
enabled = true
color   = @accent.primary
alpha   = 0.08
[decor.vignette]
enabled  = true
strength = 0.55

[mood.alert]
motion.alarm_blink.enabled = true
component.alarm_bar.fill   = alpha(@severity.critical.text, 0.18)
glow.focus_ring.enabled    = true
glow.focus_ring.radius     = 1.4u
wash = #FF2A35 / 0.22
"#;
        let (doc, d, src) = parse_str(text);
        let rendered: String = d.iter().map(|x| x.render(&src)).collect();
        assert!(d.is_empty(), "the shipped example must parse clean:\n{rendered}");
        assert_eq!(doc.overlays(SectionKind::Mood), vec!["alert".to_string()]);
        assert_eq!(only(&doc, "severity.contained.text"), Expr::Color(Color::from_hex("#E8B33A").unwrap().to_linear()));
        assert_eq!(only(&doc, "decor.vignette.strength"), Expr::Num(0.55));
        assert!(matches!(only(&doc, "render.hull"), Expr::Call(Func::Sat, _)));
        assert_eq!(doc.meta_text("meta.name").as_deref(), Some("Lockdown"));
        // `mode = hue` is an enum word, not a call to the hue() function
        assert_eq!(only(&doc, "severity.mode"), Expr::Word("hue".into()));
    }

    #[test]
    fn percent_is_also_a_frac_spelling() {
        let (e, d) = val("55%");
        assert!(d.is_empty());
        assert_eq!(e, Expr::Len(55.0, Unit::Pct));
    }

    #[test]
    fn oklch_takes_references_and_ratios_as_components() {
        let (e, d) = val("oklch(0.870, 0.55x@chroma.accent, @hue.accent)");
        assert!(d.is_empty(), "{d:?}");
        match e {
            Expr::Oklch(l, c, h, a) => {
                assert_eq!(*l, Expr::Num(0.870));
                assert!(matches!(*c, Expr::Ratio(..)));
                assert_eq!(*h, Expr::Ref("hue.accent".into(), None));
                assert!(a.is_none());
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn rgb_accepts_both_component_scales_and_an_alpha() {
        let (e, d) = val("rgb(63 227 174 / 0.5)");
        assert!(d.is_empty(), "{d:?}");
        assert!(matches!(e, Expr::Rgb(_, _, _, Some(_))));
    }
}
