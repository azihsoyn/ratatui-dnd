//! A row-major grid of tiles, sorted like anything else.
//!
//! Nothing here tells the crate it is looking at a grid: slots are
//! measured from the rectangles the tiles were drawn in, and the same
//! midline rule that walks a list walks the rows of a grid.
//!
//! Mouse: drag tiles. Keyboard: arrows move, space lifts, ←/→ carry a
//! step, ↑/↓ carry a row, space drops, esc lets go, q quits.

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use ratatui_dnd::{Act, Sortable};
use std::time::Duration;

const TICK: Duration = Duration::from_millis(100);
const COLS: isize = 4;
const TILE_W: u16 = 16;
const TILE_H: u16 = 3;

struct App {
    tiles: Vec<(u64, &'static str, Color)>,
    sort: Sortable<u8, u64>,
    cursor: usize,
}

impl App {
    fn seed() -> Self {
        let names: [(&str, Color); 8] = [
            ("mercury", Color::Gray),
            ("venus", Color::Yellow),
            ("earth", Color::Blue),
            ("mars", Color::Red),
            ("jupiter", Color::LightRed),
            ("saturn", Color::LightYellow),
            ("uranus", Color::Cyan),
            ("neptune", Color::LightBlue),
        ];
        let tiles = names
            .iter()
            .enumerate()
            .map(|(i, (n, c))| (i as u64 + 1, *n, *c))
            .collect();
        App {
            tiles,
            sort: Sortable::new(),
            cursor: 0,
        }
    }

    fn apply(&mut self, id: u64, slot: usize) {
        let Some(idx) = self.tiles.iter().position(|(k, ..)| *k == id) else {
            return;
        };
        let tile = self.tiles.remove(idx);
        let slot = slot.min(self.tiles.len());
        self.tiles.insert(slot, tile);
        self.cursor = slot;
    }

    fn on_mouse(&mut self, m: event::MouseEvent) {
        match self.sort.on_mouse(m) {
            Act::Drop { key, slot, .. } => self.apply(key, slot),
            Act::Click(id) => {
                if let Some(i) = self.tiles.iter().position(|(k, ..)| *k == id) {
                    self.cursor = i;
                }
            }
            _ => {}
        }
    }

    fn on_key(&mut self, code: KeyCode) -> bool {
        if self.sort.held().is_some() {
            match code {
                KeyCode::Left | KeyCode::Char('h') => self.sort.shift(-1),
                KeyCode::Right | KeyCode::Char('l') => self.sort.shift(1),
                KeyCode::Up | KeyCode::Char('k') => self.sort.shift(-COLS),
                KeyCode::Down | KeyCode::Char('j') => self.sort.shift(COLS),
                KeyCode::Char(' ') | KeyCode::Char('　') | KeyCode::Enter => {
                    if let Some((id, _, slot)) = self.sort.put() {
                        self.apply(id, slot);
                    }
                }
                KeyCode::Esc => self.sort.cancel(),
                _ => {}
            }
            return false;
        }
        let last = self.tiles.len().saturating_sub(1);
        match code {
            KeyCode::Left | KeyCode::Char('h') => self.cursor = self.cursor.saturating_sub(1),
            KeyCode::Right | KeyCode::Char('l') => self.cursor = (self.cursor + 1).min(last),
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(COLS as usize);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + COLS as usize).min(last);
            }
            KeyCode::Char(' ') | KeyCode::Char('　') | KeyCode::Enter => {
                if let Some((id, ..)) = self.tiles.get(self.cursor) {
                    self.sort.lift(*id);
                }
            }
            KeyCode::Char('q') | KeyCode::Esc => return true,
            _ => {}
        }
        false
    }

    fn render(&mut self, f: &mut ratatui::Frame) {
        let [board, status] =
            Layout::vertical([Constraint::Min(0), Constraint::Length(1)]).areas(f.area());
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray))
            .title(" nearest first ")
            .title_style(
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            );
        let body = block.inner(board);
        f.render_widget(block, board);

        let over = self.sort.over().map(|(_, s)| s);
        let held = self.sort.held().copied();
        let carried = self.sort.carried().copied();

        // Reflow by hand: the held tile out, a hole in. Row-major with
        // a cell of air, which is all the "grid" there is.
        let mut cells: Vec<Option<(u64, &str, Color)>> = self
            .tiles
            .iter()
            .filter(|(id, ..)| held != Some(*id))
            .map(|t| Some(*t))
            .collect();
        if let Some(slot) = over {
            cells.insert(slot.min(cells.len()), None);
        }

        let mut spots: Vec<(u64, Rect)> = Vec::new();
        for (i, cell) in cells.iter().enumerate() {
            let (col, row) = (i as u16 % COLS as u16, i as u16 / COLS as u16);
            let r = Rect::new(
                body.x + col * (TILE_W + 1),
                body.y + row * (TILE_H + 1),
                TILE_W,
                TILE_H,
            )
            .intersection(body);
            if r.height < 2 {
                continue;
            }
            match cell {
                Some((id, name, color)) => {
                    let edge = if held.is_none() && self.cursor == i {
                        Color::White
                    } else {
                        *color
                    };
                    draw_tile(f, r, name, *color, edge);
                    spots.push((*id, r));
                }
                None => match carried.and_then(|id| self.tile_of(id)) {
                    Some((name, color)) => draw_tile(f, r, name, color, Color::Yellow),
                    None => f.render_widget(
                        Block::bordered()
                            .border_type(BorderType::Rounded)
                            .border_style(Style::default().fg(Color::Cyan)),
                        r,
                    ),
                },
            }
        }
        self.sort.container(0, body, &spots);

        if let Some(g) = self.sort.ghost(f.area())
            && let Some((name, color)) = held.and_then(|id| self.tile_of(id))
        {
            f.render_widget(Clear, g);
            draw_tile(f, g, name, color, Color::Yellow);
        }

        let hint = if held.is_some() {
            " carrying: ←/→ a step, ↑/↓ a row · space drops · esc lets go"
        } else {
            " drag with the mouse, or: arrows move · space lifts · q quits"
        };
        f.render_widget(
            Paragraph::new(hint).style(Style::default().fg(Color::DarkGray)),
            status,
        );
    }

    fn tile_of(&self, id: u64) -> Option<(&'static str, Color)> {
        self.tiles
            .iter()
            .find(|(k, ..)| *k == id)
            .map(|(_, n, c)| (*n, *c))
    }
}

fn draw_tile(f: &mut ratatui::Frame, r: Rect, name: &str, color: Color, edge: Color) {
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::default().fg(edge));
    let inner = block.inner(r);
    f.render_widget(block, r);
    f.render_widget(
        Paragraph::new(name).style(Style::default().fg(color).add_modifier(Modifier::BOLD)),
        inner,
    );
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
