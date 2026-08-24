//! The shared corner resolver — ONE answer to "which cut does this word
//! name", for every `*_corner_style` / `*_corner_mode` sibling §5.4d
//! declares.
//!
//! Before this module existed the answer was copied four times, once per
//! reading path: `object/window.rs` compared memoised enum INDICES,
//! `object/tabs.rs` and `view/paint.rs` matched a `Surface`'s word, and
//! `object/focus_ring.rs` matched `ui::theme_word`'s. Four `match`
//! arms over the same three words, and every one of them ended in a
//! catch-all that spells Square — so a variant added to [`CornerStyle`]
//! and written into three of the four would not fail to compile. It
//! would draw a square where the theme asked for the new shape, in
//! whichever quarter of the interface was missed. That is the defect
//! `motion.rs` was written to end for easing, in the other vocabulary
//! this file owns.
//!
//! [`WORDS`] is the whole vocabulary, and every reading below walks it:
//! the two that take a theme's WORD, and [`of_code`], which takes the
//! NUMBER that same cut travels as across the plugin ABI. A cut added to
//! the table reaches all three without a second edit — and [`code`], the
//! one match here left exhaustive on purpose, is what stops a cut being
//! added without an ABI number at all: it fails to compile until it has
//! one.
//!
//! ONE boundary this table does not reach, named here rather than
//! implied: a shape record carries its four cuts as TWO BITS each
//! (`draw::DrawList::shape`), and the fragment shader that reads those
//! bits lives in the renderer. Two bits hold four cuts, so a fourth
//! still fits, but it arrives on screen the day `fs_shape` learns the
//! value — not the day this table grows. `sdf.rs`'s decoder mirrors that
//! shader deliberately, fallback and all, because it is the shader's
//! specification and stops being one the moment the two part.
//!
//! TWO readings of a theme, because a theme is read two ways and neither
//! is wrong:
//!
//! * [`cut`] takes the WORD. It is what a `Surface` can answer across
//!   the plugin ABI, which ships words and not indices, and it is the
//!   only correct reading for a token whose vocabulary the master does
//!   not declare: `shape.badge.corners[0]` has no `enum:` list, so its
//!   word table grows out of the values a cascade happens to load and an
//!   index memoised before a variant is read would freeze at the wrong
//!   answer (`tests/enum_vocabulary_declared.rs` states the rule).
//! * [`Cuts`] takes the INDICES, memoised once per token. Enum words
//!   intern in load order, so an index only names a word against the
//!   vocabulary it was interned in — which is exactly why this is
//!   reserved for the `*_corner_style` / `*_corner_mode` siblings, whose
//!   lines DO spell `enum: square | round | chamfer` and whose numbering
//!   is therefore the master's and stable for the process. It costs no
//!   allocation per frame, which is why the objects drawing every frame
//!   against `Ctx` read this way.
//!
//! A word the vocabulary does not name degrades to Square, SILENTLY and
//! on purpose: `chevron` and `hexagon` are legal in the shape presets'
//! own vocabulary and in no surface's, and a warning per frame per badge
//! is not a diagnostic. Square is the shape a ring generator can draw
//! honestly, and one a theme can already ask for.

use crate::draw::CornerStyle;
use crate::theme::{self, ResolvedTheme, TokenId};
use std::sync::OnceLock;

/// Every cut this pipeline can draw, with the word a theme names it by —
/// §5.4d's `enum: square | round | chamfer`, in the master's declared
/// order. THE one place a new cut is named.
pub const WORDS: [(&str, CornerStyle); 3] = [
    ("square", CornerStyle::Square),
    ("round", CornerStyle::Round),
    ("chamfer", CornerStyle::Chamfer),
];

/// The cut a WORD names. Anything outside [`WORDS`] is Square — see the
/// header for why that is silent.
pub fn cut(word: &str) -> CornerStyle {
    cut_or(word, CornerStyle::Square)
}

/// The same table, with the caller naming what "anything else" means.
///
/// The two readings genuinely differ, which is why the fallback is not
/// settled here. On a preset's own `corners` an unknown word is Square —
/// the raw look of an unstyled rect. On a PER-CORNER key
/// (`shape.<preset>.corners_tl` and its three kin) it is the corner that
/// ARRIVED, because a word there may name a whole silhouette (`chevron`,
/// `hexagon`) and one corner of a hexagon is not a shape. Collapsing the
/// two would answer a question the key exists to ask.
pub fn cut_or(word: &str, fallback: CornerStyle) -> CornerStyle {
    WORDS
        .iter()
        .find(|(w, _)| *w == word)
        .map(|(_, style)| *style)
        .unwrap_or(fallback)
}

/// The number this cut travels as across the plugin ABI —
/// [`crate::runtime::CORNER_SQUARE`] and its two kin.
///
/// A number rather than a word because the theme's enum indices intern
/// in load order and mean nothing across a library edge, and a word
/// costs an allocation per corner per frame.
///
/// The match is EXHAUSTIVE and stays that way: this is the one place a
/// new cut has to be given a number, and until it is given one the crate
/// does not build. Everything else about a new cut is a table entry;
/// this is the part that has to be decided.
pub fn code(style: CornerStyle) -> u32 {
    match style {
        CornerStyle::Square => crate::runtime::CORNER_SQUARE,
        CornerStyle::Round => crate::runtime::CORNER_ROUND,
        CornerStyle::Chamfer => crate::runtime::CORNER_CHAMFER,
    }
}

/// [`cut`] and [`code`] in one step — a WORD straight to the ABI number
/// a plugin's own `*_corner_style`/`*_corner_mode` reader wants, for the
/// caller on the SENDING side of the boundary rather than the receiving
/// one [`of_code`] serves.
///
/// This is the match four plugin `.so` files each wrote for themselves
/// (`"round" => CORNER_ROUND, "chamfer" => CORNER_CHAMFER, _ =>
/// CORNER_SQUARE`) before this existed — the same defect the module
/// header describes for the object layer, one boundary further out.
/// Unknown word: Square, silently, for the header's own reason.
pub fn code_of(word: &str) -> u32 {
    code(cut(word))
}

/// The cut an ABI number names — [`code`]'s inverse, and the reading a
/// plugin's boundary arrives through.
///
/// Walked over [`WORDS`] rather than matched, so the day a cut is added
/// to the table and given a number by [`code`] it is understood here
/// too. A number outside the set is Square for the header's reason: a
/// plugin can invent one, and Square is the shape a ring generator draws
/// honestly.
pub fn of_code(n: u32) -> CornerStyle {
    WORDS
        .iter()
        .map(|(_, style)| *style)
        .find(|style| code(*style) == n)
        .unwrap_or(CornerStyle::Square)
}

/// Where each of [`WORDS`] sits in ONE token's vocabulary — the index
/// table a caller memoises so the hot path never allocates a word.
///
/// A slot is `None` when the token's declared list omits that word,
/// which several master lines do (`panel.corner_mode` spells `enum:
/// round | chamfer`): a theme writing the missing word gets Square,
/// the same answer it would get for a word nobody declared.
/// `Debug` alone, and it is derived because a failing assertion prints
/// the table. A table settled once and read behind a `OnceLock` is never
/// compared or defaulted; the derives for those were written against a
/// use that does not exist, and a derive with no reader is the same claim
/// as a token with no reader.
///
/// It IS copied, and by exactly one reader: `elev::Level` keeps the table
/// beside the token ids it resolved once, so a rung answers the corner
/// word without walking the vocabulary per frame. That reader arrived with
/// the elevation ladder, after the derives were pruned here.
#[derive(Debug, Clone, Copy)]
pub struct Cuts([Option<u16>; WORDS.len()]);

impl Cuts {
    /// The table for `mode`. Settled once — the vocabulary is the
    /// master's, and the numbering it hands out is stable for the life
    /// of the process because the key's own line declares the list.
    pub fn of(mode: TokenId) -> Cuts {
        let mut out = [None; WORDS.len()];
        for (i, (word, _)) in WORDS.iter().enumerate() {
            out[i] = theme::enum_index(mode, word);
        }
        Cuts(out)
    }

    /// The cut `mode` currently resolves to, with the table in hand.
    ///
    /// Taken apart from [`style`] because a caller that reads a WHOLE
    /// dictionary at once — [`crate::object::elev::Level`], which
    /// memoises every key of one `[elev.*]` level in a single struct —
    /// has nowhere to hang a `static` per token and no reason to.
    pub fn read(&self, t: &ResolvedTheme, mode: TokenId) -> CornerStyle {
        let cur = Some(t.enum_of(mode));
        for (i, (_, style)) in WORDS.iter().enumerate() {
            if self.0[i] == cur {
                return *style;
            }
        }
        CornerStyle::Square
    }
}

/// The cut a corner-mode enum token resolves to, memoising its
/// vocabulary in the caller's own `static`.
///
/// This is the entry point for an object drawing against `Ctx`: the
/// token id lives in one `OnceLock`, its index table in another, and
/// neither is asked for twice.
pub fn style(t: &ResolvedTheme, mode: TokenId, idx: &'static OnceLock<Cuts>) -> CornerStyle {
    idx.get_or_init(|| Cuts::of(mode)).read(t, mode)
}

/// How finely a cut of `size` is tessellated: the theme's
/// `corner.segments` is the ceiling and [`crate::draw::ring_segments`]'
/// quarter-pixel chord-error rule (r1 §3.4) sits under it.
///
/// IT LIVES BESIDE THE CUT BECAUSE IT IS PART OF THE SAME ANSWER. A
/// caller that has resolved a corner still cannot draw it without a
/// segment count, and until 2026-08-18 the only statement of this rule
/// was `object::window::corner_segments`, which is `pub(crate)` — so a
/// drawing OUTSIDE this crate could reach the vocabulary and not the
/// tessellation, and had to either spell the 0.25 tolerance itself or
/// give up and draw a plain rectangle. `nacelle-desktop`'s cycler did
/// the second for as long as it existed. A tolerance restated in another
/// repository is the four-`match` defect this module was written to end,
/// in the other half of the same sentence.
pub fn segments(t: &ResolvedTheme, cell: &'static OnceLock<TokenId>, size: f32) -> u8 {
    let ceiling = *cell.get_or_init(|| theme::id("corner.segments").unwrap_or(TokenId::MISSING));
    crate::draw::ring_segments(size, 0.25, t.px(ceiling) as u8)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The catch-all is the whole point: `chevron` is a word the shape
    /// presets carry and no ring generator draws, and an empty string is
    /// what a missing token answers.
    #[test]
    fn every_declared_word_names_its_cut_and_nothing_else_does() {
        assert_eq!(cut("square"), CornerStyle::Square);
        assert_eq!(cut("round"), CornerStyle::Round);
        assert_eq!(cut("chamfer"), CornerStyle::Chamfer);
        assert_eq!(cut("chevron"), CornerStyle::Square);
        assert_eq!(cut("hexagon"), CornerStyle::Square);
        assert_eq!(cut(""), CornerStyle::Square);
        assert_eq!(cut("Round"), CornerStyle::Square, "words are exact");
    }

    /// The word reading and the NUMBER reading answer the same cut, and
    /// the number reading is walked and not matched.
    ///
    /// `code` is exhaustive so that a cut cannot be added without an ABI
    /// number; `of_code` walks [`WORDS`] so that once it has one, the
    /// plugin door understands it with no further edit. Asserting the
    /// round trip over the whole table is what makes the pair a pair —
    /// two `match`es facing each other would pass every case written
    /// here and still drift on the case nobody wrote.
    #[test]
    fn the_word_and_the_number_name_the_same_cut() {
        for (word, style) in WORDS {
            assert_eq!(of_code(code(style)), style, "{word} did not survive the crossing");
            assert_eq!(cut(word), style);
        }
        // The numbers themselves are the ABI's, not this table's order:
        // a plugin built against §6 sends these three and nothing else.
        assert_eq!(code(CornerStyle::Square), crate::runtime::CORNER_SQUARE);
        assert_eq!(code(CornerStyle::Round), crate::runtime::CORNER_ROUND);
        assert_eq!(code(CornerStyle::Chamfer), crate::runtime::CORNER_CHAMFER);
        // A number the boundary does not name — the header's rule, from
        // the far side of the ABI rather than from a theme.
        assert_eq!(of_code(9), CornerStyle::Square);
        assert_eq!(of_code(u32::MAX), CornerStyle::Square);
    }

    /// [`code_of`] is [`cut`] then [`code`], so the four plugin `.so`
    /// files this replaced (`"round" => CORNER_ROUND, "chamfer" =>
    /// CORNER_CHAMFER, _ => CORNER_SQUARE`) get exactly the number their
    /// own match gave, for every declared word and for the unknown one.
    #[test]
    fn code_of_is_cut_then_code() {
        for (word, _) in WORDS {
            assert_eq!(code_of(word), code(cut(word)), "{word}");
        }
        assert_eq!(code_of("round"), crate::runtime::CORNER_ROUND);
        assert_eq!(code_of("chamfer"), crate::runtime::CORNER_CHAMFER);
        assert_eq!(code_of("square"), crate::runtime::CORNER_SQUARE);
        assert_eq!(code_of("hexagon"), crate::runtime::CORNER_SQUARE, "unnamed is square");
    }

    /// The index reading and the word reading answer the same cut for
    /// the same token. They are one vocabulary or they are the four
    /// `match`es this module replaced, drifting apart again.
    ///
    /// `scrollbar.corner_style` is the witness because its master line
    /// spells the whole list (`enum: square | round | chamfer`), which
    /// is the condition under which an index may be memoised at all.
    #[test]
    fn the_two_readings_answer_the_same_cut_for_one_token() {
        let id = theme::id("scrollbar.corner_style")
            .expect("the master declares scrollbar.corner_style");
        let table = Cuts::of(id);
        assert!(
            table.0.iter().all(|slot| slot.is_some()),
            "a memoisable token must declare every word of the set: {table:?}"
        );
        let word = theme::enum_word_of(id).expect("an enum token names a word");
        assert_eq!(table.read(theme::resolved(), id), cut(&word));
    }

    /// Every key the master states, section-qualified, in the order the
    /// file writes them.
    ///
    /// The master is read as a DOCUMENT and not through the engine, for
    /// [`theme::master_source`]'s own reason: the baker fills every
    /// declared token whether the file wrote it or not, so "which keys
    /// does this file state, and next to what" is a fact about the FILE
    /// that no resolved theme can answer.
    fn master_keys() -> Vec<String> {
        let mut section = String::new();
        let mut out = Vec::new();
        for line in theme::master_source().lines() {
            let line = line.trim();
            if line.starts_with('#') || line.is_empty() {
                continue;
            }
            // A section header stands alone on its line; a value that
            // happens to be a list (`button.order = [...]`) has its key
            // in front of the bracket and is caught below.
            if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
                section = name.to_string();
                continue;
            }
            let Some((key, _)) = line.split_once('=') else { continue };
            let key = key.trim();
            let named = !key.is_empty()
                && key.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '_');
            if named && !section.is_empty() {
                out.push(format!("{section}.{key}"));
            }
        }
        out
    }

    /// The `[corner]` header — the paragraph other repositories cite by
    /// name, from the section rule down to the section itself.
    fn corner_header() -> String {
        let src = theme::master_source();
        let start = src.find("# 5.4d corner").expect("the master heads [corner] with its rule");
        let end = start + src[start..].find("\n[corner]").expect("that header ends at [corner]");
        src[start..end].to_string()
    }

    /// The radii whose cut is stated somewhere other than a
    /// `_style` / `_mode` sibling, and the key that states it.
    ///
    /// Both are in the header, in the paragraph that begins "TWO keys
    /// below still look like a bare radius and are not".
    const CUT_STATED_ELSEWHERE: [(&str, &str); 2] =
        [("badge.corner", "shape.badge.corners"), ("tile.corner", "tile.shape")];

    /// A key that reads like a radius and is not one, so no cut belongs
    /// beside it: `keyboard.sub_corner` names WHICH corner a sub-legend
    /// sits in.
    const NOT_A_RADIUS: [&str; 1] = ["keyboard.sub_corner"];

    /// The siblings that exist and are read by NOBODY — the radius each
    /// belongs to is drawn outside this crate, in the addons or the
    /// desktop, and those drawings still spell the cut themselves.
    ///
    /// This is the debt `[corner]`'s header keeps open by name, and the
    /// reason the old "a bare radius is round" rule was narrowed rather
    /// than struck: the code citing it did not change when the sentence
    /// did. A name leaves this list when its drawing reads its sibling.
    const SIBLING_WITHOUT_A_READER: [&str; 3] =
        ["filetile.corner_style", "keyboard.key_corner_style", "dialog.corner_mode"];

    /// Every radius in the master states the shape of its own cut, and
    /// the header's list of the exceptions is the file's own.
    ///
    /// This is `[corner]`'s central paragraph read back as an assertion.
    /// It has to be machine-checked because it is not only documentation:
    /// four places in `nacelle-addons` cite that paragraph as the reason
    /// they are allowed to name a cut in Rust, so a claim made there is a
    /// licence granted elsewhere. Seventeen radii stood without a sibling
    /// when this was written; a prose header cannot notice the eighteenth.
    ///
    /// The `elev.*` rungs are the one section where the naming inverts:
    /// there `corner` IS the cut and `radius` is the length, which is why
    /// `[button] corner_style = @elev.panel.corner` reads as it does. The
    /// pair is checked in that order rather than skipped.
    #[test]
    fn every_radius_in_the_master_states_the_shape_of_its_own_cut() {
        let keys = master_keys();
        let header = corner_header();
        let has = |name: &str| keys.iter().any(|k| k == name);

        let mut radii = 0;
        for key in &keys {
            if !(key.ends_with(".corner") || key.ends_with("_corner")) {
                continue;
            }
            if let Some(rung) = key.strip_suffix(".corner").filter(|k| k.starts_with("elev.")) {
                let length = format!("{rung}.radius");
                assert!(has(&length), "{key} is a cut with no length beside it ({length})");
                continue;
            }
            if NOT_A_RADIUS.contains(&key.as_str()) {
                assert!(header.contains(key.as_str()), "{key} is excused by nothing in the header");
                continue;
            }
            radii += 1;
            if let Some((_, stated_by)) =
                CUT_STATED_ELSEWHERE.iter().find(|(radius, _)| radius == key)
            {
                assert!(has(stated_by), "{key} points its cut at {stated_by}, which is not a key");
                assert!(header.contains(key.as_str()), "{key} is excused by nothing in the header");
                assert!(header.contains(stated_by), "the header does not say where {key} is cut");
                continue;
            }
            let siblings = [format!("{key}_style"), format!("{key}_mode")];
            assert!(
                siblings.iter().any(|s| has(s)),
                "{key} states a length and no shape: neither {} nor {} is a key, and the rule \
                 that used to cover it is narrowed to the radii named in [corner]'s header",
                siblings[0],
                siblings[1]
            );
        }

        // Fail closed: a parser that matched nothing would pass every
        // assertion above, and zero checked is not zero broken.
        // Twenty-nine radii stood here on 2026-08-17, beside nine
        // `elev.*` cuts and one key that only reads like a radius.
        assert!(radii >= 29, "the master was read as {radii} radii, so it was not read");

        for sibling in SIBLING_WITHOUT_A_READER {
            assert!(has(sibling), "{sibling} is the debt list's, and the master does not state it");
            assert!(
                header.contains(sibling),
                "{sibling} has no reader and the header does not say so — a debt is written \
                 where its creditor looks"
            );
        }
    }
}
