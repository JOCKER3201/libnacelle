//! libnacelle — the toolkit the nacelle project is built on.
//!
//! One crate, the way libcosmic is one crate for COSMIC: everything a
//! nacelle application needs, from the pixels up.
//!
//! * [`draw`], [`font`], [`theme`] — the drawing primitives, the glyph
//!   atlas and colours.
//! * [`sdf`] — the CPU reference of the vector core's distance field:
//!   the formulas the renderer's `fs_shape` computes, provable without
//!   a GPU.
//! * [`icon`] — SVG icons, rasterized once into a coverage mask and
//!   packed into [`font::FontSystem`]'s shared atlas beside the glyphs
//!   — not a distance field per icon (K8).
//! * [`base`] — geometry, the drawing context, the panel model and the
//!   widget registry. Re-exported at the root, so `nacelle::Rect` works.
//! * [`flex`] — the responsive layout engine (the algorithm web pages
//!   use), recomputed from the actual window size every frame.
//! * [`focus`] — keyboard focus: the per-world chain controls register
//!   into, Tab/arrow navigation, neutral key events and the shortcut
//!   registry.
//! * [`clipboard`] — the clipboard seam: the trait the application's
//!   backend implements, and the process-local fallback that keeps
//!   copy/paste alive without one.
//! * [`channel`] — what one widget tells another: named values the host
//!   holds, so two compiled widgets in two `.so` files can agree on a
//!   fact without sharing memory.
//! * [`settings`] — what an addon's user asked of it: the host reads
//!   the RON file and the addon parses it into its own type, so a
//!   compiled widget never opens a file and never holds a path.
//! * [`pointer`] — where the pointer is and who may see it: the rule
//!   that a control with something drawn over it is not the control
//!   under the hand.
//! * [`object`] — reusable on-screen objects: windows and dialogs,
//!   buttons, sliders, drop-downs, checkboxes, and the frame put
//!   around windows the application does not own.
//! * [`view`] — the model/view core: which rows a viewport shows, the
//!   scroll offset and its physics, and the hit list a view records
//!   while it draws.
//! * [`ui`] — the drawing vocabulary widgets are composed from.
//! * [`widget`] — the contract widgets are written against and that an
//!   application drives them through. Re-exported at the root.
//! * [`script`] — widgets written as Rhai scripts, rendered through
//!   that same vocabulary.
//! * [`term`] — terminal emulation, a pure VT state machine.
//! * [`telemetry`] — the system data model widgets render.
//! * [`sound`] — sound events, themes and mixing.
//! * [`runtime`] — the process-wide state, and how a compiled plugin
//!   shares the host's copy of it instead of quietly getting its own.
//! * [`plugin`] — the host side of the plugin boundary: the functions a
//!   plugin draws through, and the wrapper that makes one look like any
//!   other widget.
//! * [`wm`] — the window-management vocabulary (JEDEN MODEL OKNA): what
//!   can be asked of a window and what came back, the same for a window
//!   built into the desktop and a window of an outside application. The
//!   application supplies the backend that actually speaks to a
//!   compositor, the same pattern [`clipboard`] runs on.
//!
//! Everything here is platform-independent. Creating a window, opening a
//! PTY, collecting telemetry and handing audio frames to a device are
//! the platform's job and live in the application.

pub mod assets;
pub mod base;
pub mod channel;
pub mod clipboard;
pub mod corner;
pub mod deco;
pub mod draw;
pub mod layout;
pub use layout::flex;
pub mod focus;
pub mod font;
pub mod icon;
pub mod motion;
pub mod num;
pub mod object;
pub mod plugin;
pub mod pointer;
pub mod runtime;
pub mod script;
pub mod sdf;
pub mod settings;
pub mod sound;
pub mod stage;
pub mod telemetry;
pub mod term;
pub mod theme;
pub mod ui;
pub mod view;
pub mod widget;
pub mod wm;

pub use base::*;
pub use motion::{Crossfade, Easing, Effect};
pub use widget::*;
