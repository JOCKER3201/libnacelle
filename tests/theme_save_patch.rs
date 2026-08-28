//! THE SAVE PATCHES, IT DOES NOT REGENERATE — the owner's report of
//! 2026-08-17: "the halo does not blink any more, but it disappears when I
//! press save".
//!
//! In its own process for the reason every other theme test file is: the
//! engine is one global and this one LOADS a theme. One test, because a
//! second one in the same process would be looking at the first one's
//! engine.
//!
//! What is pinned here is the seam between `theme::edit`'s edit set and the
//! file it lands in. The set is an OVERLAY vocabulary — saying nothing about
//! a token means "leave it standing", which is exactly how `focus_ring_edits`
//! protects a theme that dressed its own ring halo. A save that generated the
//! file whole read that same silence as "delete it", and the author's halo
//! went out on the first SAVE. So this file asks the question the report
//! asks: after a save, is the theme still what it was, everywhere the editor
//! did not touch?
//!
//! (The dressed halo here was the panel edge's until the whole panel-edge
//! effect was removed on 2026-08-27 at the owner's order; the focus ring
//! wears the same silence contract, so the claim carries over unchanged.)

use nacelle::theme::{
    self,
    color::Oklch,
    edit::{border_colour_edit, focus_ring_edits, Edit, FocusRing, RingStyle, Scope},
};

/// A theme as a PERSON would have it: notes of their own, a ring halo dressed
/// with their numbers, a mood that re-declares one of the tokens the editor
/// writes, and a value that runs over two lines.
const AUTHORED: &str = r#"# Motyw wlasciciela. Ta notatka ma przezyc zapis.
[meta]
name = "Ubrany"

[glow.focus_ring]
enabled = false                       # the editor is about to turn this on
radius  = 2.40u                       # the author's own halo - not the editor's seed

[border]
default = oklch(0.5000, 0.0500, 200.00)   # a note that sits after a value

[palette]
accent = oklch(0.6000, 0.1000, 300.00)

# A value that runs over two lines. Patching it by its span's numbers would
# cut the file in half, so the save must leave it and append instead.
[motion.glow_pulse]
easing_p = [0.25, 0.10,
            0.25, 1.00]

[mood.alert]
glow.focus_ring.enabled = false       # the MOOD's own word, not the theme's
"#;

#[test]
fn a_dressed_theme_saved_by_the_editor_still_wears_its_dress() {
    let scratch =
        std::env::temp_dir().join(format!("nacelle-theme-patch-{}", std::process::id()));
    // The family folder. This test predates the search path learning it,
    // and a save lands where the program is heading. The old folder stays
    // readable — "read both, move nothing" — but nothing here needs it,
    // because this theme is created for the test rather than found.
    let dir = scratch.join("nacelle/themes");
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: one test in its own process, so nothing races the environment.
    unsafe {
        std::env::set_var("XDG_DATA_HOME", &scratch);
        std::env::remove_var("NACELLE_THEME_DIR");
        std::env::remove_var("NACELLE_THEME_LOCAL");
    }
    let path = dir.join("ubrany.theme");
    std::fs::write(&path, AUTHORED).unwrap();

    // The set the editor really produces for a theme that HAS dressed its
    // ring halo: `halo_dressed = true`, so the radius is not mentioned.
    // Hand-written edits here would test a hand-written claim; these come
    // from the model the window calls.
    let colour = Oklch { l: 0.7000, c: 0.1200, h: 40.00, alpha: 1.0 };
    let mut edits = vec![border_colour_edit(Scope::Theme, colour)];
    edits.extend(focus_ring_edits(
        Scope::Theme,
        true,
        &FocusRing {
            style: RingStyle::Solid,
            width_u: 0.20,
            offset_u: 0.30,
            colour,
            dash_u: 0.0,
            gap_u: 0.0,
            halo: true,
            halo_alpha: 0.30,
            halo_dressed: true,
        },
    ));
    // Two more, for the two shapes a patch has to refuse and to add: a value
    // that runs over lines, and a token this file simply does not carry.
    edits.push(Edit {
        token: "motion.glow_pulse.easing_p",
        value: "[0.30, 0.20, 0.40, 0.90]".to_string(),
    });
    edits.push(Edit { token: "corner.mode", value: "chamfer".to_string() });

    for token in ["glow.focus_ring.radius"] {
        assert!(
            !edits.iter().any(|e| e.token == token),
            "the editor mentioned {token} — this test can no longer say anything \
             about what SILENCE costs, and the model changed under it"
        );
    }

    // WHAT THE OWNER SEES, MEASURED BEFORE THE BUTTON. The report is a
    // comparison — "it disappears when I press save" — so the test has to be
    // one too. A threshold picked by hand would let the editor's own seed
    // through: `1.6u` and the author's `2.40u` are both "a radius", and the
    // bug is precisely a save that swaps one for the other.
    theme::load_with(theme::LoadRequest {
        name: Some("ubrany".to_string()),
        ..Default::default()
    });
    let px = |n: &str| theme::id(n).map(|i| theme::resolved().px(i)).expect(n);
    let flag = |n: &str| theme::id(n).map(|i| theme::resolved().flag(i)).expect(n);
    let radius_before = px("glow.focus_ring.radius");
    assert!(
        radius_before > 0.0,
        "the fixture theme is not dressed, so nothing below compares anything: \
         radius {radius_before}"
    );

    let saved = theme::save_theme("ubrany", &edits).expect("the save refused");
    assert_eq!(saved, path, "the save landed somewhere other than the theme's own file");
    let after = std::fs::read_to_string(&path).unwrap();

    // ---- 1. the dress the set never mentions is still there, verbatim ----
    assert!(
        after.contains("radius  = 2.40u"),
        "the author's halo radius did not survive the save:\n{after}"
    );

    // ---- 2. the author's own notes survive --------------------------------
    assert!(
        after.contains("# Motyw wlasciciela. Ta notatka ma przezyc zapis."),
        "the opening note was rewritten away:\n{after}"
    );
    assert!(
        after.contains("# the author's own halo - not the editor's seed"),
        "a note beside an untouched value was lost:\n{after}"
    );
    assert!(
        after.contains("# a note that sits after a value"),
        "the note after a value the editor DID rewrite was eaten by the new \
         value:\n{after}"
    );
    assert!(
        after.contains("name = \"Ubrany\""),
        "a token the editor has no control for was dropped:\n{after}"
    );

    // ---- 3. what the editor DID say landed, IN PLACE ------------------------
    // By the line, not by `contains`: a substring search would call any of
    // the file's other lines a pass. What is being asked is that the value
    // in THIS line changed and the rest of the line did not.
    let line_with = |starts: &str| -> String {
        after
            .lines()
            .find(|l| l.trim_start().starts_with(starts))
            .unwrap_or_else(|| panic!("the file lost its `{starts}` line:\n{after}"))
            .to_string()
    };
    let switch = line_with("enabled =");
    assert!(switch.contains("= true"), "the ring's switch never reached the file: {switch}");
    assert!(
        switch.contains("# the editor is about to turn this on"),
        "the value was rewritten and took the note beside it with it: {switch}"
    );
    // The colour lands on the shared root `border.default`, not the
    // `elev.panel` leaf — one edit that moves every frame, the settings
    // window and each widget alike (`edit::border_colour_edit`).
    let colour = line_with("default =");
    assert!(
        colour.contains("oklch(0.7000, 0.1200, 40.00)"),
        "the border colour the sliders set never reached the file: {colour}"
    );
    assert!(
        !colour.contains("oklch(0.5000, 0.0500, 200.00)"),
        "the old border colour is still standing beside the new one: {colour}"
    );

    // ---- 4. a MOOD is not the theme ---------------------------------------
    let mood = after
        .split("[mood.alert]")
        .nth(1)
        .expect("the mood section was dropped by the save");
    assert!(
        mood.contains("glow.focus_ring.enabled = false"),
        "the save wrote the theme's word into the mood's line:\n{after}"
    );

    // ---- 5. a value that runs over lines is left whole ---------------------
    assert!(
        after.contains("easing_p = [0.25, 0.10,\n            0.25, 1.00]"),
        "the two-line value was cut by a patch that trusted its span:\n{after}"
    );
    assert!(
        after.contains("glow_pulse.easing_p = [0.30, 0.20, 0.40, 0.90]"),
        "the value the save refused to patch in place was not appended \
         either, so the edit was lost:\n{after}"
    );

    // ---- 6. a token the file did not carry is appended ---------------------
    assert!(
        after.contains("mode = chamfer"),
        "a token the file did not have was not added:\n{after}"
    );

    // ---- 7. and the whole of it holds up under the real loader -------------
    theme::load_with(theme::LoadRequest {
        name: Some("ubrany".to_string()),
        ..Default::default()
    });
    assert!(flag("glow.focus_ring.enabled"), "the halo is switched off after the save");
    // THE OWNER'S REPORT, as the owner stated it: the halo before the button
    // and the halo after it are the same halo. Not "a radius" — THAT radius.
    assert_eq!(
        px("glow.focus_ring.radius"),
        radius_before,
        "THE OWNER'S REPORT: the halo the theme wore before SAVE is not the \
         halo it wears after it"
    );
    // The ring the editor switched on carries the width it was given.
    assert!(
        px("focus.ring.width") > 0.0,
        "the ring the editor switched on has no width: {}",
        px("focus.ring.width")
    );
    // The appended two-line value won, because a later declaration is the one
    // the cascade keeps.
    let mode = theme::id("corner.mode").expect("corner.mode");
    assert_eq!(
        Some(theme::resolved().enum_of(mode)),
        theme::enum_index(mode, "chamfer"),
        "the appended enum word did not reach the cascade"
    );

    // ---- 8. and what the save replaced is still recoverable ----------------
    // The file this save patched is the person's own writing. A patch that
    // goes wrong — or a save the person did not mean — has to be undoable
    // from the disk, the way `layout/store.rs` has kept a copy of a layaut
    // it rewrote since it was written.
    assert_eq!(
        std::fs::read_to_string(dir.join("ubrany.theme.bak")).unwrap(),
        AUTHORED,
        "the bytes the save replaced are gone with no copy of them anywhere"
    );

    let _ = std::fs::remove_dir_all(&scratch);
}
