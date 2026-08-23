//! The unevaluated value tree, and the fifteen derivation functions of §6.
//!
//! `parse.rs` produces [`Expr`] and never evaluates. Everything that turns an
//! `Expr` into a [`Value`] is here: the fifteen functions, each in the colour
//! space §6 names for it and with its own clamping, and the [`Evaluator`] that
//! walks the tree.
//!
//! **Fifteen. Closed. No more.** There is no metavariable (`@severity.<r>.fill`
//! is not an expression) and no runtime query (`@severity.<highest live>.text` is
//! not one either): anything indexed by something only the host knows at draw
//! time is indexed at draw time. `composite_as_rendered` (§4.4) lives in
//! `color.rs` and is deliberately *not* here — it is not authorable and does not
//! appear in [`Func::from_name`].
//!
//! ### Evaluation (§6.3)
//!
//! A memoised DAG walk with **three-colour marking** — White unvisited, Grey
//! on-stack, Black done — and a **depth cap of 32**. Order in the file is
//! irrelevant and forward references are legal and used. Re-entering a Grey node
//! is a cycle: the walk reports the full path (`a -> b -> a`) and the caller
//! (`resolve.rs`) re-evaluates that one token from `default`'s expression
//! ([CONFLICT 9], §4.2). Every node is evaluated at most once; the memo is a
//! `Vec<Option<Value>>` indexed by token id, never a map.

use super::color::{Color, Oklab, Oklch};

// ---------------------------------------------------------------------- units

/// The units of §3.2. Units are **mandatory on every length**; a bare number is
/// a different type, not "8 px".
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unit {
    /// metric units — the default authoring unit
    U,
    /// device pixels; legal ONLY on `*_min_px` / `*_max_px`
    Px,
    /// fraction of the host rect on the token's axis; baked 0..1
    Pct,
    /// multiple of the owning type role's resolved px
    Em,
    Deg,
    Ms,
    S,
    Hz,
    /// DEPRECATED: 1 ux = 1 px at 1440p = 0.13889u
    Ux,
    /// DEPRECATED migration units
    Vh,
    Vw,
}

impl Unit {
    pub fn from_str(s: &str) -> Option<Unit> {
        Some(match s {
            "u" => Unit::U,
            "px" => Unit::Px,
            "%" => Unit::Pct,
            "em" => Unit::Em,
            "deg" => Unit::Deg,
            "ms" => Unit::Ms,
            "s" => Unit::S,
            "hz" => Unit::Hz,
            "ux" => Unit::Ux,
            "vh" => Unit::Vh,
            "vw" => Unit::Vw,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Unit::U => "u",
            Unit::Px => "px",
            Unit::Pct => "%",
            Unit::Em => "em",
            Unit::Deg => "deg",
            Unit::Ms => "ms",
            Unit::S => "s",
            Unit::Hz => "hz",
            Unit::Ux => "ux",
            Unit::Vh => "vh",
            Unit::Vw => "vw",
        }
    }

    /// `ux`, `vh` and `vw` warn on use and are rewritten by §4.2's alias table.
    pub fn deprecated(self) -> bool {
        matches!(self, Unit::Ux | Unit::Vh | Unit::Vw)
    }
}

// ------------------------------------------------------------------ functions

/// The fifteen legal function names of §3.2's `fn-name`. Closed set.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Alpha,
    Fade,
    Mix,
    Over,
    Shade,
    Tint,
    Lum,
    LumMin,
    LumMax,
    Sat,
    Hue,
    Ramp,
    ContrastOn,
    Ensure,
    /// `toward(colour, target, pull, clamp_deg)` — the fifteenth, added
    /// 2026-08-18 for the one thing §5.10 and §5.11 both describe in prose
    /// and neither could write down: **a canonical hue that leans toward
    /// the theme's own without ever leaving itself.**
    ///
    /// The master says of the severity roles that "the engine pulls each of
    /// them toward the accent by `severity.pull`, clamped to
    /// `severity.pull_clamp`", and of the ANSI row the same thing with
    /// `term.ansi.pull`. That engine never existed: the four severity
    /// controls sat on `theme::edit`'s DEAD list "until someone writes their
    /// reader", and the sentence in the theme file was a promise about a
    /// machine. Written as a FUNCTION rather than as a pass in `bake.rs`,
    /// the promise keeps itself:
    ///
    /// * a role's value goes on being ONE expression in the theme file, so
    ///   the pull is visible where the colour is written and a theme can
    ///   change it, drop it, or point it somewhere else entirely;
    /// * **a theme that pins a role escapes the pull by construction** —
    ///   §5.10 promises exactly that, and a bake-time pass could not keep it
    ///   without provenance the baker does not have. Writing
    ///   `severity.critical.text = oklch(...)` overrides the whole
    ///   expression, `toward()` and all;
    /// * the numbers stay in the theme (`@severity.pull`,
    ///   `@severity.pull_clamp`), which is the project's rule about where
    ///   appearance lives. This function contributes arithmetic and no
    ///   value at all.
    ///
    /// HUE ONLY, and both walls are what the numbers MEAN rather than
    /// taste: `pull` is a fraction of the SHORTEST way round the circle
    /// (so a colour never takes the long way to a target 10 deg away), and
    /// `clamp_deg` is a hard ceiling in degrees on the walk — the master's
    /// own measurement of why it must be there is at
    /// `default.theme`'s `severity.pull_clamp`. L and C are untouched: a
    /// canonical red pulled toward a mint accent has to stay as red and as
    /// bright as its author made it, and only lean.
    Toward,
}

impl Func {
    pub const ALL: [Func; 15] = [
        Func::Alpha, Func::Fade, Func::Mix, Func::Over, Func::Shade, Func::Tint,
        Func::Lum, Func::LumMin, Func::LumMax, Func::Sat, Func::Hue, Func::Ramp,
        Func::ContrastOn, Func::Ensure, Func::Toward,
    ];

    pub fn from_name(s: &str) -> Option<Func> {
        Some(match s {
            "alpha" => Func::Alpha,
            "fade" => Func::Fade,
            "mix" => Func::Mix,
            "over" => Func::Over,
            "shade" => Func::Shade,
            "tint" => Func::Tint,
            "lum" => Func::Lum,
            "lum_min" => Func::LumMin,
            "lum_max" => Func::LumMax,
            "sat" => Func::Sat,
            "hue" => Func::Hue,
            "ramp" => Func::Ramp,
            "contrast_on" => Func::ContrastOn,
            "ensure" => Func::Ensure,
            "toward" => Func::Toward,
            _ => return None,
        })
    }

    pub fn name(self) -> &'static str {
        match self {
            Func::Alpha => "alpha",
            Func::Fade => "fade",
            Func::Mix => "mix",
            Func::Over => "over",
            Func::Shade => "shade",
            Func::Tint => "tint",
            Func::Lum => "lum",
            Func::LumMin => "lum_min",
            Func::LumMax => "lum_max",
            Func::Sat => "sat",
            Func::Hue => "hue",
            Func::Ramp => "ramp",
            Func::ContrastOn => "contrast_on",
            Func::Ensure => "ensure",
            Func::Toward => "toward",
        }
    }

    pub fn arity(self) -> usize {
        match self {
            Func::Alpha | Func::Fade | Func::Shade | Func::Tint | Func::Lum
            | Func::LumMin | Func::LumMax | Func::Sat | Func::Hue | Func::Over => 2,
            Func::Mix | Func::Ramp | Func::ContrastOn | Func::Ensure => 3,
            Func::Toward => 4,
        }
    }

    /// The one line §4.2 requires when a theme names something else:
    /// "names the function and lists the legal names".
    pub fn legal_names() -> String {
        Func::ALL.iter().map(|f| f.name()).collect::<Vec<_>>().join(" ")
    }
}

// ----------------------------------------------------------------------- AST

/// One authored value, **unevaluated**, exactly as written.
#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    /// A colour literal, already decoded to **linear light, straight alpha**.
    Color(Color),
    /// A bare number: a count, a multiplier, a ratio's left side, a `frac`.
    Num(f32),
    /// A number with a mandatory unit.
    Len(f32, Unit),
    Bool(bool),
    /// A bare identifier: an enum word, a sentinel (`none`/`auto`/`pill`/
    /// `same_as_parent`/`element`) or the `base` keyword of a state block.
    Word(String),
    /// A quoted string, for the fifteen genuinely textual tokens of §3.2.
    Text(String),
    /// `U+2726`
    Codepoint(u32),
    Array(Vec<Expr>),
    /// `@path` or `@path[3]`, resolved against the **merged** tree.
    Ref(String, Option<u32>),
    /// `0.62x @winframe.title_h` — the right side is always a reference.
    Ratio(f32, Box<Expr>),
    /// `rgb(r g b [/ a])`, components 0..255 or 0..1, sRGB-encoded.
    Rgb(Box<Expr>, Box<Expr>, Box<Expr>, Option<Box<Expr>>),
    /// `oklch(L C H [/ a])` — L 0..1, C chroma, H degrees. Any argument may
    /// itself be a reference or a ratio (`oklch(0.87, 0.55x@chroma.accent, ...)`).
    Oklch(Box<Expr>, Box<Expr>, Box<Expr>, Option<Box<Expr>>),
    Call(Func, Vec<Expr>),
    /// A value the parser could not make sense of. It carries the reason so the
    /// diagnostic is produced where the *token* is known, not where the bytes
    /// were. Evaluating one is an error and falls back per §4.2.
    Bad(String),
}

impl Expr {
    /// Does this expression mention the `base` keyword of a `[state]` block?
    /// Such a token is a *template*: its real value is materialised per class by
    /// the class x state bake, which lands with `enforce.rs`.
    pub fn mentions_base(&self) -> bool {
        match self {
            Expr::Word(w) => w == "base",
            Expr::Array(v) | Expr::Call(_, v) => v.iter().any(Expr::mentions_base),
            Expr::Ratio(_, e) => e.mentions_base(),
            Expr::Rgb(a, b, c, d) | Expr::Oklch(a, b, c, d) => {
                a.mentions_base()
                    || b.mentions_base()
                    || c.mentions_base()
                    || d.as_ref().is_some_and(|e| e.mentions_base())
            }
            _ => false,
        }
    }

    /// Every `@token` this expression depends on, for the dependency walk and
    /// for `--dump-theme`'s default chain.
    pub fn refs(&self, out: &mut Vec<(String, Option<u32>)>) {
        match self {
            Expr::Ref(p, i) => out.push((p.clone(), *i)),
            Expr::Array(v) | Expr::Call(_, v) => v.iter().for_each(|e| e.refs(out)),
            Expr::Ratio(_, e) => e.refs(out),
            Expr::Rgb(a, b, c, d) | Expr::Oklch(a, b, c, d) => {
                a.refs(out);
                b.refs(out);
                c.refs(out);
                if let Some(e) = d {
                    e.refs(out);
                }
            }
            _ => {}
        }
    }
}

// --------------------------------------------------------------------- values

/// A resolved value. Still symbolic about *units* — `bake.rs` turns those into
/// absolute px — but no longer symbolic about references or functions.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Color(Color),
    Num(f32),
    Len(f32, Unit),
    Bool(bool),
    Word(String),
    Text(String),
    Codepoint(u32),
    Array(Vec<Value>),
}

/// Which of `ResolvedTheme`'s four arrays a token lands in.
///
/// Text is the fifth kind and lands in none of them: it lives in
/// `ThemeDiagnostics`, off every draw path (§7.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Color,
    Scalar,
    Flag,
    Enum,
    Text,
}

impl Value {
    pub fn kind(&self) -> Kind {
        match self {
            Value::Color(_) => Kind::Color,
            Value::Num(_) | Value::Len(..) | Value::Codepoint(_) => Kind::Scalar,
            Value::Bool(_) => Kind::Flag,
            // A sentinel word (`none`, `auto`, `pill`, `same_as_parent`) is a
            // scalar with a negative magic value (§5.0); every other bare word
            // is an enum member.
            Value::Word(w) => {
                if sentinel(w).is_some() {
                    Kind::Scalar
                } else {
                    Kind::Enum
                }
            }
            Value::Text(_) => Kind::Text,
            // An array only survives as the gathered view of an indexed family
            // (`@term.ansi`); its slots are separate tokens.
            Value::Array(v) => v.first().map(Value::kind).unwrap_or(Kind::Scalar),
        }
    }

    pub fn as_color(&self) -> Option<Color> {
        match self {
            Value::Color(c) => Some(*c),
            _ => None,
        }
    }

    /// The bare word of an enum-typed value (`family = base` reads as
    /// `Some("base")`), for a caller matching against a small fixed set of
    /// names rather than wanting a colour or a number.
    pub fn as_word(&self) -> Option<&str> {
        match self {
            Value::Word(w) => Some(w.as_str()),
            _ => None,
        }
    }

    /// The scalar reading of a value, for a function argument that wants a
    /// number. `%` reads as its fraction so `55%` and `0.55` are the same
    /// argument (§3.2).
    pub fn as_num(&self) -> Option<f32> {
        match self {
            Value::Num(v) => Some(*v),
            Value::Len(v, Unit::Pct) => Some(v / 100.0),
            Value::Len(v, _) => Some(*v),
            Value::Codepoint(c) => Some(*c as f32),
            Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            Value::Word(w) => sentinel(w),
            _ => None,
        }
    }

    pub fn type_name(&self) -> &'static str {
        match self {
            Value::Color(_) => "colour",
            Value::Num(_) => "number",
            Value::Len(..) => "length",
            Value::Bool(_) => "bool",
            Value::Word(_) => "enum word",
            Value::Text(_) => "text",
            Value::Codepoint(_) => "codepoint",
            Value::Array(_) => "array",
        }
    }
}

/// §5.0's sentinel table — one word, one baked `f32`, never overloaded onto a
/// colour channel. A consumer testing `if v < 0.0` handles all of them.
pub fn sentinel(word: &str) -> Option<f32> {
    Some(match word {
        "none" => 0.0,
        "auto" => -1.0,
        "pill" => -2.0,
        "same_as_parent" => -3.0,
        _ => return None,
    })
}

// ------------------------------------------------------------------- eval ctx

#[derive(Clone, Debug, PartialEq)]
pub enum EvalError {
    /// `accent.hover -> text.title -> accent.hover`
    Cycle(Vec<String>),
    /// depth > 32; §4.2 treats it as a cycle
    TooDeep(Vec<String>),
    UnknownToken(String),
    /// wrong arity, wrong argument type, a colour where a number was wanted
    Bad(String),
}

impl EvalError {
    pub fn message(&self) -> String {
        match self {
            EvalError::Cycle(path) => format!("reference cycle: {}", path.join(" -> ")),
            EvalError::TooDeep(path) => format!(
                "reference depth > 32 (treated as a cycle): {}",
                path.join(" -> ")
            ),
            EvalError::UnknownToken(t) => format!("reference to an unknown token \"{t}\""),
            EvalError::Bad(m) => m.clone(),
        }
    }
}

/// What the evaluator needs from the outside world: the merged tree.
///
/// `resolve.rs` implements this over the cascaded [`super::cascade::ThemeSpec`];
/// the tests here implement it over a small map, which is the point of it being
/// a trait — the fourteen functions are testable without a cascade.
pub trait Env {
    /// Resolve `@name` / `@name[i]` to a value, recursing as needed.
    fn resolve(&mut self, name: &str, index: Option<u32>) -> Result<Value, EvalError>;
    /// `shade()`'s target. §5.2 requires `palette.black` to be a literal.
    fn black(&mut self) -> Color {
        self.resolve("palette.black", None)
            .ok()
            .and_then(|v| v.as_color())
            .unwrap_or(Color::BLACK)
    }
    /// `tint()`'s target.
    fn white(&mut self) -> Color {
        self.resolve("palette.white", None)
            .ok()
            .and_then(|v| v.as_color())
            .unwrap_or(Color::WHITE)
    }
    /// `ramp()`'s spread in OKLCh lightness — how far apart the rungs of a
    /// data ladder stand. Absent, it is ZERO and every step of the ladder
    /// comes out the base colour: a ladder with no spread is a visible
    /// hole, and a spread invented here would be a look no theme file
    /// could account for.
    fn ramp_span(&mut self) -> f32 {
        self.resolve("metric.ramp_span", None)
            .ok()
            .and_then(|v| v.as_num())
            .unwrap_or(0.0)
    }
    /// The `base` keyword inside a `[state]` block: "the class's own base
    /// colour". At the global ladder there is no class, so the value is a
    /// placeholder and the token is flagged as a template.
    fn base(&mut self) -> Option<Color> {
        None
    }
}

// ----------------------------------------------------------------- evaluation

/// Evaluate one expression against an environment.
pub fn eval(e: &Expr, env: &mut dyn Env) -> Result<Value, EvalError> {
    Ok(match e {
        Expr::Color(c) => Value::Color(*c),
        Expr::Num(v) => Value::Num(*v),
        Expr::Len(v, u) => Value::Len(*v, *u),
        Expr::Bool(b) => Value::Bool(*b),
        Expr::Text(t) => Value::Text(t.clone()),
        Expr::Codepoint(c) => Value::Codepoint(*c),
        Expr::Word(w) => {
            if w == "base" {
                // Unbound at the global ladder. White is the identity for the
                // operators that consume it (`alpha(base, 0.07)` becomes a
                // 7 % white wash), and `resolve.rs` marks the token deferred.
                return Ok(Value::Color(env.base().unwrap_or(Color::WHITE)));
            }
            Value::Word(w.clone())
        }
        Expr::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for it in items {
                out.push(eval(it, env)?);
            }
            Value::Array(out)
        }
        Expr::Ref(path, idx) => env.resolve(path, *idx)?,
        Expr::Ratio(k, target) => {
            let v = eval(target, env)?;
            match v {
                Value::Len(x, u) => Value::Len(k * x, u),
                Value::Num(x) => Value::Num(k * x),
                other => {
                    return Err(EvalError::Bad(format!(
                        "a ratio's target must be a length, found a {}",
                        other.type_name()
                    )))
                }
            }
        }
        Expr::Rgb(r, g, b, a) => {
            let (r, g, b) = (num(r, env)?, num(g, env)?, num(b, env)?);
            let a = match a {
                Some(x) => num(x, env)?,
                None => 1.0,
            };
            // `rgb(63 227 174)` and `rgb(0.25 0.89 0.68)` are both legal; any
            // component above 1 means the author is writing 0..255.
            let s = if r > 1.0 || g > 1.0 || b > 1.0 { 1.0 / 255.0 } else { 1.0 };
            Value::Color(Color::new(r * s, g * s, b * s, a.clamp(0.0, 1.0)).to_linear())
        }
        Expr::Oklch(l, c, h, a) => {
            let (l, c, h) = (num(l, env)?, num(c, env)?, num(h, env)?);
            let a = match a {
                Some(x) => num(x, env)?,
                None => 1.0,
            };
            Value::Color(Color::from_oklch(Oklch { l, c, h, alpha: a.clamp(0.0, 1.0) }))
        }
        Expr::Call(f, args) => {
            // `sat` and `hue` are arity-overloaded: with one argument they
            // READ the colour's chroma or hue as a number, with two they
            // change it. Reading is what lets a tinted near-neutral be
            // written as "the accent's hue at 8 % of its chroma" and follow
            // the seed when a theme moves it (52 such surfaces in the
            // master). Every other function keeps its fixed arity.
            let extracting = matches!(f, Func::Sat | Func::Hue) && args.len() == 1;
            if !extracting && args.len() != f.arity() {
                return Err(EvalError::Bad(format!(
                    "{}() takes {} arguments, found {}",
                    f.name(),
                    f.arity(),
                    args.len()
                )));
            }
            if extracting {
                let p = col(&args[0], env)?.to_oklch();
                return Ok(Value::Num(if matches!(f, Func::Sat) { p.c } else { p.h }));
            }
            Value::Color(call(*f, args, env)?)
        }
        Expr::Bad(reason) => return Err(EvalError::Bad(reason.clone())),
    })
}

fn num(e: &Expr, env: &mut dyn Env) -> Result<f32, EvalError> {
    let v = eval(e, env)?;
    v.as_num()
        .ok_or_else(|| EvalError::Bad(format!("expected a number, found a {}", v.type_name())))
}

fn col(e: &Expr, env: &mut dyn Env) -> Result<Color, EvalError> {
    let v = eval(e, env)?;
    v.as_color()
        .ok_or_else(|| EvalError::Bad(format!("expected a colour, found a {}", v.type_name())))
}

/// The fourteen. Every one is pure, evaluable at load, dependent on no runtime
/// state, and in the space §6 names for it.
fn call(f: Func, a: &[Expr], env: &mut dyn Env) -> Result<Color, EvalError> {
    Ok(match f {
        // --- no colour-channel arithmetic at all -------------------------
        Func::Alpha => col(&a[0], env)?.alpha(num(&a[1], env)?.clamp(0.0, 1.0)),
        Func::Fade => col(&a[0], env)?.fade(num(&a[1], env)?.max(0.0)),
        Func::ContrastOn => {
            let bg = col(&a[0], env)?;
            let x = col(&a[1], env)?;
            let y = col(&a[2], env)?;
            if Color::wcag_contrast(x, bg) >= Color::wcag_contrast(y, bg) { x } else { y }
        }

        // --- linear light: physical compositing --------------------------
        Func::Mix => {
            let (x, y) = (col(&a[0], env)?, col(&a[1], env)?);
            let t = num(&a[2], env)?.clamp(0.0, 1.0);
            mix(x, y, t)
        }
        Func::Over => Color::over(col(&a[0], env)?, col(&a[1], env)?),

        // --- OKLab / OKLCh: perception -----------------------------------
        Func::Shade => {
            let t = num(&a[1], env)?.clamp(0.0, 1.0);
            let target = env.black();
            oklab_toward(col(&a[0], env)?, target, t)
        }
        Func::Tint => {
            let t = num(&a[1], env)?.clamp(0.0, 1.0);
            let target = env.white();
            oklab_toward(col(&a[0], env)?, target, t)
        }
        Func::Lum => {
            let c = col(&a[0], env)?;
            let k = num(&a[1], env)?.max(0.0);
            let mut p = c.to_oklch();
            p.l = (p.l * k).clamp(0.0, 1.0);
            Color::from_oklch(p)
        }
        Func::LumMin => {
            let c = col(&a[0], env)?;
            let l = num(&a[1], env)?.clamp(0.0, 1.0);
            let mut p = c.to_oklch();
            if p.l < l {
                p.l = l;
            }
            Color::from_oklch(p)
        }
        Func::LumMax => {
            let c = col(&a[0], env)?;
            let l = num(&a[1], env)?.clamp(0.0, 1.0);
            let mut p = c.to_oklch();
            if p.l > l {
                p.l = l;
            }
            Color::from_oklch(p)
        }
        Func::Sat => {
            let c = col(&a[0], env)?;
            let k = num(&a[1], env)?.max(0.0);
            let mut p = c.to_oklch();
            p.c = (p.c * k).max(0.0);
            Color::from_oklch(p)
        }
        Func::Hue => {
            let c = col(&a[0], env)?;
            let d = num(&a[1], env)?;
            let mut p = c.to_oklch();
            p.h = (p.h + d).rem_euclid(360.0);
            Color::from_oklch(p)
        }
        Func::Ramp => {
            let c = col(&a[0], env)?;
            let n = num(&a[1], env)?.round().max(1.0) as usize;
            let i = num(&a[2], env)?.round().max(0.0) as usize;
            let span = env.ramp_span();
            ramp(c, n, i, span)
        }
        Func::Ensure => {
            let fg = col(&a[0], env)?;
            let bg = col(&a[1], env)?;
            let ratio = num(&a[2], env)?;
            ensure(fg, bg, ratio)
        }
        Func::Toward => {
            let c = col(&a[0], env)?;
            let target = col(&a[1], env)?;
            let pull = num(&a[2], env)?;
            let clamp_deg = num(&a[3], env)?;
            toward(c, target, pull, clamp_deg)
        }
    })
}

/// `toward(c, target, pull, clamp_deg)`: lean `c`'s hue toward `target`'s,
/// by `pull` of the way, never further than `clamp_deg` degrees.
///
/// THE SHORTEST WAY ROUND, which is what makes this a lean and not a
/// journey: the difference is folded into -180..180 before the fraction is
/// taken, so a colour 10 deg clockwise of its target goes 10 deg clockwise
/// and not 350 the other way. `pull` is held to 0..1 (0 is "never move",
/// which the master documents, and past 1 the colour would overshoot its
/// target and come out on the far side); `clamp_deg` is held at or above
/// zero, and a zero clamp is a second way to say "never move".
///
/// L, C AND ALPHA ARE NOT TOUCHED. A canonical severity red leaning toward
/// a mint accent has to stay the red its author wrote, at the lightness the
/// contrast floors were measured at; all this may do is turn it a little.
/// Anything else belongs to `sat()`, `lum()` or `ensure()`, which the master
/// already wraps around this one where it wants them.
///
/// A GREY TARGET STILL HAS A HUE, and that is deliberate rather than
/// overlooked: `Oklch` carries `h` through zero chroma (`color.rs` keeps it
/// so a drag onto the grey axis does not lose where it came from), so a
/// theme whose accent is a desaturated near-grey still says which way its
/// severity leans. A theme that wants no lean says `pull = 0`.
pub fn toward(c: Color, target: Color, pull: f32, clamp_deg: f32) -> Color {
    let pull = pull.clamp(0.0, 1.0);
    let clamp_deg = clamp_deg.max(0.0);
    if pull == 0.0 || clamp_deg == 0.0 {
        return c;
    }
    let mut p = c.to_oklch();
    let want = target.to_oklch().h;
    // -180..180: the signed shortest arc from `p.h` to `want`.
    let d = (want - p.h + 180.0).rem_euclid(360.0) - 180.0;
    let step = (d * pull).clamp(-clamp_deg, clamp_deg);
    p.h = (p.h + step).rem_euclid(360.0);
    Color::from_oklch(p)
}

/// `mix(a, b, t)`: premultiplied lerp in linear light, then un-premultiply.
/// `out.a <= 0 => rgb = 0` (§6).
pub fn mix(x: Color, y: Color, t: f32) -> Color {
    let a = x.a + (y.a - x.a) * t;
    if a <= 0.0 {
        return Color::TRANSPARENT;
    }
    let ch = |p: f32, q: f32| {
        let pm = p * x.a + (q * y.a - p * x.a) * t;
        pm / a
    };
    Color { r: ch(x.r, y.r), g: ch(x.g, y.g), b: ch(x.b, y.b), a }
}

/// The shared body of `shade` and `tint`: a perceptual mix toward a *target*
/// colour in OKLab. Distinct from `mix(c, black, t)`, which produces muddy
/// midpoints; this moves lightness evenly and drags chroma down smoothly.
pub fn oklab_toward(c: Color, target: Color, t: f32) -> Color {
    let (x, y) = (c.to_oklab(), target.to_oklab());
    let lab = Oklab {
        l: x.l + (y.l - x.l) * t,
        a: x.a + (y.a - x.a) * t,
        b: x.b + (y.b - x.b) * t,
        alpha: x.alpha + (y.alpha - x.alpha) * t,
    };
    Color::from_oklab(lab)
}

/// `ramp(c, n, i)`: step `i` of an `n`-step lightness ladder centred on
/// `c`'s L (§6). Image 3 in one expression.
///
/// The SPAN — how far the top rung stands from the bottom — is the
/// theme's `metric.ramp_span`, handed in by the caller. It used to be a
/// literal here, which meant an author could state the number of rungs
/// and which rung a series stood on, but not how far apart they were:
/// the spacing of every data ladder in the program lived in the binary.
pub fn ramp(c: Color, n: usize, i: usize, span: f32) -> Color {
    let mut p = c.to_oklch();
    if n > 1 {
        let i = i.min(n - 1) as f32;
        let f = i / (n - 1) as f32; // 0 .. 1
        p.l = (p.l - span * 0.5 + span * f).clamp(0.0, 1.0);
    }
    Color::from_oklch(p)
}

/// `ensure(fg, bg, ratio)`: walk `fg`'s L **away from** `bg` until the WCAG
/// ratio is met — 48 bounded steps, hue and chroma held, then clamp (§6).
/// This is what turns §4.4 from an aspiration into a mechanism.
pub fn ensure(fg: Color, bg: Color, ratio: f32) -> Color {
    if !(ratio > 1.0) || Color::wcag_contrast(fg, bg) >= ratio {
        return fg;
    }
    let mut p = fg.to_oklch();
    // Away from the background: lighter on a dark background, darker on a light
    // one. Hue is never touched, because hue is the theme's identity (§5.23).
    let up = fg.to_oklch().l >= bg.to_oklch().l;
    let step = 1.0 / 48.0;
    let mut best = fg;
    for _ in 0..48 {
        p.l = (p.l + if up { step } else { -step }).clamp(0.0, 1.0);
        let cand = Color::from_oklch(p);
        best = cand;
        if Color::wcag_contrast(cand, bg) >= ratio {
            return cand;
        }
        if p.l <= 0.0 || p.l >= 1.0 {
            break;
        }
    }
    best
}

// ----------------------------------------------------- the memoised DAG walk

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mark {
    White,
    Grey,
    Black,
}

/// The three-colour marking walk of §6.3, over a token table addressed by index.
///
/// The caller supplies the expressions; the evaluator supplies the memo, the
/// cycle detection and the depth cap. Kept generic over the table so `resolve.rs`
/// can drive it for a whole theme and a test can drive it for four tokens.
pub struct Evaluator {
    memo: Vec<Option<Value>>,
    mark: Vec<Mark>,
    stack: Vec<usize>,
    /// depth cap of 32 (§6.3); exceeding it is treated as a cycle (§4.2)
    pub max_depth: usize,
}

impl Evaluator {
    pub fn new(tokens: usize) -> Self {
        Evaluator {
            memo: vec![None; tokens],
            mark: vec![Mark::White; tokens],
            stack: Vec::with_capacity(34),
            max_depth: 32,
        }
    }

    pub fn cached(&self, id: usize) -> Option<&Value> {
        self.memo.get(id).and_then(|v| v.as_ref())
    }

    /// Drop one token's memo so it can be re-evaluated from `default`'s
    /// expression after a cycle (§4.2).
    pub fn invalidate(&mut self, id: usize) {
        if let Some(slot) = self.memo.get_mut(id) {
            *slot = None;
        }
        if let Some(m) = self.mark.get_mut(id) {
            *m = Mark::White;
        }
    }

    pub fn enter(&mut self, id: usize, name: &str) -> Result<(), EvalError> {
        if self.stack.len() >= self.max_depth {
            return Err(EvalError::TooDeep(self.path_to(id, name)));
        }
        match self.mark.get(id).copied().unwrap_or(Mark::White) {
            Mark::Grey => return Err(EvalError::Cycle(self.path_to(id, name))),
            _ => {}
        }
        self.mark[id] = Mark::Grey;
        self.stack.push(id);
        Ok(())
    }

    pub fn leave(&mut self, id: usize, value: Value) {
        self.stack.pop();
        self.mark[id] = Mark::Black;
        self.memo[id] = Some(value);
    }

    /// Unwind without recording a value — the token failed and will take a
    /// fallback. The mark goes back to White so a *different* path to the same
    /// token is not reported as a second cycle.
    pub fn abandon(&mut self, id: usize) {
        self.stack.pop();
        self.mark[id] = Mark::White;
    }

    /// The full cycle path §4.2 requires, from the first occurrence of the
    /// re-entered node: `accent.hover -> text.title -> accent.hover`.
    fn path_to(&self, id: usize, name: &str) -> Vec<String> {
        let start = self.stack.iter().position(|&x| x == id).unwrap_or(0);
        let mut path: Vec<String> = self.stack[start..].iter().map(|&i| format!("#{i}")).collect();
        path.push(name.to_string());
        path
    }

    pub fn depth(&self) -> usize {
        self.stack.len()
    }

    pub fn on_stack(&self) -> &[usize] {
        &self.stack
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// A four-token environment, which is all the fourteen functions need.
    struct Map(HashMap<String, Expr>, usize);

    impl Map {
        fn new(pairs: &[(&str, Expr)]) -> Map {
            Map(pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect(), 0)
        }
    }

    impl Env for Map {
        fn resolve(&mut self, name: &str, _i: Option<u32>) -> Result<Value, EvalError> {
            self.1 += 1;
            if self.1 > 64 {
                return Err(EvalError::TooDeep(vec![name.to_string()]));
            }
            let e = self
                .0
                .get(name)
                .cloned()
                .ok_or_else(|| EvalError::UnknownToken(name.to_string()))?;
            let v = eval(&e, self);
            self.1 -= 1;
            v
        }
    }

    fn hex(s: &str) -> Color {
        Color::from_hex(s).unwrap().to_linear()
    }

    fn empty() -> Map {
        Map::new(&[
            ("palette.black", Expr::Color(hex("#0A100E"))),
            ("palette.white", Expr::Color(hex("#EAF6F1"))),
        ])
    }

    #[test]
    fn alpha_sets_and_fade_multiplies() {
        let mut env = empty();
        let c = Expr::Color(hex("#3FE3AE").alpha(0.5));
        let a = eval(&Expr::Call(Func::Alpha, vec![c.clone(), Expr::Num(0.6)]), &mut env).unwrap();
        assert_eq!(a.as_color().unwrap().a, 0.6);
        let f = eval(&Expr::Call(Func::Fade, vec![c, Expr::Num(0.5)]), &mut env).unwrap();
        assert_eq!(f.as_color().unwrap().a, 0.25);
    }

    #[test]
    fn mix_is_linear_and_degenerate_alpha_zeroes_rgb() {
        let mut env = empty();
        // half-way between black and white in LINEAR light is ~#BCBCBC in sRGB,
        // not #808080. That difference is the whole reason mix() is linear.
        let m = mix(Color::BLACK, Color::WHITE, 0.5).to_srgb();
        assert!(m.r > 0.7 && m.r < 0.75, "linear midpoint encoded to {}", m.r);
        let _ = &mut env;
        let z = mix(Color::WHITE.alpha(0.0), Color::BLACK.alpha(0.0), 0.5);
        assert_eq!(z, Color::TRANSPARENT);
    }

    #[test]
    fn over_is_the_authoring_composite_and_returns_opaque() {
        let mut env = empty();
        let e = Expr::Call(
            Func::Over,
            vec![
                Expr::Color(hex("#15201B").alpha(0.82)),
                Expr::Color(hex("#0B1310")),
            ],
        );
        let out = eval(&e, &mut env).unwrap().as_color().unwrap();
        assert_eq!(out.a, 1.0);
        // §4.4: the authoring composite lands one 8-bit step from the
        // as-rendered #131E1A. Both numbers are correct for their own question,
        // and enforcement measures the other one.
        let hex = out.to_srgb().to_hex();
        assert!(hex == "#141F1A" || hex == "#141E19", "{hex}");
        assert_ne!(hex, Color::composite_as_rendered(
            Color::from_hex("#15201B").unwrap().alpha(0.82),
            Color::from_hex("#0B1310").unwrap()).to_hex());
    }

    #[test]
    fn shade_and_tint_move_in_oklab_toward_the_palette_anchors() {
        let mut env = empty();
        let accent = hex("#3FE3AE");
        let sh = eval(
            &Expr::Call(Func::Shade, vec![Expr::Color(accent), Expr::Num(0.5)]),
            &mut env,
        )
        .unwrap()
        .as_color()
        .unwrap();
        assert!(sh.to_oklch().l < accent.to_oklch().l);
        // toward palette.black, not toward a muddy sRGB midpoint
        assert!(sh.to_oklch().c < accent.to_oklch().c);

        // §6's worked value: tint(azure, 0.18) lands on image 6's highlight.
        let azure = hex("#29B6F6");
        let ti = eval(
            &Expr::Call(Func::Tint, vec![Expr::Color(azure), Expr::Num(0.18)]),
            &mut env,
        )
        .unwrap()
        .as_color()
        .unwrap();
        let want = hex("#4FC3F7");
        let de = Color::delta_e_ok(ti, want);
        assert!(de < 0.035, "tint(#29B6F6, 0.18) = {} (ΔE {de})", ti.to_srgb().to_hex());
        assert!(ti.to_oklch().l > azure.to_oklch().l);
    }

    #[test]
    fn lum_keeps_the_same_green_where_shade_would_not() {
        let mut env = empty();
        let accent = hex("#3FE3AE");
        let l = eval(
            &Expr::Call(Func::Lum, vec![Expr::Color(accent), Expr::Num(0.62)]),
            &mut env,
        )
        .unwrap()
        .as_color()
        .unwrap();
        let (a, b) = (accent.to_oklch(), l.to_oklch());
        assert!((b.l - a.l * 0.62).abs() < 2e-3, "L {} vs {}", b.l, a.l * 0.62);
        assert!((b.h - a.h).abs() < 1.0, "hue drifted {} -> {}", a.h, b.h);
        // "stays *the same green*, just dimmer": chroma is asked for unchanged
        // and only the MANDATORY gamut map (§6.2) may reduce it. That is
        // exactly `from_oklch` of the same request.
        let asked = Color::from_oklch(Oklch { l: a.l * 0.62, c: a.c, h: a.h, alpha: 1.0 });
        assert!(Color::delta_e_ok(l, asked) < 1e-4);
        // and it keeps far more chroma than shade() at the same lightness,
        // which is the distinction §6 draws between the two.
        let mut env2 = empty();
        let t = (a.l - a.l * 0.62) / (a.l - hex("#0A100E").to_oklch().l);
        let sh = eval(&Expr::Call(Func::Shade, vec![Expr::Color(accent), Expr::Num(t)]), &mut env2)
            .unwrap().as_color().unwrap();
        assert!(b.c > sh.to_oklch().c, "lum {} vs shade {}", b.c, sh.to_oklch().c);
    }

    #[test]
    fn lum_min_and_max_are_one_sided() {
        let mut env = empty();
        let c = Expr::Color(hex("#3FE3AE"));
        let l0 = hex("#3FE3AE").to_oklch().l;
        let up = eval(&Expr::Call(Func::LumMin, vec![c.clone(), Expr::Num(0.95)]), &mut env)
            .unwrap()
            .as_color()
            .unwrap();
        assert!(up.to_oklch().l > l0);
        let noop = eval(&Expr::Call(Func::LumMin, vec![c.clone(), Expr::Num(0.10)]), &mut env)
            .unwrap()
            .as_color()
            .unwrap();
        assert!((noop.to_oklch().l - l0).abs() < 1e-3);
        let down = eval(&Expr::Call(Func::LumMax, vec![c, Expr::Num(0.30)]), &mut env)
            .unwrap()
            .as_color()
            .unwrap();
        assert!(down.to_oklch().l < l0);
    }

    #[test]
    fn sat_zero_is_the_greyscale_hull_of_image_5() {
        let mut env = Map::new(&[("data.line", Expr::Color(hex("#35A7FF")))]);
        // render.hull = sat(@data.line, 0.0)
        let e = Expr::Call(
            Func::Sat,
            vec![Expr::Ref("data.line".into(), None), Expr::Num(0.0)],
        );
        let hull = eval(&e, &mut env).unwrap().as_color().unwrap();
        assert!(hull.to_oklch().c < 1e-3, "chroma {}", hull.to_oklch().c);
        // grey: all three channels equal
        assert!((hull.r - hull.g).abs() < 2e-3 && (hull.g - hull.b).abs() < 2e-3);
    }

    #[test]
    fn hue_rotates_and_wraps() {
        let mut env = empty();
        let c = hex("#3FE3AE");
        let e = Expr::Call(Func::Hue, vec![Expr::Color(c), Expr::Num(240.0)]);
        let out = eval(&e, &mut env).unwrap().as_color().unwrap();
        let want = (c.to_oklch().h + 240.0).rem_euclid(360.0);
        assert!((out.to_oklch().h - want).abs() < 1.0);
        // full turn is a no-op
        let full = eval(
            &Expr::Call(Func::Hue, vec![Expr::Color(c), Expr::Num(360.0)]),
            &mut env,
        )
        .unwrap()
        .as_color()
        .unwrap();
        assert!(Color::delta_e_ok(full, c) < 1e-3);
    }

    #[test]
    fn a_ramp_spans_the_lightness_the_theme_asks_for() {
        // Centred on a mid lightness, so the ladder's full span fits inside
        // 0..1 and the clamp is not what is being measured.
        let c = Color::from_oklch(Oklch { l: 0.5, c: 0.08, h: 165.0, alpha: 1.0 });
        for span in [0.62, 0.30] {
            let lo = ramp(c, 5, 0, span).to_oklch().l;
            let hi = ramp(c, 5, 4, span).to_oklch().l;
            assert!((hi - lo - span).abs() < 2e-3, "span {}", hi - lo);
            // the middle step is the input's own lightness
            assert!((ramp(c, 5, 2, span).to_oklch().l - c.to_oklch().l).abs() < 2e-3);
            // out-of-range i clamps rather than panics
            assert_eq!(ramp(c, 5, 99, span).to_oklch().l, hi);
            assert_eq!(ramp(c, 1, 0, span).to_oklch().l, c.to_oklch().l);
        }
        // No span at all is a ladder with no rungs apart: every step is the
        // colour it was centred on, which is the hole a missing key should
        // leave rather than a spacing this file invented.
        assert_eq!(ramp(c, 5, 0, 0.0).to_oklch().l, ramp(c, 5, 4, 0.0).to_oklch().l);
    }

    #[test]
    fn contrast_on_picks_the_greater_wcag_contrast() {
        let mut env = empty();
        let pick = |bg: &str, env: &mut Map| {
            eval(
                &Expr::Call(
                    Func::ContrastOn,
                    vec![
                        Expr::Color(hex(bg)),
                        Expr::Color(Color::BLACK),
                        Expr::Color(Color::WHITE),
                    ],
                ),
                env,
            )
            .unwrap()
            .as_color()
            .unwrap()
        };
        // §6: the azure #29B6F6 chip needs dark text. WCAG agrees.
        assert_eq!(pick("#29B6F6", &mut env), Color::BLACK);
        // and a near-black chip needs light text.
        assert_eq!(pick("#0A100E", &mut env), Color::WHITE);
    }

    #[test]
    fn ensure_lifts_until_the_floor_is_met_and_holds_hue() {
        let bg = hex("#0A100E");
        let fg = hex("#1A3A30"); // far below AAA on that background
        assert!(Color::wcag_contrast(fg, bg) < 7.0);
        let out = ensure(fg, bg, 7.0);
        assert!(Color::wcag_contrast(out, bg) >= 7.0, "{}", Color::wcag_contrast(out, bg));
        let dh = (out.to_oklch().h - fg.to_oklch().h).abs();
        assert!(dh < 2.0, "hue moved {dh} deg — hue is the theme's identity");
        // already-passing colours are returned untouched
        assert_eq!(ensure(hex("#EAF6F1"), bg, 7.0), hex("#EAF6F1"));
    }

    #[test]
    fn ensure_is_bounded_and_never_spins() {
        // An impossible floor must terminate at the clamp, not loop.
        let bg = hex("#7F7F7F");
        let out = ensure(hex("#808080"), bg, 21.0);
        assert!(out.is_finite());
    }

    #[test]
    fn wrong_arity_is_an_error_not_a_panic() {
        let mut env = empty();
        let e = Expr::Call(Func::Mix, vec![Expr::Color(Color::WHITE)]);
        match eval(&e, &mut env) {
            Err(EvalError::Bad(m)) => assert!(m.contains("mix() takes 3")),
            other => panic!("expected an arity error, got {other:?}"),
        }
    }

    #[test]
    fn a_colour_where_a_number_is_wanted_is_a_type_error() {
        let mut env = empty();
        let e = Expr::Call(
            Func::Alpha,
            vec![Expr::Color(Color::WHITE), Expr::Color(Color::BLACK)],
        );
        match eval(&e, &mut env) {
            Err(EvalError::Bad(m)) => assert!(m.contains("expected a number")),
            other => panic!("expected a type error, got {other:?}"),
        }
    }

    #[test]
    fn unknown_token_reference_reports_the_name() {
        let mut env = empty();
        let e = Expr::Ref("panel.boarder".into(), None);
        assert_eq!(
            eval(&e, &mut env),
            Err(EvalError::UnknownToken("panel.boarder".into()))
        );
    }

    #[test]
    fn ratio_multiplies_a_length_and_refuses_an_enum() {
        let mut env = Map::new(&[
            ("menu.row_h", Expr::Len(4.0, Unit::U)),
            ("shape.mode", Expr::Word("round".into())),
        ]);
        let ok = eval(
            &Expr::Ratio(0.35, Box::new(Expr::Ref("menu.row_h".into(), None))),
            &mut env,
        )
        .unwrap();
        assert_eq!(ok, Value::Len(1.4, Unit::U));
        let bad = eval(
            &Expr::Ratio(0.27, Box::new(Expr::Ref("shape.mode".into(), None))),
            &mut env,
        );
        assert!(matches!(bad, Err(EvalError::Bad(_))), "{bad:?}");
    }

    #[test]
    fn percent_reads_as_a_fraction_for_a_function_argument() {
        assert_eq!(Value::Len(55.0, Unit::Pct).as_num(), Some(0.55));
    }

    #[test]
    fn sentinels_bake_negative_and_are_scalars_not_enums() {
        assert_eq!(sentinel("same_as_parent"), Some(-3.0));
        assert_eq!(Value::Word("pill".into()).kind(), Kind::Scalar);
        assert_eq!(Value::Word("round".into()).kind(), Kind::Enum);
        // nothing else is a sentinel — `inherit` is the mood chain keyword only
        assert_eq!(sentinel("inherit"), None);
        assert_eq!(sentinel("element"), None);
    }

    #[test]
    fn three_colour_marking_reports_a_cycle_and_recovers() {
        let mut ev = Evaluator::new(4);
        ev.enter(0, "accent.hover").unwrap();
        ev.enter(1, "text.title").unwrap();
        match ev.enter(0, "accent.hover") {
            Err(EvalError::Cycle(path)) => {
                assert_eq!(path.len(), 3);
                assert_eq!(path.last().unwrap(), "accent.hover");
            }
            other => panic!("expected a cycle, got {other:?}"),
        }
        // the failed node is abandoned, not left Grey, so a second honest path
        // to it is not reported as a phantom cycle
        ev.abandon(1);
        ev.leave(0, Value::Num(1.0));
        assert_eq!(ev.cached(0), Some(&Value::Num(1.0)));
    }

    #[test]
    fn depth_cap_is_32_and_reports_as_a_cycle() {
        let mut ev = Evaluator::new(64);
        for i in 0..32 {
            ev.enter(i, "t").unwrap();
        }
        assert!(matches!(ev.enter(32, "t"), Err(EvalError::TooDeep(_))));
    }

    #[test]
    fn mentions_base_finds_the_state_template_keyword() {
        let e = Expr::Call(
            Func::Alpha,
            vec![Expr::Word("base".into()), Expr::Num(0.07)],
        );
        assert!(e.mentions_base());
        assert!(!Expr::Call(Func::Alpha, vec![Expr::Color(Color::WHITE), Expr::Num(0.07)])
            .mentions_base());
    }

    /// `toward()` LEANS and never travels: a canonical hue moves by the
    /// fraction the theme asks for, and stops dead at the theme's cap
    /// however far the target is. The numbers below are the arithmetic's,
    /// not the master's — a fraction of a signed arc, and a ceiling.
    #[test]
    fn a_hue_leans_by_the_fraction_and_stops_at_the_cap() {
        let h_of = |c: Color| c.to_oklch().h;
        let red = Color::from_oklch(Oklch { l: 0.68, c: 0.21, h: 27.0, alpha: 1.0 });
        let mint = Color::from_oklch(Oklch { l: 0.82, c: 0.15, h: 166.5, alpha: 1.0 });
        // Uncapped, a fifth of the way: 27 -> 166.5 is +139.5, a fifth is
        // +27.9, so the lean lands on 54.9.
        let free = toward(red, mint, 0.2, 360.0);
        assert!((h_of(free) - 54.9).abs() < 0.2, "{}", h_of(free));
        // Capped at seven degrees, it goes seven — the same direction, no
        // further. This is the whole reason the cap exists: red that walks
        // 27.9 deg is orange, and `git diff` stops reading.
        let held = toward(red, mint, 0.2, 7.0);
        assert!((h_of(held) - 34.0).abs() < 0.2, "{}", h_of(held));
        // A zero pull and a zero cap are both "never move", which the
        // master documents as a theme's way of opting out.
        assert!((h_of(toward(red, mint, 0.0, 7.0)) - 27.0).abs() < 0.2);
        assert!((h_of(toward(red, mint, 0.9, 0.0)) - 27.0).abs() < 0.2);
    }

    /// THE SHORT WAY ROUND, and only the hue.
    ///
    /// A colour ten degrees clockwise of its target must lean ten degrees
    /// clockwise and not three hundred and fifty the other way — the wrap is
    /// the one place this arithmetic can go silently wrong, and it goes
    /// wrong invisibly, as a colour that leans away from the theme.
    #[test]
    fn the_lean_takes_the_short_way_and_leaves_lightness_and_chroma_alone() {
        let at = |h: f32| Color::from_oklch(Oklch { l: 0.6, c: 0.12, h, alpha: 1.0 });
        // 350 -> 10 is +20 the short way, +20 * 0.5 = +10, landing on 0/360.
        let over_the_seam = toward(at(350.0), at(10.0), 0.5, 90.0);
        let h = over_the_seam.to_oklch().h;
        assert!(h < 0.2 || h > 359.8, "the lean took the long way round: {h}");
        // and the other direction across the same seam.
        let back = toward(at(10.0), at(350.0), 0.5, 90.0);
        assert!((back.to_oklch().h - 0.0).abs() < 0.2 || back.to_oklch().h > 359.8);
        // L and C are the author's, whatever the lean.
        let before = at(200.0).to_oklch();
        let after = toward(at(200.0), at(20.0), 1.0, 45.0).to_oklch();
        assert!((after.l - before.l).abs() < 0.002, "the lean moved the lightness");
        assert!((after.c - before.c).abs() < 0.002, "the lean moved the chroma");
    }

    #[test]
    fn the_closed_set_is_closed() {
        assert_eq!(Func::ALL.len(), 15);
        for f in Func::ALL {
            assert_eq!(Func::from_name(f.name()), Some(f));
        }
        // the names §6.1 cut, and the one §4.4 keeps internal
        for cut in ["darken", "lighten", "dim", "lift", "gamut", "on", "linear", "lerp",
                    "composite_as_rendered"] {
            assert_eq!(Func::from_name(cut), None, "{cut} must not be authorable");
        }
    }
}
