//! The corner SHAPE of the four objects that used to freeze it.
//!
//! `menu.corner_mode`, `tooltip.corner_mode`, `field.corner_style` and
//! `segmented.corner_style` were the audit's phase-A finding: the radius
//! came from the theme while the cut was written in Rust, so the window's
//! own menu (which reads `winframe.corner_mode`) came out chamfered and
//! the context menu beside it came out round — one program, two corner
//! languages.
//!
//! This asks the only question that settles it: does editing the token
//! change what is drawn? The draw list's command register (`NACELLE_DRAW_CMDS`,
//! `DrawList::recording`) is the witness — it prints the corner a command
//! actually carried, not the vertices it happened to fan into, so a shape
//! that changed cannot hide behind an equal triangle count.
//!
//! ONE test in a binary of its own, because the resolved theme is
//! process-wide (§7.1 hands every draw path the same `&'static
//! ResolvedTheme`): a test that swaps it must not run beside one that
//! reads it.

use nacelle::draw::DrawList;
use nacelle::focus::FocusId;
use nacelle::font::FontSystem;
use nacelle::object::menu::{MenuEntry, MenuItem, MenuState};
use nacelle::object::segmented::{self, StripState};
use nacelle::object::text_input::{self, InputModel, InputStyle};
use nacelle::object::tooltip::{key, Tooltips};
use nacelle::pointer::Pointer;
use nacelle::theme;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;

fn ctx<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem, t: f64) -> Ctx<'a> {
    Ctx {
        access: None,
        dl,
        fonts,
        w: W,
        h: H,
        t,
        mouse: Pointer::new(150.0, 110.0),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    }
}

/// The corner word of the FIRST ring the object drew — its own box,
/// before any wash or ring laid over it. `None` when the object drew no
/// ring at all, which is itself a failure worth naming.
fn first_ring_corner(dl: &DrawList) -> Option<String> {
    dl.cmds().iter().find_map(|c| {
        let line = c.to_string();
        let rest = line.strip_prefix("ring_fill at")?;
        let word = rest.split(" corners ").nth(1)?.split_whitespace().next()?;
        // `round:8.10` — the cut and the length, of which only the cut
        // is under test here.
        Some(word.split(':').next().unwrap_or(word).to_string())
    })
}

/// Where the caret of `model` lands, in a field wide enough that the
/// value never scrolls — so the answer is the measured text width and
/// nothing else. Focused by construction, which is what puts a caret on
/// the screen at all with no focus chain in the world.
fn caret_of(model: &mut InputModel, fonts: &mut FontSystem) -> f32 {
    let mut dl = DrawList::new();
    let mut c = ctx(&mut dl, fonts, 0.0);
    let out = text_input::draw(
        &mut c,
        Rect::new(300.0, 300.0, 400.0, 32.0),
        model,
        FocusId::of("corner/kept"),
        &InputStyle { focused_fallback: true, ..InputStyle::default() },
    );
    out.caret.expect("a focused field drew no caret").x
}

/// The four boxes, each drawn into a recording list of its own, in the
/// theme that is loaded right now.
fn corners(fonts: &mut FontSystem) -> [Option<String>; 4] {
    let delay = theme::resolved().px(theme::id("tooltip.delay_ms").unwrap()) as f64 / 1000.0;

    let mut menu_dl = DrawList::recording();
    {
        let items = vec![
            MenuEntry::Item(MenuItem::new("Open", 1)),
            MenuEntry::Item(MenuItem::new("Close", 2)),
        ];
        // Opened a whole second ago: the accordion is finished, so the
        // box is drawn at its full height and not mid-unfold.
        let mut m = MenuState::open_at(items, 200.0, 200.0, 0.0);
        let mut c = ctx(&mut menu_dl, fonts, 1.0);
        m.draw(&mut c);
    }

    let mut tip_dl = DrawList::recording();
    {
        let anchor = Rect::new(100.0, 100.0, 200.0, 40.0);
        let mut tips = Tooltips::new();
        // The manager only draws once the pointer has rested for
        // `tooltip.delay_ms`, so the first frame is the wait.
        let mut warm = DrawList::new();
        {
            let mut c = ctx(&mut warm, fonts, 0.0);
            tips.hover(&c, key("CPU LOAD"), anchor, "CPU LOAD");
            tips.draw(&mut c);
        }
        let mut c = ctx(&mut tip_dl, fonts, delay);
        tips.hover(&c, key("CPU LOAD"), anchor, "CPU LOAD");
        tips.draw(&mut c);
    }

    let mut field_dl = DrawList::recording();
    {
        let mut model = InputModel::new();
        model.set_value("search");
        let mut c = ctx(&mut field_dl, fonts, 0.0);
        text_input::draw(
            &mut c,
            Rect::new(300.0, 300.0, 240.0, 32.0),
            &mut model,
            FocusId::of("corner/field"),
            &InputStyle::default(),
        );
    }

    let mut seg_dl = DrawList::recording();
    {
        let st = StripState::new(0);
        let mut c = ctx(&mut seg_dl, fonts, 0.0);
        segmented::draw(&mut c, Rect::new(400.0, 500.0, 360.0, 40.0), &["One", "Two"], &st);
    }

    [
        first_ring_corner(&menu_dl),
        first_ring_corner(&tip_dl),
        first_ring_corner(&field_dl),
        first_ring_corner(&seg_dl),
    ]
}

#[test]
fn the_four_boxes_take_the_cut_the_theme_names_and_change_when_it_changes() {
    let _ = theme::load();
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();

    // ---- the delivered look -------------------------------------------
    // ONE root, and four boxes that repeat it. Twelve presets used to state
    // the cut for themselves, and the shipped file said `chamfer` in four of
    // them and `round` in the rest — a window frame, a context menu, a
    // tooltip and a modal cut, everything hanging off the panel rounded. It
    // was defended as a language ("floating chrome is chamfered, controls
    // are round") and read on screen as a program that had not decided.
    //
    // So the question this asks is no longer "is it chamfer". A remembered
    // word is exactly what let the two halves drift apart while every
    // assertion about them passed. The question is whether all four say what
    // the ROOT says, whatever the root happens to say.
    let root = theme::enum_word_of(theme::id("corner.mode").expect("@corner.mode must exist"));
    assert!(root.is_some(), "the root names no word at all");
    let shipped = corners(&mut fonts);
    assert_eq!(
        shipped,
        [root.clone(), root.clone(), root.clone(), root.clone()],
        "the four boxes did not all take the cut @corner.mode names"
    );

    // Every preset that used to hold a literal now derives, so none of them
    // can disagree with the root while this passes.
    for name in [
        "panel.corner_mode",
        "winframe.corner_mode",
        "modal.corner_mode",
        "menu.corner_mode",
        "tooltip.corner_mode",
        "scrollbar.corner_style",
        "elev.popover.corner",
        "elev.overlay.corner",
    ] {
        assert_eq!(
            theme::enum_word_of(theme::id(name).unwrap_or_else(|| panic!("{name} must exist"))),
            root,
            "{name} states a cut of its own instead of deriving from @corner.mode"
        );
    }

    // One field that OUTLIVES the theme swap, so its measure cache is
    // the stale one: the widths in it were measured through the old
    // tracking, and nothing about the value or the caret is about to
    // change.
    let mut kept = InputModel::new();
    kept.set_value("search");
    let before = caret_of(&mut kept, &mut fonts);

    // ---- and now a theme that says otherwise --------------------------
    // ONE line. The fixture states the cut once and touches none of the
    // four objects by name, so anything that kept its shape is an object
    // the root does not reach — which is the whole failure this file was
    // written to catch, moved up to where the decision now lives.
    let path = std::env::temp_dir()
        .join(format!("nacelle-corner-fixture-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        "[meta]\nschema = 1\nname = \"Fixture\"\nbase = \"default\"\n\n\
         [corner]\nmode = chamfer\n\n\
         [type]\nfield.tracking = 0.30em\n",
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(theme::LoadRequest {
        path: Some(path.clone()),
        ..Default::default()
    });
    let _ = std::fs::remove_file(&path);

    let flipped = corners(&mut fonts);
    assert_eq!(
        flipped,
        [
            Some("chamfer".into()),
            Some("chamfer".into()),
            Some("chamfer".into()),
            Some("chamfer".into()),
        ],
        "one line named the cut and some box did not follow it"
    );

    // ---- and a theme that pins ONE object against the root ------------
    // The master promises the twelve presets keep their sibling keys, so a
    // theme may still cut one object differently. A promise written in the
    // master and checked nowhere is the next thing to drift apart, so it is
    // checked here: root chamfer, menu pinned round.
    //
    // The tooltip moves WITH the menu and that is not a leak — it derives
    // from `@menu.corner_mode` on purpose (a tip and the menu it rises from
    // are one surface), and a chain under the root is exactly what the root
    // is for. The field and the segmented strip hang off the panel instead,
    // so they must stay where the root put them.
    let pin = std::env::temp_dir()
        .join(format!("nacelle-corner-pin-{}.theme", std::process::id()));
    std::fs::write(
        &pin,
        "[meta]\nschema = 1\nname = \"Pin\"\nbase = \"default\"\n\n\
         [corner]\nmode = chamfer\n\n\
         [menu]\ncorner_mode = round\n",
    )
    .expect("the pin fixture must be writable");
    let _ = theme::load_with(theme::LoadRequest {
        path: Some(pin.clone()),
        ..Default::default()
    });
    let _ = std::fs::remove_file(&pin);

    let pinned = corners(&mut fonts);
    assert_eq!(
        pinned,
        [
            Some("round".into()),
            Some("round".into()),
            Some("chamfer".into()),
            Some("chamfer".into()),
        ],
        "pinning the menu either did not take, or reached past its own chain"
    );

    // Back to the fixture the caret assertion below was measured against.
    std::fs::write(
        &path,
        "[meta]\nschema = 1\nname = \"Fixture\"\nbase = \"default\"\n\n\
         [corner]\nmode = chamfer\n\n\
         [type]\nfield.tracking = 0.30em\n",
    )
    .expect("the fixture theme must be writable");
    let _ = theme::load_with(theme::LoadRequest {
        path: Some(path.clone()),
        ..Default::default()
    });
    let _ = std::fs::remove_file(&path);

    // ---- the inverse trap ---------------------------------------------
    // The fixture widens `type.field.tracking` and touches neither the
    // size nor the text, so every part of the old cache key still
    // matches. Only the theme epoch moved — and if the key does not
    // carry it, the caret stands where the previous theme put it until
    // the user next types.
    let after = caret_of(&mut kept, &mut fonts);
    assert!(
        after > before + 0.5,
        "the caret survived a theme that widened its tracking: {before} -> {after}"
    );
}
