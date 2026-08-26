//! ratatui's own `List` widget, made sortable.
//!
//! The point of this example: the crate attaches to a widget it has
//! never heard of. `List` draws the rows; we merely know where they
//! land (one line each, from the top of the inner area) and register
//! that. While a row is held we build the display list ourselves —
//! the held row taken out, a placeholder line where it would drop —
//! which is the same trick any widget-backed list can play.
//!
//! Mouse: drag rows. Keyboard: ↑/↓ move, space lifts, ↑/↓ carry,
//! space drops, esc lets go, q quits.

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
};
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, BorderType, Clear, List, ListState, Paragraph};
use ratatui_dnd::{Act, Sortable};
use std::time::Duration;

const TICK: Duration = Duration::from_millis(100);

struct App {
    rows: Vec<(u64, String)>,
    sort: Sortable<u8, u64>,
    state: ListState,
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
            sort: Sortable::new(),
            state,
        }
    }

    fn apply(&mut self, id: u64, slot: usize) {
        let Some(idx) = self.rows.iter().position(|(k, _)| *k == id) else {
            return;
        };
        let row = self.rows.remove(idx);
        let slot = slot.min(self.rows.len());
        self.rows.insert(slot, row);
        self.state.select(Some(slot));
    }

    fn on_mouse(&mut self, m: event::MouseEvent) {
        match self.sort.on_mouse(m) {
            Act::Drop { key, slot, .. } => self.apply(key, slot),
            Act::Click(id) => self
                .state
                .select(self.rows.iter().position(|(k, _)| *k == id)),
            _ => {}
        }
    }

    fn on_key(&mut self, code: KeyCode) -> bool {
        if self.sort.held().is_some() {
            match code {
                KeyCode::Up | KeyCode::Char('k') => self.sort.shift(-1),
                KeyCode::Down | KeyCode::Char('j') => self.sort.shift(1),
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
        match code {
            KeyCode::Up | KeyCode::Char('k') => self.state.select_previous(),
            KeyCode::Down | KeyCode::Char('j') => self.state.select_next(),
            KeyCode::Char(' ') | KeyCode::Char('　') | KeyCode::Enter => {
                if let Some((id, _)) = self.state.selected().and_then(|i| self.rows.get(i)) {
                    self.sort.lift(*id);
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
            .title(" a pour-over, in order ")
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

        let over = self.sort.over().map(|(_, s)| s);
        let held = self.sort.held().copied();
        let carried = self.sort.carried().copied();

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

        // The rows land one per line from the top of the inner area —
        // that is the whole contract List has to meet to be sortable.
        let spots: Vec<(u64, Rect)> = lines
            .iter()
            .enumerate()
            .filter_map(|(i, (id, _))| {
                let r = Rect::new(body.x, body.y + i as u16, body.width, 1).intersection(body);
                id.filter(|_| r.height == 1).map(|id| (id, r))
            })
            .collect();
        self.sort.container(0, body, &spots);

        let list = List::new(lines.into_iter().map(|(_, l)| l))
            .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
        if held.is_some() {
            self.state.select(None);
        } else if self.state.selected().is_none() {
            self.state.select(Some(0));
        }
        f.render_stateful_widget(list, body, &mut self.state);

        if let Some(g) = self.sort.ghost(f.area())
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
