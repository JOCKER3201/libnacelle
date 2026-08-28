//! The theme engine is ONE global, and a preview lays values over it. A test
//! that previews therefore changes what every other test sees, so this file
//! holds exactly one — cargo gives an integration file its own process, and a
//! file with one test has nothing to interleave with.
//!
//! (The same shape as `viewport_memo.rs`, and for the same reason: the first
//! version of THAT test was written beside the unit tests and broke
//! `text_input`, which had measured a line at a height it changed underneath.)

use nacelle::theme;

/// A preview is seen, is not a theme change, and comes back off cleanly.
///
/// All four claims in one test because they are one behaviour, and because
/// splitting them across a file whose whole point is to hold one test would
/// defeat the isolation.
#[test]
fn a_preview_is_seen_costs_no_content_epoch_and_lifts_off_again() {
    theme::load();
    let id = theme::id("elev.panel.edge.color").expect("elev.panel.edge.color");

    let before = theme::resolved().color(id);
    let epoch_before = theme::content_epoch();

    // A colour no theme would ship, so the assertion cannot pass by accident.
    let refused = theme::set_preview(&[("elev.panel.edge.color", "oklch(0.8000, 0.2000, 30.00)")]);
    assert!(refused.is_empty(), "the engine refused a value it should take: {refused:?}");

    let during = theme::resolved().color(id);
    assert_ne!(
        (before.r, before.g, before.b),
        (during.r, during.g, during.b),
        "the preview changed nothing the theme answers with"
    );
    assert!(theme::previewing(), "values are laid over and the engine says they are not");

    // THE POINT OF THE WHOLE DESIGN. `content_epoch` is what the font system
    // guards its face reload with, and that reload walks the font directories
    // and resets the atlas. If a preview moved it, every settled slider would
    // pay for a rescan of the system's fonts — the same cost that had
    // `--desktop` pinned at 100 % CPU this morning, arriving through a
    // different door.
    assert_eq!(
        epoch_before,
        theme::content_epoch(),
        "previewing a colour moved the CONTENT epoch, which re-reads every font face"
    );

    // CANCEL.
    theme::clear_preview();
    let after = theme::resolved().color(id);
    assert_eq!(
        (before.r, before.g, before.b),
        (after.r, after.g, after.b),
        "clearing the preview did not put the theme back"
    );
    assert!(!theme::previewing());

    // A name the master does not declare is refused BY NAME, and refusing it
    // does not throw away the values that were good — an editor with one stale
    // token in its set must still be able to show the rest.
    let refused = theme::set_preview(&[
        ("elev.panel.edge.color", "oklch(0.8000, 0.2000, 30.00)"),
        ("elev.panel.edge.gradient_that_does_not_exist", "none"),
    ]);
    assert_eq!(refused.len(), 1, "expected exactly one refusal, got {refused:?}");
    assert!(
        refused[0].contains("elev.panel.edge.gradient_that_does_not_exist"),
        "the refusal does not name the token that caused it: {}",
        refused[0]
    );
    assert_ne!(
        (before.r, before.g, before.b),
        (theme::resolved().color(id).r, theme::resolved().color(id).g, theme::resolved().color(id).b),
        "one bad token threw away a good one"
    );

    theme::clear_preview();

    // A FLAG previews too — not only a colour. If the flag half silently
    // failed, an editor switch would still LOOK right wherever a colour
    // moved beside it while the switch itself changed nothing at all.
    // (`glow.focus_ring.enabled` since 2026-08-27 — the panel-edge class
    // left with the effect, at the owner's order.)
    let flag = theme::id("glow.focus_ring.enabled").expect("glow.focus_ring.enabled");
    let flag_before = theme::resolved().flag(flag);
    let want = !flag_before;
    let refused = theme::set_preview(&[(
        "glow.focus_ring.enabled",
        if want { "true" } else { "false" },
    )]);
    assert!(refused.is_empty(), "the flag was refused: {refused:?}");
    assert_eq!(
        theme::resolved().flag(flag),
        want,
        "a previewed flag did not reach the bake — the editor's switches cannot switch"
    );
    theme::clear_preview();
    assert_eq!(theme::resolved().flag(flag), flag_before);
}
