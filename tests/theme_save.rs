//! The save round-trip, in its own process for the same reason every other
//! theme test file is: the engine is one global, and this one LOADS a theme.
//!
//! What is pinned here is the seam the whole editor stands on: a file
//! `save_theme` generates is found by the loader's walk, parses under the
//! master, and resolves to EXACTLY the values that were saved. The
//! generator splits a token into `[section]` and key at the first dot and
//! trusts the loader's `section.key` concatenation to reassemble it — this
//! test is where that trust is earned rather than assumed.

use nacelle::theme::{self, edit::Edit};

fn edit(token: &'static str, value: &str) -> Edit {
    Edit { token, value: value.to_string() }
}

#[test]
fn a_saved_theme_loads_back_with_the_values_it_was_saved_with() {
    // The save lands where NACELLE_THEME_DIR points... no — `save_theme`
    // writes to the USER dir on purpose (a save is the person's, wherever
    // the search override points this session). So the test steers the
    // user dir itself through XDG_DATA_HOME, into scratch.
    let scratch = std::env::temp_dir().join(format!("nacelle-theme-save-{}", std::process::id()));
    std::fs::create_dir_all(&scratch).unwrap();
    // SAFETY: this test runs in its own process (an integration file with
    // one test), so nothing races the environment.
    unsafe {
        std::env::set_var("XDG_DATA_HOME", &scratch);
        std::env::remove_var("NACELLE_THEME_DIR");
    }

    let edits = [
        // Every value SHAPE the editor produces, not merely every section:
        // a fractional scalar, a unit length, a plain zero, a bool, an
        // opaque oklch, and a fully transparent one (the wash's "none
        // without the word none"). The adversarial review caught the first
        // cut of this list covering half the shapes while claiming all.
        edit("elev.panel.glass.rank", "2.50"),
        edit("elev.panel.glass.tint", "oklch(0.7000, 0.1000, 200.00)"),
        edit("elev.panel.glass.wash", "oklch(0.0000, 0.0000, 0.00 / 0.000)"),
        edit("glow.focus_ring.enabled", "true"),
        edit("glow.focus_ring.radius", "1.6u"),
        edit("glow.focus_ring.alpha", "0.34"),
        edit("component.panel.fill", "oklch(0.3000, 0.0500, 120.00)"),
        // 2026-08-16, the whole-theme groups: one witness per group, and
        // the shapes the first list did not have — a bare enum word
        // (corner.mode), an ms duration (scrollbar.fade_ms), a signed
        // scalar (surface.lift) and a REFERENCE (surface.hue), which is
        // the load-bearing one: the editor's FOLLOW-THE-ACCENT is written
        // as a reference, and if the generator's section split mangled
        // one, the surfaces would silently stop following. `@hue.data` and
        // not the editor's own `@hue.accent`, because the master's DEFAULT
        // is `@hue.accent` — a dropped line would resolve to the same
        // number and the mangling would hide; the data hue is observably
        // elsewhere.
        //
        // Colour witnesses sit on hues where the working-space drift (the
        // same representation caveat as above) stays under ~2.5 deg —
        // measured 2026-08-16: amber h=80 comes back h=94, and h=150
        // exactly 154.0, the tolerance itself. Red 25, cyan 200, violet
        // 300/315 hold still.
        edit("palette.accent", "oklch(0.5500, 0.1000, 315.00)"),
        // The master ships `palette.data = @palette.accent`, which would
        // make the reference witness below indistinguishable from the
        // default it is meant to be told apart from — so the data seed is
        // pinned to its own hue first, the exact one line §5.9 promises a
        // theme it costs.
        edit("palette.data", "oklch(0.7000, 0.1200, 230.00)"),
        edit("surface.hue", "@hue.data"),
        edit("surface.lift", "-0.0500"),
        edit("text.chroma", "1.800"),
        edit("severity.warning.text", "oklch(0.7000, 0.1400, 25.00)"),
        edit("corner.mode", "chamfer"),
        edit("focus.ring.offset", "0.9u"),
        edit("component.menu.hint", "oklch(0.6500, 0.0800, 300.00)"),
        edit("scrollbar.fade_ms", "320ms"),
    ];
    let path = theme::save_theme("proba-edytora", &edits).expect("the save refused");
    assert!(path.is_file(), "saved and yet no file at {path:?}");

    theme::load_with(theme::LoadRequest {
        name: Some("proba-edytora".to_string()),
        ..Default::default()
    });

    let px = |n: &str| theme::id(n).map(|i| theme::resolved().px(i)).expect(n);
    let flag = |n: &str| theme::id(n).map(|i| theme::resolved().flag(i)).expect(n);
    let col = |n: &str| theme::id(n).map(|i| theme::resolved().color(i)).expect(n);

    assert!(
        (px("elev.panel.glass.rank") - 2.5).abs() < 0.01,
        "the fractional rank did not survive the trip: {}",
        px("elev.panel.glass.rank")
    );
    assert!(flag("glow.focus_ring.enabled"), "the flag did not survive the trip");
    assert!(
        px("glow.focus_ring.radius") > 1.0,
        "the unit length baked to nothing: {}",
        px("glow.focus_ring.radius")
    );
    assert!(
        (px("glow.focus_ring.alpha") - 0.34).abs() < 0.01,
        "the plain scalar drifted: {}",
        px("glow.focus_ring.alpha")
    );
    assert!(
        col("elev.panel.glass.wash").a < 0.01,
        "the transparent wash came back with coverage"
    );
    // Colours are asserted by HUE and by having left the master's white —
    // not by exact lightness. The bake stores colours in the renderer's
    // working space, so a byte-for-byte comparison would test the colour
    // pipeline's representation, not the save's fidelity; the scalar and
    // the flag above are the exact witnesses, the hues pin that each token
    // carries OUR colour and not another's.
    let c = col("elev.panel.glass.tint").to_oklch();
    assert!(
        (c.h - 200.0).abs() < 4.0 && c.c > 0.02,
        "the tint came back as another colour: l={} c={} h={}",
        c.l,
        c.c,
        c.h
    );
    let f = col("component.panel.fill").to_oklch();
    assert!(
        (f.h - 120.0).abs() < 4.0 && f.c > 0.01,
        "the shared fill came back as another colour: h={} c={}",
        f.h,
        f.c
    );

    // ---- the whole-theme groups (2026-08-16), one witness each ----------
    let a = col("palette.accent").to_oklch();
    assert!(
        (a.h - 315.0).abs() < 4.0 && a.c > 0.02,
        "the accent seed came back as another colour: h={} c={}",
        a.h,
        a.c
    );
    // The reference shape: surface.hue was saved as `@hue.data`, so it
    // must resolve to EXACTLY the data seed's hue — engine number against
    // engine number, no representation in between — and must have LEFT the
    // default `@hue.accent`, or a mangled reference would be indistinct
    // from a working one.
    assert!(
        (px("surface.hue") - px("hue.data")).abs() < 0.01,
        "the reference did not survive the trip: surface.hue={} hue.data={}",
        px("surface.hue"),
        px("hue.data")
    );
    assert!(
        (px("surface.hue") - px("hue.accent")).abs() > 5.0,
        "surface.hue still sits on the accent — the saved reference was \
         dropped and the default hid it: {}",
        px("surface.hue")
    );
    assert!(
        (px("surface.lift") + 0.05).abs() < 0.005,
        "the signed scalar drifted: {}",
        px("surface.lift")
    );
    assert!(
        (px("text.chroma") - 1.8).abs() < 0.01,
        "the ladder knob drifted: {}",
        px("text.chroma")
    );
    let w = col("severity.warning.text").to_oklch();
    assert!(
        (w.h - 25.0).abs() < 4.0 && w.c > 0.02,
        "the pinned severity author came back as another colour: h={} c={}",
        w.h,
        w.c
    );
    // The bare-word shape: an enum compares by interned index, never by a
    // remembered number, so the assertion asks the schema for `chamfer`'s
    // index in THIS load.
    let mode = theme::id("corner.mode").expect("corner.mode");
    assert_eq!(
        Some(theme::resolved().enum_of(mode)),
        theme::enum_index(mode, "chamfer"),
        "the corner language did not survive the trip"
    );
    assert!(
        px("focus.ring.offset") > 1.0,
        "the ring offset baked to nothing: {}",
        px("focus.ring.offset")
    );
    let hint = col("component.menu.hint").to_oklch();
    assert!(
        (hint.h - 300.0).abs() < 4.0 && hint.c > 0.02,
        "the menu hint came back as another colour: h={} c={}",
        hint.h,
        hint.c
    );
    // The duration shape: ms bakes to its own number, not to pixels.
    assert!(
        (px("scrollbar.fade_ms") - 320.0).abs() < 1.0,
        "the duration drifted: {}",
        px("scrollbar.fade_ms")
    );

    // `default` is not a file and may never become one.
    assert!(
        theme::save_theme("default", &edits).is_err(),
        "the master let itself be saved over"
    );
    assert!(
        theme::save_theme("../ucieczka", &edits).is_err(),
        "a path traversal dressed as a name was accepted"
    );

    // And the saved name shows up where the settings list looks.
    assert!(
        theme::available_themes().iter().any(|n| n == "proba-edytora"),
        "the saved theme is invisible to the list"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}
