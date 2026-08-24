//! The chrome's type-role bindings and its control row, proved by moving
//! them.
//!
//! `button.role`, `panel.title.role`, `winframe.title.role`,
//! `menu.item.role`, `winframe.button.order` and `winframe.button.corner`
//! were all declared by the master and read by no line in the tree: a
//! theme could rewrite any of them and the screen would not move. Each
//! stage below rewrites one of them and requires the drawing to move with
//! it.
//!
//! The screen is compared through the command register, not through the
//! vertex buffer. A command carries every number a binding can reach — a
//! run's text, px, tracking and ink, a ring's four corners — while a
//! vertex also carries an atlas coordinate, and the atlas repacks when a
//! fixture asks for a size nobody has drawn yet. That would report a
//! difference no theme made.
//!
//! One stage goes the other way, and it is the one that matters most: a
//! role RESTATED under a different name, key for key, must draw the
//! master's picture command for command. That is what separates "the code
//! noticed that a token changed" from "the code reads the ladder of
//! whichever role the binding lands on" — the first passes a difference
//! test, only the second survives this one.
//!
//! ONE test function, on purpose: the resolved theme is process-wide
//! (§7.1 hands every draw path the same `&'static ResolvedTheme`), so a
//! test that switches it must not run beside a test that reads it.
//! tests/mood_engine.rs is built the same way and says so.

use nacelle::draw::DrawList;
use nacelle::font::FontSystem;
use nacelle::object::{button, panel, winframe};
use nacelle::pointer::Pointer;
use nacelle::theme::{self, LoadRequest};
use nacelle::widget::Chrome;
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
const TITLE: &str = "Monitor zasobow";
const RIGHT: &str = "/var/log/nacelle";
const CAP: &str = "Reset";
const WINDOW: &str = "Terminal";

const FRAME: Rect = Rect { x: 700.0, y: 120.0, w: 700.0, h: 480.0 };

/// The frame's own measurements, hand-built exactly as winframe.rs's unit
/// tests build them, and NOT the theme's: the shipped `winframe.grip` is
/// 1.1x the title bar height, so a real frame's entire bar lies inside the
/// top resize band and no plate on it can be hit at all. That is a fault
/// in the master's numbers, reported on its own, and it would otherwise
/// hide the row this file is about.
fn metrics() -> winframe::Metrics {
    winframe::Metrics { title_h: 26.0, border: 1.8, cut: 11.0, grip: 6.0, corner_zone: 26.0 }
}

/// Runs one question on a thread of its own.
///
/// The toolkit memoises the WORD an enum token stands at per (token,
/// index), in thread-local state. A reload renumbers the open word sets a
/// role binding lives in — index 0 is the master's own word and index 1 is
/// whatever the loaded theme named, whichever theme that is — so asking
/// twice on one thread answers the FIRST fixture's role for the second
/// fixture's index. A fresh thread asks the engine instead of the memo.
/// The staleness is real and is filed as its own finding; it is not what
/// this file is here to prove.
fn fresh<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| s.spawn(f).join().expect("the drawing thread panicked"))
}

fn ctx<'a>(dl: &'a mut DrawList, fonts: &'a mut FontSystem) -> Ctx<'a> {
    Ctx {
        access: None,
        dl,
        fonts,
        w: W,
        h: H,
        t: 0.0,
        mouse: Pointer::new(0.0, 0.0),
        term_font_scale: 1.0,
        ui_font_scale: 1.0,
        panel_scale: 1.0,
        focus: None,
        tips: None,
    }
}

/// Everything the three objects put on a screen: a button cap, a panel's
/// title band, and a window frame with its menu unfolded.
fn chrome(fonts: &mut FontSystem) -> Vec<String> {
    fresh(|| {
        let mut dl = DrawList::recording();
        {
            let mut c = ctx(&mut dl, fonts);
            button::draw(
                &mut c,
                Rect::new(40.0, 40.0, 220.0, 48.0),
                CAP,
                button::ButtonState::default(),
            );
            let band = Chrome {
                title: Some(TITLE.to_string()),
                right: Some(RIGHT.to_string()),
                ..Chrome::none()
            };
            panel::draw(&mut c, Rect::new(40.0, 120.0, 620.0, 380.0), &band, 0);
            let mut f = winframe::Frame::new();
            f.toggle_menu();
            f.draw(&mut c, &metrics(), FRAME, WINDOW, true);
        }
        dl.cmds().iter().map(|c| c.to_string()).collect()
    })
}

/// What the three control plates answer, slot 0 nearest the right edge.
fn row() -> Vec<winframe::Part> {
    fresh(|| {
        let m = metrics();
        let f = winframe::Frame::new();
        let t = theme::resolved();
        let px =
            |n: &str| t.px(theme::id(n).unwrap_or_else(|| panic!("the master declares no {n}")));
        // button_rect's arithmetic, from the same three tokens.
        let s = px("winframe.button.size");
        let step = s + px("winframe.button.gap");
        let y = FRAME.y + m.border + m.title_h / 2.0;
        (0..3)
            .map(|slot| {
                let x = FRAME.x + FRAME.w - m.border - px("winframe.button.pad") - s / 2.0
                    - step * slot as f32;
                f.hit(FRAME, &m, x, y)
            })
            .collect()
    })
}

/// Loads the master, or a theme based on it that rewrites a few keys.
fn apply(fixture: Option<&str>) {
    match fixture {
        None => {
            let _ = theme::load();
        }
        Some(text) => {
            let path = std::env::temp_dir()
                .join(format!("nacelle-role-binding-{}.theme", std::process::id()));
            std::fs::write(&path, text).expect("the fixture theme must be writable");
            let _ = theme::load_with(LoadRequest { path: Some(path), ..Default::default() });
        }
    }
}

/// Every difference between two screens, as readable lines. Empty means the
/// two are the same screen.
fn diff(a: &[String], b: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    if a.len() != b.len() {
        out.push(format!("command count {} vs {}", a.len(), b.len()));
    }
    for (i, (x, y)) in a.iter().zip(b.iter()).enumerate() {
        if x != y {
            out.push(format!("command {i}:\n  {x}\n  {y}"));
        }
        if out.len() > 4 {
            out.push("...".to_string());
            break;
        }
    }
    out
}

const HEAD: &str = "[meta]\nschema = 1\nname = \"Role binding fixture\"\nbase = \"default\"\n\n";

/// The four bindings, each moved on its own so that a difference can only
/// be that one binding's doing. `caption` is a shipped role that differs
/// from every one of them in size, tracking, case and leading at once.
const BUTTON_ROLE: &str = "[button]\nrole = caption\n";
const PANEL_ROLE: &str = "[panel]\ntitle.role = caption\n";
const WINFRAME_ROLE: &str = "[winframe]\ntitle.role = caption\n";
const MENU_ITEM_ROLE: &str = "[menu]\nitem.role = caption\n";

/// The same four bindings moved to spare roles that RESTATE, key for key,
/// the roles the master binds today. `min_px` is written out with them:
/// the floor belongs to the role, and a restatement that left it out would
/// pass this stage for the wrong reason.
///
/// `face` is written out for exactly that reason too, and it is the key
/// that made this stage fail the day the chrome stopped naming a font slot
/// in Rust and started asking the role for one. A restatement missing its
/// face is a role whose `face` nobody states, which is the interface slot
/// by fallback — so the stage would have compared the master's `ui_medium`
/// chrome against a `ui` one and blamed the binding. Each face is written
/// as a reference to the role being restated rather than as a word, so
/// this fixture cannot drift from the master the day a face moves there.
fn restated(floor: &str) -> String {
    let mut s = String::from("[type]\n");
    for (spare, size, track, case, lead, face) in [
        ("spare0", "2.21u", "0.100em", "upper", "1.00", "@type.button.face"),
        ("spare1", "2.47u", "0.140em", "smallcaps", "1.45", "@type.title.panel.face"),
        ("spare2", "2.99u", "0.120em", "smallcaps", "1.40", "@type.title.window.face"),
        ("spare3", "2.47u", "0.020em", "none", "1.45", "@type.body.face"),
    ] {
        s.push_str(&format!(
            "{spare}.size = {size}\n{spare}.min_px = {floor}\n{spare}.tracking = {track}\n\
             {spare}.case = {case}\n{spare}.leading = {lead}\n{spare}.alpha = 1.0\n\
             {spare}.face = {face}\n"
        ));
    }
    s.push_str(
        "\n[button]\nrole = spare0\n\n[panel]\ntitle.role = spare1\n\n\
         [winframe]\ntitle.role = spare2\n\n[menu]\nitem.role = spare3\n",
    );
    s
}

/// The band's ink dimmed by the ROLE's own alpha rather than by the alpha
/// of the role whose name used to be spelled in the code.
const PANEL_ALPHA: &str = "[type]\nspare1.size = 2.47u\nspare1.min_px = @type.min_px\n\
                           spare1.tracking = 0.140em\nspare1.case = smallcaps\n\
                           spare1.leading = 1.45\nspare1.alpha = 0.4\n\
                           spare1.face = @type.title.panel.face\n\n\
                           [panel]\ntitle.role = spare1\n";

/// A reordered row with a control dropped: close moves to the far left,
/// minimise takes the outermost place, and the middle slot carries a word
/// the row cannot name, which is the same nothing a dropped slot carries.
///
/// The master spells that drop `none`, and `none` cannot be written here:
/// the parser reads it as a §5.0 sentinel before the slot is ever asked
/// for a word, so the slot answers its own master literal and the control
/// silently stays. Filed as a finding of its own — the consumer below is
/// the same word comparison either way.
const ORDER: &str = "[winframe]\nbutton.order = [close, dropped, minimise]\n";

/// A radius on the control plates, which ship square.
const PLATE_CORNER: &str = "[winframe]\nbutton.corner = @corner.md\n";

#[test]
fn the_chrome_reads_the_role_its_binding_names_and_the_row_its_order_names() {
    theme::set_viewport(H, 1.0);
    let mut fonts = FontSystem::new();

    let mut shot = |fixture: Option<String>| {
        apply(fixture.as_deref());
        chrome(&mut fonts)
    };

    let master = shot(None);
    assert!(!master.is_empty(), "the chrome drew nothing — the harness is broken");

    // Each binding, alone: moving it must move the screen.
    for (name, fixture) in [
        ("button.role", BUTTON_ROLE),
        ("panel.title.role", PANEL_ROLE),
        ("winframe.title.role", WINFRAME_ROLE),
        ("menu.item.role", MENU_ITEM_ROLE),
        ("the title role's own alpha", PANEL_ALPHA),
        ("winframe.button.corner", PLATE_CORNER),
    ] {
        let moved = shot(Some(format!("{HEAD}{fixture}")));
        assert!(
            !diff(&master, &moved).is_empty(),
            "{name} changed nothing on the screen — the token is still unread"
        );
    }

    // And the other direction: the same ladder under four other names is
    // the same screen, which is what says the LADDER is what gets read.
    let restated_as_shipped = shot(Some(format!("{HEAD}{}", restated("@type.min_px"))));
    let d = diff(&master, &restated_as_shipped);
    assert!(
        d.is_empty(),
        "a role restated under another name did not draw the master's chrome:\n{}",
        d.join("\n")
    );
    // One key apart from that restatement, and it is the roles' own floor.
    let floored = shot(Some(format!("{HEAD}{}", restated("40px"))));
    assert!(
        !diff(&restated_as_shipped, &floored).is_empty(),
        "type.<role>.min_px does not floor the bound role — the key is unread"
    );

    // ---- the control row ----------------------------------------------
    apply(None);
    assert_eq!(
        row(),
        [winframe::Part::Close, winframe::Part::Maximize, winframe::Part::Minimize],
        "the shipped [minimise, maximise, close] is not the row every screenshot shows"
    );

    let reordered = shot(Some(format!("{HEAD}{ORDER}")));
    assert_eq!(
        row(),
        [winframe::Part::Minimize, winframe::Part::Title, winframe::Part::Close],
        "winframe.button.order moved neither the controls nor the slot it dropped"
    );
    assert!(!diff(&master, &reordered).is_empty(), "the reordered row drew the shipped row");

    // The master's screen comes back when the fixtures are let go.
    let again = shot(None);
    let d = diff(&master, &again);
    assert!(d.is_empty(), "the master did not come back:\n{}", d.join("\n"));
}
