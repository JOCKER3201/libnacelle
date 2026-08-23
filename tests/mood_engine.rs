//! §5.24 end to end: the master's own alarm skin, actually reached.
//!
//! The unit tests hold the `when` parser and the sibling table still. This
//! one asks the question the whole feature exists to answer — does asking
//! for a mood change what would be drawn? — against the embedded master, the
//! real cascade and the real bake.
//!
//! It lives in a binary of its own, and it is ONE test, because the active
//! sibling is process-wide by design (§7.1 hands every draw path the same
//! `&'static ResolvedTheme`): a test that switches it must not run beside a
//! test that reads it.

use nacelle::theme::{self, MoodWhen};

#[test]
fn the_masters_alarm_skin_is_reachable_and_changes_what_would_be_drawn() {
    let _ = theme::load();

    // `glow.panel_edge.enabled` was false in the resting master and true in
    // `[mood.alert]` until 2026-08-23; the master ships it lit at rest now
    // (the neon-by-default change), so the flag alone no longer tells
    // resting and alert apart — `[mood.alert]`'s own radius and alpha,
    // both raised above the resting master's, are what carry that now
    // (image 4's screen-wide lit edges is still louder, just off a louder
    // floor).
    let glow = theme::id("glow.panel_edge.enabled").expect("the master declares the panel glow");
    let radius = theme::id("glow.panel_edge.radius").expect("the master declares the glow's radius");
    let alpha = theme::id("glow.panel_edge.alpha").expect("the master declares the glow's alpha");
    assert!(theme::resolved().flag(glow), "the resting theme does not glow");
    let resting_radius = theme::resolved().px(radius);
    let resting_alpha = theme::resolved().px(alpha);

    let rules = theme::mood_rules();
    let alert = rules.iter().find(|r| r.name == "alert").expect("[mood.alert] is declared");
    assert_eq!(
        alert.when,
        MoodWhen::SeverityAtLeast(3),
        "the master's alert rule is not the one §5.24 documents"
    );

    assert!(theme::set_mood(Some("alert")), "the alarm mood resolved to no sibling");
    assert_eq!(theme::current_mood().as_deref(), Some("alert"));
    assert!(theme::resolved().flag(glow), "the alarm skin did not reach the bake");
    assert!(
        theme::resolved().px(radius) > resting_radius && theme::resolved().px(alpha) > resting_alpha,
        "the alarm's edges are not louder than the resting glow they are supposed to outshout"
    );
    // The transition tint the host fades to zero over motion.mood_change.
    assert!(theme::mood_wash().is_some(), "the alarm arrives without a wash");

    // Lockdown is alert plus a data hue: it inherits, so it glows louder too.
    assert!(theme::set_mood(Some("lockdown")));
    assert!(theme::resolved().flag(glow), "lockdown did not inherit alert");
    assert!(
        theme::resolved().px(radius) > resting_radius && theme::resolved().px(alpha) > resting_alpha,
        "lockdown did not inherit alert's louder edges"
    );

    // And letting go puts the resting picture back, unchanged.
    assert!(theme::set_mood(None));
    assert_eq!(theme::current_mood(), None);
    assert!(theme::resolved().flag(glow), "letting go changed whether the resting theme glows");
    assert_eq!(theme::resolved().px(radius), resting_radius, "letting go moved the resting radius");
    assert_eq!(theme::resolved().px(alpha), resting_alpha, "letting go moved the resting alpha");

    // ---- and now with a theme chosen, which is the usual case ----------
    // Not one theme in the catalogue declares a mood of its own, so a mood
    // only exists at all if the master's reaches a chosen theme. It must,
    // and it must arrive as a RE-MAP: the theme's own seeds are still the
    // theme's while the alarm is up (§5.24 — a mood may not change what the
    // interface is, only how loudly it says it).
    let path = std::env::temp_dir().join(format!("nacelle-mood-fixture-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        "[meta]\nschema = 1\nname = \"Fixture\"\nbase = \"default\"\n\n\
         [palette]\naccent = #FF00FF\n",
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(theme::LoadRequest { path: Some(path.clone()), ..Default::default() });
    let _ = std::fs::remove_file(&path);

    let accent = theme::id("palette.accent").expect("the master declares the accent");
    let chosen = theme::resolved().color(accent);
    assert!(theme::resolved().flag(glow), "the chosen theme does not glow at rest either");
    let chosen_radius = theme::resolved().px(radius);
    let chosen_alpha = theme::resolved().px(alpha);
    assert!(
        theme::mood_rules().iter().any(|r| r.name == "alert"),
        "a chosen theme lost the master's moods"
    );
    assert!(theme::set_mood(Some("alert")), "the alarm does not reach a chosen theme");
    assert!(theme::resolved().flag(glow));
    assert!(
        theme::resolved().px(radius) > chosen_radius && theme::resolved().px(alpha) > chosen_alpha,
        "the alarm's edges are not louder than the chosen theme's resting glow"
    );
    let alarmed = theme::resolved().color(accent);
    assert_eq!(
        (chosen.r, chosen.g, chosen.b),
        (alarmed.r, alarmed.g, alarmed.b),
        "the alarm repainted the theme's own accent"
    );
    assert!(theme::set_mood(None));
}
