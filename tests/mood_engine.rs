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
//!
//! The signal measured is the pair `[mood.alert]` actually writes —
//! `motion.alarm_blink.enabled` and `component.alarm_bar.fill` — since the
//! panel-edge glow it also used to raise was removed with the whole effect
//! on 2026-08-27 at the owner's order.

use nacelle::theme::{self, MoodWhen};

#[test]
fn the_masters_alarm_skin_is_reachable_and_changes_what_would_be_drawn() {
    let _ = theme::load();

    // The resting master ships the blink off and the alarmed bar bodiless
    // (`component.alarm_bar.fill = none`); `[mood.alert]` turns the first
    // on and gives the second a real alpha. That pair is the proof's
    // signal: both are the master's own lines, not fixture inventions.
    let blink =
        theme::id("motion.alarm_blink.enabled").expect("the master declares the alarm blink");
    let bar =
        theme::id("component.alarm_bar.fill").expect("the master declares the alarmed bar's body");
    assert!(!theme::resolved().flag(blink), "the resting theme is already blinking");
    let resting_bar_a = theme::resolved().color(bar).a;
    assert!(
        resting_bar_a <= 0.0,
        "the resting alarmed bar already has a body (alpha {resting_bar_a}), so nothing below \
         tells resting and alert apart"
    );

    let rules = theme::mood_rules();
    let alert = rules.iter().find(|r| r.name == "alert").expect("[mood.alert] is declared");
    assert_eq!(
        alert.when,
        MoodWhen::SeverityAtLeast(3),
        "the master's alert rule is not the one §5.24 documents"
    );

    assert!(theme::set_mood(Some("alert")), "the alarm mood resolved to no sibling");
    assert_eq!(theme::current_mood().as_deref(), Some("alert"));
    assert!(theme::resolved().flag(blink), "the alarm skin did not reach the bake");
    assert!(
        theme::resolved().color(bar).a > resting_bar_a,
        "the alarm's bar has no more body than the resting one it is supposed to outshout"
    );
    // The transition tint the host fades to zero over motion.mood_change.
    assert!(theme::mood_wash().is_some(), "the alarm arrives without a wash");

    // Lockdown is alert plus a data hue: it inherits, so it blinks too.
    assert!(theme::set_mood(Some("lockdown")));
    assert!(theme::resolved().flag(blink), "lockdown did not inherit alert");
    assert!(
        theme::resolved().color(bar).a > resting_bar_a,
        "lockdown did not inherit alert's bar"
    );

    // And letting go puts the resting picture back, unchanged.
    assert!(theme::set_mood(None));
    assert_eq!(theme::current_mood(), None);
    assert!(!theme::resolved().flag(blink), "letting go left the blink running");
    assert_eq!(
        theme::resolved().color(bar).a,
        resting_bar_a,
        "letting go left a body on the alarmed bar"
    );

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
    assert!(!theme::resolved().flag(blink), "the chosen theme blinks at rest");
    let chosen_bar_a = theme::resolved().color(bar).a;
    assert!(
        theme::mood_rules().iter().any(|r| r.name == "alert"),
        "a chosen theme lost the master's moods"
    );
    assert!(theme::set_mood(Some("alert")), "the alarm does not reach a chosen theme");
    assert!(theme::resolved().flag(blink));
    assert!(
        theme::resolved().color(bar).a > chosen_bar_a,
        "the alarm's bar has no more body than the chosen theme's resting one"
    );
    let alarmed = theme::resolved().color(accent);
    assert_eq!(
        (chosen.r, chosen.g, chosen.b),
        (alarmed.r, alarmed.g, alarmed.b),
        "the alarm repainted the theme's own accent"
    );
    assert!(theme::set_mood(None));
}
