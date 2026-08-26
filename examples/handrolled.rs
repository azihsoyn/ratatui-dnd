//! The same sortable list as `list.rs`, with the crate left out.
//!
//! This file exists to be read next to `list.rs`: the model, the
//! rendering, and the key map are copied line for line, and everything
//! else — the mouse state machine, the press that is still a click
//! until it moves, the stuck drag a lost mouse-up leaves behind, the
//! grab offset, the ghost that must not leave the frame, the midline
//! arithmetic, and a second, separate way of holding a row for the
//! keyboard — is written out by hand. Every line that is not in
//! `list.rs` is what ratatui-dnd is for. And this is the small case:
//! one list. Lanes, grids, and scrolled windows each grow their own
//! arithmetic from here.
//!
//! Mouse: drag rows. Keyboard: ↑/↓ move, space lifts, ↑/↓ carry,
//! space drops, esc lets go, q quits.

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseButton, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, List, ListState, Paragraph};
use std::time::Duration;

const TICK: Duration = Duration::from_millis(100);

// ---- everything the crate would otherwise hold ----------------------

/// The mouse, holding a row or on its way to holding one.
enum Drag {
    Idle,
    /// Pressed on a row but not yet moved: could still be a click, so
    /// nothing is lifted until the cursor leaves the press cell.
    Armed {
        id: u64,
        rect: Rect,
        x: u16,
        y: u16,
    },
    /// Holding it. `dx`/`dy` is where inside the row it was grabbed,
    /// so the ghost hangs from the same point.
    Moving {
        id: u64,
        rect: Rect,
        dx: u16,
        dy: u16,
        x: u16,
        y: u16,
    },
}

struct App {
    rows: Vec<(u64, String)>,
    state: ListState,
    drag: Drag,
    /// The keyboard's own way of holding a row: which one, and which
    /// gap it hovers over. A separate state from the mouse's, with its
    /// own lift, step, and drop.
    carry: Option<(u64, usize)>,
    /// Where each row was drawn last frame; what a mouse coordinate
    /// means.
    spots: Vec<(u64, Rect)>,
}

impl App {
    fn seed() -> Self {
        let rows = [
            "pour the water before the grounds",
            "bloom for thirty seconds",
            "stir once, not twice",
            "start the timer late",
            "grind a little coarser",
            "warm the cup first",
        ]
        .iter()
        .enumerate()
        .map(|(i, s)| (i as u64 + 1, s.to_string()))
        .collect();
        let mut state = ListState::default();
        state.select(Some(0));
        App {
            rows,
            state,
            drag: Drag::Idle,
            carry: None,
            spots: Vec::new(),
        }
    }

    /// What a press at this cell touches. Later entries win, because
    /// whatever was drawn later is drawn on top.
    fn hit(&self, x: u16, y: u16) -> Option<(u64, Rect)> {
        self.spots
            .iter()
            .rev()
            .find(|(_, r)| r.contains(Position::new(x, y)))
            .copied()
    }

    /// Which gap a drop at `y` means: rows are passed at their
    /// midlines. Only vertical, only this one list — a second lane
    /// would need the nearest-container search too.
    fn slot_at(&self, y: u16) -> usize {
        self.spots
            .iter()
            .filter(|(_, r)| y >= r.y + r.height / 2)
            .count()
    }

    /// What is held right now, by either hand.
    fn held(&self) -> Option<u64> {
        match &self.drag {
            Drag::Moving { id, .. } => Some(*id),
            _ => self.carry.map(|(id, _)| id),
        }
    }

    /// Where the gap should open this frame, by either hand.
    fn over(&self) -> Option<usize> {
        match &self.drag {
            Drag::Moving { y, .. } => Some(self.slot_at(*y)),
            _ => self.carry.map(|(_, slot)| slot),
        }
    }

    /// Where to draw the held row: its own size, hanging from the grab
    /// point, and never past the edges of the frame.
    fn ghost(&self, within: Rect) -> Option<Rect> {
        let Drag::Moving {
            rect, dx, dy, x, y, ..
        } = &self.drag
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

    fn on_mouse(&mut self, m: event::MouseEvent) {
        let (x, y) = (m.column, m.row);
        match m.kind {
            // A press always resets: the mouse wins over a carry, and
            // a drag stuck on a lost mouse-up ends here rather than
            // living forever.
            MouseEventKind::Down(MouseButton::Left) => {
                self.carry = None;
                self.drag = match self.hit(x, y) {
                    Some((id, rect)) => Drag::Armed { id, rect, x, y },
                    None => Drag::Idle,
                };
            }
            MouseEventKind::Drag(MouseButton::Left) => {
                self.drag = match std::mem::replace(&mut self.drag, Drag::Idle) {
                    Drag::Armed {
                        id,
                        rect,
                        x: px,
                        y: py,
                    } if x == px && y == py => {
                        // Wobble on the press cell is still a click in
                        // the making.
                        Drag::Armed {
                            id,
                            rect,
                            x: px,
                            y: py,
                        }
                    }
                    Drag::Armed {
                        id,
                        rect,
                        x: px,
                        y: py,
                    } => Drag::Moving {
                        id,
                        rect,
                        dx: px.saturating_sub(rect.x),
                        dy: py.saturating_sub(rect.y),
                        x,
                        y,
                    },
                    Drag::Moving {
                        id, rect, dx, dy, ..
                    } => Drag::Moving {
                        id,
                        rect,
                        dx,
                        dy,
                        x,
                        y,
                    },
                    Drag::Idle => Drag::Idle,
                };
            }
            MouseEventKind::Up(MouseButton::Left) => {
                match std::mem::replace(&mut self.drag, Drag::Idle) {
                    // Released without moving: a click, not a drop.
                    Drag::Armed { id, .. } => {
                        self.state
                            .select(self.rows.iter().position(|(k, _)| *k == id));
                    }
                    Drag::Moving { id, y, .. } => {
                        let slot = self.slot_at(y);
                        self.apply(id, slot);
                    }
                    Drag::Idle => {}
                }
            }
            _ => {}
        }
    }

    // ---- from here down, `list.rs` again, line for line ------------

    fn apply(&mut self, id: u64, slot: usize) {
        let Some(idx) = self.rows.iter().position(|(k, _)| *k == id) else {
            return;
        };
        let row = self.rows.remove(idx);
        let slot = slot.min(self.rows.len());
        self.rows.insert(slot, row);
        self.state.select(Some(slot));
    }

    fn on_key(&mut self, code: KeyCode) -> bool {
        if self.held().is_some() {
            match code {
                KeyCode::Up | KeyCode::Char('k') => {
                    if let Some((_, slot)) = &mut self.carry {
                        *slot = slot.saturating_sub(1);
                    }
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    if let Some((_, slot)) = &mut self.carry {
                        *slot = (*slot + 1).min(self.rows.len().saturating_sub(1));
                    }
                }
                KeyCode::Char(' ') | KeyCode::Char('　') | KeyCode::Enter => {
                    if let Some((id, slot)) = self.carry.take() {
                        self.apply(id, slot);
                    }
                }
                KeyCode::Esc => {
                    self.carry = None;
                    self.drag = Drag::Idle;
                }
                _ => {}
            }
            return false;
        }
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.state.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.state.select_next(),
            KeyCode::Char(' ') | KeyCode::Char('　') | KeyCode::Enter => {
                if let Some((i, (id, _))) = self
                    .state
                    .selected()
                    .and_then(|i| self.rows.get(i).map(|r| (i, r)))
                {
                    self.carry = Some((*id, i));
                }
            }
            KeyCode::Char('q') | KeyCode::Esc => return true,
            _ => {}
        }
        false
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" a pour-over, by hand ")
            .title_style(
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            )
            .title_bottom(
                Line::from(" space lifts · q quits ")
                    .style(Style::default().fg(Color::DarkGray))
                    .right_aligned(),
            );
        let body = block.inner(f.area());
        f.render_widget(block, f.area());

        let over = self.over();
        let held = self.held();
        let carried = self.carry.map(|(id, _)| id);

        // Build what List will draw: the held row out, a placeholder in.
        let mut lines: Vec<(Option<u64>, Line)> = Vec::new();
        for (id, text) in &self.rows {
            if held == Some(*id) {
                continue;
            }
            lines.push((Some(*id), Line::from(format!("  {text}"))));
        }
        if let Some(slot) = over {
            let slot = slot.min(lines.len());
            let line = match carried.and_then(|id| self.text_of(id)) {
                Some(text) => {
                    Line::from(format!("  {text}")).style(Style::default().fg(Color::Yellow))
                }
                None => Line::from("  · · ·").style(Style::default().fg(Color::Cyan)),
            };
            lines.insert(slot, (None, line));
        }

        self.spots = lines
            .iter()
            .enumerate()
            .filter_map(|(i, (id, _))| {
                let r = Rect::new(body.x, body.y + i as u16, body.width, 1).intersection(body);
                id.filter(|_| r.height == 1).map(|id| (id, r))
            })
            .collect();

        let list = List::new(lines.into_iter().map(|(_, l)| l))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        if held.is_some() {
            self.state.select(None);
        } else if self.state.selected().is_none() {
            self.state.select(Some(0));
        }
        f.render_stateful_widget(list, body, &mut self.state);

        if let Some(g) = self.ghost(f.area())
            && let Some(text) = held.and_then(|id| self.text_of(id))
        {
            f.render_widget(Clear, g);
            f.render_widget(
                Paragraph::new(format!("  {text}")).style(Style::default().fg(Color::Yellow)),
                g,
            );
        }
    }

    fn text_of(&self, id: u64) -> Option<String> {
        self.rows
            .iter()
            .find(|(k, _)| *k == id)
            .map(|(_, t)| t.clone())
    }
}

fn main() -> Result<()> {
    let mut terminal = ratatui::init();
    let _ = crossterm::execute!(std::io::stdout(), EnableMouseCapture);
    let out = run(&mut terminal);
    let _ = crossterm::execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    out
}

fn run(terminal: &mut ratatui::DefaultTerminal) -> Result<()> {
    let mut app = App::seed();
    loop {
        terminal.draw(|f| app.render(f))?;
        if !event::poll(TICK)? {
            continue;
        }
        loop {
            match event::read()? {
                Event::Key(k) if k.kind == KeyEventKind::Press => {
                    if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                        return Ok(());
                    }
                    if app.on_key(k.code) {
                        return Ok(());
                    }
                }
                Event::Mouse(m) => app.on_mouse(m),
                _ => {}
            }
            if !event::poll(Duration::ZERO)? {
                break;
            }
        }
    }
}
