//! Named `.layaut` files on a filesystem (u3 §3.2 `LayautStore`).
//!
//! Reads from every root of an [`AssetRoots`] in order; writes only
//! ever into its write root. Editing a layout that came from a system
//! directory copies it into the user's on the first save, rather than
//! failing on a path only root can write.
//!
//! This is also where a version 1 file becomes a version 2 one — once,
//! with a copy of the original left beside it (see [`LayautStore::
//! migrate`]). A user's saved layouts are the one thing in the program
//! he cannot make again from memory, so the rewrite keeps the old bytes
//! rather than trusting itself.

use super::def::{BoardDef, BoardId, LayoutDef, ResOverride, ScreenKey};
use super::instance::{InstanceId, InstanceList};
use super::layaut;
use crate::assets::AssetRoots;
use crate::base::{LayoutMode, PanelSpec};
use std::path::{Path, PathBuf};

/// The suffix the pre-migration copy of a layaut keeps. Not a `.layaut`
/// itself, so the backup never turns up in the list of layouts to pick.
pub const BACKUP_SUFFIX: &str = ".layaut.v1";

pub struct LayautStore {
    roots: AssetRoots,
}

impl LayautStore {
    pub fn new(roots: AssetRoots) -> Self {
        Self { roots }
    }

    /// "default" plus every `<name>.layaut` on the search path, first
    /// root holding a name wins. Dotfiles are the toolkit's own
    /// bookkeeping and are not offered.
    pub fn list(&self) -> Vec<String> {
        let mut out = vec!["default".to_string()];
        for dir in self.roots.dirs("layauts") {
            for stem in list_stems(&dir, "layaut") {
                if stem != "default" && !out.contains(&stem) {
                    out.push(stem);
                }
            }
        }
        out
    }

    /// None when the name is not installed. "default" with no file is
    /// the generated responsive arrangement, carrying the size table it
    /// was composed from: the ref/min heights belong to the LAYOUT — a
    /// .layaut names its own in its ref/min column — and the generated
    /// one has no numbers of its own, so it hands on what the installed
    /// addons declared, spelled out rather than left empty.
    ///
    /// A version 1 file of the user's own is migrated on the way past
    /// (see [`Self::migrate`]); one that lives in a system directory is
    /// read as version 1 every time, which gives the same instances
    /// every time because their ids come from the file's own order.
    pub fn load(&self, name: &str) -> Option<LayoutDef> {
        // Best effort: a read-only home, a full disk or a file someone
        // else owns must not stop the layout from loading.
        let _ = self.migrate(name);
        if let Some(text) = self
            .roots
            .find("layauts", &format!("{name}.layaut"))
            .and_then(|p| std::fs::read_to_string(p).ok())
        {
            return Some(layaut::parse(&text, name));
        }
        if name == "default" {
            return Some(LayoutDef {
                base: LayoutMode::Flex,
                sizes: crate::flex::builtin_sizes(),
                instances: crate::flex::default_instances(),
                ..LayoutDef::default()
            });
        }
        None
    }

    /// The path of the user's own copy of a layaut, whether or not it
    /// exists yet.
    fn user_path(&self, name: &str) -> PathBuf {
        self.roots.write_dir("layauts").join(format!("{name}.layaut"))
    }

    /// Rewrites the user's own version 1 file as version 2, ONCE.
    ///
    /// Returns whether it did anything. The original is copied to
    /// `<name>[BACKUP_SUFFIX]` first and never overwritten afterwards,
    /// so a second migration (of a file the user restored by hand, say)
    /// cannot destroy the first backup. Files in the system directories
    /// are left alone: they are not ours to rewrite, and the version 1
    /// reader gives them the same instances on every start anyway.
    pub fn migrate(&self, name: &str) -> std::io::Result<bool> {
        let path = self.user_path(name);
        let Ok(text) = std::fs::read_to_string(&path) else { return Ok(false) };
        if !layaut::is_legacy(&text) {
            return Ok(false);
        }
        let backup = self.roots.write_dir("layauts").join(format!("{name}{BACKUP_SUFFIX}"));
        if !backup.exists() {
            std::fs::write(&backup, &text)?;
        }
        // Read with the version 1 grammar, write with the current one:
        // one instance per placement, in the order the file named them,
        // so the arrangement that comes out is the arrangement that
        // went in.
        let def = layaut::parse(&text, name);
        std::fs::write(&path, layaut::write_file(&def))?;
        Ok(true)
    }

    /// The layaut file's current text: the user's copy, or the
    /// installed one it would be copied from on first save.
    fn read_text(&self, name: &str) -> Option<String> {
        std::fs::read_to_string(self.user_path(name)).ok().or_else(|| {
            self.roots
                .find("layauts", &format!("{name}.layaut"))
                .and_then(|p| std::fs::read_to_string(p).ok())
        })
    }

    /// The named layout as a model, empty when there is no file yet.
    fn read_def(&self, name: &str) -> LayoutDef {
        layaut::parse(&self.read_text(name).unwrap_or_default(), name)
    }

    /// Writes a whole edited layout — the general door, for an editor
    /// that has the model in hand. Comments in the previous file are
    /// not carried over; the narrower operations below exist for the
    /// cases where only one section changes.
    ///
    /// `def` is taken by value-and-back because writing a layout down
    /// is what turns its COMPOSED placements into saved ones
    /// ([`LayoutDef::materialize`]): the caller's copy has to learn the
    /// new identities, or it would go on holding ids the file does not
    /// have.
    pub fn save_layout(&self, name: &str, def: &mut LayoutDef) -> std::io::Result<()> {
        let dir = self.roots.ensure("layauts")?;
        def.materialize();
        std::fs::write(dir.join(format!("{name}.layaut")), layaut::write_file(def))
    }

    /// SAVE: the whole edited layout becomes the one arrangement every
    /// screen shares. The complete placement set is written as the base,
    /// scaled to each monitor's own pixels when it is read, so a second
    /// monitor can no longer hold half of one arrangement and half of
    /// another. NO per-screen `[WxH@D]` section is written — the old
    /// ones, and the divergence they caused, are dropped. The boards the
    /// file carries survive the rewrite.
    pub fn save_full(
        &self,
        name: &str,
        def: &mut LayoutDef,
        key: ScreenKey,
    ) -> std::io::Result<()> {
        let dir = self.roots.ensure("layauts")?;
        def.materialize();
        let old = self.read_def(name);
        let mut out = layaut::serialize_base(&def.instances, key);
        layaut::serialize_boards(&mut out, &old.boards, &def.instances);
        std::fs::write(dir.join(format!("{name}.layaut")), out)
    }

    /// SAVE: on the screen the base was created on, the base itself is
    /// rewritten with the full layout; on ANY OTHER screen only the
    /// changed instances are written into that screen's `[WxH@D]`
    /// section. The rest of the file always stays untouched.
    /// `def` is the caller's own model — the one it got from `load` and
    /// has been editing — and it is written back materialized, so its
    /// ids and the file's are the same ids afterwards. A caller that
    /// holds instance ids of its own should call
    /// [`LayoutDef::materialize`] first and follow the map it returns.
    pub fn save_overrides(
        &self,
        name: &str,
        key: ScreenKey,
        changes: &[(InstanceId, PanelSpec)],
        def: &mut LayoutDef,
    ) -> std::io::Result<()> {
        let dir = self.roots.ensure("layauts")?;
        let path = dir.join(format!("{name}.layaut"));
        let text = self.read_text(name).unwrap_or_default();
        let (base, _, _) = layaut::split_raw(&text);

        if def.base_screen == Some(key) {
            // Editing on the base's own screen: rewrite the base in full.
            def.materialize();
            let mut out = layaut::serialize_base(&def.instances, key);
            layaut::serialize_sections(&mut out, &def.overrides, &def.instances);
            layaut::serialize_boards(&mut out, &def.boards, &def.instances);
            return std::fs::write(path, out);
        }

        // Another screen: merge the changes into its section. Merged
        // BEFORE materialising, so ids the caller passes in are still
        // the ids it is holding.
        let sec = match def.overrides.iter_mut().find(|o| (o.w, o.h, o.diag) == key) {
            Some(s) => s,
            None => {
                def.overrides.push(ResOverride {
                    w: key.0,
                    h: key.1,
                    diag: key.2,
                    rects: Vec::new(),
                });
                def.overrides.last_mut().unwrap()
            }
        };
        for (id, spec) in changes {
            sec.rects.retain(|(i, _)| i != id);
            sec.rects.push((*id, *spec));
        }
        def.materialize();

        let mut out = preserved_base(
            &base,
            &def.instances,
            "# nacelle layout: per-screen overrides on top of the default layout.\n",
        );
        layaut::serialize_sections(&mut out, &def.overrides, &def.instances);
        layaut::serialize_boards(&mut out, &def.boards, &def.instances);
        std::fs::write(path, out)
    }

    /// Rewrites the boards of the named layout, leaving everything else
    /// in its file alone. The shared tail of the three board operations.
    fn write_boards(
        &self,
        name: &str,
        boards: Vec<(BoardId, BoardDef)>,
        insts: &mut InstanceList,
    ) -> std::io::Result<()> {
        let dir = self.roots.ensure("layauts")?;
        let text = match std::fs::read_to_string(self.user_path(name)) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                match self.roots.find("layauts", &format!("{name}.layaut")) {
                    Some(p) => std::fs::read_to_string(p)?,
                    None => String::new(),
                }
            }
            Err(e) => return Err(e),
        };
        let (base, _, _) = layaut::split_raw(&text);
        let def = layaut::parse(&text, name);
        let boards = layaut::normalize_boards(boards, insts);
        // No base yet: the boards hang off the built-in default layout,
        // and the file says only what it knows.
        let mut out = preserved_base(
            &base,
            insts,
            "# nacelle layout: boards on top of the default layout.\n",
        );
        layaut::serialize_sections(&mut out, &def.overrides, insts);
        layaut::serialize_boards(&mut out, &boards, insts);
        std::fs::write(dir.join(format!("{name}.layaut")), out)
    }

    /// SAVE while on a board: that board's instances at the rectangles
    /// the grid editor gave them. Instances the caller did not name are
    /// no longer on the board, and go.
    pub fn set_board(
        &self,
        name: &str,
        k: BoardId,
        rects: &[(InstanceId, PanelSpec)],
    ) -> std::io::Result<()> {
        let mut def = self.read_def(name);
        for id in board_ids(&def.instances, k) {
            if !rects.iter().any(|(i, _)| *i == id) {
                def.instances.remove(id);
            }
        }
        for (id, spec) in rects {
            def.instances.set_board(*id, k);
            def.instances.set_rect(*id, Some(*spec));
        }
        let mut boards = def.boards.clone();
        boards.retain(|(i, _)| *i != k);
        // The grid editor speaks rectangles, so a board saved from it is
        // a rectangle board.
        boards.push((k, BoardDef { base: LayoutMode::Rects, sizes: Vec::new() }));
        self.write_boards(name, boards, &mut def.instances)
    }

    /// A new, empty board at the given end of the horizontal row:
    /// negative is left, positive right. Only the row grows — the top
    /// and bottom boards are fixtures, one each, like home.
    pub fn add_board(&self, name: &str, side: i8) -> std::io::Result<()> {
        let mut def = self.read_def(name);
        let s: i32 = if side < 0 { -1 } else { 1 };
        let next = def
            .boards
            .iter()
            .filter_map(|(id, _)| (id.1 == 0 && id.0 * s > 0).then_some(id.0 * s))
            .max()
            .unwrap_or(0)
            + 1;
        let mut boards = def.boards.clone();
        boards.push((
            (next * s, 0),
            BoardDef { base: LayoutMode::Rects, sizes: Vec::new() },
        ));
        self.write_boards(name, boards, &mut def.instances)
    }

    /// Removes a horizontal board; the ones beyond it close ranks,
    /// which normalisation does on the way out. The top and bottom
    /// boards are permanent and stay whatever is asked.
    pub fn remove_board(&self, name: &str, k: BoardId) -> std::io::Result<()> {
        if k.1 != 0 {
            return Ok(());
        }
        let mut def = self.read_def(name);
        let mut boards = def.boards.clone();
        boards.retain(|(i, _)| *i != k);
        // The widgets that stood there go with it; their ids are
        // retired and never handed out again.
        def.instances.remove_board(k);
        self.write_boards(name, boards, &mut def.instances)
    }

    /// Deletes just the `[WxH@D]` section of one layaut, leaving its
    /// base, its other screens and its boards untouched. The inverse of
    /// save_overrides.
    pub fn clear_screen_section(&self, name: &str, key: ScreenKey) -> std::io::Result<()> {
        let dir = self.roots.ensure("layauts")?;
        let text = self.read_text(name).unwrap_or_default();
        let (base, _, _) = layaut::split_raw(&text);
        let mut def = layaut::parse(&text, name);
        let before = def.overrides.len();
        def.overrides.retain(|o| (o.w, o.h, o.diag) != key);
        if def.overrides.len() == before {
            // Nothing pinned for this screen: the file is left alone.
            return Ok(());
        }
        let mut out = preserved_base(
            &base,
            &def.instances,
            "# nacelle layout: per-screen overrides on top of the default layout.\n",
        );
        layaut::serialize_sections(&mut out, &def.overrides, &def.instances);
        layaut::serialize_boards(&mut out, &def.boards, &def.instances);
        std::fs::write(dir.join(format!("{name}.layaut")), out)
    }
}

/// The base of a file as it will be written back: the user's own text
/// (comments and all) with the bookkeeping lines brought up to date, or
/// the given banner when there is no base yet.
fn preserved_base(base: &str, insts: &InstanceList, banner: &str) -> String {
    let trimmed = base.trim_end();
    let text = if trimmed.is_empty() {
        banner.to_string()
    } else {
        format!("{trimmed}\n")
    };
    layaut::with_header(&text, insts)
}

/// The ids standing on one board.
fn board_ids(insts: &InstanceList, k: BoardId) -> Vec<InstanceId> {
    insts.on_board(k).into_iter().map(|i| i.id).collect()
}

/// Stems of `<stem>.<ext>` files in a directory, dotfiles excluded.
fn list_stems(dir: &Path, ext: &str) -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let p = entry.path();
            let matches = p.is_file()
                && p.extension()
                    .and_then(|e| e.to_str())
                    .map(|e| e.eq_ignore_ascii_case(ext))
                    .unwrap_or(false);
            if matches {
                if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                    // Dotfiles are the toolkit's own bookkeeping — the
                    // extra widget boards live in .board<k>.layaut —
                    // and are not offered as selectable layouts.
                    if stem.starts_with('.') {
                        continue;
                    }
                    out.push(stem.to_string());
                }
            }
        }
    }
    out.sort();
    out
}
