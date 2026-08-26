//! A list too long for the screen, inside tui-scrollview — and still
//! sortable.
//!
//! The point of this example: the crate composes with widgets from the
//! wider ratatui ecosystem, and it never learns about scrolling. Rows
//! are registered where they actually land on screen — their content
//! position minus the scroll offset the `ScrollView` reports — and rows
//! that scrolled out of view are simply not registered.
//!
//! Mouse: wheel scrolls, dragging a row sorts. Keyboard: ↑/↓ move,
//! space lifts, ↑/↓ carry, space drops, esc lets go, q quits.

use anyhow::Result;
use crossterm::event::{
    self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, KeyEventKind, KeyModifiers,
    MouseEventKind,
};
use ratatui::layout::{Position, Rect, Size};
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use ratatui_dnd::{Act, Sortable};
use std::time::Duration;
use tui_scrollview::{ScrollView, ScrollViewState};

const TICK: Duration = Duration::from_millis(100);

struct App {
    rows: Vec<(u64, String)>,
    sort: Sortable<u8, u64>,
    scroll: ScrollViewState,
    cursor: usize,
    /// How many rows fit, as of the last frame — what "on screen" means
    /// when a key moves the cursor or the gap.
    view_h: u16,
}

impl App {
    fn seed() -> Self {
        let chores = [
            "water the ferns",
            "sharpen the knives",
            "oil the door that sings",
            "answer the long letter",
            "sweep behind the stove",
            "mend the bicycle bell",
            "sort the seed packets",
            "wind the hall clock",
        ];
        let rows = (0..32)
            .map(|i| {
                (
                    i + 1,
                    format!("{:02} · {}", i + 1, chores[i as usize % chores.len()]),
                )
            })
            .collect();
        App {
            rows,
            sort: Sortable::new(),
            scroll: ScrollViewState::new(),
            cursor: 0,
            view_h: 1,
        }
    }

    /// Scroll just enough that row `want` is visible. Only keys call
    /// this: the wheel must stay free to scroll away from the cursor.
    fn ensure(&mut self, want: usize) {
        let want = want as u16;
        let off = self.scroll.offset().y;
        if want < off {
            self.scroll.set_offset(Position::new(0, want));
        } else if want >= off + self.view_h {
            self.scroll
                .set_offset(Position::new(0, want + 1 - self.view_h));
        }
    }

    fn apply(&mut self, id: u64, slot: usize) {
        let Some(idx) = self.rows.iter().position(|(k, _)| *k == id) else {
            return;
        };
        let row = self.rows.remove(idx);
        let slot = slot.min(self.rows.len());
        self.rows.insert(slot, row);
        self.cursor = slot;
        self.ensure(slot);
    }

    fn on_mouse(&mut self, m: event::MouseEvent) {
        match m.kind {
            MouseEventKind::ScrollUp => {
                for _ in 0..3 {
                    self.scroll.scroll_up();
                }
            }
            MouseEventKind::ScrollDown => {
                for _ in 0..3 {
                    self.scroll.scroll_down();
                }
            }
            _ => {}
        }
        match self.sort.on_mouse(m) {
            Act::Drop { key, slot, .. } => self.apply(key, slot),
            Act::Click(id) => {
                if let Some(i) = self.rows.iter().position(|(k, _)| *k == id) {
                    self.cursor = i;
                }
            }
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
            if let Some((_, slot)) = self.sort.over() {
                self.ensure(slot);
            }
            return false;
        }
        match code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.cursor = self.cursor.saturating_sub(1);
                self.ensure(self.cursor);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.cursor = (self.cursor + 1).min(self.rows.len().saturating_sub(1));
                self.ensure(self.cursor);
            }
            KeyCode::PageUp => self.scroll.scroll_page_up(),
            KeyCode::PageDown => self.scroll.scroll_page_down(),
            KeyCode::Char(' ') | KeyCode::Char('　') | KeyCode::Enter => {
                if let Some((id, _)) = self.rows.get(self.cursor) {
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
            .title(format!(" a week's worth · {} ", self.rows.len()))
            .title_style(
                Style::default()
                    .fg(Color::Gray)
                    .add_modifier(Modifier::BOLD),
            )
            .title_bottom(" wheel scrolls · space lifts · q quits ");
        let body = block.inner(f.area());
        f.render_widget(block, f.area());

        let over = self.sort.over().map(|(_, s)| s);
        let held = self.sort.held().copied();
        let carried = self.sort.carried().copied();

        // Build the content: the held row out, a placeholder in.
        let mut lines: Vec<(Option<u64>, String, Style)> = Vec::new();
        for (i, (id, text)) in self.rows.iter().enumerate() {
            if held == Some(*id) {
                continue;
            }
            let style = if held.is_none() && i == self.cursor {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            lines.push((Some(*id), format!("  {text}"), style));
        }
        if let Some(slot) = over {
            let slot = slot.min(lines.len());
            let line = match carried.and_then(|id| self.text_of(id)) {
                Some(text) => (
                    None,
                    format!("  {text}"),
                    Style::default().fg(Color::Yellow),
                ),
                None => (None, "  · · ·".into(), Style::default().fg(Color::Cyan)),
            };
            lines.insert(slot, line);
        }

        self.view_h = body.height;
        let off = self.scroll.offset().y;

        // The ScrollView draws the content; we only say where each row
        // landed on the actual screen, and skip the ones that didn't.
        let width = body.width.saturating_sub(1); // room for the scrollbar
        let mut view = ScrollView::new(Size::new(width, lines.len() as u16));
        let mut spots: Vec<(u64, Rect)> = Vec::new();
        for (i, (id, text, style)) in lines.iter().enumerate() {
            view.render_widget(
                Paragraph::new(text.as_str()).style(*style),
                Rect::new(0, i as u16, width, 1),
            );
            let i = i as u16;
            if let Some(id) = id
                && i >= off
                && i - off < body.height
            {
                spots.push((*id, Rect::new(body.x, body.y + i - off, width, 1)));
            }
        }
        // Only the visible rows are registered, so say how many rows
        // sit above the window — the placeholder is not a row.
        let start = lines[..(off as usize).min(lines.len())]
            .iter()
            .filter(|(id, ..)| id.is_some())
            .count();
        self.sort.window(0, body, &spots, start);
        f.render_stateful_widget(view, body, &mut self.scroll);

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
