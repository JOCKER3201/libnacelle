//! The script stack spends the theme's air, and its meter is typed by the
//! theme's roles.
//!
//! Both facts used to be false in the same file. `draw_stack` moved down
//! by an element's height and nothing else, so `script.element_gap`,
//! `script.meter_gap` and `script.dots_gap` were declared, documented and
//! unreadable — every panel was tighter than the theme said and no edit
//! to `space.2` could loosen it. The `meter` element, meanwhile, typed
//! its own label and readout from the pass's legacy base size with the
//! tracking of two HARD-CODED role names, so `script.meter_label_role`
//! and `script.meter_value_role` had no reader at all.
//!
//! A fix that only reads the token is a fix on paper. What is asked here
//! is the question a theme author asks: does CHANGING the token change
//! the picture, by exactly what it says? Every claim below is a
//! difference between two frames drawn from the same script under two
//! themes that differ in one line.
//!
//! It lives in a binary of its own, and it is ONE test, because the
//! active theme is process-wide (§7.1 hands every draw path the same
//! `&'static ResolvedTheme`): a test that reloads it must not run beside
//! a test that reads it.

use nacelle::draw::{DrawCmd, DrawList};
use nacelle::font::FontSystem;
use nacelle::pointer::Pointer;
use nacelle::script::{Script, ScriptWidget};
use nacelle::telemetry::Snapshot;
use nacelle::theme::{self, LoadRequest};
use nacelle::widget::{Host, Widget};
use nacelle::{Ctx, Rect};

const W: f32 = 1920.0;
const H: f32 = 1080.0;
/// Roomy enough that the shrink-to-fit never engages: a scaled stack
/// would still prove the rule, but it would prove it in scaled pixels.
const PANEL: (f32, f32, f32, f32) = (120.0, 90.0, 600.0, 700.0);
/// Lengths agree to a thousandth of a pixel — the grain the command
/// register already prints at.
const GRAIN: f32 = 1e-3;

fn px_of(name: &str) -> f32 {
    let id = theme::id(name).unwrap_or_else(|| panic!("the master declares no {name}"));
    theme::resolved().px(id)
}

/// One frame of a script, as the drawing commands it asked for.
fn frame(src: &str) -> Vec<DrawCmd> {
    let path = std::env::temp_dir()
        .join(format!("nacelle-script-stack-{}.rhai", std::process::id()));
    std::fs::write(&path, src).expect("the fixture script must be writable");
    let script = Script::load(&path).expect("the fixture script must compile");
    let mut widget = ScriptWidget::new(script);
    let mut dl = DrawList::recording();
    let mut fonts = FontSystem::new();
    let snap = Snapshot::default();
    let host = Host {
        snap: &snap,
        term: None,
        tabs: &[],
        tab_active: 0,
        shell_cwd: None,
        t: 0.0,
        window: (W, H),
    };
    {
        let mut ctx = Ctx {
            access: None,
            dl: &mut dl,
            fonts: &mut fonts,
            w: W,
            h: H,
            t: 0.0,
            mouse: Pointer::new(0.0, 0.0),
            term_font_scale: 1.0,
            ui_font_scale: 1.0,
            panel_scale: 1.0,
            focus: None,
            tips: None,
        };
        let (x, y, w, h) = PANEL;
        widget.draw(&mut ctx, Rect::new(x, y, w, h), &host);
    }
    dl.cmds().to_vec()
}

/// The one text command carrying `s`, as (top y, px, tracking).
fn glyphs(cmds: &[DrawCmd], s: &str) -> (f32, f32, f32) {
    let mut found = cmds.iter().filter_map(|c| match c {
        DrawCmd::Text { at, px, tracking, text, .. } if text == s => {
            Some((at[1], *px, *tracking))
        }
        _ => None,
    });
    let one = found.next().unwrap_or_else(|| panic!("nothing drew \"{s}\""));
    assert!(found.next().is_none(), "\"{s}\" was drawn more than once");
    one
}

/// The top of the highest filled rect — for a `dots` element, its first
/// cell, and so the top of the block itself.
fn first_rect_top(cmds: &[DrawCmd]) -> f32 {
    cmds.iter()
        .filter_map(|c| match c {
            DrawCmd::Rect { r, .. } => Some(r[1]),
            _ => None,
        })
        .fold(f32::INFINITY, f32::min)
}

/// Every measurement one theme yields, so the two themes can be compared
/// line by line instead of frame by frame.
struct Probe {
    /// `script.element_gap`, `script.meter_gap`, `script.dots_gap`.
    element_gap: f32,
    meter_gap: f32,
    dots_gap: f32,
    /// Tops of the three plain `text` elements of the first stack.
    text_run: [f32; 3],
    /// Tops of the two `text` elements a `meter` stands between.
    around_meter: [f32; 2],
    /// The `text` above a `dots` block, and the block's own first cell.
    above_dots: f32,
    dots_top: f32,
    /// The meter's label and readout: size and letter spacing, each as
    /// drawn.
    label: (f32, f32),
    value: (f32, f32),
}

fn probe() -> Probe {
    let run = frame(
        r#"fn draw() { [
            text("ALPHA", "left", #{ role: "body" }),
            text("BETA",  "left", #{ role: "body" }),
            text("GAMMA", "left", #{ role: "body" }),
        ] }"#,
    );
    let meter = frame(
        r#"fn draw() { [
            text("OVER", "left", #{ role: "body" }),
            meter("SWAP", 0.5, "50%"),
            text("UNDER", "left", #{ role: "body" }),
        ] }"#,
    );
    let dots = frame(
        r#"fn draw() { [
            text("OVER", "left", #{ role: "body" }),
            dots(0.5),
        ] }"#,
    );
    let (label_y, label_px, label_track) = glyphs(&meter, "SWAP");
    let (value_y, value_px, value_track) = glyphs(&meter, "50%");
    assert!(
        label_y.is_finite() && value_y.is_finite(),
        "the meter drew neither string"
    );
    Probe {
        element_gap: px_of("script.element_gap"),
        meter_gap: px_of("script.meter_gap"),
        dots_gap: px_of("script.dots_gap"),
        text_run: [
            glyphs(&run, "ALPHA").0,
            glyphs(&run, "BETA").0,
            glyphs(&run, "GAMMA").0,
        ],
        around_meter: [glyphs(&meter, "OVER").0, glyphs(&meter, "UNDER").0],
        above_dots: glyphs(&dots, "OVER").0,
        dots_top: first_rect_top(&dots),
        label: (label_px, label_track),
        value: (value_px, value_track),
    }
}

/// A theme that says one thing differently in every line this test reads,
/// and nothing else at all: what changes in the picture can only have
/// come from the line that changed.
fn fixture() -> std::path::PathBuf {
    let path = std::env::temp_dir()
        .join(format!("nacelle-script-stack-{}.theme", std::process::id()));
    std::fs::write(
        &path,
        "[meta]\nschema = 1\nname = \"Fixture\"\nbase = \"default\"\n\n\
         [script]\n\
         element_gap = 4u\n\
         meter_gap = 6u\n\
         dots_gap = 8u\n\
         meter_label_role = title.panel\n\
         meter_value_role = caption\n",
    )
    .expect("the fixture theme must be writable");
    path
}

#[test]
fn the_stack_spends_the_themes_air_and_types_its_meter_by_the_themes_roles() {
    theme::set_viewport(H, 1.0);
    let _ = theme::load();
    let master = probe();

    // ---- the master's own numbers reach the glass -----------------------
    // Only the SIZES are the master's business; that they are these
    // particular tokens is what the difference below proves.
    assert!(
        master.element_gap > 0.0 && master.meter_gap > 0.0 && master.dots_gap > 0.0,
        "the master declares no air between stack elements — nothing to prove"
    );
    // The two halves of a meter line are two ROLES, so they are two
    // sizes: one base size for both is exactly the bug.
    let caption_px = px_of("type.caption.size").max(px_of("type.min_px"));
    let value_px = px_of("type.value.size").max(px_of("type.min_px"));
    assert!(
        (master.label.0 - caption_px).abs() < GRAIN,
        "the meter's label drew at {} px, not caption's {caption_px}",
        master.label.0
    );
    assert!(
        (master.value.0 - value_px).abs() < GRAIN,
        "the meter's readout drew at {} px, not value's {value_px}",
        master.value.0
    );
    assert!(
        (master.label.1 - caption_px * px_of("type.caption.tracking")).abs() < GRAIN,
        "the meter's label is not tracked by its role"
    );
    assert!(
        (master.value.1 - value_px * px_of("type.value.tracking")).abs() < GRAIN,
        "the meter's readout is not tracked by its role"
    );

    // ---- and now the same script under a theme that differs by five lines
    let path = fixture();
    let _ = theme::load_with(LoadRequest { path: Some(path.clone()), ..LoadRequest::default() });
    let fixed = probe();
    let _ = std::fs::remove_file(&path);

    // `script.element_gap`: three plain texts, so the implicit gap is the
    // only thing between them. Type and leading did not change, so the
    // whole difference in the run is the gap the theme spends twice.
    let stride = |p: &Probe| [p.text_run[1] - p.text_run[0], p.text_run[2] - p.text_run[1]];
    let (m, f) = (stride(&master), stride(&fixed));
    assert!(
        (m[0] - m[1]).abs() < GRAIN && (f[0] - f[1]).abs() < GRAIN,
        "the stack's gap is not the same between every pair"
    );
    assert!(
        ((f[0] - m[0]) - (fixed.element_gap - master.element_gap)).abs() < GRAIN,
        "raising script.element_gap by {} moved the next element by {}",
        fixed.element_gap - master.element_gap,
        f[0] - m[0]
    );

    // `script.meter_gap`: the element's own claim, spent on BOTH sides of
    // it — and it overrides the implicit gap rather than adding to it,
    // which is why the fixture moved element_gap the other way.
    let span = |p: &Probe| p.around_meter[1] - p.around_meter[0];
    assert!(
        ((span(&fixed) - span(&master)) - 2.0 * (fixed.meter_gap - master.meter_gap)).abs()
            < GRAIN,
        "a meter's neighbours do not stand off by script.meter_gap"
    );

    // `script.dots_gap`: the block's first cell is the block's top.
    let drop = |p: &Probe| p.dots_top - p.above_dots;
    assert!(
        ((drop(&fixed) - drop(&master)) - (fixed.dots_gap - master.dots_gap)).abs() < GRAIN,
        "the dots grid does not stand off by script.dots_gap"
    );

    // `script.meter_label_role` / `script.meter_value_role`: the fixture
    // swaps the two bindings for roles of other sizes, and the meter is
    // retyped without one line of the script changing.
    let title_px = px_of("type.title.panel.size").max(px_of("type.min_px"));
    let caption_px = px_of("type.caption.size").max(px_of("type.min_px"));
    assert!(
        (fixed.label.0 - title_px).abs() < GRAIN,
        "rebinding script.meter_label_role left the label at {} px",
        fixed.label.0
    );
    assert!(
        (fixed.value.0 - caption_px).abs() < GRAIN,
        "rebinding script.meter_value_role left the readout at {} px",
        fixed.value.0
    );
    assert!(
        (fixed.label.1 - title_px * px_of("type.title.panel.tracking")).abs() < GRAIN,
        "the rebound label kept the old role's tracking"
    );
}
