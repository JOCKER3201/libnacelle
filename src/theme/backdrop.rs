//! The wallpaper half of `[backdrop]`: `source = image` (`default.theme`,
//! `[backdrop]`). Companion to [`super::plate`]'s decoration layers — same
//! [`Plate`] struct, same "bake once, upload as a texture, draw one quad"
//! contract — but a DIFFERENT switch: `decor.*` bakes regardless of
//! `backdrop.source`, while this file draws NOTHING unless the theme's own
//! word for its background is `image`.
//!
//! # What the token names promise, and what this file keeps of it
//!
//! * `backdrop.source` — the gate. Anything but `image` and
//!   [`bake_wallpaper`] returns `None`, the same "nothing enabled, nothing
//!   drawn" contract every other bake in this crate keeps.
//! * `backdrop.image` — a TEXT token (§3: "the bytes ARE the value"), so it
//!   lives in [`super::ThemeDiagnostics::texts`] and not in the POD
//!   [`ResolvedTheme`]; read the cold way, by name, once per bake, exactly
//!   like `num.rs`'s decimal separator.
//! * `backdrop.fit` — `cover | contain | stretch | centre`, the same four
//!   words CSS `background-size` uses and the same meanings: `cover` scales
//!   to fill the rect and crops the overflow, `contain` scales to fit
//!   inside it and leaves the rest transparent, `stretch` ignores aspect
//!   ratio, `centre` does not scale at all.
//! * `backdrop.treat.*` — dim, saturation, blur, tint: a small grade baked
//!   into the plate's own pixels once, never a per-frame shader.
//!
//! # Where the file comes from
//!
//! "relative to the theme directory or absolute" is the token's own words.
//! The theme directory is [`super::diagnostics`]`().path`'s parent — which,
//! until `mod.rs`'s `FsThemes::open` started recording it (2026-08-23), was
//! only ever populated for an EXPLICIT `--theme /path/to/x.theme` load: a
//! theme picked by NAME, the ordinary desktop path, published no `path` at
//! all, and a wallpaper such a theme named relative to its own directory
//! had nothing to resolve against.
//!
//! # Failure is silent and total, on purpose
//!
//! A file that will not decode, an empty `image` token, a `source` that is
//! not `image` — every one of them is `None`, never a partial plate and
//! never a panic. The caller already owns the fallback: `deco::board_ground`
//! draws `backdrop.solid` FIRST and this plate over it, so `None` here just
//! means that bed stays visible, exactly as it did before this file existed.
//!
//! # Why the graphics work and the token reads are two different halves
//!
//! [`bake_wallpaper`] is a thin doorway: it reads the four `backdrop.*`
//! and `backdrop.treat.*` tokens off the ONE published [`super::resolved`]
//! theme (`backdrop.source`'s enum index can only be compared against a
//! word this process actually interned, which is a property of the live,
//! global schema — see `super::enum_index`) and hands plain values —
//! a [`Fit`] word, a [`Treat`] struct, a resolved file path — to
//! [`bake_from_path`], which does the decode, the fit and the grade and
//! knows nothing about tokens at all. Every test below drives
//! `bake_from_path` and the small raster functions under it directly, the
//! same split `plate.rs` draws between `gather()` (tokens) and
//! `bake_params()` (pixels).

use super::bake::ResolvedTheme;
use super::color::Color;
use super::plate::{blend_px, Plate};
use super::TokenId;
use image::imageops::FilterType;
use image::RgbaImage;
use std::path::{Path, PathBuf};

/// `backdrop.fit`'s four words, decoupled from the token that names them —
/// see the module header's last section.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Fit {
    Cover,
    Contain,
    Stretch,
    Centre,
}

/// The `treat.*` grade, already read from the theme: four plain numbers
/// (well, three and a colour) so a raster test can hand them to
/// [`apply_treat`] without a published theme in the loop. Order of
/// application is [`apply_treat`]'s, not this struct's.
#[derive(Clone, Copy)]
pub(crate) struct Treat {
    /// `treat.dim` · frac 0..1 · 0.0 = untouched.
    pub dim: f32,
    /// `treat.saturation` · f, 0..2 · 1.0 = untouched.
    pub saturation: f32,
    /// `treat.blur` · len (u), already baked to device px · 0.0 = untouched.
    pub blur: f32,
    /// `treat.tint` · colour or none (alpha 0 = none, the §5.0 sentinel
    /// every other "colour or none" token in this crate reads the same
    /// way).
    pub tint: Color,
}

/// Bake `[backdrop]`'s wallpaper for the CURRENT resolved theme at the
/// given surface size. `None` for every reason there is nothing to draw;
/// see the module header. Reads the theme once at entry, the same contract
/// [`super::plate::bake_backdrop`] states for the same reason: a theme
/// swap mid-bake cannot mix two designs.
pub fn bake_wallpaper(w: u32, h: u32) -> Option<Plate> {
    let t = super::resolved();
    if !source_is_image(t) {
        return None;
    }
    let diags = super::diagnostics();
    let raw = diags.text("backdrop.image").unwrap_or_default();
    let dir = diags.path.as_deref().and_then(Path::parent);
    let path = resolve_image_path(raw, dir)?;
    let treat = Treat {
        dim: t.px(tid("backdrop.treat.dim")),
        saturation: t.px(tid("backdrop.treat.saturation")),
        blur: t.px(tid("backdrop.treat.blur")),
        tint: t.color(tid("backdrop.treat.tint")),
    };
    bake_from_path(&path, w, h, fit_of(t), treat)
}

/// The decode-fit-grade pipeline alone, split from [`bake_wallpaper`] so a
/// test can drive it from a file on disk with no published theme at all.
/// `None` on a zero-sized surface, a file that will not open, or a decode
/// this build's formats cannot read (see `Cargo.toml`'s `image` entry for
/// which those are) — the theme's `backdrop.solid` bed is always the
/// fallback the caller already drew, so this file never needs a second one
/// of its own.
pub(crate) fn bake_from_path(path: &Path, w: u32, h: u32, fit: Fit, treat: Treat) -> Option<Plate> {
    if w == 0 || h == 0 {
        return None;
    }
    let t0 = std::time::Instant::now();
    let src = match image::open(path) {
        Ok(decoded) => decoded.into_rgba8(),
        Err(e) => {
            eprintln!(
                "nacelle: backdrop.image {} did not decode ({e}) — \
                 the theme's solid bed shows through instead",
                path.display()
            );
            return None;
        }
    };
    if src.width() == 0 || src.height() == 0 {
        return None;
    }
    let canvas = apply_treat(fit_image(&src, w, h, fit), treat);
    debug_assert_eq!((canvas.width(), canvas.height()), (w, h), "a fit_* left the canvas the wrong size");
    Some(Plate {
        w,
        h,
        rgba: canvas.into_raw(),
        bake_ms: t0.elapsed().as_secs_f32() * 1000.0,
    })
}

fn tid(name: &str) -> TokenId {
    super::id(name).unwrap_or(TokenId::MISSING)
}

/// `backdrop.source == image`, read the same way `plate::gather_vignette`
/// reads `decor.vignette.layer`: the word may never have been interned at
/// all (no theme this process has loaded ever wrote it), and an unmatched
/// lookup on either side has to read as "no", not as index 0 of whatever
/// word happened to land there first.
fn source_is_image(t: &ResolvedTheme) -> bool {
    let Some(id) = super::id("backdrop.source") else { return false };
    let Some(image) = super::enum_index(id, "image") else { return false };
    t.enum_of(id) == image
}

/// `backdrop.fit`, by word. The unmatched arm is `cover` — the master's
/// own default and the token's own "never letterboxes" — so a theme that
/// names a fit word this build has never seen degrades to a filled screen
/// rather than a blank one.
fn fit_of(t: &ResolvedTheme) -> Fit {
    let Some(id) = super::id("backdrop.fit") else { return Fit::Cover };
    let e = t.enum_of(id);
    let word = |name: &str| super::enum_index(id, name);
    if Some(e) == word("contain") {
        Fit::Contain
    } else if Some(e) == word("centre") {
        Fit::Centre
    } else if Some(e) == word("stretch") {
        Fit::Stretch
    } else {
        Fit::Cover
    }
}

/// `raw` exactly as the token declares it: `""` is the "no image" sentinel
/// and not a path; an absolute path is itself; a relative one resolves
/// against `dir` — the loaded theme's own directory — or resolves to
/// nothing at all when the running theme has none (the embedded master,
/// which is never a file on disk).
fn resolve_image_path(raw: &str, dir: Option<&Path>) -> Option<PathBuf> {
    if raw.is_empty() {
        return None;
    }
    let p = Path::new(raw);
    if p.is_absolute() {
        return Some(p.to_path_buf());
    }
    Some(dir?.join(p))
}

// ------------------------------------------------------------------ fit

/// Dispatch to the one CSS-`background-size` meaning `word` names.
fn fit_image(src: &RgbaImage, w: u32, h: u32, word: Fit) -> RgbaImage {
    match word {
        Fit::Cover => fit_cover(src, w, h),
        Fit::Contain => fit_contain(src, w, h),
        Fit::Stretch => image::imageops::resize(src, w.max(1), h.max(1), FilterType::Triangle),
        Fit::Centre => place_centred(src, w, h),
    }
}

/// `cover`: scale until the rect is entirely filled, crop the overflow
/// centred. `ceil`, not `round`, on the scaled size — a scale that rounds
/// DOWN can leave the scaled image a device pixel short of the canvas on
/// one axis, and `cover`'s whole promise (the token's own words: "cover
/// never letterboxes") is that it does not fall short.
fn fit_cover(src: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (iw, ih) = (src.width().max(1) as f32, src.height().max(1) as f32);
    let scale = (w as f32 / iw).max(h as f32 / ih);
    let sw = (iw * scale).ceil().max(w as f32) as u32;
    let sh = (ih * scale).ceil().max(h as f32) as u32;
    let scaled = image::imageops::resize(src, sw, sh, FilterType::Triangle);
    let cx = (sw - w) / 2;
    let cy = (sh - h) / 2;
    image::imageops::crop_imm(&scaled, cx, cy, w, h).to_image()
}

/// `contain`: scale until the whole image fits inside the rect with no
/// crop, centred on a transparent canvas. The bars a `contain` picture
/// asks for are `backdrop.solid` showing through the transparency —
/// `board_ground` draws that bed FIRST — not a colour this file invents.
fn fit_contain(src: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let (iw, ih) = (src.width().max(1) as f32, src.height().max(1) as f32);
    let scale = (w as f32 / iw).min(h as f32 / ih);
    let sw = ((iw * scale).round().max(1.0)) as u32;
    let sh = ((ih * scale).round().max(1.0)) as u32;
    let scaled = image::imageops::resize(src, sw, sh, FilterType::Triangle);
    place_centred(&scaled, w, h)
}

/// `centre`: no scaling at all — `src` at its own size, centred on a
/// transparent `w x h` canvas, cropped by the rect if it is bigger and
/// bordered by it (transparent, the same as `contain`'s bars) if smaller.
///
/// One function for both `centre` and `contain`'s placement step, because
/// `image::imageops::overlay` already clips on every side at once —
/// negative offsets AND an oversized source both — which is exactly the
/// pair of cases those two words need and hand-rolled offset arithmetic
/// would have to special-case separately.
fn place_centred(src: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    let mut canvas = RgbaImage::new(w, h);
    let x = (w as i64 - src.width() as i64) / 2;
    let y = (h as i64 - src.height() as i64) / 2;
    image::imageops::overlay(&mut canvas, src, x, y);
    canvas
}

// ---------------------------------------------------------------- treat

/// The `treat.*` grade, in the master's own order: blur first (a spatial
/// pass, so it runs on the picture BEFORE colour work rather than
/// smearing an already-tinted edge), then dim and saturation together in
/// one pixel pass, then the tint wash last — a flat colour laid over the
/// FINISHED picture rather than something the blur or the saturation
/// change would touch.
fn apply_treat(mut canvas: RgbaImage, treat: Treat) -> RgbaImage {
    if treat.blur > 0.0 {
        canvas = image::imageops::blur(&canvas, treat.blur);
    }
    let dim = treat.dim.clamp(0.0, 1.0);
    let sat = treat.saturation.clamp(0.0, 2.0);
    if dim > 0.0 || (sat - 1.0).abs() > f32::EPSILON {
        grade(&mut canvas, dim, sat);
    }
    if treat.tint.a > 0.0 {
        wash(&mut canvas, treat.tint);
    }
    canvas
}

/// Dim toward black and desaturate toward luma, per pixel, alpha
/// untouched. The luma weights are `Color::luminance`'s own (`color.rs`)
/// — one perceptual formula for the whole engine, kept in step with it
/// rather than a second set invented here for a photo instead of a UI
/// colour.
fn grade(canvas: &mut RgbaImage, dim: f32, sat: f32) {
    let keep = 1.0 - dim;
    for px in canvas.pixels_mut() {
        let (r, g, b) = (px[0] as f32 / 255.0, px[1] as f32 / 255.0, px[2] as f32 / 255.0);
        let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
        let c = |v: f32| ((luma + (v - luma) * sat) * keep * 255.0).round().clamp(0.0, 255.0) as u8;
        px[0] = c(r);
        px[1] = c(g);
        px[2] = c(b);
    }
}

/// `treat.tint` straight-alpha OVER every pixel — [`blend_px`], shared
/// with `plate.rs`'s vignette and grain layers rather than copied.
fn wash(canvas: &mut RgbaImage, tint: Color) {
    let (w, h) = (canvas.width() as usize, canvas.height() as usize);
    let rgba: &mut [u8] = canvas;
    for i in 0..(w * h) {
        blend_px(rgba, i, tint.r, tint.g, tint.b, tint.a);
    }
}

// --------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat colour, opaque, for a source image simple enough that a
    /// test can state exactly what every output pixel should be.
    fn flat(w: u32, h: u32, px: [u8; 4]) -> RgbaImage {
        RgbaImage::from_fn(w, h, |_, _| image::Rgba(px))
    }

    fn no_treat() -> Treat {
        Treat { dim: 0.0, saturation: 1.0, blur: 0.0, tint: Color { r: 0.0, g: 0.0, b: 0.0, a: 0.0 } }
    }

    /// `resolve_image_path`: an absolute path is itself, whatever `dir`
    /// says; a relative one joins `dir`; and `""`, the token's own "no
    /// image" sentinel, resolves to nothing at all — never to `dir`
    /// itself, which a naive `dir.join("")` would silently produce.
    #[test]
    fn a_relative_path_joins_the_theme_directory_and_an_absolute_one_ignores_it() {
        let dir = Path::new("/themes/aurora");
        assert_eq!(
            resolve_image_path("wall.png", Some(dir)),
            Some(PathBuf::from("/themes/aurora/wall.png"))
        );
        assert_eq!(
            resolve_image_path("/opt/wallpapers/nebula.jpg", Some(dir)),
            Some(PathBuf::from("/opt/wallpapers/nebula.jpg")),
            "an absolute path must not be re-based under the theme directory"
        );
        assert_eq!(
            resolve_image_path("/opt/wallpapers/nebula.jpg", None),
            Some(PathBuf::from("/opt/wallpapers/nebula.jpg")),
            "an absolute path needs no theme directory at all"
        );
        assert_eq!(resolve_image_path("", Some(dir)), None, "the empty sentinel must resolve to nothing");
        assert_eq!(
            resolve_image_path("wall.png", None),
            None,
            "a relative path with no theme directory has nothing to resolve against"
        );
    }

    /// `cover`: the whole rect is opaque, whatever the aspect ratio
    /// mismatch — the token's own "cover never letterboxes" — and the
    /// centre pixel is still the source's colour, not the edge it cropped.
    #[test]
    fn cover_fills_every_pixel_and_never_letterboxes() {
        let src = flat(4, 40, [10, 200, 30, 255]); // tall and thin
        let out = fit_cover(&src, 64, 32); // short and wide
        assert_eq!((out.width(), out.height()), (64, 32));
        for px in out.pixels() {
            assert_eq!(px[3], 255, "cover left a transparent pixel: {px:?}");
            assert_eq!([px[0], px[1], px[2]], [10, 200, 30]);
        }
    }

    /// `contain`: the whole image survives with no crop, and the bars a
    /// mismatched aspect ratio forces are TRANSPARENT — `backdrop.solid`'s
    /// job, per the module header, not a colour this file paints.
    #[test]
    fn contain_letterboxes_with_transparency_not_a_colour() {
        let src = flat(40, 4, [200, 10, 30, 255]); // short and wide
        let out = fit_contain(&src, 32, 32); // square
        assert_eq!((out.width(), out.height()), (32, 32));
        let centre = out.get_pixel(16, 16);
        assert_eq!([centre[0], centre[1], centre[2], centre[3]], [200, 10, 30, 255]);
        let corner = out.get_pixel(0, 0);
        assert_eq!(corner[3], 0, "contain painted its own bar colour instead of leaving it transparent");
    }

    /// `centre`: a bigger source is cropped around its own middle rather
    /// than rescaled — the pixel this test samples is the source's exact
    /// centre, which only holds if nothing along the way resized it.
    #[test]
    fn centre_crops_a_larger_image_around_its_own_middle() {
        let mut src = flat(10, 10, [0, 0, 0, 255]);
        *src.get_pixel_mut(5, 5) = image::Rgba([255, 255, 255, 255]); // the source's own centre
        let out = place_centred(&src, 4, 4);
        assert_eq!((out.width(), out.height()), (4, 4));
        // A 4x4 window centred on a 10x10 image's centre lands on (5,5).
        assert_eq!(out.get_pixel(2, 2).0, [255, 255, 255, 255]);
    }

    /// `centre`: a smaller source sits in the middle of a transparent
    /// canvas rather than being stretched to fill it.
    #[test]
    fn centre_pads_a_smaller_image_with_transparency() {
        let src = flat(2, 2, [1, 2, 3, 255]);
        let out = place_centred(&src, 6, 6);
        assert_eq!(out.get_pixel(0, 0).0[3], 0, "a smaller centred image must not fill the canvas");
        assert_eq!(out.get_pixel(2, 2).0, [1, 2, 3, 255], "the source did not land in the middle");
    }

    /// `stretch` alone ignores aspect ratio: a 1x1 source becomes a solid
    /// rect of its own colour at exactly the requested size, whatever
    /// shape that is.
    #[test]
    fn stretch_fills_the_exact_size_ignoring_aspect_ratio() {
        let src = flat(1, 1, [9, 8, 7, 255]);
        let out = fit_image(&src, 50, 5, Fit::Stretch);
        assert_eq!((out.width(), out.height()), (50, 5));
        assert!(out.pixels().all(|p| [p[0], p[1], p[2], p[3]] == [9, 8, 7, 255]));
    }

    /// `treat.dim = 0` and `treat.saturation = 1` — the master's own
    /// values — must leave every byte exactly as the fit stage produced
    /// it: the "untouched" a token's own comment promises for 0.0/1.0
    /// has to be BIT untouched, not merely close.
    #[test]
    fn dim_zero_and_saturation_one_is_a_byte_for_byte_no_op() {
        let src = flat(3, 3, [12, 200, 90, 255]);
        let before = src.clone().into_raw();
        let graded = apply_treat(src, no_treat());
        assert_eq!(graded.into_raw(), before);
    }

    /// `treat.dim = 1.0` takes rgb to black and leaves alpha alone — dim
    /// darkens TOWARD black, never fades the plate out.
    #[test]
    fn dim_at_one_is_black_with_alpha_untouched() {
        let src = flat(2, 2, [255, 128, 64, 200]);
        let graded = apply_treat(src, Treat { dim: 1.0, ..no_treat() });
        for px in graded.pixels() {
            assert_eq!([px[0], px[1], px[2]], [0, 0, 0], "{px:?}");
            assert_eq!(px[3], 200, "dim touched alpha: {px:?}");
        }
    }

    /// `treat.saturation = 0` desaturates every pixel to its own luma —
    /// r, g and b converge on one grey, not on zero.
    #[test]
    fn saturation_zero_converges_every_channel_on_the_pixels_own_luma() {
        let src = flat(1, 1, [255, 0, 0, 255]); // pure red
        let graded = apply_treat(src, Treat { saturation: 0.0, ..no_treat() });
        let px = graded.get_pixel(0, 0);
        assert_eq!(px[0], px[1], "{px:?}");
        assert_eq!(px[1], px[2], "{px:?}");
        // Rec. 709 luma of pure red (0.2126) rounds to 54, not to 0: a
        // desaturated red is a dim grey, not black.
        assert_eq!(px[0], 54, "{px:?}");
    }

    /// `treat.tint`'s wash is a straight-alpha OVER, the exact formula
    /// `blend_px` states — checked here so a future change to either
    /// this file's call or `plate.rs`'s shared function is caught by
    /// whichever one runs the test.
    #[test]
    fn tint_washes_by_straight_alpha_over_the_finished_picture() {
        let src = flat(1, 1, [0, 0, 0, 255]);
        let tint = Color { r: 1.0, g: 1.0, b: 1.0, a: 0.5 };
        let graded = apply_treat(src, Treat { tint, ..no_treat() });
        let px = graded.get_pixel(0, 0);
        // white(1.0) * 0.5 + black(0.0) * 0.5 = mid grey, straight alpha.
        assert!((px[0] as i32 - 128).abs() <= 1, "{px:?}");
        assert_eq!(px[3], 255, "an opaque base under a partial wash must stay opaque");
    }

    /// A tint of alpha 0 — the `none` sentinel's own alpha — is not just
    /// a wash with nothing to see: `apply_treat` must skip the pass
    /// entirely rather than run `blend_px` at `a = 0.0` on every pixel.
    #[test]
    fn a_tint_of_none_never_touches_a_pixel() {
        let src = flat(2, 2, [11, 22, 33, 44]);
        let before = src.clone().into_raw();
        let graded = apply_treat(src, no_treat());
        assert_eq!(graded.into_raw(), before);
    }

    /// End to end, off a real file: a tiny PNG this test writes and
    /// decodes back, through `bake_from_path`'s whole pipeline including
    /// the "did it decode" branch. `w * h * 4` bytes out, and the plate's
    /// own colour where the picture's decoded pixel says it should be.
    #[test]
    fn bake_from_path_decodes_fits_and_grades_a_real_file() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("nacelle-backdrop-fixture-{}.png", std::process::id()));
        flat(8, 8, [40, 60, 80, 255]).save(&path).expect("the fixture png must encode");
        let plate = bake_from_path(&path, 16, 16, Fit::Stretch, no_treat());
        let _ = std::fs::remove_file(&path);
        let plate = plate.expect("a decodable file must bake a plate");
        assert_eq!((plate.w, plate.h), (16, 16));
        assert_eq!(plate.rgba.len(), 16 * 16 * 4);
        assert_eq!(&plate.rgba[0..4], &[40, 60, 80, 255]);
    }

    /// A path that does not exist is exactly the "nothing to draw" case
    /// every other bake in this crate answers with `None` — never a
    /// panic, and the message goes to stderr, not to a return value the
    /// caller has to unwrap around.
    #[test]
    fn a_missing_file_bakes_nothing() {
        let path = Path::new("/no/such/file/nacelle-backdrop-does-not-exist.png");
        assert!(bake_from_path(path, 16, 16, Fit::Cover, no_treat()).is_none());
    }

    /// A zero-sized surface bakes nothing, before the file is even
    /// opened — the same guard `plate::bake_backdrop` states for the
    /// same reason: there is no rect to fit a picture into.
    #[test]
    fn a_zero_sized_surface_bakes_nothing() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("nacelle-backdrop-zero-{}.png", std::process::id()));
        flat(2, 2, [1, 1, 1, 255]).save(&path).expect("the fixture png must encode");
        let a = bake_from_path(&path, 0, 16, Fit::Cover, no_treat());
        let b = bake_from_path(&path, 16, 0, Fit::Cover, no_treat());
        let _ = std::fs::remove_file(&path);
        assert!(a.is_none() && b.is_none());
    }
}
