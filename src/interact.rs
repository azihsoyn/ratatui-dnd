//! The ground floor: pointing at things and holding them.
//!
//! A terminal has no notion of picking things up, hovering them, or
//! grabbing their corners; all it gives you is a stream of cell
//! coordinates. These are the two parts every interaction needs on top
//! of that, and nothing more: a state machine that turns raw mouse
//! events into click / lift / move / drop, and a per-frame map from
//! screen cells back to the things drawn on them. Nothing here knows
//! about sorting — a scrubber head, a chart brush, or a resize handle
//! sits on this floor just as well.

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};

/// What a mouse event meant, once the state machine has seen it.
#[derive(Debug, PartialEq)]
pub enum Did<K> {
    Nothing,
    /// Pressed and released without moving. Not this module's business,
    /// but the caller should not have to track presses to tell.
    Click(K),
    /// The press has become a drag: time to take the thing out of the flow.
    Lift(K),
    /// The cursor moved while holding something.
    Move,
    /// Released while holding something, at this cell.
    Drop {
        key: K,
        x: u16,
        y: u16,
    },
}

enum State<K> {
    Idle,
    /// Pressed on something but not yet moved: could still be a click,
    /// so nothing is lifted until the cursor leaves the press cell.
    Armed {
        key: K,
        rect: Rect,
        x: u16,
        y: u16,
    },
    /// Holding it. `dx`/`dy` is where inside the thing it was grabbed,
    /// so a ghost can hang from the same point instead of snapping to
    /// its corner.
    Moving {
        key: K,
        rect: Rect,
        dx: u16,
        dy: u16,
        x: u16,
        y: u16,
    },
}

/// The drag itself: feed it every mouse event plus what was under the
/// cursor, and it says what the event amounted to.
pub struct Drag<K> {
    state: State<K>,
}

impl<K: Clone> Drag<K> {
    pub fn new() -> Self {
        Self { state: State::Idle }
    }

    /// `hit` is what a left press at this cell would pick up, with the
    /// rectangle it was drawn in. It is only looked at on the press;
    /// afterwards the drag keeps what it grabbed.
    pub fn on_mouse(&mut self, ev: MouseEvent, hit: Option<(K, Rect)>) -> Did<K> {
        match ev.kind {
            // A press always resets: if an Up was lost to focus changes,
            // the stuck drag ends here rather than living forever.
            MouseEventKind::Down(MouseButton::Left) => {
                self.state = match hit {
                    Some((key, rect)) => State::Armed {
                        key,
                        rect,
                        x: ev.column,
                        y: ev.row,
                    },
                    None => State::Idle,
                };
                Did::Nothing
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                match std::mem::replace(&mut self.state, State::Idle) {
                    State::Armed { key, rect, x, y } if ev.column == x && ev.row == y => {
                        self.state = State::Armed { key, rect, x, y };
                        Did::Nothing
                    }
                    State::Armed { key, rect, x, y } => {
                        let lifted = key.clone();
                        self.state = State::Moving {
                            key,
                            rect,
                            dx: x.saturating_sub(rect.x),
                            dy: y.saturating_sub(rect.y),
                            x: ev.column,
                            y: ev.row,
                        };
                        Did::Lift(lifted)
                    }
                    State::Moving {
                        key, rect, dx, dy, ..
                    } => {
                        self.state = State::Moving {
                            key,
                            rect,
                            dx,
                            dy,
                            x: ev.column,
                            y: ev.row,
                        };
                        Did::Move
                    }
                    State::Idle => Did::Nothing,
                }
            }
            MouseEventKind::Up(MouseButton::Left) => {
                match std::mem::replace(&mut self.state, State::Idle) {
                    State::Armed { key, .. } => Did::Click(key),
                    State::Moving { key, .. } => Did::Drop {
                        key,
                        x: ev.column,
                        y: ev.row,
                    },
                    State::Idle => Did::Nothing,
                }
            }
            _ => Did::Nothing,
        }
    }

    /// Let go of whatever is held without dropping it anywhere.
    pub fn cancel(&mut self) {
        self.state = State::Idle;
    }

    /// What is being held, if a drag is underway.
    pub fn moving(&self) -> Option<&K> {
        match &self.state {
            State::Moving { key, .. } => Some(key),
            _ => None,
        }
    }

    /// Where the cursor is, while a drag is underway.
    pub fn cursor(&self) -> Option<(u16, u16)> {
        match &self.state {
            State::Moving { x, y, .. } => Some((*x, *y)),
            _ => None,
        }
    }

    /// Where to draw the held thing: its own size, hanging from the grab
    /// point, and never past the edges of `within`.
    pub fn ghost(&self, within: Rect) -> Option<Rect> {
        let State::Moving {
            rect, dx, dy, x, y, ..
        } = &self.state
        else {
            return None;
        };
        let w = rect.width.min(within.width);
        let h = rect.height.min(within.height);
        let gx = x
            .saturating_sub(*dx)
            .max(within.x)
            .min(within.x + within.width - w);
        let gy = y
            .saturating_sub(*dy)
            .max(within.y)
            .min(within.y + within.height - h);
        Some(Rect::new(gx, gy, w, h))
    }
}

impl<K: Clone> Default for Drag<K> {
    fn default() -> Self {
        Self::new()
    }
}

/// A map from screen cells back to the things drawn on them, rebuilt
/// every frame while rendering. The screen is the only place layout is
/// known, so the renderer records it here and the event handler asks.
pub struct Hits<K> {
    spots: Vec<(Rect, K)>,
}

impl<K: Clone> Hits<K> {
    pub fn new() -> Self {
        Self { spots: Vec::new() }
    }

    pub fn clear(&mut self) {
        self.spots.clear();
    }

    pub fn put(&mut self, area: Rect, key: K) {
        self.spots.push((area, key));
    }

    /// What a press at this cell touches. Later entries win, because
    /// whatever was drawn later is drawn on top.
    pub fn at(&self, x: u16, y: u16) -> Option<(K, Rect)> {
        self.spots
            .iter()
            .rev()
            .find(|(r, _)| r.contains(Position::new(x, y)))
            .map(|(r, k)| (k.clone(), *r))
    }
}

impl<K: Clone> Default for Hits<K> {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn ev(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
        MouseEvent {
            kind,
            column: x,
            row: y,
            modifiers: KeyModifiers::empty(),
        }
    }

    fn down(x: u16, y: u16) -> MouseEvent {
        ev(MouseEventKind::Down(MouseButton::Left), x, y)
    }

    fn drag(x: u16, y: u16) -> MouseEvent {
        ev(MouseEventKind::Drag(MouseButton::Left), x, y)
    }

    fn up(x: u16, y: u16) -> MouseEvent {
        ev(MouseEventKind::Up(MouseButton::Left), x, y)
    }

    #[test]
    fn press_and_release_is_a_click() {
        let mut d: Drag<u8> = Drag::new();
        assert_eq!(
            d.on_mouse(down(5, 5), Some((7, Rect::new(4, 4, 10, 3)))),
            Did::Nothing
        );
        assert_eq!(d.on_mouse(up(5, 5), None), Did::Click(7));
        assert!(d.moving().is_none());
    }

    #[test]
    fn moving_off_the_press_cell_lifts_then_drops() {
        let mut d: Drag<u8> = Drag::new();
        d.on_mouse(down(5, 5), Some((7, Rect::new(4, 4, 10, 3))));
        // Wobble on the press cell is still a click in the making.
        assert_eq!(d.on_mouse(drag(5, 5), None), Did::Nothing);
        assert_eq!(d.on_mouse(drag(6, 5), None), Did::Lift(7));
        assert_eq!(d.moving(), Some(&7));
        assert_eq!(d.on_mouse(drag(9, 8), None), Did::Move);
        assert_eq!(d.cursor(), Some((9, 8)));
        assert_eq!(d.on_mouse(up(9, 8), None), Did::Drop { key: 7, x: 9, y: 8 });
        assert!(d.moving().is_none());
    }

    #[test]
    fn pressing_on_nothing_drags_nothing() {
        let mut d: Drag<u8> = Drag::new();
        d.on_mouse(down(5, 5), None);
        assert_eq!(d.on_mouse(drag(9, 9), None), Did::Nothing);
        assert_eq!(d.on_mouse(up(9, 9), None), Did::Nothing);
    }

    #[test]
    fn cancel_lets_go_without_a_drop() {
        let mut d: Drag<u8> = Drag::new();
        d.on_mouse(down(5, 5), Some((7, Rect::new(4, 4, 10, 3))));
        d.on_mouse(drag(9, 9), None);
        d.cancel();
        assert!(d.moving().is_none());
        assert_eq!(d.on_mouse(up(9, 9), None), Did::Nothing);
    }

    #[test]
    fn the_ghost_hangs_from_the_grab_point_and_stays_inside() {
        let mut d: Drag<u8> = Drag::new();
        // Grabbed two cells in from the left edge of a 10x3 thing.
        d.on_mouse(down(6, 5), Some((7, Rect::new(4, 4, 10, 3))));
        d.on_mouse(drag(20, 10), None);
        let frame = Rect::new(0, 0, 80, 24);
        assert_eq!(d.ghost(frame), Some(Rect::new(18, 9, 10, 3)));
        // Dragged past the corner, the ghost stops at the edge.
        d.on_mouse(drag(79, 23), None);
        assert_eq!(d.ghost(frame), Some(Rect::new(70, 21, 10, 3)));
        d.on_mouse(drag(0, 0), None);
        assert_eq!(d.ghost(frame), Some(Rect::new(0, 0, 10, 3)));
    }

    #[test]
    fn hits_prefer_what_was_drawn_last() {
        let mut h: Hits<u8> = Hits::new();
        h.put(Rect::new(0, 0, 10, 10), 1);
        h.put(Rect::new(5, 5, 10, 10), 2);
        assert_eq!(h.at(7, 7).map(|(k, _)| k), Some(2));
        assert_eq!(h.at(2, 2).map(|(k, _)| k), Some(1));
        assert_eq!(h.at(30, 30), None);
    }
}
