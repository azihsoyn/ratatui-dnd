//! Sorting, built on the ground floor.
//!
//! Containers you can drag between, insertion slots measured from the
//! rectangles you actually drew, and a keyboard carry that does what a
//! mouse drag does without a mouse.
//! Rendering stays with the caller; while something is held,
//! leave it out of what you register, and ask [`Sortable::over`] where
//! to open the gap.

use crate::interact::{Did, Drag};
use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

/// Which of `len + 1` gaps a cursor over a container means, measured
/// from where its items were actually drawn, in drawing order.
///
/// Rows of one item read by their vertical midline, rows of several by
/// each item's horizontal midline — so the same rule serves a vertical
/// list of any heights, a horizontal strip, and a row-major grid,
/// without being told which one it is looking at.
pub fn slot(items: &[Rect], x: u16, y: u16) -> usize {
    let mut i = 0;
    while i < items.len() {
        // A row is every consecutive item that overlaps it vertically.
        let top = items[i].y;
        let mut bottom = items[i].bottom();
        let mut j = i + 1;
        while j < items.len() && items[j].y < bottom {
            bottom = bottom.max(items[j].bottom());
            j += 1;
        }
        if y < top {
            return i;
        }
        if y < bottom {
            let row = &items[i..j];
            if row.len() == 1 {
                return i + (y >= row[0].y + row[0].height / 2) as usize;
            }
            return i + row.iter().filter(|r| x >= r.x + r.width / 2).count();
        }
        i = j;
    }
    items.len()
}

/// What a mouse event amounted to, with the drop already resolved to a
/// container and a slot.
#[derive(Debug, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Act<C, K> {
    Nothing,
    Click(K),
    Lift(K),
    Move,
    Drop { key: K, container: C, slot: usize },
}

/// What happened, told after the fact: the hook side of the crate.
///
/// Everything a drag does — by mouse or keyboard — is also queued here
/// as data, for whoever wants to watch rather than steer: an undo log,
/// autosave, a sound, a peer to sync. Drain with
/// [`Sortable::hooks`] once a frame. `from` is where the held thing
/// was picked up; [`Hook::Target`] fires once each time the gap moves
/// to a new place, not on every mouse event.
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum Hook<C, K> {
    /// Something was picked up, by either hand.
    Grab { key: K, from: Option<(C, usize)> },
    /// The gap moved: the held thing now hovers over this place.
    Target { key: K, container: C, slot: usize },
    /// Put down here, having come from there.
    Drop {
        key: K,
        from: Option<(C, usize)>,
        container: C,
        slot: usize,
    },
    /// Let go without a drop — esc, or a mouse press ending a carry.
    Cancel { key: K },
    /// Pressed and released without ever dragging.
    Click { key: K },
}

struct Con<C, K> {
    id: C,
    area: Rect,
    items: Vec<(K, Rect)>,
    /// Index the first registered item has in the container's full
    /// list. A scrolled view registers only what is on screen; slots
    /// still come back in full-list terms.
    start: usize,
}

/// Something held by the keyboard rather than the mouse. It has no
/// cursor; its position is a slot, moved a step at a time.
struct Carry<K> {
    key: K,
    con: usize,
    slot: usize,
}

/// The whole of a sortable surface: any number of containers, each
/// holding items wherever the caller drew them.
///
/// Register every container each frame with [`container`](Self::container),
/// in the places things were actually drawn; hand every mouse event to
/// [`on_mouse`](Self::on_mouse); use [`lift`](Self::lift) /
/// [`shift`](Self::shift) / [`put`](Self::put) for the keyboard. While
/// anything is held, skip it when registering — it is in the hand, not
/// in a list — and draw a gap where [`over`](Self::over) says.
pub struct Sortable<C, K> {
    drag: Drag<K>,
    carry: Option<Carry<K>>,
    cons: Vec<Con<C, K>>,
    /// The story so far, waiting to be drained by [`hooks`](Self::hooks).
    queue: Vec<Hook<C, K>>,
    /// Where the held thing was picked up.
    from: Option<(C, usize)>,
    /// The last place a [`Hook::Target`] was told about, so the gap
    /// moving reports once per move rather than once per event.
    aimed: Option<(C, usize)>,
}

impl<C: Clone + PartialEq, K: Clone + PartialEq> Sortable<C, K> {
    pub fn new() -> Self {
        Self {
            drag: Drag::new(),
            carry: None,
            cons: Vec::new(),
            queue: Vec::new(),
            from: None,
            aimed: None,
        }
    }

    /// Everything that happened since the last drain, in order. Call
    /// once a frame; what you do with it — log, save, sync, nothing —
    /// is yours.
    pub fn hooks(&mut self) -> Vec<Hook<C, K>> {
        std::mem::take(&mut self.queue)
    }

    /// Where this key was last registered, in full-list terms.
    fn locate(&self, key: &K) -> Option<(C, usize)> {
        self.cons.iter().find_map(|con| {
            con.items
                .iter()
                .position(|(k, _)| k == key)
                .map(|i| (con.id.clone(), con.start + i))
        })
    }

    /// Report the gap's place if it moved since last reported.
    fn aim(&mut self, key: K) {
        let Some(t) = self.over() else { return };
        if self.aimed.as_ref() != Some(&t) {
            self.aimed = Some(t.clone());
            self.queue.push(Hook::Target {
                key,
                container: t.0,
                slot: t.1,
            });
        }
    }

    /// Say where a container and its items are this frame, in drawing
    /// order. A container already known is refreshed in place, so the
    /// order containers were first registered in is the order the
    /// keyboard walks them in.
    pub fn container(&mut self, id: C, area: Rect, items: &[(K, Rect)]) {
        self.window(id, area, items, 0);
    }

    /// [`container`](Self::container) for a scrolled view: only what is
    /// on screen can be registered, so say what index the first
    /// registered item has in the full list. Slots keep coming back in
    /// full-list terms, and a keyboard carry may step one past either
    /// edge of the window — scroll to keep the gap visible and the
    /// window follows it.
    pub fn window(&mut self, id: C, area: Rect, items: &[(K, Rect)], start: usize) {
        match self.cons.iter_mut().find(|c| c.id == id) {
            Some(c) => {
                c.area = area;
                c.items = items.to_vec();
                c.start = start;
            }
            None => self.cons.push(Con {
                id,
                area,
                items: items.to_vec(),
                start,
            }),
        }
    }

    /// Feed every mouse event through. A drop comes back already
    /// resolved to the container and slot it means.
    pub fn on_mouse(&mut self, ev: MouseEvent) -> Act<C, K> {
        // The mouse takes precedence: pressing anywhere ends a carry.
        if matches!(ev.kind, MouseEventKind::Down(MouseButton::Left))
            && let Some(c) = self.carry.take()
        {
            self.queue.push(Hook::Cancel { key: c.key });
            self.aimed = None;
            self.from = None;
        }
        let hit = self.hit(ev.column, ev.row);
        match self.drag.on_mouse(ev, hit) {
            Did::Nothing => Act::Nothing,
            Did::Click(k) => {
                self.queue.push(Hook::Click { key: k.clone() });
                Act::Click(k)
            }
            Did::Lift(k) => {
                self.from = self.locate(&k);
                self.queue.push(Hook::Grab {
                    key: k.clone(),
                    from: self.from.clone(),
                });
                self.aim(k.clone());
                Act::Lift(k)
            }
            Did::Move => {
                if let Some(k) = self.drag.moving().cloned() {
                    self.aim(k);
                }
                Act::Move
            }
            Did::Drop { key, x, y } => {
                self.aimed = None;
                match self.place(x, y) {
                    Some((container, slot)) => {
                        self.queue.push(Hook::Drop {
                            key: key.clone(),
                            from: self.from.take(),
                            container: container.clone(),
                            slot,
                        });
                        Act::Drop {
                            key,
                            container,
                            slot,
                        }
                    }
                    None => {
                        self.from = None;
                        Act::Nothing
                    }
                }
            }
        }
    }

    /// Pick something up with the keyboard, from wherever it was last
    /// registered. Ignored while the mouse already holds something.
    pub fn lift(&mut self, key: K) {
        if self.drag.moving().is_some() {
            return;
        }
        for (ci, con) in self.cons.iter().enumerate() {
            if let Some(i) = con.items.iter().position(|(k, _)| *k == key) {
                self.from = Some((con.id.clone(), con.start + i));
                self.carry = Some(Carry {
                    key: key.clone(),
                    con: ci,
                    slot: con.start + i,
                });
                self.queue.push(Hook::Grab {
                    key: key.clone(),
                    from: self.from.clone(),
                });
                self.aim(key);
                return;
            }
        }
    }

    /// Step a carried thing's slot within its container. The caller
    /// decides what a step means: -1/+1 walks a list, -columns/+columns
    /// walks a grid by rows.
    pub fn shift(&mut self, delta: isize) {
        let Some(c) = &mut self.carry else { return };
        let end = self
            .cons
            .get(c.con)
            .map_or(0, |con| con.start + con.items.len());
        c.slot = (c.slot as isize + delta).clamp(0, end as isize) as usize;
        let key = c.key.clone();
        self.aim(key);
    }

    /// Step a carried thing to another container, keeping its slot
    /// where the new container is long enough for it.
    pub fn shift_container(&mut self, delta: isize) {
        let Some(c) = &mut self.carry else { return };
        if self.cons.is_empty() {
            return;
        }
        c.con = (c.con as isize + delta).clamp(0, self.cons.len() as isize - 1) as usize;
        let to = &self.cons[c.con];
        c.slot = c.slot.clamp(to.start, to.start + to.items.len());
        let key = c.key.clone();
        self.aim(key);
    }

    /// Put a carried thing down where it is. The caller moves its own
    /// data; clamp the slot on insert, since the registered picture is
    /// a frame old.
    pub fn put(&mut self) -> Option<(K, C, usize)> {
        let c = self.carry.take()?;
        self.aimed = None;
        let from = self.from.take();
        let id = self.cons.get(c.con)?.id.clone();
        self.queue.push(Hook::Drop {
            key: c.key.clone(),
            from,
            container: id.clone(),
            slot: c.slot,
        });
        Some((c.key, id, c.slot))
    }

    /// Let go of whatever is held — mouse or keyboard — without a drop.
    pub fn cancel(&mut self) {
        if let Some(k) = self.held().cloned() {
            self.queue.push(Hook::Cancel { key: k });
        }
        self.carry = None;
        self.drag.cancel();
        self.from = None;
        self.aimed = None;
    }

    /// What is held right now, by either hand.
    pub fn held(&self) -> Option<&K> {
        self.drag.moving().or(self.carry.as_ref().map(|c| &c.key))
    }

    /// What the keyboard holds. A mouse drag has a ghost to draw; a
    /// carry has no cursor, so the held thing is drawn in the gap
    /// itself — this is how a renderer tells the two apart.
    pub fn carried(&self) -> Option<&K> {
        self.carry.as_ref().map(|c| &c.key)
    }

    /// Where the gap should open this frame: the container and slot the
    /// held thing would land in, whether it hangs from the mouse or the
    /// keyboard. `None` when nothing is held.
    pub fn over(&self) -> Option<(C, usize)> {
        if let Some((x, y)) = self.drag.cursor() {
            return self.place(x, y);
        }
        let c = self.carry.as_ref()?;
        Some((self.cons.get(c.con)?.id.clone(), c.slot))
    }

    /// Where to draw the mouse-held thing, hanging from its grab point.
    /// A keyboard carry has no cursor and no ghost: the gap is where it
    /// is.
    pub fn ghost(&self, within: Rect) -> Option<Rect> {
        self.drag.ghost(within)
    }

    /// The container under this cell, or the nearest one — a drop just
    /// past a border should land, not evaporate.
    fn place(&self, x: u16, y: u16) -> Option<(C, usize)> {
        let con = self.cons.iter().min_by_key(|c| distance(c.area, x, y))?;
        let rects: Vec<Rect> = con.items.iter().map(|(_, r)| *r).collect();
        Some((con.id.clone(), con.start + slot(&rects, x, y)))
    }

    fn hit(&self, x: u16, y: u16) -> Option<(K, Rect)> {
        self.cons
            .iter()
            .rev()
            .flat_map(|c| c.items.iter().rev())
            .find(|(_, r)| r.contains(Position::new(x, y)))
            .map(|(k, r)| (k.clone(), *r))
    }
}

impl<C: Clone + PartialEq, K: Clone + PartialEq> Default for Sortable<C, K> {
    fn default() -> Self {
        Self::new()
    }
}

fn distance(r: Rect, x: u16, y: u16) -> u32 {
    let dx = if x < r.x {
        r.x - x
    } else {
        x.saturating_sub(r.x + r.width.saturating_sub(1))
    };
    let dy = if y < r.y {
        r.y - y
    } else {
        y.saturating_sub(r.y + r.height.saturating_sub(1))
    };
    dx as u32 + dy as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::{KeyModifiers, MouseEventKind};

    fn stack(top: u16, heights: &[u16]) -> Vec<Rect> {
        let mut y = top;
        heights
            .iter()
            .map(|h| {
                let r = Rect::new(2, y, 20, *h);
                y += h + 1;
                r
            })
            .collect()
    }

    #[test]
    fn a_vertical_list_reads_by_vertical_midlines() {
        let rows = stack(1, &[3, 3, 3]);
        assert_eq!(slot(&rows, 10, 0), 0);
        assert_eq!(slot(&rows, 10, 1), 0);
        assert_eq!(slot(&rows, 10, 3), 1); // past the first midline
        assert_eq!(slot(&rows, 10, 4), 1); // in the gap between rows
        assert_eq!(slot(&rows, 10, 7), 2);
        assert_eq!(slot(&rows, 10, 50), 3);
    }

    #[test]
    fn heights_are_measured_not_assumed() {
        // A tall row takes more travel to pass than a short one.
        let rows = stack(0, &[7, 1]);
        assert_eq!(slot(&rows, 10, 2), 0);
        assert_eq!(slot(&rows, 10, 3), 1);
        assert_eq!(slot(&rows, 10, 8), 2);
    }

    #[test]
    fn a_horizontal_strip_reads_by_horizontal_midlines() {
        let row = vec![
            Rect::new(0, 2, 10, 3),
            Rect::new(12, 2, 10, 3),
            Rect::new(24, 2, 10, 3),
        ];
        assert_eq!(slot(&row, 3, 3), 0);
        assert_eq!(slot(&row, 6, 3), 1);
        assert_eq!(slot(&row, 30, 3), 3);
        // Above and below the strip fall before and after everything.
        assert_eq!(slot(&row, 30, 0), 0);
        assert_eq!(slot(&row, 3, 20), 3);
    }

    #[test]
    fn a_grid_reads_row_by_row() {
        // Two rows of two, row-major.
        let grid = vec![
            Rect::new(0, 0, 10, 3),
            Rect::new(12, 0, 10, 3),
            Rect::new(0, 4, 10, 3),
            Rect::new(12, 4, 10, 3),
        ];
        assert_eq!(slot(&grid, 2, 1), 0);
        assert_eq!(slot(&grid, 13, 1), 1);
        assert_eq!(slot(&grid, 20, 1), 2);
        assert_eq!(slot(&grid, 2, 5), 2);
        assert_eq!(slot(&grid, 20, 5), 4);
        assert_eq!(slot(&grid, 20, 30), 4);
    }

    fn sortable() -> Sortable<&'static str, u8> {
        let mut s = Sortable::new();
        s.container(
            "left",
            Rect::new(0, 0, 24, 20),
            &[(1, Rect::new(2, 1, 20, 3)), (2, Rect::new(2, 5, 20, 3))],
        );
        s.container(
            "right",
            Rect::new(26, 0, 24, 20),
            &[(3, Rect::new(28, 1, 20, 3))],
        );
        s
    }

    fn mouse(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::empty(),
        }
    }

    #[test]
    fn a_mouse_drag_resolves_to_a_container_and_slot() {
        let mut s = sortable();
        s.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 2));
        s.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 30, 7));
        assert_eq!(s.held(), Some(&1));
        assert_eq!(s.over(), Some(("right", 1)));
        assert_eq!(
            s.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 30, 7)),
            Act::Drop {
                key: 1,
                container: "right",
                slot: 1
            }
        );
    }

    #[test]
    fn a_drop_off_every_container_lands_on_the_nearest() {
        let mut s = sortable();
        s.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 2));
        s.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 60, 2));
        assert_eq!(
            s.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 60, 2)),
            Act::Drop {
                key: 1,
                container: "right",
                slot: 1
            }
        );
    }

    #[test]
    fn the_keyboard_carries_between_containers() {
        let mut s = sortable();
        s.lift(2);
        assert_eq!(s.held(), Some(&2));
        assert_eq!(s.over(), Some(("left", 1)));
        s.shift(-1);
        assert_eq!(s.over(), Some(("left", 0)));
        s.shift(-1); // already at the top: stays
        assert_eq!(s.over(), Some(("left", 0)));
        s.shift_container(1);
        assert_eq!(s.over(), Some(("right", 0)));
        assert_eq!(s.put(), Some((2, "right", 0)));
        assert!(s.held().is_none());
    }

    #[test]
    fn hooks_tell_the_story_of_a_drag() {
        let mut s = sortable();
        s.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 2));
        s.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 30, 7));
        s.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 31, 7));
        s.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 31, 7));
        assert_eq!(
            s.hooks(),
            vec![
                Hook::Grab {
                    key: 1,
                    from: Some(("left", 0))
                },
                // The gap moved once; the second, same-place drag event
                // reported nothing.
                Hook::Target {
                    key: 1,
                    container: "right",
                    slot: 1
                },
                Hook::Drop {
                    key: 1,
                    from: Some(("left", 0)),
                    container: "right",
                    slot: 1
                },
            ]
        );
        assert!(s.hooks().is_empty());
    }

    #[test]
    fn hooks_tell_the_story_of_a_carry() {
        let mut s = sortable();
        s.lift(2);
        s.shift(-1);
        s.shift(-1); // already at the top: no new target
        s.put();
        assert_eq!(
            s.hooks(),
            vec![
                Hook::Grab {
                    key: 2,
                    from: Some(("left", 1))
                },
                Hook::Target {
                    key: 2,
                    container: "left",
                    slot: 1
                },
                Hook::Target {
                    key: 2,
                    container: "left",
                    slot: 0
                },
                Hook::Drop {
                    key: 2,
                    from: Some(("left", 1)),
                    container: "left",
                    slot: 0
                },
            ]
        );
    }

    #[test]
    fn a_press_that_ends_a_carry_is_a_cancel() {
        let mut s = sortable();
        s.lift(2);
        s.hooks();
        s.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 40, 18));
        assert_eq!(s.hooks(), vec![Hook::Cancel { key: 2 }]);
        s.lift(3);
        s.hooks();
        s.cancel();
        assert_eq!(s.hooks(), vec![Hook::Cancel { key: 3 }]);
    }

    #[test]
    fn a_scrolled_window_answers_in_full_list_terms() {
        let mut s: Sortable<&'static str, u8> = Sortable::new();
        // Rows 6..9 of a longer list are what is on screen.
        s.window(
            "list",
            Rect::new(0, 0, 20, 3),
            &[
                (6, Rect::new(0, 0, 20, 1)),
                (7, Rect::new(0, 1, 20, 1)),
                (8, Rect::new(0, 2, 20, 1)),
            ],
            6,
        );
        // A mouse drop lands where the row really is in the full list.
        s.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 5, 0));
        s.on_mouse(mouse(MouseEventKind::Drag(MouseButton::Left), 5, 1));
        assert_eq!(
            s.on_mouse(mouse(MouseEventKind::Up(MouseButton::Left), 5, 1)),
            Act::Drop {
                key: 6,
                container: "list",
                slot: 8
            }
        );
        // A carry starts at its full-list index and can step one past
        // the bottom of the window, where scrolling will catch up.
        s.lift(7);
        assert_eq!(s.over(), Some(("list", 7)));
        s.shift(100);
        assert_eq!(s.over(), Some(("list", 9)));
    }

    #[test]
    fn a_mouse_press_ends_a_carry() {
        let mut s = sortable();
        s.lift(2);
        s.on_mouse(mouse(MouseEventKind::Down(MouseButton::Left), 40, 18));
        assert!(s.held().is_none());
    }
}
