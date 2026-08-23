//! `motion.glow_pulse` — the breathing halo, measured in the vertices it
//! actually produces.
//!
//! §5.22 declares the effect and describes its `amplitude` as "± swing
//! applied to glow_alpha", and until now nothing in the program read
//! either key. This binary is that reader's proof, and the condition on
//! the whole stone: the picture must not move unless a theme asked TWICE
//! — once for `glow.panel_edge.enabled` and once for
//! `motion.glow_pulse.enabled`. Until 2026-08-23 the master shipped both
//! false; the first now ships true (the neon-by-default change), so the
//! "no glow at all" leg of the proof below asks for it explicitly, the
//! same way "glow, no swing" already had to.
//!
//! Everything is measured on `object::window::frame`, which is the
//! shortest path to `panel_edge_glow`, and everything is compared as
//! VERTICES rather than as numbers off the resolver: the question this
//! file answers is not "does the multiplier swing" (that is
//! `motion_effects.rs`) but "does the frame a host draws change, and only
//! when it should".
//!
//! Time is a parameter — `Ctx.t` — so every clock below is a literal.
//! Nothing sleeps.
//!
//! One test in a binary of its own: the resolved theme is process-wide.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::object::window;
use nacelle::pointer::Pointer;
use nacelle::{theme, Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const BOX: Rect = Rect { x: 200.0, y: 120.0, w: 640.0, h: 360.0 };

/// A halo distinct from the master's own resting one (4u at 0.34,
/// 2026-08-23) in both numbers, so a comparison against it is a
/// comparison against a fixture's OWN values and not an accident of
/// matching the master's by construction.
const HALO: &str = "[glow]\npanel_edge.enabled = true\npanel_edge.radius = 2u\n\
                    panel_edge.alpha = 0.5\n";

/// No halo at all. Until 2026-08-23 this was the master's own resting
/// state and `master()` alone drew it; the master now ships `panel_edge`
/// lit, so silence has to be asked for the same way a swing does.
const NO_GLOW: &str = "[glow]\npanel_edge.enabled = false\n";

fn skin(body: &str) {
    let path = std::env::temp_dir().join(format!("nacelle-glow-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        format!("[meta]\nschema = 1\nname = \"Fixture\"\nbase = \"default\"\n\n{body}"),
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(theme::LoadRequest { path: Some(path.clone()), ..Default::default() });
    let _ = std::fs::remove_file(&path);
    theme::set_viewport(H, 1.0);
}

fn master() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
}

/// One window frame, drawn at `now`, as the vertex buffer a renderer
/// would be handed.
fn frame_at(fonts: &mut FontSystem, now: f64) -> Vec<[f32; 8]> {
    let mut dl = DrawList::new();
    let mut ctx = Ctx {
        dl: &mut dl,
        fonts,
        w: W,
        h: H,
        t: now,
        mouse: Pointer::new(0.0, 0.0),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    };
    window::frame(&mut ctx, BOX);
    dl.verts
        .iter()
        .map(|v| {
            [
                v.pos[0],
                v.pos[1],
                v.uv[0],
                v.uv[1],
                v.color[0],
                v.color[1],
                v.color[2],
                v.color[3],
            ]
        })
        .collect()
}

// =====================================================================

#[test]
fn the_halo_breathes_only_when_a_theme_asks_twice() {
    let mut fonts = FontSystem::new();

    // ---- explicitly off: no halo at all, and the clock changes nothing.
    skin(NO_GLOW);
    let plain = frame_at(&mut fonts, 0.0);
    assert_eq!(plain, frame_at(&mut fonts, 0.4), "a disabled panel_edge moved with the clock");
    assert_eq!(plain, frame_at(&mut fonts, 123.456), "…and at any other clock");

    // ---- a halo, and `glow_pulse` left as the master ships it: OFF.
    skin(HALO);
    let still = frame_at(&mut fonts, 0.0);
    assert!(
        still.len() > plain.len(),
        "the fixture drew no halo — nothing below is measuring anything"
    );
    for t in [0.0, 0.2, 0.4, 0.8, 1.6, 99.0] {
        assert_eq!(still, frame_at(&mut fonts, t), "a disabled pulse moved the halo at t={t}");
    }

    // ---- the pulse on. The master's `glow_pulse` is 1600 ms of `sine`
    // at amplitude 0.25, and `sine` read cyclically is 0.5-0.5*cos(2*pi*p)
    // — so the multiplier is 1 at a QUARTER and at three quarters of the
    // period, its floor at the start of one and its ceiling in the middle.
    //
    // That is what makes the mean assertable exactly rather than
    // statistically: at t = 0.4 s and t = 1.2 s the halo must be the
    // frozen halo, vertex for vertex, and nowhere else.
    skin(&format!("{HALO}\n[motion.glow_pulse]\nenabled = true\n"));
    let frozen = frame_at(&mut fonts, 0.4);
    assert_eq!(frozen, still, "the mean of the swing is not the number the theme wrote");
    assert_eq!(frame_at(&mut fonts, 1.2), still, "…and it comes back a period later");
    for t in [0.0, 0.8, 1.6] {
        assert_ne!(frame_at(&mut fonts, t), still, "the pulse did not swing at t={t}");
    }
    // The swing is on the ALPHA and nothing else: same vertex count, same
    // positions, same atlas coordinates — only the colours moved.
    let dark = frame_at(&mut fonts, 0.0);
    assert_eq!(dark.len(), still.len(), "the pulse changed the halo's geometry");
    for (a, b) in dark.iter().zip(still.iter()) {
        assert_eq!(a[0..4], b[0..4], "a vertex moved, or took a different sprite");
    }

    // ---- reduced motion, both ways in. The picture is the frozen one,
    // bit for bit, at every clock — which is §5.22's rule for a cyclic
    // source that breathes: freeze at the MEAN, not at the peak.
    skin(&format!("{HALO}\n[motion.glow_pulse]\nenabled = true\n\n[motion]\nscale = 0.0\n"));
    for t in [0.0, 0.4, 0.8, 1.6] {
        assert_eq!(frame_at(&mut fonts, t), still, "motion.scale = 0 let the halo breathe");
    }
    skin(&format!("{HALO}\n[motion.glow_pulse]\nenabled = true\n\n[a11y]\nreduced_motion = on\n"));
    for t in [0.0, 0.4, 0.8, 1.6] {
        assert_eq!(frame_at(&mut fonts, t), still, "a11y.reduced_motion let the halo breathe");
    }

    // ---- an amplitude of zero is the master's own value, and it is the
    // shortest freeze of all: a swing of nothing multiplies by one.
    skin(&format!("{HALO}\n[motion.glow_pulse]\nenabled = true\namplitude = 0.0\n"));
    for t in [0.0, 0.4, 0.8] {
        assert_eq!(frame_at(&mut fonts, t), still, "a zero amplitude swung anyway");
    }

    master();
}
