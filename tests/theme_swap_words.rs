//! A binding answers the theme that is loaded NOW.
//!
//! Every `*_role` key is an OPEN word set: the master's own word is
//! interned first and whatever a loaded theme writes is interned after
//! it. The index is therefore a fact about ONE schema, and every
//! `theme::load_with` builds the schema afresh — so index 1 names the
//! first theme's word under the first schema and the second theme's word
//! under the second.
//!
//! The toolkit memoises the word behind an index, which is what keeps a
//! draw loop off the engine lock. Until the epoch joined that key, the
//! memo survived the swap and every role binding in the program went on
//! answering the PREVIOUS theme's role for the life of the thread. Two
//! fixtures, ONE thread, on purpose: a fresh thread per reading is the
//! workaround this test exists to make unnecessary.
//!
//! ONE test function, for the reason `mood_engine` gives: the resolved
//! theme is process-wide, so a test that swaps it must not run beside a
//! test that reads it.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::theme::{self, LoadRequest};
use nacelle::view::surface::{CtxSurface, Surface};
use nacelle::Ctx;

/// Loads a fixture theme whose base is the master, so every token but the
/// ones in `body` is the master's own.
fn skin(tag: &str, body: &str) {
    let path = std::env::temp_dir()
        .join(format!("nacelle-swap-{tag}-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        format!("[meta]\nschema = 1\nname = \"{tag}\"\nbase = \"default\"\n\n{body}"),
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(LoadRequest { path: Some(path.clone()), ..Default::default() });
    let _ = std::fs::remove_file(&path);
}

/// The word a binding stands at, asked exactly the way a drawing object
/// asks it — through the surface, which is where the memo lives.
fn word(name: &str) -> String {
    let mut dl = DrawList::new();
    let mut fonts = FontSystem::new();
    let mut ctx = Ctx {
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
    CtxSurface::new(&mut ctx).word(name)
}

#[test]
fn a_role_binding_follows_the_theme_across_a_swap() {
    // Two fixtures that write DIFFERENT words into the same binding. Both
    // land at the same index — one past the master's own word — which is
    // precisely why an index alone cannot name either of them.
    skin("first", "[button]\nrole = caption\n");
    assert_eq!(word("button.role"), "caption", "the first theme's word");

    skin("second", "[button]\nrole = title.window\n");
    assert_eq!(
        word("button.role"),
        "title.window",
        "the binding answered the PREVIOUS theme: the word memo outlived its schema"
    );

    // Back the other way, so the reading is not an artefact of the order.
    skin("third", "[button]\nrole = caption\n");
    assert_eq!(word("button.role"), "caption");

    // And the master is put back, so anything reading the theme after this
    // file is looking at the shipped picture again.
    let _ = theme::load_with(LoadRequest::default());
    assert_eq!(word("button.role"), "button", "the master's own word");
}
