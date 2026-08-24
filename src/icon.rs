//! SVG icons, rasterized once into a COVERAGE MASK — not a signed-distance
//! field per icon (K8).
//!
//! [`sdf`](crate::sdf) already carries the one distance field this crate
//! maintains, and it is the vector CORE's: box, arc, hexagon, chevron —
//! shapes the shader itself parametrises, cheap to evaluate exactly at
//! every pixel because there are only a handful of them and every panel
//! on screen is one. An icon is the opposite case — hundreds of distinct,
//! arbitrary outlines, each used at a handful of fixed pixel sizes and
//! drawn many times over — and that is exactly the shape the GLYPH atlas
//! was already built for. A font does not carry a distance field per
//! glyph either; it rasterizes one coverage bitmap per (face, size,
//! character) and samples it. This module does the same thing for one
//! more source of outlines: an SVG file stands in for the character, and
//! [`FontSystem::icon`](crate::font::FontSystem::icon) is
//! [`FontSystem::glyph`](crate::font::FontSystem::glyph)'s twin — same
//! atlas, same shelf packer, same cache-by-size, same R8 texel meaning
//! (0 = nothing there, 255 = fully covered). Maintaining a per-icon SDF
//! beside that would be a second technique answering a question the
//! first one already answers, at several times the authoring cost (an
//! SDF needs the outline's true distance function or a generated
//! approximation; a coverage mask needs a rasterizer, which an SVG
//! renderer already is).
//!
//! # What "coverage" means here
//!
//! The byte this module produces is the shape's ANTIALIASED COVERAGE at
//! that texel — how much of the pixel the icon's ink touches, the same
//! quantity `fontdue::Font::rasterize` already hands [`FontSystem::glyph`]
//! for a glyph outline. It carries no colour: an icon is drawn tinted by
//! the caller's own ink colour at draw time, exactly as a glyph is,
//! which is what lets one grey wrench icon become the hover, press and
//! disabled colours of a control without touching a texel.
//!
//! That equivalence rests on one assumption about the SOURCE svg: its
//! shapes are opaque (fill-opacity 1, no partial-alpha fill, no
//! gradient) so that the rendered alpha channel — what [`rasterize_to_mask`]
//! keeps and RGB it discards — equals shape coverage and nothing else.
//! Every icon set this project is likely to ship (flat, single-colour
//! outlines) already satisfies it, which is why the mask is read
//! straight off `resvg`'s render rather than re-derived by forcing every
//! paint to solid black first. An icon authored with genuine partial
//! opacity would bake a DIMMER mask than its outline, not a wrong one —
//! stated as the one place a follow-up normalisation pass would matter,
//! not silently.
//!
//! # The crate, and why this version
//!
//! `resvg` 0.45.1, `default-features = false` — see the dependency's own
//! comment in `Cargo.toml` for the full licence audit (the short version:
//! `resvg`/`usvg` changed OWNER and LICENCE between 0.44 [MPL-2.0,
//! `RazrFalcon/resvg`] and 0.45 [`MIT OR Apache-2.0`,
//! `linebender/resvg`] — pinning by name without checking the exact
//! version's own metadata would have linked MPL-2.0 code into an
//! MIT-only project). `usvg` (the parser) and `tiny-skia` (the
//! rasterizer) arrive as `resvg`'s own re-exports — one dependency
//! entry, not three — and neither the `text` nor `raster-images`
//! features are asked for: an icon is a flat vector shape with no
//! embedded font or photo, so pulling `fontdb`/`rustybuzz` for SVG
//! `<text>` support would be dependency weight for a case this project's
//! icons do not use.

use std::sync::Arc;

// `usvg` (parsing) and `tiny_skia` (rasterizing) are not dependencies of
// this crate in their own right — they arrive as `resvg`'s own
// re-exports (`pub use usvg; pub use tiny_skia;`), which is the whole
// reason `Cargo.toml` names only `resvg`: one dependency entry to audit
// and pin instead of three that would drift out of lock-step with it.
use resvg::{tiny_skia, usvg};

/// Why an SVG could not become a usable icon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconError {
    /// `usvg` rejected the bytes — malformed XML, an unsupported SVG
    /// construct, or not SVG at all. Carries `usvg`'s own message,
    /// which already names the line.
    Parse(String),
    /// The SVG parsed to a zero-area viewBox, or the caller asked to
    /// rasterize it at 0px. Either way there is no box to allocate in
    /// the atlas and nothing a shelf packer could place.
    ZeroSize,
}

impl std::fmt::Display for IconError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            IconError::Parse(msg) => write!(f, "svg icon: {msg}"),
            IconError::ZeroSize => write!(f, "svg icon: zero-size viewBox or request"),
        }
    }
}

impl std::error::Error for IconError {}

/// One icon's parsed SVG, held ready to rasterize at whatever pixel
/// sizes the interface asks for over the life of the process.
///
/// Parsing is the expensive half of turning a `.svg` file into pixels —
/// walking the XML, resolving `use`/`style`/viewBox — and it happens
/// ONCE here, in [`IconSource::parse`], no matter how many sizes or how
/// many frames the icon is drawn at afterwards. This is [`Font`]'s own
/// division of labour (`Font::from_bytes` parses the outline tables
/// once; `Font::rasterize` is called per glyph per size) carried over to
/// one more outline source, and the reason [`FontSystem`] keeps a
/// registry of these rather than re-parsing on every
/// [`FontSystem::icon`] call.
///
/// [`Font`]: fontdue::Font
/// [`FontSystem`]: crate::font::FontSystem
pub struct IconSource {
    tree: usvg::Tree,
}

impl IconSource {
    /// Parses `svg` (a complete `.svg` document's bytes) into a source
    /// ready to rasterize.
    ///
    /// `usvg::Options::default()` is used throughout rather than one
    /// this module builds up: every field it carries with the `text`
    /// feature off is about resolving relative hrefs, DPI and default
    /// sizing for a `<svg>` with no `width`/`height` — none of it is a
    /// decision an ICON needs to make differently from `usvg`'s own
    /// idea of "no opinion", and a caller with an unusual document
    /// (external stylesheets, a `resources_dir` for relative image
    /// hrefs) is free to build one and use `usvg::Tree::from_data`
    /// directly instead of this wrapper.
    pub fn parse(svg: &[u8]) -> Result<Self, IconError> {
        let opt = usvg::Options::default();
        let tree = usvg::Tree::from_data(svg, &opt).map_err(|e| IconError::Parse(e.to_string()))?;
        let size = tree.size();
        if size.width() <= 0.0 || size.height() <= 0.0 {
            return Err(IconError::ZeroSize);
        }
        Ok(Self { tree })
    }
}

/// [`IconSource`], shared: [`FontSystem`](crate::font::FontSystem) keeps
/// one registry entry per icon id and hands a rasterize call a
/// reference without cloning the parsed tree — `usvg::Tree` owns its
/// whole document graph, and an icon drawn at three sizes on screen at
/// once (a toolbar, a tile, a menu row) should not carry three copies
/// of it.
pub type SharedIconSource = Arc<IconSource>;

/// Rasterizes `src` into a square coverage mask `px` texels on a side,
/// row-major, one byte per texel: 0 = nothing there, 255 = fully
/// covered, and every value between is antialiased edge coverage — the
/// same scale [`fontdue`] hands a glyph bitmap in.
///
/// The SVG's own viewBox is scaled to fill the `px x px` box on BOTH
/// axes independently (`sx`, `sy` below), which is deliberately the
/// same behaviour an icon FONT gives a caller that
/// draws every glyph in a square cell — an icon authored on a
/// non-square viewBox (rare; every icon set this project is likely to
/// use ships square artwork) is stretched to fit rather than centred
/// and padded, so the caller never has to reason about letterboxing
/// inside the cell it asked for.
///
/// Errors: [`IconError::ZeroSize`] for `px == 0`, or
/// [`IconError::ZeroSize`] again if `tiny_skia::Pixmap::new` refuses the
/// size (it also rejects zero, and nothing above `i32::MAX / 4` reaches
/// an icon glyph anyway).
pub fn rasterize_to_mask(src: &IconSource, px: u32) -> Result<Vec<u8>, IconError> {
    if px == 0 {
        return Err(IconError::ZeroSize);
    }
    let mut pixmap = tiny_skia::Pixmap::new(px, px).ok_or(IconError::ZeroSize)?;
    let size = src.tree.size();
    let sx = px as f32 / size.width();
    let sy = px as f32 / size.height();
    let transform = tiny_skia::Transform::from_scale(sx, sy);
    resvg::render(&src.tree, transform, &mut pixmap.as_mut());
    // `Pixmap::data()` is premultiplied RGBA8, byte order R,G,B,A. The
    // mask is the ALPHA byte alone — see this module's doc comment for
    // why that already equals coverage for the opaque, flat-fill icons
    // this project ships, with no colour or premultiplication folded
    // in, exactly like the greyscale byte `fontdue` hands a glyph.
    let data = pixmap.data();
    let n = (px * px) as usize;
    let mut mask = vec![0u8; n];
    for i in 0..n {
        mask[i] = data[i * 4 + 3];
    }
    Ok(mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A filled circle, the whole viewBox, fill defaulting to SVG's own
    /// black — chosen because a circle's coverage at its CENTRE and at
    /// its CORNER cannot agree by accident: any bug that rasterizes a
    /// bounding box instead of the shape, or leaves the mask blank, or
    /// inverts it, fails at least one of the two assertions below. Kept
    /// inline rather than only as a fixture file so the test states
    /// exactly what shape it expects a byte to answer for.
    const CIRCLE_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
  <circle cx="12" cy="12" r="10"/>
</svg>"#;

    /// A shape touching every edge of its viewBox, fill again defaulting
    /// to black — the corner-vs-centre pair this shape offers is the
    /// opposite of the circle's: covered where the circle is EMPTY (the
    /// corner) is meaningless for a diamond, so this fixture instead
    /// proves coverage follows the OUTLINE (edges half-covered by an
    /// antialiased slope) rather than a bounding box (which would read
    /// solid at the same texels).
    const DIAMOND_SVG: &str = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
  <polygon points="12,1 23,12 12,23 1,12"/>
</svg>"#;

    #[test]
    fn a_filled_circle_covers_its_centre_and_not_its_corner() {
        let src = IconSource::parse(CIRCLE_SVG.as_bytes()).unwrap();
        let px = 32u32;
        let mask = rasterize_to_mask(&src, px).unwrap();
        let at = |x: u32, y: u32| mask[(y * px + x) as usize];
        let centre = at(px / 2, px / 2);
        let corner = at(1, 1);
        assert!(
            centre > 250,
            "the circle's own centre should read almost fully covered, got {centre}"
        );
        assert_eq!(
            corner, 0,
            "a corner outside a circle inscribed in its viewBox must read \
             uncovered — {corner} means either the mask is a bounding box \
             or the alpha channel was not read where this test expects it"
        );
    }

    #[test]
    fn zero_pixels_is_refused_not_a_panic() {
        let src = IconSource::parse(CIRCLE_SVG.as_bytes()).unwrap();
        assert_eq!(rasterize_to_mask(&src, 0), Err(IconError::ZeroSize));
    }

    #[test]
    fn malformed_svg_is_a_parse_error() {
        match IconSource::parse(b"not an svg at all") {
            Err(IconError::Parse(_)) => {}
            Err(IconError::ZeroSize) => panic!("expected IconError::Parse, got ZeroSize"),
            Ok(_) => panic!("expected IconError::Parse, got a parsed tree"),
        }
    }

    #[test]
    fn a_diamond_is_covered_on_its_diagonal_and_empty_off_it() {
        let src = IconSource::parse(DIAMOND_SVG.as_bytes()).unwrap();
        let px = 32u32;
        let mask = rasterize_to_mask(&src, px).unwrap();
        let at = |x: u32, y: u32| mask[(y * px + x) as usize];
        // The diamond's own centre sits on every one of its diagonals —
        // covered. Its own corner (viewBox corner, not the diamond's
        // point) sits outside all four edges — empty. A rasterizer that
        // filled the bounding BOX instead of the polygon would answer
        // this pair identically wrong (both covered); one that never
        // rendered anything answers it identically wrong the other way
        // (both empty) — only the true outline answers one of each.
        let centre = at(px / 2, px / 2);
        let corner = at(1, 1);
        assert!(centre > 250, "the diamond's centre should read covered, got {centre}");
        assert_eq!(corner, 0, "the viewBox corner sits outside the diamond, got {corner}");
    }

    #[test]
    fn two_sizes_of_the_same_icon_scale_the_mask_not_just_the_box() {
        let src = IconSource::parse(CIRCLE_SVG.as_bytes()).unwrap();
        let small = rasterize_to_mask(&src, 8).unwrap();
        let large = rasterize_to_mask(&src, 64).unwrap();
        assert_eq!(small.len(), 64);
        assert_eq!(large.len(), 64 * 64);
        // Both sizes cover their own centre — the scale changed the
        // BOX, and the render followed it, rather than the mask always
        // coming back at one fixed resolution regardless of what was
        // asked for.
        assert!(small[4 * 8 + 4] > 200);
        assert!(large[32 * 64 + 32] > 200);
    }
}
