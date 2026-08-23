//! Font loading and the glyph atlas (single-channel, R8).
//!
//! eDEX-UI uses "United Sans" (UI) and "Fira Mono" (terminal). The .woff2
//! files from the eDEX repository can be converted to .ttf and installed —
//! `make install` puts a checkout's `fonts/` under
//! `$(PREFIX)/share/fonts/nacelle-desktop`, which for either of the
//! prefixes it defaults to is a directory the walk already reaches — and
//! `NACELLE_FONT_DIR` names one more absolute directory to look in first.
//! Otherwise we look for similar system fonts. See [`font_dirs`] for why
//! no entry is ever relative.

use fontdue::{Font, FontSettings};
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub const ATLAS_W: usize = 2048;
pub const ATLAS_H: usize = 2048;
/// The mask band: the atlas's last rows, reserved for procedural R8 masks —
/// the soft-disk sprite glow and shadows are built from. The shelf packer
/// never allocates from it and reset_atlas() never clears it (r1 §4, M0).
pub const MASK_BAND_H: usize = 128;
/// First row of the band; glyphs may only ever pack above this line.
pub const MASK_BAND_Y: usize = ATLAS_H - MASK_BAND_H;
/// The soft disk: a 64x64 radial gaussian falloff at the band's origin.
/// Drawn as a nine-slice it is a rounded soft rectangle at any size — one
/// sprite serves every glow, shadow and soft box in the program.
pub const MASK_SOFT: (usize, usize, usize, usize) = (0, MASK_BAND_Y, 64, 64);

// ---------------------------------------------------------------- faces
//
// The master's eight `[face.*]` blocks, at the ids it numbers them with.
// The first two keep the engine ids they have always had, so no plugin
// and no `CellC.font` changes meaning; the other six were declared with a
// family, a weight and a fallback chain and had nowhere to land.
//
// Two slots was why `type.value.face = ui_medium`, `type.title.panel.face
// = ui_medium` and `type.display.clock.face = display` all came out as
// the one interface Regular: the reader mapped every word that did not
// begin `mono` onto slot 0, so the master's five distinct declarations
// arrived at the atlas as two. WEIGHT in particular had no way through at
// all — 400, 500, 600 and 700 are four requests and there were two boxes.

pub const FONT_UI: u8 = 0;
pub const FONT_MONO: u8 = 1;
pub const FACE_UI_MEDIUM: u8 = 2;
pub const FACE_UI_BOLD: u8 = 3;
pub const FACE_DISPLAY: u8 = 4;
pub const FACE_MONO_BOLD: u8 = 5;
pub const FACE_ICON: u8 = 6;
pub const FACE_RESERVED: u8 = 7;
/// How many fonts there are. A font id arriving from outside this crate
/// — from a plugin, say — is an index into a fixed array, so it has to
/// be checked against this rather than trusted.
pub const FONT_COUNT: u8 = 8;

/// The master's face ids, in slot order. The ORDER is the master's own —
/// it spells the numbering out above `[face.ui]` — and this array is what
/// makes it one list instead of a rule written in three files.
pub const FACE_IDS: [&str; FONT_COUNT as usize] = [
    "ui",
    "mono",
    "ui_medium",
    "ui_bold",
    "display",
    "mono_bold",
    "icon",
    "reserved",
];

/// The slot a `type.<role>.face` word names.
///
/// Compared as a WORD against the master's own list, and never read as an
/// index: an index would turn `display` into monospace on the day a theme
/// reordered its face blocks. A word outside the list is a defect in the
/// theme, so it is said once and lands where an undesigned run has always
/// landed — the monospace slot for a word that begins `mono`, since that
/// prefix is the one thing a reader can honestly infer, and the interface
/// slot otherwise.
///
/// The ONE answer to "which family is this role set in": [`crate::ui`]
/// asks it for the objects that draw against `Ctx`, [`crate::view::paint`]
/// for every view and for the whole plugin side. Two answers is how one
/// role came to be monospace in a widget's own drawing and the interface
/// face in the toolkit's.
pub fn face_slot(word: &str) -> u8 {
    if let Some(i) = FACE_IDS.iter().position(|f| *f == word) {
        return i as u8;
    }
    if word.is_empty() {
        // A role whose master states no `face` at all. Said once, like
        // every other missing half of a role, and drawn in the slot an
        // undesigned run has always been drawn in.
        crate::ui::warn_once(
            "face:<none>",
            "a type role states no `face` — it is set in the interface slot",
        );
        return FONT_UI;
    }
    crate::ui::warn_once(
        &format!("face:{word}"),
        &format!(
            "\"{word}\" is not one of the master's eight faces \
             ({}) — the nearest slot is used",
            FACE_IDS.join(", ")
        ),
    );
    if word.starts_with("mono") {
        FONT_MONO
    } else {
        FONT_UI
    }
}

#[derive(Clone, Copy)]
pub struct Glyph {
    pub u0: f32,
    pub v0: f32,
    pub u1: f32,
    pub v1: f32,
    pub w: f32,
    pub h: f32,
    /// Offset of the bitmap's left edge relative to the pen.
    pub xmin: f32,
    /// Offset of the bitmap's bottom edge relative to the baseline (Y axis up).
    pub ymin: f32,
    pub advance: f32,
}

// ------------------------------------------------------------ tabular figures
//
// `fontdue` exposes no OpenType features, so there is no `tnum` to ask the
// face for. §5.17 therefore puts tabular figures in the TOOLKIT: the box is
// the widest advance among the characters of `num.tabular_set` at one
// (face, px), every member of the set is stepped by that box and centred
// inside it, and every other character keeps the advance the face gave it.
//
// Two consequences, both the reason the token exists: a value stops
// changing width when its content changes — `21:57:30` does not shiver on
// the tick, an IP address does not drag its column about — and the width of
// an all-figure string is `len x advance`, which [`Figures::advance_of`]
// answers without touching the atlas at all.

/// The punctuation `num.tabular_punct` pins into the figure box.
///
/// These seven characters are the token's DEFINITION, not a look this file
/// chose: the master spells them out in the comment beside the token, as
/// `. , : + - space %`, and offers the bool as the switch. A theme that
/// wants other characters in the box widens `num.tabular_set`, which is the
/// token carrying the set — this constant can never grow past the bool.
pub const TABULAR_PUNCT: [char; 7] = ['.', ',', ':', '+', '-', ' ', '%'];

/// The one member of [`TABULAR_PUNCT`] that is also a WORD SEPARATOR.
///
/// Every other mark in the list attaches to the number it touches — the
/// dots of an address, the colons of a clock, a leading sign, a trailing
/// `%` — so being next to a figure is enough to say it belongs to one.
/// The space has a second job that has nothing to do with numbers, and
/// pinning it into a figure box while it is doing that job stretches
/// PROSE: with `type.value.tabular` on, `AMD RYZEN 7` grew by half a
/// figure per gap, because every value in a `rows` line is set in the one
/// `value` role whether it holds a reading or the name of a machine.
///
/// So the space is boxed only where it stands INSIDE a number — a figure
/// on both sides, which is the grouping space of `1 234 567` — and keeps
/// the face's own advance between two words. [`Figures::advance_of`] is
/// where the rule is spent.
const TABULAR_WORD_SEP: char = ' ';

/// One role's figure box: how wide it is at this (face, px), and which
/// characters are stepped by it.
///
/// Resolved once per draw and carried into the row loop wherever a caller
/// CAN carry it — every object drawing against `Ctx` does, and that is
/// where the row loops are. The [`crate::view::Surface`] path cannot: the
/// trait carries the role's `tabular` BOOL and nothing else, because a
/// `Figures` is a host-side value with no way across the plugin ABI, so
/// `text_tab` and `measure_tab` re-resolve it per call. That costs one FNV
/// over ten bytes and two hash hits per cell per frame, against a box
/// that is already cached per (face, px); it is stated here rather than
/// glossed, because the cheap version of this comment used to claim the
/// row loop never repeated the work and one of the two paths did.
///
/// [`Figures::NONE`] is a run that is not tabular, and every accessor
/// below answers "the face's own advance" for it — a role without the
/// token draws exactly as it drew before.
#[derive(Clone, Copy, Debug)]
pub struct Figures {
    /// The shared advance, in px. Zero means the run is not tabular; it is
    /// the discriminant rather than an `Option` so that the per-glyph test
    /// on the draw path is one compare.
    advance: f32,
    /// Membership for U+0000..U+007F as a bitmap. The test runs once per
    /// glyph on a draw path; the alternative is re-scanning the theme's
    /// string for every character of every number on screen.
    ascii: u128,
    /// Members outside ASCII — a theme may well put U+2212 MINUS SIGN or a
    /// thin space in the set. Inline rather than boxed because a `Figures`
    /// is copied into every text call and the draw path allocates nothing.
    extra: [char; Figures::EXTRA],
    n_extra: u8,
    /// `num.tabular_punct`: whether [`TABULAR_PUNCT`] may join the box.
    ///
    /// A flag rather than seven more bits in `ascii`, because the marks
    /// join CONDITIONALLY — only where they are part of a number — and
    /// the two bitmaps have to stay tellable apart to decide that. The
    /// set proper is what "part of a number" is measured against.
    punct: bool,
}

impl Figures {
    /// How many non-ASCII members a set may carry. Past this the extras
    /// are dropped with a warning rather than silently ignored: a set the
    /// toolkit cannot hold whole would align some of a column and not the
    /// rest, which reads as a rendering bug rather than as a theme defect.
    pub const EXTRA: usize = 8;

    /// A run that is not tabular. Every character keeps the face's advance,
    /// which is the behaviour of every text path that predates the token.
    pub const NONE: Figures = Figures {
        advance: 0.0,
        ascii: 0,
        extra: ['\0'; Self::EXTRA],
        n_extra: 0,
        punct: false,
    };

    /// Whether this run has a figure box at all.
    pub fn is_on(&self) -> bool {
        self.advance > 0.0
    }

    /// The box width, or 0.0 when the run is not tabular. For a caller
    /// sizing a numeric column ahead of the text: `digits x advance` is the
    /// width of any string of that many figures, whatever they turn out to
    /// be — the property that makes a right-aligned numeric column free.
    pub fn advance(&self) -> f32 {
        self.advance
    }

    /// Whether `ch` is a member of the SET PROPER — `num.tabular_set`,
    /// the characters the box exists for. The punctuation of
    /// [`TABULAR_PUNCT`] is deliberately not counted here: "is this
    /// character part of a number" is the question the marks are judged
    /// by, and a question cannot be its own answer.
    fn in_set(&self, ch: char) -> bool {
        if (ch as u32) < 128 {
            (self.ascii >> (ch as u32)) & 1 == 1
        } else {
            self.extra[..self.n_extra as usize].contains(&ch)
        }
    }

    /// The advance `ch` is stepped by where it stands between `prev` and
    /// `next`, or `None` when the face's own advance stands.
    ///
    /// Answered WITHOUT touching the atlas, which is the whole performance
    /// argument of §5.17: measuring `192.168.000.101` costs fifteen bitmap
    /// tests and no rasterisation.
    ///
    /// The neighbours are what keeps the box on NUMBERS. A member of the
    /// set proper is boxed wherever it stands — that is what the set is.
    /// A `tabular_punct` mark is boxed where it is part of a number, and a
    /// mark is part of a number when a figure stands beside it: the dots
    /// of `192.168.1.1`, the colons of `21:57:30`, the sign of `-40`, the
    /// `%` of `74%`. The word space is the exception, and it is the reason
    /// the rule reads neighbours at all — see [`TABULAR_WORD_SEP`].
    pub fn advance_of(&self, prev: Option<char>, ch: char, next: Option<char>) -> Option<f32> {
        if !self.is_on() {
            return None;
        }
        if self.in_set(ch) {
            return Some(self.advance);
        }
        if !self.punct || !TABULAR_PUNCT.contains(&ch) {
            return None;
        }
        let left = prev.is_some_and(|c| self.in_set(c));
        let right = next.is_some_and(|c| self.in_set(c));
        let part_of_a_number = if ch == TABULAR_WORD_SEP {
            left && right
        } else {
            left || right
        };
        part_of_a_number.then_some(self.advance)
    }

    /// Where a glyph sits inside a box of `box_advance`: centred, so a
    /// narrow '1' keeps the optical rhythm of a wide '8' instead of
    /// hugging the box's left edge.
    ///
    /// Takes the box the caller already resolved rather than looking the
    /// character up a second time: [`Figures::advance_of`] now reads
    /// neighbours, so a second lookup would be a second walk of the same
    /// question — and the only way for the offset to disagree with the
    /// step it belongs to.
    pub fn centre_in(box_advance: f32, glyph_advance: f32) -> f32 {
        (box_advance - glyph_advance) / 2.0
    }

    fn add(&mut self, ch: char) -> bool {
        if (ch as u32) < 128 {
            self.ascii |= 1u128 << (ch as u32);
            return true;
        }
        if self.extra[..self.n_extra as usize].contains(&ch) {
            return true;
        }
        if (self.n_extra as usize) < Self::EXTRA {
            self.extra[self.n_extra as usize] = ch;
            self.n_extra += 1;
            return true;
        }
        false
    }
}

/// A run's characters with each one's neighbours, so a caller can ask
/// [`Figures::advance_of`] without collecting the string first.
///
/// The draw path allocates nothing, which rules out `Vec<char>`; the
/// three-character window is what the rule needs and all it needs.
pub fn with_neighbours(text: &str) -> impl Iterator<Item = (Option<char>, char, Option<char>)> + '_ {
    let mut prev: Option<char> = None;
    let mut it = text.chars().peekable();
    std::iter::from_fn(move || {
        let ch = it.next()?;
        let next = it.peek().copied();
        let out = (prev, ch, next);
        prev = Some(ch);
        Some(out)
    })
}

/// FNV-1a over the tabular set, so the cache key does not own a `String`.
/// There is one set per theme, so this hash is asked a handful of distinct
/// questions in the life of the process.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

pub struct FontSystem {
    fonts: [Font; FONT_COUNT as usize],
    /// Per slot, the x-offset of the second draw pass that FAKES weight,
    /// in em — `face.<id>.synthetic_bold`, and zero on every slot that got
    /// the weight it asked for.
    ///
    /// Kept beside the faces because it is a property of what the load
    /// SETTLED ON, not of the theme alone: the token says how wide the
    /// fake would be, and only the loader knows whether a >=600 request
    /// really did come back with a Regular file. It never touches an
    /// advance, so a bolded run measures exactly like the run it bolds and
    /// a tabular column cannot be widened by weight.
    synthetic: [f32; FONT_COUNT as usize],
    pub atlas: Vec<u8>,
    /// The rows touched since the last take_dirty_rows(): (lo, hi exclusive).
    /// Uploading only these is what keeps a glyph-churn frame at microseconds
    /// instead of re-copying four megabytes (r1's mandatory rider on M0).
    dirty_rows: Option<(usize, usize)>,
    pub atlas_dirty: bool,
    cache: HashMap<(u8, u32, char), Option<Glyph>>,
    /// The figure box per (face, px, set, punct). §5.17 says "ten glyph
    /// lookups once, cached beside the glyph cache" — this is that cache,
    /// and it lives here rather than beside the role because the box is a
    /// property of the FACE at a size, which is what this type owns.
    fig_cache: HashMap<(u8, u32, u64, bool), Figures>,
    // simple shelf packer
    cur_x: usize,
    cur_y: usize,
    row_h: usize,
    /// The atlas filled up mid-frame; reset it at the next frame start so
    /// glyphs already emitted into the current draw list keep valid UVs.
    reset_pending: bool,
    /// The theme epoch the faces above were resolved at, and the user's
    /// settings they were resolved with.
    ///
    /// A face is a THEME value — `face.<id>.family` is a list the master
    /// writes and a theme may replace — so a theme swap has to re-resolve
    /// the slots or the new file's typography never reaches the atlas.
    /// Watching the epoch here rather than asking every host to remember
    /// to call [`FontSystem::reload_faces`] is the same ruling the rest of
    /// the toolkit makes about the theme: the value has ONE reader, and it
    /// is the one that owns the thing being read.
    faces_epoch: u32,
    choice: FaceChoice,
}

impl FontSystem {
    pub fn new() -> Self {
        let (fonts, synthetic) = load_faces();
        let mut fs = FontSystem {
            fonts,
            synthetic,
            atlas: vec![0u8; ATLAS_W * ATLAS_H],
            dirty_rows: Some((0, ATLAS_H)),
            atlas_dirty: true,
            cache: HashMap::new(),
            fig_cache: HashMap::new(),
            cur_x: 2,
            cur_y: 2,
            row_h: 0,
            reset_pending: false,
            faces_epoch: crate::theme::content_epoch(),
            choice: FaceChoice::default(),
        };
        // White pixel (0,0..2x2) for solid fills.
        for y in 0..2 {
            for x in 0..2 {
                fs.atlas[y * ATLAS_W + x] = 255;
            }
        }
        fs.bake_masks();
        fs
    }

    /// Re-resolves every face slot with the user's settings folded in, and
    /// resets the atlas once for all eight.
    ///
    /// This replaced a pair of `set_ui`/`set_mono` calls that each took a
    /// ready-made [`Font`] and dropped it into one slot. With eight slots
    /// that shape cannot work: the family the user picked has to reach
    /// every slot of its kind, and each of those slots has its own WEIGHT
    /// to ask for — which is precisely what a single loaded `Font` cannot
    /// carry. The loader is the only place that knows how to turn
    /// (family, weight) into a file, so the settings go to the loader.
    pub fn reload_faces(&mut self, ov: &FaceChoice) {
        let (fonts, synthetic) = load_faces_with(ov);
        self.fonts = fonts;
        self.synthetic = synthetic;
        self.choice = ov.clone();
        self.faces_epoch = crate::theme::content_epoch();
        self.reset_atlas();
    }

    /// The em x-offset of the fake-bold second pass for this slot, or zero
    /// where the slot got the weight it asked for. Read by the draw list,
    /// which is the only place a second pass can happen.
    pub fn synthetic_bold(&self, font: u8) -> f32 {
        self.synthetic.get(font as usize).copied().unwrap_or(0.0)
    }

    /// UV of the white pixel — used by solid shapes.
    pub fn white_uv() -> (f32, f32) {
        (0.5 / ATLAS_W as f32, 0.5 / ATLAS_H as f32)
    }

    /// Clears the atlas and cache (e.g. when full after many resizes).
    /// Call once at the start of each frame, before any glyph() calls:
    /// performs a deferred atlas reset requested when the atlas filled
    /// during the previous frame. Resetting here (never mid-frame) keeps
    /// the UVs of glyphs already in the draw list valid.
    fn mark_rows(&mut self, y0: usize, y1: usize) {
        let (lo, hi) = self.dirty_rows.unwrap_or((y0, y1));
        self.dirty_rows = Some((lo.min(y0), hi.max(y1)));
        self.atlas_dirty = true;
    }

    /// The rows the renderer must re-upload, and the reset of the tracker.
    /// None = nothing changed since the last call.
    pub fn take_dirty_rows(&mut self) -> Option<(u32, u32)> {
        let r = self.dirty_rows.take();
        self.atlas_dirty = false;
        r.map(|(lo, hi)| (lo as u32, (hi - lo) as u32))
    }

    /// Bakes the procedural masks into the reserved band. Once, at startup —
    /// the band survives every atlas reset, so the bake never re-runs.
    fn bake_masks(&mut self) {
        let (mx, my, mw, mh) = MASK_SOFT;
        let (cx, cy) = (mw as f32 / 2.0 - 0.5, mh as f32 / 2.0 - 0.5);
        // Gaussian falloff, sigma at a third of the radius: reads as light,
        // not as a hard-edged disk, and the nine-slice keeps the profile.
        let r = mw as f32 / 2.0;
        let sigma = r / 3.0;
        for y in 0..mh {
            for x in 0..mw {
                let dx = x as f32 - cx;
                let dy = y as f32 - cy;
                let d = (dx * dx + dy * dy).sqrt();
                let v = if d >= r {
                    0.0
                } else {
                    (-(d * d) / (2.0 * sigma * sigma)).exp()
                };
                self.atlas[(my + y) * ATLAS_W + (mx + x)] = (v * 255.0) as u8;
            }
        }
        self.mark_rows(my, my + mh);
    }

    /// The soft-disk mask's uv rect, for the draw list's sprite emitters.
    pub fn mask_soft_uv() -> (f32, f32, f32, f32) {
        let (x, y, w, h) = MASK_SOFT;
        (
            x as f32 / ATLAS_W as f32,
            y as f32 / ATLAS_H as f32,
            (x + w) as f32 / ATLAS_W as f32,
            (y + h) as f32 / ATLAS_H as f32,
        )
    }

    pub fn begin_frame(&mut self) {
        // A theme swap changes which families and weights the eight slots
        // are meant to be, so the slots are resolved again — at a frame
        // BOUNDARY, never mid-frame, for the same reason the atlas reset
        // waits here: glyphs already in a draw list have to keep their
        // UVs. One atomic load per frame when nothing has changed.
        // The CONTENT epoch, never `theme::epoch()`. That one answers which
        // BAKE is published, and a desktop whose monitors are unequal heights
        // has two of them — one per unit size — published in turn as each
        // screen draws. Against a single remembered value it reads as a
        // change on every frame, and this guard stands in front of a walk of
        // the font directories and a re-parse of every face file. It was
        // written here, and it was `--desktop` at 100 % CPU on two monitors
        // and 5 % on one. Which families the slots name is a property of the
        // theme's text, not of the height it was baked for.
        let epoch = crate::theme::content_epoch();
        if epoch != self.faces_epoch {
            let choice = self.choice.clone();
            self.reload_faces(&choice);
            self.reset_pending = false;
            return;
        }
        if self.reset_pending {
            self.reset_atlas();
            self.reset_pending = false;
        }
    }

    fn reset_atlas(&mut self) {
        // Clear the glyph shelves only — the mask band below MASK_BAND_Y is
        // baked once and survives every reset.
        self.atlas[..MASK_BAND_Y * ATLAS_W].iter_mut().for_each(|p| *p = 0);
        for y in 0..2 {
            for x in 0..2 {
                self.atlas[y * ATLAS_W + x] = 255;
            }
        }
        self.cache.clear();
        // The figure box is measured from the FACE, so a reset that follows
        // set_ui()/set_mono() invalidates it as surely as it invalidates the
        // glyphs. Clearing on the atlas-full path too costs one re-measure
        // per (face, px) and keeps the invalidation rule to one line.
        self.fig_cache.clear();
        self.cur_x = 2;
        self.cur_y = 2;
        self.row_h = 0;
        self.mark_rows(0, MASK_BAND_Y);
    }

    pub fn glyph(&mut self, font: u8, px: f32, ch: char) -> Option<Glyph> {
        let key = (font, (px * 4.0).round() as u32, ch);
        if let Some(g) = self.cache.get(&key) {
            return *g;
        }
        let f = &self.fonts[font as usize];
        let (metrics, bitmap) = f.rasterize(ch, px);
        if metrics.width == 0 || metrics.height == 0 {
            let g = Some(Glyph {
                u0: 0.0,
                v0: 0.0,
                u1: 0.0,
                v1: 0.0,
                w: 0.0,
                h: 0.0,
                xmin: 0.0,
                ymin: 0.0,
                advance: metrics.advance_width,
            });
            self.cache.insert(key, g);
            return g;
        }
        let (w, h) = (metrics.width, metrics.height);
        if self.cur_x + w + 2 > ATLAS_W {
            self.cur_x = 2;
            self.cur_y += self.row_h + 2;
            self.row_h = 0;
        }
        if self.cur_y + h + 2 > MASK_BAND_Y {
            // Atlas full — defer the reset to the next frame (begin_frame)
            // instead of zeroing it mid-frame under the current draw list.
            // This glyph is skipped for one frame, then rendered cleanly.
            self.reset_pending = true;
            return None;
        }
        let (ax, ay) = (self.cur_x, self.cur_y);
        for row in 0..h {
            let dst = (ay + row) * ATLAS_W + ax;
            self.atlas[dst..dst + w].copy_from_slice(&bitmap[row * w..row * w + w]);
        }
        self.cur_x += w + 2;
        self.row_h = self.row_h.max(h);
        self.mark_rows(ay, ay + h);

        let g = Some(Glyph {
            u0: ax as f32 / ATLAS_W as f32,
            v0: ay as f32 / ATLAS_H as f32,
            u1: (ax + w) as f32 / ATLAS_W as f32,
            v1: (ay + h) as f32 / ATLAS_H as f32,
            w: w as f32,
            h: h as f32,
            xmin: metrics.xmin as f32,
            ymin: metrics.ymin as f32,
            advance: metrics.advance_width,
        });
        self.cache.insert(key, g);
        g
    }

    /// Line metrics: (ascent, line height).
    pub fn line_metrics(&self, font: u8, px: f32) -> (f32, f32) {
        if let Some(m) = self.fonts[font as usize].horizontal_line_metrics(px) {
            (m.ascent, m.ascent - m.descent + m.line_gap)
        } else {
            (px * 0.8, px * 1.2)
        }
    }

    /// Cell width for the monospace font.
    pub fn mono_advance(&mut self, px: f32) -> f32 {
        self.glyph(FONT_MONO, px, 'M').map(|g| g.advance).unwrap_or(px * 0.6)
    }

    /// The figure box of a tabular role at this (face, px): `set` is
    /// `num.tabular_set` and `punct` is `num.tabular_punct`, both read from
    /// the theme by the caller — this type owns faces, not tokens.
    ///
    /// An empty set answers [`Figures::NONE`]: a theme that empties the set
    /// has said which characters get the box, and the answer was "none".
    pub fn figures(&mut self, font: u8, px: f32, set: &str, punct: bool) -> Figures {
        if set.is_empty() || font >= FONT_COUNT {
            return Figures::NONE;
        }
        let key = (font, (px * 4.0).round() as u32, fnv1a(set.as_bytes()), punct);
        if let Some(f) = self.fig_cache.get(&key) {
            return *f;
        }
        let mut fig = Figures::NONE;
        let mut advance = 0.0f32;
        let mut complete = true;
        let mut overflow = false;
        for ch in set.chars() {
            // The box is the widest advance among the SET's own characters.
            match self.glyph(font, px, ch) {
                Some(g) => advance = advance.max(g.advance),
                // The atlas filled up mid-frame. A box measured from a
                // partial set is NARROWER than a figure, which draws the
                // column as overlapping glyphs — worse than no box at all
                // — so this frame draws proportionally and the next one,
                // after begin_frame()'s reset, measures it properly.
                None => complete = false,
            }
            overflow |= !fig.add(ch);
        }
        // `tabular_punct` pins punctuation INTO the box; it must never
        // widen it. '%' is wider than any digit in most faces, and letting
        // the punctuation into the max would grow every number on screen
        // the moment the flag went on. The marks are carried as the flag
        // rather than folded into the bitmap because they join the box
        // only where they are part of a number, which is a question about
        // their NEIGHBOURS and so cannot be settled here.
        fig.punct = punct;
        if overflow {
            crate::ui::warn_once(
                "num.tabular_set",
                &format!(
                    "num.tabular_set carries more than {} characters outside ASCII \
                     — the extras keep the face's advance",
                    Figures::EXTRA
                ),
            );
        }
        if !complete || advance <= 0.0 {
            return Figures::NONE;
        }
        fig.advance = advance;
        self.fig_cache.insert(key, fig);
        fig
    }

    pub fn measure(&mut self, font: u8, px: f32, text: &str, letter_spacing: f32) -> f32 {
        self.measure_fig(font, px, text, letter_spacing, &Figures::NONE)
    }

    /// [`FontSystem::measure`] under a figure box. A member of the box is
    /// counted at the box's width without rasterising anything, so an
    /// all-figure string measures `len x (advance + letter_spacing)` with
    /// no atlas traffic — §5.17's reason for doing this in the toolkit.
    pub fn measure_fig(
        &mut self,
        font: u8,
        px: f32,
        text: &str,
        letter_spacing: f32,
        fig: &Figures,
    ) -> f32 {
        let mut w = 0.0;
        for (prev, ch, next) in with_neighbours(text) {
            if let Some(a) = fig.advance_of(prev, ch, next) {
                w += a + letter_spacing;
            } else if let Some(g) = self.glyph(font, px, ch) {
                w += g.advance + letter_spacing;
            }
        }
        w
    }
}

fn try_load(path: &Path) -> Option<Font> {
    SCAN_PARSES.fetch_add(1, Ordering::Relaxed);
    let data = std::fs::read(path).ok()?;
    Font::from_bytes(data, FontSettings::default()).ok()
}

// --------------------------------------------------------- the scan meter
//
// What loading a face COSTS, counted where it is spent rather than
// asserted in a comment. A face is answered from a list of file names, and
// the only honest way to say whether that list is read once or once per
// slot is to count the reads: `walks` is a traversal of the directory
// list, `dirs` is an `openat(..., O_DIRECTORY)`, `stats` is a `statx` and
// `parses` is a font file read whole and decoded. Three of the four are
// what a system trace of this program shows, so a measurement here and a
// measurement out there are largely the same measurement.
//
// LARGELY, and the exception is worth naming where the numbers are, not
// only at the field: `stats` counts the stats this file ASKS for. On a
// filesystem that does not carry an entry's kind in the directory entry
// itself — NFS, some overlays — the standard library asks for us, once per
// entry, and that call is invisible from here. So `stats` is a true count
// of syscalls on a machine whose font directories carry the kind (which is
// every local filesystem this program has been traced on), and a floor
// rather than a count on one that does not. `walks`, `dirs` and `parses`
// are exact everywhere, and the defect this meter was written for shows in
// all four.
//
// Always compiled, never behind a test flag: a counter that only exists
// under `cfg(test)` measures a different program than the one that ships,
// and this file's whole defect was that nobody was counting.

static SCAN_WALKS: AtomicU64 = AtomicU64::new(0);
static SCAN_DIRS: AtomicU64 = AtomicU64::new(0);
static SCAN_STATS: AtomicU64 = AtomicU64::new(0);
static SCAN_PARSES: AtomicU64 = AtomicU64::new(0);

/// The font loading's cost so far, in this process.
///
/// Monotonic, so a caller measures an OPERATION by the difference across
/// it. That is deliberate: the index below is process-wide, so "what did
/// this theme load cost" is a question about a delta, never about a total.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScanCount {
    /// Traversals of the whole font directory list.
    pub walks: u64,
    /// Directories opened.
    pub dirs: u64,
    /// Entries stat-ed to learn whether they are a directory. Only the
    /// stats this file ASKS for: a filesystem that does not carry the kind
    /// in the directory entry makes the standard library ask for us, and
    /// that one is not visible from here.
    pub stats: u64,
    /// Font files read off the disk and parsed into a glyph table. The
    /// other half of the startup cost, and the one that is not syscalls.
    pub parses: u64,
}

/// Reads [`ScanCount`]. See its documentation for why the numbers only
/// mean something as a difference.
pub fn scan_count() -> ScanCount {
    ScanCount {
        walks: SCAN_WALKS.load(Ordering::Relaxed),
        dirs: SCAN_DIRS.load(Ordering::Relaxed),
        stats: SCAN_STATS.load(Ordering::Relaxed),
        parses: SCAN_PARSES.load(Ordering::Relaxed),
    }
}

/// How deep under a font directory the search looks. Not a theme value —
/// it bounds a walk of the FILESYSTEM, which no theme owns — and the same
/// bound the recursive search has always carried.
const SCAN_DEPTH: u32 = 4;

/// One candidate file: the name it is compared under, and where it is.
///
/// The name is normalised exactly the way the patterns are written —
/// lowercased, everything but letters and digits dropped — so `matches`
/// below is a plain substring test and the normalisation is paid once per
/// file instead of once per file per question.
struct FontFile {
    name: String,
    path: PathBuf,
}

/// Every font file the directory list holds, in the order the recursive
/// search used to reach them in.
///
/// The ORDER is the load-bearing part. The old search answered with the
/// first file it walked into that matched, so "which file does this
/// pattern get" is a question about traversal order and nothing else. This
/// list is built by that same traversal — directories in the list's order,
/// depth-first, entries in the order the filesystem hands them back — so
/// answering from it and answering from a fresh walk are the same answer.
/// The only thing that changes is how many times the disk is asked.
struct FontIndex {
    /// The list this index was built for. The one thing that invalidates
    /// it: a different set of directories is a different question, where a
    /// different THEME is not — a theme changes which family a slot asks
    /// for, never which files exist.
    dirs: Vec<PathBuf>,
    files: Vec<FontFile>,
}

impl FontIndex {
    fn build(dirs: &[PathBuf]) -> FontIndex {
        fn walk(dir: &Path, depth: u32, out: &mut Vec<FontFile>) {
            if depth > SCAN_DEPTH {
                return;
            }
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            SCAN_DIRS.fetch_add(1, Ordering::Relaxed);
            for entry in rd.flatten() {
                let p = entry.path();
                // `readdir` already said whether this is a directory on
                // every filesystem that carries the kind in the directory
                // entry, so asking the kernel again is a syscall for an
                // answer already in hand. A SYMLINK is the exception: its
                // own kind says nothing about what it points at, and this
                // search has always followed them, so that is the one
                // entry still worth a stat.
                let is_dir = match entry.file_type() {
                    Ok(t) if !t.is_symlink() => t.is_dir(),
                    _ => {
                        SCAN_STATS.fetch_add(1, Ordering::Relaxed);
                        p.is_dir()
                    }
                };
                if is_dir {
                    walk(&p, depth + 1, out);
                    continue;
                }
                let name: String = p
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_lowercase()
                    .chars()
                    .filter(|c| c.is_ascii_alphanumeric())
                    .collect();
                if !(name.ends_with("ttf") || name.ends_with("otf")) {
                    continue;
                }
                // Avoid italic variants; bold only when explicitly
                // requested. Italics are dropped here rather than at the
                // question, because a file no pattern may ever answer with
                // is not a candidate at all — the index holds what the
                // search is allowed to return.
                if name.contains("italic") || name.contains("oblique") {
                    continue;
                }
                out.push(FontFile { name, path: p });
            }
        }
        SCAN_WALKS.fetch_add(1, Ordering::Relaxed);
        let mut files = Vec::new();
        for d in dirs {
            walk(d, 0, &mut files);
        }
        FontIndex { dirs: dirs.to_vec(), files }
    }

    /// The first file this pattern names, in traversal order.
    fn find(&self, pat: &str) -> Option<PathBuf> {
        self.files
            .iter()
            .find(|f| matches(&f.name, pat))
            .map(|f| f.path.clone())
    }
}

/// Whether a normalised file name answers a pattern. Bold is only ever
/// handed to a pattern that asked for it: the weight words live in the
/// file name, so a bare family pattern would otherwise settle on whichever
/// weight the directory happened to list first.
fn matches(name: &str, pat: &str) -> bool {
    name.contains(pat) && !(name.contains("bold") && !pat.contains("bold"))
}

/// The index, built once and kept.
///
/// A `Mutex` and not a `OnceLock` because the directory list can change
/// under a running program — `HOME` is read for two of the five entries —
/// and because a rescan has to be possible at all (see
/// [`forget_font_index`]).
static INDEX: Mutex<Option<Arc<FontIndex>>> = Mutex::new(None);

/// The index for this directory list, building it if the list is new.
fn font_index(dirs: &[PathBuf]) -> Arc<FontIndex> {
    let mut slot = INDEX.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(ix) = slot.as_ref() {
        if ix.dirs.as_slice() == dirs {
            return Arc::clone(ix);
        }
    }
    let ix = Arc::new(FontIndex::build(dirs));
    *slot = Some(Arc::clone(&ix));
    ix
}

/// Drops the index, so the next question reads the directories again.
///
/// For the one thing the index cannot see: a font INSTALLED while the
/// program runs. [`fresh_index`] below is the toolkit's own caller — the
/// family lists a settings page shows — and this stays public for a host
/// that offers a "look again" somewhere else. A theme swap is not such a
/// moment and must never call this: re-reading the tree on every theme
/// swap is the defect this index exists to remove.
pub fn forget_font_index() {
    *INDEX.lock().unwrap_or_else(|e| e.into_inner()) = None;
}

/// The index built again from the disk, whatever the old one held.
///
/// The one question that has to see the directories again is "what does
/// this machine have RIGHT NOW", and there is exactly one asker: a person
/// who installed a font and opened the settings page to pick it. Every
/// other question — a theme swap, a weight change, a face slot resolving —
/// asks which of the files that EXIST answers a pattern, and that cannot
/// have changed without somebody putting a file on the disk.
///
/// So the family lists cost ONE traversal each, where before the index
/// they cost one per curated name: twelve for the monospace list and
/// nineteen for the interface one, thirty-one for the page. Two rather
/// than one for the page as a whole, because the toolkit is asked two
/// questions and cannot see that they are one moment — the host would have
/// to say so, and no host API here says it.
fn fresh_index() -> Arc<FontIndex> {
    forget_font_index();
    font_index(&font_dirs())
}

/// The file a pattern names, from the index of the font directories.
///
/// The patterns are tried in order and the first that names a file wins,
/// which is what the caller's ordering means: a candidate list is written
/// best-first. Every pattern is answered from the SAME reading of the
/// directories — the search used to walk the whole tree once per pattern,
/// and the eight face slots between them spell out sixty-odd patterns.
fn find_font(dirs: &[PathBuf], patterns: &[&str]) -> Option<PathBuf> {
    let index = font_index(dirs);
    patterns.iter().find_map(|pat| index.find(pat))
}

/// Where a font file is looked for, most specific first.
///
/// EVERY ENTRY IS ABSOLUTE, and that is the point of this list rather
/// than a detail of it. The first entry used to be the bare name
/// `fonts` — a directory relative to whatever the working directory
/// happened to be — so which typeface the interface came up in was
/// decided by where the program had been STARTED from, and any
/// directory a person could `cd` into could hand it the files it draws
/// with. It was there for a checkout's own `fonts/` folder, which is
/// the one case it cannot serve honestly: no font file is ever
/// committed to these repositories (licence — see
/// `nacelle-desktop/fonts/README.md`), so on any machine but the one
/// that put files there by hand it matched nothing at all, 94 failed
/// openat calls per run.
///
/// The deliberate door replaces it: `NACELLE_FONT_DIR`, named the way
/// `NACELLE_THEME_DIR` already names the same wish for themes, and
/// ACCEPTED ONLY IF ABSOLUTE — a relative value would put the old
/// behaviour back under a new name. A checkout that wants its own
/// fonts says so; an INSTALLED one needs nothing, because
/// `nacelle-desktop/Makefile` puts a checkout's `fonts/` under
/// `$(PREFIX)/share/fonts/nacelle-desktop` and both of the prefixes it
/// defaults to are already below — `~/.local/share/fonts` for a user
/// install, `/usr/local/share/fonts` for `sudo make install`.
fn font_dirs() -> Vec<PathBuf> {
    font_search_path(
        std::env::var_os("NACELLE_FONT_DIR"),
        std::env::var_os("HOME"),
    )
}

/// [`font_dirs`] with the environment handed in, so that what the list
/// IS can be read and tested without one — this crate's tests share a
/// process, and a test that set `HOME` would be deciding what another
/// one saw.
fn font_search_path(explicit: Option<OsString>, home: Option<OsString>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = explicit {
        dirs.push(PathBuf::from(d));
    }
    if let Some(home) = home {
        let home = PathBuf::from(home);
        dirs.push(home.join(".local/share/fonts"));
        dirs.push(home.join(".fonts"));
    }
    dirs.push(PathBuf::from("/usr/share/fonts"));
    dirs.push(PathBuf::from("/usr/local/share/fonts"));
    // The invariant, kept in one place rather than at every push: a
    // relative entry is a directory that moves when the program is
    // started from somewhere else, and both of the outside values above
    // — `NACELLE_FONT_DIR` and `HOME` — are things a person can set to
    // anything at all.
    dirs.retain(|d| d.is_absolute());
    dirs
}

/// Curated monospace families for the settings dropdown
/// (display name, normalized filename pattern).
const MONO_FAMILIES: [(&str, &str); 12] = [
    ("Fira Mono", "firamono"),
    ("Fira Code", "firacode"),
    ("JetBrains Mono", "jetbrainsmono"),
    ("DejaVu Sans Mono", "dejavusansmono"),
    ("Liberation Mono", "liberationmono"),
    ("Noto Sans Mono", "notosansmono"),
    ("Ubuntu Mono", "ubuntumono"),
    ("Source Code Pro", "sourcecodepro"),
    ("Hack", "hack"),
    ("IBM Plex Mono", "ibmplexmono"),
    ("Cascadia Code", "cascadiacode"),
    ("Inconsolata", "inconsolata"),
];

/// Curated interface (UI) families (display name, filename pattern).
const UI_FAMILIES: [(&str, &str); 7] = [
    ("United Sans", "unitedsans"),
    ("Oxanium", "oxanium"),
    ("Rajdhani", "rajdhani"),
    ("Exo 2", "exo2"),
    ("Orbitron", "orbitron"),
    ("Saira Condensed", "sairacondensed"),
    ("Saira", "saira"),
];

fn pattern_for(display: &str) -> Option<&'static str> {
    MONO_FAMILIES
        .iter()
        .chain(UI_FAMILIES.iter())
        .find(|(name, _)| *name == display)
        .map(|(_, pat)| *pat)
}

/// The curated names of `table` that this reading of the directories has a
/// file for. Takes the index rather than fetching one, so that a caller
/// asking two tables asks them of the SAME reading.
fn available_from(index: &FontIndex, table: &[(&str, &str)]) -> Vec<String> {
    table
        .iter()
        .filter(|(_, pat)| index.find(pat).is_some())
        .map(|(name, _)| name.to_string())
        .collect()
}

/// Monospace families actually available on this system (terminal font).
///
/// Reads the directories again — see [`fresh_index`]. This is the settings
/// page's list, and a list of what is installed that cannot see a font
/// installed since the program started is a list of the wrong thing.
pub fn available_mono_families() -> Vec<String> {
    available_from(&fresh_index(), &MONO_FAMILIES)
}

/// Interface families available on this system (UI list first, then mono).
///
/// Reads the directories again, once, for both tables — see [`fresh_index`].
pub fn available_ui_families() -> Vec<String> {
    let index = fresh_index();
    let mut out = available_from(&index, &UI_FAMILIES);
    out.extend(available_from(&index, &MONO_FAMILIES));
    out
}

/// Default search patterns used when no family is selected. The master's
/// own `face.mono.family` and `face.ui.family` lists, in their order — the
/// floor a slot lands on is the theme's list, never a family of this file's
/// own choosing.
const DEFAULT_MONO_PATTERNS: [&str; 4] = [
    "jetbrainsmonoregular", "jetbrainsmono", "firamonoregular", "firamono",
];
const DEFAULT_UI_PATTERNS: [&str; 5] = [
    "rajdhani", "saira", "exo2", "unitedsansmedium", "unitedsans",
];

/// Loads a font by family display name and weight
/// (Light/Regular/Medium/SemiBold/Bold). With no family selected the
/// weight is searched across the default families of the given kind.
pub fn load_variant_for(
    family: Option<&str>,
    weight: Option<&str>,
    ui: bool,
) -> Option<Font> {
    let dirs = font_dirs();
    let w = weight.unwrap_or("Regular").to_lowercase().replace(' ', "");
    let base: Vec<&str> = match family.and_then(pattern_for) {
        Some(p) => vec![p],
        None => {
            if ui {
                DEFAULT_UI_PATTERNS.to_vec()
            } else {
                DEFAULT_MONO_PATTERNS.to_vec()
            }
        }
    };
    // The requested weight first, across all candidate families. For the
    // default UI font the weighted search also covers the mono families,
    // because United Sans ships in a single weight only.
    let mut weighted = base.clone();
    if ui && family.is_none() {
        weighted.extend(DEFAULT_MONO_PATTERNS);
    }
    if w != "regular" {
        for pat in &weighted {
            let c = format!("{pat}{w}");
            if let Some(p) = find_font(&dirs, &[c.as_str()]) {
                if let Some(f) = try_load(&p) {
                    return Some(f);
                }
            }
        }
    }
    // ...then the regular variants.
    for pat in &base {
        for c in [format!("{pat}regular"), pat.to_string()] {
            if let Some(p) = find_font(&dirs, &[c.as_str()]) {
                if let Some(f) = try_load(&p) {
                    return Some(f);
                }
            }
        }
    }
    None
}

/// Loads the default terminal font (the master's own `face.mono.family`).
pub fn load_default_mono() -> Font {
    let dirs = font_dirs();
    let mono_path = std::env::var("NACELLE_FONT_MONO").ok().map(PathBuf::from).or_else(|| {
        find_font(&dirs, &DEFAULT_MONO_PATTERNS)
    });
    mono_path.as_deref().and_then(try_load).unwrap_or_else(|| {
        panic!(
            "nacelle-desktop: no monospace font (.ttf/.otf) found.\n\
             Point NACELLE_FONT_MONO at one or drop it into ./fonts"
        )
    })
}

/// Loads the default interface font (the master's own `face.ui.family`;
/// falls back to the monospace font).
pub fn load_default_ui() -> Font {
    let dirs = font_dirs();
    let ui_path = std::env::var("NACELLE_FONT_UI").ok().map(PathBuf::from).or_else(|| {
        find_font(&dirs, &DEFAULT_UI_PATTERNS)
    });
    ui_path.as_deref().and_then(try_load).unwrap_or_else(|| {
        eprintln!("nacelle-desktop: no UI font (United Sans) — using the monospace font");
        load_default_mono()
    })
}

// -------------------------------------------------------- face resolution
//
// §5.16's eight slots, resolved once at load. Until now this file loaded
// two files and every `face` the master declared — five distinct words
// across the twenty-four roles, four distinct WEIGHTS across the eight
// slots — arrived at the atlas as one of those two. The theme said 400,
// 500, 600 and 700; the screen said Regular, and said it silently.
//
// The order below is the master's own, written above `[face.ui]`:
// requested weight -> Regular (+ synthetic bold if >=600) -> the fallback
// chain (cycles broken at depth 8) -> FACE_UI (FACE_MONO for mono*) -> if
// FACE_MONO itself is unresolvable the load fails. Every substitution
// warns; silent substitution is forbidden, which is why every arm here
// that settles for less than it asked for says so.

/// The user's font settings — the two families and two weights the
/// settings panel offers. Not a theme value: the master says what the
/// interface is SET IN, this says what this user asked to read it in.
#[derive(Default, Clone, Debug)]
pub struct FaceChoice {
    pub ui_family: Option<String>,
    pub ui_weight: Option<String>,
    pub mono_family: Option<String>,
    pub mono_weight: Option<String>,
}

/// The filename word a numeric weight is looked for under. The master
/// spells five of these out beside `face.<id>.weight`; the other four are
/// the same scale continued, because a theme may write any of 100..900 and
/// a number the table did not hold would silently become Regular.
fn weight_word(w: u32) -> &'static str {
    match w {
        0..=149 => "Thin",
        150..=249 => "ExtraLight",
        250..=349 => "Light",
        350..=449 => "Regular",
        450..=549 => "Medium",
        550..=649 => "SemiBold",
        650..=749 => "Bold",
        750..=849 => "ExtraBold",
        _ => "Black",
    }
}

/// The filename pattern a family DISPLAY name is searched under.
///
/// The curated tables answer for the families the settings panel offers;
/// anything else is normalised the way [`find_font`] normalises the files
/// it compares against. Without that second half a theme could name only
/// the twenty families this file happens to know, which would make the
/// engine — not the master — the authority on which fonts exist.
fn family_pattern(display: &str) -> String {
    if let Some(p) = pattern_for(display) {
        return p.to_string();
    }
    display.to_lowercase().chars().filter(|c| c.is_ascii_alphanumeric()).collect()
}

/// One slot as the master declares it.
struct FaceSpec {
    families: Vec<String>,
    weight: u32,
    /// `face.<id>.file` — a theme-shipped binary, the one path allowed to
    /// escape the system font directories.
    file: String,
    /// `face.<id>.fallback`: another face id, `builtin`, or `none`.
    fallback: String,
}

/// Reads `[face.<id>]` out of the live theme.
fn face_spec(id: &str) -> FaceSpec {
    let t = crate::theme::resolved();
    let d = crate::theme::diagnostics();
    // A family list is an indexed family in the cascade — `family[0]`,
    // `family[1]`, ... — so it is walked until the master stops declaring
    // one, rather than asked for a length the schema does not publish.
    let mut families = Vec::new();
    for i in 0.. {
        match d.text(&format!("face.{id}.family[{i}]")) {
            Some(f) if !f.is_empty() => families.push(f.to_string()),
            Some(_) => continue,
            None => break,
        }
    }
    let weight = crate::theme::id(&format!("face.{id}.weight"))
        .map(|k| t.px(k))
        .filter(|w| *w > 0.0)
        .unwrap_or(400.0) as u32;
    let fallback = crate::theme::id(&format!("face.{id}.fallback"))
        .and_then(crate::theme::enum_word_of)
        .unwrap_or_default();
    FaceSpec {
        families,
        weight,
        file: d.text(&format!("face.{id}.file")).unwrap_or_default().to_string(),
        fallback,
    }
}

/// A slot's answer: the file it settled on, and the em offset of the
/// fake-bold pass it needs to look like the weight it could not find.
struct Resolved {
    path: PathBuf,
    synthetic: f32,
}

/// Tries one slot's OWN families at one weight. `exact` asks for the
/// weight the master wrote; otherwise Regular, which is the rung §5.16
/// drops to before it leaves the slot.
fn try_families(spec: &FaceSpec, dirs: &[PathBuf], exact: bool) -> Option<PathBuf> {
    let w = if exact { weight_word(spec.weight) } else { "Regular" };
    let w = w.to_lowercase();
    for fam in &spec.families {
        let pat = family_pattern(fam);
        let mut names = vec![format!("{pat}{w}")];
        if !exact {
            // A family shipped as one file carries no weight word at all.
            names.push(pat.clone());
        }
        for n in &names {
            if let Some(p) = find_font(dirs, &[n.as_str()]) {
                return Some(p);
            }
        }
    }
    None
}

/// One slot, resolved down §5.16's ladder. `depth` is the fallback chain's
/// cycle brake — the master states the cap, so it is spent here and not
/// invented per call.
fn resolve_face(id: &str, dirs: &[PathBuf], ov: &FaceChoice, depth: u32) -> Option<Resolved> {
    if depth > FONT_COUNT as u32 {
        crate::ui::warn_once(
            &format!("face.chain:{id}"),
            &format!("face.{id}.fallback chains more than {FONT_COUNT} deep — the chain is cut"),
        );
        return None;
    }
    let mut spec = face_spec(id);
    // The user's family goes in FRONT of the master's list rather than
    // replacing it: a family the user picked and the system has since lost
    // must fall through to the theme's, not to nothing. The weight moves
    // as a DELTA so the ladder survives — asking for Bold makes `ui_bold`
    // heavier than `ui_medium` still, where pinning every slot at 700
    // would flatten the master's four weights into one.
    let (fam, wgt) = if id.starts_with("mono") {
        (&ov.mono_family, &ov.mono_weight)
    } else {
        (&ov.ui_family, &ov.ui_weight)
    };
    if let Some(f) = fam {
        if !f.is_empty() {
            spec.families.insert(0, f.clone());
        }
    }
    if let Some(w) = wgt.as_deref().filter(|w| !w.is_empty()) {
        let base = face_spec(if id.starts_with("mono") { "mono" } else { "ui" }).weight as i32;
        let asked = weight_number(w);
        spec.weight = (spec.weight as i32 + (asked - base)).clamp(100, 900) as u32;
    }

    // A theme-shipped binary outranks every search: the master calls it
    // the only path allowed out of the system font directories.
    if !spec.file.is_empty() {
        if let Some(dir) = crate::theme::diagnostics().path.as_ref().and_then(|p| p.parent()) {
            let p = dir.join(&spec.file);
            if p.is_file() {
                return Some(Resolved { path: p, synthetic: 0.0 });
            }
            crate::ui::warn_once(
                &format!("face.file:{id}"),
                &format!("face.{id}.file names \"{}\", which is not there", spec.file),
            );
        }
    }
    if let Some(path) = try_families(&spec, dirs, true) {
        return Some(Resolved { path, synthetic: 0.0 });
    }
    if let Some(path) = try_families(&spec, dirs, false) {
        // §5.16: a >=600 request that came back with a Regular file is
        // faked, and the fake is announced. Below 600 there is nothing to
        // fake — Light drawn as Regular is a substitution, not a weight.
        let synthetic = if spec.weight >= 600 {
            crate::theme::id(&format!("face.{id}.synthetic_bold"))
                .map(|k| crate::theme::resolved().px(k))
                .unwrap_or(0.0)
        } else {
            0.0
        };
        // A slot that asked for Regular and got Regular substituted
        // nothing: the two passes above are the same request written two
        // ways (`<family>regular` and the bare family), and announcing
        // that as a substitution would put six lines of noise on every
        // start for a theme in which nothing at all went wrong.
        if weight_word(spec.weight) != "Regular" {
            crate::ui::warn_once(
                &format!("face.weight:{id}"),
                &format!(
                    "face.{id} asked for {} ({}) and the system has only Regular — {}",
                    spec.weight,
                    weight_word(spec.weight),
                    if synthetic > 0.0 { "the weight is faked" } else { "Regular is used" }
                ),
            );
        }
        return Some(Resolved { path, synthetic });
    }
    match spec.fallback.as_str() {
        // `builtin` is the icon slot's answer: the compiled-in vector set
        // draws those names, and no file is wanted. `none` ends the chain.
        "" | "none" | "builtin" => None,
        next => {
            crate::ui::warn_once(
                &format!("face.fallback:{id}"),
                &format!("face.{id} resolved to no file — falling back to face.{next}"),
            );
            resolve_face(next, dirs, ov, depth + 1)
        }
    }
}

/// The number a weight WORD stands for, for folding the user's choice into
/// the master's ladder. The inverse of [`weight_word`] over the five names
/// the settings panel offers.
fn weight_number(word: &str) -> i32 {
    match word.to_lowercase().replace(' ', "").as_str() {
        "thin" => 100,
        "extralight" => 200,
        "light" => 300,
        "medium" => 500,
        "semibold" => 600,
        "bold" => 700,
        "extrabold" => 800,
        "black" => 900,
        _ => 400,
    }
}

/// Every slot, at the theme's own settings.
fn load_faces() -> ([Font; FONT_COUNT as usize], [f32; FONT_COUNT as usize]) {
    load_faces_with(&FaceChoice::default())
}

/// Every slot, with the user's settings folded in.
///
/// The two engine slots keep the environment overrides they have always
/// had (`NACELLE_FONT_UI`, `NACELLE_FONT_MONO`), because those are how a
/// machine with no matching family in its font directories starts at all —
/// including the one this suite runs on.
fn load_faces_with(ov: &FaceChoice) -> ([Font; FONT_COUNT as usize], [f32; FONT_COUNT as usize]) {
    let dirs = font_dirs();
    let mut paths: [Option<Resolved>; FONT_COUNT as usize] = Default::default();
    for (i, id) in FACE_IDS.iter().enumerate() {
        paths[i] = resolve_face(id, &dirs, ov, 0);
    }
    // One parse per FILE, not per slot: six of the eight slots commonly
    // resolve onto two or three files, and a `Font` is a parsed table.
    //
    // The two ENGINE slots go through the same map as the other six, and
    // that is the whole of the change here. They used to be parsed above
    // it, so the file they landed on was parsed once for the slot and
    // again for the first of the six that fell back onto it — and falling
    // back onto them is what §5.16's step 5 makes every chain END in. On
    // the owner's machine `display -> ui_bold -> ui_medium -> ui` is one
    // file resolved four times, and the parse is the expensive half of
    // loading a face.
    let mut by_path: HashMap<PathBuf, Font> = HashMap::new();
    // The two ends of every chain, per §5.16's step 5. `load_default_*`
    // carries the environment override and the historical search order, so
    // a slot that found nothing lands exactly where the two-slot engine
    // put every one of them — which is the behaviour this replaces, kept
    // as the floor under it rather than as the rule above it.
    let ui = paths[FONT_UI as usize]
        .as_ref()
        .and_then(|r| load_into(&r.path, &mut by_path))
        .unwrap_or_else(load_default_ui);
    let mono = paths[FONT_MONO as usize]
        .as_ref()
        .and_then(|r| load_into(&r.path, &mut by_path))
        .unwrap_or_else(load_default_mono);
    let fonts: [Font; FONT_COUNT as usize] = std::array::from_fn(|i| {
        // The two engine slots are already loaded above, because every
        // other slot's last resort is one of them.
        if i == FONT_UI as usize {
            return ui.clone();
        }
        if i == FONT_MONO as usize {
            return mono.clone();
        }
        match &paths[i] {
            Some(r) => load_into(&r.path, &mut by_path)
                .unwrap_or_else(|| alias_of(FACE_IDS[i], &ui, &mono)),
            None => alias_of(FACE_IDS[i], &ui, &mono),
        }
    });
    let synthetic = std::array::from_fn(|i| paths[i].as_ref().map_or(0.0, |r| r.synthetic));
    (fonts, synthetic)
}

/// This file's glyph table, parsed at most once per load.
///
/// The second slot to ask for a file gets a clone, which copies the tables
/// `fontdue` built but does not read the file or decode its outlines
/// again. That is the trade this map has always made for six of the eight
/// slots; the two engine slots simply were not in it.
fn load_into(path: &Path, by_path: &mut HashMap<PathBuf, Font>) -> Option<Font> {
    if let Some(f) = by_path.get(path) {
        return Some(f.clone());
    }
    let f = try_load(path)?;
    by_path.insert(path.to_path_buf(), f.clone());
    Some(f)
}

/// Where a slot with no file of its own lands: the monospace slot when its
/// name says monospace, the interface slot otherwise. The same rule
/// [`face_slot`] applies to an unknown WORD, so the two ways of failing to
/// name a face agree.
fn alias_of(id: &str, ui: &Font, mono: &Font) -> Font {
    if id.starts_with("mono") {
        mono.clone()
    } else {
        ui.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The search as it was before the index: a fresh recursive walk per
    /// pattern, answering with the first file it reaches that matches.
    ///
    /// Written out in full, and deliberately not sharing a line with the
    /// code above. The index's whole claim is that reading the tree once
    /// and answering from a list gives the SAME file as reading it again
    /// for every question — a claim only a second, independent
    /// implementation can check. Sharing the matching rule with the thing
    /// under test would make this a test of nothing.
    fn reference(dirs: &[PathBuf], pat: &str) -> Option<PathBuf> {
        fn walk(dir: &Path, pat: &str, depth: u32, out: &mut Option<PathBuf>) {
            if depth > 4 || out.is_some() {
                return;
            }
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            for entry in rd.flatten() {
                if out.is_some() {
                    return;
                }
                let p = entry.path();
                if p.is_dir() {
                    walk(&p, pat, depth + 1, out);
                } else {
                    let name: String = p
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_lowercase()
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric())
                        .collect();
                    if !(name.ends_with("ttf") || name.ends_with("otf")) {
                        continue;
                    }
                    if name.contains("italic") || name.contains("oblique") {
                        continue;
                    }
                    if name.contains(pat) {
                        if name.contains("bold") && !pat.contains("bold") {
                            continue;
                        }
                        *out = Some(p.clone());
                    }
                }
            }
        }
        for d in dirs {
            let mut found = None;
            walk(d, pat, 0, &mut found);
            if found.is_some() {
                return found;
            }
        }
        None
    }

    /// Every pattern this file can put to the search: the curated families,
    /// the default chains, and the weight spellings a `[face.*]` block
    /// produces. A pattern nothing on this machine answers is as much a
    /// case as one that hits — "found nothing" has to agree too.
    fn every_pattern() -> Vec<String> {
        let mut pats: Vec<String> = MONO_FAMILIES
            .iter()
            .chain(UI_FAMILIES.iter())
            .map(|(_, p)| p.to_string())
            .chain(DEFAULT_MONO_PATTERNS.iter().map(|p| p.to_string()))
            .chain(DEFAULT_UI_PATTERNS.iter().map(|p| p.to_string()))
            .collect();
        // What `try_families` spells: the family, and the family with each
        // weight word the master may write, in the same lowercase form.
        let words: Vec<String> = (1..=9)
            .map(|w| weight_word(w * 100).to_lowercase())
            .collect();
        for fam in ["firamono", "jetbrainsmono", "rajdhani", "orbitron", "notosansmono"] {
            for w in &words {
                pats.push(format!("{fam}{w}"));
            }
        }
        // And a family this machine certainly does not have, so the
        // "nothing" answer is measured rather than assumed.
        pats.push("nosuchfamilyanywhere".into());
        pats
    }

    #[test]
    fn the_index_answers_what_a_fresh_walk_answers() {
        let dirs = font_dirs();
        let index = font_index(&dirs);
        let mut hits = 0;
        for pat in every_pattern() {
            let from_index = index.find(&pat);
            let from_disk = reference(&dirs, &pat);
            assert_eq!(
                from_index, from_disk,
                "pattern {pat:?} resolves to {from_index:?} out of the index \
                 and to {from_disk:?} out of a fresh walk — the index is not \
                 in traversal order, or its filters are not the search's"
            );
            hits += usize::from(from_index.is_some());
        }
        // Fail closed: on a machine where every pattern answers None the
        // loop above compares nothing to nothing.
        assert!(
            hits > 0,
            "no pattern found a file at all — this machine's font \
             directories hold nothing the search can answer with, so the \
             agreement above is agreement about an empty tree"
        );
    }

    /// The DISK-READING three of the meter's four numbers.
    ///
    /// `parses` is dropped on purpose, and the reason is that the counters
    /// are PROCESS-wide. Sixteen tests in this binary build a `FontSystem`
    /// and every one of them parses a face, on whatever thread the harness
    /// runs it on, so a `parses` read here is partly somebody else's work.
    /// The other three cannot move under this test's feet: they only move
    /// when the index is BUILT, a build happens under the index's own
    /// mutex, and the only thing in the whole crate that discards a built
    /// index is this test (`fresh_index` is reached from the family lists,
    /// which nothing in this binary calls).
    ///
    /// Named for what it keeps rather than what it drops, because the
    /// question the test asks is "were the directories read again".
    fn scan_only(c: ScanCount) -> ScanCount {
        ScanCount { parses: 0, ..c }
    }

    /// The index is per DIRECTORY LIST, and only per directory list. A
    /// second question about the same list must not go back to the disk;
    /// a different list must.
    #[test]
    fn a_new_directory_list_is_the_one_thing_that_rebuilds() {
        let dirs = font_dirs();
        let first = font_index(&dirs);

        let before = scan_count();
        for _ in 0..20 {
            // Two assertions about the same call, and both are wanted. The
            // pointer says a rebuild did not happen at all — a build puts a
            // NEW allocation in the slot, so sameness here is not evidence
            // about a counter but about identity. The counter below says it
            // in the audit's own units.
            assert!(
                Arc::ptr_eq(&first, &font_index(&dirs)),
                "the same directory list was answered with a different \
                 index — the list is being read afresh per question"
            );
        }
        assert_eq!(
            scan_only(scan_count()), scan_only(before),
            "twenty questions about the same directory list read the disk \
             again — the index is keyed on something that moves"
        );

        // A list this process has not seen. It need not exist: what is
        // being measured is that the index NOTICED, not what it found.
        let mut other = dirs.clone();
        other.push(std::env::temp_dir().join("nacelle-no-such-font-dir"));
        let before = scan_count();
        let rebuilt = font_index(&other);
        // What the index is FOR the new list, not merely that a walk
        // happened somewhere: an index that kept the old list's files and
        // answered anyway is exactly the failure being guarded, and only
        // the key it carries can tell the two apart.
        assert_eq!(
            rebuilt.dirs.as_slice(), other.as_slice(),
            "a directory list this process had never seen was answered out \
             of the index built for another one — a font directory added at \
             runtime would stay invisible"
        );
        assert!(
            scan_count().walks > before.walks,
            "the index carries the new directory list but no traversal was \
             counted for it — the meter and the index disagree, and the \
             audit's numbers come from the meter"
        );

        // ...and the original list is a new list again now, which is the
        // honest cost of keeping one index rather than one per list. Left
        // built so the file is put back the way the rest of the suite
        // expects to find it.
        let _ = font_index(&dirs);
    }

    /// How many directories on `dirs` MOVE when the program is started
    /// somewhere else.
    ///
    /// Measured rather than asserted about: each entry is resolved
    /// against two different working directories and counted if the two
    /// resolutions differ, which is precisely what "depends on the
    /// working directory" means. `Path::join` with an absolute right
    /// side discards the left, so an absolute entry answers the same
    /// twice and a relative one does not.
    fn cwd_dependent(dirs: &[PathBuf]) -> Vec<PathBuf> {
        dirs.iter()
            .filter(|d| Path::new("/one").join(d) != Path::new("/two").join(d))
            .cloned()
            .collect()
    }

    /// WHAT THE INTERFACE IS DRAWN WITH MAY NOT DEPEND ON WHERE THE
    /// PROGRAM WAS STARTED FROM — COUNTED, AT ZERO.
    ///
    /// The first entry of this list used to be the bare name `fonts`,
    /// resolved against the working directory: a shell sitting in a
    /// folder that happened to have a `fonts/` subdirectory handed the
    /// desktop its typefaces, and every other launch got a failed
    /// openat instead — 94 of them in the 89-second run the audit of
    /// 2026-08-18 traced, one per consultation of this list.
    ///
    /// TWO separate things hold the count at zero, and this test fails
    /// if EITHER is undone. Restoring `"fonts"` at the head fails the
    /// first block; deleting the `retain` fails the second, which is
    /// the block that hands the builder a relative value from outside
    /// on purpose. (Restoring `"fonts"` alone, with the `retain` left
    /// in place, does NOT fail: the filter eats it, and the code before
    /// this change was both together.)
    #[test]
    fn no_font_directory_is_relative_to_wherever_the_program_was_started() {
        let dirs = font_dirs();
        assert_eq!(
            cwd_dependent(&dirs),
            Vec::<PathBuf>::new(),
            "directories that move with the working directory, in {dirs:?}"
        );
        // And the list is not empty by way of being clean: the two
        // system trees are on it whatever the environment says.
        assert!(dirs.contains(&PathBuf::from("/usr/share/fonts")), "{dirs:?}");
        assert!(dirs.contains(&PathBuf::from("/usr/local/share/fonts")), "{dirs:?}");

        // The two values that come from outside, both relative, both
        // refused. `NACELLE_FONT_DIR` is the deliberate door and is
        // held to the same rule as the accident it replaced — a
        // relative value there would be the old behaviour under a new
        // name — and a strange `HOME` may not smuggle one in either.
        let hostile = font_search_path(
            Some(OsString::from("fonts")),
            Some(OsString::from("relative/home")),
        );
        assert_eq!(
            cwd_dependent(&hostile),
            Vec::<PathBuf>::new(),
            "an outside value put a moving directory on the list: {hostile:?}"
        );
        assert_eq!(
            hostile,
            vec![
                PathBuf::from("/usr/share/fonts"),
                PathBuf::from("/usr/local/share/fonts"),
            ],
            "what survives a hostile environment is the system trees"
        );

        // And an absolute one is honoured, first: the door has to open.
        let named = font_search_path(Some(OsString::from("/opt/faces")), None);
        assert_eq!(named.first(), Some(&PathBuf::from("/opt/faces")));
    }

    /// WHAT ONE SWEEP OF THE FONT TREE COSTS, COUNTED.
    ///
    /// `find_font` walks every directory on this list for every pattern
    /// it is given, so the list's length is the number of directory
    /// roots opened per sweep and the number of MISSING ones is what a
    /// sweep pays for nothing. The bare `fonts` entry was missing on
    /// every machine that had not put files there by hand, which is how
    /// one accident became 94 failed openat calls in one session.
    ///
    /// Stated as an equality so the arithmetic is in the record: with
    /// `NACELLE_FONT_DIR` unset and a `HOME` set, four roots, of which
    /// zero are relative — where it was five, of which one was.
    #[test]
    fn the_price_of_a_sweep_is_the_length_of_the_font_search_path() {
        let dirs = font_search_path(None, Some(OsString::from("/x/home")));
        assert_eq!(
            dirs,
            vec![
                PathBuf::from("/x/home/.local/share/fonts"),
                PathBuf::from("/x/home/.fonts"),
                PathBuf::from("/usr/share/fonts"),
                PathBuf::from("/usr/local/share/fonts"),
            ]
        );
        assert_eq!(dirs.len(), 4);
        assert_eq!(cwd_dependent(&dirs).len(), 0);
        // A machine with no HOME at all is two, and still not one that
        // asks the working directory anything.
        let bare = font_search_path(None, None);
        assert_eq!(bare.len(), 2, "{bare:?}");
        assert_eq!(cwd_dependent(&bare).len(), 0, "{bare:?}");
    }
}
