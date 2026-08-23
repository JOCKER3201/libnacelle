//! Spec -> resolved values: the memoised DAG walk over the merged tree, the
//! per-token fallback, and §4.2's error policy.
//!
//! > **This program must never fail to start because a theme file is wrong.**
//!
//! That is the load-bearing rule of the whole design, so it is the rule this
//! module is written against: **nothing here panics and nothing here refuses to
//! produce a theme.** Every recoverable defect degrades to `default`'s value for
//! the affected token and is reported with file, line, column, the offending
//! text and the reason. Silent substitution is forbidden — it is the failure
//! mode this project already calls out as the worst kind.
//!
//! The walk itself is §6.3's: an explicit stack, three-colour marking, depth cap
//! 32, every node evaluated at most once. Order in the file is irrelevant and
//! forward references are legal and used.
//!
//! Not in this stage: `enforce.rs`. The contrast floors, the perceptual
//! separation repair and the honesty lints of §4.4 run *after* `encode.rs` on
//! the values the GPU will actually blend, so they depend on the swapchain
//! format and on renderer work (Appendix B). Everything they need is already
//! here — `ensure()` (§6), `Color::wcag_contrast`, `Color::apca_lc`,
//! `Color::delta_e_ok` and `Color::composite_as_rendered` — and the engine is
//! complete and useful without them.

use super::cascade::{Schema, ThemeSpec, TokenId};
use super::color::Color;
use super::expr::{self, EvalError, Evaluator, Expr, Kind, Value};
use super::parse::{Diagnostic, Span};

/// One fully resolved theme, still symbolic about units: `bake.rs` turns those
/// into absolute px once a screen height is known.
pub struct Resolved {
    pub label: String,
    /// Indexed by [`TokenId`]; always exactly `schema.len()` long.
    pub values: Vec<Value>,
    /// The mood's transition tint (§5.24), if this spec came from one.
    pub wash: Option<Color>,
    /// Tokens whose expression names the `[state]` block's `base` keyword and
    /// which are therefore templates, not values. Reported once, not per token.
    pub deferred: usize,
    /// The `class.*` tokens, in declaration order — the classes of §5.27.
    pub class_ids: Vec<TokenId>,
    /// `class_ids[i]`'s state-ladder family: 0 = the bare `[state]` ladder
    /// (every class not named in `[family]`, "button" in spirit), 1 =
    /// `[state.input]`, 2 = `[state.window]`. A theme file names families by
    /// word (`field = base`, `window = window` under `[family]`), not by
    /// number — this array is where that word becomes an index.
    pub class_family: Vec<u8>,
    /// The three ladders a class can be assigned to, indexed by
    /// `class_family`'s 0/1/2: the bare `state.<state>.<channel>` tokens,
    /// then `state.input.*` and `state.window.*`. Each class's row in
    /// `class_states` was evaluated against exactly one of these three.
    pub state_ladders: [Vec<TokenId>; 3],
    /// `class_states[class][i]` = the value of
    /// `state_ladders[class_family[class]][i]` with `base` bound to
    /// `class_ids[class]`'s own colour. Raw values — bake turns them into
    /// [`super::bake::StateStyle`]s.
    pub class_states: Vec<Vec<Value>>,
}

impl Resolved {
    pub fn get(&self, id: TokenId) -> Option<&Value> {
        self.values.get(id.index())
    }
}

/// Resolve a merged spec. Always succeeds.
pub fn resolve(schema: &Schema, spec: &ThemeSpec, out: &mut Vec<Diagnostic>) -> Resolved {
    let n = schema.len();
    let mut r = Resolver {
        schema,
        spec,
        ev: Evaluator::new(n),
        diags: Vec::new(),
        deferred: 0,
        state_base: None,
        breaking: Vec::new(),
    };
    let mut values = Vec::with_capacity(n);
    for i in 0..n {
        let id = TokenId(i as u16);
        let v = r.token(id).unwrap_or_else(|e| {
            // Unreachable in practice: `token()` already applied the fallback
            // ladder. Kept because "never panics" must survive a future edit.
            r.diags.push(Diagnostic::warn(spec.span(id), e.message()));
            fallback(schema, id)
        });
        values.push(v);
    }
    let wash = spec
        .wash
        .as_ref()
        .and_then(|e| expr::eval(e, &mut Wash { inner: &mut r }).ok())
        .and_then(|v| v.as_color());
    let deferred = r.deferred;
    out.append(&mut r.diags);

    // The class x state pass (§5.21 x §5.27). Classes are the `class.*`
    // colour tokens in declaration order. The ladder used to be one global
    // `state.*` list; it is now up to three, because a container (window,
    // panel, dialog) and an input surface (field, checkbox, a slider's
    // track and knob...) do not press or select the way a button does, and
    // dressing all 26 classes in one formula was the whole reason 22 of
    // them read as the same object with a different name. A bare
    // `state.<state>.<channel>` token belongs to ladder 0 (the ladder every
    // class had before this split, and still the default for a class
    // `[family]` says nothing about); `state.input.*` and `state.window.*`
    // are the two named ladders `[family]` can point a class at. All three
    // lists come straight from the schema, so a master with no `[class]`
    // block simply produces empty matrices and every control draws RAW —
    // the governing principle working, not an error.
    let mut class_ids = Vec::new();
    let mut state_ladders: [Vec<TokenId>; 3] = [Vec::new(), Vec::new(), Vec::new()];
    let mut family_ids = Vec::new();
    for i in 0..n {
        let id = TokenId(i as u16);
        let name = schema.name(id);
        if name.starts_with("class.") {
            class_ids.push(id);
        } else if let Some(rest) = name.strip_prefix("state.") {
            let first = rest.split('.').next().unwrap_or("");
            if super::parse::State::from_name(first).is_some() {
                state_ladders[0].push(id);
            } else if first == "input" {
                state_ladders[1].push(id);
            } else if first == "window" {
                state_ladders[2].push(id);
            }
            // An unrecognised word after "state." names no ladder this
            // engine knows: the token still exists (cascade.rs already
            // warned if nothing declared it), it just never enters a
            // class x state pass.
        } else if name.starts_with("family.") {
            family_ids.push(id);
        }
    }
    let bases: Vec<Color> = class_ids
        .iter()
        .map(|&id| values[id.index()].as_color().unwrap_or(Color::GREY))
        .collect();

    // `[family]` names a class by the same dotted key `[class]` uses
    // (`slider.track = input`), so the lookup strips "class." off the
    // class token's own name and asks the schema for "family.<that>".
    // A class `[family]` says nothing about keeps ladder 0 - the bare
    // `[state]` ladder is the one every class read before this split, so
    // silence means "unchanged", not "undefined". The family word is NOT
    // "base": `base` is already the state ladder's own keyword for "this
    // class's own colour" (`resolve_states`' `state_base`), and reusing it
    // here would make `field = base` try to evaluate that binding instead
    // of naming a family - hence "input" for the ladder Qt calls Base.
    let class_family: Vec<u8> = class_ids
        .iter()
        .map(|&cid| {
            let class_name = schema.name(cid).strip_prefix("class.").unwrap_or("");
            let want = format!("family.{class_name}");
            family_ids
                .iter()
                .find(|&&fid| schema.name(fid) == want)
                .and_then(|&fid| values[fid.index()].as_word())
                .map(|w| match w {
                    "input" => 1u8,
                    "window" => 2u8,
                    _ => 0u8,
                })
                .unwrap_or(0u8)
        })
        .collect();

    let rows_per_ladder: [Vec<Vec<Value>>; 3] = [
        resolve_states(schema, spec, &bases, &state_ladders[0], out),
        resolve_states(schema, spec, &bases, &state_ladders[1], out),
        resolve_states(schema, spec, &bases, &state_ladders[2], out),
    ];
    let class_states: Vec<Vec<Value>> = class_family
        .iter()
        .enumerate()
        .map(|(ci, &fam)| rows_per_ladder[fam as usize][ci].clone())
        .collect();

    Resolved {
        label: spec.label.clone(),
        values,
        wash,
        deferred,
        class_ids,
        class_family,
        state_ladders,
        class_states,
    }
}

/// Resolving `default.theme` alone, for [`Schema::adopt_kinds`] and for the
/// unit test that asserts the shipped master is cycle-free by construction
/// (§6.3 step 4).
pub fn resolve_default(schema: &Schema, out: &mut Vec<Diagnostic>) -> Resolved {
    let spec = schema.base_spec();
    resolve(schema, &spec, out)
}

/// The class x state pass (§5.21 x §5.27): evaluates the given `[state]`
/// tokens once per class, with the ladder's `base` keyword bound to that
/// class's own resolved colour.
///
/// A fresh evaluator per class, deliberately: a memo warmed with one class's
/// base would leak its colours into the next. The state ladder is ~56 tokens
/// and the class list ~25, so the whole pass is ~1400 evaluations of two-call
/// expressions — load-time noise.
pub fn resolve_states(
    schema: &Schema,
    spec: &ThemeSpec,
    bases: &[Color],
    ids: &[TokenId],
    out: &mut Vec<Diagnostic>,
) -> Vec<Vec<Value>> {
    let n = schema.len();
    let mut per_class = Vec::with_capacity(bases.len());
    for &base in bases {
        let mut r = Resolver {
            schema,
            spec,
            ev: Evaluator::new(n),
            diags: Vec::new(),
            deferred: 0,
            state_base: Some(base),
            breaking: Vec::new(),
        };
        let mut row = Vec::with_capacity(ids.len());
        for &id in ids {
            let v = r.token(id).unwrap_or_else(|_| fallback(schema, id));
            row.push(v);
        }
        out.append(&mut r.diags);
        per_class.push(row);
    }
    per_class
}

// ------------------------------------------------------------------ resolver

struct Resolver<'a> {
    schema: &'a Schema,
    spec: &'a ThemeSpec,
    ev: Evaluator,
    diags: Vec<Diagnostic>,
    deferred: usize,
    /// The class base colour the `[state]` ladder's `base` keyword binds to,
    /// during the class x state pass. `None` outside it — the global walk
    /// leaves state templates deferred exactly as before.
    state_base: Option<Color>,
    /// Tokens currently being evaluated from `default` after a cycle. Without
    /// it, a cyclic `default` would recurse forever instead of falling through
    /// to the per-kind value.
    breaking: Vec<usize>,
}

/// A tiny adaptor so the mood's `wash` — which is not a token — can be
/// evaluated against the same tree.
struct Wash<'a, 'b> {
    inner: &'a mut Resolver<'b>,
}

impl expr::Env for Wash<'_, '_> {
    fn resolve(&mut self, name: &str, index: Option<u32>) -> Result<Value, EvalError> {
        expr::Env::resolve(self.inner, name, index)
    }
}

impl Resolver<'_> {
    /// The value of one token, memoised, with §4.2's fallback ladder:
    ///
    /// 1. the merged expression,
    /// 2. on any failure, **`default`'s** expression for that token,
    /// 3. and if that also fails, the per-kind fallback — §4.1's stage 1, which
    ///    under "`default.theme` is the schema" is a kind rather than a table.
    fn token(&mut self, id: TokenId) -> Result<Value, EvalError> {
        if let Some(v) = self.ev.cached(id.index()) {
            return Ok(v.clone());
        }
        let name = self.schema.name(id).to_string();
        if let Err(err) = self.ev.enter(id.index(), &name) {
            // Re-entering a Grey node, or past depth 32. §4.2 attributes both
            // to the token that was RE-ENTERED — the head of the cycle — not to
            // whichever of its dependents happened to close it, because the
            // head is the declaration the author wrote and the one whose
            // default expression is about to be used instead.
            return Ok(self.break_cycle(id, &name, &err));
        }
        let e = match self.spec.get(id) {
            Some(e) => e.clone(),
            None => {
                self.ev.abandon(id.index());
                return Ok(fallback(self.schema, id));
            }
        };
        if self.schema.deferred(id) {
            self.deferred += 1;
        }
        match expr::eval(&e, self) {
            Ok(v) => {
                self.ev.leave(id.index(), v.clone());
                Ok(v)
            }
            Err(err) => {
                self.ev.abandon(id.index());
                self.report(id, &name, &err);
                self.from_default(id, &name)
            }
        }
    }

    /// The cycle break. Reports the full path, then evaluates this one token
    /// from `default`'s expression without re-entering the guard — the id is
    /// already Grey on an outer frame, which is precisely the situation.
    /// `breaking` stops a `default` that is *itself* cyclic from recursing.
    fn break_cycle(&mut self, id: TokenId, name: &str, err: &EvalError) -> Value {
        if self.breaking.contains(&id.index()) {
            return fallback(self.schema, id);
        }
        self.report(id, name, err);
        let Some(d) = self.schema.default_expr(id).cloned() else {
            return fallback(self.schema, id);
        };
        if Some(&d) == self.spec.get(id) {
            // The cyclic expression *was* default's. Nothing left to try, and
            // §4.2 says so: "If that also cycles the token takes its FALLBACK
            // value" — which under "default.theme is the schema" is per kind.
            return fallback(self.schema, id);
        }
        self.breaking.push(id.index());
        let v = expr::eval(&d, self).unwrap_or_else(|_| fallback(self.schema, id));
        self.breaking.pop();
        v
    }

    /// Step 2 and 3 of the ladder. Deliberately not recursive through
    /// [`Resolver::token`], so a `default` that also fails cannot loop.
    fn from_default(&mut self, id: TokenId, name: &str) -> Result<Value, EvalError> {
        let Some(d) = self.schema.default_expr(id).cloned() else {
            return Ok(fallback(self.schema, id));
        };
        if Some(&d) == self.spec.get(id) {
            // The failing expression *was* default's. Nothing left to try.
            return Ok(fallback(self.schema, id));
        }
        // Re-enter under the same cycle guard: default's own expression may
        // reference tokens, and those must still be depth- and cycle-checked.
        if self.ev.enter(id.index(), name).is_err() {
            return Ok(fallback(self.schema, id));
        }
        match expr::eval(&d, self) {
            Ok(v) => {
                self.ev.leave(id.index(), v.clone());
                Ok(v)
            }
            Err(err) => {
                self.ev.abandon(id.index());
                self.diags.push(Diagnostic::warn(
                    self.schema.default_span(id),
                    format!(
                        "default's expression for \"{name}\" also failed ({}); \
                         using the compiled-in fallback",
                        err.message()
                    ),
                ));
                Ok(fallback(self.schema, id))
            }
        }
    }

    /// §4.2 and §4.3: name the token, the reason, and — for a cycle — the full
    /// path plus what `default`'s expression will be used instead.
    fn report(&mut self, id: TokenId, name: &str, err: &EvalError) {
        let span = self.spec.span(id);
        let msg = match err {
            EvalError::Cycle(path) | EvalError::TooDeep(path) => {
                let human = self.humanise(path, name);
                let d = self
                    .schema
                    .default_expr(id)
                    .map(|e| describe(e))
                    .unwrap_or_else(|| "the compiled-in fallback".into());
                let head = if matches!(err, EvalError::TooDeep(_)) {
                    "reference depth > 32 (treated as a cycle)"
                } else {
                    "reference cycle"
                };
                format!("{head}: {}\n({name} uses default's expression: {d})", human.join(" -> "))
            }
            other => format!(
                "{} (using default's expression for \"{name}\")",
                other.message()
            ),
        };
        self.diags.push(Diagnostic::warn(span, msg));
    }

    /// The evaluator records ids; §4.3 prints names.
    fn humanise(&self, path: &[String], last: &str) -> Vec<String> {
        path.iter()
            .map(|s| match s.strip_prefix('#').and_then(|n| n.parse::<u16>().ok()) {
                Some(i) => self.schema.name(TokenId(i)).to_string(),
                None => last.to_string(),
            })
            .collect()
    }
}

impl expr::Env for Resolver<'_> {
    fn resolve(&mut self, name: &str, index: Option<u32>) -> Result<Value, EvalError> {
        let full = match index {
            Some(i) => format!("{name}[{i}]"),
            None => name.to_string(),
        };
        if let Some(id) = self.schema.id(&full) {
            return self.token(id);
        }
        // `@term.ansi` with no index gathers the family's slots — the view §7.1
        // describes, so an indexed family stays addressable as a whole without
        // a second storage.
        if index.is_none() {
            if let Some(fam) = self.schema.family(name).map(|f| f.to_vec()) {
                let mut out = Vec::with_capacity(fam.len());
                for (_, id) in fam {
                    out.push(self.token(id)?);
                }
                return Ok(Value::Array(out));
            }
        }
        // A reference may name a GROUP rather than a value: `@grad.spectrum` is
        // "the gradient called spectrum", `@glow.panel_edge` "the glow class
        // called panel_edge". The group's members are declared
        // (`grad.spectrum.stops`, `glow.panel_edge.radius`), the bare name is
        // not, and the consumer looks the group up by name at draw time. So a
        // prefix with declared children resolves to the name itself — a handle,
        // not a colour — and only a prefix with nothing under it is unknown.
        if index.is_none() {
            let prefix = format!("{name}.");
            if self.schema.names().any(|n| n.starts_with(&prefix)) {
                return Ok(Value::Word(name.to_string()));
            }
        }
        Err(EvalError::UnknownToken(full))
    }

    fn base(&mut self) -> Option<Color> {
        // "the class's own base colour". Bound during the class x state pass
        // (resolve_states); None on the global walk, where a state template
        // resolves against white and stays counted as deferred.
        self.state_base
    }
}

/// §4.1 stage 1, the compiled-in fallback. It cannot be a `const
/// FALLBACK: ResolvedTheme` under "`default.theme` is the schema" — there is no
/// hand-written table to hold one — so it is a value per *kind*, which is the
/// same guarantee: stage 2 itself failing still yields a running program.
pub fn fallback(schema: &Schema, id: TokenId) -> Value {
    match schema.kind(id) {
        // The raw look, which is the same raw look a MISSING token draws:
        // an expression that would not resolve and a token that was never
        // declared are the same story to a reader, and telling them apart
        // by two shades of grey nobody can name is not a diagnostic — the
        // accompanying warning is.
        Kind::Color => Value::Color(super::raw::INK),
        Kind::Scalar => Value::Num(0.0),
        Kind::Flag => Value::Bool(false),
        Kind::Enum => Value::Word(
            // The master's own default word, not the declared list's first
            // entry: a comment-declared list is ordered for NUMBERING
            // (`enum_of` indexes it), not for preference.
            match schema.default_expr(id) {
                Some(Expr::Word(w)) => w.clone(),
                _ => schema.enum_words(id).first().cloned().unwrap_or_else(|| "none".into()),
            },
        ),
        Kind::Text => Value::Text(String::new()),
    }
}

fn describe(e: &Expr) -> String {
    match e {
        Expr::Color(c) => c.to_srgb().to_hex(),
        Expr::Num(v) => format!("{v}"),
        Expr::Len(v, u) => format!("{v}{}", u.name()),
        Expr::Bool(b) => format!("{b}"),
        Expr::Word(w) | Expr::Text(w) => w.clone(),
        Expr::Codepoint(c) => format!("U+{c:04X}"),
        Expr::Ref(p, None) => format!("@{p}"),
        Expr::Ref(p, Some(i)) => format!("@{p}[{i}]"),
        Expr::Ratio(k, t) => format!("{k}x {}", describe(t)),
        Expr::Array(v) => format!("[{} values]", v.len()),
        Expr::Rgb(..) => "rgb(...)".into(),
        Expr::Oklch(..) => "oklch(...)".into(),
        Expr::Call(f, a) => format!(
            "{}({})",
            f.name(),
            a.iter().map(describe).collect::<Vec<_>>().join(", ")
        ),
        Expr::Bad(_) => "<malformed>".into(),
    }
}

/// A one-line summary for stderr, so a load reports something even when nothing
/// went wrong (§4.2: reports go to four places, and stderr is the first).
pub fn summarise(r: &Resolved, diags: &[Diagnostic]) -> String {
    let warns = diags.iter().filter(|d| d.level != super::parse::Level::Note).count();
    format!(
        "theme \"{}\": {} tokens, {} warnings, {} deferred state templates",
        r.label,
        r.values.len(),
        warns,
        r.deferred
    )
}

/// Where a token's winning declaration came from, for `--dump-theme` (§9.5).
pub fn origin(spec: &ThemeSpec, id: TokenId) -> Span {
    spec.span(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::cascade::{cascade, Options, Schema, Stage};
    use crate::theme::expr::Unit;
    use crate::theme::parse::{parse, Document, Sources};

    const DEFAULT: &str = "\
[palette]
black = #0A100E
white = #EAF6F1
accent = #3FE3AE
[accent]
primary = @palette.accent
hover = tint(@accent.primary, 0.18)
[text]
title = @accent.hover
[stroke]
hair = 0.2
[border]
width = @stroke.hair
[surface]
panel = mix(@palette.black, @palette.accent, 0.06)
[panel]
content_pad = 2.8u
title_h = 5.2u
pad_ratio = 0.35x @panel.title_h
mode = round
[decor]
enabled = true
[term]
ansi = [ #000000, #CD3131, #0DBC79, #E5E510 ]
";

    fn setup(theme: &str) -> (Schema, Document, Sources, Vec<Diagnostic>) {
        let mut src = Sources::new();
        let mut out = Vec::new();
        let f = src.add("default.theme", DEFAULT);
        let d = parse(&mut src, f, None, &mut out);
        let mut schema = Schema::from_default(&d, &mut out);
        let r = resolve_default(&schema, &mut out);
        schema.adopt_kinds(&r.values);
        let g = src.add("t.theme", theme);
        let doc = parse(&mut src, g, None, &mut out);
        (schema, doc, src, out)
    }

    fn spec_of(schema: &mut Schema, doc: &Document, out: &mut Vec<Diagnostic>) -> ThemeSpec {
        cascade(schema, &[Stage::Document(doc)], Options::default(), out)
    }

    #[test]
    fn the_master_resolves_clean_and_is_cycle_free_by_construction() {
        let (schema, _, src, mut out) = setup("");
        let r = resolve_default(&schema, &mut out);
        let rendered: String = out.iter().map(|d| d.render(&src)).collect();
        assert!(out.is_empty(), "default must resolve clean:\n{rendered}");
        assert_eq!(r.values.len(), schema.len());
        // forward references are legal: text.title reads accent.hover, which is
        // declared after it in file order for accent.primary's chain.
        assert!(r.get(schema.id("text.title").unwrap()).unwrap().as_color().is_some());
    }

    #[test]
    fn a_single_seed_change_re_derives_every_dependent_token() {
        let (mut schema, doc, _, mut out) = setup("[palette]\naccent = #FF2A35\n");
        let spec = spec_of(&mut schema, &doc, &mut out);
        let r = resolve(&schema, &spec, &mut out);
        assert!(out.is_empty(), "{out:?}");
        let hover = r.get(schema.id("accent.hover").unwrap()).unwrap().as_color().unwrap();
        // it followed the new seed's hue, not the old one's
        let red = Color::from_hex("#FF2A35").unwrap().to_linear().to_oklch().h;
        assert!((hover.to_oklch().h - red).abs() < 20.0, "hue {} vs {red}", hover.to_oklch().h);
    }

    #[test]
    fn a_deliberate_cycle_reports_the_full_path_and_uses_defaults_expression() {
        // accent.hover = @text.title, and text.title = @accent.hover already.
        let (mut schema, doc, _, mut out) = setup("[accent]\nhover = @text.title\n");
        let spec = spec_of(&mut schema, &doc, &mut out);
        out.clear();
        let r = resolve(&schema, &spec, &mut out);
        let msg = out
            .iter()
            .map(|d| d.message.clone())
            .find(|m| m.contains("reference cycle"))
            .unwrap_or_else(|| format!("no cycle reported: {out:?}"));
        assert!(msg.contains("accent.hover"), "{msg}");
        assert!(msg.contains("text.title"), "{msg}");
        // §4.2: "then evaluate the token using default's expression"
        assert!(msg.contains("uses default's expression"), "{msg}");
        assert!(msg.contains("tint("), "{msg}");
        // and the theme still resolved to something usable
        let hover = r.get(schema.id("accent.hover").unwrap()).unwrap();
        assert!(hover.as_color().is_some(), "{hover:?}");
    }

    #[test]
    fn a_self_reference_is_a_cycle_not_a_hang() {
        let (mut schema, doc, _, mut out) = setup("[accent]\nprimary = @accent.primary\n");
        let spec = spec_of(&mut schema, &doc, &mut out);
        out.clear();
        let r = resolve(&schema, &spec, &mut out);
        assert!(out.iter().any(|d| d.message.contains("reference cycle")), "{out:?}");
        assert!(r.get(schema.id("accent.primary").unwrap()).unwrap().as_color().is_some());
    }

    #[test]
    fn a_reference_to_an_unknown_token_falls_back_for_the_referring_token() {
        let (mut schema, doc, _, mut out) = setup("[accent]\nhover = @palette.nonesuch\n");
        let spec = spec_of(&mut schema, &doc, &mut out);
        out.clear();
        let r = resolve(&schema, &spec, &mut out);
        let m = &out[0].message;
        assert!(m.contains("unknown token \"palette.nonesuch\""), "{m}");
        assert!(m.contains("using default's expression for \"accent.hover\""), "{m}");
        // default's tint(@accent.primary, 0.18) was used, so the value is sane
        let hover = r.get(schema.id("accent.hover").unwrap()).unwrap().as_color().unwrap();
        let base = r.get(schema.id("accent.primary").unwrap()).unwrap().as_color().unwrap();
        assert!(hover.to_oklch().l > base.to_oklch().l);
    }

    #[test]
    fn nothing_refuses_to_produce_a_theme_even_when_everything_is_wrong() {
        let (mut schema, doc, _, mut out) = setup(
            "[palette]\naccent = @palette.accent\n[accent]\nhover = @nope.nope\n\
             [panel]\ncontent_pad = @panel.pad_ratio\n",
        );
        let spec = spec_of(&mut schema, &doc, &mut out);
        let r = resolve(&schema, &spec, &mut out);
        assert_eq!(r.values.len(), schema.len());
        for (i, v) in r.values.iter().enumerate() {
            let id = TokenId(i as u16);
            match schema.kind(id) {
                Kind::Color => assert!(v.as_color().is_some(), "{} is not a colour", schema.name(id)),
                Kind::Scalar => assert!(v.as_num().is_some(), "{} is not a number", schema.name(id)),
                _ => {}
            }
        }
    }

    #[test]
    fn every_colour_that_resolved_is_finite_and_no_nan_reaches_a_vertex() {
        let (schema, _, _, mut out) = setup("");
        let r = resolve_default(&schema, &mut out);
        for v in &r.values {
            if let Some(c) = v.as_color() {
                assert!(c.is_finite(), "{c:?}");
                assert!(c.a >= 0.0, "a sentinel reached a colour channel: {c:?}");
            }
            if let Some(n) = v.as_num() {
                assert!(!n.is_nan());
            }
        }
    }

    #[test]
    fn a_ratio_multiplies_out_against_the_merged_tree() {
        let (mut schema, doc, _, mut out) = setup("[panel]\ntitle_h = 10u\n");
        let spec = spec_of(&mut schema, &doc, &mut out);
        let r = resolve(&schema, &spec, &mut out);
        assert_eq!(r.get(schema.id("panel.pad_ratio").unwrap()), Some(&Value::Len(3.5, Unit::U)));
    }

    #[test]
    fn an_indexed_family_is_addressable_per_slot_and_as_a_whole() {
        let (schema, _, _, mut out) = setup("");
        let r = resolve_default(&schema, &mut out);
        assert_eq!(
            r.get(schema.id("term.ansi[1]").unwrap()).unwrap().as_color(),
            Some(Color::from_hex("#CD3131").unwrap().to_linear())
        );
        // @term.ansi with no index gathers the row
        let mut rr = Resolver {
            schema: &schema,
            spec: &schema.base_spec(),
            ev: Evaluator::new(schema.len()),
            diags: Vec::new(),
            deferred: 0,
            state_base: None,
            breaking: Vec::new(),
        };
        match expr::Env::resolve(&mut rr, "term.ansi", None) {
            Ok(Value::Array(v)) => assert_eq!(v.len(), 4),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn the_state_base_keyword_marks_a_template_rather_than_failing() {
        let mut src = Sources::new();
        let mut out = Vec::new();
        let f = src.add("default.theme", "[state]\nidle.fill = alpha(base, 0.07)\n");
        let d = parse(&mut src, f, None, &mut out);
        let schema = Schema::from_default(&d, &mut out);
        let r = resolve_default(&schema, &mut out);
        assert!(out.is_empty(), "{out:?}");
        assert_eq!(r.deferred, 1);
        let v = r.get(schema.id("state.idle.fill").unwrap()).unwrap().as_color().unwrap();
        assert_eq!(v.a, 0.07);
    }

    #[test]
    fn a_depth_32_chain_is_treated_as_a_cycle_and_recovers() {
        // Declared deepest-first on purpose: a memoised walk over a chain
        // written in dependency order never descends at all, so only this
        // ordering actually exercises the cap.
        let mut text = String::from("[palette]\nblack = #000000\nwhite = #FFFFFF\n[chain]\n");
        for i in (1..40).rev() {
            text.push_str(&format!("t{i} = @chain.t{}\n", i - 1));
        }
        text.push_str("t0 = #3FE3AE\n");
        let mut src = Sources::new();
        let mut out = Vec::new();
        let f = src.add("default.theme", &text);
        let d = parse(&mut src, f, None, &mut out);
        let schema = Schema::from_default(&d, &mut out);
        let r = resolve_default(&schema, &mut out);
        assert!(
            out.iter().any(|x| x.message.contains("depth > 32")),
            "no depth diagnostic: {:?}",
            out.iter().map(|x| &x.message).collect::<Vec<_>>()
        );
        // every token still has a value
        assert_eq!(r.values.len(), schema.len());
        assert!(r.get(schema.id("chain.t39").unwrap()).unwrap().as_color().is_some());
    }

    #[test]
    fn the_fallback_ladder_ends_at_a_kind_not_a_panic() {
        let mut src = Sources::new();
        let mut out = Vec::new();
        // default's own expression is broken: step 3 of the ladder.
        let f = src.add("default.theme", "[a]\nb = @nowhere.at.all\nc = 3u\n");
        let d = parse(&mut src, f, None, &mut out);
        let schema = Schema::from_default(&d, &mut out);
        let r = resolve_default(&schema, &mut out);
        assert!(!out.is_empty());
        assert_eq!(r.values.len(), 2);
        assert!(r.get(schema.id("a.b").unwrap()).is_some());
    }

    #[test]
    fn summarise_says_something_even_on_a_clean_load() {
        let (schema, _, _, mut out) = setup("");
        let r = resolve_default(&schema, &mut out);
        let s = summarise(&r, &out);
        assert!(s.contains("tokens") && s.contains("warnings"), "{s}");
    }
}
