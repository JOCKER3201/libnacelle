//! `[panel]`'s three surface keys — `elev`, `glass.rect`, `glass.inset` —
//! stop being declarations nobody reads.
//!
//! The 2026-08-17 audit found all three dead. `panel.elev` says WHICH
//! rung of the elevation ladder a resting panel takes its material from,
//! and `panel.rs` had `"elev.panel"` written into it, so a theme moving a
//! board of cards up to `elev.raised` got the rung it started with.
//! `glass.rect` and `glass.inset` say which rectangle the frost is poured
//! into and how far inside the ring it stops; the quad was laid on the
//! widget box whatever the theme asked.
//!
//! Measured against the resolved theme rather than against numbers
//! written here: the claim is "the panel draws the rung the token names",
//! not "the panel draws this particular grey".

use nacelle::draw::{DrawCmd, DrawList};
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::theme::{self, Color, LoadRequest};
use nacelle::widget::Chrome;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const BOX: Rect = Rect { x: 100.0, y: 80.0, w: 600.0, h: 400.0 };

fn fresh<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| s.spawn(f).join().expect("the drawing thread panicked"))
}

fn apply(fixture: Option<&str>) {
    match fixture {
        None => {
            let _ = theme::load();
        }
        Some(text) => {
            let path = std::env::temp_dir()
                .join(format!("nacelle-panel-surface-{}.theme", std::process::id()));
            std::fs::write(&path, text).expect("the fixture theme must be writable");
            let _ = theme::load_with(LoadRequest { path: Some(path), ..Default::default() });
        }
    }
}

const HEAD: &str = "[meta]\nschema = 1\nname = \"Panel surface fixture\"\nbase = \"default\"\n\n";

/// One panel drawn into a recording list: the commands, and the content
/// box the object answered.
fn panel() -> (Vec<DrawCmd>, Rect) {
    fresh(|| {
        let mut fonts = FontSystem::new();
        let mut dl = DrawList::recording();
        let content = {
            let mut c = Ctx {
                access: None,
                dl: &mut dl,
                fonts: &mut fonts,
                w: W,
                h: H,
                t: 0.0,
                mouse: Pointer::new(-1.0, -1.0),
                term_font_scale: 1.0,
                ui_font_scale: 1.0,
                panel_scale: 1.0,
                focus: None,
                tips: None,
            };
            let chrome = Chrome { title: Some("CPU".into()), ..Chrome::default() };
            nacelle::object::panel::draw(&mut c, BOX, &chrome, 0)
        };
        (dl.cmds().to_vec(), content)
    })
}

/// The colour a token bakes to as a BED, under whatever theme is loaded.
fn bed(name: &str) -> Color {
    let owned = name.to_string();
    fresh(move || {
        let t = theme::resolved();
        t.bed(theme::id(&owned).unwrap_or_else(|| panic!("{owned} is not declared")))
    })
}

/// The length a token bakes to, under whatever theme is loaded.
fn len(name: &str) -> f32 {
    let owned = name.to_string();
    fresh(move || {
        let t = theme::resolved();
        t.px(theme::id(&owned).unwrap_or_else(|| panic!("{owned} is not declared")))
    })
}

/// The body quad of the panel: the first `RingFill` covering the whole
/// widget box.
fn body(cmds: &[DrawCmd]) -> Color {
    cmds.iter()
        .find_map(|c| match c {
            DrawCmd::RingFill { r, color, .. } if r[0] == BOX.x && r[1] == BOX.y => Some(*color),
            _ => None,
        })
        .expect("a panel draws its body")
}

/// The frosted quad, if the rung asked for one.
fn glass(cmds: &[DrawCmd]) -> Option<[f32; 4]> {
    cmds.iter().find_map(|c| match c {
        DrawCmd::GlassFill { r, .. } => Some(*r),
        _ => None,
    })
}

#[test]
fn a_panel_wears_the_rung_and_the_glass_box_its_tokens_name() {
    the_rung_is_the_one_panel_elev_names();
    the_frost_fills_the_rectangle_glass_rect_names();
    apply(None);
}

fn the_rung_is_the_one_panel_elev_names() {
    apply(None);
    let (cmds, _) = panel();
    let shipped = body(&cmds);
    assert_eq!(
        shipped,
        bed("elev.panel.fill"),
        "the shipped master puts a panel on elev.panel and it drew something else"
    );

    // `raised` is a real rung one step up, and its fill is a different
    // token (`@surface.raised`), so following the binding is visible.
    apply(Some(&format!("{HEAD}[panel]\nelev = raised\n")));
    let (cmds, _) = panel();
    let moved = body(&cmds);
    assert_eq!(
        moved,
        bed("elev.raised.fill"),
        "panel.elev = raised and the panel still drew elev.panel's body"
    );
    assert_ne!(
        moved, shipped,
        "the two rungs bake to one colour, so this file cannot tell a panel that \
         reads its binding from one that does not"
    );
}

fn the_frost_fills_the_rectangle_glass_rect_names() {
    // A rung with frost on it: the shipped master keeps every rank at 0,
    // so nothing here would draw a glass quad at all.
    const FROSTED: &str = "[elev.panel]\nglass.rank = 2\n";

    apply(Some(&format!("{HEAD}{FROSTED}")));
    let (cmds, content) = panel();
    let border_box = glass(&cmds).expect("a rung at rank 2 pours a frosted quad");
    assert_eq!(
        border_box,
        [BOX.x, BOX.y, BOX.w, BOX.h],
        "the master's glass.rect = border_box did not frost the whole container"
    );

    apply(Some(&format!("{HEAD}{FROSTED}\n[panel]\nglass.rect = content_box\n")));
    let (cmds, content_again) = panel();
    assert_eq!(
        [content.x, content.y, content.w, content.h],
        [content_again.x, content_again.y, content_again.w, content_again.h],
        "the content box moved between fixtures"
    );
    let inner = glass(&cmds).expect("a rung at rank 2 pours a frosted quad");
    assert_eq!(
        inner,
        [content.x, content.y, content.w, content.h],
        "panel.glass.rect = content_box left the frost on the border box"
    );

    // And the inset pulls whichever box it is off the ring — BY THE
    // LENGTH THE TOKEN BAKES TO, which is the half of the claim a quad
    // measured against itself cannot make. Reading the distance back out
    // of `pulled[0]` and then asserting `pulled[0] == BOX.x + it` says
    // only that the number equals itself: an inset applied twice over, or
    // half over, passes that shape of test unchanged.
    apply(Some(&format!("{HEAD}{FROSTED}\n[panel]\nglass.inset = 2u\n")));
    let (cmds, _) = panel();
    let pulled = glass(&cmds).expect("a rung at rank 2 pours a frosted quad");
    let inset = len("panel.glass.inset");
    assert!(inset > 0.0, "the fixture must ask for a visible inset");
    assert_eq!(
        pulled,
        [BOX.x + inset, BOX.y + inset, BOX.w - 2.0 * inset, BOX.h - 2.0 * inset],
        "panel.glass.inset = 2u bakes to {inset} px and the frost was not pulled in by it"
    );
}
