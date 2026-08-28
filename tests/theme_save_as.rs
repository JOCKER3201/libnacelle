//! SAVE AS IS A COPY OF THE THEME ON SCREEN — including onto a name that
//! already carries a file.
//!
//! The patching save of 2026-08-18 closed "the halo disappears when I press
//! save" by teaching the file to read the edit set's silences the way a bake
//! does: a token the set does not mention is a token left standing. But
//! "standing" is a fact about A FILE, and the first draft patched the file
//! under the name being WRITTEN. Save a dressed theme AS a name some other
//! theme already occupies and the silence was answered by that other file:
//! the saved theme came out wearing a halo from a theme the person was not
//! even looking at, matching neither the preview nor the theme it replaced.
//! The same bug the patch closed, moved one button along.
//!
//! The dressed halo here is the FOCUS RING's (`glow.focus_ring`), the one
//! lit class left after the panel-edge effect was removed on 2026-08-27 at
//! the owner's order — the silence contract under test is the same.
//!
//! Its own process because it steers `HOME` and `XDG_DATA_HOME`, which
//! `save_theme_as` and the loader's search walk both read. It never loads a
//! theme; the claim is entirely about bytes.

use nacelle::theme::{
    self,
    color::Oklch,
    edit::{border_colour_edit, focus_ring_edits, FocusRing, RingStyle, Scope},
};

/// The theme the editor has open: a ring halo the AUTHOR dressed, so the
/// edit set says nothing at all about `radius`.
const SOURCE: &str = r#"# Zrodlo. To jest motyw, ktory widac na ekranie.
[glow.focus_ring]
enabled = false
radius  = 2.40u                       # the author's own halo

[elev.panel]
edge.color = oklch(0.5000, 0.0500, 200.00)

[palette]
accent = oklch(0.6000, 0.1000, 300.00)
"#;

/// A DIFFERENT theme that already owns the name SAVE AS is about to take.
/// Its numbers are the ones a save must not quietly inherit.
const TARGET: &str = r#"# Docelowy. Inny motyw, ktory juz zajmuje te nazwe.
[glow.focus_ring]
enabled = true
radius  = 3.00u

[palette]
accent = oklch(0.2000, 0.0100, 10.00)
"#;

#[test]
fn saving_as_a_name_that_is_taken_writes_this_theme_not_a_hybrid() {
    let scratch = std::env::temp_dir().join(format!("nacelle-theme-as-{}", std::process::id()));
    let home = scratch.join("home");
    let data = scratch.join("data");
    // Both names, because this test was written before the search path
    // learned the family folder and a save now lands in the NEW one.
    // The old folder is still READ — that is the whole migration contract,
    // "read both, move nothing" — so the source theme is put there, where
    // a user's existing file would actually be, and the save is expected
    // in the new one. A test that created only one folder would be a test
    // of whichever name happened to win.
    let old_dir = data.join("nacelle-desktop/themes");
    let dir = data.join("nacelle/themes");
    std::fs::create_dir_all(&old_dir).unwrap();
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(&home).unwrap();
    // SAFETY: one test in its own process, so nothing races the environment.
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::set_var("XDG_DATA_HOME", &data);
        std::env::remove_var("NACELLE_THEME_DIR");
        std::env::remove_var("NACELLE_THEME_LOCAL");
    }
    let source = old_dir.join("zrodlo.theme");
    let target = dir.join("docelowy.theme");
    std::fs::write(&source, SOURCE).unwrap();
    std::fs::write(&target, TARGET).unwrap();

    // The set the editor really produces for a theme that HAS dressed its
    // ring halo: `halo_dressed = true`, so the radius is not in it, and only
    // the file it is laid against can decide what it ends up being.
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
    for token in ["glow.focus_ring.radius"] {
        assert!(
            !edits.iter().any(|e| e.token == token),
            "the editor mentioned {token} — this test can no longer say anything \
             about WHICH file answers a silence, and the model changed under it"
        );
    }

    let saved =
        theme::save_theme_as(Some("zrodlo"), "docelowy", &edits).expect("the save refused");
    assert_eq!(saved, target, "SAVE AS landed somewhere other than the name it was given");
    let after = std::fs::read_to_string(&target).unwrap();

    // ---- 1. the dress that travelled is the SOURCE's ----------------------
    assert!(
        after.contains("radius  = 2.40u"),
        "the theme saved under a new name did not bring its own halo:\n{after}"
    );
    assert!(
        !after.contains("3.00u"),
        "THE HYBRID: the saved theme is wearing the halo of the file it \
         replaced, so it matches neither the preview nor that file:\n{after}"
    );

    // ---- 2. and so did everything else of the source's --------------------
    assert!(
        after.contains("# Zrodlo. To jest motyw, ktory widac na ekranie.")
            && after.contains("accent = oklch(0.6000, 0.1000, 300.00)"),
        "SAVE AS kept only the editor's dozen values instead of copying the \
         theme:\n{after}"
    );
    assert!(
        !after.contains("# Docelowy."),
        "the replaced theme's notes are still in the file that replaced \
         it:\n{after}"
    );

    // ---- 3. what the editor DID say landed --------------------------------
    assert!(
        after.contains("oklch(0.7000, 0.1200, 40.00)"),
        "the border colour the sliders set never reached the file:\n{after}"
    );
    let switch = after
        .lines()
        .find(|l| l.trim_start().starts_with("enabled ="))
        .expect("the file lost its enabled line");
    assert!(switch.contains("= true"), "the ring's switch never reached the file: {switch}");

    // ---- 4. the theme it was saved FROM is untouched ----------------------
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        SOURCE,
        "SAVE AS wrote back into the theme it copied"
    );

    // ---- 5. the theme it was saved OVER is recoverable ---------------------
    // A save is now a patch of somebody's own work, and SAVE AS onto a taken
    // name replaces that work wholesale. One rescue copy is the difference
    // between a mistake and a loss.
    assert_eq!(
        std::fs::read_to_string(dir.join("docelowy.theme.bak")).unwrap(),
        TARGET,
        "the theme SAVE AS replaced is gone with no copy of it anywhere"
    );
    // And the rescue copy is not itself a theme: the loader joins
    // `<name>.theme` and `available_themes` tests the extension, so neither
    // can see it.
    assert!(
        !theme::available_themes().iter().any(|n| n.contains("bak")),
        "the rescue copy turned up in the theme list: {:?}",
        theme::available_themes()
    );

    let _ = std::fs::remove_dir_all(&scratch);
}
