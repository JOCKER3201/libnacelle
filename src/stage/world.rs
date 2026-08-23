//! One layout's world of boards, moved verbatim from the desktop's
//! refresh_boards!/def_of!/has_board!/all_boards! macros: home at
//! (0, 0), the horizontal row it sits on, and the two fixtures above
//! and below — SEARCH AND AI at (0, -1) and APPGRID at (0, 1) — which
//! exist whether or not the layout's file has anything on them.
//!
//! Each position holds its own instances, so a board is a full desktop
//! of its own rather than a copy of home: its own widgets, its own
//! second terminal, its own arrangement.

use crate::base::{Layout, LayoutMode, SizeTable};
use crate::layout::{board_key, BoardId, InstanceId, LayoutDef, ScreenKey};
use std::collections::{HashMap, HashSet};

pub struct BoardWorld {
    /// The selected layaut itself — what position (0, 0) shows. Held
    /// whole, instances of every board included, because it is also
    /// what the caller hands back to be saved.
    home: LayoutDef,
    /// The extra boards, keyed by their FOLDED position.
    boards: HashMap<BoardId, LayoutDef>,
    /// How far the horizontal row reaches: (left, right).
    ext: (u32, u32),
    current: BoardId,
    /// What a position without a board shows: rectangles, and none of
    /// them. Owned so `def` can always answer a reference.
    empty: LayoutDef,
}

impl BoardWorld {
    /// Builds the world of a layout. Boards named by the file keep
    /// their own sizes when they name any and share the layout's
    /// otherwise; the two fixtures exist regardless.
    pub fn new(home: LayoutDef) -> Self {
        let mut w = Self {
            home: LayoutDef::default(),
            boards: HashMap::new(),
            ext: (0, 0),
            current: (0, 0),
            empty: LayoutDef::empty_board(),
        };
        w.rebuild(home);
        w
    }

    /// Re-reads the world from a (new) layout, keeping the current
    /// position when it still exists — home is the one place that
    /// always does.
    pub fn rebuild(&mut self, home: LayoutDef) {
        self.boards.clear();
        let (mut l, mut r) = (0u32, 0u32);
        for (k, bd) in &home.boards {
            let (x, y) = *k;
            if y == 0 && x < 0 {
                l = l.max(-x as u32);
            } else if y == 0 {
                r = r.max(x as u32);
            }
            // A board is whatever its section holds — rectangles or
            // flexbox columns — over the instances that stand on it; a
            // board that names its own sizes uses them, the rest share
            // the layout's.
            self.boards.insert(
                *k,
                LayoutDef {
                    base: bd.base.clone(),
                    sizes: if bd.sizes.is_empty() {
                        home.sizes.clone()
                    } else {
                        bd.sizes.clone()
                    },
                    instances: home.instances.clone(),
                    ..LayoutDef::default()
                },
            );
        }
        // The fixtures exist whether or not the file has anything on
        // them. Under the project's own compositor these two will live
        // in the OVERLAY layer, above every window; here they are
        // ordinary boards that ride over home when opened.
        for k in [(0, -1), (0, 1)] {
            self.boards.entry(k).or_insert_with(|| LayoutDef {
                base: LayoutMode::Rects,
                sizes: home.sizes.clone(),
                instances: home.instances.clone(),
                ..LayoutDef::default()
            });
        }
        self.ext = (l, r);
        self.home = home;
        if !self.has_board(self.current) {
            self.current = (0, 0);
        }
    }

    pub fn current(&self) -> BoardId {
        self.current
    }

    /// Moves the current position; a position outside the world is
    /// refused and the current stands.
    pub fn set_current(&mut self, k: BoardId) {
        if self.has_board(k) {
            self.current = k;
        }
    }

    /// Whether the gesture can stand on this position: any place on
    /// the row, and the fixed top and bottom above and below EACH of
    /// them — (x, ±1) shows the one fixture, x only remembers where
    /// the hand came from.
    pub fn has_board(&self, k: BoardId) -> bool {
        let (x, y) = k;
        let (l, r) = self.ext;
        x >= -(l as i32) && x <= r as i32 && (-1..=1).contains(&y)
    }

    /// The definition a position shows — home for (0, 0), the folded
    /// board for anything else, the empty board for a position that
    /// exists but holds nothing.
    pub fn def(&self, k: BoardId) -> &LayoutDef {
        let key = board_key(k);
        if key == (0, 0) {
            &self.home
        } else {
            self.boards.get(&key).unwrap_or(&self.empty)
        }
    }

    pub fn current_def(&self) -> &LayoutDef {
        self.def(self.current)
    }

    /// The whole selected layout, instances of every board included —
    /// what an editor hands to the store to be saved.
    pub fn layout(&self) -> &LayoutDef {
        &self.home
    }

    /// The rectangles ONE board shows at this window size. Every def
    /// carries the whole layout's instances, so which of them belong
    /// here is decided by the position, not by the def.
    pub fn solve(
        &self,
        k: BoardId,
        w: f32,
        h: f32,
        pad: f32,
        screen: ScreenKey,
        t: &SizeTable,
    ) -> Layout {
        let key = board_key(k);
        self.def(key).solve_on(key, w, h, pad, screen, t)
    }

    /// How far the horizontal row reaches: (left, right).
    pub fn arms(&self) -> (u32, u32) {
        self.ext
    }

    /// Every board that exists, home first, the rest sorted — the
    /// order every scan and every save walks, so it can never depend
    /// on a hash map's whim.
    pub fn ids(&self) -> Vec<BoardId> {
        let mut ids: Vec<BoardId> = vec![(0, 0)];
        let mut rest: Vec<BoardId> = self.boards.keys().copied().collect();
        rest.sort();
        ids.extend(rest);
        ids
    }

    /// Which INSTANCES are visible on ANY board at this window size —
    /// the presence scan widget lifetime hangs on (u3 §5 trap 1). An
    /// instance whose rectangle starts inside the window on some board
    /// is present; one hidden everywhere is not, and its widget must
    /// not run. The x < w rule is the OFF_SPEC convention: hidden
    /// panels park far outside.
    ///
    /// Per INSTANCE and not per widget, because two terminals are two
    /// shells: closing one of them may not take the other's process
    /// with it.
    pub fn present(
        &self,
        w: f32,
        h: f32,
        pad: f32,
        screen: ScreenKey,
        t: &SizeTable,
    ) -> Vec<InstanceId> {
        let mut seen: HashSet<InstanceId> = HashSet::new();
        let mut out: Vec<InstanceId> = Vec::new();
        for k in self.ids() {
            for p in self.solve(k, w, h, pad, screen, t).iter() {
                if p.rect.x < w && seen.insert(p.id) {
                    out.push(p.id);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::base::{Panel, PanelSpec};
    use crate::layout::{BoardDef, InstanceList};

    fn world_with(boards: &[BoardId]) -> BoardWorld {
        crate::flex::install_test_registry();
        let mut home = LayoutDef::from_base(LayoutMode::Flex);
        home.boards = boards
            .iter()
            .map(|k| (*k, BoardDef { base: LayoutMode::Rects, sizes: Vec::new() }))
            .collect();
        BoardWorld::new(home)
    }

    #[test]
    fn the_fixtures_always_exist_and_home_is_first() {
        let w = world_with(&[]);
        assert_eq!(w.arms(), (0, 0));
        assert_eq!(w.ids(), vec![(0, 0), (0, -1), (0, 1)]);
        assert!(w.has_board((0, 0)) && w.has_board((0, -1)) && w.has_board((0, 1)));
        assert!(!w.has_board((1, 0)) && !w.has_board((0, 2)));
    }

    #[test]
    fn every_row_position_carries_the_one_fixture() {
        let w = world_with(&[(-1, 0), (1, 0), (2, 0)]);
        assert_eq!(w.arms(), (1, 2));
        // (2, 1) folds to the ONE bottom board; (2, 2) is not a place.
        assert!(w.has_board((2, 1)));
        assert!(!w.has_board((2, 2)));
        assert!(std::ptr::eq(w.def((2, 1)), w.def((0, 1))), "every (x, 1) shows (0, 1)");
        assert!(std::ptr::eq(w.def((-1, -1)), w.def((0, -1))));
    }

    #[test]
    fn a_position_that_exists_but_holds_nothing_is_the_empty_board() {
        let w = world_with(&[(1, 0)]);
        // The fixture positions exist with no content: no instance
        // stands on them, so the board solves to nothing at all.
        let d = w.def((0, -1));
        assert!(matches!(d.base, LayoutMode::Rects));
        assert!(w.solve((0, -1), 1920.0, 1080.0, 8.0, (0, 0, 0), &crate::base::size_table())
            .is_empty());
    }

    #[test]
    fn a_shrunken_world_returns_the_wanderer_home() {
        let mut w = world_with(&[(1, 0), (2, 0)]);
        w.set_current((2, 0));
        assert_eq!(w.current(), (2, 0));
        let mut smaller = LayoutDef::from_base(LayoutMode::Flex);
        smaller.boards =
            vec![((1, 0), BoardDef { base: LayoutMode::Rects, sizes: Vec::new() })];
        w.rebuild(smaller);
        assert_eq!(w.current(), (0, 0), "home is the one place that always exists");
        assert_eq!(w.arms(), (0, 1));
    }

    #[test]
    fn set_current_refuses_a_place_that_is_not_there() {
        let mut w = world_with(&[]);
        w.set_current((3, 0));
        assert_eq!(w.current(), (0, 0));
        w.set_current((0, 1));
        assert_eq!(w.current(), (0, 1));
    }

    #[test]
    fn boards_share_the_layouts_sizes_unless_they_name_their_own() {
        // The registry has to exist before a Panel does — see
        // `flex::install_test_registry`.
        crate::flex::install_test_registry();
        let mut home = LayoutDef::from_base(LayoutMode::Flex);
        home.sizes = vec![(Panel::all()[0], 9.0, 5.0)];
        home.boards = vec![
            ((1, 0), BoardDef { base: LayoutMode::Rects, sizes: Vec::new() }),
            (
                (2, 0),
                BoardDef {
                    base: LayoutMode::Rects,
                    sizes: vec![(Panel::all()[0], 20.0, 10.0)],
                },
            ),
        ];
        let w = BoardWorld::new(home);
        assert_eq!(w.def((1, 0)).sizes[0].1, 9.0, "no own sizes: the layout's table");
        assert_eq!(w.def((2, 0)).sizes[0].1, 20.0, "own sizes win");
    }

    /// Goal A: a second board is a desktop of its own, with its OWN
    /// widgets. A board's instances are its own — solving home must
    /// never place them, and solving the board must never place home's.
    #[test]
    fn every_board_places_only_its_own_instances() {
        crate::flex::install_test_registry();
        let w01 = Panel::from_name("w01").unwrap();
        let w07 = Panel::from_name("w07").unwrap();
        let mut insts = InstanceList::new();
        let at_home = insts.add(w01, (0, 0), None);
        let away = insts.add(w07, (1, 0), Some(PanelSpec { x: 5.0, y: 5.0, w: 50.0, h: 50.0 }));
        let mut home = LayoutDef::from_base(LayoutMode::Flex);
        home.boards = vec![((1, 0), BoardDef { base: LayoutMode::Rects, sizes: Vec::new() })];
        home.instances = insts;
        let world = BoardWorld::new(home);
        let t = crate::base::size_table();
        let (w, h) = (1920.0, 1080.0);
        let at0 = world.solve((0, 0), w, h, 8.0, (0, 0, 0), &t);
        let at1 = world.solve((1, 0), w, h, 8.0, (0, 0, 0), &t);
        assert_eq!(at0.len(), 1);
        assert_eq!(at1.len(), 1);
        assert!(at0.of(at_home).x < w && at0.of(away).x >= w);
        assert!(at1.of(away).x < w && at1.of(at_home).x >= w);
        // Both are present somewhere, which is what keeps both widgets
        // alive.
        let present = world.present(w, h, 8.0, (0, 0, 0), &t);
        assert!(present.contains(&at_home) && present.contains(&away));
    }
}
