//! A role's own px floor and px ceiling, proved by moving them.
//!
//! The master gives all 24 roles `min_px` and `max_px` — 48 declared
//! tokens — and until now `Role::px` read neither. It applied the GLOBAL
//! `type.min_px` to every role and had no ceiling at all, so a theme could
//! write any number it liked into either key and no text on any screen
//! would move. The shipped file hides this perfectly: every role's floor
//! is written `@type.min_px`, which is the same number the global answered,
//! and every ceiling is written `0px`, which means uncapped. The two keys
//! were therefore invisible in exactly the theme anyone would test with.
//!
//! Each stage below writes a real number into one of them and requires the
//! resolved px to obey it.
//!
//! ONE test function, on purpose: the resolved theme is process-wide, so a
//! test that switches it must not run beside a test that reads it. The same
//! reason tests/role_bindings_chrome.rs gives.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::theme::{self, LoadRequest};
use nacelle::ui;
use nacelle::Ctx;

/// Runs one question on a thread of its own, for the reason
/// tests/role_bindings_chrome.rs sets out: the toolkit memoises a resolved
/// role per thread, so asking twice on one thread answers the first
/// fixture's ladder for the second fixture's question.
fn fresh<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| s.spawn(f).join().expect("the measuring thread panicked"))
}

/// `body`'s resolved px under whatever theme is loaded, at no scaling of
/// its own, so the number is the theme's alone.
fn body_px() -> f32 {
    fresh(|| {
        let mut dl = DrawList::new();
        let mut fonts = FontSystem::new();
        let c = Ctx {
            access: None,
            dl: &mut dl,
            fonts: &mut fonts,
            w: 1920.0,
            h: 1080.0,
            t: 0.0,
            mouse: Pointer::new(0.0, 0.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        };
        ui::role("body").px(&c, 1.0)
    })
}

/// Loads a fixture theme built out of the given `[type]` lines.
fn load(tag: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("nacelle-bounds-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("the fixture tree must be writable");
    let path = dir.join("fixture.theme");
    std::fs::write(&path, format!("[meta]\nschema = 1\nname = \"{tag}\"\n\n{body}"))
        .expect("the fixture theme must be writable");
    let _ = theme::load_with(LoadRequest { path: Some(path), ..Default::default() });
    dir
}

#[test]
fn a_role_obeys_its_own_px_floor_and_its_own_px_ceiling() {
    // The master's own answer, with the floor at `@type.min_px` and the
    // ceiling uncapped. Everything below is measured against this.
    let shipped = body_px();
    assert!(shipped > 0.0, "the master must give body a size to begin with");

    // ---------------------------------------------------------- ceiling
    // A ceiling under the shipped size but still above the readable floor,
    // so this stage measures the ceiling alone. (A ceiling BELOW the floor
    // is a contradiction, and which of the two wins is the last stage.)
    let global_floor = fresh(|| {
        let t = theme::resolved();
        t.px(theme::id("type.min_px").expect("the master declares type.min_px"))
    });
    assert!(
        shipped > global_floor + 2.0,
        "body must sit far enough above the floor ({global_floor}) for a ceiling \
         to be measurable between the two; it is {shipped}"
    );
    let cap = (shipped + global_floor) / 2.0;
    let dir = load("cap", &format!("[type]\nbody.max_px = {cap}px\n"));
    let capped = body_px();
    assert_eq!(
        capped, cap,
        "a role whose theme writes max_px must draw at that ceiling, not at \
         its own size ({shipped})"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // `0px` is how the master spells "uncapped", and it must not be
    // mistaken for a ceiling of zero — that would erase every role at once.
    let dir = load("uncapped", "[type]\nbody.max_px = 0px\n");
    assert_eq!(
        body_px(),
        shipped,
        "max_px = 0px means uncapped, not a ceiling of nothing"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // ------------------------------------------------------------ floor
    // A floor well over the shipped size, proving the role's OWN key is
    // what is read and not the global one the code used to reach for.
    let floor = shipped * 2.0;
    let dir = load("floor", &format!("[type]\nbody.min_px = {floor}px\n"));
    assert_eq!(
        body_px(),
        floor,
        "a role whose theme raises its own min_px must draw at that floor"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // ------------------------------------------------- the two disagree
    // A theme that caps a role below the floor has contradicted itself.
    // `type.min_px` is described in the master as the last defence against
    // unreadable type, so it is the one that wins.
    let dir = load(
        "conflict",
        &format!("[type]\nbody.min_px = {floor}px\nbody.max_px = 1px\n"),
    );
    assert_eq!(
        body_px(),
        floor,
        "when a ceiling sits below the floor the floor wins: the floor is the \
         last defence against unreadable type"
    );
    let _ = std::fs::remove_dir_all(&dir);

    // And the master is put back, so anything reading the theme after this
    // file is looking at the shipped picture again.
    let _ = theme::load_with(LoadRequest::default());
    assert_eq!(body_px(), shipped);
}
